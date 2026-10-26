//! The agent's edits as the user sees them land: the lines each changed,
//! tinted in the editor for a few seconds and marked in its margin until
//! the user's next edit; the part they touched, flashed in the view; and
//! the edits of one answer of the agent's, grouped to undo together.
//!
//! The lifecycles are plain data apart from egui ([`Highlight`],
//! [`AgentGroup`], [`Flash`]); the app keeps one of each in every tab.

use std::ops::Range;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use glam::Vec3;
use serde_json::{Value, json};

use super::live::{self, part_calls};
use super::review;
use super::StudioApp;
use crate::console::Level;
use crate::i18n::tr;
use crate::theme;

/// How long the lines of an edit stay fully tinted, then how long the tint
/// takes to fade out.
const HOLD: Duration = Duration::from_millis(1000);
const FADE: Duration = Duration::from_millis(2000);
/// How long the touched part flashes in the view, in two pulses.
const FLASH: Duration = Duration::from_millis(1600);
/// The pause that ends a group of edits of an agent outside the studio.
const IDLE: Duration = Duration::from_secs(10);

/// The lines the agent changed in a tab's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    /// The text as the agent's last edit left it: the marks show while
    /// the text is still this, and go at the user's next edit.
    text: String,
    /// The lines (from 0, the end excluded) the last edit changed: tinted,
    /// then faded.
    pub lines: Vec<Range<usize>>,
    /// The lines every edit of the agent's changed since the user last
    /// edited: marked in the margin.
    pub markers: Vec<Range<usize>>,
    since: Instant,
    /// The editor has drawn it: the tint started then, not when the edit
    /// landed out of sight, the agent shown in the editor's place.
    seen: bool,
}

impl Highlight {
    /// The marks of an edit from `before` to `after` that changed `lines`
    /// of `after`, made at `now`; the marks of the agent's edits before it,
    /// in `previous`, carry on where their lines went when the user did not
    /// edit in between.
    pub fn new(previous: Option<&Highlight>, before: &str, after: &str, lines: Vec<Range<usize>>, now: Instant) -> Self {
        let mut marked: Vec<usize> = lines.iter().flat_map(Clone::clone).collect();
        if let Some(previous) = previous.filter(|p| p.text == before) {
            let map = review::line_map(before, after);
            marked.extend(previous.markers.iter().flat_map(Clone::clone).filter_map(|line| map.get(line).copied().flatten()));
        }
        Highlight { text: after.to_string(), markers: ranges(marked), lines, since: now, seen: false }
    }

    /// The editor draws it at `now`: the first time, the tint starts over.
    pub fn see(&mut self, now: Instant) {
        if !self.seen {
            self.seen = true;
            self.since = now;
        }
    }

    /// Whether the marks still show on `text`: the user has not edited it.
    pub fn shows(&self, text: &str) -> bool {
        self.text == text
    }

    /// How strong the tint of the last edit's lines is at `now`, from 1
    /// to 0 once it has faded.
    pub fn tint(&self, now: Instant) -> f32 {
        let age = now.saturating_duration_since(self.since);
        if age <= HOLD {
            return 1.0;
        }
        1.0 - ((age - HOLD).as_secs_f32() / FADE.as_secs_f32()).min(1.0)
    }
}

/// Line numbers as sorted ranges, those that touch merged.
fn ranges(mut lines: Vec<usize>) -> Vec<Range<usize>> {
    lines.sort_unstable();
    lines.dedup();
    let mut out: Vec<Range<usize>> = Vec::new();
    for line in lines {
        match out.last_mut() {
            Some(last) if last.end == line => last.end += 1,
            _ => out.push(line..line + 1),
        }
    }
    out
}

/// The edits the agent made to a tab during one of its answers, to undo
/// together.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentGroup {
    /// The text before the first edit, and as the last left it.
    pub text_before: String,
    pub text_after: String,
    /// The tab's id.
    pub tab: u64,
    pub started: Instant,
    /// When its last edit landed.
    pub last: Instant,
    /// The answer goes on: its next edits join the group.
    pub open: bool,
    pub edits: usize,
}

