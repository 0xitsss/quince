# Quince

**High-frequency algorithmic trading framework for cryptocurrency futures.**

Quince is a Rust-native trading engine purpose-built for latency-sensitive strategies that operate directly on exchange WebSocket streams. It combines a custom bytecode VM (QFL) with a non-blocking async event loop to achieve sub-100-microsecond tick-to-order latency.

---

## Why Quince?

Existing open-source trading frameworks share a common bottleneck: they are written in interpreted or garbage-collected languages (Python, JavaScript) and rely on general-purpose scripting runtimes (Lua, Python eval) for strategy execution. This imposes:

| Problem | Freqtrade | Hummingbot | Quince |
|---------|-----------|------------|--------|
| Language | Python | Python | Rust |
| Strategy runtime | Python eval | Python eval | Custom VM (QFL) |
| Tick latency | ~1-10 ms | ~5-50 ms | <10 us |
| GC pauses | Yes | Yes | No |
| Per-tick allocation | Yes | Yes | Zero |
| Backtest fidelity | Simulated | Simulated | Bit-identical |

Quince eliminates runtime indeterminism through:

- **No heap allocation in the hot path.** The QFL VM uses pre-allocated register files and a fixed-size instruction buffer. No garbage collector, no malloc during trading.
- **Compiled strategy code.** QFL source is parsed, type-checked, optimized (constant folding, dead code elimination, CSE, SCCP), and compiled to a compact bytecode. The optimizer produces code that runs at native-adjacent speed.
- **Direct exchange integration.** WebSocket streams feed directly into the strategy VM without serialization/deserialization overhead. Market data updates are mapped to register writes in <100 ns.
- **Deterministic execution.** Every tick produces identical results regardless of system load. The VM has no floating-point environment dependencies and uses strict IEEE 754 semantics.

---

## Architecture

```
Exchange WebSocket
       |
       v
  Engine loop (async, non-blocking)
       |
       +---> OrderManager (pending orders, SL/TP)
       +---> IndicatorBank (50+ technical indicators)
       +---> RiskControls (position limits, drawdown, daily loss)
       +---> QflRuntime
                |
                +---> QFL VM (bytecode interpreter)
                        |
                        +---> on_trade() / on_eval() / on_fill() / on_depth()
```

### Engine

The event loop (`engine/src/loop.rs`) is a single-threaded async loop that:

1. Pumps stream messages from the exchange via a bounded channel
2. Dispatches to indicators, risk, and strategy VM
3. Evaluates strategies on a fixed 1-second interval
4. Checks SL/TP levels and timeouts
5. Dumps VM logs to `qflvm.log` on Ctrl-C

### QFL VM

The strategy runtime (`qfl/src/vm.rs`) is a register-based bytecode interpreter with:

- 64 general-purpose registers (32 int + 32 float)
- 65 opcodes covering arithmetic, comparison, control flow, and exchange intrinsics
- Zero-cost entry dispatch via computed goto (`core::arch::asm!` with `br_table` equivalent)
- Debug-only ring-buffer logging written to `qflvm.log` on graceful shutdown

### Profiling

Two profiling layers exist behind `--features profiling`:

1. **Puffin scopes** -- 19 annotated regions across engine and VM call boundaries
2. **RDTSC opcode profiling** -- per-opcode cycle counters using x86_64 timestamp counter

---

## Performance

Release build benchmarks on x86_64 (10000 ticks each):

| Strategy | Instrs | Time | ns/tick | ops/ms |
|----------|--------|------|---------|--------|
| scalper.qfl | 79 | 819 us | 81 | 12,201 |
| ema_cross.qfl | 65 | 800 us | 80 | 12,489 |
| momentum.qfl | 118 | 1,781 us | 178 | 5,615 |
| heavy_test.qfl (100k) | 188 | 19,367 us | 193 | 5,164 |

All times are for a single-threaded, zero-allocation hot path. The VM dispatch loop runs at approximately 12 million opcodes per second on commodity hardware.

---

## Project Structure

| Crate | Lines (code) | Description |
|-------|-------------|-------------|
| `core/` | 595 | Shared types (Side, Order, Position, RingBuffer, RingVec) |
| `exchange/` | 929 | Binance Futures WebSocket + REST client |
| `engine/` | 2,212 | Event loop, OrderManager, IndicatorBank |
| `indicators/` | 2,026 | 50+ technical indicators (SMA, EMA, RSI, MACD, ATR, BB, etc.) |
| `logger/` | 223 | Trade fill logger to JSON Lines |
| `qfl/` | 15,783 | Parser, type checker, optimizer, compiler, VM, profiler, tracer |
| `risk/` | 246 | Position sizing, drawdown, daily loss, rate limiting |
| `quince/` | 519 | CLI binary, MockExchange |

Total: **22,533 lines** of Rust code across 42 source files.

---

## Testing

- **944 unit + integration tests**
- Full coverage of compiler, optimizer (DCE, CSE, SCCP, constant folding), parser, type checker, VM, and engine
- Strategies tested end-to-end: scalper, ema_cross, momentum, macd_cross, rsi_reversion, grid_trade, bb_bounce, atr_trail
- 100k-event heavy load test validating zero-allocation hot path
