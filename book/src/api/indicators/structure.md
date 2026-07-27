# Module: `structure`

> Source: `structure.rs`

Market structure indicators.
Provides [`Adx`] (Average Directional Index) for trend strength measurement
and [`Psar`] (Parabolic SAR) for trend direction and reversal points.

## Structs

### `pub struct Adx`

```rust
pub struct Adx {
    period: usize,
    tr_buffer: RingVec,
    plus_dm_buffer: RingVec,
    minus_dm_buffer: RingVec,
    prev_candle: Option < Candle >,
    count: usize,
    tr_smooth: Option < f64 >,
    plus_di: Option < f64 >,
    minus_di: Option < f64 >,
    adx_ema: Option < f64 >,
};
```


### `pub struct BidAskImbalance`

```rust
pub struct BidAskImbalance;;
```


### `pub struct DomDepth`

```rust
pub struct DomDepth;;
```


### `pub struct ZScore`

```rust
pub struct ZScore {
    period: usize,
    buffer: RingVec,
};
```


### `pub struct NetOpenInterest`

```rust
pub struct NetOpenInterest;;
```


### `pub struct NetOiOutput`

```rust
pub struct NetOiOutput {
    pub taker_long: f64,
    pub taker_short: f64,
    pub volume_delta: f64,
    pub oi_delta: f64,
};
```