impl AgentGroup {
    /// `group` with the edit from `before` to `after` of tab `tab` in it:
    /// the same group while it is open and the user did not edit since its
    /// last edit, else a new one. Without a turn of the panel's under way
    /// (`turn`), the edits come from an agent outside the studio, whose
    /// answers are not seen: a pause of [`IDLE`] ends its group.
    pub fn record(group: Option<AgentGroup>, tab: u64, before: &str, after: &str, now: Instant, turn: bool) -> AgentGroup {
        match group {
            Some(mut group) if group.open && group.text_after == before && (turn || now.saturating_duration_since(group.last) < IDLE) => {
                group.text_after = after.to_string();
                group.edits += 1;
                group.last = now;
                group
            }
            _ => AgentGroup { text_before: before.to_string(), text_after: after.to_string(), tab, started: now, last: now, open: true, edits: 1 },
        }
    }

    /// The text with the group's edits undone, from `current`; refused when
    /// the text changed since the agent's last edit, which undoing would
    /// throw away.
    pub fn undo(&self, current: &str) -> Result<String, String> {
        if current != self.text_after {
            return Err(tr(
                "The code changed since the agent's change: undo step by step with Cmd+Z instead.",
                "Le code a changé depuis la modification de l'agent : annulez pas à pas avec Cmd+Z.",
            )
            .into());
        }
        Ok(self.text_before.clone())
    }
}

/// The box of a part the agent touched, flashed in the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flash {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    since: Instant,
}

impl Flash {
    pub fn new(lo: [f32; 3], hi: [f32; 3], now: Instant) -> Self {
        Flash { lo, hi, since: now }
    }

    /// How bright the flash is at `now`, from 0 to 1 and back twice; `None`
    /// once it is over.
    pub fn strength(&self, now: Instant) -> Option<f32> {
        let t = now.saturating_duration_since(self.since).as_secs_f32() / FLASH.as_secs_f32();
        (t < 1.0).then(|| (t * 2.0 * std::f32::consts::PI).sin().powi(2))
    }
}

/// Where char `cursor` of `old` is in `new`, `new` being `old` with one
/// stretch of it changed: before the change it stays, after it moves with
/// the text, and inside it goes to the change's end. The user's cursor
/// thus stays where they were typing when an edit of the agent's lands.
pub fn map_cursor(old: &str, new: &str, cursor: usize) -> usize {
    let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = old.bytes().rev().zip(new.bytes().rev()).take(room).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let at = old.char_indices().nth(cursor).map_or(old.len(), |(at, _)| at);
    if at <= prefix {
        cursor
    } else if at >= old.len() - suffix {
        new.chars().count() - old[at..].chars().count()
    } else {
        new[..new.len() - suffix].chars().count()
    }
}

/// The byte range of the statement that `part("…")` at byte `at` names:
/// up to its `;`, or the `}` closing its block.
fn part_extent(text: &str, at: usize) -> Range<usize> {
    let bytes = text.as_bytes();
    let (mut depth, mut i, mut opened) = (0usize, at, false);
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..].find("*/").map_or(bytes.len(), |end| i + 2 + end + 1);
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                opened = true;
            }
            b')' | b']' => depth = depth.saturating_sub(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && opened {
                    return at..i + 1;
                }
            }
            b';' if depth == 0 => return at..i + 1,
            _ => {}
        }
        i += 1;
    }
    at..text.len()
}

/// The innermost `part("name")` whose statement holds the first of the
/// changed `lines` (from 0, the end excluded) of `text`.
pub fn enclosing_part(text: &str, lines: &[Range<usize>]) -> Option<String> {
    let first = lines.first()?;
    let offset = |line: usize| if line == 0 { 0 } else { text.match_indices('\n').nth(line - 1).map_or(text.len(), |(at, _)| at + 1) };
    let changed = offset(first.start)..offset(first.end.max(first.start + 1)).max(offset(first.start) + 1);
    part_calls(text)
        .map(|(at, name)| (part_extent(text, at), name))
        .filter(|(extent, _)| extent.start < changed.end && changed.start < extent.end)
        .min_by_key(|(extent, _)| extent.len())
        .map(|(_, name)| name.to_string())
}

