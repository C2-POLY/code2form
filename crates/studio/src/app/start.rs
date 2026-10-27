//! The start screen of a new file: a card over the view that offers to
//! open a file, to start in code, or to start with the agent from an idea,
//! the agent then planning the object with the user.
//!
//! It shows on a tab made by File › New or the tabs' "+", and at a first
//! launch with nothing to restore; never on a tab restored or a file
//! opened. It goes as soon as the tab is typed in, a file opens in it,
//! or the agent is asked something; Escape starts in code.

use eframe::egui::{self, Align2, Button, Id, Key, Margin, RichText, Sense, Stroke, Vec2};

use super::{Arrangement, StudioApp};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme;

/// The id of the field the idea is typed in.
fn idea_id() -> Id {
    Id::new("start-idea")
}

/// What the user picked on the start screen.
enum Choice {
    Open,
    Recent(std::path::PathBuf),
    Code,
    Agent,
}

impl StudioApp {
    /// Whether the active tab shows its start screen: a new tab, still
    /// blank, nothing asked of the agent yet.
    pub(super) fn start_shown(&self) -> bool {
        self.doc.start.is_some() && self.doc.is_blank() && !self.agent.running()
    }

    /// Starts in code: the code beside the view, ready to type in.
    pub(super) fn start_in_code(&mut self, ctx: &egui::Context) {
        self.doc.start = None;
        if self.arrangement == Arrangement::View {
            self.arrangement = Arrangement::Split;
        }
        self.doc.editor.focus(ctx);
    }

    /// Starts with the agent: the chat opens over the view alone and the
    /// plan prompt goes with `idea`, the plan written in the tab's folder.
    pub(super) fn start_with_agent(&mut self, ctx: &egui::Context, idea: &str) {
        if idea.trim().is_empty() {
            return;
        }
        self.doc.start = None;
        // The folder the plan goes in, before the next frame sets it.
        self.agent.set_tab(self.doc.path.as_deref(), Some(self.agent_folder()));
        self.agent.start_plan(idea);
        self.open_chat(ctx);
    }

    /// The start screen over the view (or the window, the code shown
    /// alone), when the active tab shows it.
    pub(super) fn start_screen(&mut self, ctx: &egui::Context) {
        if self.doc.start.is_some() && !self.start_shown() {
            self.doc.start = None;
        }
        if !self.start_shown() {
            return;
        }
        if self.confirm.is_none() && !self.settings_open && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.start_in_code(ctx);
            return;
        }
        let rect = if self.arrangement == Arrangement::Code || !self.view_rect.is_positive() { ctx.screen_rect() } else { self.view_rect };
        let mut idea = self.doc.start.take().unwrap_or_default();
        let mut choice = None;
        let palette = theme::palette();
        let width = (rect.width() - 48.0).clamp(280.0, 460.0);
        egui::Area::new(Id::new(("start", self.doc.id)))
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(rect.center())
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                theme::card_frame().inner_margin(Margin::same(20)).show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label(RichText::new(tr("New file", "Nouveau fichier")).size(18.0).strong().color(palette.text_strong));
                    ui.label(RichText::new(tr("How do you want to start?", "Comment voulez-vous commencer ?")).size(13.0).color(palette.text_dim));
                    ui.add_space(4.0);
                    if option(ui, Icon::Folder, tr("Open a file…", "Ouvrir un fichier…"), tr("A design, a circuit or a mesh", "Un design, un circuit ou un maillage")).clicked() {
                        choice = Some(Choice::Open);
                    }
                    let recent: Vec<_> = self.recent.paths().iter().take(4).cloned().collect();
                    if !recent.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            ui.add_space(38.0);
                            for path in recent {
                                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                                let link = ui.add(egui::Link::new(RichText::new(name).size(12.0)));
                                if link.on_hover_text(path.display().to_string()).clicked() {
                                    choice = Some(Choice::Recent(path));
                                }
                            }
                        });
                    }
                    if option(ui, Icon::CodeOnly, tr("Start in code", "Commencer en code"), tr("An empty file, the code beside the view", "Un fichier vide, le code à côté de la vue")).clicked() {
                        choice = Some(Choice::Code);
                    }
                    let agent = option(
                        ui,
                        Icon::Agent,
                        tr("Start with the agent", "Commencer avec l'agent"),
                        tr(
                            "It plans the object with you: its questions, a review of the mechanics, then the brief",
                            "Il planifie l'objet avec vous : ses questions, une revue mécanique, puis le cahier des charges",
                        ),
                    );
                    if agent.clicked() {
                        ui.memory_mut(|m| m.request_focus(idea_id()));
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(38.0);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let field = egui::TextEdit::singleline(&mut idea)
                            .id(idea_id())
                            .hint_text(tr("Describe your idea", "Décrivez votre idée"))
                            .frame(false)
                            .desired_width((ui.available_width() - 104.0).max(120.0));
                        let response = crate::agent_panel::field_frame(ui, |ui| ui.add(field));
                        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        let send = Button::new(RichText::new(tr("Send", "Envoyer")).color(palette.on_accent).strong())
                            .fill(palette.accent)
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::new(0.0, 24.0));
                        let clicked = ui.add_enabled(!idea.trim().is_empty(), send).clicked();
                        if (enter || clicked) && !idea.trim().is_empty() {
                            choice = Some(Choice::Agent);
                        }
                    });
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("Or start from", "Ou partir des")).size(12.0).color(palette.text_faint));
                        self.examples_menu(ui);
                    });
                });
            });
        // An example opened takes the tab: the screen is gone.
        if self.doc.is_blank() {
            self.doc.start = Some(idea.clone());
        }
        match choice {
            Some(Choice::Open) => self.open_dialog(ctx),
            Some(Choice::Recent(path)) => self.open_recent(ctx, path),
            Some(Choice::Code) => self.start_in_code(ctx),
            Some(Choice::Agent) => self.start_with_agent(ctx, &idea),
            None => {}
        }
    }
}

/// A choice of the start screen: its icon, title and what it does, as a
/// row that lights up under the pointer.
fn option(ui: &mut egui::Ui, icon: Icon, title: &str, detail: &str) -> egui::Response {
    let palette = theme::palette();
    let width = ui.available_width();
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), egui::FontId::proportional(14.0), palette.text_strong);
    let detail_galley = ui.painter().layout(detail.to_string(), egui::FontId::proportional(12.0), palette.text_dim, width - 50.0);
    let height = title_galley.size().y + detail_galley.size().y + 16.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let fill = if hovered { palette.hover } else { palette.raised };
    let stroke = Stroke::new(1.0, if hovered { palette.border_strong } else { palette.border });
    ui.painter().rect(rect, egui::CornerRadius::same(8), fill, stroke, egui::StrokeKind::Inside);
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.top() + 8.0 + title_galley.size().y / 2.0), Vec2::splat(18.0));
    icons::paint(ui.painter(), icon_rect, icon, if hovered { palette.accent_strong } else { palette.text });
    let title_pos = egui::pos2(rect.left() + 38.0, rect.top() + 8.0);
    let detail_pos = title_pos + Vec2::new(0.0, title_galley.size().y + 2.0);
    ui.painter().galley(title_pos, title_galley, palette.text_strong);
    ui.painter().galley(detail_pos, detail_galley, palette.text_dim);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
