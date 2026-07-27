// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Reproducible QFL strategy latency matrix.
//!
//! Measures the single-threaded hot path: indicator update, indicator-slot
//! writes and `on_trade` VM dispatch. Networking, disk I/O, logging sinks and
//! exchange acknowledgements are deliberately outside the measurement.

use quince_core::types::{Side, Trade};
use quince_engine::indicators::{parse_using, IndicatorBank};
use quince_qfl::{compiler, optimize, parser, vm};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const STRATEGIES: [&str; 3] = ["rare_signal", "scalper", "heavy_test"];
const HISTOGRAM_BUCKET_NS: u64 = 10;
const HISTOGRAM_BUCKETS: usize = 2_049;
const STRATEGIES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../strategies");

struct ResultRow {
    name: &'static str,
    ticks: u64,
    elapsed: Duration,
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    max_ns: u64,
}

fn usage() -> ! {
    eprintln!("usage: cargo run -p quince-engine --bin strategy_bench -- [--duration-secs 300] [--output-dir target/strategy-bench]");
    std::process::exit(2);
}

fn args() -> (Duration, PathBuf) {
    let mut duration = Duration::from_secs(300);
    let mut output = PathBuf::from("target/strategy-bench");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--duration-secs" => {
                let secs = args
                    .next()
                    .unwrap_or_else(|| usage())
                    .parse::<u64>()
                    .unwrap_or_else(|_| usage());
                if secs < STRATEGIES.len() as u64 {
                    usage();
                }
                duration = Duration::from_secs(secs);
            }
            "--output-dir" => output = PathBuf::from(args.next().unwrap_or_else(|| usage())),
            "--help" | "-h" => usage(),
            _ => usage(),
        }
    }
    (duration, output)
}

fn load_vm(path: &Path) -> vm::Vm {
    let source = fs::read_to_string(path).expect("read strategy");
    let program = parser::parse(&source).expect("parse strategy");
    let mut compiled = compiler::compile_checked(&program).expect("compile strategy");
    optimize::optimize(&mut compiled);
    vm::Vm::new(compiled)
}

fn make_bank_and_slots(vm: &mut vm::Vm, source: &str) -> IndicatorBank {
    let config = parse_using(source);
    let mut bank = IndicatorBank::new(&config);
    for entry in &config {
        bank.set_name_to_slot(&entry.name, vm.ensure_indicator_slot(&entry.name));
        match entry.name.as_str() {
            "macd" => {
                bank.set_name_to_slot("macd.signal", vm.ensure_indicator_slot("macd.signal"));
                bank.set_name_to_slot("macd.histogram", vm.ensure_indicator_slot("macd.histogram"));
            }
            "bb" => {
                for name in ["bb.middle", "bb.upper", "bb.lower", "bb.bandwidth"] {
                    bank.set_name_to_slot(name, vm.ensure_indicator_slot(name));
                }
            }
            "kc" => {
                for name in ["kc.middle", "kc.upper", "kc.lower"] {
                    bank.set_name_to_slot(name, vm.ensure_indicator_slot(name));
                }
            }
            _ => {}
        }
    }
    bank
}

fn histogram_percentile(histogram: &[u64; HISTOGRAM_BUCKETS], ticks: u64, percentile: u64) -> u64 {
    let target = ticks.saturating_mul(percentile).div_ceil(100).max(1);
    let mut seen = 0;
    for (bucket, count) in histogram.iter().enumerate() {
        seen += count;
        if seen >= target {
            return (bucket as u64).saturating_mul(HISTOGRAM_BUCKET_NS);
        }
    }
    (HISTOGRAM_BUCKETS as u64 - 1) * HISTOGRAM_BUCKET_NS
}

