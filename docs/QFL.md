# QFL Language Reference

**Quince-flavored Language** -- a statically-typed, compiled domain-specific language for algorithmic trading strategies.

QFL is parsed, type-checked, optimized (constant folding, DCE, CSE, SCCP), and compiled to a compact register-based bytecode that runs in the Quince VM. The language is designed for deterministic, high-frequency execution with zero heap allocation in the hot path.

---

## Syntax Overview

QFL syntax is a minimal Lua-inspired dialect. A strategy consists of **entry functions** that are called by the engine on market events, **state variables** that persist across ticks, and helper functions.

```
-- This is a comment

// This is also a comment

/* Block comments work too */
```

---

## Entry Functions

The engine calls specific entry points when market data arrives:

| Entry | Called When | Registers |
|-------|-------------|-----------|
| `on_trade(t)` | A trade (aggTrade) arrives | r0=price, r1=qty, r2=side, r3=trade_id, r4=timestamp |
| `on_depth(d)` | Depth snapshot arrives | r0=bids[0].price, r1=bids[0].qty, r2=asks[0].price, r3=asks[0].qty |
| `on_fill(f)` | Order fill confirmed | r0=fill.price, r1=fill.qty, r2=side, r3=fee |
| `on_eval()` | Periodic evaluation (1 Hz) | All state + indicators available |

```
entry on_trade(t)
    if t.price > sma(close, 20) then
        quince.order(Side.Buy, 1.0)
    end
end
```

---

## State Variables

State variables persist across ticks and survive hot reloads. Declared with `@persist`:

```
@persist counter = 0
@persist running_sum = 0.0
@p resist last_signal = "none"
```

State can also be typed:

```
state counter: i64 = 0
state threshold: f64 = 0.02
state name: string = "default"
```

---

## Types

| Type | Description | Literal Examples |
|------|-------------|------------------|
| `i64` | Signed 64-bit integer | `42`, `-1`, `0xFF` |
| `f64` | IEEE 754 double | `3.14`, `1e-5`, `100.0` |
| `bool` | Boolean | `true`, `false` |
| `string` | UTF-8 string | `"hello"`, `'world'` |
| `symbol` | Market symbol | (internal, from `quince.get()`) |
| `price` | Price value (f64 subtype) | `t.price` |
| `qty` | Quantity value (f64 subtype) | `t.qty` |
| `duration` | Time value (i64 subtype) | From indicators |
| `timestamp` | Timestamp (i64 subtype) | `t.time` |
| `nil` | Unit type | `nil` |
| `Side` | Enumeration | `Side.Buy`, `Side.Sell` |
| `OrderType` | Enumeration | `OrderType.Market`, `OrderType.Limit` |

---

## Operators

### Arithmetic

| Op | Description | Example |
|----|-------------|---------|
| `+` | Add | `a + b` |
| `-` | Subtract / Negate | `a - b`, `-x` |
| `*` | Multiply | `a * b` |
| `/` | Float division | `a / b` |
| `//` | Integer division | `a // b` |
| `%` | Modulo | `a % b` |
| `^` | Power | `a ^ 2` |
| `..` | String concat | `"hello" .. " world"` |

### Comparison

| Op | Description |
|----|-------------|
| `==` | Equal |
| `~=` | Not equal |
| `<` | Less than |
| `>` | Greater than |
| `<=` | Less or equal |
| `>=` | Greater or equal |

### Logical

| Op | Description |
|----|-------------|
| `and` | Short-circuit AND |
| `or` | Short-circuit OR |
| `not` | Logical NOT |

---

## Control Flow

### If / Else

```
if condition then
    -- body
elseif other then
    -- else-if body
else
    -- else body
end
```

### While Loop

```
while condition do
    -- body
end
```

### Repeat Loop

```
repeat
    -- body
until condition
```

### Numeric For Loop

```
for i = 1, 10 do
    -- i goes from 1 to 10 inclusive (step=1)
end

for i = 10, 1, -1 do
    -- i goes from 10 down to 1
end

for i = 0, 1, 0.1 do
    -- i = 0, 0.1, 0.2, ..., 1.0
end
```

---

## Built-in Functions

### Order Placement

