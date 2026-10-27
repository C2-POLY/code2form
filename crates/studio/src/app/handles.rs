//! The move, turn and scale handles (C2F-39), drawn on the selected object
//! in the 3D view, written back into the code as `translate`, `rotate` and
//! `scale`.
//!
//! The handles show on a single selected object written in the main file,
//! at the centre of its box ([`crate::gizmo`] has their geometry). Pressing
//! one starts a gesture: the object's highlight follows the drag as a
//! preview, the amount shows by the pointer (« 12 mm », « 45° », « ×1.5 »),
//! snapped to the steps of the bar at the top of the view unless Shift is
//! held, or typed on the keyboard (digits, then Enter). Releasing writes
//! the change into the code as one undo step, and the design renders
//! again; Escape gives the gesture up.
//!
//! # The code and the model
//!
//! The edits of [`crate::code_edit`] keep an object's transforms in the
//! usual order, `translate(T) rotate(R) scale(S) object`, so that a point
//! `p` of the object lands at `T + R·S·p`, `R` being `rotate()`'s turn
//! around X, then Y, then Z (see [`gizmo::rotation`]). The handles turn
//! and scale about the centre `c` of the object's box, while the code turns
//! and scales about `T`, so each such change also moves the object to put
//! `c` back:
//!
//! - moving by `d` is `T + d` ([`code_edit::translate_by`]); along the
//!   object's own axis `k`, `d` is `R·eₖ` times the distance;
//! - turning by `ΔR` (around a world axis, or around the object's own axis
//!   `R·eₖ`, which is the same as `R·Rₖ`) makes the turn `R' = ΔR·R`,
//!   written as one `rotate` ([`gizmo::euler`]), and moves the object by
//!   `(I − ΔR)·(c − T)`;
//! - scaling by factors `F` along the object's own axes makes the scale
//!   `F·S` ([`code_edit::scale_by`]) and moves the object by
//!   `(c − T) − R·F·Rᵀ·(c − T)`.
//!
//! Each of these does to the points of the model exactly what
//! [`gizmo::Change::matrix`] does, which the preview shows. They assume
//! the object's chain holds one `rotate`, inside its `translate`, and that
//! the statements around it do not turn or scale it: a `translate` around
//! the whole group is fine, a `rotate` around it would turn the handles'
//! axes away from the code's.

use std::ops::Range;

use eframe::egui::{self, Color32, Key, Pos2, Rect, Shape, Stroke, Vec2};
use glam::{DMat3, DVec3, Vec3};

use super::StudioApp;
use crate::code_edit::{self, Edit, Transform};
use crate::console::Level;
use crate::gizmo::{self, Change, Gizmo, Handle, Mark};
use crate::select::{Picking, Statement};
use crate::theme;
use crate::viewer::Viewer;

/// The colors of the axes, as the view's compass draws them.
const AXIS_COLORS: [Color32; 3] =
    [Color32::from_rgb(240, 84, 90), Color32::from_rgb(102, 209, 97), Color32::from_rgb(92, 143, 255)];
const WORLD: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
/// Scale factors snap to tenths.
const SCALE_STEP: f64 = 0.1;

/// How the handles work, for every tab.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Settings {
    /// Along the object's own axes rather than the world's.
    local: bool,
    /// The snapping steps of moves (mm) and turns (degrees); 0 for none.
    step_mm: f64,
    step_degrees: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { local: false, step_mm: 1.0, step_degrees: 15.0 }
    }
}

/// Where the code puts the object: its `translate` and its `rotate`, and
/// how far the statements around it move it, or why they cannot be read.
#[derive(Clone, Debug)]
struct Pose {
    translate: Result<DVec3, String>,
    rotation: Result<DMat3, String>,
    parents: Result<DVec3, String>,
}

impl Pose {
    fn read(src: &str, range: &Range<usize>, picking: Option<&Picking>, key: Statement) -> Pose {
        let read = |kind| code_edit::transform_of(src, range.clone(), kind).map(DVec3::from);
        Pose {
            translate: read(Transform::Translate),
            rotation: read(Transform::Rotate).map(|v| gizmo::rotation(v.to_array())),
            parents: match picking {
                Some(picking) => parents_offset(src, picking, key),
                None => Err(String::new()),
            },
        }
    }

    /// The object's own axes, the world's when its turn cannot be read.
    fn own_axes(&self) -> [Vec3; 3] {
        let r = self.rotation.as_ref().copied().unwrap_or(DMat3::IDENTITY);
        [0, 1, 2].map(|k| r.col(k).as_vec3())
    }
}

/// The builtins an object's placement climbs through when it is their only
/// child, as [`crate::code_edit`] climbs them: the object's own transforms,
/// or the inside of an extrusion, never transforms around it.
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

/// Statements around an object that leave it where it is.
const GROUPS: [&str; 9] =
    ["union", "difference", "intersection", "hull", "minkowski", "group", "color", "render", "children"];

const AROUND: &str = "This object is turned or scaled by the code around it: change it in the code.";
const COPIES: &str = "This statement makes several objects (in a loop or a module): change it in the code.";

