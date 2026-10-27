//! The agent's replies, their Markdown drawn: a small renderer for what a
//! reply uses (headings, paragraphs, nested lists, quotes, rules, fenced
//! code, GitHub's pipe tables, and inline bold, italics, code and links).
//!
//! It reads the reply anew at each frame, as it streams in: a block not
//! finished yet reads as what it is so far, and a table shows as text
//! until its separator row has come.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontId, Sense, Stroke, TextFormat, Vec2, text::LayoutJob};

use crate::agent_panel::code_block;
use crate::theme;

/// A block of the reply.
#[derive(Debug, PartialEq)]
pub enum Block {
    Paragraph(String),
    /// A heading, its level from 1 to 6.
    Heading(usize, String),
    /// A list item: how deep it is nested (0 at the top), its marker (`•`
    /// or `1.`) and its text.
    Item(usize, String, String),
    /// A quote's text, its paragraphs on lines of their own.
    Quote(String),
    Rule,
    Code(String),
    Table(Table),
}

/// A pipe table: its header, then its rows, each as many cells as the
/// header.
#[derive(Debug, PartialEq)]
pub struct Table {
    pub align: Vec<Align>,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// How a table's column is aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// The blocks of `text`. A fenced code block runs to the end while its
/// fence is not closed yet.
pub fn blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    // Whether the next plain line carries on the last block.
    let mut joinable = false;
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        let trimmed = line.trim_start();
        at += 1;
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence = &trimmed[..3];
            let mut code = Vec::new();
            while at < lines.len() && !lines[at].trim_start().starts_with(fence) {
                code.push(lines[at]);
                at += 1;
            }
            at += 1;
            blocks.push(Block::Code(code.join("\n")));
            joinable = false;
        } else if trimmed.is_empty() {
            joinable = false;
        } else if let Some(table) = lines.get(at).and_then(|next| table_start(trimmed, next)) {
            let mut table = table;
            at += 1;
            while let Some(row) = lines.get(at).filter(|row| row.contains('|') && !row.trim().is_empty()) {
                table.rows.push(fit(cells(row), table.header.len()));
                at += 1;
            }
            blocks.push(Block::Table(table));
            joinable = false;
        } else if is_rule(trimmed) {
            blocks.push(Block::Rule);
            joinable = false;
        } else if let Some((level, heading)) = heading(trimmed) {
            blocks.push(Block::Heading(level, heading.to_string()));
            joinable = false;
        } else if let Some(quoted) = trimmed.strip_prefix('>') {
            let quoted = quoted.strip_prefix(' ').unwrap_or(quoted).trim_end();
            match blocks.last_mut() {
                Some(Block::Quote(text)) if joinable => {
                    if quoted.is_empty() {
                        text.push('\n');
                    } else {
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push(' ');
                        }
                        text.push_str(quoted);
                    }
                }
                _ => blocks.push(Block::Quote(quoted.to_string())),
            }
            joinable = true;
        } else if let Some((marker, item)) = list_item(trimmed) {
            let indent = line.len() - trimmed.len() + line[..line.len() - trimmed.len()].matches('\t').count() * 3;
            blocks.push(Block::Item((indent / 2).min(4), marker, item.trim_end().to_string()));
            joinable = true;
        } else {
            match blocks.last_mut() {
                Some(Block::Paragraph(text) | Block::Item(_, _, text)) if joinable => {
                    text.push(' ');
                    text.push_str(trimmed.trim_end());
                }
                _ => blocks.push(Block::Paragraph(trimmed.trim_end().to_string())),
            }
            joinable = true;
        }
    }
    blocks
}

/// A heading's level and text: `#` to `######`, then a space.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    let rest = &line[level..];
    ((1..=6).contains(&level) && (rest.is_empty() || rest.starts_with(' ')))
        .then(|| (level, rest.trim().trim_end_matches('#').trim_end()))
}

/// Whether `line` is a horizontal rule: three or more `-`, `*` or `_`,
/// spaces between them allowed.
fn is_rule(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && ['-', '*', '_'].iter().any(|&mark| marks.iter().all(|&c| c == mark))
}

