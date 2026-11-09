//! The project open in the studio: its panel on the left, by section
//! (Brief, Designs, Scans, Circuit, Firmware, Exports) or as the files of
//! its folder, each file a click away; its own tabs, put away while
//! another project is open and back with it; File › New Project, Open
//! Project and Make a Project of a folder; and the files the agent or the
//! iPhone puts in its folders, taken in as they come.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use c2f_project::{Kind, Project};
use eframe::egui::{self, Button, Frame, Margin, RichText, Sense};

use super::{StudioApp, reveal};
use crate::console::Level;
use crate::i18n::tr;
use crate::theme;

/// How often the project's folder is looked at for files added, by the
/// agent or from the iPhone.
const POLL: Duration = Duration::from_secs(2);

/// The project open, and what its panel shows.
pub(super) struct OpenProject {
    pub project: Project,
    /// The files in its exports folder, as last listed.
    exports: Vec<PathBuf>,
    /// The files seen in its models/ and scans/: only those that arrive
    /// after join the project, so that one taken out of it stays out.
    seen: HashSet<PathBuf>,
    polled: Instant,
}

impl OpenProject {
    fn new(project: Project) -> OpenProject {
        let mut open = OpenProject { project, exports: Vec::new(), seen: HashSet::new(), polled: Instant::now() };
        open.seen = open.arrivals().into_iter().map(|(_, p)| p).collect();
        open.list_exports();
        open
    }

    /// The designs and scans in the project's folders, with their kind.
    fn arrivals(&self) -> Vec<(Kind, PathBuf)> {
        let mut found = Vec::new();
        for (kind, folder) in [(Kind::Design, "models"), (Kind::Scan, "scans")] {
            let Ok(entries) = std::fs::read_dir(self.project.dir.join(folder)) else { continue };
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| !hidden(p) && Kind::of(p) == Some(kind)).collect();
            files.sort();
            found.extend(files.into_iter().map(|f| (kind, f)));
        }
        found
    }

    fn list_exports(&mut self) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.project.exports_dir())
            .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && !hidden(p)).collect())
            .unwrap_or_default();
        files.sort();
        self.exports = files;
    }

    /// Reads the project's file again, and takes in the designs and scans
    /// put in its folders since; whether it changed.
    fn refresh(&mut self) -> Result<bool, String> {
        let mut project = reread(&self.project.file)?;
        let mut added = false;
        for (kind, file) in self.arrivals() {
            if !self.seen.insert(file.clone()) {
                continue;
            }
            let listed = project.relative(&file).is_some_and(|r| project.designs.contains(&r) || project.scans.contains(&r));
            if !listed && project.add(kind, &file).is_ok() {
                added = true;
            }
        }
        if added {
            project.save()?;
        }
        let changed = project != self.project;
        self.project = project;
        self.list_exports();
        Ok(changed)
    }
}

/// The project's file as it is on disk now: a change made since it was
/// last read (by hand, by the agent, from the iPhone) is kept.
fn reread(file: &Path) -> Result<Project, String> {
    Project::open(file).map_err(|p| p.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))
}

fn hidden(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// What the panel on the left shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PanelView {
    /// The project by section.
    #[default]
    Project,
    /// The files of its folder, as they are on disk.
    Files,
}

/// A folder's files, as last listed, for the Files view.
pub(super) struct FileTree {
    root: PathBuf,
    nodes: Vec<Node>,
    listed: Instant,
}

struct Node {
    path: PathBuf,
    name: String,
    /// A folder's files; `None` for a file.
    children: Option<Vec<Node>>,
}

/// The folders the Files view leaves out: a firmware's build, a cache.
const SKIPPED: [&str; 4] = [".pio", "node_modules", "target", "__pycache__"];

impl FileTree {
    fn of(root: &Path) -> FileTree {
        let mut count = 0;
        FileTree { root: root.to_path_buf(), nodes: list(root, 0, &mut count), listed: Instant::now() }
    }
}

/// The files and folders in `dir`, folders first, down to six levels and
/// two thousand entries.
fn list(dir: &Path, depth: usize, count: &mut usize) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut entries: Vec<(bool, PathBuf, String)> = entries
        .flatten()
        .map(|e| (e.file_type().is_ok_and(|t| t.is_dir()), e.path(), e.file_name().to_string_lossy().into_owned()))
        .filter(|(_, _, name)| !name.starts_with('.') && !SKIPPED.contains(&name.as_str()))
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase())));
    let mut nodes = Vec::new();
    for (folder, path, name) in entries {
        *count += 1;
        if *count > 2000 {
            break;
        }
        let children = folder.then(|| if depth < 6 { list(&path, depth + 1, count) } else { Vec::new() });
        nodes.push(Node { path, name, children });
    }
    nodes
}

