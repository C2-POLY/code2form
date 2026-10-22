//! The 3D view: camera controls, the OpenGL paint callback and the
//! overlays drawn on top (hints, status badges, the axis gizmo, the
//! labels of the print's pauses, the layer slider).

use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use eframe::{egui_glow, glow};
use c2f_geom::RenderMesh;
use glam::{Mat4, Vec2 as GVec2, Vec3, Vec4};

use crate::camera::Camera;
use crate::pause_edit;
use crate::renderer::{self, Grid, Renderer};
use crate::theme;
use crate::worker::Scene;

/// What the viewport shows besides the model.
pub struct Status<'a> {
    pub rendering: bool,
    /// The last render failed; the model on screen (if any) is older.
    pub failed: bool,
    pub has_model: bool,
    pub is_2d: bool,
    /// A short hint about what is being edited, for the empty state.
    pub empty_hint: &'a str,
}

/// A change to the pauses asked for in the layer view, for the app to make
/// in the source.
#[derive(Clone, Debug, PartialEq)]
pub enum PauseEdit {
    Add { z: f32 },
    /// Writes a new height over the number at these bytes.
    Move { number: std::ops::Range<usize>, z: f32 },
    /// Removes the call whose name starts at this byte.
    Remove { call: usize },
}

/// A pause as the view shows it.
struct PauseMark {
    z: f32,
    /// Its rectangle's corners in the 3D view.
    corners: [[f32; 3]; 4],
    label: String,
    source: Option<c2f_eval::PauseSource>,
}

/// What the layer slider's context menu is about.
#[derive(Clone, Copy)]
enum SliderMenu {
    /// A layer, to add a pause at its top.
    Layer(u32),
    /// A pause, to remove it.
    Pause(usize),
}

/// Standard views: name, yaw and pitch in degrees.
pub const PRESETS: &[(&str, f32, f32)] = &[
    ("Diagonal", -60.0, 28.0),
    ("Top", -90.0, 90.0),
    ("Bottom", -90.0, -90.0),
    ("Front", -90.0, 0.0),
    ("Back", 90.0, 0.0),
    ("Left", 180.0, 0.0),
    ("Right", 0.0, 0.0),
];

pub struct Viewer {
    renderer: Result<Arc<Mutex<Renderer>>, String>,
    pub camera: Camera,
    pub show_axes: bool,
    pub show_grid: bool,
    pub show_pauses: bool,
    /// The layer view: the model cut at the top of a layer, picked with
    /// the slider on the right of the view.
    pub layer_view: bool,
    pub layer_height: f32,
    /// The layer the cut shows the top of, from 1; `None` for the last.
    layer: Option<u32>,
    /// The solid's bottom and top.
    heights: Option<(f32, f32)>,
    /// The camera turns around the model by itself until a drag in the
    /// view or the toggle stops it.
    pub turntable: bool,
    bounds: Option<(Vec3, Vec3)>,
    /// The box around a circuit's wires and pins, framed with the model.
    overlay_bounds: Option<(Vec3, Vec3)>,
    pauses: Vec<PauseMark>,
    /// The text the scene was rendered from.
    scene_source: String,
    /// The pause being dragged on the slider, and the layer it is over.
    dragging: Option<(usize, u32)>,
    menu: Option<SliderMenu>,
    edit: Option<PauseEdit>,
    grid: Grid,
    /// Width over height of the viewport, as last shown.
    aspect: f32,
}

impl Viewer {
    pub fn new(gl: Option<&Arc<glow::Context>>) -> Self {
        let renderer = match gl {
            Some(gl) => Renderer::new(gl).map(|r| Arc::new(Mutex::new(r))),
            None => Err("the 3D view needs the OpenGL (glow) renderer".to_string()),
        };
        let grid = Grid::for_bounds(None);
        if let Ok(renderer) = &renderer {
            if let Ok(mut r) = renderer.lock() {
                r.set_grid(grid);
            }
        }
        let mut viewer = Viewer {
            renderer,
            camera: Camera::default(),
            show_axes: true,
            show_grid: true,
            show_pauses: true,
            layer_view: false,
            layer_height: 0.2,
            layer: None,
            heights: None,
            turntable: false,
            bounds: None,
            overlay_bounds: None,
            pauses: Vec::new(),
            scene_source: String::new(),
            dragging: None,
            menu: None,
            edit: None,
            grid,
            aspect: 1.5,
        };
        viewer.reset();
        viewer
    }

