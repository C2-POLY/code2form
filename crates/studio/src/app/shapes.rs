//! Making a model without code (C2F-40): shapes put down from a toolbar,
//! booleans of the selection, holes, copies and deletions.

use eframe::egui;

use super::StudioApp;

impl StudioApp {
    /// The shapes toolbar over the 3D view, and its keys.
    pub(super) fn shapes_view(&mut self, _ctx: &egui::Context, _ui: &mut egui::Ui, _response: &egui::Response) {}
}
