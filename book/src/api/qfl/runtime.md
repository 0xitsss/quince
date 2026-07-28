# Module: `runtime`

> Source: `runtime.rs`

QFL runtime — high-level interface between the trading engine and the VM.

Owns a [`Vm`], a compiled strategy path, symbol context, an order-sending
channel, and a [`RiskEngine`](quince_risk::RiskEngine). Exposes `feed_*`
methods that push external events (trade, depth, fill, eval) into the VM.

Entry point: [`QflRuntime::load()`].

## Structs

### `pub struct QflRuntime`

```rust
pub struct QflRuntime {
    vm: Vm,
    path_qfl: PathBuf,
    current_symbol: Arc < str >,
    orders_tx: Option < crossbeam_channel :: Sender < quince_core :: types :: Order > >,
    pub risk_engine: crate :: risk :: RiskEngine,
};
```



## Enums

### `pub enum Event`

```rust
pub enum Event {
    Trade(Trade,),
    Depth(Depth,),
    Fill(OrderFill,),
    Eval,
}
```

Unified exchange event dispatched to the QFL runtime.
Each variant triggers a different handler (`on_trade`, `on_depth`, etc.)
inside the VM.

