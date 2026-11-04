//! A circuit's Firmware tab, beside its code: the project's files in the
//! editor, built and flashed from its bar, with the compiler's messages
//! and the pins whose wire went listed under it; the serial monitor, a
//! tab of the console; the Firmware module's page in the settings; and
//! the questions the board asks (an upload of the agent's, the module to
//! install).

use std::time::{Duration, Instant};

use c2f_board::{Severity, Status, UploadProblem};
use eframe::egui::{self, Align, Button, CornerRadius, Frame, Id, Layout, Margin, RichText, Sense, Vec2};

use super::{Section, StudioApp, plural, status_dot};
use crate::console::Level;
pub(super) use crate::firmware::JobKind;
use crate::firmware::{self, Event, Project, format_bytes};
use crate::i18n::tr;
use crate::theme;

/// What the left panel of a circuit shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CircuitTab {
    #[default]
    Circuit,
    Firmware,
}

/// What the console's panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ConsoleTab {
    #[default]
    Console,
    Serial,
}

/// The speeds the monitor offers.
const BAUDS: [u32; 8] = [9_600, 19_200, 38_400, 57_600, 74_880, 115_200, 230_400, 921_600];

/// A message about a line of the project: the compiler's, or a pin whose
/// wire went.
struct Problem {
    file: usize,
    /// From 0.
    line: usize,
    error: bool,
    text: String,
}

/// What the Firmware tab's bar asks for.
enum BarAction {
    Run(JobKind),
    Cancel,
}

impl StudioApp {
    /// The active circuit's firmware project, opened when first asked for,
    /// or again once the circuit is saved elsewhere; `None` without one.
    pub(super) fn firmware_project(&mut self) -> Option<&mut Project> {
        let circuit = self.doc.path.clone().filter(|_| self.doc.is_circuit());
        // Looked for once a second: it reads the project's file.
        let fresh = match (&self.doc.firmware_dir, &circuit) {
            (Some((path, at, dir)), Some(circuit)) if path == circuit && at.elapsed() < firmware::DISK_POLL => Some(dir.clone()),
            _ => None,
        };
        let dir = match fresh {
            Some(dir) => dir,
            None => {
                let dir = circuit.as_deref().and_then(firmware::project_dir);
                self.doc.firmware_dir = circuit.map(|c| (c, Instant::now(), dir.clone()));
                dir
            }
        };
        if self.doc.firmware.as_ref().is_some_and(|p| Some(&p.dir) != dir.as_ref()) {
            // The project the edits were made in keeps them.
            if let Some(mut old) = self.doc.firmware.take() {
                for error in old.save_all() {
                    self.doc.console.log(Level::Error, tr!("Cannot save {error}", "Impossible d'enregistrer {error}"));
                }
            }
        }
        let dir = dir?;
        if self.doc.firmware.is_none() {
            self.doc.firmware = Some(Project::open(dir, self.doc.id));
        }
        self.doc.firmware.as_mut()
    }

