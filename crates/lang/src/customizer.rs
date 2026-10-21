//! Customizer annotations: the comments the reference implementation reads
//! to build its parameter panel.
//!
//! Rules observed in the reference release (2021.01):
//!
//! - Only top-level assignments of literals count, and only on lines before
//!   the first `{` of the file (braces inside comments are ignored, braces
//!   inside strings are not).
//! - Group: the last single-line `/* ... */` comment on an earlier line.
//!   Its name is the text between `[` and `]` when the comment opens with
//!   `[`, otherwise empty.
//! - Description: the line just above, if it is a non-empty `//` comment.
//! - Parameter: a `//` comment after the first `;` of the assignment's line
//!   (block comments may sit in between). Its text is read as a small value
//!   grammar (numbers, strings, free text, `[min:max]`, `[a, b:label]`);
//!   text that does not fit yields an empty string.

use crate::ast::{Assignment, Element, Expr, ExprKind, File};
use crate::dump::{number, string};
use crate::scope::Scope;
use crate::span::{LineIndex, Span};

/// A constant, as customizer comments and parameter sets write them.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Undef,
    Bool(bool),
    Number(f64),
    Str(String),
    /// `[begin : end]` or `[begin : step : end]`, as written.
    Range { begin: f64, step: Option<f64>, end: f64 },
    Vector(Vec<Literal>),
}

impl Literal {
    /// The reference's notation for the value, as `-o x.ast` prints it.
    pub fn to_source(&self) -> String {
        match self {
            Literal::Undef => "undef".into(),
            Literal::Bool(b) => b.to_string(),
            Literal::Number(n) => number(*n),
            Literal::Str(s) => string(s),
            Literal::Range { begin, step: None, end } => format!("[{} : {}]", number(*begin), number(*end)),
            Literal::Range { begin, step: Some(step), end } => {
                format!("[{} : {} : {}]", number(*begin), number(*step), number(*end))
            }
            Literal::Vector(items) => {
                let items: Vec<String> = items.iter().map(Literal::to_source).collect();
                format!("[{}]", items.join(", "))
            }
        }
    }

    /// The expression that writes this value, every node spanning `span`.
    pub fn to_expr(&self, span: Span) -> Expr {
        let boxed = |n: f64| Box::new(Expr { kind: ExprKind::Number(n), span });
        let kind = match self {
            Literal::Undef => ExprKind::Undef,
            Literal::Bool(b) => ExprKind::Bool(*b),
            Literal::Number(n) => ExprKind::Number(*n),
            Literal::Str(s) => ExprKind::Str(s.clone()),
            Literal::Range { begin, step, end } => {
                ExprKind::Range { start: boxed(*begin), step: step.map(boxed), end: boxed(*end) }
            }
            Literal::Vector(items) => {
                ExprKind::Vector(items.iter().map(|item| Element::Expr(item.to_expr(span))).collect())
            }
        };
        Expr { kind, span }
    }
}

/// What the comments around a customizer parameter say.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    /// The group opened by the last `/* [Name] */` above, if any.
    pub group: Option<String>,
    /// The `//` comment on the line above.
    pub description: Option<String>,
    /// The trailing `// ...` comment: the allowed values. An empty string
    /// when there is none or it does not parse.
    pub parameter: Literal,
}

/// The top-level assignments the reference's customizer reads, with their
/// annotations, in scope order: a repeated name keeps its first position
/// and its last value.
pub fn annotated<'f>(file: &'f File, src: &str) -> Vec<(&'f Assignment, Annotation)> {
    let customizer = Customizer::new(src);
    Scope::collect(&file.stmts)
        .assignments
        .into_iter()
        .filter_map(|(stmt, a)| Some((a, customizer.annotate(stmt.span, &a.value)?)))
        .collect()
}

/// Reads a value in the comments' grammar, as parameter sets store them:
/// `12`, `[1, 2]`, `[0:10]`, `"text"`, or bare words (a string).
pub fn parse_value(text: &str) -> Option<Literal> {
    ValueParser::new(text).parse()
}

pub(crate) struct Customizer<'a> {
    src: &'a str,
    lines: LineIndex,
    stop_line: usize,
    /// (line where the comment starts, group name), in source order.
    groups: Vec<(usize, String)>,
}

