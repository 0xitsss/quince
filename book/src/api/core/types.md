# Module: `types`

> Source: `types.rs`

Core domain types shared across all Quince crates.
Defines [`Trade`], [`Side`], [`Depth`], [`Order`], [`Position`], [`Balance`],
and related types used throughout the trading pipeline.

## Structs

### `pub struct Trade`

```rust
pub struct Trade {
    pub price: f64,
    pub qty: f64,
    pub time: DateTime < Utc >,
    pub side: Side,
    pub trade_id: u64,
};
```


### `pub struct DepthLevel`

```rust
pub struct DepthLevel {
    pub price: f64,
    pub qty: f64,
};
```


### `pub struct Depth`

```rust
pub struct Depth {
    pub bids: Vec < DepthLevel >,
    pub asks: Vec < DepthLevel >,
};
```


### `pub struct Order`

```rust
pub struct Order {
    pub symbol: Arc < str >,
    pub side: Side,
    pub qty: f64,
    pub price: Option < f64 >,
    pub order_type: OrderType,
    pub reduce_only: bool,
    pub stop_loss: Option < f64 >,
    pub take_profit: Option < f64 >,
};
```


### `pub struct OrderFill`

```rust
pub struct OrderFill {
    pub order_id: String,
    pub side: Side,
    pub price: f64,
    pub qty: f64,
    pub fee: f64,
    pub fee_asset: String,
    pub time: DateTime < Utc >,
};
```


### `pub struct AccountInfo`

```rust
pub struct AccountInfo {
    pub balances: Vec < Balance >,
    pub positions: Vec < Position >,
};
```


### `pub struct Balance`

```rust
pub struct Balance {
    pub asset: String,
    pub wallet: f64,
    pub cross_wallet: f64,
};
```


### `pub struct Position`

```rust
pub struct Position {
    pub symbol: String,
    pub side: PositionSide,
    pub size: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
};
```



## Enums

### `pub enum Side`

```rust
pub enum Side {
    Buy,
    Sell,
}
```


### `pub enum OrderType`

```rust
pub enum OrderType {
    Market,
    Limit,
}
```


### `pub enum PositionSide`

```rust
pub enum PositionSide {
    Long,
    Short,
    None,
}
```


