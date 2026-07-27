# Module: `hyperliquid/public`

> Source: `hyperliquid/public.rs`

Read-only Hyperliquid market-data adapter.

Subscribes to the official `trades` and `l2Book` WebSocket feeds. Trading
is deliberately rejected here: Hyperliquid requires EIP-712 action signing,
which must be implemented as a dedicated authenticated adapter.

## Structs

### `pub struct HyperliquidPublic`

```rust
pub struct HyperliquidPublic {
    testnet: bool,
};
```


