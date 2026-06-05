/// QFL type system — strong domain-specific types for trading.
///
/// Rules:
/// - Numeric types (I64, F64, Price, Qty, Timestamp, Duration) support
///   arithmetic within their group and with direct promotion rules.
/// - Domain types (Symbol, Side, OrderId, Bool) are NOT numeric and
///   do NOT support arithmetic.
/// - Price + Price → Price (valid)
/// - Price + Duration → Price (valid — offset in time)
/// - Price * Qty → Price (valid — notional)
/// - Price + Side → TypeError (invalid)

use std::fmt;

/// Strongly-typed domain value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl QflType {
    pub fn is_numeric(self) -> bool {
        matches!(self, QflType::I64 | QflType::F64 | QflType::Price
            | QflType::Qty | QflType::Timestamp | QflType::Duration)
    }

    pub fn is_float(self) -> bool {
        matches!(self, QflType::F64 | QflType::Price | QflType::Qty)
    }

    pub fn is_integer(self) -> bool {
        matches!(self, QflType::I64 | QflType::Timestamp | QflType::Duration
            | QflType::Side | QflType::OrderId)
    }
}

impl fmt::Display for QflType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QflType::I64 => write!(f, "i64"),
            QflType::F64 => write!(f, "f64"),
            QflType::Bool => write!(f, "bool"),
            QflType::Timestamp => write!(f, "timestamp"),
            QflType::Duration => write!(f, "duration"),
            QflType::Price => write!(f, "price"),
            QflType::Qty => write!(f, "qty"),
            QflType::Symbol => write!(f, "symbol"),
            QflType::Side => write!(f, "side"),
            QflType::OrderId => write!(f, "order_id"),
        }
    }
}

/// Parse a state declaration type string to QflType.
/// e.g. "f64" → QflType::F64, "qty" → QflType::Qty, "i32" → QflType::I64
pub fn parse_state_type(s: &str) -> QflType {
    match s {
        "f64" => QflType::F64,
        "i32" | "i64" => QflType::I64,
        "price" => QflType::Price,
        "qty" => QflType::Qty,
        "side" => QflType::Side,
        "bool" => QflType::Bool,
        "timestamp" => QflType::Timestamp,
        "duration" => QflType::Duration,
        "symbol" => QflType::Symbol,
        "order_id" => QflType::OrderId,
        _ => QflType::I64, // default
    }
}

/// A type error with a message.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeError {
    pub msg: String,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypeError: {}", self.msg)
    }
}

// ── Arithmetic type rules ──

/// Result type for binary operations, or a TypeError.
pub type TypeResult = Result<QflType, TypeError>;

/// Determine the result type for `lhs op rhs`.
/// Returns `Err(TypeError)` if the operation is invalid.
pub fn bin_op_type(lhs: QflType, op: &crate::ast::BinOp, rhs: QflType) -> TypeResult {
    use crate::ast::BinOp;
    use QflType::*;

    // Comparison ops always return Bool
    match op {
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            if lhs == rhs || (lhs.is_numeric() && rhs.is_numeric()) {
                return Ok(Bool);
            }
            return Err(TypeError {
                msg: format!("cannot compare {} with {}", lhs, rhs),
            });
        }
        BinOp::And | BinOp::Or => {
            if lhs == Bool && rhs == Bool {
                return Ok(Bool);
            }
            return Err(TypeError {
                msg: format!("logical ops require bool, got {} and {}", lhs, rhs),
            });
        }
        _ => {}
    }

    // Arithmetic ops (Add, Sub, Mul, Div, IDiv, Mod, Pow, Concat)
    match (lhs, rhs) {
        // Same type → same type (numeric)
        (a, b) if a == b && a.is_numeric() => Ok(a),

        // Price + Duration → Price (time offset)
        (Price, Duration) | (Duration, Price) => Ok(Price),

        // Price * Qty → Price (notional)
        (Price, Qty) | (Qty, Price) if matches!(op, BinOp::Mul) => Ok(Price),

        // Promotion: I64 + F64/Price/Qty → F64/Price/Qty
        (I64, F64) | (F64, I64) => Ok(F64),
        (I64, Price) | (Price, I64) => Ok(Price),
        (I64, Qty) | (Qty, I64) => Ok(Qty),
        (I64, Duration) | (Duration, I64) => Ok(Duration),
        (I64, Timestamp) | (Timestamp, I64) => Ok(Timestamp),

        // Concat requires Symbol or String-like types
        (Symbol, Symbol) if matches!(op, BinOp::Concat) => Ok(Symbol),
        _ => Err(TypeError {
            msg: format!("invalid operation {} {} {}", lhs, op, rhs),
        }),
    }
}

