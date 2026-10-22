//! The changes manual modelling makes to the source: the properties panel
//! rewriting an argument (C2F-38), the move, turn and scale handles
//! (C2F-39), new shapes, booleans, holes, copies (C2F-40) and the tree of
//! the program, with its names, modifiers and drag and drop (C2F-41).
//!
//! Every edit is a pure function of the source text that returns the whole
//! new text in an [`Edit`], so that the editor applies it as one undo step,
//! or a short message, in French, saying why it cannot be done. Only the
//! edited region changes: formatting and comments elsewhere stay as written.
//!
//! An object is named by the byte range of the instance statement that
//! makes it, in the main file, as the 3D view's selection gives it: from
//! its first modifier (`#`, `%`, `!`, `*`) or its module name through its
//! closing `;` or `}`, children included. In `translate([5, 0, 0]) cube(2);`
//! both the whole line and `cube(2);` are statements. Edits that move or
//! remove an object take the transforms written around it along: they work
//! on its *placement*, the outermost statement reached by climbing through
//! parents that are transforms with that single child (see [`placed`]).
//!
//! The source is read with the language's own parser and lexer, so that
//! comments and strings never mislead the edits; a source that does not
//! parse is refused, as an edit could not know what it changes.


use std::ops::Range;

use c2f_lang::ast::{Arg, Element, Expr, ExprKind, File, Ident, InstanceKind, Stmt, StmtKind, UnaryOp};
use c2f_lang::lexer::{Tok, Token};

mod create;
mod props;
mod transform;
mod tree;

#[cfg(test)]
mod test_util;

pub use create::{
    Boolean, Place, Primitive, copy_text, delete, duplicate, insert_shape, make_hole, paste, primitive_code,
    wrap_boolean,
};
pub use props::{ArgKey, ArgValue, Call, call_at, set_arg, set_arg_component};
#[cfg(test)]
pub use props::{add_arg, builtin_params};
pub use transform::{Transform, scale_by, set_transform, transform_of, translate_by};
pub use tree::{Modifier, NodeKind, Target, TreeNode, move_into, rename, set_modifier, tree};

/// The result of an edit: the whole new source, where to put the cursor
/// (a char index into it), and the byte range in it of the statement the
/// edit leaves selected (the object edited, created or moved; an empty
/// range where a deleted object was).
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub text: String,
    pub cursor: usize,
    pub selection: Range<usize>,
}

/// A value the source writes literally, which a panel can show and
/// rewrite: a number, a boolean, a string, a vector of numbers or `undef`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Undef,
    Bool(bool),
    Number(f64),
    Str(String),
    Vector(Vec<f64>),
}

