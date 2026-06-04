use crate::ast::*;
use crate::lexer::Token;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub msg: String,
    pub pos: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at token {}: {}", self.pos, self.msg)
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    pub fn parse(&mut self) -> Result<Program, ParseError> {
        let mut stmts = Vec::new();
        while !self.is_eof() {
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    // --- helpers ---

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek_at(&self, n: usize) -> &Token {
        self.tokens.get(self.pos + n).unwrap_or(&Token::Eof)
    }

    fn is_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(self.err(&format!("expected {}, got {}", expected, self.peek())))
        }
    }

    fn err(&self, msg: &str) -> ParseError {
        ParseError { msg: msg.to_string(), pos: self.pos }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(s) => {
                self.advance();
                Ok(s)
            }
            ref other => Err(self.err(&format!("expected identifier, got {}", other))),
        }
    }

    // --- statements ---

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        let persist = if matches!(self.peek(), Token::AtPersist) {
            self.advance();
            true
        } else {
            false
        };

        match self.peek() {
            Token::Local => {
                self.advance();
                self.parse_var_decl(true, persist)
            }
            Token::Function => self.parse_function_decl(),
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::Repeat => self.parse_repeat(),
            Token::For => self.parse_for(),
            Token::Return => self.parse_return(),
            Token::Semi => {
                self.advance();
                Ok(Stmt::ExprStmt(Expr::Literal(Literal::Nil)))
            }
            Token::Using | Token::Comment(_) => {
                self.advance();
                self.parse_stmt()
            }
            _ => {
                if persist {
                    self.parse_var_decl(false, true)
                } else {
                    self.parse_assign_or_call()
                }
            }
        }
    }

    fn parse_var_decl(&mut self, is_local: bool, persist: bool) -> Result<Stmt, ParseError> {
        let names = self.parse_name_list()?;
        let init = if matches!(self.peek(), Token::Eq) {
            self.advance();
            Some(self.parse_expr_list()?)
        } else {
            None
        };
        Ok(Stmt::VarDecl { names, init, is_local, persist })
    }

    fn parse_name_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut names = Vec::new();
        names.push(self.expect_ident()?);
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            names.push(self.expect_ident()?);
        }
        Ok(names)
    }

    fn parse_function_decl(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        if !matches!(self.peek(), Token::RParen) {
            params.push(self.expect_ident()?);
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                params.push(self.expect_ident()?);
            }
        }
        self.expect(&Token::RParen)?;
        let body = self.parse_block_until(&[Token::End])?;
        self.expect(&Token::End)?;
        Ok(Stmt::FunctionDecl { name, params, body })
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let cond = Box::new(self.parse_expr()?);
        self.expect(&Token::Then)?;
        let then_body = self.parse_block_until(&[Token::Else, Token::ElseIf, Token::End])?;

        let mut elseif_branches = Vec::new();
        let mut else_body = Vec::new();

        while matches!(self.peek(), Token::ElseIf) {
            self.advance();
            let econd = Box::new(self.parse_expr()?);
            self.expect(&Token::Then)?;
            let ebody = self.parse_block_until(&[Token::Else, Token::ElseIf, Token::End])?;
            elseif_branches.push((econd, ebody));
        }

        if matches!(self.peek(), Token::Else) {
            self.advance();
            else_body = self.parse_block_until(&[Token::End])?;
        }

        self.expect(&Token::End)?;
        Ok(Stmt::If { cond, then_body, elseif_branches, else_body })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let cond = Box::new(self.parse_expr()?);
        self.expect(&Token::Do)?;
        let body = self.parse_block_until(&[Token::End])?;
        self.expect(&Token::End)?;
        Ok(Stmt::While { cond, body })
    }

    fn parse_repeat(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let body = self.parse_block_until(&[Token::Until])?;
        self.advance();
        let cond = Box::new(self.parse_expr()?);
        Ok(Stmt::Repeat { body, until: cond })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let first = self.expect_ident()?;
        if matches!(self.peek(), Token::Eq) {
            self.advance();
            let from = Box::new(self.parse_expr()?);
            self.expect(&Token::Comma)?;
            let to = Box::new(self.parse_expr()?);
            let step = if matches!(self.peek(), Token::Comma) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            self.expect(&Token::Do)?;
            let body = self.parse_block_until(&[Token::End])?;
            self.expect(&Token::End)?;
            Ok(Stmt::ForNum { var: first, from, to, step, body })
        } else {
            let mut vars = vec![first];
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                vars.push(self.expect_ident()?);
            }
            self.expect(&Token::In)?;
            let exprs = self.parse_expr_list()?;
            self.expect(&Token::Do)?;
            let body = self.parse_block_until(&[Token::End])?;
            self.expect(&Token::End)?;
            Ok(Stmt::ForIn { vars, exprs, body })
        }
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let exprs = if self.is_eof()
            || matches!(self.peek(), Token::End | Token::Else | Token::ElseIf | Token::Until)
        {
            Vec::new()
        } else {
            self.parse_expr_list()?
        };
        Ok(Stmt::Return { exprs })
    }

    fn parse_assign_or_call(&mut self) -> Result<Stmt, ParseError> {
        // Parse LHS as a full expression — needed for binary ops like `a + b`
        let first = self.parse_expr()?;

        // Multi-target: a, b = 1, 2
        if matches!(self.peek(), Token::Comma) {
            let mut targets = vec![first];
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                targets.push(self.parse_expr()?);
            }
            self.expect(&Token::Eq)?;
            let exprs = self.parse_expr_list()?;
            return Ok(Stmt::Assign { targets, exprs });
        }

        // Single assignment: a = 1 or a.b = 1
        if matches!(self.peek(), Token::Eq) {
            self.advance();
            let exprs = self.parse_expr_list()?;
            return Ok(Stmt::Assign { targets: vec![first], exprs });
        }

        Ok(Stmt::ExprStmt(first))
    }

    fn parse_block_until(&mut self, delimiters: &[Token]) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        while !self.is_eof() {
            if delimiters.iter().any(|d| self.peek() == d) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    // --- expressions (precedence climbing) ---

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(0)
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut exprs = Vec::new();
        exprs.push(self.parse_expr()?);
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }

    /// Precedence climbing for binary operators.
    /// min_prec: minimum precedence to accept (higher = tighter binding).
    fn parse_binary(&mut self, min_prec: u32) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;

        loop {
            let op_data = match self.peek() {
                Token::Or => Some((BinOp::Or, 10, false)),
                Token::And => Some((BinOp::And, 20, false)),
                Token::EqEq => Some((BinOp::Eq, 30, false)),
                Token::TildeEq => Some((BinOp::Ne, 30, false)),
                Token::Lt => Some((BinOp::Lt, 30, false)),
                Token::Gt => Some((BinOp::Gt, 30, false)),
                Token::LtEq => Some((BinOp::Le, 30, false)),
                Token::GtEq => Some((BinOp::Ge, 30, false)),
                Token::Concat => Some((BinOp::Concat, 40, false)),
                Token::Plus => Some((BinOp::Add, 50, false)),
                Token::Minus => Some((BinOp::Sub, 50, false)),
                Token::Star => Some((BinOp::Mul, 60, false)),
                Token::Slash => Some((BinOp::Div, 60, false)),
                Token::SlashSlash => Some((BinOp::IDiv, 60, false)),
                Token::Percent => Some((BinOp::Mod, 60, false)),
                Token::Caret => Some((BinOp::Pow, 70, true)),
                _ => None,
            };

            let (op, prec, right_assoc) = match op_data {
                Some(d) => d,
                None => break,
            };

            if prec < min_prec {
                break;
            }

            self.advance();
            let next_prec = if right_assoc { prec } else { prec + 1 };
            let rhs = self.parse_binary(next_prec)?;
            lhs = Expr::Binary { lhs: Box::new(lhs), op, rhs: Box::new(rhs) };
        }

        Ok(lhs)
    }

    /// Unary ops + power (right-assoc) + postfix + prefix
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(expr) });
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(expr) });
            }
            Token::Hash => {
                self.advance();
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary { op: UnaryOp::Len, expr: Box::new(expr) });
            }
            _ => {}
        }

        // Power: right-associative
        let mut lhs = self.parse_postfix()?;
        if matches!(self.peek(), Token::Caret) {
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Binary { lhs: Box::new(lhs), op: BinOp::Pow, rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }

    /// Postfix: field access, method call, index, function call
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_prefix()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    self.advance();
                    let field = self.expect_ident()?;
                    expr = Expr::FieldAccess { obj: Box::new(expr), field };
                }
                Token::Colon => {
                    self.advance();
                    let method = self.expect_ident()?;
                    self.expect(&Token::LParen)?;
                    let args = if matches!(self.peek(), Token::RParen) {
                        Vec::new()
                    } else {
                        self.parse_expr_list()?
                    };
                    self.expect(&Token::RParen)?;
                    let obj = match &expr {
                        Expr::Ident(name) => name.clone(),
                        Expr::FieldAccess { field, .. } => field.clone(),
                        _ => return Err(self.err("method call requires an object")),
                    };
                    expr = Expr::MethodCall { obj, method, args };
                }
                Token::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    expr = Expr::Index { obj: Box::new(expr), index: Box::new(index) };
                }
                Token::LParen => {
                    self.advance();
                    let args = if matches!(self.peek(), Token::RParen) {
                        Vec::new()
                    } else {
                        self.parse_expr_list()?
                    };
                    self.expect(&Token::RParen)?;
                    match expr {
                        Expr::Ident(name) => {
                            expr = Expr::FnCall { name, args };
                        }
                        Expr::FieldAccess { obj, field } => {
                            let obj_name = match obj.as_ref() {
                                Expr::Ident(s) => s.clone(),
                                Expr::FieldAccess { field: f, .. } => f.clone(),
                                other => return Err(self.err(
                                    &format!("cannot call method on {:?}", other)
                                )),
                            };
                            expr = Expr::MethodCall { obj: obj_name, method: field, args };
                        }
                        _ => return Err(self.err("cannot call non-function expression")),
                    }
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek().clone();
        match tok {
            Token::Nil => { self.advance(); Ok(Expr::Literal(Literal::Nil)) }
            Token::True => { self.advance(); Ok(Expr::Literal(Literal::Bool(true))) }
            Token::False => { self.advance(); Ok(Expr::Literal(Literal::Bool(false))) }
            Token::Number(s) => {
                self.advance();
                Ok(Expr::Literal(parse_number(&s)?))
            }
            Token::String(s) => {
                self.advance();
                Ok(Expr::Literal(Literal::String(s)))
            }
            Token::Ident(s) => {
                self.advance();
                Ok(Expr::Ident(s))
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBrace => {
                self.parse_table()
            }
            Token::VarArg => {
                self.advance();
                Ok(Expr::Ident("...".to_string()))
            }
            ref other => Err(self.err(&format!("unexpected token in expression: {}", other))),
        }
    }

    fn parse_table(&mut self) -> Result<Expr, ParseError> {
        self.advance();
        let mut fields = Vec::new();
        while !matches!(self.peek(), Token::RBrace) && !self.is_eof() {
            if matches!(self.peek(), Token::LBracket) {
                self.advance();
                let key = self.parse_expr()?;
                self.expect(&Token::RBracket)?;
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                fields.push(TableField::KeyValue { key, value });
            } else if matches!(self.peek_at(1), Token::Eq) {
                let name = self.expect_ident()?;
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                fields.push(TableField::KeyValue {
                    key: Expr::Literal(Literal::String(name)),
                    value,
                });
            } else {
                let value = self.parse_expr()?;
                fields.push(TableField::Value(value));
            }
            if matches!(self.peek(), Token::Comma | Token::Semi) {
                self.advance();
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(Expr::Table(fields))
    }
}

fn parse_number(s: &str) -> Result<Literal, ParseError> {
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s.parse::<f64>().map(Literal::F64).map_err(|_| {
            ParseError { msg: format!("invalid float: {}", s), pos: 0 }
        })
    } else if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16).map(Literal::I64).map_err(|_| {
            ParseError { msg: format!("invalid hex: {}", s), pos: 0 }
        })
    } else {
        s.parse::<i64>().map(Literal::I64).map_err(|_| {
            ParseError { msg: format!("invalid integer: {}", s), pos: 0 }
        })
    }
}

