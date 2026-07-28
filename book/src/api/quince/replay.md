# Module: `replay`

> Source: `replay.rs`

Deterministic, offline QFL market-data replay.

The input is newline-delimited JSON. Every line has `schema_version: 1`
and one of these event shapes:
`{"schema_version":1,"type":"trade","timestamp_ms":...,"price":...,
"qty":...,"side":"buy|sell","trade_id":...}`;
`{"schema_version":1,"type":"depth","timestamp_ms":...,"bids":[{"price":...,"qty":...}],
"asks":[...]}`; or `{"schema_version":1,"type":"eval","timestamp_ms":...}`.

Replay never opens a socket and never sends an exchange order.  QFL order
intents are captured in-memory and reported as deterministic counters.

## Structs

### `pub struct ReplayCostModel`

```rust
pub struct ReplayCostModel {
    pub fee_bps: f64,
    pub slippage_bps: f64,
};
```

Taker-style cost assumptions for offline paper execution.
The defaults are intentionally conservative: 10 bps fee and 5 bps of
adverse slippage per fill.  They are not an exchange fee schedule.  Set
`QUINCE_REPLAY_FEE_BPS` and `QUINCE_REPLAY_SLIPPAGE_BPS` to model a
specific venue/account tier.

### `pub struct ReplaySummary`

```rust
pub struct ReplaySummary {
    pub schema_version: u8,
    pub events: u64,
    pub trades: u64,
    pub depth_snapshots: u64,
    pub eval_ticks: u64,
    pub order_intents: u64,
    pub buy_intents: u64,
    pub sell_intents: u64,
    pub strategy_logs: u64,
    pub signal_logs: u64,
    pub log_samples: Vec < String >,
    pub cost_model: ReplayCostModel,
    pub paper_fills: u64,
    pub unfilled_intents: u64,
    pub filled_notional_quote: f64,
    pub fees_quote: f64,
    pub slippage_cost_quote: f64,
    pub realized_gross_pnl_quote: f64,
    pub unrealized_gross_pnl_quote: f64,
    pub gross_pnl_quote: f64,
    pub net_pnl_quote: f64,
    pub ending_position_qty: f64,
    pub ending_mark_price: Option < f64 >,
};
```



## Enums

### `pub enum ReplayError`

```rust
pub enum ReplayError {
    Open(path: String,
    source: std :: io :: Error,),
    Read(line: usize,
    source: std :: io :: Error,),
    Invalid(line: usize,
    reason: String,),
    Strategy(String,),
    CostModel(String,),
}
```



## Functions

### `pub fn run`

```rust
pub fn run(...) { ... }
```

Replay a versioned JSONL market-data capture through a QFL strategy.
Event order is the file order, deliberately: no wall-clock scheduling,
random identifiers, exchange requests, or parallel dispatch are involved.

### `pub fn run_with_cost_model`

```rust
pub fn run_with_cost_model(...) { ... }
```

As [`run`], with explicit cost assumptions for deterministic tests and
programmatic callers. It is still strictly offline paper execution.