impl Value {
    /// The value as the source writes it: `5`, `true`, `"M3"`,
    /// `[10, 0, 2.5]`.
    pub fn code(&self) -> String {
        match self {
            Value::Undef => "undef".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(x) => number(*x),
            Value::Str(s) => {
                let mut out = String::from('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        '\r' => out.push_str("\\r"),
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            Value::Vector(v) => vector(v),
        }
    }
}

/// A number as the source writes it: at most three decimals, no trailing
/// zeros, `0` rather than `-0` (`18.4`, `3`); a value that is not finite
/// is written `0`.
pub fn number(x: f64) -> String {
    if !x.is_finite() {
        return "0".into();
    }
    let text = format!("{x:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

/// `[1, 2, 3]`.
pub fn vector(v: &[f64]) -> String {
    let parts: Vec<String> = v.iter().map(|x| number(*x)).collect();
    format!("[{}]", parts.join(", "))
}

// ---- messages ----

const SYNTAX: &str = "Le code contient une erreur : corrigez-la avant de modifier le modèle.";
const STALE: &str = "La sélection ne correspond plus au code : sélectionnez l'objet à nouveau.";
const NOT_A_CALL: &str = "Cet élément n'est pas un appel de module.";
const UNBRACED: &str =
    "Cet objet est la seule instruction d'un if ou d'une boucle : ajoutez des accolades autour pour le modifier.";

/// The message for a value the source computes, which an edit would lose:
/// « La valeur est calculée (width / 2) : remplacez-la par un nombre pour
/// la modifier. »
fn computed(src: &str, expr: &Expr) -> String {
    let text: Vec<&str> = src[expr.span.range()].split_whitespace().collect();
    let mut text = text.join(" ");
    if text.chars().count() > 40 {
        text = text.chars().take(39).collect::<String>() + "…";
    }
    format!("La valeur est calculée ({text}) : remplacez-la par un nombre pour la modifier.")
}

// ---- the parsed source ----

/// The source with its syntax tree and its tokens (comments left out),
/// which locate the parentheses and braces the tree does not keep.
struct Doc<'s> {
    src: &'s str,
    file: File,
    tokens: Vec<Token>,
}

/// A statement and the list of statements it belongs to, one step of the
/// path from the top of the file down to a statement.
#[derive(Clone, Copy)]
struct Loc<'a> {
    stmt: &'a Stmt,
    list: &'a [Stmt],
    index: usize,
}

impl<'s> Doc<'s> {
    fn parse(src: &'s str) -> Result<Self, String> {
        let file = c2f_lang::parse(src).map_err(|_| SYNTAX.to_string())?;
        let tokens = c2f_lang::lexer::tokenize(src).map_err(|_| SYNTAX.to_string())?;
        Ok(Doc { src, file, tokens })
    }

    /// Index of the first token that starts at or after `offset`.
    fn token_at(&self, offset: usize) -> usize {
        self.tokens.partition_point(|t| (t.span.start as usize) < offset)
    }

    /// The path from the top of the file to the statement written exactly
    /// at `range`, the statement last.
    fn locate(&self, range: &Range<usize>) -> Result<Vec<Loc<'_>>, String> {
        let mut path = Vec::new();
        if find_path(&self.file.stmts, range, &mut path) { Ok(path) } else { Err(STALE.into()) }
    }

    /// Like [`Doc::locate`], for an instance statement.
    fn locate_instance(&self, range: &Range<usize>) -> Result<Vec<Loc<'_>>, String> {
        let path = self.locate(range)?;
        match path.last().map(|l| &l.stmt.kind) {
            Some(StmtKind::Instance(_)) => Ok(path),
            _ => Err(NOT_A_CALL.into()),
        }
    }

    /// The parentheses around a call's arguments: the bytes of `(` and of
    /// `)`.
    fn parens(&self, name: &Ident) -> Option<(usize, usize)> {
        let open = self.token_at(name.span.end as usize);
        if self.tokens.get(open)?.tok != Tok::LParen {
            return None;
        }
        let mut depth = 0usize;
        for token in &self.tokens[open..] {
            match token.tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((self.tokens[open].span.start as usize, token.span.start as usize));
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
        }
        None
    }

    /// The token right after a call's arguments: `;`, `{` or the start of
    /// its single child.
    fn after_args(&self, name: &Ident) -> Option<&Token> {
        let (_, close) = self.parens(name)?;
        self.tokens.get(self.token_at(close + 1))
    }

    /// Where a statement's modifiers end: its `if` or module name.
    fn after_modifiers(&self, stmt: &Stmt) -> usize {
        let mut i = self.token_at(stmt.span.start as usize);
        while matches!(self.tokens[i].tok, Tok::Bang | Tok::Hash | Tok::Percent | Tok::Star) {
            i += 1;
        }
        self.tokens[i].span.start as usize
    }
}

/// The statement lists a statement holds: a block's statements, a module's
/// body, a call's children, an `if`'s branches.
fn child_lists(stmt: &Stmt) -> Vec<&[Stmt]> {
    match &stmt.kind {
        StmtKind::Block(stmts) => vec![stmts],
        StmtKind::Module(m) => vec![std::slice::from_ref(&*m.body)],
        StmtKind::Instance(i) => match &i.kind {
            InstanceKind::Call { children, .. } => vec![children],
            InstanceKind::If { then, otherwise, .. } => {
                let mut lists: Vec<&[Stmt]> = vec![then];
                if let Some(otherwise) = otherwise {
                    lists.push(otherwise);
                }
                lists
            }
        },
        _ => Vec::new(),
    }
}

fn find_path<'a>(list: &'a [Stmt], range: &Range<usize>, path: &mut Vec<Loc<'a>>) -> bool {
    for (index, stmt) in list.iter().enumerate() {
        let r = stmt.span.range();
        if stmt.span.file != 0 || r.start > range.start || r.end < range.end {
            continue;
        }
        path.push(Loc { stmt, list, index });
        if r == *range {
            return true;
        }
        if child_lists(stmt).into_iter().any(|l| find_path(l, range, path)) {
            return true;
        }
        path.pop();
    }
    false
}

