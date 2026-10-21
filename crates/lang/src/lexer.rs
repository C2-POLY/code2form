//! Tokenizer.
//!
//! Matches the lexical quirks of the reference implementation: identifiers
//! may start with a digit (`1abc`, `0x10`), and when a number and an
//! identifier match the same text, the longer match wins with ties going to
//! the number (`1e3` is a number, `1e` is an identifier).

use crate::diagnostic::Diagnostic;
use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Number(f64),
    Str(String),
    Include(String),
    Use(String),

    Module,
    Function,
    If,
    Else,
    For,
    Let,
    Assert,
    Echo,
    Each,
    True,
    False,
    Undef,

    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Colon,
    Question,
    Dot,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Bang,
    Hash,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,

    Eof,
}

impl Tok {
    /// How the token reads in an error message.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(name) => format!("identifier `{name}`"),
            Tok::Number(_) => "number".into(),
            Tok::Str(_) => "string".into(),
            Tok::Include(_) => "`include`".into(),
            Tok::Use(_) => "`use`".into(),
            Tok::Eof => "end of file".into(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Tok::Module => "module",
            Tok::Function => "function",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::For => "for",
            Tok::Let => "let",
            Tok::Assert => "assert",
            Tok::Echo => "echo",
            Tok::Each => "each",
            Tok::True => "true",
            Tok::False => "false",
            Tok::Undef => "undef",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::Semi => ";",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Question => "?",
            Tok::Dot => ".",
            Tok::Assign => "=",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Caret => "^",
            Tok::Bang => "!",
            Tok::Hash => "#",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::EqEq => "==",
            Tok::Ne => "!=",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            _ => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, Diagnostic> {
    tokenize_file(src, 0, None)
}

/// Tokenizes file `file`. With `includes`, each `include <f>` is replaced by
/// the tokens of `f`, as the reference's lexer does: an include may sit
/// anywhere, even inside an expression. Without it, includes stay as tokens.
pub fn tokenize_file(
    src: &str,
    file: u32,
    mut includes: Option<&mut dyn crate::parser::Includes>,
) -> Result<Vec<Token>, Diagnostic> {
    let mut tokens = Vec::new();
    lex_into(src, file, &mut includes, &mut tokens)?;
    tokens.push(Token { tok: Tok::Eof, span: Span::in_file(src.len(), src.len(), file) });
    Ok(tokens)
}

fn lex_into(
    src: &str,
    file: u32,
    includes: &mut Option<&mut dyn crate::parser::Includes>,
    tokens: &mut Vec<Token>,
) -> Result<(), Diagnostic> {
    let mut lexer = Lexer { src, bytes: src.as_bytes(), pos: 0, file };
    loop {
        let token = lexer.next_token()?;
        match (&token.tok, includes.as_deref_mut()) {
            (Tok::Eof, _) => return Ok(()),
            (Tok::Include(name), Some(resolver)) => {
                if let Some((id, text)) = resolver.open(name, file) {
                    let result = lex_into(&text, id, includes, tokens);
                    if let Some(resolver) = includes.as_deref_mut() {
                        resolver.close(id);
                    }
                    result?;
                }
            }
            _ => tokens.push(token),
        }
    }
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    file: u32,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Lexer<'_> {
    fn span(&self, start: usize, end: usize) -> Span {
        Span::in_file(start, end, self.file)
    }

    fn peek(&self, ahead: usize) -> u8 {
        self.bytes.get(self.pos + ahead).copied().unwrap_or(0)
    }

    fn skip_trivia(&mut self) -> Result<(), Diagnostic> {
        loop {
            match (self.peek(0), self.peek(1)) {
                // The reference puts an end-of-text character between the
                // file and the `-D` assignments it appends, and ignores it.
                (b' ' | b'\t' | b'\r' | b'\n' | 0x03, _) => self.pos += 1,
                // The reference also skips a no-break space, and the
                // replacement character for a stray non-UTF-8 byte.
                (0xC2, 0xA0) => self.pos += 2,
                (0xEF, 0xBF) if self.peek(2) == 0xBD => self.pos += 3,
                (b'/', b'/') => {
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                (b'/', b'*') => {
                    let start = self.pos;
                    match self.src[self.pos + 2..].find("*/") {
                        Some(end) => self.pos += 2 + end + 2,
                        None => {
                            return Err(Diagnostic::new(
                                "unterminated comment",
                                self.span(start, start + 2),
                            ));
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, Diagnostic> {
        loop {
            self.skip_trivia()?;
            let start = self.pos;
            if start >= self.bytes.len() {
                return Ok(Token { tok: Tok::Eof, span: self.span(start, start) });
            }

            let b = self.bytes[start];
            let tok = if is_ident_byte(b) || b == b'$' || (b == b'.' && self.peek(1).is_ascii_digit()) {
                self.word()?
            } else if b == b'"' {
                Some(self.string()?)
            } else {
                Some(self.punct()?)
            };
            // `None`: a literal the reference's lexer drops; read on.
            if let Some(tok) = tok {
                return Ok(Token { tok, span: self.span(start, self.pos) });
            }
        }
    }

    /// Length of the number starting at the cursor, or 0.
    fn number_len(&self) -> usize {
        let digits = |mut i: usize| {
            while self.bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            i
        };
        let start = self.pos;
        let int_end = digits(start);
        let mut end = int_end;
        if self.bytes.get(end) == Some(&b'.') {
            let frac_end = digits(end + 1);
            if int_end > start || frac_end > end + 1 {
                end = frac_end;
            }
        }
        if end == start {
            return 0;
        }
        if matches!(self.bytes.get(end), Some(b'e' | b'E')) {
            let mut exp = end + 1;
            if matches!(self.bytes.get(exp), Some(b'+' | b'-')) {
                exp += 1;
            }
            let exp_end = digits(exp);
            if exp_end > exp {
                end = exp_end;
            }
        }
        end - start
    }

    fn ident_len(&self) -> usize {
        let mut i = self.pos;
        if self.bytes[i] == b'$' {
            i += 1;
        }
        let body = i;
        while self.bytes.get(i).copied().is_some_and(is_ident_byte) {
            i += 1;
        }
        if i == body { 0 } else { i - self.pos }
    }

    /// A number, keyword or identifier. A number that overflows or
    /// underflows a double yields no token at all (`None`), like the
    /// `strtod` range error that makes the reference's lexer drop it.
    fn word(&mut self) -> Result<Option<Tok>, Diagnostic> {
        let start = self.pos;
        let number = self.number_len();
        let ident = self.ident_len();

        if number > 0 && number >= ident {
            self.pos += number;
            let text = &self.src[start..self.pos];
            let value: f64 = text.parse().map_err(|_| {
                Diagnostic::new(format!("invalid number `{text}`"), self.span(start, self.pos))
            })?;
            let mantissa = text.split(['e', 'E']).next().unwrap_or("");
            let underflow = value == 0.0 && mantissa.bytes().any(|b| (b'1'..=b'9').contains(&b));
            if value.is_infinite() || underflow || (value != 0.0 && value.abs() < f64::MIN_POSITIVE) {
                return Ok(None);
            }
            return Ok(Some(Tok::Number(value)));
        }
        if ident == 0 {
            return Err(Diagnostic::new("expected an identifier after `$`", self.span(start, start + 1)));
        }

        self.pos += ident;
        let text = &self.src[start..self.pos];
        Ok(Some(match text {
            "module" => Tok::Module,
            "function" => Tok::Function,
            "if" => Tok::If,
            "else" => Tok::Else,
            "for" => Tok::For,
            "let" => Tok::Let,
            "assert" => Tok::Assert,
            "echo" => Tok::Echo,
            "each" => Tok::Each,
            "true" => Tok::True,
            "false" => Tok::False,
            "undef" => Tok::Undef,
            "include" | "use" => match self.import_path()? {
                Some(path) if text == "include" => Tok::Include(path),
                Some(path) => Tok::Use(path),
                None => Tok::Ident(text.to_string()),
            },
            _ => Tok::Ident(text.to_string()),
        }))
    }

    /// After `include`/`use`: an optional `<path>`, possibly preceded by
    /// whitespace. Leaves the cursor untouched when there is none.
    fn import_path(&mut self) -> Result<Option<String>, Diagnostic> {
        let mut i = self.pos;
        while matches!(self.bytes.get(i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            i += 1;
        }
        if self.bytes.get(i) != Some(&b'<') {
            return Ok(None);
        }
        let open = i;
        match self.src[open + 1..].find('>') {
            Some(len) => {
                self.pos = open + 1 + len + 1;
                Ok(Some(self.src[open + 1..open + 1 + len].to_string()))
            }
            None => Err(Diagnostic::new("unterminated import path", self.span(open, open + 1))),
        }
    }

    fn string(&mut self) -> Result<Tok, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        loop {
            let Some(c) = self.src[self.pos..].chars().next() else {
                return Err(Diagnostic::new("unterminated string", self.span(start, start + 1)));
            };
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(Tok::Str(value)),
                '\\' => self.escape(&mut value),
                // The reference drops a raw line break inside a string.
                '\n' => {}
                c => value.push(c),
            }
        }
    }

    /// Decodes one escape sequence (the backslash is already consumed).
    /// Unknown escapes stand for the escaped character, as in the reference.
    fn escape(&mut self, out: &mut String) {
        let hex = |s: &str, n: usize| {
            let digits = s.get(..n)?;
            if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            u32::from_str_radix(digits, 16).ok()
        };
        let rest = &self.src[self.pos..];
        let Some(c) = rest.chars().next() else { return };
        self.pos += c.len_utf8();
        match c {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            // `\x` takes an ASCII code: two hex digits, the first one 0-7.
            // The reference turns a NUL from `\x00` or `\u0000` into a space.
            'x' => match hex(&rest[1..], 2).filter(|&v| v < 0x80) {
                Some(v) => {
                    self.pos += 2;
                    out.push(if v == 0 { ' ' } else { char::from(v as u8) });
                }
                None => out.push('x'),
            },
            'u' | 'U' => {
                let len = if c == 'u' { 4 } else { 6 };
                match hex(&rest[1..], len) {
                    Some(v) => {
                        self.pos += len;
                        // NUL and code points with no character (surrogates,
                        // beyond U+10FFFF) become a space.
                        out.push(char::from_u32(v).filter(|&c| c != '\0').unwrap_or(' '));
                    }
                    None => out.push(c),
                }
            }
            // A backslash before a line break removes both.
            '\n' => {}
            c => out.push(c),
        }
    }

    fn punct(&mut self) -> Result<Tok, Diagnostic> {
        let two = |second: u8, double: Tok, single: Tok| {
            if self.peek(1) == second { (double, 2) } else { (single, 1) }
        };
        let (tok, len) = match self.bytes[self.pos] {
            b'(' => (Tok::LParen, 1),
            b')' => (Tok::RParen, 1),
            b'[' => (Tok::LBracket, 1),
            b']' => (Tok::RBracket, 1),
            b'{' => (Tok::LBrace, 1),
            b'}' => (Tok::RBrace, 1),
            b';' => (Tok::Semi, 1),
            b',' => (Tok::Comma, 1),
            b':' => (Tok::Colon, 1),
            b'?' => (Tok::Question, 1),
            b'.' => (Tok::Dot, 1),
            b'+' => (Tok::Plus, 1),
            b'-' => (Tok::Minus, 1),
            b'*' => (Tok::Star, 1),
            b'/' => (Tok::Slash, 1),
            b'%' => (Tok::Percent, 1),
            b'^' => (Tok::Caret, 1),
            b'#' => (Tok::Hash, 1),
            b'=' => two(b'=', Tok::EqEq, Tok::Assign),
            b'!' => two(b'=', Tok::Ne, Tok::Bang),
            b'<' => two(b'=', Tok::Le, Tok::Lt),
            b'>' => two(b'=', Tok::Ge, Tok::Gt),
            b'&' if self.peek(1) == b'&' => (Tok::AndAnd, 2),
            b'|' if self.peek(1) == b'|' => (Tok::OrOr, 2),
            _ => {
                let c = self.src[self.pos..].chars().next().unwrap_or('?');
                return Err(Diagnostic::new(
                    format!("unexpected character `{c}`"),
                    self.span(self.pos, self.pos + c.len_utf8()),
                ));
            }
        };
        self.pos += len;
        Ok(tok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        tokenize(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn number_identifier_overlap() {
        assert_eq!(toks("1e3"), [Tok::Number(1000.0), Tok::Eof]);
        assert_eq!(toks("1e"), [Tok::Ident("1e".into()), Tok::Eof]);
        assert_eq!(toks("0x10"), [Tok::Ident("0x10".into()), Tok::Eof]);
        assert_eq!(toks("1abc"), [Tok::Ident("1abc".into()), Tok::Eof]);
        assert_eq!(toks("1. .5 1.e2"), [Tok::Number(1.0), Tok::Number(0.5), Tok::Number(100.0), Tok::Eof]);
        assert_eq!(toks("$fn"), [Tok::Ident("$fn".into()), Tok::Eof]);
    }

    #[test]
    fn string_escapes() {
        let src = r#""a\"b\\c\n\x41\x80é\U01F600\q""#;
        assert_eq!(toks(src)[0], Tok::Str("a\"b\\c\nAx80é😀q".into()));
    }

    #[test]
    fn whitespace_and_escape_quirks() {
        assert_eq!(toks("a\u{a0}=\u{a0}1"), [Tok::Ident("a".into()), Tok::Assign, Tok::Number(1.0), Tok::Eof]);
        assert_eq!(toks(r#""[\uD800][\U110000]""#)[0], Tok::Str("[ ][ ]".into()));
        assert_eq!(toks("\"a\\\nb\"")[0], Tok::Str("ab".into()));
        assert_eq!(toks("a\n\x03\n1"), [Tok::Ident("a".into()), Tok::Number(1.0), Tok::Eof]);
    }

    #[test]
    fn imports() {
        assert_eq!(toks("include <a/b.scad>"), [Tok::Include("a/b.scad".into()), Tok::Eof]);
        assert_eq!(toks("use<x.scad>"), [Tok::Use("x.scad".into()), Tok::Eof]);
        assert_eq!(toks("use = 1"), [Tok::Ident("use".into()), Tok::Assign, Tok::Number(1.0), Tok::Eof]);
    }

    #[test]
    fn out_of_range_literals_vanish() {
        assert_eq!(toks("1, 5e-324, 1e400, 1e-400, 2"), [Tok::Number(1.0), Tok::Comma, Tok::Comma, Tok::Comma, Tok::Comma, Tok::Number(2.0), Tok::Eof]);
        assert_eq!(toks("2.2250738585072014e-308"), [Tok::Number(f64::MIN_POSITIVE), Tok::Eof]);
        assert_eq!(toks("0e5 0.0"), [Tok::Number(0.0), Tok::Number(0.0), Tok::Eof]);
    }

    #[test]
    fn errors() {
        assert!(tokenize("\"open").is_err());
        assert!(tokenize("/* open").is_err());
    }
}