/// How far the statements around the object at `key` move it, beyond its
/// own transforms: the sum of the `translate`s it sits in. Refused when one
/// of them turns or scales it, or when the statement runs more than once
/// or is the child of a module (which places it as it likes).
fn parents_offset(src: &str, picking: &Picking, key: Statement) -> Result<DVec3, String> {
    let mut runs = picking.origins.iter().enumerate().filter(|(_, o)| o.unit == key.unit && o.span == key.span);
    let Some((first, _)) = runs.next() else { return Err(String::new()) };
    if runs.next().is_some() {
        return Err(COPIES.into());
    }
    let mut offset = DVec3::ZERO;
    let mut climbing = true;
    let own = key.span.range();
    let contains = |outer: &Range<usize>, inner: &Range<usize>| outer.start <= inner.start && inner.end <= outer.end;
    // The statement last climbed through. A parent that does not hold it
    // in its text is in the body of a module the climb went into (through
    // its `children()`): it places the object, it is not its own chain.
    let mut inner = own.clone();
    let mut at = picking.origins[first].parent;
    while let Some(index) = at {
        let origin = &picking.origins[index as usize];
        at = origin.parent;
        if origin.kind == c2f_eval::CallKind::Flow {
            continue;
        }
        if !origin.in_main_file() {
            return Err(AROUND.into());
        }
        let range = origin.span.range();
        if origin.kind == c2f_eval::CallKind::Module {
            // Its children are written inside the call, its body elsewhere:
            // an object written in the call is one of its children, which
            // the module places as it likes. Checked against the object's
            // own text, as the climb may have gone through the body.
            if contains(&range, &own) {
                return Err(AROUND.into());
            }
            climbing = false;
            inner = range;
            continue;
        }
        let call = code_edit::call_at(src, range.clone())?;
        if !contains(&range, &inner) {
            climbing = false;
        }
        inner = range;
        if climbing && WRAPPERS.contains(&call.name.as_str()) && call.children.len() == 1 {
            continue;
        }
        climbing = false;
        match call.name.as_str() {
            "translate" => {
                let v = match call.args.first().map(|a| &a.value) {
                    None => Vec::new(),
                    Some(code_edit::ArgValue::Literal(code_edit::Value::Vector(v))) if v.len() <= 3 => v.clone(),
                    Some(_) => return Err(AROUND.into()),
                };
                for (k, x) in v.iter().enumerate() {
                    offset[k] += x;
                }
            }
            name if GROUPS.contains(&name) => {}
            _ => return Err(AROUND.into()),
        }
    }
    Ok(offset)
}

/// How a gesture reads the drag, from where it was pressed.
#[derive(Clone, Copy, Debug)]
enum Start {
    /// The position along the dragged axis at the press.
    Along(f32),
    /// Where the press met the dragged plane.
    InPlane(Vec3),
    /// Round a ring: the pointer last seen, and the angle turned so far.
    Round { last: Pos2, degrees: f32 },
    /// The press's distance from the centre on screen, in points: the
    /// scale grows as the pointer goes further out.
    Away(f32),
    /// Held at a set change, for a scripted screenshot.
    Held,
}

/// A handle being dragged.
#[derive(Clone, Debug)]
struct Gesture {
    handle: Handle,
    key: Statement,
    range: Range<usize>,
    /// The handles as they were at the press.
    gizmo: Gizmo,
    translate: DVec3,
    rotation: DMat3,
    start: Start,
    /// What was typed for an exact value.
    typed: String,
    change: Change,
    /// The amount shown by the pointer.
    label: String,
}

/// The handles' state in one tab.
#[derive(Clone, Debug, Default)]
struct State {
    /// The handle under the pointer.
    hot: Option<Handle>,
    gesture: Option<Gesture>,
    /// The pose last read, for the code (a hash of it) and the statement
    /// it was read for.
    pose: Option<(u64, Statement, Pose)>,
    /// The settings bar, as last drawn, which the handles stay clear of.
    bar: Option<Rect>,
    /// The scripted gesture of a screenshot was set up.
    staged: bool,
}

impl State {
    /// The pose of the object `key` at `range` of `src`, read again only
    /// when they changed.
    fn pose(&mut self, src: &str, range: &Range<usize>, viewer: &Viewer, key: Statement) -> Pose {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        src.hash(&mut hasher);
        let hash = hasher.finish();
        match &self.pose {
            Some((h, k, pose)) if *h == hash && *k == key => pose.clone(),
            _ => {
                let pose = Pose::read(src, range, viewer.picking().map(|p| &**p), key);
                self.pose = Some((hash, key, pose.clone()));
                pose
            }
        }
    }
}

/// The handles on the selection of `viewer`, shown in `rect`.
fn gizmo_for(viewer: &Viewer, rect: Rect, pose: &Pose, settings: &Settings) -> Option<Gizmo> {
    let (lo, hi) = viewer.selection_bounds()?;
    let origin = (lo + hi) * 0.5;
    let own = pose.own_axes();
    let axes = if settings.local { own } else { WORLD };
    let right = viewer.camera.view().row(0).truncate();
    let unit = gizmo::unit_at(|p| viewer.to_screen(rect, p), origin, right)?;
    Some(Gizmo::new(origin, axes, own, unit))
}

impl Gesture {
    /// Starts dragging `handle`, pressed at `press`; refused with the
    /// reason when the code computes a value the gesture would change (an
    /// empty reason when the drag cannot start from there).
    #[allow(clippy::too_many_arguments)]
    fn start(
        handle: Handle,
        press: Pos2,
        viewer: &Viewer,
        rect: Rect,
        gizmo: Gizmo,
        pose: &Pose,
        key: Statement,
        range: Range<usize>,
    ) -> Result<Gesture, String> {
        // Moving along world axes does without the turn; everything needs
        // the translate, which a move rewrites and the others shift.
        let local = gizmo.axes != WORLD;
        let rotation = match &pose.rotation {
            Ok(r) => *r,
            Err(why) if local || !matches!(handle, Handle::Move(_) | Handle::Plane(_)) => return Err(why.clone()),
            Err(_) => DMat3::IDENTITY,
        };
        // Where the code turns and scales about, in the model's space.
        let translate = pose.translate.clone()? + pose.parents.clone()?;
        let ray = viewer.ray(rect, press);
        let start = match handle {
            Handle::Move(k) => {
                Start::Along(gizmo::axis_param(ray.origin, ray.direction, gizmo.origin, gizmo.axes[k]).ok_or_default()?)
            }
            Handle::Scale(k) => Start::Along(
                gizmo::axis_param(ray.origin, ray.direction, gizmo.origin, gizmo.own_axes[k]).ok_or_default()?,
            ),
            Handle::Plane(k) => Start::InPlane(
                gizmo::plane_point(ray.origin, ray.direction, gizmo.origin, gizmo.axes[k]).ok_or_default()?,
            ),
            Handle::Turn(_) => Start::Round { last: press, degrees: 0.0 },
            Handle::ScaleAll => {
                let center = viewer.to_screen(rect, gizmo.origin).ok_or_default()?;
                Start::Away(center.distance(press))
            }
        };
        let change = match handle {
            Handle::Move(_) | Handle::Plane(_) => Change::Move(DVec3::ZERO),
            Handle::Turn(k) => Change::Turn { axis: gizmo.axes[k].as_dvec3(), degrees: 0.0 },
            Handle::Scale(_) | Handle::ScaleAll => Change::Scale([1.0; 3]),
        };
        let mut gesture = Gesture {
            handle,
            key,
            range,
            gizmo,
            translate,
            rotation,
            start,
            typed: String::new(),
            change,
            label: String::new(),
        };
        gesture.label = gesture.describe(&[0.0, 0.0]);
        Ok(gesture)
    }

