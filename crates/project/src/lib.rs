//! A project: the folder of an object, and its `.c2fproj` file, which says
//! which of its files is what: the brief, the designs (the first is the
//! main one), the circuit, the firmware's project, the scans of the part it
//! starts from, and where the exports go.
//!
//! ```toml
//! version = 1
//! name = "Grogu"
//! brief = "brief.md"
//! designs = ["models/grogu_plate.c2f"]
//! circuit = "circuit/grogu.c2fc"
//! firmware = "firmware"
//! scans = ["scans/scan-2026-11-03.stl"]
//! exports = "exports"
//! ```
//!
//! The paths are relative to the project's folder and stay in it. Every
//! field but `version` and `name` may be left out: the exports then go in
//! `exports/`, and the firmware is `firmware/` beside the circuit, as for a
//! circuit outside any project. A folder becomes a project once it has
//! its file ([`adopt`]), its files left where they are.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use toml::Spanned;

/// The extension of a project's file.
pub const EXTENSION: &str = "c2fproj";

/// The version of the format this crate writes.
pub const VERSION: i64 = 1;

/// The folders a new project starts with.
pub const FOLDERS: [&str; 5] = ["models", "scans", "circuit", "firmware", "exports"];

/// A problem in a project's file, at its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// From 1; 0 when it is about no line.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 { write!(f, "line {}: {}", self.line, self.message) } else { f.write_str(&self.message) }
    }
}

/// What a file is in a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Brief,
    Design,
    Circuit,
    Firmware,
    Scan,
}

impl Kind {
    /// From its name: `brief`, `design`, `circuit`, `firmware`, `scan`.
    pub fn from_name(name: &str) -> Option<Kind> {
        Some(match name {
            "brief" => Kind::Brief,
            "design" => Kind::Design,
            "circuit" => Kind::Circuit,
            "firmware" => Kind::Firmware,
            "scan" => Kind::Scan,
            _ => return None,
        })
    }

    /// What a file is, from its name: a design, a circuit, a scan or a
    /// brief; `None` for another.
    pub fn of(path: &Path) -> Option<Kind> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match extension.as_str() {
            "c2f" | "scad" => Kind::Design,
            "c2fc" => Kind::Circuit,
            "stl" | "obj" | "3mf" | "ply" => Kind::Scan,
            "md" => Kind::Brief,
            _ => return None,
        })
    }

    /// The folder a new file of the kind goes in.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Brief => "",
            Kind::Design => "models",
            Kind::Circuit => "circuit",
            Kind::Firmware => "firmware",
            Kind::Scan => "scans",
        }
    }
}

/// A project, as its file says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// Its `.c2fproj` file.
    pub file: PathBuf,
    /// Its folder: the file's.
    pub dir: PathBuf,
    pub name: String,
    /// The paths as written, relative to the folder.
    pub brief: Option<String>,
    pub designs: Vec<String>,
    pub circuit: Option<String>,
    pub firmware: Option<String>,
    pub scans: Vec<String>,
    pub exports: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    version: Option<Spanned<i64>>,
    name: Option<Spanned<String>>,
    brief: Option<Spanned<String>>,
    #[serde(default)]
    designs: Vec<Spanned<String>>,
    circuit: Option<Spanned<String>>,
    firmware: Option<Spanned<String>>,
    #[serde(default)]
    scans: Vec<Spanned<String>>,
    exports: Option<Spanned<String>>,
}

impl Project {
    /// Reads the project `path` names: its `.c2fproj`, or a folder that
    /// holds one.
    pub fn open(path: &Path) -> Result<Project, Vec<Problem>> {
        let file = if path.is_dir() {
            file_in(path).ok_or_else(|| vec![problem(0, format!("{} has no .{EXTENSION} file", path.display()))])?
        } else {
            path.to_path_buf()
        };
        // One path for one project, however it was named.
        let file = absolute(&file);
        let src = std::fs::read_to_string(&file).map_err(|e| vec![problem(0, format!("cannot read {}: {e}", file.display()))])?;
        Project::parse(&src, &file)
    }

