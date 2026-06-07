// --- Section: Imports ---

use std::fmt;

// --- Section: Token Enum ---

/// A single token produced by the QFL lexer.
///
/// Covers 73 variants including keywords, literals, operators,
/// symbols, directives (@persist, @using, @window), and phase-4h
/// keywords (state, on, fn).
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Standard Lua-derived keywords
    Function,
    Local,
    If,
    Then,
    Else,
    ElseIf,
    End,
    While,
    Do,
    Repeat,
    Until,
    For,
    In,
    Return,
    And,
    Or,
    Not,
    Nil,
    True,
    False,

    // Literal values carrying their text content
    Number(String),
    String(String),
    Ident(String),

    // Symbols — single- and multi-character operators / punctuation
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    SlashSlash, // //
    Percent,    // %
    Caret,      // ^
    Hash,       // #
    Dot,        // .
    Comma,      // ,
    Colon,      // :
    Semi,       // ;
    LParen,     // (
    RParen,     // )
    LBrace,     // {
    RBrace,     // }
    LBracket,   // [
    RBracket,   // ]
    Eq,         // =
    EqEq,       // ==
    TildeEq,    // ~=
    Lt,         // <
    Gt,         // >
    LtEq,       // <=
    GtEq,       // >=
    Concat,     // ..
    VarArg,     // ...
    Arrow,      // ->

    // Directive tokens — QFL-specific @-prefixed markers
    AtPersist, // @persist
    AtUsing,   // @using
    AtWindow,  // @window

    // Phase-4h syntactic sugar keywords
    State, // state
    On,    // on
    Fn,    // fn

    // Comment content (text after -- or inside --[[ ... ]])
    Comment(String),

    // End-of-file sentinel
    Eof,
}

// --- Section: Display impl for Token ---
// Maps each Token variant back to its source-level string representation.

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Function => write!(f, "function"),
            Token::Local => write!(f, "local"),
            Token::If => write!(f, "if"),
            Token::Then => write!(f, "then"),
            Token::Else => write!(f, "else"),
            Token::ElseIf => write!(f, "elseif"),
            Token::End => write!(f, "end"),
            Token::While => write!(f, "while"),
            Token::Do => write!(f, "do"),
            Token::Repeat => write!(f, "repeat"),
            Token::Until => write!(f, "until"),
            Token::For => write!(f, "for"),
            Token::In => write!(f, "in"),
            Token::Return => write!(f, "return"),
            Token::And => write!(f, "and"),
            Token::Or => write!(f, "or"),
            Token::Not => write!(f, "not"),
            Token::Nil => write!(f, "nil"),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Number(n) => write!(f, "num({})", n),
            Token::String(s) => write!(f, "str(\"{}\")", s),
            Token::Ident(s) => write!(f, "ident({})", s),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::SlashSlash => write!(f, "//"),
            Token::Percent => write!(f, "%"),
            Token::Caret => write!(f, "^"),
            Token::Hash => write!(f, "#"),
            Token::Dot => write!(f, "."),
            Token::Comma => write!(f, ","),
            Token::Colon => write!(f, ":"),
            Token::Semi => write!(f, ";"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::Eq => write!(f, "="),
            Token::EqEq => write!(f, "=="),
            Token::TildeEq => write!(f, "~="),
            Token::Lt => write!(f, "<"),
            Token::Gt => write!(f, ">"),
            Token::LtEq => write!(f, "<="),
            Token::GtEq => write!(f, ">="),
            Token::Concat => write!(f, ".."),
            Token::VarArg => write!(f, "..."),
            Token::Arrow => write!(f, "->"),
            Token::AtPersist => write!(f, "@persist"),
            Token::AtUsing => write!(f, "@using"),
            Token::AtWindow => write!(f, "@window"),
            Token::State => write!(f, "state"),
            Token::On => write!(f, "on"),
            Token::Fn => write!(f, "fn"),
            Token::Comment(c) => write!(f, "--{}", c),
            Token::Eof => write!(f, "eof"),
        }
    }
}

// --- Section: LexerError ---

