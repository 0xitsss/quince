# Module: `lib`

> Source: `lib.rs`

Structured trade logging.
[`TradeLog`] writes JSON-formatted fill records to a CSV-compatible
log file for post-session analysis and reconciliation.

## Structs

### `pub struct TradeLog`

```rust
pub struct TradeLog {
    writer: Option < BufWriter < File > >,
};
```


