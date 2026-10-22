//! The studio window: toolbar, editor, 3D view, console and status bar,
//! and the document they work on.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, Frame, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText,
    Sense, Stroke, Vec2,
};
use eframe::glow;
use c2f_eval::customizer::Literal;
use c2f_geom::Stats;

use crate::console::{Console, Level};
use crate::customizer::{self, Customizer};
use crate::editor::{Editor, FindRequest};
use crate::open_events;
use crate::screenshot::Screenshot;
use crate::theme;
use crate::viewer::{self, Viewer};
use crate::worker::{Format, Reply, Worker, group_digits};

/// File dialog filters: a name and an extension.
const DESIGN_FILES: (&str, &str) = ("Code2Form design", "c2f");
const SCAD_FILES: (&str, &str) = ("SCAD design", "scad");

/// How long typing must pause before a re-render.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// How long a customizer control must rest before a re-render.
const PARAMETER_DEBOUNCE: Duration = Duration::from_millis(120);
/// How often the file is checked for changes on disk.
const POLL: Duration = Duration::from_secs(1);

const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::S);
const RENDER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F7);
const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const FIND_REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers { alt: true, ..Modifiers::COMMAND }, Key::F);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
const FIND_PREVIOUS: KeyboardShortcut = KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::G);

/// Something that would lose unsaved edits, waiting for confirmation.
#[derive(Clone)]
enum Pending {
    New,
    Open,
    OpenPath(PathBuf),
    Quit,
}

impl Pending {
    fn verb(&self) -> &'static str {
        match self {
            Pending::New => "starting a new file",
            Pending::Open | Pending::OpenPath(_) => "opening another file",
            Pending::Quit => "quitting",
        }
    }
}

/// What the status bar and toolbar know about the model on screen.
struct ModelInfo {
    stats: Stats,
    is_2d: bool,
    /// The formats it exports to.
    formats: &'static [Format],
    empty: bool,
}

pub struct StudioApp {
    text: String,
    /// The text as last loaded or saved, to tell unsaved edits.
    saved_text: String,
    path: Option<PathBuf>,
    /// Modification time of the file when last read or written.
    disk_mtime: Option<SystemTime>,
    last_poll: Instant,

    worker: Worker,
    /// When to re-render after typing.
    render_deadline: Option<Instant>,
    /// The source and customizer values of the last render request.
    sent_source: Option<String>,
    sent_values: Vec<(String, Literal)>,
    /// The newest render that came back.
    finished: u64,
    elapsed: Option<Duration>,
    failed: bool,
    model: Option<ModelInfo>,
    /// Frame the next model that arrives (after opening a file).
    frame_next: bool,