    /// Shows a new scene; `frame` also moves the camera to fit it.
    pub fn set_scene(&mut self, scene: Scene, frame: bool) {
        self.bounds = scene.bounds().map(|(lo, hi)| (Vec3::from(lo), Vec3::from(hi)));
        self.grid = Grid::for_bounds(self.bounds);
        self.heights = if scene.is_2d { None } else { scene.solid.bounds().map(|(lo, hi)| (lo[2], hi[2])) };
        self.pauses = match renderer::pause_footprint(&scene) {
            Some(footprint) => scene
                .pauses
                .iter()
                .map(|p| PauseMark {
                    z: p.z as f32,
                    corners: renderer::pause_corners(p.z as f32, footprint),
                    label: pause_label(p),
                    source: p.source.clone(),
                })
                .collect(),
            None => Vec::new(),
        };
        self.scene_source = scene.source.clone();
        // Indices into the old pauses mean nothing now.
        self.dragging = None;
        self.menu = None;
        if let Ok(renderer) = &self.renderer {
            if let Ok(mut r) = renderer.lock() {
                r.set_scene(scene);
                r.set_grid(self.grid);
            }
        }
        if frame {
            self.view_all();
        }
    }

    /// Shows a circuit's wires and pin dots, or nothing; `bounds` is the
    /// box around them.
    pub fn set_overlay(&mut self, wires: RenderMesh, pins: RenderMesh, bounds: Option<([f32; 3], [f32; 3])>) {
        self.overlay_bounds = bounds.map(|(lo, hi)| (Vec3::from(lo), Vec3::from(hi)));
        if let Ok(renderer) = &self.renderer {
            if let Ok(mut r) = renderer.lock() {
                r.set_overlay(wires, pins);
            }
        }
    }

    /// The model's box and the overlay's, together.
    fn drawn_bounds(&self) -> Option<(Vec3, Vec3)> {
        match (self.bounds, self.overlay_bounds) {
            (Some((l1, h1)), Some((l2, h2))) => Some((l1.min(l2), h1.max(h2))),
            (a, b) => a.or(b),
        }
    }

    /// The box to frame: the model, or a default 20-unit cube around the
    /// origin when there is none.
    fn frame_box(&self) -> (Vec3, Vec3) {
        self.drawn_bounds().unwrap_or((Vec3::splat(-10.0), Vec3::splat(10.0)))
    }

    /// The number of layers the solid prints in, if there is one.
    pub fn layer_count(&self) -> Option<u32> {
        let (lo, hi) = self.heights?;
        let layers = ((hi - lo) / self.layer_height.max(0.01) - 1e-3).ceil().max(1.0);
        Some(layers.min(1e6) as u32)
    }

    /// The layer shown, from 1 to the last.
    pub fn layer(&self) -> Option<u32> {
        let count = self.layer_count()?;
        Some(self.layer.unwrap_or(count).clamp(1, count))
    }

    fn set_layer(&mut self, layer: i64) {
        if let Some(count) = self.layer_count() {
            let layer = layer.clamp(1, i64::from(count)) as u32;
            self.layer = (layer < count).then_some(layer);
        }
    }

    /// The top of a layer.
    fn layer_top(&self, layer: u32) -> Option<f32> {
        let (lo, hi) = self.heights?;
        Some((lo + layer as f32 * self.layer_height).min(hi))
    }

    /// The height the layer view cuts the model at; `None` when it is off.
    pub fn cut(&self) -> Option<f32> {
        if !self.layer_view {
            return None;
        }
        self.layer_top(self.layer()?)
    }

    /// The change to the pauses asked for in the layer view, if any.
    pub fn take_pause_edit(&mut self) -> Option<PauseEdit> {
        self.edit.take()
    }

    /// The text the scene shown was rendered from.
    pub fn scene_source(&self) -> &str {
        &self.scene_source
    }

    /// The model asks for print pauses.
    pub fn has_pauses(&self) -> bool {
        !self.pauses.is_empty()
    }

    pub fn view_all(&mut self) {
        let (lo, hi) = self.frame_box();
        self.camera.frame(lo, hi, self.aspect);
    }

    pub fn reset(&mut self) {
        let (lo, hi) = self.frame_box();
        self.camera.reset(lo, hi, self.aspect);
    }

    pub fn look_from(&mut self, yaw: f32, pitch: f32) {
        self.camera.look_from(yaw, pitch);
        self.view_all();
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        if let Ok(renderer) = &self.renderer {
            if let Ok(mut r) = renderer.lock() {
                r.destroy(gl);
            }
        }
    }

