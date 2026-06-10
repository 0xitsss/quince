// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL AST в†’ IR bytecode compiler.
//!
//! Translates a type-checked [`Program`] AST into a [`QfrProgram`] bytecode
//! representation. Allocates registers, emits opcodes, and builds the constant
//! pool and entry-point table.
//!
//! Entry point: [`compile()`].

use crate::ast::*;
use crate::ir::QfrProgram;
use crate::opcodes::{Instruction, Opcode as O};
use crate::vm::{
    PERSIST_SLOTS, REG_SEND_PRICE, REG_SEND_QTY, REG_SEND_REDUCE, REG_SEND_SIDE, REG_SEND_TYPE,
};
use std::collections::HashMap;

// --- Section: Public API ---

/// Top-level entry point: compile a QFL AST Program into a QfrProgram (bytecode).
/// Returns `Err(Vec<TypeError>)` if compilation errors occur (e.g. register overflow).
pub fn compile(program: &Program) -> Result<QfrProgram, Vec<crate::types::TypeError>> {
    let mut c = Compiler::new();
    c.compile_program(program);
    if !c.errors.is_empty() {
        return Err(c.errors);
    }
    Ok(c.prog)
}

/// Type-check the program first, then compile if it passes.
/// Returns `Err(Vec<TypeError>)` if type checking or compilation fails.
pub fn compile_checked(program: &Program) -> Result<QfrProgram, Vec<crate::types::TypeError>> {
    crate::types::type_check(program)?;
    compile(program)
}

// --- Section: Compiler Struct ---

/// Core compiler state: holds the output bytecode, symbol tables, register allocators,
/// scope tracking, persist variable mappings, fused indicator info, and error accumulator.
struct Compiler {
    prog: QfrProgram,
    /// Stack of scopes, each containing (var_name, register, is_float) tuples
    symbols: Vec<Vec<(String, u8, bool)>>,
    /// Next available integer register (regs 0-191)
    next_int_reg: u8,
    /// Next available float register (regs 192-255)
    next_float_reg: u8,
    /// Total count of integer registers allocated so far
    int_reg_count: u16,
    /// Total count of float registers allocated so far
    float_reg_count: u16,
    /// Saved register marks for inner scope pop/restore
    scope_saved_marks: Vec<(u8, u8, u16, u16)>,
    /// Persist variables: (name, slot_index)
    persist_slots: Vec<(String, u8)>,
    /// Next available persist slot index
    next_persist_slot: u8,
    /// Name of the function currently being compiled (for handler-context lookups)
    current_fn: Option<String>,
    /// Name of the handler parameter (e.g. "trade", "fill") for field access resolution
    handler_param: Option<String>,
    /// Maps state variable names to their floatness (true = f64, false = i64)
    state_types: HashMap<String, bool>,
    /// Maps indicator names (e.g. "ema5") to their FusedInfo (e.g. which state ID)
    fused_indicators: HashMap<String, FusedInfo>,
    /// Next available EMA state ID (for fused EMA lowering)
    next_ema_state: u8,
    /// Cache of int registers for active variables (avoids redundant Mov)
    active_int_regs: HashMap<String, u8>,
    /// Cache of float registers for active variables
    active_float_regs: HashMap<String, u8>,
    /// Accumulated compilation errors
    errors: Vec<crate::types::TypeError>,
}

/// Describes a fused (compiler-lowered) indicator.
/// Currently only EMA is fused; the VM executes the EMA state update inline.
#[derive(Clone)]
enum FusedInfo {
    Ema { state_id: u8 },
}

// --- Section: Compiler Implementation ---

impl Compiler {
    // --- Constructor ---

    fn new() -> Self {
        Compiler {
            prog: QfrProgram::new(),
            symbols: vec![Vec::new()],
            // Integer registers start at 0 and go up
            next_int_reg: 0,
            // Float registers start at 192 (after 192 int regs) and go up
            next_float_reg: 192,
            int_reg_count: 0,
            float_reg_count: 0,
            scope_saved_marks: Vec::new(),
            persist_slots: Vec::new(),
            next_persist_slot: 0,
            current_fn: None,
            handler_param: None,
            state_types: HashMap::new(),
            fused_indicators: HashMap::new(),
            next_ema_state: 0,
            active_int_regs: HashMap::new(),
            active_float_regs: HashMap::new(),
            errors: Vec::new(),
        }
    }

    // --- Section: Top-Level Program Compilation ---

    /// Compile every statement in the program sequentially.
    fn compile_program(&mut self, program: &Program) {
        for stmt in program {
            self.compile_stmt(stmt);
        }
    }

    // --- Section: Register Allocation ---

    /// Allocate an integer register (0-191).
    /// Pushes a register overflow error if we exceed the limit.
    /// Returns the allocated register number.
    fn alloc_int(&mut self) -> u8 {
        if self.int_reg_count >= 192 {
            panic!("register overflow: integer register limit (max 192)");
        }
        let r = self.next_int_reg;
        self.next_int_reg += 1;
        self.int_reg_count += 1;
        r
    }

    /// Allocate a float register (192-255).
    /// Pushes a register overflow error if we exceed the limit.
    /// Returns the allocated register number.
    fn alloc_float(&mut self) -> u8 {
        if self.float_reg_count >= 64 {
            panic!("register overflow: float register limit (max 64)");
        }
        let r = self.next_float_reg;
        self.next_float_reg += 1;
        self.float_reg_count += 1;
        r
    }

    /// Allocate a register of the appropriate type (int or float).
    fn alloc_type(&mut self, is_float: bool) -> u8 {
        if is_float {
            self.alloc_float()
        } else {
            self.alloc_int()
        }
    }

    /// Look up a variable by name across all scopes (innermost first).
    /// Returns `(register, is_float)` if found.
    fn lookup_var(&mut self, name: &str) -> Option<(u8, bool)> {
        for scope in self.symbols.iter().rev() {
            for (n, reg, is_float) in scope.iter().rev() {
                if n == name {
                    return Some((*reg, *is_float));
                }
            }
        }
        None
    }

    /// Define a new variable in the current (innermost) scope.
    fn define_var(&mut self, name: &str, reg: u8, is_float: bool) {
        if let Some(scope) = self.symbols.last_mut() {
            scope.push((name.to_string(), reg, is_float));
        }
    }

    // --- Section: Scope Management ---

    /// Push a new empty scope (no register marks saved — variables survive pop).
    fn push_scope(&mut self) {
        self.symbols.push(Vec::new());
    }

    /// Pop the innermost scope (variables are forgotten but registers NOT freed).
    fn pop_scope(&mut self) {
        self.symbols.pop();
    }

    /// Push a scope AND save the current register marks so that all registers
    /// allocated within this scope can be reclaimed (reused) on pop.
    fn push_inner_scope(&mut self) {
        self.scope_saved_marks.push((
            self.next_int_reg,
            self.next_float_reg,
            self.int_reg_count,
            self.float_reg_count,
        ));
        self.symbols.push(Vec::new());
    }

    /// Pop an inner scope and restore the saved register marks,
    /// effectively freeing all registers allocated within that scope.
    fn pop_inner_scope(&mut self) {
        self.symbols.pop();
        if let Some((saved_int, saved_float, saved_int_cnt, saved_float_cnt)) =
            self.scope_saved_marks.pop()
        {
            self.next_int_reg = saved_int;
            self.next_float_reg = saved_float;
            self.int_reg_count = saved_int_cnt;
            self.float_reg_count = saved_float_cnt;
        }
    }

    // --- Section: Persist Variable Slots ---

    /// Get or assign a persistent storage slot for the given variable name.
    /// Persist slots are indexed 0..N and are used by PersistGet/PersistSet opcodes.
    fn persist_slot(&mut self, name: &str) -> u8 {
        for (n, slot) in &self.persist_slots {
            if n == name {
                return *slot;
            }
        }
        let slot = self.next_persist_slot;
        if slot >= PERSIST_SLOTS as u8 {
            panic!("persist slot overflow: max 64 persist variables");
        }
        self.next_persist_slot += 1;
        self.persist_slots.push((name.to_string(), slot));
        slot
    }

    // --- Section: Bytecode Emission Helpers ---

    /// Append an instruction to the end of the code buffer.
    fn emit(&mut self, instr: Instruction) {
        self.prog.code.push(instr);
    }

    /// Overwrite an instruction at a specific index (used for patching jumps).
    fn emit_at(&mut self, idx: usize, instr: Instruction) {
        debug_assert!(
            idx < self.prog.code.len(),
            "emit_at {idx} >= code.len() {}",
            self.prog.code.len()
        );
        self.prog.code[idx] = instr;
    }

    /// Return the current code length as an offset (used for jump calculations).
    fn current_offset(&self) -> u32 {
        self.prog.code.len() as u32
    }

    /// Register an entry point (function/handler) with its code offset.
    /// Validates that the entry name does not exceed 8 characters.
    fn register_entry(&mut self, name: &str, offset: u32) {
        if name.len() > 8 {
            self.errors.push(crate::types::TypeError {
                msg: format!(
                    "entry name '{}' exceeds 8 character limit (len={})",
                    name,
                    name.len()
                ),
            });
        }
        self.prog.entries.push(crate::ir::EntryPoint {
            name: name.to_string(),
            code_offset: offset,
        });
    }

    // --- Section: Statement Compilation ---

