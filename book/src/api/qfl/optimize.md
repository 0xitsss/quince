# Module: `optimize`

> Source: `optimize.rs`

QFL bytecode optimizer — 11-pass pipeline over compiled `QfrProgram`s.

Pipeline (each pass feeds the next):
1. `constant_fold` — evaluate constant expressions within basic blocks
2. `cfg_simplify` — merge blocks, remove unreachable code, simplify jumps
3. `sccp` — sparse conditional constant propagation (cross-block)
4. `cse` — common subexpression elimination (per-block)
5. `local_shadowing` — PersistGet/Set forwarding within blocks
6. `licm` — loop-invariant code motion
7. `loop_unroll` — unroll small constant-iteration loops
8. `fused_lowering` — peephole patterns (Mov chains, zero-based idioms)
9. `persist_coalesce` — merge adjacent persist operations
10. `dead_code_eliminate` — remove unreachable or unused instructions
11. `global_value_numbering` — redundant computation elimination

Entry point: [`optimize()`].

## Functions

### `pub fn optimize`

```rust
pub fn optimize(...) { ... }
```

Run the full optimization pipeline on a compiled program.
Pipeline order (each pass feeds the next):
1. constant_fold    — evaluate constant expressions within blocks
2. cfg_simplify     — merge blocks, remove unreachable code, simplify jumps
3. sccp             — sparse conditional constant propagation (cross-block)
4. cse              — common subexpression elimination (per-block)
5. local_shadowing  — PersistGet/Set forwarding within blocks
6. licm             — loop-invariant code motion
7. loop_unroll      — unroll small constant-iteration loops
8. fused_lowering   — peephole patterns (Mov chains, zero-based idioms)
9. gvn              — global value numbering (cross-block CSE via dominators)
10. dce              — dead code elimination (instruction-level reachability)
11. persist_coalesce — redundant PersistGet/Set removal (slot-shadowing)

### `pub fn dead_code_eliminate`

```rust
pub fn dead_code_eliminate(...) { ... }
```

Dead Code Elimination pass.
Removes instructions unreachable from any entry point.
Uses instruction-level reachability tracing (unlike CFG-based which traces blocks).
Correctly adjusts jump offsets for remaining instructions.

### `pub fn common_subexpr_elim`

```rust
pub fn common_subexpr_elim(...) { ... }
```

Common Subexpression Elimination pass.
Within a basic block, replaces repeated identical computations
with Mov from the first result register. Uses a hashmap keyed on
(opcode, rs1, operand2) to detect duplicates within the block.

### `pub fn constant_fold`

```rust
pub fn constant_fold(...) { ... }
```

Constant-folding pass.
Folds arithmetic on known-constant registers within each basic block.

### `pub fn cfg_simplify`

```rust
pub fn cfg_simplify(...) { ... }
```

CFG Simplification pass.
Builds a control flow graph, merges consecutive basic blocks,
removes unreachable blocks, and simplifies jump chains.

### `pub fn sccp`

```rust
pub fn sccp(...) { ... }
```

Sparse Conditional Constant Propagation.
Uses a lattice (Top в†’ Constant в†’ Bottom) per register, propagating
across the CFG.  Conditional branches with constant predicates are
folded: the unreachable successor is marked non-executable.
After convergence, known-constant expressions are replaced with
Ldi/Ldi64/Ldc, and blocks gated by a folded branch are removed.

### `pub fn persist_coalesce`

```rust
pub fn persist_coalesce(...) { ... }
```

PersistGet/Set coalescing optimization.
Removes redundant PersistGet when the same slot is already cached in a
register, and removes redundant PersistSet when the register value hasn't
changed since the last PersistGet of the same slot.

