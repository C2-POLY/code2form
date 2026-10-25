//! The parts catalogue: every part under `electronics/`, then under
//! `mechanical/`, in the library folders, the library's and the user's
//! own, with a picture of each, to
//! drag into a circuit or add with a button; and new parts made from a
//! template, in the user's parts folder.
//!
//! Adding a part writes a `[parts.NAME]` table into the circuit's text,
//! after its last part, as one step to undo: the file keeps its comments
//! and layout.
//!
//! The search ranks the parts by what they declare ([`c2f_circuit::search`],
//! which the MCP server's `library_parts` shares), and a part's sheet (its
//! description, specs, sources and pins by type) opens under its row.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::SystemTime;

use c2f_circuit::search::{self, Document};
use c2f_lang::annotations::{Annotations, PinKind};
use eframe::egui::{self, Button, ColorImage, RichText, TextEdit, TextureHandle, TextureOptions, Vec2};
use glam::Vec3;

use crate::i18n::tr;
use crate::pick::Ray;
use crate::theme;

/// The folder of parts in a library folder, where new parts go.
pub const FOLDER: &str = "electronics";

/// The folders of parts in a library folder, in the order the catalogue
/// lists them: the electronics, then the mechanical parts (bearings,
/// screws, gears...) and their helpers.
pub const FOLDERS: [&str; 2] = [FOLDER, "mechanical"];

/// A thumbnail's size, in pixels.
const THUMB: [usize; 2] = [96, 72];

/// A part the catalogue lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// As a circuit names it: `electronics/xiao_esp32s3.c2f`.
    pub rel: String,
    /// The folder it is in, one of [`FOLDERS`].
    pub folder: &'static str,
    pub path: PathBuf,
    /// The file's stem.
    pub name: String,
    /// The first sentence of its `@description`, else of the file's
    /// opening comment.
    pub description: String,
    /// Its pins' names.
    pub pins: Vec<String>,
    pub modified: Option<SystemTime>,
    /// Its sheet and its pins, as the file declares them.
    pub annotations: Annotations,
    /// What the search reads of it.
    pub document: Document,
}

/// The parts under `electronics/` then `mechanical/` in each of `roots`,
/// the first folder winning for a path found in several, by name within
/// each.
pub fn list(roots: &[PathBuf]) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for root in roots {
        for folder in FOLDERS {
            entries.extend(list_folder(root, folder, &mut seen));
        }
    }
    let rank = |entry: &Entry| FOLDERS.iter().position(|f| *f == entry.folder);
    entries.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// The parts in `root`'s `folder` whose path is not in `seen` yet.
fn list_folder(root: &Path, folder: &'static str, seen: &mut HashSet<String>) -> Vec<Entry> {
    let mut entries = Vec::new();
    let Ok(dir) = std::fs::read_dir(root.join(folder)) else { return entries };
    let mut files: Vec<PathBuf> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "c2f"))
        .collect();
    files.sort();
    for path in files {
        let Some(file) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let rel = format!("{folder}/{file}");
        if !seen.insert(rel.clone()) {
            continue;
        }
        let source = c2f_eval::read_source(&path).unwrap_or_default();
        let (annotations, _) = c2f_lang::annotations::read(&source);
        let name = file.trim_end_matches(".c2f").to_string();
        // The search reads the whole description, the row its first
        // sentence.
        let full = match &annotations.description {
            Some(description) => description.text.clone(),
            None => description(&source),
        };
        entries.push(Entry {
            description: first_sentence(&full),
            document: Document::new(&name, &rel, &full, &annotations),
            rel,
            folder,
            name,
            pins: annotations.pins.iter().map(|p| p.name.clone()).collect(),
            modified: std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
            annotations,
            path,
        });
    }
    entries
}

/// The first sentence of the comment a file opens with, at most a line
/// or so: up to its first `. ` or `: `.
pub fn description(source: &str) -> String {
    let block: Vec<&str> = source
        .lines()
        .map(str::trim)
        .take_while(|line| line.starts_with("//"))
        .map(|line| line.trim_start_matches('/').trim())
        .take_while(|line| !line.is_empty() && !line.starts_with('@') && !line.starts_with("http"))
        .collect();
    first_sentence(&block.join(" "))
}

