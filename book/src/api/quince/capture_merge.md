# Module: `capture_merge`

> Source: `capture_merge.rs`

Deterministic, offline merger for independently captured replay streams.

This tool is deliberately narrow: it joins one converted trade capture and
one converted depth capture.  It opens no network connection and preserves
every input JSON object verbatim.  When the millisecond timestamps tie, a
trade is emitted before depth.  That conservative ordering prevents a
same-timestamp depth snapshot from influencing a preceding trade; ties are
counted in the report because their true exchange ordering is unknown.

## Structs

### `pub struct MergeSummary`

```rust
pub struct MergeSummary {
    pub trades: u64,
    pub depth_snapshots: u64,
    pub timestamp_ties: u64,
    pub output: String,
};
```



## Functions

### `pub fn merge`

```rust
pub fn merge(...) { ... }
```

Merge converted trade and depth JSONL captures in market-time order.
Input files must each be nondecreasing by `timestamp_ms`.  Same-millisecond
cross-stream events are allowed but reported; trades come first as the
conservative deterministic tie-breaker described in this module's docs.

