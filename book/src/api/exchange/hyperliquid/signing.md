# Module: `hyperliquid/signing`

> Source: `hyperliquid/signing.rs`

Minimal, vector-tested primitives for Hyperliquid L1-action signatures.

This intentionally covers only the EIP-712 envelope around an already
canonical `connection_id`. Action MessagePack encoding belongs in a separate
module and must earn the same test-vector coverage before live submission.

## Functions

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

