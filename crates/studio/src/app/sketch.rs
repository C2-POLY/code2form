//! The sketch mode (C2F-42): a 2D profile drawn on a plane or a flat face
//! with lines, rectangles, circles and arcs, held by constraints and
//! dimensions, then written into the code as a `sketch_<name>()` module,
//! extruded or revolved.
//!
//! The view turns to face the plane and dims the model; a grid in the
//! plane's coordinates and the sketch are drawn over it with the egui
//! painter. The panel on the right holds the tools, the constraint and
//! dimension palette, the dimensions with their values, and the buttons
//! that finish the sketch. The sketch is solved after every change, and
//! live while a point is dragged (holding it), so that its colors always
//! tell what is left free: blue can still move, the text color is pinned
//! down, red is held by a constraint that conflicts.
//!
//! Finishing writes the sketch as one undo step (`sketch_view::write`).
//! Leaving without finishing (Esc, Cancel) keeps the sketch in memory,
//! once, for "Resume unfinished": an Esc too many must not lose a
//! drawing, and the code stays untouched until the sketch is finished.
//!
//! The logic without the window (the plane, the tools, hit tests, the
//! palette, the code) is in `crate::sketch_view`.

use std::ops::Range;

use c2f_sketch::{
    Constraint, ConstraintId, Dimension, EntityId, ExtrudeMode, Geometry, Plane, Point, PointId, Sketch, SnapKind, SolveOptions,
    SolveReport, extrude_code, from_code, is_identifier, module_name, profile, revolve_code, sketches_in, solve,
    solve_with,
};
use eframe::egui::{
    self, Align2, Button, Color32, CornerRadius, FontId, Key, PointerButton, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, TextEdit, Vec2,
};
use glam::Vec3;

use super::StudioApp;
use super::toolbar::{self, Icon, Row};
use crate::code_edit::number;
use crate::console::Level;
use crate::sketch_view::{
    self as view, Drawing, Feature, Item, Measure, Pick, PlaneFrame, Preview, Rule, Target, Tool,
};
use crate::theme;

/// What is this close to the pointer, in points, is under it.
const REACH: f32 = 8.0;
/// The extrusion's height handle: its grab radius, in points.
const HANDLE: f32 = 10.0;

/// The sketch mode of a tab.
#[derive(Default)]
pub(super) struct SketchState {
    mode: Option<Box<SketchMode>>,
    /// The last sketch left without finishing it, to resume.
    draft: Option<Box<SketchMode>>,
    /// Choosing the plane of a new sketch.
    picking: Option<Picking>,
    /// What the screenshot hook asks for (`sketch:xy`, `sketch:open:name`,
    /// `sketch:extrude:name`), done at the next frame.
    pub(super) screenshot: Option<String>,
}

impl SketchState {
    /// Whether a sketch is being drawn (or extruded).
    pub(super) fn active(&self) -> bool {
        self.mode.is_some()
    }

    /// The sketch's plane, and what it will cut.
    #[cfg(test)]
    pub(super) fn plane_and_cut(&self) -> Option<(Plane, Option<Range<usize>>)> {
        self.mode.as_ref().map(|m| (m.sketch.plane, m.cut_from.clone()))
    }
}

/// Choosing the plane of a new sketch: the origin planes show in the view,
/// and the plane or the flat face of the model under the pointer lights up.
#[derive(Debug, Default)]
struct Picking {
    /// What the sketch cuts once extruded: the object selected when Sketch
    /// was clicked (a click on a face cuts the face's object instead).
    cut_from: Option<Range<usize>>,
    /// A plane lit without the pointer on it: the one whose button the
    /// pointer is on, or a screenshot's.
    lit: Option<Plane>,
}

/// What the pointer is on while a sketch's plane is chosen.
#[derive(Clone, Copy, Debug)]
enum Pointed {
    Origin(Plane),
    /// A face of the model: a point of it, its normal, its statement.
    Face(Vec3, Vec3, Option<crate::select::Statement>),
}

/// The colors of the origin planes: each the color of the axis it is
/// across, as the view's axes and gizmo draw them; darker in the light
/// theme, to read on its light buttons.
fn plane_color(plane: &Plane) -> Color32 {
    let dark = theme::palette().dark;
    match (plane, dark) {
        (Plane::Xz, true) => Color32::from_rgb(0x5f, 0xb8, 0x6b),
        (Plane::Xz, false) => Color32::from_rgb(0x1e, 0x7a, 0x34),
        (Plane::Yz, true) => Color32::from_rgb(0xe0, 0x5a, 0x5a),
        (Plane::Yz, false) => Color32::from_rgb(0xb8, 0x2e, 0x2e),
        (_, true) => Color32::from_rgb(0x5f, 0x8d, 0xff),
        (_, false) => Color32::from_rgb(0x1f, 0x5f, 0xd1),
    }
}

/// How large the origin planes show: half the model's size, within
/// bounds, so that they read next to it without hiding it.
fn plane_size(bounds: Option<(Vec3, Vec3)>) -> f32 {
    bounds.map_or(20.0, |(lo, hi)| ((hi - lo).max_element() * 0.5).clamp(10.0, 500.0))
}

/// A sketch being drawn.
struct SketchMode {
    sketch: Sketch,
    /// Its name: the module is `sketch_<name>`.
    name: String,
    /// Reopened from the code: finishing replaces its module.
    reopened: bool,
    frame: PlaneFrame,
    drawing: Drawing,
    report: SolveReport,
    selection: Vec<Item>,
    hover: Option<Item>,
    /// Where a drawing tool's next click would land.
    pick: Option<Pick>,
    drag: Option<Drag>,
    prompt: Option<Prompt>,
    stage: Stage,
    /// The object an extrusion can cut, as the range of its statement in
    /// `source`, the code when the mode began.
    cut_from: Option<Range<usize>>,
    source: String,
    /// What the palette last asked to select.
    hint: Option<String>,
    /// Whether a drag in the view may turn the camera next frame.
    orbit: bool,
}

/// Points dragged: where they were and where the pointer was when the
/// drag began.
struct Drag {
    points: Vec<(PointId, Point)>,
    from: Point,
}

/// The popup asking a dimension's value and variable.
struct Prompt {
    constraint: Constraint,
    /// The constraint edited; `None` for a new one.
    editing: Option<usize>,
    value: String,
    name: String,
    at: Pos2,
    focus: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Draw,
    /// A height along the plane's normal (negative goes the other way),
    /// its variable, and whether it cuts.
    Extrude {
        height: f64,
        name: String,
        cut: bool,
        dragging: bool,
    },
    /// Around a line of the sketch, by an angle in degrees.
    Revolve {
        axis: EntityId,
        angle: f64,
        name: String,
        cut: bool,
    },
}

/// What the panel and the keys ask of the app.
enum Action {
    Finish,
    Leave,
    Apply,
}

impl SketchMode {
    fn new(mut sketch: Sketch, name: String, reopened: bool, cut_from: Option<Range<usize>>, source: String) -> Self {
        let report = solve(&mut sketch);
        SketchMode {
            frame: PlaneFrame::of(&sketch.plane),
            sketch,
            name,
            reopened,
            drawing: Drawing::default(),
            report,
            selection: Vec::new(),
            hover: None,
            pick: None,
            drag: None,
            prompt: None,
            stage: Stage::Draw,
            cut_from,
            source,
            hint: None,
            orbit: false,
        }
    }

    /// Solves the sketch again after a change.
    fn resolve(&mut self) {
        self.report = solve(&mut self.sketch);
    }

    /// After points, entities or constraints were removed, which numbers
    /// what stays afresh: whatever still holds an id (the shape being
    /// drawn, a drag, the dimension prompt) would name the wrong item, or
    /// one past the end, so it is dropped. Removing points and entities
    /// also clears the selection, which named them.
    fn renumbered(&mut self) {
        self.drawing.cancel();
        self.drag = None;
        self.prompt = None;
        self.hover = None;
        self.pick = None;
        self.resolve();
    }

    /// The color of an item: red when a conflicting constraint holds it,
    /// the text color when pinned down, blue when it can still move.
    fn color(&self, item: Item) -> Color32 {
        let conflicting = self
            .report
            .conflicting
            .iter()
            .chain(&self.report.unsatisfied)
            .filter_map(|c| self.sketch.constraints.get(c.0))
            .any(|c| view::constraint_items(&self.sketch, c).contains(&item));
        let pinned = match item {
            Item::Point(p) => self.report.point_constrained.get(p.0).copied().unwrap_or(false),
            Item::Entity(e) => self.report.entity_constrained.get(e.0).copied().unwrap_or(false),
        };
        if conflicting {
            theme::palette().error
        } else if pinned {
            theme::palette().text
        } else {
            theme::palette().accent
        }
    }

    /// The sketch's middle and size, for the camera.
    fn extent(&self) -> Option<(Point, f64)> {
        let first = *self.sketch.points.first()?;
        let (mut lo, mut hi) = (first, first);
        let mut grow = |p: Point, r: f64| {
            lo = Point::new(lo.x.min(p.x - r), lo.y.min(p.y - r));
            hi = Point::new(hi.x.max(p.x + r), hi.y.max(p.y + r));
        };
        for p in &self.sketch.points {
            grow(*p, 0.0);
        }
        for e in 0..self.sketch.entities.len() {
            if let Geometry::Circle { center, radius } = self.sketch.entities[e].geometry {
                grow(self.sketch.point(center), radius);
            }
        }
        Some((lo.midpoint(hi), lo.distance(hi).max(10.0)))
    }

