# Module: `lib`

> Source: `lib.rs`

Technical analysis indicators for trading strategies.
Provides moving averages, oscillators, volatility measures, flow indicators,
and structure detection — all operating on the shared [`Candle`] type.

## Structs

### `pub struct Candle`

```rust
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
};
```


