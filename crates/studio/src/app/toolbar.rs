//! The toolbar across the window under the top bar, as Fusion's: one row
//! of tools in groups, each tool an icon over its name and each group named
//! under its tools. Modelling, it holds SKETCH, CREATE, MODIFY and COMBINE;
//! while a sketch is open, the sketch's own DRAW and CONSTRAIN, and the
//! button that finishes it. Too narrow a window keeps the icons alone, the
//! names in their tooltips.

use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use super::StudioApp;
use crate::theme;

/// A tool's size, with its name under the icon and without.
const TOOL: Vec2 = Vec2::new(48.0, 46.0);
const COMPACT_TOOL: Vec2 = Vec2::new(34.0, 34.0);
const ICON: f32 = 20.0;
/// The tools and the separators of the modelling row, and the sketch's
/// with its context and its buttons, for the width they need.
const TOOLS: f32 = 14.0;
const SEPARATORS: f32 = 3.0;
const SKETCH_ROW: f32 = 1060.0;
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
    // The sketch's.
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Horizontal,
    Vertical,
    Coincident,
    Parallel,
    Perpendicular,
    Tangent,
    Equal,
    Fix,
    Dimension,
}

/// How the row lays its tools out this frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    compact: bool,
}

impl StudioApp {
    /// The toolbar across the window, under the top bar: the sketch's
    /// while one is open, else the modelling tools in the Model workspace.
    /// Added before the side panels, it spans them too.
    pub(super) fn tools_panel(&mut self, ctx: &egui::Context) {
        if self.doc.is_circuit() {
            return;
        }
        let sketching = self.doc.sketch.active();
        if !sketching && !self.models() {
            return;
        }
        let palette = theme::palette();
        // A tint of the accent: the window is in the sketch mode.
        let fill = if sketching { palette.panel.lerp_to_gamma(palette.accent, 0.07) } else { palette.panel };
        let frame = egui::Frame::new().fill(fill).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 3 });
        egui::TopBottomPanel::top("modelling-toolbar").frame(frame).show_separator_line(true).show(ctx, |ui| {
            // The names under the icons when they fit.
            let full = if sketching {
                SKETCH_ROW
            } else {
                TOOLS * (TOOL.x + GAP) + SEPARATORS * (SEPARATOR + GAP)
            };
            let row = Row { compact: ui.available_width() < full };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(GAP, 0.0);
                if sketching {
                    self.sketch_toolbar(ctx, ui, row);
                } else {
                    self.sketch_tools(ctx, ui, row);
                    separator(ui);
                    self.shape_tools(ctx, ui, row);
                }
            });
            if sketching {
                // The row's edge in the accent: the window is in a mode.
                let rect = ui.max_rect().expand2(Vec2::new(8.0, 3.0));
                let painter = ui.painter().with_clip_rect(rect);
                painter.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, palette.accent));
            }
        });
    }
}

/// What the toolbar is about, at its left: `kind` in small capitals over
/// `detail`.
pub(super) fn context(ui: &mut egui::Ui, kind: &str, detail: &str) {
    let palette = theme::palette();
    ui.add_space(4.0);
    ui.vertical(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(kind).size(10.0).strong().color(palette.accent_strong));
        ui.label(egui::RichText::new(detail).monospace().size(12.5).color(palette.text));
    });
    ui.add_space(8.0);
}

/// The green button that ends a mode, as Finish sketch.
pub(super) fn finish_button(ui: &mut egui::Ui, label: &str, tip: &str) -> egui::Response {
    let palette = theme::palette();
    let text = if palette.dark { Color32::from_rgb(13, 26, 18) } else { Color32::WHITE };
    // Room on the left for the check mark, drawn: the fonts have none.
    let button = egui::Button::new(egui::RichText::new(format!("      {label}")).strong().color(text))
        .fill(palette.success)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 34.0));
    let response = ui.add(button);
    // Its name without the room left for the mark.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let c = Pos2::new(response.rect.left() + 17.0, response.rect.center().y);
    let check = vec![c + Vec2::new(-5.0, 0.5), c + Vec2::new(-1.5, 4.0), c + Vec2::new(5.0, -4.0)];
    ui.painter().add(Shape::line(check, Stroke::new(2.0, text)));
    response.on_hover_text(tip)
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
    let font = FontId::proportional(10.5);
    let size = if row.compact {
        COMPACT_TOOL
    } else {
        let name_width = ui.painter().layout_no_wrap(name.to_string(), font.clone(), palette.text).size().x;
        Vec2::new(TOOL.x.max(name_width + 10.0), TOOL.y)
    };
    let sense = if enabled.is_ok() { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    // Named for screen readers, as a button is.
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled.is_ok(), on, name));
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
        painter.text(at, Align2::CENTER_CENTER, name, font, text);
    }
    let response = if row.compact { response.on_hover_text(format!("{name}: {tip}")) } else { response.on_hover_text(tip) };
    match enabled {
        Ok(()) => response.on_hover_cursor(egui::CursorIcon::PointingHand),
        Err(why) => response.on_hover_text(why),
    }
}

