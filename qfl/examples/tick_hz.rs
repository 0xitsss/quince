// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
use quince_qfl::{compiler, optimize, parser, vm};
use std::fs;
use std::time::Instant;

fn main() {
    let strategies = [
        "simple_test",
        "ema_cross",
        "sma_cross",
        "scalper",
        "heavy_test",
        "momentum",
        "macd_cross",
        "rsi_reversion",
        "bb_bounce",
        "grid_trade",
        "atr_trail",
        "test_all",
    ];
    for name in &strategies {
        let path = format!("D:\\kokosmain\\quince\\strategies\\{}.qfl", name);
        let src = fs::read_to_string(&path).expect("read");
        let prog = parser::parse(&src).expect("parse");
        let mut qfr = compiler::compile_checked(&prog).expect("compile");
        optimize::optimize(&mut qfr);
        let instrs = qfr.code.len();
        let mut vm = vm::Vm::new(qfr);
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
        let n = 1_000_000;
        let start = Instant::now();
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
        let dur = start.elapsed();
        let secs = dur.as_secs_f64();
        let hz = n as f64 / secs;
        let ns_per_tick = secs * 1_000_000_000.0 / n as f64;
        println!(
            "{name:18} instrs={instrs:3}  {hz:>10.0} Hz  {ns_per_tick:>8.1} ns/tick  ({dur:.3?})"
        );
    }
}
