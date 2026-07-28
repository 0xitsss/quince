# Module: `vm`

> Source: `vm.rs`

QFL bytecode VM — register-based interpreter with direct threaded dispatch.

# Architecture

The VM executes compiled [`QfrProgram`]s via a 256-entry function pointer table
([`DISPATCH_TABLE`]). Each instruction is a packed `u64`, decoded by bit-field
extractors (`rd`, `rs1`, `rs2`, `imm`).

## Hot / Cold split

The hot path (registers, PC, call stack, raw code pointer) lives in [`Vm`] (~2 KB,
fits in L1). Cold data (indicators, balances, depth book, windows, persist) lives
behind `Box<ColdVm>` (~30+ KB, L2/L3). This keeps the dispatch loop cache-friendly.

## Register file

256 slots: regs `0..=191` are conventionally integer (`i64`), `192..=255` float
(`f64`). Stored as a `union Register` (`#[repr(C)]`) for zero-overhead access.

## Dispatch

The single entry point is [`Vm::call`] which looks up an entry offset by name,
sets `vm.pc`, and calls [`Vm::run`]. `run` fetches the first instruction and
dispatches via [`DISPATCH_TABLE`]. Each handler finishes with
`become dispatch_next(vm, instr)` — a guaranteed tail-call that advances `pc`,
fetches, and dispatches the next instruction. Control-flow handlers (`vm_jmp`,
`vm_call`, etc.) set `pc` directly before tail-calling. `vm_halt` returns
normally, unwinding the flat dispatch stack back to `run`.

## Safety

Handlers use unchecked register access (`get_unchecked`) and raw pointer arithmetic
on `code_ptr`. Preconditions are documented per-handler via `# Safety` sections.
The VM is not thread-safe; each [`Vm`] is pinned to one thread.

## Structs

### `pub struct PersistSlot`

```rust
pub struct PersistSlot {
    pub tag: u8,
    pub int_val: i64,
    pub float_val: f64,
};
```

A single persist slot — survives across hot-reload cycles.
`tag` determines which field carries the value:
- `0` в†’ [`int_val`](Self::int_val)
- `1` в†’ [`float_val`](Self::float_val)

### `pub struct EmaState`

```rust
pub struct EmaState {
    pub alpha: f64,
    pub value: f64,
    pub initialized: bool,
};
```

EMA (Exponential Moving Average) state for one slot.
Used by the `vm_ema` opcode. On first push (`initialized == false`) the value
is seeded directly; thereafter it updates as `value = alpha * input + (1 - alpha) * value`.

### `pub struct WindowMeta`

```rust
pub struct WindowMeta {
    pub offset: u16,
    pub capacity: u16,
    pub head: u16,
    pub len: u16,
    pub sum: f64,
    pub sum_sq: f64,
    pub min: f64,
    pub max: f64,
    pub min_deque: [u8 ; 64],
    pub max_deque: [u8 ; 64],
    pub min_dq_front: u8,
    pub min_dq_back: u8,
    pub max_dq_front: u8,
    pub max_dq_back: u8,
};
```


### `pub struct ColdVm`

```rust
pub struct ColdVm {
    pub indicators: [f64 ; MAX_INDICATORS],
    pub indicator_by_str: Vec < u16 >,
    pub balances: [f64 ; MAX_BALANCES],
    pub balance_by_str: Vec < u16 >,
    pub depth_bids_price: [f64 ; MAX_DEPTH_LEVELS],
    pub depth_bids_qty: [f64 ; MAX_DEPTH_LEVELS],
    pub depth_asks_price: [f64 ; MAX_DEPTH_LEVELS],
    pub depth_asks_qty: [f64 ; MAX_DEPTH_LEVELS],
    pub depth_bids_len: u8,
    pub depth_asks_len: u8,
    pub persist: [PersistSlot ; PERSIST_SLOTS],
    pub window_arena: Vec < f64 >,
    pub window_meta: [WindowMeta ; MAX_WINDOWS],
    pub ema_states: [EmaState ; MAX_EMA_STATES],
    _code_owned: Vec < u64 >,
    _consts_owned: Vec < f64 >,
    _i64_consts_owned: Vec < i64 >,
    pub const_pool: Vec < ConstEntry >,
    pub const_strings: Vec < String >,
    pub indicator_map: HashMap < String , u16 >,
    pub balance_map: HashMap < String , u16 >,
    pub profiler: Option < crate :: profiler :: Profiler >,
    pub tracer: Option < crate :: tracer :: Tracer >,
    pub trace_vm_enabled: bool,
    pub trace_file: Option < std :: io :: BufWriter < std :: fs :: File > >,
    pub trace_start: std :: time :: Instant,
    pub log_buffer: Option < crate :: log_buffer :: LogBuffer >,
};
```

