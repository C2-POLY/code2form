//! The view bar, floating at the bottom of the 3D view: fit the model,
//! the views, the grid, the axes, the layers and the pauses, and the
//! mouse's help on hover.

use eframe::egui::{self, Align2, CornerRadius, Frame, Id, Margin, Rect, Stroke, Vec2};

use super::StudioApp;
use super::toolbar::{self, Icon};
use crate::{theme, viewer};

/// What the mouse does in the view.
const MOUSE: &str = "Click: select\nShift-click: add to the selection\nAlt-click: select what is hidden\n\
Drag: orbit\nRight-drag or Shift-drag: pan\nScroll: zoom\nDouble-click: fit the model";

/// How far above the view's bottom edge the bar floats.
const MARGIN: f32 = 12.0;

/// The height the bar takes at the bottom of the view, for what floats
/// above it.
pub(super) const HEIGHT: f32 = 36.0 + MARGIN;

impl StudioApp {
    /// The view bar, centered at the bottom of `view`.
    pub(super) fn view_bar(&mut self, ctx: &egui::Context, view: Rect) {
        let palette = theme::palette();
        let frame = Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(4));
        egui::Area::new(Id::new("view-bar"))
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(egui::pos2(view.center().x, view.bottom() - MARGIN))
            .constrain_to(view)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        self.view_bar_contents(ui);
                    });
                });
            });
    }

    fn view_bar_contents(&mut self, ui: &mut egui::Ui) {
        let viewer = &mut self.doc.viewer;
        let fit = "Fit the model, from the default angle (or double-click the view)";
        if toolbar::icon_button(ui, Icon::Fit, false, fit).clicked() {
            viewer.reset();
        }
        ui.menu_button("Views", |ui| {
            for &(name, yaw, pitch) in viewer::PRESETS {
                if ui.button(name).clicked() {
                    viewer.look_from(yaw, pitch);
                    ui.close();
                }
            }
            ui.separator();
            ui.checkbox(&mut viewer.turntable, "Turntable")
                .on_hover_text("Turn the view around the model (a drag in the view stops it)");
            if viewer.has_enclosure() {
                ui.separator();
                let mut shown = viewer.shows_enclosure();
                let toggle = ui.checkbox(&mut shown, "Enclosure");
                if toggle.on_hover_text("Show the enclosure around the parts, see-through").changed() {
                    viewer.set_show_enclosure(shown);
                }
            }
        })
        .response
        .on_hover_text("Look from the front, the top, a side…");
        separator(ui);
        if toolbar::icon_button(ui, Icon::Grid, viewer.show_grid, "Show the XY grid").clicked() {
            viewer.show_grid = !viewer.show_grid;
        }
        if toolbar::icon_button(ui, Icon::Axes, viewer.show_axes, "Show the axes").clicked() {
            viewer.show_axes = !viewer.show_axes;
        }
        let layers = "Cut the model at the top of a layer, picked with the slider on the right";
        if toolbar::icon_button(ui, Icon::Layers, viewer.layer_view, layers).clicked() {
            viewer.layer_view = !viewer.layer_view;
        }
        if viewer.has_pauses() {
            let pauses = "Show the heights where the print pauses";
            if toolbar::icon_button(ui, Icon::Pauses, viewer.show_pauses, pauses).clicked() {
                viewer.show_pauses = !viewer.show_pauses;
            }
        }
        separator(ui);
        toolbar::icon_button(ui, Icon::Help, false, MOUSE);
    }
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 20.0), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, theme::palette().border));
}
