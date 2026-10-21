//! The 3D view: camera controls, the OpenGL paint callback and the
//! overlays drawn on top (hints, status badges, the axis gizmo).

use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use eframe::{egui_glow, glow};
use glam::{Vec2 as GVec2, Vec3};

use crate::camera::Camera;
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
    bounds: Option<(Vec3, Vec3)>,
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
            bounds: None,
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

    /// The box to frame: the model, or a default 20-unit cube around the
    /// origin when there is none.
    fn frame_box(&self) -> (Vec3, Vec3) {
        self.bounds.unwrap_or((Vec3::splat(-10.0), Vec3::splat(10.0)))
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
        if let Some((l, h)) = self.bounds {
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

    pub fn show(&mut self, ui: &mut egui::Ui, status: &Status) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        self.aspect = rect.width() / rect.height();
        self.handle_input(ui, rect, &response);

        let view = self.camera.view();
        let projection = self.camera.projection(self.aspect, self.scene_radius());
        match &self.renderer {
            Ok(renderer) => {
                let frame = renderer::Frame { view, projection, show_grid: self.show_grid, show_axes: self.show_axes };
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
        self.paint_gizmo(&painter, rect);
        paint_overlays(ui, &painter, rect, status);
    }

    fn handle_input(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
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
