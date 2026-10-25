//! The studio window: the header with the File menu, the tabs and the
//! layout switch; the code with its console; the 3D view with its tools,
//! the parameters and the animation bar over it; the status bar; and the
//! document they work on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, Frame, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Rect,
    RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use eframe::glow;
use c2f_circuit::End;
use c2f_eval::customizer::Literal;
use c2f_geom::Stats;

use crate::agent_panel::{self, AgentPanel};
use crate::animation::{self, Animation, Step};
use crate::catalogue::{self, Catalogue};
use crate::circuit;
use crate::app_icon::{self, Tint};
use crate::console::{Console, Level};
use crate::customizer::{self, Customizer};
use crate::editor::{Editor, FindRequest};
use crate::examples;
use crate::i18n::{self, Language, tr};
use crate::icon_setting;
use crate::icons::{self, Icon};
use crate::insert;
use crate::open_events;
use crate::pause_edit;
use crate::part_edit::{self, Grab};
use crate::recent::RecentFiles;
use crate::tabs::{self, Owners, Session, Shelf};
use crate::renderer;
use crate::rewire::{self, Edit, Side};
use crate::screenshot::Screenshot;
use crate::settings::{self, GLTF_SCALES, Placement, Settings};
use crate::shortcuts::{self, Action, Shortcuts};
use crate::theme::{self, Appearance};
use crate::viewer::{self, PauseEdit, Viewer};
use crate::inspect;
use crate::mount::{self, Mount};
use crate::pick::{self, Hit};
use crate::prefs::WireMargin;
use crate::window;
use crate::wiring::{self, Emphasis, Look, Wiring};
use crate::worker::{ExportOptions, Format, Reply, Worker, group_digits};

/// The agent's live tools: they read and edit the open documents.
pub mod live;
/// The agent's edits as they land: marked, flashed, undone together.
mod agent_edits;
/// The review of the agent's changes before they apply.
mod review;

/// File dialog filters: a name and an extension.
fn design_files() -> (&'static str, &'static str) {
    (tr("Code2Form design", "Fichier Code2Form"), "c2f")
}
fn scad_files() -> (&'static str, &'static str) {
    (tr("SCAD design", "Fichier SCAD"), "scad")
}
fn circuit_files() -> (&'static str, &'static str) {
    (tr("Code2Form circuit", "Circuit Code2Form"), "c2fc")
}

/// How fast the exploded view slides its parts apart and back: the whole
/// way in a third of a second.
const EXPLODE_SPEED: f32 = 3.0;
/// The height of the view's tools and the gap under them, where the
/// exploded view's card goes.
const VIEW_TOOLS_HEIGHT: f32 = 44.0;

/// How far apart the exploded view first draws the parts: a quarter of the
/// way, enough to see the gaps without losing the assembly.
const EXPLODE_START: f32 = 0.25;

/// How long a customizer control must rest before a re-render.
const PARAMETER_DEBOUNCE: Duration = Duration::from_millis(120);
/// How often the file is checked for changes on disk.
const POLL: Duration = Duration::from_secs(1);
/// The pause in typing before an autosave.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(1000);

/// Outside the editor and the text fields, which have their own; the other
/// shortcuts are the settings'.
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers { shift: true, ..Modifiers::COMMAND }, Key::Z);

/// The app icons in the settings, in points, and how many in a row.
const ICON_CHOICE: f32 = 44.0;
const ICONS_PER_ROW: usize = 5;
/// The height of the settings' pages, so that the window keeps its size.
const SETTINGS_HEIGHT: f32 = 380.0;

/// A color the settings offer: its name, in the interface's language, and
/// its value.
type NamedColor = (fn() -> &'static str, [u8; 3]);

/// The model colors the settings offer, besides the color picker.
const MODEL_COLORS: [NamedColor; 8] = [
    (|| tr("Yellow", "Jaune"), settings::MODEL_COLOR),
    (|| tr("Orange", "Orange"), [240, 140, 50]),
    (|| tr("Red", "Rouge"), [214, 72, 66]),
    (|| tr("Green", "Vert"), [96, 186, 98]),
    (|| tr("Blue", "Bleu"), [72, 140, 226]),
    (|| tr("Violet", "Violet"), [150, 104, 214]),
    (|| tr("Gray", "Gris"), [168, 174, 184]),
    (|| tr("White", "Blanc"), [238, 238, 236]),
];

/// The pages of the settings window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Appearance,
    Editor,
    Model,
    Export,
    Libraries,
    Shortcuts,
}

impl Section {
    const ALL: [Section; 7] = [
        Section::General,
        Section::Appearance,
        Section::Editor,
        Section::Model,
        Section::Export,
        Section::Libraries,
        Section::Shortcuts,
    ];

    /// As a screenshot run names it.
    fn id(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Editor => "editor",
            Section::Model => "model",
            Section::Export => "export",
            Section::Libraries => "libraries",
            Section::Shortcuts => "shortcuts",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Section::General => tr("General", "Général"),
            Section::Appearance => tr("Appearance", "Apparence"),
            Section::Editor => tr("Editor", "Éditeur"),
            Section::Model => tr("Model", "Modèle"),
            Section::Export => tr("Export", "Export"),
            Section::Libraries => tr("Libraries", "Bibliothèques"),
            Section::Shortcuts => tr("Shortcuts", "Raccourcis"),
        }
    }
}

/// A change to a list of folders in the settings.
enum FolderEdit {
    Add,
    Remove(usize),
}

/// A change asked for on the Shortcuts page.
enum ShortcutEdit {
    /// Records the next keys pressed for the action.
    Record(Action),
    Reset(Action),
    ResetAll,
}

/// A change to the folder exports go to.
enum ExportFolderEdit {
    NextToTheDesign,
    Choose,
}

/// Something that would lose the active tab's unsaved edits, waiting for
/// confirmation.
#[derive(Clone, Copy)]
enum Pending {
    /// Closing the tab.
    Close,
    /// Quitting: asked of each modified tab in turn.
    Quit,
}

impl Pending {
    fn verb(self) -> &'static str {
        match self {
            Pending::Close => tr("closing it", "de le fermer"),
            Pending::Quit => tr("quitting", "de quitter"),
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
    /// The corners of the box around what is drawn.
    bounds: Option<([f32; 3], [f32; 3])>,
}

/// An open file and everything that goes with it: its text and undo
/// history, its 3D view, console and customizer, and its circuit's state.
struct Tab {
    /// Tells the tabs apart, the replies of the worker and the state egui
    /// keeps for each editor.
    id: u64,
    text: String,
    /// The text as last loaded or saved, to tell unsaved edits.
    saved_text: String,
    path: Option<PathBuf>,
    /// Modification time of the file when last read or written.
    disk_mtime: Option<SystemTime>,

    /// When to re-render after typing.
    render_deadline: Option<Instant>,
    /// When to save after typing, with autosave on.
    autosave_deadline: Option<Instant>,
    /// The source and customizer values of the last render request.
    sent_source: Option<String>,
    sent_values: Vec<(String, Literal)>,
    /// The newest render asked for, and the newest that came back.
    sent: u64,
    finished: u64,
    /// A file it depends on changed while it was not shown: it renders
    /// again once shown.
    stale: bool,
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
    /// A drag rewiring the circuit, from a pin or a wire's end.
    rewire: Option<Rewire>,
    /// A part being dragged: the drag, where the press was, whether it
    /// moved, and the circuit as read before it.
    part_drag: Option<(part_edit::Drag, egui::Pos2, bool, Arc<c2f_circuit::Circuit>)>,
    /// The generation it was read for.
    circuit_generation: u64,
    /// The circuit's part files and their modification times, to render
    /// again when one changes on disk.
    part_files: Vec<(PathBuf, Option<SystemTime>)>,

    editor: Editor,
    viewer: Viewer,
    console: Console,
    customizer: Customizer,
    /// Shown or hidden by the user; by default shown when the file has
    /// parameters.
    customizer_open: Option<bool>,
    /// Playing the design along `$t`, when it reads it.
    animation: Animation,
    /// The exploded view: whether it is on, how far apart the slider sets
    /// the parts (0 to 1), and how far apart they are drawn, on their way.
    exploded: bool,
    explode_amount: f32,
    explode_shown: f32,
    /// Radially, or along each part's axis.
    explode_how: c2f_geom::Explode,
    /// The lines (from 0, the end excluded) the agent's last edit changed.
    agent_marks: Vec<std::ops::Range<usize>>,
    /// The agent's edits as they show in the editor and the view, and the
    /// last answer's, to undo together.
    agent_highlight: Option<agent_edits::Highlight>,
    agent_group: Option<agent_edits::AgentGroup>,
    agent_flash: Option<agent_edits::Flash>,
}

impl Tab {
    /// An untitled tab, with nothing in it.
    fn new(id: u64, gl: Option<&Arc<glow::Context>>) -> Self {
        Tab {
            id,
            text: String::new(),
            saved_text: String::new(),
            path: None,
            disk_mtime: None,
            render_deadline: None,
            autosave_deadline: None,
            sent_source: None,
            sent_values: Vec::new(),
            sent: 0,
            finished: 0,
            stale: false,
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
            rewire: None,
            part_drag: None,
            circuit_generation: 0,
            part_files: Vec::new(),
            editor: Editor::for_tab(id),
            viewer: Viewer::new(gl),
            console: Console::default(),
            customizer: Customizer::default(),
            customizer_open: None,
            animation: Animation::default(),
            exploded: false,
            explode_amount: EXPLODE_START,
            explode_shown: 0.0,
            explode_how: c2f_geom::Explode::default(),
            agent_marks: Vec::new(),
            agent_highlight: None,
            agent_group: None,
            agent_flash: None,
        }
    }

    fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }

    /// An untitled tab never typed in: a file opened takes its place.
    fn is_blank(&self) -> bool {
        self.path.is_none() && self.text.is_empty() && self.saved_text.is_empty()
    }

    fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| tr("Untitled", "Sans titre").to_string(), |n| n.to_string_lossy().into_owned())
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
        self.sent > self.finished
    }

    /// Whether the text or the values changed since the last render.
    fn unrendered(&self) -> bool {
        self.sent_source.as_deref() != Some(self.text.as_str()) || self.sent_values != self.customizer.values()
    }

    fn customizer_shown(&self) -> bool {
        self.customizer_open.unwrap_or_else(|| self.customizer.has_parameters())
    }
}

/// Where the open tabs and the settings are kept; `None` for this run only.
struct Stores {
    session: Option<PathBuf>,
    settings: Option<PathBuf>,
    /// The Agent panel's conversations.
    conversations: Option<PathBuf>,
}

pub struct StudioApp {
    /// The active tab; the others wait on the shelf.
    doc: Tab,
    shelf: Shelf<Tab>,
    /// The id of the next tab opened.
    next_id: u64,
    /// Which tab asked for each render on its way.
    owners: Owners,
    /// The tab whose model the worker keeps for exports.
    exportable: Option<u64>,
    /// The tabs open, as last written, and where they are kept.
    session: Session,
    session_store: Option<PathBuf>,
    /// Quitting: the tabs whose unsaved edits were saved or given up.
    quit_cleared: Vec<u64>,
    gl: Option<Arc<glow::Context>>,
    last_poll: Instant,

    worker: Worker,
    /// Added to each wire's routed length.
    wire_margin: WireMargin,
    /// When the app started, for the pins that pulse.
    started: Instant,
    /// Where the 3D view is, as last shown.
    view_rect: egui::Rect,

    console_open: bool,
    /// The parts catalogue, shown in its panel for a circuit.
    catalogue: Catalogue,
    catalogue_open: bool,
    recent: RecentFiles,
    /// The settings, as last saved, and where they are kept: those of the
    /// view apply to every tab's.
    settings: Settings,
    saved_settings: Settings,
    settings_store: Option<PathBuf>,
    settings_open: bool,
    settings_section: Section,
    /// The action whose keys the Shortcuts page waits for, and why the
    /// last keys pressed were refused.
    recording: Option<Action>,
    record_problem: Option<String>,
    /// The icons the settings offer, drawn when first shown.
    icon_choices: Vec<TextureHandle>,
    /// The icon at the toolbar's left, in the tint picked.
    logo: TextureHandle,
    /// The window's size and place as last seen, and whether it is
    /// maximized; the settings take it when they are saved.
    window: Option<(Placement, bool)>,
    /// The zoom of the last frame, which the window was measured with.
    measured_zoom: f32,
    confirm: Option<Pending>,
    allow_close: bool,
    title: String,
    screenshot: Option<Screenshot>,
    /// The code beside the view, or one of them alone.
    arrangement: Arrangement,
    /// The widths of the header's buttons on the right and of its layout
    /// switch, as last drawn.
    header_right: f32,
    header_switch: f32,
    /// The conversation with the agent, docked on the right.
    agent: AgentPanel,
    /// The requests of the agent's live tools.
    live: live::Live,
    /// Whether the agent's turn was under way at the last frame.
    agent_running: bool,
}