fn bench_strategy(name: &'static str, duration: Duration) -> ResultRow {
    let path = Path::new(STRATEGIES_DIR).join(format!("{name}.qfl"));
    let source = fs::read_to_string(&path).expect("read strategy");
    let mut vm = load_vm(&path);
    let mut bank = make_bank_and_slots(&mut vm, &source);
    let time = chrono::Utc::now();
    let mut trade = Trade {
        price: 50_000.0,
        qty: 0.1,
        time,
        side: Side::Buy,
        trade_id: 0,
    };
    // Warm code and indicator windows. This period is intentionally excluded.
    for tick in 0..10_000 {
        feed_tick(&mut vm, &mut bank, &mut trade, tick);
    }

    let mut histogram = [0_u64; HISTOGRAM_BUCKETS];
    let start = Instant::now();
    let mut tick = 0_u64;
    let mut max_ns = 0_u64;
    while start.elapsed() < duration {
        let tick_start = Instant::now();
        feed_tick(&mut vm, &mut bank, &mut trade, tick);
        let ns: u64 = tick_start
            .elapsed()
            .as_nanos()
            .try_into()
            .unwrap_or(u64::MAX);
        let bucket = (ns / HISTOGRAM_BUCKET_NS).min(HISTOGRAM_BUCKETS as u64 - 1) as usize;
        histogram[bucket] += 1;
        max_ns = max_ns.max(ns);
        tick += 1;
    }
    let elapsed = start.elapsed();
    // Register zero is initialized as a price before every dispatch.
    black_box(unsafe { vm.regs[0].f });
    ResultRow {
        name,
        ticks: tick,
        elapsed,
        p50_ns: histogram_percentile(&histogram, tick, 50),
        p95_ns: histogram_percentile(&histogram, tick, 95),
        p99_ns: histogram_percentile(&histogram, tick, 99),
        max_ns,
    }
}

fn feed_tick(vm: &mut vm::Vm, bank: &mut IndicatorBank, trade: &mut Trade, tick: u64) {
    let phase = tick as f64 * 0.01;
    trade.price = 50_000.0 + 1_000.0 * phase.sin();
    trade.qty = 0.1 + (tick % 20) as f64 * 0.1;
    trade.side = if tick.is_multiple_of(2) {
        Side::Buy
    } else {
        Side::Sell
    };
    trade.trade_id = tick;
    for &(slot, value) in bank.on_trade(trade) {
        vm.set_indicator_by_slot(slot, value);
    }
    vm.set_last_price(trade.price);
    vm.set_position_size(0.0);
    vm.regs[0].f = trade.price;
    vm.regs[1].f = trade.qty;
    vm.regs[2].i = if matches!(trade.side, Side::Buy) {
        0
    } else {
        1
    };
    vm.regs[3].i = tick as i64;
    vm.regs[4].i = 0;
    vm.call("on_trade");
}

fn write_reports(output: &Path, total: Duration, rows: &[ResultRow]) {
    fs::create_dir_all(output).expect("create benchmark output directory");
    let mut json = String::new();
    writeln!(json, "{{\n  \"schema_version\": 1,\n  \"total_requested_seconds\": {},\n  \"host_os\": \"{}\",\n  \"arch\": \"{}\",\n  \"scope\": \"indicator update + slot writes + QFL on_trade VM dispatch; excludes network, exchange, disk and log sinks\",\n  \"results\": [", total.as_secs(), std::env::consts::OS, std::env::consts::ARCH).unwrap();
    for (index, row) in rows.iter().enumerate() {
        writeln!(json, "    {{\"strategy\":\"{}\",\"ticks\":{},\"elapsed_seconds\":{:.6},\"throughput_ticks_per_second\":{:.2},\"p50_ns_upper_bound\":{},\"p95_ns_upper_bound\":{},\"p99_ns_upper_bound\":{},\"max_ns\":{}}}{}", row.name, row.ticks, row.elapsed.as_secs_f64(), row.ticks as f64 / row.elapsed.as_secs_f64(), row.p50_ns, row.p95_ns, row.p99_ns, row.max_ns, if index + 1 == rows.len() { "" } else { "," }).unwrap();
    }
    json.push_str("  ]\n}\n");
    fs::write(output.join("results.json"), json).expect("write JSON report");
    fs::write(output.join("latency.svg"), latency_svg(rows)).expect("write latency graph");
    fs::write(output.join("throughput.svg"), throughput_svg(rows)).expect("write throughput graph");
}

