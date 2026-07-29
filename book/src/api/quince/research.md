# Module: `research`

> Source: `research.rs`

Reproducible offline research reports built on deterministic replay.

## Structs

### `pub struct ResearchReport`

```rust
pub struct ResearchReport {
    pub schema_version: u8,
    pub capture: String,
    pub symbol: String,
    pub strategies_discovered: u64,
    pub strategies_succeeded: u64,
    pub strategies_failed: u64,
    pub results: Vec < ReplaySuiteResult >,
};
```

Stable, machine-readable outcome of replaying a strategy set on one capture.


## Enums

### `pub enum ResearchError`

```rust
pub enum ResearchError {
    ReplaySuite(replay_suite :: ReplaySuiteError,),
    CreateDirectory(path: String,
    source: std :: io :: Error,),
    Serialize(serde_json :: Error,),
    Write(path: String,
    source: std :: io :: Error,),
}
```



## Functions

### `pub fn write_report`

```rust
pub fn write_report(...) { ... }
```

Run the replay suite and atomically materialize JSON and self-contained HTML
under `output_directory`. The report has no wall-clock timestamp so equal
inputs produce byte-for-byte equal JSON.

