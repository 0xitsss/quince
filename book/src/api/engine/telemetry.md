# Module: `telemetry`

> Source: `telemetry.rs`

Lock-free runtime counters exposed to an out-of-band operator surface.

## Structs

### `pub struct RuntimeTelemetrySnapshot`

```rust
pub struct RuntimeTelemetrySnapshot {
    pub strategy_version: u64,
    pub execution_mode: & 'static str,
    pub artifact_digest: String,
    pub market_events: u64,
    pub order_intents: u64,
    pub suppressed_orders: u64,
    pub market_event_latency_samples: u64,
    pub market_event_latency_p50_us: u64,
    pub market_event_latency_p95_us: u64,
    pub market_event_latency_p99_us: u64,
};
```


### `pub struct RuntimeTelemetry`

```rust
pub struct RuntimeTelemetry {
    strategy_version: AtomicU64,
    mode: AtomicU8,
    digest_prefix: AtomicU64,
    market_events: AtomicU64,
    order_intents: AtomicU64,
    suppressed_orders: AtomicU64,
    market_event_latency_ns: [AtomicU64 ; LATENCY_BUCKETS],
};
```

Atomic counters only: recording telemetry is safe in the market-data hot
path and never waits for an operator client.

