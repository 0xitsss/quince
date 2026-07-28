# Module: `profiler`

> Source: `profiler.rs`

QFL VM performance profiler.

Tracks opcode execution counts, per-opcode RDTSC cycles, and per-handler
timing. Zero-allocation in the hot path when `None`.

Entry points: [`Profiler::record_opcode()`], [`Profiler::profile()`].

## Structs

### `pub struct OpcodeProfile`

```rust
pub struct OpcodeProfile {
    pub opcode: Opcode,
    pub count: u64,
    pub cycles: u64,
};
```

Opcode execution profile for a single run.

### `pub struct HandlerSample`

```rust
pub struct HandlerSample {
    pub name: String,
    pub elapsed_ns: u64,
    pub instr_count: u64,
};
```

Per-handler timing sample.

### `pub struct Profiler`

```rust
pub struct Profiler {
    opcode_counts: [u64 ; 65],
    opcode_cycles: [u64 ; 65],
    handler_samples: Vec < HandlerSample >,
    current_handler: Option < String >,
    handler_start: Option < Instant >,
    handler_start_instr: u64,
    pub total_instructions: u64,
};
```

Execution profiler.


## Functions

### `pub fn rdtsc`

```rust
pub fn rdtsc(...) { ... }
```

Read the x86_64 timestamp counter (RDTSC) for cycle-accurate profiling.
Returns 0 on non-x86 platforms (no cycle data available).

