# Module: `wallet`

> Source: `wallet.rs`

Local EVM wallet onboarding for Hyperliquid.

The public profile is stored separately from a file-encrypted private key.
The private key is encrypted with AES-256-CBC and authenticated with
HMAC-SHA-256 (encrypt-then-MAC). The passphrase is never persisted.

## Structs

### `pub struct WalletProfile`

```rust
pub struct WalletProfile {
    pub version: u8,
    pub hyperliquid_address: String,
};
```


### `pub struct EncryptedFileHyperliquidSigner`

```rust
pub struct EncryptedFileHyperliquidSigner {
    address: String,
    passphrase: Zeroizing < String >,
};
```

Signer backed by the encrypted wallet file. It retains a passphrase only
for the lifetime of this process; the decrypted signing key is zeroized
after every signature.


## Functions

### `pub fn load_profile`

```rust
pub fn load_profile(...) { ... }
```


### `pub fn has_private_key`

```rust
pub fn has_private_key(...) { ... }
```


### `pub fn load_hyperliquid_signer`

```rust
pub fn load_hyperliquid_signer(...) { ... }
```

Opens the encrypted-file signer only if its secret belongs to the public
profile. This catches a replaced encrypted file before authenticated use.

### `pub fn is_interactive`

```rust
pub fn is_interactive(...) { ... }
```


### `pub fn needs_setup`

```rust
pub fn needs_setup(...) { ... }
```


### `pub fn create_wallet`

```rust
pub fn create_wallet(...) { ... }
```


### `pub fn import_wallet`

```rust
pub fn import_wallet(...) { ... }
```


### `pub fn run_setup_wizard`

```rust
pub fn run_setup_wizard(...) { ... }
```

Start a terminal-only setup wizard. Private-key and passphrase input is
never echoed.

