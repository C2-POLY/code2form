use std::fmt::Write;

use crate::span::{LineIndex, Span};

/// An error with the source range it points at.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Diagnostic { message: message.into(), span }
    }

    /// Renders the error with its source line and a caret underline:
    ///
    /// ```text
    /// error: expected `;`, found `}`
    ///  --> part.scad:3:12
    ///   |
    /// 3 | cube(10) }
    ///   |          ^
    /// ```
    pub fn render(&self, src: &str, file_name: &str) -> String {
        let index = LineIndex::new(src);
        let offset = (self.span.start as usize).min(src.len());
        let (line, col) = index.line_col(src, offset);
        let text = index.line_text(src, line);
        // Underline at most to the end of the first line of the span.
        let end = (self.span.end as usize).clamp(offset, index.line_start(line) + text.len());
        let width = src[offset..end].chars().count().max(1);
        let gutter = (line + 1).to_string().len();

        let mut out = String::new();
        let _ = writeln!(out, "error: {}", self.message);
        let _ = writeln!(out, "{:gutter$}--> {file_name}:{}:{}", "", line + 1, col + 1);
        let _ = writeln!(out, "{:gutter$} |", "");
        let _ = writeln!(out, "{} | {text}", line + 1);
        let _ = writeln!(out, "{:gutter$} | {:col$}{}", "", "", "^".repeat(width));
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_spans_after_multibyte_characters() {
        let src = "x = \"éé\" \"\né\";";
        let err = crate::parse(src).unwrap_err();
        let rendered = err.render(src, "t.scad");
        assert!(rendered.contains("t.scad:1:10"), "{rendered}");
        assert!(rendered.ends_with("  |          ^\n"), "{rendered}");
    }
}