    /// Reads a project's file from its text `src`, as if at `file`.
    pub fn parse(src: &str, file: &Path) -> Result<Project, Vec<Problem>> {
        let line = |at: usize| src[..at.min(src.len())].matches('\n').count() + 1;
        let entry: FileEntry = toml::from_str(src).map_err(|err| {
            let at = err.span().map_or(0, |s| line(s.start));
            vec![problem(at, err.message().to_string())]
        })?;
        let mut problems = Vec::new();
        match &entry.version {
            Some(v) if *v.get_ref() == VERSION => {}
            Some(v) => problems.push(problem(line(v.span().start), format!("version {} is not one this version of Code2Form reads ({VERSION})", v.get_ref()))),
            None => problems.push(problem(0, format!("`version = {VERSION}` is missing"))),
        }
        let name = match &entry.name {
            Some(n) if !n.get_ref().trim().is_empty() => n.get_ref().trim().to_string(),
            Some(n) => {
                problems.push(problem(line(n.span().start), "the project's `name` is empty".into()));
                String::new()
            }
            None => {
                problems.push(problem(0, "`name` is missing".into()));
                String::new()
            }
        };
        let mut path = |value: &Spanned<String>, what: &str| -> String {
            if !inside(Path::new(value.get_ref())) {
                problems.push(problem(
                    line(value.span().start),
                    format!("{what} `{}` leaves the project's folder: give a path inside it", value.get_ref()),
                ));
            }
            value.get_ref().clone()
        };
        let brief = entry.brief.as_ref().map(|v| path(v, "brief"));
        let designs = entry.designs.iter().map(|v| path(v, "design")).collect();
        let circuit = entry.circuit.as_ref().map(|v| path(v, "circuit"));
        let firmware = entry.firmware.as_ref().map(|v| path(v, "firmware"));
        let scans = entry.scans.iter().map(|v| path(v, "scan")).collect();
        let exports = entry.exports.as_ref().map(|v| path(v, "exports"));
        if !problems.is_empty() {
            return Err(problems);
        }
        let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
        Ok(Project { file: file.to_path_buf(), dir, name, brief, designs, circuit, firmware, scans, exports })
    }

    /// The project `file` belongs to: the nearest folder above it, up to
    /// four levels, with a `.c2fproj`. A project file that does not read
    /// stops the search: the file is in no project rather than in one
    /// further up.
    pub fn of(file: &Path) -> Option<Project> {
        let file = absolute(file);
        let manifest = file.ancestors().skip(1).take(4).find_map(file_in)?;
        Project::open(&manifest).ok()
    }

    pub fn brief_path(&self) -> Option<PathBuf> {
        self.brief.as_ref().map(|p| self.dir.join(p))
    }

    pub fn design_paths(&self) -> Vec<PathBuf> {
        self.designs.iter().map(|p| self.dir.join(p)).collect()
    }

    /// The design the project is about: its first.
    pub fn main_design(&self) -> Option<PathBuf> {
        self.designs.first().map(|p| self.dir.join(p))
    }

    pub fn circuit_path(&self) -> Option<PathBuf> {
        self.circuit.as_ref().map(|p| self.dir.join(p))
    }

    pub fn scan_paths(&self) -> Vec<PathBuf> {
        self.scans.iter().map(|p| self.dir.join(p)).collect()
    }

    /// The firmware's PlatformIO project: as the file says, else
    /// `firmware/` beside the circuit, else in the project's folder.
    pub fn firmware_dir(&self) -> PathBuf {
        match (&self.firmware, self.circuit_path()) {
            (Some(dir), _) => self.dir.join(dir),
            (None, Some(circuit)) => circuit.parent().unwrap_or(&self.dir).join("firmware"),
            (None, None) => self.dir.join("firmware"),
        }
    }

    /// Where the exports go: `exports/` unless the file says otherwise.
    pub fn exports_dir(&self) -> PathBuf {
        self.dir.join(self.exports.as_deref().unwrap_or("exports"))
    }

    /// The files the project names that are not there. A firmware not
    /// written yet is not missing: its folder waits for it.
    pub fn missing(&self) -> Vec<String> {
        let files = self.brief.iter().chain(&self.designs).chain(&self.circuit).chain(&self.scans);
        files.filter(|p| !self.dir.join(p).is_file()).cloned().collect()
    }

    /// Whether the firmware is written: its folder has a platformio.ini.
    pub fn has_firmware(&self) -> bool {
        self.firmware_dir().join("platformio.ini").is_file()
    }

