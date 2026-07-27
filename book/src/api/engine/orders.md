# Module: `orders`

> Source: `orders.rs`

Order lifecycle management.
Tracks pending orders, active stop-loss/take-profit levels, and order fill
reconciliation via [`OrderManager`], [`PendingOrder`], and [`ActiveStop`].

## Structs

### `pub struct ActiveStop`

```rust
pub struct ActiveStop {
    pub client_id: String,
    pub side: Side,
    pub qty: f64,
    pub entry_price: f64,
    pub stop_loss: Option < f64 >,
    pub take_profit: Option < f64 >,
};
```


### `pub struct PendingOrder`

```rust
pub struct PendingOrder {
    pub client_id: String,
    pub order: Order,
    pub status: PendingStatus,
    pub placed_at: Instant,
    pub last_update: Instant,
    pub filled_qty: f64,
    pub avg_price: f64,
};
```


### `pub struct OrderManager`

```rust
pub struct OrderManager {
    pub orders: HashMap < String , PendingOrder >,
    pub exchange_to_client: HashMap < String , String >,
    next_id: u64,
};
```



## Enums

### `pub enum PendingStatus`

```rust
pub enum PendingStatus {
    Waiting,
    Placed(order_id: String,),
    PartiallyFilled(order_id: String,
    filled_qty: f64,),
    Filled,
    Cancelled,
    Failed(String,),
}
```


