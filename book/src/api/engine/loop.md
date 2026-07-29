# Module: `loop`

> Source: `loop.rs`

Main trading engine event loop.
Drives the [`Engine`] lifecycle: subscribes to exchange streams, evaluates
strategy conditions via QFL runtime, manages order placement/tracking,
applies risk controls, and coordinates all subsystems.

## Structs

### `pub struct Engine<E : Exchange>`

```rust
pub struct Engine<E : Exchange> {
    exchange: E,
    symbols: Vec < String >,
    orders_rx: crossbeam_channel :: Receiver < Order >,
    control_rx: StrategyControlReceiver,
    control_sender: StrategyControlSender,
    qfl: QflRuntime,
    risk: RiskControls,
    logger: TradeLog,
    order_manager: OrderManager,
    order_journal: OrderJournal,
    execution_sync_ready: bool,
    execution_halted: bool,
    strategy_lifecycle: StrategyLifecycle,
    telemetry: Arc < RuntimeTelemetry >,
    indicators: IndicatorBank,
    last_price: f64,
    daily_pnl: f64,
    peak_equity: f64,
    balance_names: Vec < String >,
    balance_values: Vec < f64 >,
    position: Option < Position >,
    next_eval: Instant,
    next_account: Instant,
    entry_price_slot: u16,
    unrealized_pnl_slot: u16,
    profiling_frame: u64,
};
```



## Enums

### `pub enum EngineError`

```rust
pub enum EngineError {
    Exchange(ExchangeError,),
    Strategy(String,),
    RiskRejected(String,),
    OrderTimeout(String,),
    Journal(JournalError,),
}
```