/// A module call's name, arguments and children.
fn call(stmt: &Stmt) -> Option<(&Ident, &[Arg], &[Stmt])> {
    match &stmt.kind {
        StmtKind::Instance(i) => match &i.kind {
            InstanceKind::Call { name, args, children } => Some((name, args, children)),
            InstanceKind::If { .. } => None,
        },
        _ => None,
    }
}

fn call_name(stmt: &Stmt) -> Option<&str> {
    call(stmt).map(|(name, _, _)| name.name.as_str())
}

/// Builtins that place, color or extrude their children and that an
/// object's placement climbs through.
const WRAPPERS: [&str; 11] = [
    "translate",
    "rotate",
    "scale",
    "mirror",
    "multmatrix",
    "resize",
    "color",
    "offset",
    "render",
    "linear_extrude",
    "rotate_extrude",
];

/// The single child of a wrapper builtin with one child.
fn wrapped_child(stmt: &Stmt) -> Option<&Stmt> {
    let (name, _, children) = call(stmt)?;
    match children {
        [child] if WRAPPERS.contains(&name.name.as_str()) && matches!(child.kind, StmtKind::Instance(_)) => Some(child),
        _ => None,
    }
}

/// The path to an object's placement: the outermost statement reached
/// from the last one by climbing through wrapper builtins of which it is
/// the single child (`translate`, `rotate`, `color`, `linear_extrude`...).
fn placed<'p, 'a>(path: &'p [Loc<'a>]) -> &'p [Loc<'a>] {
    let mut n = path.len();
    while n >= 2 && wrapped_child(path[n - 2].stmt).is_some() {
        n -= 1;
    }
    &path[..n]
}

/// Whether the last statement of the path sits in a list that may gain or
/// lose statements: the file, a block, or braced children. The single
/// unbraced child of a call or `if` does not.
fn in_list(doc: &Doc, path: &[Loc]) -> bool {
    let n = path.len();
    if n < 2 || matches!(path[n - 2].stmt.kind, StmtKind::Block(_)) {
        return true;
    }
    let first = &path[n - 1].list[0];
    let i = doc.token_at(first.span.start as usize);
    i > 0 && doc.tokens[i - 1].tok == Tok::LBrace
}

/// The path to the statement that moves, is copied or is removed with an
/// object: its placement, climbing on through calls and `if`s (without
/// `else`) of which it is the unbraced single child, so that removing it
/// never leaves `for (...)` without a body.
fn detached<'p, 'a>(doc: &Doc, path: &'p [Loc<'a>]) -> &'p [Loc<'a>] {
    let mut path = placed(path);
    while !in_list(doc, path) {
        let parent = path[path.len() - 2].stmt;
        let climbs = match &parent.kind {
            StmtKind::Instance(i) => match &i.kind {
                InstanceKind::Call { .. } => true,
                InstanceKind::If { otherwise, .. } => otherwise.is_none(),
            },
            _ => false,
        };
        if !climbs {
            break;
        }
        path = placed(&path[..path.len() - 1]);
    }
    path
}

// ---- literals ----

/// The value of an expression written as a literal: a number (possibly
/// signed or parenthesized), a boolean, a string, `undef` or a vector of
/// numbers.
fn literal(expr: &Expr) -> Option<Value> {
    match &expr.kind {
        ExprKind::Undef => Some(Value::Undef),
        ExprKind::Bool(b) => Some(Value::Bool(*b)),
        ExprKind::Str(s) => Some(Value::Str(s.clone())),
        ExprKind::Vector(elements) => {
            let numbers = elements.iter().map(|e| match e {
                Element::Expr(e) => scalar(e),
                Element::Comp(_) => None,
            });
            numbers.collect::<Option<Vec<f64>>>().map(Value::Vector)
        }
        _ => scalar(expr).map(Value::Number),
    }
}