    /// The status line: free, fully constrained, or conflicting.
    fn status(&self) -> (String, Color32) {
        if let Some(&c) = self.report.conflicting.first() {
            let what = self.sketch.constraints.get(c.0).map(describe).unwrap_or_default();
            return (format!("Conflict: {what}"), theme::palette().error);
        }
        if self.sketch.entities.is_empty() {
            return ("Empty sketch".into(), theme::palette().text_dim);
        }
        match self.report.dof {
            0 if self.report.converged() => ("Fully constrained".into(), theme::palette().success),
            0 => ("Cannot solve the constraints".into(), theme::palette().error),
            1 => ("1 degree of freedom left".into(), theme::palette().accent),
            n => (format!("{n} degrees of freedom left"), theme::palette().accent),
        }
    }
}

impl StudioApp {
    // ---- entering and leaving ----

    /// Opens the sketch held by the design's module `module_name`
    /// (`sketch_l_bracket`, or `l_bracket`) with its constraints, solved
    /// with the values its variables have in the code.
    pub(super) fn open_sketch(&mut self, ctx: &egui::Context, module_name: &str) {
        let module =
            if module_name.starts_with("sketch_") { module_name.to_string() } else { format!("sketch_{module_name}") };
        match from_code(&self.doc.text, &module) {
            Ok(sketch) => {
                let name = module["sketch_".len()..].to_string();
                let mode = SketchMode::new(sketch, name, true, None, self.doc.text.clone());
                self.enter_sketch(Box::new(mode));
                ctx.request_repaint();
            }
            Err(e) => self.doc.console.log(Level::Warning, format!("Cannot open the sketch {module}: {e}")),
        }
    }

    /// A new sketch on `plane`; an extrusion of it can cut `cut_from`.
    fn new_sketch(&mut self, plane: Plane, cut_from: Option<Range<usize>>) {
        let text = &self.doc.text;
        let name = (1..)
            .map(|n| format!("profile{n}"))
            .find(|name| !view::module_taken(text, &module_name(name)))
            .expect("a free name");
        let mode = SketchMode::new(Sketch::new(plane), name, false, cut_from, self.doc.text.clone());
        self.enter_sketch(Box::new(mode));
    }

    fn enter_sketch(&mut self, mode: Box<SketchMode>) {
        let goal = face_camera(&mode, &self.doc.viewer);
        self.doc.viewer.fly_to(goal);
        self.doc.viewer.clear_selection();
        self.doc.sketch.picking = None;
        self.doc.sketch.mode = Some(mode);
    }

    /// Leaves the sketch mode without writing anything; a sketch with
    /// something drawn is kept to resume.
    fn leave_sketch(&mut self) {
        if let Some(mode) = self.doc.sketch.mode.take() {
            if !mode.sketch.entities.is_empty() {
                let message = "Sketch left unfinished: Sketch, then Resume unfinished, takes it up again";
                self.doc.console.log(Level::Info, message);
                // One draft is kept, the newest; the one it replaces is
                // said to be gone rather than lost silently.
                if let Some(older) = self.doc.sketch.draft.replace(mode) {
                    let message = format!(
                        "The older unfinished sketch {} was dropped for this one",
                        module_name(&older.name)
                    );
                    self.doc.console.log(Level::Warning, message);
                }
            }
        }
    }

    /// What the view may do with the pointer while sketching: no
    /// selecting, no framing on double-click, and no turning the camera
    /// except around an extrusion. Called just before the view is shown.
    pub(super) fn sketch_before_view(&mut self) {
        let state = &self.doc.sketch;
        // Neither while a sketch's plane is chosen: a click picks a plane.
        self.doc.viewer.select_enabled = state.mode.is_none() && state.picking.is_none();
        if let Some(mode) = &state.mode {
            self.doc.viewer.frame_on_double_click = false;
            self.doc.viewer.orbit_enabled &= mode.orbit;
        }
    }

    /// The object the selection names, when it can be cut: the first
    /// selected statement of the main file, as on screen.
    fn cut_target(&self) -> Option<Range<usize>> {
        self.selected_ranges()?.into_iter().next()
    }

    /// The sketch module the selected statement calls, if any: what "Edit
    /// sketch" reopens.
    fn selected_sketch(&self) -> Option<String> {
        let range = self.cut_target()?;
        let module = view::sketch_called(self.doc.text.get(range)?)?;
        sketches_in(&self.doc.text).contains(&module).then_some(module)
    }

    /// The screenshot hook's request, now that the design is on screen.
    fn sketch_screenshot(&mut self, ctx: &egui::Context) {
        let Some(spec) = self.doc.sketch.screenshot.take() else { return };
        let spec = spec.strip_prefix("sketch:").unwrap_or(&spec).to_string();
        match spec.as_str() {
            "pick" => self.doc.sketch.picking = Some(Picking::default()),
            "pick:xz" => self.doc.sketch.picking = Some(Picking { lit: Some(Plane::Xz), ..Picking::default() }),
            "xy" => self.new_sketch(Plane::Xy, None),
            "xz" => self.new_sketch(Plane::Xz, None),
            "yz" => self.new_sketch(Plane::Yz, None),
            _ => {
                if let Some(name) = spec.strip_prefix("open:") {
                    self.open_sketch(ctx, name);
                } else if let Some(name) = spec.strip_prefix("extrude:") {
                    self.open_sketch(ctx, name);
                    if let Some(mode) = &mut self.doc.sketch.mode {
                        self.doc.viewer.fly_to(extrude_camera(mode, &self.doc.viewer));
                        mode.stage = Stage::Extrude { height: 10.0, name: String::new(), cut: false, dragging: false };
                    }
                }
            }
        }
    }

    // ---- the panel ----

