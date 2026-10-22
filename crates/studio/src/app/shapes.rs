//! Making a model without code (C2F-40): shapes put down from a toolbar,
//! booleans of the selection, holes, copies and deletions.
//!
//! The CREATE group of the modelling toolbar arms a shape: the next
//! click puts it on the build plate, or on the face under the pointer with
//! its own Z axis along the face's normal, and the app writes its call at
//! the end of the code (inside the boolean selected, when one is). With
//! "Hole" on, the shape is cut out of the object clicked instead. Two
//! objects selected or more can be united, subtracted (the first minus the
//! others, in the order they were clicked) or intersected; any selection
//! can be duplicated, copied, cut, pasted and deleted, with the keys of a
//! text editor while no text field has the keyboard.
//!
//! Every action is one edit of [`crate::code_edit`], applied as one undo
//! step by [`StudioApp::apply_code_edit`], which selects what it created.
//!
//! What the toolbar keeps between frames (the shape armed, the internal
//! clipboard, the selection as the last frame left it) lives in egui's
//! memory rather than in the app, as nothing else needs it.

use std::ops::Range;

use eframe::egui::{
    self, Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, Stroke,
};
use glam::Vec3;

use super::StudioApp;
use super::toolbar::{self, Icon, Row};
use super::workspace::CodeTab;
use crate::code_edit::{self, Boolean, Edit, Place, Primitive};
use crate::pick::Ray;
use crate::select::Statement;
use crate::settings::Workspace;
use crate::theme;

/// The toolbar's shapes: what each makes, its label and its tooltip.
const SHAPES: [(Primitive, &str, &str); 6] = [
    (Primitive::Cube, "Cube", "A 10 mm cube"),
    (Primitive::Cylinder, "Cylinder", "A cylinder 10 mm across and 10 mm high"),
    (Primitive::Sphere, "Sphere", "A sphere 10 mm across"),
    (Primitive::Cone, "Cone", "A cone 10 mm across and 10 mm high"),
    (Primitive::Tube, "Tube", "A tube 10 mm across, 6 mm inside"),
    (Primitive::Text, "Text", "Text 10 mm high, 2 mm thick"),
];

/// The booleans a click of the toolbar applies to the selection.
const BOOLEANS: [(Boolean, &str, &str); 3] = [
    (Boolean::Union, "Unite", "Join the selected objects into one"),
    (Boolean::Difference, "Subtract", "Cut the other selected objects out of the first one clicked"),
    (Boolean::Intersection, "Intersect", "Keep only what the selected objects share"),
];

/// The calls a new shape goes inside of when one is selected: they do not
/// move what they hold, so the shape lands where it was put.
const BLOCKS: [&str; 5] = ["union", "difference", "intersection", "hull", "minkowski"];

/// A step of the plate's grid that points snap to, and the finer one on
/// faces, in millimetres.
const PLATE_STEP: f64 = 1.0;
const FACE_STEP: f64 = 0.1;

/// How far above the face a hole starts, so that its top is not flush with
/// the face it cuts (coplanar faces render badly), in millimetres.
const HOLE_LEAD: f64 = 1.0;

const READ_ONLY: &str = "This shape comes from a library or an included file: it is read-only here";
const STALE: &str = "The code changed since the last render: wait for the model to update, then try again";

/// The shape a click puts down, if any, and whether it makes a hole.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tool {
    armed: Option<Primitive>,
    hole: bool,
}

impl Tool {
    /// A shape's button: arms it, or disarms it when it is armed already.
    fn pick(&mut self, kind: Primitive) {
        if self.armed == Some(kind) {
            self.disarm();
        } else {
            self.armed = Some(kind);
        }
    }

    /// The Hole button: turns hole mode on or off, arming a cylinder (the
    /// usual hole) when nothing is armed.
    fn toggle_hole(&mut self) {
        self.hole = !self.hole;
        if self.hole && self.armed.is_none() {
            self.armed = Some(Primitive::Cylinder);
        }
    }

    /// Esc, or a shape put down without Shift: back to selecting.
    fn disarm(&mut self) {
        *self = Tool::default();
    }

    /// A shape was put down; Shift (`keep`) keeps the tool for the next.
    fn placed(&mut self, keep: bool) {
        if !keep {
            self.disarm();
        }
    }
}

/// What the toolbar keeps between frames.
#[derive(Clone, Debug, Default)]
struct Shapes {
    tool: Tool,
    /// The objects last copied, as [`code_edit::copy_text`] wrote them.
    clipboard: Option<String>,
    /// The selection as the last frame left it, and the tab it was in: the
    /// click that puts a shape down has selected in the view by the time
    /// the toolbar sees it, and the shape goes into what was selected
    /// before.
    selection: (u64, Vec<Statement>),
}

impl Shapes {
    fn id() -> Id {
        Id::new("shapes-toolbar")
    }

    fn load(ctx: &egui::Context) -> Shapes {
        ctx.data(|d| d.get_temp(Shapes::id())).unwrap_or_default()
    }

