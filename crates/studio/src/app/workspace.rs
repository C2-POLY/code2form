//! The two workspaces of a design. Model: the tree on the left, the
//! modelling tools above the view and the inspector on the right, to make
//! an object with the mouse. Code: the editor on the left, with the tree a
//! tab away, and the file's parameters on the right. A circuit has its own
//! layout, always with its code.

use std::ops::Range;

use eframe::egui::{self, Button, CornerRadius, Frame, Label, Margin, RichText, Sense, Stroke, Vec2};

use super::StudioApp;
use crate::settings::Workspace;
use crate::shortcuts::{self, Action};
use crate::theme;

/// What the left panel of the Code workspace shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CodeTab {
    #[default]
    Code,
    Tree,
}

/// What the inspector, the right panel of the Model workspace, shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum InspectorTab {
    #[default]
    Properties,
    Parameters,
}

/// The tabs picked in the panels, for the whole window.
#[derive(Debug, Default)]
pub(super) struct Panels {
    pub code_tab: CodeTab,
    pub inspector: InspectorTab,
    /// The selection the inspector last saw, by statement: a new one
    /// brings its Properties tab forward, the same one after a render
    /// leaves the tab picked.
    seen: Vec<Option<Range<usize>>>,
    /// A widget that asked for the keyboard before it showed: it gets it
    /// at the next frame.
    refocus: Option<egui::Id>,
    /// The widget last given the keyboard back: if it still does not show,
    /// it is not given it again.
    refocused: Option<egui::Id>,
}

impl StudioApp {

    /// The Model workspace for a design.
    pub(super) fn models(&self) -> bool {
        !self.doc.is_circuit() && self.settings.workspace == Workspace::Model
    }

    /// The code editor is on screen: a circuit's, or the Code workspace's
    /// Code tab.
    pub(super) fn editor_shown(&self) -> bool {
        self.arrangement() != super::Arrangement::View
            && (self.doc.is_circuit() || (self.settings.workspace == Workspace::Code && self.panels.code_tab == CodeTab::Code))
    }

    /// Gives the editor the keyboard, when it is on screen: a widget out of
    /// sight with the keyboard leaves screen readers a focus on nothing.
    pub(super) fn focus_editor(&self, ctx: &egui::Context) {
        if self.editor_shown() {
            self.doc.editor.focus(ctx);
        }
    }

    /// The code at byte `offset`, in the Code workspace's editor, with the
    /// keyboard.
    pub(super) fn go_to_code(&mut self, ctx: &egui::Context, offset: usize) {
        if !self.doc.is_circuit() {
            self.set_workspace(ctx, Workspace::Code);
            self.panels.code_tab = CodeTab::Code;
        }
        if self.arrangement == super::Arrangement::View {
            self.arrangement = super::Arrangement::Split;
        }
        let text = self.doc.text.clone();
        self.doc.editor.go_to(ctx, &text, offset);
        self.doc.editor.focus(ctx);
    }

    /// Takes the keyboard, at the end of a frame, from a widget that is not
    /// on screen: egui would tell screen readers of a focus on nothing,
    /// which the macOS one does not survive. A widget that asked for it
    /// before it shows (the editor, as the Code workspace opens) gets it at
    /// the next frame, once.
    pub(super) fn drop_unseen_focus(&mut self, ctx: &egui::Context) {
        let Some(id) = ctx.memory(|m| m.focused()) else { return };
        if ctx.viewport(|v| v.this_pass.widgets.get(id).is_some()) {
            self.panels.refocused = None;
            return;
        }
        ctx.memory_mut(|m| m.surrender_focus(id));
        if self.panels.refocused != Some(id) {
            self.panels.refocus = Some(id);
        }
    }

    /// Gives the keyboard back, at the start of a frame, to the widget that
    /// asked for it before it showed.
    pub(super) fn refocus(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.panels.refocus.take() {
            ctx.memory_mut(|m| m.request_focus(id));
            self.panels.refocused = Some(id);
        }
    }

    pub(super) fn set_workspace(&mut self, ctx: &egui::Context, workspace: Workspace) {
        if workspace == self.settings.workspace {
            return;
        }
        self.settings.workspace = workspace;
        if workspace == Workspace::Model {
            // The keys typed next are not for a hidden editor.
            ctx.memory_mut(|m| m.stop_text_input());
        }
    }

    /// The toolbar's switch between the workspaces.
    pub(super) fn workspace_switch(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let keys = shortcuts::describe(ctx, &self.settings.shortcuts.get(Action::SwitchWorkspace));
        let palette = theme::palette();
        let frame = Frame::new()
            .fill(palette.editor)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::same(2));
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for workspace in Workspace::ALL {
                let on = self.settings.workspace == workspace;
                let text = RichText::new(workspace.name()).size(12.5);
                let (text, fill) =
                    if on { (text.strong().color(palette.on_accent), palette.accent) } else { (text, egui::Color32::TRANSPARENT) };
                let button = Button::new(text)
                    .fill(fill)
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(5))
                    .min_size(Vec2::new(58.0, 22.0));
                let hint = match workspace {
                    Workspace::Model => "Make the object with the mouse: the tree, the modelling tools and its properties",
                    Workspace::Code => "Write the object's code, with the file's parameters beside it",
                };
                if ui.add(button).on_hover_text(format!("{hint} ({keys})")).clicked() {
                    self.set_workspace(ctx, workspace);
                }
            }
        });
    }

    /// Brings the Properties tab forward when something new is selected.
    pub(super) fn follow_selection(&mut self) {
        let selection: Vec<_> = self.doc.viewer.selection().iter().map(|s| s.range()).collect();
        if !selection.is_empty() && selection != self.panels.seen {
            self.panels.inspector = InspectorTab::Properties;
            self.panels.seen = selection;
        }
    }
}

/// A row of tabs over a panel, the one picked underlined. Returns whether
/// the pick changed.
pub(super) fn tab_strip<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, tabs: &[(T, &str)]) -> bool {
    let palette = theme::palette();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for &(tab, label) in tabs {
            let on = *current == tab;
            let text = RichText::new(label).size(12.5);
            let text = if on { text.strong().color(palette.text) } else { text.color(palette.text_dim) };
            let response = ui.add(Label::new(text).selectable(false).sense(Sense::click()));
            // A tab, picked or not, for screen readers.
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, label));
            if on {
                let y = response.rect.bottom() + 5.0;
                ui.painter().hline(response.rect.x_range(), y, Stroke::new(2.0, palette.accent));
            }
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                *current = tab;
                changed = true;
            }
        }
    });
    ui.add_space(6.0);
    let rect = ui.max_rect();
    let y = ui.cursor().top();
    ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, palette.border));
    ui.add_space(8.0);
    changed
}