    /// The sketch's panel, on the right, while the mode is on.
    pub(super) fn sketch_panel(&mut self, ctx: &egui::Context) {
        self.sketch_screenshot(ctx);
        let Some(mut mode) = self.doc.sketch.mode.take() else { return };
        let mut action = None;
        let stage = std::mem::discriminant(&mode.stage);
        egui::SidePanel::right("sketch_panel")
            .frame(crate::customizer::frame())
            .resizable(false)
            .exact_width(280.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| action = panel_ui(ui, &mut mode));
            });
        self.follow_stage(&mode, stage);
        self.doc.sketch.mode = Some(mode);
        self.sketch_action(ctx, action);
    }

    /// Extruding is seen from three quarters, drawing face on: the camera
    /// flies when the stage changed from `before`.
    fn follow_stage(&mut self, mode: &SketchMode, before: std::mem::Discriminant<Stage>) {
        if std::mem::discriminant(&mode.stage) != before {
            let goal = if mode.stage == Stage::Draw {
                face_camera(mode, &self.doc.viewer)
            } else {
                extrude_camera(mode, &self.doc.viewer)
            };
            self.doc.viewer.fly_to(goal);
        }
    }

    /// The sketch's row of the toolbar: while drawing, DRAW and CONSTRAIN
    /// and the green Finish sketch; while extruding or revolving, Back and
    /// the green button that writes it.
    pub(super) fn sketch_toolbar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, row: Row) {
        let Some(mut mode) = self.doc.sketch.mode.take() else { return };
        let stage = std::mem::discriminant(&mode.stage);
        let action = toolbar_ui(ui, row, &mut mode);
        self.follow_stage(&mode, stage);
        self.doc.sketch.mode = Some(mode);
        self.sketch_action(ctx, action);
    }

    fn sketch_action(&mut self, ctx: &egui::Context, action: Option<Action>) {
        match action {
            Some(Action::Finish) => self.finish_sketch(ctx, false),
            Some(Action::Apply) => self.finish_sketch(ctx, true),
            Some(Action::Leave) => self.leave_sketch(),
            None => {}
        }
    }

    /// Writes the sketch into the code as one undo step, with its
    /// extrusion or revolution when `feature` (the stage says which), and
    /// leaves the mode; a refusal is logged and the mode stays.
    fn finish_sketch(&mut self, ctx: &egui::Context, feature: bool) {
        let Some(mode) = self.doc.sketch.mode.take() else { return };
        match self.write_sketch(&mode, feature) {
            // The draft, if any, is another sketch: resuming takes the
            // draft out, so the one finished here is never still in it.
            Ok((written, done)) => self.apply_code_edits(ctx, Ok(written), &done),
            Err(why) => {
                self.doc.console.log(Level::Warning, why);
                self.doc.sketch.mode = Some(mode);
            }
        }
    }

    fn write_sketch(&self, mode: &SketchMode, feature: bool) -> Result<(view::Written, String), String> {
        let module = module_name(&mode.name);
        if mode.name.trim().is_empty() {
            return Err("Give the sketch a name.".into());
        }
        // Any module of that name, sketch or not: writing would replace it.
        if !mode.reopened && view::module_taken(&self.doc.text, &module) {
            return Err(format!("The design already has a module {module}: give this sketch another name."));
        }
        if mode.sketch.entities.is_empty() {
            return Err("The sketch is empty: draw something first.".into());
        }
        let variable = |name: &str, value: f64| match name.trim() {
            "" => Ok(Dimension::new(value)),
            name if is_identifier(name) => Ok(Dimension::named(name, value)),
            name => Err(format!("`{name}` cannot name a variable.")),
        };
        let cut_target = |cut: bool| -> Result<Option<Range<usize>>, String> {
            if !cut {
                return Ok(None);
            }
            if self.doc.text != mode.source {
                return Err("The code changed since the sketch began: cutting would miss its object.".into());
            }
            Ok(mode.cut_from.clone())
        };
        let (code, cut_from, done) = match (&mode.stage, feature) {
            (Stage::Extrude { height, name, cut, .. }, true) => {
                let height = variable(name, round(*height))?;
                if height.value == 0.0 {
                    return Err("The extrusion has no height.".into());
                }
                let code = extrude_code(&mode.name, &mode.sketch.plane, &height, &ExtrudeMode::Add);
                (Some(code), cut_target(*cut)?, format!("Extruded the sketch {module}"))
            }
            (Stage::Revolve { axis, angle, name, cut }, true) => {
                let angle = variable(name, round(*angle))?;
                let code = revolve_code(&mode.sketch, &mode.name, *axis, &angle, &ExtrudeMode::Add)
                    .map_err(|e| format!("Cannot revolve the sketch: {e}"))?;
                (Some(code), cut_target(*cut)?, format!("Revolved the sketch {module}"))
            }
            _ => (None, None, format!("Wrote the sketch {module}")),
        };
        let feature = code.as_ref().map(|code| Feature { code, cut_from });
        let written = view::write(&self.doc.text, &mode.sketch, &mode.name, feature)?;
        Ok((written, done))
    }

    // ---- the view ----

    /// What the sketch mode draws over the 3D view and does with the
    /// pointer and the keys there; whether it took the view, so that the
    /// other modelling features leave it alone.
    pub(super) fn sketch_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, response: &egui::Response) -> bool {
        if self.doc.sketch.picking.is_some() {
            self.pick_plane(ctx, ui, response);
            return true;
        }
        let Some(mut mode) = self.doc.sketch.mode.take() else {
            return false;
        };
        let action = match mode.stage {
            Stage::Draw => self.draw_stage(ctx, ui, response, &mut mode),
            _ => self.feature_stage(ctx, ui, response, &mut mode),
        };
        if mode.prompt.is_some() {
            prompt_ui(ctx, &mut mode);
        }
        self.doc.sketch.mode = Some(mode);
        self.sketch_action(ctx, action);
        true
    }

    /// The SKETCH group of the modelling toolbar: a new sketch, from a menu
    /// of planes, and the sketch selected reopened.
    pub(super) fn sketch_tools(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, row: Row) {
        let edit = self.selected_sketch();
        let picking = self.doc.sketch.picking.is_some();
        let (mut start, mut open) = (false, false);
        toolbar::group(ui, "SKETCH", row, |ui| {
            let tip = "Draw a 2D profile on a plane or a face of the model, constrain it, then extrude or revolve it";
            start = toolbar::tool(ui, row, Icon::Sketch, "Sketch", picking, Ok(()), tip).clicked();
            let (enabled, tip) = match &edit {
                Some(module) => (Ok(()), format!("Reopen {module} with its constraints")),
                None => (Err("Select a sketch to reopen it"), String::new()),
            };
            open = toolbar::tool(ui, row, Icon::EditSketch, "Edit", false, enabled, &tip).clicked();
        });
        // Sketch again while choosing gives up.
        if start && picking {
            self.doc.sketch.picking = None;
        } else if start {
            self.doc.sketch.picking = Some(Picking { cut_from: self.cut_target(), lit: None });
        } else if let (true, Some(module)) = (open, edit) {
            self.open_sketch(ctx, &module);
        }
    }

    /// Takes up the sketch left unfinished.
    fn resume_sketch(&mut self) {
        if let Some(mut draft) = self.doc.sketch.draft.take() {
            // The code may have changed meanwhile: no cutting then.
            if draft.source != self.doc.text {
                draft.cut_from = None;
            }
            draft.stage = Stage::Draw;
            self.enter_sketch(draft);
        }
    }

    /// Choosing the plane of a new sketch: the origin planes drawn in the
    /// view, the plane or flat face under the pointer lit, a click to
    /// sketch on it, and a bar of the planes by name. Esc gives up.
    fn pick_plane(&mut self, ctx: &egui::Context, ui: &egui::Ui, response: &egui::Response) {
        let rect = response.rect;
        let (chosen, lit_by_bar) = self.plane_bar(ctx, rect);
        if chosen {
            return;
        }
        let viewer = &self.doc.viewer;
        let size = plane_size(viewer.model_bounds());
        // The nearer of an origin plane and a face of the model.
        let pointed = response.hover_pos().and_then(|pos| {
            let ray = viewer.ray(rect, pos);
            let plane = view::origin_plane_hit(ray.origin, ray.direction, size);
            let face = viewer.surface_at(rect, pos).map(|(point, normal, statement)| {
                ((point - ray.origin).dot(ray.direction), Pointed::Face(point, normal, statement))
            });
            match (plane, face) {
                (Some((plane, t)), Some((at, _))) if t <= at => Some(Pointed::Origin(plane)),
                (_, Some((_, face))) => Some(face),
                (Some((plane, _)), None) => Some(Pointed::Origin(plane)),
                (None, None) => None,
            }
        });
        let picking = self.doc.sketch.picking.as_ref();
        let lit = match pointed {
            Some(Pointed::Origin(plane)) => Some(plane),
            _ => lit_by_bar.or(picking.and_then(|p| p.lit)),
        };

        // The squares, the farthest first.
        let painter = ui.painter_at(rect);
        let eye = viewer.ray(rect, rect.center()).origin;
        let mut planes = view::ORIGIN_PLANES;
        let distance = |plane: &Plane| -> f32 {
            let corners = view::origin_square(plane, size);
            (corners[2] * 0.5).distance(eye)
        };
        planes.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
        for plane in planes {
            let corners = view::origin_square(&plane, size);
            let Some(points) = corners.iter().map(|c| viewer.to_screen(rect, *c)).collect::<Option<Vec<Pos2>>>() else {
                continue;
            };
            let on = lit == Some(plane);
            let color = plane_color(&plane);
            let fill = color.gamma_multiply(if on { 0.4 } else { 0.14 });
            painter.add(egui::Shape::convex_polygon(points.clone(), fill, Stroke::new(if on { 2.0 } else { 1.0 }, color)));
            let name = match plane {
                Plane::Xz => "XZ",
                Plane::Yz => "YZ",
                _ => "XY",
            };
            let font = FontId::proportional(if on { 14.0 } else { 12.0 });
            painter.text(points[2], Align2::LEFT_BOTTOM, name, font, color);
        }
        if let Some(Pointed::Face(point, normal, _)) = pointed {
            let accent = theme::palette().accent_strong;
            if let (Some(at), Some(tip)) = (viewer.to_screen(rect, point), viewer.to_screen(rect, point + normal * size * 0.25)) {
                painter.line_segment([at, tip], Stroke::new(2.0, accent));
                painter.circle(at, 5.0, accent.gamma_multiply(0.35), Stroke::new(1.5, accent));
            }
        }
        let hint = match pointed {
            Some(Pointed::Face(..)) => "Click to sketch on this face · Esc: cancel",
            Some(Pointed::Origin(_)) => "Click to sketch on this plane · Esc: cancel",
            None => "Click a plane or a flat face of the model to sketch on it · Esc: cancel",
        };
        hint_bar(&painter, rect, hint);
        if pointed.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.doc.sketch.picking = None;
            return;
        }
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let cut_from = self.doc.sketch.picking.take().and_then(|p| p.cut_from);
        match pointed {
            Some(Pointed::Origin(plane)) => self.new_sketch(plane, cut_from),
            Some(Pointed::Face(point, normal, statement)) => {
                let current = self.doc.viewer.scene_source() == self.doc.text;
                let cut_from = statement.and_then(|s| s.range()).filter(|_| current);
                self.new_sketch(view::face_plane(point, normal), cut_from);
            }
            // A click beside them keeps choosing.
            None => self.doc.sketch.picking = Some(Picking { cut_from, lit: None }),
        }
    }

    /// The bar over the view while a sketch's plane is chosen: the origin
    /// planes by name, the sketch left unfinished, and Cancel. Returns
    /// whether it settled the choice, and the plane whose button the
    /// pointer is on.
    fn plane_bar(&mut self, ctx: &egui::Context, view: Rect) -> (bool, Option<Plane>) {
        let palette = theme::palette();
        let has_draft = self.doc.sketch.draft.is_some();
        let (mut chosen, mut hovered, mut resume, mut cancel) = (None, None, false, false);
        let frame = egui::Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(10, 6));
        egui::Area::new(egui::Id::new("sketch-planes"))
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(view.center().x, view.top() + 12.0))
            .constrain_to(view)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Sketch on").color(palette.text_dim));
                        for (plane, name, tip) in [
                            (Plane::Xy, "XY", "The ground plane, across Z"),
                            (Plane::Xz, "XZ", "The front plane, across Y"),
                            (Plane::Yz, "YZ", "The side plane, across X"),
                        ] {
                            let text = RichText::new(name).strong().color(plane_color(&plane));
                            let button = ui.add(Button::new(text).min_size(Vec2::new(40.0, 0.0))).on_hover_text(tip);
                            if button.hovered() {
                                hovered = Some(plane);
                            }
                            if button.clicked() {
                                chosen = Some(plane);
                            }
                        }
                        ui.label(RichText::new("or a face of the model").color(palette.text_dim));
                        ui.separator();
                        if has_draft {
                            resume = ui.button("Resume unfinished").on_hover_text("Take up the sketch left unfinished").clicked();
                        }
                        cancel = ui.button("Cancel").on_hover_text("Give up (Esc)").clicked();
                    });
                });
            });
        if let Some(plane) = chosen {
            let cut_from = self.doc.sketch.picking.take().and_then(|p| p.cut_from);
            self.new_sketch(plane, cut_from);
        } else if resume {
            self.resume_sketch();
        } else if cancel {
            self.doc.sketch.picking = None;
        }
        (chosen.is_some() || resume || cancel, hovered)
    }

    /// Drawing, selecting, dragging and dimensioning.
    fn draw_stage(
        &mut self,
        ctx: &egui::Context,
        ui: &egui::Ui,
        response: &egui::Response,
        mode: &mut SketchMode,
    ) -> Option<Action> {
        let rect = response.rect;
        let painter = ui.painter_at(rect);
        let viewer = &mut self.doc.viewer;
        mode.orbit = false;
        pan(viewer, response);
        let ppm = pixels_per_mm(viewer, rect, &mode.frame);
        let tolerance = f64::from(REACH / ppm);
        let cursor = response.hover_pos().and_then(|pos| {
            let ray = viewer.ray(rect, pos);
            mode.frame.intersect(ray.origin, ray.direction)
        });
        let labels = dimension_labels(viewer, rect, &painter, mode);

        // The keys, unless typing in a field or answering the prompt.
        let mut action = None;
        let typing = ctx.memory(|m| m.focused().is_some()) || mode.prompt.is_some();
        if !typing {
            let pressed = |key| ctx.input(|i| i.key_pressed(key));
            if pressed(Key::Escape) && !mode.drawing.cancel() {
                if mode.drawing.tool != Tool::Select {
                    mode.drawing.set_tool(Tool::Select);
                } else if !mode.selection.is_empty() {
                    mode.selection.clear();
                } else {
                    action = Some(Action::Leave);
                }
            }
            if pressed(Key::Enter) && !mode.drawing.cancel() {
                action = Some(Action::Finish);
            }
            if (pressed(Key::Delete) || pressed(Key::Backspace)) && !mode.selection.is_empty() {
                mode.sketch = view::remove(&mode.sketch, &mode.selection);
                mode.selection.clear();
                mode.renumbered();
            }
            for (key, tool) in [
                (Key::S, Tool::Select),
                (Key::L, Tool::Line),
                (Key::R, Tool::Rectangle),
                (Key::C, Tool::Circle),
                (Key::A, Tool::Arc),
            ] {
                if pressed(key) && !ctx.input(|i| i.modifiers.command) {
                    mode.drawing.set_tool(tool);
                }
            }
        }

        // The pointer.
        let clicked = response.clicked_by(PointerButton::Primary);
        let label_clicked = clicked
            .then(|| response.interact_pointer_pos())
            .flatten()
            .and_then(|pos| labels.iter().find(|l| l.rect.contains(pos)))
            .map(|l| (l.constraint, l.rect.left_bottom()));
        mode.pick = None;
        mode.hover = None;
        if let Some((index, at)) = label_clicked {
            open_prompt(mode, index, at);
        } else if mode.drawing.tool == Tool::Select {
            select_input(ctx, response, mode, cursor, tolerance);
        } else if let Some(at) = cursor {
            // The grid drawn, unless Alt is held.
            let grid = (!ctx.input(|i| i.modifiers.alt)).then(|| view::grid_step(f64::from(12.0 / ppm)).0);
            let pick = view::pick(&mode.sketch, at, tolerance, grid);
            if clicked && !mode.drawing.click(&mut mode.sketch, &pick).is_empty() {
                mode.resolve();
            }
            if response.double_clicked_by(PointerButton::Primary) {
                mode.drawing.cancel();
            }
            mode.pick = Some(pick);
            if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }

        // The picture.
        let viewer = &self.doc.viewer;
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(110));
        paint_grid(viewer, rect, &painter, &mode.frame, ppm);
        paint_sketch(viewer, rect, &painter, mode, None);
        for label in &labels {
            paint_label(&painter, label);
        }
        let next = cursor.map(|at| mode.pick.map_or(at, |p| p.at));
        if let Some(preview) = next.and_then(|at| mode.drawing.preview(&mode.sketch, at)) {
            paint_preview(viewer, rect, &painter, &mode.frame, preview);
        }
        if let Some(pick) = mode.pick {
            paint_pick(viewer, rect, &painter, &mode.frame, &pick);
        }
        let hint = mode.hint.clone().unwrap_or_else(|| mode.drawing.tool.hint(mode.drawing.clicks()).to_string());
        hint_bar(&painter, rect, &format!("{hint} · Esc: cancel · Enter: finish"));
        if response.hovered() || mode.drag.is_some() {
            ctx.request_repaint();
        }
        action
    }

    /// Extruding or revolving: the profile and the feature's preview, the
    /// height dragged by its handle.
    fn feature_stage(
        &mut self,
        ctx: &egui::Context,
        ui: &egui::Ui,
        response: &egui::Response,
        mode: &mut SketchMode,
    ) -> Option<Action> {
        let rect = response.rect;
        let painter = ui.painter_at(rect);
        let viewer = &mut self.doc.viewer;
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(60));
        let frame = mode.frame;
        let shape = profile(&mode.sketch, 64);
        let mut action = None;
        if !ctx.memory(|m| m.focused().is_some()) {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                mode.stage = Stage::Draw;
                viewer.fly_to(face_camera(mode, viewer));
                return None;
            }
            if ctx.input(|i| i.key_pressed(Key::Enter)) {
                action = Some(Action::Apply);
            }
        }
        match &mut mode.stage {
            Stage::Extrude { height, dragging, .. } => {
                let n = frame.normal;
                let (sum, count) = shape
                    .loops
                    .iter()
                    .filter(|l| !l.is_hole())
                    .flat_map(|l| &l.points)
                    .fold((Point::default(), 0.0), |(s, k), p| (Point::new(s.x + p.x, s.y + p.y), k + 1.0));
                let center = frame.to_world(if count > 0.0 {
                    Point::new(sum.x / count, sum.y / count)
                } else {
                    Point::default()
                });
                let tip = center + n * *height as f32;
                let (base_screen, tip_screen) = (viewer.to_screen(rect, center), viewer.to_screen(rect, tip));
                let near_handle =
                    response.hover_pos().zip(tip_screen).is_some_and(|(pos, tip)| pos.distance(tip) <= HANDLE);
                if response.drag_started_by(PointerButton::Primary) && near_handle {
                    *dragging = true;
                }
                if *dragging {
                    if response.dragged() {
                        let delta = response.drag_delta();
                        let along = viewer.to_screen(rect, center + n).zip(base_screen).map(|(a, b)| a - b);
                        let step = match along {
                            Some(dir) if dir.length() > 0.5 => delta.dot(dir) / dir.length_sq(),
                            _ => -delta.y / pixels_per_mm(viewer, rect, &frame),
                        };
                        *height += f64::from(step);
                    } else {
                        *dragging = false;
                        *height = (*height * 10.0).round() / 10.0;
                    }
                }
                // The camera turns with a drag anywhere but on the handle.
                mode.orbit = !near_handle && !*dragging;
                for l in &shape.loops {
                    let lift = |p: &Point| frame.to_world(*p) + n * *height as f32;
                    let base: Vec<Vec3> = l.points.iter().map(|p| frame.to_world(*p)).collect();
                    let top: Vec<Vec3> = l.points.iter().map(lift).collect();
                    let color = theme::palette().accent_strong;
                    paint_world_loop(viewer, rect, &painter, &base, Stroke::new(1.5, color));
                    paint_world_loop(viewer, rect, &painter, &top, Stroke::new(1.5, color));
                    let every = (l.points.len() / 24).max(1);
                    for (k, (a, b)) in base.iter().zip(&top).enumerate() {
                        if k % every == 0 {
                            paint_world_segment(
                                viewer,
                                rect,
                                &painter,
                                *a,
                                *b,
                                Stroke::new(1.0, color.gamma_multiply(0.6)),
                            );
                        }
                    }
                }
                if let (Some(a), Some(b)) = (base_screen, tip_screen) {
                    let hot = near_handle || *dragging;
                    let color = if hot { theme::palette().warning } else { theme::palette().accent_strong };
                    painter.line_segment([a, b], Stroke::new(2.0, color));
                    painter.circle(b, if hot { 7.0 } else { 6.0 }, color, Stroke::new(1.5, Color32::WHITE));
                    let text = format!("{} mm", number(*height));
                    painter.text(
                        b + Vec2::new(12.0, -12.0),
                        Align2::LEFT_BOTTOM,
                        text,
                        FontId::proportional(13.0),
                        theme::palette().text,
                    );
                    if hot {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                }
                hint_bar(
                    &painter,
                    rect,
                    "Drag the handle or type the height · Enter: extrude · Esc: back to the sketch",
                );
            }
            Stage::Revolve { axis, .. } => {
                mode.orbit = true;
                let axis = *axis;
                paint_sketch(viewer, rect, &painter, mode, Some(axis));
                hint_bar(&painter, rect, "Enter: revolve around the highlighted line · Esc: back to the sketch");
            }
            Stage::Draw => {}
        }
        if response.hovered() {
            ctx.request_repaint();
        }
        action
    }
}

