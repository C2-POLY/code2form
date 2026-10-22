//! The move, turn and scale handles (C2F-39), drawn on the selected object
//! in the 3D view, written back into the code as `translate`, `rotate` and
//! `scale`.

use eframe::egui;

use super::StudioApp;

impl StudioApp {
    /// Draws the handles on the selection and follows their drags.
    pub(super) fn handles_view(&mut self, _ctx: &egui::Context, _ui: &mut egui::Ui, _response: &egui::Response) {}
}
