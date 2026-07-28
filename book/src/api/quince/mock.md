# Module: `mock`

> Source: `mock.rs`

Mock exchange for local strategy testing.
[`MockExchange`] simulates order matching, position tracking, and price
streams without network dependencies — suitable for integration tests
and strategy dry-runs.

## Structs

### `pub struct MockExchange`

```rust
pub struct MockExchange {
    order_counter: AtomicU64,
    public: Option < BinancePublic >,
    state: Arc < Mutex < MockState > >,
};
```