impl StudioApp {
    /// Opens the tabs of the last session, then `paths` (from the command
    /// line or the Finder), the last of them active.
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Vec<PathBuf>, settings: Settings) -> Self {
        theme::apply(&cc.egui_ctx);
        // ⌘+ and ⌘− change the interface size in the settings instead.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        open_events::wake(&cc.egui_ctx);
        // Launched by opening files in the Finder.
        let paths = paths.into_iter().chain(open_events::take()).collect();
        let recent = RecentFiles::load(crate::recent::store());
        let stores = Stores { session: tabs::store(), settings: settings::store(), conversations: agent_panel::store() };
        let mut app = StudioApp::with(&cc.egui_ctx, cc.gl.clone(), stores, recent, settings, paths);
        let icon = app.settings.icon;
        if let Err(err) = icon_setting::restore(icon) {
            app.doc.console.log(Level::Warning, tr!("Cannot give the app its {} icon: {err}", "Impossible de donner à l'app l'icône {} : {err}", icon.name()));
        }
        // Agents reach the open documents through `c2f mcp --studio`;
        // not from a screenshot run, which is no one's studio, nor yet on
        // Windows.
        if app.screenshot.is_none() && cfg!(unix) {
            if let Err(err) = c2f_agent::studio_link::serve(Arc::new(app.live_tools())) {
                app.doc.console.log(Level::Info, tr!("Agents cannot reach this window: {err}", "Les agents ne peuvent pas joindre cette fenêtre : {err}"));
            }
        }
        app
    }

    /// The app, with its tabs and settings kept in `stores`.
    fn with(
        ctx: &egui::Context,
        gl: Option<Arc<glow::Context>>,
        stores: Stores,
        recent: RecentFiles,
        settings: Settings,
        paths: Vec<PathBuf>,
    ) -> Self {
        let session_store = stores.session;
        let engine_settings = settings.clone();
        let agent = AgentPanel::new(move || agent_panel::engine_for(&engine_settings), stores.conversations);
        let mut app = StudioApp {
            doc: Tab::new(0, gl.as_ref()),
            shelf: Shelf::default(),
            next_id: 1,
            owners: Owners::default(),
            exportable: None,
            session: Session::load(session_store.as_deref()),
            session_store,
            quit_cleared: Vec::new(),
            gl,
            last_poll: Instant::now(),
            worker: Worker::spawn(ctx.clone()),
            wire_margin: WireMargin::load(crate::prefs::wire_margin_store()),
            started: Instant::now(),
            view_rect: egui::Rect::NOTHING,
            console_open: true,
            catalogue: Catalogue::new(ctx),
            catalogue_open: false,
            recent,
            logo: logo_texture(ctx, settings.icon),
            saved_settings: settings.clone(),
            settings,
            settings_store: stores.settings,
            settings_open: false,
            settings_section: Section::General,
            recording: None,
            record_problem: None,
            icon_choices: Vec::new(),
            window: None,
            measured_zoom: ctx.zoom_factor(),
            confirm: None,
            allow_close: false,
            title: String::new(),
            screenshot: Screenshot::from_env(),
            arrangement: Arrangement::Split,
            header_right: 0.0,
            header_switch: 0.0,
            agent,
            live: live::Live::new(ctx),
            agent_running: false,
        };
        app.apply_settings(ctx);
        // A screenshot of a layout, as `layout:view`.
        let layout = app.screenshot.as_ref().and_then(|s| s.select.as_deref()?.strip_prefix("layout:"));
        if let Some(arrangement) = layout.and_then(Arrangement::from_id) {
            app.arrangement = arrangement;
        }
        // A screenshot of the Agent panel, a message sent.
        if app.screenshot.as_ref().is_some_and(|s| s.select.as_deref() == Some("agent")) {
            app.agent.open = true;
            app.agent.send("Make the box 40 mm wide, with a **lid**.");
        }
        // A screenshot of a settings page, as `settings:shortcuts`.
        let page = app.screenshot.as_ref().and_then(|s| s.select.as_deref()?.strip_prefix("settings:"));
        if let Some(section) = page.and_then(|page| Section::ALL.into_iter().find(|s| s.id() == page)) {
            app.settings_open = true;
            app.settings_section = section;
        }
        // The tabs of the last session, quietly: a file gone since is
        // simply not reopened.
        let session = std::mem::take(&mut app.session);
        let mut active = None;
        let reopened = if app.settings.reopen_tabs { session.paths.as_slice() } else { &[] };
        for (i, path) in reopened.iter().enumerate().filter(|(_, p)| p.exists()) {
            app.open(ctx, path.clone());
            if session.active == Some(i) {
                active = Some(app.shelf.active());
            }
        }
        if let Some(index) = active {
            app.select_tab(ctx, index);
        }
        app.session = session;
        for path in paths {
            if path.exists() {
                app.open(ctx, path);
                continue;
            }
            if !app.doc.is_blank() {
                app.add_tab();
            }
            app.doc.console.log(Level::Info, tr!("New file {} (not saved yet)", "Nouveau fichier {} (pas encore enregistré)", path.display()));
            app.doc.path = Some(absolute(&path));
            app.doc.customizer.open(app.doc.path.as_deref());
            app.render_now();
        }
        app.show_tab(ctx);
        app.screenshot_agent(ctx);
        app
    }

    // ---- Tabs ----

    /// Opens a new untitled tab, after the others, and makes it active.
    fn add_tab(&mut self) {
        let tab = self.fresh_tab();
        self.shelf.push(&mut self.doc, tab);
    }

    /// An untitled tab, its view and editor as the settings say.
    fn fresh_tab(&mut self) -> Tab {
        let mut tab = Tab::new(self.next_id, self.gl.as_ref());
        self.next_id += 1;
        apply_tab_settings(&self.settings, &mut tab);
        tab
    }

    /// The tab of `id`, when it is still open.
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.shelf.iter(&self.doc).position(|tab| tab.id == id)
    }

    /// The tab of the file at `path`, when it is open.
    fn tab_of(&self, path: &Path) -> Option<usize> {
        self.shelf.iter(&self.doc).position(|tab| tab.path.as_deref().is_some_and(|p| tabs::same_file(p, path)))
    }

    /// Runs `f` with tab `index` in the active tab's place, so that the
    /// code written for the active tab works on any tab.
    fn in_tab(&mut self, index: usize, f: impl FnOnce(&mut Self)) {
        let Some(other) = self.shelf.other_mut(index) else { return f(self) };
        std::mem::swap(&mut self.doc, other);
        f(self);
        if let Some(other) = self.shelf.other_mut(index) {
            std::mem::swap(&mut self.doc, other);
        }
    }

    /// Makes tab `index` the active one.
    fn select_tab(&mut self, ctx: &egui::Context, index: usize) {
        if index == self.shelf.active() || index >= self.shelf.len() {
            return;
        }
        self.autosave_now();
        // A drag or a hover in the view belongs to the view it began in.
        self.doc.rewire = None;
        self.doc.hover = None;
        // A part being dragged goes back where it was.
        if let Some((drag, _, moved, original)) = self.doc.part_drag.take() {
            if moved {
                self.preview_part(&part_edit::Drag { at: drag.from, ..drag }, &original);
            }
        }
        self.shelf.select(&mut self.doc, index);
        self.show_tab(ctx);
    }

    /// The active tab comes on screen: its last scene shows at once, and it
    /// renders again only when something changed while it was hidden, or
    /// when its render gave way to another tab's.
    fn show_tab(&mut self, ctx: &egui::Context) {
        let lost = self.doc.busy() && self.doc.sent != self.worker.latest();
        if self.doc.sent == 0 || self.doc.stale || lost {
            self.render_now();
        }
        self.doc.editor.focus(ctx);
    }

    /// Closes tab `index`, first asking about its unsaved edits, in the
    /// tab itself.
    fn close_tab(&mut self, ctx: &egui::Context, index: usize) {
        let dirty = self.shelf.iter(&self.doc).nth(index).is_some_and(Tab::is_dirty);
        if dirty {
            self.select_tab(ctx, index);
            self.confirm = Some(Pending::Close);
        } else {
            self.close_now(ctx, index);
        }
    }

    /// Closes tab `index`. The last tab gives way to an untitled one: the
    /// window always has a tab.
    fn close_now(&mut self, ctx: &egui::Context, index: usize) {
        let active = index == self.shelf.active();
        let closed = match self.shelf.close(&mut self.doc, index) {
            Some(closed) => closed,
            None if active => {
                let tab = self.fresh_tab();
                std::mem::replace(&mut self.doc, tab)
            }
            None => return,
        };
        closed.editor.forget(ctx);
        let mut viewer = closed.viewer;
        if let Some(gl) = &self.gl {
            viewer.destroy(gl);
        }
        if active {
            self.show_tab(ctx);
        }
    }

    /// The next tab with unsaved edits not yet asked about while quitting.
    fn next_unsaved(&self) -> Option<usize> {
        self.shelf.iter(&self.doc).position(|tab| tab.is_dirty() && !self.quit_cleared.contains(&tab.id))
    }

    /// Writes the open tabs down when they changed, for the next launch.
    fn save_session(&mut self) {
        let mut session = Session::default();
        for (i, tab) in self.shelf.iter(&self.doc).enumerate() {
            let Some(path) = &tab.path else { continue };
            if i == self.shelf.active() {
                session.active = Some(session.paths.len());
            }
            session.paths.push(path.clone());
        }
        if session == self.session {
            return;
        }
        if let Err(err) = session.save(self.session_store.as_deref()) {
            self.doc.console.log(Level::Warning, tr!("Cannot save the open tabs: {err}", "Impossible d'enregistrer les onglets ouverts : {err}"));
        }
        self.session = session;
    }

    // ---- Rendering ----

    fn render_now(&mut self) {
        let doc = &mut self.doc;
        doc.render_deadline = None;
        doc.stale = false;
        doc.editor.set_circuit(doc.is_circuit());
        doc.sent_source = Some(doc.text.clone());
        doc.sent_values = doc.customizer.values();
        let t = doc.animation.t();
        doc.sent = self.worker.render(doc.text.clone(), doc.source_path(), doc.sent_values.clone(), t);
        doc.animation.sent(doc.sent);
        self.owners.claim(doc.sent, doc.id);
    }

    /// Renders the active tab, or marks a hidden one to render once shown.
    fn render_when_shown(&mut self, shown: bool) {
        if shown {
            self.render_now();
        } else {
            self.doc.stale = true;
        }
    }

    /// Renders unless the text and values are what was last rendered.
    fn render_if_changed(&mut self) {
        self.doc.render_deadline = None;
        if self.doc.unrendered() {
            self.render_now();
        }
    }

    /// Hands each reply of the worker to the tab that asked for it; the
    /// replies of a tab closed since are dropped.
    fn receive(&mut self) {
        for reply in self.worker.poll() {
            let generation = match &reply {
                Reply::Rendered(outcome) => outcome.generation,
                Reply::Circuit { generation, .. } => *generation,
                Reply::Exported { target, result } => {
                    // Asked for by the active tab.
                    match result {
                        Ok(message) => {
                            self.doc.console.log(Level::Info, message.clone());
                            if self.settings.reveal_export {
                                if let Err(err) = reveal(target) {
                                    let message = tr!("Cannot show {}: {}", "Impossible de montrer {} : {}", target.display(), err);
                                    self.doc.console.log(Level::Warning, message);
                                }
                            }
                        }
                        Err(error) => {
                            let message = tr!("Exporting {} failed: {error}", "Échec de l'export de {} : {error}", target.display());
                            self.doc.console.log(Level::Error, message);
                        }
                    }
                    continue;
                }
            };
            let owner = self.owners.owner(generation);
            if let Reply::Rendered(outcome) = &reply {
                self.owners.finished(generation);
                // The worker keeps a design's model for exports, and none
                // for a circuit.
                match owner.and_then(|id| self.shelf.iter(&self.doc).find(|tab| tab.id == id)) {
                    Some(tab) if tab.is_circuit() => self.exportable = None,
                    // A failed render leaves the model as it was.
                    Some(_) if outcome.result.is_err() => {}
                    Some(_) => self.exportable = owner,
                    None => self.exportable = None,
                }
            }
            let Some(index) = owner.and_then(|id| self.tab_index(id)) else { continue };
            let shown = index == self.shelf.active();
            self.in_tab(index, |app| app.accept(reply, shown));
        }
    }

    /// Takes a reply to a render of the active tab, or of the tab put in
    /// its place by `in_tab` when not `shown`.
    fn accept(&mut self, reply: Reply, shown: bool) {
        match reply {
            Reply::Rendered(outcome) => {
                if outcome.generation <= self.doc.finished {
                    return;
                }
                self.doc.finished = outcome.generation;
                self.doc.elapsed = Some(outcome.elapsed);
                // A reply from before a file was opened would bring back
                // the previous file's parameters.
                let current = outcome.generation >= self.doc.sent;
                if let Some(parameters) = outcome.parameters.filter(|_| current) {
                    self.doc.customizer.set_parameters(parameters);
                }
                self.doc.console.set_render_output(outcome.console);
                let source = (self.doc.sent_source.clone().unwrap_or_default(), self.doc.sent_values.clone());
                let scene = outcome.result.as_ref().ok();
                let on_screen = self.doc.animation.rendered(outcome.generation, outcome.animated, &source, scene);
                let asks = |what: &str| {
                    self.screenshot.as_ref().and_then(|s| s.select.as_deref()).is_some_and(|s| s.split(',').any(|w| w == what))
                };
                if asks("play") && shown && self.doc.animation.animated && self.doc.animation.cached() == 1 {
                    self.doc.animation.play();
                }
                let explode = asks("exploded") && shown && !self.doc.exploded;
                let along_axis = asks("axis");
                let (cut, opacity) = self.screenshot.as_ref().and_then(|s| s.select.as_deref()).map_or((None, None), section_asked);
                match outcome.result {
                    Ok(scene) => {
                        self.doc.failed = false;
                        self.doc.editor.error_line = outcome.annotation_error;
                        let empty = scene.is_empty();
                        let had_model = self.doc.model.as_ref().is_some_and(|m| !m.empty);
                        let frame = !empty && (self.doc.frame_next || !had_model);
                        if frame {
                            self.doc.frame_next = false;
                        }
                        self.doc.model = Some(ModelInfo {
                            stats: scene.stats,
                            is_2d: scene.is_2d,
                            formats: scene.formats,
                            empty,
                            parts: scene.parts.len(),
                            bounds: scene.bounds(),
                        });
                        if !self.doc.is_circuit() && current {
                            // A design's @circuit and its parts.
                            self.watch(outcome.watch.clone());
                        }
                        self.doc.part_boxes = part_boxes(&scene);
                        if on_screen {
                            self.doc.viewer.set_scene(scene, frame);
                            if let Some(section) = cut.filter(|_| shown) {
                                self.doc.viewer.show_section(section);
                            }
                            if let Some(opacity) = opacity.filter(|_| shown) {
                                self.doc.viewer.enclosure_opacity = opacity;
                            }
                        }
                        if explode && self.doc.viewer.can_explode() {
                            self.doc.exploded = true;
                            self.doc.explode_amount = 1.0;
                            self.doc.explode_shown = 1.0;
                            if along_axis {
                                self.doc.explode_how = c2f_geom::Explode::Axis;
                                self.doc.viewer.set_explode_how(c2f_geom::Explode::Axis);
                            }
                            self.doc.viewer.set_explode(1.0);
                            self.doc.viewer.view_all();
                        }
                        // The new solid comes in its own colors.
                        self.update_overlay();
                        // A part still being dragged stays where the drag has it.
                        if let Some((drag, _, true, original)) = self.doc.part_drag.clone() {
                            self.preview_part(&drag, &original);
                        }
                    }
                    Err(line) => {
                        self.doc.failed = true;
                        self.doc.editor.error_line = line.or(outcome.annotation_error);
                    }
                }
            }
            Reply::Circuit { generation, circuit } => {
                if generation < self.doc.circuit_generation || generation < self.doc.sent {
                    return;
                }
                self.doc.circuit_generation = generation;
                self.set_circuit(Some(circuit));
                if let Some(target) = self.screenshot.as_ref().and_then(|s| s.select.clone()).filter(|_| shown) {
                    // The section and the opacity are the view's, set with the scene.
                    let target = target.split(',').filter(|w| section_asked(w) == (None, None)).collect::<Vec<_>>().join(",");
                    let rewire = target.strip_prefix("rewire:").and_then(|t| t.split_once('>'));
                    let moved = target.strip_prefix("move:").and_then(|t| t.split_once(':'));
                    if target == "catalogue" {
                        self.catalogue_open = true;
                    } else if let Some((name, to)) = moved {
                        // A part held mid-drag, as `move:mic:40,10` asks.
                        let xy: Vec<f64> = to.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                        let circuit = self.doc.wiring.as_ref().map(|w| w.circuit.clone());
                        let part = circuit.as_ref().and_then(|c| c.parts.iter().position(|p| p.name == name));
                        if let (Some(circuit), Some(part), &[x, y]) = (circuit, part, xy.as_slice()) {
                            let from = circuit.parts[part].at;
                            let drag = part_edit::Drag { part, offset: [0.0; 2], from, at: [x, y, from[2]], turns: 0 };
                            self.doc.part_drag = Some((drag.clone(), egui::Pos2::ZERO, true, circuit.clone()));
                            self.preview_part(&drag, &circuit);
                        }
                    } else if let Some((from, to)) = rewire {
                        // A new wire held over a pin, as a drag shows it.
                        let end = |name: &str| match self.doc.wiring.as_ref().and_then(|w| find_hit(&w.circuit, name)) {
                            Some(Hit::Pin(end)) => Some(end),
                            _ => None,
                        };
                        if let (Some(anchor), Some(to), Some(wiring)) = (end(from), end(to), &self.doc.wiring) {
                            let error = rewire::new_error(&wiring.circuit, Edit::Add { from: anchor, to });
                            let origin = egui::Pos2::ZERO;
                            let held = Rewire { grabbed: None, anchor, origin, moved: true, target: Some(to), error };
                            self.doc.rewire = Some(held);
                            self.update_overlay();
                        }
                    } else if let Some(step) = target.strip_prefix("assembly:").and_then(|n| n.parse::<usize>().ok()) {
                        // Set before turning the mode on: a screenshot
                        // leaves the saved progress as it is.
                        self.doc.mount.step = step.saturating_sub(1);
                        self.set_mount(true);
                    } else {
                        self.doc.selection = self.doc.wiring.as_ref().and_then(|w| find_hit(&w.circuit, &target));
                        self.update_overlay();
                    }
                }
            }
            // Handled by `receive`.
            Reply::Exported { .. } => {}
        }
    }

    /// The file this tab goes with, when it exists: a circuit's enclosure
    /// design (`true`), or the circuit a design names in `// @circuit`.
    fn linked_file(&self) -> Option<(PathBuf, bool)> {
        if self.doc.is_circuit() {
            let enclosure = self.doc.wiring.as_ref()?.circuit.enclosure.clone()?;
            return enclosure.is_file().then_some((enclosure, true));
        }
        // The toolbar asks every frame: most designs name no circuit.
        if !self.doc.text.contains("@circuit") {
            return None;
        }
        let link = c2f_lang::annotations::read(&self.doc.text).0.circuit?;
        let path = self.doc.directory()?.join(link.path);
        path.is_file().then_some((path, false))
    }

    /// Shows a circuit just read, or none, and watches its part files.
    fn set_circuit(&mut self, found: Option<Arc<c2f_circuit::Circuit>>) {
        // The parts' files and the enclosure's: an edit to one renders again.
        let files: Vec<PathBuf> = found
            .iter()
            .flat_map(|c| c.parts.iter().map(|p| p.path.clone()).chain(c.enclosure.clone()))
            .collect();
        self.watch(files);
        self.doc.wiring = found.map(Wiring::new);
        // The boxes are the last render's, of the circuit before: the
        // render of this one brings its own.
        self.doc.part_boxes.clear();
        // A selection outlives an edit when what it names is still there.
        let valid = |hit: Option<Hit>| match (hit, &self.doc.wiring) {
            (Some(Hit::Pin(end)), Some(w)) => {
                w.circuit.parts.get(end.part).is_some_and(|p| end.pin < p.annotations.pins.len())
            }
            (Some(Hit::Wire(i)), Some(w)) => i < w.circuit.wires.len(),
            (Some(Hit::Part(i)), Some(w)) => i < w.circuit.parts.len(),
            _ => false,
        };
        if !valid(self.doc.selection) {
            self.doc.selection = None;
        }
        self.doc.hover = self.doc.hover.filter(|&h| valid(Some(h)));
        self.update_overlay();
    }

    /// Watches `files` for changes on disk, besides the open file.
    fn watch(&mut self, mut files: Vec<PathBuf>) {
        files.sort();
        files.dedup();
        let known = std::mem::take(&mut self.doc.part_files);
        self.doc.part_files = files
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
        if let (Some(wiring), Some(rewire)) = (&self.doc.wiring, &self.doc.rewire) {
            // The wire being moved fades; the pins at stake stand out.
            let wires = (0..wiring.circuit.wires.len())
                .map(|i| if rewire.grabbed.is_some_and(|(w, _)| w == i) { Emphasis::Dim } else { Emphasis::Normal })
                .collect();
            let strong_pins = std::iter::once(rewire.anchor).chain(rewire.target).collect();
            let error_pins = rewire.target.filter(|_| rewire.error.is_some()).into_iter().collect();
            return Look { wires, strong_pins, error_pins, ..Look::default() };
        }
        let (Some(wiring), Some(selection)) = (&self.doc.wiring, self.doc.selection) else { return Look::default() };
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
        match &self.doc.wiring {
            Some(wiring) => {
                let (wires, pins) = wiring.meshes(&look);
                self.doc.viewer.set_overlay(wires, pins, wiring.bounds());
            }
            None => self.doc.viewer.set_overlay(Default::default(), Default::default(), None),
        }
        if look.parts != self.doc.shown_parts {
            self.doc.viewer.set_part_emphasis(&look.parts);
            self.doc.shown_parts = look.parts;
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
        self.doc.path.as_deref().filter(|p| self.doc.mount.is_for(p))?;
        self.doc.wiring.as_ref().filter(|w| !w.names.is_empty())
    }

    /// The assembly mode's look: the wire on screen picked out, its pins
    /// pulsing, the wires done as they are, the others greyed out.
    fn mount_look(&self) -> Option<Look> {
        if !self.doc.mount.active {
            return None;
        }
        let wiring = self.mount_wiring()?;
        let names = &wiring.names;
        let circuit = &wiring.circuit;
        let step = self.doc.mount.step.min(names.len() - 1);
        let current = &circuit.wires[step];
        let wires = (0..names.len())
            .map(|i| match i {
                _ if i == step => Emphasis::Strong,
                _ if self.doc.mount.is_done(names, i) => Emphasis::Normal,
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
        self.doc.mount.active = active && self.mount_names().is_some();
        if self.doc.mount.active {
            self.doc.selection = None;
            self.go_to_wire(self.doc.mount.step);
        }
        self.update_overlay();
    }

    /// Shows wire `step` of the assembly, the camera turning to its pins.
    fn go_to_wire(&mut self, step: usize) {
        let Some(count) = self.mount_names().map(|n| n.len()) else { return };
        self.doc.mount.go(step, count);
        self.report_mount_problem();
        let Some(wiring) = &self.doc.wiring else { return };
        let wire = &wiring.circuit.wires[self.doc.mount.step];
        let place = |end| glam::Vec3::from(wiring.circuit.place(end).position.map(|c| c as f32));
        let goal = mount::framing(place(wire.from), place(wire.to), &self.doc.viewer.camera, self.doc.viewer.aspect());
        self.doc.viewer.fly_to(goal);
        self.update_overlay();
    }

    fn mount_next(&mut self) {
        if let Some(names) = self.mount_names() {
            self.doc.mount.next(&names);
            self.go_to_wire(self.doc.mount.step);
        }
    }

    fn mount_previous(&mut self) {
        if let Some(names) = self.mount_names() {
            self.doc.mount.previous(names.len());
            self.go_to_wire(self.doc.mount.step);
        }
    }

    fn report_mount_problem(&mut self) {
        if let Some(problem) = self.doc.mount.problem.take() {
            self.doc.console.log(Level::Warning, problem);
        }
    }

    /// The assembly bar, over the top of the view: the instruction, the
    /// wire's place in the order, and the buttons that step through.
    fn mount_bar(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let (Some(wiring), Some(names)) = (&self.doc.wiring, self.mount_names()) else { return };
        let step = self.doc.mount.step.min(names.len() - 1);
        let instruction = mount::instruction(&wiring.circuit, step);
        let label = wiring.circuit.wires[step].label.clone();
        let mut done = self.doc.mount.is_done(&names, step);
        let done_count = self.doc.mount.done_count(&names);
        let (mut previous, mut next, mut restart, mut toggled) = (false, false, false, false);
        egui::Area::new(Id::new("assembly-bar"))
            .fixed_pos(egui::pos2(rect.left() + 12.0, rect.top() + 60.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::palette().veil(200))
                    .stroke(Stroke::new(1.0, theme::palette().line(30)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(rect.width() - 48.0);
                        ui.horizontal(|ui| {
                            toggled = ui.checkbox(&mut done, "").on_hover_text(tr("Done", "Fait")).changed();
                            ui.label(
                                RichText::new(&instruction).strong().size(14.0).color(theme::palette().overlay_text),
                            );
                            if let Some(label) = &label {
                                ui.label(
                                    RichText::new(format!("· {label}"))
                                        .size(13.0)
                                        .color(theme::palette().overlay_text_dim),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            let count = names.len();
                            ui.label(
                                RichText::new(format!("{} / {count}", step + 1))
                                    .strong()
                                    .color(theme::palette().overlay_text),
                            );
                            ui.label(
                                RichText::new(tr!("· {done_count} done", "· {done_count} faits")).color(theme::palette().overlay_text_dim),
                            );
                            ui.add_space(8.0);
                            previous = ui
                                .add_enabled(step > 0, Button::new(tr("◀ Previous", "◀ Précédent")))
                                .on_hover_text(tr("The previous wire (Left arrow)", "Le fil précédent (flèche gauche)"))
                                .clicked();
                            next = ui
                                .button(tr("Next ▶", "Suivant ▶"))
                                .on_hover_text(tr(
                                    "Check this wire off and go to the next (Right arrow)",
                                    "Cocher ce fil et passer au suivant (flèche droite)",
                                ))
                                .clicked();
                            restart = ui
                                .button(tr("Start over", "Recommencer"))
                                .on_hover_text(tr("Uncheck every wire and go back to the first", "Décocher tous les fils et revenir au premier"))
                                .clicked();
                        });
                    });
            });
        if toggled {
            self.doc.mount.set_done(&names, step, done);
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
            self.doc.mount.restart();
            self.report_mount_problem();
            self.go_to_wire(0);
        }
    }

    // ---- Exploded view ----

    /// Moves the parts towards where the exploded view wants them, a bit
    /// each frame, so that they slide apart and back together.
    fn explode(&mut self, ctx: &egui::Context) {
        let doc = &mut self.doc;
        if !doc.viewer.can_explode() {
            doc.exploded = false;
        }
        let target = if doc.exploded { doc.explode_amount } else { 0.0 };
        if doc.explode_shown != target {
            let step = ctx.input(|i| i.stable_dt).min(0.1) * EXPLODE_SPEED;
            let gap = target - doc.explode_shown;
            doc.explode_shown = if gap.abs() <= step { target } else { doc.explode_shown + step * gap.signum() };
            ctx.request_repaint();
        }
        doc.viewer.set_explode(doc.explode_shown);
    }

    // ---- Animation ----

    /// What the code and the values are now: the frames rendered from
    /// something else are out of date.
    fn animation_source(&self) -> (String, Vec<(String, Literal)>) {
        (self.doc.text.clone(), self.doc.customizer.values())
    }

    /// Moves the animation on when a frame is due.
    fn animate(&mut self, ctx: &egui::Context) {
        if !self.doc.animation.playing {
            return;
        }
        let source = self.animation_source();
        let step = self.doc.animation.tick(Instant::now(), self.doc.busy(), &source);
        self.take_step(ctx, step);
    }

    fn take_step(&mut self, ctx: &egui::Context, step: Option<Step>) {
        match step {
            Some(Step::Wait(wait)) => ctx.request_repaint_after(wait),
            Some(Step::Show(scene)) => {
                self.doc.viewer.set_scene(*scene, false);
                ctx.request_repaint();
            }
            Some(Step::Render) => self.render_now(),
            // A reply wakes the window.
            None => {}
        }
    }

    /// Takes what was done in the animation bar.
    fn animation_input(&mut self, ctx: &egui::Context, input: BarInput) {
        let source = self.animation_source();
        let animation = &mut self.doc.animation;
        animation.set_fps(input.fps);
        let mut step = None;
        // The worker drops a render asked for before a newer one.
        if input.steps != animation.steps {
            animation.set_steps(input.steps);
            step = Some(animation.seek(animation.frame, &source));
        } else if input.frame != animation.frame {
            animation.stop();
            step = Some(animation.seek(input.frame, &source));
        }
        if input.toggle {
            if animation.playing {
                animation.stop();
            } else {
                animation.play();
            }
            ctx.request_repaint();
        }
        self.take_step(ctx, step);
    }

    /// Selects something of the circuit, or nothing; a wire also puts the
    /// editor on its line.
    fn select(&mut self, ctx: &egui::Context, hit: Option<Hit>) {
        if hit == self.doc.selection {
            return;
        }
        self.doc.selection = hit;
        if let (Some(Hit::Wire(index)), Some(wiring)) = (hit, &self.doc.wiring) {
            let offset = wiring.circuit.wires[index].span.start as usize;
            self.doc.editor.go_to(ctx, &self.doc.text, offset);
        }
        self.update_overlay();
    }

    // ---- Agent ----

    /// The Agent panel, on the right of the window, following the active
    /// tab; an edit's link shows its line in the code.
    fn agent_panel(&mut self, ctx: &egui::Context) {
        self.agent.set_tab(self.doc.path.as_deref(), self.doc.directory());
        self.agent.undo_offer = self.agent_undo_offer();
        if let Some(line) = self.agent.show(ctx) {
            if self.arrangement == Arrangement::View {
                self.arrangement = Arrangement::Split;
            }
            let offset = agent_panel::line_offset(&self.doc.text, line);
            self.doc.editor.go_to(ctx, &self.doc.text, offset);
        }
        if std::mem::take(&mut self.agent.undo_asked) {
            self.undo_agent_change(ctx);
        }
        self.follow_agent_turn(ctx, self.agent.running());
    }

    // ---- Catalogue ----

    fn catalogue_panel(&mut self, ctx: &egui::Context) {
        if !self.doc.is_circuit() || !self.catalogue_open {
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
                let at = catalogue::free_spot(&self.doc.part_boxes);
                self.add_part(ctx, &entry, at);
            }
            Some(catalogue::Action::Created(path)) => {
                self.doc.console.log(Level::Info, tr!("Made the part {}", "Composant créé : {}", path.display()));
                // In a tab of its own, next to the circuit.
                self.open(ctx, path);
            }
            None => {}
        }
    }

    /// Writes a `[parts.NAME]` table for `entry` at `at` into the circuit,
    /// as one step of the editor's undo history, and renders it.
    fn add_part(&mut self, ctx: &egui::Context, entry: &catalogue::Entry, at: [f64; 2]) {
        if !self.doc.is_circuit() {
            return;
        }
        let name = catalogue::part_name(&entry.name, &catalogue::part_names(&self.doc.text));
        let (text, cursor) = catalogue::insert_part(&self.doc.text, &name, &entry.rel, at);
        self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
        self.doc.console.log(Level::Info, tr!("Added the part `{name}` ({})", "Composant `{name}` ajouté ({})", entry.rel));
        self.render_now();
    }

    // ---- Rewiring ----

    /// A press in the view on a pin, or on a wire near one of its ends,
    /// starts rewiring: before the view handles the press, so that it does
    /// not turn the camera. Not while assembling.
    fn start_rewire(&mut self, ctx: &egui::Context) {
        let (pressed, origin, shift) =
            ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.press_origin(), i.modifiers.shift));
        let (Some(wiring), Some(origin)) = (&self.doc.wiring, origin.filter(|_| pressed)) else { return };
        let slider = self.doc.viewer.layer_view && origin.x > self.view_rect.right() - 60.0;
        // Not through a menu, a popup or the assembly bar.
        let covered = ctx.layer_id_at(origin).is_some_and(|layer| layer.order != egui::Order::Background);
        if shift || self.doc.mount.active || !self.view_rect.contains(origin) || slider || covered {
            return;
        }
        let ray = self.doc.viewer.ray(self.view_rect, origin);
        let grabbed = match part_edit::grab(wiring, &self.doc.part_boxes, &ray) {
            Some(Grab::Pin(end)) => Some((None, end)),
            Some(Grab::WireEnd(index, side)) => {
                let wire = &wiring.circuit.wires[index];
                Some((Some((index, side)), if side == Side::From { wire.to } else { wire.from }))
            }
            Some(Grab::Part(part)) => {
                // A part moves flat, at its own height.
                let circuit = wiring.circuit.clone();
                let drag = part_edit::Drag::start(&circuit, part, &ray);
                self.doc.part_drag = drag.map(|drag| (drag, origin, false, circuit));
                None
            }
            None => None,
        };
        if let Some((grabbed, anchor)) = grabbed {
            self.doc.rewire = Some(Rewire { grabbed, anchor, origin, moved: false, target: None, error: None });
        }
    }

    /// Follows a rewiring drag: the pin under the pointer and what a drop
    /// there would break; on release, the edit. Returns false when the
    /// press turned out to be a click, for the view to handle.
    fn follow_rewire(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        let (Some(wiring), Some(rewire)) = (&self.doc.wiring, &mut self.doc.rewire) else {
            self.doc.rewire = None;
            return false;
        };
        let (pointer, released) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_released()));
        let mut changed = false;
        if let Some(pos) = pointer {
            rewire.moved |= pos.distance(rewire.origin) > 4.0;
            let ray = self.doc.viewer.ray(response.rect, pos);
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
        let Some(rewire) = self.doc.rewire.take() else { return false };
        self.update_overlay();
        // A press and release in place is a click.
        if !rewire.moved {
            return false;
        }
        // Back onto the pin it came from: nothing to do.
        let unchanged = match (&self.doc.wiring, rewire.grabbed) {
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
        let (Some(wiring), Some(rewire)) = (&self.doc.wiring, &self.doc.rewire) else { return };
        if !rewire.moved {
            return;
        }
        let circuit = &wiring.circuit;
        let place = |end: End| glam::Vec3::from(circuit.place(end).position.map(|c| c as f32));
        // Without a pointer (a scripted screenshot), at the pin targeted.
        let held = rewire.target.and_then(|t| self.doc.viewer.to_screen(rect, place(t)));
        let Some(pointer) = pointer.or(held) else { return };
        let color = if rewire.error.is_some() { theme::palette().error } else { theme::palette().accent_strong };
        if let Some(anchor) = self.doc.viewer.to_screen(rect, place(rewire.anchor)) {
            let to = rewire.target.and_then(|t| self.doc.viewer.to_screen(rect, place(t))).unwrap_or(pointer);
            painter.line_segment([anchor, to], Stroke::new(2.0, color));
        }
        let text = match (&rewire.error, rewire.grabbed, rewire.target) {
            (Some(error), _, _) => tr!("{error} (drop anyway to wire it)", "{error} (lâchez quand même pour le relier)"),
            (None, Some(_), Some(to)) => tr!("Move this end to {}", "Déplacer ce bout vers {}", circuit.end_name(to)),
            (None, Some(_), None) => tr("Drop away from a pin to remove this wire", "Lâchez hors d'une broche pour retirer ce fil").to_string(),
            (None, None, Some(to)) => {
                tr!("New wire: {} ➡ {}", "Nouveau fil : {} ➡ {}", circuit.end_name(rewire.anchor), circuit.end_name(to))
            }
            (None, None, None) => tr("Drop on a pin to wire it", "Lâchez sur une broche pour la relier").to_string(),
        };
        let color = if rewire.error.is_some() { theme::palette().error } else { theme::palette().overlay_text };
        inspect::paint_bubble_in(painter, rect, pointer, &text, color);
    }

    /// Writes a rewiring into the circuit's text, as one step of the
    /// editor's undo history, and renders it.
    fn apply_rewire(&mut self, ctx: &egui::Context, edit: Edit) {
        let Some(wiring) = &self.doc.wiring else { return };
        let circuit = &wiring.circuit;
        let done = match edit {
            Edit::Move { to, .. } => tr!("Moved a wire's end to {}", "Bout de fil déplacé vers {}", circuit.end_name(to)),
            Edit::Add { from, to } => tr!("Added a wire: {} ➡ {}", "Fil ajouté : {} ➡ {}", circuit.end_name(from), circuit.end_name(to)),
            Edit::Remove { wire } => {
                let w = &circuit.wires[wire];
                tr!("Removed the wire {} ➡ {}", "Fil retiré : {} ➡ {}", circuit.end_name(w.from), circuit.end_name(w.to))
            }
        };
        let fresh = self.doc.sent_source.as_deref() == Some(self.doc.text.as_str());
        let result = if fresh {
            rewire::apply(&self.doc.text, circuit, edit)
        } else {
            Err(tr(
                "the code changed since the last render: try again once it is rendered",
                "le code a changé depuis le dernier rendu : réessayez une fois le rendu fait",
            )
            .to_string())
        };
        match result {
            Ok((text, cursor)) => {
                if matches!(edit, Edit::Remove { .. }) {
                    self.doc.selection = None;
                }
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                self.doc.console.log(Level::Info, done);
                self.render_now();
            }
            Err(why) => self.doc.console.log(Level::Warning, tr!("Cannot rewire: {why}", "Impossible de recâbler : {why}")),
        }
    }

    // ---- Moving parts ----

    /// Follows a part's drag: the part and its wires where the pointer
    /// puts it, R turning it; on release, the edit. Returns false when the
    /// press turned out to be a click.
    fn follow_part_drag(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        let Some((drag, origin, moved, original)) = &mut self.doc.part_drag else { return false };
        let (pointer, released, fine, turn) = ctx.input(|i| {
            (i.pointer.interact_pos(), i.pointer.primary_released(), i.modifiers.shift, i.key_pressed(Key::R))
        });
        if turn {
            drag.turns += 1;
        }
        let mut changed = false;
        if let Some(pos) = pointer {
            *moved |= pos.distance(*origin) > 4.0;
            let step = if fine { part_edit::FINE_SNAP } else { part_edit::SNAP };
            changed = *moved && drag.follow(&self.doc.viewer.ray(response.rect, pos), step);
        }
        if changed {
            let (drag, original) = (drag.clone(), original.clone());
            self.preview_part(&drag, &original);
        }
        if !released {
            ctx.request_repaint();
            return true;
        }
        let Some((drag, _, moved, original)) = self.doc.part_drag.take() else { return false };
        if !moved && drag.turns == 0 {
            return false;
        }
        let at = drag.moved().then_some(drag.at);
        if !self.place_part(ctx, &original, drag.part, at, drag.turns) {
            // Back where it was.
            self.preview_part(&part_edit::Drag { at: drag.from, ..drag }, &original);
        }
        true
    }

    /// Shows a dragged part, and its wires, where the drag has it.
    fn preview_part(&mut self, drag: &part_edit::Drag, original: &Arc<c2f_circuit::Circuit>) {
        let delta = std::array::from_fn::<f32, 3, _>(|i| (drag.at[i] - drag.from[i]) as f32);
        self.doc.viewer.preview_part_move(drag.part, glam::Vec3::from(delta));
        let mut moved = (**original).clone();
        moved.parts[drag.part].at = drag.at;
        self.doc.wiring = Some(Wiring::new(Arc::new(moved)));
        self.update_overlay();
    }

    /// Writes part `part`'s new place and turns into the circuit's text, as
    /// one step of the editor's undo history, and renders it.
    fn place_part(
        &mut self,
        ctx: &egui::Context,
        circuit: &c2f_circuit::Circuit,
        part: usize,
        at: Option<[f64; 3]>,
        turns: u32,
    ) -> bool {
        let Some(instance) = circuit.parts.get(part) else { return false };
        let fresh = self.doc.sent_source.as_deref() == Some(self.doc.text.as_str());
        let result = if fresh {
            part_edit::place(&self.doc.text, instance, at, turns)
        } else {
            Err(tr(
                "the code changed since the last render: try again once it is rendered",
                "le code a changé depuis le dernier rendu : réessayez une fois le rendu fait",
            )
            .to_string())
        };
        match result {
            Ok((text, cursor)) => {
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                let mut done = Vec::new();
                if let Some(at) = at {
                    done.push(tr!("moved `{}` to [{}, {}]", "`{}` déplacé en [{}, {}]", instance.name, at[0], at[1]));
                }
                if turns > 0 {
                    done.push(tr!("turned `{}` by {}°", "`{}` tourné de {}°", instance.name, 90 * turns));
                }
                let done = done.join(", ");
                let mut chars = done.chars();
                let done: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
                self.doc.console.log(Level::Info, done);
                self.render_now();
                true
            }
            Err(why) => {
                self.doc.console.log(Level::Warning, tr!("Cannot move the part: {why}", "Impossible de déplacer le composant : {why}"));
                false
            }
        }
    }

    /// Removes part `part` and the wires to it, as one step of the
    /// editor's undo history.
    fn remove_part(&mut self, ctx: &egui::Context, part: usize) {
        let Some(wiring) = &self.doc.wiring else { return };
        let circuit = wiring.circuit.clone();
        let fresh = self.doc.sent_source.as_deref() == Some(self.doc.text.as_str());
        let result = if fresh {
            part_edit::remove(&self.doc.text, &circuit, part)
        } else {
            Err(tr(
                "the code changed since the last render: try again once it is rendered",
                "le code a changé depuis le dernier rendu : réessayez une fois le rendu fait",
            )
            .to_string())
        };
        match result {
            Ok((text, cursor, wires)) => {
                self.doc.selection = None;
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                let name = &circuit.parts[part].name;
                let what = match wires {
                    0 => tr!("Removed `{name}`", "`{name}` retiré"),
                    1 => tr!("Removed `{name}` and its wire", "`{name}` retiré avec son fil"),
                    n => tr!("Removed `{name}` and its {n} wires", "`{name}` retiré avec ses {n} fils"),
                };
                self.doc.console.log(Level::Info, what);
                self.render_now();
            }
            Err(why) => self.doc.console.log(Level::Warning, tr!("Cannot remove the part: {why}", "Impossible de retirer le composant : {why}")),
        }
    }

    /// Hovering, clicks and Escape over the view of a circuit.
    fn circuit_input(&mut self, ctx: &egui::Context, response: &egui::Response) {
        if self.doc.rewire.is_some() && self.follow_rewire(ctx, response) {
            self.doc.hover = None;
            return;
        }
        if self.doc.part_drag.is_some() && self.follow_part_drag(ctx, response) {
            self.doc.hover = None;
            return;
        }
        // Delete or Backspace removes the wire or the part selected, and R
        // turns the part selected.
        let idle = !self.doc.mount.active && ctx.memory(|m| m.focused().is_none());
        let (delete, turn) =
            ctx.input(|i| (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace), i.key_pressed(Key::R)));
        if delete && idle {
            match self.doc.selection {
                Some(Hit::Wire(wire)) => self.apply_rewire(ctx, Edit::Remove { wire }),
                Some(Hit::Part(part)) => self.remove_part(ctx, part),
                _ => {}
            }
        }
        if turn && idle {
            if let (Some(Hit::Part(part)), Some(wiring)) = (self.doc.selection, &self.doc.wiring) {
                let circuit = wiring.circuit.clone();
                self.place_part(ctx, &circuit, part, None, 1);
            }
        }
        let Some(wiring) = &self.doc.wiring else { return };
        if !response.dragged() {
            self.doc.hover = response.hover_pos().and_then(|pos| {
                pick::pick(wiring, &self.doc.part_boxes, &self.doc.viewer.ray(response.rect, pos))
            });
        }
        if response.double_clicked() {
            if let Some(Hit::Part(index)) = self.doc.hover {
                // In a tab of its own: the circuit stays open in its tab.
                let path = wiring.circuit.parts[index].path.clone();
                self.doc.hover = None;
                self.open(ctx, path);
                return;
            }
        }
        if response.clicked() {
            if self.doc.mount.active {
                // Assembling, a click on a wire goes to its step.
                if let Some(Hit::Wire(index)) = self.doc.hover {
                    self.go_to_wire(index);
                }
            } else {
                self.select(ctx, self.doc.hover);
            }
        }
        let escape = ctx.input(|i| i.key_pressed(Key::Escape)) && ctx.memory(|m| m.focused().is_none());
        if escape && self.doc.selection.is_some() {
            self.select(ctx, None);
        }
    }

    // ---- Files ----

    /// Opens the file at `path` in a tab: its own tab when it is open
    /// already, the active one when that is a blank untitled tab, a new
    /// one otherwise.
    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        let path = absolute(&path);
        if let Some(index) = self.tab_of(&path) {
            self.select_tab(ctx, index);
            return;
        }
        if insert::is_mesh(&path) {
            self.open_mesh(ctx, &path);
            return;
        }
        match c2f_eval::read_source(&path) {
            Ok(text) => {
                if !self.doc.is_blank() {
                    self.add_tab();
                }
                self.load(path, text);
                self.doc.editor.focus(ctx);
            }
            Err(err) => self.doc.console.log(Level::Error, tr!("Cannot open {}: {err}", "Impossible d'ouvrir {} : {err}", path.display())),
        }
    }

    /// Opens a mesh as an untitled design that imports it, to look at it
    /// and build on it.
    fn open_mesh(&mut self, ctx: &egui::Context, path: &Path) {
        if !path.is_file() {
            self.doc.console.log(Level::Error, tr!("Cannot open {}: no such file", "Impossible d'ouvrir {} : fichier introuvable", path.display()));
            return;
        }
        if !self.doc.is_blank() {
            self.add_tab();
        }
        let text = insert::mesh_design(path, &c2f_geom::mesh_objects(path));
        // Nothing to save until it is changed.
        self.doc.text = text.clone();
        self.doc.saved_text = text;
        self.doc.console.log(Level::Info, tr!("Opened {}", "Ouvert : {}", path.display()));
        self.remember(path);
        self.doc.frame_next = true;
        self.render_now();
        self.doc.editor.focus(ctx);
    }

    /// Asks for a mesh and imports it in the design, on the cursor's line.
    fn insert_mesh_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Insert a mesh", "Insérer un maillage"))
            .add_filter(tr("Meshes", "Maillages"), insert::MESH_EXTENSIONS);
        if let Some(dir) = self.doc.directory() {
            dialog = dialog.set_directory(dir);
        }
        let Some(file) = dialog.pick_file() else { return };
        let reference = insert::reference(&file, self.doc.path.as_deref());
        let cursor = self.doc.editor.cursor_char().unwrap_or_else(|| self.doc.text.chars().count());
        let (text, cursor) = insert::insert_mesh(&self.doc.text, cursor, &reference);
        self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
        self.doc.console.log(Level::Info, tr!("Imported {}", "Importé : {}", reference));
        self.render_now();
    }

    /// Asks for a design and brings it in at the top of this one: its
    /// modules and functions (`use`), or all of it (`include`, `whole`).
    fn insert_design_dialog(&mut self, ctx: &egui::Context, whole: bool) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Insert a design", "Insérer un design"))
            .add_filter(tr("Code2Form or SCAD", "Code2Form ou SCAD"), insert::DESIGN_EXTENSIONS);
        if let Some(dir) = self.doc.directory() {
            dialog = dialog.set_directory(dir);
        }
        let Some(file) = dialog.pick_file() else { return };
        let reference = insert::reference(&file, self.doc.path.as_deref());
        match insert::insert_design(&self.doc.text, &reference, whole) {
            Some((text, cursor)) => {
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                self.render_now();
            }
            None => self.doc.console.log(Level::Info, tr!("{} is already in the design", "{} est déjà dans le design", reference)),
        }
    }

    /// The Insert menu: a mesh, or another design.
    fn insert_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let enabled = !self.doc.is_circuit();
        let menu = ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button(tr("Insert", "Insérer"), |ui| {
                let mesh = ui.button(tr("Mesh…", "Maillage…")).on_hover_text(tr(
                    "Import an STL, 3MF, OBJ, OFF or AMF file (STEP with FreeCAD) on the cursor's line",
                    "Importer un fichier STL, 3MF, OBJ, OFF ou AMF (STEP avec FreeCAD) sur la ligne du curseur",
                ));
                if mesh.clicked() {
                    ui.close();
                    self.insert_mesh_dialog(ctx);
                }
                ui.separator();
                let modules = ui.button(tr("Design's Modules (use)…", "Modules d'un design (use)…")).on_hover_text(tr(
                    "Use another design's modules and functions, without its model",
                    "Utiliser les modules et fonctions d'un autre design, sans son modèle",
                ));
                if modules.clicked() {
                    ui.close();
                    self.insert_design_dialog(ctx, false);
                }
                let whole = ui.button(tr("Whole Design (include)…", "Design entier (include)…")).on_hover_text(tr(
                    "Include another design, its model with it",
                    "Inclure un autre design, avec son modèle",
                ));
                if whole.clicked() {
                    ui.close();
                    self.insert_design_dialog(ctx, true);
                }
            })
        });
        menu.response.on_disabled_hover_text(tr("For designs, not circuits", "Pour les designs, pas les circuits"));
    }

    /// Puts the file at `path`, as read, in the active tab.
    fn load(&mut self, path: PathBuf, text: String) {
        self.doc.text = text.clone();
        self.doc.saved_text = text;
        self.doc.disk_mtime = modified(&path);
        self.doc.console.log(Level::Info, tr!("Opened {}", "Ouvert : {}", path.display()));
        self.remember(&path);
        // A circuit has no parameters, nor presets next to it.
        self.doc.customizer.open(Some(path.as_path()).filter(|p| !circuit::is_circuit(p)));
        self.doc.mount.open(Some(path.as_path()).filter(|p| circuit::is_circuit(p)));
        self.report_mount_problem();
        self.set_circuit(None);
        self.doc.path = Some(path);
        self.doc.frame_next = true;
        self.doc.editor.error_line = None;
        self.render_now();
    }

    /// Opens a new untitled tab.
    fn new_tab(&mut self, ctx: &egui::Context) {
        self.add_tab();
        self.doc.console.log(Level::Info, tr("New file", "Nouveau fichier"));
        self.render_now();
        self.doc.editor.focus(ctx);
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Open a design", "Ouvrir un fichier"))
            .add_filter(tr("Code2Form, SCAD or circuit", "Code2Form, SCAD ou circuit"), &["c2f", "scad", "c2fc"])
            .add_filter(tr("Meshes (STL, 3MF, OBJ, STEP…)", "Maillages (STL, 3MF, OBJ, STEP…)"), insert::MESH_EXTENSIONS)
            .add_filter(tr("All files", "Tous les fichiers"), &["*"]);
        if let Some(dir) = self.doc.directory() {
            dialog = dialog.set_directory(dir);
        }
        for path in dialog.pick_files().unwrap_or_default() {
            self.open(ctx, path);
        }
    }

    /// Opens a file from Open Recent, or drops it from the list when it is
    /// gone.
    fn open_recent(&mut self, ctx: &egui::Context, path: PathBuf) {
        if path.exists() {
            self.open(ctx, path);
            return;
        }
        self.doc.console.log(Level::Warning, tr!("{} no longer exists; removed from Open Recent", "{} n'existe plus ; retiré des récents", path.display()));
        self.recent.remove(&path);
        self.save_recent();
    }

    fn remember(&mut self, path: &Path) {
        self.recent.push(path);
        self.save_recent();
    }

    fn save_recent(&mut self) {
        if let Err(err) = self.recent.save() {
            self.doc.console.log(Level::Warning, tr!("Cannot save the recent files list: {err}", "Impossible d'enregistrer la liste des récents : {err}"));
        }
    }

    /// Saves; returns false when cancelled or failed.
    fn save(&mut self) -> bool {
        match self.doc.path.clone() {
            Some(path) => self.write_to(path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> bool {
        // Designs are .c2f files; a .scad file keeps its extension by default.
        let scad = self.doc.path.as_deref().and_then(Path::extension).is_some_and(|e| e == "scad");
        let filters = if self.doc.is_circuit() {
            [circuit_files(), design_files(), scad_files()]
        } else if scad {
            [scad_files(), design_files(), circuit_files()]
        } else {
            [design_files(), scad_files(), circuit_files()]
        };
        let mut dialog = rfd::FileDialog::new().set_title(tr("Save as", "Enregistrer sous"));
        for (name, extension) in filters {
            dialog = dialog.add_filter(name, &[extension]);
        }
        let name = if self.doc.path.is_some() { self.doc.file_name() } else { "untitled.c2f".to_string() };
        dialog = dialog.set_file_name(name);
        if let Some(dir) = self.doc.directory() {
            dialog = dialog.set_directory(dir);
        }
        match dialog.save_file() {
            Some(path) => self.write_to(path),
            None => false,
        }
    }

    fn write_to(&mut self, path: PathBuf) -> bool {
        let path = absolute(&path);
        match std::fs::write(&path, &self.doc.text) {
            Ok(()) => {
                self.doc.saved_text = self.doc.text.clone();
                self.doc.disk_mtime = modified(&path);
                self.doc.console.log(Level::Info, tr!("Saved {}", "Enregistré : {}", path.display()));
                self.remember(&path);
                if circuit::is_circuit(&path) {
                    self.doc.customizer.open(None);
                    if self.doc.path.as_deref() != Some(path.as_path()) {
                        self.doc.mount.moved(&path);
                        self.report_mount_problem();
                    }
                } else {
                    self.doc.mount.open(None);
                    self.doc.customizer.moved(&path);
                    self.set_circuit(None);
                }
                self.doc.path = Some(path);
                self.render_now();
                true
            }
            Err(err) => {
                self.doc.console.log(Level::Error, tr!("Cannot save {}: {err}", "Impossible d'enregistrer {} : {err}", path.display()));
                false
            }
        }
    }

    /// The format the Export shortcut writes the model in: the settings'
    /// for a 3D model, SVG for a 2D one.
    fn shortcut_format(&self) -> Format {
        match self.doc.model.as_ref().map(|m| m.formats) {
            Some(formats) if formats == Format::FLAT => Format::Svg,
            _ => self.settings.export_format,
        }
    }

    /// With autosave on, saves the active tab once typing has paused for
    /// `AUTOSAVE_DELAY`, when it has a file.
    fn autosave(&mut self, ctx: &egui::Context) {
        if !self.settings.autosave || self.doc.path.is_none() || !self.doc.is_dirty() {
            self.doc.autosave_deadline = None;
            return;
        }
        let now = Instant::now();
        let deadline = *self.doc.autosave_deadline.get_or_insert(now + AUTOSAVE_DELAY);
        // Not in the middle of a drag in the view, nor of a question.
        if now < deadline || ctx.input(|i| i.pointer.any_down()) || self.confirm.is_some() {
            ctx.request_repaint_after(deadline.saturating_duration_since(now).max(Duration::from_millis(50)));
            return;
        }
        self.autosave_now();
    }

    /// With autosave on, saves the active tab's edits now, quietly, when
    /// it has a file.
    fn autosave_now(&mut self) {
        self.doc.autosave_deadline = None;
        if !self.settings.autosave || !self.doc.is_dirty() {
            return;
        }
        let Some(path) = self.doc.path.clone() else { return };
        match std::fs::write(&path, &self.doc.text) {
            Ok(()) => {
                self.doc.saved_text = self.doc.text.clone();
                self.doc.disk_mtime = modified(&path);
            }
            Err(err) => {
                let message = tr!("Cannot save {}: {}", "Impossible d'enregistrer {} : {}", path.display(), err);
                self.doc.console.log(Level::Error, message);
                // Not again at every frame: the next edit tries again.
                self.doc.autosave_deadline = Some(Instant::now() + Duration::from_secs(3600));
            }
        }
    }

    /// With autosave on, saves every tab with a file, as when quitting.
    fn autosave_all(&mut self) {
        if !self.settings.autosave {
            return;
        }
        for index in 0..self.shelf.len() {
            self.in_tab(index, |app| app.autosave_now());
        }
    }

    fn export(&mut self, format: Format) {
        self.export_as(format, None);
    }

    /// Exports the outline of a 3D model seen from the view, as DXF.
    fn export_view(&mut self) {
        let camera = &self.doc.viewer.camera;
        let view = (f64::from(camera.yaw.to_degrees()), f64::from(camera.pitch.to_degrees()));
        self.export_as(Format::Dxf, Some(view));
    }

    /// Asks where to save the sheet of views of the model, then has the
    /// worker draw it.
    fn export_sheet(&mut self) {
        let stem = self.doc.path.as_deref().and_then(Path::file_stem).map_or_else(|| "model".into(), |s| s.to_string_lossy());
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Save the sheet of views", "Enregistrer la planche de vues"))
            .add_filter("PNG", &["png"])
            .set_file_name(format!("{stem}-views.png"));
        let folder = self.settings.export_folder.clone().filter(|folder| folder.is_dir());
        if let Some(dir) = folder.or_else(|| self.doc.directory()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(target) = dialog.save_file() {
            // As for an export: this tab's model first.
            if self.exportable != Some(self.doc.id) {
                self.render_now();
            }
            self.worker.sheet(target);
        }
    }

    /// Asks where to write the model in `format`, then has the worker
    /// write it; a DXF of a 3D model is its outline seen from `view`.
    fn export_as(&mut self, format: Format, view: Option<(f64, f64)>) {
        let formats = self.doc.model.as_ref().map_or(&[][..], |m| m.formats);
        if view.is_some() && formats == Format::SOLID {
            // A 3D model drawn from the view.
        } else if !formats.contains(&format) {
            let why = match formats.first() {
                None => tr("the model is empty", "le modèle est vide").to_string(),
                Some(other) if Format::FLAT.contains(other) => {
                    tr("the model is 2D; export it as SVG or DXF", "le modèle est en 2D ; exportez-le en SVG ou DXF")
                        .to_string()
                }
                Some(_) => {
                    tr(
                        "the model is 3D; export it as STL, 3MF, glTF, OBJ or STEP, or its outline as the DXF of the view",
                        "le modèle est en 3D ; exportez-le en STL, 3MF, glTF, OBJ ou STEP, ou son contour avec le DXF de la vue",
                    )
                        .to_string()
                }
            };
            self.doc.console.log(Level::Warning, tr!("Nothing to export as {}: {why}", "Rien à exporter en {} : {why}", format.name()));
            return;
        }
        let stem = self
            .doc
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "model".to_string(), |s| s.to_string_lossy().into_owned());
        let (name, extension) = (format.name(), format.extension());
        let title = if view.is_some() {
            tr("Export the view as DXF", "Exporter la vue en DXF").to_string()
        } else {
            tr!("Export {name}", "Exporter en {name}")
        };
        let mut dialog =
            rfd::FileDialog::new().set_title(title).add_filter(name, &[extension]).set_file_name(format!("{stem}.{extension}"));
        // The folder picked in the settings, while it exists.
        let folder = self.settings.export_folder.clone().filter(|folder| folder.is_dir());
        if let Some(dir) = folder.or_else(|| self.doc.directory()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(target) = dialog.save_file() {
            // The worker keeps the model of the last design rendered, maybe
            // another tab's: this one's comes first.
            if self.exportable != Some(self.doc.id) {
                self.render_now();
            }
            let [r, g, b] = self.settings.model_color.map(|c| f32::from(c) / 255.0);
            let animation = c2f_geom::GltfAnimation {
                duration: self.settings.gltf_duration,
                fps: f64::from(self.settings.gltf_fps),
                looped: self.settings.gltf_loop,
                ..Default::default()
            };
            let gltf = c2f_geom::GltfOptions {
                scale: self.settings.gltf_scale,
                color: [r, g, b],
                animation: Some(animation),
                ..Default::default()
            };
            let options = ExportOptions { binary_stl: self.settings.binary_stl, gltf, view, ..Default::default() };
            self.worker.export(target, format, options);
        }
    }

    /// Checks every tab's files on disk, once a second.
    fn poll_disk(&mut self, ctx: &egui::Context) {
        if self.shelf.iter(&self.doc).all(|tab| tab.path.is_none()) {
            return;
        }
        ctx.request_repaint_after(POLL);
        if self.last_poll.elapsed() < POLL {
            return;
        }
        self.last_poll = Instant::now();
        let active = self.shelf.active();
        for index in 0..self.shelf.len() {
            self.in_tab(index, |app| app.poll_tab(index == active));
        }
    }

    /// Reloads the file when it changed on disk and has no unsaved edits,
    /// and renders a circuit again when one of its part files changed: at
    /// once when the tab is `shown`, once it is shown otherwise.
    fn poll_tab(&mut self, shown: bool) {
        let Some(path) = self.doc.path.clone() else { return };
        let mut changed_parts = Vec::new();
        for (file, mtime) in &mut self.doc.part_files {
            let now = modified(file);
            if now != *mtime {
                *mtime = now;
                changed_parts.push(file.clone());
            }
        }
        if !changed_parts.is_empty() {
            self.doc.animation.forget();
            for file in &changed_parts {
                self.doc.console.log(Level::Info, tr!("Rendering again: {} changed on disk", "Nouveau rendu : {} modifié sur le disque", file.display()));
            }
            self.render_when_shown(shown);
        }
        let Some(mtime) = modified(&path) else { return };
        if self.doc.disk_mtime == Some(mtime) {
            return;
        }
        self.doc.disk_mtime = Some(mtime);
        if self.doc.is_dirty() {
            let message = tr!("{} changed on disk; keeping your unsaved edits", "{} modifié sur le disque ; vos modifications non enregistrées sont gardées", path.display());
            self.doc.console.log(Level::Warning, message);
            return;
        }
        match c2f_eval::read_source(&path) {
            Ok(text) if text != self.doc.text => {
                self.doc.text = text.clone();
                self.doc.saved_text = text;
                self.doc.console.log(Level::Info, tr!("Reloaded {} (changed on disk)", "Rechargé : {} (modifié sur le disque)", path.display()));
                self.render_when_shown(shown);
            }
            Ok(_) => {}
            Err(err) => self.doc.console.log(Level::Error, tr!("Cannot reload {}: {err}", "Impossible de recharger {} : {err}", path.display())),
        }
    }

    /// Goes on quitting once the active tab's unsaved edits were saved or
    /// given up: asks about the next tab's, or closes the window.
    fn quit(&mut self, ctx: &egui::Context) {
        self.quit_cleared.push(self.doc.id);
        match self.next_unsaved() {
            Some(index) => {
                self.select_tab(ctx, index);
                self.confirm = Some(Pending::Quit);
            }
            None => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    // ---- Input ----

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            self.autosave_all();
            if let Some(index) = self.next_unsaved() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.select_tab(ctx, index);
                self.confirm = Some(Pending::Quit);
            }
        }
        // The question is about the active tab: it stays until answered.
        if self.confirm.is_some() {
            return;
        }
        // The keys go to the shortcut being recorded, and to nothing else.
        if self.recording.is_some() {
            self.record_keys(ctx);
            return;
        }
        let actions = ctx.input_mut(|i| self.settings.shortcuts.take(i));
        let (mut zoom_steps, mut zoom_reset) = (0, false);
        for action in actions {
            // Closing a modified tab asks first: what follows waits.
            if self.confirm.is_some() {
                break;
            }
            match action {
                Action::New => self.new_tab(ctx),
                Action::Open => self.open_dialog(ctx),
                Action::Save => {
                    self.save();
                }
                Action::SaveAs => {
                    self.save_as();
                }
                Action::CloseTab => self.close_tab(ctx, self.shelf.active()),
                Action::NextTab | Action::PreviousTab => {
                    let forward = action == Action::NextTab;
                    self.select_tab(ctx, tabs::step(self.shelf.active(), self.shelf.len(), forward));
                }
                Action::Render => self.render_now(),
                Action::Export => self.export(self.shortcut_format()),
                Action::Find | Action::FindReplace => {
                    self.doc.editor.find(FindRequest::Open { replace: action == Action::FindReplace });
                }
                Action::FindNext | Action::FindPrevious => {
                    self.doc.editor.find(FindRequest::Step { forward: action == Action::FindNext });
                }
                Action::ZoomIn => zoom_steps += 1,
                Action::ZoomOut => zoom_steps -= 1,
                Action::ZoomReset => zoom_reset = true,
                Action::Agent => self.agent.toggle(),
                Action::UndoAgent => self.undo_agent_change(ctx),
                Action::Settings => self.settings_open = true,
            }
        }
        let size = if zoom_reset {
            Some(1.0)
        } else {
            (zoom_steps != 0).then(|| zoom_step(self.settings.interface_size, zoom_steps))
        };
        if let Some(size) = size.filter(|&size| size != self.settings.interface_size) {
            self.settings.interface_size = size;
            ctx.set_zoom_factor(size);
        }

        // Cmd+Z in the editor or a text field is theirs. Elsewhere, as after
        // an edit made in the 3D view, it undoes the file's last edit.
        if ctx.memory(|m| m.focused().is_none()) {
            // Shift+Cmd+Z first: Cmd+Z also matches it.
            let [redo, undo] = ctx.input_mut(|i| [REDO, UNDO].map(|s| i.consume_shortcut(&s)));
            if (undo || redo) && self.doc.editor.undo_step(ctx, &mut self.doc.text, undo) {
                self.render_now();
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for path in dropped {
            self.open(ctx, path);
        }
    }

    /// Gives the action being recorded the keys pressed, or says why not;
    /// Escape gives up. Meanwhile no key reaches anything else.
    fn record_keys(&mut self, ctx: &egui::Context) {
        let Some(action) = self.recording else { return };
        let pressed = ctx.input_mut(|i| {
            let mut pressed = None;
            i.events.retain(|event| match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    pressed = pressed.or(Some((*key, *modifiers)));
                    false
                }
                egui::Event::Key { .. }
                | egui::Event::Text(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Paste(_) => false,
                _ => true,
            });
            pressed
        });
        match pressed {
            Some((Key::Escape, modifiers)) if modifiers.is_none() => self.stop_recording(),
            Some((key, modifiers)) => match shortcuts::record(modifiers, key) {
                Ok(keys) => {
                    self.settings.shortcuts.set(action, keys);
                    self.stop_recording();
                }
                Err(why) => self.record_problem = Some(why),
            },
            None => {}
        }
    }

    fn stop_recording(&mut self) {
        self.recording = None;
        self.record_problem = None;
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!("{}{} — Code2Form", if self.doc.is_dirty() { "● " } else { "" }, self.doc.file_name());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    // ---- Panels ----

    /// The bar across the top: the app's icon and the File menu, the open
    /// files' tabs, the layout switch in the middle, and on the right,
    /// Render, Export and the settings.
    fn header(&mut self, ctx: &egui::Context) {
        let keys = self.settings.shortcuts.clone();
        let palette = theme::palette();
        let names = tabs::names(&self.shelf.iter(&self.doc).map(|tab| tab.path.as_deref()).collect::<Vec<_>>());
        let tabs: Vec<(u64, String, bool, Option<String>)> = self
            .shelf
            .iter(&self.doc)
            .zip(names)
            .map(|(tab, name)| (tab.id, name, tab.is_dirty(), tab.path.as_ref().map(|p| p.display().to_string())))
            .collect();
        let active = self.shelf.active();
        let (mut select, mut close, mut new) = (None, None, false);
        let frame = Frame::new().fill(palette.bg).inner_margin(Margin { left: 10, right: 10, top: 6, bottom: 6 });
        egui::TopBottomPanel::top("header").frame(frame).show_separator_line(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Snug buttons: room for the longer French labels.
                ui.spacing_mut().button_padding.x = 8.0;
                let full = ui.available_rect_before_wrap();
                ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(18.0)));
                // The app's name beside its icon, while there is room for it.
                if full.width() > 1000.0 {
                    ui.label(RichText::new("Code2Form").strong().color(palette.text).size(13.5));
                }
                self.file_menu(ui, ctx, &keys);
                toolbar_separator(ui);

                // The layout switch sits in the middle, unless the buttons on
                // the right need its place; the tabs fill the room left of it
                // and scroll when there are more.
                let switch_left = (full.center().x - self.header_switch / 2.0)
                    .min(full.right() - self.header_right - 12.0 - self.header_switch);
                let room = (switch_left - 16.0 - ui.cursor().min.x).max(80.0);
                egui::ScrollArea::horizontal()
                    .id_salt("header-tabs")
                    .max_width(room)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (index, (id, name, dirty, path)) in tabs.iter().enumerate() {
                                match tab(ui, *id, name, index == active, *dirty) {
                                    TabClick::Select => select = Some(index),
                                    TabClick::Close => close = Some(index),
                                    TabClick::None(response) => {
                                        if let Some(path) = path {
                                            response.on_hover_text(path);
                                        }
                                    }
                                }
                            }
                            let shortcut = shortcuts::describe(ctx, &keys.get(Action::New));
                            let plus = ui.add(Button::new(RichText::new("+").size(15.0).color(palette.text_dim)).frame(false));
                            let hint = tr!("New file in a new tab ({shortcut})", "Nouveau fichier dans un nouvel onglet ({shortcut})");
                            new = plus.on_hover_text(hint).clicked();
                        });
                    });
                ui.add_space((switch_left - ui.cursor().min.x).max(0.0));
                let switch = ui.scope(|ui| {
                    if let Some(arrangement) = layout_switch(ui, self.arrangement) {
                        self.arrangement = arrangement;
                    }
                });
                self.header_switch = switch.response.rect.width();

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let shortcut = shortcuts::describe(ctx, &keys.get(Action::Settings));
                    if icons::button(ui, Icon::Settings, self.settings_open, &tr!("Settings ({shortcut})", "Réglages ({shortcut})")).clicked() {
                        self.settings_open = true;
                    }
                    let shortcut = shortcuts::describe(ctx, &keys.get(Action::Agent));
                    let hint = tr!("Talk to the agent ({shortcut})", "Parler à l'agent ({shortcut})");
                    if icons::button(ui, Icon::Agent, self.agent.open, &hint).clicked() {
                        self.agent.toggle();
                    }
                    self.export_menu(ui, ctx, &keys);
                    let render = Button::new(RichText::new(tr("Render", "Rendu")).color(palette.on_accent).strong())
                        .fill(palette.accent)
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(0.0, 26.0));
                    let shortcut = shortcuts::describe(ctx, &keys.get(Action::Render));
                    if ui.add(render).on_hover_text(tr!("Render now ({shortcut})", "Lancer le rendu ({shortcut})")).clicked() {
                        self.render_now();
                    }
                    if let Some((linked, is_part)) = self.linked_file() {
                        let (label, hint) = if is_part {
                            (tr("Part", "Pièce"), tr("Open the design this circuit sits in", "Ouvrir le design dans lequel ce circuit s'intègre"))
                        } else {
                            (tr("Electronics", "Électronique"), tr("Open this design's circuit: its parts and wires", "Ouvrir le circuit de ce design : ses composants et ses fils"))
                        };
                        if ui.button(label).on_hover_text(format!("{hint}\n{}", linked.display())).clicked() {
                            self.open(ctx, linked);
                        }
                    }
                    if self.doc.is_circuit() {
                        toolbar_separator(ui);
                        let button = egui::Button::selectable(self.doc.mount.active, tr("Assembly", "Montage"));
                        let toggle = ui.add_enabled(self.mount_names().is_some(), button);
                        let hint = tr("Wire the circuit one wire at a time, in the file's order", "Câbler le circuit fil par fil, dans l'ordre du fichier");
                        if toggle.on_hover_text(hint).on_disabled_hover_text(tr("The circuit has no wires", "Le circuit n'a aucun fil")).clicked() {
                            self.set_mount(!self.doc.mount.active);
                        }
                        let toggle = ui.toggle_value(&mut self.catalogue_open, "Catalogue");
                        toggle.on_hover_text(tr(
                            "The parts of the library and your own, to add to the circuit",
                            "Les composants de la bibliothèque et les vôtres, à ajouter au circuit",
                        ));
                    }
                    self.header_right = ui.min_rect().width();
                });
            });
        });
        if let Some(index) = close {
            self.close_tab(ctx, index);
        } else if let Some(index) = select {
            self.select_tab(ctx, index);
        }
        if new {
            self.new_tab(ctx);
        }
    }

    /// The File menu: new, open, the recent files and the examples, what to
    /// insert in the design, save, and the settings.
    fn file_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, keys: &Shortcuts) {
        ui.menu_button(tr("File", "Fichier"), |ui| {
            ui.set_min_width(230.0);
            let item = |ui: &mut egui::Ui, label: &str, action: Action| {
                ui.add(Button::new(label).shortcut_text(shortcuts::describe(ctx, &keys.get(action))))
            };
            if item(ui, tr("New", "Nouveau"), Action::New).clicked() {
                self.new_tab(ctx);
                ui.close();
            }
            if item(ui, tr("Open…", "Ouvrir…"), Action::Open).clicked() {
                self.open_dialog(ctx);
                ui.close();
            }
            self.recent_menu(ui);
            self.examples_menu(ui);
            self.insert_menu(ui, ctx);
            ui.separator();
            if item(ui, tr("Save", "Enregistrer"), Action::Save).clicked() {
                self.save();
                ui.close();
            }
            if item(ui, tr("Save As…", "Enregistrer sous…"), Action::SaveAs).clicked() {
                self.save_as();
                ui.close();
            }
            ui.separator();
            if item(ui, tr("Settings…", "Réglages…"), Action::Settings).clicked() {
                self.settings_open = true;
                ui.close();
            }
        });
    }

    /// The formats the model can be exported in: solids, then drawings.
    fn export_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, keys: &Shortcuts) {
        let formats = self.doc.model.as_ref().map_or(&[][..], |m| m.formats);
        let export = ui.add_enabled_ui(!formats.is_empty(), |ui| {
            ui.menu_button(tr("Export", "Exporter"), |ui| {
                // Solids above the separator, 2D models below.
                let stl = if self.settings.binary_stl { tr("A binary STL", "Un STL binaire") } else { tr("An ASCII STL", "Un STL ASCII") };
                let entries = [
                    (Format::Stl, stl),
                    (Format::ThreeMf, tr("A 3MF package, in millimeters", "Un paquet 3MF, en millimètres")),
                    (
                        Format::Glb,
                        tr(
                            "A glTF model for game engines (Godot, Unity), with its colors",
                            "Un modèle glTF pour moteurs de jeu (Godot, Unity), avec ses couleurs",
                        ),
                    ),
                    (
                        Format::Obj,
                        tr(
                            "A Wavefront OBJ for 3D programs (Blender…), an object per part, its colors in a .mtl",
                            "Un OBJ Wavefront pour les logiciels 3D (Blender…), un objet par pièce, ses couleurs dans un .mtl",
                        ),
                    ),
                    (
                        Format::Step,
                        tr(
                            "A STEP for CAD programs (Fusion, SolidWorks, FreeCAD): faceted solids",
                            "Un STEP pour les logiciels de CAO (Fusion, SolidWorks, FreeCAD) : des solides à facettes",
                        ),
                    ),
                    (
                        Format::Urdf,
                        tr(
                            "A robot for ROS, Gazebo or MuJoCo: a link per part(), joined by part(joint = …)",
                            "Un robot pour ROS, Gazebo ou MuJoCo : un lien par part(), reliés par part(joint = …)",
                        ),
                    ),
                    (
                        Format::Json,
                        tr(
                            "The parts as JSON: each one's place, joint, size, volume and colors, and the parameters",
                            "Les pièces en JSON : la place, l'articulation, la taille, le volume et les couleurs de chacune, et les paramètres",
                        ),
                    ),
                    (Format::Svg, tr("An SVG drawing, in millimeters", "Un dessin SVG, en millimètres")),
                    (Format::Dxf, tr("A DXF drawing of line segments", "Un dessin DXF en segments")),
                ];
                for (format, hover) in entries {
                    if format == Format::Svg {
                        ui.separator();
                    }
                    let mut button = Button::new(format!("{}…", format.name()));
                    if format == self.shortcut_format() {
                        button = button.shortcut_text(shortcuts::describe(ctx, &keys.get(Action::Export)));
                    }
                    let disabled =
                        if Format::SOLID.contains(&format) { tr("For 3D models", "Pour les modèles 3D") } else { tr("For 2D models", "Pour les modèles 2D") };
                    let response = ui.add_enabled(formats.contains(&format), button);
                    if response.on_hover_text(hover).on_disabled_hover_text(disabled).clicked() {
                        self.export(format);
                        ui.close();
                    }
                }
                let view = Button::new(tr("DXF of the View…", "DXF de la vue…"));
                let response = ui.add_enabled(formats == Format::SOLID, view);
                let hover = tr(
                    "The outline of the 3D model seen from the view, at its size: for a laser cutter or a drawing",
                    "Le contour du modèle 3D vu depuis la vue, à sa taille : pour une découpe laser ou un plan",
                );
                if response.on_hover_text(hover).on_disabled_hover_text(tr("For 3D models", "Pour les modèles 3D")).clicked() {
                    self.export_view();
                    ui.close();
                }
                let hover = tr(
                    "A PNG of the model from four sides at one scale, with its size in millimeters and a 3D model cut \
                     through its middle: to show it for approval",
                    "Un PNG du modèle sous quatre angles à la même échelle, avec sa taille en millimètres et un modèle 3D \
                     coupé en son milieu : pour le faire valider",
                );
                if ui.button(tr("View Sheet…", "Planche de vues…")).on_hover_text(hover).clicked() {
                    self.export_sheet();
                    ui.close();
                }
            })
        });
        export.response.on_disabled_hover_text(tr("Render a non-empty model first", "Faites d'abord le rendu d'un modèle non vide"));
    }

    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let mut clear = false;
        let menu = ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
            ui.menu_button(tr("Open Recent", "Récents"), |ui| {
                for path in self.recent.paths() {
                    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                    // A long folder is cut short, so that the name keeps its
                    // line; the whole path shows on hover.
                    let folder = path.parent().map(display_folder).unwrap_or_default();
                    let button = Button::new(name)
                        .shortcut_text(shorten_middle(&folder, 48))
                        .wrap_mode(egui::TextWrapMode::Extend);
                    if ui.add(button).on_hover_text(path.display().to_string()).clicked() {
                        chosen = Some(path.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(tr("Clear Menu", "Effacer la liste")).clicked() {
                    clear = true;
                    ui.close();
                }
            })
        });
        menu.response
            .on_hover_text(tr("Reopen one of the last files", "Rouvrir un des derniers fichiers"))
            .on_disabled_hover_text(tr("No recent files", "Aucun fichier récent"));
        if let Some(path) = chosen {
            self.open_recent(ui.ctx(), path);
        }
        if clear {
            self.recent.clear();
            self.save_recent();
        }
    }

    /// The examples that come with the app, a submenu for each subject;
    /// one opens as a copy in the user's documents.
    fn examples_menu(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let source = examples::source_dir();
        let response = ui.menu_button(tr("Examples", "Exemples"), |ui| {
            let found = source.as_deref().map(examples::list).unwrap_or_default();
            if found.is_empty() {
                ui.label(RichText::new(tr("No examples found", "Aucun exemple trouvé")).color(theme::palette().text_dim));
            }
            let mut subjects: Vec<&str> = found.iter().map(|e| e.subject.as_str()).collect();
            subjects.dedup();
            for subject in subjects {
                ui.menu_button(subject, |ui| {
                    for example in found.iter().filter(|e| e.subject == subject) {
                        if ui.button(&example.file_name).on_hover_text(&example.summary).clicked() {
                            chosen = Some(example.clone());
                            ui.close();
                        }
                    }
                });
            }
        });
        response.response.on_hover_text(tr(
            "Open an example, as a copy in Documents/Code2Form/Examples",
            "Ouvrir un exemple, en copie dans Documents/Code2Form/Examples",
        ));
        let (Some(example), Some(source)) = (chosen, source) else { return };
        let Some(user) = examples::user_dir() else {
            self.doc.console.log(Level::Error, tr("Cannot open the example: no home folder", "Impossible d'ouvrir l'exemple : pas de dossier personnel"));
            return;
        };
        match examples::copy_out(&example, &source, &user) {
            Ok(copy) => self.open(ui.ctx(), copy),
            Err(err) => {
                let message = tr!("Cannot copy the example {} to {}: {err}", "Impossible de copier l'exemple {} dans {} : {err}", example.file_name, user.display());
                self.doc.console.log(Level::Error, message);
            }
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let frame = Frame::new().fill(theme::palette().bg).inner_margin(Margin::symmetric(12, 4));
        egui::TopBottomPanel::bottom("status").frame(frame).exact_height(26.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let small = |text: String| RichText::new(text).size(12.0).color(theme::palette().text_dim);
                if self.doc.busy() {
                    ui.add(egui::Spinner::new().size(11.0).color(theme::palette().accent_strong));
                    ui.label(RichText::new(tr("Rendering…", "Rendu en cours…")).size(12.0).color(theme::palette().accent_strong));
                } else if !self.settings.render_while_typing && self.doc.unrendered() {
                    status_dot(ui, theme::palette().warning);
                    let shortcut = shortcuts::describe(ctx, &self.settings.shortcuts.get(Action::Render));
                    ui.label(
                        RichText::new(tr!("Not rendered · {shortcut}", "Non rendu · {shortcut}")).size(12.0).color(theme::palette().warning),
                    )
                    .on_hover_text(tr("Rendering as you type is off in the settings", "Le rendu pendant la frappe est désactivé dans les réglages"));
                } else if self.doc.failed {
                    status_dot(ui, theme::palette().error);
                    ui.label(RichText::new(tr("Render failed", "Échec du rendu")).size(12.0).color(theme::palette().error));
                } else {
                    status_dot(ui, theme::palette().success);
                    ui.label(RichText::new(tr("Ready", "Prêt")).size(12.0).color(theme::palette().text));
                }
                if let Some(elapsed) = self.doc.elapsed {
                    ui.label(small(format!("· {}", format_duration(elapsed))));
                }
                if let Some(model) = &self.doc.model {
                    if !model.empty {
                        let s = model.stats;
                        if model.parts > 0 {
                            ui.label(small(tr!("· {} part{}", "· {} pièce{}", model.parts, plural(model.parts))));
                            ui.label(small(format!("· {} triangles", group_digits(s.triangles))));
                        } else if model.is_2d {
                            ui.label(small(tr!("· 2D · {} vertices", "· 2D · {} sommets", group_digits(s.vertices))));
                            ui.label(small(tr!("· area {}", "· aire {}", format_quantity(s.volume))));
                        } else {
                            ui.label(small(format!("· {} triangles", group_digits(s.triangles))));
                            ui.label(small(format!("· volume {}", format_quantity(s.volume))));
                        }
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // The tab names the file; its path, on hover.
                    if self.arrangement != Arrangement::View {
                        if let Some((line, column)) = self.doc.editor.cursor {
                            ui.label(small(tr!("Ln {}, Col {}", "Ligne {}, col {}", line + 1, column + 1)));
                        }
                    }
                });
            });
        });
    }

    /// The console's title row, which folds it, its counts, and while it is
    /// open, its lines.
    fn console_contents(&mut self, ui: &mut egui::Ui) {
        let console = &mut self.doc.console;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let icon = if self.console_open { Icon::FoldDown } else { Icon::FoldUp };
            let title = RichText::new("Console").strong().size(12.5).color(theme::palette().text);
            let (toggle, _) = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                icons::paint(ui.painter(), rect, icon, theme::palette().text_dim);
                let label = ui.add(Button::new(title).frame(false));
                (response.clicked() || label.clicked(), ())
            })
            .inner;
            if toggle {
                self.console_open = !self.console_open;
            }
            let errors = console.count(Level::Error);
            let warnings = console.count(Level::Warning);
            let echoes = console.count(Level::Echo);
            if errors > 0 {
                pill(ui, &tr!("{errors} error{}", "{errors} erreur{}", plural(errors)), theme::palette().error);
            }
            if warnings > 0 {
                pill(ui, &tr!("{warnings} warning{}", "{warnings} avertissement{}", plural(warnings)), theme::palette().warning);
            }
            if echoes > 0 {
                pill(ui, &format!("{echoes} echo"), theme::palette().text_dim);
            }
            if self.console_open {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(!console.is_empty(), Button::new(RichText::new(tr("Clear", "Effacer")).size(12.0))).clicked() {
                        console.clear();
                    }
                });
            }
        });
        if self.console_open {
            ui.add_space(2.0);
            Frame::new()
                .fill(theme::palette().editor)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| self.doc.console.show(ui));
        }
    }

    /// The console's panel, at the bottom of the code or of the window:
    /// a title row while folded, resizable while open.
    fn console_dock(&self, max_height: f32) -> egui::TopBottomPanel {
        let frame =
            Frame::new().fill(theme::palette().panel).inner_margin(Margin { left: 10, right: 10, top: 5, bottom: 6 });
        if self.console_open {
            egui::TopBottomPanel::bottom("console")
                .frame(frame)
                .resizable(true)
                .default_height(170.0)
                .height_range(90.0..=max_height.max(120.0))
        } else {
            egui::TopBottomPanel::bottom("console-folded").frame(frame).resizable(false)
        }
    }

    /// The code, and the console under it.
    fn code_column(&mut self, ui: &mut egui::Ui) {
        let max = ui.available_height() * 0.6;
        self.console_dock(max).show_inside(ui, |ui| self.console_contents(ui));
        self.show_agent_marks(ui.ctx());
        egui::CentralPanel::default().frame(Frame::new().fill(theme::palette().editor)).show_inside(ui, |ui| {
            if self.doc.editor.show(ui, &mut self.doc.text) {
                if self.settings.render_while_typing {
                    let delay = Duration::from_millis(self.settings.render_delay.into());
                    self.doc.render_deadline = Some(Instant::now() + delay);
                }
                // Typing on puts the save off.
                self.doc.autosave_deadline = None;
            }
        });
    }

    /// The code: beside the view, or across the window when it is shown
    /// alone. With the view alone, the console takes the bottom of the
    /// window.
    fn code_panel(&mut self, ctx: &egui::Context) {
        let width = ctx.screen_rect().width();
        match self.arrangement {
            Arrangement::View => {
                let max = ctx.screen_rect().height() * 0.6;
                self.console_dock(max).show(ctx, |ui| self.console_contents(ui));
            }
            Arrangement::Code => {
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(theme::palette().editor))
                    .show(ctx, |ui| self.code_column(ui));
            }
            Arrangement::Split => {
                egui::SidePanel::left("editor")
                    .frame(Frame::new().fill(theme::palette().editor))
                    .resizable(true)
                    .default_width((width * 0.4).max(320.0))
                    .width_range(260.0..=(width - 300.0).max(300.0))
                    .show(ctx, |ui| self.code_column(ui));
            }
        }
    }

    fn view_panel(&mut self, ctx: &egui::Context) {
        if self.arrangement == Arrangement::Code {
            return;
        }
        let mut remove_part = None;
        let animated = self.doc.animation.animated && !self.doc.is_circuit();
        let card = !self.doc.is_circuit();
        // The floating bars cover the view's edges: its own overlays keep
        // clear of them.
        let exploding = self.doc.exploded && self.doc.viewer.can_explode() && !self.doc.is_circuit();
        let cards = usize::from(exploding) + usize::from(self.doc.viewer.section_view && self.doc.viewer.can_section());
        self.doc.viewer.insets = viewer::Insets {
            top: if cards > 0 { 12.0 + (1 + cards) as f32 * VIEW_TOOLS_HEIGHT } else { 48.0 },
            bottom: if animated { 58.0 } else { 0.0 },
            right: if card && self.doc.customizer_shown() { 300.0 } else { 0.0 },
        };
        egui::CentralPanel::default().frame(Frame::new().fill(theme::palette().bg)).show(ctx, |ui| {
            let has_model = self.doc.model.as_ref().is_some_and(|m| !m.empty);
            let status = viewer::Status {
                rendering: self.doc.busy(),
                failed: self.doc.failed,
                has_model,
                is_2d: self.doc.model.as_ref().is_some_and(|m| m.is_2d),
                empty_hint: if self.doc.is_circuit() {
                    tr(
                        "The circuit places no parts yet. Add a [parts.name] table with the part's file.",
                        "Le circuit ne place encore aucun composant. Ajoutez une table [parts.name] avec le fichier du composant.",
                    )
                } else {
                    viewer::empty_hint(!self.doc.text.trim().is_empty())
                },
            };
            // A double-click on a part opens it rather than framing the view.
            self.doc.viewer.frame_on_double_click = !matches!(self.doc.hover, Some(Hit::Part(_)));
            self.start_rewire(ctx);
            self.doc.viewer.orbit_enabled = self.doc.rewire.is_none() && self.doc.part_drag.is_none();
            let response = self.doc.viewer.show(ui, &status);
            self.view_rect = response.rect;
            self.circuit_input(ctx, &response);
            self.paint_rewire(&ui.painter_at(response.rect), response.rect, ctx.input(|i| i.pointer.interact_pos()));
            self.paint_agent_flash(ctx, &ui.painter_at(response.rect), response.rect);
            if let (Some((drag, _, true, original)), Some(pointer)) = (&self.doc.part_drag, response.hover_pos()) {
                let name = &original.parts[drag.part].name;
                let turned = if drag.turns > 0 { tr!(", turned {}°", ", tourné de {}°", 90 * drag.turns) } else { String::new() };
                let text = tr!(
                    "`{name}` at [{}, {}]{turned} · Shift: 0.1 mm · R: turn",
                    "`{name}` en [{}, {}]{turned} · Maj : 0,1 mm · R : tourner",
                    drag.at[0],
                    drag.at[1]
                );
                inspect::paint_bubble(&ui.painter_at(response.rect), response.rect, pointer, &text);
            }
            // A part dragged from the catalogue lands where it is dropped.
            if let Some(dragged) = response.dnd_release_payload::<catalogue::Dragged>() {
                let pointer = ctx.input(|i| i.pointer.interact_pos());
                let at = pointer.and_then(|pos| catalogue::on_ground(&self.doc.viewer.ray(response.rect, pos)));
                let at = at.unwrap_or_else(|| catalogue::free_spot(&self.doc.part_boxes));
                self.add_part(ctx, &dragged.0, at);
            }
            if let Some(wiring) = &self.doc.wiring {
                let painter = ui.painter_at(response.rect);
                let right = if self.doc.viewer.layer_view { 70.0 } else { 12.0 };
                // Below the view's tools, and the assembly bar when it is
                // shown.
                let top = if self.doc.mount.active { 158.0 } else { 12.0 };
                let legend = wiring::paint_legend(&painter, response.rect, right, top, &wiring.legend());
                let lengths = inspect::Lengths { routed: &wiring.lengths, margin: self.wire_margin.mm };
                if let Some(selection) = self.doc.selection.filter(|_| !self.doc.mount.active) {
                    let (title, lines) = inspect::card(&wiring.circuit, selection, &self.doc.text, lengths);
                    // Under the section's card when it is shown.
                    let sectioning = self.doc.viewer.section_view && self.doc.viewer.can_section();
                    let below = response.rect.with_min_y(response.rect.top() + if sectioning { VIEW_TOOLS_HEIGHT } else { 0.0 });
                    let card = inspect::paint_card(&painter, below, &title, &lines);
                    if let Hit::Part(part) = selection {
                        let button = egui::Rect::from_min_size(card.left_bottom() + Vec2::new(0.0, 6.0), Vec2::new(150.0, 22.0));
                        let remove = ui.put(button, Button::new(tr("Remove with its wires", "Retirer avec ses fils")));
                        if remove.on_hover_text(tr("Remove the part and every wire to it (Delete)", "Retirer le composant et tous ses fils (Suppr)")).clicked() {
                            remove_part = Some(part);
                        }
                    }
                }
                if let (Some(hover), Some(pos)) = (self.doc.hover, response.hover_pos()) {
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
                        .prefix(tr("Wire margin: ", "Marge des fils : "))
                        .suffix(" mm");
                    let hint = tr(
                        "Added to each wire's routed length, for its stripped ends and some slack",
                        "Ajoutée à la longueur tracée de chaque fil, pour ses bouts dénudés et un peu de mou",
                    );
                    if ui.put(field, value).on_hover_text(hint).changed() {
                        self.wire_margin.mm = mm;
                        if let Err(err) = self.wire_margin.save() {
                            self.doc.console.log(Level::Warning, tr!("Cannot save the wire margin: {err}", "Impossible d'enregistrer la marge des fils : {err}"));
                        }
                    }
                }
            }
        });
        let rect = self.view_rect;
        self.view_tools(ctx, rect);
        if self.doc.is_circuit() && !self.doc.mount.active {
            self.parts_legend(ctx, rect);
        }
        if card {
            self.parameters_card(ctx, rect);
        }
        if animated {
            let mut bar = None;
            egui::Area::new(Id::new(("animation", self.doc.id)))
                .pivot(Align2::LEFT_BOTTOM)
                .fixed_pos(egui::pos2(rect.left() + 12.0, rect.bottom() - 12.0))
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    card_frame().inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                        ui.set_width((rect.width() - 44.0).max(200.0));
                        bar = Some(animation_bar(ui, &self.doc.animation));
                    });
                });
            if let Some(input) = bar {
                self.animation_input(ctx, input);
            }
        }
        if self.doc.mount.active {
            self.mount_bar(ctx, self.view_rect);
            // The pins of the wire on screen pulse.
            self.update_overlay();
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some(part) = remove_part {
            self.remove_part(ctx, part);
        }
        if let Some(edit) = self.doc.viewer.take_pause_edit() {
            self.apply_pause_edit(ctx, edit);
        }
    }

    /// The view's tools, over its top left: fit, the views, the grid, the
    /// axes, the layers and the pauses; and for a design with part()s, the
    /// exploded view, how far apart and how: radially or along each axis.
    fn view_tools(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let explodable = self.doc.viewer.can_explode() && !self.doc.is_circuit();
        let circuit = self.doc.is_circuit();
        let on = self.doc.exploded;
        let mut amount = self.doc.explode_amount;
        let mut how = self.doc.explode_how;
        let mut explode = false;
        egui::Area::new(Id::new(("view-tools", self.doc.id)))
            .fixed_pos(rect.left_top() + Vec2::splat(12.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                card_frame().inner_margin(Margin::same(4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let viewer = &mut self.doc.viewer;
                        let hint = tr("Fit the model, at the default angle (or double-click the view)", "Cadrer le modèle, à l'angle par défaut (ou double-clic dans la vue)");
                        if icons::button(ui, Icon::Fit, false, hint).clicked() {
                            viewer.reset();
                        }
                        ui.menu_button(tr("Views", "Vues"), |ui| {
                            for &(name, yaw, pitch) in viewer::PRESETS {
                                if ui.button(viewer::preset_label(name)).clicked() {
                                    viewer.look_from(yaw, pitch);
                                    ui.close();
                                }
                            }
                            ui.separator();
                            ui.checkbox(&mut viewer.turntable, tr("Turntable", "Rotation auto")).on_hover_text(tr(
                                "Turn the view around the model (a drag in the view stops it)",
                                "Tourner la vue autour du modèle (un glisser dans la vue l'arrête)",
                            ));
                            if circuit && viewer.has_enclosure() {
                                ui.separator();
                                let mut shown = viewer.shows_enclosure();
                                let toggle = ui.checkbox(&mut shown, tr("Enclosure", "Boîtier"));
                                if toggle.on_hover_text(tr("Show the enclosure around the parts, see-through", "Afficher le boîtier autour des composants, transparent")).changed() {
                                    viewer.set_show_enclosure(shown);
                                }
                                opacity_slider(ui, viewer);
                            }
                        });
                        tool_separator(ui);
                        if icons::button(ui, Icon::Grid, viewer.show_grid, tr("Show the XY grid", "Afficher la grille XY")).clicked() {
                            viewer.show_grid = !viewer.show_grid;
                        }
                        if icons::button(ui, Icon::Axes, viewer.show_axes, tr("Show the axes", "Afficher les axes")).clicked() {
                            viewer.show_axes = !viewer.show_axes;
                        }
                        let hint = tr(
                            "Layers: cut the model at the top of a layer, picked with the slider on the right",
                            "Couches : couper le modèle au sommet d'une couche, choisie avec le curseur à droite",
                        );
                        if icons::button(ui, Icon::Layers, viewer.layer_view, hint).clicked() {
                            viewer.layer_view = !viewer.layer_view;
                        }
                        if viewer.has_pauses() {
                            let hint = tr("Show the heights where the print pauses", "Afficher les hauteurs où l'impression fait une pause");
                            if icons::button(ui, Icon::Pauses, viewer.show_pauses, hint).clicked() {
                                viewer.show_pauses = !viewer.show_pauses;
                            }
                        }
                        if explodable {
                            tool_separator(ui);
                            let hint = tr("Exploded view: draw the model's part()s apart, to see how they fit together", "Vue éclatée : écarter les part() du modèle, pour voir comment elles s'assemblent");
                            explode = icons::button(ui, Icon::Explode, on, hint).clicked();
                        }
                        if viewer.can_section() {
                            tool_separator(ui);
                            let hint = if circuit {
                                tr(
                                    "Section: cut the enclosure open across an axis, to see the parts and wires inside",
                                    "Coupe : ouvrir le boîtier selon un axe, pour voir les composants et les fils à l'intérieur",
                                )
                            } else {
                                tr("Section: cut the model open across an axis, to see inside", "Coupe : ouvrir le modèle selon un axe, pour voir l'intérieur")
                            };
                            if icons::button(ui, Icon::Section, viewer.section_view, hint).clicked() {
                                viewer.section_view = !viewer.section_view;
                            }
                        }
                    });
                });
            });
        // How far apart and how, on a card of its own under the tools, so
        // that they keep clear of the parameters.
        if explodable && on {
            egui::Area::new(Id::new(("explode-tools", self.doc.id)))
                .fixed_pos(rect.left_top() + Vec2::new(12.0, 12.0 + VIEW_TOOLS_HEIGHT))
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    card_frame().inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = 110.0;
                            let slider = egui::Slider::new(&mut amount, 0.0..=1.0)
                                    .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                                    .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0));
                                ui.add(slider).on_hover_text(tr("How far apart the parts are drawn", "L'écart entre les pièces"));
                                ui.add_space(4.0);
                                let radial = tr(
                                    "Each part carries on along the line from the center of gravity of what holds it",
                                    "Chaque pièce prolonge la ligne qui part du centre de gravité de ce qui la porte",
                                );
                                ui.selectable_value(&mut how, c2f_geom::Explode::Radial, tr("Radial", "Radiale"))
                                    .on_hover_text(radial);
                                let axis = tr(
                                    "Each part slides along its own axis, the one it is built and turns about",
                                    "Chaque pièce glisse le long de son axe, celui autour duquel elle est construite et tourne",
                                );
                                ui.selectable_value(&mut how, c2f_geom::Explode::Axis, tr("Axis", "Axe")).on_hover_text(axis);
                        });
                    });
                });
        }
        if self.doc.viewer.section_view && self.doc.viewer.can_section() {
            let below = if explodable && on { 2.0 } else { 1.0 };
            self.section_card(ctx, rect.left_top() + Vec2::new(12.0, 12.0 + below * VIEW_TOOLS_HEIGHT));
        }
        if how != self.doc.explode_how {
            self.doc.explode_how = how;
            self.doc.viewer.set_explode_how(how);
        }
        if amount != self.doc.explode_amount {
            self.doc.explode_amount = amount;
            // The slider moves the parts at once, without sliding.
            self.doc.explode_shown = amount;
        }
        if explode {
            self.doc.exploded = !on;
            if self.doc.exploded {
                // The parts all the way apart are framed as they slide.
                self.doc.viewer.set_explode(self.doc.explode_shown.max(0.01));
                self.doc.viewer.view_all();
            }
        }
    }

    /// The section's card, at `at`: the axis it cuts across, where along
    /// it, and for a circuit the enclosure's opacity. Only uniforms
    /// change: nothing renders again.
    fn section_card(&mut self, ctx: &egui::Context, at: egui::Pos2) {
        let circuit = self.doc.is_circuit();
        let viewer = &mut self.doc.viewer;
        let section = viewer.section();
        egui::Area::new(Id::new(("section-tools", self.doc.id)))
            .fixed_pos(at)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                card_frame().inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (axis, name) in renderer::Section::AXES.into_iter().enumerate() {
                            let hint = tr!("Cut across {}", "Couper selon {}", name);
                            if ui.selectable_label(section.axis == axis, name).on_hover_text(hint).clicked() && section.axis != axis {
                                viewer.set_section_axis(axis);
                            }
                        }
                        ui.add_space(4.0);
                        if let Some((lo, hi)) = viewer.section_range(section.axis) {
                            ui.spacing_mut().slider_width = 130.0;
                            let mut value = section.at;
                            let slider = egui::Slider::new(&mut value, lo..=hi).max_decimals(1).suffix(" mm");
                            let hint = tr("Where the cut runs: what lies beyond it is hidden", "Où passe la coupe : ce qui est au-delà est masqué");
                            if ui.add(slider).on_hover_text(hint).changed() {
                                viewer.set_section_at(value);
                            }
                        }
                        if circuit && viewer.has_enclosure() {
                            ui.add_space(4.0);
                            opacity_slider(ui, viewer);
                        }
                    });
                });
            });
    }

    /// A circuit's parts, each in the color it shows in, over the view's
    /// bottom left: a click picks one out with its wires, as a click on
    /// it in the view does.
    fn parts_legend(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let Some(wiring) = &self.doc.wiring else { return };
        let colors = self.doc.viewer.part_colors();
        if wiring.circuit.parts.is_empty() || colors.len() != wiring.circuit.parts.len() {
            return;
        }
        let parts: Vec<(String, [f32; 4])> = wiring.circuit.parts.iter().map(|p| p.name.clone()).zip(colors.iter().copied()).collect();
        let selected = match self.doc.selection {
            Some(Hit::Part(part)) => Some(part),
            _ => None,
        };
        let mut clicked = None;
        // Above the axis gizmo.
        egui::Area::new(Id::new(("parts-legend", self.doc.id)))
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(rect.left() + 12.0, rect.bottom() - 100.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                card_frame().inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(egui::RichText::new(tr("Parts", "Composants")).size(12.5).color(theme::palette().text_strong));
                    for (index, (name, color)) in parts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                            ui.painter().circle(dot.center(), 5.0, wiring::color32(*color), Stroke::new(1.0, theme::palette().line(110)));
                            let hint = tr("Pick out this part and its wires (click again to clear)", "Faire ressortir ce composant et ses fils (cliquer à nouveau pour effacer)");
                            if ui.selectable_label(selected == Some(index), name).on_hover_text(hint).clicked() {
                                clicked = Some(index);
                            }
                        });
                    }
                });
            });
        if let Some(part) = clicked {
            let hit = if selected == Some(part) { None } else { Some(Hit::Part(part)) };
            self.select(ctx, hit);
        }
    }

    /// The parameters the file declares, on a card over the view's top
    /// right, which folds to its title. Clear of the layer slider.
    fn parameters_card(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let right = if self.doc.viewer.layer_view { 72.0 } else { 12.0 };
        let shown = self.doc.customizer_shown();
        let mut folded = !shown;
        let max_height = (rect.height() - 24.0 - self.doc.viewer.insets.bottom).max(120.0);
        let mut changed = false;
        egui::Area::new(Id::new(("parameters", self.doc.id)))
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(egui::pos2(rect.right() - right, rect.top() + 12.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let bottom = if folded { 4 } else { 10 };
                card_frame().inner_margin(Margin { left: 12, right: 4, top: 4, bottom }).show(ui, |ui| {
                    ui.set_width(if folded { 190.0 } else { 276.0 });
                    ui.set_max_height(max_height);
                    changed = self.doc.customizer.show(ui, &mut folded);
                });
            });
        if folded == shown {
            self.doc.customizer_open = Some(!folded);
        }
        if changed && self.settings.render_while_typing {
            self.doc.render_deadline = Some(Instant::now() + PARAMETER_DEBOUNCE);
        }
    }

    /// Makes a change to the pauses asked for in the layer view in the
    /// code, as one step of the editor's undo history, and renders it.
    fn apply_pause_edit(&mut self, ctx: &egui::Context, edit: PauseEdit) {
        // The pauses point into the text they were rendered from.
        if self.doc.viewer.scene_source() != self.doc.text {
            let message = tr(
                "The code changed since the last render: try again once it is rendered",
                "Le code a changé depuis le dernier rendu : réessayez une fois le rendu fait",
            );
            self.doc.console.log(Level::Warning, message);
            return;
        }
        let (result, done) = match edit {
            PauseEdit::Add { z } => {
                (Ok(pause_edit::add(&self.doc.text, z)), tr!("Added a pause at {} mm", "Pause ajoutée à {} mm", pause_edit::number(z)))
            }
            PauseEdit::Move { number, z } => (
                pause_edit::set_height(&self.doc.text, number, z),
                tr!("Moved a pause to {} mm", "Pause déplacée à {} mm", pause_edit::number(z)),
            ),
            PauseEdit::Remove { call } => (pause_edit::remove(&self.doc.text, call), tr("Removed a pause", "Pause retirée").to_string()),
        };
        match result {
            Ok((text, cursor)) => {
                self.doc.editor.replace_text(ctx, &mut self.doc.text, text, cursor);
                self.doc.console.log(Level::Info, done);
                self.render_now();
            }
            Err(why) => self.doc.console.log(Level::Warning, tr!("Cannot change the pause: {why}", "Impossible de modifier la pause : {why}")),
        }
    }

    /// Gives the app, the tabs and the render worker the settings.
    fn apply_settings(&mut self, ctx: &egui::Context) {
        self.apply_tab_settings();
        let settings = &self.settings;
        theme::set_code_size(ctx, settings.code_size);
        theme::style_window(ctx, settings.appearance);
        theme::set_accent(ctx, settings.accent_from_icon.then_some(settings.icon));
        ctx.set_zoom_factor(settings.interface_size);
        self.worker.set_libraries(settings.library_folders.clone());
        self.catalogue.set_libraries(settings.library_folders.clone());
        c2f_geom::set_font_folders(settings.font_folders.clone());
    }

    /// Gives the views the theme's colors where they computed them: the
    /// greyed-out parts and wires fade towards its grey.
    fn restyle(&mut self) {
        for index in 0..self.shelf.len() {
            self.in_tab(index, |app| {
                app.doc.viewer.restyle();
                app.update_overlay();
            });
        }
    }

    /// Gives every tab the settings of its view and its editor.
    fn apply_tab_settings(&mut self) {
        apply_tab_settings(&self.settings, &mut self.doc);
        for index in 0..self.shelf.len() {
            if let Some(tab) = self.shelf.other_mut(index) {
                apply_tab_settings(&self.settings, tab);
            }
        }
    }

    /// Saves the settings when they changed, the view's among them, but not
    /// while a drag (of a slider, say) is still changing them. The view's
    /// are the active tab's, and go to the other tabs.
    fn keep_settings(&mut self, ctx: &egui::Context) {
        let viewer = &self.doc.viewer;
        let view = (viewer.show_grid, viewer.show_axes, viewer.layer_height);
        if view != (self.settings.show_grid, self.settings.show_axes, self.settings.layer_height) {
            (self.settings.show_grid, self.settings.show_axes, self.settings.layer_height) = view;
            self.apply_tab_settings();
        }
        // Kept aside, not to save the settings at each step of a move.
        let zoom = ctx.zoom_factor();
        if zoom == self.measured_zoom {
            if let Some(window) = window::current(ctx) {
                self.window = Some(window);
            }
        }
        self.measured_zoom = zoom;
        if self.settings == self.saved_settings || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.note_window();
        if let Err(err) = self.settings.save(self.settings_store.as_deref()) {
            self.doc.console.log(Level::Warning, tr!("Cannot save the settings: {err}", "Impossible d'enregistrer les réglages : {err}"));
        }
        self.saved_settings = self.settings.clone();
    }

    /// Gives the settings the window's size and place. A maximized window
    /// keeps the size and place it had before.
    fn note_window(&mut self) {
        match self.window {
            Some((placement, false)) => {
                self.settings.window = Some(placement);
                self.settings.window_maximized = false;
            }
            Some((_, true)) => self.settings.window_maximized = true,
            None => {}
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        if self.icon_choices.is_empty() {
            let pixels = (ICON_CHOICE * ctx.pixels_per_point()).ceil() as usize;
            self.icon_choices = Tint::ALL.iter().map(|&tint| icon_texture(ctx, tint, pixels)).collect();
        }
        let mut chosen = None;
        let mut libraries = None;
        let mut fonts = None;
        let mut export_folder = None;
        let mut shortcut_edit = None;
        let mut done = false;
        let before = self.settings.clone();
        let modal = egui::Modal::new(Id::new("settings")).show(ctx, |ui| {
            ui.label(RichText::new(tr("Settings", "Réglages")).heading().color(theme::palette().text));
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(120.0);
                    for section in Section::ALL {
                        let selected = self.settings_section == section;
                        let label = egui::Button::selectable(selected, section.name()).min_size(Vec2::new(120.0, 28.0));
                        if ui.add(label).clicked() && self.settings_section != section {
                            self.settings_section = section;
                            self.stop_recording();
                        }
                    }
                });
                ui.add_space(8.0);
                let (line, _) = ui.allocate_exact_size(Vec2::new(1.0, SETTINGS_HEIGHT), Sense::hover());
                ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, theme::palette().border));
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.set_width(440.0);
                    ui.set_min_height(SETTINGS_HEIGHT);
                    // The window is the controls' usual color: lift them off it.
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.inactive.bg_fill = theme::palette().hover;
                    widgets.inactive.weak_bg_fill = theme::palette().hover;
                    widgets.hovered.bg_fill = theme::palette().border_strong;
                    widgets.hovered.weak_bg_fill = theme::palette().border_strong;
                    if self.settings_section == Section::Shortcuts {
                        // A list of its own, which scrolls.
                        let problem = self.record_problem.as_deref();
                        shortcut_edit = shortcuts_page(ui, ctx, &self.settings.shortcuts, self.recording, problem);
                        return;
                    }
                    let scroll = egui::ScrollArea::vertical()
                        .id_salt(("settings-page", self.settings_section.id()))
                        .max_height(SETTINGS_HEIGHT)
                        .auto_shrink([false, true]);
                    scroll.show(ui, |ui| match self.settings_section {
                        Section::General => general_page(ui, &mut self.settings),
                        Section::Appearance => chosen = self.appearance_page(ui, ctx),
                        Section::Editor => editor_page(ui, &mut self.settings),
                        Section::Model => model_page(ui, ctx, &mut self.settings),
                        Section::Export => export_folder = export_page(ui, ctx, &mut self.settings),
                        Section::Libraries => (libraries, fonts) = libraries_page(ui, &self.settings),
                        Section::Shortcuts => {}
                    });
                });
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = Button::new(RichText::new(tr("Done", "OK")).color(theme::palette().on_accent).strong())
                    .fill(theme::palette().accent);
                if ui.add(button).clicked() {
                    done = true;
                }
            });
        });
        if let Some(tint) = chosen {
            self.choose_icon(ctx, tint);
        }
        if self.settings.code_size != before.code_size {
            theme::set_code_size(ctx, self.settings.code_size);
        }
        if self.settings.interface_size != before.interface_size {
            ctx.set_zoom_factor(self.settings.interface_size);
        }
        if self.settings.language != before.language {
            i18n::set_language(self.settings.language);
            ctx.request_repaint();
        }
        if self.settings.accent_from_icon != before.accent_from_icon || self.settings.icon != before.icon {
            self.apply_accent(ctx);
        }
        if self.settings.appearance != before.appearance {
            theme::style_window(ctx, self.settings.appearance);
            // The next frame shows it, from its start.
            ctx.request_repaint();
        }
        match shortcut_edit {
            Some(ShortcutEdit::Record(action)) if self.recording == Some(action) => self.stop_recording(),
            Some(ShortcutEdit::Record(action)) => {
                self.recording = Some(action);
                self.record_problem = None;
            }
            Some(ShortcutEdit::Reset(action)) => {
                self.settings.shortcuts.reset(action);
                self.stop_recording();
            }
            Some(ShortcutEdit::ResetAll) => {
                self.settings.shortcuts = Shortcuts::default();
                self.stop_recording();
            }
            None => {}
        }
        let tab = |s: &Settings| (s.model_color, s.indent_width, s.shortcuts.clone());
        if tab(&self.settings) != tab(&before) {
            self.apply_tab_settings();
        }
        match export_folder {
            Some(ExportFolderEdit::NextToTheDesign) => self.settings.export_folder = None,
            Some(ExportFolderEdit::Choose) => {
                let mut dialog = rfd::FileDialog::new().set_title(tr("Export to this folder", "Exporter dans ce dossier"));
                if let Some(folder) = self.settings.export_folder.as_deref().filter(|f| f.is_dir()) {
                    dialog = dialog.set_directory(folder);
                }
                if let Some(folder) = dialog.pick_folder() {
                    self.settings.export_folder = Some(folder);
                }
            }
            None => {}
        }
        if !self.settings.render_while_typing {
            self.doc.render_deadline = None;
        }
        if let Some(edit) = libraries {
            if edit_folders(&mut self.settings.library_folders, edit, tr("Add a library folder", "Ajouter un dossier de bibliothèques")) {
                self.worker.set_libraries(self.settings.library_folders.clone());
                self.catalogue.set_libraries(self.settings.library_folders.clone());
                self.render_now();
            }
        }
        if let Some(edit) = fonts {
            if edit_folders(&mut self.settings.font_folders, edit, tr("Add a font folder", "Ajouter un dossier de polices")) {
                c2f_geom::set_font_folders(self.settings.font_folders.clone());
                self.render_now();
            }
        }
        if done || modal.should_close() {
            self.settings_open = false;
            self.stop_recording();
        }
    }

    /// The theme, the interface's size and the app icon; the icon clicked.
    fn appearance_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Option<Tint> {
        let settings = &mut self.settings;
        let system = if cfg!(target_os = "macos") { "macOS" } else { tr("the system", "le système") };
        section_heading(
            ui,
            tr("Appearance", "Apparence"),
            &tr!("System follows {} as it turns light or dark.", "Système suit {}, en clair ou en sombre.", system),
        );
        egui::Grid::new("appearance").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(tr("Theme", "Thème"));
            ui.horizontal(|ui| {
                for appearance in Appearance::ALL {
                    ui.selectable_value(&mut settings.appearance, appearance, appearance.name());
                }
            });
            ui.end_row();
            let keys = tr!(
                "{} and {} change it too; {} sets it back to 100%.",
                "{} et {} la changent aussi ; {} la remet à 100 %.",
                shortcuts::describe(ctx, &settings.shortcuts.get(Action::ZoomIn)),
                shortcuts::describe(ctx, &settings.shortcuts.get(Action::ZoomOut)),
                shortcuts::describe(ctx, &settings.shortcuts.get(Action::ZoomReset))
            );
            ui.label(tr("Size", "Taille")).on_hover_text(keys);
            ui.horizontal(|ui| {
                // Eight sizes on one row.
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().button_padding.x = 5.0;
                let (low, high) = (*settings::INTERFACE_SIZES.start(), *settings::INTERFACE_SIZES.end());
                for tenths in (low * 10.0).round() as u32..=(high * 10.0).round() as u32 {
                    let size = tenths as f32 / 10.0;
                    let selected = (settings.interface_size - size).abs() < 0.01;
                    if ui.selectable_label(selected, format!("{}%", tenths * 10)).clicked() {
                        settings.interface_size = size;
                    }
                }
            });
            ui.end_row();
        });
        ui.add_space(14.0);
        let mut chosen = None;
        let shown_in = if cfg!(target_os = "macos") {
            tr("Shown in the Dock and the Finder.", "Affichée dans le Dock et le Finder.")
        } else {
            tr("Shown in the window's title bar.", "Affichée dans la barre de titre.")
        };
        section_heading(ui, tr("App icon", "Icône de l'app"), shown_in);
        for (row, tints) in Tint::ALL.chunks(ICONS_PER_ROW).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for (column, &tint) in tints.iter().enumerate() {
                    let texture = &self.icon_choices[row * ICONS_PER_ROW + column];
                    if icon_choice(ui, texture, tint, tint == self.settings.icon).clicked() {
                        chosen = Some(tint);
                    }
                }
            });
        }
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.settings.accent_from_icon,
            tr("Buttons and highlights in the icon's color", "Boutons et sélections de la couleur de l'icône"),
        )
        .on_hover_text(tr("Otherwise blue", "Sinon en bleu"));
        chosen
    }

    /// Gives the theme the accent the settings ask for.
    fn apply_accent(&mut self, ctx: &egui::Context) {
        let tint = self.settings.accent_from_icon.then_some(self.settings.icon);
        if theme::set_accent(ctx, tint) {
            self.restyle();
        }
        ctx.request_repaint();
    }

    /// Shows `tint` from now on; the settings keep it.
    fn choose_icon(&mut self, ctx: &egui::Context, tint: Tint) {
        if tint == self.settings.icon {
            return;
        }
        self.settings.icon = tint;
        self.logo = logo_texture(ctx, tint);
        match icon_setting::apply(ctx, tint) {
            Ok(()) => self.doc.console.log(Level::Info, tr!("App icon: {}", "Icône de l'app : {}", tint.name())),
            Err(err) => {
                let message = tr!(
                    "The Dock shows the {} icon while the app runs, but {err}",
                    "Le Dock montre l'icône {} tant que l'app tourne, mais {err}",
                    tint.name()
                );
                self.doc.console.log(Level::Warning, message);
            }
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm else { return };
        #[derive(Clone, Copy)]
        enum Choice {
            Save,
            Discard,
            Cancel,
        }
        let mut choice = None;
        let name = self.doc.file_name();
        let modal = egui::Modal::new(Id::new("confirm-unsaved")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(RichText::new(tr("Unsaved changes", "Modifications non enregistrées")).heading().color(theme::palette().text));
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr!(
                    "Do you want to save the changes to {name} before {}?",
                    "Voulez-vous enregistrer les modifications de {name} avant {} ?",
                    action.verb()
                ))
                    .color(theme::palette().text_dim),
            );
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let save = Button::new(RichText::new(tr("Save", "Enregistrer")).color(theme::palette().on_accent).strong())
                    .fill(theme::palette().accent);
                if ui.add(save).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.button(tr("Don't save", "Ne pas enregistrer")).clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button(tr("Cancel", "Annuler")).clicked() {
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
                } else {
                    self.quit_cleared.clear();
                }
            }
            Some(Choice::Discard) => {
                self.confirm = None;
                self.finish(ctx, action);
            }
            Some(Choice::Cancel) => {
                self.confirm = None;
                self.quit_cleared.clear();
            }
            None => {}
        }
    }

    fn finish(&mut self, ctx: &egui::Context, action: Pending) {
        match action {
            Pending::Close => self.close_now(ctx, self.shelf.active()),
            Pending::Quit => self.quit(ctx),
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if theme::follow(ctx, self.settings.appearance) {
            self.restyle();
        }
        // Not while a question about the active tab waits.
        if self.confirm.is_none() {
            for path in open_events::take() {
                self.open(ctx, path);
            }
        }
        self.receive();
        self.serve_live(ctx);
        self.animate(ctx);
        self.explode(ctx);
        self.shortcuts(ctx);
        self.poll_disk(ctx);

        self.header(ctx);
        self.status_bar(ctx);
        self.agent_panel(ctx);
        self.catalogue_panel(ctx);
        self.code_panel(ctx);
        self.view_panel(ctx);
        self.settings_dialog(ctx);
        self.confirm_dialog(ctx);
        self.review_window(ctx);
        self.keep_settings(ctx);

        if let Some(deadline) = self.doc.render_deadline {
            let now = Instant::now();
            if now >= deadline {
                self.render_if_changed();
            } else {
                ctx.request_repaint_after(deadline - now);
            }
        }
        self.autosave(ctx);
        if self.doc.busy() {
            // Keep the spinners turning.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.update_title(ctx);
        self.save_session();

        let ready = self.doc.finished > 0 && !self.doc.busy() && self.live.review.as_ref().is_none_or(|r| r.settled());
        if let Some(result) = self.screenshot.as_mut().and_then(|s| s.update(ctx, ready)) {
            match result {
                Ok(message) => self.doc.console.log(Level::Info, message),
                Err(error) => self.doc.console.log(Level::Error, error),
            }
        }
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.save_session();
        self.note_window();
        if self.settings != self.saved_settings {
            let _ = self.settings.save(self.settings_store.as_deref());
        }
        if let Some(gl) = gl {
            self.doc.viewer.destroy(gl);
            for index in 0..self.shelf.len() {
                if let Some(tab) = self.shelf.other_mut(index) {
                    tab.viewer.destroy(gl);
                }
            }
        }
    }
}

/// What was set in the animation bar.
struct BarInput {
    frame: u32,
    fps: u32,
    steps: u32,
    /// Play or pause was clicked.
    toggle: bool,
}

/// The bar under the view of a design that reads `$t`: play or pause, the
/// time and its slider, the frames a second and the frames from 0 to 1.
fn animation_bar(ui: &mut egui::Ui, animation: &Animation) -> BarInput {
    let mut input = BarInput { frame: animation.frame, fps: animation.fps, steps: animation.steps, toggle: false };
    ui.horizontal(|ui| {
        let (icon, hint) = if animation.playing {
            (Icon::Pause, tr("Pause", "Pause"))
        } else {
            (Icon::Play, tr("Play the model along $t", "Animer le modèle selon $t"))
        };
        input.toggle = icons::button(ui, icon, true, hint).clicked();
        let t = f64::from(input.frame) / f64::from(input.steps);
        ui.label(RichText::new(format!("$t = {t:.3}")).monospace().color(theme::palette().text));
        ui.spacing_mut().slider_width = (ui.available_width() - 210.0).max(60.0);
        let last = input.steps - 1;
        let frame = tr!("Frame {} of {}", "Image {} sur {}", input.frame + 1, input.steps);
        ui.add(egui::Slider::new(&mut input.frame, 0..=last).show_value(false)).on_hover_text(frame);
        ui.add(egui::DragValue::new(&mut input.fps).range(animation::FPS).suffix(tr(" fps", " i/s")))
            .on_hover_text(tr("Frames shown a second", "Images affichées par seconde"));
        ui.add(egui::DragValue::new(&mut input.steps).range(animation::STEPS).suffix(tr(" steps", " pas")))
            .on_hover_text(tr(
                "Frames from $t = 0 to 1, which it does not reach: the last frame leads back to the first",
                "Images de $t = 0 à 1, qu'il n'atteint pas : la dernière image ramène à la première",
            ));
        let cached = RichText::new(format!("{}/{}", animation.cached(), animation.steps)).size(12.0);
        ui.label(cached.color(theme::palette().text_dim)).on_hover_text(
            tr(
                "Frames rendered and kept in memory: once all of them are, the animation plays at full speed",
                "Images rendues et gardées en mémoire : une fois toutes prêtes, l'animation tourne à pleine vitesse",
            ),
        );
    });
    input
}

/// What a click on a tab asks.
enum TabClick {
    Select,
    Close,
    /// Nothing: the tab, for its tooltip.
    None(egui::Response),
}

/// A tab of the tab bar: its name, and an × that shows on the active tab
/// and under the pointer; a modified tab shows a dot there instead.
fn tab(ui: &mut egui::Ui, id: u64, name: &str, active: bool, dirty: bool) -> TabClick {
    let color = if active { theme::palette().text } else { theme::palette().text_dim };
    let galley = ui.painter().layout_no_wrap(name.to_string(), egui::FontId::proportional(13.0), color);
    let size = Vec2::new(galley.size().x + 44.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let close_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), Vec2::splat(18.0));
    // Over the tab: it takes the clicks within its box.
    let close = ui.interact(close_rect, Id::new(("tab-close", id)), Sense::click());
    let painter = ui.painter_at(rect);
    let hovered = response.hovered() || close.hovered();
    let fill = if active {
        theme::palette().raised
    } else if hovered {
        theme::palette().faint_bg
    } else {
        theme::palette().bg
    };
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    painter.galley(egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if close.hovered() {
        painter.rect_filled(close_rect, CornerRadius::same(4), theme::palette().hover);
    }
    if dirty && !close.hovered() {
        painter.circle_filled(close_rect.center(), 3.5, theme::palette().warning);
    } else if active || hovered {
        let font = egui::FontId::proportional(14.0);
        let tint = if close.hovered() { theme::palette().text } else { theme::palette().text_dim };
        painter.text(close_rect.center(), egui::Align2::CENTER_CENTER, "×", font, tint);
    }
    if close.clicked() || response.middle_clicked() {
        TabClick::Close
    } else if response.clicked() {
        TabClick::Select
    } else {
        TabClick::None(response)
    }
}

/// Gives `tab` the grid, axes, layer height and model color of
/// `settings`, and their indent width and find keys.
fn apply_tab_settings(settings: &Settings, tab: &mut Tab) {
    let viewer = &mut tab.viewer;
    viewer.show_grid = settings.show_grid;
    viewer.show_axes = settings.show_axes;
    viewer.layer_height = settings.layer_height;
    viewer.model_color = color_floats(settings.model_color);
    tab.editor.indent_width = settings.indent_width;
    tab.editor.find_keys = [Action::FindNext, Action::FindPrevious].map(|a| settings.shortcuts.get(a));
}

/// A settings section's title and what it is for.
fn section_heading(ui: &mut egui::Ui, title: &str, about: &str) {
    ui.label(RichText::new(title).strong().color(theme::palette().text));
    ui.label(RichText::new(about).color(theme::palette().text_dim));
    ui.add_space(8.0);
}

/// The language, and what comes back at launch.
fn general_page(ui: &mut egui::Ui, settings: &mut Settings) {
    section_heading(ui, tr("Language", "Langue"), tr("The interface's language.", "La langue de l'interface."));
    ui.horizontal(|ui| {
        for language in Language::ALL {
            ui.selectable_value(&mut settings.language, language, language.name());
        }
    });
    ui.add_space(14.0);
    section_heading(
        ui,
        tr("At launch", "Au lancement"),
        tr("Files opened from the Finder open too.", "Les fichiers ouverts depuis le Finder s'ouvrent aussi."),
    );
    ui.checkbox(
        &mut settings.reopen_tabs,
        tr("Reopen the tabs open when the app was last closed", "Rouvrir les onglets laissés ouverts"),
    );
    ui.add_space(14.0);
    let kept = tr(
        "The grid, the axes, the layer height and the window's size and place are kept as you leave them.",
        "La grille, les axes, la hauteur de couche, la taille et la place de la fenêtre restent comme vous les laissez.",
    );
    ui.label(RichText::new(kept).color(theme::palette().text_dim));
}

/// The code's size, Tab's indent and saving.
fn editor_page(ui: &mut egui::Ui, settings: &mut Settings) {
    section_heading(
        ui,
        tr("Code", "Code"),
        tr("The editor's and the console's text.", "Le texte de l'éditeur et de la console."),
    );
    ui.horizontal(|ui| {
        ui.label(tr("Size", "Taille"));
        let slider = egui::Slider::new(&mut settings.code_size, settings::CODE_SIZES)
            .step_by(0.5)
            .max_decimals(1)
            .suffix(" pt")
            .trailing_fill(true);
        ui.add(slider);
    });
    ui.horizontal(|ui| {
        ui.label(tr("Tab indents by", "Tab indente de"));
        for width in settings::INDENT_WIDTHS {
            ui.selectable_value(&mut settings.indent_width, width, width.to_string());
        }
        ui.label(tr("spaces", "espaces"));
    });
    ui.add_space(14.0);
    section_heading(
        ui,
        tr("Saving", "Enregistrement"),
        tr(
            "An untitled file waits for Save As… to get a name.",
            "Un fichier sans titre attend Enregistrer sous… pour avoir un nom.",
        ),
    );
    ui.checkbox(&mut settings.autosave, tr("Save files as soon as typing pauses", "Enregistrer dès que la frappe s'arrête"));
}

/// When the model renders, and its color.
fn model_page(ui: &mut egui::Ui, ctx: &egui::Context, settings: &mut Settings) {
    let shortcut = shortcuts::describe(ctx, &settings.shortcuts.get(Action::Render));
    section_heading(
        ui,
        tr("Rendering", "Rendu"),
        &tr!(
            "Off, the model renders only on Render ({}), which suits models that take long.",
            "Désactivé, le modèle ne se calcule que sur Rendu ({}) : pratique pour les modèles longs.",
            shortcut
        ),
    );
    ui.checkbox(
        &mut settings.render_while_typing,
        tr("Render as you type and as parameters change", "Rendre pendant la frappe et quand un paramètre change"),
    );
    ui.add_enabled_ui(settings.render_while_typing, |ui| {
        ui.horizontal(|ui| {
            ui.label(tr("After a pause of", "Après une pause de"));
            let slider = egui::Slider::new(&mut settings.render_delay, settings::RENDER_DELAYS)
                .step_by(50.0)
                .suffix(" ms")
                .trailing_fill(true);
            ui.add(slider);
        });
    });
    ui.add_space(14.0);
    section_heading(
        ui,
        tr("Model color", "Couleur du modèle"),
        tr("The color of the parts without color().", "La couleur des pièces sans color()."),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (name, color) in MODEL_COLORS {
            if color_swatch(ui, color, settings.model_color == color).on_hover_text(name()).clicked() {
                settings.model_color = color;
            }
        }
        ui.add_space(4.0);
        egui::color_picker::color_edit_button_srgb(ui, &mut settings.model_color)
            .on_hover_text(tr("Any color", "Une autre couleur"));
    });
}