/// An icon's own color: blue for sketches, orange for holes, the text's
/// for the rest.
fn icon_color(icon: Icon) -> Color32 {
    let palette = theme::palette();
    match icon {
        Icon::Sketch | Icon::EditSketch => palette.accent_strong,
        Icon::Hole | Icon::Dimension => palette.warning,
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
        Icon::Select => {
            let arrow = vec![at(-0.5, -0.85), at(0.55, 0.05), at(0.05, 0.12), at(-0.2, 0.75), at(-0.5, -0.85)];
            painter.add(Shape::closed_line(arrow, stroke));
        }
        Icon::Line => {
            painter.line_segment([at(-0.7, 0.7), at(0.7, -0.7)], stroke);
            painter.circle_filled(at(-0.7, 0.7), 2.2, color);
            painter.circle_filled(at(0.7, -0.7), 2.2, color);
        }
        Icon::Rectangle => {
            painter.rect_stroke(Rect::from_center_size(c, Vec2::new(r * 1.7, r * 1.2)), 1.0, stroke, egui::StrokeKind::Middle);
        }
        Icon::Circle => {
            painter.circle_stroke(c, r * 0.8, stroke);
            painter.circle_filled(c, 1.5, color);
        }
        Icon::Arc => {
            painter.add(line(ellipse(at(0.0, 0.45), r * 0.8, r * 0.8, PI, TAU)));
        }
        Icon::Horizontal => {
            painter.line_segment([at(-0.85, 0.0), at(0.85, 0.0)], Stroke::new(1.8, color));
            painter.text(at(0.0, -0.55), Align2::CENTER_CENTER, "H", FontId::proportional(r * 0.8), color);
        }
        Icon::Vertical => {
            painter.line_segment([at(0.2, -0.85), at(0.2, 0.85)], Stroke::new(1.8, color));
            painter.text(at(-0.45, 0.0), Align2::CENTER_CENTER, "V", FontId::proportional(r * 0.8), color);
        }
        Icon::Coincident => {
            painter.line_segment([at(-0.85, 0.7), at(0.0, 0.0)], stroke);
            painter.line_segment([at(0.85, 0.7), at(0.0, 0.0)], stroke);
            painter.circle_filled(c, 2.6, color);
        }
        Icon::Parallel => {
            painter.line_segment([at(-0.6, 0.8), at(0.0, -0.8)], stroke);
            painter.line_segment([at(0.0, 0.8), at(0.6, -0.8)], stroke);
        }
        Icon::Perpendicular => {
            painter.line_segment([at(-0.85, 0.75), at(0.85, 0.75)], stroke);
            painter.line_segment([at(0.0, 0.75), at(0.0, -0.85)], stroke);
            painter.rect_stroke(Rect::from_min_size(at(0.0, 0.4), Vec2::splat(r * 0.35)), 0.0, stroke, egui::StrokeKind::Middle);
        }
        Icon::Tangent => {
            painter.circle_stroke(at(0.0, 0.25), r * 0.6, stroke);
            painter.line_segment([at(-0.9, -0.35), at(0.9, -0.35)], stroke);
        }
        Icon::Equal => {
            painter.line_segment([at(-0.7, -0.3), at(0.7, -0.3)], Stroke::new(1.8, color));
            painter.line_segment([at(-0.7, 0.3), at(0.7, 0.3)], Stroke::new(1.8, color));
        }
        Icon::Fix => {
            // A padlock.
            painter.rect_stroke(Rect::from_min_max(at(-0.6, -0.1), at(0.6, 0.85)), 2.0, stroke, egui::StrokeKind::Middle);
            painter.add(line(ellipse(at(0.0, -0.1), r * 0.38, r * 0.55, PI, TAU)));
        }
        Icon::Dimension => {
            painter.line_segment([at(-0.85, 0.2), at(0.85, 0.2)], stroke);
            for x in [-0.85, 0.85] {
                painter.line_segment([at(x, -0.3), at(x, 0.7)], stroke);
            }
            painter.add(line(vec![at(-0.55, 0.0), at(-0.85, 0.2), at(-0.55, 0.4)]));
            painter.add(line(vec![at(0.55, 0.0), at(0.85, 0.2), at(0.55, 0.4)]));
            painter.line_segment([at(-0.35, -0.6), at(0.35, -0.6)], stroke);
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