pub fn parse(input: &str) -> Result<Program, ParseError> {
    let tokens = crate::lexer::tokenize(input).map_err(|e| ParseError {
        msg: e.msg,
        pos: 0,
    })?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let prog = parse("").unwrap();
        assert!(prog.is_empty());
    }

    #[test]
    fn test_literal_expr() {
        let prog = parse("42").unwrap();
        assert_eq!(prog.len(), 1);
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::I64(42))));
    }

    #[test]
    fn test_float() {
        let prog = parse("3.14").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::F64(3.14))));
    }

    #[test]
    fn test_string() {
        let prog = parse("\"hello\"").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::String("hello".into()))));
    }

    #[test]
    fn test_bool() {
        let prog = parse("true false nil").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::Bool(true))));
        assert_eq!(prog[1], Stmt::ExprStmt(Expr::Literal(Literal::Bool(false))));
        assert_eq!(prog[2], Stmt::ExprStmt(Expr::Literal(Literal::Nil)));
    }

    #[test]
    fn test_binary_ops() {
        let prog = parse("1 + 2 * 3").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Binary {
                lhs: Box::new(Expr::Literal(Literal::I64(1))),
                op: BinOp::Add,
                rhs: Box::new(Expr::Binary {
                    lhs: Box::new(Expr::Literal(Literal::I64(2))),
                    op: BinOp::Mul,
                    rhs: Box::new(Expr::Literal(Literal::I64(3))),
                }),
            })
        );
    }

    #[test]
    fn test_comparison() {
        let prog = parse("a > 5").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Binary {
                lhs: Box::new(Expr::Ident("a".into())),
                op: BinOp::Gt,
                rhs: Box::new(Expr::Literal(Literal::I64(5))),
            })
        );
    }

    #[test]
    fn test_local_var() {
        let prog = parse("local x = 42").unwrap();
        assert_eq!(
            prog[0],
            Stmt::VarDecl {
                names: vec!["x".into()],
                init: Some(vec![Expr::Literal(Literal::I64(42))]),
                is_local: true,
                persist: false,
            }
        );
    }

    #[test]
    fn test_persist_var() {
        let prog = parse("@persist position_size").unwrap();
        assert_eq!(
            prog[0],
            Stmt::VarDecl {
                names: vec!["position_size".into()],
                init: None,
                is_local: false,
                persist: true,
            }
        );
    }

    #[test]
    fn test_local_persist() {
        let prog = parse("@persist local x = 1").unwrap();
        assert_eq!(
            prog[0],
            Stmt::VarDecl {
                names: vec!["x".into()],
                init: Some(vec![Expr::Literal(Literal::I64(1))]),
                is_local: true,
                persist: true,
            }
        );
    }

    #[test]
    fn test_assign() {
        let prog = parse("x = 10").unwrap();
        assert_eq!(
            prog[0],
            Stmt::Assign {
                targets: vec![Expr::Ident("x".into())],
                exprs: vec![Expr::Literal(Literal::I64(10))],
            }
        );
    }

    #[test]
    fn test_multi_assign() {
        let prog = parse("a, b = 1, 2").unwrap();
        assert_eq!(
            prog[0],
            Stmt::Assign {
                targets: vec![Expr::Ident("a".into()), Expr::Ident("b".into())],
                exprs: vec![
                    Expr::Literal(Literal::I64(1)),
                    Expr::Literal(Literal::I64(2)),
                ],
            }
        );
    }

    #[test]
    fn test_fn_decl() {
        let prog = parse("function foo(a, b) return a + b end").unwrap();
        assert_eq!(
            prog[0],
            Stmt::FunctionDecl {
                name: "foo".into(),
                params: vec!["a".into(), "b".into()],
                body: vec![Stmt::Return {
                    exprs: vec![Expr::Binary {
                        lhs: Box::new(Expr::Ident("a".into())),
                        op: BinOp::Add,
                        rhs: Box::new(Expr::Ident("b".into())),
                    }],
                }],
            }
        );
    }

    #[test]
    fn test_if_stmt() {
        let src = "if x > 0 then return x else return -x end";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::If { cond, then_body, elseif_branches, else_body } => {
                assert_eq!(cond.as_ref(), &Expr::Binary {
                    lhs: Box::new(Expr::Ident("x".into())),
                    op: BinOp::Gt,
                    rhs: Box::new(Expr::Literal(Literal::I64(0))),
                });
                assert_eq!(then_body.len(), 1);
                assert!(elseif_branches.is_empty());
                assert_eq!(else_body.len(), 1);
            }
            _ => panic!("expected If stmt"),
        }
    }

    #[test]
    fn test_while_loop() {
        let prog = parse("while x < 10 do x = x + 1 end").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::While { cond, body } => {
                assert_eq!(cond.as_ref(), &Expr::Binary {
                    lhs: Box::new(Expr::Ident("x".into())),
                    op: BinOp::Lt,
                    rhs: Box::new(Expr::Literal(Literal::I64(10))),
                });
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected While stmt"),
        }
    }

    #[test]
    fn test_repeat_loop() {
        let prog = parse("repeat x = x - 1 until x == 0").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::Repeat { body, until } => {
                assert_eq!(body.len(), 1);
                assert_eq!(until.as_ref(), &Expr::Binary {
                    lhs: Box::new(Expr::Ident("x".into())),
                    op: BinOp::Eq,
                    rhs: Box::new(Expr::Literal(Literal::I64(0))),
                });
            }
            _ => panic!("expected Repeat stmt"),
        }
    }

    #[test]
    fn test_numeric_for() {
        let prog = parse("for i = 1, 10 do print(i) end").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::ForNum { var, from, to, step, body } => {
                assert_eq!(var, "i");
                assert_eq!(from.as_ref(), &Expr::Literal(Literal::I64(1)));
                assert_eq!(to.as_ref(), &Expr::Literal(Literal::I64(10)));
                assert!(step.is_none());
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected ForNum stmt"),
        }
    }

    #[test]
    fn test_for_with_step() {
        let prog = parse("for i = 1, 10, 2 do end").unwrap();
        match &prog[0] {
            Stmt::ForNum { step, .. } => {
                assert!(step.is_some());
                assert_eq!(step.as_ref().unwrap().as_ref(), &Expr::Literal(Literal::I64(2)));
            }
            _ => panic!("expected ForNum stmt"),
        }
    }

    #[test]
    fn test_generic_for() {
        let prog = parse("for k, v in pairs(t) do print(k, v) end").unwrap();
        match &prog[0] {
            Stmt::ForIn { vars, exprs, body } => {
                assert_eq!(vars, &vec!["k".to_string(), "v".to_string()]);
                assert_eq!(exprs.len(), 1);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected ForIn stmt"),
        }
    }

    #[test]
    fn test_fn_call() {
        let prog = parse("print(\"hello\")").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::FnCall {
                name: "print".into(),
                args: vec![Expr::Literal(Literal::String("hello".into()))],
            })
        );
    }

    #[test]
    fn test_method_call() {
        let prog = parse("obj:method(1, 2)").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::MethodCall {
                obj: "obj".into(),
                method: "method".into(),
                args: vec![
                    Expr::Literal(Literal::I64(1)),
                    Expr::Literal(Literal::I64(2)),
                ],
            })
        );
    }

    #[test]
    fn test_field_access() {
        let prog = parse("a.b.c").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::FieldAccess {
                obj: Box::new(Expr::FieldAccess {
                    obj: Box::new(Expr::Ident("a".into())),
                    field: "b".into(),
                }),
                field: "c".into(),
            })
        );
    }

    #[test]
    fn test_table_simple() {
        let prog = parse("{}").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Table(vec![])));
    }

    #[test]
    fn test_table_list() {
        let prog = parse("{1, 2, 3}").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Table(fields)) => {
                assert_eq!(fields.len(), 3);
                assert_eq!(fields[0], TableField::Value(Expr::Literal(Literal::I64(1))));
            }
            _ => panic!("expected table"),
        }
    }

    #[test]
    fn test_table_keyvalue() {
        let prog = parse("{key = \"val\", [1] = \"one\"}").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Table(fields)) => {
                assert_eq!(fields.len(), 2);
            }
            _ => panic!("expected table"),
        }
    }

    #[test]
    fn test_unary_minus() {
        let prog = parse("-42").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Literal(Literal::I64(42))),
            })
        );
    }

    #[test]
    fn test_unary_not() {
        let prog = parse("not true").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(Expr::Literal(Literal::Bool(true))),
            })
        );
    }

    #[test]
    fn test_concat() {
        let prog = parse("\"a\" .. \"b\"").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Binary {
                lhs: Box::new(Expr::Literal(Literal::String("a".into()))),
                op: BinOp::Concat,
                rhs: Box::new(Expr::Literal(Literal::String("b".into()))),
            })
        );
    }

    #[test]
    fn test_power() {
        let prog = parse("2 ^ 3 ^ 2").unwrap();
        // right-associative: 2 ^ (3 ^ 2)
        match &prog[0] {
            Stmt::ExprStmt(Expr::Binary { lhs, op: BinOp::Pow, rhs }) => {
                assert_eq!(lhs.as_ref(), &Expr::Literal(Literal::I64(2)));
                match rhs.as_ref() {
                    Expr::Binary { lhs: rl, op: BinOp::Pow, rhs: rr } => {
                        assert_eq!(rl.as_ref(), &Expr::Literal(Literal::I64(3)));
                        assert_eq!(rr.as_ref(), &Expr::Literal(Literal::I64(2)));
                    }
                    _ => panic!("expected nested pow"),
                }
            }
            _ => panic!("expected pow"),
        }
    }

    #[test]
    fn test_chained_calls() {
        let prog = parse("foo.bar:baz(x).qux").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::FieldAccess { obj, field }) => {
                assert_eq!(field, "qux");
                match obj.as_ref() {
                    Expr::MethodCall { obj: o, method: m, args: _ } => {
                        assert_eq!(o, "bar");
                        assert_eq!(m, "baz");
                    }
                    _ => panic!("expected MethodCall"),
                }
            }
            _ => panic!("expected FieldAccess"),
        }
    }

    #[test]
    fn test_empty_fn() {
        let prog = parse("function empty() end").unwrap();
        assert_eq!(prog[0], Stmt::FunctionDecl {
            name: "empty".into(),
            params: vec![],
            body: vec![],
        });
    }

    #[test]
    fn test_nested_blocks() {
        let src = "
if x > 0 then
    if y > 0 then
        return x + y
    end
end
";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::If { then_body, .. } => {
                assert_eq!(then_body.len(), 1);
                match &then_body[0] {
                    Stmt::If { .. } => {}
                    _ => panic!("expected nested If"),
                }
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_elseif() {
        let src = "
if a == 1 then
    return \"one\"
elseif a == 2 then
    return \"two\"
else
    return \"other\"
end
";
        let prog = parse(src).unwrap();
        match &prog[0] {
            Stmt::If { elseif_branches, else_body, .. } => {
                assert_eq!(elseif_branches.len(), 1);
                assert!(!else_body.is_empty());
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_multiple_returns() {
        let prog = parse("return 1, 2, 3").unwrap();
        assert_eq!(
            prog[0],
            Stmt::Return {
                exprs: vec![
                    Expr::Literal(Literal::I64(1)),
                    Expr::Literal(Literal::I64(2)),
                    Expr::Literal(Literal::I64(3)),
                ],
            }
        );
    }

    #[test]
    fn test_naked_return() {
        let prog = parse("function f() return end").unwrap();
        match &prog[0] {
            Stmt::FunctionDecl { body, .. } => {
                assert_eq!(body[0], Stmt::Return { exprs: vec![] });
            }
            _ => panic!("expected FunctionDecl"),
        }
    }

    #[test]
    fn test_multi_local() {
        let prog = parse("local a, b = 1, 2").unwrap();
        assert_eq!(
            prog[0],
            Stmt::VarDecl {
                names: vec!["a".into(), "b".into()],
                init: Some(vec![
                    Expr::Literal(Literal::I64(1)),
                    Expr::Literal(Literal::I64(2)),
                ]),
                is_local: true,
                persist: false,
            }
        );
    }

    #[test]
    fn test_expr_in_parens() {
        let prog = parse("(1 + 2) * 3").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Binary {
                lhs: Box::new(Expr::Binary {
                    lhs: Box::new(Expr::Literal(Literal::I64(1))),
                    op: BinOp::Add,
                    rhs: Box::new(Expr::Literal(Literal::I64(2))),
                }),
                op: BinOp::Mul,
                rhs: Box::new(Expr::Literal(Literal::I64(3))),
            })
        );
    }

    #[test]
    fn test_idiv_mod() {
        let prog = parse("10 // 3 % 2").unwrap();
        // (10 // 3) % 2 — same precedence, left-assoc
        match &prog[0] {
            Stmt::ExprStmt(Expr::Binary { lhs, op: BinOp::Mod, rhs }) => {
                match lhs.as_ref() {
                    Expr::Binary { lhs: ll, op: BinOp::IDiv, rhs: lr } => {
                        assert_eq!(ll.as_ref(), &Expr::Literal(Literal::I64(10)));
                        assert_eq!(lr.as_ref(), &Expr::Literal(Literal::I64(3)));
                    }
                    _ => panic!("expected IDiv"),
                }
                assert_eq!(rhs.as_ref(), &Expr::Literal(Literal::I64(2)));
            }
            _ => panic!("expected Mod"),
        }
    }

    #[test]
    fn test_hex_number() {
        let prog = parse("0xff").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::I64(255))));
    }

    #[test]
    fn test_semi_stmt() {
        let prog = parse(";").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::Nil)));
    }

    #[test]
    fn test_table_field_access() {
        let prog = parse("t[\"key\"]").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Index {
                obj: Box::new(Expr::Ident("t".into())),
                index: Box::new(Expr::Literal(Literal::String("key".into()))),
            })
        );
    }

    #[test]
    fn test_unary_not_precedence() {
        // In Lua, `not` has HIGHER precedence than comparison.
        // `not a > b` parses as `(not a) > b`.
        let prog = parse("not a > b").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Binary { lhs, op: BinOp::Gt, rhs }) => {
                match lhs.as_ref() {
                    Expr::Unary { op: UnaryOp::Not, expr } => {
                        assert_eq!(expr.as_ref(), &Expr::Ident("a".into()));
                    }
                    _ => panic!("expected unary not"),
                }
                assert_eq!(rhs.as_ref(), &Expr::Ident("b".into()));
            }
            _ => panic!("expected comparison"),
        }
    }

    // ── Edge case parser tests ──

    #[test]
    fn test_empty_input() {
        let prog = parse("").unwrap();
        assert!(prog.is_empty());
    }

    #[test]
    fn test_whitespace_only() {
        let prog = parse("   \n  \t  ").unwrap();
        assert!(prog.is_empty());
    }

    #[test]
    fn test_comment_only() {
        let result = parse("-- comment\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiline_string() {
        // QFL doesn't support [[ long strings
        let result = parse("local s = [[hello\nworld]]");
        assert!(result.is_err());
    }

    #[test]
    fn test_escape_string() {
        let prog = parse("\"hello\\nworld\"").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::String("hello\nworld".into()))));
    }

    #[test]
    fn test_nested_parens() {
        let prog = parse("((((42))))").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::I64(42))));
    }

    #[test]
    fn test_trailing_semicolon() {
        let prog = parse("local x = 1;").unwrap();
        assert_eq!(prog.len(), 2);
        assert_eq!(prog[1], Stmt::ExprStmt(Expr::Literal(Literal::Nil)));
    }

    #[test]
    fn test_consecutive_semicolons() {
        let prog = parse(";;;").unwrap();
        assert_eq!(prog.len(), 3);
    }

    #[test]
    fn test_semicolon_in_statements() {
        let prog = parse("local a = 1; local b = 2").unwrap();
        // semicolon is an empty statement (Nil), so we get 3 stmts
        assert_eq!(prog.len(), 3);
    }

    #[test]
    fn test_float_scientific() {
        // QFL doesn't support scientific notation, parse "1.5e10" as number "1.5" then ident "e10"
        let result = parse("1.5e10");
        assert!(result.is_ok());
    }

    #[test]
    fn test_float_small() {
        let prog = parse("0.0001").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::F64(0.0001))));
    }

    #[test]
    fn test_negative_float() {
        let prog = parse("-3.14").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Literal(Literal::F64(3.14))),
        }));
    }

    #[test]
    fn test_nil() {
        let prog = parse("nil").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::Nil)));
    }

    #[test]
    fn test_bool_expr() {
        let prog = parse("true and false or not true").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_vararg() {
        // QFL doesn't support varargs
        let result = parse("function f(...) return ... end");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_table() {
        let prog = parse("t = {}").unwrap();
        match &prog[0] {
            Stmt::Assign { targets, exprs } => {
                assert_eq!(targets.len(), 1);
                match &exprs[0] {
                    Expr::Table(fields) => assert!(fields.is_empty()),
                    _ => panic!("expected Table"),
                }
            }
            _ => panic!("expected Assign"),
        }
    }

    #[test]
    fn test_mixed_table() {
        let prog = parse("{1, key = \"val\", [2] = \"two\"}").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Table(fields)) => {
                assert_eq!(fields.len(), 3);
            }
            _ => panic!("expected Table"),
        }
    }

    #[test]
    fn test_complex_cond_in_if() {
        let prog = parse("if a > 0 and b < 10 or not c then end").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_fn_decl_no_params() {
        let prog = parse("function f() return 42 end").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::FunctionDecl { name, params, body } => {
                assert_eq!(name, "f");
                assert!(params.is_empty());
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected FunctionDecl"),
        }
    }

    #[test]
    fn test_fn_decl_many_params() {
        let params = (0..50).map(|i| format!("p{}", i)).collect::<Vec<_>>().join(", ");
        let src = format!("function f({}) end", params);
        let prog = parse(&src).unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_assign_chained() {
        // QFL doesn't support chained assignment
        let result = parse("a = b = 42");
        assert!(result.is_err());
    }

    #[test]
    fn test_or_short_circuit() {
        let prog = parse("true or false").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_and_short_circuit() {
        let prog = parse("false and true").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_concat_multi() {
        let prog = parse("\"a\" .. \"b\" .. \"c\"").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_index_expr() {
        let prog = parse("t[1]").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Index { obj, index }) => {
                assert_eq!(obj.as_ref(), &Expr::Ident("t".into()));
                assert_eq!(index.as_ref(), &Expr::Literal(Literal::I64(1)));
            }
            _ => panic!("expected Index"),
        }
    }

    #[test]
    fn test_complex_field_access() {
        let prog = parse("a.b.c.d.e").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_long_identifier() {
        let long = "a".repeat(256);
        let prog = parse(&format!("{} = 1", long)).unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_zero_as_float() {
        let prog = parse("0.0").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::F64(0.0))));
    }

    #[test]
    fn test_large_integer() {
        let prog = parse("9999999999999").unwrap();
        assert_eq!(prog.len(), 1);
    }

    // ── Syntax edge cases ──

    #[test]
    fn test_empty_block_in_fn() {
        let prog = parse("function f() end").unwrap();
        assert_eq!(prog[0], Stmt::FunctionDecl {
            name: "f".into(),
            params: vec![],
            body: vec![],
        });
    }

    #[test]
    fn test_if_without_else() {
        let prog = parse("if 1 then return 42 end").unwrap();
        match &prog[0] {
            Stmt::If { cond, then_body, elseif_branches, else_body } => {
                assert_eq!(cond.as_ref(), &Expr::Literal(Literal::I64(1)));
                assert_eq!(then_body.len(), 1);
                assert!(elseif_branches.is_empty());
                assert!(else_body.is_empty());
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_if_elseif_only() {
        let prog = parse("if 1 then elseif 2 then end").unwrap();
        match &prog[0] {
            Stmt::If { cond, then_body, elseif_branches, else_body } => {
                assert_eq!(cond.as_ref(), &Expr::Literal(Literal::I64(1)));
                assert!(then_body.is_empty());
                assert_eq!(elseif_branches.len(), 1);
                assert!(else_body.is_empty());
            }
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn test_nested_if_three_levels() {
        let src = "
if 1 then
    if 2 then
        if 3 then
            return 99
        end
    end
end
";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::If { then_body, .. } => {
                assert_eq!(then_body.len(), 1);
                match &then_body[0] {
                    Stmt::If { then_body: inner2, .. } => {
                        assert_eq!(inner2.len(), 1);
                        match &inner2[0] {
                            Stmt::If { then_body: inner3, .. } => {
                                assert_eq!(inner3.len(), 1);
                            }
                            _ => panic!("expected third If"),
                        }
                    }
                    _ => panic!("expected second If"),
                }
            }
            _ => panic!("expected first If"),
        }
    }

    #[test]
    fn test_while_with_complex_body() {
        let src = "
while 1 do
    if x > 0 then
        x = x - 1
    else
        break
    end
end
";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::While { cond, body } => {
                assert_eq!(cond.as_ref(), &Expr::Literal(Literal::I64(1)));
                assert_eq!(body.len(), 1);
                match &body[0] {
                    Stmt::If { then_body, else_body, .. } => {
                        assert_eq!(then_body.len(), 1);
                        assert_eq!(else_body.len(), 1);
                    }
                    _ => panic!("expected If inside while"),
                }
            }
            _ => panic!("expected While"),
        }
    }

    #[test]
    fn test_repeat_with_side_effects() {
        let src = "
repeat
    x = x + 1
    y = y * 2
until x >= 10
";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::Repeat { body, until } => {
                assert_eq!(body.len(), 2);
                assert_eq!(until.as_ref(), &Expr::Binary {
                    lhs: Box::new(Expr::Ident("x".into())),
                    op: BinOp::Ge,
                    rhs: Box::new(Expr::Literal(Literal::I64(10))),
                });
            }
            _ => panic!("expected Repeat"),
        }
    }

    #[test]
    fn test_for_with_float_bounds() {
        let prog = parse("for i = 1.5, 10.5 do end").unwrap();
        match &prog[0] {
            Stmt::ForNum { var, from, to, step, body } => {
                assert_eq!(var, "i");
                assert_eq!(from.as_ref(), &Expr::Literal(Literal::F64(1.5)));
                assert_eq!(to.as_ref(), &Expr::Literal(Literal::F64(10.5)));
                assert!(step.is_none());
                assert!(body.is_empty());
            }
            _ => panic!("expected ForNum"),
        }
    }

    #[test]
    fn test_for_with_zero_step() {
        let prog = parse("for i = 1, 10, 0 do end").unwrap();
        match &prog[0] {
            Stmt::ForNum { step, .. } => {
                assert!(step.is_some());
                assert_eq!(step.as_ref().unwrap().as_ref(), &Expr::Literal(Literal::I64(0)));
            }
            _ => panic!("expected ForNum"),
        }
    }

    #[test]
    fn test_multiple_vars_in_for_in() {
        let prog = parse("for k, v in pairs(t) do end").unwrap();
        match &prog[0] {
            Stmt::ForIn { vars, exprs, body } => {
                assert_eq!(vars, &vec!["k".to_string(), "v".to_string()]);
                assert_eq!(exprs.len(), 1);
                assert!(body.is_empty());
            }
            _ => panic!("expected ForIn"),
        }
    }

    #[test]
    fn test_lua_style_comments() {
        // Multi-line comment token is consumed; code after parses normally
        let prog = parse("--[[ multi-line comment ]]\nlocal x = 1").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::VarDecl { names, init, is_local, persist } => {
                assert_eq!(names, &vec!["x".to_string()]);
                assert!(is_local);
                assert!(!persist);
                assert!(init.is_some());
            }
            _ => panic!("expected VarDecl"),
        }
    }

    // ── Error recovery ──

    #[test]
    fn test_unexpected_token() {
        let result = parse("local if = 1");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_then() {
        let result = parse("if 1\n return end");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_do() {
        let result = parse("while 1\n end");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_end() {
        let result = parse("if 1 then");
        assert!(result.is_err());
    }

    #[test]
    fn test_extra_end() {
        let result = parse("if 1 then end end");
        assert!(result.is_err());
    }

    #[test]
    fn test_unclosed_brace_in_table() {
        let result = parse("{1, 2, 3");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_parens() {
        let result = parse("()");
        assert!(result.is_err());
    }

    #[test]
    fn test_trailing_comma_in_table() {
        let prog = parse("{1, 2, 3,}").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Table(fields)) => {
                assert_eq!(fields.len(), 3);
            }
            _ => panic!("expected Table"),
        }
    }

    // ── Expression edge cases ──

    #[test]
    fn test_consecutive_unary_ops() {
        let prog = parse("not not true").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(Expr::Literal(Literal::Bool(true))),
                }),
            })
        );
    }

    #[test]
    fn test_negate_expression() {
        let prog = parse("-(1 + 2)").unwrap();
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Binary {
                    lhs: Box::new(Expr::Literal(Literal::I64(1))),
                    op: BinOp::Add,
                    rhs: Box::new(Expr::Literal(Literal::I64(2))),
                }),
            })
        );
    }

    #[test]
    fn test_multiple_assignments() {
        let prog = parse("a, b, c = 1, 2, 3").unwrap();
        assert_eq!(
            prog[0],
            Stmt::Assign {
                targets: vec![
                    Expr::Ident("a".into()),
                    Expr::Ident("b".into()),
                    Expr::Ident("c".into()),
                ],
                exprs: vec![
                    Expr::Literal(Literal::I64(1)),
                    Expr::Literal(Literal::I64(2)),
                    Expr::Literal(Literal::I64(3)),
                ],
            }
        );
    }

    #[test]
    fn test_assign_to_field() {
        let prog = parse("t.key = 1").unwrap();
        assert_eq!(
            prog[0],
            Stmt::Assign {
                targets: vec![Expr::FieldAccess {
                    obj: Box::new(Expr::Ident("t".into())),
                    field: "key".into(),
                }],
                exprs: vec![Expr::Literal(Literal::I64(1))],
            }
        );
    }

    #[test]
    fn test_index_on_lhs() {
        let prog = parse("t[1] = 42").unwrap();
        assert_eq!(
            prog[0],
            Stmt::Assign {
                targets: vec![Expr::Index {
                    obj: Box::new(Expr::Ident("t".into())),
                    index: Box::new(Expr::Literal(Literal::I64(1))),
                }],
                exprs: vec![Expr::Literal(Literal::I64(42))],
            }
        );
    }

    #[test]
    fn test_chained_method_calls() {
        let prog = parse("obj:method1():method2()").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::MethodCall { obj, method, args }) => {
                assert_eq!(method, "method2");
                assert!(args.is_empty());
                // obj should be "method1" (result of first method call)
                assert_eq!(obj, "method1");
            }
            _ => panic!("expected MethodCall"),
        }
    }

    #[test]
    fn test_very_long_expression() {
        let expr: String = (1..=100).map(|i| i.to_string()).collect::<Vec<_>>().join(" + ");
        let prog = parse(&expr).unwrap();
        assert_eq!(prog.len(), 1);
        // Walk the left-associative tree without borrowing temporaries
        fn count_add_depth(stmt: &Stmt) -> u32 {
            match stmt {
                Stmt::ExprStmt(Expr::Binary { lhs, op: BinOp::Add, .. }) => {
                    1 + count_add_depth(&Stmt::ExprStmt(*lhs.clone()))
                }
                _ => 0,
            }
        }
        assert_eq!(count_add_depth(&prog[0]), 99);
    }
}