    /// Dispatch compilation of a single statement based on its variant.
    fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::VarDecl {
                names,
                type_name,
                init,
                persist,
                ..
            } => {
                self.compile_var_decl(names, type_name.as_deref(), init.as_ref(), *persist);
            }
            Stmt::Assign { targets, exprs } => {
                self.compile_assign(targets, exprs);
            }
            Stmt::If {
                cond,
                then_body,
                elseif_branches,
                else_body,
            } => {
                self.compile_if(cond, then_body, elseif_branches, else_body);
            }
            Stmt::While { cond, body } => {
                self.compile_while(cond, body);
            }
            Stmt::Repeat { body, until } => {
                self.compile_repeat(body, until);
            }
            Stmt::ForNum {
                var,
                from,
                to,
                step,
                body,
            } => {
                self.compile_for_num(var, from, to, step, body);
            }
            Stmt::ForIn { vars, exprs, body } => {
                self.compile_for_in(vars, exprs, body);
            }
            Stmt::FunctionDecl { name, params, body } => {
                self.compile_fn_decl(name, params, body);
            }
            Stmt::Return { exprs } => {
                self.compile_return(exprs);
            }
            Stmt::ExprStmt(expr) => {
                self.compile_expr_discard(expr);
            }
            Stmt::Using { indicators } => {
                // Lower each `using` directive: pre-allocate fused indicator state IDs.
                // Currently only supports EMA as a fused indicator.
                for entry in indicators {
                    let name = entry.name.as_str();
                    for &period in &entry.params {
                        if name == "ema" {
                            // Assign a new state ID for this EMA; the VM will
                            // update this state on each tick using the EMA opcode.
                            let sid = self.next_ema_state;
                            self.next_ema_state += 1;
                            let alpha = 2.0 / (period + 1.0);
                            self.prog.ema_alphas.push(alpha);
                            let ind_name = format!("ema{}", period as i64);
                            self.fused_indicators
                                .insert(ind_name, FusedInfo::Ema { state_id: sid });
                        }
                    }
                }
            }
            Stmt::Window { .. } => {
                // setup directive — no bytecode emitted
            }
            Stmt::FnDecl {
                name,
                params,
                return_type: _,
                body,
            } => {
                // Compile a typed function declaration.
                let offset = self.prog.code.len() as u32;
                self.register_entry(name, offset);
                self.current_fn = Some(name.clone());
                self.handler_param = None;
                // Allocate registers for each parameter based on its declared type
                self.push_scope();
                for p in params {
                    let is_float = crate::types::parse_state_type(&p.type_name).is_float();
                    let pr = if is_float {
                        self.alloc_float()
                    } else {
                        self.alloc_int()
                    };
                    self.define_var(&p.name, pr, is_float);
                }
                for stmt in body {
                    self.compile_stmt(stmt);
                }
                // Emit Ret at end to prevent fall-through
                self.emit(Instruction::single(O::Ret));
                self.pop_scope();
                self.current_fn = None;
            }
            Stmt::EventHandler { event, param, body } => {
                // Compile a block-style event handler (e.g. `on eval() { ... }`).
                let fn_name = format!("on_{}", event);
                let offset = self.prog.code.len() as u32;
                self.register_entry(&fn_name, offset);
                self.current_fn = Some(fn_name);
                self.handler_param = param.clone();
                self.push_scope();
                // Allocate a register for the handler parameter (e.g. "t" for on_trade(t))
                if let Some(ref p) = param {
                    let pr = self.alloc_int();
                    self.define_var(p, pr, false);
                }
                // Reserve registers 0-4: pre-loaded by runtime with handler param fields
                // (price->r0, qty->r1, side->r2, trade_id->r3, time->r4)
                if self.next_int_reg < 5 {
                    self.next_int_reg = 5;
                }
                for stmt in body {
                    self.compile_stmt(stmt);
                }
                self.emit(Instruction::single(O::Ret));
                self.pop_scope();
                self.current_fn = None;
                self.handler_param = None;
            }
            Stmt::Feature { name, expr } => {
                // Compile a feature expression; result register is float by convention.
                let (r, _) = self.compile_expr(expr);
                self.define_var(name, r, true);
            }
            Stmt::Signal { name, expr } => {
                // Compile a signal expression; result register is int (boolean) by convention.
                let (r, _) = self.compile_expr(expr);
                self.define_var(name, r, false);
            }
        }
    }

    // --- Section: Variable Declaration Compilation ---

    /// Compile a variable declaration (local or persist).
    /// For persist: loads the persisted value via PersistGet; skips init (persist carries across calls).
    /// For local with init: compiles the init expression and defines the variable.
    /// For local without init: defaults to 0.
    fn compile_var_decl(
        &mut self,
        names: &[String],
        type_name: Option<&str>,
        init: Option<&Vec<Expr>>,
        persist: bool,
    ) {
        if persist {
            // For persist variables, emit PersistGet to load stored value.
            // Init values are NOT applied since persist slots carry state across calls.
            for (i, name) in names.iter().enumerate() {
                let slot = self.persist_slot(name);
                // Determine type from type annotation, or fall back to init expression
                let is_float = if let Some(tn) = type_name {
                    crate::types::parse_state_type(tn).is_float()
                } else {
                    init.is_some_and(|exprs| {
                        exprs
                            .get(i)
                            .is_some_and(|e| matches!(e, Expr::Literal(Literal::F64(_))))
                    })
                };
                let r = if is_float {
                    self.alloc_float()
                } else {
                    self.alloc_int()
                };
                self.emit(Instruction::rri(O::PersistGet, r, 0, slot as u32));
                self.define_var(name, r, is_float);
                self.state_types.insert(name.clone(), is_float);
            }
            return;
        }

        if init.is_none() {
            // local x — declare without init, default to 0
            for name in names {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                self.define_var(name, r, false);
            }
            return;
        }

        // local x = expr — compile the init expression(s) and define variables
        let exprs = init.unwrap();
        for (i, name) in names.iter().enumerate() {
            let (r, is_float) = if i < exprs.len() {
                self.compile_expr(&exprs[i])
            } else {
                // More names than expressions: pad with 0
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            };
            self.define_var(name, r, is_float);
        }
    }

    // --- Section: Assignment Compilation ---

    /// Compile assignment: evaluate RHS expressions, then move results to LHS targets.
    /// For persist variables, also emits PersistSet to store back to persistent memory.
    /// Supports field-assign targets (e.g. `obj.field = val`) via compile_quince_set.
    fn compile_assign(&mut self, targets: &[Expr], exprs: &[Expr]) {
        // Evaluate all RHS expressions first (left-to-right)
        let rhs_regs: Vec<(u8, bool)> = exprs.iter().map(|e| self.compile_expr(e)).collect();

        // Assign each RHS result to its corresponding LHS target
        for (i, target) in targets.iter().enumerate() {
            let (r, is_float) = rhs_regs.get(i).copied().unwrap_or((0, false));
            match target {
                Expr::Ident(name) => {
                    // Resolve or lazily allocate the target register
                    let vr = if let Some((vr, _)) = self.lookup_var(name) {
                        vr
                    } else {
                        let nr = self.alloc_type(is_float);
                        self.define_var(name, nr, is_float);
                        nr
                    };
                    self.emit(Instruction::rr(O::Mov, vr, r));
                    // If this is a persist variable, write the value back to its persist slot
                    for (pn, slot) in &self.persist_slots {
                        if pn == name {
                            self.emit(Instruction::rri(O::PersistSet, vr, 0, *slot as u32));
                            break;
                        }
                    }
                }
                Expr::FieldAccess { obj, field } => {
                    self.compile_quince_set(obj, field, r, is_float);
                }
                Expr::Index { obj, index } => {
                    // Index assignment is not supported yet
                    let _ = (obj, index);
                }
                _ => {}
            }
        }
    }

    // --- Section: If/Elseif/Else Compilation ---

    /// Compile an if/then/elseif/else statement chain.
    /// Uses conditional jumps (Jz) to skip branches and Jmp to skip past them.
    /// Patches placeholder offsets after each branch body is compiled.
    fn compile_if(
        &mut self,
        cond: &Expr,
        then_body: &[Stmt],
        elseif_branches: &[(Box<Expr>, Vec<Stmt>)],
        else_body: &[Stmt],
    ) {
        // Compile condition в†’ boolean int register
        let cond_reg = self.compile_cond(cond);
        let jz_to_else = self.current_offset() as usize;
        self.emit(Instruction::rri(O::Jz, 0, cond_reg, 0)); // placeholder, patched below

        // Compile the then-body in an inner scope (registers reclaimed after)
        self.push_inner_scope();
        for stmt in then_body {
            self.compile_stmt(stmt);
        }
        self.pop_inner_scope();

        // Check if the then-body ended with a terminal (Ret/Halt/Jmp)
        let then_ended_terminal = self
            .prog
            .code
            .last()
            .is_some_and(|i| matches!(i.opcode(), O::Ret | O::Halt | O::Jmp));
        // Emit a Jmp over the else/elseif branches (if there are any and then didn't end terminal)
        let jmp_to_end = if !else_body.is_empty() || !elseif_branches.is_empty() {
            if then_ended_terminal {
                None
            } else {
                let jmp = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jmp, 0, 0, 0)); // placeholder
                Some(jmp)
            }
        } else {
            None
        };

        // Patch the JZ: jump to else/elseif/end
        let after_then = self.current_offset();
        let jz_offset = after_then - jz_to_else as u32 - 1;
        self.emit_at(jz_to_else, Instruction::rri(O::Jz, 0, cond_reg, jz_offset));

        // Collect JMP-over offsets from elseif branches for patching later
        let mut elseif_jmps: Vec<(usize, u8)> = Vec::new();
        for (econd, ebody) in elseif_branches {
            let econd_reg = self.compile_cond(econd);
            let jz_to_elseif = self.current_offset() as usize;
            self.emit(Instruction::rri(O::Jz, 0, econd_reg, 0)); // placeholder

            self.push_inner_scope();
            for stmt in ebody {
                self.compile_stmt(stmt);
            }
            self.pop_inner_scope();

            // If this elseif body ended terminal, mark it (no Jmp needed)
            let ebody_ended_terminal = self
                .prog
                .code
                .last()
                .is_some_and(|i| matches!(i.opcode(), O::Ret | O::Halt | O::Jmp));
            if ebody_ended_terminal {
                elseif_jmps.push((usize::MAX, econd_reg));
            } else {
                let jmp = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jmp, 0, 0, 0)); // placeholder
                elseif_jmps.push((jmp, econd_reg));
            }

            // Patch the JZ for this elseif to skip its body if condition is false
            let after_ebody = self.current_offset();
            let jz_off = after_ebody - jz_to_elseif as u32 - 1;
            self.emit_at(jz_to_elseif, Instruction::rri(O::Jz, 0, econd_reg, jz_off));
        }
        // Patch all elseif JMPs to point past the else/end
        let after_elseif = self.current_offset();
        for (jmp_pos, _) in &elseif_jmps {
            if *jmp_pos != usize::MAX {
                let jmp_off = after_elseif - *jmp_pos as u32 - 1;
                self.emit_at(*jmp_pos, Instruction::rri(O::Jmp, 0, 0, jmp_off));
            }
        }

        // Compile the else body if present
        if !else_body.is_empty() {
            self.push_inner_scope();
            for stmt in else_body {
                self.compile_stmt(stmt);
            }
            self.pop_inner_scope();
        }

        // Patch the then-to-end JMP (skip else/elseif)
        let after_else = self.current_offset();
        if let Some(jmp_pos) = jmp_to_end {
            let jmp_off = after_else - jmp_pos as u32 - 1;
            self.emit_at(jmp_pos, Instruction::rri(O::Jmp, 0, 0, jmp_off));
        }
    }

    // --- Section: While Loop Compilation ---

    /// Compile a while loop:
    ///   loop_start: evaluate condition; if false, jump to after_loop
    ///   body; jump back to loop_start; after_loop:
    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) {
        let loop_start = self.current_offset();
        let cond_reg = self.compile_cond(cond);
        let jz_exit = self.current_offset() as usize;
        self.emit(Instruction::rri(O::Jz, 0, cond_reg, 0)); // placeholder

        // Body in inner scope (registers freed after body)
        self.push_inner_scope();
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_inner_scope();

        // Jump back to loop start to re-evaluate condition
        let jmp_back = self.current_offset();
        let back_offset = loop_start as i64 - jmp_back as i64 - 1;
        self.emit(Instruction::rri(O::Jmp, 0, 0, back_offset as u32));

        // Patch JZ to exit the loop
        let after_loop = self.current_offset();
        let jz_off = after_loop - jz_exit as u32 - 1;
        self.emit_at(jz_exit, Instruction::rri(O::Jz, 0, cond_reg, jz_off));
    }

    // --- Section: Repeat-Until Loop Compilation ---

    /// Compile a repeat-until loop:
    ///   loop_start: body; evaluate condition; if false, jump back to loop_start
    fn compile_repeat(&mut self, body: &[Stmt], until: &Expr) {
        let loop_start = self.current_offset();

        self.push_inner_scope();
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_inner_scope();

        // Evaluate condition; Jz (false) loops back to repeat (repeat-until в‰Ў while-not)
        let cond_reg = self.compile_cond(until);
        let jz_back = self.current_offset();
        let back_offset = loop_start as i64 - jz_back as i64 - 1;
        self.emit(Instruction::rri(O::Jz, 0, cond_reg, back_offset as u32));
    }

    // --- Section: Numeric For Loop Compilation ---

    /// Compile a numeric for loop:
    ///   for var = from, to, step do body end
    /// Translated to: var = from; while cond(i): body; i += step
    /// For positive step: cond = i <= to (exit when i > to)
    /// For negative step: cond = i >= to (exit when i < to)
    fn compile_for_num(
        &mut self,
        var: &str,
        from: &Expr,
        to: &Expr,
        step: &Option<Box<Expr>>,
        body: &[Stmt],
    ) {
        // Evaluate from, to, step expressions
        let (from_r, _) = self.compile_expr(from);
        let (to_r, _) = self.compile_expr(to);

        // Detect if step is a compile-time known negative constant
        let step_is_neg = |e: &Expr| -> bool {
            match e {
                Expr::Literal(Literal::I64(n)) => *n < 0,
                Expr::Literal(Literal::F64(n)) => *n < 0.0,
                Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: inner,
                } => match inner.as_ref() {
                    Expr::Literal(Literal::I64(n)) => *n > 0,
                    Expr::Literal(Literal::F64(n)) => *n > 0.0,
                    _ => false,
                },
                _ => false,
            }
        };
        let is_neg_step = step.as_ref().is_some_and(|s| step_is_neg(s));

        let step_r = if let Some(s) = step {
            let (r, _) = self.compile_expr(s);
            r
        } else {
            // Default step is 1
            let r = self.alloc_int();
            self.emit(Instruction::rri(O::Ldi, r, 0, 1));
            r
        };

        // Initialize loop variable: var = from
        let i_reg = self.alloc_int();
        self.emit(Instruction::rr(O::Mov, i_reg, from_r));
        self.define_var(var, i_reg, false);

        let loop_start = self.current_offset();
        // Check condition: for positive step exit when i > to; for negative exit when i < to
        let cmp = self.alloc_int();
        let exit_op = if is_neg_step { O::Lt } else { O::Gt };
        self.emit(Instruction::rrr(exit_op, cmp, i_reg, to_r));
        let jz_exit = self.current_offset() as usize;
        self.emit(Instruction::rri(O::Jnz, 0, cmp, 0)); // placeholder: jumps if exit condition met

        // Body in inner scope
        self.push_inner_scope();
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_inner_scope();

        // Step: i = i + step
        let tmp = self.alloc_int();
        self.emit(Instruction::rrr(O::Add, tmp, i_reg, step_r));
        self.emit(Instruction::rr(O::Mov, i_reg, tmp));

        // Jump back to loop start
        let jmp_back = self.current_offset();
        let back_offset = loop_start as i64 - jmp_back as i64 - 1;
        self.emit(Instruction::rri(O::Jmp, 0, 0, back_offset as u32));

        // Patch the exit jump
        let after_loop = self.current_offset();
        let jz_off = after_loop - jz_exit as u32 - 1;
        self.emit_at(jz_exit, Instruction::rri(O::Jnz, 0, cmp, jz_off));
    }

    // --- Section: For-In Loop Compilation (Stub) ---

    /// Compile a for-in loop.
    /// Currently a stub: allocates variables, compiles body once but does not iterate.
    fn compile_for_in(&mut self, vars: &[String], _exprs: &[Expr], body: &[Stmt]) {
        // For-in is complex (requires iterator protocol) — not yet implemented.
        // For now, initialize loop vars to 0 and compile body once.
        self.push_inner_scope();
        for v in vars {
            let r = self.alloc_int();
            self.emit(Instruction::rri(O::Ldi, r, 0, 0));
            self.define_var(v, r, false);
        }
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_inner_scope();
    }

    // --- Section: Function Declaration Compilation ---

    /// Compile a function declaration (legacy style: `function name(params) body end`).
    /// Registers the entry point, allocates parameter registers, compiles the body,
    /// and emits a trailing Ret if the body doesn't end with one.
    /// Handles special entry conventions for on_trade/on_fill (registers 0-4 pre-loaded).
    fn compile_fn_decl(&mut self, name: &str, params: &[String], body: &[Stmt]) {
        let offset = self.current_offset();
        self.register_entry(name, offset);

        // Check if this is a trade/fill handler using the legacy convention
        let is_trade_entry = name == "on_trade";
        let is_fill_entry = name == "on_fill";
        let saved_fn = self.current_fn.replace(name.to_string());

        self.push_scope();
        for (i, param) in params.iter().enumerate() {
            if (is_trade_entry || is_fill_entry) && i < 5 {
                // Trade entry convention: registers r0-r4 are pre-loaded by the VM with:
                // r0=price(float), r1=qty(float), r2=side(int), r3=trade_id(int), r4=time(int)
                let src = i as u8;
                let is_float = i < 2; // price, qty are floats; side, id, time are ints
                let dst = if is_float {
                    self.alloc_float()
                } else {
                    self.alloc_int()
                };
                self.emit(Instruction::rr(O::Mov, dst, src));
                self.define_var(param, dst, is_float);
            } else {
                // Regular parameter: allocate int register
                let r = self.alloc_int();
                self.define_var(param, r, false);
            }
        }

        // Set handler_param for on_trade/on_fill/on_depth for field access resolution
        if (name == "on_trade" || name == "on_fill" || name == "on_depth") && !params.is_empty() {
            self.handler_param = Some(params[0].clone());
        }

        // Reserve registers 0-4 (they're pre-loaded by runtime for trade event handlers)
        if (is_trade_entry || is_fill_entry) && self.next_int_reg < 5 {
            self.next_int_reg = 5;
        }

        // Compile the function body
        for stmt in body {
            self.compile_stmt(stmt);
        }
        // Emit Ret at end to prevent fall-through (unless body already ends with Ret)
        let last_op = self.prog.code.last().map(|i| i.opcode());
        if last_op != Some(O::Ret) {
            self.emit(Instruction::single(O::Ret));
        }
        self.pop_scope();

        self.current_fn = saved_fn;
    }

    // --- Section: Return Statement Compilation ---

    /// Compile a return statement. Evaluates the first expression (if any),
    /// then emits Ret. Multiple return values beyond the first are ignored.
    fn compile_return(&mut self, exprs: &[Expr]) {
        if let Some(expr) = exprs.first() {
            let (_r, _) = self.compile_expr(expr);
        }
        self.emit(Instruction::single(O::Ret));
    }

    // --- Section: Expression Compilation ---

    /// Compile an expression, returning `(register, is_float)`.
    /// The caller owns the returned register (it is a temp / SSA-like value).
    fn compile_expr(&mut self, expr: &Expr) -> (u8, bool) {
        match expr {
            Expr::Literal(lit) => self.compile_literal(lit),
            Expr::Ident(name) => self.compile_ident(name),
            Expr::FnCall { name, args } => self.compile_fn_call(name, args),
            Expr::MethodCall { obj, method, args } => self.compile_method_call(obj, method, args),
            Expr::Binary { lhs, op, rhs } => self.compile_binary(lhs, op, rhs),
            Expr::Unary { op, expr: inner } => self.compile_unary(op, inner),
            Expr::FieldAccess { obj, field } => self.compile_field_access(obj, field),
            Expr::Index { obj, index } => {
                // Index expressions are not supported
                self.errors.push(crate::types::TypeError {
                    msg: format!(
                        "index expression is not supported (index {:?} on {:?})",
                        index, obj
                    ),
                });
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
            Expr::Table(_) => {
                // Table literals are not supported
                self.errors.push(crate::types::TypeError {
                    msg: "table literal is not supported".into(),
                });
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
        }
    }

    /// Compile an expression but discard its result (used for expression statements).
    /// Special-cases `quince.order(...)` / `order(...)` and method calls with side effects
    /// (order, log, get) to ensure they still execute.
    fn compile_expr_discard(&mut self, expr: &Expr) {
        // Check for standalone `quince.order()` or `order()` calls (side-effectful)
        if let Expr::FnCall { name, args } = expr {
            if name == "quince.order" || name == "order" {
                self.compile_send_order(args);
                return;
            }
        }
        // Check for side-effectful method calls
        if let Expr::MethodCall { obj, method, args } = expr {
            if method == "order" || method == "log" || method == "get" {
                self.compile_method_call(obj, method, args);
                return;
            }
        }
        // Otherwise, compile and discard normally (result register is just lost)
        self.compile_expr(expr);
    }

    // --- Section: Literal Compilation ---

    /// Compile a literal value, allocating a register and loading the constant.
    fn compile_literal(&mut self, lit: &Literal) -> (u8, bool) {
        match lit {
            Literal::Nil => {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
            Literal::Bool(b) => {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, if *b { 1 } else { 0 }));
                (r, false)
            }
            Literal::I64(n) => {
                let r = self.alloc_int();
                self.emit_ldi(r, *n);
                (r, false)
            }
            Literal::F64(n) => {
                let r = self.alloc_float();
                let idx = self.prog.intern_f64(*n);
                self.emit(Instruction::rri(O::LdcF64, r, 0, idx));
                (r, true)
            }
            Literal::String(s) => {
                let r = self.alloc_int();
                let idx = self.prog.intern_string(s);
                self.emit(Instruction::rri(O::LdcStr, r, 0, idx));
                (r, false)
            }
        }
    }

    /// Emit an Ldi or Ldi64 instruction, selecting the optimal encoding for the immediate value.
    /// - 32-bit signed fits в†’ Ldi (3-byte encoding)
    /// - 40-bit signed fits в†’ Ldi64 with ri40 encoding
    /// - Otherwise в†’ intern into i64 constant pool and use LdI64
    fn emit_ldi(&mut self, r: u8, val: i64) {
        if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
            self.emit(Instruction::rri(O::Ldi, r, 0, val as u32));
        } else if (-(1i64 << 39)..(1i64 << 39)).contains(&val) {
            // Use ri40 for large immediates (fits in 40-bit signed)
            self.emit(Instruction::ri40(O::Ldi64, r, val));
        } else {
            // Fall back to i64 const pool (no precision loss)
            let idx = self.prog.intern_i64(val);
            self.emit(Instruction::ri(O::LdI64, r, idx));
        }
    }

    // --- Section: Identifier / Variable Access Compilation ---

    /// Compile a variable/identifier reference.
    /// Checks: persist slots в†’ trade-convention builtins (on_trade only) в†’ symbol table
    /// When found in the symbol table, caches the register for reuse (avoids redundant Mov).
    /// If not found anywhere, defaults to 0.
    fn compile_ident(&mut self, name: &str) -> (u8, bool) {
        // Check persist/state first — lazy PersistGet on first use
        for (pn, _) in &self.persist_slots {
            if pn == name {
                let is_float = self.state_types.get(name).copied().unwrap_or(false);
                let r = if is_float {
                    self.alloc_float()
                } else {
                    self.alloc_int()
                };
                let slot = self.persist_slot(name);
                self.emit(Instruction::rri(O::PersistGet, r, 0, slot as u32));
                return (r, is_float);
            }
        }
        // In on_trade handler, map built-in field names directly to pre-loaded registers
        if let Some(fname) = &self.current_fn {
            if fname == "on_trade" {
                match name {
                    "price" => {
                        let r = self.alloc_float();
                        self.emit(Instruction::rr(O::Mov, r, 0)); // r0 = price
                        return (r, true);
                    }
                    "qty" => {
                        let r = self.alloc_float();
                        self.emit(Instruction::rr(O::Mov, r, 1)); // r1 = qty
                        return (r, true);
                    }
                    "side" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 2));
                        return (r, false);
                    }
                    "trade_id" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 3));
                        return (r, false);
                    }
                    "time" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 4));
                        return (r, false);
                    }
                    _ => {}
                }
            }
        }
        // Look up in the symbol table
        if let Some((reg, is_float)) = self.lookup_var(name) {
            // Check the active-register cache to avoid redundant Mov when reusing
            let cache = if is_float {
                &mut self.active_float_regs
            } else {
                &mut self.active_int_regs
            };
            if cache.get(name) == Some(&reg) {
                // Cache hit — variable is still in the same register, return directly
                return (reg, is_float);
            }
            // First read or register was reassigned — cache and return
            cache.insert(name.to_string(), reg);
            (reg, is_float)
        } else {
            // Unknown variable: default to 0
            let r = self.alloc_int();
            self.emit(Instruction::rri(O::Ldi, r, 0, 0));
            (r, false)
        }
    }

    // --- Section: Binary Expression Compilation ---

    /// Compile a binary operation (arithmetic, comparison, logical, concatenation).
    /// Handles operand type promotion (intв†’float in mixed-type operations).
    /// For logical AND/OR: uses short-circuit evaluation with conditional jumps.
    /// For float IDiv/Mod: converts to ints, divides, converts back.
    fn compile_binary(&mut self, lhs: &Expr, op: &BinOp, rhs: &Expr) -> (u8, bool) {
        // Compile both operands
        let (left_r, left_float) = self.compile_expr(lhs);
        let (right_r, right_float) = self.compile_expr(rhs);
        let is_float = left_float || right_float;

        // If one operand is int and the other is float, promote the int to float
        let (l_final, r_final) = if left_float != right_float {
            if left_float {
                let conv = self.alloc_float();
                self.emit(Instruction::rr(O::I2F, conv, right_r));
                (left_r, conv)
            } else {
                let conv = self.alloc_float();
                self.emit(Instruction::rr(O::I2F, conv, left_r));
                (conv, right_r)
            }
        } else {
            (left_r, right_r)
        };

        let rd = self.alloc_type(is_float);

        // Dispatch to the correct opcode based on operator and floatness
        match (op, is_float) {
            (BinOp::Add, false) => self.emit(Instruction::rrr(O::Add, rd, l_final, r_final)),
            (BinOp::Sub, false) => self.emit(Instruction::rrr(O::Sub, rd, l_final, r_final)),
            (BinOp::Mul, false) => self.emit(Instruction::rrr(O::Mul, rd, l_final, r_final)),
            (BinOp::Div, false) => self.emit(Instruction::rrr(O::Div, rd, l_final, r_final)),
            (BinOp::IDiv, false) => self.emit(Instruction::rrr(O::Div, rd, l_final, r_final)),
            (BinOp::Mod, false) => self.emit(Instruction::rrr(O::Mod, rd, l_final, r_final)),
            (BinOp::Pow, false) => {
                self.emit(Instruction::rrr(O::Pow, rd, l_final, r_final));
            }
            (BinOp::Add, true) => self.emit(Instruction::rrr(O::FAdd, rd, l_final, r_final)),
            (BinOp::Sub, true) => self.emit(Instruction::rrr(O::FSub, rd, l_final, r_final)),
            (BinOp::Mul, true) => self.emit(Instruction::rrr(O::FMul, rd, l_final, r_final)),
            (BinOp::Div, true) => self.emit(Instruction::rrr(O::FDiv, rd, l_final, r_final)),
            (BinOp::IDiv, true) => {
                // Float floor division: convert to int, divide, convert back
                let tmp_i = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp_i, l_final));
                let tmp_i2 = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp_i2, r_final));
                let tmp = self.alloc_int();
                self.emit(Instruction::rrr(O::Div, tmp, tmp_i, tmp_i2));
                self.emit(Instruction::rr(O::I2F, rd, tmp));
            }
            (BinOp::Mod, true) => {
                // Float modulo: convert to int, mod, convert back
                let tmp_i = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp_i, l_final));
                let tmp_i2 = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp_i2, r_final));
                let tmp = self.alloc_int();
                self.emit(Instruction::rrr(O::Mod, tmp, tmp_i, tmp_i2));
                self.emit(Instruction::rr(O::I2F, rd, tmp));
            }
            (BinOp::Pow, true) => {
                self.emit(Instruction::rrr(O::FPow, rd, l_final, r_final));
            }
            // Comparisons (all produce int result regardless of operand types)
            (BinOp::Eq, false) => self.emit(Instruction::rrr(O::Eq, rd, l_final, r_final)),
            (BinOp::Ne, false) => self.emit(Instruction::rrr(O::Ne, rd, l_final, r_final)),
            (BinOp::Lt, false) => self.emit(Instruction::rrr(O::Lt, rd, l_final, r_final)),
            (BinOp::Gt, false) => self.emit(Instruction::rrr(O::Gt, rd, l_final, r_final)),
            (BinOp::Le, false) => self.emit(Instruction::rrr(O::Le, rd, l_final, r_final)),
            (BinOp::Ge, false) => self.emit(Instruction::rrr(O::Ge, rd, l_final, r_final)),
            (BinOp::Eq, true) => self.emit(Instruction::rrr(O::FEq, rd, l_final, r_final)),
            (BinOp::Ne, true) => self.emit(Instruction::rrr(O::FNe, rd, l_final, r_final)),
            (BinOp::Lt, true) => self.emit(Instruction::rrr(O::FLt, rd, l_final, r_final)),
            (BinOp::Gt, true) => self.emit(Instruction::rrr(O::FGt, rd, l_final, r_final)),
            (BinOp::Le, true) => self.emit(Instruction::rrr(O::FLe, rd, l_final, r_final)),
            (BinOp::Ge, true) => self.emit(Instruction::rrr(O::FGe, rd, l_final, r_final)),
            (BinOp::And, _) => {
                // Short-circuit AND: if left is falsy, return left; else evaluate and return right
                let check_r = if is_float {
                    let conv = self.alloc_int();
                    self.emit(Instruction::rr(O::F2I, conv, l_final));
                    conv
                } else {
                    l_final
                };
                // If left is falsy (jz), jump over the right-side evaluation
                let jz = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jz, 0, check_r, 0));
                self.emit(Instruction::rr(O::Mov, rd, r_final));
                let jmp = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jmp, 0, 0, 0));
                let after = self.current_offset();
                let jz_off = after - jz as u32 - 1;
                self.emit_at(jz, Instruction::rri(O::Jz, 0, check_r, jz_off));
                // Patch the JMP to skip over the right-side result
                let jmp_off = after - jmp as u32 - 1;
                self.emit_at(jmp, Instruction::rri(O::Jmp, 0, 0, jmp_off));
            }
            (BinOp::Or, _) => {
                // Short-circuit OR: if left is truthy, return left; else evaluate and return right
                let check_r = if is_float {
                    let conv = self.alloc_int();
                    self.emit(Instruction::rr(O::F2I, conv, l_final));
                    conv
                } else {
                    l_final
                };
                // If left is truthy (jnz), jump over the right-side evaluation
                let jnz = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jnz, 0, check_r, 0));
                self.emit(Instruction::rr(O::Mov, rd, r_final));
                let jmp = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jmp, 0, 0, 0));
                let after = self.current_offset();
                let jnz_off = after - jnz as u32 - 1;
                self.emit_at(jnz, Instruction::rri(O::Jnz, 0, check_r, jnz_off));
                let jmp_off = after - jmp as u32 - 1;
                self.emit_at(jmp, Instruction::rri(O::Jmp, 0, 0, jmp_off));
            }
            (BinOp::Concat, _) => {
                // String concatenation: just Mov left into result (stub)
                self.emit(Instruction::rr(O::Mov, rd, l_final));
            }
        }

        (rd, is_float)
    }

    // --- Section: Unary Expression Compilation ---

    /// Compile a unary expression (negation, logical not, length).
    /// Handles float/int dispatch and type conversion for `not`.
    fn compile_unary(&mut self, op: &UnaryOp, inner: &Expr) -> (u8, bool) {
        let (r, is_float) = self.compile_expr(inner);
        let rd = self.alloc_type(is_float);
        match (op, is_float) {
            (UnaryOp::Neg, false) => self.emit(Instruction::rr(O::Neg, rd, r)),
            (UnaryOp::Neg, true) => self.emit(Instruction::rr(O::FNeg, rd, r)),
            (UnaryOp::Not, _) => {
                // Convert float to int first if needed, then compare with 0
                let tmp = if is_float {
                    let t = self.alloc_int();
                    self.emit(Instruction::rr(O::F2I, t, r));
                    t
                } else {
                    r
                };
                self.emit(Instruction::rri(O::EqI, rd, tmp, 0));
            }
            (UnaryOp::Len, _) => {
                // Length operator: stub returning 0
                self.emit(Instruction::rri(O::Ldi, rd, 0, 0));
            }
        }
        (rd, is_float)
    }

    // --- Section: Function Call Compilation ---

    /// Compile a function call expression.
    /// Handles built-in quince.* functions (get, price, position, balance, etc.)
    /// and unknown function calls (errors).
    /// Special case: `quince.get("emaN")` is lowered to inline EMA opcode if fused.
    fn compile_fn_call(&mut self, name: &str, args: &[Expr]) -> (u8, bool) {
        match name {
            "quince.get" | "get" => {
                let r = self.alloc_float();
                // Check if the argument is a string literal naming a fused indicator
                let fused = args.first().and_then(|a| {
                    if let Expr::Literal(Literal::String(name)) = a {
                        self.fused_indicators.get(name).cloned()
                    } else {
                        None
                    }
                });
                if let Some(FusedInfo::Ema { state_id }) = fused {
                    // Inline the EMA computation: get current price, run EMA opcode
                    let val_r = self.alloc_float();
                    self.emit(Instruction::ri(O::GetPrice, val_r, 0));
                    self.emit(Instruction::rrr(O::Ema, r, val_r, state_id));
                    return (r, true);
                }
                // Non-fused indicator: delegate to runtime GetInd
                let arg_r = if args.is_empty() {
                    self.compile_literal(&Literal::String(String::new())).0
                } else {
                    self.compile_expr(&args[0]).0
                };
                self.emit(Instruction::rr(O::GetInd, r, arg_r));
                (r, true)
            }
            "quince.price" | "price" => {
                let r = self.alloc_float();
                self.emit(Instruction::ri(O::GetPrice, r, 0));
                (r, true)
            }
            "quince.position" | "position" => {
                let r = self.alloc_float();
                self.emit(Instruction::ri(O::GetPos, r, 0));
                (r, true)
            }
            "quince.balance" | "balance" => {
                let r = self.alloc_float();
                let name_r = if args.is_empty() {
                    self.compile_literal(&Literal::String("USDT".into())).0
                } else {
                    self.compile_expr(&args[0]).0
                };
                self.emit(Instruction::rr(O::GetBal, r, name_r));
                (r, true)
            }
            "quince.depth_bid" | "depth_bid" => {
                let r = self.alloc_float();
                let level_r = if let Some(arg) = args.first() {
                    self.compile_expr(arg).0
                } else {
                    let t = self.alloc_int();
                    self.emit(Instruction::rri(O::Ldi, t, 0, 0));
                    t
                };
                self.emit(Instruction::rrr(O::GetDepthBid, r, level_r, 0));
                (r, true)
            }
            "quince.depth_ask" | "depth_ask" => {
                let r = self.alloc_float();
                let level_r = if let Some(arg) = args.first() {
                    self.compile_expr(arg).0
                } else {
                    let t = self.alloc_int();
                    self.emit(Instruction::rri(O::Ldi, t, 0, 0));
                    t
                };
                self.emit(Instruction::rrr(O::GetDepthAsk, r, level_r, 0));
                (r, true)
            }
            "quince.log" | "log" => {
                let r = self.alloc_int();
                if let Some(arg) = args.first() {
                    let (arg_r, _) = self.compile_expr(arg);
                    if args.len() >= 2 {
                        // Two-argument log: (label, value) — uses Log2 opcode
                        let (val_r, is_float) = self.compile_expr(&args[1]);
                        let val_f = if is_float {
                            val_r
                        } else {
                            // Log2 requires float value, so convert if needed
                            let fr = self.alloc_float();
                            self.emit(Instruction::rr(O::I2F, fr, val_r));
                            fr
                        };
                        self.emit(Instruction::rrr(O::Log2, r, arg_r, val_f));
                    } else {
                        // Single-argument log — uses Log opcode
                        self.emit(Instruction::rr(O::Log, r, arg_r));
                    }
                }
                (r, false)
            }
            _ => {
                // Unknown function: emit error and default to 0
                let r = self.alloc_int();
                self.errors.push(crate::types::TypeError {
                    msg: format!("unknown function call: {}", name),
                });
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
        }
    }

    // --- Section: Method Call Compilation ---

    /// Compile a method call expression (e.g. `quince:get("name")`).
    /// Dispatches to the corresponding compile_fn_call or compile_send_order.
    fn compile_method_call(&mut self, obj: &str, method: &str, args: &[Expr]) -> (u8, bool) {
        match (obj, method) {
            ("quince", "get") => self.compile_fn_call("quince.get", args),
            ("quince", "price") => self.compile_fn_call("quince.price", &[]),
            ("quince", "position") => self.compile_fn_call("quince.position", &[]),
            ("quince", "balance") => self.compile_fn_call("quince.balance", args),
            ("quince", "order") => {
                self.compile_send_order(args);
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
            ("quince", "log2") => self.compile_fn_call("quince.log", args),
            ("quince", "depth_bid") => self.compile_fn_call("quince.depth_bid", args),
            ("quince", "depth_ask") => self.compile_fn_call("quince.depth_ask", args),
            ("quince", "log") => self.compile_fn_call("quince.log", args),
            _ => {
                self.errors.push(crate::types::TypeError {
                    msg: format!("unknown method call: {}:{}", obj, method),
                });
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
        }
    }

    // --- Section: Send Order Compilation (quince.order) ---

    /// Compile a `quince.order(side, qty, price, type?, reduce_only?)` call.
    /// Moves each argument into the designated send registers (REG_SEND_*),
    /// then emits the SendOrder opcode.
    fn compile_send_order(&mut self, args: &[Expr]) {
        // Arg 0: side (int) в†’ REG_SEND_SIDE
        if let Some(arg) = args.first() {
            let (r, is_float) = self.compile_expr(arg);
            if is_float {
                let tmp = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp, r));
                self.emit(Instruction::rr(O::Mov, REG_SEND_SIDE, tmp));
            } else {
                self.emit(Instruction::rr(O::Mov, REG_SEND_SIDE, r));
            }
        }
        // Arg 1: quantity (float) в†’ REG_SEND_QTY
        if let Some(arg) = args.get(1) {
            let (r, is_float) = self.compile_expr(arg);
            if is_float {
                self.emit(Instruction::rr(O::Mov, REG_SEND_QTY, r));
            } else {
                let tmp = self.alloc_float();
                self.emit(Instruction::rr(O::I2F, tmp, r));
                self.emit(Instruction::rr(O::Mov, REG_SEND_QTY, tmp));
            }
        }
        // Arg 2: price (float) в†’ REG_SEND_PRICE
        if let Some(arg) = args.get(2) {
            let (r, is_float) = self.compile_expr(arg);
            if is_float {
                self.emit(Instruction::rr(O::Mov, REG_SEND_PRICE, r));
            } else {
                let tmp = self.alloc_float();
                self.emit(Instruction::rr(O::I2F, tmp, r));
                self.emit(Instruction::rr(O::Mov, REG_SEND_PRICE, tmp));
            }
        }
        // Arg 3: order type (int, optional) в†’ REG_SEND_TYPE, defaults to 0
        if let Some(arg) = args.get(3) {
            let (r, is_float) = self.compile_expr(arg);
            if is_float {
                let tmp = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp, r));
                self.emit(Instruction::rr(O::Mov, REG_SEND_TYPE, tmp));
            } else {
                self.emit(Instruction::rr(O::Mov, REG_SEND_TYPE, r));
            }
        } else {
            self.emit(Instruction::rri(O::Ldi, REG_SEND_TYPE, 0, 0));
        }
        // Arg 4: reduce_only (int, optional) в†’ REG_SEND_REDUCE, defaults to 0
        if let Some(arg) = args.get(4) {
            let (r, is_float) = self.compile_expr(arg);
            if is_float {
                let tmp = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp, r));
                self.emit(Instruction::rr(O::Mov, REG_SEND_REDUCE, tmp));
            } else {
                self.emit(Instruction::rr(O::Mov, REG_SEND_REDUCE, r));
            }
        } else {
            self.emit(Instruction::rri(O::Ldi, REG_SEND_REDUCE, 0, 0));
        }
        self.emit(Instruction::single(O::SendOrder));
    }

    // --- Section: Field Access Compilation ---

    /// Compile a field access expression (e.g. `trade.price`, `fill.qty`, `depth.bids`).
    /// Resolves handler parameter fields to pre-loaded registers (r0-r4).
    /// Also handles `quince.xxx` dot-style access as builtins.
    fn compile_field_access(&mut self, obj: &Expr, field: &str) -> (u8, bool) {
        // If we're in a handler function (on_trade/on_fill/on_depth), check if the object
        // is the handler parameter — if so, map its fields to pre-loaded registers.
        if let Some(fname) = &self.current_fn {
            let is_trade_fill = matches!(
                (fname.as_str(), obj),
                ("on_trade" | "on_fill", Expr::Ident(n))
                    if self.handler_param.as_ref().is_some_and(|p| p == n)
            );
            let is_depth = matches!(
                (fname.as_str(), obj),
                ("on_depth", Expr::Ident(n))
                    if self.handler_param.as_ref().is_some_and(|p| p == n)
            );
            if is_trade_fill {
                // Map trade/fill fields to the pre-loaded registers:
                // r0=price(float), r1=qty(float), r2=side(int), r3=trade_id(int), r4=time(int)
                match field {
                    "price" => {
                        let r = self.alloc_float();
                        self.emit(Instruction::rr(O::Mov, r, 0));
                        return (r, true);
                    }
                    "qty" => {
                        let r = self.alloc_float();
                        self.emit(Instruction::rr(O::Mov, r, 1));
                        return (r, true);
                    }
                    "side" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 2));
                        return (r, false);
                    }
                    "trade_id" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 3));
                        return (r, false);
                    }
                    "time" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 4));
                        return (r, false);
                    }
                    _ => {}
                }
            }
            if is_depth {
                // Map depth fields: r2=bids(int), r3=asks(int), r0=price(float), r1=qty(float)
                match field {
                    "bids" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 2));
                        return (r, false);
                    }
                    "asks" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rr(O::Mov, r, 3));
                        return (r, false);
                    }
                    "price" => {
                        let r = self.alloc_float();
                        self.emit(Instruction::rr(O::Mov, r, 0));
                        return (r, true);
                    }
                    "qty" => {
                        let r = self.alloc_float();
                        self.emit(Instruction::rr(O::Mov, r, 1));
                        return (r, true);
                    }
                    // side, trade_id, time are not meaningful for depth — stub to 0
                    "side" | "trade_id" | "time" => {
                        let r = self.alloc_int();
                        self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                        return (r, false);
                    }
                    _ => {}
                }
            }
        }
        // Handle `quince.xxx` as builtin calls (dot notation)
        if let Expr::Ident(obj_name) = obj {
            if obj_name == "quince" {
                match field {
                    "get" => {
                        return self.compile_fn_call("quince.get", &[]);
                    }
                    "price" => return self.compile_fn_call("quince.price", &[]),
                    "position" => return self.compile_fn_call("quince.position", &[]),
                    "balance" => return self.compile_fn_call("quince.balance", &[]),
                    "log" => return self.compile_fn_call("quince.log", &[]),
                    "order" => {
                        self.emit(Instruction::single(O::SendOrder));
                        let r = self.alloc_int();
                        self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                        return (r, false);
                    }
                    _ => {}
                }
            }
        }

        // Fallback: stub (e.g. table field access not yet implemented)
        let r = self.alloc_int();
        self.emit(Instruction::rri(O::Ldi, r, 0, 0));
        (r, false)
    }

    // --- Section: Field Assignment Compilation (Stub) ---

    /// Compile assignment to a field (e.g. `obj.field = value`).
    /// Currently a stub — no bytecode emitted for non-trivial field targets.
    fn compile_quince_set(&mut self, obj: &Expr, field: &str, _val_reg: u8, _is_float: bool) {
        let _ = (obj, field);
    }

    // --- Section: Condition Compilation ---

    /// Compile an expression used as a condition, ensuring the result is
    /// in an integer register (converting float to int if needed).
    /// Returns the integer register holding the boolean result.
    fn compile_cond(&mut self, expr: &Expr) -> u8 {
        let (r, is_float) = self.compile_expr(expr);
        if is_float {
            let tmp = self.alloc_int();
            self.emit(Instruction::rr(O::F2I, tmp, r));
            tmp
        } else {
            r
        }
    }
}