/// A number written literally: `5`, `-2.5`, `(+3)`.
fn scalar(expr: &Expr) -> Option<f64> {
    match &expr.kind {
        ExprKind::Number(n) => Some(*n),
        ExprKind::Paren(inner) => scalar(inner),
        ExprKind::Unary { op: UnaryOp::Neg, operand } => scalar(operand).map(|n| -n),
        ExprKind::Unary { op: UnaryOp::Plus, operand } => scalar(operand),
        _ => None,
    }
}

// ---- text ----

fn line_start(src: &str, offset: usize) -> usize {
    src[..offset].rfind('\n').map_or(0, |i| i + 1)
}

/// The byte of the `\n` ending the line of `offset` (before a `\r`), or
/// the end of the source.
fn line_end(src: &str, offset: usize) -> usize {
    let end = src[offset..].find('\n').map_or(src.len(), |i| offset + i);
    if end > offset && src.as_bytes()[end - 1] == b'\r' { end - 1 } else { end }
}

/// The whitespace that starts the line of `offset`.
fn indent_of(src: &str, offset: usize) -> &str {
    let start = line_start(src, offset);
    let line = &src[start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Whether only whitespace comes before `offset` on its line.
fn starts_line(src: &str, offset: usize) -> bool {
    src[line_start(src, offset)..offset].trim().is_empty()
}

/// The `// ...` comment that follows `end` on its line, after spaces.
fn trailing_comment(src: &str, end: usize) -> Option<Range<usize>> {
    let rest = &src[end..line_end(src, end)];
    let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    rest[spaces..].starts_with("//").then(|| end + spaces..end + rest.len())
}

/// A statement with the `// ...` comment that follows it on its line,
/// which names it or says what it is for.
fn with_comment(src: &str, range: &Range<usize>) -> Range<usize> {
    range.start..trailing_comment(src, range.end).map_or(range.end, |c| c.end)
}

/// The indentation the source uses for one level: a tab when most
/// indented lines start with one, else the smallest indentation of two
/// spaces or more, else four spaces.
fn indent_unit(src: &str) -> String {
    let (mut tabs, mut spaced, mut smallest) = (0, 0, usize::MAX);
    for line in src.lines().filter(|l| !l.trim().is_empty()) {
        let width = line.len() - line.trim_start_matches(' ').len();
        if line.starts_with('\t') {
            tabs += 1;
        } else if width > 0 {
            spaced += 1;
            if width >= 2 {
                smallest = smallest.min(width);
            }
        }
    }
    if tabs > spaced {
        "\t".into()
    } else if (2..=8).contains(&smallest) {
        " ".repeat(smallest)
    } else {
        "    ".into()
    }
}

/// For each line of `text` (code that starts outside any string or
/// comment), whether it starts inside a string or a block comment. Such a
/// line is left as written by re-indentation: the language keeps the
/// spaces after a raw line break in a string, so they are part of its
/// value.
fn verbatim_lines(text: &str) -> Vec<bool> {
    #[derive(PartialEq)]
    enum In {
        Code,
        Str,
        Line,
        Block,
    }
    let mut state = In::Code;
    let mut lines = vec![false];
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let next = bytes.get(i + 1).copied();
        match (&state, bytes[i]) {
            (_, b'\n') => {
                if state == In::Line {
                    state = In::Code;
                }
                lines.push(matches!(state, In::Str | In::Block));
            }
            (In::Code, b'"') => state = In::Str,
            (In::Code, b'/') if next == Some(b'/') => state = In::Line,
            (In::Code, b'/') if next == Some(b'*') => {
                state = In::Block;
                i += 1;
            }
            // An escaped character, a line break included, stays in.
            (In::Str, b'\\') if next != Some(b'\n') => i += 1,
            (In::Str, b'"') => state = In::Code,
            (In::Block, b'*') if next == Some(b'/') => {
                state = In::Code;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    lines
}

/// Moves text written at indentation `from` to indentation `to`: each
/// line after the first trades its leading `from` for `to` (lines
/// indented less lose their indentation and get `to`); blank lines stay
/// blank, and lines that start inside a string or a block comment stay as
/// written. The first line is the rest of a line and keeps what it has.
fn reindent(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let verbatim = verbatim_lines(text);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            if verbatim[i] {
                out.push_str(line);
                continue;
            }
            if line.trim().is_empty() {
                if line.ends_with('\r') {
                    out.push('\r');
                }
                continue;
            }
            out.push_str(to);
            out.push_str(line.strip_prefix(from).unwrap_or_else(|| line.trim_start()));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Text indented by levels of `from` (four spaces, a tab...), indented by
/// as many levels of `to`; what is left of an indentation stays, and so
/// do lines that start inside a string or a block comment.
fn reunit(text: &str, from: &str, to: &str) -> String {
    if from == to {
        return text.to_string();
    }
    let verbatim = verbatim_lines(text);
    let lines: Vec<String> = text
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            if verbatim[i] {
                return line.to_string();
            }
            let mut rest = line;
            let mut levels = 0;
            while let Some(r) = rest.strip_prefix(from) {
                rest = r;
                levels += 1;
            }
            format!("{}{rest}", to.repeat(levels))
        })
        .collect();
    lines.join("\n")
}

fn chars(src: &str, byte: usize) -> usize {
    src[..byte].chars().count()
}

/// The bytes removed with a statement: its whole line(s) and the comment
/// after it when it stands alone on them; the spaces before it and the
/// comment after it when it ends a line others start (`a(); b(); // B`:
/// the comment names it, and goes with it as [`with_comment`] copies it);
/// else the statement and the spaces after it.
fn removal(src: &str, range: &Range<usize>) -> Range<usize> {
    let end = with_comment(src, range).end;
    let eol = line_end(src, end);
    if starts_line(src, range.start) && src[end..eol].trim().is_empty() {
        let after = src[eol..].find('\n').map_or(src.len(), |i| eol + i + 1);
        return line_start(src, range.start)..after;
    }
    if end > range.end {
        let before = &src[line_start(src, range.start)..range.start];
        return range.start - (before.len() - before.trim_end_matches([' ', '\t']).len())..end;
    }
    let rest = &src[range.end..];
    range.start..range.end + (rest.len() - rest.trim_start_matches([' ', '\t']).len())
}

/// The line break to write after the line ending at `eol` (as
/// [`line_end`] gives it): the one there, else, at the end of the source,
/// the one the source uses, so that a file written with `\r\n` keeps
/// them all.
fn newline(src: &str, eol: usize) -> &'static str {
    let crlf = if eol < src.len() { src[eol..].starts_with("\r\n") } else { src.contains("\r\n") };
    if crlf { "\r\n" } else { "\n" }
}

/// A replacement of `range` by `text` in the source.
#[derive(Clone, Debug)]
struct Splice {
    range: Range<usize>,
    text: String,
}

impl Splice {
    fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Splice { range, text: text.into() }
    }

    fn insert(at: usize, text: impl Into<String>) -> Self {
        Splice { range: at..at, text: text.into() }
    }

    /// Where `offset` of the old source lands in the new one;
    /// `after_insertion` decides for an offset where text is inserted.
    fn shift(&self, offset: usize, after_insertion: bool) -> usize {
        let grown = |o: usize| o + self.text.len() - self.range.len();
        if offset < self.range.start || (offset == self.range.start && !after_insertion) {
            offset
        } else if offset >= self.range.end {
            grown(offset)
        } else {
            // Inside replaced text: its start.
            self.range.start
        }
    }
}

/// Applies non-overlapping splices; returns the new text and where each
/// splice's text starts in it, in the order given.
fn apply(src: &str, splices: &[Splice]) -> Result<(String, Vec<usize>), String> {
    let mut order: Vec<usize> = (0..splices.len()).collect();
    order.sort_by_key(|&i| (splices[i].range.start, splices[i].range.end));
    let mut out = String::with_capacity(src.len());
    let mut starts = vec![0; splices.len()];
    let mut at = 0;
    for i in order {
        let s = &splices[i];
        if s.range.start < at {
            return Err("Ces objets se chevauchent dans le code.".into());
        }
        out.push_str(&src[at..s.range.start]);
        starts[i] = out.len();
        out.push_str(&s.text);
        at = s.range.end;
    }
    out.push_str(&src[at..]);
    Ok((out, starts))
}

/// One splice applied, keeping the statement at `selected` selected; text
/// inserted right at its start is left out of it when `start_after`. The
/// cursor goes after the new text when `cursor_after`, else at the
/// selection.
fn single(
    src: &str,
    splice: Splice,
    selected: &Range<usize>,
    start_after: bool,
    cursor_after: bool,
) -> Result<Edit, String> {
    let (text, starts) = apply(src, std::slice::from_ref(&splice))?;
    let selection = splice.shift(selected.start, start_after)..splice.shift(selected.end, true);
    let cursor_byte = if cursor_after { starts[0] + splice.text.len() } else { selection.start };
    Ok(Edit { cursor: chars(&text, cursor_byte), text, selection })
}

/// The statement a snippet of code holds, ready to insert: its first line
/// starts the text, its other lines are indented relative to it with the
/// indentation unit `unit` (the snippet's own unit is converted), and its
/// first statement is at the returned range.
fn snippet(code: &str, unit: &str) -> Result<(String, Range<usize>), String> {
    let invalid = || "Le code à insérer est invalide.".to_string();
    let start = code.len() - code.trim_start().len();
    let first = line_start(code, start);
    let indent = &code[first..start];
    let text = reindent(code[start..].trim_end(), indent, "");
    let text = reunit(&text, &indent_unit(&text), unit);
    let file = c2f_lang::parse(&text).map_err(|_| invalid())?;
    let stmt = file.stmts.iter().find(|s| matches!(s.kind, StmtKind::Instance(_))).ok_or_else(invalid)?;
    let range = stmt.span.range();
    Ok((text, range))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(18.400_000_2), "18.4");
        assert_eq!(number(3.0), "3");
        assert_eq!(number(-0.0001), "0");
        assert_eq!(number(-2.5), "-2.5");
        assert_eq!(Value::Vector(vec![1.0, -0.0, 2.25]).code(), "[1, 0, 2.25]");
        assert_eq!(Value::Str("a \"b\"\\".into()).code(), r#""a \"b\"\\""#);
    }

    #[test]
    fn the_indentation_unit_is_detected() {
        assert_eq!(indent_unit("cube();\n"), "    ");
        assert_eq!(indent_unit("union() {\n  cube();\n  if (a) {\n    sphere();\n  }\n}\n"), "  ");
        assert_eq!(indent_unit("union() {\n\tcube();\n}\n"), "\t");
        // A block comment's ` * ` lines do not count.
        assert_eq!(indent_unit("/*\n * a\n */\nunion() {\n    cube();\n}\n"), "    ");
    }

    #[test]
    fn reindenting_keeps_relative_indentation() {
        let text = "difference() {\n        cube();\n\n        sphere();\n    }";
        assert_eq!(reindent(text, "    ", ""), "difference() {\n    cube();\n\n    sphere();\n}");
        assert_eq!(reindent("a {\n    b;\n}", "", "  "), "a {\n      b;\n  }");
        assert_eq!(reunit("a {\n    b {\n        c;\n     }\n}", "    ", "\t"), "a {\n\tb {\n\t\tc;\n\t }\n}");
    }

    #[test]
    fn statements_are_located_by_their_exact_range() {
        let src = "translate([1, 0, 0]) #cube(2);\n";
        let doc = Doc::parse(src).unwrap();
        assert_eq!(doc.locate(&(0..30)).unwrap().len(), 1);
        let inner = doc.locate(&(21..30)).unwrap();
        assert_eq!(inner.len(), 2);
        assert_eq!(placed(&inner).len(), 1);
        assert!(doc.locate(&(22..30)).is_err());
        assert!(Doc::parse("cube(").is_err());
    }

    #[test]
    fn removal_takes_the_line_and_its_comment() {
        let src = "a();\n    b(); // name\nc();\n";
        let b = src.find("b()").unwrap();
        assert_eq!(&src[removal(src, &(b..b + 4))], "    b(); // name\n");
        let src = "a(); b();  c();";
        let b = src.find("b()").unwrap();
        assert_eq!(&src[removal(src, &(b..b + 4))], "b();  ");
        // Last on a line others start, it takes the comment naming it.
        let src = "a(); b(); // B\nc();\n";
        let b = src.find("b()").unwrap();
        assert_eq!(&src[removal(src, &(b..b + 4))], " b(); // B");
    }

    #[test]
    fn line_breaks_follow_the_source() {
        assert_eq!(newline("a();\r\nb();", 4), "\r\n");
        assert_eq!(newline("a();\nb();", 4), "\n");
        assert_eq!(newline("a();\r\nb();", 10), "\r\n");
        assert_eq!(newline("a();", 4), "\n");
    }

    /// A raw line break in a string keeps the spaces after it in the
    /// value: re-indenting the line would change the text drawn.
    #[test]
    fn lines_inside_strings_and_comments_keep_their_indentation() {
        let text = "union() {\n    text(\"a\n    b\");\n    /* x\n       y */\n    cube(\"\\\"\");\n}";
        assert_eq!(
            reindent(text, "", "  "),
            "union() {\n      text(\"a\n    b\");\n      /* x\n       y */\n      cube(\"\\\"\");\n  }"
        );
        assert_eq!(
            reunit(text, "    ", "\t"),
            "union() {\n\ttext(\"a\n    b\");\n\t/* x\n       y */\n\tcube(\"\\\"\");\n}"
        );
        // A `//` comment ends with its line; a backslash before a break
        // keeps the string open.
        assert_eq!(verbatim_lines("a(); // \"\nb(\"\\\nc\");\nd();"), [false, false, true, false]);
        let (code, _) = snippet("    text(\"a\n    b\");", "  ").unwrap();
        assert_eq!(code, "text(\"a\n    b\");");
    }

    /// Every statement of the tree.
    fn all(nodes: &[TreeNode], out: &mut Vec<Range<usize>>) {
        for node in nodes {
            out.push(node.range.clone());
            all(&node.children, out);
        }
    }

    /// Each edit, on each object of a program written in many styles,
    /// either is refused or leaves a source that parses, with its comments
    /// and the selection on a statement of it.
    #[test]
    fn every_edit_leaves_a_program_that_parses() {
        let src = "\
/* A box */
w = 20; // [10:50]
module peg(h = 4) { cylinder(h, r = 1); }
difference() { // Coque
  #cube([w, w, 10]);
  translate([w / 2, w / 2, 2])
    cylinder(h = 20, r = 3); // Trou
  for (i = [0 : 3]) rotate([0, 0, i * 90]) translate([4, 0, -1]) cube(1);
}
if (w > 10) peg(); else { sphere(2); }
color(\"red\") scale(2) { cube(1); sphere(1); }
translate([30, 0, 0]) linear_extrude(2) text(\"A\");
";
        let mut ranges = Vec::new();
        all(&tree(src).unwrap(), &mut ranges);
        assert!(ranges.len() > 15);
        let check = |what: &str, result: Result<Edit, String>| {
            if let Ok(edit) = result {
                if let Err(err) = c2f_lang::parse(&edit.text) {
                    panic!("{what}: {}\n{}", err.message, edit.text);
                }
                assert!(edit.text.contains("/* A box */") && edit.text.contains("// [10:50]"), "{what}");
                assert!(edit.cursor <= edit.text.chars().count(), "{what}");
                if !edit.selection.is_empty() {
                    let doc = Doc::parse(&edit.text).unwrap();
                    assert!(doc.locate(&edit.selection).is_ok(), "{what}: selection\n{}", edit.text);
                }
            }
        };
        for r in &ranges {
            let r = r.clone();
            let what = &src[r.clone()];
            check(what, set_transform(src, r.clone(), Transform::Translate, [1.0, 2.0, 3.0]));
            check(what, scale_by(src, r.clone(), [2.0, 2.0, 2.0]));
            check(what, delete(src, r.clone()));
            check(what, duplicate(src, r.clone()));
            check(what, make_hole(src, r.clone(), "sphere(1);"));
            check(what, wrap_boolean(src, std::slice::from_ref(&r), Boolean::Union));
            check(what, rename(src, r.clone(), "Nom"));
            check(what, set_modifier(src, r.clone(), Some(Modifier::Disable), true));
            check(what, set_modifier(src, r.clone(), None, false));
            check(what, insert_shape(src, &Place::InBlock(r.clone()), "cube(2);"));
            check(what, add_arg(src, r.clone(), "convexity", &Value::Number(2.0)));
            if let Ok(text) = copy_text(src, r.clone()) {
                check(what, paste(src, &Place::End, &text));
            }
            for t in &ranges {
                check(what, move_into(src, r.clone(), Target::Before(t.clone())));
                check(what, move_into(src, r.clone(), Target::After(t.clone())));
                check(what, move_into(src, r.clone(), Target::IntoBlock(t.clone())));
            }
        }
    }
}