impl<'a> Customizer<'a> {
    pub fn new(src: &'a str) -> Self {
        let lines = LineIndex::new(src);
        let bytes = src.as_bytes();
        let mut stop_line = usize::MAX;
        let mut groups = Vec::new();
        let mut in_string = false;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'{' if stop_line == usize::MAX => stop_line = lines.line(i),
                b'\\' if in_string => i += 1,
                b'"' => in_string = !in_string,
                b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => {
                    i = src[i..].find('\n').map_or(bytes.len(), |n| i + n);
                }
                b'/' if !in_string && bytes.get(i + 1) == Some(&b'*') => {
                    let end = src[i + 2..].find("*/").map_or(bytes.len(), |n| i + 2 + n);
                    let body = &src[i + 2..end];
                    if !body.contains('\n') {
                        groups.push((lines.line(i), group_name(body)));
                    }
                    i = end + 2;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        Customizer { src, lines, stop_line, groups }
    }

    pub fn annotate(&self, span: Span, value: &Expr) -> Option<Annotation> {
        // Comments are only read from the main file's own text.
        if span.file != 0 {
            return None;
        }
        let line = self.lines.line(span.start as usize);
        if !value.is_literal() || line >= self.stop_line {
            return None;
        }
        let group = self.groups.iter().rev().find(|(l, _)| *l < line).map(|(_, name)| name.clone());
        let description = line
            .checked_sub(1)
            .and_then(|above| self.lines.line_text(self.src, above).trim_start().strip_prefix("//"))
            .map(|text| text.trim_start().to_string())
            .filter(|text| !text.is_empty());
        // The reference keeps the `\r` of CRLF files in the comment, which
        // then fails to parse; the raw line reproduces that.
        let parameter = trailing_comment(self.lines.raw_line_text(self.src, line))
            .and_then(|text| ValueParser::new(text).parse())
            .unwrap_or_else(|| Literal::Str(String::new()));
        Some(Annotation { group, description, parameter })
    }
}

fn group_name(comment: &str) -> String {
    comment
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map_or(String::new(), |(name, _)| name.to_string())
}

/// The text of a `//` comment following the first `;` of the line.
fn trailing_comment(line: &str) -> Option<&str> {
    let mut in_string = false;
    let mut escaped = false;
    let semi = line.char_indices().find_map(|(i, c)| {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            ';' if !in_string => return Some(i),
            _ => {}
        }
        None
    })?;
    let mut rest = line[semi + 1..].trim_start();
    while let Some(after) = rest.strip_prefix("/*") {
        rest = after.split_once("*/")?.1.trim_start();
    }
    rest.strip_prefix("//")
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A number, with its source text for use inside free text.
    Num(f64, String),
    Word(String),
    Str(String),
    LBracket,
    RBracket,
    Comma,
    Colon,
}

/// The annotation value grammar:
///
/// ```text
/// value := atom | '[' num ':' num (':' num)? ']' | '[' item (',' item)* ']'
/// item  := atom (':' atom)?        -- `value:label` becomes [value, label]
/// atom  := num | "string" | text
/// text  := (word | num)+           -- at least one word, joined by spaces
/// ```
///
/// Words are runs of anything but whitespace, `[ ] , : " ( )`; parentheses
/// are skipped like whitespace.
struct ValueParser {
    tokens: Vec<Token>,
    pos: usize,
}

impl ValueParser {
    fn new(text: &str) -> Self {
        ValueParser { tokens: lex_value(text).unwrap_or_default(), pos: 0 }
    }

    fn parse(mut self) -> Option<Literal> {
        if self.tokens.is_empty() {
            return None;
        }
        let value = if self.tokens[0] == Token::LBracket { self.bracket()? } else { self.atom()? };
        (self.pos == self.tokens.len()).then_some(value)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == Some(token);
        self.pos += found as usize;
        found
    }

    fn number(&mut self) -> Option<f64> {
        match self.peek()? {
            &Token::Num(n, _) => {
                self.pos += 1;
                Some(n)
            }
            _ => None,
        }
    }

    fn atom(&mut self) -> Option<Literal> {
        if let Some(Token::Str(s)) = self.peek() {
            let s = Literal::Str(s.clone());
            self.pos += 1;
            return Some(s);
        }
        let start = self.pos;
        let mut parts = Vec::new();
        let mut has_word = false;
        while let Some(token) = self.peek() {
            match token {
                Token::Word(w) => {
                    has_word = true;
                    parts.push(w.clone());
                }
                Token::Num(_, text) => parts.push(text.clone()),
                _ => break,
            }
            self.pos += 1;
        }
        match (parts.len(), has_word) {
            (0, _) => None,
            (_, true) => Some(Literal::Str(parts.join(" "))),
            (1, false) => match self.tokens[start] {
                Token::Num(n, _) => Some(Literal::Number(n)),
                _ => None,
            },
            _ => None,
        }
    }

    fn bracket(&mut self) -> Option<Literal> {
        self.eat(&Token::LBracket);
        let start = self.pos;
        if let Some(range) = self.range() {
            return Some(range);
        }
        self.pos = start;

        let mut items = Vec::new();
        loop {
            let value = self.atom()?;
            items.push(if self.eat(&Token::Colon) { Literal::Vector(vec![value, self.atom()?]) } else { value });
            if self.eat(&Token::RBracket) {
                return Some(Literal::Vector(items));
            }
            if !self.eat(&Token::Comma) {
                return None;
            }
        }
    }

