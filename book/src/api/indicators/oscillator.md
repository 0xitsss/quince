# Module: `oscillator`

> Source: `oscillator.rs`

Oscillator indicators for momentum and mean-reversion analysis.
Includes [`Rsi`] (Relative Strength Index), [`Stochastic`], [`Cci`]
(Commodity Channel Index), and [`WilliamsR`] (%R).

## Structs

### `pub struct Rsi`

```rust
pub struct Rsi {
    period: usize,
    gains: RingVec,
    losses: RingVec,
    avg_gain: Option < f64 >,
    avg_loss: Option < f64 >,
    prev: Option < f64 >,
    count: usize,
};
```


### `pub struct Macd`

```rust
pub struct Macd {
    fast_ema: super :: ma :: Ema,
    slow_ema: super :: ma :: Ema,
    signal_ema: super :: ma :: Ema,
};
```


### `pub struct MacdOutput`

```rust
pub struct MacdOutput {
    pub macd_line: f64,
    pub signal_line: f64,
    pub histogram: f64,
};
```


### `pub struct Cci`

```rust
pub struct Cci {
    period: usize,
    typical_buffer: RingVec,
    constant: f64,
};
```


### `pub struct Roc`

```rust
pub struct Roc {
    period: usize,
    buffer: RingVec,
};
```


### `pub struct Stochastic`

```rust
pub struct Stochastic {
    period: usize,
    high_buffer: RingVec,
    low_buffer: RingVec,
};
```


