//! The tree of the program (C2F-41), as Fusion's browser shows a design,
//! and the code-free mode that hides the editor.
//!
//! The tree is read from the code ([`code_edit::tree`]): one row per
//! instance statement of the main file, nested as the code nests them,
//! named by the `// name` comment ending the line it starts on. Every
//! action on a row is an edit of the code, applied as one undo step: a
//! double-click (or F2) renames, the eye and the `!` and `%` buttons write
//! the modifiers, a drag moves the object before, after or into another.
//! A click selects the object in the 3D view, and what the view selects
//! lights up in the tree, the two matched by the statement's bytes.
//!
//! The code-free mode only hides the editor: the view, the tree, the
//! panels and the toolbar stay, so that a design is made with the mouse
//! alone while its code is still written underneath.

use std::collections::HashSet;
use std::ops::Range;

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke,
    TextEdit, Vec2,
};

use super::{CODE, StudioApp};
use crate::code_edit::{self, Modifier, NodeKind, Target, TreeNode};
use crate::theme;

/// The height of a row and the indentation of each level.
const ROW: f32 = 22.0;
const INDENT: f32 = 14.0;
/// The side of the toggles at a row's right end.
const TOGGLE: f32 = 18.0;

/// The tree of the active tab's program, read again only when its text
/// changes, and what the panel is doing with it.
#[derive(Default)]
pub(super) struct TreeState {
    /// The tab and the text the tree was last read from.
    tab: Option<u64>,
    text: String,
    /// The tree of the last text that parsed.
    nodes: Vec<TreeNode>,
    /// Why the last text does not parse: `nodes` is then an older tree,
    /// shown greyed.
    error: Option<String>,
    /// The rows folded, by their path of child indices.
    collapsed: HashSet<Vec<usize>>,
    /// The row last clicked, for F2 when the view selects nothing (an
    /// object left out with `*` cannot be selected there).
    focus: Option<Range<usize>>,
    /// The object being renamed and the name typed so far.
    renaming: Option<Renaming>,
    /// The selection as last shown, to scroll to a new one.
    shown_selection: Vec<Range<usize>>,
}

struct Renaming {
    range: Range<usize>,
    name: String,
    /// The field has had the keyboard focus once.
    focused: bool,
}

/// What a row of the tree asks of the code.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Action {
    /// Select the object in the view; `add` (Shift) adds it or takes it
    /// out.
    Select {
        range: Range<usize>,
        add: bool,
    },
    Rename {
        range: Range<usize>,
        name: String,
    },
    SetModifier {
        range: Range<usize>,
        modifier: Modifier,
        on: bool,
    },
    Move {
        range: Range<usize>,
        target: Target,
    },
}

/// The object dragged in the tree, and the text it was read from: a drag
/// begun before the code changed means nothing in the new one.
struct Dragged {
    range: Range<usize>,
    name: String,
    text: String,
}

impl TreeState {
    /// Reads the tree of `text`, the code of tab `tab`, unless it was read
    /// from it already; returns whether it was read. A text that does not
    /// parse keeps the last tree of that tab.
    pub(super) fn refresh(&mut self, tab: u64, text: &str) -> bool {
        if self.tab == Some(tab) && self.text == text {
            return false;
        }
        if self.tab != Some(tab) {
            *self = TreeState { tab: Some(tab), ..TreeState::default() };
        }
        self.text = text.to_string();
        match code_edit::tree(text) {
            Ok(nodes) => {
                self.nodes = nodes;
                self.error = None;
            }
            Err(why) => self.error = Some(why),
        }
        true
    }

    /// Whether the tree shown is not the code's: the code does not parse.
    pub(super) fn stale(&self) -> bool {
        self.error.is_some()
    }

    /// Starts renaming the object at `range`, when it has a row.
    pub(super) fn start_rename(&mut self, range: &Range<usize>) {
        if self.stale() {
            return;
        }
        if let Some(path) = path_of(&self.nodes, range) {
            let node = node_at(&self.nodes, &path);
            self.renaming = Some(Renaming { range: range.clone(), name: node.name.clone(), focused: false });
        }
    }

