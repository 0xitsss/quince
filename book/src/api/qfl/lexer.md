# Module: `lexer`

> Source: `lexer.rs`

QFL lexer — tokenises source text into 73 token kinds.

Produces a [`Token`] stream consumed by the Pratt parser. Handles string
escapes, block comments, Lua-style `--` comments, and `@directive` markers.

Entry point: [`tokenize()`] or [`Lexer::tokenize()`].

## Structs

### `pub struct LexerError`

```rust
pub struct LexerError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
};
```

An error produced during lexing with source position information.

### `pub struct Lexer`

```rust
pub struct Lexer {
    chars: Vec < char >,
    pos: usize,
    line: usize,
    col: usize,
};
```

Character-level lexer that scans QFL source text into tokens.


## Enums

### `pub enum Token`

```rust
pub enum Token {
    Function,
    Local,
    If,
    Then,
    Else,
    ElseIf,
    End,
    While,
    Do,
    Repeat,
    Until,
    For,
    In,
    Return,
    And,
    Or,
    Not,
    Nil,
    True,
    False,
    Number(String,),
    String(String,),
    Ident(String,),
    Plus,
    Minus,
    Star,
    Slash,
    SlashSlash,
    Percent,
    Caret,
    Hash,
    Dot,
    Comma,
    Colon,
    Semi,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Eq,
    EqEq,
    TildeEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Concat,
    VarArg,
    Arrow,
    AtPersist,
    AtUsing,
    AtWindow,
    AtExchange,
    AtNetwork,
    On,
    Fn,
    Comment(String,),
    Eof,
}
```

A single token produced by the QFL lexer.
Covers 73 variants including keywords, literals, operators,
symbols, directives (@persist, @using, @window), and phase-4h
keywords (state, on, fn).


## Functions

### `pub fn tokenize`

```rust
pub fn tokenize(...) { ... }
```

Tokenise a QFL source string into a token vector.
Validates input size (max 1 MiB), rejects null bytes,
and reports line/col on errors.

