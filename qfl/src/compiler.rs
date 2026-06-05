use crate::ast::*;
use crate::ir::{self, QfrProgram};
use crate::opcodes::{Instruction, Opcode as O};
use std::collections::HashMap;

/// Compile a QFL Program into a QfrProgram (bytecode).
pub fn compile(program: &Program) -> QfrProgram {
    let mut c = Compiler::new();
    c.compile_program(program);
    c.prog
}

/// Type-check then compile.
/// Returns `Err(Vec<TypeError>)` if type checking fails.
pub fn compile_checked(program: &Program) -> Result<QfrProgram, Vec<crate::types::TypeError>> {
    crate::types::type_check(program)?;
    Ok(compile(program))
}

struct Compiler {
    prog: QfrProgram,
    symbols: Vec<Vec<(String, u8, bool)>>,
    next_int_reg: u8,
    next_float_reg: u8,
    persist_slots: Vec<(String, u8)>,
    next_persist_slot: u8,
    current_fn: Option<String>,
    state_types: HashMap<String, bool>, // name → is_float
    fused_indicators: HashMap<String, FusedInfo>, // name → fused indicator info
    next_ema_state: u8,
}

#[derive(Clone)]
enum FusedInfo {
    Ema { state_id: u8 },
}

impl Compiler {
    fn new() -> Self {
        Compiler {
            prog: QfrProgram::new(),
            symbols: vec![Vec::new()],
            next_int_reg: 0,
            next_float_reg: 192,
            persist_slots: Vec::new(),
            next_persist_slot: 0,
            current_fn: None,
            state_types: HashMap::new(),
            fused_indicators: HashMap::new(),
            next_ema_state: 0,
        }
    }

    fn compile_program(&mut self, program: &Program) {
        for stmt in program {
            self.compile_stmt(stmt);
        }
    }

    // --- Register allocation ---

    fn alloc_int(&mut self) -> u8 {
        let r = self.next_int_reg;
        self.next_int_reg += 1;
        r
    }

    fn alloc_float(&mut self) -> u8 {
        let r = self.next_float_reg;
        self.next_float_reg += 1;
        r
    }

    fn alloc_type(&mut self, is_float: bool) -> u8 {
        if is_float { self.alloc_float() } else { self.alloc_int() }
    }

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

    fn define_var(&mut self, name: &str, reg: u8, is_float: bool) {
        if let Some(scope) = self.symbols.last_mut() {
            scope.push((name.to_string(), reg, is_float));
        }
    }

    fn push_scope(&mut self) {
        self.symbols.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        self.symbols.pop();
    }

    fn persist_slot(&mut self, name: &str) -> u8 {
        for (n, slot) in &self.persist_slots {
            if n == name {
                return *slot;
            }
        }
        let slot = self.next_persist_slot;
        self.next_persist_slot += 1;
        self.persist_slots.push((name.to_string(), slot));
        slot
    }

    // --- Emit ---

    fn emit(&mut self, instr: Instruction) {
        self.prog.code.push(instr);
    }

    fn emit_at(&mut self, idx: usize, instr: Instruction) {
        self.prog.code[idx] = instr;
    }

    fn current_offset(&self) -> u32 {
        self.prog.code.len() as u32
    }

    fn register_entry(&mut self, name: &str, offset: u32) {
        self.prog.entries.push(crate::ir::EntryPoint {
            name: name.to_string(),
            code_offset: offset,
        });
    }

    // --- Statement compilation ---

    fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::VarDecl { names, init, is_local: _, persist } => {
                self.compile_var_decl(names, init.as_ref(), *persist);
            }
            Stmt::Assign { targets, exprs } => {
                self.compile_assign(targets, exprs);
            }
            Stmt::If { cond, then_body, elseif_branches, else_body } => {
                self.compile_if(cond, then_body, elseif_branches, else_body);
            }
            Stmt::While { cond, body } => {
                self.compile_while(cond, body);
            }
            Stmt::Repeat { body, until } => {
                self.compile_repeat(body, until);
            }
            Stmt::ForNum { var, from, to, step, body } => {
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
                for entry in indicators {
                    let name = entry.name.as_str();
                    for &period in &entry.params {
                        match name {
                            "ema" => {
                                let sid = self.next_ema_state;
                                self.next_ema_state += 1;
                                let alpha = 2.0 / (period + 1.0);
                                self.prog.ema_alphas.push(alpha);
                                let ind_name = format!("ema{}", period as i64);
                                self.fused_indicators.insert(ind_name, FusedInfo::Ema { state_id: sid });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Stmt::Window { .. } => {
                // setup directive — no bytecode emitted
            }
            Stmt::State { name, type_name, default: _ } => {
                // Allocate persist slot — compile_ident/compile_assign handle
                // PersistGet/PersistSet lazily on first use in any function.
                self.persist_slot(name);
                let is_float = crate::types::parse_state_type(type_name).is_float();
                self.state_types.insert(name.clone(), is_float);
            }
            Stmt::FnDecl { name, params, return_type: _, body } => {
                let offset = self.prog.code.len() as u32;
                self.prog.entries.push(ir::EntryPoint { name: name.clone(), code_offset: offset });
                self.current_fn = Some(name.clone());
                self.push_scope();
                for p in params {
                    let is_float = crate::types::parse_state_type(&p.type_name).is_float();
                    let pr = if is_float { self.alloc_float() } else { self.alloc_int() };
                    self.define_var(&p.name, pr, is_float);
                }
                for stmt in body {
                    self.compile_stmt(stmt);
                }
                self.emit(Instruction::single(O::Ret));
                self.pop_scope();
                self.current_fn = None;
            }
            Stmt::EventHandler { event, param, body } => {
                let fn_name = format!("on_{}", event);
                let offset = self.prog.code.len() as u32;
                self.prog.entries.push(ir::EntryPoint { name: fn_name.clone(), code_offset: offset });
                self.current_fn = Some(fn_name);
                self.push_scope();
                if let Some(p) = param {
                    let pr = self.alloc_int();
                    self.define_var(p, pr, false);
                }
                for stmt in body {
                    self.compile_stmt(stmt);
                }
                self.emit(Instruction::single(O::Ret));
                self.pop_scope();
                self.current_fn = None;
            }
            Stmt::Feature { name, expr } => {
                let (r, _) = self.compile_expr(expr);
                self.define_var(name, r, true);
            }
            Stmt::Signal { name, expr } => {
                let (r, _) = self.compile_expr(expr);
                self.define_var(name, r, false);
            }
        }
    }

    fn compile_var_decl(&mut self, names: &[String], init: Option<&Vec<Expr>>, persist: bool) {
        if persist {
            for (i, name) in names.iter().enumerate() {
                let slot = self.persist_slot(name);
                let is_float = init.map_or(false, |exprs| {
                    exprs.get(i).map_or(false, |e| matches!(e, Expr::Literal(Literal::F64(_))))
                });
                let r = if is_float { self.alloc_float() } else { self.alloc_int() };
                self.emit(Instruction::rri(O::PersistGet, r, 0, slot as u32));
                self.define_var(name, r, is_float);
                if is_float {
                    self.state_types.insert(name.clone(), true);
                }
            }
            // Note: init values are NOT applied on every call.
            // Persist variables carry their value across calls.
            // The init in source is documentation / default for a fresh state,
            // but we skip it here since persist slots default to 0.
            return;
        }

        if init.is_none() {
            // local x — declare but no init (default to 0/nil)
            for name in names {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                self.define_var(name, r, false);
            }
            return;
        }

        let exprs = init.unwrap();
        for (i, name) in names.iter().enumerate() {
            let (r, is_float) = if i < exprs.len() {
                self.compile_expr(&exprs[i])
            } else {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            };
            self.define_var(name, r, is_float);
        }
    }

    fn compile_assign(&mut self, targets: &[Expr], exprs: &[Expr]) {
        let rhs_regs: Vec<(u8, bool)> = exprs.iter().map(|e| self.compile_expr(e)).collect();

        for (i, target) in targets.iter().enumerate() {
            let (r, is_float) = rhs_regs.get(i).copied().unwrap_or((0, false));
            match target {
                Expr::Ident(name) => {
                    let vr = if let Some((vr, _)) = self.lookup_var(name) {
                        vr
                    } else {
                        let nr = self.alloc_type(is_float);
                        self.define_var(name, nr, is_float);
                        nr
                    };
                    self.emit(Instruction::rr(O::Mov, vr, r));
                    // Persist back if this is a persist variable
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
                    let _ = (obj, index);
                }
                _ => {}
            }
        }
    }

    fn compile_if(&mut self, cond: &Expr, then_body: &[Stmt],
                  elseif_branches: &[(Box<Expr>, Vec<Stmt>)], else_body: &[Stmt])
    {
        // Compile condition → get boolean result in int reg
        let cond_reg = self.compile_cond(cond);
        let jz_to_else = self.current_offset() as usize;
        self.emit(Instruction::rri(O::Jz, 0, cond_reg, 0)); // placeholder

        self.push_scope();
        for stmt in then_body {
            self.compile_stmt(stmt);
        }
        self.pop_scope();

        let jmp_to_end = if !else_body.is_empty() || !elseif_branches.is_empty() {
            let jmp = self.current_offset() as usize;
            self.emit(Instruction::rri(O::Jmp, 0, 0, 0)); // placeholder
            Some(jmp)
        } else {
            None
        };

        // Patch JZ: jump to else/elseif/end
        let after_then = self.current_offset();
        let jz_offset = after_then - jz_to_else as u32 - 1;
        self.emit_at(jz_to_else, Instruction::rri(O::Jz, 0, cond_reg, jz_offset));

        for (econd, ebody) in elseif_branches {
            let econd_reg = self.compile_cond(econd);
            let jz_to_elseif = self.current_offset() as usize;
            self.emit(Instruction::rri(O::Jz, 0, econd_reg, 0));

            self.push_scope();
            for stmt in ebody {
                self.compile_stmt(stmt);
            }
            self.pop_scope();

            let jmp = self.current_offset() as usize;
            self.emit(Instruction::rri(O::Jmp, 0, 0, 0));

            let after_ebody = self.current_offset();
            let jz_off = after_ebody - jz_to_elseif as u32 - 1;
            self.emit_at(jz_to_elseif, Instruction::rri(O::Jz, 0, econd_reg, jz_off));
            // chain jmps
            let last_jmp = jmp;
            let after = self.current_offset();
            let jmp_off = after - last_jmp as u32 - 1;
            self.emit_at(last_jmp, Instruction::rri(O::Jmp, 0, 0, jmp_off));
        }

        if !else_body.is_empty() {
            self.push_scope();
            for stmt in else_body {
                self.compile_stmt(stmt);
            }
            self.pop_scope();
        }

        if let Some(jmp) = jmp_to_end {
            let after_end = self.current_offset();
            let jmp_off = after_end - jmp as u32 - 1;
            self.emit_at(jmp, Instruction::rri(O::Jmp, 0, 0, jmp_off));
        }
    }

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) {
        let loop_start = self.current_offset();
        let cond_reg = self.compile_cond(cond);
        let jz_exit = self.current_offset() as usize;
        self.emit(Instruction::rri(O::Jz, 0, cond_reg, 0));

        self.push_scope();
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_scope();

        // Jump back to condition
        let jmp_back = self.current_offset();
        let back_offset = loop_start as i64 - jmp_back as i64 - 1;
        self.emit(Instruction::rri(O::Jmp, 0, 0, back_offset as u32));

        // Patch JZ
        let after_loop = self.current_offset();
        let jz_off = after_loop - jz_exit as u32 - 1;
        self.emit_at(jz_exit, Instruction::rri(O::Jz, 0, cond_reg, jz_off));
    }

    fn compile_repeat(&mut self, body: &[Stmt], until: &Expr) {
        let loop_start = self.current_offset();

        self.push_scope();
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_scope();

        let cond_reg = self.compile_cond(until);
        // JZ loops back (repeat until cond == true → loop while cond == false)
        let jz_back = self.current_offset();
        let back_offset = loop_start as i64 - jz_back as i64 - 1;
        self.emit(Instruction::rri(O::Jz, 0, cond_reg, back_offset as u32));
    }

    fn compile_for_num(&mut self, var: &str, from: &Expr, to: &Expr, step: &Option<Box<Expr>>, body: &[Stmt]) {
        let (from_r, _) = self.compile_expr(from);
        let (to_r, _) = self.compile_expr(to);
        let step_r = if let Some(s) = step {
            let (r, _) = self.compile_expr(s);
            r
        } else {
            let r = self.alloc_int();
            self.emit(Instruction::rri(O::Ldi, r, 0, 1));
            r
        };

        // for i = from, to, step → i = from; while i <= to: body; i += step
        let i_reg = self.alloc_int();
        self.emit(Instruction::rr(O::Mov, i_reg, from_r));
        self.define_var(var, i_reg, false);

        let loop_start = self.current_offset();
        // i <= to ? If not, break
        let cmp = self.alloc_int();
        self.emit(Instruction::rrr(O::Gt, cmp, i_reg, to_r));
        let jz_exit = self.current_offset() as usize;
        self.emit(Instruction::rri(O::Jz, 0, cmp, 0));

        self.push_scope();
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_scope();

        // i = i + step
        let tmp = self.alloc_int();
        self.emit(Instruction::rrr(O::Add, tmp, i_reg, step_r));
        self.emit(Instruction::rr(O::Mov, i_reg, tmp));

        // Jump back
        let jmp_back = self.current_offset();
        let back_offset = loop_start as i64 - jmp_back as i64 - 1;
        self.emit(Instruction::rri(O::Jmp, 0, 0, back_offset as u32));

        let after_loop = self.current_offset();
        let jz_off = after_loop - jz_exit as u32 - 1;
        self.emit_at(jz_exit, Instruction::rri(O::Jz, 0, cmp, jz_off));
    }

    fn compile_for_in(&mut self, vars: &[String], _exprs: &[Expr], body: &[Stmt]) {
        // Stub: for-in is complex (requires iterator protocol)
        // For now, just compile the body once
        self.push_scope();
        for v in vars {
            let r = self.alloc_int();
            self.emit(Instruction::rri(O::Ldi, r, 0, 0));
            self.define_var(v, r, false);
        }
        for stmt in body {
            self.compile_stmt(stmt);
        }
        self.pop_scope();
    }

    fn compile_fn_decl(&mut self, name: &str, params: &[String], body: &[Stmt]) {
        // Register entry point
        let offset = self.current_offset();
        self.register_entry(name, offset);

        // Pre-define function parameters as registers
        // Check for special entry point names to use trade convention
        let is_trade_entry = name == "on_trade";
        let is_fill_entry = name == "on_fill";
        let saved_fn = self.current_fn.replace(name.to_string());

        self.push_scope();
        for (i, param) in params.iter().enumerate() {
            if (is_trade_entry || is_fill_entry) && i < 5 {
                // Trade entry convention: r0-r4 pre-loaded by VM
                let r = i as u8;
                let is_float = i < 2; // price, qty are floats; side, id, time are ints
                // The register is already in the correct bank
                let actual_reg = if is_float { 192 + r } else { r };
                self.define_var(param, actual_reg, is_float);
            } else {
                let r = self.alloc_int();
                self.define_var(param, r, false);
            }
        }

        for stmt in body {
            self.compile_stmt(stmt);
        }
        // Emit Ret at end of function to prevent fall-through
        let last_op = self.prog.code.last().map(|i| i.opcode());
        if last_op != Some(O::Ret) {
            self.emit(Instruction::single(O::Ret));
        }
        self.pop_scope();

        self.current_fn = saved_fn;
    }

    fn compile_return(&mut self, exprs: &[Expr]) {
        if let Some(expr) = exprs.first() {
            let (_r, _) = self.compile_expr(expr);
        }
        self.emit(Instruction::single(O::Ret));
    }

    // --- Expression compilation ---

    /// Compile an expression, return (register, is_float).
    /// The caller owns the register (it's a temp).
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
                let _ = (obj, index);
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
            Expr::Table(_) => {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
        }
    }

    /// Compile expression but discard result (for expression statements)
    fn compile_expr_discard(&mut self, expr: &Expr) {
        // Check if it's a quince.order() call (side-effectful)
        if let Expr::FnCall { name, args } = expr {
            if name == "quince.order" || name == "order" {
                self.compile_send_order(args);
                return;
            }
        }
        if let Expr::MethodCall { obj, method, args } = expr {
            if method == "order" || method == "log" || method == "get" {
                self.compile_method_call(obj, method, args);
                return;
            }
        }
        self.compile_expr(expr);
    }

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
                self.emit(Instruction::rri(O::Ldc, r, 0, idx));
                (r, true)
            }
            Literal::String(s) => {
                let r = self.alloc_int();
                let idx = self.prog.intern_string(s);
                self.emit(Instruction::rri(O::Ldc, r, 0, idx));
                (r, false)
            }
        }
    }

    fn emit_ldi(&mut self, r: u8, val: i64) {
        if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
            self.emit(Instruction::rri(O::Ldi, r, 0, val as u32));
        } else if val >= -(1i64 << 39) && val < (1i64 << 39) {
            // Use ri40 for large immediates (fits in 40-bit signed)
            self.emit(Instruction::ri40(O::Ldi64, r, val));
        } else {
            // Fall back to constant pool — store as f64 (preserves i64 up to 2^53)
            let idx = self.prog.intern_f64(val as f64);
            self.emit(Instruction::rri(O::Ldc, r, 0, idx));
        }
    }

    fn compile_ident(&mut self, name: &str) -> (u8, bool) {
        // Check persist/state first — lazy PersistGet on first use
        for (pn, _) in &self.persist_slots {
            if pn == name {
                let is_float = self.state_types.get(name).copied().unwrap_or(false);
                let r = if is_float { self.alloc_float() } else { self.alloc_int() };
                let slot = self.persist_slot(name);
                self.emit(Instruction::rri(O::PersistGet, r, 0, slot as u32));
                return (r, is_float);
            }
        }
        // Trade convention builtins
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
                        self.emit(Instruction::rr(O::Mov, r, 193)); // r1 = qty
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
        if let Some((reg, is_float)) = self.lookup_var(name) {
            let r = self.alloc_type(is_float);
            self.emit(Instruction::rr(O::Mov, r, reg));
            (r, is_float)
        } else {
            let r = self.alloc_int();
            self.emit(Instruction::rri(O::Ldi, r, 0, 0));
            (r, false)
        }
    }

    fn compile_binary(&mut self, lhs: &Expr, op: &BinOp, rhs: &Expr) -> (u8, bool) {
        let (left_r, left_float) = self.compile_expr(lhs);
        let (right_r, right_float) = self.compile_expr(rhs);
        let is_float = left_float || right_float;

        // Promote if mixed types
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

        match (op, is_float) {
            (BinOp::Add, false) => self.emit(Instruction::rrr(O::Add, rd, l_final, r_final)),
            (BinOp::Sub, false) => self.emit(Instruction::rrr(O::Sub, rd, l_final, r_final)),
            (BinOp::Mul, false) => self.emit(Instruction::rrr(O::Mul, rd, l_final, r_final)),
            (BinOp::Div, false) => self.emit(Instruction::rrr(O::Div, rd, l_final, r_final)),
            (BinOp::IDiv, false) => self.emit(Instruction::rrr(O::Div, rd, l_final, r_final)),
            (BinOp::Mod, false) => self.emit(Instruction::rrr(O::Mod, rd, l_final, r_final)),
            (BinOp::Pow, false) => {
                // Stub: pow not implemented in VM
                self.emit(Instruction::rri(O::Ldi, rd, 0, 0));
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
                let tmp_i = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp_i, l_final));
                let tmp_i2 = self.alloc_int();
                self.emit(Instruction::rr(O::F2I, tmp_i2, r_final));
                let tmp = self.alloc_int();
                self.emit(Instruction::rrr(O::Mod, tmp, tmp_i, tmp_i2));
                self.emit(Instruction::rr(O::I2F, rd, tmp));
            }
            (BinOp::Pow, true) => {
                self.emit(Instruction::rri(O::Ldi, rd, 0, 0));
            }
            // Comparisons
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
                // a and b: if a is truthy, return b; else return a
                let check_r = if is_float {
                    let conv = self.alloc_int();
                    self.emit(Instruction::rr(O::F2I, conv, l_final));
                    conv
                } else {
                    l_final
                };
                let jz = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jz, 0, check_r, 0));
                self.emit(Instruction::rr(O::Mov, rd, r_final));
                let jmp = self.current_offset() as usize;
                self.emit(Instruction::rri(O::Jmp, 0, 0, 0));
                let after = self.current_offset();
                let jz_off = after - jz as u32 - 1;
                self.emit_at(jz, Instruction::rri(O::Jz, 0, check_r, jz_off));
                // patch jmp
                let jmp_off = after - jmp as u32 - 1;
                self.emit_at(jmp, Instruction::rri(O::Jmp, 0, 0, jmp_off));
                // Fall-through: rd already set from Mov
            }
            (BinOp::Or, _) => {
                let check_r = if is_float {
                    let conv = self.alloc_int();
                    self.emit(Instruction::rr(O::F2I, conv, l_final));
                    conv
                } else {
                    l_final
                };
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
                self.emit(Instruction::rr(O::Mov, rd, l_final));
            }
        }

        (rd, is_float)
    }

    fn compile_unary(&mut self, op: &UnaryOp, inner: &Expr) -> (u8, bool) {
        let (r, is_float) = self.compile_expr(inner);
        let rd = self.alloc_type(is_float);
        match (op, is_float) {
            (UnaryOp::Neg, false) => self.emit(Instruction::rr(O::Neg, rd, r)),
            (UnaryOp::Neg, true) => self.emit(Instruction::rr(O::FNeg, rd, r)),
            (UnaryOp::Not, _) => {
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
                self.emit(Instruction::rri(O::Ldi, rd, 0, 0));
            }
        }
        (rd, is_float)
    }

    fn compile_fn_call(&mut self, name: &str, args: &[Expr]) -> (u8, bool) {
        match name {
            "quince.get" | "get" => {
                let r = self.alloc_float();
                // Check if this is a fused indicator (string literal name)
                let fused = args.first().and_then(|a| {
                    if let Expr::Literal(Literal::String(name)) = a {
                        self.fused_indicators.get(name).cloned()
                    } else {
                        None
                    }
                });
                if let Some(FusedInfo::Ema { state_id }) = fused {
                    let val_r = self.alloc_float();
                    self.emit(Instruction::ri(O::GetPrice, val_r, 0));
                    self.emit(Instruction::rrr(O::Ema, val_r, state_id, r));
                    return (r, true);
                }
                // Fallback: runtime GetInd
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
            "quince.log" | "log" => {
                let r = self.alloc_int();
                if let Some(arg) = args.first() {
                    let (arg_r, _) = self.compile_expr(arg);
                    if args.len() >= 2 {
                        let (val_r, is_float) = self.compile_expr(&args[1]);
                        // Ensure value is in float register for Log2
                        let val_f = if is_float { val_r } else {
                            let fr = self.alloc_float();
                            self.emit(Instruction::rr(O::I2F, fr, val_r));
                            fr
                        };
                        self.emit(Instruction::rrr(O::Log2, r, arg_r, val_f));
                    } else {
                        self.emit(Instruction::rr(O::Log, r, arg_r));
                    }
                }
                (r, false)
            }
            _ => {
                // Regular function call — unknown, just return 0
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                (r, false)
            }
        }
    }

    fn compile_method_call(&mut self, obj: &str, method: &str, args: &[Expr]) -> (u8, bool) {
        // quince:get("name") → GetInd
        // quince:order(...) → SendOrder
        // quince:log(...) → Log
        match (obj, method) {
            ("quince", "get") => {
                return self.compile_fn_call("quince.get", args);
            }
            ("quince", "price") => {
                return self.compile_fn_call("quince.price", &[]);
            }
            ("quince", "position") => {
                return self.compile_fn_call("quince.position", &[]);
            }
            ("quince", "balance") => {
                return self.compile_fn_call("quince.balance", args);
            }
            ("quince", "order") => {
                self.compile_send_order(args);
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                return (r, false);
            }
            ("quince", "log") => {
                return self.compile_fn_call("quince.log", args);
            }
            _ => {
                let r = self.alloc_int();
                self.emit(Instruction::rri(O::Ldi, r, 0, 0));
                return (r, false);
            }
        }
    }

    fn compile_send_order(&mut self, args: &[Expr]) {
        // quince.order(side, qty, price, type?, reduce_only?)
        // Convention: r250=side (int), r192=qty (float), r193=price (float),
        //             r253=type (int), r254=reduce (int)
        if let Some(arg) = args.get(0) {
            let (r, _) = self.compile_expr(arg);
            self.emit(Instruction::rr(O::Mov, 250, r));
        }
        if let Some(arg) = args.get(1) {
            let (r, _) = self.compile_expr(arg);
            self.emit(Instruction::rr(O::Mov, 192, r));
        }
        if let Some(arg) = args.get(2) {
            let (r, _) = self.compile_expr(arg);
            self.emit(Instruction::rr(O::Mov, 193, r));
        }
        if let Some(arg) = args.get(3) {
            let (r, _) = self.compile_expr(arg);
            self.emit(Instruction::rr(O::Mov, 253, r));
        } else {
            self.emit(Instruction::rri(O::Ldi, 253, 0, 0));
        }
        if let Some(arg) = args.get(4) {
            let (r, _) = self.compile_expr(arg);
            self.emit(Instruction::rr(O::Mov, 254, r));
        } else {
            self.emit(Instruction::rri(O::Ldi, 254, 0, 0));
        }
        self.emit(Instruction::single(O::SendOrder));
    }

    fn compile_field_access(&mut self, obj: &Expr, field: &str) -> (u8, bool) {
        // Handle `trade.price` → GetPrice, `trade.qty` → r1, etc.
        if let Expr::Ident(obj_name) = obj {
            if let Some(fname) = &self.current_fn {
                let is_handler = (fname == "on_trade" && obj_name == "trade")
                    || (fname == "on_fill" && obj_name == "fill")
                    || (fname == "on_depth" && obj_name == "depth");
                if is_handler {
                    match field {
                        "price" => {
                            let r = self.alloc_float();
                            self.emit(Instruction::rr(O::Mov, r, 192)); // r0 from trade convention
                            return (r, true);
                        }
                        "qty" => {
                            let r = self.alloc_float();
                            self.emit(Instruction::rr(O::Mov, r, 193)); // r1
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
            // quince.xxx → builtin calls
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

        // For other field accesses, this might be a table lookup
        // For now, return 0 as stub
        let r = self.alloc_int();
        self.emit(Instruction::rri(O::Ldi, r, 0, 0));
        (r, false)
    }

    fn compile_quince_set(&mut self, obj: &Expr, field: &str, _val_reg: u8, _is_float: bool) {
        // Stub for quince.xxx = value assignments
        let _ = (obj, field);
    }

    /// Compile condition into a boolean int register
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::serialize;
    use crate::parser;

    fn compile_str(input: &str) -> QfrProgram {
        let program = parser::parse(input).unwrap();
        compile(&program)
    }

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
        let prog = compile_str("3.14");
        assert!(prog.code.len() >= 1);
        assert_eq!(prog.code[0].opcode(), O::Ldc);
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
    fn test_if_stmt() {
        let prog = compile_str("if 1 then return 42 else return 0 end");
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
        assert!(instructions.contains(&O::Add), "for must emit Add (i += step)");
        assert!(instructions.contains(&O::Gt), "for must emit Gt (i <= to)");
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
        // LDI r0, 5; MOV r1, r0
        assert_eq!(prog.code.len(), 2);
        assert_eq!(prog.code[1].opcode(), O::Mov);
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
        assert_eq!(prog.code.len(), 5);
        assert_eq!(prog.code[4].opcode(), O::Add);
    }

    #[test]
    fn test_quince_mutate() {
        // Test that calling sync doesn't crash
        let prog = compile_str("quince.log(\"hello\")");
        assert!(prog.code.len() >= 1);
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
        assert!(prog.code.len() > 20, "scalper should produce >20 instructions, got {}", prog.code.len());
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
        assert!(prog.code.len() > 15, "ema_cross should produce >15 instructions, got {}", prog.code.len());
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
        assert!(prog.code.len() > 15, "test_all should produce >15 instructions, got {}", prog.code.len());
    }

    // ── 10 strategy compilation tests ──

    macro_rules! strategy_compiles {
        ($name:ident, $src:expr, $entries:expr, $min_instr:expr) => {
            #[test]
            fn $name() {
                let prog = compile_str($src);
                assert_eq!(prog.entries.len(), $entries);
                assert!(!prog.code.is_empty(), "strategy must emit code");
                assert!(prog.code.len() >= $min_instr, "strategy must emit >= {} instructions, got {}", $min_instr, prog.code.len());
            }
        };
        ($name:ident, $src:expr, $entries:expr) => {
            strategy_compiles!($name, $src, $entries, 6);
        };
    }

    strategy_compiles!(test_sma_cross, "
@persist local position_size = 0
function on_trade(trade)
    local fast = quince.get(\"sma10\")
    local slow = quince.get(\"sma50\")
    if fast > slow and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if fast < slow and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
", 2);

    strategy_compiles!(test_rsi_reversion, "
@persist local position_size = 0
function on_trade(trade)
    local rsi = quince.get(\"rsi\")
    if rsi < 30 and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if rsi > 70 and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
", 2);

    strategy_compiles!(test_bb_bounce, "
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
", 2);

    strategy_compiles!(test_macd_cross, "
@persist local position_size = 0
function on_trade(trade)
    local macd = quince.get(\"macd.macd\")
    local signal = quince.get(\"macd.signal\")
    if macd > signal and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if macd < signal and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
", 2);

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

    strategy_compiles!(test_grid_trade, "
@persist local grid_level = 0
function on_trade(trade)
    local price = trade.price
    local ema = quince.get(\"ema\")
    local step = ema * 0.002
    if price - quince.get(\"ema\") > step then quince.order(1, 0.2, 0) end
    if price - quince.get(\"ema\") < -step then quince.order(0, 0.2, 0) end
end
function on_eval() quince.log(\"eval\") end
", 2);

    strategy_compiles!(test_momentum, "
@persist local position_size = 0
function on_trade(trade)
    local roc = quince.get(\"roc\")
    if roc > 2 and position_size <= 0 then quince.order(0, 1.0, 0) position_size = 1 end
    if roc < -2 and position_size > 0 then quince.order(1, 1.0, 0) position_size = 0 end
end
function on_eval() quince.log(\"eval\") end
", 2);

    strategy_compiles!(test_persist_multi, "
@persist local a = 0
@persist local b = 0
@persist local c = 0
function on_eval() a = a + 1 b = b + 2 c = c + 3 end
", 1);

    strategy_compiles!(test_quince_chained, "
function on_trade(trade)
    local p = quince.price()
    local pos = quince.position()
    local bal = quince.balance(\"USDT\")
    quince.log(\"test\")
end
", 1);

    strategy_compiles!(test_trade_fields, "
function on_trade(trade)
    local p = trade.price
    local q = trade.qty
    local s = trade.side
    local id = trade.trade_id
    local t = trade.time
end
", 1, 4); // at least 4 instr (5 field accesses via Mov)

    // ── Expression tests ──

    macro_rules! expr_compiles {
        ($name:ident, $src:expr, $opcode:path) => {
            #[test]
            fn $name() {
                let prog = compile_str($src);
                assert!(prog.code.iter().any(|i| i.opcode() == $opcode),
                    "{} must emit {:?}", $src, $opcode);
            }
        };
    }

    expr_compiles!(test_bin_add, "1 + 2", O::Add);
    expr_compiles!(test_bin_sub, "5 - 3", O::Sub);
    expr_compiles!(test_bin_mul, "3 * 4", O::Mul);
    expr_compiles!(test_bin_div, "10 / 3", O::Div);
    expr_compiles!(test_bin_mod, "10 % 3", O::Mod);
    expr_compiles!(test_bin_pow, "2 ^ 3", O::Ldi);
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
    // unary_neg, quince_get, quince_price already tested above
    expr_compiles!(test_expr_not, "not 1", O::EqI);
    expr_compiles!(test_expr_len, "#\"hello\"", O::Ldi);
    expr_compiles!(test_expr_fneg, "-1.5", O::FNeg);

    expr_compiles!(test_expr_get, "quince.get(\"x\")", O::GetInd);
    expr_compiles!(test_expr_price, "quince.price()", O::GetPrice);
    expr_compiles!(test_expr_pos, "quince.position()", O::GetPos);
    expr_compiles!(test_expr_bal, "quince.balance(\"USDT\")", O::GetBal);
    expr_compiles!(test_expr_log_str, "quince.log(\"msg\")", O::Log);
    expr_compiles!(test_expr_log_ident, "quince.log(\"x\")", O::Log);

    // ── Edge cases ──

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
        // Last instruction is Mov (assign result back to x)
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

    // ── Error path tests ──

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
        // Should be valid: empty fn body
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
        assert!(prog.code.len() > 0);
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
        // while true with body that halts
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
        let prog = compile_str("@persist local x = 3.14 function on_eval() end");
        // Persist local with float init should compile without crash
        assert!(prog.entries.len() >= 1);
    }

    #[test]
    fn test_func_param_names() {
        let prog = compile_str("function on_trade(a, b, c) local x = a + b + c end");
        assert_eq!(prog.entries.len(), 1);
        assert!(prog.code.len() > 4);
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

    // ── compile_checked (type-checked compilation) ──

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
        assert!(result.is_ok(), "strategy should type-check: {:?}", result.err());
    }

    // ── Phase 4g: feature/signal compilation ──

    strategy_compiles!(test_feature_signal, "
feature f1 = 1.0 + 2.0
signal s1 = 1.0 > 0.5
function on_eval() quince.log(\"ok\") end
", 1, 4);

    strategy_compiles!(test_state_persist_simple, "
state x : f64 = 0.0
function on_trade(t)
    x = t
end
", 1, 4);

    strategy_compiles!(test_state_event_handler, "
state acc : f64 = 0.0
on eval() {
    quince.log(\"ok\")
}
", 1, 3);

    #[test]
    fn test_state_typed_compiles() {
        let src = "state price : f64 = 100.0\nfunction on_eval() quince.log(\"ok\") end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "state decl should type-check: {:?}", result.err());
    }

    #[test]
    fn test_fn_typed_compiles() {
        let src = "fn add(x: f64, y: f64) -> f64 { return x + y }\nfunction on_eval() quince.log(\"ok\") end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "fn decl should type-check: {:?}", result.err());
    }

    #[test]
    fn test_event_handler_compiles() {
        let src = "on eval() { quince.log(\"ok\") }\nfunction on_eval_old() quince.log(\"done\") end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "event handler should type-check: {:?}", result.err());
    }

    #[test]
    fn test_state_persists_across_functions() {
        // state x used in two functions — each should emit PersistGet
        let src = "\
state x : f64 = 0.0
function on_trade(v)
    x = x + 1.0
end
function on_eval()
    quince.log(\"x.val\", x)
end
";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "state cross-fn should type-check: {:?}", result.err());
        let prog = compile(&program);
        // Should have 2 entry points
        assert!(prog.entries.len() >= 2, "should have at least 2 entries (on_trade + on_eval)");
        // Should contain PersistGet opcode (54)
        let has_persist_get = prog.code.iter().any(|i| i.opcode() == crate::opcodes::Opcode::PersistGet);
        let has_persist_set = prog.code.iter().any(|i| i.opcode() == crate::opcodes::Opcode::PersistSet);
        assert!(has_persist_get, "state must emit PersistGet");
        assert!(has_persist_set, "state x = x + 1.0 must emit PersistSet");
    }

    #[test]
    fn test_event_handler_type_check() {
        let src = "\
state acc : f64 = 0.0
on eval() {
    acc = acc + 1.0
}
function on_eval_old() quince.log(\"ok\") end
";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "event handler should type-check: {:?}", result.err());
    }

    #[test]
    fn test_on_fill_field_access_compiles() {
        let src = "function on_fill(fill) local p = fill.price local q = fill.qty end";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "on_fill with field access: {:?}", result.err());
    }

    #[test]
    fn test_state_persist_used_in_event_handler() {
        let src = "\
state x : f64 = 0.0
on eval() {
    x = x + 1.0
}
";
        let program = parser::parse(src).unwrap();
        let result = compile_checked(&program);
        assert!(result.is_ok(), "state in event handler: {:?}", result.err());
        let prog = compile(&program);
        assert!(prog.code.iter().any(|i| i.opcode() == O::PersistGet));
        assert!(prog.code.iter().any(|i| i.opcode() == O::PersistSet));
    }

    #[test]
    fn test_expr_table_compiles() {
        let prog = compile_str("local t = {}");
        assert!(prog.code.len() >= 1);
    }

    #[test]
    fn test_index_expr_compiles() {
        let prog = compile_str("local t = {} local x = t[1]");
        assert!(prog.code.len() >= 2);
    }
}