    editor: Editor,
    viewer: Viewer,
    console: Console,
    console_open: bool,
    customizer: Customizer,
    /// Shown or hidden by the user; by default shown when the file has
    /// parameters.
    customizer_open: Option<bool>,
    confirm: Option<Pending>,
    allow_close: bool,
    title: String,
    screenshot: Option<Screenshot>,
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        open_events::wake(&cc.egui_ctx);
        // Launched by opening a file in the Finder.
        let path = path.or_else(|| open_events::take().pop());
        let mut app = StudioApp {
            text: String::new(),
            saved_text: String::new(),
            path: None,
            disk_mtime: None,
            last_poll: Instant::now(),
            worker: Worker::spawn(cc.egui_ctx.clone()),
            render_deadline: None,
            sent_source: None,
            sent_values: Vec::new(),
            finished: 0,
            elapsed: None,
            failed: false,
            model: None,
            frame_next: true,
            editor: Editor::new(),
            viewer: Viewer::new(cc.gl.as_ref()),
            console: Console::default(),
            console_open: true,
            customizer: Customizer::default(),
            customizer_open: None,
            confirm: None,
            allow_close: false,
            title: String::new(),
            screenshot: Screenshot::from_env(),
        };
        match path {
            Some(path) if path.exists() => app.load(path),
            Some(path) => {
                app.console.log(Level::Info, format!("New file {} (not saved yet)", path.display()));
                app.path = Some(absolute(&path));
                app.customizer.open(app.path.as_deref());
                app.render_now();
            }
            None => app.render_now(),
        }
        app.editor.focus(&cc.egui_ctx);
        app
    }

    fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }

    fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Where the script lives, for resolving its imports.
    fn source_path(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| working_directory().unwrap_or_default().join("untitled.c2f"))
    }

    fn directory(&self) -> Option<PathBuf> {
        self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf).or_else(working_directory)
    }

    fn busy(&self) -> bool {
        self.worker.latest() > self.finished
    }

    // ---- Rendering ----

    fn render_now(&mut self) {
        self.render_deadline = None;
        self.sent_source = Some(self.text.clone());
        self.sent_values = self.customizer.values();
        self.worker.render(self.text.clone(), self.source_path(), self.sent_values.clone());
    }

    /// Renders unless the text and values are what was last rendered.
    fn render_if_changed(&mut self) {
        self.render_deadline = None;
        if self.sent_source.as_deref() != Some(self.text.as_str()) || self.sent_values != self.customizer.values() {
            self.render_now();
        }
    }

    fn customizer_shown(&self) -> bool {
        self.customizer_open.unwrap_or_else(|| self.customizer.has_parameters())
    }

    fn receive(&mut self) {
        for reply in self.worker.poll() {
            match reply {
                Reply::Rendered(outcome) => {
                    if outcome.generation <= self.finished {
                        continue;
                    }
                    self.finished = outcome.generation;
                    self.elapsed = Some(outcome.elapsed);
                    // A reply from before a file was opened would bring back
                    // the previous file's parameters.
                    let current = outcome.generation >= self.worker.latest();
                    if let Some(parameters) = outcome.parameters.filter(|_| current) {
                        self.customizer.set_parameters(parameters);
                    }
                    self.console.set_render_output(outcome.console);
                    match outcome.result {
                        Ok(scene) => {
                            self.failed = false;
                            self.editor.error_line = None;
                            let empty = scene.is_empty();
                            let had_model = self.model.as_ref().is_some_and(|m| !m.empty);
                            let frame = !empty && (self.frame_next || !had_model);
                            if frame {
                                self.frame_next = false;
                            }
                            self.model = Some(ModelInfo {
                                stats: scene.stats,
                                is_2d: scene.is_2d,
                                formats: scene.formats,
                                empty,
                            });
                            self.viewer.set_scene(scene, frame);
                        }
                        Err(line) => {
                            self.failed = true;
                            self.editor.error_line = line;
                        }
                    }
                }
                Reply::Exported { target, result } => match result {
                    Ok(message) => self.console.log(Level::Info, message),
                    Err(error) => {
                        self.console.log(Level::Error, format!("Exporting {} failed: {error}", target.display()));
                    }
                },
            }
        }
    }

    // ---- Files ----

    fn load(&mut self, path: PathBuf) {
        let path = absolute(&path);
        match c2f_eval::read_source(&path) {
            Ok(text) => {
                self.text = text.clone();
                self.saved_text = text;
                self.disk_mtime = modified(&path);
                self.console.log(Level::Info, format!("Opened {}", path.display()));
                self.customizer.open(Some(&path));
                self.path = Some(path);
                self.frame_next = true;
                self.editor.error_line = None;
                self.render_now();
            }
            Err(err) => self.console.log(Level::Error, format!("Cannot open {}: {err}", path.display())),
        }
    }

    fn new_document(&mut self) {
        self.text.clear();
        self.saved_text.clear();
        self.path = None;
        self.disk_mtime = None;
        self.frame_next = true;
        self.editor.error_line = None;
        self.customizer.open(None);
        self.console.log(Level::Info, "New file");
        self.render_now();
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open a design")
            .add_filter("Code2Form or SCAD", &["c2f", "scad"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.directory() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.load(path);
        }
    }

    /// Saves; returns false when cancelled or failed.
    fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(path) => self.write_to(path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> bool {
        // Designs are .c2f files; a .scad file keeps its extension by default.
        let scad = self.path.as_deref().and_then(Path::extension).is_some_and(|e| e == "scad");
        let (first, second) = if scad { (SCAD_FILES, DESIGN_FILES) } else { (DESIGN_FILES, SCAD_FILES) };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save as")
            .add_filter(first.0, &[first.1])
            .add_filter(second.0, &[second.1])
            .set_file_name(if self.path.is_some() { self.file_name() } else { "untitled.c2f".to_string() });
        if let Some(dir) = self.directory() {
            dialog = dialog.set_directory(dir);
        }
        match dialog.save_file() {
            Some(path) => self.write_to(path),
            None => false,
        }
    }

    fn write_to(&mut self, path: PathBuf) -> bool {
        let path = absolute(&path);
        match std::fs::write(&path, &self.text) {
            Ok(()) => {
                self.saved_text = self.text.clone();
                self.disk_mtime = modified(&path);
                self.console.log(Level::Info, format!("Saved {}", path.display()));
                self.customizer.moved(&path);
                self.path = Some(path);
                self.render_now();
                true
            }
            Err(err) => {
                self.console.log(Level::Error, format!("Cannot save {}: {err}", path.display()));
                false
            }
        }
    }

    fn export(&mut self, format: Format) {
        let formats = self.model.as_ref().map_or(&[][..], |m| m.formats);
        if !formats.contains(&format) {
            let why = match formats.first() {
                None => "the model is empty".to_string(),
                Some(other) if Format::FLAT.contains(other) => "the model is 2D; export it as SVG or DXF".to_string(),
                Some(_) => "the model is 3D; export it as STL or 3MF".to_string(),
            };
            self.console.log(Level::Warning, format!("Nothing to export as {}: {why}", format.name()));
            return;
        }
        let stem = self
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "model".to_string(), |s| s.to_string_lossy().into_owned());
        let (name, extension) = (format.name(), format.extension());
        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Export {name}"))
            .add_filter(name, &[extension])
            .set_file_name(format!("{stem}.{extension}"));
        if let Some(dir) = self.directory() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(target) = dialog.save_file() {
            self.worker.export(target, format);
        }
    }

    /// Reloads the file when it changed on disk and has no unsaved edits.
    fn poll_disk(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else { return };
        ctx.request_repaint_after(POLL);
        if self.last_poll.elapsed() < POLL {
            return;
        }
        self.last_poll = Instant::now();
        let Some(mtime) = modified(&path) else { return };
        if self.disk_mtime == Some(mtime) {
            return;
        }
        self.disk_mtime = Some(mtime);
        if self.is_dirty() {
            self.console.log(Level::Warning, format!("{} changed on disk; keeping your unsaved edits", path.display()));
            return;
        }
        match c2f_eval::read_source(&path) {
            Ok(text) if text != self.text => {
                self.text = text.clone();
                self.saved_text = text;
                self.console.log(Level::Info, format!("Reloaded {} (changed on disk)", path.display()));
                self.render_now();
            }
            Ok(_) => {}
            Err(err) => self.console.log(Level::Error, format!("Cannot reload {}: {err}", path.display())),
        }
    }

    /// Runs `action`, first asking about unsaved edits.
    fn guard(&mut self, action: Pending) {
        if self.is_dirty() {
            self.confirm = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Pending) {
        match action {
            Pending::New => self.new_document(),
            Pending::Open => self.open_dialog(),
            Pending::OpenPath(path) => self.load(path),
            Pending::Quit => {
                self.allow_close = true;
            }
        }
    }

    // ---- Input ----

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let [save_as, save, open, new, render, export] = ctx.input_mut(|i| {
            // Shift+Cmd+S first: Cmd+S also matches it.
            [SAVE_AS, SAVE, OPEN, NEW, RENDER, EXPORT].map(|s| i.consume_shortcut(&s))
        });
        if save_as {
            self.save_as();
        } else if save {
            self.save();
        }
        if open {
            self.guard(Pending::Open);
        }
        if new {
            self.guard(Pending::New);
        }
        if render {
            self.render_now();
        }
        if export {
            self.export(Format::Stl);
        }

        // Option+Cmd+F and Shift+Cmd+G first: their shorter forms match them.
        let [replace, find, previous, next] =
            ctx.input_mut(|i| [FIND_REPLACE, FIND, FIND_PREVIOUS, FIND_NEXT].map(|s| i.consume_shortcut(&s)));
        if replace || find {
            self.editor.find(FindRequest::Open { replace });
        }
        if previous || next {
            self.editor.find(FindRequest::Step { forward: next });
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(path) = dropped.into_iter().next() {
            self.guard(Pending::OpenPath(path));
        }

        if ctx.input(|i| i.viewport().close_requested()) && self.is_dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Pending::Quit);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!("{}{} — Code2Form", if self.is_dirty() { "● " } else { "" }, self.file_name());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    // ---- Panels ----

    fn toolbar(&mut self, ctx: &egui::Context) {
        let frame = Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(12, 8));
        egui::TopBottomPanel::top("toolbar").frame(frame).show_separator_line(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                logo(ui);
                ui.label(RichText::new("Code2Form").strong().color(theme::TEXT).size(14.5));
                ui.add_space(10.0);
                toolbar_separator(ui);

                if tool_button(ui, "New", "New file", Some(NEW)).clicked() {
                    self.guard(Pending::New);
                }
                if tool_button(ui, "Open…", "Open a file", Some(OPEN)).clicked() {
                    self.guard(Pending::Open);
                }
                if tool_button(ui, "Save", "Save the file", Some(SAVE)).clicked() {
                    self.save();
                }
                if tool_button(ui, "Save As…", "Save under a new name", Some(SAVE_AS)).clicked() {
                    self.save_as();
                }
                toolbar_separator(ui);

                let render = Button::new(RichText::new("Render").color(Color32::WHITE).strong())
                    .fill(theme::ACCENT)
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(6));
                let shortcut = ctx.format_shortcut(&RENDER);
                if ui.add(render).on_hover_text(format!("Render now ({shortcut})")).clicked() {
                    self.render_now();
                }
                let formats = self.model.as_ref().map_or(&[][..], |m| m.formats);
                let export = ui.add_enabled_ui(!formats.is_empty(), |ui| {
                    ui.menu_button("Export", |ui| {
                        // Solids above the separator, 2D models below.
                        let entries = [
                            (Format::Stl, "An ASCII STL"),
                            (Format::ThreeMf, "A 3MF package, in millimeters"),
                            (Format::Svg, "An SVG drawing, in millimeters"),
                            (Format::Dxf, "A DXF drawing of line segments"),
                        ];
                        for (format, hover) in entries {
                            if format == Format::Svg {
                                ui.separator();
                            }
                            let mut button = Button::new(format!("{}…", format.name()));
                            if format == Format::Stl {
                                button = button.shortcut_text(ctx.format_shortcut(&EXPORT));
                            }
                            let disabled = if Format::SOLID.contains(&format) { "For 3D models" } else { "For 2D models" };
                            let response = ui.add_enabled(formats.contains(&format), button);
                            if response.on_hover_text(hover).on_disabled_hover_text(disabled).clicked() {
                                self.export(format);
                                ui.close();
                            }
                        }
                    })
                });
                export.response.on_disabled_hover_text("Render a non-empty model first");

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut shown = self.customizer_shown();
                    let toggle = ui.toggle_value(&mut shown, "Customizer");
                    if toggle.on_hover_text("Show the parameters the file declares").changed() {
                        self.customizer_open = Some(shown);
                    }
                    toolbar_separator(ui);
                    ui.toggle_value(&mut self.viewer.show_grid, "Grid").on_hover_text("Show the XY grid");
                    ui.toggle_value(&mut self.viewer.show_axes, "Axes").on_hover_text("Show the axes");
                    toolbar_separator(ui);
                    ui.menu_button("View", |ui| {
                        for &(name, yaw, pitch) in viewer::PRESETS {
                            if ui.button(name).clicked() {
                                self.viewer.look_from(yaw, pitch);
                                ui.close();
                            }
                        }
                    });
                    if ui
                        .button("Reset view")
                        .on_hover_text("Default angle, fit the model (or double-click the view)")
                        .clicked()
                    {
                        self.viewer.reset();
                    }
                    ui.toggle_value(&mut self.viewer.turntable, "Turntable")
                        .on_hover_text("Turn the view around the model (a drag in the view stops it)");
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let frame = Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(12, 4));
        egui::TopBottomPanel::bottom("status").frame(frame).exact_height(26.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let small = |text: String| RichText::new(text).size(12.0).color(theme::TEXT_DIM);
                if self.busy() {
                    ui.add(egui::Spinner::new().size(11.0).color(theme::ACCENT_STRONG));
                    ui.label(RichText::new("Rendering…").size(12.0).color(theme::ACCENT_STRONG));
                } else if self.failed {
                    status_dot(ui, theme::ERROR);
                    ui.label(RichText::new("Render failed").size(12.0).color(theme::ERROR));
                } else {
                    status_dot(ui, theme::SUCCESS);
                    ui.label(RichText::new("Ready").size(12.0).color(theme::TEXT));
                }
                if let Some(elapsed) = self.elapsed {
                    ui.label(small(format!("· {}", format_duration(elapsed))));
                }
                if let Some(model) = &self.model {
                    if !model.empty {
                        let s = model.stats;
                        if model.is_2d {
                            ui.label(small(format!("· 2D · {} vertices", group_digits(s.vertices))));
                            ui.label(small(format!("· area {}", format_quantity(s.volume))));
                        } else {
                            ui.label(small(format!("· {} triangles", group_digits(s.triangles))));
                            ui.label(small(format!("· volume {}", format_quantity(s.volume))));
                        }
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if self.is_dirty() {
                        ui.label(RichText::new("●").size(12.0).color(theme::WARNING)).on_hover_text("Unsaved changes");
                    }
                    let name = ui.label(RichText::new(self.file_name()).size(12.0).color(theme::TEXT));
                    if let Some(path) = &self.path {
                        name.on_hover_text(path.display().to_string());
                    }
                    if let Some((line, column)) = self.editor.cursor {
                        ui.label(small(format!("Ln {}, Col {}   ", line + 1, column + 1)));
                    }
                });
            });
        });
    }

    fn console_panel(&mut self, ctx: &egui::Context) {
        let header = |ui: &mut egui::Ui, open: &mut bool, console: &mut Console| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let arrow = if *open { "⏷" } else { "⏵" };
                let toggle = ui.add(
                    Button::new(RichText::new(format!("{arrow}  Console")).strong().size(12.5).color(theme::TEXT))
                        .frame(false),
                );
                if toggle.clicked() {
                    *open = !*open;
                }
                let errors = console.count(Level::Error);
                let warnings = console.count(Level::Warning);
                let echoes = console.count(Level::Echo);
                if errors > 0 {
                    pill(ui, &format!("{errors} error{}", plural(errors)), theme::ERROR);
                }
                if warnings > 0 {
                    pill(ui, &format!("{warnings} warning{}", plural(warnings)), theme::WARNING);
                }
                if echoes > 0 {
                    pill(ui, &format!("{echoes} echo"), theme::TEXT_DIM);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(!console.is_empty(), Button::new(RichText::new("Clear").size(12.0))).clicked() {
                        console.clear();
                    }
                });
            });
        };
        let frame = Frame::new().fill(theme::PANEL).inner_margin(Margin { left: 12, right: 12, top: 6, bottom: 6 });
        if self.console_open {
            egui::TopBottomPanel::bottom("console")
                .frame(frame)
                .resizable(true)
                .default_height(170.0)
                .height_range(90.0..=ctx.screen_rect().height() * 0.6)
                .show(ctx, |ui| {
                    header(ui, &mut self.console_open, &mut self.console);
                    ui.add_space(2.0);
                    Frame::new()
                        .fill(theme::EDITOR)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| self.console.show(ui));
                });
        } else {
            egui::TopBottomPanel::bottom("console-collapsed")
                .frame(frame)
                .resizable(false)
                .show(ctx, |ui| header(ui, &mut self.console_open, &mut self.console));
        }
    }

    fn editor_panel(&mut self, ctx: &egui::Context) {
        let width = ctx.screen_rect().width();
        egui::SidePanel::left("editor")
            .frame(Frame::new().fill(theme::EDITOR))
            .resizable(true)
            .default_width((width * 0.42).max(320.0))
            .width_range(260.0..=(width - 300.0).max(300.0))
            .show(ctx, |ui| {
                self.editor_tab(ui);
                if self.editor.show(ui, &mut self.text) {
                    self.render_deadline = Some(Instant::now() + DEBOUNCE);
                }
            });
    }

    /// The strip above the editor with the file name.
    fn editor_tab(&self, ui: &mut egui::Ui) {
        let (strip, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
        let painter = ui.painter_at(strip);
        painter.rect_filled(strip, CornerRadius::ZERO, theme::BG);
        let name = self.file_name();
        let font = egui::FontId::proportional(13.0);
        let galley = painter.layout_no_wrap(name, font, theme::TEXT);
        let dot = if self.is_dirty() { 16.0 } else { 0.0 };
        let tab = egui::Rect::from_min_size(
            strip.left_top() + Vec2::new(8.0, 6.0),
            Vec2::new(galley.size().x + 28.0 + dot, strip.height() - 6.0),
        );
        painter.rect_filled(tab, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, theme::EDITOR);
        painter.line_segment(
            [tab.left_top() + Vec2::new(6.0, 0.0), tab.right_top() - Vec2::new(6.0, 0.0)],
            Stroke::new(2.0, theme::ACCENT),
        );
        let text_pos = egui::Pos2::new(tab.left() + 14.0, tab.center().y - galley.size().y / 2.0);
        let text_width = galley.size().x;
        painter.galley(text_pos, galley, theme::TEXT);
        if self.is_dirty() {
            painter.circle_filled(egui::Pos2::new(text_pos.x + text_width + 12.0, tab.center().y), 3.5, theme::WARNING);
        }
    }

    fn customizer_panel(&mut self, ctx: &egui::Context) {
        if !self.customizer_shown() {
            return;
        }
        let width = ctx.screen_rect().width();
        egui::SidePanel::right("customizer")
            .frame(customizer::frame())
            .resizable(true)
            .default_width(300.0)
            .width_range(220.0..=(width * 0.4).max(240.0))
            .show(ctx, |ui| {
                if self.customizer.show(ui) {
                    self.render_deadline = Some(Instant::now() + PARAMETER_DEBOUNCE);
                }
            });
    }

    fn view_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(Frame::new().fill(theme::BG)).show(ctx, |ui| {
            let has_model = self.model.as_ref().is_some_and(|m| !m.empty);
            let status = viewer::Status {
                rendering: self.busy(),
                failed: self.failed,
                has_model,
                is_2d: self.model.as_ref().is_some_and(|m| m.is_2d),
                empty_hint: viewer::empty_hint(!self.text.trim().is_empty()),
            };
            self.viewer.show(ui, &status);
        });
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm.clone() else { return };
        #[derive(Clone, Copy)]
        enum Choice {
            Save,
            Discard,
            Cancel,
        }
        let mut choice = None;
        let name = self.file_name();
        let modal = egui::Modal::new(Id::new("confirm-unsaved")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(RichText::new("Unsaved changes").heading().color(theme::TEXT));
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("Do you want to save the changes to {name} before {}?", action.verb()))
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let save = Button::new(RichText::new("Save").color(Color32::WHITE).strong()).fill(theme::ACCENT);
                if ui.add(save).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        if choice.is_none() && modal.should_close() {
            choice = Some(Choice::Cancel);
        }
        match choice {
            Some(Choice::Save) => {
                self.confirm = None;
                if self.save() {
                    self.finish(ctx, action);
                }
            }
            Some(Choice::Discard) => {
                self.confirm = None;
                self.finish(ctx, action);
            }
            Some(Choice::Cancel) => self.confirm = None,
            None => {}
        }
    }

    fn finish(&mut self, ctx: &egui::Context, action: Pending) {
        let quit = matches!(action, Pending::Quit);
        self.perform(action);
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(path) = open_events::take().pop() {
            self.guard(Pending::OpenPath(path));
        }
        self.receive();
        self.shortcuts(ctx);
        self.poll_disk(ctx);

        self.toolbar(ctx);
        self.status_bar(ctx);
        self.console_panel(ctx);
        self.editor_panel(ctx);
        self.customizer_panel(ctx);
        self.view_panel(ctx);
        self.confirm_dialog(ctx);

        if let Some(deadline) = self.render_deadline {
            let now = Instant::now();
            if now >= deadline {
                self.render_if_changed();
            } else {
                ctx.request_repaint_after(deadline - now);
            }
        }
        if self.busy() {
            // Keep the spinners turning.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.update_title(ctx);

        let ready = self.finished > 0 && !self.busy();
        if let Some(result) = self.screenshot.as_mut().and_then(|s| s.update(ctx, ready)) {
            match result {
                Ok(message) => self.console.log(Level::Info, message),
                Err(error) => self.console.log(Level::Error, error),
            }
        }
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        if let Some(gl) = gl {
            self.viewer.destroy(gl);
        }
    }
}

