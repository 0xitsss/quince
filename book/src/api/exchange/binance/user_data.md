# Module: `binance/user_data`

> Source: `binance/user_data.rs`

Strict parser for Binance USDⓈ-M Futures user-data events.

It owns the listen-key lifecycle as well as strict payload decoding. Socket
producers use a bounded crossbeam ingress and never wait for the engine.

## Enums

### `pub enum UserDataParseError`

```rust
pub enum UserDataParseError {
    Json(String,),
    Invalid(& 'static str,),
}
```

A malformed event must not be allowed to silently alter risk/accounting
state. Unknown event names are deliberately returned as `Ok(None)` so a
future Binance addition does not take down the stream by itself.


## Functions

### `pub fn start_user_data_stream`

```rust
pub fn start_user_data_stream(...) { ... }
```

Starts a self-healing private-stream supervisor. Every disconnect, parser
error, queue overflow, or listen-key failure emits `ReconcileRequired`
before reconnecting. Thus a transient stream gap never becomes invisible.

### `pub fn parse_user_data_msg`

```rust
pub fn parse_user_data_msg(...) { ... }
```

Parses `ORDER_TRADE_UPDATE` and `ACCOUNT_UPDATE` payloads from the Binance
USDⓈ-M Futures user-data stream.
`ORDER_TRADE_UPDATE` produces `OrderUpdate` only for an actual `TRADE`
execution with positive last-fill quantity. Other valid order lifecycle
events have no corresponding lossless `StreamMsg` variant and return
`Ok(None)`; they must still be consumed by a future order-status/reconcile
layer rather than being mistaken for fills.


## Type Aliases

### `pub type Result`

```rust
pub type Result = std :: result :: Result < T , UserDataParseError >;
```