/// Each action's keys, which a click records anew, with the actions that
/// share keys; the change asked for.
fn shortcuts_page(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    shortcuts: &Shortcuts,
    recording: Option<Action>,
    problem: Option<&str>,
) -> Option<ShortcutEdit> {
    let p = theme::palette();
    let mut edit = None;
    section_heading(
        ui,
        tr("Keyboard shortcuts", "Raccourcis clavier"),
        tr(
            "Click an action's keys, then press new ones; Escape gives up.",
            "Cliquez sur les touches d'une action, puis tapez les nouvelles ; Échap annule.",
        ),
    );
    let conflicts = shortcuts.conflicts();
    let notes = conflicts.len() + usize::from(problem.is_some());
    let height = SETTINGS_HEIGHT - 92.0 - 22.0 * notes as f32;
    egui::ScrollArea::vertical().max_height(height.max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
        for action in Action::ALL {
            ui.horizontal(|ui| {
                ui.label(RichText::new(action.name()).color(p.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Room for the scroll bar.
                    ui.add_space(10.0);
                    let default = shortcuts::describe(ctx, &action.default_keys());
                    let reset = ui.add_enabled(!shortcuts.is_default(action), Button::new("↺").small());
                    if reset.on_hover_text(tr!("Back to {default}", "Revenir à {default}")).clicked() {
                        edit = Some(ShortcutEdit::Reset(action));
                    }
                    let sharing = shortcuts.sharing(action);
                    let listening = recording == Some(action);
                    let (text, color) = if listening {
                        (tr("Type the keys…", "Tapez les touches…").to_string(), p.accent_strong)
                    } else {
                        let color = if sharing.is_empty() { p.text } else { p.warning };
                        (shortcuts::describe(ctx, &shortcuts.get(action)), color)
                    };
                    let button = Button::new(RichText::new(text).color(color))
                        .selected(listening)
                        .min_size(Vec2::new(140.0, 0.0));
                    let tip = if listening {
                        tr("Press the new keys, or Escape to keep these", "Tapez les nouvelles touches, ou Échap pour garder celles-ci").to_string()
                    } else if sharing.is_empty() {
                        tr("Click, then press new keys", "Cliquez, puis tapez les nouvelles touches").to_string()
                    } else {
                        tr!("Also the keys of {}", "Aussi les touches de {}", names(&sharing))
                    };
                    if ui.add(button).on_hover_text(tip).clicked() {
                        edit = Some(ShortcutEdit::Record(action));
                    }
                });
            });
        }
    });
    ui.add_space(6.0);
    if let Some(problem) = problem {
        ui.label(RichText::new(problem).color(p.warning));
    }
    for group in &conflicts {
        let keys = shortcuts::describe(ctx, &shortcuts.get(group[0]));
        let note = tr!("⚠ {} share {keys}: it does {}.", "⚠ {} partagent {keys} : c'est {} qui s'exécute.", names(group), group[0].name());
        ui.label(RichText::new(note).color(p.warning));
    }
    ui.add_space(4.0);
    if ui.add_enabled(shortcuts.customized(), Button::new(tr("Reset All", "Tout réinitialiser"))).clicked() {
        edit = Some(ShortcutEdit::ResetAll);
    }
    edit
}