    /// The distance from the target to the farthest point drawn.
    fn scene_radius(&self) -> f32 {
        let half = self.grid.half_size();
        let (mut lo, mut hi) = (Vec3::new(-half, -half, 0.0), Vec3::new(half, half, 0.0));
        if let Some((l, h)) = self.drawn_bounds() {
            lo = lo.min(l);
            hi = hi.max(h);
        }
        let target = self.camera.target;
        let mut radius: f32 = 0.0;
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            radius = radius.max(corner.distance(target));
        }
        radius.max(1e-3)
    }

    /// Draws the view; returns its response, for what the app draws over
    /// it.
    pub fn show(&mut self, ui: &mut egui::Ui, status: &Status) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return response;
        }
        self.aspect = rect.width() / rect.height();
        self.handle_input(ui, rect, &response);
        if self.turntable {
            // Time-based, so the speed holds whatever the frame rate; capped
            // so a stalled frame does not jump.
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            self.camera.spin(dt);
            ui.ctx().request_repaint();
        }

        let view = self.camera.view();
        let projection = self.camera.projection(self.aspect, self.scene_radius());
        match &self.renderer {
            Ok(renderer) => {
                let frame = renderer::Frame {
                    view,
                    projection,
                    show_grid: self.show_grid,
                    show_axes: self.show_axes,
                    show_pauses: self.show_pauses,
                    cut: self.cut(),
                };
                let renderer = renderer.clone();
                let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                    if let Ok(mut r) = renderer.lock() {
                        r.paint(painter.gl(), &frame);
                    }
                });
                ui.painter().add(egui::PaintCallback { rect, callback: Arc::new(callback) });
            }
            Err(message) => {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, theme::BG);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    format!("3D view unavailable: {message}"),
                    FontId::proportional(14.0),
                    theme::ERROR,
                );
            }
        }

        let painter = ui.painter_at(rect);
        if self.show_pauses {
            self.paint_pause_labels(&painter, rect, projection * view);
        }
        if self.layer_view && self.heights.is_some() {
            self.layer_slider(ui, &painter, rect, &response);
        }
        self.paint_gizmo(&painter, rect);
        paint_overlays(ui, &painter, rect, status);
        response
    }

    fn handle_input(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        if response.drag_started() {
            // Hand the camera back to the mouse.
            self.turntable = false;
        }
        let delta = response.drag_delta();
        if response.dragged() && delta != Vec2::ZERO {
            let shift = ui.input(|i| i.modifiers.shift);
            let delta = GVec2::new(delta.x, delta.y);
            if response.dragged_by(PointerButton::Secondary)
                || response.dragged_by(PointerButton::Middle)
                || (response.dragged_by(PointerButton::Primary) && shift)
            {
                self.camera.pan(delta, rect.height());
            } else if response.dragged_by(PointerButton::Primary) {
                self.camera.orbit(delta);
            }
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.double_clicked() {
            self.view_all();
        }
        if response.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 || zoom != 1.0 {
                let factor = (-scroll * 0.0025).exp() / zoom.max(1e-3);
                let focus = response.hover_pos().map(|pos| {
                    let ndc = GVec2::new(
                        (pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
                        1.0 - (pos.y - rect.top()) / rect.height() * 2.0,
                    );
                    self.camera.point_at(ndc, self.aspect)
                });
                self.camera.zoom(factor, focus);
            }
        }
    }

    /// Each pause's height and message, next to the corner of its rectangle
    /// that shows farthest right, kept inside the view.
    fn paint_pause_labels(&self, painter: &egui::Painter, rect: Rect, view_projection: Mat4) {
        let to_screen = |p: [f32; 3]| {
            let clip = view_projection * Vec4::new(p[0], p[1], p[2], 1.0);
            (clip.w > 1e-6).then(|| {
                let ndc = clip.truncate() / clip.w;
                Pos2::new(rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(), rect.top() + (1.0 - ndc.y) * 0.5 * rect.height())
            })
        };
        let cut = self.cut();
        for PauseMark { z, corners, label, .. } in &self.pauses {
            if cut.is_some_and(|cut| *z > cut + 1e-4) {
                continue;
            }
            let Some(anchor) = corners.iter().filter_map(|&c| to_screen(c)).max_by(|a, b| a.x.total_cmp(&b.x)) else {
                continue;
            };
            let galley = painter.layout_no_wrap(label.clone(), FontId::proportional(12.0), Color32::WHITE);
            let size = galley.size();
            let mut inside = rect.shrink(10.0);
            if self.layer_view {
                // Clear of the layer slider.
                inside.max.x -= 60.0;
            }
            let min = Pos2::new(
                (anchor.x + 8.0).min(inside.right() - size.x).max(inside.left()),
                (anchor.y - size.y / 2.0).clamp(inside.top(), (inside.bottom() - size.y).max(inside.top())),
            );
            let text = Rect::from_min_size(min, size);
            let badge = text.expand2(Vec2::new(6.0, 3.0));
            painter.rect_filled(badge, CornerRadius::same(4), Color32::from_black_alpha(160));
            painter.rect_stroke(badge, CornerRadius::same(4), Stroke::new(1.0, PAUSE_COLOR), StrokeKind::Inside);
            painter.galley(text.min, galley, Color32::WHITE);
        }
    }

    /// The layer slider, on the right of the view: drag or click it, scroll
    /// over it, or press the up and down arrows (with Shift, ten layers) over
    /// the view. Marks show the pauses: drag one to move it, right-click the
    /// slider to add a pause at a layer or remove one. The layer height is
    /// set below it.
    fn layer_slider(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, view: &egui::Response) {
        let (Some(count), Some((lo, hi))) = (self.layer_count(), self.heights) else { return };
        let x = rect.right() - 34.0;
        let track = Track { top: rect.top() + 56.0, bottom: rect.bottom() - 84.0, count, lo, layer_height: self.layer_height };
        if track.bottom - track.top < 40.0 {
            return;
        }
        let area = Rect::from_x_y_ranges(x - 14.0..=x + 14.0, track.top - 10.0..=track.bottom + 10.0);
        let response = ui.interact(area, ui.id().with("layer_slider"), Sense::click_and_drag());
        // Each mark's height on the track, and whether its height is a
        // number in the source, which a drag can rewrite.
        let marks: Vec<(f32, bool)> = self
            .pauses
            .iter()
            .map(|mark| (track.y_of_height(mark.z), mark.source.as_ref().is_some_and(|s| s.number.is_some())))
            .collect();
        let mark_near = |y: f32| {
            marks
                .iter()
                .enumerate()
                .map(|(i, &(at, _))| (i, (at - y).abs()))
                .filter(|&(_, distance)| distance <= 6.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        let hovered_mark = response.hover_pos().and_then(|pos| mark_near(pos.y));
        let movable = |i: usize| marks[i].1;

        // A press on a mark that can move starts dragging it; anywhere else
        // it picks the layer.
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        if pressed && response.hovered() && self.dragging.is_none() {
            if let Some(i) = hovered_mark.filter(|&i| movable(i)) {
                self.dragging = Some((i, track.layer_of_height(self.pauses[i].z)));
            }
        }
        let pointer = response.interact_pointer_pos().filter(|_| response.is_pointer_button_down_on());
        match (self.dragging, pointer) {
            (Some((i, _)), Some(pos)) => self.dragging = Some((i, track.layer_at(pos.y).max(1))),
            (Some((i, layer)), None) => {
                self.dragging = None;
                let z = self.layer_top(layer).unwrap_or(hi);
                let mark = &self.pauses[i];
                if let Some(number) = mark.source.as_ref().and_then(|s| s.number.clone()) {
                    if (z - mark.z).abs() > 1e-4 {
                        self.edit = Some(PauseEdit::Move { number, z });
                    }
                }
            }
            (None, Some(pos)) => self.set_layer(i64::from(track.layer_at(pos.y))),
            (None, None) => {}
        }
        if response.secondary_clicked() {
            self.menu = response.interact_pointer_pos().map(|pos| match mark_near(pos.y) {
                Some(i) => SliderMenu::Pause(i),
                None => SliderMenu::Layer(track.layer_at(pos.y).max(1)),
            });
        }

        let current = i64::from(self.layer().unwrap_or(count));
        if response.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                self.set_layer(current + scroll.signum() as i64);
            }
        }
        if (view.hovered() || response.hovered()) && ui.memory(|m| m.focused().is_none()) {
            let (up, down, shift) =
                ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.modifiers.shift));
            let step = if shift { 10 } else { 1 };
            if up {
                self.set_layer(current + step);
            } else if down {
                self.set_layer(current - step);
            }
        }
        let layer = self.layer().unwrap_or(count);
        let z = self.layer_top(layer).unwrap_or(hi);

        // The track, filled up to the layer shown, and the pause marks.
        let handle = Pos2::new(x, track.y_of_layer(layer as f32));
        painter.line_segment(
            [Pos2::new(x, track.top), Pos2::new(x, track.bottom)],
            Stroke::new(4.0, Color32::from_white_alpha(30)),
        );
        painter.line_segment([Pos2::new(x, track.bottom), handle], Stroke::new(4.0, theme::ACCENT));
        let active = response.hovered() || response.dragged();
        painter.circle(handle, if active { 8.0 } else { 7.0 }, theme::TEXT, Stroke::new(2.0, theme::ACCENT));
        for (i, mark) in self.pauses.iter().enumerate() {
            let dragged = self.dragging.filter(|&(d, _)| d == i);
            let y = match dragged {
                Some((_, layer)) => track.y_of_layer(layer as f32),
                None => track.y_of_height(mark.z),
            };
            let strong = dragged.is_some() || hovered_mark == Some(i);
            let (half, width) = if strong { (12.0, 3.0) } else { (9.0, 2.0) };
            painter.line_segment([Pos2::new(x - half, y), Pos2::new(x + half, y)], Stroke::new(width, PAUSE_COLOR));
            if let Some((_, layer)) = dragged {
                let z = self.layer_top(layer).unwrap_or(hi);
                badge_left_of(painter, Pos2::new(x - 14.0, y), &format!("Pause → {} mm", pause_edit::number(z)), PAUSE_COLOR);
            }
        }
        if self.dragging.is_none() {
            // What the cut shows, left of the handle.
            badge_left_of(painter, handle - Vec2::new(14.0, 0.0), &format!("Layer {layer} / {count} · {z:.2} mm"), theme::TEXT);
        }

        // The layer height, under the track.
        let field = Rect::from_center_size(Pos2::new(x - 18.0, track.bottom + 38.0), Vec2::new(76.0, 20.0));
        let height = egui::DragValue::new(&mut self.layer_height).range(0.05..=1.0).speed(0.005).max_decimals(2).suffix(" mm");
        ui.put(field, height).on_hover_text("Layer height");

        if hovered_mark.is_some_and(movable) || self.dragging.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let hint = match hovered_mark {
            Some(i) if movable(i) => "Drag to move this pause · Right-click to remove it",
            Some(i) if self.pauses[i].source.is_some() => {
                "Its height is computed in the code, so it moves there · Right-click to remove it"
            }
            Some(_) => "This pause is written in a library or an included file",
            None => "Drag to pick a layer · Right-click to add a pause",
        };
        let response = if self.dragging.is_none() { response.on_hover_text(hint) } else { response };
        response.context_menu(|ui| self.slider_menu(ui));
    }

    fn slider_menu(&mut self, ui: &mut egui::Ui) {
        match self.menu {
            Some(SliderMenu::Layer(layer)) => {
                let z = self.layer_top(layer).unwrap_or(0.0);
                if ui.button(format!("Add a pause at {} mm", pause_edit::number(z))).clicked() {
                    self.edit = Some(PauseEdit::Add { z });
                    ui.close();
                }
            }
            Some(SliderMenu::Pause(i)) => {
                let Some(mark) = self.pauses.get(i) else { return };
                ui.label(egui::RichText::new(&mark.label).color(PAUSE_COLOR));
                match &mark.source {
                    Some(source) => {
                        if ui.button("Remove this pause").clicked() {
                            self.edit = Some(PauseEdit::Remove { call: source.call });
                            ui.close();
                        }
                    }
                    None => {
                        ui.label(egui::RichText::new("Written in a library or an included file").color(theme::TEXT_DIM));
                    }
                }
            }
            None => {}
        }
    }

    /// A small X/Y/Z compass in the bottom-left corner.
    fn paint_gizmo(&self, painter: &egui::Painter, rect: Rect) {
        let center = Pos2::new(rect.left() + 46.0, rect.bottom() - 46.0);
        let length = 28.0;
        let view = self.camera.view();
        let axes = [
            (Vec3::X, "X", Color32::from_rgb(240, 84, 90)),
            (Vec3::Y, "Y", Color32::from_rgb(102, 209, 97)),
            (Vec3::Z, "Z", Color32::from_rgb(92, 143, 255)),
        ];
        let mut projected: Vec<_> = axes
            .iter()
            .map(|&(axis, label, color)| {
                let v = view.transform_vector3(axis);
                (v.z, Vec2::new(v.x, -v.y) * length, label, color)
            })
            .collect();
        // Farthest first, so nearer axes draw on top.
        projected.sort_by(|a, b| a.0.total_cmp(&b.0));
        painter.circle(
            center,
            length + 12.0,
            Color32::from_black_alpha(36),
            Stroke::new(1.0, Color32::from_white_alpha(14)),
        );
        for (_, offset, label, color) in projected {
            let tip = center + offset;
            painter.line_segment([center, tip], Stroke::new(2.0, color));
            painter.circle_filled(tip, 7.5, color);
            painter.text(tip, Align2::CENTER_CENTER, label, FontId::proportional(10.0), Color32::from_gray(20));
        }
    }
}

