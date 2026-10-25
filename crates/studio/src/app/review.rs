//! The review of an agent's change before it applies, when the settings
//! say "Ask before each change": the code's diff, old lines and new, and
//! in the view the new model with the current one as a translucent ghost;
//! Accept applies the change as the agent's edits apply, Decline tells
//! the agent no.
//!
//! [`review`] reads a change into a [`ReviewState`], plain data apart from
//! egui; [`StudioApp::review_agent_edit`] is the entry point the live
//! `edit_code` handler calls, which keeps the agent waiting for the
//! user's answer.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use c2f_eval::customizer::Literal;
use eframe::egui::{self, Align, Button, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Vec2};
use serde_json::Value;

use super::StudioApp;
use super::live::{self, CodeEdit};
use crate::console::Level;
use crate::i18n::tr;
use crate::theme;
use crate::worker::{self, Scene};

/// Lines of context around each change in the diff.
const CONTEXT: usize = 3;
/// Past this many cells, the middle of two texts is diffed as replaced
/// whole rather than line by line.
const MAX_CELLS: usize = 4_000_000;
/// How see-through the current model shows behind the proposed one.
const GHOST: [f32; 4] = [0.55, 0.6, 0.7, 0.22];

/// What a line of the diff is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Same,
    Removed,
    Added,
}

/// A line of the diff, with its number (from 0) in the old text, the new,
/// or both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub change: Change,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
}

/// The lines of `old` and `new`, those removed before those added where
/// they differ, the others once.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (middle_a, middle_b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let same = |i: usize, j: usize, text: &str| DiffLine { change: Change::Same, old: Some(i), new: Some(j), text: text.to_string() };
    let mut out: Vec<DiffLine> = (0..prefix).map(|i| same(i, i, a[i])).collect();
    // The longest common subsequence of the middle, from the end.
    let (n, m) = (middle_a.len(), middle_b.len());
    let mut lcs = vec![0u32; if n * m <= MAX_CELLS { (n + 1) * (m + 1) } else { 0 }];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    if !lcs.is_empty() {
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[at(i, j)] = if middle_a[i] == middle_b[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
            }
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let flush = |out: &mut Vec<DiffLine>, removed: &mut Vec<DiffLine>, added: &mut Vec<DiffLine>| {
        out.append(removed);
        out.append(added);
    };
    while i < n || j < m {
        let both = i < n && j < m && !lcs.is_empty();
        if both && middle_a[i] == middle_b[j] {
            flush(&mut out, &mut removed, &mut added);
            out.push(same(prefix + i, prefix + j, middle_a[i]));
            i += 1;
            j += 1;
        } else if i < n && (j >= m || lcs.is_empty() || lcs[at(i + 1, j)] >= lcs[at(i, j + 1)]) {
            removed.push(DiffLine { change: Change::Removed, old: Some(prefix + i), new: None, text: middle_a[i].to_string() });
            i += 1;
        } else {
            added.push(DiffLine { change: Change::Added, old: None, new: Some(prefix + j), text: middle_b[j].to_string() });
            j += 1;
        }
    }
    flush(&mut out, &mut removed, &mut added);
    let (start_a, start_b) = (a.len() - suffix, b.len() - suffix);
    out.extend((0..suffix).map(|k| same(start_a + k, start_b + k, a[start_a + k])));
    out
}

/// Where each line of `old` (from 0) is in `new`, for those the change
/// from one to the other left as they were.
pub fn line_map(old: &str, new: &str) -> Vec<Option<usize>> {
    let mut map = vec![None; old.lines().count()];
    for line in diff_lines(old, new) {
        if let (Change::Same, Some(i), Some(j)) = (line.change, line.old, line.new) {
            map[i] = Some(j);
        }
    }
    map
}

/// The parts of the diff to show: each change with [`CONTEXT`] lines
/// around it, as ranges of the diff's lines, those that touch merged.
pub fn hunks(diff: &[DiffLine]) -> Vec<Range<usize>> {
    let mut hunks: Vec<Range<usize>> = Vec::new();
    for index in (0..diff.len()).filter(|&i| diff[i].change != Change::Same) {
        let range = index.saturating_sub(CONTEXT)..(index + CONTEXT + 1).min(diff.len());
        match hunks.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => hunks.push(range),
        }
    }
    hunks
}

