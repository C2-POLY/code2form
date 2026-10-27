//! Loading a program: the main file and the libraries it `use`s, each with
//! its `include`d files spliced in token by token, as the lexer of the
//! reference implementation does.

use std::path::{Path, PathBuf};

use c2f_lang::ast::{File, Stmt, StmtKind};
use c2f_lang::{Diagnostic, FileScope, Includes};

pub struct Program {
    /// The main file first, then libraries.
    pub files: Vec<SourceFile>,
    /// Problems found while loading, such as missing files.
    pub warnings: Vec<String>,
    /// The circuit file the main file names in `// @circuit`, read when
    /// the program is loaded: an app re-renders when it changes.
    pub circuit_path: Option<PathBuf>,
    /// That circuit, when it was read without error, whose parts the
    /// enclosure built-ins (`standoffs_for()`, `cutout_for()`...) place.
    pub circuit: Option<c2f_circuit::Circuit>,
    /// Where the circuit's parts and their libraries were looked for.
    pub(crate) library_paths: Vec<PathBuf>,
    /// The folders its files may be read from, when an agent runs it.
    pub(crate) scope: Option<FileScope>,
    /// `$t`, the animation's time, from 0 to 1: 0 unless set, as in the
    /// reference. An assignment to `$t` in the program wins.
    pub t: f64,
}

pub struct SourceFile {
    pub path: PathBuf,
    pub ast: File,
    /// Indices into [`Program::files`] of the libraries this file `use`s,
    /// in the order they are searched: the most recent `use` first.
    pub uses: Vec<usize>,
}

#[derive(Debug)]
pub struct LoadError {
    pub path: PathBuf,
    pub src: String,
    pub diagnostic: Diagnostic,
}

impl LoadError {
    pub fn render(&self) -> String {
        self.diagnostic.render(&self.src, &self.path.display().to_string())
    }
}

impl Program {
    /// Loads `src` as the main file located at `path`. Imports resolve
    /// against the directory of the file that contains them, then
    /// `library_paths`.
    ///
    /// When the main file names a circuit (`// @circuit robot.c2fc`, next
    /// to it), the circuit is read too, its parts found next to it, then in
    /// `library_paths`; a circuit that cannot be read is a warning.
    pub fn load(src: &str, path: &Path, library_paths: &[PathBuf]) -> Result<Program, LoadError> {
        Program::load_in(src, path, library_paths, None)
    }

    /// [`Program::load`] for an agent: with a `scope`, every file the
    /// program names (includes, `use`s, data files, its circuit and the
    /// circuit's parts) is read only in its folders; one outside is a
    /// warning that names it and does not quote it.
    pub fn load_in(src: &str, path: &Path, library_paths: &[PathBuf], scope: Option<&FileScope>) -> Result<Program, LoadError> {
        let mut program = Program::load_alone(src, path, library_paths, scope)?;
        let (annotations, _) = c2f_lang::annotations::read(src);
        if let Some(link) = annotations.circuit {
            let circuit_path = path.parent().unwrap_or(Path::new("")).join(&link.path);
            if scope.is_some_and(|scope| !scope.allows(&circuit_path)) {
                program.warnings.push(format!("Can't read circuit: {}", FileScope::refusal(&link.path)));
                return Ok(program);
            }
            match c2f_circuit::load_in(&circuit_path, library_paths, scope) {
                Ok(circuit) => program.circuit = Some(circuit),
                Err(errors) => {
                    let errors: String = errors.iter().map(c2f_circuit::Error::render).collect();
                    program.warnings.push(format!("Can't read circuit '{}':\n{errors}", link.path));
                }
            }
            program.circuit_path = Some(circuit_path);
        }
        Ok(program)
    }

    /// The folders its files may be read from, when it was loaded for an
    /// agent.
    pub fn scope(&self) -> Option<&FileScope> {
        self.scope.as_ref()
    }

    /// [`Program::load`] without the circuit the file may name: for a
    /// part of that circuit.
    pub(crate) fn load_alone(
        src: &str,
        path: &Path,
        library_paths: &[PathBuf],
        scope: Option<&FileScope>,
    ) -> Result<Program, LoadError> {
        let mut loader = Loader { library_paths, scope, warnings: Vec::new(), files: Vec::new(), canonical: Vec::new() };
        loader.load_file(src.to_string(), path.to_path_buf())?;
        Ok(Program {
            files: loader.files,
            warnings: loader.warnings,
            circuit_path: None,
            circuit: None,
            library_paths: library_paths.to_vec(),
            scope: scope.cloned(),
            t: 0.0,
        })
    }
}

