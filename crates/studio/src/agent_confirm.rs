//! Asking the user before the agent changes the open document, when the
//! settings say "Ask before each change": a small dialog with the text
//! before and after, for the changes the review of an edit
//! (`app/review.rs`) does not show: parameters set, a circuit's parts
//! placed and wired.

use eframe::egui::{self, Align, Button, Id, Layout, RichText};

use crate::i18n::tr;
use crate::theme;

/// A change the agent proposes.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// What it changes, in a line: "Edit lines 3–5 of box.c2f".
    pub title: String,
    /// The text before and after.
    pub before: String,
    pub after: String,
}

/// Shows `proposal` and asks the user: `Some(true)` once accepted,
/// `Some(false)` once declined, `None` while they have not answered.
pub fn confirm_agent_change(ctx: &egui::Context, proposal: &Proposal) -> Option<bool> {
    let palette = theme::palette();
    let mut answer = None;
    let modal = egui::Modal::new(Id::new("agent-confirm")).show(ctx, |ui| {
        ui.set_max_width(560.0);
        ui.label(RichText::new(tr("The agent proposes a change", "L'agent propose une modification")).heading().color(palette.text));
        ui.label(RichText::new(&proposal.title).color(palette.text_dim));
        ui.add_space(8.0);
        let side = |ui: &mut egui::Ui, title: &str, text: &str, color| {
            ui.label(RichText::new(title).strong().color(palette.text));
            egui::ScrollArea::vertical().id_salt(title).max_height(160.0).show(ui, |ui| {
                ui.label(RichText::new(text).monospace().color(color));
            });
        };
        side(ui, tr("Before", "Avant"), &proposal.before, palette.error);
        ui.add_space(6.0);
        side(ui, tr("After", "Après"), &proposal.after, palette.text);
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let apply = Button::new(RichText::new(tr("Apply", "Appliquer")).color(palette.on_accent).strong()).fill(palette.accent);
            if ui.add(apply).clicked() {
                answer = Some(true);
            }
            if ui.button(tr("Decline", "Refuser")).clicked() {
                answer = Some(false);
            }
        });
    });
    if answer.is_none() && modal.should_close() {
        answer = Some(false);
    }
    answer
}