/// A change proposed to a document, read for the review.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewState {
    /// The text with the change made, and the text it was made to.
    pub proposed: String,
    pub current: String,
    pub diff: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
}

/// The review of `proposed_text`, a change to `current_text`.
pub fn review(proposed_text: &str, current_text: &str) -> ReviewState {
    let diff = diff_lines(current_text, proposed_text);
    let count = |change: Change| diff.iter().filter(|l| l.change == change).count();
    ReviewState {
        added: count(Change::Added),
        removed: count(Change::Removed),
        proposed: proposed_text.to_string(),
        current: current_text.to_string(),
        diff,
    }
}

/// The new model with the current one as a ghost, rendered off the UI
/// thread; or why the new one does not render.
type Preview = Result<Scene, String>;

/// A change waiting for the user's answer.
pub struct Pending {
    pub state: ReviewState,
    /// The tab it is for, the edit asked and the lines it changes in the
    /// proposed text.
    tab: u64,
    edit: CodeEdit,
    lines: Vec<Range<usize>>,
    /// Where the answer goes: the agent waits on it. `None` for a review
    /// shown without an agent (a screenshot).
    reply: Option<Sender<Result<Value, String>>>,
    preview: Option<Receiver<Preview>>,
    /// Why the proposed model does not render, once known.
    problem: Option<String>,
}

impl Pending {
    /// Whether the preview is in, or will never be.
    pub fn settled(&self) -> bool {
        self.preview.is_none()
    }
}

/// The current scene behind `proposed`, see-through.
fn ghosted(mut proposed: Scene, current: Scene) -> Scene {
    let ghost = &mut proposed.backgrounds;
    let solid = current.solid;
    ghost.colors.extend(std::iter::repeat_n(GHOST, solid.positions.len()));
    ghost.positions.extend(solid.positions);
    ghost.normals.extend(solid.normals);
    proposed
}

