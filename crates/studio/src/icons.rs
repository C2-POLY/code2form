//! The toolbars' icons, drawn as strokes in the theme's colors, and the
//! square buttons that hold them.

use std::f32::consts::{PI, TAU};

use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, vec2,
};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Frame the model: four corners.
    Fit,
    Grid,
    Axes,
    Layers,
    Pauses,
    /// The parts drawn apart.
    Explode,
    /// A box cut open by a plane.
    Section,
    Settings,
    Play,
    Pause,
    /// Back to the values written in the file.
    Reset,
    /// A card folded to its title, or unfolded.
    FoldUp,
    FoldDown,
    /// A row folded to its title, as a tool call in the Agent panel.
    FoldRight,
    /// The window's layouts: the code alone, the code beside the view, the
    /// view alone.
    CodeOnly,
    SideBySide,
    ViewOnly,
    /// The agent's chat: a speech bubble.
    Agent,
    /// The agent's prompts: a checklist to plan, a pencil to design, a
    /// chip for the electronics.
    Plan,
    Design,
    Electronics,
    /// A picture of the view: a camera.
    Camera,
    /// A card folded to a bar, or closed.
    Minimise,
    Close,
    /// A file to open: a folder.
    Folder,
}

/// A button holding `icon`, pressed-looking when `on`.
pub fn button(ui: &mut egui::Ui, icon: Icon, on: bool, hint: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::click());
    let palette = theme::palette();
    let fill = if on {
        palette.accent_muted
    } else if response.hovered() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let color = if on {
        on_muted()
    } else if response.hovered() {
        palette.text_strong
    } else {
        palette.text
    };
    paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
        icon,
        color,
    );
    response.on_hover_text(hint)
}

/// A button with `icon` before `text`, in the style of the app's buttons,
/// pressed-looking when `on`.
pub fn labelled(ui: &mut egui::Ui, icon: Icon, text: &str, on: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(12.5), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 38.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, on);
        let fill = if on { theme::palette().accent_muted } else { visuals.weak_bg_fill };
        ui.painter().rect(rect, CornerRadius::same(6), fill, visuals.bg_stroke, StrokeKind::Inside);
        let color = if on { on_muted() } else { visuals.text_color() };
        let center = rect.center().y;
        paint(ui.painter(), Rect::from_center_size(Pos2::new(rect.left() + 17.0, center), Vec2::splat(15.0)), icon, color);
        ui.painter().galley(Pos2::new(rect.left() + 29.0, center - galley.size().y / 2.0), galley, color);
    }
    response
}

/// Text and icons over `accent_muted`, as on a pressed toggle.
pub fn on_muted() -> Color32 {
    let palette = theme::palette();
    if palette.dark {
        palette.text_strong
    } else {
        palette.accent_strong
    }
}