fn paint_overlays(ui: &egui::Ui, painter: &egui::Painter, rect: Rect, status: &Status) {
    let margin = 12.0;

    if !status.has_model && !status.rendering {
        let title = painter.layout_no_wrap("Nothing to show yet".into(), FontId::proportional(19.0), theme::TEXT);
        let hint = painter.layout(
            status.empty_hint.into(),
            FontId::proportional(13.5),
            theme::TEXT_DIM,
            (rect.width() - 80.0).clamp(120.0, 340.0),
        );
        let width = title.size().x.max(hint.size().x) + 48.0;
        let height = title.size().y + hint.size().y + 46.0;
        let card = Rect::from_center_size(rect.center(), Vec2::new(width, height));
        painter.rect(
            card,
            CornerRadius::same(12),
            Color32::from_rgba_unmultiplied(22, 25, 31, 225),
            Stroke::new(1.0, Color32::from_white_alpha(16)),
            StrokeKind::Inside,
        );
        let title_pos = Pos2::new(card.center().x - title.size().x / 2.0, card.top() + 18.0);
        let hint_pos = Pos2::new(card.center().x - hint.size().x / 2.0, title_pos.y + title.size().y + 10.0);
        painter.galley(title_pos, title, theme::TEXT);
        painter.galley(hint_pos, hint, theme::TEXT_DIM);
    }

    // Badges, top-left.
    let mut at = rect.left_top() + Vec2::splat(margin);
    if status.rendering {
        let width = badge(painter, at, "Rendering…", theme::ACCENT_STRONG, true);
        let spinner = Rect::from_center_size(at + Vec2::new(14.0, 12.0), Vec2::splat(12.0));
        egui::Spinner::new().size(12.0).color(theme::ACCENT_STRONG).paint_at(ui, spinner);
        at.x += width + 8.0;
    }
    if status.failed {
        let text = if status.has_model {
            "Render failed: showing the last good model"
        } else {
            "Render failed: see the console"
        };
        badge(painter, at, text, theme::ERROR, false);
    }

    // Top-right: 2D note.
    if status.is_2d && status.has_model {
        let galley = painter.layout_no_wrap("2D".into(), FontId::proportional(12.0), theme::TEXT_DIM);
        let pos = Pos2::new(rect.right() - margin - galley.size().x - 16.0, rect.top() + margin);
        badge(painter, pos, "2D", theme::TEXT_DIM, false);
    }

    // Bottom-right: mouse help.
    painter.text(
        rect.right_bottom() - Vec2::new(margin, margin),
        Align2::RIGHT_BOTTOM,
        "Drag: orbit   Right/Shift-drag: pan   Scroll: zoom   Double-click: fit",
        FontId::proportional(11.0),
        Color32::from_white_alpha(70),
    );
}

