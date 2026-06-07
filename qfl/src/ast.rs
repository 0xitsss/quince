use std::fmt;

/// Binary operators supported in QFL expressions.
///
/// Includes arithmetic (`+`, `-`, `*`, `/`, `//`, `%`, `^`),
/// comparison (`==`, `~=`, `<`, `>`, `<=`, `>=`),
/// concatenation (`..`), and logical (`and`, `or`).
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv, // //
    Mod,
    Pow,    // ^
    Concat, // ..
    Eq,     // ==
    Ne,     // ~=
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::IDiv => write!(f, "//"),
            BinOp::Mod => write!(f, "%%"),
            BinOp::Pow => write!(f, "^"),
            BinOp::Concat => write!(f, ".."),
            BinOp::Eq => write!(f, "=="),
            BinOp::Ne => write!(f, "~="),
            BinOp::Lt => write!(f, "<"),
            BinOp::Gt => write!(f, ">"),
            BinOp::Le => write!(f, "<="),
            BinOp::Ge => write!(f, ">="),
            BinOp::And => write!(f, "and"),
            BinOp::Or => write!(f, "or"),
        }
    }
}

/// Unary operators: negation (`-`), logical not (`not`), length (`#`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg, // -
    Not, // not
    Len, // #
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOp::Neg => write!(f, "-"),
            UnaryOp::Not => write!(f, "not"),
            UnaryOp::Len => write!(f, "#"),
        }
    }
}

/// Literal values in QFL: nil, booleans, integers, floats, and strings.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Nil,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Nil => write!(f, "nil"),
            Literal::Bool(b) => write!(f, "{}", b),
            Literal::I64(n) => write!(f, "{}", n),
            Literal::F64(n) => write!(f, "{}", n),
            Literal::String(s) => write!(f, "\"{}\"", s),
        }
    }
}

/// QFL expression node.
///
/// Covers literals, identifiers, function/method calls, field/index access,
/// unary and binary operations, and table constructors.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Ident(String),
    FnCall {
        name: String,
        args: Vec<Expr>,
    },
    MethodCall {
        obj: String,
        method: String,
        args: Vec<Expr>,
    },
    FieldAccess {
        obj: Box<Expr>,
        field: String,
    },
    Index {
        obj: Box<Expr>,
        index: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        lhs: Box<Expr>,
        op: BinOp,
        rhs: Box<Expr>,
    },
    Table(Vec<TableField>),
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Literal(lit) => write!(f, "{}", lit),
            Expr::Ident(name) => write!(f, "{}", name),
            Expr::FnCall { name, args } => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Expr::MethodCall {
                obj,
                method,
                args: _,
            } => {
                write!(f, "{}:{}(...)", obj, method)
            }
            Expr::FieldAccess { obj, field } => {
                write!(f, "{}.{}", obj, field)
            }
            Expr::Index { obj, index } => {
                write!(f, "{}[{}]", obj, index)
            }
            Expr::Unary { op, expr } => {
                write!(f, "{}{}", op, expr)
            }
            Expr::Binary { lhs, op, rhs } => {
                write!(f, "({} {} {})", lhs, op, rhs)
            }
            Expr::Table(fields) => {
                write!(f, "{{")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", field)?;
                }
                write!(f, "}}")
            }
        }
    }
}

/// A field in a table constructor: either `[key] = value` or a plain value.
#[derive(Debug, Clone, PartialEq)]
pub enum TableField {
    KeyValue { key: Expr, value: Expr },
    Value(Expr),
}

impl fmt::Display for TableField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableField::KeyValue { key, value } => write!(f, "[{}] = {}", key, value),
            TableField::Value(expr) => write!(f, "{}", expr),
        }
    }
}