/// "Find, Render and Export".
fn names(actions: &[Action]) -> String {
    match actions {
        [] => String::new(),
        [one] => one.name().to_string(),
        [rest @ .., last] => {
            let rest: Vec<&str> = rest.iter().map(|a| a.name()).collect();
            tr!("{} and {}", "{} et {}", rest.join(", "), last.name())
        }
    }
}

/// Where exports go, the Export shortcut's format and each format's
/// options; the change asked for in the folder.
fn export_page(ui: &mut egui::Ui, ctx: &egui::Context, settings: &mut Settings) -> Option<ExportFolderEdit> {
    section_heading(ui, tr("Export folder", "Dossier d'export"), tr("Where Export… opens.", "Là où s'ouvre Exporter…"));
    let mut edit = None;
    if ui.radio(settings.export_folder.is_none(), tr("Next to the design", "À côté du fichier")).clicked() {
        edit = Some(ExportFolderEdit::NextToTheDesign);
    }
    ui.horizontal(|ui| {
        let folder = settings.export_folder.as_deref();
        if ui.radio(folder.is_some(), tr("In this folder:", "Dans ce dossier :")).clicked() && folder.is_none() {
            edit = Some(ExportFolderEdit::Choose);
        }
        if let Some(folder) = folder {
            let missing = !folder.is_dir();
            let color = if missing { theme::palette().warning } else { theme::palette().text };
            let label = ui.add(egui::Label::new(RichText::new(display_folder(folder)).color(color)).truncate());
            let note = if missing {
                tr(" (not found: Export… opens next to the design)", " (introuvable : Exporter… s'ouvre à côté du fichier)")
            } else {
                ""
            };
            label.on_hover_text(format!("{}{note}", folder.display()));
        }
    });
    if ui.button(tr("Choose Folder…", "Choisir un dossier…")).clicked() {
        edit = Some(ExportFolderEdit::Choose);
    }
    let reveal = if cfg!(target_os = "macos") {
        tr("Show each exported file in the Finder", "Montrer chaque fichier exporté dans le Finder")
    } else {
        tr("Show each exported file in its folder", "Montrer chaque fichier exporté dans son dossier")
    };
    ui.checkbox(&mut settings.reveal_export, reveal);
    ui.add_space(14.0);
    let keys = shortcuts::describe(ctx, &settings.shortcuts.get(Action::Export));
    section_heading(
        ui,
        tr("Export shortcut", "Raccourci d'export"),
        &tr!(
            "{} writes a 3D model in this format, a 2D one as SVG.",
            "{} écrit un modèle 3D dans ce format, un modèle 2D en SVG.",
            keys
        ),
    );
    ui.horizontal(|ui| {
        for &format in Format::SOLID {
            ui.selectable_value(&mut settings.export_format, format, format.name());
        }
    });
    ui.add_space(14.0);
    section_heading(
        ui,
        tr("STL files", "Fichiers STL"),
        tr("3D printers' slicers read both.", "Les slicers d'impression 3D lisent les deux."),
    );
    ui.horizontal(|ui| {
        ui.radio_value(&mut settings.binary_stl, false, "ASCII")
            .on_hover_text(tr("Text, the most widely read", "Du texte, lu partout"));
        ui.radio_value(&mut settings.binary_stl, true, tr("Binary", "Binaire")).on_hover_text(tr(
            "About a fifth of the size, quicker to open",
            "Environ cinq fois plus léger, plus rapide à ouvrir",
        ));
    });
    ui.add_space(14.0);
    section_heading(
        ui,
        tr("glTF models", "Modèles glTF"),
        tr(
            "For game engines: Godot, Unity. glTF counts in meters.",
            "Pour les moteurs de jeu : Godot, Unity. glTF compte en mètres.",
        ),
    );
    ui.radio_value(
        &mut settings.gltf_scale,
        GLTF_SCALES[0],
        tr("Real size: a millimeter is a millimeter", "Taille réelle : un millimètre reste un millimètre"),
    );
    ui.radio_value(
        &mut settings.gltf_scale,
        GLTF_SCALES[1],
        tr("A unit is a meter: for levels and scenery", "Une unité vaut un mètre : pour les niveaux et les décors"),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new(tr("A design that reads $t moves its part()s along it:", "Un fichier qui lit $t anime ses part() :"))
            .color(theme::palette().text_dim),
    );
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut settings.gltf_duration).range(settings::GLTF_DURATIONS).speed(0.1).suffix(" s"))
            .on_hover_text(tr("The seconds from $t = 0 to 1", "Les secondes de $t = 0 à 1"));
        ui.add(egui::DragValue::new(&mut settings.gltf_fps).range(settings::GLTF_FPS).suffix(tr(" fps", " i/s")))
            .on_hover_text(tr("Frames sampled a second", "Images par seconde"));
        ui.checkbox(&mut settings.gltf_loop, tr("Loop", "En boucle")).on_hover_text(tr(
            "Its name ends in -loop, which Godot plays over and over",
            "Son nom finit par -loop, que Godot joue en boucle",
        ));
    });
    edit
}

