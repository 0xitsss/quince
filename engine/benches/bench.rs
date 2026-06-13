use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use quince_core::types::{Side, Trade};
use quince_engine::indicators::{parse_using, IndicatorBank};
use quince_qfl::runtime::QflRuntime;

const STRATEGIES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../strategies/");
const TICK_ITERS: u64 = 10_000;

fn strategies() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(STRATEGIES_DIR)
        .expect("strategies dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "qfl"))
        .map(|e| e.path().file_stem().unwrap().to_string_lossy().to_string())
        .collect();
    files.sort();
    files
}

fn strategy_path(name: &str) -> String {
    std::path::Path::new(STRATEGIES_DIR)
        .join(format!("{}.qfl", name))
        .to_string_lossy()
        .to_string()
}

fn generate_trades(n: u64) -> Vec<Trade> {
    (0..n)
        .map(|i| {
            let phase = (i as f64) * 0.01;
            let price = 50000.0 + 1000.0 * phase.sin();
            let qty = 0.1 + (i % 5) as f64 * 0.1;
            Trade {
                price,
                qty,
                time: chrono::Utc::now(),
                side: if i % 2 == 0 { Side::Buy } else { Side::Sell },
                trade_id: i,
            }
        })
        .collect()
}

fn make_indicator_bank(src: &str) -> IndicatorBank {
    let cfg = parse_using(src);
    let mut bank = IndicatorBank::new(&cfg);
    bank.assign_all_slots();
    bank
}

fn warmup_bank(bank: &mut IndicatorBank, trades: &[Trade]) {
    for t in trades.iter().take(100) {
        bank.on_trade(t);
    }
}

fn bench_indicator_only(c: &mut Criterion) {
    let trades = generate_trades(TICK_ITERS);
    let mut group = c.benchmark_group("indicator_only");
    group.throughput(Throughput::Elements(TICK_ITERS));
    group.sample_size(20);

    for name in strategies() {
        let src = std::fs::read_to_string(strategy_path(&name)).expect("read src");
        let mut bank = make_indicator_bank(&src);
        warmup_bank(&mut bank, &trades);

        group.bench_function(&name, |b| {
            b.iter(|| {
                for t in &trades {
                    black_box(bank.on_trade(t));
                }
            });
        });
    }
    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let trades = generate_trades(TICK_ITERS);
    let mut group = c.benchmark_group("full_pipeline");
    group.throughput(Throughput::Elements(TICK_ITERS));
    group.sample_size(10);

    for name in strategies() {
        let path = strategy_path(&name);
        let src = std::fs::read_to_string(&path).expect("read src");

        let cfg = parse_using(&src);
        if cfg.is_empty() {
            group.bench_function(&format!("{name} (no indicators)"), |b| {
                b.iter(|| {
                    let mut rt = QflRuntime::load(&path).expect("load");
                    for t in &trades {
                        rt.feed_trade(*t);
                    }
                    black_box(());
                });
            });
            continue;
        }

        group.bench_function(&name, |b| {
            b.iter(|| {
                let mut rt = QflRuntime::load(&path).expect("load");
                let mut bank = IndicatorBank::new(&cfg);
                for entry in &cfg {
                    let slot = rt.ensure_indicator_slot(&entry.name);
                    bank.set_name_to_slot(&entry.name, slot);
                    if entry.name == "macd" {
                        let s = rt.ensure_indicator_slot("macd.signal");
                        bank.set_name_to_slot("macd.signal", s);
                        let h = rt.ensure_indicator_slot("macd.histogram");
                        bank.set_name_to_slot("macd.histogram", h);
                    }
                    if entry.name == "bb" {
                        for sub in &["bb.middle", "bb.upper", "bb.lower", "bb.bandwidth"] {
                            let s = rt.ensure_indicator_slot(sub);
                            bank.set_name_to_slot(sub, s);
                        }
                    }
                    if entry.name == "kc" {
                        for sub in &["kc.middle", "kc.upper", "kc.lower"] {
                            let s = rt.ensure_indicator_slot(sub);
                            bank.set_name_to_slot(sub, s);
                        }
                    }
                }
                warmup_bank(&mut bank, &trades);
                for t in &trades {
                    for &(slot, v) in bank.on_trade(t) {
                        rt.set_indicator_by_slot(slot, v);
                    }
                    rt.feed_trade(*t);
                }
                black_box(());
            });
        });
    }
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_indicator_only, bench_full_pipeline
);
criterion_main!(benches);