/// QFL statement node.
///
/// Includes variable declarations, assignments, control flow
/// (if/while/repeat/for), function definitions, event handlers,
/// and declarative pipeline statements (using, window, feature, signal, state).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    VarDecl {
        names: Vec<String>,
        init: Option<Vec<Expr>>,
        is_local: bool,
        persist: bool,
    },
    Assign {
        targets: Vec<Expr>,
        exprs: Vec<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_body: Vec<Stmt>,
        elseif_branches: Vec<(Box<Expr>, Vec<Stmt>)>,
        else_body: Vec<Stmt>,
    },
    While {
        cond: Box<Expr>,
        body: Vec<Stmt>,
    },
    Repeat {
        body: Vec<Stmt>,
        until: Box<Expr>,
    },
    ForNum {
        var: String,
        from: Box<Expr>,
        to: Box<Expr>,
        step: Option<Box<Expr>>,
        body: Vec<Stmt>,
    },
    ForIn {
        vars: Vec<String>,
        exprs: Vec<Expr>,
        body: Vec<Stmt>,
    },
    FunctionDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Return {
        exprs: Vec<Expr>,
    },
    ExprStmt(Expr),

    // Phase 4g: declarative feature pipeline
    Using {
        indicators: Vec<UsingEntry>,
    },
    Window {
        name: String,
        capacity: usize,
    },
    Feature {
        name: String,
        expr: Box<Expr>,
    },
    Signal {
        name: String,
        expr: Box<Expr>,
    },

    // Phase 4h: typed state variables
    State {
        name: String,
        type_name: String,
        default: Option<Box<Expr>>,
    },

    // Phase 4h: event handlers
    EventHandler {
        event: String,
        param: Option<String>,
        body: Vec<Stmt>,
    },

    // Phase 4h: typed user functions
    FnDecl {
        name: String,
        params: Vec<FnParam>,
        return_type: String,
        body: Vec<Stmt>,
    },
}

/// A typed function parameter: `name: type`.
#[derive(Debug, Clone, PartialEq)]
pub struct FnParam {
    pub name: String,
    pub type_name: String,
}

/// An entry in the `@using` directive specifying an indicator and its parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct UsingEntry {
    pub name: String,
    pub params: Vec<f64>,
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stmt::VarDecl {
                names,
                init,
                is_local,
                persist,
            } => {
                if *persist {
                    write!(f, "@persist ")?;
                }
                if *is_local {
                    write!(f, "local ")?;
                }
                for (i, name) in names.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", name)?;
                }
                if let Some(exprs) = init {
                    write!(f, " = ")?;
                    for (i, expr) in exprs.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", expr)?;
                    }
                }
                Ok(())
            }
            Stmt::If { cond, .. } => write!(f, "if {} then ... end", cond),
            Stmt::While { cond, .. } => write!(f, "while {} do ... end", cond),
            Stmt::Repeat { until, .. } => write!(f, "repeat ... until {}", until),
            Stmt::ForNum {
                var, from, to: _, ..
            } => write!(f, "for {} = {} to ... end", var, from),
            Stmt::ForIn { vars, .. } => write!(f, "for {} in ... end", vars.join(", ")),
            Stmt::FunctionDecl { name, params, .. } => {
                write!(f, "function {}({}) ... end", name, params.join(", "))
            }
            Stmt::Return { exprs } => {
                write!(f, "return")?;
                for expr in exprs {
                    write!(f, " {}", expr)?;
                }
                Ok(())
            }
            Stmt::ExprStmt(expr) => write!(f, "{}", expr),
            Stmt::Assign { targets, exprs } => {
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", target)?;
                }
                write!(f, " = ")?;
                for (i, expr) in exprs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", expr)?;
                }
                Ok(())
            }
            Stmt::Using { indicators } => {
                write!(f, "@using")?;
                for entry in indicators {
                    write!(
                        f,
                        " {}:{}",
                        entry.name,
                        entry
                            .params
                            .iter()
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(":")
                    )?;
                }
                Ok(())
            }
            Stmt::Window { name, capacity } => {
                write!(f, "window {} {}", name, capacity)
            }
            Stmt::Feature { name, expr } => {
                write!(f, "feature {} = {}", name, expr)
            }
            Stmt::Signal { name, expr } => {
                write!(f, "signal {} = {}", name, expr)
            }
            Stmt::State {
                name,
                type_name,
                default,
            } => {
                write!(f, "state {} : {}", name, type_name)?;
                if let Some(expr) = default {
                    write!(f, " = {}", expr)?;
                }
                Ok(())
            }
            Stmt::EventHandler { event, param, body } => {
                write!(f, "on {}(", event)?;
                if let Some(p) = param {
                    write!(f, "{}", p)?;
                }
                write!(f, ") {{ ... }} ({} stmts)", body.len())
            }
            Stmt::FnDecl {
                name,
                params,
                return_type,
                body,
            } => {
                write!(f, "fn {}(", name)?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", p.name, p.type_name)?;
                }
                write!(f, ") -> {} {{ ... }} ({} stmts)", return_type, body.len())
            }
        }
    }
}

