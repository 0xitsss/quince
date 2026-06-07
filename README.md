# Quince

[![Build](https://img.shields.io/badge/build-passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Tests](https://img.shields.io/badge/tests-944%20passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue?style=for-the-badge)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.80+-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.3-purple?style=for-the-badge)](https://github.com/0xitsss/quince/releases)
[![Performance](https://img.shields.io/badge/latency-%3C10%C2%B5s-red?style=for-the-badge)](https://github.com/0xitsss/quince)

**Q**uantitative **U**ltra-low-latency **I**nterpreter for **N**etwork-centric **C**ompetitive **E**xecution

Low-latency trading engine using crossbeam channels throughout. Engine loop uses priority polling with `try_recv`. Custom bytecode VM (QFL) delivers sub-10-microsecond tick-to-order latency with zero heap allocation in the hot path.

---

## Project Structure

| Crate | Lines (code) | Description |
|-------|-------------|-------------|
| `core/` | 595 | Shared types, RingBuffer, RingVec |
| `exchange/` | 929 | Binance Futures WS + REST client |
| `engine/` | 2,212 | Event loop, OrderManager, IndicatorBank, RiskControls |
| `indicators/` | 2,026 | 50+ technical indicators |
| `logger/` | 223 | Trade fill logger (JSON Lines) |
| `qfl/` | 15,783 | Parser, type checker, optimizer, compiler, VM, tracer |
| `risk/` | 246 | Position limits, drawdown, rate limiting |
| `quince/` | 519 | CLI binary, MockExchange |
| **Total** | **22,533** | **42 Rust source files** |

---

## Performance

Release build benchmarks (x86_64, 10,000 ticks per strategy):

| Strategy | Instrs | Time | ns/tick | ops/ms |
|----------|--------|------|---------|--------|
| scalper.qfl | 79 | 819 us | 81 | 12,201 |
| ema_cross.qfl | 65 | 800 us | 80 | 12,489 |
| momentum.qfl | 118 | 1,781 us | 178 | 5,615 |
| heavy_test.qfl (100k) | 188 | 19,367 us | 193 | 5,164 |

VM dispatch: ~12 million opcodes/second. Zero heap allocation, zero GC pauses.

---

## Quick Start

```bash
# Mock mode (simulated data, no API keys)
QUINCE_MOCK=1 cargo run

# Public WS mode (real Binance data, no API keys)
QUINCE_PUBLIC=1 cargo run

# With custom QFL strategy
QUINCE_MOCK=1 QUINCE_STRATEGY=strategies/scalper.qfl QUINCE_SYMBOL=btcusdt cargo run

# Testnet mode (Binance testnet credentials)
BINANCE_API_KEY=xxx BINANCE_SECRET_KEY=xxx QUINCE_TESTNET=1 cargo run

# Live mode (real Binance credentials)
BINANCE_API_KEY=xxx BINANCE_SECRET_KEY=xxx cargo run

# With profiling (http://127.0.0.1:29012)
cargo run --features profiling

# Run all tests
cargo test
```

---

## Documentation

- **[`docs/QUINCE.md`](docs/QUINCE.md)** — Architecture, performance benchmarks, crate breakdown
- **[`docs/QFL.md`](docs/QFL.md)** — Quince-Flavored Language syntax, types, indicators, example strategies

---

## Architecture

### Engine Loop

The engine (`engine/src/loop.rs`) is a single-threaded async loop that:

1. Pumps stream messages from the exchange via a bounded crossbeam channel
2. Dispatches trades, depth, fills to indicators, risk, and strategy VM
3. Evaluates strategies on a fixed 1-second interval
4. Checks SL/TP levels and order timeouts
5. Dumps VM logs to `qflvm.log` on Ctrl-C

### QFL VM

The strategy runtime (`qfl/src/vm.rs`) is a register-based bytecode interpreter:

- 64 general-purpose registers (32 int + 32 float)
- 65 opcodes: arithmetic, comparison, control flow, exchange intrinsics
- Zero-cost opcode dispatch via computed goto
- Debug-only ring-buffer logging written to `qflvm.log` on graceful shutdown
- RDTSC opcode profiling behind `--features profiling`

### Compilation Pipeline

```
Source (.qfl) → Lexer → Parser → Type Checker → Optimizer → Compiler → Bytecode (.qfr) → VM
```

Optimizations: constant folding, dead code elimination (DCE), common subexpression elimination (CSE), sparse conditional constant propagation (SCCP), CFG simplification.

### Indicators

50+ technical indicators with zero-allocation ring buffer backend:

| Category | Indicators |
|----------|------------|
| Moving Averages | SMA, EMA, WMA, VWMA, LSMA |
| Oscillators | RSI, MACD, Stochastic, CCI, ROC |
| Volatility | ATR, Bollinger Bands, Keltner Channels |
| Flow | OBV, CVD, Delta, MFI, AccDist, ADX, PMDI, NMDI |
| Structure | DOM Imbalance, Z-Score, Net OI |

---

## Version History

| Version | Phase | Changes |
| ------- | ----- | ------- |
| v0.6.3 | 7b | Ctrl-C graceful shutdown fix, realized PnL tracking, MockExchange position fix, WS subscribe response validation, NaN guard for SL/TP, RiskControls daily loss unification, RingVec zero-capacity fix, OrderManager exchange mapping cleanup |
| v0.6.2 | 7 | Perf audit fixes: VM bounds, monotonic deque bitops, HashMap indicators/balances, StreamMsg profiling |
| v0.6.1 | 6b | Compiler safety hardening, VM debug_asserts, fix Jz/Jnz fencepost |
| v0.6.0 | 6a | Handler field access, persist coalesce, window O(1) deque |
| v0.5.3 | 5c | Mov elimination (reuse analysis) |
| v0.5.2 | 5b | run_bare specialization, engine HashMap removal |
| v0.5.1 | 5a | Engine hot path optimizations |
| v0.5.0 | 4i | Optimization pipeline v2 |
| v0.4.0 | 4g+4h | Feature pipeline, state declarations, event handlers |
| v0.3.6 | 4e | Tracer |
| v0.3.5 | 4d | Profiler |
| v0.3.4 | 4c | CSE |
| v0.3.3 | 4b | Dead Code Elimination |
| v0.3.2 | 4a | Constant folding |
| v0.3.1 | 3 | Risk Engine |
| v0.3.0 | 2 | StrategyGraph, Snapshot Restore |
| v0.2.2 | 1.x | Rolling Window Engine |
| v0.2.0 | 1 | Typed IR |
| v0.1.1 | 0 | Crossbeam migration |

---

## License

GNU Affero General Public License v3.0
