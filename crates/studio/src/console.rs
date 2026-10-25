//! The console: what the last render printed, plus file events.

use eframe::egui::{self, RichText, TextStyle};

use crate::i18n::tr;
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Something the studio did, such as saving a file.
    Info,
    Echo,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub level: Level,
    pub text: String,
}

impl Line {
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Line { level, text: text.into() }
    }
}

/// Keeps the console bounded when a script echoes in a loop.
const MAX_ROWS: usize = 10_000;

struct Row {
    level: Level,
    text: String,
    from_render: bool,
    /// The first row of a message (multi-line messages span several rows).
    first: bool,
}

#[derive(Default)]
pub struct Console {
    rows: Vec<Row>,
}

impl Console {
    pub fn log(&mut self, level: Level, text: impl Into<String>) {
        self.push(Line::new(level, text), false);
    }

    /// Replaces the output of the previous render with that of the latest.
    pub fn set_render_output(&mut self, lines: Vec<Line>) {
        self.rows.retain(|row| !row.from_render);
        for line in lines {
            self.push(line, true);
        }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    fn push(&mut self, line: Line, from_render: bool) {
        let prefix = match line.level {
            Level::Warning if !line.text.to_ascii_lowercase().starts_with("warning") => tr("WARNING: ", "ATTENTION : "),
            Level::Error if !line.text.to_ascii_lowercase().starts_with("error") => tr("ERROR: ", "ERREUR : "),
            _ => "",
        };
        let text = line.text.trim_end();
        for (i, part) in text.split('\n').enumerate() {
            let part = part.trim_end_matches('\r');
            let text = if i == 0 { format!("{prefix}{part}") } else { part.to_string() };
            self.rows.push(Row { level: line.level, text, from_render, first: i == 0 });
        }
        if self.rows.len() > MAX_ROWS {
            let excess = self.rows.len() - MAX_ROWS;
            self.rows.drain(..excess);
        }
    }

    /// The number of messages at `level`.
    pub fn count(&self, level: Level) -> usize {
        self.rows.iter().filter(|row| row.first && row.level == level).count()
    }

    /// Each row, oldest first: its level, its text, and whether the last
    /// render printed it.
    pub fn rows(&self) -> impl DoubleEndedIterator<Item = (Level, &str, bool)> {
        self.rows.iter().map(|row| (row.level, row.text.as_str(), row.from_render))
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        let font = theme::code_font();
        let row_height = ui.text_style_height(&TextStyle::Monospace);
        ui.set_min_size(ui.available_size());
        ui.spacing_mut().item_spacing.y = 2.0;
        if self.rows.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr("Nothing to report yet.", "Rien à signaler pour l'instant.")).color(theme::palette().text_faint).font(font));
            return;
        }
        egui::ScrollArea::both().auto_shrink(false).stick_to_bottom(true).show_rows(
            ui,
            row_height,
            self.rows.len(),
            |ui, range| {
                for row in &self.rows[range] {
                    let color = match row.level {
                        Level::Info => theme::palette().text_dim,
                        Level::Echo => theme::palette().text,
                        Level::Warning => theme::palette().warning,
                        Level::Error => theme::palette().error,
                    };
                    let text = if row.text.is_empty() { " " } else { &row.text };
                    ui.add(
                        egui::Label::new(RichText::new(text).font(font.clone()).color(color))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_that_names_its_level_is_not_prefixed_again() {
        let mut console = Console::default();
        console.log(Level::Warning, "unused variable");
        console.log(Level::Warning, "warning: `xiao.D2` is reserved");
        console.log(Level::Error, "error: unknown pin");
        let texts: Vec<&str> = console.rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, ["WARNING: unused variable", "warning: `xiao.D2` is reserved", "error: unknown pin"]);
    }
}
