//! QFL (Quince-flavored Language) — a domain-specific embedded language
//! for algorithmic trading strategies.
//!
//! The pipeline: source text → [lexer] → tokens → [parser] → AST →
//! [type checker] → annotated AST → [compiler] → QfrProgram (IR) →
//! [optimizer] → optimized bytecode → [VM] execution.
//!
//! # Architecture
//!
//! | Module | Role |
//! |--------|------|
//! | [`lexer`] | Tokenises QFL source into 73 token kinds |
//! | [`parser`] | Pratt parser producing an AST |
//! | [`ast`] | AST node definitions (Expr, Stmt, BinOp, etc.) |
//! | [`types`] | Domain-specific type system (10 types) |
//! | [`compiler`] | AST → IR bytecode compilation |
//! | [`opcodes`] | 70 opcodes with jump-table dispatch |
//! | [`ir`] | QfrProgram bytecode format (V1/V2) |
//! | [`optimize`] | 11-pass optimisation pipeline |
//! | [`vm`] | Register-based VM (Hot/Cold split) |
//! | [`runtime`] | QFL <-> trading engine bridge |
//! | [`risk`] | Risk limits and order validation |
//! | [`profiler`] | Opcode counts and handler timing |
//! | [`tracer`] | Event ring buffer (signals, fills, risk) |

pub mod ast;
pub mod compiler;
pub mod ir;
pub mod lexer;
pub mod opcodes;
pub mod optimize;
pub mod parser;
pub mod profiler;
pub mod risk;
pub mod runtime;
pub mod tracer;
pub mod types;
pub mod vm;
