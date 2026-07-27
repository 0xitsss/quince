# Module: `wallet`

> Source: `wallet.rs`

Local EVM wallet onboarding for Hyperliquid.

Private keys are stored only in the operating system credential store. The
local profile contains the public address and no signing secret.

## Structs

### `pub struct WalletProfile`

```rust
pub struct WalletProfile {
    pub version: u8,
    pub hyperliquid_address: String,
};
```



## Functions

### `pub fn load_profile`

```rust
pub fn load_profile(...) { ... }
```


### `pub fn has_private_key`

```rust
pub fn has_private_key(...) { ... }
```


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

Start a terminal-only setup wizard. Private-key input is never echoed.

