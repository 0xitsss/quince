use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub suggestion: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{}:{}:{}: {}", self.line, self.col, tag, self.message)?;
        if let Some(sugg) = &self.suggestion {
            write!(f, "\n  help: {}", sugg)?;
        }
        Ok(())
    }
}

struct Cursor<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    diagnostics: Vec<Diagnostic>,
    known_directives: &'a [(&'a str, &'a str)],
}

impl<'a> Cursor<'a> {
    fn new(source: &str, known_directives: &'a [(&'a str, &'a str)]) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            diagnostics: Vec::new(),
            known_directives,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_ahead(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn error(&mut self, msg: impl Into<String>, sugg: Option<&str>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            line: self.line,
            col: self.col,
            message: msg.into(),
            suggestion: sugg.map(|s| s.to_string()),
        });
    }

    fn warn(&mut self, msg: impl Into<String>, sugg: Option<&str>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            line: self.line,
            col: self.col,
            message: msg.into(),
            suggestion: sugg.map(|s| s.to_string()),
        });
    }

    fn edit_distance(&self, a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let (m, n) = (a.len(), b.len());
        if m == 0 {
            return n;
        }
        if n == 0 {
            return m;
        }
        let mut prev: Vec<usize> = (0..=n).collect();
        let mut curr: Vec<usize> = vec![0; n + 1];
        for i in 0..m {
            curr[0] = i + 1;
            for j in 0..n {
                let cost = if a[i] == b[j] { 0 } else { 1 };
                curr[j + 1] = (curr[j] + 1).min(prev[j + 1] + 1).min(prev[j] + cost);
            }
            std::mem::swap(&mut prev, &mut curr);
        }
        prev[n]
    }

    fn suggest_directive(&self, name: &str) -> Option<String> {
        let mut best = None;
        let mut best_dist = 3;
        for &(directive, _) in self.known_directives {
            let dist = self.edit_distance(name, directive);
            if dist < best_dist {
                best_dist = dist;
                best = Some(directive);
            }
        }
        best.map(|d| format!("did you mean '@{}'?", d))
    }

    fn check_suspicious_operators(&mut self) {
        if self.peek() == Some('!') {
            if self.peek_ahead(1) == Some('=') {
                self.advance();
                self.advance();
                if self.chars.get(self.pos - 2..self.pos) == Some(&['!', '=']) {
                    if self.peek() == Some('=') {
                        self.advance();
                        self.error(
                            "'!==' is not valid QFL syntax",
                            Some("use '~=' for not-equal"),
                        );
                    } else {
                        self.error(
                            "'!=' is not valid QFL syntax",
                            Some("use '~=' for not-equal"),
                        );
                    }
                }
            }
        } else if self.peek() == Some('&') && self.peek_ahead(1) == Some('&') {
            self.advance();
            self.advance();
            self.error(
                "'&&' is not valid QFL syntax",
                Some("use 'and' instead of '&&'"),
            );
        } else if self.peek() == Some('|') && self.peek_ahead(1) == Some('|') {
            self.advance();
            self.advance();
            self.error(
                "'||' is not valid QFL syntax",
                Some("use 'or' instead of '||'"),
            );
        } else if self.peek() == Some(':') && self.peek_ahead(1) == Some('=') {
            self.advance();
            self.advance();
            self.error(
                "':=' is not valid QFL syntax",
                Some("use '=' for assignment"),
            );
        } else if self.peek() == Some('+') && self.peek_ahead(1) == Some('+') {
            self.advance();
            self.advance();
            self.error(
                "'++' is not valid QFL syntax",
                Some("use 'x = x + 1' instead"),
            );
        }
    }

}

pub fn check(source: &str) -> Vec<Diagnostic> {
    let known_directives = &[
        ("persist", "@persist — persist variables across reloads"),
        ("using", "@using — declare indicator dependencies"),
        ("window", "@window — declare rolling window"),
    ];

    let mut d = Diagnostics::new(source, known_directives);

    d.check_metadata();
    d.check_lines();
    d.check_char_stream();

    d.cursor.diagnostics
}

struct Diagnostics<'a> {
    cursor: Cursor<'a>,
    source: &'a str,
}

impl<'a> Diagnostics<'a> {
    fn new(source: &'a str, known: &'a [(&'a str, &'a str)]) -> Self {
        Self {
            cursor: Cursor::new(source, known),
            source,
        }
    }