fn latency_svg(rows: &[ResultRow]) -> String {
    let max = rows.iter().map(|r| r.p99_ns.max(1)).max().unwrap_or(1) as f64;
    let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"900\" height=\"360\"><style>text{font:14px system-ui;fill:#d8fff6}.label{fill:#9ec9c0}.p50{fill:#56d9c9}.p95{fill:#5b9dff}.p99{fill:#ff8b5c}</style><rect width=\"100%\" height=\"100%\" fill=\"#101717\"/><text x=\"30\" y=\"32\">QFL tick latency (histogram upper bounds, ns)</text>");
    for (i, row) in rows.iter().enumerate() {
        let x = 80 + i * 260;
        for (offset, value, class, label) in [
            (0, row.p50_ns, "p50", "p50"),
            (55, row.p95_ns, "p95", "p95"),
            (110, row.p99_ns, "p99", "p99"),
        ] {
            let height = (value as f64 / max * 220.0).max(1.0);
            let y = 290.0 - height;
            write!(svg, "<rect class=\"{class}\" x=\"{}\" y=\"{y:.1}\" width=\"42\" height=\"{height:.1}\"/><text class=\"label\" x=\"{}\" y=\"310\">{label}</text><text x=\"{}\" y=\"{:.1}\">{value}</text>", x + offset, x + offset, x + offset, y - 7.0).unwrap();
        }
        write!(svg, "<text x=\"{}\" y=\"340\">{}</text>", x, row.name).unwrap();
    }
    svg.push_str("</svg>");
    svg
}

fn throughput_svg(rows: &[ResultRow]) -> String {
    let max = rows
        .iter()
        .map(|r| r.ticks as f64 / r.elapsed.as_secs_f64())
        .fold(1.0, f64::max);
    let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"900\" height=\"360\"><style>text{font:14px system-ui;fill:#d8fff6}.label{fill:#9ec9c0}</style><rect width=\"100%\" height=\"100%\" fill=\"#101717\"/><text x=\"30\" y=\"32\">QFL hot-path throughput (ticks/s)</text>");
    for (i, row) in rows.iter().enumerate() {
        let x = 100 + i * 260;
        let value = row.ticks as f64 / row.elapsed.as_secs_f64();
        let height = (value / max * 220.0).max(1.0);
        let y = 290.0 - height;
        write!(svg, "<rect fill=\"#56d9c9\" x=\"{x}\" y=\"{y:.1}\" width=\"120\" height=\"{height:.1}\"/><text x=\"{x}\" y=\"{:.1}\">{value:.0}</text><text class=\"label\" x=\"{x}\" y=\"340\">{}</text>", y - 7.0, row.name).unwrap();
    }
    svg.push_str("</svg>");
    svg
}

fn main() {
    let (total, output) = args();
    let per_strategy = total / STRATEGIES.len() as u32;
    eprintln!(
        "Quince strategy matrix: {} total, {} per strategy",
        humantime(total),
        humantime(per_strategy)
    );
    let rows: Vec<_> = STRATEGIES
        .into_iter()
        .map(|name| {
            eprintln!("running {name}…");
            bench_strategy(name, per_strategy)
        })
        .collect();
    write_reports(&output, total, &rows);
    for row in &rows {
        println!(
            "{}: {:.0} ticks/s | p50 ≤ {}ns | p95 ≤ {}ns | p99 ≤ {}ns | max {}ns",
            row.name,
            row.ticks as f64 / row.elapsed.as_secs_f64(),
            row.p50_ns,
            row.p95_ns,
            row.p99_ns,
            row.max_ns
        );
    }
    println!("artifacts: {}", output.display());
}

fn humantime(duration: Duration) -> String {
    format!("{}s", duration.as_secs())
}