    fn store(self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(Shapes::id(), self));
    }
}

/// Where a click puts a shape: the point, the outward normal of the face
/// there (`None` on the plate, which faces up) and the statement that made
/// the face.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Landing {
    at: [f64; 3],
    normal: Option<[f64; 3]>,
    target: Option<Statement>,
}

/// Where a click lands: on the face of the model it hits (`surface`, as
/// [`crate::viewer::Viewer::surface_at`] gives it), else where `ray` meets
/// the plate, snapped to its grid; `None` when it misses both.
fn landing(surface: Option<(Vec3, Vec3, Option<Statement>)>, ray: &Ray) -> Option<Landing> {
    match surface {
        Some((point, normal, target)) => {
            let at = point.to_array().map(|x| snap(f64::from(x), FACE_STEP));
            let normal = normal.normalize_or(Vec3::Z).to_array().map(|x| snap(f64::from(x), 1e-3));
            Some(Landing { at, normal: Some(normal), target })
        }
        None => {
            let [x, y] = crate::catalogue::on_ground(ray)?;
            Some(Landing { at: [snap(x, PLATE_STEP), snap(y, PLATE_STEP), 0.0], normal: None, target: None })
        }
    }
}

/// `x` to the nearest multiple of `step`, without `-0`.
fn snap(x: f64, step: f64) -> f64 {
    let x = (x / step).round() * step;
    if x == 0.0 { 0.0 } else { x }
}

/// The code of the shape a click puts down: standing on the face, or for a
/// hole, sunk into it from a little above, growing against the normal.
fn shape_code(kind: Primitive, landing: &Landing, hole: bool) -> String {
    if !hole {
        return code_edit::primitive_code(kind, landing.at, landing.normal);
    }
    let n = landing.normal.unwrap_or([0.0, 0.0, 1.0]);
    let start = [0, 1, 2].map(|i| landing.at[i] + n[i] * HOLE_LEAD);
    // `0.0 - x` rather than `-x`: a `-0` would turn the shape by -180° for 180°.
    code_edit::primitive_code(kind, start, Some(n.map(|x| 0.0 - x)))
}

/// Where a new shape goes: inside the boolean selected when the selection
/// is one boolean with its children in braces, else at the end of the file.
fn place_for(src: &str, selection: &[Range<usize>]) -> Place {
    if let [range] = selection {
        if let Ok(call) = code_edit::call_at(src, range.clone()) {
            if call.block.is_some() && BLOCKS.contains(&call.name.as_str()) {
                return Place::InBlock(call.statement);
            }
        }
    }
    Place::End
}

/// The ranges to act on one by one: those not inside another, last first,
/// so that an edit never moves the ones still to do.
fn outermost_last_first(ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let inside = |r: &Range<usize>, o: &Range<usize>| r != o && o.start <= r.start && r.end <= o.end;
    let mut kept: Vec<Range<usize>> = ranges.iter().filter(|r| !ranges.iter().any(|o| inside(r, o))).cloned().collect();
    kept.sort_by_key(|r| std::cmp::Reverse(r.start));
    kept.dedup();
    kept
}

/// `edit` applied to each object at `ranges` (see
/// [`outermost_last_first`]), as one change: the new text, the cursor of
/// the last edit, and what each edit left selected, in the order of the
/// code. Each edit must change the code only around its own object, as
/// deleting and duplicating do.
fn each_object(
    src: &str,
    ranges: &[Range<usize>],
    edit: impl Fn(&str, Range<usize>) -> Result<Edit, String>,
) -> Result<(String, usize, Vec<Range<usize>>), String> {
    let mut text = src.to_string();
    let mut cursor = 0;
    let mut selection: Vec<Range<usize>> = Vec::new();
    for range in outermost_last_first(ranges) {
        let done = edit(&text, range)?;
        // What came after the object moved by what the edit added or removed.
        let grown = done.text.len() as isize - text.len() as isize;
        for r in &mut selection {
            *r = r.start.saturating_add_signed(grown)..r.end.saturating_add_signed(grown);
        }
        selection.insert(0, done.selection);
        cursor = done.cursor;
        text = done.text;
    }
    Ok((text, cursor, selection))
}

/// The keys of the toolbar, read while no text field has the keyboard (the
/// editor's own Delete, Cmd+D and clipboard stay its own). Delete and
/// Cmd+D are consumed, so that no other panel acts on them too.
#[derive(Clone, Debug, Default, PartialEq)]
struct Keys {
    escape: bool,
    delete: bool,
    duplicate: bool,
    copy: bool,
    cut: bool,
    /// The system clipboard's text, for Cmd+V; empty when the platform
    /// gives the key without it.
    paste: Option<String>,
}

