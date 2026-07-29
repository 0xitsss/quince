# Module: `checker`

> Source: `checker.rs`

QFL static analysis — linter for common mistakes and anti-patterns.

Checks source files for:
- C-style operators (`!=`, `&&`, `||`, `:=`, `++`) that are invalid in QFL
- Misspelled directives (`@persit` в†’ `@persist`)
- Trailing whitespace, mixed indentation, overly long lines
- Unterminated strings and block comments
- UTF-8 BOM, shebang lines, carriage returns, missing trailing newlines

Entry point: [`check()`] returns a list of [`Diagnostic`]s.

## Structs

### `pub struct Diagnostic`

```rust
pub struct Diagnostic {
    pub severity: Severity,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub suggestion: Option < String >,
};
```

A single diagnostic: error or warning at a specific source location.


## Enums

### `pub enum Severity`

```rust
pub enum Severity {
    Error,
    Warning,
}
```

Severity level of a diagnostic message.


## Functions

### `pub fn check`

```rust
pub fn check(...) { ... }
```

Run all static checks on a QFL source string.
Returns a list of [`Diagnostic`]s (sorted by appearance order).
Returns an empty vec for valid, clean code.

