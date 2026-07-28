# Module: `tracer`

> Source: `tracer.rs`

QFL event tracer — ring buffer for strategy execution events.

Records [`TraceEvent`]s (Signal, Feature, Fill, RiskAction) for post-hoc
analysis. Fixed-capacity ring buffer; drops oldest events when full.

Entry point: [`Tracer::record()`].

## Structs

### `pub struct Tracer`

```rust
pub struct Tracer {
    events: Vec < TraceEvent >,
    capacity: usize,
};
```

Ring-buffer event tracer for strategy execution.
Records signals, features, fills, and risk actions for post-hoc analysis.
Zero-allocation in the hot path when capacity is 0.


## Enums

### `pub enum TraceEvent`

```rust
pub enum TraceEvent {
    Signal(kind: String,
    result: bool,),
    Feature(name: String,
    value: f64,),
    Fill(price: f64,
    qty: f64,
    side: String,),
    RiskAction(verdict: String,
    reason: String,),
}
```

A recorded event for post-hoc analysis of strategy execution.
Each variant carries domain-specific payload:
- `Signal`: a strategy signal (opcode comparison result)
- `Feature`: a computed feature value (e.g. EMA, SMA)
- `Fill`: an executed order fill
- `RiskAction`: a risk engine verdict

