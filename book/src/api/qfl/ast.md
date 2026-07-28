# Module: `ast`

> Source: `ast.rs`

QFL AST node definitions — expressions, statements, and the program root.

Defines the typed AST produced by the parser and consumed by the compiler:
[`Expr`], [`Stmt`], [`Literal`], [`BinOp`], [`UnaryOp`], and [`Program`].

## Structs

### `pub struct FnParam`

```rust
pub struct FnParam {
    pub name: String,
    pub type_name: String,
};
```

A typed function parameter: `name: type`.

### `pub struct UsingEntry`

```rust
pub struct UsingEntry {
    pub name: String,
    pub params: Vec < f64 >,
};
```

An entry in the `@using` directive specifying an indicator and its parameters.


## Enums

### `pub enum BinOp`

```rust
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}
```

Binary operators supported in QFL expressions.
Includes arithmetic (`+`, `-`, `*`, `/`, `//`, `%`, `^`),
comparison (`==`, `~=`, `<`, `>`, `<=`, `>=`),
concatenation (`..`), and logical (`and`, `or`).

### `pub enum UnaryOp`

```rust
pub enum UnaryOp {
    Neg,
    Not,
    Len,
}
```

Unary operators: negation (`-`), logical not (`not`), length (`#`).

### `pub enum Literal`

```rust
pub enum Literal {
    Nil,
    Bool(bool,),
    I64(i64,),
    F64(f64,),
    String(String,),
}
```

Literal values in QFL: nil, booleans, integers, floats, and strings.

### `pub enum Expr`

```rust
pub enum Expr {
    Literal(Literal,),
    Ident(String,),
    FnCall(name: String,
    args: Vec < Expr >,),
    MethodCall(obj: String,
    method: String,
    args: Vec < Expr >,),
    FieldAccess(obj: Box < Expr >,
    field: String,),
    Index(obj: Box < Expr >,
    index: Box < Expr >,),
    Unary(op: UnaryOp,
    expr: Box < Expr >,),
    Binary(lhs: Box < Expr >,
    op: BinOp,
    rhs: Box < Expr >,),
    Table(Vec < TableField >,),
}
```

QFL expression node.
Covers literals, identifiers, function/method calls, field/index access,
unary and binary operations, and table constructors.

### `pub enum TableField`

```rust
pub enum TableField {
    KeyValue(key: Expr,
    value: Expr,),
    Value(Expr,),
}
```

A field in a table constructor: either `[key] = value` or a plain value.

### `pub enum Stmt`

```rust
pub enum Stmt {
    VarDecl(names: Vec < String >,
    type_name: Option < String >,
    init: Option < Vec < Expr > >,
    is_local: bool,
    persist: bool,),
    Assign(targets: Vec < Expr >,
    exprs: Vec < Expr >,),
    If(cond: Box < Expr >,
    then_body: Vec < Stmt >,
    elseif_branches: Vec < (Box < Expr > , Vec < Stmt >) >,
    else_body: Vec < Stmt >,),
    While(cond: Box < Expr >,
    body: Vec < Stmt >,),
    Repeat(body: Vec < Stmt >,
    until: Box < Expr >,),
    ForNum(var: String,
    from: Box < Expr >,
    to: Box < Expr >,
    step: Option < Box < Expr > >,
    body: Vec < Stmt >,),
    ForIn(vars: Vec < String >,
    exprs: Vec < Expr >,
    body: Vec < Stmt >,),
    FunctionDecl(name: String,
    params: Vec < String >,
    body: Vec < Stmt >,),
    Return(exprs: Vec < Expr >,),
    ExprStmt(Expr,),
    Using(indicators: Vec < UsingEntry >,),
    Window(name: String,
    capacity: usize,),
    Exchange(name: String,),
    Network(name: String,),
    Feature(name: String,
    expr: Box < Expr >,),
    Signal(name: String,
    expr: Box < Expr >,),
    EventHandler(event: String,
    param: Option < String >,
    body: Vec < Stmt >,),
    FnDecl(name: String,
    params: Vec < FnParam >,
    return_type: String,
    body: Vec < Stmt >,),
}
```

QFL statement node.
Includes variable declarations, assignments, control flow
(if/while/repeat/for), function definitions, event handlers,
and declarative pipeline statements (using, window, feature, signal, state).


## Type Aliases

### `pub type Program`

```rust
pub type Program = Vec < Stmt >;
```

The top-level QFL program: a list of statements

