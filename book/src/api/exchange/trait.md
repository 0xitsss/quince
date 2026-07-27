# Module: `trait`

> Source: `trait.rs`

Exchange trait definitions and shared types.
Defines [`Exchange`], [`ExchangeError`], [`StreamMsg`], [`OrderStatus`],
and the [`Stream`] subscription handle used by all exchange backends.

## Structs

### `pub struct Stream`

```rust
pub struct Stream {
    pub rx: crossbeam_channel :: Receiver < StreamMsg >,
};
```


### `pub struct OrderStatus`

```rust
pub struct OrderStatus {
    pub order_id: String,
    pub symbol: String,
    pub side: Side,
    pub qty: f64,
    pub filled_qty: f64,
    pub price: f64,
    pub avg_price: f64,
    pub status: String,
};
```



## Enums

### `pub enum ExchangeError`

```rust
pub enum ExchangeError {
    Ws(String,),
    Rest(String,),
    Auth(String,),
    Order(String,),
    Timeout,
    Disconnected,
}
```


### `pub enum StreamMsg`

```rust
pub enum StreamMsg {
    Trade(Trade,),
    Depth(Depth,),
    MarkPrice(price: f64,
    time: chrono :: DateTime < chrono :: Utc >,),
    OpenInterest(qty: f64,
    time: chrono :: DateTime < chrono :: Utc >,),
    ForceOrder(Trade,),
    AccountUpdate(AccountInfo,),
    OrderUpdate(OrderFill,),
}
```



## Traits

### `pub trait Exchange`

```rust
pub trait Exchange: Send: Sync {
    async fn subscribe (& self , symbols : & [String]) -> Result < Stream >;
    async fn place_order (& self , order : Order) -> Result < String >;
    async fn cancel_order (& self , symbol : & str , order_id : & str) -> Result < () >;
    async fn order_status (& self , symbol : & str , order_id : & str) -> Result < OrderStatus >;
    async fn account_info (& self) -> Result < AccountInfo >;
    async fn current_price (& self , symbol : & str) -> Result < f64 >;
}
```



## Type Aliases

### `pub type Result`

```rust
pub type Result = std :: result :: Result < T , ExchangeError >;
```


