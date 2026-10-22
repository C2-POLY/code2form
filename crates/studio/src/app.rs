//! The studio window: toolbar, editor, 3D view, console and status bar,
//! and the document they work on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, Frame, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText,
    Sense, Stroke, Vec2,
};
use eframe::glow;
use c2f_circuit::End;
use c2f_eval::customizer::Literal;
use c2f_geom::Stats;

use crate::catalogue::{self, Catalogue};
use crate::circuit;
use crate::console::{Console, Level};
use crate::customizer::{self, Customizer};
use crate::editor::{Editor, FindRequest};
use crate::open_events;
use crate::pause_edit;
use crate::recent::RecentFiles;
use crate::rewire::{self, Edit, Side};
use crate::screenshot::Screenshot;
use crate::theme;
use crate::viewer::{self, PauseEdit, Viewer};
use crate::inspect;
use crate::mount::{self, Mount};
use crate::pick::{self, Hit};
use crate::prefs::WireMargin;
use crate::wiring::{self, Emphasis, Look, Wiring};
use crate::worker::{Format, Reply, Worker, group_digits};

/// File dialog filters: a name and an extension.
const DESIGN_FILES: (&str, &str) = ("Code2Form design", "c2f");
const SCAD_FILES: (&str, &str) = ("SCAD design", "scad");
const CIRCUIT_FILES: (&str, &str) = ("Code2Form circuit", "c2fc");

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

/// A drag in the view that rewires the circuit.
struct Rewire {
    /// The wire end grabbed; `None` for a new wire from `anchor`.
    grabbed: Option<(usize, Side)>,
    /// The pin the wire keeps: its other end, or where a new one starts.
    anchor: End,
    /// Where the press was, to tell a drag from a click.
    origin: egui::Pos2,
    moved: bool,
    /// The pin under the pointer, and the check error dropping there
    /// would bring.
    target: Option<End>,
    error: Option<String>,
}

