# Module: `okx_import`

> Source: `okx_import.rs`

Streaming importer for reconstructed OKX/Tardis `book_snapshot_25` CSV.

Input is read from stdin so compressed archives can be decompressed outside
the process without buffering a trading day in memory.

## Functions

### `pub fn import_snapshot_25`

```rust
pub fn import_snapshot_25(...) { ... }
```


### `pub fn import_trades`

```rust
pub fn import_trades(...) { ... }
```


