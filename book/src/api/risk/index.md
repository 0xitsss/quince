# Module: `lib`

> Source: `lib.rs`

Risk management configuration and controls.
Defines [`RiskConfig`] for parameterizing position sizing, drawdown limits,
order frequency, daily loss caps, and cooldown periods.

## Structs

### `pub struct RiskConfig`

```rust
pub struct RiskConfig {
    pub max_position_size: f64,
    pub max_order_notional: f64,
    pub max_position_notional: f64,
    pub max_drawdown: f64,
    pub max_order_freq: u32,
    pub max_daily_loss: f64,
    pub cooldown_after_loss_secs: u64,
};
```