/// The library and font folders; the change asked for in each list.
fn libraries_page(ui: &mut egui::Ui, settings: &Settings) -> (Option<FolderEdit>, Option<FolderEdit>) {
    section_heading(
        ui,
        tr("Library folders", "Dossiers de bibliothèques"),
        tr(
            "Searched for include <…> and use <…> before ~/Documents/Code2Form/libraries, C2F_LIBRARY_PATH and the app's own.",
            "Parcourus pour include <…> et use <…> avant ~/Documents/Code2Form/libraries, C2F_LIBRARY_PATH et ceux de l'app.",
        ),
    );
    let libraries = folder_list(ui, "libraries", &settings.library_folders);
    ui.add_space(14.0);
    section_heading(
        ui,
        tr("Font folders", "Dossiers de polices"),
        tr(
            "Searched for text()'s fonts, with their subfolders, before C2F_FONT_PATH, the app's and the system's.",
            "Parcourus avec leurs sous-dossiers pour les polices de text(), avant C2F_FONT_PATH, ceux de l'app et du système.",
        ),
    );
    let fonts = folder_list(ui, "fonts", &settings.font_folders);
    (libraries, fonts)
}

/// A list of folders, each removable, and a button to add one.
fn folder_list(ui: &mut egui::Ui, id: &str, folders: &[PathBuf]) -> Option<FolderEdit> {
    let mut edit = None;
    let frame =
        Frame::new().fill(theme::palette().editor).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if folders.is_empty() {
            ui.label(RichText::new(tr("None", "Aucun")).color(theme::palette().text_faint));
        }
        for (i, folder) in folders.iter().enumerate() {
            ui.push_id((id, i), |ui| {
                ui.horizontal(|ui| {
                    let missing = !folder.is_dir();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(tr("Remove", "Retirer")).clicked() {
                            edit = Some(FolderEdit::Remove(i));
                        }
                        let color = if missing { theme::palette().warning } else { theme::palette().text };
                        let name = RichText::new(display_folder(folder)).color(color);
                        let label = ui.add(egui::Label::new(name).truncate());
                        let note = if missing { tr(" (not found)", " (introuvable)") } else { "" };
                        label.on_hover_text(format!("{}{note}", folder.display()));
                    });
                });
            });
        }
    });
    if ui.button(tr("Add Folder…", "Ajouter un dossier…")).clicked() {
        edit = Some(FolderEdit::Add);
    }
    edit
}

