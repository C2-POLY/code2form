//! Manual modelling: changing a design with the mouse, as in Fusion, while
//! the code stays the source of the model. Every gesture is a pure edit of
//! the code ([`crate::code_edit`]) that the editor applies as one undo step,
//! then the design renders again with the edited object still selected.
//!
//! Each feature lives in a file of its own: the workspaces and their
//! panels (`workspace.rs`), the inspector and its properties
//! (`properties.rs`), the move, turn and scale handles (`handles.rs`), the
//! shapes toolbar and the booleans (`shapes.rs`), the tree of the program
//! (`tree.rs`), the modelling toolbar (`toolbar.rs`), and the sketch mode
//! (`sketch.rs`).

use std::ops::Range;

use eframe::egui;

use super::StudioApp;
use crate::code_edit::Edit;
use crate::console::Level;

impl StudioApp {
    /// The panels of manual modelling, before the 3D view takes the rest
    /// of the window: in the Model workspace, the browser, the inspector
    /// and the modelling tools.
    pub(super) fn modelling_panels(&mut self, ctx: &egui::Context) {
        if self.doc.is_circuit() {
            return;
        }
        self.sketch_panel(ctx);
        let models = self.models();
        // The view's card names the selection where no inspector does:
        // in the Code workspace, and while sketching.
        self.doc.viewer.selection_card = self.doc.sketch.active() || !models;
        if !models {
            return;
        }
        self.tree_panel(ctx);
        // The sketch's own panel and tools take their place while sketching.
        if !self.doc.sketch.active() {
            self.inspector_panel(ctx);
            self.modelling_toolbar(ctx);
        }
    }

    /// What manual modelling draws over the 3D view and does with the
    /// pointer and the keys there, after the view took its own input.
    pub(super) fn modelling_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, response: &egui::Response) {
        if self.doc.is_circuit() {
            return;
        }
        // While sketching, the view is the sketch's alone.
        if self.sketch_view(ctx, ui, response) {
            return;
        }
        self.handles_view(ctx, ui, response);
        self.shapes_view(ctx, ui, response);
    }

    /// The statements selected in the view, as byte ranges of the code;
    /// `None` while the code is not the one on screen (typed since the last
    /// render), which the ranges would not point into.
    pub(super) fn selected_ranges(&self) -> Option<Vec<Range<usize>>> {
        if self.doc.viewer.scene_source() != self.doc.text {
            return None;
        }
        self.doc.viewer.selection().iter().map(|key| key.range()).collect()
    }

    /// Applies an edit of the code as one step of the editor's undo
    /// history, logs `done`, renders, and selects what the edit left
    /// selected once rendered. A refused edit logs why.
    pub(super) fn apply_code_edit(&mut self, ctx: &egui::Context, result: Result<Edit, String>, done: &str) {
        self.apply_code_edits(ctx, result.map(|edit| (edit.text, edit.cursor, vec![edit.selection])), done);
    }

    /// [`Self::apply_code_edit`] for an edit that leaves several objects
    /// selected: the new text, the cursor (a char index) and their ranges.
    pub(super) fn apply_code_edits(
        &mut self,
        ctx: &egui::Context,
        result: Result<(String, usize, Vec<Range<usize>>), String>,
        done: &str,
    ) {
        match result {
            Ok((text, cursor, selection)) => {
                if text == self.doc.text {
                    return;
                }
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                let selection = selection.into_iter().filter(|r| !r.is_empty()).collect();
                self.doc.viewer.select_after_render(self.doc.text.clone(), selection);
                if !done.is_empty() {
                    self.doc.console.log(Level::Info, done);
                }
                self.render_now();
            }
            Err(why) => self.doc.console.log(Level::Warning, why),
        }
    }
}
