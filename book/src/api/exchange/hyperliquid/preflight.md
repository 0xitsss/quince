# Module: `hyperliquid/preflight`

> Source: `hyperliquid/preflight.rs`

Fail-closed market-context checks for authenticated execution.

This module is deliberately transport-free: a caller must bind an order to
a specific, fresh, finite market observation before it is signed.  A wall
clock timestamp alone is not evidence that a quote is usable for an order.

## Structs

### `pub struct MarketSnapshot`

```rust
pub struct MarketSnapshot {
    pub symbol: String,
    pub observed_at: DateTime < Utc >,
    pub reference_price: f64,
};
```

Immutable quote evidence captured at the decision boundary.

### `pub struct MarketContextPolicy`

```rust
pub struct MarketContextPolicy {
    pub max_age: Duration,
    pub max_limit_deviation_bps: u32,
};
```

Explicit bounds for accepting a market observation for execution.

