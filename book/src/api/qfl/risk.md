# Module: `risk`

> Source: `risk.rs`

QFL risk engine — runtime-enforced trading limits.

Intercepts orders before they reach the exchange connector. Rejects orders
that violate configured limits (max position, max notional, max orders/cycle).

Entry point: [`RiskEngine::check_order()`].

## Structs

### `pub struct RiskLimits`

```rust
pub struct RiskLimits {
    pub max_position: f64,
    pub max_order_notional: f64,
    pub max_orders_per_cycle: u32,
};
```

Runtime-enforced risk limits.

### `pub struct RiskEngine`

```rust
pub struct RiskEngine {
    pub limits: RiskLimits,
    pub current_position: f64,
    orders_this_cycle: u32,
};
```

Runtime risk engine.


## Enums

### `pub enum RiskVerdict`

```rust
pub enum RiskVerdict {
    Allowed,
    Rejected(String,),
}
```

Result of a risk check.