// ---- the panel's content ----

fn panel_ui(ui: &mut egui::Ui, mode: &mut SketchMode) -> Option<Action> {
    let mut action = None;
    let palette = theme::palette();
    ui.label(RichText::new("Sketch").strong().size(14.0).color(palette.text));
    ui.add_space(6.0);
    let (status, color) = mode.status();
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.0, color);
        ui.label(RichText::new(status).color(color));
    });
    ui.add_space(6.0);
    egui::Grid::new("sketch_identity").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(RichText::new("Name").color(palette.text_dim));
        if mode.reopened {
            ui.label(RichText::new(module_name(&mode.name)).monospace().color(palette.code_text));
        } else {
            ui.add(TextEdit::singleline(&mut mode.name).desired_width(160.0))
                .on_hover_text("The module is sketch_<name>");
        }
        ui.end_row();
        ui.label(RichText::new("Plane").color(palette.text_dim));
        ui.label(RichText::new(plane_name(&mode.sketch.plane)).color(palette.text));
        ui.end_row();
    });

    match mode.stage.clone() {
        Stage::Draw => draw_panel(ui, mode, &mut action),
        Stage::Extrude { .. } | Stage::Revolve { .. } => feature_panel(ui, mode, &mut action),
    }
    action
}

fn plane_name(plane: &Plane) -> &'static str {
    match plane {
        Plane::Xy => "XY",
        Plane::Xz => "XZ",
        Plane::Yz => "YZ",
        Plane::Face { .. } => "a face",
    }
}

