# Module: `log_buffer`

> Source: `log_buffer.rs`

Debug-only ring buffer for strategy log messages.

Stores the most recent `max` log entries, dropping oldest when full.
Only compiled in `debug_assertions` builds.

## Structs

### `pub struct LogBuffer`

```rust
pub struct LogBuffer {
    entries: VecDeque < String >,
    max: usize,
};
```

Ring buffer for strategy log messages.
Drops oldest entries when `max` capacity is reached.