/// Applies `edit` to `folders`, asking which folder to add; whether the
/// list changed.
fn edit_folders(folders: &mut Vec<PathBuf>, edit: FolderEdit, title: &str) -> bool {
    match edit {
        FolderEdit::Remove(i) if i < folders.len() => {
            folders.remove(i);
            true
        }
        FolderEdit::Remove(_) => false,
        FolderEdit::Add => {
            let Some(folder) = rfd::FileDialog::new().set_title(title).pick_folder() else { return false };
            if folders.contains(&folder) {
                return false;
            }
            folders.push(folder);
            true
        }
    }
}

/// The app icon in `tint`, `pixels` square, for the settings.
fn icon_texture(ctx: &egui::Context, tint: Tint, pixels: usize) -> TextureHandle {
    let image = app_icon::render(tint, pixels, app_icon::Layout::Full);
    let image = egui::ColorImage::from_rgba_premultiplied([pixels, pixels], &image.premultiplied);
    ctx.load_texture(format!("app-icon-{}-{pixels}", tint.id()), image, TextureOptions::LINEAR)
}

/// The icon at the toolbar's left: 18 points, drawn for sharp screens.
fn logo_texture(ctx: &egui::Context, tint: Tint) -> TextureHandle {
    icon_texture(ctx, tint, (18.0 * ctx.pixels_per_point().max(2.0)).ceil() as usize)
}