/// A rounded label; returns its width.
fn badge(painter: &egui::Painter, at: Pos2, text: &str, color: Color32, spinner: bool) -> f32 {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.5), color);
    let lead = if spinner { 26.0 } else { 10.0 };
    let size = Vec2::new(galley.size().x + lead + 10.0, 24.0);
    let rect = Rect::from_min_size(at, size);
    painter.rect(
        rect,
        CornerRadius::same(12),
        Color32::from_rgba_unmultiplied(18, 20, 25, 215),
        Stroke::new(1.0, color.gamma_multiply(0.45)),
        StrokeKind::Inside,
    );
    painter.galley(Pos2::new(rect.left() + lead, rect.center().y - galley.size().y / 2.0), galley, color);
    size.x
}

/// The hint shown when there is nothing to draw.
pub fn empty_hint(has_text: bool) -> &'static str {
    if has_text {
        "The script ran but produced no geometry. Its echo output and warnings are in the console."
    } else {
        "Write some code in the editor, for example \"cube(10);\", or open a .c2f or .scad file."
    }
}

/// The layer slider's track: layers from the bottom to the top.
#[derive(Clone, Copy)]
struct Track {
    top: f32,
    bottom: f32,
    count: u32,
    /// The solid's bottom.
    lo: f32,
    layer_height: f32,
}