/// The tabs of a project not open, put away until it is again: `None` for
/// the files outside any project.
pub(super) struct Parked {
    pub project: Option<PathBuf>,
    pub tabs: Vec<super::Tab>,
    pub active: usize,
}

/// What a click in the panel asks for.
enum PanelAction {
    Open(PathBuf),
    OpenFirmware(PathBuf),
    /// A file of the Files view: opened in the tab it takes.
    OpenFile(PathBuf),
    Reveal(PathBuf),
    NewDesign,
    ImportScan,
    /// A scan to set straight, once confirmed.
    LevelScan(PathBuf),
    NewCircuit,
    View(PanelView),
    Close,
}

impl StudioApp {
    /// Opens the project `path` names (its file, or its folder), in its
    /// panel, and its main design or circuit when no file is open.
    pub(super) fn open_project(&mut self, ctx: &egui::Context, path: &Path) {
        match Project::open(path) {
            Ok(project) => {
                self.remember(&project.file);
                let first = project.main_design().or_else(|| project.circuit_path()).filter(|p| p.is_file());
                self.switch_project(ctx, Some(project));
                if self.doc.is_blank() && self.shelf.len() == 1 {
                    if let Some(first) = first {
                        self.open(ctx, first);
                    }
                }
            }
            Err(problems) => {
                for problem in problems {
                    self.doc.console.log(Level::Error, tr!("{}: {problem}", "{} : {problem}", path.display()));
                }
                self.console_open = true;
            }
        }
    }

    /// A file about to open goes with its project's tabs: the project open
    /// changes when the file is another's, or in none.
    pub(super) fn follow_project(&mut self, ctx: &egui::Context, file: &Path) {
        let project = Project::of(file);
        let key = |file: &Path| std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let now = self.project.as_ref().map(|o| key(&o.project.file));
        if project.as_ref().map(|p| key(&p.file)) != now {
            self.switch_project(ctx, project);
        }
    }

    /// Opens `project`, or none: the tabs open go away with the project
    /// they belong to, and those of `project` come back as they were left.
    pub(super) fn switch_project(&mut self, ctx: &egui::Context, project: Option<Project>) {
        let key = |file: &Path| std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let from = self.project.as_ref().map(|o| key(&o.project.file));
        let to = project.as_ref().map(|p| key(&p.file));
        if from == to {
            if let Some(project) = project {
                self.project = Some(OpenProject::new(project));
            }
            return;
        }
        // The tabs open now, untitled empty ones left out.
        let blank = self.fresh_tab();
        let (tabs, active) = self.shelf.take_all(&mut self.doc, blank);
        let active_id = tabs.get(active).map(|t| t.id);
        let mut kept: Vec<super::Tab> = Vec::new();
        for mut tab in tabs {
            if tab.is_blank() {
                tab.editor.forget(ctx);
                if let Some(gl) = &self.gl {
                    tab.viewer.destroy(gl);
                }
                continue;
            }
            // Out of sight, it renders again once back.
            tab.stale = true;
            kept.push(tab);
        }
        if !kept.is_empty() {
            let active = active_id.and_then(|id| kept.iter().position(|t| t.id == id)).unwrap_or(0);
            self.parked.retain(|p| p.project != from);
            self.parked.push(Parked { project: from, tabs: kept, active });
        }
        self.project = project.map(OpenProject::new);
        self.folder_tree = None;
        if let Some(index) = self.parked.iter().position(|p| p.project == to) {
            let parked = self.parked.remove(index);
            let blank = self.shelf.put_all(&mut self.doc, parked.tabs, parked.active);
            if let Some(gl) = &self.gl {
                let mut viewer = blank.viewer;
                viewer.destroy(gl);
            }
            self.show_tab(ctx);
        }
    }

    /// The project put away that has unsaved edits, to show while
    /// quitting, those already asked about left out.
    pub(super) fn parked_unsaved(&self) -> Option<Option<PathBuf>> {
        self.parked
            .iter()
            .find(|p| p.tabs.iter().any(|t| t.is_dirty() && !self.quit_cleared.contains(&t.id)))
            .map(|p| p.project.clone())
    }

