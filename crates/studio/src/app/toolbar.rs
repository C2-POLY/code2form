//! The modelling toolbar above the view, as Fusion's: one row of tools in
//! groups (SKETCH, CREATE, MODIFY, COMBINE), each tool an icon over its
//! name and each group named under its tools. Too narrow a view keeps the
//! icons alone, the names in their tooltips.

use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use super::StudioApp;
use crate::theme;

/// A tool's size, with its name under the icon and without.
const TOOL: Vec2 = Vec2::new(48.0, 46.0);
const COMPACT_TOOL: Vec2 = Vec2::new(34.0, 34.0);
const ICON: f32 = 20.0;
/// The tools and the separators of the row, for the width it needs.
const TOOLS: f32 = 14.0;
const SEPARATORS: f32 = 3.0;
const SEPARATOR: f32 = 14.0;
/// The space between two tools, or a tool and a separator.
const GAP: f32 = 1.0;

/// What a tool's icon draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Icon {
    Sketch,
    EditSketch,
    Cube,
    Cylinder,
    Sphere,
    Cone,
    Tube,
    Text,
    Hole,
    Fillet,
    Chamfer,
    Unite,
    Subtract,
    Intersect,
    // The view bar's and the top bar's.
    Fit,
    Grid,
    Axes,
    Layers,
    Pauses,
    Help,
    Gear,
}

/// How the row lays its tools out this frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    compact: bool,
}

impl StudioApp {
    /// The row of tools across the top of the 3D view. Added after the
    /// side panels, it spans the view only.
    pub(super) fn modelling_toolbar(&mut self, ctx: &egui::Context) {
        let palette = theme::palette();
        let frame = egui::Frame::new().fill(palette.panel).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 3 });
        egui::TopBottomPanel::top("modelling-toolbar").frame(frame).show_separator_line(true).show(ctx, |ui| {
            // The names under the icons when they fit.
            let full = TOOLS * (TOOL.x + GAP) + SEPARATORS * (SEPARATOR + GAP);
            let row = Row { compact: ui.available_width() < full };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(GAP, 0.0);
                self.sketch_tools(ctx, ui, row);
                separator(ui);
                self.shape_tools(ctx, ui, row);
            });
        });
    }
}

/// A group of tools and its name under them.
pub(super) fn group<R>(ui: &mut egui::Ui, name: &str, row: Row, tools: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let inner = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            tools(ui)
        });
        if !row.compact {
            let width = inner.response.rect.width();
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
            let font = FontId::proportional(9.5);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, name, font, theme::palette().text_dim);
        }
        inner.inner
    })
    .inner
}

/// The line between two groups.
pub(super) fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(SEPARATOR, TOOL.y), Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(x, rect.y_range().shrink(6.0), Stroke::new(1.0, theme::palette().border));
}

/// A tool: its icon, its name under it unless the row is compact, filled
/// while `on`. A disabled one shows `why` on hover.
pub(super) fn tool(
    ui: &mut egui::Ui,
    row: Row,
    icon: Icon,
    name: &str,
    on: bool,
    enabled: Result<(), &str>,
    tip: &str,
) -> egui::Response {
    let palette = theme::palette();
    let size = if row.compact { COMPACT_TOOL } else { TOOL };
    let sense = if enabled.is_ok() { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hovered = enabled.is_ok() && response.hovered();
    let fill = if on {
        palette.accent_muted
    } else if hovered {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    let color = match (enabled, on) {
        (Err(_), _) => palette.text_faint,
        (Ok(()), true) => palette.text_strong,
        (Ok(()), false) => icon_color(icon),
    };
    let icon_center = if row.compact { rect.center() } else { Pos2::new(rect.center().x, rect.top() + 15.0) };
    paint(painter, Rect::from_center_size(icon_center, Vec2::splat(ICON)), icon, color);
    if !row.compact {
        let text = if enabled.is_ok() { palette.text } else { palette.text_faint };
        let at = Pos2::new(rect.center().x, rect.bottom() - 9.0);
        painter.text(at, Align2::CENTER_CENTER, name, FontId::proportional(10.5), text);
    }
    let response = if row.compact { response.on_hover_text(format!("{name}: {tip}")) } else { response.on_hover_text(tip) };
    match enabled {
        Ok(()) => response.on_hover_cursor(egui::CursorIcon::PointingHand),
        Err(why) => response.on_hover_text(why),
    }
}

/// A small button with an icon only, as the view bar's: filled while `on`.
pub(super) fn icon_button(ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str) -> egui::Response {
    let palette = theme::palette();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0, 28.0), Sense::click());
    let fill = if on {
        palette.accent_muted
    } else if response.hovered() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let color = if on { palette.text_strong } else { palette.text };
    paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(16.0)), icon, color);
    response.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An icon's own color: blue for sketches, orange for holes, the text's