/// Determine the result type for `op expr`.
pub fn unary_op_type(op: &crate::ast::UnaryOp, expr: QflType) -> TypeResult {
    use crate::ast::UnaryOp;
    use QflType::*;

    match op {
        UnaryOp::Neg => {
            if expr.is_numeric() {
                Ok(expr)
            } else {
                Err(TypeError {
                    msg: format!("cannot negate {}", expr),
                })
            }
        }
        UnaryOp::Not => {
            if expr == Bool {
                Ok(Bool)
            } else {
                Err(TypeError {
                    msg: format!("not requires bool, got {}", expr),
                })
            }
        }
        UnaryOp::Len => {
            // Length works on symbols (strings)
            if expr == Symbol {
                Ok(I64)
            } else {
                Err(TypeError {
                    msg: format!("len requires symbol, got {}", expr),
                })
            }
        }
    }
}

/// Infer the type of an AST literal.
pub fn literal_type(lit: &crate::ast::Literal) -> QflType {
    match lit {
        crate::ast::Literal::Nil => QflType::I64,
        crate::ast::Literal::Bool(_) => QflType::Bool,
        crate::ast::Literal::I64(_) => QflType::I64,
        crate::ast::Literal::F64(_) => QflType::F64,
        crate::ast::Literal::String(_) => QflType::Symbol,
    }
}

// ── Program-level type checker ──

fn is_side_compat(t: QflType) -> bool {
    t == QflType::Side || t == QflType::I64
}

fn is_qty_compat(t: QflType) -> bool {
    t == QflType::Qty || t == QflType::F64 || t == QflType::I64 || t == QflType::Price
}

struct TypeChecker {
    scopes: Vec<Scope>,
    errors: Vec<TypeError>,
}

struct Scope {
    vars: Vec<(String, QflType)>,
}

impl TypeChecker {
    fn new() -> Self {
        TypeChecker { scopes: vec![Scope { vars: Vec::new() }], errors: Vec::new() }
    }

    fn lookup(&self, name: &str) -> Option<QflType> {
        for scope in self.scopes.iter().rev() {
            for (n, t) in scope.vars.iter().rev() {
                if n == name { return Some(*t); }
            }
        }
        None
    }

    fn define(&mut self, name: &str, typ: QflType) {
        if let Some(scope) = self.scopes.last_mut() {
            // Replace if already exists in current scope
            for (n, t) in scope.vars.iter_mut() {
                if n == name { *t = typ; return; }
            }
            scope.vars.push((name.to_string(), typ));
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope { vars: Vec::new() });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(TypeError { msg: msg.into() });
    }

    fn check_program(&mut self, program: &crate::ast::Program) {
        for stmt in program {
            self.check_stmt(stmt);
        }
    }