    /// Draws the rows, `selected` the ranges the view selects, and returns
    /// what the user asked of one of them.
    fn show(&mut self, ui: &mut egui::Ui, selected: &[Range<usize>]) -> Option<Action> {
        let live = !self.stale();
        let nodes = std::mem::take(&mut self.nodes);
        let rows = visible_rows(&nodes, &self.collapsed);
        let reveal = selected != self.shown_selection.as_slice();
        if reveal {
            self.shown_selection = selected.to_vec();
            // Unfold what holds a newly selected object.
            for range in selected {
                if let Some(path) = path_of(&nodes, range) {
                    for end in 1..path.len() {
                        self.collapsed.remove(&path[..end]);
                    }
                }
            }
        }
        let dragged = egui::DragAndDrop::payload::<Dragged>(ui.ctx()).filter(|d| d.text == self.text);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let released = ui.input(|i| i.pointer.any_released());
        let mut action = None;
        // A name typed is kept apart: the click on another row that ends
        // its field asks for a selection in the same frame, which would
        // otherwise take its place and lose the name.
        let mut rename = None;
        let mut last = Rect::NOTHING;
        for (path, depth) in &rows {
            let node = node_at(&nodes, path);
            let is_selected = selected.contains(&node.range);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click_and_drag());
            last = rect;
            if reveal && is_selected {
                response.scroll_to_me(None);
            }
            // Where a drag held over this row would drop.
            let drop = match (&dragged, pointer) {
                (Some(d), Some(p)) if live && rect.contains(p) => {
                    let target = drop_target(node, (p.y - rect.top()) / rect.height());
                    valid_drop(&nodes, &d.range, &target).then_some(target)
                }
                _ => None,
            };
            // Left out with `*`, itself or what holds it.
            let hidden = (1..=path.len()).any(|n| node_at(&nodes, &path[..n]).modifiers.disabled);
            let row =
                self.paint_row(ui, rect, &response, node, path, (*depth, hidden), is_selected, live, drop.as_ref());
            if !live {
                continue;
            }
            match row {
                Some(r @ Action::Rename { .. }) => rename = Some(r),
                r => action = action.or(r),
            }
            if response.clicked() {
                self.focus = Some(node.range.clone());
                action = Some(Action::Select { range: node.range.clone(), add: ui.input(|i| i.modifiers.shift) });
            }
            if response.double_clicked() {
                self.start_rename(&node.range);
            }
            response.dnd_set_drag_payload(Dragged {
                range: node.range.clone(),
                name: node.name.clone(),
                text: self.text.clone(),
            });
            if let (Some(target), true) = (drop, released) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                action = Some(Action::Move { range: dragged.as_ref().expect("dragging").range.clone(), target });
            }
        }
        // Below the last row: after the last object of the program.
        if let (Some(d), Some(p), Some(end)) = (&dragged, pointer, nodes.last()) {
            let below = Rect::from_min_max(
                Pos2::new(last.left(), last.bottom()),
                Pos2::new(last.right(), ui.max_rect().bottom()),
            );
            let target = Target::After(end.range.clone());
            if live && below.contains(p) && valid_drop(&nodes, &d.range, &target) {
                ui.painter().hline(last.x_range(), last.bottom(), Stroke::new(2.0, theme::ACCENT));
                if released {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    action = Some(Action::Move { range: d.range.clone(), target });
                }
            }
        }
        if let (Some(d), Some(p)) = (&dragged, pointer) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("dragged")));
            painter.text(
                p + Vec2::new(14.0, 4.0),
                egui::Align2::LEFT_CENTER,
                &d.name,
                FontId::proportional(12.5),
                theme::TEXT,
            );
        }
        self.nodes = nodes;
        rename.or(action)
    }

    /// Draws one row and its controls; returns what its controls ask.
    #[allow(clippy::too_many_arguments)]
    fn paint_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        response: &egui::Response,
        node: &TreeNode,
        path: &[usize],
        (depth, hidden): (usize, bool),
        selected: bool,
        live: bool,
        drop: Option<&Target>,
    ) -> Option<Action> {
        let painter = ui.painter_at(rect);
        let fill = if selected {
            theme::ACCENT_MUTED
        } else if live && response.hovered() {
            theme::HOVER
        } else if self.focus.as_ref() == Some(&node.range) {
            theme::RAISED
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, CornerRadius::same(4), fill);
        match drop {
            Some(Target::Before(_)) => {
                painter.hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0, theme::ACCENT));
            }
            Some(Target::After(_)) => {
                painter.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, theme::ACCENT));
            }
            Some(Target::IntoBlock(_)) => {
                let stroke = Stroke::new(1.5, theme::ACCENT);
                painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(4), stroke, egui::StrokeKind::Inside);
            }
            None => {}
        }
        let dim = !live || hidden;
        let text_color = if dim { theme::TEXT_FAINT } else { theme::TEXT };
        let mut action = None;

        let left = rect.left() + 4.0 + depth as f32 * INDENT;
        if !node.children.is_empty() {
            let arrow = Rect::from_center_size(Pos2::new(left + 6.0, rect.center().y), Vec2::splat(14.0));
            let folded = self.collapsed.contains(path);
            paint_arrow(&painter, arrow.center(), folded, theme::TEXT_DIM);
            if ui.interact(arrow, response.id.with("fold"), Sense::click()).clicked() {
                if folded {
                    self.collapsed.remove(path);
                } else {
                    self.collapsed.insert(path.to_vec());
                }
            }
        }
        let icon = Rect::from_center_size(Pos2::new(left + 21.0, rect.center().y), Vec2::splat(12.0));
        paint_icon(&painter, icon, node.kind, if dim { theme::TEXT_FAINT } else { kind_color(node.kind) });

        // The toggles, from the right end, each always in its place.
        let mut right = rect.right() - 4.0;
        let toggles = [
            (Modifier::Disable, node.modifiers.disabled, "Hide: leave it out of the model (*)"),
            (Modifier::Root, node.modifiers.root, "Isolate: show only this object (!)"),
            (
                Modifier::Background,
                node.modifiers.background,
                "See-through: drawn transparent, left out of the model (%)",
            ),
        ];
        // Not `hovered`, which a press on a toggle turns off.
        let over = live && ui.rect_contains_pointer(rect);
        for (modifier, on, hint) in toggles {
            let area =
                Rect::from_min_max(Pos2::new(right - TOGGLE, rect.top() + 2.0), Pos2::new(right, rect.bottom() - 2.0));
            right -= TOGGLE + 2.0;
            // The eye always shows; `!` and `%` when set or over the row.
            let shown = on || modifier == Modifier::Disable || over;
            if !shown {
                continue;
            }
            let toggle = ui.interact(area, response.id.with(modifier.symbol()), Sense::click());
            let toggle = if live { toggle.on_hover_text(hint) } else { toggle };
            if live && toggle.hovered() {
                painter.rect_filled(area, CornerRadius::same(3), theme::BORDER_STRONG);
            }
            let color = match (modifier, on) {
                (Modifier::Disable, true) => theme::TEXT_FAINT,
                (Modifier::Disable, false) => theme::TEXT_DIM,
                (_, true) => theme::WARNING,
                (_, false) => theme::TEXT_FAINT,
            };
            if modifier == Modifier::Disable {
                paint_eye(&painter, area.center(), !on, color);
            } else {
                let symbol = modifier.symbol().to_string();
                painter.text(area.center(), egui::Align2::CENTER_CENTER, symbol, FontId::monospace(12.0), color);
            }
            if live && toggle.clicked() {
                action = Some(Action::SetModifier { range: node.range.clone(), modifier, on: !on });
            }
        }

        let name_rect = Rect::from_min_max(Pos2::new(left + 32.0, rect.top()), Pos2::new(right, rect.bottom()));
        let renaming = self.renaming.as_mut().filter(|r| live && r.range == node.range);
        if let Some(renaming) = renaming {
            let field = TextEdit::singleline(&mut renaming.name)
                .font(FontId::proportional(13.0))
                .margin(Margin::symmetric(4, 1));
            let edit = ui.put(name_rect.shrink2(Vec2::new(0.0, 1.0)), field);
            if !renaming.focused {
                edit.request_focus();
                renaming.focused = true;
            }
            if edit.lost_focus() {
                let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
                let name = renaming.name.trim().to_string();
                if !cancelled && name != node.name {
                    action = Some(Action::Rename { range: node.range.clone(), name });
                }
                self.renaming = None;
            }
        } else {
            let mut job = egui::text::LayoutJob::default();
            let font = FontId::proportional(13.0);
            job.append(&node.name, 0.0, egui::TextFormat::simple(font.clone(), text_color));
            if node.label.is_some() {
                let call = egui::TextFormat::simple(FontId::proportional(11.5), theme::TEXT_FAINT);
                job.append(&node.call, 6.0, call);
            }
            job.wrap = egui::text::TextWrapping::truncate_at_width(name_rect.width().max(0.0));
            let galley = ui.fonts(|f| f.layout_job(job));
            let at = Pos2::new(name_rect.left(), rect.center().y - galley.size().y / 2.0);
            painter.galley(at, galley, text_color);
        }
        action
    }
}