fn read_keys(ctx: &egui::Context) -> Keys {
    if ctx.memory(|m| m.focused().is_some()) {
        return Keys::default();
    }
    ctx.input_mut(|i| {
        let mut keys = Keys {
            escape: i.key_pressed(Key::Escape),
            delete: i.consume_key(Modifiers::NONE, Key::Delete) | i.consume_key(Modifiers::NONE, Key::Backspace),
            duplicate: i.consume_key(Modifiers::COMMAND, Key::D),
            ..Keys::default()
        };
        // The windowing layer turns Cmd+C, Cmd+X and Cmd+V into these.
        for event in &i.events {
            match event {
                Event::Copy => keys.copy = true,
                Event::Cut => keys.cut = true,
                Event::Paste(text) => keys.paste = Some(text.clone()),
                _ => {}
            }
        }
        keys.copy |= i.consume_key(Modifiers::COMMAND, Key::C);
        keys.cut |= i.consume_key(Modifiers::COMMAND, Key::X);
        if i.consume_key(Modifiers::COMMAND, Key::V) && keys.paste.is_none() {
            keys.paste = Some(String::new());
        }
        keys
    })
}

/// A click of the toolbar.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Shape(Primitive),
    Hole,
    /// Fillet (true) or Chamfer: the inspector's edges section.
    Edges(bool),
    Combine(Boolean),
}

/// What the right-click menu of an object asks, in the view or the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Duplicate,
    Copy,
    Cut,
    Paste,
    Delete,
    ShowInCode,
}

impl StudioApp {
    /// The CREATE, MODIFY and COMBINE groups of the modelling toolbar.
    pub(super) fn shape_tools(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, row: Row) {
        let mut state = Shapes::load(ctx);
        let selected = self.doc.viewer.selection().len();
        let edges = self.edges_ready(ctx);
        match toolbar(ui, row, state.tool, selected, edges.as_ref().map(|_| ()).map_err(String::as_str)) {
            Some(Action::Shape(kind)) => state.tool.pick(kind),
            Some(Action::Hole) => state.tool.toggle_hole(),
            Some(Action::Edges(fillet)) => self.ask_edges(ctx, fillet),
            Some(Action::Combine(op)) => self.combine(ctx, op),
            None => {}
        }
        state.store(ctx);
    }

    /// Does what an object's right-click menu asked.
    pub(super) fn object_command(&mut self, ctx: &egui::Context, command: Command) {
        let mut state = Shapes::load(ctx);
        match command {
            Command::Duplicate => self.duplicate_selection(ctx),
            Command::Copy => {
                self.copy_selection(ctx, &mut state);
            }
            Command::Cut => {
                if self.copy_selection(ctx, &mut state) {
                    self.delete_selection(ctx);
                }
            }
            Command::Paste => {
                if let Some(copied) = state.clipboard.clone() {
                    self.paste(ctx, &copied, None);
                }
            }
            Command::Delete => self.delete_selection(ctx),
            Command::ShowInCode => self.show_in_code(ctx),
        }
        state.store(ctx);
    }

    /// The Code workspace, its cursor on the object selected last.
    fn show_in_code(&mut self, ctx: &egui::Context) {
        let Some(range) = self.selected_ranges().and_then(|ranges| ranges.last().cloned()) else {
            return;
        };
        self.set_workspace(ctx, Workspace::Code);
        self.panels.code_tab = CodeTab::Code;
        let text = self.doc.text.clone();
        self.doc.editor.go_to(ctx, &text, range.start);
        self.doc.editor.focus(ctx);
    }

    /// Whether the clipboard holds objects copied here, for Paste.
    pub(super) fn can_paste(ctx: &egui::Context) -> bool {
        Shapes::load(ctx).clipboard.is_some()
    }