/// The sketch's row of the toolbar; returns what it asks of the app.
fn toolbar_ui(ui: &mut egui::Ui, row: Row, mode: &mut SketchMode) -> Option<Action> {
    let mut action = None;
    let name = if mode.name.trim().is_empty() { "sketch".to_string() } else { mode.name.trim().to_string() };
    let detail = format!("{name} · {}", plane_name(&mode.sketch.plane));
    match mode.stage {
        Stage::Draw => {
            toolbar::context(ui, "SKETCH", &detail);
            toolbar::separator(ui);
            draw_tools(ui, row, mode);
            toolbar::separator(ui);
            constrain_tools(ui, row, mode);
        }
        Stage::Extrude { .. } => toolbar::context(ui, "EXTRUDE", &detail),
        Stage::Revolve { .. } => toolbar::context(ui, "REVOLVE", &detail),
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space(4.0);
        match mode.stage {
            Stage::Draw => {
                if toolbar::finish_button(ui, "Finish sketch", "Write the sketch into the code (Enter)").clicked() {
                    action = Some(Action::Finish);
                }
                let tip = "Leave without writing; Sketch, then Resume unfinished, takes it up again (Esc)";
                if ui.button("Cancel").on_hover_text(tip).clicked() {
                    action = Some(Action::Leave);
                }
            }
            _ => {
                let verb = if matches!(mode.stage, Stage::Extrude { .. }) { "Extrude" } else { "Revolve" };
                let tip = "Write the sketch and the feature into the code (Enter)";
                if toolbar::finish_button(ui, verb, tip).clicked() {
                    action = Some(Action::Apply);
                }
                if ui.button("Back").on_hover_text("Back to the sketch (Esc)").clicked() {
                    mode.stage = Stage::Draw;
                }
            }
        }
    });
    action
}

/// The DRAW group: the tools that draw, and Select.
fn draw_tools(ui: &mut egui::Ui, row: Row, mode: &mut SketchMode) {
    toolbar::group(ui, "DRAW", row, |ui| {
        for tool in Tool::ALL {
            let (icon, key) = match tool {
                Tool::Select => (Icon::Select, "S"),
                Tool::Line => (Icon::Line, "L"),
                Tool::Rectangle => (Icon::Rectangle, "R"),
                Tool::Circle => (Icon::Circle, "C"),
                Tool::Arc => (Icon::Arc, "A"),
            };
            let tip = format!("{} ({key})", tool.hint(0));
            if toolbar::tool(ui, row, icon, tool.name(), mode.drawing.tool == tool, Ok(()), &tip).clicked() {
                mode.drawing.set_tool(tool);
                mode.hint = None;
            }
        }
    });
}

/// The CONSTRAIN group: the rules on the selection, and the dimensions.
fn constrain_tools(ui: &mut egui::Ui, row: Row, mode: &mut SketchMode) {
    toolbar::group(ui, "CONSTRAIN", row, |ui| {
        for rule in Rule::ALL {
            let (icon, short) = match rule {
                Rule::Horizontal => (Icon::Horizontal, "Horiz."),
                Rule::Vertical => (Icon::Vertical, "Vert."),
                Rule::Parallel => (Icon::Parallel, "Parallel"),
                Rule::Perpendicular => (Icon::Perpendicular, "Perp."),
                Rule::Coincident => (Icon::Coincident, "Coincid."),
                Rule::Tangent => (Icon::Tangent, "Tangent"),
                Rule::Equal => (Icon::Equal, "Equal"),
                Rule::Fix => (Icon::Fix, "Fix"),
            };
            let tip = format!("{}: {}", rule.name(), rule.help());
            if toolbar::tool(ui, row, icon, short, false, Ok(()), &tip).clicked() {
                match view::constraints_for(rule, &mode.sketch, &mode.selection) {
                    Ok(made) => {
                        mode.sketch.constraints.extend(made);
                        mode.resolve();
                        mode.hint = None;
                    }
                    Err(help) => mode.hint = Some(help.to_string()),
                }
            }
        }
        let tip = "A length, a radius or an angle on the selection, with its value";
        let dimension = toolbar::tool(ui, row, Icon::Dimension, "Dimension", false, Ok(()), tip);
        egui::Popup::menu(&dimension).show(|ui| {
            for measure in Measure::ALL {
                if ui.button(measure.name()).on_hover_text(measure.help()).clicked() {
                    match view::dimension_for(measure, &mode.sketch, &mode.selection) {
                        Ok(constraint) => {
                            let at = ui.ctx().screen_rect().center() - Vec2::new(120.0, 40.0);
                            mode.prompt = Some(new_prompt(constraint, None, at));
                            mode.hint = None;
                        }
                        Err(help) => mode.hint = Some(help.to_string()),
                    }
                }
            }
        });
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).strong().color(theme::palette().text_dim));
}

fn draw_panel(ui: &mut egui::Ui, mode: &mut SketchMode, _action: &mut Option<Action>) {
    if let Some(hint) = &mode.hint {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(hint).color(theme::palette().warning)).wrap());
    }
    section(ui, "Selection");
    let entities: Vec<EntityId> =
        mode.selection.iter().filter_map(|i| if let Item::Entity(e) = i { Some(*e) } else { None }).collect();
    ui.horizontal(|ui| {
        let construction = ui
            .add_enabled(!entities.is_empty(), Button::new("Construction"))
            .on_hover_text("Construction geometry guides the drawing (an axis) and stays out of the profile")
            .on_disabled_hover_text("Select lines, circles or arcs");
        if construction.clicked() {
            let on = !entities.iter().all(|e| mode.sketch.entity(*e).construction);
            for e in &entities {
                mode.sketch.set_construction(*e, on);
            }
        }
        let delete =
            ui.add_enabled(!mode.selection.is_empty(), Button::new("Delete")).on_hover_text("Delete the selection (⌫)");
        if delete.clicked() {
            mode.sketch = view::remove(&mode.sketch, &mode.selection);
            mode.selection.clear();
            mode.renumbered();
        }
    });

    // The dimensions, with their values to change.
    let dimensions: Vec<usize> =
        (0..mode.sketch.constraints.len()).filter(|&i| mode.sketch.constraints[i].dimension().is_some()).collect();
    if !dimensions.is_empty() {
        section(ui, "Dimensions");
        let mut changed = None;
        let mut removed = None;
        for i in dimensions {
            let conflicting = mode.report.conflicting.iter().any(|c| c.0 == i);
            ui.horizontal(|ui| {
                let constraint = &mut mode.sketch.constraints[i];
                let label = match constraint.dimension().and_then(|d| d.name.clone()) {
                    Some(name) => name,
                    None => short(constraint),
                };
                let color = if conflicting { theme::palette().error } else { theme::palette().text };
                ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new(label).color(color)).truncate())
                    .on_hover_text(describe(constraint));
                let angle = matches!(constraint, Constraint::Angle(..));
                let d = constraint.dimension_mut().expect("a dimension");
                let speed = if angle { 0.5 } else { 0.1 };
                let drag = egui::DragValue::new(&mut d.value).speed(speed).suffix(if angle { "°" } else { " mm" });
                let range = if angle { -360.0..=360.0 } else { 0.001..=1e6 };
                if ui.add(drag.range(range)).changed() {
                    changed = Some((i, d.name.clone(), d.value));
                }
                if ui.small_button("🗑").on_hover_text("Remove this dimension").clicked() {
                    removed = Some(i);
                }
            });
        }
        if let Some((_, Some(name), value)) = &changed {
            mode.sketch.set_variable(name, *value);
        }
        if changed.is_some() {
            mode.resolve();
        }
        if let Some(i) = removed {
            mode.sketch.constraints.remove(i);
            mode.renumbered();
        }
    }

    // The other constraints, to remove one that is in the way.
    let others: Vec<usize> =
        (0..mode.sketch.constraints.len()).filter(|&i| mode.sketch.constraints[i].dimension().is_none()).collect();
    if !others.is_empty() {
        let mut removed = None;
        egui::CollapsingHeader::new(format!("Constraints ({})", others.len())).id_salt("sketch_constraints").show(
            ui,
            |ui| {
                for i in others {
                    let bad = mode.report.conflicting.iter().chain(&mode.report.unsatisfied).any(|c| c.0 == i);
                    ui.horizontal(|ui| {
                        let color = if bad { theme::palette().error } else { theme::palette().text_dim };
                        ui.label(RichText::new(describe(&mode.sketch.constraints[i])).color(color));
                        if ui.small_button("🗑").on_hover_text("Remove this constraint").clicked() {
                            removed = Some(i);
                        }
                    });
                }
            },
        );
        if let Some(i) = removed {
            mode.sketch.constraints.remove(i);
            mode.renumbered();
        }
    }

    ui.add_space(6.0);
    ui.separator();
    section(ui, "Then");
    ui.label(RichText::new("Finish sketch writes the profile alone, or:").size(12.0).color(theme::palette().text_dim));
    ui.horizontal(|ui| {
        let closed = !profile(&mode.sketch, 64).is_empty();
        let extrude = ui
            .add_enabled(closed, Button::new("Extrude…"))
            .on_hover_text("Finish and extrude the profile (linear_extrude)")
            .on_disabled_hover_text("Close a profile first");
        if extrude.clicked() {
            let cut = mode.cut_from.is_some() && matches!(mode.sketch.plane, Plane::Face { .. });
            let height = if cut { -5.0 } else { 10.0 };
            mode.stage = Stage::Extrude { height, name: String::new(), cut, dragging: false };
            mode.drawing.cancel();
        }
        let axis = match mode.selection.as_slice() {
            [Item::Entity(e)] if matches!(mode.sketch.entity(*e).geometry, Geometry::Line { .. }) => Some(*e),
            _ => None,
        };
        let revolve = ui
            .add_enabled(closed && axis.is_some(), Button::new("Revolve…"))
            .on_hover_text("Finish and revolve the profile around the selected line (rotate_extrude)")
            .on_disabled_hover_text("Close a profile and select the line to revolve around");
        if let (true, Some(axis)) = (revolve.clicked(), axis) {
            mode.stage = Stage::Revolve { axis, angle: 360.0, name: String::new(), cut: false };
            mode.drawing.cancel();
        }
    });
}