impl StudioApp {
    /// A panel with the program as a tree of objects and operations,
    /// between the editor and the view.
    pub(super) fn tree_panel(&mut self, ctx: &egui::Context) {
        if !self.settings.show_tree {
            return;
        }
        self.tree.refresh(self.doc.id, &self.doc.text);
        // Only while the view shows this code, which its selection's bytes
        // point into.
        let selected = self.selected_ranges().unwrap_or_default();
        let frame = Frame::new().fill(theme::PANEL).inner_margin(Margin { left: 6, right: 6, top: 8, bottom: 6 });
        let mut action = None;
        egui::SidePanel::left("tree")
            .frame(frame)
            .resizable(true)
            .default_width(220.0)
            .width_range(160.0..=420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Tree").strong().size(12.5).color(theme::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let close = ui.add(egui::Button::new(RichText::new("×").size(14.0)).frame(false));
                        if close.on_hover_text("Hide the tree").clicked() {
                            self.settings.show_tree = false;
                        }
                    });
                });
                if let Some(why) = &self.tree.error {
                    let note = format!("The code does not read: the tree is as it last was.\n{why}");
                    ui.label(RichText::new(note).size(11.5).color(theme::WARNING));
                }
                ui.add_space(4.0);
                if self.tree.nodes.is_empty() && !self.tree.stale() {
                    ui.label(RichText::new("No objects yet").size(12.0).color(theme::TEXT_FAINT));
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    action = self.tree.show(ui, &selected);
                });
            });
        // F2 renames the object selected, or the row last clicked.
        let free = ctx.memory(|m| m.focused().is_none());
        if free && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F2)) {
            if let Some(range) = selected.last().cloned().or_else(|| self.tree.focus.clone()) {
                self.tree.start_rename(&range);
            }
        }
        if let Some(action) = action {
            self.tree_action(ctx, action);
        }
    }

    /// Does what a row of the tree asked, as one edit of the code.
    pub(super) fn tree_action(&mut self, ctx: &egui::Context, action: Action) {
        let text = &self.doc.text;
        match action {
            Action::Select { range, add } => {
                // A view still showing older code would select the wrong
                // bytes: the row only is marked.
                if self.doc.viewer.scene_source() == text.as_str() {
                    self.doc.viewer.select_range(range, add);
                }
            }
            Action::Rename { range, name } => {
                let done = if name.is_empty() { "Name removed".to_string() } else { format!("Renamed {name}") };
                self.apply_code_edit(ctx, code_edit::rename(text, range, &name), &done);
            }
            Action::SetModifier { range, modifier, on } => {
                let result = code_edit::set_modifier(text, range.clone(), Some(modifier), on);
                if let Ok(edit) = &result {
                    self.tree.focus = Some(edit.selection.clone());
                }
                self.apply_code_edit(ctx, result, "");
            }
            Action::Move { range, target } => {
                let result = code_edit::move_into(text, range, target);
                if let Ok(edit) = &result {
                    self.tree.focus = Some(edit.selection.clone());
                }
                self.apply_code_edit(ctx, result, "");
            }
        }
    }

    /// The toolbar's toggles of the tree and of the code-free mode, laid
    /// out right to left.
    pub(super) fn tree_toggles(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let label = if self.settings.show_code { "Code" } else { "No code" };
        let keys = ctx.format_shortcut(&CODE);
        let hint = format!("Show or hide the code: without it, model with the view, the tree and the panels ({keys})");
        if ui.add(egui::Button::selectable(self.settings.show_code, label)).on_hover_text(hint).clicked() {
            self.toggle_code(ctx);
        }
        if !self.doc.is_circuit() {
            let toggle = ui.toggle_value(&mut self.settings.show_tree, "Tree");
            toggle.on_hover_text("Show the program as a tree of objects and operations");
        }
        super::toolbar_separator(ui);
    }

    /// Hides the code editor, or shows it again.
    pub(super) fn toggle_code(&mut self, ctx: &egui::Context) {
        self.settings.show_code = !self.settings.show_code;
        if !self.settings.show_code {
            // The keys typed next are not for a hidden editor.
            ctx.memory_mut(|m| m.stop_text_input());
        }
    }
}