    /// `path`, relative to the project's folder, with `/`; `None` when it
    /// is not in it.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let relative = absolute(path).strip_prefix(absolute(&self.dir)).ok()?.to_path_buf();
        let parts: Vec<String> = relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        (!parts.is_empty()).then(|| parts.join("/"))
    }

    /// Names the file at `path`, in the project's folder, as a `kind` of
    /// the project: a design or a scan is added to its list, the brief,
    /// the circuit or the firmware replaces the one there was. Does not
    /// write the file; [`Project::save`] does.
    pub fn add(&mut self, kind: Kind, path: &Path) -> Result<(), String> {
        let relative = self
            .relative(path)
            .filter(|r| inside(Path::new(r)))
            .ok_or_else(|| format!("{} is not in the project's folder, {}", path.display(), self.dir.display()))?;
        if kind == Kind::Firmware && !path.is_dir() {
            return Err(format!("the firmware is a folder, its PlatformIO project: {} is not one", path.display()));
        }
        match kind {
            Kind::Brief => self.brief = Some(relative),
            Kind::Design if !self.designs.contains(&relative) => self.designs.push(relative),
            Kind::Scan if !self.scans.contains(&relative) => self.scans.push(relative),
            Kind::Design | Kind::Scan => {}
            Kind::Circuit => self.circuit = Some(relative),
            Kind::Firmware => self.firmware = Some(relative),
        }
        Ok(())
    }

    /// Takes out of the project the files that are no longer there: a
    /// design or a scan deleted.
    pub fn forget_missing(&mut self) {
        let dir = self.dir.clone();
        self.designs.retain(|p| dir.join(p).is_file());
        self.scans.retain(|p| dir.join(p).is_file());
    }

    /// The project's file, as text.
    pub fn to_toml(&self) -> String {
        let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
        let list = |items: &[String]| {
            if items.is_empty() {
                "[]".to_string()
            } else {
                let mut out = String::from("[\n");
                for item in items {
                    let _ = writeln!(out, "  {},", quote(item));
                }
                out + "]"
            }
        };
        let mut out = String::from("# A Code2Form project: its paths are relative to this file's folder.\n");
        let _ = writeln!(out, "version = {VERSION}");
        let _ = writeln!(out, "name = {}", quote(&self.name));
        if let Some(brief) = &self.brief {
            let _ = writeln!(out, "brief = {}", quote(brief));
        }
        let _ = writeln!(out, "# The first design is the main one.");
        let _ = writeln!(out, "designs = {}", list(&self.designs));
        if let Some(circuit) = &self.circuit {
            let _ = writeln!(out, "circuit = {}", quote(circuit));
        }
        if let Some(firmware) = &self.firmware {
            let _ = writeln!(out, "firmware = {}", quote(firmware));
        }
        let _ = writeln!(out, "scans = {}", list(&self.scans));
        if let Some(exports) = &self.exports {
            let _ = writeln!(out, "exports = {}", quote(exports));
        }
        out
    }

    /// Writes the project's file.
    /// Writes the project's file, whole or not at all: a reader (the
    /// studio, the iPhone through iCloud) never sees it half written.
    pub fn save(&self) -> Result<(), String> {
        let failed = |e: std::io::Error| format!("cannot write {}: {e}", self.file.display());
        let temporary = self.file.with_extension(format!("{EXTENSION}.saving"));
        std::fs::write(&temporary, self.to_toml()).map_err(failed)?;
        std::fs::rename(&temporary, &self.file).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            failed(e)
        })
    }
}

/// Creates a project named `name` in the new folder `dir`: its file
/// (`<name>.c2fproj`), a brief to fill in and its folders. Refuses a
/// folder that already holds a project.
pub fn create(dir: &Path, name: &str) -> Result<Project, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a project needs a name".into());
    }
    if file_in(dir).is_some() {
        return Err(format!("{} is already a project", dir.display()));
    }
    for folder in FOLDERS {
        std::fs::create_dir_all(dir.join(folder)).map_err(|e| format!("cannot create {}: {e}", dir.join(folder).display()))?;
    }
    let brief = dir.join("brief.md");
    if !brief.exists() {
        let text = format!("# {name}\n\n## Requirements\n\n");
        std::fs::write(&brief, text).map_err(|e| format!("cannot write {}: {e}", brief.display()))?;
    }
    let project = Project {
        file: dir.join(format!("{}.{EXTENSION}", slug(name))),
        dir: dir.to_path_buf(),
        name: name.to_string(),
        brief: Some("brief.md".into()),
        designs: Vec::new(),
        circuit: None,
        firmware: Some("firmware".into()),
        scans: Vec::new(),
        exports: None,
    };
    project.save()?;
    Ok(project)
}

