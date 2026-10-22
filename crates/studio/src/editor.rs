//! The code editor: a highlighted, line-numbered `TextEdit` where Tab
//! indents with spaces, with a find and replace bar above it.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Galley, Id, Key, Layout, Margin, Modifiers, Pos2,
    Rect, RichText, Shape, Stroke, TextBuffer, TextEdit, Vec2,
};

use crate::find;
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
    /// The selection as last shown, to start a search from.
    selection: Option<CCursorRange>,
    /// A byte offset to scroll into view once laid out.
    reveal_at: Option<usize>,
    find: FindBar,
}

/// What the app asks of the find bar, from its shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindRequest {
    /// Open the bar (with the replace row or not) and focus the query.
    Open { replace: bool },
    /// Go to the next match, or the previous one.
    Step { forward: bool },
}

/// Something to do this frame, from a key or a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FindAction {
    Step { forward: bool },
    Replace,
    ReplaceAll,
    Close,
}

/// The find bar and what it found.
#[derive(Default)]
struct FindBar {
    open: bool,
    replace: bool,
    query: String,
    replacement: String,
    case_sensitive: bool,
    /// What `matches` were found in, and for which query.
    searched: Option<(String, String, bool)>,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    /// A byte offset: the search picks the first match at or after it.
    anchor: usize,
    requests: Vec<FindRequest>,
    /// Focus the query field and select its text.
    focus_query: bool,
    /// Scroll the current match into view once laid out.
    scroll: bool,
    /// Whether the bar or the editor had the focus at the end of the last
    /// frame: egui drops the focus on Escape before the bar sees the key.
    had_focus: bool,
}

const QUERY_ID: &str = "c2f-find-query";
const REPLACEMENT_ID: &str = "c2f-find-replacement";

impl FindBar {
    /// Finds the matches again when the text or the query changed.
    fn refresh(&mut self, text: &str) {
        let fresh = self
            .searched
            .as_ref()
            .is_some_and(|(t, q, c)| t == text && *q == self.query && *c == self.case_sensitive);
        if fresh {
            return;
        }
        self.matches = find::find_all(text, &self.query, self.case_sensitive);
        self.current = find::match_from(&self.matches, self.anchor);
        self.searched = Some((text.to_owned(), self.query.clone(), self.case_sensitive));
    }

