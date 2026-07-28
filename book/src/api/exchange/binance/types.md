# Module: `binance/types`

> Source: `binance/types.rs`

Binance WebSocket message parsing.
Fast JSON deserialization of Binance stream events (aggTrade, depth, kline)
into [`StreamMsg`] variants using simd-json.

## Functions

### `pub fn parse_ws_msg`

```rust
pub fn parse_ws_msg(...) { ... }
```