/// The first sentence of `text`, at most a line or so: up to its first
/// `. ` or `: `.
pub fn first_sentence(text: &str) -> String {
    let end = [". ", ": "].iter().filter_map(|stop| text.find(stop)).min().unwrap_or(text.len());
    let sentence = text[..end].trim_end_matches('.');
    const MAX: usize = 110;
    if sentence.chars().count() > MAX {
        let cut: String = sentence.chars().take(MAX).collect();
        format!("{}…", cut.trim_end())
    } else {
        sentence.to_string()
    }
}

/// The entries that answer `query`, best first; all of them, in order,
/// for an empty query.
pub fn search<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let documents: Vec<Document> = entries.iter().map(|e| e.document.clone()).collect();
    search::rank(&documents, query).into_iter().map(|(index, _)| &entries[index]).collect()
}

/// The kinds of pin a sheet sums up, in the order it lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinGroup {
    Power,
    Ground,
    Signal,
    Passive,
    Unconnected,
}

impl PinGroup {
    fn of(kind: PinKind) -> PinGroup {
        match kind {
            PinKind::PowerIn | PinKind::PowerOut => PinGroup::Power,
            PinKind::Ground => PinGroup::Ground,
            PinKind::Io | PinKind::Input | PinKind::Output => PinGroup::Signal,
            PinKind::Passive => PinGroup::Passive,
            PinKind::Nc => PinGroup::Unconnected,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PinGroup::Power => tr("Power", "Alimentation"),
            PinGroup::Ground => tr("Ground", "Masse"),
            PinGroup::Signal => tr("Inputs and outputs", "Entrées et sorties"),
            PinGroup::Passive => tr("Passive", "Passives"),
            PinGroup::Unconnected => tr("Not connected", "Non connectées"),
        }
    }
}

/// A part's pins by group, each with its pins' names in the file's order;
/// the groups without pins left out.
pub fn pin_groups(annotations: &Annotations) -> Vec<(PinGroup, Vec<String>)> {
    let mut groups: Vec<(PinGroup, Vec<String>)> = Vec::new();
    for group in [PinGroup::Power, PinGroup::Ground, PinGroup::Signal, PinGroup::Passive, PinGroup::Unconnected] {
        let names: Vec<String> =
            annotations.pins.iter().filter(|pin| PinGroup::of(pin.kind) == group).map(|pin| pin.name.clone()).collect();
        if !names.is_empty() {
            groups.push((group, names));
        }
    }
    groups
}

/// A part's sheet: its description, specs, sources as links, and pins by
/// group.
fn sheet(ui: &mut egui::Ui, entry: &Entry) {
    let palette = theme::palette();
    let annotations = &entry.annotations;
    if let Some(description) = &annotations.description {
        ui.add(egui::Label::new(RichText::new(&description.text).color(palette.text)).wrap());
    }
    if !annotations.specs.is_empty() {
        egui::Grid::new(("sheet-specs", &entry.rel)).num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
            for spec in &annotations.specs {
                ui.label(RichText::new(&spec.key).size(11.5).color(palette.text_dim));
                ui.add(egui::Label::new(RichText::new(&spec.value).size(11.5).color(palette.text)).wrap());
                ui.end_row();
            }
        });
    }
    for (group, names) in pin_groups(annotations) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} ({})", group.label(), names.len())).size(11.5).color(palette.text_dim));
            ui.label(RichText::new(names.join(", ")).size(11.5).color(palette.text));
        });
    }
    for source in &annotations.sources {
        ui.hyperlink_to(source.label.as_deref().unwrap_or(&source.url), &source.url).on_hover_text(&source.url);
    }
    if annotations.description.is_none() && annotations.sources.is_empty() && annotations.specs.is_empty() {
        ui.label(
            RichText::new(tr(
                "No sheet: the file has no @description, @spec or @source.",
                "Pas de fiche : le fichier n'a ni @description, ni @spec, ni @source.",
            ))
            .size(11.5)
            .color(palette.text_dim),
        );
    }
}

/// The part names a circuit's text gives, from its `[parts.NAME]` headers.
pub fn part_names(src: &str) -> HashSet<String> {
    src.lines()
        .filter_map(|line| {
            line.trim().strip_prefix("[parts.")?.strip_suffix(']').map(|n| n.trim().trim_matches('"').to_string())
        })
        .collect()
}

/// A part name for the file `stem`: what a circuit takes (no spaces, dots,
/// commas or `=`), `_2`, `_3`… when `taken` has it.
pub fn part_name(stem: &str, taken: &HashSet<String>) -> String {
    let base: String =
        stem.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let base = if base.is_empty() { "part".to_string() } else { base };
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}_{n}")).find(|name| !taken.contains(name)).expect("a free name")
}