    fn current_match(&self) -> Option<Range<usize>> {
        self.current.and_then(|i| self.matches.get(i)).cloned()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Indent {
    In,
    Out,
}

impl Editor {
    #[cfg(test)]
    pub fn new() -> Self {
        Editor::for_tab(0)
    }

    /// The editor of tab `tab`: each keeps its own cursor, scroll and undo
    /// history, which egui stores under the editor's id.
    pub fn for_tab(tab: u64) -> Self {
        Editor {
            id: Id::new(("c2f-editor", tab)),
            highlighter: Highlighter::default(),
            error_line: None,
            cursor: None,
            selection: None,
            reveal_at: None,
            find: FindBar::default(),
        }
    }

    /// Opens the find bar or moves between matches, on the next frame.
    pub fn find(&mut self, request: FindRequest) {
        self.find.requests.push(request);
    }

    /// Highlights the text as a circuit file, or as code.
    pub fn set_circuit(&mut self, circuit: bool) {
        self.highlighter.hash_comments = circuit;
    }

    /// Gives the editor keyboard focus.
    pub fn focus(&self, ctx: &egui::Context) {
        ctx.memory_mut(|m| m.request_focus(self.id));
    }

    /// Drops what egui keeps of the editor, its undo history first, once
    /// its tab is closed.
    pub fn forget(&self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.remove::<egui::text_edit::TextEditState>(self.id));
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

        let mut changed = self.find_bar(ui, text);
        egui::ScrollArea::vertical().id_salt(self.id.with("scroll")).auto_shrink(false).show(ui, |ui| {
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
                let find_bar = &mut self.find;
                let layout_font = font.clone();
                let mut layouter = |ui: &egui::Ui, buffer: &dyn TextBuffer, wrap_width: f32| -> Arc<Galley> {
                    let mut job = highlighter.layout_job(buffer.as_str(), &layout_font, wrap_width);
                    if find_bar.open {
                        find_bar.refresh(buffer.as_str());
                        let (matches, current) = (&find_bar.matches, find_bar.current);
                        find::mark(&mut job, matches, current, theme::FIND_MATCH, theme::FIND_CURRENT);
                    }
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
                changed |= output.response.changed();

                let cursor_range = output.cursor_range;
                if let (Some(direction), Some(range)) = (indent, cursor_range) {
                    let length = text.len();
                    let new_range = apply_indent(text, range, direction);
                    output.state.cursor.set_char_range(Some(new_range));
                    output.state.store(ui.ctx(), self.id);
                    changed |= text.len() != length;
                    ui.ctx().request_repaint();
                }

                if let Some(offset) = self.reveal_at.take() {
                    let at = output.galley.pos_from_cursor(CCursor::new(char_index(text, offset.min(text.len()))));
                    ui.scroll_to_rect(at.translate(output.galley_pos.to_vec2()), Some(Align::Center));
                }
                if std::mem::take(&mut self.find.scroll) {
                    if let Some(m) = self.find.current_match() {
                        let start = output.galley.pos_from_cursor(CCursor::new(char_index(text, m.start)));
                        ui.scroll_to_rect(start.translate(output.galley_pos.to_vec2()), Some(Align::Center));
                    }
                }

                let has_focus = output.response.has_focus();
                self.selection = cursor_range;
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
        self.find.had_focus = ui.memory(|m| {
            m.has_focus(self.id) || m.has_focus(Id::new(QUERY_ID)) || m.has_focus(Id::new(REPLACEMENT_ID))
        });
        changed
    }

    /// Handles the find requests and keys, shows the bar when it is open
    /// and carries out what it asks; returns whether the text changed.
    fn find_bar(&mut self, ui: &mut egui::Ui, text: &mut String) -> bool {
        let ctx = ui.ctx().clone();
        let query_id = Id::new(QUERY_ID);
        let replacement_id = Id::new(REPLACEMENT_ID);
        let mut actions = Vec::new();
        for request in std::mem::take(&mut self.find.requests) {
            match request {
                FindRequest::Open { replace } => self.open_find(&ctx, text, replace),
                FindRequest::Step { forward } if !self.find.query.is_empty() => {
                    actions.push(FindAction::Step { forward });
                }
                FindRequest::Step { .. } => self.open_find(&ctx, text, false),
            }
        }
        if !self.find.open {
            return self.apply(&ctx, text, actions);
        }

        // Keys in the bar's fields, before the fields see them.
        let in_query = ctx.memory(|m| m.has_focus(query_id));
        let in_replacement = ctx.memory(|m| m.has_focus(replacement_id));
        let had_focus = self.find.had_focus;
        ctx.input_mut(|i| {
            if had_focus && i.consume_key(Modifiers::NONE, Key::Escape) {
                actions.push(FindAction::Close);
            } else if in_query && i.consume_key(Modifiers::SHIFT, Key::Enter) {
                actions.push(FindAction::Step { forward: false });
            } else if in_query && i.consume_key(Modifiers::NONE, Key::Enter) {
                actions.push(FindAction::Step { forward: true });
            } else if in_replacement && i.consume_key(Modifiers::NONE, Key::Enter) {
                actions.push(FindAction::Replace);
            }
        });

        if std::mem::take(&mut self.find.focus_query) {
            let mut state = TextEdit::load_state(&ctx, query_id).unwrap_or_default();
            let all = CCursorRange::two(CCursor::new(0), CCursor::new(self.find.query.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(&ctx, query_id);
            ctx.memory_mut(|m| m.request_focus(query_id));
        }

        let frame = Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(10, 6));
        let bar = frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 6.0);
            // The buttons and the count are laid out first, from the right;
            // each field then takes what is left of the row and never more,
            // since a wider row would widen the resizable editor panel,
            // whose next frame would widen the field again.
            let font = theme::code_font();
            let mut searched = false;
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 22.0), Layout::right_to_left(Align::Center), |ui| {
                if ui.add(small_button("×")).on_hover_text("Close (Esc)").clicked() {
                    actions.push(FindAction::Close);
                }
                let any = !self.find.matches.is_empty() && !self.find.query.is_empty();
                let next = ui.add_enabled(any, small_button("⏷")).on_hover_text("Next match (⌘G)");
                if next.clicked() {
                    actions.push(FindAction::Step { forward: true });
                }
                let previous = ui.add_enabled(any, small_button("⏶")).on_hover_text("Previous match (⇧⌘G)");
                if previous.clicked() {
                    actions.push(FindAction::Step { forward: false });
                }
                let count = if self.find.query.is_empty() {
                    String::new()
                } else {
                    self.find.refresh(text);
                    find::count_label(self.find.matches.len(), self.find.current)
                };
                let color = if self.find.matches.is_empty() { theme::ERROR } else { theme::TEXT_DIM };
                ui.add_sized([68.0, 20.0], egui::Label::new(RichText::new(count).size(12.0).color(color)));
                ui.add_space(4.0);
                let case = ui.toggle_value(&mut self.find.case_sensitive, RichText::new("Aa").size(12.5));
                searched |= case.on_hover_text("Match case").changed();
                let query = TextEdit::singleline(&mut self.find.query)
                    .id(query_id)
                    .font(font.clone())
                    .hint_text("Find")
                    .desired_width(field_width(ui));
                searched |= ui.add(query).changed();
            });
            if self.find.replace {
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 22.0), Layout::right_to_left(Align::Center), |ui| {
                    let any = self.find.current.is_some() && !self.find.query.is_empty();
                    if ui.add_enabled(any, Button::new("Replace All")).on_hover_text("Replace every match").clicked() {
                        actions.push(FindAction::ReplaceAll);
                    }
                    if ui.add_enabled(any, Button::new("Replace")).on_hover_text("Replace this match").clicked() {
                        actions.push(FindAction::Replace);
                    }
                    let replacement = TextEdit::singleline(&mut self.find.replacement)
                        .id(replacement_id)
                        .font(font)
                        .hint_text("Replace")
                        .desired_width(field_width(ui));
                    ui.add(replacement);
                });
            }
            searched
        });
        let rect = bar.response.rect;
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::BORDER));
        if bar.inner {
            // Search as you type, from where the bar was opened.
            self.find.refresh(text);
            self.reveal(&ctx, text);
        }
        self.apply(&ctx, text, actions)
    }

    /// Opens the bar (keeping its replace row when it is already open),
    /// taking a one-line selection as the query when coming from the editor.
    fn open_find(&mut self, ctx: &egui::Context, text: &str, replace: bool) {
        let from_editor = !self.find.open || ctx.memory(|m| m.has_focus(self.id));
        let find = &mut self.find;
        find.replace = replace || (find.open && find.replace);
        find.open = true;
        find.focus_query = true;
        if let Some(range) = self.selection.filter(|_| from_editor) {
            let [start, end] = range.sorted_cursors();
            let (start, end) = (byte_index(text, start.index), byte_index(text, end.index));
            let selected = &text[start..end];
            if !selected.is_empty() && !selected.contains('\n') {
                find.query = selected.to_owned();
            }
            find.anchor = start;
        }
        find.searched = None;
    }

    /// Carries out the bar's actions; returns whether the text changed.
    fn apply(&mut self, ctx: &egui::Context, text: &mut String, actions: Vec<FindAction>) -> bool {
        let mut changed = false;
        for action in actions {
            self.find.refresh(text);
            match action {
                FindAction::Step { forward } => {
                    self.find.current = find::step(self.find.matches.len(), self.find.current, forward);
                    if let Some(m) = self.find.current_match() {
                        self.find.anchor = m.start;
                    }
                    self.reveal(ctx, text);
                }
                FindAction::Replace => {
                    let Some(m) = self.find.current_match() else { continue };
                    let mut new_text = text.clone();
                    new_text.replace_range(m.clone(), &self.find.replacement);
                    // Go on with the match after the replaced one, selected
                    // as part of the edit so that undo keeps no extra step.
                    let after = m.start + self.find.replacement.len();
                    self.find.anchor = after;
                    self.find.refresh(&new_text);
                    let selection = match self.find.current_match() {
                        Some(next) => match_selection(&new_text, next),
                        None => CCursorRange::one(CCursor::new(char_index(&new_text, after))),
                    };
                    self.edit(ctx, text, new_text, selection);
                    self.find.scroll = true;
                    changed = true;
                }
                FindAction::ReplaceAll => {
                    if self.find.matches.is_empty() {
                        continue;
                    }
                    let new_text = find::replace_all(text, &self.find.matches, &self.find.replacement);
                    let cursor = self.selection.map_or(0, |r| r.primary.index).min(new_text.chars().count());
                    self.edit(ctx, text, new_text, CCursorRange::one(CCursor::new(cursor)));
                    self.find.anchor = 0;
                    self.find.refresh(text);
                    // So that the next Cmd+Z goes to the editor.
                    self.focus(ctx);
                    changed = true;
                }
                FindAction::Close => {
                    self.find.open = false;
                    self.focus(ctx);
                }
            }
        }
        changed
    }

    /// Replaces the text from outside the editor, as one step of its undo
    /// history, with the cursor at char `cursor`.
    pub fn replace_text(&self, ctx: &egui::Context, text: &mut String, new_text: String, cursor: usize) {
        self.edit(ctx, text, new_text, CCursorRange::one(CCursor::new(cursor)));
    }

    /// Undoes (`back`) or redoes one step of the editor's history from
    /// outside the editor, as Cmd+Z does in it: for the edits made in the
    /// 3D view while the editor has no focus. Returns whether the text
    /// changed.
    pub fn undo_step(&self, ctx: &egui::Context, text: &mut String, back: bool) -> bool {
        let mut state = TextEdit::load_state(ctx, self.id).unwrap_or_default();
        let current = (state.cursor.char_range().unwrap_or_default(), text.clone());
        let mut undoer = state.undoer();
        let found = if back { undoer.undo(&current) } else { undoer.redo(&current) }.cloned();
        let Some((selection, previous)) = found else { return false };
        let changed = previous != *text;
        *text = previous;
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(selection));
        state.store(ctx, self.id);
        changed
    }

    /// Replaces the text as one step of the editor's undo history.
    fn edit(&self, ctx: &egui::Context, text: &mut String, new_text: String, selection: CCursorRange) {
        let mut state = TextEdit::load_state(ctx, self.id).unwrap_or_default();
        let before = state.cursor.char_range().unwrap_or_default();
        let mut undoer = state.undoer();
        undoer.add_undo(&(before, text.clone()));
        *text = new_text;
        // Feeding the new state first drops the redo steps it replaces.
        let after = (selection, text.clone());
        undoer.feed_state(ctx.input(|i| i.time), &after);
        undoer.add_undo(&after);
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(selection));
        state.store(ctx, self.id);
    }

    /// Puts the cursor at the start of the line holding byte `offset` of
    /// `text`, and scrolls it into view.
    pub fn go_to(&mut self, ctx: &egui::Context, text: &str, offset: usize) {
        let line_start = line_start(text, offset);
        let mut state = TextEdit::load_state(ctx, self.id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(char_index(text, line_start)))));
        state.store(ctx, self.id);
        self.reveal_at = Some(line_start);
    }

    /// Selects the current match in the editor and scrolls to it.
    fn reveal(&mut self, ctx: &egui::Context, text: &str) {
        let Some(m) = self.find.current_match() else { return };
        let mut state = TextEdit::load_state(ctx, self.id).unwrap_or_default();
        state.cursor.set_char_range(Some(match_selection(text, m)));
        state.store(ctx, self.id);
        self.find.scroll = true;
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

/// The character index of a byte offset on a character boundary.
/// The start of the line holding byte `offset` of `text`, wherever the
/// offset falls: past the end, or inside a character (it may come from an
/// older text).
fn line_start(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}

fn char_index(text: &str, byte: usize) -> usize {
    text.get(..byte).map_or_else(|| text.chars().count(), |before| before.chars().count())
}

/// A match, as a selection in the editor.
fn match_selection(text: &str, m: Range<usize>) -> CCursorRange {
    CCursorRange::two(CCursor::new(char_index(text, m.start)), CCursor::new(char_index(text, m.end)))
}

/// A compact button for the find bar's arrows and close.
/// The text width a single-line field can ask for and still fit, frame
/// included, in what is left of the row.
fn field_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - 2.0 * ui.spacing().button_padding.x - 2.0).max(40.0)
}