impl StudioApp {
    /// Puts the agent's edit of the active tab in place, `text` with its
    /// changed `lines`, as one step of the undo history: the user's cursor
    /// stays where they were typing; the lines are marked, the edit joins
    /// the agent's group, and the tab renders. The answer for the agent:
    /// the tab, the render to wait for, the lines and the part changed.
    pub(super) fn land_agent_edit(&mut self, ctx: &egui::Context, text: String, lines: Vec<Range<usize>>) -> Value {
        let now = Instant::now();
        let before = self.doc.text.clone();
        let typing = self.doc.editor.cursor_char().filter(|_| self.doc.editor.has_focus(ctx));
        let start = lines.first().map_or(0, |l| crate::agent_panel::line_offset(&text, l.start + 1));
        let cursor = match typing {
            Some(cursor) => map_cursor(&before, &text, cursor),
            None => text[..start].chars().count(),
        };
        self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
        if typing.is_none() {
            // The user is not typing: the editor shows the edited lines.
            self.doc.editor.go_to(ctx, &self.doc.text, start);
        }
        let described: Vec<String> =
            lines.iter().map(|l| if l.len() <= 1 { format!("{}", l.start + 1) } else { format!("{}–{}", l.start + 1, l.end) }).collect();
        self.doc.console.log(Level::Info, tr!("The agent edited line {}", "L'agent a modifié la ligne {}", described.join(", ")));
        let part = if self.doc.is_circuit() { None } else { enclosing_part(&self.doc.text, &lines) };
        self.doc.agent_highlight = Some(Highlight::new(self.doc.agent_highlight.as_ref(), &before, &self.doc.text, lines.clone(), now));
        let turn = self.agent_running;
        self.doc.agent_group = Some(AgentGroup::record(self.doc.agent_group.take(), self.doc.id, &before, &self.doc.text, now, turn));
        self.doc.agent_marks = lines;
        self.render_now();
        json!({
            "tab_id": self.doc.id,
            "generation": self.doc.sent,
            "changed_lines": live::marks_json(&self.doc.agent_marks),
            "revision": live::revision(&self.doc.text),
            "part": part,
            "text_before": before,
        })
    }

    /// Follows the agent's turns: one starting or ending closes the groups
    /// of edits, so that each answer's edits undo together.
    pub(super) fn follow_agent_turn(&mut self, ctx: &egui::Context, running: bool) {
        if running == self.agent_running {
            return;
        }
        self.agent_running = running;
        if !running {
            // The turn is over (done, failed or stopped): no change of its
            // waits for the user any more.
            self.drop_live_questions(ctx, "the agent's turn ended before the user answered: the change was not made");
        }
        if let Some(group) = &mut self.doc.agent_group {
            group.open = false;
        }
        for index in 0..self.shelf.len() {
            if let Some(group) = self.shelf.other_mut(index).and_then(|tab| tab.agent_group.as_mut()) {
                group.open = false;
            }
        }
        ctx.request_repaint();
    }

    /// The label of the action that undoes the agent's last group of edits
    /// of the active tab, while it can.
    pub(super) fn agent_undo_offer(&self) -> Option<String> {
        let group = self.doc.agent_group.as_ref().filter(|g| g.text_after == self.doc.text)?;
        Some(if group.edits > 1 {
            tr!("Undo the agent's change ({} edits)", "Annuler la modification de l'agent ({} modifications)", group.edits)
        } else {
            tr("Undo the agent's change", "Annuler la modification de l'agent").to_string()
        })
    }

    /// Undoes the agent's last group of edits of the active tab as one
    /// step, or says why it cannot.
    pub(super) fn undo_agent_change(&mut self, ctx: &egui::Context) {
        let Some(group) = &self.doc.agent_group else {
            self.doc.console.log(Level::Info, tr("The agent made no change to undo here.", "L'agent n'a fait aucune modification à annuler ici."));
            return;
        };
        match group.undo(&self.doc.text) {
            Ok(text) => {
                let edits = group.edits;
                let cursor = self.doc.editor.cursor_char().unwrap_or(0).min(text.chars().count());
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                self.doc.agent_group = None;
                self.doc.agent_highlight = None;
                self.doc.agent_marks.clear();
                let message = tr!("Undid the agent's change ({} edits)", "Modification de l'agent annulée ({} modifications)", edits);
                self.doc.console.log(Level::Info, message.clone());
                self.agent.note(message, false);
                self.render_now();
            }
            Err(why) => {
                self.doc.console.log(Level::Warning, why.clone());
                self.agent.note(why, true);
            }
        }
    }