struct Loader<'a> {
    library_paths: &'a [PathBuf],
    scope: Option<&'a FileScope>,
    warnings: Vec<String>,
    files: Vec<SourceFile>,
    /// Canonical path of each loaded file, to load each only once.
    canonical: Vec<PathBuf>,
}

impl Loader<'_> {
    fn load_file(&mut self, src: String, path: PathBuf) -> Result<usize, LoadError> {
        let mut splicer = Splicer {
            library_paths: self.library_paths,
            scope: self.scope,
            warnings: &mut self.warnings,
            paths: vec![path.clone()],
            parents: vec![0],
            texts: vec![String::new()],
            open: Vec::new(),
        };
        let ast = match c2f_lang::parse_with_includes(&src, &mut splicer) {
            Ok(ast) => ast,
            Err(diagnostic) => {
                let file = diagnostic.span.file as usize;
                let (path, src) = match file {
                    0 => (path, src),
                    _ => (splicer.paths[file].clone(), std::mem::take(&mut splicer.texts[file])),
                };
                return Err(LoadError { path, src, diagnostic });
            }
        };
        let (paths, parents) = (splicer.paths, splicer.parents);

        let index = self.files.len();
        self.canonical.push(path.canonicalize().unwrap_or(path.clone()));
        self.files.push(SourceFile { path, ast, uses: Vec::new() });

        // Each `use` resolves against the directory of the file it is
        // written in, which may be an included one.
        let mut used = Vec::new();
        collect_uses(&self.files[index].ast.stmts, &mut used);
        let mut uses = Vec::new();
        for (name, file) in used {
            let dir = paths[file as usize].parent().map(Path::to_path_buf).unwrap_or_default();
            let is_font = Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"));
            // Files open when the `use` was read (its file and the ones that
            // included it) are skipped, as in the reference.
            let mut open = Vec::new();
            let mut at = file as usize;
            while at != 0 {
                open.push(paths[at].clone());
                at = parents[at] as usize;
            }
            let unit_dir = paths[0].parent().map(Path::to_path_buf).unwrap_or_default();
            // Not found next to its file: the reference retries next to
            // the file being loaded.
            let find = |scope| {
                resolve(self.library_paths, &name, &dir, &open, scope)
                    .or_else(|| resolve(self.library_paths, &name, &unit_dir, &open, scope))
            };
            let Some(found) = find(self.scope) else {
                match self.scope.is_some() && find(None).is_some() {
                    true => self.warnings.push(format!("Can't open library: {}", FileScope::refusal(&name))),
                    false => self.warnings.push(format!("Can't open library '{name}'.")),
                }
                continue;
            };
            if is_font {
                // Fonts are registered for text(), not loaded as libraries.
                continue;
            }
            let canonical = found.canonicalize().unwrap_or(found.clone());
            let existing = self.canonical.iter().position(|c| *c == canonical);
            let library = match existing {
                Some(existing) => existing,
                None => match read_source(&found) {
                    // A broken library is reported and skipped; the rest
                    // still runs, as in the reference.
                    Ok(text) => match self.load_file(text, found) {
                        Ok(library) => library,
                        Err(err) => {
                            self.warnings.push(format!("Can't parse library '{name}':\n{}", err.render()));
                            continue;
                        }
                    },
                    Err(err) => {
                        self.warnings.push(format!("Can't read library '{name}': {err}"));
                        continue;
                    }
                },
            };
            uses.push(library);
        }
        // A repeated `use` counts at its last position; the last is searched
        // first.
        let mut searched = Vec::new();
        for library in uses.into_iter().rev() {
            if !searched.contains(&library) {
                searched.push(library);
            }
        }
        self.files[index].uses = searched;
        Ok(index)
    }
}