/// The top-level QFL program: a list of statements
pub type Program = Vec<Stmt>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_binop_add() {
        assert_eq!(BinOp::Add.to_string(), "+");
    }
    #[test]
    fn display_binop_sub() {
        assert_eq!(BinOp::Sub.to_string(), "-");
    }
    #[test]
    fn display_binop_mul() {
        assert_eq!(BinOp::Mul.to_string(), "*");
    }
    #[test]
    fn display_binop_div() {
        assert_eq!(BinOp::Div.to_string(), "/");
    }
    #[test]
    fn display_binop_idiv() {
        assert_eq!(BinOp::IDiv.to_string(), "//");
    }
    #[test]
    fn display_binop_mod() {
        assert_eq!(BinOp::Mod.to_string(), "%%");
    }
    #[test]
    fn display_binop_pow() {
        assert_eq!(BinOp::Pow.to_string(), "^");
    }
    #[test]
    fn display_binop_concat() {
        assert_eq!(BinOp::Concat.to_string(), "..");
    }
    #[test]
    fn display_binop_eq() {
        assert_eq!(BinOp::Eq.to_string(), "==");
    }
    #[test]
    fn display_binop_ne() {
        assert_eq!(BinOp::Ne.to_string(), "~=");
    }
    #[test]
    fn display_binop_lt() {
        assert_eq!(BinOp::Lt.to_string(), "<");
    }
    #[test]
    fn display_unary_neg() {
        assert_eq!(UnaryOp::Neg.to_string(), "-");
    }
    #[test]
    fn display_unary_not() {
        assert_eq!(UnaryOp::Not.to_string(), "not");
    }
    #[test]
    fn display_unary_len() {
        assert_eq!(UnaryOp::Len.to_string(), "#");
    }
    #[test]
    fn display_literal_i64() {
        assert_eq!(Literal::I64(42).to_string(), "42");
    }
    #[test]
    fn display_literal_f64() {
        assert_eq!(Literal::F64(3.14).to_string(), "3.14");
    }
    #[test]
    fn display_literal_bool() {
        assert_eq!(Literal::Bool(true).to_string(), "true");
        assert_eq!(Literal::Bool(false).to_string(), "false");
    }
    #[test]
    fn display_literal_string() {
        assert_eq!(Literal::String("hello".into()).to_string(), "\"hello\"");
    }
    #[test]
    fn display_literal_nil() {
        assert_eq!(Literal::Nil.to_string(), "nil");
    }
    #[test]
    fn display_expr_ident() {
        assert_eq!(Expr::Ident("x".into()).to_string(), "x");
    }
    #[test]
    fn display_expr_fcall() {
        let e = Expr::FnCall {
            name: "foo".into(),
            args: vec![Expr::Literal(Literal::I64(1))],
        };
        assert_eq!(e.to_string(), "foo(1)");
    }
    #[test]
    fn display_expr_binop() {
        let e = Expr::Binary {
            lhs: Box::new(Expr::Literal(Literal::I64(1))),
            op: BinOp::Add,
            rhs: Box::new(Expr::Literal(Literal::I64(2))),
        };
        assert_eq!(e.to_string(), "(1 + 2)");
    }
}
