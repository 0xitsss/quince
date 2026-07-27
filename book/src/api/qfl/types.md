# Module: `types`

> Source: `types.rs`

QFL type system — strong domain-specific types for algorithmic trading.

Rules:
- **Numeric types** (`I64`, `F64`, `Price`, `Qty`, `Timestamp`, `Duration`)
support arithmetic within their group and with direct promotion rules.
- **Domain types** (`Symbol`, `Side`, `OrderId`, `Bool`) are NOT numeric and
do NOT support arithmetic.
- `Price + Price в†’ Price`, `Price + Duration в†’ Price`, `Price * Qty в†’ Price`
- `Price + Side в†’ TypeError`

Entry point: [`check_program()`] validates a typed AST.

## Structs

### `pub struct TypeError`

```rust
pub struct TypeError {
    pub msg: String,
};
```

A type error with a message.


## Enums

### `pub enum QflType`

```rust
pub enum QflType {
    I64,
    F64,
    Bool,
    Timestamp,
    Duration,
    Price,
    Qty,
    Symbol,
    Side,
    OrderId,
}
```

Strongly-typed domain value.


## Functions

### `pub fn parse_state_type`

```rust
pub fn parse_state_type(...) { ... }
```

Parse a state declaration type string to QflType.
e.g. "f64" в†’ QflType::F64, "qty" в†’ QflType::Qty, "i32" в†’ QflType::I64

### `pub fn bin_op_type`

```rust
pub fn bin_op_type(...) { ... }
```

Determine the result type for `lhs op rhs`.
Returns `Err(TypeError)` if the operation is invalid.

### `pub fn unary_op_type`

```rust
pub fn unary_op_type(...) { ... }
```

Determine the result type for `op expr`.

### `pub fn literal_type`

```rust
pub fn literal_type(...) { ... }
```

Infer the type of an AST literal.

### `pub fn type_check`

```rust
pub fn type_check(...) { ... }
```

Run type-checking on a parsed QFL program.
Returns `Ok(())` if valid, or `Err(Vec<TypeError>)` listing all errors.


## Type Aliases

### `pub type TypeResult`

```rust
pub type TypeResult = Result < QflType , TypeError >;
```

Result type for binary operations, or a TypeError.