/// Resolves includes for one file being parsed. Files currently open are
/// refused, which cuts include cycles; the file being parsed itself is not
/// on that stack, so it can include itself exactly once.
struct Splicer<'a> {
    library_paths: &'a [PathBuf],
    scope: Option<&'a FileScope>,
    warnings: &'a mut Vec<String>,
    /// Path of each file id handed out; id 0 is the file being parsed.
    paths: Vec<PathBuf>,
    /// The file that included each file id.
    parents: Vec<u32>,
    /// Text of each included file, for diagnostics.
    texts: Vec<String>,
    /// Canonical paths of the includes open right now.
    open: Vec<(u32, PathBuf)>,
}

impl Includes for Splicer<'_> {
    fn open(&mut self, name: &str, from: u32) -> Option<(u32, String)> {
        let dir = self.paths[from as usize].parent().map(Path::to_path_buf).unwrap_or_default();
        let open: Vec<PathBuf> = self.open.iter().map(|(_, p)| p.clone()).collect();
        // An open file is passed over for the next candidate on the path.
        let opened = resolve(self.library_paths, name, &dir, &open, self.scope).and_then(|path| {
            let canonical = path.canonicalize().ok()?;
            let text = read_source(&canonical).ok()?;
            Some((canonical, text))
        });
        let Some((canonical, text)) = opened else {
            match self.scope.is_some() && resolve(self.library_paths, name, &dir, &open, None).is_some() {
                true => self.warnings.push(format!("Can't open include file: {}", FileScope::refusal(name))),
                false => self.warnings.push(format!("Can't open include file '{name}'.")),
            }
            return None;
        };
        let id = self.paths.len() as u32;
        self.paths.push(canonical.clone());
        self.parents.push(from);
        self.texts.push(text.clone());
        self.open.push((id, canonical));
        Some((id, text))
    }

    fn close(&mut self, file: u32) {
        self.open.retain(|(id, _)| *id != file);
    }
}

/// The directory of the containing file first, then the library paths,
/// passing over files in `skip` (compared canonically) and, with a
/// `scope`, those outside it.
fn resolve(library_paths: &[PathBuf], name: &str, dir: &Path, skip: &[PathBuf], scope: Option<&FileScope>) -> Option<PathBuf> {
    std::iter::once(dir.to_path_buf())
        .chain(library_paths.iter().cloned())
        .map(|base| base.join(name))
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| scope.is_none_or(|scope| scope.allows(candidate)))
        .find(|candidate| {
            let canonical = candidate.canonicalize().unwrap_or(candidate.clone());
            !skip.iter().any(|s| *s == canonical)
        })
}

/// `use` statements of a file's top level with the id of the file they are
/// written in (an included file's, when they came through an include).
fn collect_uses(stmts: &[Stmt], out: &mut Vec<(String, u32)>) {
    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Use(name) => out.push((name.clone(), stmt.span.file)),
            StmtKind::Block(inner) => collect_uses(inner, out),
            _ => {}
        }
    }
}

/// Reads a source file. The reference accepts any bytes (a Latin-1 `°` in
/// a comment is common); invalid UTF-8 is replaced rather than refused.
pub fn read_source(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    })
}

/// The `Resources` folder of the macOS app this program runs in, where the
/// app keeps the fonts and libraries the reference ships, so that it works
/// without the reference installed. `None` outside an app.
pub fn app_resources() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let resources = macos.parent()?.join("Resources");
    resources.is_dir().then_some(resources)
}

/// The library search path: `C2F_LIBRARY_PATH`, then the user library
/// folder (`~/Documents/Code2Form/libraries`), then this app's own copy,
/// then the repository's `library` folder when this program runs where it
/// was built (`cargo run`).
pub fn default_library_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::env::var_os("C2F_LIBRARY_PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(Path::new(&home).join(USER_LIBRARIES));
    }
    paths.extend(app_resources().map(|resources| resources.join("libraries")));
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    if repository.is_dir() {
        paths.push(repository);
    }
    paths
}

/// The user library folder, under the home folder.
pub const USER_LIBRARIES: &str = "Documents/Code2Form/libraries";

/// The folder of the user's own electronics parts, in the user library:
/// where the app's "New part…" and the MCP server's `save_part` write
/// them, and the catalogue finds them. `None` without a home folder.
pub fn user_parts_folder() -> Option<PathBuf> {
    Some(Path::new(&std::env::var_os("HOME")?).join(USER_LIBRARIES).join("electronics"))
}
