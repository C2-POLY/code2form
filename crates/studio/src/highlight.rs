//! Syntax highlighting for the editor (the .scad language): a small lexer that splits
//! the text into colored runs, and the [`LayoutJob`] egui lays out.

use std::ops::Range;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Keyword,
    /// A built-in module such as `cube` or `translate`.
    Module,
    /// A built-in function or constant such as `sin` or `PI`.
    Function,
    Number,
    Str,
    Comment,
    /// A `$fn`-style special variable.
    Special,
    Operator,
    /// The name right after `module` or `function`.
    Definition,
}

const KEYWORDS: &[&str] = &[
    "module", "function", "if", "else", "for", "let", "each", "assert", "echo", "include", "use", "true", "false",
    "undef",
];

const MODULES: &[&str] = &[
    "cube",
    "sphere",
    "cylinder",
    "polyhedron",
    "square",
    "circle",
    "polygon",
    "text",
    "translate",
    "rotate",
    "scale",
    "mirror",
    "multmatrix",
    "color",
    "offset",
    "hull",
    "minkowski",
    "union",
    "difference",
    "intersection",
    "linear_extrude",
    "rotate_extrude",
    "projection",
    "children",
    "render",
    "import",
    "surface",
    "resize",
    "group",
    "roof",
    "intersection_for",
];

const FUNCTIONS: &[&str] = &[
    "abs",
    "sign",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "floor",
    "round",
    "ceil",
    "ln",
    "log",
    "pow",
    "sqrt",
    "exp",
    "len",
    "min",
    "max",
    "norm",
    "cross",
    "concat",
    "lookup",
    "str",
    "chr",
    "ord",
    "search",
    "version",
    "version_num",
    "rands",
    "is_undef",
    "is_num",
    "is_bool",
    "is_string",
    "is_list",
    "is_function",
    "parent_module",
    "dxf_cross",
    "dxf_dim",
    "PI",
];

/// Splits `src` into runs of one kind. The runs cover the whole text, in
/// order, and adjacent runs of the same kind are merged.
pub fn tokenize(src: &str) -> Vec<(Range<usize>, Kind)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out: Vec<(Range<usize>, Kind)> = Vec::new();
    let mut i = 0;
    // `module`/`function` was just seen: the next word is a definition.
    let mut expect_definition = false;
    // `include`/`use` was just seen: a `<path>` follows.
    let mut expect_path = false;

    while i < len {
        let start = i;
        let b = bytes[i];
        let next = bytes.get(i + 1).copied().unwrap_or(0);
        let kind = if b == b'/' && next == b'/' {
            i = src[i..].find('\n').map_or(len, |n| i + n);
            Kind::Comment
        } else if b == b'/' && next == b'*' {
            i = src[i + 2..].find("*/").map_or(len, |n| i + 2 + n + 2);
            Kind::Comment
        } else if b == b'"' {
            i += 1;
            while i < len {
                match bytes[i] {
                    b'\\' => i = skip_char(src, i + 1),
                    b'"' => {
                        i += 1;
                        break;
                    }
                    _ => i = skip_char(src, i),
                }
            }
            Kind::Str
        } else if expect_path && b == b'<' {
            i = src[i..].find(['>', '\n']).map_or(len, |n| i + n);
            if bytes.get(i) == Some(&b'>') {
                i += 1;
            }
            Kind::Str
        } else if b.is_ascii_digit() || (b == b'.' && next.is_ascii_digit()) {
            i = number_end(bytes, i);
            Kind::Number
        } else if b == b'$' || b == b'_' || b.is_ascii_alphabetic() {
            i += 1;
            while i < len && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let word = &src[start..i];
            if word.starts_with('$') {
                Kind::Special
            } else if expect_definition {
                Kind::Definition
            } else if KEYWORDS.contains(&word) {
                Kind::Keyword
            } else if MODULES.contains(&word) {
                Kind::Module
            } else if FUNCTIONS.contains(&word) {
                Kind::Function
            } else {
                Kind::Plain
            }
        } else if b.is_ascii_whitespace() {
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            push(&mut out, start..i, Kind::Plain);
            continue;
        } else if b.is_ascii_punctuation() {
            i += 1;
            Kind::Operator
        } else {
            i = skip_char(src, i);
            Kind::Plain
        };

        let word = &src[start..i];
        expect_definition = kind == Kind::Keyword && (word == "module" || word == "function");
        expect_path = kind == Kind::Keyword && (word == "include" || word == "use");
        push(&mut out, start..i, kind);
    }
    out
}