fn logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(5), theme::ACCENT);
    let inner = rect.shrink(5.0);
    painter.rect_filled(inner, CornerRadius::same(1), Color32::from_rgb(0xe8, 0xf0, 0xff));
}

fn toolbar_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 20.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, theme::BORDER));
}

fn tool_button(ui: &mut egui::Ui, label: &str, help: &str, shortcut: Option<KeyboardShortcut>) -> egui::Response {
    let response = ui.button(label);
    match shortcut {
        Some(shortcut) => {
            let keys = ui.ctx().format_shortcut(&shortcut);
            response.on_hover_text(format!("{help} ({keys})"))
        }
        None => response.on_hover_text(help),
    }
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.14))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(11.5).color(color)));
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 { format!("{ms:.0} ms") } else { format!("{:.2} s", ms / 1000.0) }
}

/// A volume or area: grouped digits for large values, a few decimals for
/// small ones.
fn format_quantity(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v.abs() >= 1000.0 {
        let tenths = (v.abs() * 10.0).round() as usize;
        format!("{}{}.{}", if v < 0.0 { "-" } else { "" }, group_digits(tenths / 10), tenths % 10)
    } else {
        let text = format!("{v:.3}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        if text.is_empty() || text == "-" { "0".to_string() } else { text.to_string() }
    }
}

/// Where new files go: the current directory, or the home folder when the
/// Finder or the Dock started the app (it then runs in `/`).
fn working_directory() -> Option<PathBuf> {
    match std::env::current_dir() {
        Ok(dir) if dir != Path::new("/") => Some(dir),
        _ => std::env::var_os("HOME").map(PathBuf::from),
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_quantities() {
        assert_eq!(format_quantity(1000.0), "1,000.0");
        assert_eq!(format_quantity(1234567.25), "1,234,567.3");
        assert_eq!(format_quantity(12.5), "12.5");
        assert_eq!(format_quantity(0.0), "0");
        assert_eq!(format_quantity(523.598_775), "523.599");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_millis(42)), "42 ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.50 s");
    }
}
