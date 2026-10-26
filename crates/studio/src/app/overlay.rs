//! What floats over the 3D view besides its own tools: the conversation
//! with the agent, a card at the view's bottom left. A Manual mode, its
//! tools floating over the view as well, would be one more [`Floating`].
//!
//! The state is the window's, not each tab's: the conversation follows
//! the active tab, and switching tabs should not open or close what
//! floats under the user's hands. Whether the chat is open comes back at
//! the next launch with the open tabs.
//!
//! Opening the chat shows the view alone, the arrangement before it kept
//! and brought back when the chat closes; the user may still pick another
//! arrangement meanwhile, the chat then floating over the view's part.

use super::Arrangement;

/// The room kept between a floating card and the view's edges.
pub const MARGIN: f32 = 16.0;

/// The room the chat takes along the view's bottom, folded to its field.
pub const FOLDED_HEIGHT: f32 = 64.0;

/// What floats over the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Floating {
    /// The conversation with the agent.
    Agent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    shown: Option<Floating>,
    /// Folded to its message field.
    pub minimised: bool,
    /// The arrangement the chat replaced with the view alone.
    previous: Option<Arrangement>,
    /// Whether the user unfolded (or folded) the Parameters card while the
    /// chat is open: it starts folded then, the agent setting the values.
    pub parameters: Option<bool>,
}

impl Overlay {
    /// Whether the chat is open, minimised or not.
    pub fn chat_open(&self) -> bool {
        self.shown == Some(Floating::Agent)
    }

    /// Whether the conversation shows: open, not minimised, over a view
    /// that is on screen.
    pub fn chat_on_screen(&self, arrangement: Arrangement) -> bool {
        self.chat_open() && !self.minimised && arrangement != Arrangement::Code
    }

    /// Opens the chat, unfolded; returns the arrangement to show: the view
    /// alone, the one before kept to come back to. Open already, it stays
    /// as the user has it.
    pub fn open_chat(&mut self, arrangement: Arrangement) -> Arrangement {
        self.minimised = false;
        if self.chat_open() {
            return arrangement;
        }
        self.shown = Some(Floating::Agent);
        self.previous = Some(arrangement);
        self.parameters = None;
        Arrangement::View
    }

    /// Closes the chat; returns the arrangement to show: the one before it
    /// opened, unless the user has picked another than the view alone
    /// since.
    pub fn close_chat(&mut self, arrangement: Arrangement) -> Arrangement {
        if !self.chat_open() {
            return arrangement;
        }
        self.shown = None;
        self.minimised = false;
        self.parameters = None;
        let previous = self.previous.take();
        if arrangement == Arrangement::View { previous.unwrap_or(arrangement) } else { arrangement }
    }

    /// The Agent button and its shortcut: opens the chat, unfolds it when
    /// minimised or out of sight, or closes it.
    pub fn toggle_chat(&mut self, arrangement: Arrangement) -> Arrangement {
        if self.chat_on_screen(arrangement) {
            self.close_chat(arrangement)
        } else if self.chat_open() {
            self.minimised = false;
            if arrangement == Arrangement::Code { Arrangement::Split } else { arrangement }
        } else {
            self.open_chat(arrangement)
        }
    }
}

/// The width of the chat's card over a view `view_width` wide: two
/// fifths of it, from 360 to 520 points, and never wider than the view
/// less its margins.
pub fn card_width(view_width: f32) -> f32 {
    (view_width * 0.4).clamp(360.0, 520.0).min((view_width - 2.0 * MARGIN).max(0.0))
}

/// The tallest the card grows, in a view `view_height` high whose own
/// tools take `top` and its bottom bars `bottom`: all of it but those and
/// a margin. Beyond, the conversation scrolls.
pub fn height_cap(view_height: f32, top: f32, bottom: f32) -> f32 {
    (view_height - top - bottom - MARGIN).max(120.0)
}

/// The most the conversation takes in the card, its messages as tall as
/// they are up to it, then scrolling: what is left of `cap` once the
/// card's header and message field (`chrome`) are drawn.
pub fn conversation_room(chrome: f32, cap: f32) -> f32 {
    (cap - chrome).max(80.0)
}

/// The room the card takes on the view's left, for the model to be framed
/// beside it.
pub fn left_inset(open: bool, view_width: f32) -> f32 {
    if open { card_width(view_width) + MARGIN } else { 0.0 }
}

/// Whether the agent's output waits unread: once a turn ends while the
/// conversation is not on screen, until it is.
pub fn unread(unread: bool, was_running: bool, running: bool, on_screen: bool) -> bool {
    !on_screen && (unread || (was_running && !running))
}

