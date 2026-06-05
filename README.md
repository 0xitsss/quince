# Quince 🚧

[![Work In Progress](https://img.shields.io/badge/status-WIP-yellow?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Build](https://img.shields.io/badge/build-passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Tests](https://img.shields.io/badge/tests-800%20passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue?style=for-the-badge)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.80+-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.4.4-purple?style=for-the-badge)](https://github.com/0xitsss/quince/releases)
[![Performance](https://img.shields.io/badge/ULL-<100µs-red?style=for-the-badge)](https://github.com/0xitsss/quince)

**Q**uantitative **U**ltra-low-latency **I**nterpreter for **N**etwork-centric **C**ompetitive **E**xecution

Low-latency trading engine using crossbeam channels throughout. No `tokio::sync::mpsc` or `tokio::sync::watch` — only `tokio::sync::oneshot` for request-response pairs. Engine loop uses ULL priority polling with `try_recv`.

---

## Architecture

```mermaid
graph TB
    subgraph Exchange["🔌 Exchange Layer"]
        T[Exchange Trait]
        B[Binance WS/REST]
        M[Mock Exchange]
        P[BinancePublic WS]
        T --> B
        T --> M
        T --> P
    end

    subgraph Engine["⚙️ Engine Core"]
        EL[ULL Priority Loop<br/>try_recv stream<br/>Priority: Eval > Stream > Orders]
        OM[Order Manager<br/>O(1) exchange_to_client<br/>SL/TP tracking]
        IB[Indicator Bank<br/>20+ RingVec indicators<br/>Slot-based writes]
        RC[Risk Controls<br/>pos/drawdown/freq/loss]
        PR[Profiler<br/>opcode counts + timing]
        TR[Tracer<br/>signal/feature/fill/risk]
        EL --> OM
        EL --> IB
        EL --> RC
        EL --> PR
        EL --> TR
    end

    subgraph Strategy["📈 Strategy Layer"]
        Q[QFL Register VM<br/>192 int + 64 float regs<br/>64 persist + 256 EMA slots]
        S[Handlers: on_trade/on_depth/on_fill/on_eval]
        Q --> S
    end

    subgraph Channels["📡 Crossbeam Channels"]
        MD[Market Data<br/>Trade/Depth/MarkPrice]
        QE[QFL Events<br/>Trade/Fill/Depth/Eval]
        OC[Orders<br/>Buy/Sell/Market/Limit]
    end

    subgraph Data["💾 State & Storage"]
        VL[VM Snapshot<br/>Full state capture]
        SG[Strategy Graph<br/>Window/Signal detection]
        RW[Rolling Windows<br/>Mean/StdDev/Min/Max/Sum]
    end

    subgraph Output["📤 Output"]
        TL[Trade Log<br/>JSON Lines]
        CO[Console<br/>Structured logs]
        PF[Puffin Profiler<br/>http://127.0.0.1:29012]
    end

    Exchange -- "try_send" --> MD
    MD -- "try_recv (P0)" --> EL
    EL -- "try_send" --> QE
    QE -- "recv" --> Strategy
    Strategy -- "try_send" --> OC
    OC -- "try_recv (P1)" --> EL
    EL -- "Log" --> Output
    EL <--> RC
    EL --> PR
    EL --> TR
    EL --> VL
    EL --> SG
    IB --> RW
```

```mermaid
sequenceDiagram
    autonumber
    participant E as Exchange<br/>std::thread
    participant C as crossbeam<br/>Channel
    participant EL as Engine Loop<br/>ULL Priority
    participant Q as QFL VM<br/>Register VM
    participant R as Risk
    participant OM as Order Manager
    participant PR as Profiler
    participant TR as Tracer

    E->>C: try_send(Trade)
    C->>EL: try_recv → Trade (P0)
    EL->>IB: Update indicators (RingVec)
    EL->>PR: record opcode counts
    EL->>TR: trace signal/feature
    EL->>Q: call("on_trade")
    Q->>Q: execute bytecode
    Q->>PR: record opcode/handler
    Q->>TR: trace signal/feature
    Q->>C: try_send(Order)
    C->>EL: try_recv → Order (P1)

    EL->>R: check_order()
    EL->>TR: trace risk action
    alt Risk OK
        R-->>EL: ok
        EL->>OM: register(order)
        EL->>E: place_order(order)
        E-->>C: try_send(OrderUpdate)
        C-->>EL: try_recv → fill
        EL->>TR: trace fill
    else Risk Denied
        R-->>EL: deny
        EL->>TR: trace rejection
    end

    E->>C: try_send(Depth)
    C->>EL: try_recv → Depth (P0)
    EL->>Q: call("on_depth")
    Q->>Q: execute bytecode

    Note right of EL: Periodic on_eval()<br/>every 1s timer (P2)
    EL->>EL: periodic on_eval()
    EL->>PR: start_handler("on_eval")
    EL->>Q: call("on_eval")
    Q->>Q: execute bytecode
    EL->>PR: end_handler()
```

---

## Crates

```mermaid
classDiagram
    class core {
        +RingBuffer<T,N>
        +RingVec
        +Trade, Depth, Order, Position
        +OrderFill, DepthLevel
        +Side, OrderType
    }
    class exchange {
        <<interface>> Exchange
        +subscribe() Result<Stream>
        +place_order() Result<String>
        +cancel_order() Result<()>
        +account_info() Result<AccountInfo>
        +current_price() Result<f64>
    }
    class Binance {
        +api_key, secret_key
        +testnet: bool
        +signed_request()
        +ws_stream()
    }
    class BinancePublic {
        +ws_stream()
        +no_auth()
    }
    class MockExchange {
        +simulated_data()
        +position_tracking()
        +balance_management()
    }
    class strategy {
        +QFL VM in std::thread
        +crossbeam channels
        +on_trade/on_depth/on_fill/on_eval
    }
    class engine {
        +ULL Priority Loop
        +Order Manager
        +Indicator Bank
        +Risk Controls
        +Profiler + Tracer
    }
    class risk {
        +max_position_size
        +max_drawdown
        +max_order_freq
        +max_daily_loss
        +cooldown_after_loss
        +check_order() Result<()>
    }
    class logger {
        +TradeLog JSON Lines
        +log_fill()
        +log_trade()
    }
    class indicators {
        +SMA, EMA, WMA, VWMA, LSMA
        +RSI, MACD, CCI, ROC, Stoch
        +BB, KC, ATR, MFI, ADX
        +CVD, PMDI, NMDI, OBV, Z-score
        +RingVec backend
    }
    class qfl {
        +Register VM: 192 int + 64 float
        +Typed IR + Type Checker
        +Optimizer: fold + CSE + DCE
        +Profiler: counts + timing
        +Tracer: ring buffer events
        +Rolling Windows + StrategyGraph
        +VmSnapshot + Hot Reload
        +RiskEngine integration
        +Ema fused opcode (64)
        +Phase 4g: @using/@window/feature/signal
        +Phase 4h: state/fn/on event
    }

    exchange <|-- Binance
    exchange <|-- BinancePublic
    exchange <|-- MockExchange
    engine --> exchange : uses
    engine --> strategy : crossbeam
    engine --> risk : checks
    engine --> logger : writes
    engine --> indicators : feeds
    engine --> qfl : embeds
    strategy --> core : reads types
    exchange --> core : reads types
    qfl --> core : reads types
    qfl --> indicators : uses slots
```

---

## VM Internals

```mermaid
graph LR
    subgraph VM["QFL Register VM"]
        IR[Int Registers<br/>r0-r191: i64]
        FR[Float Registers<br/>r192-r255: f64]
        PC[Program Counter]
        CS[Call Stack<br/>depth=8]
    end

    subgraph State["Engine State"]
        IND[Indicators<br/>f64[256]]
        BAL[Balances<br/>f64[64]]
        POS[Position Size]
        PRC[Last Price]
        DPB[Depth Bids/Asks<br/>[f64; 32]]
        RW[Rolling Windows<br/>Arena + Meta]
        PS[Persist Slots<br/>64 tagged]
        EM[EMA States<br/>256 slots]
    end

    subgraph Dispatch["Jump Table Dispatch"]
        JT[JUMP_TABLE: [fn; 256]]
        OP[Opcode in bits 0-7]
        JT --> OP
    end

    IR --> JT
    FR --> JT
    IND --> JT
    BAL --> JT
    RW --> JT
    PS --> JT
    EM --> JT
```

```mermaid
graph TB
    subgraph HotPath["🔥 Hot Path (per tick)"]
        TRY[try_recv Stream]
        IND_UPD[Indicator Update<br/>RingVec O(1)]
        VM_RUN[VM Execute<br/>bytecode]
        ORD[Order Send<br/>crossbeam try_send]
    end

    subgraph ColdPath["❄️ Cold Path (setup)"]
        PARSE[Parse .qfl source]
        COMPILE[Compile to bytecode]
        OPTIMIZE[Optimize: fold/CSE/DCE]
        SLOTS[Assign indicator slots]
        LOOKUP[finalize_const_lookups]
    end

    TRY --> IND_UPD
    IND_UPD --> VM_RUN
    VM_RUN --> ORD
    PARSE --> COMPILE
    COMPILE --> OPTIMIZE
    OPTIMIZE --> SLOTS
    SLOTS --> LOOKUP
    LOOKUP -.->|once at init| VM_RUN
```

---

## Quick Start

```bash
# Mock mode (simulated data, no API keys)
QUINCE_MOCK=1 cargo run

# Public WS mode (real Binance data, no API keys)
QUINCE_PUBLIC=1 cargo run

# With custom QFL strategy & symbol
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

## Status

### Core Infrastructure
- ✅ Exchange trait + Binance WS/REST connector (crossbeam channels)
- ✅ BinancePublic — public WS mode (no API keys needed)
- ✅ Binance FAPI — signed requests (API key + HMAC-SHA256)
- ✅ MockExchange — simulated data + position tracking + balance management
- ✅ Auto-fallback to public WS mode when no API keys set

### Engine
- ✅ ULL priority polling loop: `try_recv` stream > orders > eval > account sync
- ✅ All crossbeam channels (no `tokio::sync::mpsc` or `watch`)
- ✅ Order manager: HashMap O(1) exchange-to-client lookup, SL/TP tracking
- ✅ Indicator bank: 20+ indicators updated per-tick, zero String alloc in hot path
- ✅ Risk controls: position limit, drawdown, rate limit, daily loss, cooldown
- ✅ Purged CV-style walkforward validation support

### QFL VM (quince-qfl crate)
- ✅ Register VM: 192 int + 64 float registers, 64 persist slots, 256 EmaState slots
- ✅ Typed IR + type checker (10 domain types: i64, f64, bool, timestamp, duration, price, qty, symbol, side, order_id)
- ✅ Optimization pipeline: constant folding → CSE → dead code elimination (71 tests)
- ✅ Profiler: opcode execution counts `[u64; 65]` + per-handler timing (12 tests)
- ✅ Tracer: signal/feature/fill/risk event ring buffer (29 tests)
- ✅ Rolling Window Engine — `RollingWindow` wrapping `RingVec` with online mean/variance/stddev/min/max/sum; 6 VM opcodes (22 tests)
- ✅ StrategyGraph — window/signal detection from bytecode (7 tests)
- ✅ VmSnapshot — full state capture + `restore()` for replay/hot-reload (5 tests)
- ✅ RiskEngine integration — risk-gated `flush_pending_order()` (6 tests)
- ✅ Ema fused opcode (opcode=64) — single-instruction EMA update with pre-allocated state (2 tests)
- ✅ Phase 4g: Declarative `@using name:param` / `@window capacity` / `feature` / `signal` syntax (13 tests)
- ✅ Phase 4h: `state name : type = default`, `on event(param?) { body }`, `fn name(params) -> type { body }` (11 tests)

### Indicators (VecDeque → RingVec)
- ✅ Trend: SMA, EMA, WMA, VWMA, LSMA
- ✅ Oscillators: RSI, MACD, CCI, ROC, Stochastic
- ✅ Volatility: Bollinger Bands, Keltner Channel, ATR
- ✅ Flow: MFI, CVD, PMDI, NMDI, OBV, Accumulation/Distribution, Volume Delta
- ✅ Structure: ADX, Z-Score, DOM Depth/Imbalance, Net OI
- ✅ All use `RingVec` — no `VecDeque`, no manual pop_front

### Data Structures
- ✅ `RingVec` — heap-allocated ring buffer, O(1) wrapping with branchless conditional subtract
- ✅ `RingBuffer<T,N>` — compile-time ring buffer with full test coverage
- ✅ `DepthLevel: Copy` — no unnecessary cloning

### Strategy
- ✅ QFL Register VM (192 int + 64 float regs), runs in dedicated `std::thread`
- ✅ Strategy API: `quince.order()`, `quince.balance()`, `quince.position()`, `quince.trades()`, `quince.depth()`, `quince.get()`
- ✅ Stop-loss / take-profit via `quince.order({stop_loss=99, take_profit=101})`
- ✅ Events: `on_trade`, `on_depth`, `on_fill`, `on_eval`

### Profiling & Observability
- ✅ `puffin` profiler behind `profiling` feature flag (http://127.0.0.1:29012)
- ✅ QFL Profiler: per-opcode counts `[u64; 65]` + per-handler timing (12 tests)
- ✅ QFL Tracer: signal/feature/fill/risk event ring buffer (29 tests)
- ✅ Hot path optimized: slot-based indicator writes (`set_indicator_by_slot`), no HashMap in tick (Phase 4g)

### Testing
- ✅ 695 tests passing in quince-qfl (16 pre-existing failures in lexer/parser/runtime unrelated to our changes)
- ✅ 28 integration tests in quince-engine (1 pre-existing failure: intg_fill_handler)
- ✅ 0 build warnings
- ✅ Mock mode tests with real position/balance tracking

---

## Version History

| Version | Phase | Changes |
|---------|-------|---------|
| v0.4.0 | 4g+4h | Feature pipeline (`@using name:param`, `@window`, `feature`, `signal`), State declarations (`state name : type`), Event handlers (`on event() { }`), Typed functions (`fn name() -> type { }`), Ema fused opcode, slot-based indicator writes |
| v0.3.6 | 4e | Tracing — signal/feature/fill/risk event ring buffer |
| v0.3.5 | 4d | Profiler — opcode counts + per-handler timing |
| v0.3.4 | 4c | CSE — Common Subexpression Elimination |
| v0.3.3 | 4b | Dead Code Elimination with jump offset adjustment |
| v0.3.2 | 4a | Constant folding optimization pass |
| v0.3.1 | 3 | Risk Engine, Event dispatch, risk-gated orders |
| v0.3.0 | 2 | Feature/Signal Graph, Snapshot/Restore, Replay |
| v0.2.2 | 1.x | Rolling Window Engine + VM opcodes |
| v0.2.0 | 1 | Typed IR, type checker, compile_checked |
| v0.1.1 | 0 | Crossbeam migration, RingVec, MockExchange |

---

## License

GNU Affero General Public License v3.0 — see [LICENSE](LICENSE) for details.