impl Track {
    fn y_of_layer(&self, layer: f32) -> f32 {
        self.bottom - (self.bottom - self.top) * layer / self.count as f32
    }

    fn y_of_height(&self, z: f32) -> f32 {
        self.y_of_layer(((z - self.lo) / self.layer_height).clamp(0.0, self.count as f32))
    }

    /// The layer at a height of the track, from 0.
    fn layer_at(&self, y: f32) -> u32 {
        let t = ((self.bottom - y) / (self.bottom - self.top)).clamp(0.0, 1.0);
        (t * self.count as f32).round() as u32
    }

    /// The layer whose top is nearest a height.
    fn layer_of_height(&self, z: f32) -> u32 {
        ((z - self.lo) / self.layer_height).round().clamp(1.0, self.count as f32) as u32
    }
}

/// A one-line badge whose right edge is at `right`, centred on its height.
fn badge_left_of(painter: &egui::Painter, right: Pos2, text: &str, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let size = galley.size();
    let text = Rect::from_min_size(right - Vec2::new(size.x + 8.0, size.y / 2.0), size);
    painter.rect_filled(text.expand2(Vec2::new(6.0, 3.0)), CornerRadius::same(4), Color32::from_black_alpha(160));
    painter.galley(text.min, galley, color);
}

/// The cyan of the pause planes, for their labels.
const PAUSE_COLOR: Color32 = Color32::from_rgb(115, 230, 255);