    /// Gives the editor the agent's marks to draw, as they are now.
    pub(super) fn show_agent_marks(&mut self, ctx: &egui::Context) {
        let doc = &mut self.doc;
        if let Some(highlight) = &mut doc.agent_highlight {
            highlight.see(Instant::now());
        }
        let shown = doc.agent_highlight.as_ref().filter(|h| h.shows(&doc.text));
        let (lines, tint, markers) = match shown {
            Some(h) => (h.lines.clone(), h.tint(Instant::now()), h.markers.clone()),
            None => (Vec::new(), 0.0, Vec::new()),
        };
        if tint > 0.0 {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if shown.is_none() {
            doc.agent_highlight = None;
        }
        doc.editor.agent_tint = (lines, tint);
        doc.editor.agent_markers = markers;
    }

    /// Flashes the box of the part the agent touched over the view in
    /// `rect`, while it lasts.
    pub(super) fn paint_agent_flash(&mut self, ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
        let Some(flash) = self.doc.agent_flash else { return };
        let Some(strength) = flash.strength(Instant::now()) else {
            self.doc.agent_flash = None;
            return;
        };
        ctx.request_repaint();
        let (lo, hi) = (Vec3::from(flash.lo), Vec3::from(flash.hi));
        let corner = |i: usize| Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let points: Vec<Option<Pos2>> = (0..8).map(|i| self.doc.viewer.to_screen(rect, corner(i))).collect();
        let accent = theme::palette().accent;
        let stroke = Stroke::new(1.5 + 1.5 * strength, accent.gamma_multiply(0.35 + 0.65 * strength));
        for a in 0..8usize {
            for bit in [1, 2, 4] {
                let b = a | bit;
                if b == a {
                    continue;
                }
                if let (Some(p), Some(q)) = (points[a], points[b]) {
                    painter.line_segment([p, q], stroke);
                }
            }
        }
        // A soft tint over the box's face on screen.
        let shown: Vec<Pos2> = points.iter().flatten().copied().collect();
        if shown.len() == 8 {
            let face = Rect::from_points(&shown);
            painter.rect_filled(face, 4.0, Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (18.0 * strength) as u8));
        }
    }

    /// Flashes `lo`..`hi` in the view of tab `id`.
    pub(super) fn flash_part(&mut self, id: u64, lo: [f32; 3], hi: [f32; 3]) {
        let Some(index) = self.tab_index(id) else { return };
        self.in_tab(index, |app| app.doc.agent_flash = Some(Flash::new(lo, hi, Instant::now())));
    }

    /// For a screenshot (`CODE2FORM_SCREENSHOT_SELECT` = `agent-edit` or
    /// `agent-review`): an edit of the agent's to the active tab, made or
    /// under review; it scales the first number in the first part.
    pub(super) fn screenshot_agent(&mut self, ctx: &egui::Context) {
        let select = self.screenshot.as_ref().and_then(|s| s.select.clone()).unwrap_or_default();
        if select != "agent-edit" && select != "agent-review" {
            return;
        }
        let text = &self.doc.text;
        let from = part_calls(text).next().map_or(0, |(at, name)| at + name.len() + 8);
        let Some((at, number)) = first_number(&text[from..]).map(|(at, n)| (from + at, n)) else { return };
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        let old = text[line_start..line_end].to_string();
        let scaled = format!("{}", (number.parse::<f64>().unwrap_or(1.0) * 1.5 * 10.0).round() / 10.0);
        let new = format!("{}{scaled}{}", &text[line_start..at], &text[at + number.len()..line_end]);
        let edit = live::CodeEdit::Replace(vec![(old, new)]);
        if select == "agent-review" {
            self.review_agent_edit(ctx, edit, None);
            return;
        }
        let Ok((text, lines)) = live::apply(&self.doc.text, &edit) else { return };
        let done = self.land_agent_edit(ctx, text, lines);
        if let Some(name) = done["part"].as_str() {
            let snapshot = json!({
                "text": self.doc.text,
                "path": self.doc.source_path().display().to_string(),
                "values": {},
            });
            if let Some((_, lo, hi)) = live::design_parts(&snapshot, None).ok().and_then(|parts| parts.into_iter().find(|p| p.0 == name)) {
                self.doc.agent_flash = Some(Flash::new(lo, hi, Instant::now() + Duration::from_millis(1200)));
            }
        }
    }
}

