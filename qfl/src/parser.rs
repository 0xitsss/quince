use crate::ast::*;
use crate::lexer::Token;

// --- Section: ParseError ---

// Error produced during parsing, carrying the error message and token position.
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

// --- Section: Parser struct ---

// Pratt parser that transforms a token stream into a QFL AST (Program).
// Uses precedence-based expression parsing with a dispatch-on-first-token approach for statements.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    // --- Section: Constructors ---

    // Creates a new parser from a tokenised source, starting at position 0.
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    // --- Section: Top-level parse entry point ---

    // Parses the full token stream into a QFL program (list of statements).
    pub fn parse(&mut self) -> Result<Program, ParseError> {
        let mut stmts = Vec::new();
        while !self.is_eof() {
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    // --- Section: Helper methods ---

    // Returns the current token without consuming it; Eof if exhausted.
    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    // Returns the token at offset n ahead without consuming; Eof if out of range.
    fn peek_at(&self, n: usize) -> &Token {
        self.tokens.get(self.pos + n).unwrap_or(&Token::Eof)
    }

    // Returns true if the parser has reached the end of the token stream.
    fn is_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    // Advances the token cursor by one position (if not already at end).
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    // If the current token matches expected, consumes it; otherwise returns an error.
    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(self.err(&format!("expected {}, got {}", expected, self.peek())))
        }
    }

    // Wraps a message string into a ParseError with the current token position.
    fn err(&self, msg: &str) -> ParseError {
        ParseError {
            msg: msg.to_string(),
            pos: self.pos,
        }
    }

    // Consumes and returns an Ident token; returns an error if current token is not an identifier.
    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(s) => {
                self.advance();
                Ok(s)
            }
            ref other => Err(self.err(&format!("expected identifier, got {}", other))),
        }
    }

    // --- Section: Statement dispatch ---

    // Dispatches to the appropriate statement parser based on the first token.
    // Handles optional @persist prefix, comments, and falls back to assign-or-call.
    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        // Check for @persist modifier; if present, consume it and pass along.
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
                // Empty statement (just a semicolon) produces a nil expression statement.
                self.advance();
                Ok(Stmt::ExprStmt(Expr::Literal(Literal::Nil)))
            }
            Token::Comment(_) => {
                // Comments are skipped; recursively parse the next meaningful statement.
                self.advance();
                self.parse_stmt()
            }
            Token::AtUsing => self.parse_using(),
            Token::AtWindow => self.parse_window(),
            Token::State => self.parse_state(),
            Token::On => self.parse_event_handler(),
            Token::Fn => self.parse_fn_decl(),
            Token::Ident(s) if s == "feature" => self.parse_feature(),
            Token::Ident(s) if s == "signal" => self.parse_signal(),
            _ => {
                // Bare @persist without `local` counts as a non-local var decl.
                if persist {
                    self.parse_var_decl(false, true)
                } else {
                    // Anything else is treated as an assignment or a function-call expression statement.
                    self.parse_assign_or_call()
                }
            }
        }
    }

    // --- Section: Variable declaration ---

    // Parses `[local] name [, name]... [= expr [, expr]...]`.
    // is_local and persist are determined by the caller (parse_stmt).
    fn parse_var_decl(&mut self, is_local: bool, persist: bool) -> Result<Stmt, ParseError> {
        let names = self.parse_name_list()?;
        let init = if matches!(self.peek(), Token::Eq) {
            self.advance();
            Some(self.parse_expr_list()?)
        } else {
            None
        };
        Ok(Stmt::VarDecl {
            names,
            init,
            is_local,
            persist,
        })
    }

    // Parses a comma-separated list of identifiers.
    fn parse_name_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut names = Vec::new();
        names.push(self.expect_ident()?);
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            names.push(self.expect_ident()?);
        }
        Ok(names)
    }

    // --- Section: Numeric literal helper ---

    // Consumes and returns a numeric token as f64 (integers are widened).
    fn parse_number_value(&mut self) -> Result<f64, ParseError> {
        match self.peek().clone() {
            Token::Number(s) => {
                self.advance();
                parse_number(&s).and_then(|lit| match lit {
                    Literal::F64(v) => Ok(v),
                    Literal::I64(v) => Ok(v as f64),
                    _ => Err(self.err("expected number")),
                })
            }
            ref tok => Err(self.err(&format!("expected number, got {}", tok))),
        }
    }

    // --- Section: @using directive ---

    // Parses `@using name[:param[:param...]] name[:param...] ...`
    // Each indicator optionally carries colon-separated numeric parameters.
    fn parse_using(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let mut indicators = Vec::new();
        // Consume identifiers as long as they appear (each is a using entry).
        while matches!(self.peek(), Token::Ident(_)) {
            let name = self.expect_ident()?.to_lowercase();
            let mut params = Vec::new();
            // After an identifier, optional colon-separated numeric parameters.
            if matches!(self.peek(), Token::Colon) {
                self.advance();
                params.push(self.parse_number_value()?);
                while matches!(self.peek(), Token::Colon) {
                    self.advance();
                    params.push(self.parse_number_value()?);
                }
            }
            indicators.push(crate::ast::UsingEntry { name, params });
        }
        Ok(Stmt::Using { indicators })
    }

    // --- Section: @window directive ---

    // Parses `@window name capacity`.
    fn parse_window(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        let cap = self.parse_number_value()? as usize;
        Ok(Stmt::Window {
            name,
            capacity: cap,
        })
    }

    // --- Section: feature declaration ---

    // Parses `feature name = expr`.
    fn parse_feature(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        self.expect(&Token::Eq)?;
        let expr = self.parse_expr()?;
        Ok(Stmt::Feature {
            name,
            expr: Box::new(expr),
        })
    }

    // --- Section: signal declaration ---

    // Parses `signal name = expr`.
    fn parse_signal(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        self.expect(&Token::Eq)?;
        let expr = self.parse_expr()?;
        Ok(Stmt::Signal {
            name,
            expr: Box::new(expr),
        })
    }

    // --- Section: state declaration/reassignment ---

    // Parses `state name : type [= expr]` (declaration) or `state name = expr` (reassignment).
    // If a colon follows the name, it is a typed declaration; otherwise it is an assignment.
    fn parse_state(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        if matches!(self.peek(), Token::Colon) {
            // Declaration: state name : type [= expr]
            self.advance();
            let type_name = self.expect_ident()?;
            let default = if matches!(self.peek(), Token::Eq) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            Ok(Stmt::State {
                name,
                type_name,
                default,
            })
        } else {
            // Reassignment: state name = expr
            self.expect(&Token::Eq)?;
            let expr = self.parse_expr()?;
            Ok(Stmt::Assign {
                targets: vec![crate::ast::Expr::Ident(name)],
                exprs: vec![expr],
            })
        }
    }

    // --- Section: event handler ---

    // Parses `on event(param?) { body }`.
    fn parse_event_handler(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let event = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        // Optional single parameter inside parentheses.
        let param = if matches!(self.peek(), Token::Ident(_)) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        self.expect(&Token::RParen)?;
        let body = self.parse_brace_block()?;
        Ok(Stmt::EventHandler { event, param, body })
    }

    // --- Section: brace block ---

    // Parses `{ stmts }` and returns the statements inside.
    fn parse_brace_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.expect(&Token::LBrace)?;
        let stmts = self.parse_block_until(&[Token::RBrace])?;
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    // --- Section: typed function declaration (new syntax) ---

    // Parses `fn name(params) -> type { body }`.
    // Parameters may optionally have type annotations; default type is "i64".
    fn parse_fn_decl(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        // Parse optional parameter list with optional `: type` annotations.
        if !matches!(self.peek(), Token::RParen) {
            let p_name = self.expect_ident()?;
            let p_type = if matches!(self.peek(), Token::Colon) {
                self.advance();
                self.expect_ident()?
            } else {
                "i64".into()
            };
            params.push(crate::ast::FnParam {
                name: p_name,
                type_name: p_type,
            });
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                let p_name = self.expect_ident()?;
                let p_type = if matches!(self.peek(), Token::Colon) {
                    self.advance();
                    self.expect_ident()?
                } else {
                    "i64".into()
                };
                params.push(crate::ast::FnParam {
                    name: p_name,
                    type_name: p_type,
                });
            }
        }
        self.expect(&Token::RParen)?;
        // Optional `-> return_type` (defaults to "i64").
        let return_type = if matches!(self.peek(), Token::Arrow) {
            self.advance();
            self.expect_ident()?
        } else {
            "i64".into()
        };
        let body = self.parse_brace_block()?;
        Ok(Stmt::FnDecl {
            name,
            params,
            return_type,
            body,
        })
    }

    // --- Section: legacy function declaration ---

    // Parses `function name(params) body end` (Lua-style, no type annotations).
    fn parse_function_decl(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        // Parse comma-separated parameter names (identifiers only, no types).
        if !matches!(self.peek(), Token::RParen) {
            params.push(self.expect_ident()?);
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                params.push(self.expect_ident()?);
            }
        }
        self.expect(&Token::RParen)?;
        // Body is a block terminated by `end`.
        let body = self.parse_block_until(&[Token::End])?;
        self.expect(&Token::End)?;
        Ok(Stmt::FunctionDecl { name, params, body })
    }

    // --- Section: if statement ---

    // Parses `if cond { body } [elseif cond { body }]* [else { body }]` (brace syntax)
    // or `if cond then body [elseif cond then body]* [else body] end` (legacy syntax).
    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let cond = Box::new(self.parse_expr()?);

        // Brace syntax: if cond { body } [elseif cond { body }]* [else { body }]
        if matches!(self.peek(), Token::LBrace) {
            let then_body = self.parse_brace_block()?;

            let mut elseif_branches = Vec::new();
            let mut else_body = Vec::new();

            while matches!(self.peek(), Token::ElseIf) {
                self.advance();
                let econd = Box::new(self.parse_expr()?);
                let ebody = self.parse_brace_block()?;
                elseif_branches.push((econd, ebody));
            }

            if matches!(self.peek(), Token::Else) {
                self.advance();
                else_body = self.parse_brace_block()?;
            }

            return Ok(Stmt::If {
                cond,
                then_body,
                elseif_branches,
                else_body,
            });
        }

        // Legacy syntax: if cond then body [elseif ...] [else ...] end
        self.expect(&Token::Then)?;
        // Parse then-body until we hit else, elseif, or end.
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
        Ok(Stmt::If {
            cond,
            then_body,
            elseif_branches,
            else_body,
        })
    }

    // --- Section: while loop ---

    // Parses `while cond do body end`.
    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let cond = Box::new(self.parse_expr()?);
        self.expect(&Token::Do)?;
        let body = self.parse_block_until(&[Token::End])?;
        self.expect(&Token::End)?;
        Ok(Stmt::While { cond, body })
    }

    // --- Section: repeat-until loop ---

    // Parses `repeat body until cond` (body executes at least once).
    fn parse_repeat(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let body = self.parse_block_until(&[Token::Until])?;
        // Consume the `until` token.
        self.advance();
        let cond = Box::new(self.parse_expr()?);
        Ok(Stmt::Repeat { body, until: cond })
    }

    // --- Section: for loop ---

    // Parses numeric `for var = from, to [, step] do body end`
    // or generic `for vars in exprs do body end`.
    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let first = self.expect_ident()?;
        if matches!(self.peek(), Token::Eq) {
            // Numeric for: for var = from, to [, step] do body end
            self.advance();
            let from = Box::new(self.parse_expr()?);
            self.expect(&Token::Comma)?;
            let to = Box::new(self.parse_expr()?);
            // Optional step (third expression after second comma).
            let step = if matches!(self.peek(), Token::Comma) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            self.expect(&Token::Do)?;
            let body = self.parse_block_until(&[Token::End])?;
            self.expect(&Token::End)?;
            Ok(Stmt::ForNum {
                var: first,
                from,
                to,
                step,
                body,
            })
        } else {
            // Generic for: for vars in exprs do body end
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

    // --- Section: return statement ---

    // Parses `return [expr [, expr]...]`.
    // Return is empty (no expressions) if the next token is a block terminator or EOF.
    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        // If the next token ends a block, there are no return values.
        let exprs = if self.is_eof()
            || matches!(
                self.peek(),
                Token::End | Token::Else | Token::ElseIf | Token::Until | Token::RBrace
            ) {
            Vec::new()
        } else {
            self.parse_expr_list()?
        };
        Ok(Stmt::Return { exprs })
    }

    // --- Section: assignment or call statement ---

    // Parses an assignment (single or multi-target) or falls through to an expression statement.
    // Handles `a = 1`, `a, b = 1, 2`, `a.b = 1`, and bare expression calls.
    fn parse_assign_or_call(&mut self) -> Result<Stmt, ParseError> {
        // Parse LHS as a full expression — needed for binary ops like `a + b`.
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
            return Ok(Stmt::Assign {
                targets: vec![first],
                exprs,
            });
        }

        // No assignment operator found — treat as a bare expression statement (e.g. function call).
        Ok(Stmt::ExprStmt(first))
    }

    // --- Section: block parsing helper ---

    // Collects statements until EOF or one of the delimiter tokens is reached.
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

    // --- Section: Expression parsing (Pratt / precedence climbing) ---

    // Entry point for expression parsing; starts at the lowest precedence.
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(0)
    }

    // Parses a comma-separated list of expressions.
    fn parse_expr_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut exprs = Vec::new();
        exprs.push(self.parse_expr()?);
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }

    // --- Section: Binary operator parsing (precedence climbing) ---

    // Precedence climbing for binary operators.
    // min_prec: minimum precedence to accept (higher = tighter binding).
    fn parse_binary(&mut self, min_prec: u32) -> Result<Expr, ParseError> {
        // Parse the left-hand side which may start with a unary or prefix operator.
        let mut lhs = self.parse_unary()?;

        loop {
            // Look up the current token in the precedence table.
            // Each entry: (operator, precedence, right_associative).
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
                // No operator found; this expression is complete.
                None => break,
            };

            // Stop if this operator binds less tightly than our minimum.
            if prec < min_prec {
                break;
            }

            // Consume the operator token.
            self.advance();
            // For right-associative operators, recurse at the same precedence;
            // for left-associative, recurse one level higher.
            let next_prec = if right_assoc { prec } else { prec + 1 };
            let rhs = self.parse_binary(next_prec)?;
            lhs = Expr::Binary {
                lhs: Box::new(lhs),
                op,
                rhs: Box::new(rhs),
            };
        }

        Ok(lhs)
    }

    // --- Section: Unary and power expression parsing ---

    // Unary ops (negation, not, length) + right-associative power.
    // `not` has higher precedence than comparisons in this parser.
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        // Prefix unary operators: -, not, # (length).
        match self.peek() {
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                });
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                });
            }
            Token::Hash => {
                self.advance();
                let expr = self.parse_unary()?;
                return Ok(Expr::Unary {
                    op: UnaryOp::Len,
                    expr: Box::new(expr),
                });
            }
            _ => {}
        }

        // Power: right-associative, so parse postfix first then optionally consume `^`.
        let mut lhs = self.parse_postfix()?;
        if matches!(self.peek(), Token::Caret) {
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Binary {
                lhs: Box::new(lhs),
                op: BinOp::Pow,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    // --- Section: Postfix expression parsing ---

    // Postfix: field access (.name), method call (:name(args)), index ([expr]), function call (args).
    // Chains repeatedly until no postfix operator is found.
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        // Start with a prefix expression, then apply postfix operators in a loop.
        let mut expr = self.parse_prefix()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    // Field access: obj.field
                    self.advance();
                    let field = self.expect_ident()?;
                    expr = Expr::FieldAccess {
                        obj: Box::new(expr),
                        field,
                    };
                }
                Token::Colon => {
                    // Method call: obj:method(args)
                    self.advance();
                    let method = self.expect_ident()?;
                    self.expect(&Token::LParen)?;
                    let args = if matches!(self.peek(), Token::RParen) {
                        Vec::new()
                    } else {
                        self.parse_expr_list()?
                    };
                    self.expect(&Token::RParen)?;
                    // Determine the object name for the MethodCall node:
                    // unwrap nested FieldAccess/MethodCall to find the chainable name.
                    let obj = match &expr {
                        Expr::Ident(name) => name.clone(),
                        Expr::FieldAccess { field, .. } => field.clone(),
                        Expr::MethodCall { method: m, .. } => m.clone(),
                        _ => return Err(self.err("method call requires an object")),
                    };
                    expr = Expr::MethodCall { obj, method, args };
                }
                Token::LBracket => {
                    // Index: obj[expr]
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    expr = Expr::Index {
                        obj: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                Token::LParen => {
                    // Function call: fn(args)
                    self.advance();
                    let args = if matches!(self.peek(), Token::RParen) {
                        Vec::new()
                    } else {
                        self.parse_expr_list()?
                    };
                    self.expect(&Token::RParen)?;
                    // Dispatch based on the preceding expression to decide between FnCall and MethodCall.
                    match expr {
                        Expr::Ident(name) => {
                            expr = Expr::FnCall { name, args };
                        }
                        Expr::FieldAccess { obj, field } => {
                            // obj.field(args) — sugar for obj:field(args)
                            let obj_name = match obj.as_ref() {
                                Expr::Ident(s) => s.clone(),
                                Expr::FieldAccess { field: f, .. } => f.clone(),
                                other => {
                                    return Err(
                                        self.err(&format!("cannot call method on {:?}", other))
                                    )
                                }
                            };
                            expr = Expr::MethodCall {
                                obj: obj_name,
                                method: field,
                                args,
                            };
                        }
                        _ => return Err(self.err("cannot call non-function expression")),
                    }
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    // --- Section: Prefix / primary expression parsing ---

    // Parses the atomic prefix of an expression: literals, identifiers, parenthesized exprs, tables, vararg.
    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek().clone();
        match tok {
            Token::Nil => {
                self.advance();
                Ok(Expr::Literal(Literal::Nil))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(false)))
            }
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
                // Parenthesized expression: ( expr )
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBrace => self.parse_table(),
            Token::VarArg => {
                // Vararg (...) is treated as the identifier "...".
                self.advance();
                Ok(Expr::Ident("...".to_string()))
            }
            ref other => Err(self.err(&format!("unexpected token in expression: {}", other))),
        }
    }

    // --- Section: Table constructor parsing ---

    // Parses `{ [field [, field]...] }`.
    // Fields can be: list-style (value), key-value with `[expr] = expr`, or record-style `name = expr`.
    fn parse_table(&mut self) -> Result<Expr, ParseError> {
        self.advance();
        let mut fields = Vec::new();
        while !matches!(self.peek(), Token::RBrace) && !self.is_eof() {
            if matches!(self.peek(), Token::LBracket) {
                // [key] = value  (computed-key field)
                self.advance();
                let key = self.parse_expr()?;
                self.expect(&Token::RBracket)?;
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                fields.push(TableField::KeyValue { key, value });
            } else if matches!(self.peek_at(1), Token::Eq) {
                // name = value  (record-style field — name becomes a string key)
                let name = self.expect_ident()?;
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                fields.push(TableField::KeyValue {
                    key: Expr::Literal(Literal::String(name)),
                    value,
                });
            } else {
                // Bare value (list-style field).
                let value = self.parse_expr()?;
                fields.push(TableField::Value(value));
            }
            // Optional comma or semicolon separator between fields.
            if matches!(self.peek(), Token::Comma | Token::Semi) {
                self.advance();
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(Expr::Table(fields))
    }
}

// --- Section: Number literal parser (free function) ---

// Converts a numeric string token into a Literal.
// Handles: integer, float (with '.' or 'e'/'E'), and hex (0x/0X).
fn parse_number(s: &str) -> Result<Literal, ParseError> {
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s.parse::<f64>().map(Literal::F64).map_err(|_| ParseError {
            msg: format!("invalid float: {}", s),
            pos: 0,
        })
    } else if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16)
            .map(Literal::I64)
            .map_err(|_| ParseError {
                msg: format!("invalid hex: {}", s),
                pos: 0,
            })
    } else {
        s.parse::<i64>().map(Literal::I64).map_err(|_| ParseError {
            msg: format!("invalid integer: {}", s),
            pos: 0,
        })
    }
}