fn feature_panel(ui: &mut egui::Ui, mode: &mut SketchMode, _action: &mut Option<Action>) {
    let can_cut = mode.cut_from.is_some();
    let cut_help = "Cut it out of the object selected (or clicked) when the sketch began";
    let no_cut = "Select the object to cut before starting the sketch, or sketch on its face";
    let (title, verb) = match mode.stage {
        Stage::Extrude { .. } => ("Extrude", "Extrude"),
        _ => ("Revolve", "Revolve"),
    };
    section(ui, title);
    match &mut mode.stage {
        Stage::Extrude { height, name, cut, .. } => {
            ui.horizontal(|ui| {
                ui.label("Height");
                ui.add(egui::DragValue::new(height).speed(0.1).suffix(" mm"))
                    .on_hover_text("Negative goes the other way, below the plane");
            });
            variable_field(ui, name);
            ui.horizontal(|ui| {
                let was = *cut;
                ui.radio_value(cut, false, "Add");
                let radio = ui.add_enabled(can_cut, egui::RadioButton::new(*cut, "Cut"));
                if radio.on_hover_text(cut_help).on_disabled_hover_text(no_cut).clicked() {
                    *cut = true;
                }
                // Cutting goes into the object, adding away from it.
                if was != *cut && (*height > 0.0) == *cut {
                    *height = -*height;
                }
            });
        }
        Stage::Revolve { angle, name, cut, .. } => {
            ui.horizontal(|ui| {
                ui.label("Angle");
                ui.add(egui::DragValue::new(angle).speed(1.0).range(-360.0..=360.0).suffix("°"));
            });
            variable_field(ui, name);
            ui.horizontal(|ui| {
                ui.radio_value(cut, false, "Add");
                let radio = ui.add_enabled(can_cut, egui::RadioButton::new(*cut, "Cut"));
                if radio.on_hover_text(cut_help).on_disabled_hover_text(no_cut).clicked() {
                    *cut = true;
                }
            });
        }
        Stage::Draw => {}
    }
    ui.add_space(6.0);
    let hint = format!("{verb} writes the sketch and the feature; Back returns to the drawing.");
    ui.add(egui::Label::new(RichText::new(hint).size(12.0).color(theme::palette().text_dim)).wrap());
}

fn variable_field(ui: &mut egui::Ui, name: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Variable");
        ui.add(TextEdit::singleline(name).hint_text("optional, as depth").desired_width(130.0))
            .on_hover_text("Named, the value becomes a Customizer parameter");
    });
}

// ---- the dimension prompt ----

fn new_prompt(constraint: Constraint, editing: Option<usize>, at: Pos2) -> Prompt {
    let d = constraint.dimension().cloned().unwrap_or(Dimension::new(0.0));
    Prompt {
        constraint,
        editing,
        value: number(d.value),
        name: d.name.unwrap_or_default(),
        at,
        focus: true,
        error: None,
    }
}

fn open_prompt(mode: &mut SketchMode, index: usize, at: Pos2) {
    if let Some(c) = mode.sketch.constraints.get(index) {
        mode.prompt = Some(new_prompt(c.clone(), Some(index), at));
    }
}

