//! Syntax highlighting for the editor (the .scad language, a circuit's
//! TOML, and a firmware's C++): a small lexer that splits the text into
//! colored runs, and the [`LayoutJob`] egui lays out.

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
    "pause",
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

/// The language of the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Syntax {
    #[default]
    Scad,
    /// A circuit file (TOML): `#` starts a comment.
    Circuit,
    /// A firmware's C++: its keywords and types, Arduino's functions, and
    /// its constants in capitals (a pin's, `HIGH`) as special.
    Cpp,
}

const CPP_KEYWORDS: &[&str] = &[
    "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "goto", "const",
    "constexpr", "static", "volatile", "extern", "inline", "struct", "class", "enum", "union", "typedef", "using",
    "namespace", "template", "typename", "public", "private", "protected", "virtual", "override", "new", "delete",
    "this", "true", "false", "nullptr", "NULL", "sizeof", "auto",
];

const CPP_TYPES: &[&str] = &[
    "void", "bool", "char", "short", "int", "long", "float", "double", "unsigned", "signed", "size_t", "int8_t",
    "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "byte", "word", "String",
];

const ARDUINO_FUNCTIONS: &[&str] = &[
    "setup", "loop", "pinMode", "digitalWrite", "digitalRead", "analogWrite", "analogRead", "analogReadResolution",
    "ledcAttach", "ledcWrite", "touchRead", "delay", "delayMicroseconds", "millis", "micros", "map", "constrain",
    "min", "max", "abs", "random", "attachInterrupt", "Serial", "Wire", "SPI", "tone", "noTone",
];

/// Splits `src` into runs of one kind. The runs cover the whole text, in
/// order, and adjacent runs of the same kind are merged.
#[cfg(test)]
pub fn tokenize(src: &str) -> Vec<(Range<usize>, Kind)> {
    tokenize_as(src, Syntax::Scad)
}

/// As [`tokenize`], for a text in `syntax`.
pub fn tokenize_as(src: &str, syntax: Syntax) -> Vec<(Range<usize>, Kind)> {
    let hash_comments = syntax == Syntax::Circuit;
    let cpp = syntax == Syntax::Cpp;
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
        // A C++ directive (`#include`) starts its line.
        let line_start = || {
            let before = src[..i].trim_end_matches([' ', '\t']);
            before.is_empty() || before.ends_with('\n')
        };
        let kind = if cpp && b == b'#' && line_start() {
            i += 1;
            while i < len && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            Kind::Keyword
        } else if cpp && b == b'\'' {
            i += 1;
            while i < len && bytes[i] != b'\'' && bytes[i] != b'\n' {
                i = if bytes[i] == b'\\' { skip_char(src, (i + 1).min(len - 1)) } else { skip_char(src, i) };
            }
            i = (i + 1).min(len);
            Kind::Str
        } else if (b == b'/' && next == b'/') || (hash_comments && b == b'#') {
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
        } else if (b == b'$' && !cpp) || b == b'_' || b.is_ascii_alphabetic() {
            i += 1;
            while i < len && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let word = &src[start..i];
            if cpp {
                if CPP_KEYWORDS.contains(&word) {
                    Kind::Keyword
                } else if CPP_TYPES.contains(&word) {
                    Kind::Module
                } else if ARDUINO_FUNCTIONS.contains(&word) {
                    Kind::Function
                } else if word.len() > 1 && word.bytes().all(|c| c == b'_' || c.is_ascii_uppercase() || c.is_ascii_digit()) {
                    Kind::Special
                } else {
                    Kind::Plain
                }
            } else if word.starts_with('$') {
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
        expect_path = kind == Kind::Keyword && (word == "include" || word == "use" || word == "#include");
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
    let p = theme::palette();
    match kind {
        Kind::Plain => p.code_text,
        Kind::Keyword => p.code_keyword,
        Kind::Module => p.code_module,
        Kind::Function => p.code_function,
        Kind::Number => p.code_number,
        Kind::Str => p.code_string,
        Kind::Comment => p.code_comment,
        Kind::Special => p.code_special,
        Kind::Operator => p.code_operator,
        Kind::Definition => p.code_definition,
    }
}

/// Remembers the last highlighted text, so typing does not re-lex the
/// whole buffer on frames where nothing changed (nor the theme).
#[derive(Default)]
pub struct Highlighter {
    source: String,
    font: Option<FontId>,
    job: LayoutJob,
    /// The language of the text.
    pub syntax: Syntax,
    highlighted_as: Syntax,
    /// Whether the colors are the dark theme's.
    dark: Option<bool>,
}

impl Highlighter {
    pub fn layout_job(&mut self, src: &str, font: &FontId, wrap_width: f32) -> LayoutJob {
        let dark = theme::palette().dark;
        if self.source != src
            || self.font.as_ref() != Some(font)
            || self.highlighted_as != self.syntax
            || self.dark != Some(dark)
        {
            self.dark = Some(dark);
            self.source.clear();
            self.source.push_str(src);
            self.font = Some(font.clone());
            self.highlighted_as = self.syntax;
            self.job = build_job(src, font, self.syntax);
        }
        let mut job = self.job.clone();
        job.wrap.max_width = wrap_width;
        job
    }
}

fn build_job(src: &str, font: &FontId, syntax: Syntax) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, kind) in tokenize_as(src, syntax) {
        let format = TextFormat {
            font_id: font.clone(),
            color: color(kind),
            italics: kind == Kind::Comment,
            ..Default::default()
        };
        job.append(&src[range], 0.0, format);
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat::simple(font.clone(), theme::palette().code_text));
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
    fn circuit_comments_start_with_a_hash() {
        let src = "# 5 V\ncolor = \"#ff0000\" # red\n";
        let comments: Vec<&str> = tokenize_as(src, Syntax::Circuit)
            .into_iter()
            .filter(|(_, kind)| *kind == Kind::Comment)
            .map(|(r, _)| src[r].trim())
            .collect();
        assert_eq!(comments, ["# 5 V", "# red"]);
        assert!(tokenize(src).iter().all(|(r, kind)| *kind != Kind::Comment || src[r.clone()].starts_with("//")));
    }

    #[test]
    fn a_firmware_reads_as_cpp() {
        let src = "#include <Arduino.h>\n#include \"pins.h\"\nvoid setup() { pinMode(AMP_BCLK, OUTPUT); char c = '\\n'; } // é\n";
        let got: Vec<(&str, Kind)> = tokenize_as(src, Syntax::Cpp)
            .into_iter()
            .filter(|(r, _)| !src[r.clone()].trim().is_empty())
            .map(|(r, k)| (src[r].trim(), k))
            .collect();
        assert_eq!(got[..2], [("#include", Kind::Keyword), ("<Arduino.h>", Kind::Str)]);
        assert!(got.contains(&("\"pins.h\"", Kind::Str)));
        assert!(got.contains(&("void", Kind::Module)));
        assert!(got.contains(&("pinMode", Kind::Function)));
        assert!(got.contains(&("AMP_BCLK", Kind::Special)));
        assert!(got.contains(&("'\\n'", Kind::Str)));
        assert_eq!(got.last(), Some(&("// é", Kind::Comment)));
        let mut at = 0;
        for (range, _) in tokenize_as(src, Syntax::Cpp) {
            assert_eq!(range.start, at);
            at = range.end;
        }
        assert_eq!(at, src.len());
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