/// The rows shown, outermost first: each node's path of child indices and
/// depth, the children of folded nodes left out.
fn visible_rows(nodes: &[TreeNode], collapsed: &HashSet<Vec<usize>>) -> Vec<(Vec<usize>, usize)> {
    fn walk(
        nodes: &[TreeNode],
        path: &mut Vec<usize>,
        collapsed: &HashSet<Vec<usize>>,
        out: &mut Vec<(Vec<usize>, usize)>,
    ) {
        for (i, node) in nodes.iter().enumerate() {
            path.push(i);
            out.push((path.clone(), path.len() - 1));
            if !collapsed.contains(path) {
                walk(&node.children, path, collapsed, out);
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(nodes, &mut Vec::new(), collapsed, &mut out);
    out
}

fn node_at<'a>(nodes: &'a [TreeNode], path: &[usize]) -> &'a TreeNode {
    let (first, rest) = path.split_first().expect("a path has a node");
    rest.iter().fold(&nodes[*first], |node, &i| &node.children[i])
}

/// The path of the node whose statement is `range`.
fn path_of(nodes: &[TreeNode], range: &Range<usize>) -> Option<Vec<usize>> {
    for (i, node) in nodes.iter().enumerate() {
        if node.range == *range {
            return Some(vec![i]);
        }
        if node.range.start <= range.start && range.end <= node.range.end {
            if let Some(mut path) = path_of(&node.children, range) {
                path.insert(0, i);
                return Some(path);
            }
        }
    }
    None
}

/// Whether an object can go inside `node`: an operation, a transform, or a
/// module call with children (not an `if`, whose branches are not one
/// block).
fn accepts_children(node: &TreeNode) -> bool {
    node.call != "if" && (matches!(node.kind, NodeKind::Boolean | NodeKind::Transform) || !node.children.is_empty())
}

/// Where an object dropped on `node`'s row goes, the pointer at `fraction`
/// of the row's height: into the node when over its middle and it takes
/// children, else before or after it.
fn drop_target(node: &TreeNode, fraction: f32) -> Target {
    let range = node.range.clone();
    if accepts_children(node) && (0.25..0.75).contains(&fraction) {
        Target::IntoBlock(range)
    } else if fraction < 0.5 {
        Target::Before(range)
    } else {
        Target::After(range)
    }
}

/// Whether the object at `dragged` can go to `target`: not into itself nor
/// next to what it holds.
fn valid_drop(nodes: &[TreeNode], dragged: &Range<usize>, target: &Target) -> bool {
    let (Target::Before(t) | Target::After(t) | Target::IntoBlock(t)) = target;
    path_of(nodes, t).is_some() && !(dragged.start <= t.start && t.end <= dragged.end)
}

fn kind_color(kind: NodeKind) -> Color32 {
    match kind {
        NodeKind::Boolean => theme::CODE_KEYWORD,
        NodeKind::Transform => theme::CODE_FUNCTION,
        NodeKind::Primitive => theme::CODE_MODULE,
        NodeKind::Module => theme::CODE_DEFINITION,
        NodeKind::Control => theme::CODE_NUMBER,
        NodeKind::Other => theme::TEXT_DIM,
    }
}

/// A small drawing of what a node is: overlapping circles for a boolean,
/// crossed arrows for a transform, a cube for a shape, a hexagon for a
/// module call, a loop for control flow and a dot for the rest.
fn paint_icon(painter: &egui::Painter, rect: Rect, kind: NodeKind, color: Color32) {
    let c = rect.center();
    let r = rect.width() / 2.0;
    let stroke = Stroke::new(1.3, color);
    match kind {
        NodeKind::Boolean => {
            painter.circle_stroke(c - Vec2::new(r * 0.35, 0.0), r * 0.6, stroke);
            painter.circle_stroke(c + Vec2::new(r * 0.35, 0.0), r * 0.6, stroke);
        }
        NodeKind::Transform => {
            for (a, b) in [(Vec2::new(-r, 0.0), Vec2::new(r, 0.0)), (Vec2::new(0.0, -r), Vec2::new(0.0, r))] {
                painter.line_segment([c + a, c + b], stroke);
            }
            for d in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                let tip = c + d * r;
                let side = Vec2::new(-d.y, d.x) * r * 0.3;
                painter.line_segment([tip, tip - d * r * 0.3 + side], stroke);
                painter.line_segment([tip, tip - d * r * 0.3 - side], stroke);
            }
        }
        NodeKind::Primitive => {
            // A cube: a front square and the top and side behind it.
            let s = r * 1.3;
            let o = r * 0.55;
            let front = Rect::from_min_size(Pos2::new(c.x - r, c.y - r + o), Vec2::splat(s));
            painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Middle);
            let back = front.translate(Vec2::new(o, -o));
            painter.line_segment([front.left_top(), back.left_top()], stroke);
            painter.line_segment([front.right_top(), back.right_top()], stroke);
            painter.line_segment([front.right_bottom(), back.right_bottom()], stroke);
            painter.line_segment([back.left_top(), back.right_top()], stroke);
            painter.line_segment([back.right_top(), back.right_bottom()], stroke);
        }
        NodeKind::Module => {
            let points = (0..6)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        NodeKind::Control => {
            let points: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = 0.6 + 5.0 * i as f32 / 20.0;
                    c + Vec2::new(a.cos(), a.sin()) * r * 0.85
                })
                .collect();
            let tip = *points.last().expect("points");
            painter.add(egui::Shape::line(points, stroke));
            painter.circle_filled(tip, 1.6, color);
        }
        NodeKind::Other => {
            painter.circle_filled(c, r * 0.35, color);
        }
    }
}