fn small_button(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).size(12.0)).min_size(Vec2::new(24.0, 22.0)).frame(false)
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

    /// The editor's selection, sorted, as character indices.
    fn selection(ctx: &egui::Context, editor: &Editor) -> Option<(usize, usize)> {
        let range = TextEdit::load_state(ctx, editor.id)?.cursor.char_range()?;
        let [start, end] = range.sorted_cursors();
        Some((start.index, end.index))
    }

    fn count(editor: &Editor) -> String {
        find::count_label(editor.find.matches.len(), editor.find.current)
    }

    #[test]
    fn find_bar_keeps_the_resizable_panel_width() {
        // The app shows the editor in a resizable side panel, which grows
        // to fit a row wider than it: the bar must never be that row.
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut editor = Editor::new();
        let mut text = "cube(10); sphere(5);".to_string();
        let mut widths = Vec::new();
        for n in 0..12 {
            if n == 1 {
                editor.find(FindRequest::Open { replace: true });
            }
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                let panel = egui::SidePanel::left("editor")
                    .frame(Frame::new().fill(theme::EDITOR))
                    .resizable(true)
                    .default_width(320.0)
                    .width_range(260.0..=900.0)
                    .show(ctx, |ui| editor.show(ui, &mut text));
                widths.push(panel.response.rect.width());
            });
        }
        assert!(editor.find.open && editor.find.replace);
        assert!(widths.iter().all(|w| (w - widths[0]).abs() < 0.5), "the panel width moved: {widths:?}");
    }

    #[test]
    fn find_bar_steps_through_matches_and_wraps() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let mut text = "cube(); sphere(); cube(); Cube();".to_string();
        editor.focus(&ctx);
        frame(&ctx, &mut editor, &mut text, vec![]);
        editor.find(FindRequest::Open { replace: false });
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert!(editor.find.open);
        assert!(ctx.memory(|m| m.has_focus(Id::new(QUERY_ID))), "the query field has the focus");

        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("cube".into())]);
        assert_eq!(editor.find.query, "cube");
        assert_eq!(count(&editor), "1 of 3", "case-insensitive by default");
        assert_eq!(selection(&ctx, &editor), Some((0, 4)), "the current match is selected");

        frame(&ctx, &mut editor, &mut text, vec![key(Key::Enter, Modifiers::NONE)]);
        assert_eq!(count(&editor), "2 of 3");
        assert_eq!(selection(&ctx, &editor), Some((18, 22)));
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Enter, Modifiers::NONE)]);
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Enter, Modifiers::NONE)]);
        assert_eq!(count(&editor), "1 of 3", "Enter wraps to the first match");
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Enter, Modifiers::SHIFT)]);
        assert_eq!(count(&editor), "3 of 3", "Shift+Enter wraps to the last match");
        assert_eq!(editor.find.query, "cube", "Enter does not reach the field");

        editor.find(FindRequest::Step { forward: true });
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert_eq!(count(&editor), "1 of 3");
        editor.find(FindRequest::Step { forward: false });
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert_eq!(count(&editor), "3 of 3");

        editor.find.case_sensitive = true;
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert_eq!(editor.find.matches.len(), 2, "Cube no longer matches");

        frame(&ctx, &mut editor, &mut text, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(!editor.find.open);
        assert!(ctx.memory(|m| m.has_focus(editor.id)), "Escape goes back to the editor");
        assert_eq!(text, "cube(); sphere(); cube(); Cube();");
    }

    #[test]
    fn find_bar_takes_the_selection_as_query() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let mut text = "a = 1;\nb = a + a;".to_string();
        editor.focus(&ctx);
        frame(&ctx, &mut editor, &mut text, vec![]);
        let mut state = TextEdit::load_state(&ctx, editor.id).unwrap();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(11), CCursor::new(12))));
        state.store(&ctx, editor.id);
        frame(&ctx, &mut editor, &mut text, vec![]);
        editor.find(FindRequest::Open { replace: true });
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert_eq!(editor.find.query, "a");
        assert!(editor.find.replace);
        assert_eq!(count(&editor), "2 of 3", "starts at the selection");
    }

    #[test]
    fn edits_from_the_view_undo_and_redo_without_the_editor() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let mut text = "at = [0, 0, 0]".to_string();
        frame(&ctx, &mut editor, &mut text, vec![]);
        editor.replace_text(&ctx, &mut text, "at = [5, 0, 0]".into(), 0);
        editor.replace_text(&ctx, &mut text, "at = [5, 7, 0]".into(), 0);
        assert!(!ctx.memory(|m| m.has_focus(editor.id)));
        assert!(editor.undo_step(&ctx, &mut text, true));
        assert_eq!(text, "at = [5, 0, 0]");
        assert!(editor.undo_step(&ctx, &mut text, true));
        assert_eq!(text, "at = [0, 0, 0]");
        assert!(!editor.undo_step(&ctx, &mut text, true), "nothing older");
        assert!(editor.undo_step(&ctx, &mut text, false));
        assert_eq!(text, "at = [5, 0, 0]");
    }

    #[test]
    fn replace_and_replace_all_undo_in_one_step() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let original = "r = 1;\nf(r);\n// r".to_string();
        let mut text = original.clone();
        editor.focus(&ctx);
        frame(&ctx, &mut editor, &mut text, vec![]);
        editor.find(FindRequest::Open { replace: true });
        frame(&ctx, &mut editor, &mut text, vec![]);
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("r".into())]);
        assert_eq!(count(&editor), "1 of 3");

        // Enter in the replacement field replaces the current match.
        ctx.memory_mut(|m| m.request_focus(Id::new(REPLACEMENT_ID)));
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("size".into())]);
        assert!(frame(&ctx, &mut editor, &mut text, vec![key(Key::Enter, Modifiers::NONE)]));
        assert_eq!(text, "size = 1;\nf(r);\n// r");
        assert_eq!(count(&editor), "1 of 2", "goes on with the next match");
        assert_eq!(selection(&ctx, &editor), Some((12, 13)));

        // The Replace All button, then one Cmd+Z in the editor.
        editor.apply(&ctx, &mut text, vec![FindAction::ReplaceAll]);
        assert_eq!(text, "size = 1;\nf(size);\n// size");
        assert_eq!(count(&editor), "No results");
        assert!(ctx.memory(|m| m.has_focus(editor.id)));
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert!(frame(&ctx, &mut editor, &mut text, vec![key(Key::Z, Modifiers::COMMAND)]));
        assert_eq!(text, "size = 1;\nf(r);\n// r", "one undo restores every match");
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Z, Modifiers::COMMAND)]);
        assert_eq!(text, original, "and the next one the single replace");
    }

    #[test]
    fn replace_all_drops_older_redo_steps() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let mut text = "x = 1;".to_string();
        editor.focus(&ctx);
        frame(&ctx, &mut editor, &mut text, vec![]);
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("y".into())]);
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Z, Modifiers::COMMAND)]);
        assert_eq!(text, "x = 1;");
        editor.find(FindRequest::Open { replace: true });
        frame(&ctx, &mut editor, &mut text, vec![]);
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("x".into())]);
        editor.find.replacement = "w".into();
        editor.apply(&ctx, &mut text, vec![FindAction::ReplaceAll]);
        frame(&ctx, &mut editor, &mut text, vec![]);
        frame(&ctx, &mut editor, &mut text, vec![key(Key::Z, Modifiers { shift: true, ..Modifiers::COMMAND })]);
        assert_eq!(text, "w = 1;", "redo has nothing to bring back");
    }

    #[test]
    fn find_again_keeps_the_typed_query_and_the_replace_row() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new();
        let mut text = "width = 1;".to_string();
        editor.focus(&ctx);
        frame(&ctx, &mut editor, &mut text, vec![]);
        let mut state = TextEdit::load_state(&ctx, editor.id).unwrap();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(5))));
        state.store(&ctx, editor.id);
        frame(&ctx, &mut editor, &mut text, vec![]);
        editor.find(FindRequest::Open { replace: true });
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert_eq!(editor.find.query, "width");
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("heigth".into())]);
        editor.find(FindRequest::Open { replace: false });
        frame(&ctx, &mut editor, &mut text, vec![]);
        assert_eq!(editor.find.query, "heigth");
        assert!(editor.find.replace);
    }

    #[test]
    fn lines_and_columns() {
        assert_eq!(line_and_column("ab\nécd", 5), (1, 2));
        assert_eq!(line_and_column("", 0), (0, 0));
    }

    #[test]
    fn go_to_takes_any_offset() {
        let text = "a\néé\nb";
        assert_eq!(line_start(text, 3), 2);
        // Inside `é`, and past the end.
        assert_eq!(line_start(text, 4), 2);
        assert_eq!(line_start(text, 99), 7);
        let mut editor = Editor::new();
        editor.go_to(&egui::Context::default(), text, 3);
        assert_eq!(editor.reveal_at, Some(2));
    }
}