    /// Follows the pointer at `pointer`; `snapping` rounds to the steps.
    fn drag(&mut self, viewer: &Viewer, rect: Rect, pointer: Pos2, snapping: bool, settings: &Settings) {
        let g = self.gizmo;
        let ray = viewer.ray(rect, pointer);
        let step = |s: f64| if snapping { s } else { 0.0 };
        let typed: Option<f64> = self.typed.parse().ok();
        let amounts: [f64; 2] = match (self.handle, &mut self.start) {
            (Handle::Move(k), Start::Along(t0)) => {
                let Some(t) = gizmo::axis_param(ray.origin, ray.direction, g.origin, g.axes[k]) else { return };
                let d = typed.unwrap_or_else(|| gizmo::snap(f64::from(t - *t0), step(settings.step_mm)));
                self.change = Change::Move(g.axes[k].as_dvec3() * d);
                [d, 0.0]
            }
            (Handle::Plane(k), Start::InPlane(p0)) => {
                let Some(p) = gizmo::plane_point(ray.origin, ray.direction, g.origin, g.axes[k]) else { return };
                let (b, c) = (g.axes[(k + 1) % 3], g.axes[(k + 2) % 3]);
                let delta = p - *p0;
                let (u, v) = (
                    gizmo::snap(f64::from(delta.dot(b)), step(settings.step_mm)),
                    gizmo::snap(f64::from(delta.dot(c)), step(settings.step_mm)),
                );
                self.change = Change::Move(b.as_dvec3() * u + c.as_dvec3() * v);
                [u, v]
            }
            (Handle::Turn(k), Start::Round { last, degrees: turned }) => {
                let Some(center) = viewer.to_screen(rect, g.origin) else { return };
                let axis = g.axes[k];
                // Bit by bit, to go round more than half a turn: in the
                // ring's plane, or round its centre on screen when seen
                // edge-on.
                let before = viewer.ray(rect, *last);
                let in_plane = |r: &crate::pick::Ray| gizmo::plane_point(r.origin, r.direction, g.origin, axis);
                let facing = ray.direction.normalize_or_zero().dot(axis).abs() > 0.2;
                *turned += match (facing, in_plane(&before), in_plane(&ray)) {
                    (true, Some(a), Some(b)) => gizmo::angle_around(axis, a - g.origin, b - g.origin),
                    _ => {
                        let toward_eye = viewer.camera.eye() - g.origin;
                        gizmo::turn_from_screen(gizmo::screen_angle(center, *last, pointer), axis, toward_eye)
                    }
                };
                *last = pointer;
                let degrees = typed.unwrap_or_else(|| gizmo::snap(f64::from(*turned), step(settings.step_degrees)));
                self.change = Change::Turn { axis: g.axes[k].as_dvec3(), degrees };
                [degrees, 0.0]
            }
            (Handle::Scale(k), Start::Along(t0)) => {
                let Some(t) = gizmo::axis_param(ray.origin, ray.direction, g.origin, g.own_axes[k]) else { return };
                let f = typed.unwrap_or_else(|| scale_snap(gizmo::scale_factor(*t0, t), snapping)).max(0.01);
                let mut factors = [1.0; 3];
                factors[k] = f;
                self.change = Change::Scale(factors);
                [f, 0.0]
            }
            (Handle::ScaleAll, Start::Away(d0)) => {
                let Some(center) = viewer.to_screen(rect, g.origin) else { return };
                // Twice as big once as far again out as the scale squares.
                let away = gizmo::SCALE_AT + center.distance(pointer) - *d0;
                let f =
                    typed.unwrap_or_else(|| scale_snap(gizmo::scale_factor(gizmo::SCALE_AT, away), snapping)).max(0.01);
                self.change = Change::Scale([f; 3]);
                [f, 0.0]
            }
            _ => return,
        };
        self.label = self.describe(&amounts);
    }

    /// The amount shown by the pointer: what was typed while typing.
    fn describe(&self, amounts: &[f64; 2]) -> String {
        let typed = (!self.typed.is_empty()).then(|| format!("{}|", self.typed));
        let n = |i: usize| typed.clone().unwrap_or_else(|| code_edit::number(amounts[i]));
        match self.handle {
            Handle::Move(_) => format!("{} mm", n(0)),
            Handle::Plane(_) => format!("{} mm, {} mm", code_edit::number(amounts[0]), code_edit::number(amounts[1])),
            Handle::Turn(_) => format!("{}°", n(0)),
            Handle::Scale(_) | Handle::ScaleAll => format!("×{}", n(0)),
        }
    }

    /// Whether typing an exact value applies to this handle.
    fn takes_typing(&self) -> bool {
        !matches!(self.handle, Handle::Plane(_))
    }

    /// The edit of `src` the gesture makes, `None` when it changes
    /// nothing.
    fn edit(&self, src: &str) -> Option<Result<Edit, String>> {
        (!self.change.is_identity()).then(|| {
            write_change(
                src,
                self.range.clone(),
                self.change,
                self.gizmo.origin.as_dvec3(),
                self.translate,
                self.rotation,
            )
        })
    }

    /// The preview of the change, a transform of the model's points.
    fn preview(&self) -> glam::Mat4 {
        self.change.preview(self.gizmo.origin.as_dvec3(), self.rotation)
    }
}