    /// Placing an armed shape with a click in the view, and the keys that
    /// act on the selection.
    pub(super) fn shapes_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, response: &egui::Response) {
        let mut state = Shapes::load(ctx);
        let rect = response.rect;
        let keys = read_keys(ctx);
        let selected = self.doc.viewer.selection().len();
        if keys.escape {
            state.tool.disarm();
        }

        if let Some(kind) = state.tool.armed {
            let viewer = &self.doc.viewer;
            let at = |pos: Pos2| landing(viewer.surface_at(rect, pos), &viewer.ray(rect, pos));
            if let Some(pos) = response.hover_pos().filter(|_| !response.dragged()) {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                if let Some(landing) = at(pos) {
                    self.paint_landing(&ui.painter_at(rect), rect, &landing, state.tool.hole);
                }
            }
            if response.clicked_by(PointerButton::Primary) {
                if let Some(landing) = response.interact_pointer_pos().and_then(at) {
                    let before = if state.selection.0 == self.doc.id { state.selection.1.clone() } else { Vec::new() };
                    self.put_shape(ctx, kind, state.tool.hole, &landing, &before);
                    state.tool.placed(ctx.input(|i| i.modifiers.shift));
                }
            }
        }

        if selected > 0 {
            if keys.delete {
                self.delete_selection(ctx);
            } else if keys.duplicate {
                self.duplicate_selection(ctx);
            } else if keys.cut {
                if self.copy_selection(ctx, &mut state) {
                    self.delete_selection(ctx);
                }
            } else if keys.copy {
                self.copy_selection(ctx, &mut state);
            }
        }
        if let Some(text) = keys.paste {
            self.paste(ctx, &text, state.clipboard.as_deref());
        }

        if let Some(kind) = state.tool.armed {
            let name = SHAPES.iter().find(|s| s.0 == kind).map_or("Shape", |s| s.1);
            let hint = if state.tool.hole {
                format!("{name} hole: click on a face of an object · Shift-click: another · Esc: cancel")
            } else {
                format!("{name}: click on the plate or on a face · Shift-click: another · Esc: cancel")
            };
            super::sketch::hint_bar(&ui.painter_at(rect), rect, &hint);
        }

        // Right-click: the menu of the selection, which the click joined.
        let mut command = None;
        let can_paste = state.clipboard.is_some();
        response.context_menu(|ui| command = object_menu(ui, selected, can_paste));

        state.selection = (self.doc.id, self.doc.viewer.selection().to_vec());
        state.store(ctx);
        if let Some(command) = command {
            self.object_command(ctx, command);
        }
    }

    /// Where an armed shape would land: a dot on the point, and a stroke
    /// along the way it grows.
    fn paint_landing(&self, painter: &egui::Painter, rect: Rect, landing: &Landing, hole: bool) {
        let point = Vec3::from_array(landing.at.map(|x| x as f32));
        let normal = Vec3::from_array(landing.normal.unwrap_or([0.0, 0.0, 1.0]).map(|x| x as f32));
        let growth = if hole { -normal } else { normal };
        let color = if hole { theme::palette().warning } else { theme::palette().accent_strong };
        let viewer = &self.doc.viewer;
        let Some(at) = viewer.to_screen(rect, point) else {
            return;
        };
        if let Some(tip) = viewer.to_screen(rect, point + growth * 10.0) {
            painter.line_segment([at, tip], Stroke::new(2.0, color));
        }
        painter.circle(at, 5.0, color.gamma_multiply(0.35), Stroke::new(1.5, color));
    }

    /// Writes the shape a click put down: cut out of the object clicked in
    /// hole mode, else at the end of the code or inside the boolean that
    /// was selected (`before`).
    fn put_shape(&mut self, ctx: &egui::Context, kind: Primitive, hole: bool, landing: &Landing, before: &[Statement]) {
        let code = shape_code(kind, landing, hole);
        let current = self.doc.viewer.scene_source() == self.doc.text;
        let name = SHAPES.iter().find(|s| s.0 == kind).map_or("shape", |s| s.1).to_lowercase();
        if hole {
            let result = match landing.target {
                None => Err("Click on a face of an object to cut a hole in it".to_string()),
                Some(target) if !target.editable() => Err(READ_ONLY.to_string()),
                Some(_) if !current => Err(STALE.to_string()),
                Some(target) => code_edit::make_hole(&self.doc.text, target.span.range(), &code),
            };
            self.apply_code_edit(ctx, result, &format!("Cut a {name} hole"));
            return;
        }
        let ranges: Vec<Range<usize>> = before.iter().filter_map(|k| k.range()).collect();
        let place = if current { place_for(&self.doc.text, &ranges) } else { Place::End };
        let result = code_edit::insert_shape(&self.doc.text, &place, &code);
        self.apply_code_edit(ctx, result, &format!("Added a {name}"));
    }

    /// The selected objects' ranges, or why they cannot be edited.
    fn editable_selection(&self) -> Result<Vec<Range<usize>>, String> {
        let selection = self.doc.viewer.selection();
        if selection.is_empty() {
            return Err("Select an object first".into());
        }
        if selection.iter().any(|k| !k.editable()) {
            return Err(READ_ONLY.into());
        }
        self.selected_ranges().ok_or_else(|| STALE.to_string())
    }

    /// Unites, subtracts or intersects the selection, in the order it was
    /// clicked.
    fn combine(&mut self, ctx: &egui::Context, op: Boolean) {
        let result = self.editable_selection().and_then(|ranges| {
            if ranges.len() < 2 {
                return Err("Select two objects or more (Shift-click)".into());
            }
            code_edit::wrap_boolean(&self.doc.text, &ranges, op)
        });
        let done = match op {
            Boolean::Union => "United the selection",
            Boolean::Difference => "Subtracted the others from the first object",
            Boolean::Intersection => "Intersected the selection",
        };
        self.apply_code_edit(ctx, result, done);
    }

    fn delete_selection(&mut self, ctx: &egui::Context) {
        let result = self
            .editable_selection()
            .and_then(|ranges| each_object(&self.doc.text, &ranges, code_edit::delete))
            .map(|(text, cursor, _)| (text, cursor, Vec::new()));
        self.apply_code_edits(ctx, result, "");
    }

    fn duplicate_selection(&mut self, ctx: &egui::Context) {
        let result =
            self.editable_selection().and_then(|ranges| each_object(&self.doc.text, &ranges, code_edit::duplicate));
        self.apply_code_edits(ctx, result, "");
    }

    /// Copies the selected objects with their transforms, to the toolbar's
    /// clipboard and the system's; returns whether it did.
    fn copy_selection(&mut self, ctx: &egui::Context, state: &mut Shapes) -> bool {
        let result = self.editable_selection().and_then(|mut ranges| {
            ranges.sort_by_key(|r| r.start);
            let texts =
                ranges.into_iter().map(|r| code_edit::copy_text(&self.doc.text, r)).collect::<Result<Vec<_>, _>>()?;
            Ok(texts.join("\n"))
        });
        match result {
            Ok(text) => {
                ctx.copy_text(text.clone());
                state.clipboard = Some(text);
                true
            }
            Err(why) => {
                self.doc.console.log(crate::console::Level::Warning, why);
                false
            }
        }
    }

    /// Pastes the system clipboard's `text` at the end of the code when it
    /// is code to paste, else the objects last copied here.
    fn paste(&mut self, ctx: &egui::Context, text: &str, copied: Option<&str>) {
        let src = &self.doc.text;
        let result = match (code_edit::paste(src, &Place::End, text), copied) {
            (Err(_), Some(copied)) => code_edit::paste(src, &Place::End, copied),
            (Err(_), None) if text.trim().is_empty() => return,
            (result, _) => result,
        };
        self.apply_code_edit(ctx, result, "");
    }
}