/// "Pause 19.4 mm", then the message, if any.
fn pause_label(pause: &c2f_eval::Pause) -> String {
    let height = format!("Pause {} mm", pause.z);
    if pause.message.is_empty() { height } else { format!("{height} · {}", pause.message) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use c2f_geom::{RenderMesh, Stats};

    const STATUS: Status = Status { rendering: false, failed: false, has_model: true, is_2d: false, empty_hint: "" };

    /// Runs one frame of the view, filling an 800 by 600 window, with
    /// `events` and a frame time of `dt` seconds.
    fn frame(ctx: &egui::Context, viewer: &mut Viewer, events: Vec<egui::Event>, dt: f32) {
        let input = egui::RawInput {
            events,
            predicted_dt: dt,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| viewer.show(ui, &STATUS));
        });
    }

    fn scene(size: f32) -> Scene {
        let mut solid = RenderMesh::default();
        solid.push_triangle([[0.0; 3], [size, 0.0, 0.0], [size, size, size]], [[1.0; 4]; 3]);
        Scene {
            solid,
            highlights: RenderMesh::default(),
            backgrounds: RenderMesh::default(),
            is_2d: false,
            stats: Stats::default(),
            formats: &[],
            pauses: Vec::new(),
            source: String::new(),
            parts: Vec::new(),
        }
    }

    /// A 20 mm model in the layer view, with a pause at 10 mm whose height
    /// is the number at bytes 6..8 of its source.
    fn paused_layer_view(ctx: &egui::Context) -> Viewer {
        let mut viewer = Viewer::new(None);
        let mut scene = scene(20.0);
        let source = c2f_eval::PauseSource { call: 0, number: Some(6..8) };
        scene.pauses = vec![c2f_eval::Pause { z: 10.0, message: String::new(), source: Some(source) }];
        viewer.set_scene(scene, false);
        viewer.layer_view = true;
        frame(ctx, &mut viewer, vec![], 1.0 / 60.0);
        viewer
    }

    fn click(at: Pos2, button: PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn a_pause_mark_drags_to_another_layer() {
        let ctx = egui::Context::default();
        let mut viewer = paused_layer_view(&ctx);
        // The track runs from y 56 (layer 100) to 516 (layer 0) at x 766.
        let (mark, to) = (Pos2::new(766.0, 286.0), Pos2::new(766.0, 240.0));
        frame(&ctx, &mut viewer, vec![egui::Event::PointerMoved(mark)], 1.0 / 60.0);
        frame(&ctx, &mut viewer, vec![click(mark, PointerButton::Primary, true)], 1.0 / 60.0);
        frame(&ctx, &mut viewer, vec![egui::Event::PointerMoved(to)], 1.0 / 60.0);
        assert_eq!(viewer.dragging, Some((0, 60)));
        frame(&ctx, &mut viewer, vec![click(to, PointerButton::Primary, false)], 1.0 / 60.0);
        frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        assert_eq!(viewer.take_pause_edit(), Some(PauseEdit::Move { number: 6..8, z: 12.0 }));
        assert_eq!(viewer.layer(), Some(100), "dragging a pause leaves the layer shown");
    }

    #[test]
    fn a_right_click_on_the_slider_targets_a_layer_or_a_pause() {
        let ctx = egui::Context::default();
        let mut viewer = paused_layer_view(&ctx);
        // A quarter of the way up, away from the mark; then on the mark.
        for y in [401.0, 287.0] {
            let at = Pos2::new(766.0, y);
            frame(&ctx, &mut viewer, vec![egui::Event::PointerMoved(at)], 1.0 / 60.0);
            frame(&ctx, &mut viewer, vec![click(at, PointerButton::Secondary, true)], 1.0 / 60.0);
            frame(&ctx, &mut viewer, vec![click(at, PointerButton::Secondary, false)], 1.0 / 60.0);
            match (y, viewer.menu) {
                // The panel's margins shift the track by a pixel or two.
                (401.0, Some(SliderMenu::Layer(layer))) => assert!((24..=26).contains(&layer), "layer {layer}"),
                (287.0, Some(SliderMenu::Pause(0))) => {}
                _ => panic!("a right-click at y {y} has no fitting menu"),
            }
        }
    }

    #[test]
    fn layers_follow_the_model_and_the_layer_height() {
        let mut viewer = Viewer::new(None);
        viewer.set_scene(scene(20.0), false);
        assert_eq!((viewer.layer_count(), viewer.layer()), (Some(100), Some(100)));
        assert_eq!(viewer.cut(), None, "the layer view starts off");
        viewer.layer_view = true;
        assert_eq!(viewer.cut(), Some(20.0));
        viewer.set_layer(92);
        assert!((viewer.cut().unwrap() - 18.4).abs() < 1e-4);
        viewer.set_layer(500);
        assert_eq!((viewer.layer(), viewer.cut()), (Some(100), Some(20.0)));
        // A last layer thinner than the others ends at the top.
        viewer.layer_height = 0.3;
        assert_eq!((viewer.layer_count(), viewer.cut()), (Some(67), Some(20.0)));
        // A smaller model keeps the layer when it still has it.
        viewer.layer_height = 0.2;
        viewer.set_layer(30);
        viewer.set_scene(scene(10.0), false);
        assert_eq!((viewer.layer_count(), viewer.layer()), (Some(50), Some(30)));
        viewer.set_scene(scene(4.0), false);
        assert_eq!(viewer.layer(), Some(20));
    }

    #[test]
    fn a_click_on_the_slider_picks_a_layer() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(None);
        viewer.set_scene(scene(20.0), false);
        viewer.layer_view = true;
        frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        // The track runs from y 56 to 516 at x 766 in an 800 by 600 view.
        let at = Pos2::new(766.0, 286.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut viewer, vec![egui::Event::PointerMoved(at), press(true)], 1.0 / 60.0);
        frame(&ctx, &mut viewer, vec![press(false)], 1.0 / 60.0);
        assert_eq!(viewer.layer(), Some(50));
        let camera = viewer.camera;
        assert_eq!((camera.yaw, camera.pitch), (Viewer::new(None).camera.yaw, Viewer::new(None).camera.pitch));
    }

    #[test]
    fn pauses_get_a_rectangle_around_the_model_and_a_label() {
        let mut viewer = Viewer::new(None);
        viewer.set_scene(scene(10.0), false);
        assert!(!viewer.has_pauses());
        let mut with_pauses = scene(10.0);
        with_pauses.pauses = vec![
            c2f_eval::Pause { z: 3.0, message: String::new(), source: None },
            c2f_eval::Pause { z: 9.4, message: "Magnets".to_string(), source: None },
        ];
        viewer.set_scene(with_pauses, false);
        assert!(viewer.has_pauses());
        let labels: Vec<&str> = viewer.pauses.iter().map(|mark| mark.label.as_str()).collect();
        assert_eq!(labels, ["Pause 3 mm", "Pause 9.4 mm · Magnets"]);
        // The 10 mm footprint, with an 8% margin on each side.
        let corners = &viewer.pauses[1].corners;
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(near(corners[0], [-0.8, -0.8, 9.4]), "{corners:?}");
        assert!(near(corners[2], [10.8, 10.8, 9.4]), "{corners:?}");
        // Drawing the labels needs no GPU.
        frame(&egui::Context::default(), &mut viewer, vec![], 1.0 / 60.0);
    }

    #[test]
    fn turntable_turns_the_yaw_and_keeps_tilt_and_zoom() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(None);
        frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        let before = viewer.camera;
        viewer.turntable = true;
        for _ in 0..30 {
            frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        }
        assert!(viewer.turntable);
        assert!((viewer.camera.yaw - before.yaw).abs() > 0.1, "the camera did not turn");
        assert_eq!(
            (viewer.camera.pitch, viewer.camera.distance, viewer.camera.target),
            (before.pitch, before.distance, before.target)
        );
        assert!(ctx.has_requested_repaint(), "the turntable must keep frames coming");
    }

    #[test]
    fn a_drag_in_the_view_stops_the_turntable() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(None);
        viewer.turntable = true;
        let at = Pos2::new(400.0, 300.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut viewer, vec![egui::Event::PointerMoved(at)], 1.0 / 60.0);
        frame(&ctx, &mut viewer, vec![press(true)], 1.0 / 60.0);
        assert!(viewer.turntable, "a press alone does not stop it");
        for step in 1..=5 {
            let to = at + Vec2::new(step as f32 * 10.0, 0.0);
            frame(&ctx, &mut viewer, vec![egui::Event::PointerMoved(to)], 1.0 / 60.0);
        }
        assert!(!viewer.turntable);
        frame(&ctx, &mut viewer, vec![press(false)], 1.0 / 60.0);
        let yaw = viewer.camera.yaw;
        frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        assert_eq!(viewer.camera.yaw, yaw, "the camera stays where the drag left it");
    }

    #[test]
    fn a_new_render_keeps_the_turntable_going() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer::new(None);
        viewer.turntable = true;
        frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        viewer.set_scene(scene(10.0), true);
        viewer.set_scene(scene(30.0), false);
        assert!(viewer.turntable);
        let yaw = viewer.camera.yaw;
        frame(&ctx, &mut viewer, vec![], 1.0 / 60.0);
        assert_ne!(viewer.camera.yaw, yaw);
    }
}
