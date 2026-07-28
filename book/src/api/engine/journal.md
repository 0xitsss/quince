# Module: `journal`

> Source: `journal.rs`

Durable append-only order journal.

The journal is deliberately independent from the live order manager.  It
records the client-order-id lifecycle before the engine attempts a remote
action, allowing a future startup recovery pass to find orders whose
submission outcome is unknown.  Each record is one versioned JSON line and
is synced before [`OrderJournal::append`] returns.

## Structs

### `pub struct JournalRecord`

```rust
pub struct JournalRecord {
    pub version: u32,
    pub sequence: u64,
    pub recorded_at_ms: u64,
    pub event: JournalEvent,
};
```


### `pub struct OrderJournal`

```rust
pub struct OrderJournal {
    path: PathBuf,
    file: File,
    next_sequence: u64,
};
```

A single-process writer for a durable order lifecycle journal.


## Enums

### `pub enum JournalEvent`

```rust
pub enum JournalEvent {
    Registered(client_order_id: String,
    symbol: String,
    side: String,
    qty: f64,
    reduce_only: bool,),
    Accepted(client_order_id: String,
    exchange_order_id: String,),
    SubmissionUnknown(client_order_id: String,
    error: String,),
    CancelRequested(client_order_id: String,
    exchange_order_id: String,),
    Terminal(client_order_id: String,
    status: String,),
}
```


### `pub enum JournalError`

```rust
pub enum JournalError {
    Io(std :: io :: Error,),
    Json(line: usize,
    source: serde_json :: Error,),
    Serialize(serde_json :: Error,),
    UnsupportedVersion(line: usize,
    version: u32,),
    InvalidSequence(line: usize,
    expected: u64,
    actual: u64,),
    Clock,
}
```



## Type Aliases

### `pub type Result`

```rust
pub type Result = std :: result :: Result < T , JournalError >;
```



## Constants

### `pub const JOURNAL_VERSION`

```rust
pub const JOURNAL_VERSION: u32 = ...;
```

Current on-disk JSONL schema version.