```
quince.order(side, qty)
quince.order(side, qty, price)
quince.order(side, qty, price, order_type)
quince.order(side, qty, price, order_type, reduce_only)
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `side` | `Side` | `Side.Buy` or `Side.Sell` |
| `qty` | `f64` | Order quantity |
| `price` | `f64?` | Limit price (nil = market) |
| `order_type` | `OrderType?` | `OrderType.Market` or `OrderType.Limit` |
| `reduce_only` | `bool?` | Reduce-only flag |

### Market Data

```
quince.get(symbol)        -- Get current price for symbol
quince.price()            -- Last trade price
quince.balance(asset)     -- Wallet balance by asset
quince.position(symbol)   -- Current position size
```

### Logging

```
log(value)
log("string value")
log("price: ", price)
```

Log output is captured to a ring buffer in debug builds and written to `qflvm.log` on graceful shutdown. No-op in release builds.

---

## Indicator Functions

Indicators are configured via `@using` directives and are available as functions in all entry points.

```
@using sma(close, 20)
@using ema(close, 10)
@using rsi(close, 14)
@using macd(close)
@using bb(close, 20, 2)
@using atr(14)
@using stoch(14, 3, 3)
```

Available indicator categories:

| Category | Indicators |
|----------|------------|
| Moving averages | SMA, EMA, WMA, VWMA, LSMA |
| Oscillators | RSI, MACD, Stochastic, CCI, ROC |
| Volatility | ATR, Bollinger Bands, Keltner Channels |
| Flow | OBV, CVD, Delta, MFI, AccDist, ADX, PMDI, NMDI |
| Structure | DOM imbalance, Z-score, Net OI |

Each indicator is called with its required parameters and returns the current value:

```
local avg = sma(close, 20)
local upper, middle, lower = bb(close, 20, 2)
local macd_line, signal, hist = macd(close)
```

---

## Feature / Signal System

QFL supports compile-time feature extraction and signal generation:

```
feature momentum(close, period) = close - close[period]

signal bullish = momentum > 0

entry on_trade(t)
    if bullish then
        quince.order(Side.Buy, 1.0)
    end
end
```

`feature` defines a named expression. `signal` defines a named boolean expression. Both are evaluated at compile time in the optimizer.

---

## Example Strategies

### Simple Moving Average Cross

```
@using sma(close, 10)
@using sma(close, 30)
@persist position = 0.0

entry on_trade(t)
    if sma(close, 10) > sma(close, 30) and position <= 0.0 then
        quince.order(Side.Buy, 1.0)
        position = 1.0
    elseif sma(close, 10) < sma(close, 30) and position >= 0.0 then
        quince.order(Side.Sell, 1.0)
        position = -1.0
    end
end
```

### RSI Reversion

```
@using rsi(close, 14)
@persist position = 0.0

entry on_trade(t)
    local r = rsi(close, 14)
    if r < 30 and position <= 0.0 then
        quince.order(Side.Buy, 1.0)
        position = 1.0
    elseif r > 70 and position >= 0.0 then
        quince.order(Side.Sell, 1.0)
        position = -1.0
    end
end
```

### Grid Trade

```
@using sma(close, 50)
@persist grid_buy = 0.0
@persist grid_sell = 0.0

entry on_trade(t)
    local base = sma(close, 50)
    if t.price < base * 0.99 and grid_buy == 0.0 then
        quince.order(Side.Buy, 0.1)
        grid_buy = t.price
    elseif t.price > base * 1.01 and grid_sell == 0.0 then
        quince.order(Side.Sell, 0.1)
        grid_sell = t.price
    end
    if t.price > grid_buy * 1.01 and grid_buy > 0.0 then
        quince.order(Side.Sell, 0.1, nil, OrderType.Market, true)
        grid_buy = 0.0
    end
    if t.price < grid_sell * 0.99 and grid_sell > 0.0 then
        quince.order(Side.Buy, 0.1, nil, OrderType.Market, true)
        grid_sell = 0.0
    end
end
```

---

## Compilation Pipeline

```
Source (.qfl)
    |
    v
Lexer (tokenizer)
    |
    v
Parser (AST)
    |
    v
Type Checker (inference + validation)
    |
    v
Optimizer (constant folding, DCE, CSE, SCCP, CFG simplification)
    |
    v
Compiler (register allocation, instruction selection)
    |
    v
Bytecode (.qfr)
    |
    v
VM (register-based interpreter with computed-goto dispatch)
```
