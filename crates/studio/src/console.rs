//! The console: what the last render printed, plus file events.

use eframe::egui::{self, RichText, TextStyle};

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
            Level::Warning if !line.text.starts_with("WARNING") => "WARNING: ",
            Level::Error if !line.text.to_ascii_lowercase().starts_with("error") => "ERROR: ",
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
            ui.label(RichText::new("Nothing to report yet.").color(theme::TEXT_FAINT).font(font));
            return;
        }
        egui::ScrollArea::both().auto_shrink(false).stick_to_bottom(true).show_rows(
            ui,
            row_height,
            self.rows.len(),
            |ui, range| {
                for row in &self.rows[range] {
                    let color = match row.level {
                        Level::Info => theme::TEXT_DIM,
                        Level::Echo => theme::TEXT,
                        Level::Warning => theme::WARNING,
                        Level::Error => theme::ERROR,
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