/// for the rest.
fn icon_color(icon: Icon) -> Color32 {
    let palette = theme::palette();
    match icon {
        Icon::Sketch | Icon::EditSketch => palette.accent_strong,
        Icon::Hole => palette.warning,
        _ => palette.text,
    }
}

/// Draws `icon` in `rect`, a square, with strokes of `color`.
pub(super) fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let r = rect.width() / 2.0;
    let stroke = Stroke::new(1.4, color);
    let at = |x: f32, y: f32| c + Vec2::new(x, y) * r;
    let line = |points: Vec<Pos2>| Shape::line(points, stroke);
    match icon {
        Icon::Sketch | Icon::EditSketch => {
            let frame = Rect::from_center_size(c, Vec2::splat(r * 1.7));
            let corners = [frame.left_top(), frame.right_top(), frame.right_bottom(), frame.left_bottom(), frame.left_top()];
            if icon == Icon::Sketch {
                painter.extend(Shape::dashed_line(&corners, stroke, 2.5, 2.0));
            } else {
                painter.add(line(corners.to_vec()));
            }
            // A pencil across it.
            painter.line_segment([at(-0.45, 0.55), at(0.55, -0.45)], Stroke::new(2.4, color));
            painter.line_segment([at(-0.62, 0.72), at(-0.45, 0.55)], Stroke::new(1.2, color));
        }
        Icon::Cube => {
            let top = vec![at(0.0, -0.9), at(0.8, -0.5), at(0.0, -0.1), at(-0.8, -0.5), at(0.0, -0.9)];
            painter.add(line(top));
            painter.add(line(vec![at(-0.8, -0.5), at(-0.8, 0.45), at(0.0, 0.9), at(0.8, 0.45), at(0.8, -0.5)]));
            painter.line_segment([at(0.0, -0.1), at(0.0, 0.9)], stroke);
        }
        Icon::Cylinder | Icon::Tube | Icon::Hole => {
            let (rx, ry) = (0.75, 0.28);
            painter.add(line(ellipse(at(0.0, -0.6), rx * r, ry * r, 0.0, TAU)));
            let sides = [[at(-rx, -0.6), at(-rx, 0.6)], [at(rx, -0.6), at(rx, 0.6)]];
            let bottom = ellipse(at(0.0, 0.6), rx * r, ry * r, 0.0, PI);
            if icon == Icon::Hole {
                for side in sides {
                    painter.extend(Shape::dashed_line(&side, stroke, 2.0, 2.0));
                }
                painter.extend(Shape::dashed_line(&bottom, stroke, 2.0, 2.0));
            } else {
                for side in sides {
                    painter.line_segment(side, stroke);
                }
                painter.add(line(bottom));
            }
            if icon == Icon::Tube {
                painter.add(line(ellipse(at(0.0, -0.6), 0.38 * r, 0.13 * r, 0.0, TAU)));
            }
        }
        Icon::Sphere => {
            painter.circle_stroke(c, r * 0.85, stroke);
            painter.add(line(ellipse(c, r * 0.85, r * 0.3, 0.0, PI)));
        }
        Icon::Cone => {
            painter.add(line(ellipse(at(0.0, 0.6), 0.75 * r, 0.28 * r, 0.0, TAU)));
            painter.add(line(vec![at(-0.75, 0.6), at(0.0, -0.85), at(0.75, 0.6)]));
        }
        Icon::Text => {
            painter.line_segment([at(-0.7, -0.75), at(0.7, -0.75)], Stroke::new(1.8, color));
            painter.line_segment([at(0.0, -0.75), at(0.0, 0.85)], Stroke::new(1.8, color));
        }
        Icon::Fillet => {
            let mut points = vec![at(-0.7, 0.85)];
            points.extend(ellipse(at(0.1, -0.05), 0.8 * r, 0.8 * r, PI, 1.5 * PI));
            points.push(at(0.85, -0.85));
            painter.add(line(points));
        }
        Icon::Chamfer => {
            painter.add(line(vec![at(-0.7, 0.85), at(-0.7, -0.15), at(0.05, -0.85), at(0.85, -0.85)]));
        }
        Icon::Unite | Icon::Subtract | Icon::Intersect => {
            let (left, right) = (at(-0.32, 0.0), at(0.32, 0.0));
            let radius = r * 0.62;
            if icon != Icon::Subtract {
                // The part both circles share, filled.
                let half = (0.32 / 0.62_f32).acos();
                let mut lens = ellipse(left, radius, radius, -half, half);
                lens.extend(ellipse(right, radius, radius, PI - half, PI + half));
                painter.add(Shape::convex_polygon(lens, color.gamma_multiply(0.45), Stroke::NONE));
            }
            if icon == Icon::Unite {
                painter.circle_filled(left, radius, color.gamma_multiply(0.2));
                painter.circle_filled(right, radius, color.gamma_multiply(0.2));
            }
            painter.circle_stroke(left, radius, stroke);
            if icon == Icon::Subtract {
                painter.extend(Shape::dashed_line(&ellipse(right, radius, radius, 0.0, TAU), stroke, 2.0, 2.0));
            } else {
                painter.circle_stroke(right, radius, stroke);
            }
        }
        Icon::Fit => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let corner = at(0.85 * x, 0.85 * y);
                painter.add(line(vec![corner + Vec2::new(0.0, -0.5 * y * r), corner, corner + Vec2::new(-0.5 * x * r, 0.0)]));
            }
        }
        Icon::Grid => {
            painter.rect_stroke(Rect::from_center_size(c, Vec2::splat(r * 1.7)), 1.0, stroke, egui::StrokeKind::Middle);
            for t in [-0.28, 0.28] {
                painter.line_segment([at(t, -0.85), at(t, 0.85)], stroke);
                painter.line_segment([at(-0.85, t), at(0.85, t)], stroke);
            }
        }
        Icon::Axes => {
            painter.add(line(vec![at(-0.75, -0.85), at(-0.75, 0.75), at(0.85, 0.75)]));
            painter.line_segment([at(-0.75, 0.75), at(0.3, -0.3)], stroke);
        }
        Icon::Layers => {
            painter.add(Shape::closed_line(vec![at(0.0, -0.8), at(0.85, -0.35), at(0.0, 0.1), at(-0.85, -0.35)], stroke));
            painter.add(line(vec![at(-0.85, 0.15), at(0.0, 0.6), at(0.85, 0.15)]));
        }
        Icon::Pauses => {
            for x in [-0.3, 0.3] {
                painter.line_segment([at(x, -0.65), at(x, 0.65)], Stroke::new(2.4, color));
            }
        }
        Icon::Help => {
            painter.circle_stroke(c, r * 0.85, stroke);
            painter.text(c, Align2::CENTER_CENTER, "?", FontId::proportional(r * 1.15), color);
        }
        Icon::Gear => {
            painter.circle_stroke(c, r * 0.38, stroke);
            for i in 0..8 {
                let a = TAU * i as f32 / 8.0;
                let d = Vec2::new(a.cos(), a.sin());
                painter.line_segment([c + d * r * 0.55, c + d * r * 0.9], Stroke::new(2.0, color));
            }
            painter.circle_stroke(c, r * 0.62, stroke);
        }
    }
}

/// Points along an ellipse around `c`, from angle `from` to `to`, y down.
fn ellipse(c: Pos2, rx: f32, ry: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = 24;
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            c + Vec2::new(a.cos() * rx, a.sin() * ry)
        })
        .collect()
}
