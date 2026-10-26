//! The editor column's modes: the code, or the conversation with the
//! agent, two ways of editing the model in the same place. A third, a
//! manual mode, would be one more variant and one more segment.
//!
//! The mode is the window's, not each tab's: the conversation follows the
//! active tab as the code does, and switching tabs should not change what
//! the column shows under the user's hands, as the arrangement does not.
//! It comes back at the next launch with the open tabs.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Sense, Stroke, Vec2};

use super::Arrangement;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme;

/// What the editor column shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorMode {
    #[default]
    Code,
    Agent,
}

impl EditorMode {
    pub const ALL: [EditorMode; 2] = [EditorMode::Code, EditorMode::Agent];

    /// As the session file and a screenshot run name it.
    pub fn id(self) -> &'static str {
        match self {
            EditorMode::Code => "code",
            EditorMode::Agent => "agent",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        EditorMode::ALL.into_iter().find(|mode| mode.id() == id)
    }

    fn label(self) -> &'static str {
        match self {
            EditorMode::Code => tr("Code", "Code"),
            EditorMode::Agent => tr("Agent", "Agent"),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            EditorMode::Code => tr("Edit the model by its code", "Modifier le modèle par son code"),
            EditorMode::Agent => tr("Edit the model by talking to the agent", "Modifier le modèle en parlant à l'agent"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            EditorMode::Code => Icon::CodeOnly,
            EditorMode::Agent => Icon::Agent,
        }
    }
}

/// The mode the Agent button and its shortcut switch to: the agent, or
/// back to the code when the agent is already on screen.
pub fn toggled(mode: EditorMode, arrangement: Arrangement) -> EditorMode {
    if mode == EditorMode::Agent && arrangement != Arrangement::View { EditorMode::Code } else { EditorMode::Agent }
}

/// The arrangement that shows the editor column: the view alone gives way
/// to the two side by side.
pub fn showing_column(arrangement: Arrangement) -> Arrangement {
    if arrangement == Arrangement::View { Arrangement::Split } else { arrangement }
}

/// Whether the agent's output waits unread: once a turn ends while the
/// agent is not on screen, until it is.
pub fn unread(unread: bool, was_running: bool, running: bool, agent_shown: bool) -> bool {
    !agent_shown && (unread || (was_running && !running))
}

/// What the Agent segment shows beside its name while the code is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    None,
    /// A turn is under way.
    Working,
    /// A turn ended that the user has not seen.
    Unread,
}

pub fn badge(mode: EditorMode, running: bool, unread: bool) -> Badge {
    match (mode, running, unread) {
        (EditorMode::Agent, _, _) => Badge::None,
        (_, true, _) => Badge::Working,
        (_, false, true) => Badge::Unread,
        _ => Badge::None,
    }
}

/// The modes side by side, the one shown pressed and the agent's with its
/// badge; the one clicked, if another.
pub fn switch(ui: &mut egui::Ui, current: EditorMode, badge: Badge) -> Option<EditorMode> {
    let palette = theme::palette();
    let mut picked = None;
    Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for mode in EditorMode::ALL {
                    let on = mode == current;
                    let font = egui::FontId::proportional(12.0);
                    let galley = ui.painter().layout_no_wrap(mode.label().to_string(), font, Color32::PLACEHOLDER);
                    let marked = mode == EditorMode::Agent && badge != Badge::None;
                    let size = Vec2::new(galley.size().x + if marked { 46.0 } else { 34.0 }, 22.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    let fill = if on {
                        palette.accent_muted
                    } else if response.hovered() {
                        palette.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
                    let color = if on {
                        icons::on_muted()
                    } else if response.hovered() {
                        palette.text
                    } else {
                        palette.text_dim
                    };
                    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), Vec2::splat(14.0));
                    icons::paint(ui.painter(), icon, mode.icon(), color);
                    let text = egui::pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0);
                    let text_right = text.x + galley.size().x;
                    ui.painter().galley(text, galley, color);
                    let mark = egui::Rect::from_center_size(egui::pos2(text_right + 9.0, rect.center().y), Vec2::splat(10.0));
                    let hint = match badge {
                        Badge::Working if marked => {
                            egui::Spinner::new().size(10.0).color(palette.accent_strong).paint_at(ui, mark);
                            ui.ctx().request_repaint();
                            tr("The agent is working", "L'agent travaille")
                        }
                        Badge::Unread if marked => {
                            ui.painter().circle_filled(mark.center(), 3.5, palette.accent_strong);
                            tr("The agent answered", "L'agent a répondu")
                        }
                        _ => mode.hint(),
                    };
                    if response.on_hover_text(hint).clicked() && !on {
                        picked = Some(mode);
                    }
                }
            });
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_read_back_from_their_ids() {
        for mode in EditorMode::ALL {
            assert_eq!(EditorMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(EditorMode::from_id("manual"), None);
        assert_eq!(EditorMode::default(), EditorMode::Code);
    }

    #[test]
    fn the_agent_button_switches_to_the_agent_and_back() {
        use Arrangement::{Code, Split, View};
        assert_eq!(toggled(EditorMode::Code, Split), EditorMode::Agent);
        assert_eq!(toggled(EditorMode::Agent, Split), EditorMode::Code);
        assert_eq!(toggled(EditorMode::Agent, Code), EditorMode::Code);
        // The agent not on screen: the button shows it rather than the code.
        assert_eq!(toggled(EditorMode::Agent, View), EditorMode::Agent);
        assert_eq!(toggled(EditorMode::Code, View), EditorMode::Agent);
        // The view alone makes room for the column; the others keep theirs.
        assert_eq!(showing_column(View), Split);
        assert_eq!(showing_column(Split), Split);
        assert_eq!(showing_column(Code), Code);
    }

    #[test]
    fn a_turn_ended_out_of_sight_stays_unread_until_the_agent_shows() {
        // Ends while the code is shown: unread.
        let after = unread(false, true, false, false);
        assert!(after);
        // Stays so frame after frame, and through the next turn.
        assert!(unread(after, false, false, false));
        assert!(unread(after, false, true, false));
        // Read once the agent shows.
        assert!(!unread(after, false, false, true));
        // A turn ended on screen was seen; one under way is not unread yet.
        assert!(!unread(false, true, false, true));
        assert!(!unread(false, false, true, false));
    }

    #[test]
    fn the_agent_segment_shows_the_turn_and_what_is_unread_beside_the_code() {
        assert_eq!(badge(EditorMode::Code, true, false), Badge::Working);
        assert_eq!(badge(EditorMode::Code, true, true), Badge::Working);
        assert_eq!(badge(EditorMode::Code, false, true), Badge::Unread);
        assert_eq!(badge(EditorMode::Code, false, false), Badge::None);
        assert_eq!(badge(EditorMode::Agent, true, true), Badge::None);
    }
}