    /// The circuit's Firmware tab.
    pub(super) fn firmware_column(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.board.refresh_ports(false);
        if self.firmware_project().is_none() {
            self.no_firmware(ui);
            return;
        }
        let mut action = None;
        let bar = Frame::new().fill(theme::palette().panel).inner_margin(Margin { left: 12, right: 12, top: 6, bottom: 6 });
        egui::TopBottomPanel::top("firmware-bar").frame(bar).show_separator_line(false).show_inside(ui, |ui| {
            action = self.firmware_bar(ui);
        });
        let problems = self.firmware_problems();
        let mut go_to = None;
        if !problems.is_empty() {
            let frame = Frame::new().fill(theme::palette().panel).inner_margin(Margin { left: 12, right: 12, top: 6, bottom: 6 });
            egui::TopBottomPanel::bottom("firmware-problems")
                .frame(frame)
                .resizable(true)
                .default_height(110.0)
                .height_range(60.0..=(ui.available_height() * 0.5).max(80.0))
                .show_inside(ui, |ui| go_to = problems_list(ui, &problems, self.doc.firmware.as_ref()));
        }
        let marks: Vec<(usize, usize, bool)> = problems.iter().map(|p| (p.file, p.line, p.error)).collect();
        egui::CentralPanel::default().frame(Frame::new().fill(theme::palette().editor)).show_inside(ui, |ui| {
            let Some(project) = self.doc.firmware.as_mut() else { return };
            let current = project.current;
            let Some(file) = project.files.get_mut(current) else { return };
            file.editor.indent_width = self.settings.indent_width;
            file.editor.marks = marks.iter().filter(|(f, _, _)| *f == current).map(|&(_, line, error)| (line, error)).collect();
            if file.read_only {
                Frame::new().fill(theme::palette().faint_bg).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let note = tr(
                        "Written from the circuit: saving the circuit writes it again. Read only.",
                        "Tiré du circuit : enregistrer le circuit le réécrit. Lecture seule.",
                    );
                    ui.label(RichText::new(note).size(12.0).color(theme::palette().text_dim));
                });
                let mut copy = file.text.clone();
                file.editor.show(ui, &mut copy);
            } else if file.editor.show(ui, &mut file.text) {
                file.edited();
            }
        });
        if let Some((file, line)) = go_to {
            if let Some(project) = self.doc.firmware.as_mut() {
                project.current = file;
                let file = &mut project.files[file];
                let offset = firmware::line_offset(&file.text, line);
                let text = file.text.clone();
                file.editor.go_to(&ctx, &text, offset);
                file.editor.focus(&ctx);
            }
        }
        match action {
            Some(BarAction::Run(kind)) => self.run_firmware(kind),
            Some(BarAction::Cancel) => self.board.cancel_job(),
            None => {}
        }
    }

    /// A circuit without a project yet: the button that writes it.
    fn no_firmware(&mut self, ui: &mut egui::Ui) {
        Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
            ui.label(RichText::new(tr("No firmware yet", "Pas encore de firmware")).strong().size(15.0).color(theme::palette().text));
            ui.add_space(6.0);
            let about = if self.doc.path.is_none() {
                tr("Save the circuit first: its firmware goes in a firmware folder beside it.", "Enregistrez d'abord le circuit : son firmware va dans un dossier firmware à côté.")
            } else {
                tr(
                    "Code2Form writes a first program from the circuit: a constant per pin wired to the board, each part set up and tried in turn. It is yours to change from then on.",
                    "Code2Form écrit un premier programme à partir du circuit : une constante par broche câblée à la carte, chaque pièce initialisée et essayée tour à tour. Il est à vous ensuite.",
                )
            };
            ui.label(RichText::new(about).color(theme::palette().text_dim));
            ui.add_space(12.0);
            let button = Button::new(RichText::new(tr("Write the firmware", "Générer le firmware")).color(theme::palette().on_accent).strong())
                .fill(theme::palette().accent);
            if ui.add_enabled(self.doc.path.is_some() && self.doc.wiring.is_some(), button).clicked() {
                self.generate_firmware();
            }
        });
    }

    /// The bar over the code: build, flash, the board's port, how it went,
    /// and the project's files.
    fn firmware_bar(&mut self, ui: &mut egui::Ui) -> Option<BarAction> {
        let mut action = None;
        let palette = theme::palette();
        let running = self.board.job.as_ref().map(|j| (j.kind, j.started, j.tab == self.doc.id));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let idle = !self.board.occupied() && self.board.removing.is_none();
            if ui.add_enabled(idle, Button::new(tr("Build", "Compiler"))).on_hover_text(tr("Build the firmware, to find its errors", "Compiler le firmware, pour trouver ses erreurs")).clicked() {
                action = Some(BarAction::Run(JobKind::Build));
            }
            let upload = Button::new(RichText::new(tr("Upload to the board", "Envoyer sur la carte")).color(palette.on_accent).strong()).fill(palette.accent);
            if ui.add_enabled(idle, upload).on_hover_text(tr("Build, then flash it on the board plugged in", "Compiler, puis l'écrire sur la carte branchée")).clicked() {
                action = Some(BarAction::Run(JobKind::Upload));
            }
            self.port_choice(ui, "firmware-port");
            match running {
                Some((kind, started, _)) => {
                    ui.add(egui::Spinner::new().size(12.0).color(palette.accent_strong));
                    let what = match kind {
                        JobKind::Build => tr("Building", "Compilation"),
                        JobKind::Upload => tr("Uploading", "Envoi"),
                    };
                    ui.label(RichText::new(format!("{what} · {} s", started.elapsed().as_secs())).size(12.0).color(palette.accent_strong));
                    if ui.small_button(tr("Stop", "Arrêter")).clicked() {
                        action = Some(BarAction::Cancel);
                    }
                }
                None if self.board.agent_upload.is_some() => {
                    ui.add(egui::Spinner::new().size(12.0).color(palette.accent_strong));
                    ui.label(RichText::new(tr("The agent is uploading", "L'agent envoie le firmware")).size(12.0).color(palette.accent_strong));
                }
                None => self.last_status(ui),
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let Some(project) = self.doc.firmware.as_mut() else { return };
            for (index, file) in project.files.iter().enumerate() {
                let name = if file.read_only { format!("{} 🔒", file.short_name()) } else { file.short_name().to_string() };
                let edited = file.text != file.saved;
                let label = RichText::new(if edited { format!("{name} •") } else { name }).size(12.0);
                if ui.add(Button::selectable(project.current == index, label)).on_hover_text(&file.name).clicked() {
                    project.current = index;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let Some((_, outcome)) = &project.last else { return };
                let usage = |name: &str, u: Option<c2f_board::Usage>| {
                    u.map(|u| format!("{name} {} / {} ({:.1} %)", format_bytes(u.used), format_bytes(u.total), u.used as f64 * 100.0 / u.total.max(1) as f64))
                };
                let sizes: Vec<String> = [usage("RAM", outcome.size.ram), usage("Flash", outcome.size.flash)].into_iter().flatten().collect();
                if !sizes.is_empty() {
                    ui.label(RichText::new(sizes.join(" · ")).size(11.5).color(palette.text_dim));
                }
            });
        });
        action
    }

    /// How the last build or upload went.
    fn last_status(&self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        let Some((kind, outcome)) = self.doc.firmware.as_ref().and_then(|p| p.last.as_ref()) else { return };
        let errors = outcome.errors().count();
        let (color, text) = if outcome.cancelled {
            (palette.text_dim, tr("Stopped", "Arrêté").to_string())
        } else if outcome.success {
            (palette.success, match kind {
                JobKind::Build => tr("Built", "Compilé").to_string(),
                JobKind::Upload => tr("On the board", "Sur la carte").to_string(),
            })
        } else if errors > 0 {
            (palette.error, tr!("{errors} error{}", "{errors} erreur{}", plural(errors)))
        } else if let Some(problem) = outcome.problem {
            (palette.error, problem_text(problem).to_string())
        } else {
            (palette.error, tr("Failed: see the console", "Échec : voir la console").to_string())
        };
        status_dot(ui, color);
        ui.label(RichText::new(text).size(12.0).color(color));
    }

    /// The board's port, to pick among those plugged in.
    fn port_choice(&mut self, ui: &mut egui::Ui, id: &str) {
        let board = &mut self.board;
        let selected = board.port.as_deref().map_or_else(|| tr("No board", "Aucune carte").to_string(), short_port);
        let mut refresh = false;
        egui::ComboBox::from_id_salt(id).selected_text(RichText::new(selected).size(12.0)).width(150.0).show_ui(ui, |ui| {
            if board.ports.is_empty() {
                ui.label(RichText::new(tr("No board plugged in", "Aucune carte branchée")).color(theme::palette().text_dim));
            }
            for port in &board.ports {
                let name = match port.board {
                    Some(b) => format!("{b} · {}", short_port(&port.name)),
                    None => short_port(&port.name),
                };
                ui.selectable_value(&mut board.port, Some(port.name.clone()), name).on_hover_text(&port.description);
            }
            ui.separator();
            refresh = ui.button(tr("Look again", "Chercher à nouveau")).clicked();
        });
        if refresh {
            board.refresh_ports(true);
        }
    }

    /// The compiler's messages about the project's files, then the pins
    /// whose wire went: errors first.
    fn firmware_problems(&self) -> Vec<Problem> {
        let Some(project) = &self.doc.firmware else { return Vec::new() };
        let mut problems = Vec::new();
        if let Some((_, outcome)) = &project.last {
            for d in &outcome.diagnostics {
                let Some(file) = project.index_of(&d.file) else { continue };
                if d.severity == Severity::Note {
                    continue;
                }
                problems.push(Problem { file, line: d.line.saturating_sub(1), error: d.severity == Severity::Error, text: d.message.clone() });
            }
        }
        if let (Some(wiring), Some(pins)) = (&self.doc.wiring, project.file(c2f_circuit::firmware::PINS_H)) {
            for (index, file) in project.files.iter().enumerate().filter(|(_, f)| f.is_code() && !f.read_only) {
                for stale in c2f_circuit::firmware::stale_pins(&wiring.circuit, &pins.text, &file.text) {
                    // The compiler already says it there.
                    let said = |p: &Problem| p.file == index && p.line == stale.line && p.text.contains(&stale.name);
                    if problems.iter().any(said) {
                        continue;
                    }
                    let text = tr!(
                        "{} is not in pins.h: its wire is no longer in the circuit",
                        "{} n'est plus dans pins.h : son fil n'est plus dans le circuit",
                        stale.name
                    );
                    problems.push(Problem { file: index, line: stale.line, error: true, text });
                }
            }
        }
        problems.sort_by_key(|p| (!p.error, p.file, p.line));
        problems
    }

    /// Builds or flashes the active circuit's firmware, its files saved
    /// first; offers to install the module when it is missing.
    pub(super) fn run_firmware(&mut self, kind: JobKind) {
        if !self.board.installed() {
            self.board.offer_install = true;
            return;
        }
        if self.board.occupied() {
            return;
        }
        let Some(project) = self.firmware_project() else { return };
        let errors = project.save_all();
        let dir = project.dir.clone();
        for error in errors {
            self.doc.console.log(Level::Error, error);
        }
        let what = match kind {
            JobKind::Build => tr!("Building {}", "Compilation de {}", dir.display()),
            JobKind::Upload => tr!("Building {} and uploading it to the board", "Compilation de {} et envoi sur la carte", dir.display()),
        };
        self.doc.console.log(Level::Info, what);
        self.console_open = true;
        self.panels.console_tab = ConsoleTab::Console;
        if let Err(error) = self.board.start(kind, self.doc.id, &dir) {
            self.doc.console.log(Level::Error, error);
        }
    }

    /// Takes what the board did in the background to the tabs, saves the
    /// firmware's files whose typing paused and reads those changed on
    /// disk.
    pub(super) fn poll_board(&mut self, ctx: &egui::Context) {
        for event in self.board.poll() {
            match event {
                Event::Line { tab, line } => {
                    let Some(index) = self.tab_index(tab) else { continue };
                    let level = match c2f_board::parse_diagnostic(&line).map(|d| d.severity) {
                        Some(Severity::Error) => Level::Error,
                        Some(Severity::Warning) => Level::Warning,
                        _ => Level::Info,
                    };
                    self.in_tab(index, |app| app.doc.console.log(level, line));
                }
                Event::Done { tab, kind, project, outcome } => {
                    let Some(index) = self.tab_index(tab) else { continue };
                    self.in_tab(index, |app| {
                        let summary = if outcome.success {
                            match kind {
                                JobKind::Build => tr("Firmware built", "Firmware compilé").to_string(),
                                JobKind::Upload => tr("Firmware uploaded to the board", "Firmware envoyé sur la carte").to_string(),
                            }
                        } else if outcome.cancelled {
                            tr("Stopped", "Arrêté").to_string()
                        } else if let Some(problem) = outcome.problem {
                            problem_text(problem).to_string()
                        } else {
                            tr("The firmware did not build: see the errors under the code", "Le firmware ne compile pas : voir les erreurs sous le code").to_string()
                        };
                        let level = if outcome.success || outcome.cancelled { Level::Info } else { Level::Error };
                        app.doc.console.log(level, summary);
                        if let Some(open) = app.doc.firmware.as_mut().filter(|p| p.dir == project) {
                            open.last = Some((kind, outcome));
                        }
                    });
                }
                Event::Installed(Ok(())) => {
                    self.doc.console.log(Level::Info, tr("The Firmware module is installed", "Le module Firmware est installé"));
                }
                Event::Uninstalled(Ok(())) => {
                    self.doc.console.log(Level::Info, tr("The Firmware module is removed", "Le module Firmware est supprimé"));
                }
                Event::Uninstalled(Err(error)) => self.doc.console.log(Level::Error, error),
                Event::Installed(Err(error)) => {
                    self.doc.console.log(Level::Error, tr!("The Firmware module did not install: {error}", "Le module Firmware ne s'est pas installé : {error}"));
                }
            }
        }
        // The tabs out of sight save their files too.
        for index in 0..self.shelf.len() {
            let errors = self.shelf.other_mut(index).and_then(|t| t.firmware.as_mut()).map(|p| p.save_due()).unwrap_or_default();
            if !errors.is_empty() {
                self.in_tab(index, |app| {
                    for error in errors {
                        app.doc.console.log(Level::Error, tr!("Cannot save {error}", "Impossible d'enregistrer {error}"));
                    }
                });
            }
        }
        if let Some(project) = self.doc.firmware.as_mut() {
            let errors = project.save_due();
            project.poll_disk();
            if let Some(at) = project.next_save() {
                ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
            }
            for error in errors {
                self.doc.console.log(Level::Error, tr!("Cannot save {error}", "Impossible d'enregistrer {error}"));
            }
            // The agent's edits show without a frame of their own.
            ctx.request_repaint_after(firmware::DISK_POLL);
        }
        if self.board.busy() || self.board.monitor().is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Writes `include/pins.h` again from the circuit just read, when it
    /// has a project and is saved: the constants follow the wires.
    pub(super) fn follow_pins(&mut self, circuit: &c2f_circuit::Circuit) {
        // The circuit saved, as it was read: not an edit undone since.
        if self.doc.text != self.doc.saved_text || self.doc.sent_source.as_deref() != Some(self.doc.saved_text.as_str()) {
            return;
        }
        let Some(path) = self.doc.path.clone() else { return };
        let Some(dir) = firmware::project_dir(&path) else { return };
        let name = path.file_name().map_or_else(|| "circuit.c2fc".into(), |n| n.to_string_lossy().into_owned());
        // A circuit with an error keeps its pins as they were.
        let Ok(generated) = c2f_circuit::firmware::firmware(circuit, &name) else { return };
        let pins = dir.join(c2f_circuit::firmware::PINS_H);
        if std::fs::read_to_string(&pins).is_ok_and(|text| text == generated.pins_h) {
            return;
        }
        match std::fs::write(&pins, &generated.pins_h) {
            Ok(()) => {
                self.doc.console.log(Level::Info, tr!("{} follows the circuit", "{} suit le circuit", pins.display()));
                if let Some(project) = self.doc.firmware.as_mut() {
                    project.reload(c2f_circuit::firmware::PINS_H);
                }
            }
            Err(error) => self.doc.console.log(Level::Error, tr!("Cannot write {}: {error}", "Impossible d'écrire {} : {error}", pins.display())),
        }
    }

    /// The serial monitor's controls, on the console's title row.
    pub(super) fn serial_controls(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        self.board.refresh_ports(false);
        let monitor = self.board.monitor();
        let (color, hint) = match monitor.as_ref().map(|m| m.status()) {
            None => (palette.text_faint, tr("Closed", "Fermé").to_string()),
            Some(Status::Connecting) => (palette.warning, tr("Opening…", "Ouverture…").to_string()),
            Some(Status::Connected) => (palette.success, tr("Connected", "Connecté").to_string()),
            Some(Status::Disconnected(why)) => (palette.error, tr!("Waiting for the board: {why}", "En attente de la carte : {why}")),
        };
        let (_, dot) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(dot.rect.center(), 4.0, color);
        dot.on_hover_text(hint);
        if monitor.is_none() {
            self.port_choice(ui, "serial-port");
            egui::ComboBox::from_id_salt("serial-baud").selected_text(RichText::new(self.board.baud.to_string()).size(12.0)).width(80.0).show_ui(ui, |ui| {
                for baud in BAUDS {
                    ui.selectable_value(&mut self.board.baud, baud, baud.to_string());
                }
            });
            let open = ui.add_enabled(self.board.port.is_some() && !self.board.occupied(), Button::new(tr("Open", "Ouvrir")));
            if open.clicked() {
                if let Some(port) = self.board.port.clone() {
                    self.board.open_monitor(&port);
                    self.console_open = true;
                }
            }
            return;
        }
        let monitor = monitor.expect("open");
        ui.label(RichText::new(format!("{} · {}", short_port(monitor.port()), monitor.baud())).size(12.0).color(palette.text_dim));
        if ui.button(tr("Close", "Fermer")).clicked() {
            self.board.close_monitor();
        }
        if ui.button(tr("Restart the board", "Redémarrer la carte")).clicked() {
            monitor.reset();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(tr("Clear", "Effacer")).clicked() {
                monitor.clear();
                self.board.paused = None;
            }
            if ui.button(tr("Copy", "Copier")).clicked() {
                let text = monitor.with_lines(|lines| lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"));
                ui.ctx().copy_text(text);
            }
            let paused = self.board.paused.is_some();
            if ui.button(if paused { tr("Resume", "Reprendre") } else { tr("Pause", "Pause") }).clicked() {
                self.board.paused = if paused { None } else { Some(monitor.with_lines(|l| l.iter().cloned().collect())) };
            }
        });
    }

    /// The lines the board printed, and the field to send it one.
    pub(super) fn serial_lines(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        let Some(monitor) = self.board.monitor() else {
            ui.add_space(4.0);
            let hint = tr(
                "Plug the board in, pick its port and open the monitor: what the program prints with Serial shows here.",
                "Branchez la carte, choisissez son port et ouvrez le moniteur : ce que le programme écrit avec Serial s'affiche ici.",
            );
            ui.label(RichText::new(hint).color(palette.text_faint));
            return;
        };
        let font = theme::code_font();
        let mut send = false;
        egui::TopBottomPanel::bottom("serial-input").frame(Frame::new()).show_separator_line(false).show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let field = egui::TextEdit::singleline(&mut self.board.serial_input)
                    .hint_text(tr("A line to send to the board", "Une ligne à envoyer à la carte"))
                    .font(font.clone())
                    .desired_width(ui.available_width() - 80.0);
                let response = ui.add(field);
                send = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                send |= ui.add_enabled(!self.board.serial_input.is_empty(), Button::new(tr("Send", "Envoyer"))).clicked();
                if send {
                    response.request_focus();
                }
            });
        });
        if send && !self.board.serial_input.is_empty() {
            monitor.send(&std::mem::take(&mut self.board.serial_input));
        }
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let show = |ui: &mut egui::Ui, lines: &std::collections::VecDeque<c2f_board::Line>| {
            egui::ScrollArea::both().id_salt("serial-lines").auto_shrink(false).stick_to_bottom(true).show_rows(ui, row_height, lines.len(), |ui, range| {
                for line in lines.range(range) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{:8.3}", line.at.as_secs_f64())).font(font.clone()).color(palette.text_faint));
                        ui.add(egui::Label::new(RichText::new(if line.text.is_empty() { " " } else { &line.text }).font(font.clone()).color(palette.text)).wrap_mode(egui::TextWrapMode::Extend));
                    });
                }
            });
        };
        Frame::new().fill(palette.editor).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            match &self.board.paused {
                Some(lines) => show(ui, &lines.iter().cloned().collect()),
                None => monitor.with_lines(|lines| show(ui, lines)),
            }
        });
    }

    /// The Firmware module's page of the settings.
    pub(super) fn firmware_page(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        super::section_heading(
            ui,
            tr("Firmware module", "Module Firmware"),
            tr(
                "Builds your circuits' firmware and uploads it to the board, from the studio and for the agent. It installs PlatformIO, then each board's toolchain at its first build: about 5 GB for an ESP32.",
                "Compile le firmware de vos circuits et l'envoie sur la carte, depuis le studio et pour l'agent. Il installe PlatformIO, puis la chaîne de chaque carte à sa première compilation : environ 5 Go pour un ESP32.",
            ),
        );
        if let Some(install) = &self.board.install {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(palette.accent_strong));
                ui.label(RichText::new(tr("Installing PlatformIO…", "Installation de PlatformIO…")).color(palette.accent_strong));
                if ui.button(tr("Cancel", "Annuler")).clicked() {
                    install.cancel();
                }
            });
            ui.add_space(6.0);
            let lines = install.lines.lock().unwrap().clone();
            Frame::new().fill(palette.editor).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("install-lines").max_height(160.0).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
                    for line in &lines {
                        ui.label(RichText::new(line).font(theme::code_font()).size(11.0).color(palette.text_dim));
                    }
                });
            });
            return;
        }
        let found = self.board.found.clone();
        match &found.toolchain {
            Some(toolchain) => {
                let version = found.version.as_deref().map(|v| format!("PlatformIO {v}")).unwrap_or_else(|| "PlatformIO".into());
                ui.horizontal(|ui| {
                    status_dot(ui, palette.success);
                    let what = match (&toolchain.module, found.size) {
                        (Some(_), Some(size)) => tr!("Installed · {version} · {}", "Installé · {version} · {}", format_bytes(size)),
                        (Some(_), None) => tr!("Installed · {version}", "Installé · {version}"),
                        (None, _) => tr!("Your own {version} is used: nothing to install", "Votre {version} est utilisé : rien à installer"),
                    };
                    ui.label(RichText::new(what).color(palette.text));
                });
                ui.label(RichText::new(toolchain.pio.display().to_string()).font(theme::code_font()).size(11.0).color(palette.text_faint));
                if toolchain.module.is_some() && self.board.removing.is_some() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(RichText::new(tr("Removing…", "Suppression…")).color(palette.text_dim));
                    });
                } else if toolchain.module.is_some() {
                    ui.add_space(10.0);
                    let busy = self.board.occupied();
                    let button = ui.add_enabled(!busy, Button::new(tr("Uninstall", "Désinstaller")));
                    let button = button.on_hover_text(tr(
                        "Removes PlatformIO and the boards' toolchains it downloaded; your projects stay",
                        "Supprime PlatformIO et les chaînes des cartes téléchargées ; vos projets restent",
                    ));
                    if button.clicked() {
                        self.board.uninstall();
                    }
                }
            }
            None => {
                ui.horizontal(|ui| {
                    status_dot(ui, palette.text_faint);
                    ui.label(RichText::new(tr("Not installed", "Non installé")).color(palette.text));
                });
                ui.add_space(10.0);
                let button = Button::new(RichText::new(tr("Install", "Installer")).color(palette.on_accent).strong()).fill(palette.accent);
                if ui.add(button).clicked() {
                    self.board.start_install();
                }
                ui.label(RichText::new(tr("Needs Python 3, which macOS installs with its developer tools.", "Nécessite Python 3, que macOS installe avec ses outils de développement.")).size(11.5).color(palette.text_faint));
                if let Some(error) = &self.board.install_error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).size(12.0).color(palette.error));
                }
            }
        }
    }

    /// The board's questions: an upload of the agent's to allow, and the
    /// module to install for a build.
    pub(super) fn board_dialogs(&mut self, ctx: &egui::Context) {
        let palette = theme::palette();
        if let Some(asking) = &self.board.asking {
            let mut answer = None;
            let modal = egui::Modal::new(Id::new("agent-upload")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(RichText::new(tr("The agent wants to upload a firmware", "L'agent veut envoyer un firmware")).heading().color(palette.text));
                ui.add_space(6.0);
                let what = tr!(
                    "It builds {} and writes it onto the board at {}, which then runs it.",
                    "Il compile {} et l'écrit sur la carte de {}, qui l'exécute ensuite.",
                    asking.project.display(),
                    short_port(&asking.port)
                );
                ui.label(RichText::new(what).color(palette.text_dim));
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let yes = Button::new(RichText::new(tr("Upload", "Envoyer")).color(palette.on_accent).strong()).fill(palette.accent);
                    if ui.add(yes).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(tr("Decline", "Refuser")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer.is_none() && modal.should_close() {
                answer = Some(false);
            }
            if let Some(yes) = answer {
                self.board.answer(yes);
            }
        }
        if self.board.offer_install {
            let mut close = false;
            let modal = egui::Modal::new(Id::new("offer-install")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(RichText::new(tr("The Firmware module is not installed", "Le module Firmware n'est pas installé")).heading().color(palette.text));
                ui.add_space(6.0);
                ui.label(RichText::new(tr(
                    "Building and uploading need it: it installs PlatformIO, then the board's toolchain at the first build.",
                    "Compiler et envoyer en ont besoin : il installe PlatformIO, puis la chaîne de la carte à la première compilation.",
                )).color(palette.text_dim));
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = Button::new(RichText::new(tr("Install…", "Installer…")).color(palette.on_accent).strong()).fill(palette.accent);
                    if ui.add(open).clicked() {
                        self.settings_open = true;
                        self.settings_section = Section::Firmware;
                        close = true;
                    }
                    if ui.button(tr("Cancel", "Annuler")).clicked() {
                        close = true;
                    }
                });
            });
            if close || modal.should_close() {
                self.board.offer_install = false;
            }
        }
    }
}

