# Module: `replay_suite`

> Source: `replay_suite.rs`

Deterministic batch replay reporting.

A suite never turns a failed/unsupported strategy into a zero-result run.
Every discovered artifact has a corresponding outcome, so an operator can
distinguish a strategy that produced no intents from one that did not load.

## Structs

### `pub struct ReplaySuiteResult`

```rust
pub struct ReplaySuiteResult {
    pub strategy: String,
    pub status: String,
    pub summary: Option < ReplaySummary >,
    pub error: Option < String >,
};
```


### `pub struct ReplaySuiteSummary`

```rust
pub struct ReplaySuiteSummary {
    pub schema_version: u8,
    pub capture: String,
    pub symbol: String,
    pub strategies_discovered: u64,
    pub strategies_succeeded: u64,
    pub strategies_failed: u64,
    pub results: Vec < ReplaySuiteResult >,
};
```



## Enums

### `pub enum ReplaySuiteError`

```rust
pub enum ReplaySuiteError {
    ReadDirectory(path: String,
    source: std :: io :: Error,),
    ReadDirectoryEntry(path: String,
    source: std :: io :: Error,),
}
```



## Functions

### `pub fn run`

```rust
pub fn run(...) { ... }
```

Run every immediate `.qfl` artifact in `strategy_directory` in a
stable lexical order. The capture is replayed separately for every strategy
so state can never leak between artifacts.

