# Quince

[![Build](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/0xitsss/quince)
[![Tests](https://img.shields.io/badge/tests-191%20passing-brightgreen)](https://github.com/0xitsss/quince)
[![License](https://img.shields.io/badge/license-GPLv3-blue)](https://www.gnu.org/licenses/gpl-3.0)

**Q**uantitative **U**ltra-low-latency **I**nterpreter for **N**etwork-centric **C**ompetitive **E**xecution

Low-latency trading engine using crossbeam channels throughout. No `tokio::sync::mpsc` or `tokio::sync::watch` — only `tokio::sync::oneshot` for request-response pairs. Engine loop uses ULL priority polling with `try_recv`.

---

## Architecture

```mermaid
graph TB
    subgraph Exchange["Exchange Layer"]
        T[Exchange Trait]
        B[Binance Connector<br/>crossbeam::Sender]
        M[Mock Exchange<br/>std::thread + crossbeam]
        P[BinancePublic<br/>Public WS Mode]
        T --> B
        T --> M
        T --> P
    end

    subgraph Engine["Engine Core"]
        EL[ULL Priority Loop<br/>try_recv: stream > orders > eval > acct]
        OM[Order Manager<br/>HashMap + exchange_to_client O(1)]
        IB[Indicator Bank<br/>RingVec-based 20+ indicators]
        RC[Risk Controls]
    end

    subgraph Strategy["Strategy Layer"]
        L[Lua VM<br/>mlua in std::thread]
        S[Strategy Script<br/>on_trade / on_depth / on_fill / on_eval]
        L --> S
    end

    subgraph Channel["Crossbeam Channels"]
        MD[Market Data<br/>bounded 1024]
        LE[Lua Events<br/>bounded 1024]
        OC[Orders<br/>crossbeam::Sender]
    end

    subgraph Output["Output"]
        TL[Trade Log<br/>JSON]
        CO[Console]
    end

    Exchange -->|try_send| MD
    MD --> EL
    EL -->|try_send| LE
    LE -->|recv| Strategy
    Strategy --> OC
    OC -->|try_recv| EL
    EL -->|Log| Output
    EL <--> RC
```

```mermaid
sequenceDiagram
    participant E as Exchange<br/>std::thread
    participant C as crossbeam<br/>Channel
    participant EL as Engine Loop<br/>ULL Priority
    participant L as Lua VM<br/>std::thread
    participant R as Risk
    participant OM as Order Manager

    E->>C: try_send(Trade)
    C->>EL: try_recv → Trade
    EL->>EL: Update indicators<br/>(RingVec)
    EL->>C: try_send(LuaEvent::Trade)
    C->>L: recv → on_trade()
    L->>L: on_trade(trade)
    L->>C: try_send(Order)
    C->>EL: try_recv → Order

    EL->>R: check_order()
    R-->>EL: ok / deny
    EL->>OM: register(order)
    EL->>E: place_order(order)
    E-->>C: try_send(OrderUpdate)
    C-->>EL: try_recv → fill

    E->>C: try_send(Depth)
    C->>EL: try_recv → Depth
    EL->>C: try_send(LuaEvent::Depth)
    C->>L: recv → on_depth()

    EL->>EL: periodic on_eval()<br/>(every 1s timer)
    EL->>C: try_send(LuaEvent::Eval)
    C->>L: recv → on_eval()
```

## Crates

```mermaid
classDiagram
    class core {
        RingBuffer<T,N>
        RingVec (power-of-2 mask)
        Trade, Depth, Order
    }
    class exchange {
        <<interface>> Exchange
        subscribe() → crossbeam::Receiver
        place_order()
        cancel_order()
        account_info()
        current_price()
    }
    class strategy {
        Lua VM in std::thread
        crossbeam channels
        on_trade() on_depth() on_fill() on_eval()
    }
    class engine {
        ULL Priority Loop
        Order Manager (HashMap O(1) lookup)
        Indicator Bank (20+ indicators)
    }
    class risk {
        max_position_size
        max_drawdown
        max_order_freq
        max_daily_loss
    }
    class logger {
        TradeLog JSON
    }
    class indicators {
        SMA, EMA, WMA, VWMA, LSMA
        RSI, MACD, CCI, ROC, Stoch
        BB, KC, ATR, MFI, ADX
        CVD, PMDI, NMDI, Z-score
    }

    engine --> exchange : uses
    engine --> strategy : crossbeam
    engine --> risk : checks
    engine --> logger : writes
    engine --> indicators : feeds
    strategy --> core : reads types
    exchange --> core : reads types
```

## Quick Start

```bash
# Mock mode (simulated data, no API keys)
QUINCE_MOCK=1 cargo run

# Public WS mode (real Binance data, no API keys)
QUINCE_PUBLIC=1 cargo run

# With custom strategy & symbol
QUINCE_MOCK=1 QUINCE_STRATEGY=strategies/scalper.lua QUINCE_SYMBOL=btcusdt cargo run

# Testnet mode (Binance testnet credentials)
BINANCE_API_KEY=xxx BINANCE_SECRET_KEY=xxx QUINCE_TESTNET=1 cargo run

# Live mode (real Binance credentials)
BINANCE_API_KEY=xxx BINANCE_SECRET_KEY=xxx cargo run

# With profiling (http://127.0.0.1:29012)
cargo run --features profiling

# Run all tests
cargo test
```

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
- ✅ Lua 5.4 runtime via mlua, runs in dedicated `std::thread`
- ✅ Strategy API: `quince.order()`, `quince.balance()`, `quince.position()`, `quince.trades()`, `quince.depth()`, `quince.get()`
- ✅ Stop-loss / take-profit via `quince.order({stop_loss=99, take_profit=101})`
- ✅ Events: `on_trade`, `on_depth`, `on_fill`, `on_eval`

### Profiling
- ✅ `puffin` profiler behind `profiling` feature flag
- ✅ Hot path optimized: indicators use `HashMap<&'static str, f64>` — zero alloc per tick

### Testing
- ✅ 191 tests across all crates
- ✅ 0 build warnings
- ✅ Mock mode tests with real position/balance tracking

## License

GNU General Public License v3.0 — see [LICENSE](LICENSE) for details.
