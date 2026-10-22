//! The properties panel (C2F-38): the arguments of the selected call, to
//! change with the mouse, written back into the code.

use eframe::egui;

use super::StudioApp;

impl StudioApp {
    /// A panel on the right with the selected call's arguments.
    pub(super) fn properties_panel(&mut self, _ctx: &egui::Context) {}
}
