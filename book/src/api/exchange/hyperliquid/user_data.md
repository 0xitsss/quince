# Module: `hyperliquid/user_data`

> Source: `hyperliquid/user_data.rs`

Hyperliquid private user-stream decoding and supervision.

The WebSocket subscriptions are read-only, but their payloads are part of
the execution integrity boundary. A malformed event, disconnect, or full
ingress queue therefore emits `ReconcileRequired` before reconnecting.

## Enums

### `pub enum UserDataParseError`

```rust
pub enum UserDataParseError {
    Json(String,),
    Invalid(& 'static str,),
}
```



## Functions

### `pub fn start_user_data_stream`

```rust
pub fn start_user_data_stream(...) { ... }
```

Starts a self-healing private user-data supervisor. A returned value means
the initial subscriptions are live; later gaps cause an immediate engine
reconciliation signal and bounded-delay reconnect.

### `pub fn parse_user_data_msgs`

```rust
pub fn parse_user_data_msgs(...) { ... }
```

Parses every lossless engine event in one private WebSocket payload.
`userFills` may contain multiple fills; dropping all but the first would
silently understate fee and position accounting.


## Type Aliases

### `pub type ParseResult`

```rust
pub type ParseResult = std :: result :: Result < T , UserDataParseError >;
```