/// Makes the existing folder `dir` a project, its files where they are:
/// its designs (the largest first, the circuit's own parts left out), its
/// circuit, its firmware, its scans and its brief, found in it and its
/// folders.
/// The project is named `name`, or after the folder.
pub fn adopt(dir: &Path, name: Option<&str>) -> Result<Project, String> {
    if let Some(file) = file_in(dir) {
        return Err(format!("{} is already a project: {}", dir.display(), file.display()));
    }
    if broad(dir) {
        return Err(format!("{} holds more than one object: make a project of the object's own folder", dir.display()));
    }
    if name.is_some_and(|n| n.trim().is_empty()) {
        return Err("a project needs a name".into());
    }
    let mut files = Vec::new();
    walk(dir, dir, 0, &mut files);
    files.sort();
    let name = name.map(|n| n.trim().to_string()).unwrap_or_else(|| {
        let folder = dir.file_name().map_or_else(|| "Project".into(), |n| n.to_string_lossy().into_owned());
        let mut chars = folder.chars();
        chars.next().map_or(folder.clone(), |c| c.to_uppercase().chain(chars).collect())
    });
    let mut project = Project {
        file: dir.join(format!("{}.{EXTENSION}", slug(&name))),
        dir: dir.to_path_buf(),
        name,
        brief: None,
        designs: Vec::new(),
        circuit: None,
        firmware: None,
        scans: Vec::new(),
        exports: None,
    };
    let relative = |p: &Path| p.strip_prefix(dir).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    for file in &files {
        let rel = relative(file);
        match Kind::of(file) {
            Some(Kind::Design) => project.designs.push(rel),
            Some(Kind::Circuit) if project.circuit.is_none() => project.circuit = Some(rel),
            Some(Kind::Scan) if rel.starts_with("scans/") || file.file_name().is_some_and(|n| n.to_string_lossy().starts_with("scan-")) => {
                project.scans.push(rel)
            }
            Some(Kind::Brief) if project.brief.is_none() && file.file_name().is_some_and(|n| n.eq_ignore_ascii_case("brief.md")) => {
                project.brief = Some(rel)
            }
            _ => {}
        }
    }
    // A part the circuit places (a board, a driver) is the circuit's, not
    // a design of the object; the main design is the largest.
    let parts = project.circuit_path().map(|c| parts_of(&c)).unwrap_or_default();
    project.designs.retain(|d| !parts.iter().any(|p| absolute(p) == absolute(&dir.join(d))));
    project.designs.sort_by_key(|d| (std::cmp::Reverse(std::fs::metadata(dir.join(d)).map_or(0, |m| m.len())), d.clone()));
    let firmware = project.firmware_dir();
    if firmware.join("platformio.ini").is_file() {
        project.firmware = project.relative(&firmware);
    }
    if dir.join("exports").is_dir() {
        project.exports = Some("exports".into());
    }
    project.save()?;
    Ok(project)
}

/// Whether `dir` is a folder of the user's that holds everything (home,
/// Documents, Desktop, Downloads, iCloud Drive, a disk's root), not one
/// object's.
fn broad(dir: &Path) -> bool {
    let dir = absolute(dir);
    if dir.parent().is_none_or(|p| p.parent().is_none()) {
        return true;
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| absolute(Path::new(&h)));
    home.is_some_and(|home| {
        dir == home
            || ["Documents", "Desktop", "Downloads", "Library/Mobile Documents/com~apple~CloudDocs", "Documents/Code2Form"]
                .iter()
                .any(|d| dir == home.join(d))
    })
}

/// The files a circuit places as its parts, from its `part =` lines,
/// beside it.
fn parts_of(circuit: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(circuit) else { return Vec::new() };
    let dir = circuit.parent().unwrap_or(Path::new(""));
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("part")?.trim_start().strip_prefix('='))
        .filter_map(|v| v.trim().strip_prefix('"')?.split('"').next().map(|p| dir.join(p)))
        .collect()
}

