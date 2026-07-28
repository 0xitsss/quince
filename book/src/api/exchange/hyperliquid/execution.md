# Module: `hyperliquid/execution`

> Source: `hyperliquid/execution.rs`

Safe boundary for authenticated Hyperliquid execution.

This module deliberately does **not** serialize or submit L1 actions yet.
Hyperliquid's action signatures depend on canonical msgpack encoding and a
protocol-specific EIP-712 payload.  A locally-valid ECDSA signature is not
sufficient proof that the exchange will recover the intended signer.  Until
that encoding is covered by official test vectors, every mutating operation
fails closed.

The types here are still useful now: they keep private-key ownership out of
the exchange adapter, bind a signer to an account, validate order intents,
and provide one place to add a reviewed signing implementation later.

## Structs

### `pub struct HyperliquidSignature`

```rust
pub struct HyperliquidSignature {
    pub r: String,
    pub s: String,
    pub v: u8,
};
```

A signature produced by an external EIP-712/L1-action signer.
The adapter never receives a private key.  The signer may be backed by an
OS keychain, hardware wallet, or a separate signing process.

### `pub struct ValidatedOrder`

```rust
pub struct ValidatedOrder {
    pub order: Order,
    pub network: HyperliquidNetwork,
    pub account_address: String,
};
```

A checked order intent.  It is intentionally not a wire request.

### `pub struct HyperliquidExecution`

```rust
pub struct HyperliquidExecution {
    network: HyperliquidNetwork,
    account_address: String,
    signer: Arc < dyn HyperliquidSigner >,
    public: HyperliquidPublic,
};
```

Authenticated adapter shell.
Public-data methods work through [`HyperliquidPublic`]. Mutating methods
reject until canonical action encoding, signing vectors, submission, and
reconciliation are all implemented together.


## Enums

### `pub enum HyperliquidNetwork`

```rust
pub enum HyperliquidNetwork {
    Mainnet,
    Testnet,
}
```

Hyperliquid deployment selected for an authenticated session.


## Traits

### `pub trait HyperliquidSigner`

```rust
pub trait HyperliquidSigner: Send: Sync {
    fn address (& self) -> & str;
    fn sign_l1_action (& self , action_hash : [u8 ; 32] , network : HyperliquidNetwork ,) -> Result < HyperliquidSignature >;
}
```

Boundary for a future, protocol-reviewed Hyperliquid L1 action signer.
`action_hash` must be produced by a canonical encoder with official test
vectors.  This crate intentionally does not manufacture it yet.