/// The problems under the code; the one clicked, as its file and line.
fn problems_list(ui: &mut egui::Ui, problems: &[Problem], project: Option<&Project>) -> Option<(usize, usize)> {
    let palette = theme::palette();
    let mut clicked = None;
    let errors = problems.iter().filter(|p| p.error).count();
    let warnings = problems.len() - errors;
    let mut title = Vec::new();
    if errors > 0 {
        title.push(tr!("{errors} error{}", "{errors} erreur{}", plural(errors)));
    }
    if warnings > 0 {
        title.push(tr!("{warnings} warning{}", "{warnings} avertissement{}", plural(warnings)));
    }
    ui.label(RichText::new(title.join(" · ")).strong().size(12.5).color(palette.text));
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("firmware-problems-list").auto_shrink(false).show(ui, |ui| {
        for (i, problem) in problems.iter().enumerate() {
            let name = project.and_then(|p| p.files.get(problem.file)).map_or("", |f| f.short_name());
            let color = if problem.error { palette.error } else { palette.warning };
            let text = RichText::new(format!("{name}:{}  {}", problem.line + 1, problem.text)).size(12.0).color(color);
            let row = ui.add(egui::Label::new(text).truncate().sense(Sense::click())).on_hover_text(&problem.text);
            if row.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked.map(|i| (problems[i].file, problems[i].line))
}

/// What to do when an upload failed for a known reason.
fn problem_text(problem: UploadProblem) -> &'static str {
    match problem {
        UploadProblem::NoBoard => tr(
            "No board found: plug it in with a USB cable that carries data",
            "Aucune carte trouvée : branchez-la avec un câble USB qui transmet les données",
        ),
        UploadProblem::PortBusy => tr(
            "The port is taken by another program: close it, then try again",
            "Le port est pris par un autre programme : fermez-le, puis réessayez",
        ),
        UploadProblem::NoAnswer => tr(
            "The board does not answer: hold BOOT, press RESET, release BOOT, then try again",
            "La carte ne répond pas : maintenez BOOT, appuyez sur RESET, relâchez BOOT, puis réessayez",
        ),
    }
}

/// A port's name without its folder: `cu.usbmodem1101`.
fn short_port(name: &str) -> String {
    name.strip_prefix("/dev/").unwrap_or(name).to_string()
}