/// A coordinate as a circuit writes it: to 0.1 mm, without a trailing `.0`.
pub fn coordinate(v: f64) -> String {
    let text = format!("{v:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

/// The circuit's text with a `[parts.NAME]` table for the part file `rel`
/// at `at` on the ground, after the last part's table (or before the first
/// wire, or at the end); and where to put the cursor, as a char index.
pub fn insert_part(src: &str, name: &str, rel: &str, at: [f64; 2]) -> (String, usize) {
    let table = format!("[parts.{name}]\npart = \"{rel}\"\nat = [{}, {}, 0]\n", coordinate(at[0]), coordinate(at[1]));
    insert_table(src, &table)
}

/// The circuit's text with the part's table `table` (its lines, each
/// ending with a line break) where [`insert_part`] puts one; and where to
/// put the cursor, as a char index.
pub fn insert_table(src: &str, table: &str) -> (String, usize) {
    let mut starts = vec![0];
    starts.extend(src.match_indices('\n').map(|(i, _)| i + 1).filter(|&i| i < src.len()));
    let line = |i: usize| {
        let end = starts.get(i + 1).copied().unwrap_or(src.len());
        src[starts[i]..end].trim()
    };
    let headers: Vec<usize> = (0..starts.len()).filter(|&i| line(i).starts_with('[')).collect();
    let last_part = headers.iter().rposition(|&i| line(i).starts_with("[parts."));
    let (at_byte, text) = if let Some(k) = last_part {
        // After the table's last key: comments after it introduce what
        // comes next.
        let next = headers.get(k + 1).copied().unwrap_or(starts.len());
        let last =
            (headers[k]..next).rev().find(|&i| !line(i).is_empty() && !line(i).starts_with('#')).unwrap_or(headers[k]);
        let end = starts.get(last + 1).copied().unwrap_or(src.len());
        let newline = if src[..end].ends_with('\n') { "" } else { "\n" };
        (end, format!("{newline}\n{table}"))
    } else if let Some(&first_wire) = headers.iter().find(|&&i| line(i) == "[[wires]]") {
        (starts[first_wire], format!("{table}\n"))
    } else {
        let mut lead = String::new();
        if !src.is_empty() && !src.ends_with('\n') {
            lead.push('\n');
        }
        if !src.is_empty() {
            lead.push('\n');
        }
        (src.len(), format!("{lead}{table}"))
    };
    let out = format!("{}{text}{}", &src[..at_byte], &src[at_byte..]);
    let cursor = src[..at_byte].chars().count() + text.trim_end().chars().count();
    (out, cursor)
}

/// A free spot on the ground: 5 mm right of everything placed, level with
/// its front; the origin when nothing is.
pub fn free_spot(boxes: &[Option<(Vec3, Vec3)>]) -> [f64; 2] {
    let all = boxes.iter().flatten().copied().reduce(|(l1, h1), (l2, h2)| (l1.min(l2), h1.max(h2)));
    match all {
        Some((lo, hi)) => [f64::from(hi.x) + 5.0, f64::from(lo.y)],
        None => [0.0, 0.0],
    }
}

/// Where the ray meets the ground (z = 0), in front of the eye.
pub fn on_ground(ray: &Ray) -> Option<[f64; 2]> {
    if ray.direction.z.abs() < 1e-6 {
        return None;
    }
    let t = -ray.origin.z / ray.direction.z;
    (t > 0.0).then(|| {
        let p = ray.origin + ray.direction * t;
        [f64::from(p.x), f64::from(p.y)]
    })
}

/// The user's own parts folder, a default library folder: where new parts
/// go.
pub fn user_folder() -> Option<PathBuf> {
    c2f_eval::user_parts_folder()
}

/// A new part's file, in the library's style: a board with two pins and a
/// mount hole, sized in the customizer.
pub fn template(name: &str) -> String {
    format!(
        r#"// {name}: a new part.
//
// A part is a design whose comments declare its pins (`// @pin`), its
// mount holes (`// @mount_hole`) and its ports (`// @port`), in its own
// coordinates: the circuit places it with `at` and `rotate`. Draw the
// board with its corner at the origin and its parts' side up, then put
// each pin where a wire meets it: name, position, direction, type,
// voltage and a "label". Its sheet says what it is (`// @description`),
// its key figures (`// @spec key: value`) and where they come from
// (`// @source`, its product page or datasheet): the catalogue shows it
// and searches it.

// @description {name}: a new part. Say here what it is and does, in a
// @description sentence or two.
// @spec supply: 5 V

/* [Size] */
// Length of the board.
length = 20; // [5:100]
// Width of the board.
width = 15; // [5:100]
// Thickness of the board.
thickness = 1.6; // [0.8:0.1:3]

// @pin VCC [2.54, 2.54, 1.6] up power_in 5V "supply"
// @pin GND [5.08, 2.54, 1.6] ground
// @mount_hole [17, 12, 0] d=2.2

module {name}(length = 20, width = 15, thickness = 1.6) {{
    $fn = 24;
    // The board, less its mount hole.
    color("SeaGreen") difference() {{
        cube([length, width, thickness]);
        translate([17, 12, -1]) cylinder(d = 2.2, h = thickness + 2);
    }}
    // A pad under each pin.
    color("Gold") for (x = [2.54, 5.08]) translate([x, 2.54, thickness]) cylinder(d = 1.6, h = 0.05);
}}

{name}(length, width, thickness);
"#
    )
}

/// Writes a new part `name` from the template into `folder`; refuses to
/// replace a file.
pub fn create(folder: &Path, name: &str) -> Result<PathBuf, String> {
    if !c2f_circuit::part::valid_name(name) {
        return Err(tr(
            "a part's name is letters, digits and `_`, not starting with a digit",
            "le nom d'un composant est fait de lettres, de chiffres et de `_`, sans commencer par un chiffre",
        )
        .to_string());
    }
    std::fs::create_dir_all(folder).map_err(|err| tr!("cannot make {}: {err}", "impossible de créer {} : {err}", folder.display()))?;
    let path = folder.join(format!("{name}.c2f"));
    if path.exists() {
        return Err(tr!("{} already exists", "{} existe déjà", path.display()));
    }
    std::fs::write(&path, template(name)).map_err(|err| tr!("cannot write {}: {err}", "impossible d'écrire {} : {err}", path.display()))?;
    Ok(path)
}

// ---- Thumbnails ----

/// Pictures of the parts, rendered on a thread of their own and kept by
/// file and modification time, in memory and in the user's cache folder.
pub struct Thumbnails {
    requests: Sender<(PathBuf, Option<SystemTime>)>,
    replies: Receiver<(PathBuf, Option<SystemTime>, Option<Vec<u8>>)>,
    textures: HashMap<(PathBuf, Option<SystemTime>), Option<TextureHandle>>,
}

impl Thumbnails {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (requests, request_rx) = mpsc::channel::<(PathBuf, Option<SystemTime>)>();
        let (reply_tx, replies) = mpsc::channel();
        std::thread::Builder::new()
            .name("c2f-thumbnails".into())
            .stack_size(crate::worker::STACK)
            .spawn(move || {
                let libraries = c2f_eval::default_library_paths();
                while let Ok((path, modified)) = request_rx.recv() {
                    let rgb = cached(&path, modified).or_else(|| {
                        let rgb = std::panic::catch_unwind(|| picture(&path, &libraries)).ok().flatten();
                        if let Some(rgb) = &rgb {
                            store(&path, modified, rgb);
                        }
                        rgb
                    });
                    if reply_tx.send((path, modified, rgb)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawning the thumbnail thread");
        Thumbnails { requests, replies, textures: HashMap::new() }
    }

    /// The picture of `entry`, once rendered; asks for it the first time.
    pub fn get(&mut self, ctx: &egui::Context, entry: &Entry) -> Option<TextureHandle> {
        for (path, modified, rgb) in self.replies.try_iter() {
            let texture = rgb.map(|rgb| {
                let image = ColorImage::from_rgb(THUMB, &rgb);
                ctx.load_texture(format!("thumbnail {}", path.display()), image, TextureOptions::LINEAR)
            });
            self.textures.insert((path, modified), texture);
        }
        let key = (entry.path.clone(), entry.modified);
        match self.textures.get(&key) {
            Some(texture) => texture.clone(),
            None => {
                self.textures.insert(key.clone(), None);
                let _ = self.requests.send(key);
                None
            }
        }
    }
}

/// A part's picture, from the diagonal view, as RGB.
fn picture(path: &Path, libraries: &[PathBuf]) -> Option<Vec<u8>> {
    let source = c2f_eval::read_source(path).ok()?;
    let program = c2f_eval::Program::load(&source, path, libraries).ok()?;
    let output = c2f_eval::evaluate(&program);
    let rendered = c2f_geom::render_in(&output.nodes, path.parent().unwrap_or(Path::new("")));
    let &(_, yaw, pitch) = c2f_geom::VIEWS.first()?;
    rendered.rgb(yaw, pitch, THUMB[0], THUMB[1])
}

/// The cache file of a part's picture.
fn cache_file(path: &Path, modified: Option<SystemTime>) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, modified, THUMB).hash(&mut hasher);
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches/Code2Form")
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".cache")))?
            .join("code2form")
    };
    Some(base.join("thumbnails").join(format!("{:016x}.rgb", hasher.finish())))
}

