//! The code editor: a highlighted, line-numbered `TextEdit` where Tab
//! indents with spaces.

use std::sync::Arc;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Galley, Id, Key, Margin, Modifiers, Pos2, Rect, Shape, TextBuffer,
    Vec2,
};

use crate::highlight::Highlighter;
use crate::theme;

const INDENT: usize = 4;

pub struct Editor {
    id: Id,
    highlighter: Highlighter,
    /// A line (0-based) to flag in the gutter, such as a syntax error.
    pub error_line: Option<usize>,
    /// Where the cursor is, 0-based line and column, for the status bar.
    pub cursor: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Indent {
    In,
    Out,
}

impl Editor {
    pub fn new() -> Self {
        Editor { id: Id::new("c2f-editor"), highlighter: Highlighter::default(), error_line: None, cursor: None }
    }

    /// Gives the editor keyboard focus.
    pub fn focus(&self, ctx: &egui::Context) {
        ctx.memory_mut(|m| m.request_focus(self.id));
    }

    /// Shows the editor filling `ui`; returns whether the text changed.
    pub fn show(&mut self, ui: &mut egui::Ui, text: &mut String) -> bool {
        let font = theme::code_font();

        // Take Tab before the TextEdit sees it: it would insert a tab.
        let indent = if ui.memory(|m| m.has_focus(self.id)) {
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                    Some(Indent::Out)
                } else if i.consume_key(Modifiers::NONE, Key::Tab) {
                    Some(Indent::In)
                } else {
                    None
                }
            })
        } else {
            None
        };

        let mut changed = false;
        egui::ScrollArea::vertical().id_salt("editor-scroll").auto_shrink(false).show(ui, |ui| {
            let line_count = text.bytes().filter(|&b| b == b'\n').count() + 1;
            let digits = line_count.to_string().len().max(3);
            let digit_width = ui.fonts(|f| f.glyph_width(&font, '0'));
            let gutter_width = digits as f32 * digit_width + 22.0;

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let gutter_left = ui.cursor().left();
                // Painted once the text is laid out, behind it.
                let backdrop = ui.painter().add(Shape::Noop);
                ui.add_space(gutter_width);

                let highlighter = &mut self.highlighter;
                let layout_font = font.clone();
                let mut layouter = |ui: &egui::Ui, buffer: &dyn TextBuffer, wrap_width: f32| -> Arc<Galley> {
                    let job = highlighter.layout_job(buffer.as_str(), &layout_font, wrap_width);
                    ui.fonts(|f| f.layout_job(job))
                };
                let min_height = ui.available_height();
                let mut output = egui::TextEdit::multiline(text)
                    .id(self.id)
                    .font(font.clone())
                    .code_editor()
                    .frame(false)
                    .margin(Margin { left: 10, right: 8, top: 8, bottom: 8 })
                    .desired_width(f32::INFINITY)
                    .min_size(Vec2::new(0.0, min_height))
                    .layouter(&mut layouter)
                    .show(ui);
                changed = output.response.changed();

                let cursor_range = output.cursor_range;
                if let (Some(direction), Some(range)) = (indent, cursor_range) {
                    let length = text.len();
                    let new_range = apply_indent(text, range, direction);
                    output.state.cursor.set_char_range(Some(new_range));
                    output.state.store(ui.ctx(), self.id);
                    changed |= text.len() != length;
                    ui.ctx().request_repaint();
                }

                let has_focus = output.response.has_focus();
                self.cursor = cursor_range.map(|r| line_and_column(text, r.primary.index));
                let current_line = if has_focus { self.cursor.map(|(line, _)| line) } else { None };
                let gutter = Rect::from_min_max(
                    Pos2::new(gutter_left, ui.clip_rect().top()),
                    Pos2::new(gutter_left + gutter_width, ui.clip_rect().bottom().max(output.response.rect.bottom())),
                );
                let shapes = self.gutter_shapes(
                    ui,
                    &output.galley,
                    output.galley_pos,
                    gutter,
                    output.response.rect,
                    current_line,
                    &font,
                );
                ui.painter().set(backdrop, Shape::Vec(shapes));
            });
        });
        changed
    }

    /// The gutter with line numbers, the current-line band and error marks.
    #[allow(clippy::too_many_arguments)]
    fn gutter_shapes(
        &self,
        ui: &egui::Ui,
        galley: &Galley,
        galley_pos: Pos2,
        gutter: Rect,
        text_rect: Rect,
        current_line: Option<usize>,
        font: &FontId,
    ) -> Vec<Shape> {
        let clip = ui.clip_rect();
        let mut shapes = vec![Shape::rect_filled(gutter, CornerRadius::ZERO, theme::GUTTER)];
        let mut numbers = Vec::new();
        let mut line = 0;
        let mut starts_line = true;
        for row in &galley.rows {
            let rect = row.rect().translate(galley_pos.to_vec2());
            let visible = rect.bottom() >= clip.top() && rect.top() <= clip.bottom();
            if visible {
                let band = Rect::from_x_y_ranges(gutter.left()..=text_rect.right(), rect.y_range());
                if current_line == Some(line) {
                    shapes.push(Shape::rect_filled(band, CornerRadius::ZERO, theme::CURRENT_LINE));
                }
                if self.error_line == Some(line) {
                    shapes.push(Shape::rect_filled(band, CornerRadius::ZERO, theme::ERROR.gamma_multiply(0.12)));
                    if starts_line {
                        let mark =
                            Rect::from_min_size(Pos2::new(gutter.left(), rect.top()), Vec2::new(3.0, rect.height()));
                        shapes.push(Shape::rect_filled(mark, CornerRadius::ZERO, theme::ERROR));
                    }
                }
                if starts_line {
                    let color = if self.error_line == Some(line) {
                        theme::ERROR
                    } else if current_line == Some(line) {
                        theme::TEXT
                    } else {
                        theme::TEXT_FAINT
                    };
                    numbers.push((Pos2::new(gutter.right() - 12.0, rect.top()), line + 1, color));
                }
            } else if rect.top() > clip.bottom() {
                break;
            }
            starts_line = row.ends_with_newline;
            if row.ends_with_newline {
                line += 1;
            }
        }
        ui.fonts(|fonts| {
            for (pos, number, color) in numbers {
                shapes.push(Shape::text(fonts, pos, Align2::RIGHT_TOP, number.to_string(), font.clone(), color));
            }
        });
        shapes.push(Shape::line_segment(
            [gutter.right_top() - Vec2::new(0.5, 0.0), gutter.right_bottom() - Vec2::new(0.5, 0.0)],
            egui::Stroke::new(1.0, Color32::from_white_alpha(10)),
        ));
        shapes
    }
}