    /// Brings back the project (or the files out of any) `key` names.
    pub(super) fn bring_back(&mut self, ctx: &egui::Context, key: Option<PathBuf>) {
        let project = key.as_deref().and_then(|file| Project::open(file).ok());
        if key.is_some() && project.is_none() {
            // Its file is gone: its tabs come back with the files out of
            // any project.
            if let Some(parked) = self.parked.iter_mut().find(|p| p.project == key) {
                parked.project = None;
            }
        }
        self.switch_project(ctx, project);
    }

    /// File › New Project: a folder picked, the project made in it.
    pub(super) fn new_project_dialog(&mut self, ctx: &egui::Context) {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        let projects = home.join("Documents").join("Code2Form").join("Projects");
        let _ = std::fs::create_dir_all(&projects);
        let dialog = rfd::FileDialog::new()
            .set_title(tr("New project: its name and where it goes", "Nouveau projet : son nom et son emplacement"))
            .set_directory(&projects)
            .set_file_name(tr("My object", "Mon objet"));
        let Some(dir) = dialog.save_file() else { return };
        let name = dir.file_name().map_or_else(|| "Project".into(), |n| n.to_string_lossy().into_owned());
        match c2f_project::create(&dir, &name) {
            Ok(project) => {
                self.doc.console.log(Level::Info, tr!("Created the project {}", "Projet créé : {}", project.dir.display()));
                self.open_project(ctx, &project.file);
            }
            Err(error) => self.doc.console.log(Level::Error, error),
        }
    }

    /// File › Open Project: a `.c2fproj`, or a folder that has one.
    pub(super) fn open_project_dialog(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new()
            .set_title(tr("Open a project", "Ouvrir un projet"))
            .add_filter(tr("Code2Form project", "Projet Code2Form"), &[c2f_project::EXTENSION]);
        if let Some(file) = dialog.pick_file() {
            self.open_project(ctx, &file);
        }
    }

    /// File › Make a Project of This Folder: the folder of the file open,
    /// its files where they are.
    pub(super) fn adopt_project(&mut self) {
        let Some(dir) = self.doc.path.as_deref().and_then(Path::parent).map(Path::to_path_buf) else { return };
        match c2f_project::adopt(&dir, None) {
            Ok(project) => {
                self.doc.console.log(Level::Info, tr!("{} is a project now", "{} est maintenant un projet", dir.display()));
                // The tabs open stay: they are the project's now.
                self.project = Some(OpenProject::new(project));
            }
            Err(error) => self.doc.console.log(Level::Error, error),
        }
    }

    /// Whether the file open could become a project: it is saved, and in
    /// none.
    pub(super) fn can_adopt(&self) -> bool {
        self.doc.path.as_deref().is_some_and(|p| Project::of(p).is_none())
    }

