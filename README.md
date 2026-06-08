# Quince

[![Build](https://img.shields.io/badge/build-passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Tests](https://img.shields.io/badge/tests-960%2B%20passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue?style=for-the-badge)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.80+-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.9-purple?style=for-the-badge)](https://github.com/0xitsss/quince/releases)
[![Benchmark](https://img.shields.io/badge/bench-Criterion-ff69b4?style=for-the-badge)](https://0xitsss.github.io/quince/dev/bench/)
[![SonarQube](https://img.shields.io/badge/sonar-passing-brightgreen?style=for-the-badge&logo=sonarcloud)](https://sonarcloud.io/project/overview?id=0xitsss_quince)

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

Criterion benchmarks (ubuntu-latest, x86_64) — 4 groups, 14 strategies:

| Group | Strategy | Throughput |
|-------|----------|-----------|
| **Pipeline** (parse + compile + optimize) | ema_cross | 42 µs |
| | scalper | 78 µs |
| | heavy_test | 412 µs |
| **VM tick** (10k iters) | ema_cross | 1,850 ops/ms |
| | scalper | 920 ops/ms |
| | momentum | 1,120 ops/ms |
| **VM scale** (heavy_test) | 1k iters | 1,780 ops/ms |
| | 10k iters | 1,690 ops/ms |
| | 100k iters | 1,550 ops/ms |
| **Runtime feed** (heavy_test) | 10k trades | 420 ops/ms |

VM dispatch: ~1.7 million ops/ms sustained. Zero heap allocation in hot path.

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

# Dump compiled QFL bytecode as assembly
cargo run --bin dump_qfl -- strategies/ema_cross.qfl

# Run all tests (960+)
cargo test

# Run benchmarks
cargo bench -p quince-qfl --bench bench

# Check clippy (zero warnings)
cargo clippy --all-targets -- -D warnings
```

---

## Documentation

- **[`docs/QUINCE.md`](docs/QUINCE.md)** — Architecture, performance benchmarks, crate breakdown
- **[`docs/QFL.md`](docs/QFL.md)** — Quince-Flavored Language syntax, types, indicators, example strategies
- **[`qfl/docs/spec/spec.pdf`](qfl/docs/spec/spec.pdf)** — Full QFL specification (46 pages, LaTeX)
- **[Criterion Benchmarks](https://0xitsss.github.io/quince/dev/bench/)** — Historical benchmark chart on gh-pages
- **[SonarQube](https://sonarcloud.io/project/overview?id=0xitsss_quince)** — Static analysis dashboard

---

## Architecture

### System Overview

```mermaid
graph TB
    subgraph CLI["quince (CLI)"]
        Main["main.rs<br/>env config"]
        Dump["dump_qfl.rs<br/>bytecode dump"]
    end

    subgraph Exchange["exchange/"]
        Trait["Exchange trait<br/>connect / subscribe / send"]
        Binance["Binance WS+REST<br/>real-time streams"]
        Mock["MockExchange<br/>simulated data"]
    end

    subgraph Engine["engine/"]
        Loop["Engine Loop<br/>priority polling<br/>crossbeam channels"]
        OM["OrderManager<br/>SL/TP tracking<br/>timeout checks"]
        IB["IndicatorBank<br/>20 indicators<br/>@using resolution"]
    end

    subgraph Risk["risk/"]
        RC["RiskControls<br/>position limits<br/>drawdown / rate<br/>daily loss / cooldown"]
    end

    subgraph QFL["qfl/"]
        RT["QflRuntime<br/>load / feed / eval"]
        VM["Register VM<br/>256 regs<br/>jump-table dispatch<br/>zero-alloc hot path"]
        Compiler["Compiler Pipeline<br/>lex → parse → typeck<br/>→ compile → optimize"]
        Opt["Optimiser<br/>11 passes<br/>CFG / CSE / SCCP / GVN"]
    end

    subgraph Core["core/"]
        Types["Types<br/>Trade / Depth / Order<br/>Side / Position"]
        Ring["RingVec / RingBuffer<br/>O(1) zero-alloc"]
    end

    subgraph Indicators["indicators/"]
        MA["Moving Averages<br/>SMA / EMA / WMA / VWMA"]
        Osc["Oscillators<br/>RSI / MACD / Stoch / CCI"]
        Vol["Volatility<br/>ATR / BB / Keltner"]
        Flow["Flow<br/>OBV / CVD / MFI / ADX"]
    end

    Main --> Loop
    Loop --> Trait
    Trait --> Binance
    Trait --> Mock
    Loop --> OM
    Loop --> IB
    Loop --> RC
    Loop --> RT
    RT --> VM
    RT --> Compiler
    Compiler --> Opt
    IB --> Indicators
    OM --> Types
    VM --> Types
    VM --> Ring
```

### Engine Loop Sequence

```mermaid
sequenceDiagram
    participant Exchange as Exchange WS
    participant Engine as Engine Loop
    participant IB as IndicatorBank
    participant Risk as RiskControls
    participant RT as QflRuntime
    participant VM as QFL VM
    participant OM as OrderManager

    loop Every 1ms tick
        Exchange->>Engine: Trade / Depth / Fill
        Engine->>IB: update(slot, value)
        Engine->>RT: feed_{trade,depth,fill}(data)

        RT->>VM: call("on_{trade,depth,fill}")
        VM->>VM: execute handler bytecode
        VM-->>RT: pending_order?
        RT-->>Engine: flush_pending_order()

        Engine->>Risk: check_order(order)
        Risk-->>Engine: Allow / Deny

        alt Risk Allow
            Engine->>OM: register(order)
            OM->>Exchange: send_order()
        else Risk Deny
            Engine->>Engine: log rejection
        end

        Engine->>Engine: process_order_responses()
        Engine->>Engine: check_sl_tp()
    end

    loop Every 1s
        Engine->>RT: feed_eval()
        RT->>VM: call("on_eval")
    end

    loop Every 10s
        Engine->>Exchange: account_info()
        Exchange-->>Engine: Balance / Position
    end
```

### QFL Compilation Pipeline

```mermaid
flowchart LR
    S[".qfl<br/>Source"] --> L["Lexer<br/>72 tokens"]
    L --> P["Parser<br/>Pratt / 10 Expr / 21 Stmt"]
    P --> TC["Type Checker<br/>10 domain types"]
    TC --> C["Compiler<br/>→ IR / QfrProgram"]
    C --> O["Optimiser<br/>11 passes"]
    O --> B[".qfr<br/>Bytecode"]
    B --> V["VM<br/>jump-table dispatch"]

    QFR["Pre-compiled .qfr"] --> LD["Loader<br/>mmap"]
    LD --> V

    style S fill:#4a9eff80
    style B fill:#4a9eff80
    style V fill:#ff6b6b80
```

### Optimiser Pass Pipeline

```mermaid
flowchart TB
    subgraph Pipeline["11 Optimisation Passes"]
        direction TB
        CF["1. Constant Fold<br/>folds int/float exprs"]
        CFG["2. CFG Simplify<br/>merge blocks, remove dead"]
        SCCP["3. SCCP<br/>lattice propagation"]
        CSE["4. CSE<br/>local value numbering"]
        LS["5. Local Shadow<br/>register reuse"]
        LICM["6. LICM<br/>hoist invariants,loop-invariant"]
        LU["7. Loop Unroll<br/>unroll small loops"]
        FL["8. Fused Lower<br/>EMA opcode fusion"]
        GVN["9. GVN<br/>global redundancy"]
        DCE["10. DCE<br/>remove unreachable"]
        PC["11. Persist Coalesce<br/>slot load/store opt"]
    end

    CF --> CFG --> SCCP --> CSE --> LS --> LICM --> LU --> FL --> GVN --> DCE --> PC
```

### QFL VM Hot/Cold Architecture

```mermaid
classDiagram
    class Vm {
        +regs: [Register; 256]
        +pc: usize
        +running: bool
        +call_stack: [usize; 64]
        +code_ptr: *const u64
        +consts_ptr: *const f64
        +last_price: f64
        +position_size: f64
        +handler_cache: [u32; 4]
        +cold: Box~ColdVm~
        +run_bare()
        +call(name)
    }

    class ColdVm {
        +indicators: [f64; 1024]
        +balances: [f64; 128]
        +depth_bids: [DepthLevel; 64]
        +depth_asks: [DepthLevel; 64]
        +persist: [PersistSlot; 64]
        +window_arena: [f64; 65536]
        +ema_states: [EmaState; 256]
        +profiler: Option~Profiler~
        +tracer: Option~Tracer~
    }

    class Register {
        <<union>>
        +i: i64
        +f: f64
    }

    class PersistSlot {
        +tag: u8
        +int_val: i64
        +float_val: f64
    }

    class EmaState {
        +alpha: f64
        +value: f64
        +initialized: bool
    }

    class WindowMeta {
        +offset: u16
        +capacity: u16
        +head: u16
        +len: u16
        +sum: f64
        +min_deque: [u8; 64]
        +max_deque: [u8; 64]
    }

    Vm --> ColdVm : Box pointer
    Vm --> Register : 256 × 8 B = 2 KB
    ColdVm --> PersistSlot : 64 slots
    ColdVm --> EmaState : 256 states
    ColdVm --> WindowMeta : 64 windows
```

### Core Domain Types

```mermaid
classDiagram
    class Trade {
        +price: f64
        +qty: f64
        +side: Side
        +trade_id: u64
        +time: i64
        +symbol: String
    }

    class Depth {
        +bids: Vec~DepthLevel~
        +asks: Vec~DepthLevel~
        +symbol: String
        +time: i64
    }

    class Order {
        +id: String
        +side: Side
        +qty: f64
        +price: f64
        +order_type: OrderType
        +reduce_only: bool
        +symbol: String
        +time: i64
    }

    class OrderFill {
        +order_id: String
        +price: f64
        +qty: f64
        +side: Side
    }

    class OrderType {
        <<enum>>
        Market
        Limit
        StopMarket
        StopLimit
        TakeProfitMarket
        TakeProfitLimit
    }

    class Side {
        <<enum>>
        Buy
        Sell
    }

    class PositionSide {
        <<enum>>
        Long
        Short
    }

    Trade --> Side
    Order --> Side
    Order --> OrderType
    OrderFill --> Side
```

### Engine Loop State Machine

```mermaid
stateDiagram-v2
    [*] --> Running

    state Running {
        [*] --> PumpStream
        PumpStream --> ProcessTrades : trade/depth/fill
        ProcessTrades --> ProcessOrders
        ProcessOrders --> CheckSLTP
        CheckSLTP --> EvalTick : 1s elapsed
        CheckSLTP --> AccountSync : 10s elapsed
        EvalTick --> PumpStream
        AccountSync --> PumpStream
    }

    Running --> ShuttingDown : Ctrl-C
    ShuttingDown --> [*] : dump logs & exit
```

### Event Handling Flow

```mermaid
flowchart TB
    subgraph Feed["Runtime feed_* methods"]
        T["feed_trade(trade)"] --> VT["VM.call(&quot;on_trade&quot;)"]
        D["feed_depth(depth)"] --> VD["VM.call(&quot;on_depth&quot;)"]
        F["feed_fill(fill)"] --> VF["VM.call(&quot;on_fill&quot;)"]
        E["feed_eval()"] --> VE["VM.call(&quot;on_eval&quot;)"]
    end

    subgraph VMExecute["VM Handler Execution"]
        VT --> D1["Dispatch via jump table"]
        VD --> D1
        VF --> D1
        VE --> D1
        D1 --> H["Execute bytecode<br/>zero-alloc hot path"]
        H --> O{"SendOrder<br/>emitted?"}
        O -->|Yes| Q["Queue order<br/>→ flush_pending_order()"]
        O -->|No| R["Return"]
    end

    subgraph Post["Post-Processing"]
        Q --> RC["RiskControls.check()"]
        RC -->|Allow| OM["OrderManager.register()"]
        RC -->|Deny| RJ["Log rejection"]
        OM --> S["Send to exchange"]
    end
```

### Trading Strategy Lifecycle (hot-reload)

```mermaid
sequenceDiagram
    participant User
    participant RT as QflRuntime
    participant Compiler as Compiler Pipeline
    participant VM as QFL VM

    User->>RT: load("strategy.qfl")
    RT->>Compiler: parse + typeck + compile + optimize
    Compiler-->>RT: QfrProgram
    RT->>VM: Vm::new(program)
    VM-->>RT: ready

    loop Trading
        RT->>VM: feed_trade(trade)
        VM-->>RT: pending order?
        RT-->>User: flush_pending_order()
    end

    User->>RT: hot_reload("strategy.qfl")
    RT->>Compiler: recompile
    Compiler-->>RT: new QfrProgram
    RT->>VM: replace VM (preserve persist!)
    Note over RT,VM: 64 persist slots survive reload
    VM-->>RT: ready (new logic, old state)
```

---

## Version History

| Version | Phase | Changes |
| ------- | ----- | ------- |
| v0.6.9 | 7d | Fix Windows .exe extension in release, restore Cargo.lock before benchmark gh-pages switch |
| v0.6.8 | 7d | Bump version, create gh-pages branch for benchmark charts |
| v0.6.7 | 7d | Overhaul release.yml (caching, version resolution, package), add caching to ci.yml |
| v0.6.6 | 7c | Fix hardcoded Windows paths in load tests (cross-platform CARGO_MANIFEST_DIR) |
| v0.6.5 | 7b | Clippy cleanup (167→0 warnings), Criterion benchmarks, CI/CD workflows, SonarQube |
| v0.6.4 | 7b | Remove state keyword, replace with @persist name : type = expr |
| v0.6.3 | 7b | Ctrl-C graceful shutdown fix, realized PnL tracking, MockExchange position fix, WS subscribe response validation, NaN guard for SL/TP, RiskControls daily loss unification, RingVec zero-capacity fix, OrderManager exchange mapping cleanup |
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
