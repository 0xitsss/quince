# Module: `hyperliquid/signing`

> Source: `hyperliquid/signing.rs`

Minimal, vector-tested primitives for Hyperliquid L1-action signatures.

This covers the EIP-712 envelope and the narrow limit-order action shape
needed by the first execution path. Each wire representation is fixed by a
protocol test vector before it becomes usable by an adapter.

## Functions

### `pub fn limit_order_connection_id`

```rust
pub fn limit_order_connection_id(...) { ... }
```

Hashes the exact MessagePack limit-order action accepted by Hyperliquid.
This intentionally supports only an IOC, non-reduce-only order with no
client ID or builder. Broader action variants must add their own vectors,
rather than sharing a permissive serializer with different semantics.

### `pub fn l1_action_signing_digest`

```rust
pub fn l1_action_signing_digest(...) { ... }
```

Returns the EIP-712 digest the wallet must sign for a canonical L1 action
connection ID. The protocol uses source `a` on mainnet and `b` on testnet.

### `pub fn sign_l1_action`

```rust
pub fn sign_l1_action(...) { ... }
```

Signs a canonical L1 action connection ID using Ethereum's `r || s || v`
shape. `v` is normalized to 27 or 28 for the exchange API.