// --- Section: Convenience parse function ---

// Tokenises and parses QFL source in one call.
pub fn parse(input: &str) -> Result<Program, ParseError> {
    let tokens = crate::lexer::tokenize(input).map_err(|e| ParseError { msg: e.msg, pos: 0 })?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}

// --- Section: Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic expression tests ──

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
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Literal(Literal::String("hello".into())))
        );
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

    // ── Variable declaration tests ──

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

    // ── Assignment tests ──

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

    // ── Legacy function declaration tests ──

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

    // ── Control flow tests ──

    #[test]
    fn test_if_stmt() {
        let src = "if x > 0 then return x else return -x end";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::If {
                cond,
                then_body,
                elseif_branches,
                else_body,
            } => {
                assert_eq!(
                    cond.as_ref(),
                    &Expr::Binary {
                        lhs: Box::new(Expr::Ident("x".into())),
                        op: BinOp::Gt,
                        rhs: Box::new(Expr::Literal(Literal::I64(0))),
                    }
                );
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
                assert_eq!(
                    cond.as_ref(),
                    &Expr::Binary {
                        lhs: Box::new(Expr::Ident("x".into())),
                        op: BinOp::Lt,
                        rhs: Box::new(Expr::Literal(Literal::I64(10))),
                    }
                );
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
                assert_eq!(
                    until.as_ref(),
                    &Expr::Binary {
                        lhs: Box::new(Expr::Ident("x".into())),
                        op: BinOp::Eq,
                        rhs: Box::new(Expr::Literal(Literal::I64(0))),
                    }
                );
            }
            _ => panic!("expected Repeat stmt"),
        }
    }

    // ── For loop tests ──

    #[test]
    fn test_numeric_for() {
        let prog = parse("for i = 1, 10 do print(i) end").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::ForNum {
                var,
                from,
                to,
                step,
                body,
            } => {
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
                assert_eq!(
                    step.as_ref().unwrap().as_ref(),
                    &Expr::Literal(Literal::I64(2))
                );
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

    // ── Function call tests ──

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

    // ── Field access tests ──

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

    // ── Table constructor tests ──

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

    // ── Unary operator tests ──

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

    // ── String concat and power tests ──

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
            Stmt::ExprStmt(Expr::Binary {
                lhs,
                op: BinOp::Pow,
                rhs,
            }) => {
                assert_eq!(lhs.as_ref(), &Expr::Literal(Literal::I64(2)));
                match rhs.as_ref() {
                    Expr::Binary {
                        lhs: rl,
                        op: BinOp::Pow,
                        rhs: rr,
                    } => {
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
                    Expr::MethodCall {
                        obj: o,
                        method: m,
                        args: _,
                    } => {
                        assert_eq!(o, "bar");
                        assert_eq!(m, "baz");
                    }
                    _ => panic!("expected MethodCall"),
                }
            }
            _ => panic!("expected FieldAccess"),
        }
    }

    // ── Edge case: empty function ──

    #[test]
    fn test_empty_fn() {
        let prog = parse("function empty() end").unwrap();
        assert_eq!(
            prog[0],
            Stmt::FunctionDecl {
                name: "empty".into(),
                params: vec![],
                body: vec![],
            }
        );
    }

    // ── Nested block tests ──

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
            Stmt::If {
                elseif_branches,
                else_body,
                ..
            } => {
                assert_eq!(elseif_branches.len(), 1);
                assert!(!else_body.is_empty());
            }
            _ => panic!("expected If"),
        }
    }

    // ── Return statement tests ──

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

    // ── Multi-variable local ──

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

    // ── Parenthesised expression ──

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

    // ── Integer division and modulo ──

    #[test]
    fn test_idiv_mod() {
        let prog = parse("10 // 3 % 2").unwrap();
        // (10 // 3) % 2 — same precedence, left-assoc
        match &prog[0] {
            Stmt::ExprStmt(Expr::Binary {
                lhs,
                op: BinOp::Mod,
                rhs,
            }) => {
                match lhs.as_ref() {
                    Expr::Binary {
                        lhs: ll,
                        op: BinOp::IDiv,
                        rhs: lr,
                    } => {
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

    // ── Hex literal ──

    #[test]
    fn test_hex_number() {
        let prog = parse("0xff").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::I64(255))));
    }

    // ── Semicolon as empty statement ──

    #[test]
    fn test_semi_stmt() {
        let prog = parse(";").unwrap();
        assert_eq!(prog[0], Stmt::ExprStmt(Expr::Literal(Literal::Nil)));
    }

    // ── Index expression ──

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

    // ── Unary not precedence ──

    #[test]
    fn test_unary_not_precedence() {
        // In Lua, `not` has HIGHER precedence than comparison.
        // `not a > b` parses as `(not a) > b`.
        let prog = parse("not a > b").unwrap();
        match &prog[0] {
            Stmt::ExprStmt(Expr::Binary {
                lhs,
                op: BinOp::Gt,
                rhs,
            }) => {
                match lhs.as_ref() {
                    Expr::Unary {
                        op: UnaryOp::Not,
                        expr,
                    } => {
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
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Literal(Literal::String("hello\nworld".into())))
        );
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
        assert_eq!(
            prog[0],
            Stmt::ExprStmt(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Literal(Literal::F64(3.14))),
            })
        );
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
        let params = (0..50)
            .map(|i| format!("p{}", i))
            .collect::<Vec<_>>()
            .join(", ");
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
        assert_eq!(
            prog[0],
            Stmt::FunctionDecl {
                name: "f".into(),
                params: vec![],
                body: vec![],
            }
        );
    }

    #[test]
    fn test_if_without_else() {
        let prog = parse("if 1 then return 42 end").unwrap();
        match &prog[0] {
            Stmt::If {
                cond,
                then_body,
                elseif_branches,
                else_body,
            } => {
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
            Stmt::If {
                cond,
                then_body,
                elseif_branches,
                else_body,
            } => {
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
                    Stmt::If {
                        then_body: inner2, ..
                    } => {
                        assert_eq!(inner2.len(), 1);
                        match &inner2[0] {
                            Stmt::If {
                                then_body: inner3, ..
                            } => {
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
                    Stmt::If {
                        then_body,
                        else_body,
                        ..
                    } => {
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
                assert_eq!(
                    until.as_ref(),
                    &Expr::Binary {
                        lhs: Box::new(Expr::Ident("x".into())),
                        op: BinOp::Ge,
                        rhs: Box::new(Expr::Literal(Literal::I64(10))),
                    }
                );
            }
            _ => panic!("expected Repeat"),
        }
    }

    #[test]
    fn test_for_with_float_bounds() {
        let prog = parse("for i = 1.5, 10.5 do end").unwrap();
        match &prog[0] {
            Stmt::ForNum {
                var,
                from,
                to,
                step,
                body,
            } => {
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
                assert_eq!(
                    step.as_ref().unwrap().as_ref(),
                    &Expr::Literal(Literal::I64(0))
                );
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
            Stmt::VarDecl {
                names,
                init,
                is_local,
                persist,
            } => {
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
        let expr: String = (1..=100)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" + ");
        let prog = parse(&expr).unwrap();
        assert_eq!(prog.len(), 1);
        // Walk the left-associative tree without borrowing temporaries
        fn count_add_depth(stmt: &Stmt) -> u32 {
            match stmt {
                Stmt::ExprStmt(Expr::Binary {
                    lhs,
                    op: BinOp::Add,
                    ..
                }) => 1 + count_add_depth(&Stmt::ExprStmt(*lhs.clone())),
                _ => 0,
            }
        }
        assert_eq!(count_add_depth(&prog[0]), 99);
    }

    // ── Phase 4g: state, event handler, typed fn tests ──

    #[test]
    fn test_state_decl() {
        let prog = parse("state x : f64 = 42.0\nstate y : i32\nstate z : qty").unwrap();
        assert_eq!(prog.len(), 3);
        match &prog[0] {
            Stmt::State {
                name,
                type_name,
                default,
            } => {
                assert_eq!(name, "x");
                assert_eq!(type_name, "f64");
                assert!(default.is_some());
            }
            _ => panic!("expected State"),
        }
        match &prog[1] {
            Stmt::State {
                name,
                type_name,
                default,
            } => {
                assert_eq!(name, "y");
                assert_eq!(type_name, "i32");
                assert!(default.is_none());
            }
            _ => panic!("expected State"),
        }
        match &prog[2] {
            Stmt::State {
                name, type_name, ..
            } => {
                assert_eq!(name, "z");
                assert_eq!(type_name, "qty");
            }
            _ => panic!("expected State"),
        }
    }

    #[test]
    fn test_event_handler() {
        let prog = parse("on trade(t) { local x = t.price }").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::EventHandler { event, param, body } => {
                assert_eq!(event, "trade");
                assert_eq!(param.as_deref(), Some("t"));
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected EventHandler"),
        }
    }

    #[test]
    fn test_event_handler_no_params() {
        let prog = parse("on eval() { quince.log(\"tick\") }").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::EventHandler { event, param, body } => {
                assert_eq!(event, "eval");
                assert_eq!(param, &None);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected EventHandler"),
        }
    }

    #[test]
    fn test_fn_typed_decl() {
        let prog = parse("fn calc(x: f64, y: f64) -> f64 { return x + y }").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::FnDecl {
                name,
                params,
                return_type,
                body,
            } => {
                assert_eq!(name, "calc");
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name, "x");
                assert_eq!(params[0].type_name, "f64");
                assert_eq!(params[1].name, "y");
                assert_eq!(params[1].type_name, "f64");
                assert_eq!(return_type, "f64");
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected FnDecl"),
        }
    }

    // ── Phase 4g parser tests ──

    #[test]
    fn test_at_using_directive() {
        let prog = parse("@using ema:12 ema:48 bbands:20:2.0").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::Using { indicators } => {
                assert_eq!(indicators.len(), 3);
                assert_eq!(indicators[0].name, "ema");
                assert_eq!(indicators[0].params, vec![12.0]);
                assert_eq!(indicators[1].name, "ema");
                assert_eq!(indicators[1].params, vec![48.0]);
                assert_eq!(indicators[2].name, "bbands");
                assert_eq!(indicators[2].params, vec![20.0, 2.0]);
            }
            _ => panic!("expected Using"),
        }
    }

    #[test]
    fn test_at_window_directive() {
        let prog = parse("@window midprice 512\n@window returns 2048").unwrap();
        assert_eq!(prog.len(), 2);
        match &prog[0] {
            Stmt::Window { name, capacity } => {
                assert_eq!(name, "midprice");
                assert_eq!(*capacity, 512);
            }
            _ => panic!("expected Window"),
        }
        match &prog[1] {
            Stmt::Window { name, capacity } => {
                assert_eq!(name, "returns");
                assert_eq!(*capacity, 2048);
            }
            _ => panic!("expected Window"),
        }
    }

    #[test]
    fn test_feature_decl() {
        let prog = parse("feature ema_fast = ema(midprice, 12)").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::Feature { name, expr } => {
                assert_eq!(name, "ema_fast");
                match expr.as_ref() {
                    Expr::FnCall {
                        name: fn_name,
                        args,
                    } => {
                        assert_eq!(fn_name, "ema");
                        assert_eq!(args.len(), 2);
                    }
                    _ => panic!("expected FnCall"),
                }
            }
            _ => panic!("expected Feature"),
        }
    }

    #[test]
    fn test_signal_decl() {
        let prog = parse("signal trend_up = ema_fast > ema_slow").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::Signal { name, expr } => {
                assert_eq!(name, "trend_up");
                match expr.as_ref() {
                    Expr::Binary { op, .. } => {
                        assert_eq!(*op, BinOp::Gt);
                    }
                    _ => panic!("expected Binary(Gt)"),
                }
            }
            _ => panic!("expected Signal"),
        }
    }

    #[test]
    fn test_feature_signal_not_reserved_inside_fn() {
        // feature and signal must NOT be reserved — usable as variable names inside functions
        let prog = parse("function on_trade() local feature = 1 local signal = 2 end").unwrap();
        assert_eq!(prog.len(), 1);
    }

    #[test]
    fn test_state_decl_all_types() {
        let prog = parse("state a : i64\nstate b : f64\nstate c : bool\nstate d : timestamp\nstate e : duration\nstate f : price\nstate g : qty\nstate h : symbol\nstate i : side\nstate j : order_id").unwrap();
        assert_eq!(prog.len(), 10);
    }

    #[test]
    fn test_state_without_default() {
        let prog = parse("state x : f64").unwrap();
        match &prog[0] {
            Stmt::State {
                name,
                type_name,
                default,
            } => {
                assert_eq!(name, "x");
                assert_eq!(type_name, "f64");
                assert!(default.is_none());
            }
            _ => panic!("expected State"),
        }
    }

    #[test]
    fn test_event_handler_trade_with_field_access() {
        let prog = parse("on trade(t) { local p = t.price local q = t.qty }").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::EventHandler { event, param, body } => {
                assert_eq!(event, "trade");
                assert_eq!(param.as_deref(), Some("t"));
                assert_eq!(body.len(), 2);
            }
            _ => panic!("expected EventHandler"),
        }
    }

    #[test]
    fn test_event_handler_fill_with_field_access() {
        let prog = parse("on fill(f) { local p = f.price local q = f.qty }").unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::EventHandler { event, param, body } => {
                assert_eq!(event, "fill");
                assert_eq!(param.as_deref(), Some("f"));
                assert_eq!(body.len(), 2);
            }
            _ => panic!("expected EventHandler"),
        }
    }

    #[test]
    fn test_event_handler_no_body() {
        let prog = parse("on eval() {}").unwrap();
        match &prog[0] {
            Stmt::EventHandler { event, body, .. } => {
                assert_eq!(event, "eval");
                assert!(body.is_empty());
            }
            _ => panic!("expected EventHandler"),
        }
    }

    #[test]
    fn test_fn_typed_with_empty_body() {
        let prog = parse("fn empty() -> i64 {}").unwrap();
        match &prog[0] {
            Stmt::FnDecl {
                name,
                return_type,
                body,
                ..
            } => {
                assert_eq!(name, "empty");
                assert_eq!(return_type, "i64");
                assert!(body.is_empty());
            }
            _ => panic!("expected FnDecl"),
        }
    }

    #[test]
    fn test_multiline_event_handler() {
        let src = "
on eval() {
    local x = 1
    local y = 2
    local z = x + y
}
";
        let prog = parse(src).unwrap();
        assert_eq!(prog.len(), 1);
        match &prog[0] {
            Stmt::EventHandler { body, .. } => {
                assert_eq!(body.len(), 3);
            }
            _ => panic!("expected EventHandler"),
        }
    }

    #[test]
    fn test_feature_and_signal_in_event_handler() {
        let prog =
            parse("feature f = 1.0 + 2.0\nsignal s = 1.0 > 0.5\non eval() { quince.log(\"ok\") }")
                .unwrap();
        assert_eq!(prog.len(), 3);
    }

    #[test]
    fn test_at_using_with_no_params() {
        let prog = parse("@using cvd pmdi nmdi").unwrap();
        match &prog[0] {
            Stmt::Using { indicators } => {
                assert_eq!(indicators.len(), 3);
                assert!(indicators[0].params.is_empty());
            }
            _ => panic!("expected Using"),
        }
    }

    #[test]
    fn test_mixed_old_and_new_syntax() {
        let prog = parse("function on_trade(trade) end\non eval() { quince.log(\"ok\") }").unwrap();
        assert_eq!(prog.len(), 2);
    }
}