/// Renders `proposed` with `current` as its ghost, on a thread of its own.
fn preview(ctx: &egui::Context, current: String, proposed: String, path: PathBuf, values: Vec<(String, Literal)>, libraries: Vec<PathBuf>) -> Receiver<Preview> {
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("c2f-review".into()).stack_size(worker::STACK).spawn(move || {
        let proposed = worker::render_preview(&proposed, &path, &values, &libraries);
        let result = proposed.map(|scene| match worker::render_preview(&current, &path, &values, &libraries) {
            Ok(current) => ghosted(scene, current),
            Err(_) => scene,
        });
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    if let Err(err) = spawned {
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(Err(err.to_string()));
        return receiver;
    }
    receiver
}

impl StudioApp {
    /// Shows the user the change `edit` makes to the active tab, and keeps
    /// `reply` until they accept or decline it: the entry point of the
    /// live `edit_code` handler. An edit that cannot be made, or a second
    /// one while a review waits, is refused at once.
    pub(super) fn review_agent_edit(&mut self, ctx: &egui::Context, edit: CodeEdit, reply: Option<Sender<Result<Value, String>>>) {
        let refuse = |reply: Option<Sender<Result<Value, String>>>, why: String| {
            if let Some(reply) = reply {
                let _ = reply.send(Err(why));
            }
        };
        if self.live.review.is_some() {
            return refuse(reply, "another change of yours waits for the user's review: wait for its answer".into());
        }
        let (proposed, lines) = match live::apply(&self.doc.text, &edit) {
            Ok(done) => done,
            Err(why) => return refuse(reply, live::refused(why)),
        };
        let state = review(&proposed, &self.doc.text);
        let libraries = self.settings.library_folders.clone();
        let receiver = if self.doc.is_circuit() {
            None
        } else {
            let values = self.doc.customizer.values();
            Some(preview(ctx, self.doc.text.clone(), proposed, self.doc.source_path(), values, libraries))
        };
        self.doc.console.log(Level::Info, tr("The agent proposes a change: review it", "L'agent propose une modification : à examiner"));
        self.live.review = Some(Pending { state, tab: self.doc.id, edit, lines, reply, preview: receiver, problem: None });
        ctx.request_repaint();
    }

    /// Takes in the preview of the change under review once rendered: it
    /// shows in the view of the change's tab.
    pub(super) fn receive_review(&mut self) {
        let Some(pending) = &mut self.live.review else { return };
        if let Some(receiver) = &pending.preview {
            match receiver.try_recv() {
                Ok(result) => {
                    pending.preview = None;
                    match result {
                        Ok(scene) => {
                            let tab = pending.tab;
                            if let Some(index) = self.tab_index(tab) {
                                self.in_tab(index, |app| app.doc.viewer.set_scene(scene, false));
                            }
                        }
                        Err(why) => pending.problem = Some(why),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => pending.preview = None,
            }
        }
    }

    /// The review's window, while a change waits: its diff, Accept and
    /// Decline; the preview shows in the view once rendered.
    pub(super) fn review_window(&mut self, ctx: &egui::Context) {
        self.receive_review();
        let Some(pending) = &self.live.review else { return };
        let palette = theme::palette();
        let mut answer = None;
        let width = (ctx.screen_rect().width() * 0.45).clamp(340.0, 620.0);
        egui::Window::new(tr("The agent's change", "Modification de l'agent"))
            .id(egui::Id::new("agent-review"))
            .collapsible(false)
            .resizable(false)
            .default_width(width)
            .max_width(width)
            .anchor(egui::Align2::LEFT_BOTTOM, Vec2::new(16.0, -40.0))
            .frame(Frame::window(&ctx.style()).fill(palette.panel).inner_margin(Margin::same(12)))
            .show(ctx, |ui| {
                let summary = tr!(
                    "Lines +{} −{}. The view shows the new model, the current one see-through.",
                    "Lignes +{} −{}. La vue montre le nouveau modèle, l'actuel en transparence.",
                    pending.state.added,
                    pending.state.removed
                );
                ui.label(RichText::new(summary).size(12.5).color(palette.text_dim));
                if !pending.settled() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new(tr("Rendering the new model…", "Rendu du nouveau modèle…")).size(12.0).color(palette.text_dim));
                    });
                }
                if let Some(problem) = &pending.problem {
                    ui.label(RichText::new(tr!("The new model does not render: {problem}", "Le nouveau modèle ne se rend pas : {problem}")).size(12.0).color(palette.error));
                }
                ui.add_space(6.0);
                Frame::new().fill(palette.editor).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                    egui::ScrollArea::both().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| diff_view(ui, &pending.state.diff));
                });
                ui.add_space(8.0);
                let row = Vec2::new(ui.available_width(), 28.0);
                ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                    let accept = Button::new(RichText::new(tr("Accept", "Accepter")).color(palette.on_accent).strong())
                        .fill(palette.accent)
                        .stroke(Stroke::NONE);
                    if ui.add(accept).on_hover_text(tr("Apply the change", "Appliquer la modification")).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(tr("Decline", "Refuser")).on_hover_text(tr("Leave the code as it is", "Laisser le code tel quel")).clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(accepted) = answer {
            self.answer_review(ctx, accepted);
        }
    }

    /// Applies the change under review, or declines it, and tells the
    /// agent.
    pub(super) fn answer_review(&mut self, ctx: &egui::Context, accepted: bool) {
        let Some(pending) = self.live.review.take() else { return };
        let Some(index) = self.tab_index(pending.tab) else {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Err("the tab was closed during the review".into()));
            }
            return;
        };
        let shown = index == self.shelf.active();
        let mut answer = Err(String::new());
        self.in_tab(index, |app| {
            answer = if accepted {
                // The user may have typed meanwhile: the edit is made again
                // on the text as it is now.
                let made = if app.doc.text == pending.state.current {
                    Ok((pending.state.proposed.clone(), pending.lines.clone()))
                } else {
                    live::apply(&app.doc.text, &pending.edit).map_err(live::refused)
                };
                made.map(|(text, lines)| app.land_agent_edit(ctx, text, lines))
            } else {
                app.doc.console.log(Level::Info, tr("Declined the agent's change", "Modification de l'agent refusée"));
                // The view shows the current model again.
                app.render_when_shown(shown);
                Err("the user declined this change after reviewing it: ask them what they want instead, or call studio_state and propose another".into())
            };
        });
        if let Some(reply) = pending.reply {
            let _ = reply.send(answer);
        }
    }
}

