# Module: `dashboard`

> Source: `dashboard.rs`

Read-only local operator dashboard.

It deliberately has no order-control endpoints. A dedicated background
worker reads the durable journal and delivers snapshots through a bounded
crossbeam channel; the engine's latency-sensitive loop never waits on HTTP,
a mutex, or a dashboard client.

## Functions

### `pub fn start`

```rust
pub fn start(...) { ... }
```