    /// The project's folder, when one is open: what the agent works in,
    /// and where its exports go.
    pub(super) fn project_dir(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| p.project.dir.clone())
    }

    /// Where an export of the file open goes: the exports of its project,
    /// when it is in one.
    pub(super) fn project_exports(&self) -> Option<PathBuf> {
        let open = self.project.as_ref()?;
        let file = self.doc.path.as_deref()?;
        open.project.relative(file)?;
        let dir = open.project.exports_dir();
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// The folder the Files view shows: the project's, else the file
    /// open's.
    fn files_root(&self) -> Option<PathBuf> {
        self.project_dir().or_else(|| self.doc.path.as_deref().and_then(Path::parent).map(Path::to_path_buf))
    }

    /// Looks at the project's folder now and then.
    pub(super) fn poll_project(&mut self, ctx: &egui::Context) {
        let shows_files = self.left_view == PanelView::Files && (self.project.is_some() || self.folder_shown);
        if shows_files {
            let root = self.files_root();
            let stale = match (&self.folder_tree, &root) {
                (Some(tree), Some(root)) => tree.root != *root || tree.listed.elapsed() >= POLL,
                (None, Some(_)) => true,
                _ => false,
            };
            if stale {
                self.folder_tree = root.as_deref().map(FileTree::of);
            }
            ctx.request_repaint_after(POLL);
        }
        let Some(open) = self.project.as_mut() else { return };
        if open.polled.elapsed() < POLL {
            ctx.request_repaint_after(POLL);
            return;
        }
        open.polled = Instant::now();
        if let Err(error) = open.refresh() {
            self.doc.console.log(Level::Error, error);
            // Not every two seconds.
            open.polled = Instant::now() + Duration::from_secs(60);
        }
        ctx.request_repaint_after(POLL);
    }

    /// The project's panel, on the left.
    pub(super) fn project_panel(&mut self, ctx: &egui::Context) {
        if self.project.is_none() && !self.folder_shown {
            return;
        }
        let palette = theme::palette();
        let mut action = None;
        let frame = Frame::new().fill(palette.panel).inner_margin(Margin { left: 12, right: 10, top: 10, bottom: 10 });
        let view = if self.project.is_some() { self.left_view } else { PanelView::Files };
        egui::SidePanel::left("project").frame(frame).resizable(true).default_width(210.0).width_range(160.0..=360.0).show(ctx, |ui| {
            let title = match &self.project {
                Some(open) => open.project.name.clone(),
                None => self.files_root().and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default(),
            };
            ui.horizontal(|ui| {
                let folder = self.files_root().map(|r| r.display().to_string()).unwrap_or_default();
                ui.label(RichText::new(title).strong().size(14.0).color(palette.text)).on_hover_text(folder);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let hint = if self.project.is_some() { tr("Close the project", "Fermer le projet") } else { tr("Hide the folder", "Masquer le dossier") };
                    if ui.small_button("×").on_hover_text(hint).clicked() {
                        action = Some(PanelAction::Close);
                    }
                });
            });
            if self.project.is_some() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for (choice, label) in [(PanelView::Project, tr("Project", "Projet")), (PanelView::Files, tr("Files", "Fichiers"))] {
                        if ui.add(Button::selectable(view == choice, RichText::new(label).size(12.0))).clicked() && view != choice {
                            action = Some(PanelAction::View(choice));
                        }
                    }
                });
            }
            ui.add_space(6.0);
            if view == PanelView::Files {
                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    let current = self.doc.path.clone();
                    match &self.folder_tree {
                        Some(tree) => show_nodes(ui, &tree.nodes, 0, current.as_deref(), &mut action),
                        None => {
                            ui.label(RichText::new(tr("Listing…", "Lecture…")).size(11.5).color(palette.text_faint));
                        }
                    }
                });
                return;
            }
            let Some(open) = &self.project else { return };
            let project = &open.project;
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                let current = self.doc.path.clone();
                // A file's line: whether it was clicked, and the line.
                let row = |ui: &mut egui::Ui, path: &Path, label: String| -> (bool, egui::Response) {
                    let exists = path.exists();
                    let on = current.as_deref() == Some(path);
                    let text = RichText::new(if exists { label } else { format!("{label} ({})", tr("missing", "manquant")) }).size(12.5);
                    let text = if on { text.strong().color(palette.accent_strong) } else if exists { text.color(palette.text) } else { text.color(palette.text_faint) };
                    let response = ui.add(egui::Label::new(text).truncate().sense(Sense::click())).on_hover_text(path.display().to_string());
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    (exists && response.clicked(), response)
                };
                let entry = |ui: &mut egui::Ui, path: &Path, label: String| -> bool { row(ui, path, label).0 };
                let name = |path: &Path| path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                // A section's title, and its + when `add` says what it adds:
                // whether it was clicked.
                let heading = |ui: &mut egui::Ui, title: &str, add: Option<&str>| -> bool {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).size(11.5).strong().color(palette.text_dim));
                        let Some(hint) = add else { return false };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(Button::new(RichText::new("+").size(12.0)).frame(false)).on_hover_text(hint).clicked()
                        })
                        .inner
                    })
                    .inner
                };

                if let Some(brief) = project.brief_path() {
                    heading(ui, tr("BRIEF", "CAHIER DES CHARGES"), None);
                    if entry(ui, &brief, name(&brief)) {
                        action = Some(PanelAction::Reveal(brief.clone()));
                    }
                }
                if heading(ui, tr("DESIGNS", "MODÈLE"), Some(tr("A new part, in models/", "Une nouvelle pièce, dans models/"))) {
                    action = Some(PanelAction::NewDesign);
                }
                for (i, design) in project.design_paths().iter().enumerate() {
                    let label = if i == 0 { format!("{} ★", name(design)) } else { name(design) };
                    if entry(ui, design, label) {
                        action = Some(PanelAction::Open(design.clone()));
                    }
                }
                if heading(ui, tr("SCANS", "SCANS"), Some(tr("Import a scan (STL, OBJ, 3MF)", "Importer un scan (STL, OBJ, 3MF)"))) {
                    action = Some(PanelAction::ImportScan);
                }
                for scan in project.scan_paths() {
                    let (clicked, response) = row(ui, &scan, name(&scan));
                    if clicked {
                        action = Some(PanelAction::Open(scan.clone()));
                    }
                    if scan.exists() {
                        response.context_menu(|ui| {
                            if ui.button(tr("Set Straight…", "Remettre d'aplomb…")).clicked() {
                                action = Some(PanelAction::LevelScan(scan.clone()));
                                ui.close();
                            }
                        });
                    }
                }
                let new_circuit = project.circuit.is_none().then_some(tr("Write the circuit", "Créer le circuit"));
                if heading(ui, tr("CIRCUIT", "CIRCUIT"), new_circuit) {
                    action = Some(PanelAction::NewCircuit);
                }
                if let Some(circuit) = project.circuit_path() {
                    if entry(ui, &circuit, name(&circuit)) {
                        action = Some(PanelAction::Open(circuit.clone()));
                    }
                }
                heading(ui, tr("FIRMWARE", "FIRMWARE"), None);
                match project.circuit_path().filter(|_| project.has_firmware()) {
                    Some(circuit) => {
                        let label = format!("{}/", project.relative(&project.firmware_dir()).unwrap_or_else(|| "firmware".into()));
                        if entry(ui, &project.firmware_dir(), label) {
                            action = Some(PanelAction::OpenFirmware(circuit));
                        }
                    }
                    None => {
                        let hint = if project.circuit.is_some() {
                            tr("Not written yet: File › Generate the Firmware", "Pas encore écrit : Fichier › Générer le firmware")
                        } else {
                            tr("The circuit's, once it has one", "Celui du circuit, une fois qu'il existe")
                        };
                        ui.label(RichText::new(hint).size(11.5).color(palette.text_faint));
                    }
                }
                heading(ui, tr("EXPORTS", "EXPORTS"), None);
                if open.exports.is_empty() {
                    ui.label(RichText::new(tr("What you export goes here", "Ce que vous exportez arrive ici")).size(11.5).color(palette.text_faint));
                }
                for export in &open.exports {
                    if entry(ui, export, name(export)) {
                        action = Some(PanelAction::Reveal(export.clone()));
                    }
                }
            });
        });
        if let Some(action) = action {
            self.panel_action(ctx, action);
        }
    }

    fn panel_action(&mut self, ctx: &egui::Context, action: PanelAction) {
        match action {
            PanelAction::Open(path) => self.open(ctx, path),
            PanelAction::OpenFile(path) => self.open_from_folder(ctx, path),
            PanelAction::View(view) => {
                self.left_view = view;
                self.folder_tree = None;
            }
            PanelAction::OpenFirmware(circuit) => {
                self.open(ctx, circuit);
                self.panels.circuit_tab = super::firmware_tab::CircuitTab::Firmware;
                self.show_code(ctx);
            }
            PanelAction::Reveal(path) => {
                let _ = reveal(&path);
            }
            PanelAction::Close if self.project.is_some() => self.switch_project(ctx, None),
            PanelAction::Close => self.folder_shown = false,
            PanelAction::NewDesign => self.add_to_project(ctx, Kind::Design),
            PanelAction::NewCircuit => self.add_to_project(ctx, Kind::Circuit),
            PanelAction::ImportScan => self.import_scan(ctx),
            PanelAction::LevelScan(scan) => self.level_asked = Some(scan),
        }
    }

    /// A file clicked in the Files view: a design, a circuit or a mesh in
    /// its tab, a firmware's file in its circuit's Firmware tab, another in
    /// the Finder.
    fn open_from_folder(&mut self, ctx: &egui::Context, path: PathBuf) {
        let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if ["c2f", "scad", "c2fc", "stl", "obj", "3mf", "ply", "off", "amf"].contains(&extension.as_str()) {
            self.open(ctx, path);
            return;
        }
        let firmware_file = ["cpp", "c", "h", "hpp", "ino", "ini"].contains(&extension.as_str());
        if let Some(circuit) = firmware_file.then(|| self.circuit_of_firmware(&path)).flatten() {
            self.open(ctx, circuit);
            self.panels.circuit_tab = super::firmware_tab::CircuitTab::Firmware;
            self.show_code(ctx);
            if let Some(project) = self.firmware_project() {
                if let Some(index) = project.index_of(&path) {
                    project.current = index;
                }
            }
            return;
        }
        let _ = reveal(&path);
    }

    /// The circuit whose firmware `file` is part of: the project's, or the
    /// one beside the firmware's folder.
    fn circuit_of_firmware(&self, file: &Path) -> Option<PathBuf> {
        if let Some(open) = &self.project {
            let circuit = open.project.circuit_path().filter(|_| file.starts_with(open.project.firmware_dir()));
            if circuit.is_some() {
                return circuit;
            }
        }
        // `…/firmware/src/main.cpp`: a circuit beside `…/firmware`.
        file.ancestors().skip(1).find(|dir| dir.join("platformio.ini").is_file()).and_then(|firmware| {
            let beside = firmware.parent()?;
            let mut circuits: Vec<PathBuf> = std::fs::read_dir(beside)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "c2fc"))
                .collect();
            circuits.sort();
            circuits.into_iter().next()
        })
    }

    /// A new design or circuit in the project, named to be free, opened.
    fn add_to_project(&mut self, ctx: &egui::Context, kind: Kind) {
        let Some(open) = self.project.as_mut() else { return };
        let project = match reread(&open.project.file) {
            Ok(project) => {
                open.project = project;
                &mut open.project
            }
            Err(error) => {
                self.doc.console.log(Level::Error, tr!("The project's file does not read: {error}", "Le fichier du projet ne se lit pas : {error}"));
                return;
            }
        };
        let folder = project.dir.join(kind.folder());
        let (stem, extension, text) = match kind {
            Kind::Circuit => {
                // Its enclosure, the project's main design, when it has one.
                let enclosure = project.main_design().map(|d| relative_to(&d, &folder));
                let line = enclosure.map(|e| format!("enclosure = \"{e}\"\n")).unwrap_or_default();
                (c2f_project::slug(&project.name), "c2fc", format!("version = 1\n{line}\n# [parts.board]\n# part = \"electronics/xiao_esp32s3.c2f\"\n"))
            }
            _ => ("part".to_string(), "c2f", "// A part of the project.\ncube([20, 20, 10]);\n".to_string()),
        };
        let mut path = folder.join(format!("{stem}.{extension}"));
        let mut n = 2;
        while path.exists() {
            path = folder.join(format!("{stem}_{n}.{extension}"));
            n += 1;
        }
        open.seen.insert(path.clone());
        let written = std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(&path, text))
            .map_err(|e| e.to_string())
            .and_then(|()| project.add(kind, &path))
            .and_then(|()| project.save());
        match written {
            Ok(()) => self.open(ctx, path),
            Err(error) => self.doc.console.log(Level::Error, error),
        }
    }

    /// A scan picked, copied into the project's scans/ and opened.
    fn import_scan(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new().set_title(tr("Import a scan", "Importer un scan")).add_filter("Mesh", &["stl", "obj", "3mf", "ply"]);
        let Some(file) = dialog.pick_file() else { return };
        let Some(open) = self.project.as_mut() else { return };
        let project = match reread(&open.project.file) {
            Ok(project) => {
                open.project = project;
                &mut open.project
            }
            Err(error) => {
                self.doc.console.log(Level::Error, tr!("The project's file does not read: {error}", "Le fichier du projet ne se lit pas : {error}"));
                return;
            }
        };
        let dir = project.dir.join(Kind::Scan.folder());
        let already = std::fs::canonicalize(&file).ok().filter(|f| f.starts_with(std::fs::canonicalize(&dir).unwrap_or_default()));
        // A scan of the same name already there stays: the new one takes a
        // free name.
        let target = already.clone().unwrap_or_else(|| free(&dir, &file));
        open.seen.insert(target.clone());
        let copied = std::fs::create_dir_all(&dir)
            .and_then(|()| if already.is_some() { Ok(0) } else { std::fs::copy(&file, &target) })
            .map_err(|e| e.to_string())
            .and_then(|_| project.add(Kind::Scan, &target))
            .and_then(|()| project.save());
        match copied {
            Ok(()) => self.open(ctx, target),
            Err(error) => self.doc.console.log(Level::Error, error),
        }
    }
}

