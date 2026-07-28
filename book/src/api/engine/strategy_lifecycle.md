# Module: `strategy_lifecycle`

> Source: `strategy_lifecycle.rs`

Versioned, rollback-safe strategy deployment state.

This module deliberately contains no VM or exchange code.  A caller must
compile and validate a candidate before calling [`StrategyLifecycle::deploy`];
deployment then changes the active slot atomically from the caller's point
of view.  The previous slot retains its own opaque runtime state, so a
rollback can never run state created by a different strategy version.

## Structs

### `pub struct StrategyRevision`

```rust
pub struct StrategyRevision {
    pub version: u64,
    pub artifact_digest: [u8 ; 32],
    pub mode: DeploymentMode,
};
```

Immutable identity of compiled strategy code.

### `pub struct StrategySlot`

```rust
pub struct StrategySlot {
    pub revision: StrategyRevision,
    pub runtime_state: Vec < u8 >,
};
```

A revision plus only the state generated while that exact revision was active.

### `pub struct StrategyLifecycle`

```rust
pub struct StrategyLifecycle {
    active: Option < StrategySlot >,
    previous: Option < StrategySlot >,
};
```

Two-slot deployment register.
At most one live revision and one known-good rollback target are retained.
`deploy` validates all invariants before mutating either slot.


## Enums

### `pub enum DeploymentMode`

```rust
pub enum DeploymentMode {
    Shadow,
    Live,
}
```

Whether a deployed strategy may emit orders.

### `pub enum StrategyLifecycleError`

```rust
pub enum StrategyLifecycleError {
    ZeroVersion,
    NonMonotonicVersion(candidate: u64,
    active: u64,),
    NoRollbackTarget,
    NoActiveRevision,
    VersionOverflow,
    ActiveRevisionIsNotShadow,
}
```