/// The byte where the first number of `text` starts, and the number.
fn first_number(text: &str) -> Option<(usize, &str)> {
    let at = text.char_indices().find(|&(i, c)| {
        c.is_ascii_digit() && !text[..i].chars().next_back().is_some_and(|b| b.is_alphanumeric() || b == '_' || b == '.')
    })?.0;
    let len = text[at..].find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(text.len() - at);
    Some((at, &text[at..at + len]))
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init, reason = "lists of line ranges, of one here")]
mod tests {
    use super::*;

    #[test]
    fn the_tint_holds_then_fades_and_the_marks_go_at_the_user_s_edit() {
        let start = Instant::now();
        let before = "a\nb\nc\n";
        let after = "a\nB\nc\n";
        let first = Highlight::new(None, before, after, vec![1..2], start);
        assert_eq!(first.tint(start), 1.0);
        assert_eq!(first.tint(start + Duration::from_millis(900)), 1.0);
        let half = first.tint(start + HOLD + FADE / 2);
        assert!((half - 0.5).abs() < 0.01, "{half}");
        assert_eq!(first.tint(start + HOLD + FADE), 0.0);
        assert_eq!(first.tint(start + Duration::from_secs(60)), 0.0);
        assert!(first.shows(after));
        // The user typed: the marks go.
        assert!(!first.shows("a\nBx\nc\n"));

        // A second edit inserts a line above: the first edit's mark moves
        // down with its line, and the new line is marked too.
        let later = "z\na\nB\nc\n";
        let second = Highlight::new(Some(&first), after, later, vec![0..1], start);
        assert_eq!(second.lines, vec![0..1]);
        assert_eq!(second.markers, vec![0..1, 2..3]);
        // After an edit of the user's in between, only the new marks show.
        let third = Highlight::new(Some(&first), "a\nB!\nc\n", "a\nB!\nc\nd\n", vec![3..4], start);
        assert_eq!(third.markers, vec![3..4]);
    }

    #[test]
    fn an_edit_landed_out_of_sight_is_tinted_once_the_editor_shows_it() {
        let start = Instant::now();
        let mut highlight = Highlight::new(None, "a\n", "b\n", vec![0..1], start);
        // The agent was shown: by the time the code shows, the tint would be gone.
        let later = start + Duration::from_secs(60);
        assert_eq!(highlight.tint(later), 0.0);
        highlight.see(later);
        assert_eq!(highlight.tint(later), 1.0);
        // Seen once: it fades as it would have.
        highlight.see(later + HOLD + FADE);
        assert_eq!(highlight.tint(later + HOLD + FADE), 0.0);
        assert_eq!(highlight.markers, vec![0..1]);
    }

    #[test]
    fn lines_merge_into_ranges() {
        assert_eq!(ranges(vec![4, 1, 2, 2, 7]), vec![1..3, 4..5, 7..8]);
        assert!(ranges(Vec::new()).is_empty());
    }