/// The files under `dir`, down to three folders, the firmware's build and
/// the exports left out.
fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            // A firmware's sources, a build's output, what the studio
            // exported: not the object's files.
            let skipped = ["exports", "node_modules", "target", "build"].contains(&name.as_str())
                || (["lib", "src", "include"].contains(&name.as_str()) && dir != root);
            if depth < 3 && !skipped && out.len() < 2000 {
                walk(root, &path, depth + 1, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// The folder of the firmware of the circuit at `circuit`: the one its
/// project says when it is the project's circuit, else `firmware/` beside
/// it.
pub fn firmware_dir_of(circuit: &Path) -> PathBuf {
    let same = |a: &Path| absolute(a) == absolute(circuit);
    match Project::of(circuit).filter(|p| p.circuit_path().is_some_and(|c| same(&c))) {
        Some(project) => project.firmware_dir(),
        None => circuit.parent().unwrap_or(Path::new("")).join("firmware"),
    }
}

/// The `.c2fproj` file in `dir`, if it has one (the first, by name).
pub fn file_in(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION) && p.is_file())
        .collect();
    // `grogu 2.c2fproj`, a copy iCloud made of a conflict, after the
    // original.
    let copy = |p: &PathBuf| p.file_stem().is_some_and(|s| s.to_string_lossy().rsplit_once(' ').is_some_and(|(_, n)| n.parse::<u32>().is_ok()));
    files.sort_by_key(|p| (copy(p), p.clone()));
    files.into_iter().next()
}

/// A name as a file name: `Grogu 2` is `grogu_2`.
pub fn slug(name: &str) -> String {
    let slug: String = name.trim().chars().map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { '_' }).collect();
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() { "project".into() } else { slug }
}

/// Whether a relative `path` stays in the folder it is relative to.
fn inside(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) }
    })
}