/// What the top bar's Agent button shows over its icon while the
/// conversation is out of sight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    None,
    /// A turn is under way.
    Working,
    /// A turn ended that the user has not seen.
    Unread,
}

pub fn badge(running: bool, unread: bool, on_screen: bool) -> Badge {
    match (on_screen, running, unread) {
        (true, _, _) => Badge::None,
        (_, true, _) => Badge::Working,
        (_, false, true) => Badge::Unread,
        _ => Badge::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Arrangement::{Code, Split, View};

    #[test]
    fn opening_the_chat_shows_the_view_alone_and_closing_it_brings_the_layout_back() {
        let mut overlay = Overlay::default();
        assert!(!overlay.chat_open());
        assert_eq!(overlay.open_chat(Split), View);
        assert!(overlay.chat_on_screen(View));
        assert_eq!(overlay.close_chat(View), Split);
        assert!(!overlay.chat_open());
        // From the editor alone, and back to it.
        assert_eq!(overlay.open_chat(Code), View);
        assert_eq!(overlay.close_chat(View), Code);
        // The user picked side by side meanwhile: it stays.
        overlay.open_chat(Code);
        assert_eq!(overlay.close_chat(Split), Split);
        // Closed already: nothing changes.
        assert_eq!(overlay.close_chat(View), View);
        // Open already: asked again, it unfolds and keeps the layout.
        overlay.open_chat(Split);
        overlay.minimised = true;
        assert_eq!(overlay.open_chat(Split), Split);
        assert!(!overlay.minimised);
    }

    #[test]
    fn the_agent_button_opens_unfolds_and_closes_the_chat() {
        let mut overlay = Overlay::default();
        assert_eq!(overlay.toggle_chat(Split), View);
        assert_eq!(overlay.toggle_chat(View), Split, "closed");
        overlay.toggle_chat(Split);
        overlay.minimised = true;
        assert!(!overlay.chat_on_screen(View));
        assert_eq!(overlay.toggle_chat(View), View);
        assert!(overlay.chat_on_screen(View), "unfolded");
        // Out of sight behind the editor alone: the view comes back beside it.
        assert_eq!(overlay.toggle_chat(Code), Split);
        assert!(overlay.chat_on_screen(Split));
    }

    #[test]
    fn the_parameters_choice_lasts_while_the_chat_is_open() {
        let mut overlay = Overlay::default();
        overlay.open_chat(Split);
        overlay.parameters = Some(true);
        overlay.open_chat(View);
        assert_eq!(overlay.parameters, Some(true), "still open");
        overlay.close_chat(View);
        assert_eq!(overlay.parameters, None);
    }

    #[test]
    fn the_card_is_two_fifths_of_the_view_and_grows_to_its_height() {
        assert_eq!(card_width(1000.0), 400.0);
        assert_eq!(card_width(600.0), 360.0);
        assert_eq!(card_width(2000.0), 520.0);
        assert_eq!(card_width(300.0), 268.0, "never wider than the view");
        assert_eq!(left_inset(true, 1000.0), 416.0);
        assert_eq!(left_inset(false, 1000.0), 0.0);
        let cap = height_cap(800.0, 48.0, 0.0);
        assert_eq!(cap, 736.0);
        // Empty: compact. Short: as tall as its messages; long: up to the
        // cap, then it scrolls.
        let card = |content: f32| 150.0 + content.min(conversation_room(150.0, cap));
        assert_eq!(card(0.0), 150.0);
        assert_eq!(card(100.0), 250.0);
        assert_eq!(card(2000.0), cap);
        assert_eq!(conversation_room(700.0, 600.0), 80.0, "always room for a message");
        assert_eq!(height_cap(100.0, 48.0, 58.0), 120.0, "a floor in a tiny view");
    }

    #[test]
    fn a_turn_ended_out_of_sight_stays_unread_until_the_chat_shows() {
        let after = unread(false, true, false, false);
        assert!(after);
        assert!(unread(after, false, false, false));
        assert!(unread(after, false, true, false));
        assert!(!unread(after, false, false, true));
        assert!(!unread(false, true, false, true));
        assert!(!unread(false, false, true, false));
    }

    #[test]
    fn the_agent_button_shows_the_turn_and_what_is_unread_while_out_of_sight() {
        assert_eq!(badge(true, false, false), Badge::Working);
        assert_eq!(badge(true, true, false), Badge::Working);
        assert_eq!(badge(false, true, false), Badge::Unread);
        assert_eq!(badge(false, false, false), Badge::None);
        assert_eq!(badge(true, true, true), Badge::None);
    }
}