/// The index just past the character at byte `i`.
fn skip_char(src: &str, i: usize) -> usize {
    src[i..].chars().next().map_or(src.len(), |c| i + c.len_utf8())
}

fn number_end(bytes: &[u8], mut i: usize) -> usize {
    let digits = |i: &mut usize| {
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
    };
    digits(&mut i);
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        digits(&mut i);
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            i = j;
            digits(&mut i);
        }
    }
    i
}

fn push(out: &mut Vec<(Range<usize>, Kind)>, range: Range<usize>, kind: Kind) {
    if let Some((last, last_kind)) = out.last_mut() {
        if *last_kind == kind && last.end == range.start {
            last.end = range.end;
            return;
        }
    }
    out.push((range, kind));
}

pub fn color(kind: Kind) -> Color32 {
    match kind {
        Kind::Plain => theme::CODE_TEXT,
        Kind::Keyword => theme::CODE_KEYWORD,
        Kind::Module => theme::CODE_MODULE,
        Kind::Function => theme::CODE_FUNCTION,
        Kind::Number => theme::CODE_NUMBER,
        Kind::Str => theme::CODE_STRING,
        Kind::Comment => theme::CODE_COMMENT,
        Kind::Special => theme::CODE_SPECIAL,
        Kind::Operator => theme::CODE_OPERATOR,
        Kind::Definition => theme::CODE_DEFINITION,
    }
}

/// Remembers the last highlighted text, so typing does not re-lex the
/// whole buffer on frames where nothing changed.
#[derive(Default)]
pub struct Highlighter {
    source: String,
    font: Option<FontId>,
    job: LayoutJob,
}

impl Highlighter {
    pub fn layout_job(&mut self, src: &str, font: &FontId, wrap_width: f32) -> LayoutJob {
        if self.source != src || self.font.as_ref() != Some(font) {
            self.source.clear();
            self.source.push_str(src);
            self.font = Some(font.clone());
            self.job = build_job(src, font);
        }
        let mut job = self.job.clone();
        job.wrap.max_width = wrap_width;
        job
    }
}

fn build_job(src: &str, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, kind) in tokenize(src) {
        let format = TextFormat {
            font_id: font.clone(),
            color: color(kind),
            italics: kind == Kind::Comment,
            ..Default::default()
        };
        job.append(&src[range], 0.0, format);
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat::simple(font.clone(), theme::CODE_TEXT));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(&str, Kind)> {
        tokenize(src)
            .into_iter()
            .filter(|(r, _)| !src[r.clone()].trim().is_empty())
            .map(|(r, k)| (src[r].trim(), k))
            .collect()
    }

    #[test]
    fn covers_the_whole_text() {
        let src = "module m(r=1) { cube([1, 2.5e3, .5]); } // é\n/* a\nb */ x = \"q\\\"é\";";
        let runs = tokenize(src);
        let mut at = 0;
        for (range, _) in &runs {
            assert_eq!(range.start, at);
            at = range.end;
        }
        assert_eq!(at, src.len());
    }

    #[test]
    fn classifies_tokens() {
        let got = kinds("module box($fn=8) cube(2e-3); include <lib/a.scad> sin(PI)");
        assert_eq!(
            got,
            vec![
                ("module", Kind::Keyword),
                ("box", Kind::Definition),
                ("(", Kind::Operator),
                ("$fn", Kind::Special),
                ("=", Kind::Operator),
                ("8", Kind::Number),
                (")", Kind::Operator),
                ("cube", Kind::Module),
                ("(", Kind::Operator),
                ("2e-3", Kind::Number),
                (");", Kind::Operator),
                ("include", Kind::Keyword),
                ("<lib/a.scad>", Kind::Str),
                ("sin", Kind::Function),
                ("(", Kind::Operator),
                ("PI", Kind::Function),
                (")", Kind::Operator),
            ]
        );
    }

    #[test]
    fn unterminated_tokens_run_to_the_end() {
        assert_eq!(kinds("x = \"abc"), vec![("x", Kind::Plain), ("=", Kind::Operator), ("\"abc", Kind::Str)]);
        assert_eq!(kinds("/* open"), vec![("/* open", Kind::Comment)]);
        assert_eq!(kinds("\"\\"), vec![("\"\\", Kind::Str)]);
    }
}
