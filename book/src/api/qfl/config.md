# Module: `config`

> Source: `config.rs`

Strategy-level exchange configuration parsed from QFL directives.

## Structs

### `pub struct StrategyConfig`

```rust
pub struct StrategyConfig {
    pub exchange: ExchangeKind,
    pub network: Network,
};
```



## Enums

### `pub enum ExchangeKind`

```rust
pub enum ExchangeKind {
    Binance,
    Hyperliquid,
}
```


### `pub enum Network`

```rust
pub enum Network {
    Mainnet,
    Testnet,
}
```



## Functions

### `pub fn parse_strategy_config`

```rust
pub fn parse_strategy_config(...) { ... }
```

Parse configuration directives without compiling the strategy bytecode.

### `pub fn load_strategy_config`

```rust
pub fn load_strategy_config(...) { ... }
```