impl Rewire {
    /// What dropping now does.
    fn edit(&self) -> Option<Edit> {
        match (self.grabbed, self.target) {
            (Some((wire, side)), Some(to)) => Some(Edit::Move { wire, side, to }),
            (Some((wire, _)), None) => Some(Edit::Remove { wire }),
            (None, Some(to)) => Some(Edit::Add { from: self.anchor, to }),
            (None, None) => None,
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
    /// The parts of a circuit; 0 for a design.
    parts: usize,
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
    /// The wires of the circuit on screen, when the file is one, as last
    /// read.
    wiring: Option<Wiring>,
    /// What is under the pointer in the view, and what was clicked.
    hover: Option<Hit>,
    selection: Option<Hit>,
    /// The box around each part of the circuit, as last rendered.
    part_boxes: Vec<Option<(glam::Vec3, glam::Vec3)>>,
    /// How the parts were last shown, not to recolor them every frame.
    shown_parts: Vec<Emphasis>,
    /// The assembly mode, and its progress.
    mount: Mount,
    /// Added to each wire's routed length.
    wire_margin: WireMargin,
    /// When the app started, for the pins that pulse.
    started: Instant,
    /// Where the 3D view is, as last shown.
    view_rect: egui::Rect,
    /// A drag rewiring the circuit, from a pin or a wire's end.
    rewire: Option<Rewire>,
    /// The generation it was read for.
    circuit_generation: u64,
    /// The circuit's part files and their modification times, to render
    /// again when one changes on disk.
    part_files: Vec<(PathBuf, Option<SystemTime>)>,

    editor: Editor,
    viewer: Viewer,
    console: Console,
    console_open: bool,
    customizer: Customizer,
    /// Shown or hidden by the user; by default shown when the file has
    /// parameters.
    customizer_open: Option<bool>,
    /// The parts catalogue, shown in its panel for a circuit.
    catalogue: Catalogue,
    catalogue_open: bool,
    recent: RecentFiles,
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
            wiring: None,
            hover: None,
            selection: None,
            part_boxes: Vec::new(),
            shown_parts: Vec::new(),
            mount: Mount::default(),
            wire_margin: WireMargin::load(crate::prefs::wire_margin_store()),
            started: Instant::now(),
            view_rect: egui::Rect::NOTHING,
            rewire: None,
            circuit_generation: 0,
            part_files: Vec::new(),
            editor: Editor::new(),
            viewer: Viewer::new(cc.gl.as_ref()),
            console: Console::default(),
            console_open: true,
            customizer: Customizer::default(),
            customizer_open: None,
            catalogue: Catalogue::new(&cc.egui_ctx),
            catalogue_open: false,
            recent: RecentFiles::load(crate::recent::store()),
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

    fn is_circuit(&self) -> bool {
        self.path.as_deref().is_some_and(circuit::is_circuit)
    }

    fn busy(&self) -> bool {
        self.worker.latest() > self.finished
    }

    // ---- Rendering ----

    fn render_now(&mut self) {
        self.render_deadline = None;
        self.editor.set_circuit(self.is_circuit());
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
                            self.editor.error_line = outcome.annotation_error;
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
                                parts: scene.parts.len(),
                            });
                            if !self.is_circuit() && current {
                                // A design's @circuit and its parts.
                                self.watch(outcome.watch.clone());
                            }
                            self.part_boxes = part_boxes(&scene);
                            self.viewer.set_scene(scene, frame);
                            // The new solid comes in its own colors.
                            self.update_overlay();
                        }
                        Err(line) => {
                            self.failed = true;
                            self.editor.error_line = line.or(outcome.annotation_error);
                        }
                    }
                }
                Reply::Circuit { generation, circuit } => {
                    if generation < self.circuit_generation || generation < self.worker.latest() {
                        continue;
                    }
                    self.circuit_generation = generation;
                    self.set_circuit(Some(circuit));
                    if let Some(target) = self.screenshot.as_ref().and_then(|s| s.select.clone()) {
                        let rewire = target.strip_prefix("rewire:").and_then(|t| t.split_once('>'));
                        if target == "catalogue" {
                            self.catalogue_open = true;
                        } else if let Some((from, to)) = rewire {
                            // A new wire held over a pin, as a drag shows it.
                            let end = |name: &str| match self.wiring.as_ref().and_then(|w| find_hit(&w.circuit, name)) {
                                Some(Hit::Pin(end)) => Some(end),
                                _ => None,
                            };
                            if let (Some(anchor), Some(to), Some(wiring)) = (end(from), end(to), &self.wiring) {
                                let error = rewire::new_error(&wiring.circuit, Edit::Add { from: anchor, to });
                                let origin = egui::Pos2::ZERO;
                                let held = Rewire { grabbed: None, anchor, origin, moved: true, target: Some(to), error };
                                self.rewire = Some(held);
                                self.update_overlay();
                            }
                        } else if let Some(step) = target.strip_prefix("assembly:").and_then(|n| n.parse::<usize>().ok()) {
                            // Set before turning the mode on: a screenshot
                            // leaves the saved progress as it is.
                            self.mount.step = step.saturating_sub(1);
                            self.set_mount(true);
                        } else {
                            self.selection = self.wiring.as_ref().and_then(|w| find_hit(&w.circuit, &target));
                            self.update_overlay();
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

    /// Shows a circuit just read, or none, and watches its part files.
    fn set_circuit(&mut self, found: Option<Arc<c2f_circuit::Circuit>>) {
        // The parts' files and the enclosure's: an edit to one renders again.
        let files: Vec<PathBuf> = found
            .iter()
            .flat_map(|c| c.parts.iter().map(|p| p.path.clone()).chain(c.enclosure.clone()))
            .collect();
        self.watch(files);
        self.wiring = found.map(Wiring::new);
        // The boxes are the last render's, of the circuit before: the
        // render of this one brings its own.
        self.part_boxes.clear();
        // A selection outlives an edit when what it names is still there.
        let valid = |hit: Option<Hit>| match (hit, &self.wiring) {
            (Some(Hit::Pin(end)), Some(w)) => {
                w.circuit.parts.get(end.part).is_some_and(|p| end.pin < p.annotations.pins.len())
            }
            (Some(Hit::Wire(i)), Some(w)) => i < w.circuit.wires.len(),
            (Some(Hit::Part(i)), Some(w)) => i < w.circuit.parts.len(),
            _ => false,
        };
        if !valid(self.selection) {
            self.selection = None;
        }
        self.hover = self.hover.filter(|&h| valid(Some(h)));
        self.update_overlay();
    }

    /// Watches `files` for changes on disk, besides the open file.
    fn watch(&mut self, mut files: Vec<PathBuf>) {
        files.sort();
        files.dedup();
        let known = std::mem::take(&mut self.part_files);
        self.part_files = files
            .into_iter()
            .map(|file| {
                // A file already watched keeps its time, so that a change
                // made since it was read is not missed.
                let mtime = known.iter().find(|(f, _)| *f == file).map_or_else(|| modified(&file), |(_, m)| *m);
                (file, mtime)
            })
            .collect();
    }

    /// How the wires, pins and parts show: what a selection picks out
    /// stands out, the rest is greyed out.
    fn look(&self) -> Look {
        if let Some(look) = self.mount_look() {
            return look;
        }
        if let (Some(wiring), Some(rewire)) = (&self.wiring, &self.rewire) {
            // The wire being moved fades; the pins at stake stand out.
            let wires = (0..wiring.circuit.wires.len())
                .map(|i| if rewire.grabbed.is_some_and(|(w, _)| w == i) { Emphasis::Dim } else { Emphasis::Normal })
                .collect();
            let strong_pins = std::iter::once(rewire.anchor).chain(rewire.target).collect();
            let error_pins = rewire.target.filter(|_| rewire.error.is_some()).into_iter().collect();
            return Look { wires, strong_pins, error_pins, ..Look::default() };
        }
        let (Some(wiring), Some(selection)) = (&self.wiring, self.selection) else { return Look::default() };
        let circuit = &wiring.circuit;
        let pick_out = |on: bool| if on { Emphasis::Strong } else { Emphasis::Dim };
        let shown = |on: bool| if on { Emphasis::Normal } else { Emphasis::Dim };
        let (wires, strong_pins, parts): (Vec<Emphasis>, _, Vec<Emphasis>) = match selection {
            Hit::Pin(end) => {
                let (pins, wires) = pick::net(circuit, end);
                let parts = (0..circuit.parts.len()).map(|p| shown(pins.iter().any(|e| e.part == p))).collect();
                ((0..circuit.wires.len()).map(|i| pick_out(wires.contains(&i))).collect(), pins, parts)
            }
            Hit::Wire(index) => {
                let wire = &circuit.wires[index];
                let parts = (0..circuit.parts.len()).map(|p| shown(p == wire.from.part || p == wire.to.part)).collect();
                ((0..circuit.wires.len()).map(|i| pick_out(i == index)).collect(), [wire.from, wire.to].into(), parts)
            }
            Hit::Part(part) => {
                let touches = |i: usize| circuit.wires[i].from.part == part || circuit.wires[i].to.part == part;
                let pins = circuit.wires.iter().flat_map(|w| [w.from, w.to]).filter(|e| e.part == part).collect();
                let parts = (0..circuit.parts.len()).map(|p| pick_out(p == part)).collect();
                ((0..circuit.wires.len()).map(|i| shown(touches(i))).collect(), pins, parts)
            }
        };
        Look { wires, strong_pins, dim_pins: true, pulse: None, parts, error_pins: Default::default() }
    }

    /// Sends the wires and pins, as they now look, to the view.
    fn update_overlay(&mut self) {
        let look = self.look();
        match &self.wiring {
            Some(wiring) => {
                let (wires, pins) = wiring.meshes(&look);
                self.viewer.set_overlay(wires, pins, wiring.bounds());
            }
            None => self.viewer.set_overlay(Default::default(), Default::default(), None),
        }
        if look.parts != self.shown_parts {
            self.viewer.set_part_emphasis(&look.parts);
            self.shown_parts = look.parts;
        }
    }

    // ---- Assembly ----

    /// The wires of the circuit, by name, when there are any and the
    /// assembly's progress is this circuit's.
    fn mount_names(&self) -> Option<Vec<String>> {
        self.mount_wiring().map(|w| w.names.clone())
    }

    /// The circuit's wiring, when the assembly's progress is its own.
    fn mount_wiring(&self) -> Option<&Wiring> {
        self.path.as_deref().filter(|p| self.mount.is_for(p))?;
        self.wiring.as_ref().filter(|w| !w.names.is_empty())
    }

    /// The assembly mode's look: the wire on screen picked out, its pins
    /// pulsing, the wires done as they are, the others greyed out.
    fn mount_look(&self) -> Option<Look> {
        if !self.mount.active {
            return None;
        }
        let wiring = self.mount_wiring()?;
        let names = &wiring.names;
        let circuit = &wiring.circuit;
        let step = self.mount.step.min(names.len() - 1);
        let current = &circuit.wires[step];
        let wires = (0..names.len())
            .map(|i| match i {
                _ if i == step => Emphasis::Strong,
                _ if self.mount.is_done(names, i) => Emphasis::Normal,
                _ => Emphasis::Dim,
            })
            .collect();
        let parts = (0..circuit.parts.len())
            .map(|p| if p == current.from.part || p == current.to.part { Emphasis::Normal } else { Emphasis::Dim })
            .collect();
        // Two pulses a second.
        let time = self.started.elapsed().as_secs_f32();
        let pulse = 0.5 + 0.5 * (time * std::f32::consts::TAU * 2.0).sin();
        let strong_pins = [current.from, current.to].into();
        Some(Look { wires, strong_pins, dim_pins: true, pulse: Some(pulse), parts, error_pins: Default::default() })
    }

    /// Turns the assembly mode on or off.
    fn set_mount(&mut self, active: bool) {
        self.mount.active = active && self.mount_names().is_some();
        if self.mount.active {
            self.selection = None;
            self.go_to_wire(self.mount.step);
        }
        self.update_overlay();
    }

    /// Shows wire `step` of the assembly, the camera turning to its pins.
    fn go_to_wire(&mut self, step: usize) {
        let Some(count) = self.mount_names().map(|n| n.len()) else { return };
        self.mount.go(step, count);
        self.report_mount_problem();
        let Some(wiring) = &self.wiring else { return };
        let wire = &wiring.circuit.wires[self.mount.step];
        let place = |end| glam::Vec3::from(wiring.circuit.place(end).position.map(|c| c as f32));
        let goal = mount::framing(place(wire.from), place(wire.to), &self.viewer.camera, self.viewer.aspect());
        self.viewer.fly_to(goal);
        self.update_overlay();
    }

    fn mount_next(&mut self) {
        if let Some(names) = self.mount_names() {
            self.mount.next(&names);
            self.go_to_wire(self.mount.step);
        }
    }

    fn mount_previous(&mut self) {
        if let Some(names) = self.mount_names() {
            self.mount.previous(names.len());
            self.go_to_wire(self.mount.step);
        }
    }

    fn report_mount_problem(&mut self) {
        if let Some(problem) = self.mount.problem.take() {
            self.console.log(Level::Warning, problem);
        }
    }

    /// The assembly bar, over the top of the view: the instruction, the
    /// wire's place in the order, and the buttons that step through.
    fn mount_bar(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let (Some(wiring), Some(names)) = (&self.wiring, self.mount_names()) else { return };
        let step = self.mount.step.min(names.len() - 1);
        let instruction = mount::instruction(&wiring.circuit, step);
        let label = wiring.circuit.wires[step].label.clone();
        let mut done = self.mount.is_done(&names, step);
        let done_count = self.mount.done_count(&names);
        let (mut previous, mut next, mut restart, mut toggled) = (false, false, false, false);
        egui::Area::new(Id::new("assembly-bar"))
            .fixed_pos(egui::pos2(rect.left() + 12.0, rect.top() + 12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_black_alpha(200))
                    .stroke(Stroke::new(1.0, Color32::from_white_alpha(30)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(rect.width() - 48.0);
                        ui.horizontal(|ui| {
                            toggled = ui.checkbox(&mut done, "").on_hover_text("Done").changed();
                            ui.label(RichText::new(&instruction).strong().size(14.0).color(wiring::TEXT));
                            if let Some(label) = &label {
                                ui.label(RichText::new(format!("· {label}")).size(13.0).color(wiring::TEXT_DIM));
                            }
                        });
                        ui.horizontal(|ui| {
                            let count = names.len();
                            ui.label(RichText::new(format!("{} / {count}", step + 1)).strong().color(wiring::TEXT));
                            ui.label(RichText::new(format!("· {done_count} done")).color(wiring::TEXT_DIM));
                            ui.add_space(8.0);
                            previous = ui
                                .add_enabled(step > 0, Button::new("◀ Previous"))
                                .on_hover_text("The previous wire (Left arrow)")
                                .clicked();
                            next = ui
                                .button("Next ▶")
                                .on_hover_text("Check this wire off and go to the next (Right arrow)")
                                .clicked();
                            restart = ui
                                .button("Start over")
                                .on_hover_text("Uncheck every wire and go back to the first")
                                .clicked();
                        });
                    });
            });
        if toggled {
            self.mount.set_done(&names, step, done);
            self.report_mount_problem();
            self.update_overlay();
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        let (left, right) = ctx.input(|i| (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight)));
        if next || (right && !typing) {
            self.mount_next();
        } else if previous || (left && !typing) {
            self.mount_previous();
        } else if restart {
            self.mount.restart();
            self.report_mount_problem();
            self.go_to_wire(0);
        }
    }

    /// Selects something of the circuit, or nothing; a wire also puts the
    /// editor on its line.
    fn select(&mut self, ctx: &egui::Context, hit: Option<Hit>) {
        if hit == self.selection {
            return;
        }
        self.selection = hit;
        if let (Some(Hit::Wire(index)), Some(wiring)) = (hit, &self.wiring) {
            let offset = wiring.circuit.wires[index].span.start as usize;
            self.editor.go_to(ctx, &self.text, offset);
        }
        self.update_overlay();
    }

    // ---- Catalogue ----

    fn catalogue_panel(&mut self, ctx: &egui::Context) {
        if !self.is_circuit() || !self.catalogue_open {
            return;
        }
        let width = ctx.screen_rect().width();
        let action = egui::SidePanel::right("catalogue")
            .frame(customizer::frame())
            .resizable(true)
            .default_width(300.0)
            .width_range(240.0..=(width * 0.4).max(260.0))
            .show(ctx, |ui| self.catalogue.show(ui))
            .inner;
        match action {
            Some(catalogue::Action::Add(entry)) => {
                let at = catalogue::free_spot(&self.part_boxes);
                self.add_part(ctx, &entry, at);
            }
            Some(catalogue::Action::Created(path)) => {
                self.console.log(Level::Info, format!("Made the part {}", path.display()));
                self.guard(Pending::OpenPath(path));
            }
            None => {}
        }
    }

    /// Writes a `[parts.NAME]` table for `entry` at `at` into the circuit,
    /// as one step of the editor's undo history, and renders it.
    fn add_part(&mut self, ctx: &egui::Context, entry: &catalogue::Entry, at: [f64; 2]) {
        if !self.is_circuit() {
            return;
        }
        let name = catalogue::part_name(&entry.name, &catalogue::part_names(&self.text));
        let (text, cursor) = catalogue::insert_part(&self.text, &name, &entry.rel, at);
        self.editor.replace_text(ctx, &mut self.text, text, cursor);
        self.console.log(Level::Info, format!("Added the part `{name}` ({})", entry.rel));
        self.render_now();
    }

    // ---- Rewiring ----

    /// A press in the view on a pin, or on a wire near one of its ends,
    /// starts rewiring: before the view handles the press, so that it does
    /// not turn the camera. Not while assembling.
    fn start_rewire(&mut self, ctx: &egui::Context) {
        let (pressed, origin, shift) =
            ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.press_origin(), i.modifiers.shift));
        let (Some(wiring), Some(origin)) = (&self.wiring, origin.filter(|_| pressed)) else { return };
        let slider = self.viewer.layer_view && origin.x > self.view_rect.right() - 60.0;
        // Not through a menu, a popup or the assembly bar.
        let covered = ctx.layer_id_at(origin).is_some_and(|layer| layer.order != egui::Order::Background);
        if shift || self.mount.active || !self.view_rect.contains(origin) || slider || covered {
            return;
        }
        let ray = self.viewer.ray(self.view_rect, origin);
        let grabbed = match pick::pick(wiring, &self.part_boxes, &ray) {
            Some(Hit::Pin(end)) => Some((None, end)),
            Some(Hit::Wire(index)) => pick::wire_side(wiring, index, &ray).map(|side| {
                let wire = &wiring.circuit.wires[index];
                (Some((index, side)), if side == Side::From { wire.to } else { wire.from })
            }),
            _ => None,
        };
        if let Some((grabbed, anchor)) = grabbed {
            self.rewire = Some(Rewire { grabbed, anchor, origin, moved: false, target: None, error: None });
        }
    }

    /// Follows a rewiring drag: the pin under the pointer and what a drop
    /// there would break; on release, the edit. Returns false when the
    /// press turned out to be a click, for the view to handle.
    fn follow_rewire(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        let (Some(wiring), Some(rewire)) = (&self.wiring, &mut self.rewire) else {
            self.rewire = None;
            return false;
        };
        let (pointer, released) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_released()));
        let mut changed = false;
        if let Some(pos) = pointer {
            rewire.moved |= pos.distance(rewire.origin) > 4.0;
            let ray = self.viewer.ray(response.rect, pos);
            let target = match pick::pick(wiring, &[], &ray) {
                Some(Hit::Pin(end)) if end != rewire.anchor => Some(end),
                _ => None,
            };
            if target != rewire.target {
                rewire.target = target;
                rewire.error = rewire.edit().and_then(|edit| rewire::new_error(&wiring.circuit, edit));
                changed = true;
            }
        }
        if !released {
            if changed {
                self.update_overlay();
            }
            ctx.request_repaint();
            return true;
        }
        let Some(rewire) = self.rewire.take() else { return false };
        self.update_overlay();
        // A press and release in place is a click.
        if !rewire.moved {
            return false;
        }
        // Back onto the pin it came from: nothing to do.
        let unchanged = match (&self.wiring, rewire.grabbed) {
            (Some(wiring), Some((wire, side))) => {
                let w = &wiring.circuit.wires[wire];
                rewire.target == Some(if side == Side::From { w.from } else { w.to })
            }
            _ => false,
        };
        if let Some(edit) = rewire.edit().filter(|_| !unchanged) {
            self.apply_rewire(ctx, edit);
        }
        true
    }

    /// The rubber band from the pin kept to the pointer, and what a drop
    /// does, in red when it would break a check.
    fn paint_rewire(&self, painter: &egui::Painter, rect: egui::Rect, pointer: Option<egui::Pos2>) {
        let (Some(wiring), Some(rewire)) = (&self.wiring, &self.rewire) else { return };
        if !rewire.moved {
            return;
        }
        let circuit = &wiring.circuit;
        let place = |end: End| glam::Vec3::from(circuit.place(end).position.map(|c| c as f32));
        // Without a pointer (a scripted screenshot), at the pin targeted.
        let held = rewire.target.and_then(|t| self.viewer.to_screen(rect, place(t)));
        let Some(pointer) = pointer.or(held) else { return };
        let color = if rewire.error.is_some() { theme::ERROR } else { theme::ACCENT_STRONG };
        if let Some(anchor) = self.viewer.to_screen(rect, place(rewire.anchor)) {
            let to = rewire.target.and_then(|t| self.viewer.to_screen(rect, place(t))).unwrap_or(pointer);
            painter.line_segment([anchor, to], Stroke::new(2.0, color));
        }
        let text = match (&rewire.error, rewire.grabbed, rewire.target) {
            (Some(error), _, _) => format!("{error} (drop anyway to wire it)"),
            (None, Some(_), Some(to)) => format!("Move this end to {}", circuit.end_name(to)),
            (None, Some(_), None) => "Drop away from a pin to remove this wire".to_string(),
            (None, None, Some(to)) => {
                format!("New wire: {} ➡ {}", circuit.end_name(rewire.anchor), circuit.end_name(to))
            }
            (None, None, None) => "Drop on a pin to wire it".to_string(),
        };
        let color = if rewire.error.is_some() { theme::ERROR } else { wiring::TEXT };
        inspect::paint_bubble_in(painter, rect, pointer, &text, color);
    }

    /// Writes a rewiring into the circuit's text, as one step of the
    /// editor's undo history, and renders it.
    fn apply_rewire(&mut self, ctx: &egui::Context, edit: Edit) {
        let Some(wiring) = &self.wiring else { return };
        let circuit = &wiring.circuit;
        let done = match edit {
            Edit::Move { to, .. } => format!("Moved a wire's end to {}", circuit.end_name(to)),
            Edit::Add { from, to } => format!("Added a wire: {} ➡ {}", circuit.end_name(from), circuit.end_name(to)),
            Edit::Remove { wire } => {
                let w = &circuit.wires[wire];
                format!("Removed the wire {} ➡ {}", circuit.end_name(w.from), circuit.end_name(w.to))
            }
        };
        let fresh = self.sent_source.as_deref() == Some(self.text.as_str());
        let result = if fresh {
            rewire::apply(&self.text, circuit, edit)
        } else {
            Err("the code changed since the last render: try again once it is rendered".to_string())
        };
        match result {
            Ok((text, cursor)) => {
                if matches!(edit, Edit::Remove { .. }) {
                    self.selection = None;
                }
                self.editor.replace_text(ctx, &mut self.text, text, cursor);
                self.console.log(Level::Info, done);
                self.render_now();
            }
            Err(why) => self.console.log(Level::Warning, format!("Cannot rewire: {why}")),
        }
    }

    /// Hovering, clicks and Escape over the view of a circuit.
    fn circuit_input(&mut self, ctx: &egui::Context, response: &egui::Response) {
        if self.rewire.is_some() && self.follow_rewire(ctx, response) {
            self.hover = None;
            return;
        }
        // Delete or Backspace removes the wire selected.
        let delete = ctx.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace));
        if delete && !self.mount.active && ctx.memory(|m| m.focused().is_none()) {
            if let Some(Hit::Wire(wire)) = self.selection {
                self.apply_rewire(ctx, Edit::Remove { wire });
            }
        }
        let Some(wiring) = &self.wiring else { return };
        if !response.dragged() {
            self.hover = response.hover_pos().and_then(|pos| {
                pick::pick(wiring, &self.part_boxes, &self.viewer.ray(response.rect, pos))
            });
        }
        if response.double_clicked() {
            if let Some(Hit::Part(index)) = self.hover {
                let path = wiring.circuit.parts[index].path.clone();
                self.guard(Pending::OpenPath(path));
                return;
            }
        }
        if response.clicked() {
            if self.mount.active {
                // Assembling, a click on a wire goes to its step.
                if let Some(Hit::Wire(index)) = self.hover {
                    self.go_to_wire(index);
                }
            } else {
                self.select(ctx, self.hover);
            }
        }
        let escape = ctx.input(|i| i.key_pressed(Key::Escape)) && ctx.memory(|m| m.focused().is_none());
        if escape && self.selection.is_some() {
            self.select(ctx, None);
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
                self.remember(&path);
                // A circuit has no parameters, nor presets next to it.
                self.customizer.open(Some(path.as_path()).filter(|p| !circuit::is_circuit(p)));
                self.mount.open(Some(path.as_path()).filter(|p| circuit::is_circuit(p)));
                self.report_mount_problem();
                // Nothing of the file before carries over, even when this
                // one does not read.
                self.set_circuit(None);
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
        self.mount.open(None);
        self.set_circuit(None);
        self.console.log(Level::Info, "New file");
        self.render_now();
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open a design")
            .add_filter("Code2Form, SCAD or circuit", &["c2f", "scad", "c2fc"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.directory() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.load(path);
        }
    }

    /// Opens a file from Open Recent, or drops it from the list when it is
    /// gone.
    fn open_recent(&mut self, path: PathBuf) {
        if path.exists() {
            self.guard(Pending::OpenPath(path));
            return;
        }
        self.console.log(Level::Warning, format!("{} no longer exists; removed from Open Recent", path.display()));
        self.recent.remove(&path);
        self.save_recent();
    }

    fn remember(&mut self, path: &Path) {
        self.recent.push(path);
        self.save_recent();
    }

    fn save_recent(&mut self) {
        if let Err(err) = self.recent.save() {
            self.console.log(Level::Warning, format!("Cannot save the recent files list: {err}"));
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
        let filters = if self.is_circuit() {
            [CIRCUIT_FILES, DESIGN_FILES, SCAD_FILES]
        } else if scad {
            [SCAD_FILES, DESIGN_FILES, CIRCUIT_FILES]
        } else {
            [DESIGN_FILES, SCAD_FILES, CIRCUIT_FILES]
        };
        let mut dialog = rfd::FileDialog::new().set_title("Save as");
        for (name, extension) in filters {
            dialog = dialog.add_filter(name, &[extension]);
        }
        dialog = dialog.set_file_name(if self.path.is_some() { self.file_name() } else { "untitled.c2f".to_string() });
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
                self.remember(&path);
                if circuit::is_circuit(&path) {
                    self.customizer.open(None);
                    if self.path.as_deref() != Some(path.as_path()) {
                        self.mount.moved(&path);
                        self.report_mount_problem();
                    }
                } else {
                    self.mount.open(None);
                    self.customizer.moved(&path);
                    self.set_circuit(None);
                }
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

    /// Reloads the file when it changed on disk and has no unsaved edits,
    /// and renders a circuit again when one of its part files changed.
    fn poll_disk(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else { return };
        ctx.request_repaint_after(POLL);
        if self.last_poll.elapsed() < POLL {
            return;
        }
        self.last_poll = Instant::now();
        let mut changed_parts = Vec::new();
        for (file, mtime) in &mut self.part_files {
            let now = modified(file);
            if now != *mtime {
                *mtime = now;
                changed_parts.push(file.clone());
            }
        }
        if !changed_parts.is_empty() {
            for file in &changed_parts {
                self.console.log(Level::Info, format!("Rendering again: {} changed on disk", file.display()));
            }
            self.render_now();
        }
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
                self.recent_menu(ui);
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
                    if self.is_circuit() {
                        let button = egui::Button::selectable(self.mount.active, "Assembly");
                        let toggle = ui.add_enabled(self.mount_names().is_some(), button);
                        let hint = "Wire the circuit one wire at a time, in the file's order";
                        if toggle.on_hover_text(hint).on_disabled_hover_text("The circuit has no wires").clicked() {
                            self.set_mount(!self.mount.active);
                        }
                        if self.viewer.has_enclosure() {
                            let mut shown = self.viewer.shows_enclosure();
                            let toggle = ui.toggle_value(&mut shown, "Enclosure");
                            if toggle.on_hover_text("Show the enclosure around the parts, see-through").changed() {
                                self.viewer.set_show_enclosure(shown);
                            }
                        }
                        toolbar_separator(ui);
                    }
                    // A circuit has no parameters: room for its own toggles.
                    if self.is_circuit() {
                        let toggle = ui.toggle_value(&mut self.catalogue_open, "Catalogue");
                        toggle.on_hover_text("The parts of the library and your own, to add to the circuit");
                    } else {
                        let mut shown = self.customizer_shown();
                        let toggle = ui.toggle_value(&mut shown, "Customizer");
                        if toggle.on_hover_text("Show the parameters the file declares").changed() {
                            self.customizer_open = Some(shown);
                        }
                    }
                    toolbar_separator(ui);
                    ui.toggle_value(&mut self.viewer.show_grid, "Grid").on_hover_text("Show the XY grid");
                    ui.toggle_value(&mut self.viewer.show_axes, "Axes").on_hover_text("Show the axes");
                    ui.toggle_value(&mut self.viewer.layer_view, "Layers")
                        .on_hover_text("Cut the model at the top of a layer, picked with the slider on the right");
                    if self.viewer.has_pauses() {
                        ui.toggle_value(&mut self.viewer.show_pauses, "Pauses")
                            .on_hover_text("Show the heights where the print pauses");
                    }
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

    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let mut clear = false;
        let menu = ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
            ui.menu_button("Open Recent", |ui| {
                for path in self.recent.paths() {
                    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                    let folder = path.parent().map(display_folder).unwrap_or_default();
                    let button = Button::new(name).shortcut_text(folder);
                    if ui.add(button).on_hover_text(path.display().to_string()).clicked() {
                        chosen = Some(path.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Clear Menu").clicked() {
                    clear = true;
                    ui.close();
                }
            })
        });
        menu.response.on_hover_text("Reopen one of the last files").on_disabled_hover_text("No recent files");
        if let Some(path) = chosen {
            self.open_recent(path);
        }
        if clear {
            self.recent.clear();
            self.save_recent();
        }
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
                        if model.parts > 0 {
                            ui.label(small(format!("· {} part{}", model.parts, plural(model.parts))));
                            ui.label(small(format!("· {} triangles", group_digits(s.triangles))));
                        } else if model.is_2d {
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
                empty_hint: if self.is_circuit() {
                    "The circuit places no parts yet. Add a [parts.name] table with the part's file."
                } else {
                    viewer::empty_hint(!self.text.trim().is_empty())
                },
            };
            // A double-click on a part opens it rather than framing the view.
            self.viewer.frame_on_double_click = !matches!(self.hover, Some(Hit::Part(_)));
            self.start_rewire(ctx);
            self.viewer.orbit_enabled = self.rewire.is_none();
            let response = self.viewer.show(ui, &status);
            self.view_rect = response.rect;
            self.circuit_input(ctx, &response);
            self.paint_rewire(&ui.painter_at(response.rect), response.rect, ctx.input(|i| i.pointer.interact_pos()));
            // A part dragged from the catalogue lands where it is dropped.
            if let Some(dragged) = response.dnd_release_payload::<catalogue::Dragged>() {
                let pointer = ctx.input(|i| i.pointer.interact_pos());
                let at = pointer.and_then(|pos| catalogue::on_ground(&self.viewer.ray(response.rect, pos)));
                let at = at.unwrap_or_else(|| catalogue::free_spot(&self.part_boxes));
                self.add_part(ctx, &dragged.0, at);
            }
            if let Some(wiring) = &self.wiring {
                let painter = ui.painter_at(response.rect);
                let right = if self.viewer.layer_view { 70.0 } else { 12.0 };
                // Below the assembly bar, when it is shown.
                let top = if self.mount.active { 110.0 } else { 12.0 };
                let legend = wiring::paint_legend(&painter, response.rect, right, top, &wiring.legend());
                let lengths = inspect::Lengths { routed: &wiring.lengths, margin: self.wire_margin.mm };
                if let Some(selection) = self.selection.filter(|_| !self.mount.active) {
                    let (title, lines) = inspect::card(&wiring.circuit, selection, &self.text, lengths);
                    inspect::paint_card(&painter, response.rect, &title, &lines);
                }
                if let (Some(hover), Some(pos)) = (self.hover, response.hover_pos()) {
                    if !response.dragged() {
                        let text = inspect::bubble(&wiring.circuit, hover, lengths);
                        inspect::paint_bubble(&painter, response.rect, pos, &text);
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
                // The margin added to each wire's length, under the legend.
                if !wiring.circuit.wires.is_empty() {
                    let field = egui::Rect::from_min_size(
                        legend.left_bottom() + Vec2::new(0.0, 6.0),
                        Vec2::new(legend.width().max(190.0), 22.0),
                    );
                    let mut mm = self.wire_margin.mm;
                    let value = egui::DragValue::new(&mut mm)
                        .range(0.0..=500.0)
                        .speed(0.5)
                        .max_decimals(1)
                        .prefix("Wire margin: ")
                        .suffix(" mm");
                    let hint = "Added to each wire's routed length, for its stripped ends and some slack";
                    if ui.put(field, value).on_hover_text(hint).changed() {
                        self.wire_margin.mm = mm;
                        if let Err(err) = self.wire_margin.save() {
                            self.console.log(Level::Warning, format!("Cannot save the wire margin: {err}"));
                        }
                    }
                }
            }
        });
        if self.mount.active {
            self.mount_bar(ctx, self.view_rect);
            // The pins of the wire on screen pulse.
            self.update_overlay();
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some(edit) = self.viewer.take_pause_edit() {
            self.apply_pause_edit(ctx, edit);
        }
    }

    /// Makes a change to the pauses asked for in the layer view in the
    /// code, as one step of the editor's undo history, and renders it.
    fn apply_pause_edit(&mut self, ctx: &egui::Context, edit: PauseEdit) {
        // The pauses point into the text they were rendered from.
        if self.viewer.scene_source() != self.text {
            self.console.log(Level::Warning, "The code changed since the last render: try again once it is rendered");
            return;
        }
        let (result, done) = match edit {
            PauseEdit::Add { z } => {
                (Ok(pause_edit::add(&self.text, z)), format!("Added a pause at {} mm", pause_edit::number(z)))
            }
            PauseEdit::Move { number, z } => {
                (pause_edit::set_height(&self.text, number, z), format!("Moved a pause to {} mm", pause_edit::number(z)))
            }
            PauseEdit::Remove { call } => (pause_edit::remove(&self.text, call), "Removed a pause".to_string()),
        };
        match result {
            Ok((text, cursor)) => {
                self.editor.replace_text(ctx, &mut self.text, text, cursor);
                self.console.log(Level::Info, done);
                self.render_now();
            }
            Err(why) => self.console.log(Level::Warning, format!("Cannot change the pause: {why}")),
        }
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
        self.catalogue_panel(ctx);
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

/// A folder for the Open Recent menu, with the home folder shown as `~`.
fn display_folder(dir: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

/// Something of a circuit named as `part.pin`, `part` or `wire:7` (from 1).
fn find_hit(circuit: &c2f_circuit::Circuit, name: &str) -> Option<Hit> {
    if let Some(number) = name.strip_prefix("wire:") {
        let index = number.parse::<usize>().ok()?.checked_sub(1)?;
        return (index < circuit.wires.len()).then_some(Hit::Wire(index));
    }
    let part_named = |n: &str| circuit.parts.iter().position(|p| p.name == n);
    match name.split_once('.') {
        Some((part, pin)) => {
            let part = part_named(part)?;
            Some(Hit::Pin(c2f_circuit::End { part, pin: circuit.parts[part].pin(pin)? }))
        }
        None => part_named(name).map(Hit::Part),
    }
}

/// The box around each part of a circuit's scene.
fn part_boxes(scene: &crate::worker::Scene) -> Vec<Option<(glam::Vec3, glam::Vec3)>> {
    scene
        .parts
        .iter()
        .map(|range| {
            let points = &scene.solid.positions[range.clone()];
            let first = glam::Vec3::from(*points.first()?);
            Some(points.iter().fold((first, first), |(lo, hi), &p| (lo.min(p.into()), hi.max(p.into()))))
        })
        .collect()
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