/// The fold arrow: pointing right when folded, down when open.
fn paint_arrow(painter: &egui::Painter, c: Pos2, folded: bool, color: Color32) {
    let points = if folded {
        vec![c + Vec2::new(-2.0, -4.0), c + Vec2::new(3.0, 0.0), c + Vec2::new(-2.0, 4.0)]
    } else {
        vec![c + Vec2::new(-4.0, -2.0), c + Vec2::new(4.0, -2.0), c + Vec2::new(0.0, 3.0)]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// An eye, open while the object shows, struck through once hidden.
fn paint_eye(painter: &egui::Painter, c: Pos2, open: bool, color: Color32) {
    let stroke = Stroke::new(1.2, color);
    let (w, h) = (6.5, 3.6);
    let lid = |sign: f32| -> Vec<Pos2> {
        (0..=12)
            .map(|i| {
                let t = i as f32 / 12.0 * std::f32::consts::PI;
                c + Vec2::new(-w * t.cos(), sign * h * t.sin())
            })
            .collect()
    };
    painter.add(egui::Shape::line(lid(-1.0), stroke));
    painter.add(egui::Shape::line(lid(1.0), stroke));
    if open {
        painter.circle_filled(c, 1.8, color);
    } else {
        painter.line_segment([c + Vec2::new(-w, h + 1.0), c + Vec2::new(w, -h - 1.0)], stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "\
difference() { // Shell
    cube(10, center = true);
    translate([0, 0, 2]) cylinder(h = 20, r = 3); // Hole
}
sphere(2); // Ball
";

    fn at(src: &str, needle: &str) -> Range<usize> {
        let start = src.find(needle).expect("in the source");
        start..start + needle.len()
    }

    #[test]
    fn rows_follow_the_tree_and_its_folds() {
        let nodes = code_edit::tree(MODEL).unwrap();
        let names = |rows: &[(Vec<usize>, usize)]| -> Vec<(String, usize)> {
            rows.iter().map(|(p, d)| (node_at(&nodes, p).name.clone(), *d)).collect()
        };
        let rows = visible_rows(&nodes, &HashSet::new());
        let expected = [("Shell", 0), ("cube", 1), ("Hole", 1), ("cylinder", 2), ("Ball", 0)];
        assert_eq!(names(&rows), expected.map(|(n, d)| (n.to_string(), d)));
        let folded = HashSet::from([vec![0]]);
        assert_eq!(names(&visible_rows(&nodes, &folded)), [("Shell".to_string(), 0), ("Ball".to_string(), 0)]);
    }

    #[test]
    fn the_views_selection_finds_its_row() {
        let nodes = code_edit::tree(MODEL).unwrap();
        let cylinder = at(MODEL, "cylinder(h = 20, r = 3);");
        assert_eq!(path_of(&nodes, &cylinder), Some(vec![0, 1, 0]));
        let hole = at(MODEL, "translate([0, 0, 2]) cylinder(h = 20, r = 3);");
        assert_eq!(path_of(&nodes, &hole), Some(vec![0, 1]));
        assert_eq!(node_at(&nodes, &[0, 1]).range, hole);
        // Bytes that are no statement have no row.
        assert_eq!(path_of(&nodes, &(0..3)), None);
    }

    #[test]
    fn the_tree_is_read_again_only_when_the_code_changes() {
        let mut state = TreeState::default();
        assert!(state.refresh(1, MODEL));
        assert!(!state.refresh(1, MODEL), "cached");
        assert_eq!(state.nodes.len(), 2);
        // Code that does not read keeps the last tree, marked stale.
        assert!(state.refresh(1, "cube(1);\nsphere("));
        assert!(state.stale());
        assert_eq!(state.nodes.len(), 2);
        state.start_rename(&state.nodes[1].range.clone());
        assert!(state.renaming.is_none(), "no renaming a stale tree");
        assert!(state.refresh(1, "cube(1);"));
        assert!(!state.stale());
        assert_eq!(state.nodes.len(), 1);
        // Another tab's code starts afresh.
        state.collapsed.insert(vec![0]);
        assert!(state.refresh(2, "cube(1);"));
        assert!(state.collapsed.is_empty());
    }

    #[test]
    fn drops_go_before_after_or_into() {
        let nodes = code_edit::tree(MODEL).unwrap();
        let shell = &nodes[0];
        let ball = &nodes[1];
        assert_eq!(drop_target(shell, 0.1), Target::Before(shell.range.clone()));
        assert_eq!(drop_target(shell, 0.5), Target::IntoBlock(shell.range.clone()));
        assert_eq!(drop_target(shell, 0.9), Target::After(shell.range.clone()));
        // A shape takes no children: its halves.
        assert_eq!(drop_target(ball, 0.4), Target::Before(ball.range.clone()));
        assert_eq!(drop_target(ball, 0.6), Target::After(ball.range.clone()));
        // Not into itself, nor next to what it holds.
        let cube = &shell.children[0];
        assert!(valid_drop(&nodes, &ball.range, &Target::IntoBlock(shell.range.clone())));
        assert!(!valid_drop(&nodes, &shell.range, &Target::IntoBlock(shell.range.clone())));
        assert!(!valid_drop(&nodes, &shell.range, &Target::Before(cube.range.clone())));
        assert!(valid_drop(&nodes, &cube.range, &Target::After(ball.range.clone())));
    }

    #[test]
    fn a_drop_into_the_difference_is_the_same_edit_as_by_hand() {
        let nodes = code_edit::tree(MODEL).unwrap();
        let target = drop_target(&nodes[0], 0.5);
        let edit = code_edit::move_into(MODEL, nodes[1].range.clone(), target).unwrap();
        let moved = code_edit::tree(&edit.text).unwrap();
        assert_eq!(moved.len(), 1);
        let names: Vec<_> = moved[0].children.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["cube", "Hole", "Ball"]);
        assert_eq!(&edit.text[edit.selection], "sphere(2);");
    }

    /// Runs one frame of the tree alone in a 300 by 400 window, with
    /// `events`; returns what a row asked.
    fn frame(
        ctx: &egui::Context,
        state: &mut TreeState,
        events: Vec<egui::Event>,
        selected: &[Range<usize>],
    ) -> Option<Action> {
        let time = ctx.input(|i| i.time) + 0.25;
        let input = egui::RawInput {
            events,
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 400.0))),
            ..Default::default()
        };
        let mut action = None;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(Frame::new()).show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                action = state.show(ui, selected);
            });
        });
        action
    }

    /// A press and release at `at`, over three frames.
    fn tap(ctx: &egui::Context, state: &mut TreeState, at: Pos2) -> Vec<Action> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]]
            .into_iter()
            .filter_map(|events| frame(ctx, state, events, &[]))
            .collect()
    }

    /// The middle of row `i`, `x` from the left.
    fn row(i: usize, x: f32) -> Pos2 {
        Pos2::new(x, i as f32 * (ROW + 1.0) + ROW / 2.0)
    }

    #[test]
    fn rows_answer_clicks() {
        let ctx = egui::Context::default();
        let mut state = TreeState::default();
        state.refresh(1, MODEL);
        frame(&ctx, &mut state, vec![], &[]);
        let nodes = code_edit::tree(MODEL).unwrap();
        // A click on the name selects; the row is remembered for F2.
        let hole = nodes[0].children[1].range.clone();
        assert_eq!(tap(&ctx, &mut state, row(2, 80.0)), [Action::Select { range: hole.clone(), add: false }]);
        assert_eq!(state.focus, Some(hole));
        // The eye at the right end hides the object.
        let ball = nodes[1].range.clone();
        let eye = row(4, 300.0 - 4.0 - TOGGLE / 2.0);
        assert_eq!(
            tap(&ctx, &mut state, eye),
            [Action::SetModifier { range: ball.clone(), modifier: Modifier::Disable, on: true }]
        );
        // The fold arrow folds the difference: two rows left.
        tap(&ctx, &mut state, row(0, 4.0 + 6.0));
        assert!(state.collapsed.contains(&vec![0]));
        // Renaming: a typed name and Enter.
        state.start_rename(&ball);
        frame(&ctx, &mut state, vec![], &[]);
        let typing = vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("Pearl".into()),
        ];
        frame(&ctx, &mut state, typing, &[]);
        let enter = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(
            frame(&ctx, &mut state, vec![enter], &[]),
            Some(Action::Rename { range: ball, name: "Pearl".into() })
        );
        assert!(state.renaming.is_none());
    }

    /// A click on another row, pressed and released within one frame,
    /// ends the field and selects in that frame: the name typed is what is
    /// kept.
    #[test]
    fn a_click_elsewhere_keeps_the_name_typed() {
        let ctx = egui::Context::default();
        let mut state = TreeState::default();
        state.refresh(1, MODEL);
        frame(&ctx, &mut state, vec![], &[]);
        let ball = code_edit::tree(MODEL).unwrap()[1].range.clone();
        state.start_rename(&ball);
        frame(&ctx, &mut state, vec![], &[]);
        let select_all = egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame(&ctx, &mut state, vec![select_all, egui::Event::Text("Pearl".into())], &[]);
        // The press and the release in one frame.
        let at = row(0, 80.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let events = vec![egui::Event::PointerMoved(at), button(true), button(false)];
        let action = frame(&ctx, &mut state, events, &[]);
        assert_eq!(action, Some(Action::Rename { range: ball, name: "Pearl".into() }));
        assert!(state.renaming.is_none());
    }

    #[test]
    fn a_drag_onto_the_difference_moves_into_it() {
        let ctx = egui::Context::default();
        let mut state = TreeState::default();
        state.refresh(1, MODEL);
        let nodes = code_edit::tree(MODEL).unwrap();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (from, to) = (row(4, 80.0), row(0, 80.0));
        let mut actions = Vec::new();
        let steps = [
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(from + Vec2::new(0.0, -20.0))],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ];
        for events in steps {
            actions.extend(frame(&ctx, &mut state, events, &[]));
        }
        let target = Target::IntoBlock(nodes[0].range.clone());
        assert_eq!(actions, [Action::Move { range: nodes[1].range.clone(), target }]);
    }

    /// The app without a window, settings nor session, on `text`.
    fn studio(ctx: &egui::Context, text: &str) -> StudioApp {
        let stores = super::super::Stores { session: None, settings: None };
        let settings = crate::settings::Settings::load(None);
        let recent = crate::recent::RecentFiles::default();
        let mut app = StudioApp::with(ctx, None, stores, recent, settings, Vec::new());
        app.doc.text = text.to_string();
        app
    }

    #[test]
    fn tree_actions_are_the_edits_made_by_hand() {
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, MODEL);
        let nodes = code_edit::tree(MODEL).unwrap();
        let ball = nodes[1].range.clone();
        app.tree_action(&ctx, Action::SetModifier { range: ball.clone(), modifier: Modifier::Disable, on: true });
        assert_eq!(app.doc.text, MODEL.replace("sphere(2);", "*sphere(2);"));
        app.tree_action(&ctx, Action::Rename { range: ball.start..ball.end + 1, name: "Pearl".into() });
        assert_eq!(app.doc.text, MODEL.replace("sphere(2); // Ball", "*sphere(2); // Pearl"));
        let ball = ball.start..ball.end + 1;
        app.tree_action(&ctx, Action::Move { range: ball, target: Target::Before(nodes[0].range.clone()) });
        assert!(app.doc.text.starts_with("*sphere(2); // Pearl\ndifference() { // Shell\n"), "{}", app.doc.text);
        // One undo step each.
        assert!(app.doc.editor.undo_step(&ctx, &mut app.doc.text, true));
        assert_eq!(app.doc.text, MODEL.replace("sphere(2); // Ball", "*sphere(2); // Pearl"));
    }

    #[test]
    fn a_row_selects_its_object_in_the_view() {
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, MODEL);
        app.render_now();
        let start = std::time::Instant::now();
        while app.doc.viewer.scene_source() != MODEL || app.doc.viewer.picking().is_none() {
            assert!(start.elapsed().as_secs() < 30, "no render");
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.receive();
        }
        let nodes = code_edit::tree(MODEL).unwrap();
        let (cube, ball) = (nodes[0].children[0].range.clone(), nodes[1].range.clone());
        app.tree_action(&ctx, Action::Select { range: cube.clone(), add: false });
        assert_eq!(app.selected_ranges(), Some(vec![cube.clone()]));
        app.tree_action(&ctx, Action::Select { range: ball.clone(), add: true });
        assert_eq!(app.selected_ranges(), Some(vec![cube.clone(), ball]));
        app.tree_action(&ctx, Action::Select { range: cube, add: false });
        assert_eq!(app.selected_ranges().map(|r| r.len()), Some(1));
    }

    #[test]
    fn the_code_free_mode_hides_only_the_editor() {
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, MODEL);
        let shown =
            |ctx: &egui::Context, id: &str| egui::containers::panel::PanelState::load(ctx, egui::Id::new(id)).is_some();
        let run = |app: &mut StudioApp| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                app.shortcuts(ctx);
                app.editor_panel(ctx);
                app.tree_panel(ctx);
            });
        };
        assert!(app.settings.show_code && app.settings.show_tree);
        app.toggle_code(&ctx);
        assert!(!app.settings.show_code);
        run(&mut app);
        assert!(!shown(&ctx, "editor"));
        assert!(shown(&ctx, "tree"));
        // Cmd+E brings the code back.
        let key = egui::Event::Key {
            key: Key::E,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let input = egui::RawInput { events: vec![key], ..Default::default() };
        let _ = ctx.run(input, |ctx| app.shortcuts(ctx));
        assert!(app.settings.show_code);
        run(&mut app);
        assert!(shown(&ctx, "editor"));
    }
}
