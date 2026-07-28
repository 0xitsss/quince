# Module: `parser`

> Source: `parser.rs`

QFL Pratt parser — token stream в†’ AST.

Implements a Pratt (precedence-climbing) parser over the [`Token`] stream
from the lexer. Produces a [`Program`] AST for subsequent type-checking
and compilation.

Entry point: [`Parser::parse()`].

## Structs

### `pub struct ParseError`

```rust
pub struct ParseError {
    pub msg: String,
    pub pos: usize,
};
```

Error produced during parsing, carrying the message and token position.

### `pub struct Parser`

```rust
pub struct Parser {
    tokens: Vec < Token >,
    pos: usize,
};
```



## Functions

### `pub fn parse`

```rust
pub fn parse(...) { ... }
```