    fn push(&mut self, diag: Diagnostic) {
        self.cursor.diagnostics.push(diag);
    }

    fn check_metadata(&mut self) {
        if self.source.is_empty() {
            self.push(Diagnostic {
                severity: Severity::Warning,
                line: 1,
                col: 1,
                message: "empty source file".into(),
                suggestion: Some("add strategy code or remove the file".into()),
            });
            return;
        }

        if self.source.starts_with('\u{feff}') {
            self.push(Diagnostic {
                severity: Severity::Warning,
                line: 1,
                col: 1,
                message: "file starts with UTF-8 BOM (U+FEFF)".into(),
                suggestion: Some("save the file without BOM (UTF-8 without signature)".into()),
            });
        }

        if self.source.starts_with("#!") {
            let end = self.source.find('\n').unwrap_or(self.source.len());
            self.push(Diagnostic {
                severity: Severity::Warning,
                line: 1,
                col: 1,
                message: format!("shebang line ignored: {}", &self.source[..end]),
                suggestion: Some("remove the shebang line for QFL source files".into()),
            });
        }

        if !self.source.ends_with('\n') && !self.source.is_empty() {
            let lines: Vec<&str> = self.source.split('\n').collect();
            let last_line = lines.len();
            self.push(Diagnostic {
                severity: Severity::Warning,
                line: last_line,
                col: lines.last().map_or(0, |l| l.len() + 1),
                message: "no newline at end of file".into(),
                suggestion: Some("add a trailing newline".into()),
            });
        }

        if self.source.contains('\r') {
            if let Some(pos) = self.source.find('\r') {
                let line = self.source[..pos].chars().filter(|&c| c == '\n').count() + 1;
                let mut col = 0;
                for c in self.source[..pos].chars().rev() {
                    if c == '\n' {
                        break;
                    }
                    col += 1;
                }
                self.push(Diagnostic {
                    severity: Severity::Warning,
                    line,
                    col: col + 1,
                    message: "carriage return (\\r) detected — use LF line endings".into(),
                    suggestion: Some("convert to Unix line endings (LF)".into()),
                });
            }
        }
    }

    fn check_lines(&mut self) {
        let lines: Vec<&str> = self.source.split('\n').collect();
        let mut has_tabs = false;
        let mut has_spaces = false;

        for (i, line) in lines.iter().enumerate() {
            let lineno = i + 1;
            if line.is_empty() {
                continue;
            }

            if line.starts_with('\t') {
                has_tabs = true;
            } else if line.starts_with(' ') {
                let indent = line.len() - line.trim_start().len();
                if indent > 0 {
                    has_spaces = true;
                }
            }

            let trimmed = line.trim_end();
            let trailing = line.len() - trimmed.len();
            if trailing > 0 {
                self.push(Diagnostic {
                    severity: Severity::Warning,
                    line: lineno,
                    col: trimmed.len().saturating_sub(0).max(1),
                    message: format!("trailing whitespace ({} space{})", trailing, if trailing == 1 { "" } else { "s" }),
                    suggestion: Some("remove trailing spaces".into()),
                });
            }

            if line.len() > 120 {
                self.push(Diagnostic {
                    severity: Severity::Warning,
                    line: lineno,
                    col: 121,
                    message: format!("line too long ({} chars, max 120)", line.len()),
                    suggestion: Some("consider breaking the line into multiple lines".into()),
                });
            }
        }

        if has_tabs && has_spaces {
            self.push(Diagnostic {
                severity: Severity::Warning,
                line: 1,
                col: 1,
                message: "mixed tabs and spaces for indentation".into(),
                suggestion: Some("pick one: use spaces (2 or 4) consistently".into()),
            });
        }
    }