/// A list item's marker and text: `- a`, `* a`, `+ a`, `1. a` or `1) a`.
fn list_item(line: &str) -> Option<(String, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(item) = line.strip_prefix(bullet) {
            return Some(("•".into(), item));
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = &line[digits..];
    let item = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    (digits > 0).then(|| (format!("{}.", &line[..digits]), item))
}

/// The table that `header` and its `separator` start, without its rows
/// yet; none unless `separator` is a separator row with as many cells as
/// the header.
fn table_start(header: &str, separator: &str) -> Option<Table> {
    if !header.contains('|') || !separator.contains('|') && !separator.contains('-') {
        return None;
    }
    let header = cells(header);
    let align: Option<Vec<Align>> = cells(separator)
        .iter()
        .map(|cell| {
            let cell = cell.trim();
            let dashes = cell.trim_start_matches(':').trim_end_matches(':');
            if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
                return None;
            }
            Some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect();
    let align = align.filter(|align| align.len() == header.len())?;
    Some(Table { align, header, rows: Vec::new() })
}

/// The cells of a table's row, trimmed: split on `|`, but not on `\|` nor
/// in inline code, the pipes at its ends left out.
fn cells(row: &str) -> Vec<String> {
    let row = row.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let row = if row.ends_with('|') && !row.ends_with("\\|") { &row[..row.len() - 1] } else { row };
    let mut cells = vec![String::new()];
    let mut code = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        let cell = cells.last_mut().expect("a cell");
        match c {
            '\\' if chars.peek() == Some(&'|') => cell.push(chars.next().expect("a pipe")),
            '`' => {
                code = !code;
                cell.push(c);
            }
            '|' if !code => cells.push(String::new()),
            _ => cell.push(c),
        }
    }
    cells.iter().map(|cell| cell.trim().to_string()).collect()
}

/// `cells` cut or filled with empty ones to `count`.
fn fit(mut cells: Vec<String>, count: usize) -> Vec<String> {
    cells.resize(count, String::new());
    cells
}

/// A run of inline Markdown.
#[derive(Debug, PartialEq)]
pub enum Span<'a> {
    Plain(&'a str),
    Bold(&'a str),
    Italic(&'a str),
    Code(&'a str),
    /// A link the user may open: http or https only.
    Link(&'a str, &'a str),
}

/// The runs of `text`: `**bold**` and `__bold__`, `*italics*` and
/// `_italics_`, `` `code` ``, `[links](https://…)` and the plain text
/// between. A marker never closed is plain text, as is a `_` within a
/// word; a link to anything but http(s) shows its text alone.
pub fn spans(text: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    // Where the plain text not yet pushed starts.
    let mut plain = 0;
    let mut at = 0;
    let bytes = text.as_bytes();
    while at < bytes.len() {
        let found = match bytes[at] {
            b'`' => text[at + 1..].find('`').filter(|&end| end > 0).map(|end| (Span::Code(&text[at + 1..at + 1 + end]), at + end + 2)),
            b'[' => link(text, at),
            b'*' | b'_' => emphasis(text, at),
            _ => None,
        };
        match found {
            Some((span, end)) => {
                if at > plain {
                    spans.push(Span::Plain(&text[plain..at]));
                }
                spans.push(span);
                at = end;
                plain = end;
            }
            None => {
                // A run of markers that opens nothing is passed whole.
                let mark = bytes[at];
                at += 1;
                while matches!(mark, b'*' | b'_') && bytes.get(at) == Some(&mark) {
                    at += 1;
                }
                while !text.is_char_boundary(at) {
                    at += 1;
                }
            }
        }
    }
    if plain < text.len() {
        spans.push(Span::Plain(&text[plain..]));
    }
    spans
}

/// The link that starts at `at` (a `[`), and where it ends.
fn link(text: &str, at: usize) -> Option<(Span<'_>, usize)> {
    let close = at + 1 + text[at + 1..].find("](")?;
    let label = &text[at + 1..close];
    let end = close + 2 + text[close + 2..].find(')')?;
    let url = text[close + 2..end].trim();
    if label.is_empty() || label.contains('[') {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let web = (lower.starts_with("https://") || lower.starts_with("http://")) && !url.contains(char::is_whitespace);
    Some((if web { Span::Link(label, url) } else { Span::Plain(label) }, end + 1))
}

/// The bold or italic run that starts at `at` (a `*` or `_`), and where
/// it ends: its marker follows no letter (for `_`) and comes before one
/// that is not a space, and its closing one follows one that is not.
fn emphasis(text: &str, at: usize) -> Option<(Span<'_>, usize)> {
    let bytes = text.as_bytes();
    let mark = bytes[at];
    let run = bytes[at..].iter().take_while(|&&b| b == mark).count();
    if run > 2 || (mark == b'_' && text[..at].chars().next_back().is_some_and(char::is_alphanumeric)) {
        return None;
    }
    let open = at + run;
    if bytes.get(open).is_none_or(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let marker = &text[at..open];
    let mut from = open;
    while let Some(found) = text[from..].find(marker) {
        let close = from + found;
        let after = close + run;
        let doubled = bytes.get(after) == Some(&mark) || (run == 1 && bytes[close - 1] == mark);
        let spaced = bytes[close - 1].is_ascii_whitespace();
        let inside_word = mark == b'_' && text[after..].chars().next().is_some_and(char::is_alphanumeric);
        if close > open && !doubled && !spaced && !inside_word {
            let inner = &text[open..close];
            return Some((if run == 2 { Span::Bold(inner) } else { Span::Italic(inner) }, after));
        }
        from = close + 1;
        while !text.is_char_boundary(from) {
            from += 1;
        }
    }
    None
}

/// `text` laid out in `width`, its inline Markdown styled, and the links
/// in it, as the ranges of characters they cover.
fn inline(ui: &egui::Ui, text: &str, size: f32, color: Color32, width: f32) -> (Arc<egui::Galley>, Vec<(std::ops::Range<usize>, String)>) {
    let palette = theme::palette();
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut links = Vec::new();
    let mut chars = 0;
    let base = TextFormat { font_id: FontId::proportional(size), color, ..Default::default() };
    for span in spans(text) {
        let (text, format) = match span {
            Span::Plain(text) => (text, base.clone()),
            Span::Bold(text) => (text, TextFormat { color: palette.text_strong, ..base.clone() }),
            Span::Italic(text) => (text, TextFormat { italics: true, ..base.clone() }),
            Span::Code(text) => (
                text,
                TextFormat { font_id: FontId::monospace(size - 1.0), color: palette.code_text, background: palette.raised, ..Default::default() },
            ),
            Span::Link(label, url) => {
                let count = label.chars().count();
                links.push((chars..chars + count, url.to_string()));
                (label, TextFormat { color: palette.accent_strong, underline: Stroke::new(1.0, palette.accent_strong), ..base.clone() })
            }
        };
        chars += text.chars().count();
        job.append(text, 0.0, format);
    }
    (ui.painter().layout_job(job), links)
}

/// A paragraph of inline Markdown, its links opening in the browser.
fn text(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    let width = ui.available_width();
    let (galley, links) = inline(ui, text, size, color, width);
    if links.is_empty() {
        ui.label(galley);
        return;
    }
    let response = ui.add(egui::Label::new(galley.clone()).sense(Sense::click()));
    let Some(pointer) = response.hover_pos() else { return };
    let index = galley.cursor_from_pos(pointer - response.rect.min).index;
    let Some((_, url)) = links.iter().find(|(range, _)| range.contains(&index)) else { return };
    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    response.on_hover_text(url);
    if clicked {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// The reply, its Markdown drawn.
pub fn markdown(ui: &mut egui::Ui, reply: &str) {
    let palette = theme::palette();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        for (index, block) in blocks(reply).into_iter().enumerate() {
            match block {
                Block::Paragraph(paragraph) => text(ui, &paragraph, 13.5, palette.text),
                Block::Heading(level, heading) => {
                    let size = match level {
                        1 => 17.0,
                        2 => 15.5,
                        _ => 14.0,
                    };
                    ui.add_space(2.0);
                    text(ui, &heading, size, palette.text_strong);
                }
                Block::Item(depth, marker, item) => {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_space(depth as f32 * 16.0);
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 16.0), Sense::hover());
                        if marker == "•" {
                            // A dot, hollow when nested: a font may lack the glyphs.
                            let center = egui::pos2(rect.right() - 5.0, rect.center().y + 1.0);
                            if depth == 0 {
                                ui.painter().circle_filled(center, 2.5, palette.text_dim);
                            } else {
                                ui.painter().circle_stroke(center, 2.5, Stroke::new(1.0, palette.text_dim));
                            }
                        } else {
                            ui.painter().text(rect.right_top(), egui::Align2::RIGHT_TOP, marker, FontId::proportional(13.5), palette.text_dim);
                        }
                        ui.vertical(|ui| text(ui, &item, 13.5, palette.text));
                    });
                }
                Block::Quote(quote) => {
                    ui.horizontal_top(|ui| {
                        let height = ui.text_style_height(&egui::TextStyle::Body);
                        let (bar, _) = ui.allocate_exact_size(Vec2::new(3.0, height), Sense::hover());
                        ui.add_space(6.0);
                        let top = ui.cursor().top();
                        let inner = ui.vertical(|ui| {
                            for paragraph in quote.split('\n') {
                                text(ui, paragraph, 13.5, palette.text_dim);
                            }
                        });
                        let bar = egui::Rect::from_min_max(egui::pos2(bar.left(), top), egui::pos2(bar.right(), inner.response.rect.bottom()));
                        ui.painter().rect_filled(bar, CornerRadius::same(1), palette.border_strong);
                    });
                }
                Block::Rule => {
                    ui.add_space(2.0);
                    let width = ui.available_width();
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
                    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, palette.border));
                    ui.add_space(2.0);
                }
                Block::Code(code) => code_block(ui, &code),
                Block::Table(table) => table_view(ui, &table, index),
            }
        }
    });
}