/// A scale factor snapped to tenths, never to nothing.
fn scale_snap(f: f64, snapping: bool) -> f64 {
    if snapping { gizmo::snap(f, SCALE_STEP).max(SCALE_STEP) } else { f }
}

trait OkOrDefault<T> {
    /// `None` as an empty refusal: the drag cannot start there.
    fn ok_or_default(self) -> Result<T, String>;
}

impl<T> OkOrDefault<T> for Option<T> {
    fn ok_or_default(self) -> Result<T, String> {
        self.ok_or_else(String::new)
    }
}

/// Writes `change`, made about `pivot`, into the code of the object at
/// `range`, whose `translate` is `translate` and whose `rotate` turns as
/// `rotation`: see the module's documentation for the arithmetic.
fn write_change(
    src: &str,
    range: Range<usize>,
    change: Change,
    pivot: DVec3,
    translate: DVec3,
    rotation: DMat3,
) -> Result<Edit, String> {
    let offset = pivot - translate;
    let (edit, shift) = match change {
        Change::Move(d) => return code_edit::translate_by(src, range, d.to_array()),
        Change::Turn { axis, degrees } => {
            let turn = gizmo::axis_rotation(axis, degrees);
            let angles = gizmo::euler(turn * rotation);
            (code_edit::set_transform(src, range, Transform::Rotate, angles)?, offset - turn * offset)
        }
        Change::Scale(factors) => {
            let scaled = rotation * DMat3::from_diagonal(DVec3::from(factors)) * rotation.transpose();
            (code_edit::scale_by(src, range, factors)?, offset - scaled * offset)
        }
    };
    // A shift too small to write would only add `translate([0, 0, 0])`.
    if shift.abs().max_element() < 5e-4 {
        return Ok(edit);
    }
    let moved = code_edit::translate_by(&edit.text, edit.selection.clone(), shift.to_array())?;
    Ok(moved)
}

/// What the console says once a gesture is written.
fn done_message(change: Change) -> String {
    match change {
        Change::Move(d) => format!("Moved by {}", code_edit::vector(&d.to_array())),
        Change::Turn { degrees, .. } => format!("Turned by {}°", code_edit::number(degrees)),
        Change::Scale(f) => format!("Scaled by {}", code_edit::vector(&f)),
    }
}

/// A gesture held at a set change, as `CODE2FORM_SCREENSHOT_HANDLE` asks
/// for a screenshot: `move-x:12`, `turn-z:45`, `scale-y:1.5` or
/// `scale:2`.
fn staged(spec: &str) -> Option<(Handle, f64)> {
    let (what, value) = spec.split_once(':')?;
    let value: f64 = value.trim().parse().ok()?;
    let axis = |name: &str| ["x", "y", "z"].iter().position(|a| *a == name);
    let handle = match what.split_once('-') {
        Some(("move", a)) => Handle::Move(axis(a)?),
        Some(("turn", a)) => Handle::Turn(axis(a)?),
        Some(("scale", a)) => Handle::Scale(axis(a)?),
        None if what == "scale" => Handle::ScaleAll,
        _ => return None,
    };
    Some((handle, value))
}

impl StudioApp {
    fn handles_id(&self) -> egui::Id {
        egui::Id::new(("handles", self.doc.id))
    }

    fn handles_load(&self, ctx: &egui::Context) -> (State, Settings) {
        let id = self.handles_id();
        ctx.data_mut(|d| {
            let state = d.get_temp::<State>(id).unwrap_or_default();
            let settings = d.get_temp::<Settings>(egui::Id::new("handles-settings")).unwrap_or_default();
            (state, settings)
        })
    }

    fn handles_store(&self, ctx: &egui::Context, state: State, settings: Settings) {
        let id = self.handles_id();
        ctx.data_mut(|d| {
            d.insert_temp(id, state);
            d.insert_temp(egui::Id::new("handles-settings"), settings);
        });
    }

    /// The object the handles are on: the one selected, when it is alone,
    /// written in the main file, and the code is the one on screen.
    fn handles_target(&self) -> Option<(Statement, Range<usize>)> {
        let selection = self.doc.viewer.selection();
        if selection.len() != 1 {
            return None;
        }
        let range = self.selected_ranges()?.pop()?;
        Some((selection[0], range))
    }

    /// Before the view takes the pointer: finds the handle under it, and
    /// starts a gesture on a press there. Returns whether the handles hold
    /// the pointer, for the view not to turn the camera or pick under them.
    pub(super) fn handles_before_view(&mut self, ctx: &egui::Context) -> bool {
        if self.doc.is_circuit() {
            return false;
        }
        let (mut state, settings) = self.handles_load(ctx);
        let rect = self.view_rect;
        state.hot = None;
        if state.gesture.is_none() {
            if let Some((key, range)) = self.handles_target() {
                let (pointer, pressed, press, dragging) = ctx.input(|i| {
                    let p = &i.pointer;
                    (p.hover_pos(), p.primary_pressed(), p.press_origin(), p.any_down() && !p.any_pressed())
                });
                // A button held since an earlier frame is a drag of the view
                // (a gesture would be under way otherwise): passing over a
                // handle while turning the camera must not light it, nor
                // take the pointer from the camera mid-turn.
                let pointer = if pressed { press } else if dragging { None } else { pointer };
                // Not through a menu, a popup or the settings bar.
                let free = |p: Pos2| {
                    rect.contains(p)
                        && !state.bar.is_some_and(|bar| bar.contains(p))
                        && ctx.layer_id_at(p).is_none_or(|layer| layer.order == egui::Order::Background)
                };
                if let Some(pointer) = pointer.filter(|&p| free(p)) {
                    let pose = state.pose(&self.doc.text, &range, &self.doc.viewer, key);
                    let gizmo = gizmo_for(&self.doc.viewer, rect, &pose, &settings);
                    let viewer = &self.doc.viewer;
                    state.hot = gizmo.and_then(|g| g.pick(|p| viewer.to_screen(rect, p), pointer));
                    if let (true, Some(handle), Some(gizmo)) = (pressed, state.hot, gizmo) {
                        match Gesture::start(handle, pointer, viewer, rect, gizmo, &pose, key, range) {
                            Ok(gesture) => state.gesture = Some(gesture),
                            Err(why) if !why.is_empty() => self.doc.console.log(Level::Warning, why),
                            Err(_) => {}
                        }
                    }
                }
            }
        }
        let holds = state.hot.is_some() || state.gesture.is_some();
        if holds {
            // A double-click on a handle does not frame the view.
            self.doc.viewer.frame_on_double_click = false;
        }
        self.handles_store(ctx, state, settings);
        holds
    }

