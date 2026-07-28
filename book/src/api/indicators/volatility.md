# Module: `volatility`

> Source: `volatility.rs`

Volatility indicators.
Provides [`TrueRange`], [`Atr`] (Average True Range), [`BollingerBands`],
and [`KeltnerChannel`] for measuring and visualizing market volatility.

## Structs

### `pub struct TrueRange`

```rust
pub struct TrueRange;;
```


### `pub struct Atr`

```rust
pub struct Atr {
    period: usize,
    atr: Option < f64 >,
    prev_close: Option < f64 >,
    count: usize,
    initial_tr: RingVec,
};
```


### `pub struct BollingerBands`

```rust
pub struct BollingerBands {
    period: usize,
    multiplier: f64,
    sma: super :: ma :: Sma,
    buffer: RingVec,
};
```


### `pub struct BollingerOutput`

```rust
pub struct BollingerOutput {
    pub middle: f64,
    pub upper: f64,
    pub lower: f64,
    pub bandwidth: f64,
};
```


### `pub struct KeltnerChannel`

```rust
pub struct KeltnerChannel {
    multiplier: f64,
    ema: super :: ma :: Ema,
    atr: Atr,
};
```


### `pub struct KeltnerOutput`

```rust
pub struct KeltnerOutput {
    pub middle: f64,
    pub upper: f64,
    pub lower: f64,
};
```