/// The popup asking a dimension's value and optional variable: Enter
/// applies it, Esc drops it.
fn prompt_ui(ctx: &egui::Context, mode: &mut SketchMode) {
    let Some(prompt) = &mut mode.prompt else { return };
    let (mut ok, mut cancel) = (false, false);
    let kind = match prompt.constraint {
        Constraint::Length(..) => "Length",
        Constraint::Distance(..) => "Distance",
        Constraint::Radius(..) => "Radius",
        Constraint::Diameter(..) => "Diameter",
        Constraint::Angle(..) => "Angle (degrees)",
        _ => "Value",
    };
    egui::Window::new("sketch-dimension")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_pos(prompt.at)
        .show(ctx, |ui| {
            ui.label(RichText::new(kind).strong());
            egui::Grid::new("sketch-dimension-grid").num_columns(2).show(ui, |ui| {
                ui.label("Value");
                let value = ui.add(TextEdit::singleline(&mut prompt.value).desired_width(110.0));
                if std::mem::take(&mut prompt.focus) {
                    value.request_focus();
                }
                ui.end_row();
                ui.label("Variable");
                ui.add(TextEdit::singleline(&mut prompt.name).hint_text("optional").desired_width(110.0))
                    .on_hover_text(
                        "Named, the dimension becomes a Customizer parameter; dimensions of one name share it",
                    );
                ui.end_row();
            });
            if let Some(error) = &prompt.error {
                ui.label(RichText::new(error).color(theme::palette().error));
            }
            ui.horizontal(|ui| {
                ok = ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    ok |= ctx.input(|i| i.key_pressed(Key::Enter));
    cancel |= ctx.input(|i| i.key_pressed(Key::Escape));
    if cancel {
        mode.prompt = None;
        return;
    }
    if !ok {
        return;
    }
    let value: Option<f64> = prompt.value.trim().replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite());
    let angle = matches!(prompt.constraint, Constraint::Angle(..));
    let name = prompt.name.trim().to_string();
    let error = match value {
        None => Some("Type a number.".to_string()),
        Some(v) if !angle && v <= 0.0 => Some("A length is more than 0.".to_string()),
        _ if !name.is_empty() && !is_identifier(&name) => Some(format!("`{name}` cannot name a variable.")),
        _ => None,
    };
    if error.is_some() {
        prompt.error = error;
        return;
    }
    let value = value.expect("checked");
    let mut constraint = prompt.constraint.clone();
    *constraint.dimension_mut().expect("a dimension") =
        if name.is_empty() { Dimension::new(value) } else { Dimension::named(&name, value) };
    match prompt.editing {
        Some(i) if i < mode.sketch.constraints.len() => mode.sketch.constraints[i] = constraint,
        _ => {
            mode.sketch.constraints.push(constraint);
        }
    }
    if !name.is_empty() {
        mode.sketch.set_variable(&name, value);
    }
    mode.prompt = None;
    mode.resolve();
}

// ---- the pointer ----

/// Pans the view with a right or middle drag: the left button draws.
fn pan(viewer: &mut crate::viewer::Viewer, response: &egui::Response) {
    if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
        let delta = response.drag_delta();
        viewer.camera.pan(glam::Vec2::new(delta.x, delta.y), response.rect.height());
    }
}

/// Selecting and dragging with the Select tool.
fn select_input(
    ctx: &egui::Context,
    response: &egui::Response,
    mode: &mut SketchMode,
    cursor: Option<Point>,
    tolerance: f64,
) {
    let Some(at) = cursor else {
        mode.drag = None;
        return;
    };
    if mode.drag.is_none() {
        mode.hover = view::hit(&mode.sketch, at, tolerance);
    }
    if response.drag_started_by(PointerButton::Primary) {
        // Fixed points stay where they are: dragging a line with one
        // fixed end turns it around that end, and a fixed point alone
        // does not move.
        if let Some(points) = mode.hover.map(|item| view::drag_points(&mode.sketch, item)).filter(|p| !p.is_empty()) {
            mode.drag =
                Some(Drag { points: points.into_iter().map(|p| (p, mode.sketch.point(p))).collect(), from: at });
        }
    }
    if let Some(drag) = &mode.drag {
        if response.dragged_by(PointerButton::Primary) {
            // Each frame from where the drag began, holding the points
            // dragged: the rest of the sketch follows them.
            let mut moved = mode.sketch.clone();
            let (dx, dy) = (at.x - drag.from.x, at.y - drag.from.y);
            for &(p, start) in &drag.points {
                moved.points[p.0] = Point::new(start.x + dx, start.y + dy);
            }
            let options =
                SolveOptions { hold: drag.points.iter().map(|(p, _)| *p).collect(), ..SolveOptions::default() };
            let report = solve_with(&mut moved, &options);
            if report.converged() {
                mode.sketch = moved;
                mode.report = report;
            }
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        } else {
            mode.drag = None;
            mode.resolve();
        }
    } else if mode.hover.is_some() && response.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked_by(PointerButton::Primary) {
        let add = ctx.input(|i| i.modifiers.shift || i.modifiers.command);
        match (mode.hover, add) {
            (Some(item), true) => match mode.selection.iter().position(|i| *i == item) {
                Some(k) => {
                    mode.selection.remove(k);
                }
                None => mode.selection.push(item),
            },
            (Some(item), false) => mode.selection = vec![item],
            (None, true) => {}
            (None, false) => mode.selection.clear(),
        }
        mode.hint = None;
    }
}

// ---- the camera ----

/// Where the camera looks at the sketch from: its middle, from far
/// enough to see it whole even in a narrow view (or from where it is,
/// for an empty sketch).
fn framing(mode: &SketchMode, viewer: &crate::viewer::Viewer) -> (Point, f32) {
    let camera = &viewer.camera;
    match mode.extent() {
        Some((center, size)) => {
            let aspect = viewer.aspect().clamp(0.2, 1.0);
            (center, size as f32 * 0.8 / (camera.fov_y * 0.5).tan() / aspect)
        }
        None => (Point::default(), camera.distance),
    }
}

fn face_camera(mode: &SketchMode, viewer: &crate::viewer::Viewer) -> crate::camera::Camera {
    let (center, distance) = framing(mode, viewer);
    mode.frame.face_on(center, distance, viewer.camera.fov_y)
}

fn extrude_camera(mode: &SketchMode, viewer: &crate::viewer::Viewer) -> crate::camera::Camera {
    let (center, distance) = framing(mode, viewer);
    mode.frame.three_quarters(center, distance * 1.2, viewer.camera.fov_y)
}

/// How many points a millimeter of the plane covers on screen, around
/// the middle of the view.
fn pixels_per_mm(viewer: &crate::viewer::Viewer, rect: Rect, frame: &PlaneFrame) -> f32 {
    let ray = viewer.ray(rect, rect.center());
    let middle = frame.intersect(ray.origin, ray.direction).unwrap_or_default();
    let a = viewer.to_screen(rect, frame.to_world(middle));
    let b = viewer.to_screen(rect, frame.to_world(Point::new(middle.x + 1.0, middle.y)));
    match (a, b) {
        (Some(a), Some(b)) => a.distance(b).max(1e-3),
        _ => 1.0,
    }
}

// ---- painting ----

fn screen(viewer: &crate::viewer::Viewer, rect: Rect, frame: &PlaneFrame, p: Point) -> Option<Pos2> {
    viewer.to_screen(rect, frame.to_world(p))
}

/// The plane's grid: lines every 1, 2 or 5 times a power of ten
/// millimeters, at least 12 points apart, stronger every power of ten,
/// and the plane's axes in the colors of X and Y.
fn paint_grid(viewer: &crate::viewer::Viewer, rect: Rect, painter: &egui::Painter, frame: &PlaneFrame, ppm: f32) {
    let corners: Option<Vec<Point>> = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()]
        .into_iter()
        .map(|c| {
            let ray = viewer.ray(rect, c);
            frame.intersect(ray.origin, ray.direction)
        })
        .collect();
    let Some(corners) = corners else { return };
    let (lo, hi) = corners.iter().fold((corners[0], corners[0]), |(lo, hi), p| {
        (Point::new(lo.x.min(p.x), lo.y.min(p.y)), Point::new(hi.x.max(p.x), hi.y.max(p.y)))
    });
    // The same spacing a click snaps to.
    let (step, major) = view::grid_step(f64::from(12.0 / ppm));
    if (hi.x - lo.x) / step > 400.0 || (hi.y - lo.y) / step > 400.0 {
        return;
    }
    let line = |a: Point, b: Point, stroke: Stroke| {
        if let (Some(a), Some(b)) = (screen(viewer, rect, frame, a), screen(viewer, rect, frame, b)) {
            painter.line_segment([a, b], stroke);
        }
    };
    let minor = Stroke::new(1.0, Color32::from_white_alpha(9));
    let strong = Stroke::new(1.0, Color32::from_white_alpha(22));
    let kind = |i: i64| {
        if i == 0 {
            2
        } else if i % major as i64 == 0 {
            1
        } else {
            0
        }
    };
    for i in (lo.x / step).ceil() as i64..=(hi.x / step).floor() as i64 {
        let x = i as f64 * step;
        let stroke = match kind(i) {
            2 => Stroke::new(1.2, Color32::from_rgba_unmultiplied(102, 209, 97, 110)),
            1 => strong,
            _ => minor,
        };
        line(Point::new(x, lo.y), Point::new(x, hi.y), stroke);
    }
    for i in (lo.y / step).ceil() as i64..=(hi.y / step).floor() as i64 {
        let y = i as f64 * step;
        let stroke = match kind(i) {
            2 => Stroke::new(1.2, Color32::from_rgba_unmultiplied(240, 84, 90, 110)),
            1 => strong,
            _ => minor,
        };
        line(Point::new(lo.x, y), Point::new(hi.x, y), stroke);
    }
}

/// The points of an arc from `start` to `end` counterclockwise around
/// `center`, or of a full circle.
fn arc_points(center: Point, start: Point, end: Point) -> Vec<Point> {
    let r = center.distance(start);
    let a0 = (start.y - center.y).atan2(start.x - center.x);
    let mut sweep = (end.y - center.y).atan2(end.x - center.x) - a0;
    if sweep <= 1e-9 {
        sweep += std::f64::consts::TAU;
    }
    let n = ((sweep / std::f64::consts::TAU) * 96.0).ceil().max(8.0) as usize;
    (0..=n)
        .map(|k| {
            let a = a0 + sweep * k as f64 / n as f64;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect()
}

fn circle_points(center: Point, radius: f64) -> Vec<Point> {
    (0..=96)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 96.0;
            Point::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect()
}

fn paint_path(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    painter: &egui::Painter,
    frame: &PlaneFrame,
    points: &[Point],
    stroke: Stroke,
    dashed: bool,
) {
    let screen: Option<Vec<Pos2>> = points.iter().map(|p| screen(viewer, rect, frame, *p)).collect();
    let Some(screen) = screen else { return };
    if dashed {
        painter.extend(egui::Shape::dashed_line(&screen, stroke, 6.0, 4.0));
    } else {
        painter.add(egui::Shape::line(screen, stroke));
    }
}

fn paint_world_segment(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    painter: &egui::Painter,
    a: Vec3,
    b: Vec3,
    stroke: Stroke,
) {
    if let (Some(a), Some(b)) = (viewer.to_screen(rect, a), viewer.to_screen(rect, b)) {
        painter.line_segment([a, b], stroke);
    }
}

fn paint_world_loop(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    painter: &egui::Painter,
    points: &[Vec3],
    stroke: Stroke,
) {
    let screen: Option<Vec<Pos2>> = points.iter().map(|p| viewer.to_screen(rect, *p)).collect();
    if let Some(screen) = screen {
        painter.add(egui::Shape::closed_line(screen, stroke));
    }
}

/// The sketch: its entities and points in the colors of their state,
/// the selection and the item under the pointer brighter, the glyphs of
/// the constraints, and `axis` (a revolution's) highlighted.
fn paint_sketch(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    painter: &egui::Painter,
    mode: &SketchMode,
    axis: Option<EntityId>,
) {
    let frame = &mode.frame;
    let sketch = &mode.sketch;
    for e in 0..sketch.entities.len() {
        let id = EntityId(e);
        let item = Item::Entity(id);
        let entity = sketch.entities[e];
        let selected = mode.selection.contains(&item) || axis == Some(id);
        let hovered = mode.hover == Some(item);
        let color = if selected { theme::palette().warning } else { mode.color(item) };
        let width = if selected || hovered {
            3.0
        } else if entity.construction {
            1.2
        } else {
            2.0
        };
        let stroke = Stroke::new(width, if hovered && !selected { color.gamma_multiply(1.3) } else { color });
        let points = match entity.geometry {
            Geometry::Line { a, b } => vec![sketch.point(a), sketch.point(b)],
            Geometry::Circle { center, radius } => circle_points(sketch.point(center), radius),
            Geometry::Arc { center, start, end } => {
                arc_points(sketch.point(center), sketch.point(start), sketch.point(end))
            }
        };
        paint_path(viewer, rect, painter, frame, &points, stroke, entity.construction);
    }
    for (i, p) in sketch.points.iter().enumerate() {
        let item = Item::Point(PointId(i));
        let Some(at) = screen(viewer, rect, frame, *p) else { continue };
        let selected = mode.selection.contains(&item);
        let hovered = mode.hover == Some(item);
        let color = if selected { theme::palette().warning } else { mode.color(item) };
        let radius = if selected || hovered { 4.5 } else { 3.0 };
        painter.circle(at, radius, color, Stroke::new(1.0, theme::palette().bg));
    }
    paint_glyphs(viewer, rect, painter, mode);
}

/// A small letter for each constraint that is not a dimension, beside
/// the entity (or point) it holds.
fn paint_glyphs(viewer: &crate::viewer::Viewer, rect: Rect, painter: &egui::Painter, mode: &SketchMode) {
    let sketch = &mode.sketch;
    let frame = &mode.frame;
    let mut count = vec![0usize; sketch.entities.len() + sketch.points.len()];
    let bad: Vec<usize> = mode.report.conflicting.iter().chain(&mode.report.unsatisfied).map(|c| c.0).collect();
    for (i, c) in sketch.constraints.iter().enumerate() {
        // A constraint naming something gone is the solver's to skip,
        // and ours: its ids would index past the end.
        if sketch.check(ConstraintId(i)).is_err() {
            continue;
        }
        let glyph = match c {
            Constraint::Horizontal(_) => "H",
            Constraint::Vertical(_) => "V",
            Constraint::Parallel(..) => "//",
            Constraint::Perpendicular(..) => "_|_",
            Constraint::Tangent(..) => "T",
            Constraint::Equal(..) => "=",
            Constraint::Fixed(_) => "F",
            Constraint::Midpoint(..) => "M",
            Constraint::PointOnLine(..) | Constraint::PointOnCircle(..) => "o",
            Constraint::Coincident(..) => "•",
            _ => continue,
        };
        let color = if bad.contains(&i) { theme::palette().error } else { theme::palette().text_dim };
        for item in view::constraint_items(sketch, c) {
            // Point constraints on a point; the others on their entities.
            let point_glyph = matches!(c, Constraint::Fixed(_) | Constraint::Coincident(..));
            let (slot, anchor, normal) = match item {
                Item::Entity(e) if !point_glyph => {
                    let Some((mid, normal)) = entity_anchor(viewer, rect, frame, sketch, e) else { continue };
                    (e.0, mid, normal)
                }
                Item::Point(p) if point_glyph => {
                    let Some(at) = screen(viewer, rect, frame, sketch.point(p)) else { continue };
                    (sketch.entities.len() + p.0, at, Vec2::new(0.7, 0.7))
                }
                _ => continue,
            };
            let k = count[slot];
            count[slot] += 1;
            let tangent = Vec2::new(-normal.y, normal.x);
            let at = anchor + normal * 13.0 + tangent * (k as f32 * 14.0);
            painter.text(at, Align2::CENTER_CENTER, glyph, FontId::proportional(11.0), color);
        }
    }
}

/// The middle of an entity on screen and the direction across it (for
/// a circle, its top and up).
fn entity_anchor(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    frame: &PlaneFrame,
    sketch: &Sketch,
    e: EntityId,
) -> Option<(Pos2, Vec2)> {
    match sketch.entities.get(e.0)?.geometry {
        Geometry::Line { a, b } => {
            let (a, b) = (screen(viewer, rect, frame, sketch.point(a))?, screen(viewer, rect, frame, sketch.point(b))?);
            let along = (b - a).normalized();
            Some((a + (b - a) * 0.5, Vec2::new(along.y, -along.x)))
        }
        Geometry::Circle { center, radius } => {
            let c = sketch.point(center);
            let top = screen(viewer, rect, frame, Point::new(c.x, c.y + radius))?;
            Some((top, Vec2::new(0.0, -1.0)))
        }
        Geometry::Arc { center, start, end } => {
            let points = arc_points(sketch.point(center), sketch.point(start), sketch.point(end));
            let mid = points[points.len() / 2];
            let (c, m) = (screen(viewer, rect, frame, sketch.point(center))?, screen(viewer, rect, frame, mid)?);
            Some((m, (m - c).normalized()))
        }
    }
}

/// A dimension's label on screen: its text, where, and the constraint.
struct Label {
    constraint: usize,
    text: String,
    rect: Rect,
    color: Color32,
}

fn dimension_labels(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    painter: &egui::Painter,
    mode: &SketchMode,
) -> Vec<Label> {
    let sketch = &mode.sketch;
    let frame = &mode.frame;
    let mut labels = Vec::new();
    for (i, c) in sketch.constraints.iter().enumerate() {
        let Some(d) = c.dimension() else { continue };
        if sketch.check(ConstraintId(i)).is_err() {
            continue;
        }
        let value = number(d.value);
        let shown = match c {
            Constraint::Radius(..) => format!("R{value}"),
            Constraint::Diameter(..) => format!("⌀{value}"),
            Constraint::Angle(..) => format!("{value}°"),
            _ => value,
        };
        let text = match &d.name {
            Some(name) => format!("{name} = {shown}"),
            None => shown,
        };
        let anchor = match c {
            Constraint::Length(e, _) | Constraint::Radius(e, _) | Constraint::Diameter(e, _) => {
                entity_anchor(viewer, rect, frame, sketch, *e).map(|(at, normal)| at - normal * 16.0)
            }
            Constraint::Angle(a, _, _) => {
                entity_anchor(viewer, rect, frame, sketch, *a).map(|(at, normal)| at - normal * 30.0)
            }
            Constraint::Distance(a, b, _) => {
                let middle = sketch.point(*a).midpoint(sketch.point(*b));
                screen(viewer, rect, frame, middle).map(|at| at + Vec2::new(0.0, -14.0))
            }
            _ => None,
        };
        let Some(at) = anchor else { continue };
        let galley = painter.layout_no_wrap(text.clone(), FontId::proportional(12.0), theme::palette().text);
        let size = galley.size() + Vec2::new(10.0, 4.0);
        let conflicting = mode.report.conflicting.iter().any(|k| k.0 == i);
        let color = if conflicting { theme::palette().error } else { theme::palette().text };
        labels.push(Label { constraint: i, text, rect: Rect::from_center_size(at, size), color });
    }
    labels
}

fn paint_label(painter: &egui::Painter, label: &Label) {
    painter.rect(
        label.rect,
        CornerRadius::same(4),
        Color32::from_black_alpha(190),
        Stroke::new(1.0, label.color.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    painter.text(label.rect.center(), Align2::CENTER_CENTER, &label.text, FontId::proportional(12.0), label.color);
}

fn paint_preview(
    viewer: &crate::viewer::Viewer,
    rect: Rect,
    painter: &egui::Painter,
    frame: &PlaneFrame,
    preview: Preview,
) {
    let stroke = Stroke::new(1.5, theme::palette().accent_strong.gamma_multiply(0.8));
    let points = match preview {
        Preview::Line(a, b) => {
            if let Some(axis) = view::auto_axis(a, b) {
                if let Some(at) = screen(viewer, rect, frame, a.midpoint(b)) {
                    let glyph = if axis == view::Axis::Horizontal { "H" } else { "V" };
                    painter.text(
                        at + Vec2::new(8.0, -10.0),
                        Align2::LEFT_BOTTOM,
                        glyph,
                        FontId::proportional(11.0),
                        theme::palette().text_dim,
                    );
                }
            }
            vec![a, b]
        }
        Preview::Circle(c, r) => circle_points(c, r),
        Preview::Arc(c, s, e) => arc_points(c, s, e),
    };
    paint_path(viewer, rect, painter, frame, &points, stroke, false);
}

/// The mark of a snap: a square on a point, a circle on a center, a
/// triangle on a midpoint, a cross on the origin.
fn paint_pick(viewer: &crate::viewer::Viewer, rect: Rect, painter: &egui::Painter, frame: &PlaneFrame, pick: &Pick) {
    let Some(at) = screen(viewer, rect, frame, pick.at) else { return };
    let stroke = Stroke::new(1.5, theme::palette().warning);
    match pick.snap {
        Some(Target::Sketch(snap)) => match snap.kind {
            SnapKind::Endpoint => {
                painter.rect_stroke(
                    Rect::from_center_size(at, Vec2::splat(10.0)),
                    CornerRadius::ZERO,
                    stroke,
                    StrokeKind::Middle,
                );
            }
            SnapKind::Center => {
                painter.circle_stroke(at, 6.0, stroke);
            }
            SnapKind::Midpoint => {
                let points = vec![at + Vec2::new(0.0, -6.0), at + Vec2::new(5.5, 4.0), at + Vec2::new(-5.5, 4.0)];
                painter.add(egui::Shape::closed_line(points, stroke));
            }
        },
        Some(Target::Origin) => {
            painter.line_segment([at - Vec2::splat(6.0), at + Vec2::splat(6.0)], stroke);
            painter.line_segment([at + Vec2::new(-6.0, 6.0), at + Vec2::new(6.0, -6.0)], stroke);
        }
        None => {
            painter.circle_filled(at, 2.0, theme::palette().accent_strong);
        }
    }
}

/// A line of help at the bottom of the view.
pub(super) fn hint_bar(painter: &egui::Painter, rect: Rect, text: &str) {
    let palette = theme::palette();
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.5), palette.text);
    let size = galley.size() + Vec2::new(20.0, 8.0);
    // Above the mouse's help.
    let bottom = rect.bottom() - 40.0;
    let card = Rect::from_center_size(Pos2::new(rect.center().x, bottom), size);
    painter.rect(card, CornerRadius::same(6), palette.window, Stroke::new(1.0, palette.border), StrokeKind::Inside);
    painter.galley(card.min + Vec2::new(10.0, 4.0), galley, theme::palette().text);
}

// ---- words ----

fn name_of(item: Item, sketch: &Sketch) -> String {
    match item {
        Item::Point(p) => format!("p{}", p.0),
        Item::Entity(e) => {
            let letter = match sketch.entities.get(e.0).map(|e| e.geometry) {
                Some(Geometry::Circle { .. }) => 'c',
                Some(Geometry::Arc { .. }) => 'a',
                _ => 'l',
            };
            format!("{letter}{}", e.0)
        }
    }
}

/// A constraint in words, with its items as the `@sketch` lines name them.
fn describe(c: &Constraint) -> String {
    let what = short(c);
    match c.dimension() {
        Some(d) => match &d.name {
            Some(name) => format!("{what} = {name} ({})", number(d.value)),
            None => format!("{what} = {}", number(d.value)),
        },
        None => what,
    }
}

/// A constraint's kind and items, as `Length l0` or `Parallel l1 l3`.
fn short(c: &Constraint) -> String {
    let empty = Sketch::default();
    let items: Vec<String> = view::constraint_items(&empty, c)
        .into_iter()
        .map(|item| match item {
            // Without the sketch: lines and round entities alike as `e`.
            Item::Entity(e) => format!("e{}", e.0),
            point => name_of(point, &empty),
        })
        .collect();
    let kind = match c {
        Constraint::Coincident(..) => "Coincident",
        Constraint::Horizontal(_) => "Horizontal",
        Constraint::Vertical(_) => "Vertical",
        Constraint::Parallel(..) => "Parallel",
        Constraint::Perpendicular(..) => "Perpendicular",
        Constraint::Tangent(..) => "Tangent",
        Constraint::Equal(..) => "Equal",
        Constraint::Distance(..) => "Distance",
        Constraint::Length(..) => "Length",
        Constraint::Radius(..) => "Radius",
        Constraint::Diameter(..) => "Diameter",
        Constraint::Angle(..) => "Angle",
        Constraint::PointOnLine(..) => "On line",
        Constraint::PointOnCircle(..) => "On circle",
        Constraint::Midpoint(..) => "Midpoint",
        Constraint::Fixed(_) => "Fixed",
    };
    format!("{kind} {}", items.join(" "))
}

/// A height or angle as written: to a hundredth.
fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