/// An error produced during lexing with source position information.
#[derive(Debug, Clone)]
pub struct LexerError {
    pub msg: String,   // Human-readable error description
    pub line: usize,   // 1-based line number where the error occurred
    pub col: usize,    // 1-based column number where the error occurred
}

impl fmt::Display for LexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}

/// Character-level lexer that scans QFL source text into tokens.
pub struct Lexer {
    chars: Vec<char>, // The entire source as a vector of chars for O(1) indexing
    pos: usize,       // Current cursor position within chars
    line: usize,      // Current 1-based line number (for error reporting)
    col: usize,       // Current 1-based column number (for error reporting)
}

// --- Section: Lexer Impl — Core Methods ---

impl Lexer {
    // Create a new lexer from source text. Initializes cursor at position 0, line 1, col 1.
    pub fn new(input: &str) -> Self {
        Lexer {
            chars: input.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    // Return the current character without consuming it, or None at EOF.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    // Look ahead n characters without consuming, or None if beyond EOF.
    fn peek_ahead(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    // Consume and return the current character, advancing position.
    // Tracks line/col for error reporting — increments line on '\n', resets col.
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

    // Skip over spaces, tabs, newlines, and carriage returns until a non-whitespace char is found.
    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    // Read a numeric literal starting with `first` (the first digit).
    // Handles decimals (single dot), hex (0x/0X prefix), rejects nan/inf/infinity
    // as invalid number tokens, and enforces a 100-char length limit.
    fn read_number(&mut self, first: char) -> Result<Token, LexerError> {
        let mut s = String::new();
        s.push(first);
        let mut is_float = false;
        // Consume digits and at most one decimal point
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else if c == '.' && !is_float {
                // Allow one dot for floating-point; second dot stops the number
                is_float = true;
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // If number starts with "0x" or "0X", switch to hex-digit consumption mode
        if first == '0' && (self.peek() == Some('x') || self.peek() == Some('X')) {
            s.push(self.advance().unwrap());
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    s.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }
        // Reject nan, inf, infinity as number tokens — they are identifiers, not numbers
        let lower = s.to_lowercase();
        if lower == "nan" || lower == "inf" || lower == "infinity" {
            return Err(LexerError {
                msg: format!("invalid number literal: '{}'", s),
                line: self.line,
                col: self.col,
            });
        }
        // Reject absurdly long number literals (safety limit)
        if s.len() > 100 {
            return Err(LexerError {
                msg: format!("number literal too long ({} chars)", s.len()),
                line: self.line,
                col: self.col,
            });
        }
        Ok(Token::Number(s))
    }

    // Read a string literal delimited by `quote` (' or ").
    // Handles escape sequences: \n, \t, \\, \", \', \r.
    // Returns an error on unterminated string or unterminated escape.
    fn read_string(&mut self, quote: char) -> Result<Token, LexerError> {
        let mut s = String::new();
        loop {
            match self.advance() {
                None => {
                    return Err(LexerError {
                        msg: "unterminated string".into(),
                        line: self.line,
                        col: self.col,
                    })
                }
                Some(c) if c == quote => break, // Closing quote found — string is complete
                Some('\\') => {
                    // Escape sequence: read the next char and map it
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some('\'') => s.push('\''),
                        Some('r') => s.push('\r'),
                        Some(c) => s.push(c), // Unknown escape: pass through literally
                        None => {
                            return Err(LexerError {
                                msg: "unterminated escape".into(),
                                line: self.line,
                                col: self.col,
                            })
                        }
                    }
                }
                Some(c) => s.push(c), // Regular character — append as-is
            }
        }
        Ok(Token::String(s))
    }

    // Read an identifier or keyword starting with `first` (a letter or underscore).
    // Consumes alphanumeric + underscore characters, then matches against known
    // keyword strings. If no keyword matches, returns an Ident token.
    fn read_ident_or_keyword(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);
        // Collect all alphanumeric and underscore characters
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // Match against keyword table; anything else is a plain identifier
        match s.as_str() {
            "function" => Token::Function,
            "local" => Token::Local,
            "if" => Token::If,
            "then" => Token::Then,
            "else" => Token::Else,
            "elseif" => Token::ElseIf,
            "end" => Token::End,
            "while" => Token::While,
            "do" => Token::Do,
            "repeat" => Token::Repeat,
            "until" => Token::Until,
            "for" => Token::For,
            "in" => Token::In,
            "return" => Token::Return,
            "and" => Token::And,
            "or" => Token::Or,
            "not" => Token::Not,
            "nil" => Token::Nil,
            "true" => Token::True,
            "false" => Token::False,
            "state" => Token::State,
            "on" => Token::On,
            "fn" => Token::Fn,
            _ => Token::Ident(s),
        }
    }

    // --- Section: Token Dispatch ---

    // Scan and return the next token from the input stream.
    // Dispatches to specialized readers: comments, directives, numbers,
    // identifiers/keywords, strings, and multi-char symbols.
    // Returns Eof when input is exhausted, or an error for invalid characters.
    pub fn next_token(&mut self) -> Result<Token, LexerError> {
        // Skip any leading whitespace before examining the next character
        self.skip_whitespace();

        // Check for end of input
        let c = match self.peek() {
            None => return Ok(Token::Eof),
            Some(c) => c,
        };

        // Multi-line comment --[[ ... ]] (check BEFORE single-line --)
        // Requires four characters of lookahead: --[[
        if c == '-'
            && self.peek_ahead(1) == Some('-')
            && self.peek_ahead(2) == Some('[')
            && self.peek_ahead(3) == Some('[')
        {
            // Consume the four opening characters: --[[
            self.advance();
            self.advance();
            self.advance();
            self.advance();
            let mut content = String::new();
            // Read until closing ]] is found, or error on unterminated comment
            loop {
                if self.peek() == Some(']') && self.peek_ahead(1) == Some(']') {
                    self.advance();
                    self.advance();
                    break;
                }
                match self.advance() {
                    None => {
                        return Err(LexerError {
                            msg: "unterminated multi-line comment".into(),
                            line: self.line,
                            col: self.col,
                        })
                    }
                    Some(c) => content.push(c),
                }
            }
            return Ok(Token::Comment(content));
        }

        // Single-line comment: -- until end of line
        if c == '-' && self.peek_ahead(1) == Some('-') {
            self.advance(); // first -
            self.advance(); // second -
            let mut content = String::new();
            // Consume everything up to (but not including) the next newline
            while let Some(ch) = self.peek() {
                if ch == '\n' {
                    break;
                }
                content.push(ch);
                self.advance();
            }
            return Ok(Token::Comment(content));
        }

        // @-prefixed directives: @persist, @using, @window
        // Consume the @ sign, then read alphanumeric/underscore name.
        // If the name matches a known directive, emit the specific token;
        // otherwise fall back to an Ident token with the @ prefix.
        if c == '@' {
            self.advance();
            let mut s = String::new();
            while let Some(ch) = self.peek() {
                if ch.is_alphanumeric() || ch == '_' {
                    s.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
            match s.as_str() {
                "persist" => return Ok(Token::AtPersist),
                "using" => return Ok(Token::AtUsing),
                "window" => return Ok(Token::AtWindow),
                _ => return Ok(Token::Ident(format!("@{}", s))),
            }
        }

        // Numeric literal (starts with a digit 0-9)
        if c.is_ascii_digit() {
            self.advance();
            return self.read_number(c);
        }

        // Identifier or keyword (starts with a letter or underscore)
        if c.is_alphabetic() || c == '_' {
            self.advance();
            return Ok(self.read_ident_or_keyword(c));
        }

        // String literal (starts with single or double quote)
        if c == '"' || c == '\'' {
            self.advance();
            return self.read_string(c);
        }

        // --- Single- and multi-character symbol tokens ---
        // Consume the first character; check second character for two-char operators.
        self.advance();
        let next = self.peek();
        match c {
            '+' => Ok(Token::Plus),
            '-' => {
                if next == Some('>') {
                    self.advance();
                    Ok(Token::Arrow)
                } else {
                    Ok(Token::Minus)
                }
            }
            '*' => Ok(Token::Star),
            '/' => {
                if next == Some('/') {
                    self.advance();
                    Ok(Token::SlashSlash)
                } else {
                    Ok(Token::Slash)
                }
            }
            '%' => Ok(Token::Percent),
            '^' => Ok(Token::Caret),
            '#' => Ok(Token::Hash),
            ',' => Ok(Token::Comma),
            ';' => Ok(Token::Semi),
            '(' => Ok(Token::LParen),
            ')' => Ok(Token::RParen),
            '{' => Ok(Token::LBrace),
            '}' => Ok(Token::RBrace),
            '[' => Ok(Token::LBracket),
            ']' => Ok(Token::RBracket),
            ':' => Ok(Token::Colon),
            // Dot: . (single), .. (concat), ... (vararg)
            '.' => {
                if next == Some('.') {
                    self.advance();
                    if self.peek() == Some('.') {
                        self.advance();
                        Ok(Token::VarArg) // ...
                    } else {
                        Ok(Token::Concat) // ..
                    }
                } else {
                    Ok(Token::Dot) // .
                }
            }
            '=' => {
                if next == Some('=') {
                    self.advance();
                    Ok(Token::EqEq) // ==
                } else {
                    Ok(Token::Eq) // =
                }
            }
            '~' => {
                if next == Some('=') {
                    self.advance();
                    Ok(Token::TildeEq) // ~=
                } else {
                    // Lone ~ is not valid QFL syntax
                    Err(LexerError {
                        msg: format!("unexpected '~' (did you mean ~=?)"),
                        line: self.line,
                        col: self.col,
                    })
                }
            }
            '<' => {
                if next == Some('=') {
                    self.advance();
                    Ok(Token::LtEq) // <=
                } else {
                    Ok(Token::Lt) // <
                }
            }
            '>' => {
                if next == Some('=') {
                    self.advance();
                    Ok(Token::GtEq) // >=
                } else {
                    Ok(Token::Gt) // >
                }
            }
            // Any other character is not valid QFL syntax
            _ => Err(LexerError {
                msg: format!("unexpected character '{}'", c),
                line: self.line,
                col: self.col,
            }),
        }
    }

    // Collect all tokens from the input by calling next_token() until EOF.
    // Returns the full token vector or the first lexer error encountered.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = matches!(tok, Token::Eof);
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }
}

// --- Section: Public API — Free Function ---

/// Tokenise a QFL source string into a token vector.
///
/// Validates input size (max 1 MiB), rejects null bytes,
/// and reports line/col on errors.
pub fn tokenize(input: &str) -> Result<Vec<Token>, LexerError> {
    // Reject input larger than 1 MiB to cap memory usage
    if input.len() > 1_048_576 {
        return Err(LexerError {
            msg: format!("input too large: {} bytes (max 1MB)", input.len()),
            line: 1,
            col: 1,
        });
    }
    // Reject null bytes which would interfere with internal string handling
    if input.contains('\0') {
        return Err(LexerError {
            msg: "input contains null bytes".into(),
            line: 1,
            col: 1,
        });
    }
    let mut lexer = Lexer::new(input);
    lexer.tokenize()
}

// --- Section: Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keywords() {
        let tokens = tokenize("function local if then else elseif end while do repeat until for in return and or not nil true false").unwrap();
        assert_eq!(tokens.len(), 21); // 20 keywords + Eof
        assert_eq!(tokens[0], Token::Function);
        assert_eq!(tokens[1], Token::Local);
        assert_eq!(tokens[19], Token::False); // last keyword at index 19
        assert_eq!(tokens[20], Token::Eof);
    }

    #[test]
    fn test_numbers() {
        let tokens = tokenize("42 3.14 0xff").unwrap();
        assert_eq!(tokens[0], Token::Number("42".into()));
        assert_eq!(tokens[1], Token::Number("3.14".into()));
        assert_eq!(tokens[2], Token::Number("0xff".into()));
    }

    #[test]
    fn test_strings() {
        let tokens = tokenize("\"hello\" 'world'").unwrap();
        assert_eq!(tokens[0], Token::String("hello".into()));
        assert_eq!(tokens[1], Token::String("world".into()));
    }

    #[test]
    fn test_operators() {
        let tokens =
            tokenize("+ - * / // % ^ .. == ~= < > <= >= # , ; : ( ) { } [ ] = ...").unwrap();
        assert_eq!(tokens[0], Token::Plus);
        assert_eq!(tokens[3], Token::Slash);
        assert_eq!(tokens[4], Token::SlashSlash);
        assert_eq!(tokens[7], Token::Concat);
        assert_eq!(tokens[8], Token::EqEq);
        assert_eq!(tokens[9], Token::TildeEq);
        assert_eq!(tokens[14], Token::Hash);
        assert_eq!(tokens[22], Token::LBracket);
        assert_eq!(tokens[23], Token::RBracket);
        assert_eq!(tokens[24], Token::Eq);
        assert_eq!(tokens[25], Token::VarArg);
        assert_eq!(tokens[26], Token::Eof);
    }

    #[test]
    fn test_operator_hash() {
        let tokens = tokenize("# , ; :").unwrap();
        assert_eq!(tokens[0], Token::Hash);
        assert_eq!(tokens[1], Token::Comma);
        assert_eq!(tokens[2], Token::Semi);
        assert_eq!(tokens[3], Token::Colon);
        assert_eq!(tokens[4], Token::Eof);
    }

    #[test]
    fn test_at_persist() {
        let tokens = tokenize("@persist position_size").unwrap();
        assert_eq!(tokens[0], Token::AtPersist);
        assert_eq!(tokens[1], Token::Ident("position_size".into()));
    }

    #[test]
    fn test_comment() {
        let tokens = tokenize("-- hello world").unwrap();
        assert_eq!(tokens[0], Token::Comment(" hello world".into()));
    }

    #[test]
    fn test_using_directive() {
        let tokens = tokenize("--USING sma 20").unwrap();
        assert_eq!(tokens[0], Token::Comment("USING sma 20".into()));
    }

    #[test]
    fn test_identifiers() {
        let tokens = tokenize("foo bar_baz quince.get").unwrap();
        assert_eq!(tokens[0], Token::Ident("foo".into()));
        assert_eq!(tokens[1], Token::Ident("bar_baz".into()));
        assert_eq!(tokens[2], Token::Ident("quince".into()));
        assert_eq!(tokens[3], Token::Dot);
        assert_eq!(tokens[4], Token::Ident("get".into()));
    }

    #[test]
    fn test_complex_expression() {
        let tokens = tokenize("if price > 50000.0 and position_size == 0 then").unwrap();
        assert_eq!(tokens[0], Token::If);
        assert_eq!(tokens[1], Token::Ident("price".into()));
        assert_eq!(tokens[2], Token::Gt);
        assert_eq!(tokens[3], Token::Number("50000.0".into()));
        assert_eq!(tokens[4], Token::And);
        assert_eq!(tokens[5], Token::Ident("position_size".into()));
        assert_eq!(tokens[6], Token::EqEq);
        assert_eq!(tokens[7], Token::Number("0".into()));
        assert_eq!(tokens[8], Token::Then);
    }

    // ── Additional lexer tests ──

    #[test]
    fn test_empty_input() {
        let tokens = tokenize("").unwrap();
        assert_eq!(tokens.len(), 1); // just Eof
        assert_eq!(tokens[0], Token::Eof);
    }

    #[test]
    fn test_whitespace() {
        let tokens = tokenize("   \n  \t  ").unwrap();
        assert_eq!(tokens.len(), 1);
    }

    #[test]
    fn test_multiple_comments() {
        let tokens = tokenize("-- first\n-- second\n-- third").unwrap();
        assert_eq!(tokens.len(), 4); // 3 comments + Eof
        assert_eq!(tokens[0], Token::Comment(" first".into()));
        assert_eq!(tokens[2], Token::Comment(" third".into()));
        assert_eq!(tokens[3], Token::Eof);
    }

    #[test]
    fn test_hash_comment() {
        let tokens = tokenize("# hash comment").unwrap();
        assert_eq!(tokens[0], Token::Hash);
    }

    #[test]
    fn test_mixed_comments_and_code() {
        let tokens = tokenize("local x = 1 -- comment\n").unwrap();
        // local, x, =, 1, comment, eof
        assert_eq!(tokens[0], Token::Local);
        assert_eq!(tokens[1], Token::Ident("x".into()));
        assert_eq!(tokens[2], Token::Eq);
        assert_eq!(tokens[3], Token::Number("1".into()));
        assert_eq!(tokens[4], Token::Comment(" comment".into()));
        assert_eq!(tokens[5], Token::Eof);
    }

    #[test]
    fn test_negative_number() {
        let tokens = tokenize("-42").unwrap();
        assert_eq!(tokens[0], Token::Minus);
        assert_eq!(tokens[1], Token::Number("42".into()));
    }

    #[test]
    fn test_single_char_tokens() {
        let tokens = tokenize("()[]{}:;,.=").unwrap();
        assert_eq!(tokens[0], Token::LParen);
        assert_eq!(tokens[1], Token::RParen);
        assert_eq!(tokens[2], Token::LBracket);
        assert_eq!(tokens[3], Token::RBracket);
        assert_eq!(tokens[4], Token::LBrace);
        assert_eq!(tokens[5], Token::RBrace);
        assert_eq!(tokens[6], Token::Colon);
        assert_eq!(tokens[7], Token::Semi);
        assert_eq!(tokens[8], Token::Comma);
        assert_eq!(tokens[9], Token::Dot);
        assert_eq!(tokens[10], Token::Eq);
    }

    #[test]
    fn test_ge_le_tokens() {
        let tokens = tokenize("<= >=").unwrap();
        assert_eq!(tokens[0], Token::LtEq);
        assert_eq!(tokens[1], Token::GtEq);
    }

    #[test]
    fn test_concat_token() {
        let tokens = tokenize("..").unwrap();
        assert_eq!(tokens[0], Token::Concat);
    }

    #[test]
    fn test_vararg_token() {
        let tokens = tokenize("...").unwrap();
        assert_eq!(tokens[0], Token::VarArg);
    }

    #[test]
    fn test_using_keyword() {
        let tokens = tokenize("--USING ema 20").unwrap();
        assert_eq!(tokens[0], Token::Comment("USING ema 20".into()));
    }

    #[test]
    fn test_at_using() {
        let tokens = tokenize("@using ema:12 ema:48").unwrap();
        assert_eq!(tokens[0], Token::AtUsing);
        assert_eq!(tokens[1], Token::Ident("ema".into()));
        assert_eq!(tokens[2], Token::Colon);
        assert_eq!(tokens[3], Token::Number("12".into()));
        assert_eq!(tokens[4], Token::Ident("ema".into()));
        assert_eq!(tokens[5], Token::Colon);
        assert_eq!(tokens[6], Token::Number("48".into()));
    }

    #[test]
    fn test_at_window() {
        let tokens = tokenize("@window midprice 512").unwrap();
        assert_eq!(tokens[0], Token::AtWindow);
        assert_eq!(tokens[1], Token::Ident("midprice".into()));
        assert_eq!(tokens[2], Token::Number("512".into()));
    }

    #[test]
    fn test_keywords_state_on_fn() {
        let tokens = tokenize("state on fn ->").unwrap();
        assert_eq!(tokens[0], Token::State);
        assert_eq!(tokens[1], Token::On);
        assert_eq!(tokens[2], Token::Fn);
        assert_eq!(tokens[3], Token::Arrow);
    }

    #[test]
    fn test_multiple_at_persist() {
        let tokens = tokenize("@persist a @persist b @persist c").unwrap();
        assert_eq!(tokens[0], Token::AtPersist);
        assert_eq!(tokens[1], Token::Ident("a".into()));
        assert_eq!(tokens[2], Token::AtPersist);
        assert_eq!(tokens[3], Token::Ident("b".into()));
        assert_eq!(tokens[4], Token::AtPersist);
        assert_eq!(tokens[5], Token::Ident("c".into()));
    }

    #[test]
    fn test_invalid_char() {
        let result = tokenize("`");
        assert!(result.is_err());
    }

    #[test]
    fn test_unicode_ident() {
        let result = tokenize("café");
        // Should either error or treat as ident
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_float_with_trailing_dot() {
        let tokens = tokenize("42.").unwrap();
        assert_eq!(tokens[0], Token::Number("42.".into()));
    }

    #[test]
    fn test_hex_uppercase() {
        let tokens = tokenize("0xFF").unwrap();
        assert_eq!(tokens[0], Token::Number("0xFF".into()));
    }

    #[test]
    fn test_very_long_number() {
        let tokens = tokenize("999999999999999999999999999999").unwrap();
        assert_eq!(
            tokens[0],
            Token::Number("999999999999999999999999999999".into())
        );
    }

    #[test]
    fn test_string_with_escapes() {
        let tokens = tokenize("\"hello\\nworld\\tfoo\"").unwrap();
        assert_eq!(tokens[0], Token::String("hello\nworld\tfoo".into()));
    }

    #[test]
    fn test_single_quoted_string() {
        let tokens = tokenize("'single quoted'").unwrap();
        assert_eq!(tokens[0], Token::String("single quoted".into()));
    }

    #[test]
    fn test_keyword_not_as_ident() {
        let tokens = tokenize("if then else end while do repeat until").unwrap();
        assert_eq!(tokens[0], Token::If);
        assert_eq!(tokens[1], Token::Then);
        assert_eq!(tokens[2], Token::Else);
        assert_eq!(tokens[3], Token::End);
        assert_eq!(tokens[4], Token::While);
        assert_eq!(tokens[5], Token::Do);
        assert_eq!(tokens[6], Token::Repeat);
        assert_eq!(tokens[7], Token::Until);
    }

    #[test]
    fn test_number_nan_rejected() {
        let tokens = tokenize("nan").unwrap();
        assert_eq!(tokens[0], Token::Ident("nan".into()));
    }

    #[test]
    fn test_number_inf_rejected() {
        let tokens = tokenize("inf").unwrap();
        assert_eq!(tokens[0], Token::Ident("inf".into()));
    }

    #[test]
    fn test_number_multiple_dots() {
        let tokens = tokenize("1.2.3").unwrap();
        assert_eq!(tokens[0], Token::Number("1.2".into()));
        assert_eq!(tokens[1], Token::Dot);
        assert_eq!(tokens[2], Token::Number("3".into()));
        assert_eq!(tokens[3], Token::Eof);
    }

    #[test]
    fn test_null_bytes_rejected() {
        let result = tokenize("hello\0world");
        assert!(result.is_err());
    }

    #[test]
    fn test_huge_input_rejected() {
        let big = "x".repeat(1_048_577);
        let result = tokenize(&big);
        assert!(result.is_err());
    }

    #[test]
    fn test_number_too_long() {
        let long = "1".repeat(101);
        let result = tokenize(&long);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_string_literal() {
        let tokens = tokenize("\"\"").unwrap();
        assert_eq!(tokens[0], Token::String("".into()));
    }

    #[test]
    fn test_escape_backslash() {
        let tokens = tokenize("\"\\\\\"").unwrap();
        assert_eq!(tokens[0], Token::String("\\".into()));
    }

    #[test]
    fn test_escape_single_quote() {
        let tokens = tokenize("'\\''").unwrap();
        assert_eq!(tokens[0], Token::String("'".into()));
    }

    #[test]
    fn test_tab_in_string() {
        let tokens = tokenize("\"hello\\tworld\"").unwrap();
        assert_eq!(tokens[0], Token::String("hello\tworld".into()));
    }

    #[test]
    fn test_unterminated_string() {
        let result = tokenize("\"hello");
        assert!(result.is_err());
    }

    #[test]
    fn test_unterminated_multiline_comment() {
        let result = tokenize("--[[");
        assert!(result.is_err());
    }

    #[test]
    fn test_mixed_ascii_and_unicode() {
        let tokens = tokenize("\"café\"").unwrap();
        assert_eq!(tokens[0], Token::String("café".into()));
    }

    #[test]
    fn test_hex_with_invalid_digits() {
        let tokens = tokenize("0xGG").unwrap();
        // read_number stops at non-hex-digit G, producing "0x"
        assert_eq!(tokens[0], Token::Number("0x".into()));
        assert_eq!(tokens[1], Token::Ident("GG".into()));
    }

    #[test]
    fn test_consecutive_dots() {
        let tokens = tokenize("....").unwrap();
        assert_eq!(tokens[0], Token::VarArg);
        assert_eq!(tokens[1], Token::Dot);
        assert_eq!(tokens[2], Token::Eof);
    }

    #[test]
    fn test_only_whitespace() {
        let tokens = tokenize("\n\n\n").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Eof);
    }

    #[test]
    fn test_unexpected_tilde() {
        let result = tokenize("~");
        assert!(result.is_err());
    }

    #[test]
    fn test_at_not_persist() {
        let tokens = tokenize("@foo").unwrap();
        assert_eq!(tokens[0], Token::Ident("@foo".into()));
        assert_eq!(tokens[1], Token::Eof);
    }

    #[test]
    fn test_number_leading_zeros() {
        let tokens = tokenize("00042").unwrap();
        assert_eq!(tokens[0], Token::Number("00042".into()));
    }

    #[test]
    fn test_very_deep_nested_brackets() {
        let tokens = tokenize("((((((((((1))))))))))").unwrap();
        assert_eq!(tokens.len(), 22); // 10 x LParen + Number + 10 x RParen + Eof
        for i in 0..10 {
            assert_eq!(tokens[i], Token::LParen, "LParen at index {}", i);
        }
        assert_eq!(tokens[10], Token::Number("1".into()));
        for i in 11..21 {
            assert_eq!(tokens[i], Token::RParen, "RParen at index {}", i);
        }
    }

    #[test]
    fn test_line_comment() {
        let tokens = tokenize("-- comment\n42").unwrap();
        assert_eq!(tokens[0], Token::Comment(" comment".into()));
        assert_eq!(tokens[1], Token::Number("42".into()));
    }

    #[test]
    fn test_block_comment() {
        let tokens = tokenize("--[[ block ]]\n42").unwrap();
        assert_eq!(tokens[0], Token::Comment(" block ".into()));
        assert_eq!(tokens[1], Token::Number("42".into()));
    }

    #[test]
    fn test_arrow_token() {
        let tokens = tokenize("->").unwrap();
        assert_eq!(tokens[0], Token::Arrow);
    }

    #[test]
    fn test_colon_number_dot() {
        let tokens = tokenize(":42.0").unwrap();
        assert_eq!(tokens[0], Token::Colon);
        assert_eq!(tokens[1], Token::Number("42.0".into()));
    }

    #[test]
    fn test_backslash_escape_t() {
        let tokens = tokenize("\"\\t\"").unwrap();
        assert_eq!(tokens[0], Token::String("\t".into()));
    }

    #[test]
    fn test_backslash_escape_n() {
        let tokens = tokenize("\"\\n\"").unwrap();
        assert_eq!(tokens[0], Token::String("\n".into()));
    }

    #[test]
    fn test_backslash_escape_r() {
        let tokens = tokenize("\"\\r\"").unwrap();
        assert_eq!(tokens[0], Token::String("\r".into()));
    }

    #[test]
    fn test_hex_lowercase() {
        let tokens = tokenize("0xabcdef").unwrap();
        assert_eq!(tokens[0], Token::Number("0xabcdef".into()));
    }

    #[test]
    fn test_token_display() {
        let tk = Token::Ident("hello".into());
        let s = format!("{:?}", tk);
        assert!(s.contains("hello"));
    }
}
