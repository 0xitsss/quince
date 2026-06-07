use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use quince_qfl::*;

// ─── Constants ───────────────────────────────────────────────────────────────

const STRATEGIES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../strategies/");
const TICK_ITERS: u64 = 10_000;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn strategies() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(STRATEGIES_DIR)
        .expect("strategies dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "qfl"))
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

fn load_vm(name: &str) -> vm::Vm {
    let src = std::fs::read_to_string(strategy_path(name)).expect("read");
    let prog = parser::parse(&src).expect("parse");
    let mut qfr = compiler::compile_checked(&prog).expect("compile");
    optimize::optimize(&mut qfr);
    vm::Vm::new(qfr)
}

fn warmup_vm(vm: &mut vm::Vm) {
    for i in 0..100 {
        vm.set_last_price(100.0);
        vm.set_position_size(0.0);
        vm.regs[0].f = 100.0;
        vm.regs[1].f = 1.0;
        vm.regs[2].i = 0;
        vm.regs[3].i = i as i64;
        vm.regs[4].i = 0;
        vm.call("on_trade");
    }
}

fn run_n_ticks(vm: &mut vm::Vm, n: u64) {
    for i in 0..n {
        vm.set_last_price(100.0);
        vm.set_position_size(0.0);
        vm.regs[0].f = 100.0;
        vm.regs[1].f = 1.0;
        vm.regs[2].i = 0;
        vm.regs[3].i = i as i64;
        vm.regs[4].i = 0;
        vm.call("on_trade");
    }
    black_box(unsafe { vm.regs[0].f });
}

// ─── Group 1: Pipeline (parse + compile + optimize) ──────────────────────────

fn bench_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");
    group.sample_size(20);
    for name in strategies() {
        let src = std::fs::read_to_string(strategy_path(&name)).expect("read");
        let (_, instrs) = load_and_count(&name);
        group.bench_with_input(
            BenchmarkId::new(&name, instrs),
            &src,
            |b, src| {
                b.iter(|| {
                    let prog = parser::parse(black_box(src)).expect("parse");
                    let mut qfr = compiler::compile_checked(&prog).expect("compile");
                    optimize::optimize(&mut qfr);
                    black_box(qfr.code.len());
                });
            },
        );
    }
    group.finish();
}

fn load_and_count(name: &str) -> (Vec<u64>, usize) {
    let src = std::fs::read_to_string(strategy_path(name)).expect("read");
    let prog = parser::parse(&src).expect("parse");
    let mut qfr = compiler::compile_checked(&prog).expect("compile");
    optimize::optimize(&mut qfr);
    (qfr.code.iter().map(|i| i.raw()).collect(), qfr.code.len())
}

// ─── Group 2: VM tick execution ──────────────────────────────────────────────

fn bench_vm_tick(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_tick");
    group.throughput(Throughput::Elements(TICK_ITERS));
    group.sample_size(20);

    for name in strategies() {
        let mut vm = load_vm(&name);
        warmup_vm(&mut vm);
        let (_, instrs) = load_and_count(&name);
        let id = BenchmarkId::new(&name, instrs);

        group.bench_function(id, |b| {
            b.iter(|| run_n_ticks(&mut vm, TICK_ITERS));
        });
    }
    group.finish();
}

// ─── Group 3: VM scale sweep (heavy_test) ────────────────────────────────────

fn bench_vm_scale(c: &mut Criterion) {
    let mut vm = load_vm("heavy_test");
    warmup_vm(&mut vm);
    let (_, instrs) = load_and_count("heavy_test");

    let mut group = c.benchmark_group("vm_scale");
    group.sample_size(20);

    for &n in &[1_000u64, 10_000, 100_000] {
        group.throughput(Throughput::Elements(n));
        let id = BenchmarkId::new("heavy_test", format!("{}_instrs_{}iters", instrs, n));

        group.bench_function(id, |b| {
            b.iter(|| run_n_ticks(&mut vm, n));
        });
    }
    group.finish();
}

// ─── Group 4: Feed benchmark (runtime-level, with setup amortized) ───────────

fn bench_runtime(c: &mut Criterion) {
    use quince_core::types::{Side, Trade};
    use std::time::Instant;

    let path = strategy_path("heavy_test");
    let trades: Vec<Trade> = (0..TICK_ITERS)
        .map(|i| Trade {
            price: 50000.0 + (i % 1000) as f64,
            qty: 0.1 + (i % 5) as f64 * 0.1,
            time: chrono::Utc::now(),
            side: if i % 2 == 0 { Side::Buy } else { Side::Sell },
            trade_id: i as u64,
        })
        .collect();

    let mut group = c.benchmark_group("runtime_feed");
    group.throughput(Throughput::Elements(TICK_ITERS));
    group.sample_size(10); // fewer samples since each loads a runtime

    group.bench_function("heavy_test_10k", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;
            for _ in 0..iters {
                let mut rt =
                    runtime::QflRuntime::load(&path).expect("load");
                let start = Instant::now();
                for t in &trades {
                    rt.feed_trade(t.clone());
                }
                total += start.elapsed();
            }
            total
        });
    });
    group.finish();
}

// ─── Macro glue ──────────────────────────────────────────────────────────────

criterion_group!(
    name = benches;
    config = Criterion::default().warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_pipeline, bench_vm_tick, bench_vm_scale, bench_runtime
);
criterion_main!(benches);