/// The CREATE, MODIFY and COMBINE groups: the shapes, Hole, Fillet and
/// Chamfer, and the booleans. What does not apply is greyed out, with why
/// on hover, rather than hidden. Returns what was clicked.
fn toolbar(ui: &mut egui::Ui, row: Row, armed: Tool, selected: usize, edges: Result<(), &str>) -> Option<Action> {
    let mut action = None;
    toolbar::group(ui, "CREATE", row, |ui| {
        for (kind, label, tip) in SHAPES {
            let on = armed.armed == Some(kind) && !armed.hole;
            if toolbar::tool(ui, row, shape_icon(kind), label, on, Ok(()), tip).clicked() {
                action = Some(Action::Shape(kind));
            }
        }
    });
    toolbar::separator(ui);
    toolbar::group(ui, "MODIFY", row, |ui| {
        let tip = "Cut the next shape out of the object it is put on";
        if toolbar::tool(ui, row, Icon::Hole, "Hole", armed.hole, Ok(()), tip).clicked() {
            action = Some(Action::Hole);
        }
        let tip = "Round the edges of the selected cube or cylinder";
        if toolbar::tool(ui, row, Icon::Fillet, "Fillet", false, edges, tip).clicked() {
            action = Some(Action::Edges(true));
        }
        let tip = "Bevel the edges of the selected cube or cylinder";
        if toolbar::tool(ui, row, Icon::Chamfer, "Chamfer", false, edges, tip).clicked() {
            action = Some(Action::Edges(false));
        }
    });
    toolbar::separator(ui);
    toolbar::group(ui, "COMBINE", row, |ui| {
        let enabled = if selected >= 2 { Ok(()) } else { Err("Shift-click to select two objects or more") };
        for (op, label, tip) in BOOLEANS {
            let icon = match op {
                Boolean::Union => Icon::Unite,
                Boolean::Difference => Icon::Subtract,
                Boolean::Intersection => Icon::Intersect,
            };
            if toolbar::tool(ui, row, icon, label, false, enabled, tip).clicked() {
                action = Some(Action::Combine(op));
            }
        }
    });
    action
}

fn shape_icon(kind: Primitive) -> Icon {
    match kind {
        Primitive::Cube => Icon::Cube,
        Primitive::Cylinder => Icon::Cylinder,
        Primitive::Sphere => Icon::Sphere,
        Primitive::Cone => Icon::Cone,
        Primitive::Tube => Icon::Tube,
        Primitive::Text => Icon::Text,
    }
}

