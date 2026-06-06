# Quince 🚧

[![Work In Progress](https://img.shields.io/badge/status-WIP-yellow?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Build](https://img.shields.io/badge/build-passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![Tests](https://img.shields.io/badge/tests-1032%20passing-brightgreen?style=for-the-badge)](https://github.com/0xitsss/quince)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue?style=for-the-badge)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.80+-orange?style=for-the-badge\&logo=rust)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.1-purple?style=for-the-badge)](https://github.com/0xitsss/quince/releases)
[![Performance](https://img.shields.io/badge/ULL-%3C0.5ms-red?style=for-the-badge)](https://github.com/0xitsss/quince)

**Q**uantitative **U**ltra-low-latency **I**nterpreter for **N**etwork-centric **C**ompetitive **E**xecution

Low-latency trading engine using crossbeam channels throughout. No `tokio::sync::mpsc` or `tokio::sync::watch` — only `tokio::sync::oneshot` for request-response pairs. Engine loop uses ULL priority polling with `try_recv`.

---

# Architecture

```mermaid
classDiagram
    class ExchangeTrait {
        <<interface>>
        +subscribe() Result~Stream~
        +place_order() Result~String~
        +cancel_order() Result~void~
        +account_info() Result~AccountInfo~
        +current_price() Result~f64~
    }

    class Binance {
        +api_key
        +secret_key
        +testnet
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

    ExchangeTrait <|-- Binance
    ExchangeTrait <|-- BinancePublic
    ExchangeTrait <|-- MockExchange

    class EngineLoop {
        +ULLPriorityLoop
        +try_recv_priority_polling()
        +PriorityEval
        +PriorityStream
        +PriorityOrders
    }

    class OrderManager {
        +exchange_to_client O(1)
        +SLTPTracking
        +register()
        +fill_tracking()
    }

    class IndicatorBank {
        +20Indicators
        +RingVecBackend
        +slot_based_writes()
        +zero_alloc_hot_path()
    }

    class RiskControls {
        +max_position_size
        +max_drawdown
        +max_order_freq
        +max_daily_loss
        +cooldown_after_loss
        +check_order()
    }

    class Profiler {
        +opcode_counts
        +handler_timing
        +record_opcode()
        +start_handler()
        +end_handler()
    }

    class Tracer {
        +signal_events
        +feature_events
        +fill_events
        +risk_events
        +ring_buffer_trace()
    }

    class QFLVM {
        +192IntRegisters
        +64FloatRegisters
        +64PersistSlots
        +256EMASlots
        +TypedIR
        +TypeChecker
        +Optimizer
    }

    class StrategyHandlers {
        +on_trade()
        +on_depth()
        +on_fill()
        +on_eval()
    }

    class VmSnapshot {
        +full_state_capture()
        +restore()
    }

    class StrategyGraph {
        +window_detection()
        +signal_detection()
    }

    class RollingWindows {
        +mean()
        +variance()
        +stddev()
        +min()
        +max()
        +sum()
    }

    class TradeLog {
        +JSONLines
        +log_fill()
        +log_trade()
    }

    class Console {
        +structured_logs()
    }

    class PuffinProfiler {
        +http_29012
    }

    EngineLoop --> OrderManager
    EngineLoop --> IndicatorBank
    EngineLoop --> RiskControls
    EngineLoop --> Profiler
    EngineLoop --> Tracer

    QFLVM --> StrategyHandlers

    EngineLoop --> VmSnapshot
    EngineLoop --> StrategyGraph
    IndicatorBank --> RollingWindows

    EngineLoop --> TradeLog
    EngineLoop --> Console
    EngineLoop --> PuffinProfiler
```

```mermaid
sequenceDiagram
    autonumber

    participant Exchange as Exchange Thread
    participant Crossbeam as crossbeam Channel
    participant Engine as Engine Loop
    participant VM as QFL VM
    participant Risk as Risk Engine
    participant OM as Order Manager
    participant Profiler
    participant Tracer

    Exchange->>Crossbeam: try_send(Trade)
    Crossbeam->>Engine: try_recv Trade P0

    Engine->>Profiler: record opcode counts
    Engine->>Tracer: trace signal feature
    Engine->>VM: call(on_trade)

    VM->>VM: execute bytecode
    VM->>Profiler: record opcode handler
    VM->>Tracer: trace signal feature

    VM->>Crossbeam: try_send(Order)
    Crossbeam->>Engine: try_recv Order P1

    Engine->>Risk: check_order()
    Engine->>Tracer: trace risk action

    alt Risk OK
        Risk-->>Engine: ok
        Engine->>OM: register(order)
        Engine->>Exchange: place_order(order)

        Exchange-->>Crossbeam: try_send(OrderUpdate)
        Crossbeam-->>Engine: try_recv(fill)

        Engine->>Tracer: trace fill
    else Risk Denied
        Risk-->>Engine: deny
        Engine->>Tracer: trace rejection
    end

    Exchange->>Crossbeam: try_send(Depth)
    Crossbeam->>Engine: try_recv Depth P0

    Engine->>VM: call(on_depth)
    VM->>VM: execute bytecode

    Note right of Engine: periodic on_eval every 1s timer P2

    Engine->>Profiler: start_handler(on_eval)
    Engine->>VM: call(on_eval)
    VM->>VM: execute bytecode
    Engine->>Profiler: end_handler()
```

---

# Crates

```mermaid
classDiagram
    class core {
        +RingBuffer(T,N)
        +RingVec
        +Trade
        +Depth
        +Order
        +Position
        +OrderFill
        +DepthLevel
        +Side
        +OrderType
    }

    class exchange {
        <<interface>>
        +subscribe()
        +place_order()
        +cancel_order()
        +account_info()
        +current_price()
    }

    class Binance {
        +api_key
        +secret_key
        +testnet
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
        +QFLVMThread
        +crossbeam_channels
        +on_trade()
        +on_depth()
        +on_fill()
        +on_eval()
    }

    class engine {
        +ULLPriorityLoop
        +OrderManager
        +IndicatorBank
        +RiskControls
        +Profiler
        +Tracer
    }

    class risk {
        +max_position_size
        +max_drawdown
        +max_order_freq
        +max_daily_loss
        +cooldown_after_loss
        +check_order()
    }

    class logger {
        +TradeLogJSON
        +log_fill()
        +log_trade()
    }

    class indicators {
        +SMA
        +EMA
        +WMA
        +VWMA
        +LSMA
        +RSI
        +MACD
        +CCI
        +ROC
        +Stochastic
        +BB
        +KC
        +ATR
        +MFI
        +ADX
        +CVD
        +PMDI
        +NMDI
        +OBV
        +ZScore
        +RingVecBackend
    }

    class qfl {
        +RegisterVM
        +192IntRegisters
        +64FloatRegisters
        +TypedIR
        +TypeChecker
        +Optimizer
        +ConstantFolding
        +CSE
        +DeadCodeElimination
        +RollingWindows
        +StrategyGraph
        +VmSnapshot
        +HotReload
        +RiskEngineIntegration
        +EmaFusedOpcode
        +DeclarativeSyntax
        +EventHandlers
        +TypedFunctions
    }

    exchange <|-- Binance
    exchange <|-- BinancePublic
    exchange <|-- MockExchange

    engine --> exchange
    engine --> strategy
    engine --> risk
    engine --> logger
    engine --> indicators
    engine --> qfl

    strategy --> core
    exchange --> core
    qfl --> core
    qfl --> indicators
```

---

# VM Internals

```mermaid
classDiagram
    class QFLRegisterVM {
        +r0_r191 i64
        +r192_r255 f64
        +ProgramCounter
        +CallStackDepth8
    }

    class EngineState {
        +Indicators256
        +Balances64
        +PositionSize
        +LastPrice
        +DepthBidsAsks32
        +RollingWindowsArena
        +PersistSlots64
        +EMAStates256
    }

    class DispatchTable {
        +JumpTable256
        +OpcodeBits0_7
        +OpcodeDispatch()
    }

    QFLRegisterVM --> DispatchTable
    EngineState --> DispatchTable
```

```mermaid
classDiagram
    class HotPath {
        +try_recv_stream()
        +indicator_update()
        +ringvec_O1()
        +vm_execute_bytecode()
        +crossbeam_try_send()
    }

    class ColdPath {
        +parse_qfl_source()
        +compile_to_bytecode()
        +optimize_fold()
        +optimize_cse()
        +optimize_dce()
        +assign_indicator_slots()
        +finalize_const_lookups()
    }

    ColdPath --> HotPath : init_once
```

---

# Quick Start

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

# Status

## Core Infrastructure

* Exchange trait + Binance WS/REST connector
* BinancePublic public WS mode
* Binance FAPI signed requests
* MockExchange simulated data
* Auto-fallback public WS mode

## Engine

* ULL priority polling loop
* try_recv stream priority
* crossbeam only
* Order manager O(1)
* SL TP tracking
* Indicator bank
* zero alloc hot path
* Risk controls
* Walkforward validation

## QFL VM

* Register VM
* 192 int registers
* 64 float registers
* 64 persist slots
* 256 EMA slots
* Typed IR
* Type checker
* Constant folding
* CSE
* DCE
* Profiler
* Tracer
* Rolling Window Engine
* StrategyGraph
* VmSnapshot
* RiskEngine integration
* Ema fused opcode
* Declarative syntax
* State declarations
* Event handlers
* Typed functions

## Indicators

* SMA
* EMA
* WMA
* VWMA
* LSMA
* RSI
* MACD
* CCI
* ROC
* Stochastic
* Bollinger Bands
* Keltner Channel
* ATR
* MFI
* CVD
* PMDI
* NMDI
* OBV
* Accumulation Distribution
* Volume Delta
* ADX
* ZScore
* DOM Imbalance
* Net OI

## Data Structures

* RingVec
* RingBuffer
* DepthLevel Copy

## Strategy

* Dedicated std thread VM
* Strategy API
* stop_loss support
* take_profit support
* on_trade
* on_depth
* on_fill
* on_eval

## Profiling

* puffin profiler
* opcode counts
* handler timing
* tracing
* slot based writes

## Testing

* 988 unit tests
* 44 integration tests
* 0 build warnings
* Mock mode validation

---

# Version History

| Version | Phase | Changes                                                                                 |
| ------- | ----- | --------------------------------------------------------------------------------------- |
| v0.6.1  | 6b    | Compiler safety hardening (register overflow, Index/Table, name length, emit_at bounds), VM debug_asserts, fix Jz/Jnz fencepost errors |
| v0.6.0  | 6a    | handler_param field access, persist coalesce, window O(1) deque, Vm hot/cold reorder |
| v0.5.3  | 5c    | Mov elimination (reuse analysis) — skip redundant Mov on variable read                 |
| v0.5.2  | 5b    | run_bare specialization, sanitize_f removal, JMP-after-Ret, TraceVM, engine HashMap removal |
| v0.5.1  | 5a    | Engine hot path optimizations — zero HashMap lookups in on_trade, VM handler cache |
| v0.5.0  | 4i    | Optimization pipeline v2 — local shadowing, LICM, loop unroll, fused lowering, GVN |
| v0.4.0  | 4g+4h | Feature pipeline, state declarations, event handlers, typed functions, Ema fused opcode |
| v0.3.6  | 4e    | Tracing                                                                                 |
| v0.3.5  | 4d    | Profiler                                                                                |
| v0.3.4  | 4c    | CSE                                                                                     |
| v0.3.3  | 4b    | Dead Code Elimination                                                                   |
| v0.3.2  | 4a    | Constant folding                                                                        |
| v0.3.1  | 3     | Risk Engine                                                                             |
| v0.3.0  | 2     | StrategyGraph, Snapshot Restore                                                         |
| v0.2.2  | 1.x   | Rolling Window Engine                                                                   |
| v0.2.0  | 1     | Typed IR                                                                                |
| v0.1.1  | 0     | Crossbeam migration                                                                     |

---

# License

GNU Affero General Public License v3.0
