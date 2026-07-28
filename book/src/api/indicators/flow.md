# Module: `flow`

> Source: `flow.rs`

Money Flow Index (MFI) indicator.
A volume-weighted momentum oscillator that uses price and volume to identify
overbought/oversold conditions. [`Mfi`] tracks positive and negative money flow.

## Structs

### `pub struct Mfi`

```rust
pub struct Mfi {
    period: usize,
    typical_prev: Option < f64 >,
    pos_flow: RingVec,
    neg_flow: RingVec,
    count: usize,
};
```


### `pub struct VolumeDelta`

```rust
pub struct VolumeDelta;;
```


### `pub struct Cvd`

```rust
pub struct Cvd {
    cumulative: f64,
};
```


### `pub struct Obv`

```rust
pub struct Obv {
    obv: f64,
    prev_close: Option < f64 >,
};
```


### `pub struct AccDist`

```rust
pub struct AccDist {
    ad: f64,
};
```


### `pub struct Pmdi`

```rust
pub struct Pmdi {
    value: f64,
    prev_data: Option < f64 >,
};
```


### `pub struct Nmdi`

```rust
pub struct Nmdi {
    value: f64,
    prev_data: Option < f64 >,
};
```


### `pub struct AverageTradeSize`

```rust
pub struct AverageTradeSize;;
```