/// The Files view's nodes; a file clicked, as an action.
fn show_nodes(ui: &mut egui::Ui, nodes: &[Node], depth: usize, current: Option<&Path>, action: &mut Option<PanelAction>) {
    let palette = theme::palette();
    for node in nodes {
        match &node.children {
            Some(children) => {
                // The object's own folders open at first.
                let open = depth == 0 && ["models", "circuit", "scans", "firmware", "src"].contains(&node.name.as_str());
                egui::CollapsingHeader::new(RichText::new(&node.name).size(12.5).color(palette.text))
                    .id_salt(&node.path)
                    .default_open(open)
                    .show(ui, |ui| show_nodes(ui, children, depth + 1, current, action));
            }
            None => {
                let on = current == Some(node.path.as_path());
                let kind = Kind::of(&node.path).is_some() || node.name.ends_with(".c2fproj");
                let text = RichText::new(&node.name).size(12.5);
                let text = if on { text.strong().color(palette.accent_strong) } else if kind { text.color(palette.text) } else { text.color(palette.text_dim) };
                let response = ui.add(egui::Label::new(text).truncate().sense(Sense::click())).on_hover_text(node.path.display().to_string());
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    *action = Some(PanelAction::OpenFile(node.path.clone()));
                }
            }
        }
    }
}