/// Draws `icon` in `rect`, a square, in `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width() / 20.0;
    // A point of the 20 by 20 grid the icons are drawn on.
    let p = |x: f32, y: f32| rect.min + vec2(x * s, y * s);
    let stroke = Stroke::new(1.5 * s.max(0.9), color);
    let line = |points: &[(f32, f32)]| {
        painter.add(Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::Fit => {
            line(&[(3.0, 7.0), (3.0, 3.0), (7.0, 3.0)]);
            line(&[(13.0, 3.0), (17.0, 3.0), (17.0, 7.0)]);
            line(&[(17.0, 13.0), (17.0, 17.0), (13.0, 17.0)]);
            line(&[(7.0, 17.0), (3.0, 17.0), (3.0, 13.0)]);
        }
        Icon::Grid => {
            painter.rect_stroke(
                Rect::from_min_max(p(3.0, 3.0), p(17.0, 17.0)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Middle,
            );
            for at in [7.67, 12.33] {
                line(&[(3.0, at), (17.0, at)]);
                line(&[(at, 3.0), (at, 17.0)]);
            }
        }
        Icon::Axes => {
            line(&[(4.0, 4.0), (4.0, 16.0), (16.0, 16.0)]);
            line(&[(4.0, 16.0), (12.0, 8.0)]);
        }
        Icon::Layers => {
            line(&[
                (10.0, 3.0),
                (17.0, 7.0),
                (10.0, 11.0),
                (3.0, 7.0),
                (10.0, 3.0),
            ]);
            line(&[(3.0, 11.0), (10.0, 15.0), (17.0, 11.0)]);
            line(&[(3.0, 14.5), (10.0, 18.5), (17.0, 14.5)]);
        }
        Icon::Pauses => {
            for x in [6.0, 11.5] {
                painter.rect_stroke(
                    Rect::from_min_max(p(x, 4.0), p(x + 2.5, 16.0)),
                    CornerRadius::same(1),
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        Icon::Explode => {
            for (x, y) in [(2.5, 2.5), (11.5, 2.5), (2.5, 11.5), (11.5, 11.5)] {
                painter.rect_stroke(
                    Rect::from_min_max(p(x, y), p(x + 6.0, y + 6.0)),
                    CornerRadius::same(1),
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        Icon::Section => {
            // The box's kept half, its cut face hatched, and the plane
            // standing past it.
            line(&[(10.0, 5.0), (3.0, 5.0), (3.0, 15.0), (10.0, 15.0)]);
            for (x, y) in [(3.0, 9.0), (3.0, 13.0), (5.0, 15.0)] {
                line(&[(x, y), (x + (10.0 - x).min(y - 5.0), y - (10.0 - x).min(y - 5.0))]);
            }
            line(&[(10.0, 2.5), (17.0, 5.5), (17.0, 17.5), (10.0, 14.5), (10.0, 2.5)]);
        }
        Icon::Reset => {
            // Three quarters of a circle, turning back, with its arrowhead.
            let center = p(10.0, 10.0);
            let r = 6.0 * s;
            let points: Vec<Pos2> = (0..=24)
                .map(|i| center + Vec2::angled(PI - i as f32 / 24.0 * 1.5 * PI) * r)
                .collect();
            let start = points[0];
            painter.add(Shape::line(points, stroke));
            painter.add(Shape::line(
                vec![
                    start + vec2(-2.5 * s, -3.0 * s),
                    start,
                    start + vec2(3.0 * s, -2.0 * s),
                ],
                stroke,
            ));
        }
        Icon::Settings => {
            let center = p(10.0, 10.0);
            painter.circle_stroke(center, 3.0 * s, stroke);
            painter.circle_stroke(center, 6.2 * s, stroke);
            for i in 0..8 {
                let dir = Vec2::angled(i as f32 / 8.0 * TAU);
                painter.line_segment([center + dir * 6.2 * s, center + dir * 8.4 * s], stroke);
            }
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![p(6.0, 4.0), p(16.0, 10.0), p(6.0, 16.0)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            for x in [5.5, 11.5] {
                painter.rect_filled(
                    Rect::from_min_max(p(x, 4.0), p(x + 3.0, 16.0)),
                    CornerRadius::same(1),
                    color,
                );
            }
        }
        Icon::FoldUp => line(&[(6.0, 12.0), (10.0, 8.0), (14.0, 12.0)]),
        Icon::FoldDown => line(&[(6.0, 8.0), (10.0, 12.0), (14.0, 8.0)]),
        Icon::FoldRight => line(&[(8.0, 6.0), (12.0, 10.0), (8.0, 14.0)]),
        Icon::CodeOnly => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.5, 4.0), p(17.5, 16.0)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            line(&[(5.5, 8.0), (11.5, 8.0)]);
            line(&[(5.5, 11.5), (14.0, 11.5)]);
        }
        Icon::SideBySide => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.5, 4.0), p(17.5, 16.0)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            line(&[(9.0, 4.0), (9.0, 16.0)]);
        }
        Icon::ViewOnly => {
            line(&[
                (10.0, 2.5),
                (16.5, 6.25),
                (16.5, 13.75),
                (10.0, 17.5),
                (3.5, 13.75),
                (3.5, 6.25),
                (10.0, 2.5),
            ]);
            line(&[(3.5, 6.25), (10.0, 10.0), (16.5, 6.25)]);
            line(&[(10.0, 10.0), (10.0, 17.5)]);
        }
        Icon::Agent => {
            // The bubble, its tail at the bottom left, and three dots.
            painter.rect_stroke(
                Rect::from_min_max(p(2.5, 3.5), p(17.5, 14.0)),
                CornerRadius::same(3),
                stroke,
                StrokeKind::Middle,
            );
            line(&[(5.5, 14.0), (5.0, 17.5), (9.5, 14.0)]);
            for x in [6.5, 10.0, 13.5] {
                painter.circle_filled(p(x, 8.75), 1.1 * s, color);
            }
        }
        Icon::Plan => {
            for y in [5.0, 10.0, 15.0] {
                line(&[(3.0, y), (4.5, y + 1.5), (7.0, y - 1.5)]);
                line(&[(9.5, y), (17.0, y)]);
            }
        }
        Icon::Design => {
            // A pencil, its point at the bottom left, and the line it drew.
            line(&[(4.0, 16.0), (5.0, 12.5), (14.0, 3.5), (16.5, 6.0), (7.5, 15.0), (4.0, 16.0)]);
            line(&[(12.0, 5.5), (14.5, 8.0)]);
            line(&[(10.0, 17.0), (17.0, 17.0)]);
        }
        Icon::Electronics => {
            painter.rect_stroke(
                Rect::from_min_max(p(5.5, 5.5), p(14.5, 14.5)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Middle,
            );
            for at in [8.0, 12.0] {
                line(&[(at, 2.5), (at, 5.5)]);
                line(&[(at, 14.5), (at, 17.5)]);
                line(&[(2.5, at), (5.5, at)]);
                line(&[(14.5, at), (17.5, at)]);
            }
        }
        Icon::Camera => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.5, 6.0), p(17.5, 16.0)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            line(&[(7.0, 6.0), (8.5, 3.5), (11.5, 3.5), (13.0, 6.0)]);
            painter.circle_stroke(p(10.0, 11.0), 3.0 * s, stroke);
        }
        Icon::Minimise => line(&[(5.0, 14.0), (15.0, 14.0)]),
        Icon::Close => {
            line(&[(5.5, 5.5), (14.5, 14.5)]);
            line(&[(14.5, 5.5), (5.5, 14.5)]);
        }
        Icon::Folder => {
            line(&[(2.5, 16.0), (2.5, 4.0), (8.0, 4.0), (9.5, 6.0), (17.5, 6.0), (17.5, 16.0), (2.5, 16.0)]);
            line(&[(2.5, 9.0), (17.5, 9.0)]);
        }
    }
}