/// One icon the settings offer, with its name, ringed when picked.
fn icon_choice(ui: &mut egui::Ui, texture: &TextureHandle, tint: Tint, selected: bool) -> egui::Response {
    let size = Vec2::new(ICON_CHOICE + 26.0, ICON_CHOICE + 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, tint.name()));
    let center = rect.center_top() + Vec2::new(0.0, 8.0 + ICON_CHOICE / 2.0);
    let icon = Rect::from_center_size(center, Vec2::splat(ICON_CHOICE));
    let painter = ui.painter();
    if selected || response.hovered() {
        let (color, width): (Color32, f32) =
            if selected { (theme::palette().accent, 2.5) } else { (theme::palette().border, 1.5) };
        painter.rect_stroke(icon.expand(4.0), CornerRadius::same(14), Stroke::new(width, color), StrokeKind::Outside);
    }
    egui::Image::new(texture).paint_at(ui, icon);
    let color = if selected { theme::palette().text } else { theme::palette().text_dim };
    painter.text(
        rect.center_bottom() - Vec2::new(0.0, 2.0),
        egui::Align2::CENTER_BOTTOM,
        tint.name(),
        egui::FontId::proportional(11.5),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A model color the settings offer, ringed when picked.
fn color_swatch(ui: &mut egui::Ui, color: [u8; 3], selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let painter = ui.painter();
    let [r, g, b] = color;
    painter.rect_filled(rect.shrink(3.0), CornerRadius::same(5), Color32::from_rgb(r, g, b));
    if selected || response.hovered() {
        let (stroke, width): (Color32, f32) =
            if selected { (theme::palette().accent, 2.0) } else { (theme::palette().border, 1.5) };
        painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(width, stroke), StrokeKind::Inside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// sRGB bytes as the renderer's floats.
fn color_floats(color: [u8; 3]) -> [f32; 3] {
    color.map(|c| f32::from(c) / 255.0)
}

/// The interface size `steps` tenths up (or down), within the settings'
/// range, in whole tenths as egui's own zoom keys.
fn zoom_step(size: f32, steps: i32) -> f32 {
    let size = ((size * 10.0).round() + steps as f32) / 10.0;
    size.clamp(*settings::INTERFACE_SIZES.start(), *settings::INTERFACE_SIZES.end())
}

fn toolbar_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 20.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, theme::palette().border));
}