    fn range(&mut self) -> Option<Literal> {
        let first = self.number()?;
        let mut parts = vec![first];
        while self.eat(&Token::Colon) {
            parts.push(self.number()?);
        }
        if !self.eat(&Token::RBracket) {
            return None;
        }
        match parts[..] {
            [begin, end] => Some(Literal::Range { begin, step: None, end }),
            [begin, step, end] => Some(Literal::Range { begin, step: Some(step), end }),
            _ => None,
        }
    }
}

fn lex_value(text: &str) -> Option<Vec<Token>> {
    let is_delimiter = |c: char| c.is_whitespace() || "[],:\"()".contains(c);
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let token = match c {
            '\r' => return None,
            c if c.is_whitespace() || c == '(' || c == ')' => None,
            '[' => Some(Token::LBracket),
            ']' => Some(Token::RBracket),
            ',' => Some(Token::Comma),
            ':' => Some(Token::Colon),
            '"' => {
                let mut value = String::new();
                let mut chars = rest[1..].char_indices();
                loop {
                    match chars.next()? {
                        (end, '"') => {
                            rest = &rest[1 + end + 1..];
                            break;
                        }
                        (_, '\\') => value.extend(chars.next().map(|(_, c)| c)),
                        (_, c) => value.push(c),
                    }
                }
                tokens.push(Token::Str(value));
                continue;
            }
            _ => {
                let end = rest.find(is_delimiter).unwrap_or(rest.len());
                let word = &rest[..end];
                rest = &rest[end..];
                tokens.push(match parse_number(word) {
                    Some(n) => Token::Num(n, word.to_string()),
                    None => Token::Word(word.to_string()),
                });
                continue;
            }
        };
        tokens.extend(token);
        rest = &rest[c.len_utf8()..];
    }
    Some(tokens)
}

/// `-?` then a number in .scad syntax: `1`, `1.`, `.5`, `1e3`, `2.5E-2`.
fn parse_number(word: &str) -> Option<f64> {
    let digits = word.strip_prefix('-').unwrap_or(word);
    let (mantissa, exponent) = match digits.find(['e', 'E']) {
        Some(i) => (&digits[..i], Some(&digits[i + 1..])),
        None => (digits, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let exponent_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && all_digits(e)
    });
    let mantissa_ok = (!int.is_empty() || !frac.is_empty()) && all_digits(int) && all_digits(frac);
    if mantissa_ok && exponent_ok { word.parse().ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Option<String> {
        parse_value(text).map(|v| v.to_source())
    }

    #[test]
    fn value_grammar() {
        assert_eq!(value(" [5:50]").as_deref(), Some("[5 : 50]"));
        assert_eq!(value(" [0.5:0.1:2.5]").as_deref(), Some("[0.5 : 0.1 : 2.5]"));
        assert_eq!(value(" [-5:5]").as_deref(), Some("[-5 : 5]"));
        assert_eq!(value(" 10").as_deref(), Some("10"));
        assert_eq!(value(" -3").as_deref(), Some("-3"));
        assert_eq!(value(" hello world").as_deref(), Some("\"hello world\""));
        assert_eq!(value(" [10:Small, 20:Large]").as_deref(), Some("[[10, \"Small\"], [20, \"Large\"]]"));
        assert_eq!(value(r#" ["a", "b":"Bee"]"#).as_deref(), Some(r#"["a", ["b", "Bee"]]"#));
        assert_eq!(value(" [1, \"x\", y z]").as_deref(), Some("[1, \"x\", \"y z\"]"));
        assert_eq!(value(" Don't go below 0.1 mm").as_deref(), Some("\"Don't go below 0.1 mm\""));
        assert_eq!(value(" 10x10 pixels").as_deref(), Some("\"10x10 pixels\""));
        assert_eq!(value(" a (b)").as_deref(), Some("\"a b\""));
        assert_eq!(value(" -a").as_deref(), Some("\"-a\""));
        assert_eq!(value(" [1:2, 3:4]").as_deref(), Some("[[1, 2], [3, 4]]"));
        for bad in ["", " hello [1:2]", " 1 2", " abc, def", " [0:1:2:3]", " []", " [[1,2]]", " \"a\":\"b\"", " [1:2]x", " a [b]", " a: b", " [1 2]", " [1:a:3]"] {
            assert_eq!(value(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn trailing_comments() {
        assert_eq!(trailing_comment("x = 1; // [1:2]"), Some(" [1:2]"));
        assert_eq!(trailing_comment("x = 1; /* c */ // [1:2]"), Some(" [1:2]"));
        assert_eq!(trailing_comment(r#"s = "a;b"; // [x]"#), Some(" [x]"));
        assert_eq!(trailing_comment("x = 1; y = 2; // [1:2]"), None);
        assert_eq!(trailing_comment("x ="), None);
    }
}
