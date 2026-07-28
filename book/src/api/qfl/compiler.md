# Module: `compiler`

> Source: `compiler.rs`

QFL AST в†’ IR bytecode compiler.

Translates a type-checked [`Program`] AST into a [`QfrProgram`] bytecode
representation. Allocates registers, emits opcodes, and builds the constant
pool and entry-point table.

Entry point: [`compile()`].

## Functions

### `pub fn compile`

```rust
pub fn compile(...) { ... }
```

Top-level entry point: compile a QFL AST Program into a QfrProgram (bytecode).
Returns `Err(Vec<TypeError>)` if compilation errors occur (e.g. register overflow).

### `pub fn compile_checked`

```rust
pub fn compile_checked(...) { ... }
```

Type-check the program first, then compile if it passes.
Returns `Err(Vec<TypeError>)` if type checking or compilation fails.

