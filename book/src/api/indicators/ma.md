# Module: `ma`

> Source: `ma.rs`

Moving average indicators.
Provides [`Sma`] (Simple), [`Ema`] (Exponential), [`Wma`] (Weighted),
and [`Hma`] (Hull) moving averages with O(1) incremental updates.

## Structs

### `pub struct Sma`

```rust
pub struct Sma {
    period: usize,
    buffer: RingVec,
    sum: f64,
};
```


### `pub struct Ema`

```rust
pub struct Ema {
    multiplier: f64,
    current: Option < f64 >,
};
```


### `pub struct Wma`

```rust
pub struct Wma {
    period: usize,
    buffer: RingVec,
    denominator: f64,
};
```


### `pub struct Vwma`

```rust
pub struct Vwma {
    period: usize,
    price_buffer: RingVec,
    vol_buffer: RingVec,
    pv_sum: f64,
    v_sum: f64,
};
```


### `pub struct Lsma`

```rust
pub struct Lsma {
    period: usize,
    buffer: RingVec,
    sum_x: f64,
    sum_x2: f64,
};
```


