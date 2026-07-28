# Module: `lib`

> Source: `lib.rs`

QFL (Quince-flavored Language) — a domain-specific embedded language
for algorithmic trading strategies.

The pipeline: source text в†’ [lexer] в†’ tokens в†’ [parser] в†’ AST в†’
[type checker] в†’ annotated AST в†’ [compiler] в†’ QfrProgram (IR) в†’
[optimizer] в†’ optimized bytecode в†’ [VM] execution.

# Architecture

| Module | Role |
|--------|------|
| [`lexer`] | Tokenises QFL source into 73 token kinds |
| [`parser`] | Pratt parser producing an AST |
| [`ast`] | AST node definitions (Expr, Stmt, BinOp, etc.) |
| [`types`] | Domain-specific type system (10 types) |
| [`compiler`] | AST в†’ IR bytecode compilation |
| [`opcodes`] | 70 opcodes with jump-table dispatch |
| [`ir`] | QfrProgram bytecode format (V1/V2) |
| [`optimize`] | 11-pass optimisation pipeline |
| [`vm`] | Register-based VM (Hot/Cold split) |
| [`runtime`] | QFL <-> trading engine bridge |
| [`risk`] | Risk limits and order validation |
| [`profiler`] | Opcode counts and handler timing |
| [`tracer`] | Event ring buffer (signals, fills, risk) |
| [`log_buffer`] | Debug-only ring buffer for strategy logs |