/// A table, its header strong over a band of its own, its rows striped
/// and ruled, scrolling sideways when wider than the card.
fn table_view(ui: &mut egui::Ui, table: &Table, index: usize) {
    const PAD: Vec2 = Vec2::new(8.0, 4.0);
    let palette = theme::palette();
    let rows: Vec<Vec<Arc<egui::Galley>>> = std::iter::once((&table.header, palette.text_strong))
        .chain(table.rows.iter().map(|row| (row, palette.text)))
        .map(|(row, color)| row.iter().map(|cell| inline(ui, cell, 13.0, color, f32::INFINITY).0).collect())
        .collect();
    let widths: Vec<f32> = (0..table.header.len())
        .map(|column| rows.iter().map(|row| row[column].size().x).fold(0.0, f32::max) + 2.0 * PAD.x)
        .collect();
    let heights: Vec<f32> = rows.iter().map(|row| row.iter().map(|g| g.size().y).fold(0.0, f32::max) + 2.0 * PAD.y).collect();
    let size = Vec2::new(widths.iter().sum(), heights.iter().sum());
    egui::ScrollArea::horizontal().id_salt(("agent-table", index)).show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect.expand(1.0));
        let line = Stroke::new(1.0, palette.border);
        let mut y = rect.top();
        for (row, (galleys, height)) in rows.iter().zip(&heights).enumerate() {
            let band = egui::Rect::from_min_size(egui::pos2(rect.left(), y), Vec2::new(size.x, *height));
            let fill = match row {
                0 => Some(palette.raised),
                row if row % 2 == 0 => Some(palette.faint_bg),
                _ => None,
            };
            if let Some(fill) = fill {
                painter.rect_filled(band, CornerRadius::ZERO, fill);
            }
            let mut x = rect.left();
            for ((galley, width), align) in galleys.iter().zip(&widths).zip(&table.align) {
                let free = width - 2.0 * PAD.x - galley.size().x;
                let shift = match align {
                    Align::Left => 0.0,
                    Align::Center => free / 2.0,
                    Align::Right => free,
                };
                painter.galley(egui::pos2(x + PAD.x + shift, y + PAD.y), galley.clone(), palette.text);
                x += width;
            }
            if row > 0 {
                painter.hline(rect.x_range(), y, line);
            }
            y += height;
        }
        let mut x = rect.left();
        for width in &widths[..widths.len() - 1] {
            x += width;
            painter.vline(x, rect.y_range(), line);
        }
        painter.hline(rect.x_range(), rect.top() + heights[0], Stroke::new(1.0, palette.border_strong));
        painter.rect_stroke(rect, CornerRadius::same(4), line, egui::StrokeKind::Inside);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(align: &[Align], header: &[&str], rows: &[&[&str]]) -> Block {
        let strings = |cells: &[&str]| cells.iter().map(|cell| cell.to_string()).collect();
        Block::Table(Table { align: align.to_vec(), header: strings(header), rows: rows.iter().map(|row| strings(row)).collect() })
    }

    #[test]
    fn markdown_splits_into_blocks_and_spans() {
        let text = "# Plan\nFirst line\nsame paragraph.\n\n- one\n  more\n2. two\n```\ncube(1);\n\nsphere(2);\n```\nafter\n```\nopen";
        assert_eq!(
            blocks(text),
            [
                Block::Heading(1, "Plan".into()),
                Block::Paragraph("First line same paragraph.".into()),
                Block::Item(0, "•".into(), "one more".into()),
                Block::Item(0, "2.".into(), "two".into()),
                Block::Code("cube(1);\n\nsphere(2);".into()),
                Block::Paragraph("after".into()),
                Block::Code("open".into()),
            ]
        );
        assert_eq!(
            spans("a **b** `c` d"),
            [Span::Plain("a "), Span::Bold("b"), Span::Plain(" "), Span::Code("c"), Span::Plain(" d")]
        );
        assert_eq!(spans("2 ** 3 and `x"), [Span::Plain("2 ** 3 and `x")]);
        assert_eq!(spans("`a**b`"), [Span::Code("a**b")]);
    }

    #[test]
    fn tables_read_with_their_alignment_and_ragged_rows() {
        let text = "Les cotes :\n\n| Cote | Valeur | Note |\n|:---|:---:|---:|\n| **Ø** | `12` mm | ok |\n| H | 40 |\n| a \\| b | 3 | x | extra |\nafter";
        assert_eq!(
            blocks(text),
            [
                Block::Paragraph("Les cotes :".into()),
                table(
                    &[Align::Left, Align::Center, Align::Right],
                    &["Cote", "Valeur", "Note"],
                    &[&["**Ø**", "`12` mm", "ok"], &["H", "40", ""], &["a | b", "3", "x"]],
                ),
                Block::Paragraph("after".into()),
            ]
        );
        // Without the pipes at the ends, and a pipe in code stays in its cell.
        assert_eq!(
            blocks("a | b\n--- | ---\n`x|y` | 2"),
            [table(&[Align::Left, Align::Left], &["a", "b"], &[&["`x|y`", "2"]])]
        );
        assert_eq!(spans("**Ø**"), [Span::Bold("Ø")]);
    }

    #[test]
    fn a_table_streaming_in_is_text_until_its_separator_row() {
        assert_eq!(blocks("| Cote | Valeur |"), [Block::Paragraph("| Cote | Valeur |".into())]);
        // The separator row still coming: one cell short.
        assert_eq!(blocks("| Cote | Valeur |\n|---|"), [Block::Paragraph("| Cote | Valeur | |---|".into())]);
        let whole = blocks("| Cote | Valeur |\n|---|---|\n| H | 4");
        assert_eq!(whole, [table(&[Align::Left, Align::Left], &["Cote", "Valeur"], &[&["H", "4"]])]);
    }

    #[test]
    fn headings_quotes_rules_and_nested_lists() {
        let text = "## Two ##\n###Not one\n> quoted\n> on\n>\n> again\n\n---\n* * *\n- top\n  - nested\n    1. deeper\n- back";
        assert_eq!(
            blocks(text),
            [
                Block::Heading(2, "Two".into()),
                Block::Paragraph("###Not one".into()),
                Block::Quote("quoted on\nagain".into()),
                Block::Rule,
                Block::Rule,
                Block::Item(0, "•".into(), "top".into()),
                Block::Item(1, "•".into(), "nested".into()),
                Block::Item(2, "1.".into(), "deeper".into()),
                Block::Item(0, "•".into(), "back".into()),
            ]
        );
    }

    #[test]
    fn links_open_only_on_the_web_and_emphasis_needs_its_markers() {
        assert_eq!(
            spans("see [the datasheet](https://example.com/a.pdf) or [this](file:///etc/passwd)."),
            [
                Span::Plain("see "),
                Span::Link("the datasheet", "https://example.com/a.pdf"),
                Span::Plain(" or "),
                Span::Plain("this"),
                Span::Plain("."),
            ]
        );
        assert_eq!(spans("[x](javascript:alert(1))")[0], Span::Plain("x"));
        assert_eq!(spans("[open](http://a.b"), [Span::Plain("[open](http://a.b")]);
        assert_eq!(spans("*a* and _b_ and __c__"), [
            Span::Italic("a"),
            Span::Plain(" and "),
            Span::Italic("b"),
            Span::Plain(" and "),
            Span::Bold("c"),
        ]);
        assert_eq!(spans("snake_case_name and 2 * 3 * 4"), [Span::Plain("snake_case_name and 2 * 3 * 4")]);
        assert_eq!(spans("**bold *with* more**"), [Span::Bold("bold *with* more")]);
        assert_eq!(spans("é_à_ ü"), [Span::Plain("é_à_ ü")]);
    }
}
