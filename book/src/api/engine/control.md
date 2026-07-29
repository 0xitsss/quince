# Module: `control`

> Source: `control.rs`

Bounded, auditable control-plane commands for strategy lifecycle changes.

HTTP and other operator transports only receive a [`StrategyControlSender`].
The engine loop owns the matching [`StrategyControlReceiver`] and applies
commands through [`StrategyLifecycle`].  This deliberately prevents
a transport handler from mutating the VM, journal, or exchange directly.

## Structs

### `pub struct StrategyControlRequest`

```rust
pub struct StrategyControlRequest {
    pub id: u64,
    pub requested_by: String,
    pub command: StrategyControlCommand,
};
```

A single command with a caller-supplied operator identity.

### `pub struct StrategyControlAuditRecord`

```rust
pub struct StrategyControlAuditRecord {
    pub audit_sequence: u64,
    pub timestamp: DateTime < Utc >,
    pub request: StrategyControlRequest,
    pub status: StrategyControlAuditStatus,
    pub detail: Option < String >,
};
```

Immutable audit event emitted when a command is queued or resolved.

### `pub struct StrategyControlSender`

```rust
pub struct StrategyControlSender {
    sender: Sender < StrategyControlRequest >,
    next_request_id: Arc < AtomicU64 >,
    audit: Arc < Mutex < AuditLog > >,
};
```

Send-only side exposed to control-plane transports.

### `pub struct StrategyControlReceiver`

```rust
pub struct StrategyControlReceiver {
    receiver: Receiver < StrategyControlRequest >,
    audit: Arc < Mutex < AuditLog > >,
};
```

Engine-owned receive side. Only this side may take a command from the queue
and append its terminal audit result.


## Enums

### `pub enum StrategyControlCommand`

```rust
pub enum StrategyControlCommand {
    DeployShadow(version: u64,
    artifact_digest: [u8 ; 32],),
    PromoteShadow,
    Rollback,
    DemoteToShadow,
    PauseExecution(reason: String,),
    ResumeExecution,
}
```

A lifecycle command that an external control plane may request.
There is intentionally no `DeployLive` or generic `SetMode(Live)` command:
an operator must deploy a candidate into shadow and explicitly promote the
active shadow revision through the lifecycle state machine.

### `pub enum StrategyControlCommandKind`

```rust
pub enum StrategyControlCommandKind {
    DeployShadow,
    PromoteShadow,
    Rollback,
    DemoteToShadow,
    PauseExecution,
    ResumeExecution,
}
```

Stable command label suitable for audit/filtering APIs.

### `pub enum StrategyControlAuditStatus`

```rust
pub enum StrategyControlAuditStatus {
    Queued,
    Applied,
    Rejected,
}
```

Lifecycle command result as retained in the audit stream.

### `pub enum StrategyControlError`

```rust
pub enum StrategyControlError {
    ZeroQueueCapacity,
    ZeroAuditCapacity,
    InvalidActor,
    QueueFull,
    Disconnected,
}
```



## Functions

### `pub fn strategy_control_channel`

```rust
pub fn strategy_control_channel(...) { ... }
```

Create a bounded control command queue and bounded audit stream.

### `pub fn default_strategy_control_channel`

```rust
pub fn default_strategy_control_channel(...) { ... }
```

Create a control queue with production defaults.


## Constants

### `pub const DEFAULT_CONTROL_QUEUE_CAPACITY`

```rust
pub const DEFAULT_CONTROL_QUEUE_CAPACITY: usize = ...;
```

Default maximum number of commands waiting for the engine loop.

### `pub const DEFAULT_CONTROL_AUDIT_CAPACITY`

```rust
pub const DEFAULT_CONTROL_AUDIT_CAPACITY: usize = ...;
```

Default number of in-memory audit records retained for operator inspection.