    /// Draws the handles on the selection and follows their drags.
    pub(super) fn handles_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, response: &egui::Response) {
        let (mut state, mut settings) = self.handles_load(ctx);
        let rect = response.rect;
        let target = self.handles_target();
        let gone = match (&state.gesture, &target) {
            (Some(g), Some((key, _))) => g.key != *key,
            (Some(_), None) => true,
            _ => false,
        };
        if gone {
            state.gesture = None;
            self.doc.viewer.preview_selection(None);
        }
        let Some((key, range)) = target else {
            state.hot = None;
            state.bar = None;
            self.handles_store(ctx, state, settings);
            return;
        };
        let pose = state.pose(&self.doc.text, &range, &self.doc.viewer, key);

        // A scripted screenshot holds a gesture.
        if !state.staged && self.screenshot.is_some() {
            state.staged = true;
            let spec = std::env::var("CODE2FORM_SCREENSHOT_HANDLE").ok();
            if let (Some((handle, value)), Some(gizmo)) =
                (spec.as_deref().and_then(staged), gizmo_for(&self.doc.viewer, rect, &pose, &settings))
            {
                let press = self.doc.viewer.to_screen(rect, gizmo.origin).unwrap_or(rect.center());
                if let Ok(mut gesture) =
                    Gesture::start(handle, press, &self.doc.viewer, rect, gizmo, &pose, key, range.clone())
                {
                    gesture.start = Start::Held;
                    gesture.change = match handle {
                        Handle::Move(k) => Change::Move(gizmo.axes[k].as_dvec3() * value),
                        Handle::Turn(k) => Change::Turn { axis: gizmo.axes[k].as_dvec3(), degrees: value },
                        Handle::Scale(k) => {
                            let mut f = [1.0; 3];
                            f[k] = value;
                            Change::Scale(f)
                        }
                        _ => Change::Scale([value; 3]),
                    };
                    gesture.label = gesture.describe(&[value, 0.0]);
                    state.gesture = Some(gesture);
                }
            }
        }

        if let Some(mut gesture) = state.gesture.take() {
            let held = matches!(gesture.start, Start::Held);
            let typing = ctx.memory(|m| m.focused().is_none());
            let (events, shift, released, pointer) = ctx.input(|i| {
                (
                    i.events.clone(),
                    i.modifiers.shift,
                    i.pointer.primary_released() || !i.pointer.primary_down(),
                    i.pointer.latest_pos(),
                )
            });
            // Taken from what comes after (the shapes' Delete key...).
            let (cancel, enter, back) = ctx.input_mut(|i| {
                let none = egui::Modifiers::NONE;
                (i.consume_key(none, Key::Escape), i.consume_key(none, Key::Enter), i.consume_key(none, Key::Backspace))
            });
            let commit = enter && !gesture.typed.is_empty();
            if back && typing {
                gesture.typed.pop();
            }
            let text = events.iter().filter_map(|e| match e {
                egui::Event::Text(text) if typing && gesture.takes_typing() => Some(text.clone()),
                _ => None,
            });
            for c in text.collect::<String>().chars() {
                let fits = c.is_ascii_digit()
                    || (c == '.' && !gesture.typed.contains('.'))
                    || (c == '-' && gesture.typed.is_empty());
                if fits {
                    gesture.typed.push(c);
                }
            }
            if let (Some(pointer), false) = (pointer, held) {
                gesture.drag(&self.doc.viewer, rect, pointer, !shift, &settings);
            }
            if cancel {
                self.doc.viewer.preview_selection(None);
            } else if commit || (released && !held) {
                self.doc.viewer.preview_selection(None);
                if let Some(result) = gesture.edit(&self.doc.text) {
                    self.apply_code_edit(ctx, result, &done_message(gesture.change));
                }
            } else {
                self.doc.viewer.preview_selection(Some(gesture.preview()));
                state.gesture = Some(gesture);
            }
            ctx.request_repaint();
        }

        let painter = ui.painter_at(rect);
        let gizmo = match &state.gesture {
            Some(g) => Some(g.gizmo),
            None => gizmo_for(&self.doc.viewer, rect, &pose, &settings),
        };
        if let Some(gizmo) = gizmo {
            let viewer = &self.doc.viewer;
            let active = state.gesture.as_ref().map(|g| g.handle);
            paint(&painter, &gizmo.marks(|p| viewer.to_screen(rect, p)), state.hot, active);
        }
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        // Without a pointer (a scripted screenshot), by the handles.
        let by_handles =
            gizmo.and_then(|g| self.doc.viewer.to_screen(rect, g.origin)).map(|c| c + Vec2::new(40.0, -40.0));
        match (&state.gesture, state.hot, pointer) {
            (Some(gesture), _, pos) if pos.or(by_handles).is_some() => {
                let pos = pos.or(by_handles).unwrap_or_default();
                crate::inspect::paint_bubble_in(&painter, rect, pos, &gesture.label, theme::palette().text);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            (None, Some(hot), Some(pos)) => {
                let hint = format!("{} · Shift: no snapping · type a value while dragging", hot.describe());
                crate::inspect::paint_bubble(&painter, rect, pos, &hint);
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
            _ => {}
        }
        state.bar = Some(settings_bar(ui, rect, &mut settings));
        self.handles_store(ctx, state, settings);
    }
}

/// The bar at the top right of the view: the frame of the handles and the
/// snapping steps. Returns where it is.
fn settings_bar(ui: &mut egui::Ui, rect: Rect, settings: &mut Settings) -> Rect {
    let size = Vec2::new(340.0, 26.0);
    let at = Rect::from_min_size(egui::pos2(rect.right() - 12.0 - size.x, rect.top() + 12.0), size);
    let bar = |ui: &mut egui::Ui| {
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(18, 20, 25, 215))
            .stroke(Stroke::new(1.0, theme::palette().border))
            .corner_radius(13)
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let world = ui
                        .selectable_label(!settings.local, "World")
                        .on_hover_text("Move and turn along the world's axes");
                    if world.clicked() {
                        settings.local = false;
                    }
                    let local = ui
                        .selectable_label(settings.local, "Local")
                        .on_hover_text("Move and turn along the object's own axes");
                    if local.clicked() {
                        settings.local = true;
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Snap").color(theme::palette().text_dim));
                    ui.add(
                        egui::DragValue::new(&mut settings.step_mm)
                            .range(0.0..=100.0)
                            .speed(0.1)
                            .max_decimals(2)
                            .suffix(" mm"),
                    )
                    .on_hover_text("Moves snap to this step · Shift: no snapping");
                    ui.add(
                        egui::DragValue::new(&mut settings.step_degrees)
                            .range(0.0..=90.0)
                            .speed(0.5)
                            .max_decimals(1)
                            .suffix("°"),
                    )
                    .on_hover_text("Turns snap to this step · Shift: no snapping");
                });
            })
            .response
    };
    ui.put(at, bar).rect
}

