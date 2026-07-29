# Module: `controls`

> Source: `controls.rs`

Risk control enforcement at runtime.
[`RiskControls`] validates orders and positions against configured limits
(max position size, max drawdown, order frequency, daily loss, cooldown).

## Structs

### `pub struct RiskControls`

```rust
pub struct RiskControls {
    pub max_position_size: f64,
    pub max_order_notional: f64,
    pub max_position_notional: f64,
    pub max_drawdown: f64,
    pub max_order_freq: u32,
    pub max_daily_loss: f64,
    pub cooldown_after_loss_secs: u64,
    order_count: u32,
    window_start: Instant,
    daily_loss: f64,
    peak_equity: f64,
    in_cooldown: bool,
    cooldown_end: Instant,
    last_market_data_at: Option < Instant >,
    max_market_data_age: Duration,
    paused: bool,
    pause_reason: Option < String >,
};
```