    fn check_char_stream(&mut self) {
        let mut cursor = Cursor::new(self.source, self.cursor.known_directives);

        enum State {
            Normal,
            LineComment,
            BlockComment,
            String(char),
        }

        let mut state = State::Normal;

        while cursor.pos < cursor.chars.len() {
            match &state {
                State::Normal => {
                    if cursor.peek() == Some('-')
                        && cursor.peek_ahead(1) == Some('-')
                        && cursor.peek_ahead(2) == Some('[')
                        && cursor.peek_ahead(3) == Some('[')
                    {
                        cursor.advance();
                        cursor.advance();
                        cursor.advance();
                        cursor.advance();
                        state = State::BlockComment;
                        continue;
                    }
                    if cursor.peek() == Some('-') && cursor.peek_ahead(1) == Some('-') {
                        cursor.advance();
                        cursor.advance();
                        state = State::LineComment;
                        continue;
                    }
                    if cursor.peek() == Some('"') || cursor.peek() == Some('\'') {
                        let quote = cursor.peek().unwrap();
                        cursor.advance();
                        state = State::String(quote);
                        continue;
                    }
                    if cursor.peek() == Some('@') {
                        cursor.advance();
                        let mut name = String::new();
                        while let Some(ch) = cursor.peek() {
                            if ch.is_alphanumeric() || ch == '_' {
                                name.push(ch);
                                cursor.advance();
                            } else {
                                break;
                            }
                        }
                        if !name.is_empty() {
                            let is_known = cursor
                                .known_directives
                                .iter()
                                .any(|(d, _)| *d == name);
                            if !is_known {
                                if let Some(sugg) = cursor.suggest_directive(&name) {
                                    cursor.error(
                                        format!("unknown directive '@{}'", name),
                                        Some(&sugg),
                                    );
                                }
                            }
                        }
                        continue;
                    }

                    let line_before = cursor.line;
                    cursor.check_suspicious_operators();
                    if cursor.line == line_before {
                        cursor.advance();
                    }
                }

                State::LineComment => {
                    if cursor.peek() == Some('\n') {
                        state = State::Normal;
                    }
                    cursor.advance();
                }

                State::BlockComment => {
                    if cursor.peek() == Some(']') && cursor.peek_ahead(1) == Some(']') {
                        cursor.advance();
                        cursor.advance();
                        state = State::Normal;
                        continue;
                    }
                    if cursor.peek().is_none() {
                        cursor.error(
                            "unterminated multi-line comment (--[[ without ]])",
                            Some("add ']]' to close the comment"),
                        );
                        break;
                    }
                    cursor.advance();
                }

                State::String(quote) => {
                    if cursor.peek() == Some(*quote) {
                        cursor.advance();
                        state = State::Normal;
                        continue;
                    }
                    if cursor.peek() == Some('\\') {
                        cursor.advance();
                        if let Some(esc) = cursor.advance() {
                            match esc {
                                'n' | 't' | 'r' | '\\' | '"' | '\'' => {}
                                _ => {
                                    cursor.warn(
                                        format!("unrecognized escape sequence '\\{}'", esc),
                                        Some("valid escapes: \\n, \\t, \\r, \\\\, \\\", \\'"),
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    if cursor.peek() == Some('\n') {
                        cursor.error(
                            "unterminated string literal (newline before closing quote)",
                            Some("add a closing quote or use a multi-line string"),
                        );
                        cursor.advance();
                        state = State::Normal;
                        continue;
                    }
                    if cursor.peek().is_none() {
                        cursor.error(
                            "unterminated string literal (reached end of file)",
                            Some("add a closing quote"),
                        );
                        break;
                    }
                    cursor.advance();
                }
            }
        }

        if let State::String(quote) = state {
            cursor.error(
                format!("unterminated string literal (opened with '{}')", quote),
                Some("add a closing quote"),
            );
        }

        if let State::BlockComment = state {
            cursor.error(
                "unterminated multi-line comment (--[[ without ]])",
                Some("add ']]' to close the comment"),
            );
        }

        self.cursor.diagnostics.extend(cursor.diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_file() {
        let diags = check("");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("empty"));
    }

    #[test]
    fn test_bom() {
        let diags = check("\u{feff}local x = 1");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("BOM"));
    }

    #[test]
    fn test_shebang() {
        let diags = check("#!/usr/bin/env qfl\nlocal x = 1");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("shebang"));
    }

    #[test]
    fn test_trailing_whitespace() {
        let diags = check("local x = 1   \nlocal y = 2");
        assert!(diags.iter().any(|d| d.message.contains("trailing")));
    }

    #[test]
    fn test_line_too_long() {
        let long = "x".repeat(121);
        let diags = check(&long);
        assert!(diags.iter().any(|d| d.message.contains("too long")));
    }

    #[test]
    fn test_not_equal_c_style() {
        let diags = check("if x != y then");
        assert!(diags.iter().any(|d| d.message.contains("!=")));
    }

    #[test]
    fn test_not_equal_strict() {
        let diags = check("if x !== y then");
        assert!(diags.iter().any(|d| d.message.contains("!==")));
    }

    #[test]
    fn test_logical_and_c_style() {
        let diags = check("if x > 0 && y > 0 then");
        assert!(diags.iter().any(|d| d.message.contains("&&")));
    }

    #[test]
    fn test_logical_or_c_style() {
        let diags = check("if x > 0 || y > 0 then");
        assert!(diags.iter().any(|d| d.message.contains("||")));
    }

    #[test]
    fn test_pascal_assignment() {
        let diags = check("x := 42");
        assert!(diags.iter().any(|d| d.message.contains(":=")));
    }

    #[test]
    fn test_increment() {
        let diags = check("x++");
        assert!(diags.iter().any(|d| d.message.contains("++")));
    }

    #[test]
    fn test_unknown_directive() {
        let diags = check("@persit x = 1");
        assert!(diags.iter().any(|d| d.message.contains("@persit")));
        assert!(diags.iter().any(|d| {
            d.suggestion.as_deref().map_or(false, |s| s.contains("@persist"))
        }));
    }

    #[test]
    fn test_misspelled_using() {
        let diags = check("@usnig ema:12");
        assert!(diags.iter().any(|d| d.message.contains("@usnig")));
    }

    #[test]
    fn test_unterminated_string_same_line() {
        let diags = check("local s = \"hello");
        assert!(diags.iter().any(|d| d.message.contains("unterminated string")));
    }

    #[test]
    fn test_unterminated_string_multi_line() {
        let diags = check("local s = \"hello\nworld");
        assert!(diags.iter().any(|d| d.message.contains("unterminated string")));
    }

    #[test]
    fn test_unterminated_block_comment() {
        let diags = check("--[[ hello world");
        assert!(diags.iter().any(|d| d.message.contains("unterminated multi-line")));
    }

    #[test]
    fn test_valid_code_no_diags() {
        let code = r#"
@persist position_size : f64 = 0.0
@using ema:12 ema:48

on trade ->
    local price = quince.price()
    if price > 50000.0 then
        quince.order(0, 0.001, price)
    end
"#;
        let diags = check(code);
        let errors: Vec<_> = diags.iter().filter(|d| d.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn test_unrecognized_escape() {
        let diags = check("\"hello\\zworld\"");
        assert!(diags.iter().any(|d| d.message.contains("unrecognized escape")));
    }

    #[test]
    fn test_mixed_indentation() {
        let diags = check("\tlocal x = 1\n    local y = 2");
        assert!(diags.iter().any(|d| d.message.contains("mixed tabs and spaces")));
    }

    #[test]
    fn test_suspicious_operators_inside_string() {
        let diags = check("local s = \"x != y\"");
        let bad: Vec<_> = diags.iter().filter(|d| d.message.contains("!=")).collect();
        assert!(bad.is_empty(), "should not flag operators inside strings");
    }

    #[test]
    fn test_suspicious_operators_inside_comment() {
        let diags = check("-- x != y is not valid");
        let bad: Vec<_> = diags.iter().filter(|d| d.message.contains("!=")).collect();
        assert!(bad.is_empty(), "should not flag operators inside comments");
    }

    #[test]
    fn test_no_false_positive_concat() {
        let diags = check("local s = \"hello\" .. \"world\"");
        let bad: Vec<_> = diags.iter().filter(|d| d.severity == Severity::Error).collect();
        assert!(bad.is_empty(), "concat should not trigger errors");
    }

    #[test]
    fn test_no_newline_at_eof() {
        let diags = check("local x = 1");
        assert!(diags.iter().any(|d| d.message.contains("no newline at end")));
    }

    #[test]
    fn test_carriage_return() {
        let diags = check("local x = 1\r\nlocal y = 2");
        assert!(diags.iter().any(|d| d.message.contains("carriage return")));
    }

    #[test]
    fn test_block_comment_correct() {
        let diags = check("--[[ valid ]] local x = 1");
        let errors: Vec<_> = diags.iter().filter(|d| d.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn test_string_with_escape_no_warning() {
        let diags = check("\"hello\\nworld\\t\\\"quote\\'\"");
        let warns: Vec<_> = diags.iter().filter(|d| d.message.contains("escape")).collect();
        assert!(warns.is_empty(), "valid escapes should not warn: {:?}", warns);
    }
}