fn problem(line: usize, message: String) -> Problem {
    Problem { line, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("c2f-project-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_project_file_names_its_files_relative_to_its_folder() {
        let src = "version = 1\nname = \"Grogu\"\nbrief = \"brief.md\"\ndesigns = [\"models/plate.c2f\"]\n\
                   circuit = \"circuit/grogu.c2fc\"\nscans = [\"scans/a.stl\"]\n";
        let project = Project::parse(src, Path::new("/p/grogu.c2fproj")).unwrap();
        assert_eq!(project.name, "Grogu");
        assert_eq!(project.main_design(), Some(PathBuf::from("/p/models/plate.c2f")));
        // No firmware said: beside the circuit, as outside a project.
        assert_eq!(project.firmware_dir(), PathBuf::from("/p/circuit/firmware"));
        assert_eq!(project.exports_dir(), PathBuf::from("/p/exports"));
        // Written again, it reads the same.
        assert_eq!(Project::parse(&project.to_toml(), &project.file).unwrap(), project);
    }

    #[test]
    fn a_path_out_of_the_folder_an_unknown_key_or_no_name_is_said_at_its_line() {
        let err = Project::parse("version = 1\nname = \"A\"\ncircuit = \"../b.c2fc\"\n", Path::new("/p/a.c2fproj")).unwrap_err();
        assert_eq!(err[0].line, 3, "{err:?}");
        assert!(err[0].message.contains("leaves"), "{err:?}");
        let err = Project::parse("version = 1\nname = \"A\"\nscans = [\"/abs.stl\"]\n", Path::new("/p/a.c2fproj")).unwrap_err();
        assert_eq!(err[0].line, 3);
        let err = Project::parse("version = 1\nname = \"A\"\n\nmodel = \"x\"\n", Path::new("/p/a.c2fproj")).unwrap_err();
        assert_eq!(err[0].line, 4, "{err:?}");
        let err = Project::parse("version = 1\n", Path::new("/p/a.c2fproj")).unwrap_err();
        assert!(err[0].message.contains("name"), "{err:?}");
        let err = Project::parse("version = 7\nname = \"A\"\n", Path::new("/p/a.c2fproj")).unwrap_err();
        assert_eq!(err[0].line, 1);
    }

    #[test]
    fn a_new_project_has_its_folders_brief_and_file() {
        let dir = folder("new").join("Lampe");
        let project = create(&dir, "Lampe de chevet").unwrap();
        assert_eq!(project.file, dir.join("lampe_de_chevet.c2fproj"));
        for folder in FOLDERS {
            assert!(dir.join(folder).is_dir(), "{folder}");
        }
        assert!(dir.join("brief.md").is_file());
        let read = Project::open(&dir).unwrap();
        assert_eq!(read.name, "Lampe de chevet");
        assert!(read.missing().is_empty(), "{:?}", read.missing());
        assert!(create(&dir, "Again").is_err(), "already a project");
        // A file in it belongs to it, and joins it.
        std::fs::write(dir.join("models/base.c2f"), "cube(1);").unwrap();
        let mut found = Project::of(&dir.join("models/base.c2f")).unwrap();
        found.add(Kind::Design, &dir.join("models/base.c2f")).unwrap();
        found.add(Kind::Design, &dir.join("models/base.c2f")).unwrap();
        assert_eq!(found.designs, ["models/base.c2f"]);
        assert!(found.add(Kind::Scan, Path::new("/elsewhere/a.stl")).is_err());
        found.save().unwrap();
        assert_eq!(Project::open(&dir).unwrap().designs, ["models/base.c2f"]);
        std::fs::remove_file(dir.join("models/base.c2f")).unwrap();
        found.forget_missing();
        assert!(found.designs.is_empty());
    }

    #[test]
    fn a_folder_is_adopted_as_it_is() {
        let dir = folder("adopt").join("grogu");
        std::fs::create_dir_all(dir.join("firmware/src")).unwrap();
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(dir.join("grogu.c2fc"), "version = 1\nenclosure = \"grogu_plate.c2f\"\n[parts.d]\npart = \"parts/driver.c2f\"\n").unwrap();
        std::fs::write(dir.join("grogu_plate.c2f"), "cube(1); // the object, the largest file").unwrap();
        std::fs::write(dir.join("stand.c2f"), "cube(1);").unwrap();
        std::fs::write(dir.join("parts/driver.c2f"), "cube(1); // the circuit's part, left out of the designs").unwrap();
        std::fs::write(dir.join("firmware/platformio.ini"), "[env:x]\n").unwrap();
        std::fs::write(dir.join("firmware/src/main.cpp"), "").unwrap();
        std::fs::write(dir.join("scan-2026-11-03.stl"), "").unwrap();
        std::fs::write(dir.join("brief.md"), "# Grogu\n").unwrap();
        let project = adopt(&dir, None).unwrap();
        assert_eq!(project.name, "Grogu");
        assert_eq!(project.designs, ["grogu_plate.c2f", "stand.c2f"], "the largest first, the circuit's part left out");
        assert_eq!(project.circuit.as_deref(), Some("grogu.c2fc"));
        assert_eq!(project.firmware.as_deref(), Some("firmware"));
        assert_eq!(project.scans, ["scan-2026-11-03.stl"]);
        assert_eq!(project.brief.as_deref(), Some("brief.md"));
        assert!(Project::open(&dir).unwrap().missing().is_empty());
        assert!(adopt(&dir, None).is_err(), "already a project");
    }

    #[test]
    fn the_nearest_project_file_wins_even_badly_written() {
        let root = folder("nearest");
        create(&root.join("Big"), "Big").unwrap();
        let sub = root.join("Big/sub");
        std::fs::create_dir_all(sub.join("models")).unwrap();
        std::fs::write(sub.join("sub.c2fproj"), "version = 1\nname = ").unwrap();
        std::fs::write(sub.join("sub 2.c2fproj"), "version = 1\nname = \"Copy\"\n").unwrap();
        // The broken one is the nearest: in no project, not in Big.
        std::fs::remove_file(sub.join("sub 2.c2fproj")).unwrap();
        assert_eq!(Project::of(&sub.join("models/x.c2f")), None);
        std::fs::write(sub.join("sub.c2fproj"), "version = 1\nname = \"Sub\"\n").unwrap();
        std::fs::write(sub.join("sub 2.c2fproj"), "version = 1\nname = \"Copy\"\n").unwrap();
        assert_eq!(Project::of(&sub.join("models/x.c2f")).unwrap().name, "Sub", "not iCloud's copy");
        assert_eq!(Project::of(&root.join("Big/models/y.c2f")).unwrap().name, "Big");
    }

    #[test]
    fn a_folder_of_everything_or_a_blank_name_is_refused() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(adopt(&home.join("Documents"), None).unwrap_err().contains("more than one object"));
        assert!(adopt(Path::new("/"), None).is_err());
        let dir = folder("blank");
        assert!(adopt(&dir, Some("   ")).is_err());
        assert!(file_in(&dir).is_none());
    }

    #[test]
    fn names_become_file_names() {
        assert_eq!(slug("Grogu 2"), "grogu_2");
        assert_eq!(slug("Lampe d'été"), "lampe_d_été");
        assert_eq!(slug("  "), "project");
    }
}
