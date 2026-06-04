use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,  // //
    Mod,
    Pow,   // ^
    Concat, // ..
    Eq,    // ==
    Ne,    // ~=
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
            Expr::MethodCall { obj, method, args: _ } => {
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
}

impl fmt::Display for Stmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stmt::VarDecl { names, init, is_local, persist } => {
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
            Stmt::ForNum { var, from, to: _, .. } => write!(f, "for {} = {} to ... end", var, from),
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
        }
    }
}

/// The top-level QFL program: a list of statements
pub type Program = Vec<Stmt>;