/// The diff's changes with their context, old lines red and new green.
fn diff_view(ui: &mut egui::Ui, diff: &[DiffLine]) {
    let palette = theme::palette();
    let font = theme::code_font();
    ui.spacing_mut().item_spacing.y = 0.0;
    let hunks = hunks(diff);
    if hunks.is_empty() {
        ui.label(RichText::new(tr("No change to the code.", "Aucune modification du code.")).color(palette.text_faint));
    }
    for (index, hunk) in hunks.into_iter().enumerate() {
        if index > 0 {
            ui.label(RichText::new("⋯").font(font.clone()).color(palette.text_faint));
        }
        for line in &diff[hunk] {
            let (sign, color, fill) = match line.change {
                Change::Same => (' ', palette.text_dim, Color32::TRANSPARENT),
                Change::Removed => ('−', palette.error, palette.error.gamma_multiply(0.14)),
                Change::Added => ('+', palette.success, palette.success.gamma_multiply(0.14)),
            };
            let number = line.new.or(line.old).map_or(String::new(), |n| (n + 1).to_string());
            let text = format!("{number:>4} {sign} {}", line.text);
            Frame::new().fill(fill).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(text).font(font.clone()).color(color)).extend());
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(diff: &[DiffLine]) -> String {
        diff.iter()
            .map(|l| match l.change {
                Change::Same => format!(" {}", l.text),
                Change::Removed => format!("-{}", l.text),
                Change::Added => format!("+{}", l.text),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_diff_shows_removed_then_added_lines() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nd\ne\n";
        assert_eq!(changes(&diff_lines(old, new)), " a\n-b\n+B\n c\n d\n+e");
        let diff = diff_lines(old, new);
        assert_eq!(diff[1], DiffLine { change: Change::Removed, old: Some(1), new: None, text: "b".into() });
        assert_eq!(diff[2], DiffLine { change: Change::Added, old: None, new: Some(1), text: "B".into() });
        assert_eq!(diff[5].new, Some(4));
        // Lines kept between changes are found again.
        assert_eq!(changes(&diff_lines("x\na\ny\nb\nz", "a\nq\nb")), "-x\n a\n-y\n+q\n b\n-z");
        assert!(diff_lines(old, old).iter().all(|l| l.change == Change::Same));
        assert_eq!(changes(&diff_lines("", "a")), "+a");
    }

    #[test]
    fn lines_left_alone_map_to_their_new_place() {
        assert_eq!(line_map("a\nb\nc\n", "z\na\nc\n"), vec![Some(1), None, Some(2)]);
    }

    #[test]
    fn hunks_keep_three_lines_around_each_change() {
        let text = |renamed: &[(usize, &str)]| -> String {
            (0..20).map(|i| renamed.iter().find(|(n, _)| *n == i).map_or(format!("{i}\n"), |(_, s)| format!("{s}\n"))).collect()
        };
        let old = text(&[]);
        // Lines 2 and 15 replaced: each a line removed and one added.
        let diff = diff_lines(&old, &text(&[(2, "two"), (15, "fifteen")]));
        assert_eq!(diff.len(), 22);
        assert_eq!(hunks(&diff), vec![0..7, 13..21]);
        let both = diff_lines(&old, &text(&[(2, "two"), (4, "four")]));
        assert_eq!(hunks(&both), vec![0..10]);
    }

    #[test]
    fn a_review_counts_its_lines() {
        let state = review("cube(20);\nsphere(5);\n", "cube(10);\n");
        assert_eq!((state.added, state.removed), (2, 1));
        assert_eq!(state.current, "cube(10);\n");
        assert_eq!(changes(&state.diff), "-cube(10);\n+cube(20);\n+sphere(5);");
    }
}