/// Draws the handles: the one under the pointer or being dragged lighter
/// and thicker, the others faded while dragging.
fn paint(painter: &egui::Painter, marks: &[(Handle, Mark)], hot: Option<Handle>, active: Option<Handle>) {
    for (handle, mark) in marks {
        let base = handle.axis().map_or(theme::palette().text, |k| AXIS_COLORS[k]);
        let lit = Some(*handle) == hot || Some(*handle) == active;
        let color = match (lit, active.is_some()) {
            (true, _) => base.lerp_to_gamma(Color32::WHITE, 0.45),
            (false, true) => base.gamma_multiply(0.35),
            (false, false) => base,
        };
        let width = if lit { 3.5 } else { 2.0 };
        match mark {
            Mark::Ring(points) => {
                painter.add(Shape::closed_line(points.clone(), Stroke::new(width - 0.5, color)));
            }
            Mark::Quad(corners) => {
                let fill = color.gamma_multiply(if lit { 0.7 } else { 0.35 });
                painter.add(Shape::convex_polygon(corners.to_vec(), fill, Stroke::new(1.0, color)));
            }
            Mark::Arrow { from, base, tip } => {
                painter.line_segment([*from, *base], Stroke::new(width, color));
                let along = (*tip - *base).normalized();
                let side = Vec2::new(-along.y, along.x) * if lit { 7.5 } else { 6.0 };
                painter.add(Shape::convex_polygon(vec![*tip, *base + side, *base - side], color, Stroke::NONE));
            }
            Mark::Knob { at, anchor } => {
                painter.line_segment([*anchor, *at], Stroke::new(1.0, color.gamma_multiply(0.6)));
                let half = if lit { 5.5 } else { 4.5 };
                painter.rect_filled(Rect::from_center_size(*at, Vec2::splat(half * 2.0)), 1.5, color);
            }
            Mark::Dot(at) => {
                let radius = if lit { 7.0 } else { 5.5 };
                painter.circle(
                    *at,
                    radius,
                    color.gamma_multiply(0.85),
                    Stroke::new(1.5, Color32::from_black_alpha(140)),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::viewer::Status;
    use crate::worker::Scene;

    const STATUS: Status = Status { rendering: false, failed: false, has_model: true, is_2d: false, empty_hint: "" };

    /// A design's scene, rendered as the app renders it.
    fn design(src: &str) -> Scene {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        let output = c2f_eval::evaluate(&program);
        let r = c2f_geom::render(&output.nodes);
        let picking = Picking::new(output.origins, &r.solid, &r.highlights, &r.backgrounds, &r.cutters);
        Scene {
            solid: r.solid,
            highlights: Default::default(),
            backgrounds: Default::default(),
            is_2d: false,
            stats: Default::default(),
            formats: &[],
            pauses: Vec::new(),
            source: src.to_string(),
            parts: Vec::new(),
            enclosure: 0..0,
            exploded: None,
            picking: Some(Arc::new(picking)),
        }
    }

    /// The corners of the triangles the statement at `range` of `src`
    /// makes, as rendered.
    fn points(src: &str, range: Range<usize>) -> Vec<DVec3> {
        let scene = design(src);
        let key = Statement { unit: 0, span: c2f_lang::Span::new(range.start, range.end) };
        let mesh = scene.picking.unwrap().highlight(&[key], &[]);
        assert!(!mesh.positions.is_empty(), "nothing made by {:?}", &src[range]);
        mesh.positions.iter().map(|p| Vec3::from(*p).as_dvec3()).collect()
    }

    /// The box around `points`, each moved by `m`.
    fn moved_box(points: &[DVec3], m: glam::DMat4) -> (DVec3, DVec3) {
        let mut out = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for p in points {
            let p = m.transform_point3(*p);
            out = (out.0.min(p), out.1.max(p));
        }
        out
    }

    fn near(a: (DVec3, DVec3), b: (DVec3, DVec3)) -> bool {
        (a.0 - b.0).abs().max_element() < 2e-3 && (a.1 - b.1).abs().max_element() < 2e-3
    }

    /// Writes `change` on the object at `needle` and checks the code renders
    /// what the preview showed: the object's box moved as the change moves
    /// it about the centre of that box.
    fn written(src: &str, needle: &str, change: Change) -> String {
        let start = src.find(needle).unwrap();
        let range = start..start + needle.len();
        let scene = design(src);
        let picking = scene.picking.as_deref().unwrap();
        let pose = Pose::read(src, &range, Some(picking), picking.at_offset(start).unwrap());
        let translate = pose.translate.unwrap() + pose.parents.unwrap();
        let rotation = pose.rotation.unwrap();
        let before = points(src, range.clone());
        let (lo, hi) = moved_box(&before, glam::DMat4::IDENTITY);
        let pivot = (lo + hi) * 0.5;
        let edit = write_change(src, range, change, pivot, translate, rotation).unwrap();
        let expected = moved_box(&before, change.matrix(pivot, rotation));
        let after = moved_box(&points(&edit.text, edit.selection.clone()), glam::DMat4::IDENTITY);
        assert!(near(after, expected), "{}\nrenders {after:?}, the preview showed {expected:?}", edit.text);
        edit.text
    }

    #[test]
    fn a_move_adds_or_rewrites_the_translate() {
        let text = written("cube(10);", "cube(10);", Change::Move(DVec3::new(12.0, 0.0, 0.0)));
        assert_eq!(text, "translate([12, 0, 0]) cube(10);");
        let text = written("translate([1, 2, 3]) cube(10);", "cube(10);", Change::Move(DVec3::new(0.0, 0.5, 0.0)));
        assert_eq!(text, "translate([1, 2.5, 3]) cube(10);");
    }

    #[test]
    fn a_turn_keeps_the_centre_where_it_is() {
        // Around Z by 90° about its centre (10, 5, 2.5).
        let text = written("cube([20, 10, 5]);", "cube([20, 10, 5]);", Change::Turn { axis: DVec3::Z, degrees: 90.0 });
        assert_eq!(text, "translate([15, -5, 0]) rotate([0, 0, 90]) cube([20, 10, 5]);");
        // A turn already there adds up, its translate moves along.
        let src = "translate([30, 0, 0]) rotate([0, 0, 30]) cube(4, center = true);";
        let text = written(src, "cube(4, center = true);", Change::Turn { axis: DVec3::Z, degrees: 15.0 });
        assert_eq!(text, "translate([30, 0, 0]) rotate([0, 0, 45]) cube(4, center = true);");
        // Around another axis, the angles of one rotate compose.
        let text = written(src, "cube(4, center = true);", Change::Turn { axis: DVec3::X, degrees: 90.0 });
        assert!(text.starts_with("translate([30, 0, 0]) rotate([90, "), "{text}");
    }

    #[test]
    fn turns_around_the_objects_own_axis_compose_inside() {
        // Turned 90° around Z, its own X is the world's Y: turning around it
        // is turning around the world's Y, written as one rotate.
        let src = "rotate([0, 0, 90]) cube(6, center = true);";
        let own = gizmo::rotation([0.0, 0.0, 90.0]);
        let text = written(src, "cube(6, center = true);", Change::Turn { axis: own * DVec3::X, degrees: 90.0 });
        assert_eq!(text, "rotate([90, 0, 90]) cube(6, center = true);");
    }

    #[test]
    fn a_scale_grows_about_the_centre_along_the_objects_axes() {
        let text = written("cube(10);", "cube(10);", Change::Scale([2.0, 1.0, 1.0]));
        assert_eq!(text, "translate([-5, 0, 0]) scale([2, 1, 1]) cube(10);");
        // Nested in a group: the object's own chain changes.
        let src = "translate([0, 0, 5]) {\n    rotate([0, 0, 90]) scale(2) sphere(1);\n    cube(1);\n}\n";
        let text = written(src, "rotate([0, 0, 90]) scale(2) sphere(1);", Change::Scale([1.5, 1.0, 1.0]));
        assert!(text.contains("rotate([0, 0, 90]) scale([3, 2, 2]) sphere(1);"), "{text}");
    }

    #[test]
    fn computed_values_are_refused_with_the_reason() {
        let src = "w = 4;\ntranslate([w, 0, 0]) cube(1);";
        let range = src.find("cube").unwrap()..src.len();
        let scene = design(src);
        let picking = scene.picking.as_deref().unwrap();
        let pose = Pose::read(src, &range, Some(picking), picking.at_offset(range.start).unwrap());
        assert!(pose.translate.is_err());
        let err =
            write_change(src, range, Change::Move(DVec3::X), DVec3::ZERO, DVec3::ZERO, DMat3::IDENTITY).unwrap_err();
        assert!(err.starts_with("The value is computed"), "{err}");
    }

    #[test]
    fn the_translates_around_an_object_add_to_its_own() {
        let offset = |src: &str, needle: &str| {
            let scene = design(src);
            let picking = scene.picking.as_deref().unwrap();
            parents_offset(src, picking, picking.at_offset(src.find(needle).unwrap()).unwrap())
        };
        let src = "translate([0, 0, 5]) {\n    translate([1, 0, 0]) cube(1);\n    sphere(1);\n}";
        // Its own translate is not around it; the group's is.
        assert_eq!(offset(src, "cube(1);"), Ok(DVec3::new(0.0, 0.0, 5.0)));
        assert_eq!(offset(src, "translate([1, 0, 0]) cube(1);"), Ok(DVec3::new(0.0, 0.0, 5.0)));
        let src = "module m() translate([2, 0, 0]) cube(1);\ntranslate([0, 3, 0]) m();";
        assert_eq!(offset(src, "cube(1);"), Ok(DVec3::new(0.0, 3.0, 0.0)));
        let src = "rotate([0, 0, 45]) { cube(1); sphere(1); }";
        assert_eq!(offset(src, "cube(1);"), Err(AROUND.to_string()));
        let src = "for (i = [0 : 2]) translate([i * 5, 0, 0]) cube(1);";
        assert_eq!(offset(src, "cube(1);"), Err(COPIES.to_string()));
        // A module's child is placed by the module's body, here turned.
        let src = "module m() rotate([0, 0, 90]) children();\nm() cube(1);";
        assert_eq!(offset(src, "cube(1);"), Err(AROUND.to_string()));
        let src = "module m() translate([5, 0, 0]) children();\ntranslate([0, 1, 0]) m() translate([1, 0, 0]) cube(1);";
        assert_eq!(offset(src, "cube(1);"), Err(AROUND.to_string()));
    }

    #[test]
    fn staged_gestures_read_their_spec() {
        assert_eq!(staged("move-x:12"), Some((Handle::Move(0), 12.0)));
        assert_eq!(staged("turn-z:45"), Some((Handle::Turn(2), 45.0)));
        assert_eq!(staged("scale:2"), Some((Handle::ScaleAll, 2.0)));
        assert_eq!(staged("spin-z:2"), None);
    }

    /// A view of `src` with `needle` selected, shown once in an 800 by 600
    /// window; returns its rectangle.
    fn selected_view(ctx: &egui::Context, src: &str, needle: &str) -> (Viewer, Rect, Range<usize>) {
        let mut viewer = Viewer::new(None);
        viewer.set_scene(design(src), true);
        let start = src.find(needle).unwrap();
        viewer.select_at(start);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            ..Default::default()
        };
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| rect = viewer.show(ui, &STATUS).rect);
        });
        (viewer, rect, start..start + needle.len())
    }

    /// How far from the centre, in points, [`drag`] presses an arrow: 60%
    /// of the way along its shaft, from 16 to 92 points.
    const PRESS: f32 = 16.0 + 0.6 * 76.0;

    /// Presses `handle` where it shows and drags it to where `to` says, in
    /// eight steps, as the app follows a mouse; returns the gesture.
    fn drag(
        src: &str,
        needle: &str,
        handle: Handle,
        settings: Settings,
        to: impl Fn(&Gizmo, &Viewer, Rect) -> Pos2,
    ) -> Gesture {
        let ctx = egui::Context::default();
        let (viewer, rect, range) = selected_view(&ctx, src, needle);
        let key = viewer.selection()[0];
        let pose = Pose::read(src, &range, viewer.picking().map(|p| &**p), key);
        let gizmo = gizmo_for(&viewer, rect, &pose, &settings).unwrap();
        let marks = gizmo.marks(|p| viewer.to_screen(rect, p));
        let press = match &marks.iter().find(|(h, _)| *h == handle).expect("the handle shows").1 {
            Mark::Arrow { .. } => {
                viewer.to_screen(rect, gizmo.origin + gizmo.axes[handle.axis().unwrap()] * PRESS * gizmo.unit).unwrap()
            }
            Mark::Ring(points) => points[3],
            Mark::Quad(c) => Pos2::new((c[0].x + c[2].x) / 2.0, (c[0].y + c[2].y) / 2.0),
            Mark::Knob { at, .. } | Mark::Dot(at) => *at,
        };
        assert_eq!(gizmo.pick(|p| viewer.to_screen(rect, p), press), Some(handle), "the press grabs it");
        let mut gesture = Gesture::start(handle, press, &viewer, rect, gizmo, &pose, key, range).unwrap();
        // In steps, as a mouse goes.
        let end = to(&gizmo, &viewer, rect);
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            gesture.drag(&viewer, rect, press + (end - press) * t, true, &settings);
        }
        gesture
    }

    #[test]
    fn dragging_an_arrow_moves_along_it_by_whole_millimetres() {
        let src = "cube(10);\nsphere(2);";
        let gesture = drag(src, "cube(10);", Handle::Move(0), Settings::default(), |g, v, rect| {
            // On screen, 12.3 mm further along X than the press.
            v.to_screen(rect, g.origin + Vec3::X * (PRESS * g.unit + 12.3)).unwrap()
        });
        assert_eq!(gesture.label, "12 mm");
        let edit = gesture.edit(src).unwrap().unwrap();
        assert_eq!(edit.text, "translate([12, 0, 0]) cube(10);\nsphere(2);");
    }

    #[test]
    fn dragging_a_ring_a_quarter_round_turns_by_ninety_degrees() {
        let src = "cube([20, 10, 4]);";
        let gesture = drag(src, src, Handle::Turn(2), Settings::default(), |g, v, rect| {
            // The ring's point a quarter turn on (anticlockwise seen from
            // above, the right-hand way round Z) from its fourth point.
            let t = 3.0 / 64.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
            v.to_screen(rect, g.origin + (Vec3::X * t.cos() + Vec3::Y * t.sin()) * 76.0 * g.unit).unwrap()
        });
        assert_eq!(gesture.label, "90°");
        let edit = gesture.edit(src).unwrap().unwrap();
        assert_eq!(edit.text, "translate([15, -5, 0]) rotate([0, 0, 90]) cube([20, 10, 4]);");
    }

    #[test]
    fn local_arrows_follow_the_turned_object() {
        let src = "rotate([0, 0, 90]) cube(10, center = true);";
        let local = Settings { local: true, ..Settings::default() };
        let gesture = drag(src, "cube(10, center = true);", Handle::Move(0), local, |g, v, rect| {
            // Its own X is the world's Y: 5 mm along it.
            v.to_screen(rect, g.origin + g.axes[0] * (PRESS * g.unit + 5.0)).unwrap()
        });
        assert_eq!(gesture.label, "5 mm");
        let edit = gesture.edit(src).unwrap().unwrap();
        assert_eq!(edit.text, "translate([0, 5, 0]) rotate([0, 0, 90]) cube(10, center = true);");
    }

    #[test]
    fn typing_a_value_sets_it_exactly() {
        let src = "sphere(5);";
        let mut gesture = drag(src, src, Handle::ScaleAll, Settings::default(), |g, v, rect| {
            v.to_screen(rect, g.origin).unwrap() + Vec2::new(29.0, 0.0)
        });
        // Half the scale squares' distance out: half as big again.
        assert_eq!(gesture.label, "×1.5");
        gesture.typed = "1.25".to_string();
        let ctx = egui::Context::default();
        let (viewer, rect, _) = selected_view(&ctx, src, src);
        gesture.drag(&viewer, rect, Pos2::new(500.0, 300.0), true, &Settings::default());
        assert_eq!(gesture.change, Change::Scale([1.25; 3]));
        assert_eq!(gesture.label, "×1.25|");
    }
}