/// The right-click menu of the selection, in the view or the tree: the
/// entry clicked.
pub(super) fn object_menu(ui: &mut egui::Ui, selected: usize, can_paste: bool) -> Option<Command> {
    let ctx = ui.ctx().clone();
    let keys = |modifiers: Modifiers, key: Key| ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key));
    let some = selected > 0;
    let mut command = None;
    let mut entry = |ui: &mut egui::Ui, enabled: bool, label: &str, keys: String, chosen: Command| {
        if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys)).clicked() {
            command = Some(chosen);
            ui.close();
        }
    };
    entry(ui, some, "Duplicate", keys(Modifiers::COMMAND, Key::D), Command::Duplicate);
    entry(ui, some, "Copy", keys(Modifiers::COMMAND, Key::C), Command::Copy);
    entry(ui, some, "Cut", keys(Modifiers::COMMAND, Key::X), Command::Cut);
    entry(ui, can_paste, "Paste", keys(Modifiers::COMMAND, Key::V), Command::Paste);
    ui.separator();
    entry(ui, some, "Delete", keys(Modifiers::NONE, Key::Backspace), Command::Delete);
    ui.separator();
    entry(ui, selected == 1, "Show in Code", String::new(), Command::ShowInCode);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_arms_disarms_and_stays_with_shift() {
        let mut tool = Tool::default();
        tool.pick(Primitive::Cube);
        assert_eq!(tool, Tool { armed: Some(Primitive::Cube), hole: false });
        // Another shape takes over; the same one again disarms.
        tool.pick(Primitive::Sphere);
        assert_eq!(tool.armed, Some(Primitive::Sphere));
        tool.pick(Primitive::Sphere);
        assert_eq!(tool, Tool::default());
        // Hole alone arms a cylinder; Shift keeps it for the next click.
        tool.toggle_hole();
        assert_eq!(tool, Tool { armed: Some(Primitive::Cylinder), hole: true });
        tool.placed(true);
        assert_eq!(tool, Tool { armed: Some(Primitive::Cylinder), hole: true });
        tool.placed(false);
        assert_eq!(tool, Tool::default());
        tool.pick(Primitive::Cube);
        tool.toggle_hole();
        tool.disarm();
        assert_eq!(tool, Tool::default());
    }

    #[test]
    fn a_click_lands_on_the_plate_or_on_a_face() {
        // Looking down at the plate from above (12.4, -3.6).
        let ray = Ray { origin: Vec3::new(12.4, -3.6, 50.0), direction: Vec3::new(0.0, 0.0, -1.0), slack: 0.0 };
        assert_eq!(landing(None, &ray), Some(Landing { at: [12.0, -4.0, 0.0], normal: None, target: None }));
        // Looking up, or along the plate: it misses.
        let up = Ray { direction: Vec3::Z, ..ray };
        assert_eq!(landing(None, &up), None);
        let level = Ray { origin: Vec3::new(0.0, 0.0, 5.0), direction: Vec3::X, slack: 0.0 };
        assert_eq!(landing(None, &level), None);
        // On a face: its point to a tenth, its normal, its statement.
        let key = Statement { unit: 0, span: c2f_lang::Span::new(0, 9) };
        let hit = Some((Vec3::new(10.0, 3.04, 4.96), Vec3::new(1.0, 0.0, 0.0), Some(key)));
        assert_eq!(
            landing(hit, &ray),
            Some(Landing { at: [10.0, 3.0, 5.0], normal: Some([1.0, 0.0, 0.0]), target: Some(key) })
        );
    }

    #[test]
    fn shapes_stand_on_their_face_and_holes_sink_into_it() {
        let top = Landing { at: [5.0, 5.0, 10.0], normal: Some([0.0, 0.0, 1.0]), target: None };
        assert_eq!(shape_code(Primitive::Cylinder, &top, false), "translate([5, 5, 10]) cylinder(h = 10, r = 5);");
        assert_eq!(
            shape_code(Primitive::Cylinder, &top, true),
            "translate([5, 5, 11]) rotate([0, 180, 0]) cylinder(h = 10, r = 5);"
        );
        let side = Landing { at: [10.0, 5.0, 5.0], normal: Some([1.0, 0.0, 0.0]), target: None };
        assert_eq!(
            shape_code(Primitive::Cylinder, &side, true),
            "translate([11, 5, 5]) rotate([0, 90, 180]) cylinder(h = 10, r = 5);"
        );
        let plate = Landing { at: [0.0, 0.0, 0.0], normal: None, target: None };
        assert_eq!(shape_code(Primitive::Cube, &plate, false), "translate([0, 0, 5]) cube(10, center = true);");
    }

    #[test]
    fn a_new_shape_goes_into_the_boolean_selected() {
        let src = "difference() {\n    cube(10);\n}\nunion() sphere(2);\ncube(3);\n";
        let block = 0..src.find("}\n").unwrap() + 1;
        assert_eq!(place_for(src, std::slice::from_ref(&block)), Place::InBlock(block.clone()));
        // Not into a primitive, a boolean without braces, nor several.
        let cube = src.find("cube(3);").unwrap();
        assert_eq!(place_for(src, std::slice::from_ref(&(cube..cube + 8))), Place::End);
        let union = src.find("union").unwrap();
        assert_eq!(place_for(src, std::slice::from_ref(&(union..union + 18))), Place::End);
        assert_eq!(place_for(src, &[block, cube..cube + 8]), Place::End);
        assert_eq!(place_for(src, &[]), Place::End);
    }

    #[test]
    fn several_objects_are_deleted_and_duplicated_at_once() {
        let src = "cube(1);\ntranslate([5, 0, 0]) sphere(2);\ncylinder(h = 3, r = 1);\n";
        let at = |needle: &str| {
            let start = src.find(needle).unwrap();
            start..start + needle.len()
        };
        let (cube, sphere, cylinder) = (at("cube(1);"), at("sphere(2);"), at("cylinder(h = 3, r = 1);"));
        // Clicked in any order; a statement inside another selected one goes with it.
        assert_eq!(
            outermost_last_first(&[
                cube.clone(),
                cylinder.clone(),
                at("translate([5, 0, 0]) sphere(2);"),
                sphere.clone()
            ]),
            [cylinder.clone(), at("translate([5, 0, 0]) sphere(2);"), cube.clone()]
        );

        let (text, _, selection) = each_object(src, &[cylinder.clone(), cube.clone()], code_edit::delete).unwrap();
        assert_eq!(text, "translate([5, 0, 0]) sphere(2);\n");
        assert!(selection.iter().all(|r| r.is_empty()));

        let (text, _, selection) = each_object(src, &[sphere, cube], code_edit::duplicate).unwrap();
        assert_eq!(
            text,
            "cube(1);\ncube(1);\ntranslate([5, 0, 0]) sphere(2);\ntranslate([5, 0, 0]) sphere(2);\ncylinder(h = 3, r = 1);\n"
        );
        let selected: Vec<&str> = selection.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(selected, ["cube(1);", "sphere(2);"]);
        // The copies, not the originals.
        assert_eq!(selection[0].start, "cube(1);\n".len());
        assert!(selection[1].start > text.find("sphere").unwrap());
    }

    /// The keys read in a frame with `events`, with a text field focused or
    /// not.
    fn keys_with(events: Vec<Event>, focused: bool) -> Keys {
        let ctx = egui::Context::default();
        let mut keys = Keys::default();
        let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
            if focused {
                ctx.memory_mut(|m| m.request_focus(Id::new("editor")));
            }
            keys = read_keys(ctx);
        });
        keys
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn the_keys_are_left_to_a_focused_text_field() {
        let events = || {
            vec![
                key(Key::Delete, Modifiers::NONE),
                key(Key::D, Modifiers::COMMAND),
                Event::Copy,
                Event::Paste("cube(2);".into()),
                key(Key::Escape, Modifiers::NONE),
            ]
        };
        let keys = keys_with(events(), false);
        assert_eq!(
            keys,
            Keys {
                escape: true,
                delete: true,
                duplicate: true,
                copy: true,
                cut: false,
                paste: Some("cube(2);".into())
            }
        );
        assert_eq!(keys_with(events(), true), Keys::default());
        // Backspace deletes too; D alone, or Cmd+V without text, are no copy.
        let keys = keys_with(vec![key(Key::Backspace, Modifiers::NONE), key(Key::D, Modifiers::NONE)], false);
        assert_eq!(keys, Keys { delete: true, ..Keys::default() });
        let keys = keys_with(vec![key(Key::V, Modifiers::COMMAND), Event::Cut], false);
        assert_eq!(keys, Keys { cut: true, paste: Some(String::new()), ..Keys::default() });
    }

    // ---- The whole app, without a window ----

    use std::time::{Duration, Instant};

    use super::super::Stores;
    use crate::recent::RecentFiles;
    use crate::settings::Settings;

    /// The app without a window, nor the user's files, on a blank design.
    fn studio(ctx: &egui::Context) -> StudioApp {
        let stores = Stores { session: None, settings: None };
        StudioApp::with(ctx, None, stores, RecentFiles::default(), Settings::load(None), Vec::new())
    }

    /// A frame of the 3D view with `events`; returns the view's rectangle.
    fn frame(ctx: &egui::Context, app: &mut StudioApp, events: Vec<Event>, modifiers: Modifiers) -> Rect {
        // A quarter of a second a frame: no two clicks make a double-click.
        let time = ctx.input(|i| i.time) + 0.25;
        let input = egui::RawInput {
            events,
            modifiers,
            time: Some(time),
            predicted_dt: 1.0 / 60.0,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.view_panel(ctx));
        app.view_rect
    }

    /// Clicks the view at `at`, with `modifiers` held.
    fn tap(ctx: &egui::Context, app: &mut StudioApp, at: Pos2, modifiers: Modifiers) {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
        frame(ctx, app, vec![Event::PointerMoved(at)], modifiers);
        frame(ctx, app, vec![button(true)], modifiers);
        frame(ctx, app, vec![button(false)], modifiers);
    }

    /// Waits for the render of the code, then shows a frame of it.
    fn rendered(ctx: &egui::Context, app: &mut StudioApp) -> Rect {
        let start = Instant::now();
        while app.doc.busy() {
            assert!(start.elapsed() < Duration::from_secs(30), "no reply");
            std::thread::sleep(Duration::from_millis(5));
            app.receive();
        }
        assert!(!app.doc.failed, "the code does not render:\n{}", app.doc.text);
        frame(ctx, app, Vec::new(), Modifiers::NONE)
    }

    fn arm(ctx: &egui::Context, kind: Primitive, hole: bool) {
        let mut state = Shapes::load(ctx);
        state.tool = Tool { armed: Some(kind), hole };
        state.store(ctx);
    }

    /// Where the point `p` of the model shows in the view.
    fn screen(app: &StudioApp, rect: Rect, p: [f32; 3]) -> Pos2 {
        app.doc.viewer.to_screen(rect, Vec3::from_array(p)).expect("in front of the eye")
    }

    fn selected(app: &StudioApp) -> Vec<&str> {
        app.selected_ranges().unwrap_or_default().into_iter().map(|r| &app.doc.text[r]).collect()
    }

    #[test]
    fn a_box_with_a_screw_hole_made_with_the_mouse() {
        let ctx = egui::Context::default();
        let mut app = studio(&ctx);
        let rect = frame(&ctx, &mut app, Vec::new(), Modifiers::NONE);

        // A cube on the plate, selected once rendered.
        arm(&ctx, Primitive::Cube, false);
        let at = screen(&app, rect, [10.0, 10.0, 0.0]);
        tap(&ctx, &mut app, at, Modifiers::NONE);
        assert_eq!(app.doc.text, "translate([10, 10, 5]) cube(10, center = true);\n");
        assert_eq!(Shapes::load(&ctx).tool, Tool::default());
        let rect = rendered(&ctx, &mut app);
        assert_eq!(selected(&app), ["translate([10, 10, 5]) cube(10, center = true);"]);

        // A hole down into its top face.
        arm(&ctx, Primitive::Cylinder, true);
        let at = screen(&app, rect, [10.0, 10.0, 10.0]);
        tap(&ctx, &mut app, at, Modifiers::NONE);
        let one_hole = "difference() {\n    translate([10, 10, 5]) cube(10, center = true);\n    \
                        translate([10, 10, 11]) rotate([0, 180, 0]) cylinder(h = 10, r = 5);\n}\n";
        assert_eq!(app.doc.text, one_hole);
        rendered(&ctx, &mut app);
        assert_eq!(selected(&app), ["translate([10, 10, 11]) rotate([0, 180, 0]) cylinder(h = 10, r = 5);"]);

        // Cmd+D duplicates the hole, Delete removes the copy.
        frame(&ctx, &mut app, vec![key(Key::D, Modifiers::COMMAND)], Modifiers::COMMAND);
        let hole = "    translate([10, 10, 11]) rotate([0, 180, 0]) cylinder(h = 10, r = 5);\n";
        assert_eq!(app.doc.text.matches(hole).count(), 2);
        rendered(&ctx, &mut app);
        assert_eq!(selected(&app).len(), 1);
        frame(&ctx, &mut app, vec![key(Key::Delete, Modifiers::NONE)], Modifiers::NONE);
        assert_eq!(app.doc.text, one_hole);

        // Not while the editor has the keyboard.
        rendered(&ctx, &mut app);
        app.doc.viewer.select_at(app.doc.text.find("translate([10, 10, 11])").unwrap());
        ctx.memory_mut(|m| m.request_focus(Id::new("editor")));
        frame(&ctx, &mut app, vec![key(Key::Delete, Modifiers::NONE)], Modifiers::NONE);
        assert_eq!(app.doc.text, one_hole);
    }

    #[test]
    fn objects_clicked_in_order_are_subtracted_copied_and_pasted() {
        let ctx = egui::Context::default();
        let mut app = studio(&ctx);
        app.doc.text = "cube(10);\ntranslate([20, 0, 0]) cube(10);\n".into();
        app.render_now();
        let rect = rendered(&ctx, &mut app);
        // The second one first: it is what the others are cut from.
        let at = screen(&app, rect, [25.0, 5.0, 10.0]);
        tap(&ctx, &mut app, at, Modifiers::NONE);
        let at = screen(&app, rect, [5.0, 5.0, 10.0]);
        tap(&ctx, &mut app, at, Modifiers::SHIFT);
        assert_eq!(selected(&app), ["cube(10);", "cube(10);"]);
        app.combine(&ctx, Boolean::Difference);
        assert_eq!(app.doc.text, "difference() {\n    translate([20, 0, 0]) cube(10);\n    cube(10);\n}\n");
        rendered(&ctx, &mut app);
        assert_eq!(selected(&app).len(), 1);

        // Copied with Cmd+C, pasted with Cmd+V at the end of the code.
        frame(&ctx, &mut app, vec![Event::Copy], Modifiers::COMMAND);
        let copied = Shapes::load(&ctx).clipboard.expect("copied");
        assert!(copied.starts_with("difference() {\n"));
        frame(&ctx, &mut app, vec![Event::Paste(copied.clone())], Modifiers::COMMAND);
        assert_eq!(app.doc.text, format!("{0}{0}", copied + "\n"));
        // Text that is not code pastes what was copied here.
        rendered(&ctx, &mut app);
        frame(&ctx, &mut app, vec![Event::Paste("hello".into())], Modifiers::COMMAND);
        assert_eq!(app.doc.text.matches("difference()").count(), 3);
    }
}