/// The name of `file` in `dir`, or `name_2`, `name_3`… when it is taken.
fn free(dir: &Path, file: &Path) -> PathBuf {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = file.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut path = dir.join(format!("{stem}{extension}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{n}{extension}"));
        n += 1;
    }
    path
}

/// `path` relative to the folder `from`, with `/`: `../models/plate.c2f`.
fn relative_to(path: &Path, from: &Path) -> String {
    let path: Vec<_> = path.components().collect();
    let from: Vec<_> = from.components().collect();
    let common = path.iter().zip(&from).take_while(|(a, b)| a == b).count();
    let ups = std::iter::repeat_n("..".to_string(), from.len() - common);
    let downs = path[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned());
    ups.chain(downs).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_view_lists_folders_first_without_builds() {
        let dir = std::env::temp_dir().join(format!("c2f-studio-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("firmware/.pio/build")).unwrap();
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::write(dir.join("models/a.c2f"), "").unwrap();
        std::fs::write(dir.join("brief.md"), "").unwrap();
        std::fs::write(dir.join(".DS_Store"), "").unwrap();
        let tree = FileTree::of(&dir);
        let names: Vec<&str> = tree.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["firmware", "models", "brief.md"]);
        assert!(tree.nodes[0].children.as_ref().unwrap().is_empty(), "the build left out");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_is_written_from_another_folder() {
        assert_eq!(relative_to(Path::new("/p/models/plate.c2f"), Path::new("/p/circuit")), "../models/plate.c2f");
        assert_eq!(relative_to(Path::new("/p/plate.c2f"), Path::new("/p")), "plate.c2f");
    }

    #[test]
    fn files_put_in_its_folders_join_the_project() {
        let dir = std::env::temp_dir().join(format!("c2f-studio-project-{}", std::process::id())).join("Lamp");
        let _ = std::fs::remove_dir_all(&dir);
        let project = c2f_project::create(&dir, "Lamp").unwrap();
        let mut open = OpenProject::new(project);
        std::fs::write(dir.join("scans/scan-2026-11-04.stl"), "").unwrap();
        std::fs::write(dir.join("models/shade.c2f"), "sphere(5);").unwrap();
        std::fs::write(dir.join("exports/shade.stl"), "").unwrap();
        assert!(open.refresh().unwrap());
        assert_eq!(open.project.scans, ["scans/scan-2026-11-04.stl"]);
        assert_eq!(open.project.designs, ["models/shade.c2f"]);
        assert_eq!(open.exports, [std::fs::canonicalize(dir.join("exports/shade.stl")).unwrap()]);
        // Written into its file too, and nothing new the next time.
        assert_eq!(Project::open(&dir).unwrap().scans, ["scans/scan-2026-11-04.stl"]);
        assert!(!open.refresh().unwrap());
        // Taken out of the project by hand, it stays out; a hand edit made
        // since is kept.
        let mut edited = Project::open(&dir).unwrap();
        edited.designs.clear();
        edited.name = "Lamp, renamed".into();
        edited.save().unwrap();
        assert!(open.refresh().unwrap());
        assert!(open.project.designs.is_empty());
        assert_eq!(open.project.name, "Lamp, renamed");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