    fn check_stmt(&mut self, stmt: &crate::ast::Stmt) {
        use crate::ast::Stmt::*;
        match stmt {
            VarDecl { names, init, persist: _, is_local: _ } => {
                if let Some(exprs) = init {
                    for (i, name) in names.iter().enumerate() {
                        let typ = if i < exprs.len() {
                            self.infer_expr(&exprs[i])
                        } else {
                            QflType::I64
                        };
                        self.define(name, typ);
                    }
                } else {
                    for name in names {
                        self.define(name, QflType::I64);
                    }
                }
            }
            Assign { targets, exprs } => {
                for expr in exprs {
                    self.infer_expr(expr);
                }
                for (i, target) in targets.iter().enumerate() {
                    match target {
                        crate::ast::Expr::Ident(name) => {
                            let rhs_type = if i < exprs.len() {
                                self.infer_expr(&exprs[i])
                            } else {
                                continue;
                            };
                            if let Some(var_type) = self.lookup(name) {
                                let compatible = var_type == rhs_type
                                    || (var_type.is_float() && rhs_type.is_float());
                                if !compatible {
                                    self.error(format!("cannot assign {} to {} variable '{}'",
                                        rhs_type, var_type, name));
                                }
                            }
                            // Re-declare if not found
                            self.define(name, rhs_type);
                        }
                        _ => { self.infer_expr(target); }
                    }
                }
            }
            If { cond, then_body, elseif_branches, else_body } => {
                let cond_type = self.infer_expr(cond);
                if cond_type != QflType::Bool {
                    self.error(format!("if condition must be bool, got {}", cond_type));
                }
                self.push_scope();
                for s in then_body { self.check_stmt(s); }
                self.pop_scope();
                for (econd, ebody) in elseif_branches {
                    let ect = self.infer_expr(econd);
                    if ect != QflType::Bool {
                        self.error(format!("elseif condition must be bool, got {}", ect));
                    }
                    self.push_scope();
                    for s in ebody { self.check_stmt(s); }
                    self.pop_scope();
                }
                self.push_scope();
                for s in else_body { self.check_stmt(s); }
                self.pop_scope();
            }
            While { cond, body } => {
                let ct = self.infer_expr(cond);
                if ct != QflType::Bool {
                    self.error(format!("while condition must be bool, got {}", ct));
                }
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            Repeat { body, until } => {
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
                let ut = self.infer_expr(until);
                if ut != QflType::Bool {
                    self.error(format!("repeat condition must be bool, got {}", ut));
                }
            }
            ForNum { var, from, to, step, body } => {
                let ft = self.infer_expr(from);
                if ft != QflType::I64 {
                    self.error(format!("for range start must be i64, got {}", ft));
                }
                let tt = self.infer_expr(to);
                if tt != QflType::I64 {
                    self.error(format!("for range end must be i64, got {}", tt));
                }
                if let Some(s) = step {
                    let st = self.infer_expr(s);
                    if st != QflType::I64 {
                        self.error(format!("for step must be i64, got {}", st));
                    }
                }
                self.define(var, QflType::I64);
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            ForIn { vars: _, exprs, body } => {
                for e in exprs { self.infer_expr(e); }
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            FunctionDecl { name, params, body } => {
                self.push_scope();
                self.define_trade_params(name, params);
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            Return { exprs } => {
                for e in exprs { self.infer_expr(e); }
            }
            ExprStmt(expr) => { self.infer_expr(expr); }
            Using { .. } => { /* setup directive — no codegen */ }
            Window { .. } => { /* setup directive — no codegen */ }
            State { name, type_name, default } => {
                let st = parse_state_type(type_name);
                if let Some(expr) = default {
                    let et = self.infer_expr(expr);
                    if et != st {
                        self.error(format!("state '{}' declared as {}, default expr is {}", name, st, et));
                    }
                }
                self.define(name, st);
            }
            FnDecl { name: _, params, return_type: _, body } => {
                self.push_scope();
                for p in params {
                    let pt = parse_state_type(&p.type_name);
                    self.define(&p.name, pt);
                }
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            EventHandler { event, param, body } => {
                self.push_scope();
                if let Some(p) = param {
                    let param_type = match event.as_str() {
                        "trade" => QflType::Symbol,
                        "depth" => QflType::Symbol,
                        "fill" => QflType::Symbol,
                        "eval" => QflType::I64,
                        "timer" => QflType::Duration,
                        "pnl_update" => QflType::F64,
                        _ => QflType::I64,
                    };
                    self.define(p, param_type);
                }
                for s in body { self.check_stmt(s); }
                self.pop_scope();
            }
            Feature { name, expr } => {
                let _typ = self.infer_expr(expr);
                self.define(name, QflType::F64);
            }
            Signal { name, expr } => {
                let _typ = self.infer_expr(expr);
                self.define(name, QflType::Bool);
            }
        }
    }

    fn define_trade_params(&mut self, fn_name: &str, params: &[String]) {
        let is_trade = fn_name == "on_trade";
        for (i, param) in params.iter().enumerate() {
            let typ = if is_trade {
                match i {
                    0 => QflType::Price,   // trade.price
                    1 => QflType::Qty,     // trade.qty
                    2 => QflType::Side,    // trade.side
                    3 => QflType::I64,     // trade.id
                    4 => QflType::Timestamp, // trade.time
                    _ => QflType::I64,
                }
            } else {
                QflType::I64
            };
            self.define(param, typ);
        }
        // For on_trade and on_fill, make the object available for field access
        if let Some(param) = params.first() {
            if fn_name == "on_trade" || fn_name == "on_fill" || fn_name == "on_depth" {
                self.define(param, QflType::Symbol);
            }
        }
    }

    fn infer_expr(&mut self, expr: &crate::ast::Expr) -> QflType {
        use crate::ast::Expr::*;
        match expr {
            Literal(lit) => literal_type(lit),
            Ident(name) => {
                self.lookup(name).unwrap_or_else(|| {
                    // Check persist variables (defined later in compilation)
                    QflType::I64
                })
            }
            FnCall { name, args } => self.infer_fn_call(name, args),
            MethodCall { obj, method, args } => self.infer_method_call(obj, method, args),
            FieldAccess { obj, field } => self.infer_field_access(obj, field),
            Index { obj, index } => {
                self.infer_expr(obj);
                self.infer_expr(index);
                QflType::I64
            }
            Unary { op, expr } => {
                let inner = self.infer_expr(expr);
                match unary_op_type(op, inner) {
                    Ok(t) => t,
                    Err(e) => { self.error(e.msg); QflType::I64 }
                }
            }
            Binary { lhs, op, rhs } => {
                let l = self.infer_expr(lhs);
                let r = self.infer_expr(rhs);
                match bin_op_type(l, op, r) {
                    Ok(t) => t,
                    Err(e) => { self.error(e.msg); QflType::I64 }
                }
            }
            Table(_) => QflType::I64,
        }
    }

    fn infer_fn_call(&mut self, name: &str, args: &[crate::ast::Expr]) -> QflType {
        // Check arg types
        let arg_types: Vec<QflType> = args.iter().map(|a| self.infer_expr(a)).collect();

        match name {
            "quince.get" | "get" => {
                if arg_types.len() >= 1 && arg_types[0] != QflType::Symbol {
                    self.error(format!("quince.get() arg must be symbol, got {}", arg_types[0]));
                }
                QflType::F64
            }
            "quince.price" | "price" => QflType::Price,
            "quince.position" | "position" => QflType::Qty,
            "quince.balance" | "balance" => {
                if arg_types.len() >= 1 && arg_types[0] != QflType::Symbol {
                    self.error(format!("quince.balance() arg must be symbol, got {}", arg_types[0]));
                }
                QflType::F64
            }
            "quince.order" | "order" => {
                if arg_types.len() >= 1 && !is_side_compat(arg_types[0]) {
                    self.error(format!("quince.order() side must be side or i64, got {}", arg_types[0]));
                }
                if arg_types.len() >= 2 && !is_qty_compat(arg_types[1]) {
                    self.error(format!("quince.order() qty must be qty or numeric, got {}", arg_types[1]));
                }
                QflType::I64
            }
            "quince.log" | "log" => {
                if let Some(arg_type) = arg_types.first() {
                    if *arg_type != QflType::Symbol {
                        self.error(format!("quince.log() first arg must be symbol, got {}", arg_type));
                    }
                }
                QflType::I64
            }
            _ => QflType::I64,
        }
    }

    fn infer_method_call(&mut self, obj: &str, method: &str, args: &[crate::ast::Expr]) -> QflType {
        let arg_types: Vec<QflType> = args.iter().map(|a| self.infer_expr(a)).collect();

        if obj == "quince" {
            match method {
                "get" => {
                    if arg_types.len() >= 1 && arg_types[0] != QflType::Symbol {
                        self.error(format!("quince:get() arg must be symbol, got {}", arg_types[0]));
                    }
                    QflType::F64
                }
                "price" => QflType::Price,
                "position" => QflType::Qty,
                "balance" => {
                    if arg_types.len() >= 1 && arg_types[0] != QflType::Symbol {
                        self.error(format!("quince:balance() arg must be symbol, got {}", arg_types[0]));
                    }
                    QflType::F64
                }
                "log" => {
                    if let Some(arg_type) = arg_types.first() {
                        if *arg_type != QflType::Symbol {
                            self.error(format!("quince:log() first arg must be symbol, got {}", arg_type));
                        }
                    }
                    QflType::I64
                }
                "order" => {
                    if arg_types.len() >= 1 && !is_side_compat(arg_types[0]) {
                        self.error(format!("quince:order() side must be side or i64, got {}",
                            arg_types[0]));
                    }
                    if arg_types.len() >= 2 && !is_qty_compat(arg_types[1]) {
                        self.error(format!("quince:order() qty must be qty or numeric, got {}", arg_types[1]));
                    }
                    QflType::I64
                }
                _ => QflType::I64,
            }
        } else {
            QflType::I64
        }
    }

    fn infer_field_access(&mut self, obj: &crate::ast::Expr, field: &str) -> QflType {
        let obj_type = self.infer_expr(obj);
        match field {
            "price" if obj_type == QflType::Symbol => QflType::Price,
            "qty" if obj_type == QflType::Symbol => QflType::Qty,
            "side" if obj_type == QflType::Symbol => QflType::Side,
            "trade_id" if obj_type == QflType::Symbol => QflType::I64,
            "time" if obj_type == QflType::Symbol => QflType::Timestamp,
            _ => {
                self.error(format!("unknown field '{}' for type {}", field, obj_type));
                QflType::I64
            }
        }
    }
}

/// Run type-checking on a parsed QFL program.
/// Returns `Ok(())` if valid, or `Err(Vec<TypeError>)` listing all errors.
pub fn type_check(program: &crate::ast::Program) -> Result<(), Vec<TypeError>> {
    let mut checker = TypeChecker::new();
    checker.check_program(program);
    if checker.errors.is_empty() {
        Ok(())
    } else {
        Err(checker.errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinOp::*;
    use crate::ast::UnaryOp;
    use QflType::*;

    // ── Basic type properties ──

    #[test]
    fn i64_is_numeric() { assert!(I64.is_numeric()); }
    #[test]
    fn f64_is_numeric() { assert!(F64.is_numeric()); }
    #[test]
    fn price_is_numeric() { assert!(Price.is_numeric()); }
    #[test]
    fn qty_is_numeric() { assert!(Qty.is_numeric()); }
    #[test]
    fn timestamp_is_numeric() { assert!(Timestamp.is_numeric()); }
    #[test]
    fn duration_is_numeric() { assert!(Duration.is_numeric()); }
    #[test]
    fn bool_is_not_numeric() { assert!(!Bool.is_numeric()); }
    #[test]
    fn symbol_is_not_numeric() { assert!(!Symbol.is_numeric()); }
    #[test]
    fn side_is_not_numeric() { assert!(!Side.is_numeric()); }
    #[test]
    fn order_id_is_not_numeric() { assert!(!OrderId.is_numeric()); }

    #[test]
    fn price_is_float() { assert!(Price.is_float()); }
    #[test]
    fn qty_is_float() { assert!(Qty.is_float()); }
    #[test]
    fn i64_is_not_float() { assert!(!I64.is_float()); }
    #[test]
    fn timestamp_is_not_float() { assert!(!Timestamp.is_float()); }
    #[test]
    fn i64_is_integer() { assert!(I64.is_integer()); }
    #[test]
    fn price_is_not_integer() { assert!(!Price.is_integer()); }

    // ── Same type arithmetic ──

    #[test]
    fn i64_add_i64_returns_i64() {
        assert_eq!(bin_op_type(I64, &Add, I64), Ok(I64));
    }
    #[test]
    fn f64_mul_f64_returns_f64() {
        assert_eq!(bin_op_type(F64, &Mul, F64), Ok(F64));
    }
    #[test]
    fn price_sub_price_returns_price() {
        assert_eq!(bin_op_type(Price, &Sub, Price), Ok(Price));
    }
    #[test]
    fn qty_div_qty_returns_qty() {
        assert_eq!(bin_op_type(Qty, &Div, Qty), Ok(Qty));
    }

    // ── Price arithmetic ──

    #[test]
    fn price_add_duration_returns_price() {
        assert_eq!(bin_op_type(Price, &Add, Duration), Ok(Price));
    }
    #[test]
    fn duration_add_price_returns_price() {
        assert_eq!(bin_op_type(Duration, &Add, Price), Ok(Price));
    }
    #[test]
    fn price_sub_duration_returns_price() {
        assert_eq!(bin_op_type(Price, &Sub, Duration), Ok(Price));
    }
    #[test]
    fn price_mul_qty_returns_price() {
        assert_eq!(bin_op_type(Price, &Mul, Qty), Ok(Price));
    }
    #[test]
    fn qty_mul_price_returns_price() {
        assert_eq!(bin_op_type(Qty, &Mul, Price), Ok(Price));
    }

    // ── Numeric promotion ──

    #[test]
    fn i64_add_f64_returns_f64() {
        assert_eq!(bin_op_type(I64, &Add, F64), Ok(F64));
    }
    #[test]
    fn f64_sub_i64_returns_f64() {
        assert_eq!(bin_op_type(F64, &Sub, I64), Ok(F64));
    }
    #[test]
    fn i64_mul_price_returns_price() {
        assert_eq!(bin_op_type(I64, &Mul, Price), Ok(Price));
    }
    #[test]
    fn price_div_i64_returns_price() {
        assert_eq!(bin_op_type(Price, &Div, I64), Ok(Price));
    }
    #[test]
    fn i64_add_duration_returns_duration() {
        assert_eq!(bin_op_type(I64, &Add, Duration), Ok(Duration));
    }
    #[test]
    fn i64_add_timestamp_returns_timestamp() {
        assert_eq!(bin_op_type(I64, &Add, Timestamp), Ok(Timestamp));
    }

    // ── Comparison ops ──

    #[test]
    fn i64_eq_i64_returns_bool() {
        assert_eq!(bin_op_type(I64, &Eq, I64), Ok(Bool));
    }
    #[test]
    fn price_lt_price_returns_bool() {
        assert_eq!(bin_op_type(Price, &Lt, Price), Ok(Bool));
    }
    #[test]
    fn price_gt_qty_returns_bool() {
        assert_eq!(bin_op_type(Price, &Gt, Qty), Ok(Bool));
    }
    #[test]
    fn symbol_eq_symbol_returns_bool() {
        assert_eq!(bin_op_type(Symbol, &Eq, Symbol), Ok(Bool));
    }

    // ── Logical ops ──

    #[test]
    fn bool_and_bool_returns_bool() {
        assert_eq!(bin_op_type(Bool, &And, Bool), Ok(Bool));
    }
    #[test]
    fn bool_or_bool_returns_bool() {
        assert_eq!(bin_op_type(Bool, &Or, Bool), Ok(Bool));
    }

    // ── Concat ──

    #[test]
    fn symbol_concat_symbol_returns_symbol() {
        assert_eq!(bin_op_type(Symbol, &Concat, Symbol), Ok(Symbol));
    }

    // ── Unary ops ──

    #[test]
    fn neg_i64_returns_i64() {
        assert_eq!(unary_op_type(&UnaryOp::Neg, I64), Ok(I64));
    }
    #[test]
    fn neg_price_returns_price() {
        assert_eq!(unary_op_type(&UnaryOp::Neg, Price), Ok(Price));
    }
    #[test]
    fn not_bool_returns_bool() {
        assert_eq!(unary_op_type(&UnaryOp::Not, Bool), Ok(Bool));
    }
    #[test]
    fn len_symbol_returns_i64() {
        assert_eq!(unary_op_type(&UnaryOp::Len, Symbol), Ok(I64));
    }

    // ── Type errors (invalid operations) ──

    #[test]
    fn price_add_side_is_error() {
        let result = bin_op_type(Price, &Add, Side);
        assert!(result.is_err());
        assert!(result.unwrap_err().msg.contains("invalid operation"));
    }
    #[test]
    fn symbol_add_symbol_is_error() {
        let result = bin_op_type(Symbol, &Add, Symbol);
        assert!(result.is_err());
    }
    #[test]
    fn bool_add_bool_is_error() {
        let result = bin_op_type(Bool, &Add, Bool);
        assert!(result.is_err());
    }
    #[test]
    fn order_id_mul_order_id_is_error() {
        let result = bin_op_type(OrderId, &Mul, OrderId);
        assert!(result.is_err());
    }
    #[test]
    fn price_mod_side_is_error() {
        let result = bin_op_type(Price, &Mod, Side);
        assert!(result.is_err());
    }
    #[test]
    fn side_eq_price_is_error() {
        let result = bin_op_type(Side, &Eq, Price);
        assert!(result.is_err());
    }
    #[test]
    fn side_add_price_is_error() {
        let result = bin_op_type(Side, &Add, Price);
        assert!(result.is_err());
    }
    #[test]
    fn price_mul_symbol_is_error() {
        let result = bin_op_type(Price, &Mul, Symbol);
        assert!(result.is_err());
    }
    #[test]
    fn not_i64_is_error() {
        let result = unary_op_type(&UnaryOp::Not, I64);
        assert!(result.is_err());
    }
    #[test]
    fn neg_bool_is_error() {
        let result = unary_op_type(&UnaryOp::Neg, Bool);
        assert!(result.is_err());
    }
    #[test]
    fn len_price_is_error() {
        let result = unary_op_type(&UnaryOp::Len, Price);
        assert!(result.is_err());
    }

    // ── And/Or require Bool ──

    #[test]
    fn i64_and_bool_is_error() {
        let result = bin_op_type(I64, &And, Bool);
        assert!(result.is_err());
    }
    #[test]
    fn bool_or_i64_is_error() {
        let result = bin_op_type(Bool, &Or, I64);
        assert!(result.is_err());
    }

    // ── Literal type inference ──

    #[test]
    fn literal_i64_type() {
        assert_eq!(literal_type(&crate::ast::Literal::I64(42)), I64);
    }
    #[test]
    fn literal_f64_type() {
        assert_eq!(literal_type(&crate::ast::Literal::F64(3.14)), F64);
    }
    #[test]
    fn literal_bool_type() {
        assert_eq!(literal_type(&crate::ast::Literal::Bool(true)), Bool);
    }
    #[test]
    fn literal_string_type() {
        assert_eq!(literal_type(&crate::ast::Literal::String("hello".into())), Symbol);
    }
    #[test]
    fn literal_nil_type() {
        assert_eq!(literal_type(&crate::ast::Literal::Nil), I64);
    }

    // ── Program-level type checker ──

    fn check(src: &str) -> Result<(), Vec<TypeError>> {
        let program = crate::parser::parse(src).unwrap();
        type_check(&program)
    }

    fn check_err(src: &str) -> Vec<TypeError> {
        check(src).unwrap_err()
    }

    #[test]
    fn check_empty_program_ok() {
        assert!(check("").is_ok());
    }

    #[test]
    fn check_i64_literal_ok() {
        assert!(check("42").is_ok());
    }

    #[test]
    fn check_f64_literal_ok() {
        assert!(check("3.14").is_ok());
    }

    #[test]
    fn check_i64_add_ok() {
        assert!(check("1 + 2").is_ok());
    }

    #[test]
    fn check_f64_add_ok() {
        assert!(check("1.0 + 2.0").is_ok());
    }

    #[test]
    fn check_price_plus_side_is_error() {
        let errs = check_err("trade.price + trade.side");
        assert!(!errs.is_empty(), "price + side should be a type error");
    }

    #[test]
    fn check_symbol_add_symbol_is_error() {
        let errs = check_err("\"a\" + \"b\"");
        assert!(!errs.is_empty());
    }

    #[test]
    fn check_bool_and_bool_ok() {
        assert!(check("true and false").is_ok());
    }

    #[test]
    fn check_i64_and_bool_is_error() {
        let errs = check_err("1 and true");
        assert!(!errs.is_empty());
    }

    #[test]
    fn check_if_condition_bool_ok() {
        assert!(check("if true then 42 end").is_ok());
    }

    #[test]
    fn check_if_condition_i64_is_error() {
        let errs = check_err("if 42 then 1 end");
        assert!(!errs.is_empty(), "if on i64 should error");
    }

    #[test]
    fn check_while_condition_bool_ok() {
        assert!(check("while false do end").is_ok());
    }

    #[test]
    fn check_while_condition_i64_is_error() {
        let errs = check_err("while 1 do end");
        assert!(!errs.is_empty());
    }

    #[test]
    fn check_var_decl_i64_ok() {
        assert!(check("local x = 42").is_ok());
    }

    #[test]
    fn check_var_decl_bool_ok() {
        assert!(check("local x = true").is_ok());
    }

    #[test]
    fn check_assign_type_mismatch_error() {
        let errs = check_err("local x = 42 x = true");
        assert!(!errs.is_empty(), "assigning bool to i64 var should error");
    }

    #[test]
    fn check_on_trade_params_ok() {
        assert!(check("function on_trade(trade) local p = trade.price end").is_ok());
    }

    #[test]
    fn check_trade_price_is_price() {
        let errs = check_err("function on_trade(trade) local x = trade.price + true end");
        assert!(!errs.is_empty(), "price + bool should error");
    }

    #[test]
    fn check_quince_get_ok() {
        assert!(check("quince.get(\"ema\")").is_ok());
    }

    #[test]
    fn check_quince_get_non_symbol_error() {
        let errs = check_err("quince.get(42)");
        assert!(!errs.is_empty(), "quince.get(i64) should error");
    }

    #[test]
    fn check_quince_order_ok() {
        assert!(check("quince.order(0, 1.0, 100.0)").is_ok());
    }

    #[test]
    fn check_for_num_i64_ok() {
        assert!(check("for i = 1, 10 do end").is_ok());
    }

    #[test]
    fn check_repeat_until_bool_ok() {
        assert!(check("repeat until true").is_ok());
    }

    #[test]
    fn check_repeat_until_i64_error() {
        let errs = check_err("repeat until 42");
        assert!(!errs.is_empty());
    }

    #[test]
    fn check_nested_scopes_ok() {
        assert!(check("local x = 1 if true then local y = x end").is_ok());
    }

    #[test]
    fn check_fn_decl_multiple_entries_ok() {
        let src = "
            function on_trade(trade) quince.log(\"trade\") end
            function on_eval() quince.log(\"eval\") end
        ";
        assert!(check(src).is_ok());
    }

    #[test]
    fn check_persist_var_ok() {
        assert!(check("@persist local x = 42 function on_eval() end").is_ok());
    }

    // ── Realistic strategies ──

    #[test]
    fn check_scalper_strategy_ok() {
        let src = "
            @persist local position_size = 0
            @persist local last_entry = 0
            function on_trade(trade)
                local price = trade.price
                local mid = quince.get(\"bb.middle\")
                local upper = quince.get(\"bb.upper\")
                local lower = quince.get(\"bb.lower\")
                local ema = quince.get(\"ema\")
                if price < lower and position_size == 0 then
                    quince.order(0, 1.0, 0)
                    position_size = 1
                end
            end
            function on_eval()
                quince.log(\"eval\")
            end
        ";
        assert!(check(src).is_ok(), "scalper should type-check: {:?}", check(src).err());
    }

    #[test]
    fn check_ema_cross_strategy_ok() {
        let src = "
            @persist local position_size = 0
            function on_trade(trade)
                local fast = quince.get(\"ema9\")
                local slow = quince.get(\"ema50\")
                if fast > slow and position_size <= 0 then
                    quince.order(0, 1.0, 0)
                    position_size = 1
                end
            end
            function on_eval() quince.log(\"eval\") end
        ";
        assert!(check(src).is_ok());
    }

    #[test]
    fn check_invalid_strategy_type_error() {
        // position_size (i64) + true (bool) is invalid
        let errs = check_err("
            @persist local position_size = 0
            function on_trade(trade)
                local x = position_size + true
            end
        ");
        assert!(!errs.is_empty(), "i64 + bool should error");
    }
}
