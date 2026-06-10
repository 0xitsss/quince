// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL (Quince-flavored Language) — a domain-specific embedded language
//! for algorithmic trading strategies.
//!
//! The pipeline: source text в†’ [lexer] в†’ tokens в†’ [parser] в†’ AST в†’
//! [type checker] в†’ annotated AST в†’ [compiler] в†’ QfrProgram (IR) в†’
//! [optimizer] в†’ optimized bytecode в†’ [VM] execution.
//!
//! # Architecture
//!
//! | Module | Role |
//! |--------|------|
//! | [`lexer`] | Tokenises QFL source into 73 token kinds |
//! | [`parser`] | Pratt parser producing an AST |
//! | [`ast`] | AST node definitions (Expr, Stmt, BinOp, etc.) |
//! | [`types`] | Domain-specific type system (10 types) |
//! | [`compiler`] | AST в†’ IR bytecode compilation |
//! | [`opcodes`] | 70 opcodes with jump-table dispatch |
//! | [`ir`] | QfrProgram bytecode format (V1/V2) |
//! | [`optimize`] | 11-pass optimisation pipeline |
//! | [`vm`] | Register-based VM (Hot/Cold split) |
//! | [`runtime`] | QFL <-> trading engine bridge |
//! | [`risk`] | Risk limits and order validation |
//! | [`profiler`] | Opcode counts and handler timing |
//! | [`tracer`] | Event ring buffer (signals, fills, risk) |
//! | [`log_buffer`] | Debug-only ring buffer for strategy logs |

#![allow(incomplete_features)]
#![feature(explicit_tail_calls)]
#![allow(internal_features)]
#![feature(core_intrinsics)]

pub mod ast;
pub mod compiler;
pub mod ir;
pub mod lexer;
pub mod log_buffer;
pub mod opcodes;
pub mod optimize;
pub mod parser;
pub mod profiler;
pub mod risk;
pub mod runtime;
pub mod tracer;
pub mod types;
pub mod vm;
