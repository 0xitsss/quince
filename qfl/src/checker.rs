// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL static analysis — linter for common mistakes and anti-patterns.
//!
//! Checks source files for:
//! - C-style operators (`!=`, `&&`, `||`, `:=`, `++`) that are invalid in QFL
//! - Misspelled directives (`@persit` в†’ `@persist`)
//! - Trailing whitespace, mixed indentation, overly long lines
//! - Unterminated strings and block comments
//! - UTF-8 BOM, shebang lines, carriage returns, missing trailing newlines
//!
//! Entry point: [`check()`] returns a list of [`Diagnostic`]s.

use std::fmt;

/// Severity level of a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A definite problem that should be fixed.
    Error,
    /// A code style or best-practice suggestion.
    Warning,
}

/// A single diagnostic: error or warning at a specific source location.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Whether this is an error or a warning.
    pub severity: Severity,
    /// 1-indexed line number.
    pub line: usize,
    /// 1-indexed column number.
    pub col: usize,
    /// Human-readable description of the issue.
    pub message: String,
    /// Optional suggestion for how to fix the issue.
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

// --- Cursor: character-level scanner with position tracking ---

/// Low-level character cursor over source text.
///
/// Tracks (line, col) as it advances, and accumulates diagnostics.
struct Cursor<'a> {
    /// All source characters for O(1) lookahead.
    chars: Vec<char>,
    /// Current index into `chars`.
    pos: usize,
    /// Current 1-indexed line.
    line: usize,
    /// Current 1-indexed column.
    col: usize,
    /// Diagnostics accumulated so far.
    diagnostics: Vec<Diagnostic>,
    /// Known `@directive` names and their descriptions (used for spell-check).
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

    /// Peek at the current character without advancing.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Peek `n` characters ahead without advancing.
    fn peek_ahead(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    /// Advance one character, updating line/col tracking.
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

    /// Register an error at the current position.
    fn error(&mut self, msg: impl Into<String>, sugg: Option<&str>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            line: self.line,
            col: self.col,
            message: msg.into(),
            suggestion: sugg.map(|s| s.to_string()),
        });
    }

    /// Register a warning at the current position.
    fn warn(&mut self, msg: impl Into<String>, sugg: Option<&str>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            line: self.line,
            col: self.col,
            message: msg.into(),
            suggestion: sugg.map(|s| s.to_string()),
        });
    }

    /// Levenshtein edit distance between two strings.
    ///
    /// Used by [`suggest_directive`] to find near-miss directive names.
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
        // Classic DP: two-row rolling buffer.
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

    /// Find the closest known directive name by edit distance (threshold: 3 edits).
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

    /// Detect C-style / Pascal / JS operators that are common QFL mistakes.
    ///
    /// QFL uses `~=` for not-equal, `and`/`or` for logical ops,
    /// plain `=` for assignment, and explicit `x = x + 1` for increment.
    fn check_suspicious_operators(&mut self) {
        // `!==` and `!=` в†’ QFL uses `~=`
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
        // `&&` в†’ QFL uses `and`
        } else if self.peek() == Some('&') && self.peek_ahead(1) == Some('&') {
            self.advance();
            self.advance();
            self.error(
                "'&&' is not valid QFL syntax",
                Some("use 'and' instead of '&&'"),
            );
        // `||` в†’ QFL uses `or`
        } else if self.peek() == Some('|') && self.peek_ahead(1) == Some('|') {
            self.advance();
            self.advance();
            self.error(
                "'||' is not valid QFL syntax",
                Some("use 'or' instead of '||'"),
            );
        // `:=` в†’ QFL uses `=`
        } else if self.peek() == Some(':') && self.peek_ahead(1) == Some('=') {
            self.advance();
            self.advance();
            self.error(
                "':=' is not valid QFL syntax",
                Some("use '=' for assignment"),
            );
        // `++` в†’ QFL uses `x = x + 1`
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

// --- High-level diagnostic checks ---

/// Run all static checks on a QFL source string.
///
/// Returns a list of [`Diagnostic`]s (sorted by appearance order).
/// Returns an empty vec for valid, clean code.
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

/// Stateful checker holding both the cursor and the raw source.
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

    /// Metadata-level checks: empty file, BOM, shebang, trailing newline, CRLF.
    fn check_metadata(&mut self) {
        // Warn on empty source files.
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

        // UTF-8 BOM (U+FEFF) — can cause issues with some parsers.
        if self.source.starts_with('\u{feff}') {
            self.push(Diagnostic {
                severity: Severity::Warning,
                line: 1,
                col: 1,
                message: "file starts with UTF-8 BOM (U+FEFF)".into(),
                suggestion: Some("save the file without BOM (UTF-8 without signature)".into()),
            });
        }

        // Shebang line (`#!/...`) — not part of QFL syntax.
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

        // POSIX convention: every text file should end with a newline.
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

        // Carriage returns (`\r`) — Windows line endings in Unix-oriented toolchain.
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

    /// Line-oriented checks: trailing whitespace, line length, mixed indentation.
    fn check_lines(&mut self) {
        let lines: Vec<&str> = self.source.split('\n').collect();
        let mut has_tabs = false;
        let mut has_spaces = false;

        for (i, line) in lines.iter().enumerate() {
            let lineno = i + 1;
            if line.is_empty() {
                continue;
            }

            // Detect indentation style.
            if line.starts_with('\t') {
                has_tabs = true;
            } else if line.starts_with(' ') {
                let indent = line.len() - line.trim_start().len();
                if indent > 0 {
                    has_spaces = true;
                }
            }

            // Trailing whitespace detection.
            let trimmed = line.trim_end();
            let trailing = line.len() - trimmed.len();
            if trailing > 0 {
                self.push(Diagnostic {
                    severity: Severity::Warning,
                    line: lineno,
                    col: trimmed.len().saturating_sub(0).max(1),
                    message: format!(
                        "trailing whitespace ({} space{})",
                        trailing,
                        if trailing == 1 { "" } else { "s" }
                    ),
                    suggestion: Some("remove trailing spaces".into()),
                });
            }

            // Line length > 120 chars.
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

        // Warn if file mixes tabs and spaces.
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

    /// Full character-stream scan using a state machine.
    ///
    /// States: `Normal`, `LineComment`, `BlockComment`, `String(quote)`.
    /// This catches:
    /// - C-style operators outside strings/comments
    /// - Unknown `@directives`
    /// - Unterminated strings and block comments
    /// - Unrecognized escape sequences
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
                    // `--[[` opens a block comment.
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
                    // `--` opens a line comment.
                    if cursor.peek() == Some('-') && cursor.peek_ahead(1) == Some('-') {
                        cursor.advance();
                        cursor.advance();
                        state = State::LineComment;
                        continue;
                    }
                    // `"` or `'` opens a string literal.
                    if cursor.peek() == Some('"') || cursor.peek() == Some('\'') {
                        let quote = cursor.peek().unwrap();
                        cursor.advance();
                        state = State::String(quote);
                        continue;
                    }
                    // `@name` — check directive spelling.
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
                            let is_known = cursor.known_directives.iter().any(|(d, _)| *d == name);
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

                    // Check for suspicious operators and advance.
                    let line_before = cursor.line;
                    cursor.check_suspicious_operators();
                    // If check_suspicious_operators didn't consume input, advance by one.
                    if cursor.line == line_before {
                        cursor.advance();
                    }
                }

                State::LineComment => {
                    // Line comment ends at newline.
                    if cursor.peek() == Some('\n') {
                        state = State::Normal;
                    }
                    cursor.advance();
                }

                State::BlockComment => {
                    // Block comment ends at `]]`.
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
                    // Matching close quote ends the string.
                    if cursor.peek() == Some(*quote) {
                        cursor.advance();
                        state = State::Normal;
                        continue;
                    }
                    // Backslash escape sequence.
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
                    // Newline before closing quote = unterminated.
                    if cursor.peek() == Some('\n') {
                        cursor.error(
                            "unterminated string literal (newline before closing quote)",
                            Some("add a closing quote or use a multi-line string"),
                        );
                        cursor.advance();
                        state = State::Normal;
                        continue;
                    }
                    // EOF before closing quote.
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

        // EOF in string or comment — flag as unterminated.
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
            d.suggestion
                .as_deref()
                .map_or(false, |s| s.contains("@persist"))
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
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unterminated string")));
    }

    #[test]
    fn test_unterminated_string_multi_line() {
        let diags = check("local s = \"hello\nworld");
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unterminated string")));
    }

    #[test]
    fn test_unterminated_block_comment() {
        let diags = check("--[[ hello world");
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unterminated multi-line")));
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
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn test_unrecognized_escape() {
        let diags = check("\"hello\\zworld\"");
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unrecognized escape")));
    }

    #[test]
    fn test_mixed_indentation() {
        let diags = check("\tlocal x = 1\n    local y = 2");
        assert!(diags
            .iter()
            .any(|d| d.message.contains("mixed tabs and spaces")));
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
        let bad: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(bad.is_empty(), "concat should not trigger errors");
    }

    #[test]
    fn test_no_newline_at_eof() {
        let diags = check("local x = 1");
        assert!(diags
            .iter()
            .any(|d| d.message.contains("no newline at end")));
    }

    #[test]
    fn test_carriage_return() {
        let diags = check("local x = 1\r\nlocal y = 2");
        assert!(diags.iter().any(|d| d.message.contains("carriage return")));
    }

    #[test]
    fn test_block_comment_correct() {
        let diags = check("--[[ valid ]] local x = 1");
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn test_string_with_escape_no_warning() {
        let diags = check("\"hello\\nworld\\t\\\"quote\\'\"");
        let warns: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("escape"))
            .collect();
        assert!(
            warns.is_empty(),
            "valid escapes should not warn: {:?}",
            warns
        );
    }
}
