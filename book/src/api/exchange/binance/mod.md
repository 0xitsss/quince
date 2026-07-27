# Module: `binance/mod`

> Source: `binance/mod.rs`

Authenticated Binance exchange implementation.
Provides REST order placement, account queries, and WebSocket-backed
market data streaming via the [`Binance`] struct.

## Structs

### `pub struct Binance`

```rust
pub struct Binance {
    api_key: String,
    secret_key: String,
    testnet: bool,
    client: OnceLock < ws :: WsClient >,
};
```


