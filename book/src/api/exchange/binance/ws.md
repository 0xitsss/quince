# Module: `binance/ws`

> Source: `binance/ws.rs`

Binance WebSocket client implementation.
Maintains a persistent WSS connection with automatic reconnection,
request/response routing, and HMAC-SHA256 signed authenticated requests.

## Structs

### `pub struct WsClient`

```rust
pub struct WsClient {
    pub req_tx: crossbeam_channel :: Sender < WsRequest >,
};
```


### `pub struct WsRequest`

```rust
pub struct WsRequest {
    pub method: String,
    pub params: Map < String , Value >,
    pub response_tx: oneshot :: Sender < Result < Value > >,
};
```


### `pub struct BinanceWs`

```rust
pub struct BinanceWs {
    url: String,
    api_key: String,
    secret_key: String,
};
```


