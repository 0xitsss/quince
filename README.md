# Quince 🚧

**Q**uantitative **U**ltra-low-latency **I**nterpreter for **N**etwork-centric **C**ompetitive **E**xecution

> **Work In Progress** — API unstable, sharp edges, may eat your portfolio.

HFT framework with LuaJIT strategy runtime. Reacts to every tick — no polling cycles.

## Architecture

```
                     ┌──────────────────┐
                     │   Exchange trait  │
                     │  (Binance, Mock)  │
                     └────────┬─────────┘
                              │ WS stream
                              ▼
┌──────────┐    ┌─────────────────────────┐    ┌──────────┐
│  Lua VM  │◄───│       Engine Loop       │───►│  Risk    │
│ (mlua)   │    │  tokio::select! { msg } │    │  Control │
│ on_trade │    │  on_trade → on_depth    │    └────┬─────┘
│ on_depth │    │  → on_fill → on_eval    │         │
│ on_fill  │    └─────────────────────────┘         │
└──────────┘                                        ▼
                                             ┌──────────┐
                                             │  Order   │
                                             │  Manager  │
                                             └──────────┘
```

## Crates

| Crate | Description |
|-------|-------------|
| `core` | RingBuffer + shared types |
| `exchange` | Exchange trait + Binance WS/REST connector |
| `strategy` | mlua runtime with Lua API bindings |
| `engine` | Event loop, order manager, indicator bank |
| `risk` | Position sizing, drawdown, rate limits |
| `logger` | JSON trade log |
| `indicators` | 20+ technical indicators (MA, RSI, MACD, BB, ATR, ADX, CVD, Z-score...) |
| `quince` | Umbrella crate + CLI + mock mode |

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

MIT