    #[test]
    fn an_answer_s_edits_undo_together_unless_the_user_edited_since() {
        let now = Instant::now();
        let group = AgentGroup::record(None, 3, "v0", "v1", now, true);
        let group = AgentGroup::record(Some(group), 3, "v1", "v2", now, true);
        assert_eq!((group.edits, group.text_before.as_str(), group.text_after.as_str()), (2, "v0", "v2"));
        assert_eq!(group.undo("v2").unwrap(), "v0");
        // The user typed after: undoing would throw their edit away.
        assert!(group.undo("v2 + mine").unwrap_err().contains("Cmd+Z"));
        // An edit after the user's starts a group of its own.
        let after_user = AgentGroup::record(Some(group.clone()), 3, "v2 + mine", "v3", now, true);
        assert_eq!((after_user.edits, after_user.text_before.as_str()), (1, "v2 + mine"));
        // So does the next answer's first edit, once the group is closed.
        let mut closed = group;
        closed.open = false;
        let next = AgentGroup::record(Some(closed), 3, "v2", "v4", now, true);
        assert_eq!((next.edits, next.text_before.as_str()), (1, "v2"));
    }

    #[test]
    fn an_outside_agent_s_group_ends_after_a_pause() {
        let now = Instant::now();
        // No turn of the panel's: an agent through `mcp --studio`.
        let group = AgentGroup::record(None, 3, "v0", "v1", now, false);
        let group = AgentGroup::record(Some(group), 3, "v1", "v2", now + Duration::from_secs(4), false);
        assert_eq!(group.edits, 2, "close together: one group");
        let later = AgentGroup::record(Some(group.clone()), 3, "v2", "v3", now + Duration::from_secs(4) + IDLE, false);
        assert_eq!((later.edits, later.text_before.as_str()), (1, "v2"), "after a pause: a group of its own");
        // The panel's turn goes on however long a tool takes.
        let during = AgentGroup::record(Some(group), 3, "v2", "v3", now + Duration::from_secs(60), true);
        assert_eq!(during.edits, 3);
    }

    #[test]
    fn the_flash_pulses_then_ends() {
        let start = Instant::now();
        let flash = Flash::new([0.0; 3], [1.0; 3], start);
        assert_eq!(flash.strength(start), Some(0.0));
        let peak = flash.strength(start + FLASH / 4).unwrap();
        assert!(peak > 0.99, "{peak}");
        assert_eq!(flash.strength(start + FLASH), None);
        // Set to start later: dark until then.
        assert_eq!(Flash::new([0.0; 3], [1.0; 3], start + FLASH).strength(start), Some(0.0));
    }

    #[test]
    fn the_cursor_stays_where_the_user_types() {
        let old = "width = 20;\ndepth = 10;\n// mine|";
        let new = "width = 40;\ndepth = 10;\n// mine|";
        let cursor = old.chars().count() - 1;
        assert_eq!(map_cursor(old, new, cursor), cursor);
        // A line added before the cursor moves it with its text.
        let longer = "// é\nwidth = 20;\ndepth = 10;\n// mine|";
        assert_eq!(map_cursor(old, longer, cursor), longer.chars().count() - 1);
        // Before the change, it stays; inside it, it goes to its end.
        assert_eq!(map_cursor(old, longer, 0), 0);
        assert_eq!(map_cursor("abcdef", "abXYZef", 3), 5);
        assert_eq!(map_cursor("é1", "é22", 1), 1);
    }

    #[test]
    fn an_edit_inside_a_part_names_it() {
        let text = "size = 4;\npart(\"base\") cube(size);\ntranslate([0, 0, 9])\n    part(\"lid\") {\n        cube([size, size, 1]);\n        part(\"knob\") sphere(1);\n    }\n";
        assert_eq!(enclosing_part(text, &[0..1]), None);
        assert_eq!(enclosing_part(text, &[1..2]).as_deref(), Some("base"));
        assert_eq!(enclosing_part(text, &[4..5]).as_deref(), Some("lid"));
        assert_eq!(enclosing_part(text, &[5..6]).as_deref(), Some("knob"));
        assert_eq!(enclosing_part(text, &[]), None);
        // A `;` in a string or a comment does not end the part.
        let tricky = "part(\"a\") { // ; here\n  text(\"x;y\");\n  cube(1);\n}\n";
        assert_eq!(enclosing_part(tricky, &[2..3]).as_deref(), Some("a"));
    }

    #[test]
    fn numbers_are_found_whole() {
        assert_eq!(first_number("cube([w2, 12.5, 3])"), Some((10, "12.5")));
        assert_eq!(first_number("no digits"), None);
    }
}