// --- Section: Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::serialize;
    use crate::parser;

    /// Helper: parse and compile a source string into a QfrProgram.
    fn compile_str(input: &str) -> QfrProgram {
        let program = parser::parse(input).unwrap();
        compile(&program).unwrap()
    }

    // --- Basic compilation tests ---

    #[test]
    fn test_empty_program() {
        let prog = compile_str("");
        assert!(prog.entries.is_empty());
        assert!(prog.code.is_empty());
    }

    #[test]
    fn test_literal_i64() {
        let prog = compile_str("42");
        assert_eq!(prog.code.len(), 1);
        assert_eq!(prog.code[0].opcode(), O::Ldi);
    }

    #[test]
    fn test_literal_f64() {
        let prog = compile_str("2.5");
        assert!(!prog.code.is_empty());
        assert_eq!(prog.code[0].opcode(), O::LdcF64);
    }

    #[test]
    fn test_binary_add() {
        let prog = compile_str("1 + 2");
        // LDI r0, 1; LDI r1, 2; ADD r2, r0, r1
        assert_eq!(prog.code.len(), 3);
        assert_eq!(prog.code[2].opcode(), O::Add);
    }

    #[test]
    fn test_local_decl() {
        let prog = compile_str("local x = 42");
        assert_eq!(prog.code.len(), 1);
        assert_eq!(prog.code[0].opcode(), O::Ldi);
    }

    #[test]
    fn test_assign() {
        let prog = compile_str("x = 42");
        // LDI + MOV
        assert!(prog.code.len() >= 2);
        assert_eq!(prog.code.last().unwrap().opcode(), O::Mov);
    }

    #[test]
    fn test_fn_decl_entry() {
        let prog = compile_str("function on_trade(price, qty) end");
        assert_eq!(prog.entries.len(), 1);
        assert_eq!(prog.entries[0].name, "on_trade");
    }

    #[test]
    fn test_mov_reuse_same_var() {
        // Repeated reads of the same variable should reuse register (no Mov)
        let prog = compile_str(
            "
function on_eval()
    local a = 42
    b = a + 1
    c = a + 2
    d = a + 3
end
",
        );
        let mov_count = prog.code.iter().filter(|i| i.opcode() == O::Mov).count();
        // Without optimization: 6 Movs (3 var reads + 3 writes)
        // With optimization:   <=4 Movs (only writes, reads reuse register)
        assert!(
            mov_count <= 4,
            "expected <=4 Mov instructions with reuse, got {mov_count}"
        );
    }

    #[test]
    fn test_mov_reuse_shadowed_var() {
        // Shadowed var in inner scope must not cause stale cache hits
        let prog = compile_str(
            "
function on_eval()
    local a = 10
    if a > 0 then
        local a = 20
        b = a + 1
    end
    c = a + 1
end
",
        );
        let mov_count = prog.code.iter().filter(|i| i.opcode() == O::Mov).count();
        assert!(
            mov_count <= 5,
            "expected <=5 Mov with shadowed vars, got {mov_count}"
        );
    }

    #[test]
    fn test_if_stmt() {
        let prog = compile_str("if 1 then a = 42 else a = 0 end");
        // Should have JZ + JMP instructions
        let has_jz = prog.code.iter().any(|i| i.opcode() == O::Jz);
        let has_jmp = prog.code.iter().any(|i| i.opcode() == O::Jmp);
        assert!(has_jz, "if must emit JZ");
        assert!(has_jmp, "if must emit JMP");
    }

    #[test]
    fn test_while_loop() {
        let prog = compile_str("while 0 do end");
        let has_jz = prog.code.iter().any(|i| i.opcode() == O::Jz);
        assert!(has_jz);
    }

    #[test]
    fn test_repeat_loop() {
        let prog = compile_str("repeat until 1");
        let has_jz = prog.code.iter().any(|i| i.opcode() == O::Jz);
        assert!(has_jz);
    }

    #[test]
    fn test_for_num() {
        let prog = compile_str("for i = 1, 10 do end");
        let instructions = prog.code.iter().map(|i| i.opcode()).collect::<Vec<_>>();
        assert!(
            instructions.contains(&O::Add),
            "for must emit Add (i += step)"
        );
        assert!(
            instructions.contains(&O::Gt),
            "for must emit Gt (exit when i > to)"
        );
        assert!(
            instructions.contains(&O::Jnz),
            "for must emit Jnz (jump if exit condition met)"
        );
    }

    #[test]
    fn test_for_num_negative_step() {
        let prog = compile_str("for i = 10, 1, -1 do end");
        let instructions = prog.code.iter().map(|i| i.opcode()).collect::<Vec<_>>();
        assert!(
            instructions.contains(&O::Lt),
            "for negative step must emit Lt (exit when i < to)"
        );
        assert!(instructions.contains(&O::Jnz), "for must emit Jnz");
    }

    #[test]
    fn test_serialize_roundtrip() {
        let prog = compile_str("42");
        let bytes = serialize(&prog);
        assert!(bytes.len() > 32);
    }

    #[test]
    fn test_quince_get() {
        let prog = compile_str("quince.get(\"ema\")");
        let has_getind = prog.code.iter().any(|i| i.opcode() == O::GetInd);
        assert!(has_getind);
    }

    #[test]
    fn test_quince_price() {
        let prog = compile_str("quince.price()");
        let has_getprice = prog.code.iter().any(|i| i.opcode() == O::GetPrice);
        assert!(has_getprice);
    }

    #[test]
    fn test_unary_neg() {
        let prog = compile_str("-42");
        assert_eq!(prog.code.len(), 2);
        assert_eq!(prog.code[1].opcode(), O::Neg);
    }

    #[test]
    fn test_comparison() {
        let prog = compile_str("1 > 2");
        assert_eq!(prog.code.last().unwrap().opcode(), O::Gt);
    }

    #[test]
    fn test_float_add() {
        let prog = compile_str("1.0 + 2.0");
        let last = prog.code.last().unwrap().opcode();
        assert_eq!(last, O::FAdd);
    }

    #[test]
    fn test_mixed_add() {
        let prog = compile_str("1 + 2.0");
        // Should promote to float
        let ops: Vec<O> = prog.code.iter().map(|i| i.opcode()).collect();
        assert!(ops.contains(&O::I2F), "mixed add must promote int to float");
        assert!(ops.contains(&O::FAdd), "mixed add must use FAdd");
    }

    #[test]
    fn test_var_use() {
        let prog = compile_str("local x = 5 local y = x");
        // LDI r0, 5 only (no Mov — x's register reused directly)
        assert_eq!(prog.code.len(), 1);
        assert_eq!(prog.code[0].opcode(), O::Ldi);
    }

    #[test]
    fn test_persist_var() {
        let prog = compile_str("@persist local x = 10");
        // Should emit PersistGet (no PersistSet — value stays from previous run)
        let has_pget = prog.code.iter().any(|i| i.opcode() == O::PersistGet);
        assert!(has_pget);
    }

    #[test]
    fn test_multiple_statements() {
        let prog = compile_str("local a = 1 local b = 2 local c = a + b");
        // Ldi r0, 1; Ldi r1, 2; Add r2, r0, r1 (no Movs — regs reused directly)
        assert_eq!(prog.code.len(), 3);
        assert_eq!(prog.code[2].opcode(), O::Add);
    }

    #[test]
    fn test_quince_mutate() {
        // Test that calling sync doesn't crash
        let prog = compile_str("quince.log(\"hello\")");
        assert!(!prog.code.is_empty());
    }

    #[test]
    fn test_scale_expr() {
        // Test a realistic scalper expression
        let src = "local price = quince.price(); local ema = quince.get(\"ema\"); local result = price > ema";
        let prog = compile_str(src);
        assert!(prog.code.len() > 5);
    }

    #[test]
    fn test_trade_field_access() {
        // Test on_trade with trade.price syntax
        let src = "function on_trade(trade) local x = trade.price end";
        let prog = compile_str(src);
        assert_eq!(prog.entries.len(), 1);
    }

    #[test]
    fn test_return_stmt() {
        let prog = compile_str("function f() return 42 end");
        assert_eq!(prog.entries.len(), 1);
        assert_eq!(prog.code.last().unwrap().opcode(), O::Ret);
    }

    #[test]
    fn test_simple_test_log() {
        let src = "
@using sma:10:50

@persist tick_count : i64 = 0
@persist hellofrom : i64 = 0

on trade(t) {
    tick_count = tick_count + 1
    hellofrom = 22 * tick_count
    quince.log(\"\", t.qty)
}

on eval() {
    quince.log(\"Hello From Quince-flavored Lua\", tick_count)
}
";
        let mut prog = crate::compiler::compile_checked(&parser::parse(src).unwrap()).unwrap();
        eprintln!(
            "=== simple_test BEFORE optimize ({} instr) ===",
            prog.code.len()
        );
        for (i, instr) in prog.code.iter().enumerate() {
            eprintln!(
                "  {:3}: {:?} rd={} rs1={} rs2={} imm={}",
                i,
                instr.opcode(),
                instr.rd(),
                instr.rs1(),
                instr.rs2(),
                instr.imm()
            );
        }
        crate::optimize::optimize(&mut prog);
        eprintln!(
            "=== simple_test AFTER optimize ({} instr) ===",
            prog.code.len()
        );
        for (i, instr) in prog.code.iter().enumerate() {
            eprintln!(
                "  {:3}: {:?} rd={} rs1={} rs2={} imm={}",
                i,
                instr.opcode(),
                instr.rd(),
                instr.rs1(),
                instr.rs2(),
                instr.imm()
            );
        }
        for e in &prog.entries {
            eprintln!("entry: {} @{}", e.name, e.code_offset);
        }
        assert!(!prog.code.is_empty());
    }

    #[test]
    fn test_full_scalper() {
        let src = "
--USING ema 5
--USING bb 20 2.0
@persist position_size
@persist last_entry

function on_trade(trade)
    local price = trade.price
    local side = trade.side
    local mid = quince.get(\"bb.middle\")
    local upper = quince.get(\"bb.upper\")
    local lower = quince.get(\"bb.lower\")
    local ema = quince.get(\"ema\")

    if price < lower and ema > mid and position_size == 0 then
        quince.order(0, 1.0, 0)
        position_size = 1
        last_entry = price
        quince.log(\"long entry\")
    end

    if price > upper and ema < mid and position_size > 0 then
        quince.order(1, 1.0, 0)
        position_size = 0
        quince.log(\"exit\")
    end
end

function on_eval()
    local bal = quince.balance(\"USDT\")
    quince.log(\"eval\")
end
";
        let prog = compile_str(src);
        assert_eq!(prog.entries.len(), 2);
        assert!(
            prog.code.len() > 20,
            "scalper should produce >20 instructions, got {}",
            prog.code.len()
        );
    }

    #[test]
    fn test_ema_cross_strategy() {
        let src = "
--USING ema 9 50
--USING ema 5 20
@persist position_size
@persist last_entry

function on_trade(trade)
    local fast = quince.get(\"ema9\")
    local slow = quince.get(\"ema50\")

    if fast > slow and position_size <= 0 then
        quince.order(0, 1.0, 0)
        position_size = 1
        last_entry = trade.price
        quince.log(\"long entry\")
    end

    if fast < slow and position_size > 0 then
        quince.order(1, 1.0, 0)
        position_size = 0
        quince.log(\"exit\")
    end
end

function on_eval()
    local bal = quince.balance(\"USDT\")
    quince.log(\"eval\")
end
";
        let prog = compile_str(src);
        assert_eq!(prog.entries.len(), 2);
        assert!(
            prog.code.len() > 15,
            "ema_cross should produce >15 instructions, got {}",
            prog.code.len()
        );
    }

    #[test]
    fn test_test_all_strategy() {
        let src = "
--USING ema 5
--USING bb 20 2.0
@persist local call_count = 0

function on_trade(trade)
    call_count = call_count + 1
    local price = trade.price
    local side = trade.side
    local qty = trade.qty
    local id = trade.trade_id
    local t = trade.time

    local px = quince.price()
    local pos = quince.position()
    local ema = quince.get(\"ema\")

    if ema > 0 and call_count > 5 then
        quince.order(0, 0.5, 0)
        quince.log(\"buy signal\")
    end
end

function on_depth()
    quince.log(\"depth update\")
end

function on_fill(fill)
    local fill_price = fill.price
    local fill_qty = fill.qty
    quince.log(\"fill received\")
end

function on_eval()
    local bal = quince.balance(\"USDT\")
    local pos = quince.position()
    local price = quince.price()
    quince.log(\"eval\")
end
";
        let prog = compile_str(src);
        assert_eq!(prog.entries.len(), 4);
        assert!(
            prog.code.len() > 15,
            "test_all should produce >15 instructions, got {}",
            prog.code.len()
        );
    }

    // в”Ђв”Ђ Macro: strategy compilation tests в”Ђв”Ђ

    macro_rules! strategy_compiles {
        ($name:ident, $src:expr, $entries:expr, $min_instr:expr) => {
            #[test]
            fn $name() {
                let prog = compile_str($src);
                assert_eq!(prog.entries.len(), $entries);
                assert!(!prog.code.is_empty(), "strategy must emit code");
                assert!(
                    prog.code.len() >= $min_instr,
                    "strategy must emit >= {} instructions, got {}",
                    $min_instr,
                    prog.code.len()
                );
            }
        };
        ($name:ident, $src:expr, $entries:expr) => {
            strategy_compiles!($name, $src, $entries, 6);
        };
    }

    // в”Ђв”Ђ Strategy compilation tests в”Ђв”Ђ

    strategy_compiles!(
        test_sma_cross,
        "
@persist local position_size = 0
function on_trade(trade)
    local fast = quince.get(\"sma10\")
    local slow = quince.get(\"sma50\")
    if fast > slow and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if fast < slow and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
",
        2
    );

    strategy_compiles!(
        test_rsi_reversion,
        "
@persist local position_size = 0
function on_trade(trade)
    local rsi = quince.get(\"rsi\")
    if rsi < 30 and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if rsi > 70 and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
",
        2
    );

    strategy_compiles!(
        test_bb_bounce,
        "
@persist local position_size = 0
function on_trade(trade)
    local price = trade.price
    local mid = quince.get(\"bb.middle\")
    local upper = quince.get(\"bb.upper\")
    local lower = quince.get(\"bb.lower\")
    if price < lower and position_size <= 0 then quince.order(0, 0.5, 0) position_size = 1 end
    if price > mid and position_size > 0 then quince.order(1, 0.5, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
",
        2
    );

    strategy_compiles!(
        test_macd_cross,
        "
@persist local position_size = 0
function on_trade(trade)
    local macd = quince.get(\"macd.macd\")
    local signal = quince.get(\"macd.signal\")
    if macd > signal and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if macd < signal and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
",
        2
    );

    strategy_compiles!(test_atr_trail, "
@persist local position_size = 0
function on_trade(trade)
    local price = trade.price
    local atr = quince.get(\"atr\")
    if position_size > 0 and price < quince.get(\"atr\") then quince.order(1, 1.0, 0) position_size = 0 end
    if position_size <= 0 and atr > 0 then quince.order(0, 1.0, 0) position_size = 1 end
end
function on_eval() quince.log(\"eval\") end
", 2);

    strategy_compiles!(
        test_grid_trade,
        "
@persist local grid_level = 0
function on_trade(trade)
    local price = trade.price
    local ema = quince.get(\"ema\")
    local step = ema * 0.002
    if price - quince.get(\"ema\") > step then quince.order(1, 0.2, 0) end
    if price - quince.get(\"ema\") < -step then quince.order(0, 0.2, 0) end
end
function on_eval() quince.log(\"eval\") end
",
        2
    );

    strategy_compiles!(
        test_momentum,
        "
@persist local position_size = 0
function on_trade(trade)
    local roc = quince.get(\"roc\")
    if roc > 2 and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if roc < -2 and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
",
        2
    );

    strategy_compiles!(
        test_persist_multi,
        "
@persist local a = 0
@persist local b = 0
@persist local c = 0
function on_eval() a = a + 1 b = b + 2 c = c + 3 end
",
        1
    );

    strategy_compiles!(
        test_quince_chained,
        "
function on_trade(trade)
    local p = quince.price()
    local pos = quince.position()
    local bal = quince.balance(\"USDT\")
    quince.log(\"test\")
end
",
        1
    );

    strategy_compiles!(
        test_trade_fields,
        "
function on_trade(trade)
    local p = trade.price
    local q = trade.qty
    local s = trade.side
    local id = trade.trade_id
    local t = trade.time
end
",
        1,
        4
    ); // at least 4 instr (5 field accesses via Mov)

    // в”Ђв”Ђ Expression compilation tests в”Ђв”Ђ

    macro_rules! expr_compiles {
        ($name:ident, $src:expr, $opcode:path) => {
            #[test]
            fn $name() {
                let prog = compile_str($src);
                assert!(
                    prog.code.iter().any(|i| i.opcode() == $opcode),
                    "{} must emit {:?}",
                    $src,
                    $opcode
                );
            }
        };
    }

    expr_compiles!(test_bin_add, "1 + 2", O::Add);
    expr_compiles!(test_bin_sub, "5 - 3", O::Sub);
    expr_compiles!(test_bin_mul, "3 * 4", O::Mul);
    expr_compiles!(test_bin_div, "10 / 3", O::Div);
    expr_compiles!(test_bin_mod, "10 % 3", O::Mod);
    expr_compiles!(test_bin_pow, "2 ^ 3", O::Pow);
    expr_compiles!(test_bin_idiv, "10 // 3", O::Div);
    expr_compiles!(test_bin_eq, "1 == 2", O::Eq);
    expr_compiles!(test_bin_ne, "1 ~= 2", O::Ne);
    expr_compiles!(test_bin_lt, "1 < 2", O::Lt);
    expr_compiles!(test_bin_gt, "2 > 1", O::Gt);
    expr_compiles!(test_bin_le, "1 <= 2", O::Le);
    expr_compiles!(test_bin_ge, "2 >= 1", O::Ge);
    expr_compiles!(test_bin_and, "1 and 0", O::Jz);
    expr_compiles!(test_bin_or, "1 or 0", O::Jnz);
    expr_compiles!(test_bin_concat, "\"a\" .. \"b\"", O::Mov);
    expr_compiles!(test_bin_fadd, "1.0 + 2.0", O::FAdd);
    expr_compiles!(test_bin_fsub, "5.0 - 3.0", O::FSub);
    expr_compiles!(test_bin_fmul, "3.0 * 4.0", O::FMul);
    expr_compiles!(test_bin_fdiv, "10.0 / 3.0", O::FDiv);
    expr_compiles!(test_bin_feq, "1.0 == 2.0", O::FEq);
    expr_compiles!(test_bin_fne, "1.0 ~= 2.0", O::FNe);
    expr_compiles!(test_bin_flt, "1.0 < 2.0", O::FLt);
    expr_compiles!(test_bin_fgt, "2.0 > 1.0", O::FGt);
    expr_compiles!(test_bin_fle, "1.0 <= 2.0", O::FLe);
    expr_compiles!(test_bin_fge, "2.0 >= 1.0", O::FGe);
    expr_compiles!(test_expr_not, "not 1", O::EqI);
    expr_compiles!(test_expr_len, "#\"hello\"", O::Ldi);
    expr_compiles!(test_expr_fneg, "-1.5", O::FNeg);

    expr_compiles!(test_expr_get, "quince.get(\"x\")", O::GetInd);
    expr_compiles!(test_expr_price, "quince.price()", O::GetPrice);
    expr_compiles!(test_expr_pos, "quince.position()", O::GetPos);
    expr_compiles!(test_expr_bal, "quince.balance(\"USDT\")", O::GetBal);
    expr_compiles!(test_expr_log_str, "quince.log(\"msg\")", O::Log);
    expr_compiles!(test_expr_log_ident, "quince.log(\"x\")", O::Log);

    // в”Ђв”Ђ Edge-case tests в”Ђв”Ђ

    #[test]
    fn test_if_elseif_else() {
        let prog = compile_str("if 1 then elseif 2 then else end");
        assert!(prog.code.iter().any(|i| i.opcode() == O::Jz));
        assert!(prog.code.iter().any(|i| i.opcode() == O::Jmp));
    }

    #[test]
    fn test_nested_if() {
        let prog = compile_str("if 1 then if 2 then end end");
        let jz_count = prog.code.iter().filter(|i| i.opcode() == O::Jz).count();
        assert_eq!(jz_count, 2);
    }

    #[test]
    fn test_nested_loops() {
        let prog = compile_str("while 1 do for i = 1, 10 do end end");
        assert!(prog.code.len() > 5);
    }

    #[test]
    fn test_multi_return() {
        let prog = compile_str("function f() return 1, 2 end");
        assert_eq!(prog.code.last().unwrap().opcode(), O::Ret);
    }

    #[test]
    fn test_var_reassign() {
        let prog = compile_str("local x = 1 x = x + 1 x = x * 2");
        let ops: Vec<O> = prog.code.iter().map(|i| i.opcode()).collect();
        assert!(ops.contains(&O::Add));
        assert!(ops.contains(&O::Mul));
        assert!(ops.contains(&O::Mov));
    }

    #[test]
    fn test_while_body_skipped() {
        let prog = compile_str("while 0 do return 42 end");
        let has_ret = prog.code.iter().any(|i| i.opcode() == O::Ret);
        assert!(has_ret);
    }

    #[test]
    fn test_repeat_until_true() {
        let prog = compile_str("repeat until 1");
        assert!(prog.code.iter().any(|i| i.opcode() == O::Jz));
    }

    #[test]
    fn test_for_with_step() {
        let prog = compile_str("for i = 1, 10, 2 do end");
        let has_add = prog.code.iter().any(|i| i.opcode() == O::Add);
        assert!(has_add);
    }

    #[test]
    fn test_fn_call_order_arg_exprs() {
        let prog = compile_str("quince.order(0, 1.0, 0)");
        assert!(prog.code.iter().any(|i| i.opcode() == O::SendOrder));
    }

    #[test]
    fn test_fn_call_order_with_side_sell() {
        let prog = compile_str("quince.order(1, 0.5, 0)");
        assert!(prog.code.iter().any(|i| i.opcode() == O::SendOrder));
    }

    #[test]
    fn test_mixed_type_add() {
        let prog = compile_str("1 + 2.0");
        let has_i2f = prog.code.iter().any(|i| i.opcode() == O::I2F);
        let has_fadd = prog.code.iter().any(|i| i.opcode() == O::FAdd);
        assert!(has_i2f, "mixed add must emit I2F");
        assert!(has_fadd, "mixed add must emit FAdd");
    }

    // в”Ђв”Ђ Error path tests в”Ђв”Ђ

    #[test]
    fn test_parse_error_invalid_syntax() {
        let result = crate::parser::parse("function @@@ invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_unclosed_string() {
        let result = crate::parser::parse("local x = \"unclosed");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_unclosed_paren() {
        let result = crate::parser::parse("local x = (1 + 2");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_trailing_garbage() {
        // "42 garbage" is two expression statements: 42 and identifier garbage
        let result = crate::parser::parse("42 garbage");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_error_unexpected_eof() {
        let result = crate::parser::parse("function foo(");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_empty_fn_body() {
        let result = crate::parser::parse("function foo() end");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_error_invalid_number() {
        // "12zz" lexes as number + identifier, parses fine
        let result = crate::parser::parse("12zz");
        assert!(result.is_ok());
    }

    #[test]
    fn test_duplicate_persist_names() {
        // Two @persist with same name — compiler reuses slot, no crash
        let prog = compile_str("@persist local x = 1 @persist local x = 2");
        assert!(!prog.code.is_empty());
    }

    #[test]
    fn test_many_persist_vars() {
        // 64 persist vars should fit
        let mut src = String::new();
        for i in 0..64 {
            src.push_str(&format!("@persist local p{} = {}\n", i, i));
        }
        src.push_str("function on_eval() end");
        let prog = compile_str(&src);
        assert_eq!(prog.entries.len(), 1);
        assert!(prog.code.len() >= 64);
    }

    #[test]
    fn test_deep_nested_expression() {
        let mut src = String::new();
        src.push_str("local x = 0");
        for i in 0..80 {
            src.push_str(&format!(" + {}", i));
        }
        let prog = compile_str(&src);
        assert!(prog.code.len() > 80);
    }

    #[test]
    fn test_many_local_vars() {
        // Allocate many local variables
        let mut src = String::new();
        for i in 0..100 {
            src.push_str(&format!("local v{} = {}\n", i, i));
        }
        let prog = compile_str(&src);
        assert!(prog.code.len() >= 100);
    }

    #[test]
    fn test_large_source() {
        let mut src = String::with_capacity(10_000);
        src.push_str("function on_eval()\n");
        for i in 0..100 {
            src.push_str(&format!("local v{} = {}\n", i, i));
        }
        src.push_str("end\n");
        let prog = compile_str(&src);
        assert_eq!(prog.entries.len(), 1);
    }

    #[test]
    fn test_while_true_body() {
        let prog = compile_str("while 1 do return 42 end");
        assert!(prog.code.iter().any(|i| i.opcode() == O::Ret));
    }

    #[test]
    fn test_repeat_until_true_body() {
        let prog = compile_str("repeat local x = 1 until x == 1");
        assert!(prog.code.len() > 2);
    }

    #[test]
    fn test_for_with_negative_step() {
        let prog = compile_str("for i = 10, 1, -1 do end");
        let has_add = prog.code.iter().any(|i| i.opcode() == O::Add);
        assert!(has_add);
    }

    #[test]
    fn test_for_body_executed() {
        let prog = compile_str("for i = 1, 5 do local x = i end");
        assert!(prog.code.len() > 3);
    }

    #[test]
    fn test_nested_if_else() {
        let prog = compile_str("if 1 then if 2 then else end else end");
        let jz_count = prog.code.iter().filter(|i| i.opcode() == O::Jz).count();
        assert_eq!(jz_count, 2);
    }

    #[test]
    fn test_log_empty_string() {
        let prog = compile_str("quince.log(\"\")");
        assert!(prog.code.iter().any(|i| i.opcode() == O::Log));
    }

    #[test]
    fn test_order_without_limit() {
        let prog = compile_str("quince.order(0, 1.0)");
        assert!(prog.code.iter().any(|i| i.opcode() == O::SendOrder));
    }

    #[test]
    fn test_order_all_args() {
        let prog = compile_str("quince.order(0, 1.0, 0, 0, 0)");
        assert!(prog.code.iter().any(|i| i.opcode() == O::SendOrder));
    }

    #[test]
    fn test_indicator_many_using() {
        let mut src = String::new();
        for i in 0..10 {
            src.push_str(&format!("--USING ema {}\n", i + 1));
        }
        src.push_str("function on_eval() end");
        let prog = compile_str(&src);
        assert_eq!(prog.entries.len(), 1);
    }

    #[test]
    fn test_persist_float_init() {
        let prog = compile_str("@persist local x = 2.5 function on_eval() end");
        // Persist local with float init should compile without crash
        assert!(!prog.entries.is_empty());
    }

    #[test]
    fn test_func_param_names() {
        let prog = compile_str("function on_trade(a, b, c) local x = a + b + c end");
        assert_eq!(prog.entries.len(), 1);
        // 3 Add + 1 Ret, no Movs for param reads
        assert!(prog.code.len() >= 4);
    }

    #[test]
    fn test_chained_method_calls() {
        let prog = compile_str("quince.log(\"a\") quince.log(\"b\")");
        let log_count = prog.code.iter().filter(|i| i.opcode() == O::Log).count();
        assert_eq!(log_count, 2);
    }

    #[test]
    fn test_empty_while_body() {
        let prog = compile_str("while 0 do end");
        assert!(prog.code.len() >= 2);
    }

    #[test]
    fn test_repeat_with_complex_cond() {
        let prog = compile_str("repeat local x = x + 1 until x > 100");
        assert!(prog.code.len() > 3);
    }

    #[test]
    fn test_string_concat() {
        let prog = compile_str("\"hello\" .. \"world\"");
        // Concat typically compiles to Mov (string refs)
        assert!(prog.code.len() >= 2);
    }

    #[test]
    fn test_hex_literal() {
        let prog = compile_str("0xff");
        assert_eq!(prog.code[0].opcode(), O::Ldi);
    }

    // в”Ђв”Ђ compile_checked (type-checked compilation) tests в”Ђв”Ђ

    #[test]
    fn compile_checked_valid_program_ok() {
        let program = parser::parse("42").unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok());
    }

    #[test]
    fn compile_checked_type_error_returns_err() {
        let program = parser::parse("42 + true").unwrap();
        let result = compile_checked(&program);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| e.msg.contains("invalid operation")));
    }

    #[test]
    fn compile_checked_valid_strategy_ok() {
        let src = "
            function on_trade(trade)
                local p = trade.price
                local q = trade.qty
                if p > 0 then quince.order(0, q, p) end
            end
        ";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "strategy should type-check: {:?}",
            result.err()
        );
    }

    // в”Ђв”Ђ Phase 4g: feature/signal compilation tests в”Ђв”Ђ

    strategy_compiles!(
        test_feature_signal,
        "
feature f1 = 1.0 + 2.0
signal s1 = 1.0 > 0.5
function on_eval() quince.log(\"ok\") end
",
        1,
        4
    );

    strategy_compiles!(
        test_state_persist_simple,
        "
@persist x : f64 = 0.0
function on_trade(t)
    x = t
end
",
        1,
        3
    );

    strategy_compiles!(
        test_state_event_handler,
        "
@persist acc : f64 = 0.0
on eval() {
    quince.log(\"ok\")
}
",
        1,
        3
    );

    #[test]
    fn test_state_typed_compiles() {
        let src = "@persist price : f64 = 100.0\nfunction on_eval() quince.log(\"ok\") end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "state decl should type-check: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_fn_typed_compiles() {
        let src = "fn add(x: f64, y: f64) -> f64 { return x + y }\nfunction on_eval() quince.log(\"ok\") end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "fn decl should type-check: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_event_handler_compiles() {
        let src = "on eval() { quince.log(\"ok\") }\nfunction on_old() quince.log(\"done\") end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "event handler should type-check: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_state_persists_across_functions() {
        // state x used in two functions — each should emit PersistGet
        let src = "\
@persist x : f64 = 0.0
function on_trade(v)
    x = x + 1.0
end
function on_eval()
    quince.log(\"x.val\", x)
end
";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "state cross-fn should type-check: {:?}",
            result.err()
        );
        let prog = compile(&program).unwrap();
        // Should have 2 entry points
        assert!(
            prog.entries.len() >= 2,
            "should have at least 2 entries (on_trade + on_eval)"
        );
        // Should contain PersistGet opcode
        let has_persist_get = prog
            .code
            .iter()
            .any(|i| i.opcode() == crate::opcodes::Opcode::PersistGet);
        let has_persist_set = prog
            .code
            .iter()
            .any(|i| i.opcode() == crate::opcodes::Opcode::PersistSet);
        assert!(has_persist_get, "state must emit PersistGet");
        assert!(has_persist_set, "state x = x + 1.0 must emit PersistSet");
    }

    #[test]
    fn test_event_handler_type_check() {
        let src = "\
@persist acc : f64 = 0.0
on eval() {
    acc = acc + 1.0
}
function on_old() quince.log(\"ok\") end
";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "event handler should type-check: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_on_fill_field_access_compiles() {
        let src = "function on_fill(fill) local p = fill.price local q = fill.qty end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(
            result.is_ok(),
            "on_fill with field access: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_state_persist_used_in_event_handler() {
        let src = "\
@persist x : f64 = 0.0
on eval() {
    x = x + 1.0
}
";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "state in event handler: {:?}", result.err());
        let prog = compile(&program).unwrap();
        assert!(prog.code.iter().any(|i| i.opcode() == O::PersistGet));
        assert!(prog.code.iter().any(|i| i.opcode() == O::PersistSet));
    }

    #[test]
    fn test_expr_table_errors() {
        let program = parser::parse("local t = {}").unwrap();
        let result = compile_checked(&program);
        assert!(result.is_err(), "table literal should be rejected");
        assert!(result
            .unwrap_err()
            .iter()
            .any(|e| e.msg.contains("table literal")));
    }

    #[test]
    fn test_index_expr_errors() {
        let program = parser::parse("local t = {} local x = t[1]").unwrap();
        let result = compile_checked(&program);
        assert!(result.is_err(), "index expression should be rejected");
    }
}