/// The slider of a circuit's enclosure's opacity; moving it shows the
/// enclosure if it was hidden.
fn opacity_slider(ui: &mut egui::Ui, viewer: &mut viewer::Viewer) {
    ui.label(tr("Opacity", "Opacité"));
    let mut opacity = viewer.enclosure_opacity;
    let slider = egui::Slider::new(&mut opacity, 0.0..=1.0)
        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
        .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0));
    let hint = tr("How opaque the enclosure is drawn", "L'opacité du boîtier");
    if ui.add(slider).on_hover_text(hint).changed() {
        viewer.enclosure_opacity = opacity;
        viewer.set_show_enclosure(true);
    }
}

/// The section and the enclosure's opacity a screenshot asks for among
/// `select`'s words: `cut:z:20`, `opacity:100` (in percent).
fn section_asked(select: &str) -> (Option<renderer::Section>, Option<f32>) {
    let words = || select.split(',').map(str::trim);
    let cut = words().find_map(|w| renderer::Section::parse(w.strip_prefix("cut:")?));
    let opacity = words().find_map(|w| w.strip_prefix("opacity:")?.trim().parse::<f32>().ok()).map(|v| (v / 100.0).clamp(0.0, 1.0));
    (cut, opacity)
}

/// A thin line between groups of the view's tools.
fn tool_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(7.0, 18.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, theme::palette().border));
}

/// The cards over the 3D view: the view's tools, the parameters, the
/// animation bar.
fn card_frame() -> Frame {
    let palette = theme::palette();
    Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(10))
        .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if palette.dark { 90 } else { 30 }) })
}

/// How the window is shared between the code and the 3D view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arrangement {
    Code,
    Split,
    View,
}

impl Arrangement {
    const ALL: [Arrangement; 3] = [Arrangement::Code, Arrangement::Split, Arrangement::View];

    /// As a screenshot run names it.
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "code" => Some(Arrangement::Code),
            "split" => Some(Arrangement::Split),
            "view" => Some(Arrangement::View),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Arrangement::Code => "Code",
            Arrangement::Split => tr("Side by side", "Côte à côte"),
            Arrangement::View => tr("View", "Vue"),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Arrangement::Code => tr("The code alone, across the window", "Le code seul, sur toute la fenêtre"),
            Arrangement::Split => tr("The code beside the 3D view", "Le code à côté de la vue 3D"),
            Arrangement::View => tr("The 3D view alone, the console under it", "La vue 3D seule, la console dessous"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Arrangement::Code => Icon::CodeOnly,
            Arrangement::Split => Icon::SideBySide,
            Arrangement::View => Icon::ViewOnly,
        }
    }
}

/// The three layouts side by side, the one shown pressed; the one
/// clicked, if another.
fn layout_switch(ui: &mut egui::Ui, current: Arrangement) -> Option<Arrangement> {
    let palette = theme::palette();
    let mut picked = None;
    Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for arrangement in Arrangement::ALL {
                    let on = arrangement == current;
                    let font = egui::FontId::proportional(12.5);
                    // Colored when painted, once it is known whether it is hovered.
                    let galley = ui.painter().layout_no_wrap(arrangement.label().to_string(), font, Color32::PLACEHOLDER);
                    let size = Vec2::new(galley.size().x + 40.0, 24.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    let fill = if on {
                        palette.accent_muted
                    } else if response.hovered() {
                        palette.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                    let color = if on {
                        icons::on_muted()
                    } else if response.hovered() {
                        palette.text
                    } else {
                        palette.text_dim
                    };
                    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(15.0));
                    icons::paint(ui.painter(), icon, arrangement.icon(), color);
                    let text = egui::pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0);
                    ui.painter().galley(text, galley, color);
                    if response.on_hover_text(arrangement.hint()).clicked() && !on {
                        picked = Some(arrangement);
                    }
                }
            });
        });
    picked
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

/// `text` cut to at most `max` characters, keeping its start and its end
/// around an ellipsis.
fn shorten_middle(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 3;
    let tail = keep - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
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

/// Shows `file` in the Finder, selected, or opens its folder elsewhere.
fn reveal(file: &Path) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(file);
        command
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("explorer");
        let mut select = std::ffi::OsString::from("/select,");
        select.push(file);
        command.arg(select);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(file.parent().unwrap_or(Path::new(".")));
        command
    };
    command.spawn().map(drop)
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
    fn zoom_keys_step_in_tenths_within_the_range() {
        assert_eq!(zoom_step(1.0, 1), 1.1);
        assert_eq!(zoom_step(1.0, -1), 0.9);
        assert_eq!(zoom_step(1.25, 1), 1.4, "back on a tenth");
        assert_eq!(zoom_step(1.5, 1), 1.5);
        assert_eq!(zoom_step(0.8, -1), 0.8);
    }

    #[test]
    fn long_folders_keep_their_start_and_end() {
        assert_eq!(shorten_middle("~/Documents/Code2Form", 48), "~/Documents/Code2Form");
        let long = "~/Library/Application Support/Claude/scratch-workspaces/f07cb7c0/scratch-2026-10-23-b6b842";
        let short = shorten_middle(long, 48);
        assert_eq!(short.chars().count(), 48);
        assert!(short.starts_with("~/Library/Appl") && short.ends_with("scratch-2026-10-23-b6b842"), "{short}");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_millis(42)), "42 ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.50 s");
    }

    /// A folder of its own for a test, with designs `a.c2f` and `b.c2f`.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code2form-app-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.c2f"), "cube(1);\n").unwrap();
        std::fs::write(dir.join("b.c2f"), "cube([2, 2, 4]);\n").unwrap();
        dir
    }

    /// The app without a window, nor the user's Open Recent list.
    fn studio(ctx: &egui::Context, store: Option<PathBuf>, paths: Vec<PathBuf>) -> StudioApp {
        let stores = Stores { session: store, settings: None, conversations: None };
        StudioApp::with(ctx, None, stores, RecentFiles::default(), Settings::load(None), paths)
    }

    /// The files of the tabs, in order, and the active tab.
    fn open_files(app: &StudioApp) -> (Vec<Option<String>>, usize) {
        let name = |tab: &Tab| tab.path.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
        (app.shelf.iter(&app.doc).map(name).collect(), app.shelf.active())
    }

    fn tab(app: &StudioApp, index: usize) -> &Tab {
        app.shelf.iter(&app.doc).nth(index).unwrap()
    }

    /// Takes the worker's replies until `done`.
    fn wait(app: &mut StudioApp, done: impl Fn(&StudioApp) -> bool) {
        let start = Instant::now();
        while !done(app) {
            assert!(start.elapsed() < Duration::from_secs(30), "no reply");
            std::thread::sleep(Duration::from_millis(5));
            app.receive();
        }
    }

    fn files(names: &[&str]) -> Vec<Option<String>> {
        names.iter().map(|n| (!n.is_empty()).then(|| n.to_string())).collect()
    }

    #[test]
    fn tabs_open_switch_and_close() {
        let dir = folder("tabs");
        let ctx = egui::Context::default();
        // The blank tab the app starts with takes the first file.
        let mut app = studio(&ctx, None, vec![dir.join("a.c2f"), dir.join("b.c2f")]);
        assert_eq!(open_files(&app), (files(&["a.c2f", "b.c2f"]), 1));
        // A file open already: its tab, by whatever path.
        app.open(&ctx, dir.join("sub").join("..").join("a.c2f"));
        assert_eq!(open_files(&app), (files(&["a.c2f", "b.c2f"]), 0));
        app.new_tab(&ctx);
        assert_eq!(open_files(&app), (files(&["a.c2f", "b.c2f", ""]), 2));
        // Round and round, both ways.
        for (forward, active) in [(true, 0), (true, 1), (false, 0), (false, 2)] {
            app.select_tab(&ctx, tabs::step(app.shelf.active(), app.shelf.len(), forward));
            assert_eq!(app.shelf.active(), active);
        }
        // The untitled tab closes at once, the previous one taking over.
        app.close_tab(&ctx, 2);
        assert_eq!(open_files(&app), (files(&["a.c2f", "b.c2f"]), 1));
        // A modified tab asks first, in its own tab.
        app.doc.text.push_str("sphere(1);\n");
        app.select_tab(&ctx, 0);
        app.close_tab(&ctx, 1);
        assert!(matches!(app.confirm, Some(Pending::Close)));
        assert_eq!(open_files(&app), (files(&["a.c2f", "b.c2f"]), 1));
        app.confirm = None;
        app.finish(&ctx, Pending::Close);
        assert_eq!(open_files(&app), (files(&["a.c2f"]), 0));
        // The last tab gives way to an untitled one.
        app.close_tab(&ctx, 0);
        assert_eq!(open_files(&app), (files(&[""]), 0));
        assert!(app.doc.is_blank());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_tab_takes_the_model_color_and_the_indent_width() {
        let dir = folder("tab-settings");
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, None, vec![dir.join("a.c2f"), dir.join("b.c2f")]);
        app.settings.model_color = [255, 0, 0];
        app.settings.indent_width = 8;
        app.apply_tab_settings();
        // And a tab opened afterwards.
        app.new_tab(&ctx);
        assert_eq!(app.shelf.len(), 3);
        for index in 0..3 {
            let tab = tab(&app, index);
            assert_eq!(tab.viewer.model_color, [1.0, 0.0, 0.0], "tab {index}");
            assert_eq!(tab.editor.indent_width, 8, "tab {index}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A frame of the app's shortcuts, with presses of `keys`.
    fn press(ctx: &egui::Context, app: &mut StudioApp, keys: &[(Modifiers, Key)]) {
        let mac = cfg!(target_os = "macos");
        let events = keys
            .iter()
            .map(|&(modifiers, key)| {
                // As winit reports ⌘ on macOS, and Ctrl elsewhere.
                let modifiers = Modifiers {
                    mac_cmd: mac && modifiers.command,
                    ctrl: modifiers.ctrl || (!mac && modifiers.command),
                    ..modifiers
                };
                egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
            })
            .collect();
        let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| app.shortcuts(ctx));
    }

    #[test]
    fn recorded_keys_do_their_action_in_every_tab() {
        let dir = folder("shortcuts");
        let ctx = egui::Context::default();
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut app = studio(&ctx, None, vec![dir.join("a.c2f"), dir.join("b.c2f")]);
        let next_tab = (Modifiers { shift: true, ..Modifiers::COMMAND }, Key::CloseBracket);
        press(&ctx, &mut app, &[next_tab]);
        assert_eq!(app.shelf.active(), 0, "the default keys");

        // Typing keys are refused; Escape gives up; ⌘K is taken.
        app.recording = Some(Action::NextTab);
        press(&ctx, &mut app, &[(Modifiers::NONE, Key::K)]);
        assert!(app.record_problem.is_some());
        assert_eq!(app.recording, Some(Action::NextTab), "still waiting");
        press(&ctx, &mut app, &[(Modifiers::NONE, Key::Escape)]);
        assert_eq!((app.recording, app.record_problem.as_ref()), (None, None));
        assert!(app.settings.shortcuts.is_default(Action::NextTab));
        app.recording = Some(Action::NextTab);
        press(&ctx, &mut app, &[(Modifiers::COMMAND, Key::K)]);
        assert_eq!(app.recording, None);
        assert_eq!(app.settings.shortcuts.get(Action::NextTab), KeyboardShortcut::new(Modifiers::COMMAND, Key::K));

        press(&ctx, &mut app, &[next_tab]);
        assert_eq!(app.shelf.active(), 0, "the old keys do nothing now");
        press(&ctx, &mut app, &[(Modifiers::COMMAND, Key::K)]);
        assert_eq!(app.shelf.active(), 1);

        // The find bar's tips show the keys in every tab.
        let alt_f3 = KeyboardShortcut::new(Modifiers::ALT, Key::F3);
        app.settings.shortcuts.set(Action::FindNext, alt_f3);
        app.apply_tab_settings();
        for index in 0..2 {
            assert_eq!(tab(&app, index).editor.find_keys[0], alt_f3, "tab {index}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replies_reach_the_tab_that_asked() {
        let dir = folder("replies");
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, None, vec![dir.join("a.c2f"), dir.join("b.c2f")]);
        wait(&mut app, |app| !app.doc.busy());
        let triangles = |tab: &Tab| tab.model.as_ref().map(|m| m.stats.triangles);
        assert_eq!(triangles(tab(&app, 1)), Some(12));
        // Shown, the first tab renders unless its render came back.
        app.select_tab(&ctx, 0);
        wait(&mut app, |app| !app.doc.busy());
        assert_eq!(triangles(&app.doc), Some(12));

        // A render on its way when the tab is left comes back to it.
        app.doc.text = "sphere(1);\n".to_string();
        app.render_now();
        let (asked, b_finished) = (app.doc.sent, tab(&app, 1).finished);
        app.select_tab(&ctx, 1);
        assert_eq!(app.worker.latest(), asked, "the second tab had nothing to render");
        wait(&mut app, |app| !tab(app, 0).busy());
        assert_eq!(tab(&app, 0).finished, asked);
        assert!(triangles(tab(&app, 0)) > Some(12), "the sphere");
        assert_eq!(tab(&app, 1).finished, b_finished);
        assert_eq!(app.exportable, Some(tab(&app, 0).id), "the worker holds the first tab's model");

        // One that gave way to another tab's render: shown, the tab
        // renders again, unless it came back meanwhile.
        app.select_tab(&ctx, 0);
        app.render_now();
        let asked = app.doc.sent;
        app.select_tab(&ctx, 1);
        app.doc.stale = true;
        app.show_tab(&ctx);
        let other = app.doc.sent;
        app.select_tab(&ctx, 0);
        assert!(app.doc.finished == asked || app.doc.sent > other, "rendered again");

        // A tab closed with a render on its way: the reply is dropped.
        wait(&mut app, |app| !app.doc.busy());
        app.doc.text = "cube(3);\n".to_string();
        app.doc.saved_text = app.doc.text.clone();
        app.render_now();
        let closed = app.doc.sent;
        app.close_tab(&ctx, 0);
        assert_eq!(open_files(&app), (files(&["b.c2f"]), 0));
        wait(&mut app, |app| app.owners.owner(closed).is_none());
        // The second tab still shows its own box, 2 × 2 × 4.
        assert_eq!(app.doc.model.as_ref().map(|m| m.stats.volume), Some(16.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tabs_stay_closed_when_the_settings_say() {
        let dir = folder("no-session");
        let store = dir.join("settings").join("open-tabs.txt");
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, Some(store.clone()), vec![dir.join("a.c2f")]);
        app.save_session();
        drop(app);
        let stores = Stores { session: Some(store), settings: None, conversations: None };
        let settings = Settings { reopen_tabs: false, ..Settings::default() };
        let app = StudioApp::with(&ctx, None, stores, RecentFiles::default(), settings, vec![dir.join("b.c2f")]);
        assert_eq!(open_files(&app), (files(&["b.c2f"]), 0), "only the file given");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn autosave_writes_once_typing_pauses() {
        let dir = folder("autosave");
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, None, vec![dir.join("a.c2f"), dir.join("b.c2f")]);
        app.doc.text.push_str("sphere(1);\n");
        // Off, nothing is written.
        app.autosave_now();
        assert!(app.doc.is_dirty());
        app.settings.autosave = true;
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.autosave(ctx));
        assert!(app.doc.is_dirty(), "not before the pause");
        app.doc.autosave_deadline = Some(Instant::now());
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.autosave(ctx));
        assert!(!app.doc.is_dirty());
        assert_eq!(std::fs::read_to_string(dir.join("b.c2f")).unwrap(), "cube([2, 2, 4]);\nsphere(1);\n");
        // Leaving a tab saves it at once.
        app.doc.text.push_str("cube(9);\n");
        app.select_tab(&ctx, 0);
        assert!(std::fs::read_to_string(dir.join("b.c2f")).unwrap().ends_with("cube(9);\n"));
        // An untitled tab waits for a name.
        app.new_tab(&ctx);
        app.doc.text.push_str("cube(1);\n");
        app.autosave_now();
        assert!(app.doc.is_dirty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_export_shortcut_takes_the_settings_format() {
        let dir = folder("export-format");
        std::fs::write(dir.join("flat.c2f"), "square(4);\n").unwrap();
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, None, vec![dir.join("a.c2f")]);
        app.settings.export_format = Format::ThreeMf;
        wait(&mut app, |app| !app.doc.busy());
        assert_eq!(app.shortcut_format(), Format::ThreeMf);
        app.open(&ctx, dir.join("flat.c2f"));
        wait(&mut app, |app| !app.doc.busy());
        assert_eq!(app.shortcut_format(), Format::Svg, "a 2D model");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tabs_come_back_at_the_next_launch() {
        let dir = folder("session");
        let store = dir.join("settings").join("open-tabs.txt");
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, Some(store.clone()), vec![dir.join("a.c2f"), dir.join("b.c2f")]);
        app.new_tab(&ctx);
        app.select_tab(&ctx, 0);
        app.save_session();
        drop(app);
        // Untitled tabs are not kept.
        let mut app = studio(&ctx, Some(store.clone()), Vec::new());
        assert_eq!(open_files(&app), (files(&["a.c2f", "b.c2f"]), 0));
        // A file gone since is not reopened; one given at launch shows.
        std::fs::remove_file(dir.join("a.c2f")).unwrap();
        std::fs::write(dir.join("c.c2f"), "cube(1);\n").unwrap();
        app.select_tab(&ctx, 1);
        app.save_session();
        drop(app);
        let app = studio(&ctx, Some(store), vec![dir.join("c.c2f")]);
        assert_eq!(open_files(&app), (files(&["b.c2f", "c.c2f"]), 1));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