Cold (L2/L3) VM data — behind a `Box` to keep [`Vm`] cache-friendly.
Contains large arrays (~30+ KB) pushed out of L1: indicators, balances,
depth book, persist slots, window arena, EMA states, and profiling/tracing
infrastructure. Accessed through [`Vm::cold`].

### `pub struct Vm`

```rust
pub struct Vm {
    pub regs: [Register ; NUM_REGS],
    pub pc: usize,
    pub running: bool,
    pub call_stack: [usize ; MAX_CALL_DEPTH],
    pub call_depth: u8,
    pub code_ptr: * const u64,
    pub code_len: usize,
    pub consts_ptr: * const f64,
    pub const_count: u32,
    pub i64_consts_ptr: * const i64,
    pub i64_const_count: u32,
    pub last_price: f64,
    pub position_size: f64,
    pub has_pending_order: bool,
    pub entry_names: [u64 ; 8],
    pub entry_offsets: [u32 ; 8],
    pub entry_count: u8,
    handler_cache: [u32 ; 4],
    pub cold: Box < ColdVm >,
};
```

Hot VM — the primary interpreter struct, sized to fit in L1 cache (~2 KB).
Registers, PC, call stack, and raw code pointers live here. All cold
(large-array) state lives in [`ColdVm`] behind a `Box`.
# Thread safety
`Vm` is **not** `Send` or `Sync`. Each instance must remain on one thread.

### `pub struct VmSnapshot`

```rust
pub struct VmSnapshot {
    pub regs: [Register ; NUM_REGS],
    pub persist: [PersistSlot ; PERSIST_SLOTS],
    pub pc: usize,
    pub indicators: [f64 ; MAX_INDICATORS],
    pub balances: [f64 ; MAX_BALANCES],
};
```

Snapshot of VM state that survives hot-reload.
Captured by [`Vm::snapshot`] before a hot-reload and restored by
[`Vm::restore`] afterwards. Carries registers, persist slots, program
counter, indicators, and balances.


## Unions

### `pub union Register`

```rust
pub union Register { ... }
```

A single register slot — stores either an `i64` or `f64` via `union`.
Regs `0..=191` are conventionally integer, `192..=255` float.
Access via [`Self::from_i64`], [`Self::from_f64`], or directly through the union
fields (`reg.i`, `reg.f`).


## Functions

### `pub fn dispatch_next`

```rust
pub fn dispatch_next(...) { ... }
```

Tail-call helper: advance PC, fetch next instruction, dispatch.
Every normal handler ends with `become dispatch_next(vm, instr)`.
Control-flow handlers (jmp, jz, jnz, call, ret) set `vm.pc` directly,
then fetch + `become DISPATCH_TABLE[...]` inline.
# Safety
- `vm.code_ptr` must point to valid bytecode with at least `vm.pc + 1` instructions.
- Caller must ensure `vm` is in a consistent state before dispatching.


## Constants

### `pub const NUM_REGS`

```rust
pub const NUM_REGS: usize = ...;
```


### `pub const INT_REG_COUNT`

```rust
pub const INT_REG_COUNT: u8 = ...;
```


### `pub const PERSIST_SLOTS`

```rust
pub const PERSIST_SLOTS: usize = ...;
```


### `pub const MAX_CALL_DEPTH`

```rust
pub const MAX_CALL_DEPTH: usize = ...;
```


### `pub const MAX_INDICATORS`

```rust
pub const MAX_INDICATORS: usize = ...;
```


### `pub const MAX_BALANCES`

```rust
pub const MAX_BALANCES: usize = ...;
```


### `pub const MAX_WINDOWS`

```rust
pub const MAX_WINDOWS: usize = ...;
```


### `pub const WINDOW_ARENA_SIZE`

```rust
pub const WINDOW_ARENA_SIZE: usize = ...;
```


### `pub const MAX_DEPTH_LEVELS`

```rust
pub const MAX_DEPTH_LEVELS: usize = ...;
```


### `pub const MAX_EMA_STATES`

```rust
pub const MAX_EMA_STATES: usize = ...;
```


### `pub const REG_SEND_SIDE`

```rust
pub const REG_SEND_SIDE: u8 = ...;
```


### `pub const REG_SEND_QTY`

```rust
pub const REG_SEND_QTY: u8 = ...;
```


### `pub const REG_SEND_PRICE`

```rust
pub const REG_SEND_PRICE: u8 = ...;
```


### `pub const REG_SEND_TYPE`

```rust
pub const REG_SEND_TYPE: u8 = ...;
```


### `pub const REG_SEND_REDUCE`

```rust
pub const REG_SEND_REDUCE: u8 = ...;
```


