# Quince 🚧

[![Work In Progress](https://img.shields.io/badge/status-WIP-yellow)](https://github.com/0xitsss/quince)
[![Build](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/0xitsss/quince)
[![Tests](https://img.shields.io/badge/tests-126%20passing-brightgreen)](https://github.com/0xitsss/quince)
[![License](https://img.shields.io/badge/license-GPLv3-blue)](https://www.gnu.org/licenses/gpl-3.0)

**Q**uantitative **U**ltra-low-latency **I**nterpreter for **N**etwork-centric **C**ompetitive **E**xecution

> **Work In Progress** — API unstable, sharp edges, may eat your portfolio.

HFT framework with LuaJIT strategy runtime. Reacts to every tick — no polling cycles.

---

## Architecture

```mermaid
graph TB
    subgraph Exchange["Exchange Layer"]
        T[Exchange Trait]
        B[Binance Connector]
        M[Mock Exchange]
        T --> B
        T --> M
    end

    subgraph Engine["Engine Core"]
        EL[Event Loop<br/>tokio::select!]
        OM[Order Manager]
        IB[Indicator Bank]
        RC[Risk Controls]
    end

    subgraph Strategy["Strategy Layer"]
        L[Lua VM<br/>mlua]
        S[Strategy Script<br/>on_trade / on_depth / on_fill / on_eval]
        L --> S
    end

    subgraph Output["Output"]
        TL[Trade Log<br/>JSON]
        CO[Console]
    end

    Exchange -->|WS Stream| Engine
    Engine -->|Lua Events| Strategy
    Strategy -->|Place Order| Engine
    Engine -->|Log| Output
    Engine <--> RC
```

```mermaid
sequenceDiagram
    participant E as Exchange
    participant EL as Engine Loop
    participant L as Lua VM
    participant R as Risk
    participant OM as Order Manager

    E->>EL: Trade(price, qty, side)
    EL->>EL: Update indicators
    EL->>L: LuaEvent::Trade
    L->>L: on_trade(trade)
    L->>EL: quince.order(...)

    EL->>R: check_order()
    R-->>EL: ✅ / ❌

    EL->>OM: register(order)
    EL->>E: place_order(order)
    E-->>EL: order_id

    E->>EL: Depth(bids, asks)
    EL->>L: LuaEvent::Depth
    L->>L: on_depth(depth)

    EL->>EL: on_eval() every 1s
    L->>L: on_eval()
```

## Crates

```mermaid
classDiagram
    class core {
        RingBuffer
        Trade
        Depth
        Order
    }
    class exchange {
        <<interface>> Exchange
        subscribe()
        place_order()
        cancel_order()
        account_info()
        current_price()
    }
    class strategy {
        Lua VM
        on_trade()
        on_depth()
        on_fill()
        on_eval()
    }
    class engine {
        Event Loop
        Order Manager
        Indicator Bank
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
    engine --> strategy : drives
    engine --> risk : checks
    engine --> logger : writes
    engine --> indicators : feeds
    strategy --> core : reads types
    exchange --> core : reads types
```

## Quick Start

```bash
# Mock mode (no API keys)
QUINCE_MOCK=1 cargo run

# With custom strategy & symbol
QUINCE_MOCK=1 QUINCE_STRATEGY=strategies/ema_cross.lua QUINCE_SYMBOL=btcusdt cargo run

# Live mode (Binance credentials required)
BINANCE_API_KEY=xxx BINANCE_SECRET_KEY=xxx cargo run
```

## Status

- ✅ Exchange trait + Binance connector (WS + REST)
- ✅ LuaJIT runtime with full API (place_order, balance, position, trades, depth, indicators)
- ✅ 20+ indicators (SMA, EMA, WMA, VWMA, LSMA, RSI, MACD, CCI, ROC, Stoch, BB, KC, ATR, MFI, ADX, CVD, PMDI, NMDI, Volume Delta, Z-score)
- ✅ Risk controls (position limit, drawdown, rate limit, daily loss, cooldown)
- ✅ Order manager (tracking, timeout, cancel)
- ✅ Mock mode for testing
- ✅ 126 tests passing

## License

GNU General Public License v3.0 — see [LICENSE](LICENSE) for details.