fn cached(path: &Path, modified: Option<SystemTime>) -> Option<Vec<u8>> {
    let bytes = std::fs::read(cache_file(path, modified)?).ok()?;
    (bytes.len() == THUMB[0] * THUMB[1] * 3).then_some(bytes)
}

fn store(path: &Path, modified: Option<SystemTime>, rgb: &[u8]) {
    if let Some(file) = cache_file(path, modified) {
        let _ = file.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(file, rgb);
    }
}

// ---- The panel ----

/// What the panel asks of the app.
pub enum Action {
    /// Add this part at a free spot.
    Add(Box<Entry>),
    /// A part was made: open it.
    Created(PathBuf),
}

/// What a row's drag carries into the view.
#[derive(Clone)]
pub struct Dragged(pub Entry);

pub struct Catalogue {
    entries: Option<Vec<Entry>>,
    query: String,
    thumbnails: Thumbnails,
    /// The "New part…" prompt: the name typed, and the last problem.
    prompt: Option<(String, Option<String>)>,
    /// The library folders set in Settings, searched before the default ones.
    folders: Vec<PathBuf>,
    /// The part whose sheet is open, by its `rel`.
    selected: Option<String>,
}

impl Catalogue {
    pub fn new(ctx: &egui::Context) -> Self {
        Catalogue {
            entries: None,
            query: String::new(),
            thumbnails: Thumbnails::spawn(ctx.clone()),
            prompt: None,
            folders: Vec::new(),
            selected: None,
        }
    }