/// The 0-based line and column of a character index.
fn line_and_column(text: &str, char_index: usize) -> (usize, usize) {
    let byte = byte_index(text, char_index);
    let before = &text[..byte];
    let line = before.bytes().filter(|&b| b == b'\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].chars().count())
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices().nth(char_index).map_or(text.len(), |(i, _)| i)
}

/// Indents (or unindents) the selected lines, or inserts spaces up to the
/// next tab stop at the cursor, and returns the new selection.
fn apply_indent(text: &mut String, range: CCursorRange, direction: Indent) -> CCursorRange {
    let [first, last] = range.sorted_cursors();
    let (start, end) = (first.index, last.index);
    let start_byte = byte_index(text, start);
    let end_byte = byte_index(text, end);
    let first_line = text[..start_byte].rfind('\n').map_or(0, |i| i + 1);

    if start == end && direction == Indent::In {
        let column = text[first_line..start_byte].chars().count();
        let spaces = INDENT - column % INDENT;
        text.insert_str(start_byte, &" ".repeat(spaces));
        return CCursorRange::one(CCursor::new(start + spaces));
    }

    // The lines the selection touches; one that the selection only reaches
    // the very start of is left alone.
    let stop = if end_byte > start_byte && text.as_bytes()[end_byte - 1] == b'\n' { end_byte - 1 } else { end_byte };
    let mut line_starts = vec![first_line];
    line_starts.extend(text[first_line..stop].match_indices('\n').map(|(i, _)| first_line + i + 1));

    // Characters added (positive) or removed per line, last line first.
    let mut deltas: Vec<isize> = Vec::with_capacity(line_starts.len());
    for &line_start in line_starts.iter().rev() {
        match direction {
            Indent::In => {
                text.insert_str(line_start, &" ".repeat(INDENT));
                deltas.push(INDENT as isize);
            }
            Indent::Out => {
                let rest = &text.as_bytes()[line_start..];
                let n = if rest.first() == Some(&b'\t') {
                    1
                } else {
                    rest.iter().take(INDENT).take_while(|&&b| b == b' ').count()
                };
                text.replace_range(line_start..line_start + n, "");
                deltas.push(-(n as isize));
            }
        }
    }
    deltas.reverse();

    // Where a position (as a char and a byte index before the edit) ends up.
    // Only ASCII spaces and tabs change, so chars and bytes shift alike.
    let total: isize = deltas.iter().sum();
    let moved = |char_index: usize, byte: usize| -> usize {
        let shifted = if byte > stop {
            // At the start of the untouched line after the selection.
            total
        } else {
            let line = line_starts.partition_point(|&s| s <= byte).saturating_sub(1);
            let before: isize = deltas[..line].iter().sum();
            let column = (byte - line_starts[line]) as isize;
            let own = if deltas[line] < 0 { deltas[line].max(-column) } else { deltas[line] };
            before + own
        };
        char_index.saturating_add_signed(shifted)
    };
    let new_start = moved(start, start_byte);
    let new_end = moved(end, end_byte);
    if range.primary.index >= range.secondary.index {
        CCursorRange::two(CCursor::new(new_start), CCursor::new(new_end))
    } else {
        CCursorRange { primary: CCursor::new(new_start), secondary: CCursor::new(new_end), h_pos: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, a: usize, b: usize, direction: Indent) -> (String, usize, usize) {
        let mut text = text.to_string();
        let range = CCursorRange::two(CCursor::new(a), CCursor::new(b));
        let out = apply_indent(&mut text, range, direction);
        let [s, e] = out.sorted_cursors();
        (text, s.index, e.index)
    }

    #[test]
    fn tab_inserts_spaces_to_the_next_stop() {
        assert_eq!(run("ab", 2, 2, Indent::In), ("ab  ".into(), 4, 4));
        assert_eq!(run("x\ncube();", 2, 2, Indent::In), ("x\n    cube();".into(), 6, 6));
    }

    #[test]
    fn indents_and_unindents_selected_lines() {
        let (text, s, e) = run("a\nb\nc", 0, 3, Indent::In);
        assert_eq!(text, "    a\n    b\nc");
        assert_eq!((s, e), (4, 11));
        let (text, s, e) = run(&text, 4, 11, Indent::Out);
        assert_eq!(text, "a\nb\nc");
        assert_eq!((s, e), (0, 3));
    }

    #[test]
    fn selection_ending_at_a_line_start_skips_that_line() {
        let (text, s, e) = run("a\nb\n", 0, 2, Indent::In);
        assert_eq!(text, "    a\nb\n");
        assert_eq!((s, e), (4, 6));
    }

    #[test]
    fn shift_tab_without_selection_unindents_the_line() {
        assert_eq!(run("      x", 7, 7, Indent::Out), ("  x".into(), 3, 3));
        assert_eq!(run("  x", 1, 1, Indent::Out), ("x".into(), 0, 0));
    }

    /// Runs one frame of the editor with `events`; returns whether the text
    /// changed.
    fn frame(ctx: &egui::Context, editor: &mut Editor, text: &mut String, events: Vec<egui::Event>) -> bool {
        let input = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            ..Default::default()
        };
        let mut changed = false;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| changed = editor.show(ui, text));
        });
        changed
    }

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn typing_and_tab_in_the_widget() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let mut text = String::new();
        editor.focus(&ctx);
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert!(frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("x".into())]));
        assert_eq!(text, "x");
        assert!(frame(&ctx, &mut editor, &mut text, vec![key(Key::Tab, Modifiers::NONE)]));
        assert_eq!(text, "x   ");
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("y".into())]);
        assert_eq!(text, "x   y");
        assert_eq!(editor.cursor, Some((0, 5)));
        assert!(!frame(&ctx, &mut editor, &mut text, vec![key(Key::Tab, Modifiers::SHIFT)]));
        assert_eq!(text, "x   y", "nothing to unindent");
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Enter, Modifiers::NONE), key(Key::Tab, Modifiers::NONE)]);
        assert_eq!(text, "x   y\n    ");
    }

    #[test]
    fn lines_and_columns() {
        assert_eq!(line_and_column("ab\nécd", 5), (1, 2));
        assert_eq!(line_and_column("", 0), (0, 0));
    }
}