    /// Searches `folders` too, before the default library folders, as the
    /// renders do.
    pub fn set_libraries(&mut self, folders: Vec<PathBuf>) {
        if folders != self.folders {
            self.folders = folders;
            self.refresh();
        }
    }

    /// Lists the parts again, as a new part or a changed folder asks.
    pub fn refresh(&mut self) {
        self.entries = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let folders = &self.folders;
        let entries = self
            .entries
            .get_or_insert_with(|| {
                let libraries: Vec<PathBuf> = folders.iter().cloned().chain(c2f_eval::default_library_paths()).collect();
                list(&libraries)
            })
            .clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Parts", "Composants")).strong().color(theme::palette().text));
            ui.label(RichText::new(format!("{}", entries.len())).color(theme::palette().text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(tr("New part…", "Nouveau composant…"))
                    .on_hover_text(tr(
                        "Make a part from a template, in your parts folder",
                        "Créer un composant à partir d'un modèle, dans votre dossier de composants",
                    ))
                    .clicked()
                {
                    self.prompt = Some((String::new(), None));
                }
                if ui.small_button("⟳").on_hover_text(tr("List the parts again", "Actualiser la liste des composants")).clicked() {
                    self.refresh();
                }
            });
        });
        ui.add(
            TextEdit::singleline(&mut self.query)
                .hint_text(tr(
                    "Search: name, use, spec or pin, as \"I2S microphone\"",
                    "Rechercher : nom, usage, caractéristique ou broche, comme « micro I2S »",
                ))
                .desired_width(f32::INFINITY),
        );
        ui.label(RichText::new(tr(
            "Drag a part into the view, or press Add",
            "Glissez un composant dans la vue, ou cliquez sur Ajouter",
        )).size(11.5).color(theme::palette().text_dim));
        ui.add_space(4.0);
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for entry in search(&entries, &self.query) {
                let thumbnail = self.thumbnails.get(&ctx, entry);
                ui.horizontal(|ui| {
                    let id = ui.id().with(("catalogue", &entry.rel));
                    ui.dnd_drag_source(id, Dragged(entry.clone()), |ui| match &thumbnail {
                        Some(texture) => {
                            ui.add(egui::Image::new(texture).fit_to_exact_size(Vec2::new(72.0, 54.0)));
                        }
                        None => {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(72.0, 54.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 4.0, theme::palette().line(10));
                        }
                    })
                    .response
                    .on_hover_text(tr!("Drag {} into the view", "Glissez {} dans la vue", entry.rel));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let open = self.selected.as_ref() == Some(&entry.rel);
                            let name = RichText::new(&entry.name).strong().color(theme::palette().text);
                            let response = ui.selectable_label(open, name).on_hover_ui(|ui| {
                                ui.set_max_width(320.0);
                                sheet(ui, entry);
                                ui.label(
                                    RichText::new(tr("Click to keep the sheet open", "Cliquez pour garder la fiche ouverte"))
                                        .size(11.5)
                                        .color(theme::palette().text_dim),
                                );
                            });
                            if response.clicked() {
                                self.selected = if open { None } else { Some(entry.rel.clone()) };
                            }
                            let pins = entry.pins.len();
                            // A mechanical part says so rather than "0 pins".
                            let kind = if entry.folder == FOLDER {
                                tr!("{pins} pin{}", "{pins} broche{}", if pins == 1 { "" } else { "s" })
                            } else {
                                tr("mechanical", "mécanique").to_string()
                            };
                            ui.label(RichText::new(kind).size(11.5).color(theme::palette().text_dim));
                        });
                        if !entry.description.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&entry.description).size(11.5).color(theme::palette().text_dim),
                                )
                                .wrap(),
                            );
                        }
                        if ui
                            .add(Button::new(tr("Add", "Ajouter")).small())
                            .on_hover_text(tr("Place it beside the circuit's parts", "Le placer à côté des composants du circuit"))
                            .clicked()
                        {
                            action = Some(Action::Add(Box::new(entry.clone())));
                        }
                    });
                });
                if self.selected.as_ref() == Some(&entry.rel) {
                    ui.add_space(2.0);
                    sheet(ui, entry);
                }
                ui.separator();
            }
        });
        if let Some(created) = self.prompt_window(&ctx) {
            action = Some(created);
        }
        action
    }

    /// The "New part…" prompt.
    fn prompt_window(&mut self, ctx: &egui::Context) -> Option<Action> {
        let (name, problem) = self.prompt.as_mut()?;
        let mut action = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("new-part")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(tr("New part", "Nouveau composant")).heading().color(theme::palette().text));
            ui.add_space(4.0);
            let folder = user_folder().map(|f| f.display().to_string()).unwrap_or_default();
            ui.label(
                RichText::new(tr!(
                    "A part made from a template, in {folder}",
                    "Un composant créé à partir d'un modèle, dans {folder}",
                )).color(theme::palette().text_dim),
            );
            ui.add_space(6.0);
            let field = ui.add(TextEdit::singleline(name).hint_text(tr("name, as my_sensor", "nom, comme my_sensor")).desired_width(f32::INFINITY));
            field.request_focus();
            if let Some(problem) = problem.as_ref() {
                ui.label(RichText::new(problem.as_str()).color(theme::palette().error));
            }
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(tr("Create", "Créer")).clicked() || enter {
                    match user_folder().ok_or(tr("no home folder", "aucun dossier personnel").to_string()).and_then(|f| create(&f, name.trim())) {
                        Ok(path) => {
                            action = Some(Action::Created(path));
                            close = true;
                        }
                        Err(why) => *problem = Some(why),
                    }
                }
                if ui.button(tr("Cancel", "Annuler")).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.prompt = None;
        }
        if action.is_some() {
            self.refresh();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code2form-catalogue-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(FOLDER)).unwrap();
        dir
    }

    #[test]
    fn lists_parts_once_with_their_description_and_pins() {
        let (mine, library) = (folder("mine"), folder("library"));
        let board = "// Small board: a test.\n// More words.\n// @pin A [0, 0, 0]\n// @pin B [1, 0, 0]\ncube(1);\n";
        std::fs::write(mine.join("electronics/board.c2f"), board).unwrap();
        std::fs::write(library.join("electronics/board.c2f"), "// Another board.\ncube(2);\n").unwrap();
        std::fs::write(library.join("electronics/Amp.c2f"), "// An amplifier, I2S in\ncube(1);\n").unwrap();
        std::fs::write(library.join("electronics/notes.txt"), "not a part").unwrap();
        std::fs::create_dir_all(library.join("mechanical")).unwrap();
        std::fs::write(library.join("mechanical/axle.c2f"), "// A steel axle.\ncylinder(d = 8, h = 50);\n").unwrap();
        let entries = list(&[mine.clone(), library.clone(), mine.join("missing")]);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        // The electronics first, then the mechanical parts.
        assert_eq!(names, ["Amp", "board", "axle"]);
        assert_eq!((entries[2].rel.as_str(), entries[2].folder), ("mechanical/axle.c2f", "mechanical"));
        assert_eq!(entries[0].folder, "electronics");
        // The first folder wins.
        assert_eq!(entries[1].path, mine.join("electronics/board.c2f"));
        assert_eq!(
            (entries[1].rel.as_str(), entries[1].description.as_str()),
            ("electronics/board.c2f", "Small board")
        );
        assert_eq!(entries[1].pins, ["A", "B"]);
        let found = |query: &str| -> Vec<&str> { search(&entries, query).iter().map(|e| e.name.as_str()).collect() };
        assert_eq!(found("BOARD b"), ["board"]);
        assert_eq!(found("ampli i2s"), ["Amp"]);
        assert_eq!(found(""), ["Amp", "board", "axle"]);
        assert_eq!(found("axe"), ["axle"]);
        for dir in [mine, library] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_part_with_a_sheet_shows_it_and_is_searched_by_it() {
        let dir = folder("sheet");
        let driver = "\
// A driver board, drawn.
// @description Dual H-bridge motor driver. Runs two DC motors.
// @spec logic: 3.3 V or 5 V
// @source https://www.ti.com/product/DRV8833 product page
// @pin VCC [0, 0, 0] power_in 10.8V
// @pin GND [1, 0, 0] ground
// @pin IN1 [2, 0, 0] input 5.75V
// @pin OUT1 [3, 0, 0] output 10.8V
// @pin VM [4, 0, 0] power_out 5V
cube(1);
";
        std::fs::write(dir.join("electronics/driver.c2f"), driver).unwrap();
        std::fs::write(dir.join("electronics/motor.c2f"), "// A DC motor.\n// @pin + [0, 0, 0]\ncube(1);\n").unwrap();
        let entries = list(&[dir.clone()]);
        let driver = entries.iter().find(|e| e.name == "driver").unwrap();
        // The row shows the description's first sentence.
        assert_eq!(driver.description, "Dual H-bridge motor driver");
        let groups: Vec<(PinGroup, Vec<String>)> = pin_groups(&driver.annotations);
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            groups,
            [
                (PinGroup::Power, names(&["VCC", "VM"])),
                (PinGroup::Ground, names(&["GND"])),
                (PinGroup::Signal, names(&["IN1", "OUT1"])),
            ]
        );
        let found = |query: &str| -> Vec<&str> { search(&entries, query).iter().map(|e| e.name.as_str()).collect() };
        assert_eq!(found("moteur"), ["motor", "driver"]);
        assert_eq!(found("driver 5V"), ["driver"]);
        assert_eq!(found("pont en H"), ["driver"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn descriptions_are_a_first_sentence() {
        assert_eq!(
            description("// DRV8833 dual H-bridge, the black\n// board: its pins.\n"),
            "DRV8833 dual H-bridge, the black board"
        );
        assert_eq!(description("cube(1); // not this"), "");
        assert_eq!(description("// Only a line.\n\n// later"), "Only a line");
        assert!(description(&format!("// {}", "word ".repeat(40))).ends_with('…'));
    }

    const CIRCUIT: &str = "version = 1\n\n# The board.\n[parts.board]\npart = \"electronics/board.c2f\"\nat = [0, 0, 0]\n\n# Signals.\n\n[[wires]]\nfrom = \"board.A\"\nto = \"board.B\"\n";

    #[test]
    fn a_part_goes_after_the_last_part_with_a_free_name() {
        let taken = part_names(CIRCUIT);
        assert_eq!(taken, HashSet::from(["board".to_string()]));
        let name = part_name("board", &taken);
        assert_eq!(name, "board_2");
        let (text, cursor) = insert_part(CIRCUIT, &name, "electronics/board.c2f", [12.345, -0.04]);
        assert_eq!(
            text,
            CIRCUIT.replace(
                "at = [0, 0, 0]\n",
                "at = [0, 0, 0]\n\n[parts.board_2]\npart = \"electronics/board.c2f\"\nat = [12.3, 0, 0]\n"
            )
        );
        assert!(text[..cursor].ends_with("at = [12.3, 0, 0]"));
        let taken = part_names(&text);
        assert_eq!(part_name("board", &taken), "board_3");
        assert_eq!(part_name("my part.v2", &taken), "my_part_v2");
        // It reads.
        let dir = folder("insert");
        std::fs::write(dir.join("electronics/board.c2f"), "// @pin A [0, 0, 0]\n// @pin B [1, 0, 0]\ncube(1);\n")
            .unwrap();
        let circuit = c2f_circuit::parse(&text, &dir.join("test.c2fc"), &[]).unwrap();
        assert_eq!(circuit.parts[1].at, [12.3, 0.0, 0.0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_parts_a_part_goes_before_the_wires_or_at_the_end() {
        let (text, _) = insert_part("# Wires.\n[[wires]]\nfrom = \"a.A\"\n", "a", "electronics/a.c2f", [1.0, 2.0]);
        assert_eq!(
            text,
            "# Wires.\n[parts.a]\npart = \"electronics/a.c2f\"\nat = [1, 2, 0]\n\n[[wires]]\nfrom = \"a.A\"\n"
        );
        let (text, _) = insert_part("version = 1", "a", "electronics/a.c2f", [0.0, 0.0]);
        assert_eq!(text, "version = 1\n\n[parts.a]\npart = \"electronics/a.c2f\"\nat = [0, 0, 0]\n");
        let (text, _) = insert_part("", "a", "electronics/a.c2f", [0.0, 0.0]);
        assert_eq!(text, "[parts.a]\npart = \"electronics/a.c2f\"\nat = [0, 0, 0]\n");
    }

    #[test]
    fn a_free_spot_is_right_of_everything() {
        assert_eq!(free_spot(&[]), [0.0, 0.0]);
        let boxes = [
            Some((Vec3::new(-10.0, -3.0, 0.0), Vec3::new(20.0, 8.0, 4.0))),
            None,
            Some((Vec3::new(0.0, 5.0, 0.0), Vec3::new(30.5, 9.0, 1.0))),
        ];
        assert_eq!(free_spot(&boxes), [35.5, -3.0]);
    }

    #[test]
    fn a_drop_lands_on_the_ground() {
        let ray =
            Ray { origin: Vec3::new(10.0, 0.0, 20.0), direction: Vec3::new(0.0, 1.0, -1.0).normalize(), slack: 0.0 };
        let [x, y] = on_ground(&ray).unwrap();
        assert!((x - 10.0).abs() < 1e-5 && (y - 20.0).abs() < 1e-4);
        let up = Ray { direction: Vec3::Z, ..ray };
        assert_eq!(on_ground(&up), None);
        let level = Ray { direction: Vec3::X, ..ray };
        assert_eq!(on_ground(&level), None);
    }

    #[test]
    fn the_template_reads_and_renders_cleanly() {
        let dir = folder("template");
        let path = create(&dir.join(FOLDER), "my_sensor").unwrap();
        assert!(create(&dir.join(FOLDER), "my_sensor").is_err(), "a file is never replaced");
        assert!(create(&dir.join(FOLDER), "2bad").is_err());
        let source = std::fs::read_to_string(&path).unwrap();
        let (annotations, errors) = c2f_lang::annotations::read(&source);
        assert!(errors.is_empty(), "{errors:?}");
        let pins: Vec<&str> = annotations.pins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!((pins, annotations.mount_holes.len()), (vec!["VCC", "GND"], 1));
        let program = c2f_eval::Program::load(&source, &path, &[]).unwrap();
        assert_eq!(program.parameters(&source).len(), 3);
        let output = c2f_eval::evaluate(&program);
        assert!(output.error.is_none() && output.warnings.is_empty(), "{:?} {:?}", output.error, output.warnings);
        let rendered = c2f_geom::render_in(&output.nodes, &dir);
        assert!(rendered.warnings.is_empty() && rendered.stats.triangles > 0);
        assert_eq!(description(&source), "my_sensor");
        // The catalogue lists it.
        let entries = list(&[dir.clone()]);
        assert_eq!((entries[0].name.as_str(), entries[0].pins.len()), ("my_sensor", 2));
        let _ = std::fs::remove_dir_all(dir);
    }
}
