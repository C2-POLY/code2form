//! What the user lets the agent do, enforced around its tools: a
//! [`Guarded`] provider refuses the tools the settings turn off, and keeps
//! the paths the tools take inside the project folder, the library
//! folders readable besides. The files a program names itself (includes,
//! `use`s, imports, a circuit's parts) are kept there too: the guard puts
//! its [`FileScope`] in the arguments ([`SCOPE_KEY`]), and the built-in
//! tools load models and circuits in it ([`file_scope`]).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

pub use c2f_lang::FileScope;
use c2f_lang::file_scope::resolve;
use serde_json::Value;

use crate::{Registry, Tools};

/// The tools' arguments that name a file or a folder.
const PATH_KEYS: [&str; 6] = ["path", "working_dir", "output", "brief", "design", "circuit"];

/// The tools that look a relative `path` up in the library when it is not
/// in the project folder.
const LIBRARY_LOOKUPS: [&str; 2] = ["part_pins", "check_part"];

/// The tools that write files.
const WRITING_TOOLS: [&str; 4] = ["export", "save_brief", "save_part", "electrify"];

/// The argument a [`Guarded`] provider sets to the folders the files a
/// program names may be read from: the project folder and the libraries,
/// as a list of paths. Whatever the agent itself gives there is replaced.
/// A host's own tools that run the built-in ones pass it on.
pub const SCOPE_KEY: &str = "_scope";

/// The scope the arguments carry ([`SCOPE_KEY`]): `None` when they carry
/// none, as with plain `c2f mcp`.
pub fn file_scope(args: &Value) -> Option<FileScope> {
    let roots = args.get(SCOPE_KEY)?.as_array()?;
    Some(FileScope::new(roots.iter().filter_map(Value::as_str).map(PathBuf::from)))
}

/// `args` with the scope `from` carries ([`SCOPE_KEY`]), if any: for a
/// tool that loads another model than the one it was given, or a host's
/// tool that runs a built-in one.
pub fn scoped(mut args: Value, from: &Value) -> Value {
    if let (Some(object), Some(scope)) = (args.as_object_mut(), from.get(SCOPE_KEY)) {
        object.insert(SCOPE_KEY.into(), scope.clone());
    }
    args
}

/// What the agent may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Write new files: `save_brief`.
    pub new_files: bool,
    /// `export` a model to a file.
    pub export: bool,
    /// `save_part` into the user's parts library.
    pub save_part: bool,
    /// `electrify` a design, which rewrites it and writes its circuit.
    pub electrify: bool,
    /// The user asks to be asked first ("Ask first" in Settings › Agent):
    /// the tools that write files are refused, as nothing asks them yet.
    pub ask_first: bool,
    /// Where the paths may lead; anywhere when `None`.
    pub scope: Option<Scope>,
}

impl Default for Policy {
    /// Everything allowed, anywhere.
    fn default() -> Policy {
        Policy { new_files: true, export: true, save_part: true, electrify: true, ask_first: false, scope: None }
    }
}

/// The folders the agent works in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// The project folder: the agent reads and writes in it, and below.
    pub folder: PathBuf,
    /// Folders the agent may read from, not write to: the libraries.
    pub read_only: Vec<PathBuf>,
}

impl Policy {
    /// Why the tool `name` is turned off, if it is.
    fn refusal(&self, name: &str) -> Option<String> {
        if self.ask_first && WRITING_TOOLS.contains(&name) {
            return Some(format!(
                "`{name}` writes a file, and Code2Form's settings ask to be asked first (Settings › Agent › \"Ask \
                 first\"), which this tool cannot do yet. Tell the user: they can switch to \"Apply directly\" in \
                 Settings › Agent, or do it from the studio themselves; do not try another way."
            ));
        }
        let (allowed, what) = match name {
            "save_brief" => (self.new_files, "Writing new files"),
            "export" => (self.export, "Exporting"),
            "save_part" => (self.save_part, "Saving parts to the library"),
            "electrify" => (self.electrify, "Electrifying a design"),
            _ => return None,
        };
        (!allowed).then(|| {
            format!(
                "`{name}` is not allowed: {what} is turned off in Code2Form's settings (Agent). Tell the user, who can \
                 allow it there; do not try another way."
            )
        })
    }

    /// The policy the studio's settings file at `settings` (`settings.txt`,
    /// see [`settings_file`]) sets: its `agent-*` keys, and its library
    /// folders readable besides the default ones, in `folder` when there
    /// is one. A missing or unreadable file allows everything, as the
    /// studio's defaults do.
    pub fn from_settings_file(settings: Option<&Path>, folder: Option<&Path>) -> Policy {
        let text = settings.and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        Policy::from_settings(&text, folder)
    }

    /// [`Policy::from_settings_file`] from the file's text.
    pub fn from_settings(text: &str, folder: Option<&Path>) -> Policy {
        let mut policy = Policy::default();
        let mut libraries: Vec<PathBuf> = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let flag = |field: &mut bool| {
                if let Ok(on) = value.parse() {
                    *field = on;
                }
            };
            match key.trim() {
                "agent-permission" => match value {
                    "ask" => policy.ask_first = true,
                    "apply" => policy.ask_first = false,
                    _ => {}
                },
                "agent-allow-new-files" => flag(&mut policy.new_files),
                "agent-allow-export" => flag(&mut policy.export),
                "agent-allow-save-part" => flag(&mut policy.save_part),
                "agent-allow-electrify" => flag(&mut policy.electrify),
                "library" => {
                    let path = PathBuf::from(value);
                    if path.is_absolute() && !libraries.contains(&path) {
                        libraries.push(path);
                    }
                }
                _ => {}
            }
        }
        let read_only = libraries.into_iter().chain(c2f_eval::default_library_paths()).collect();
        policy.scope = folder.map(|folder| Scope { folder: folder.to_path_buf(), read_only });
        policy
    }
}

/// The studio's settings file, `settings.txt` in its per-user folder;
/// `None` without a home folder, or in a screenshot run, which reads no
/// settings.
pub fn settings_file() -> Option<PathBuf> {
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    let dir = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Code2Form")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Code2Form")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?
            .join("code2form")
    };
    Some(dir.join("settings.txt"))
}

/// Whether the tool `name` writes the file its argument `key` names.
fn writes(name: &str, key: &str) -> bool {
    matches!((name, key), ("export", "output") | ("save_brief", "path") | ("electrify", "path" | "circuit"))
}

/// Whether a relative `path` stays where it is put: no `..`, no root.
fn plain(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

impl Scope {
    /// The folders the files a program names may be read from: the
    /// project folder and the libraries.
    pub fn file_scope(&self) -> FileScope {
        FileScope::new(std::iter::once(self.folder.clone()).chain(self.read_only.iter().cloned()))
    }

    /// The arguments of the tool `name` with their relative paths made
    /// absolute in the project folder (or, for a library lookup, in the
    /// library folder that has it); the error says which path leads
    /// outside.
    pub fn check(&self, name: &str, args: &Value) -> Result<Value, String> {
        let folder = resolve(&self.folder);
        let mut args = args.clone();
        for key in PATH_KEYS {
            let Some(asked) = args.get(key).and_then(Value::as_str).map(str::to_string) else { continue };
            let given = Path::new(&asked);
            if given.is_relative() {
                // Beside the design, which is checked itself.
                if name == "electrify" && key == "circuit" && plain(given) {
                    continue;
                }
                // A library part's name: its file in the library folders.
                if LIBRARY_LOOKUPS.contains(&name) && key == "path" && !folder.join(given).exists() {
                    let found = plain(given)
                        .then(|| self.read_only.iter().map(|dir| resolve(&dir.join(given))).find(|file| file.is_file()))
                        .flatten()
                        .filter(|file| self.read_only.iter().any(|dir| file.starts_with(resolve(dir))));
                    let Some(file) = found else {
                        return Err(format!(
                            "`{key}` = {asked} is neither in the project folder, {}, nor in the library folders",
                            folder.display()
                        ));
                    };
                    args[key] = Value::String(file.to_string_lossy().into_owned());
                    continue;
                }
            }
            let full = resolve(&if given.is_relative() { folder.join(given) } else { given.to_path_buf() });
            let inside = full.starts_with(&folder);
            let readable = !writes(name, key) && self.read_only.iter().any(|dir| full.starts_with(resolve(dir)));
            if !inside && !readable {
                let how = if writes(name, key) { "write" } else { "read" };
                return Err(format!(
                    "`{key}` = {asked} is outside the project folder, {}: the agent may only {how} there{}",
                    folder.display(),
                    if how == "read" { " and in the library folders" } else { "" }
                ));
            }
            args[key] = Value::String(full.to_string_lossy().into_owned());
        }
        Ok(args)
    }
}

/// Whether a file is open in the studio, by its path as the disk has it.
pub type OpenFiles = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// Tools behind a [`Policy`].
pub struct Guarded {
    tools: Registry,
    policy: Policy,
    open_files: Option<OpenFiles>,
}

impl Guarded {
    pub fn new(tools: Registry, policy: Policy) -> Guarded {
        Guarded { tools, policy, open_files: None }
    }

    /// A registry of `tools` behind `policy`.
    pub fn registry(tools: Registry, policy: Policy) -> Registry {
        Guarded::new(tools, policy).into_registry()
    }

    /// These tools in a registry of their own.
    pub fn into_registry(self) -> Registry {
        Registry::new().with(Box::new(self)).expect("a single provider")
    }

    /// Tells which files are open in the studio: `electrify`, which
    /// rewrites its design and writes its circuit, refuses when either is,
    /// as the studio would keep its own copy.
    pub fn open_files(mut self, open: OpenFiles) -> Guarded {
        self.open_files = Some(open);
        self
    }
}

impl Tools for Guarded {
    fn list(&self) -> Vec<Value> {
        self.tools.list()
    }

    /// The policy's checks, then the inner tools' call. An unknown tool is
    /// the inner tools' `None`, and a tool of a studio that closed says so
    /// itself: the list is not asked again for each call.
    fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
        if let Some(why) = self.policy.refusal(name) {
            return Some(Err(why));
        }
        let mut args = match &self.policy.scope {
            Some(scope) => match scope.check(name, args) {
                Ok(args) => args,
                Err(why) => return Some(Err(why)),
            },
            None => args.clone(),
        };
        if let Some(object) = args.as_object_mut() {
            object.remove(SCOPE_KEY);
            if let Some(scope) = &self.policy.scope {
                let roots = scope.file_scope().roots().iter().map(|root| Value::String(root.to_string_lossy().into_owned())).collect();
                object.insert(SCOPE_KEY.into(), Value::Array(roots));
            }
        }
        if name == "electrify" {
            let design = args.get("path").and_then(Value::as_str).map(|path| resolve(Path::new(path)));
            if let (Some(open), Some(design)) = (&self.open_files, design) {
                // The design and its circuit, beside it unless named.
                let circuit = match args.get("circuit").and_then(Value::as_str) {
                    Some(circuit) => resolve(&design.with_file_name(circuit)),
                    None => design.with_extension("c2fc"),
                };
                if let Some(file) = [design, circuit].into_iter().find(|file| open(file)) {
                    return Some(Err(format!(
                        "{} is open in the studio: `electrify` rewrites it, and the studio would keep its own copy. \
                         Ask the user to save and close its tab first, then try again.",
                        file.display()
                    )));
                }
            }
        }
        self.tools.call(name, &args)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A project folder with a design, and a library beside it.
    struct Folders {
        root: PathBuf,
    }

    impl Folders {
        fn new(name: &str) -> Folders {
            let root = std::env::temp_dir().join(format!("c2f-guard-{name}-{}", std::process::id()));
            std::fs::create_dir_all(root.join("project/sub")).unwrap();
            std::fs::create_dir_all(root.join("library/electronics")).unwrap();
            std::fs::write(root.join("project/box.c2f"), "cube(10);").unwrap();
            std::fs::write(root.join("library/electronics/led.c2f"), "module led() {}").unwrap();
            std::fs::write(root.join("secret.scad"), "cube(1);").unwrap();
            Folders { root }
        }

        fn scope(&self) -> Scope {
            Scope { folder: self.root.join("project"), read_only: vec![self.root.join("library")] }
        }
    }

    impl Drop for Folders {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn paths_stay_in_the_project_folder() {
        let folders = Folders::new("scope");
        let scope = folders.scope();
        let project = resolve(&folders.root.join("project"));
        let inside = scope.check("render", &json!({ "path": "box.c2f" })).unwrap();
        assert_eq!(inside["path"], project.join("box.c2f").to_string_lossy().as_ref(), "made absolute");
        let absolute = folders.root.join("project/sub/../box.c2f");
        assert!(scope.check("render", &json!({ "path": absolute })).is_ok());
        let new = scope.check("export", &json!({ "path": "box.c2f", "output": "out/box.stl" })).unwrap();
        assert_eq!(new["output"], project.join("out").join("box.stl").to_string_lossy().as_ref());

        for outside in [json!({ "path": "../secret.scad" }), json!({ "path": folders.root.join("secret.scad") }), json!({ "working_dir": "/" })] {
            let err = scope.check("render", &outside).unwrap_err();
            assert!(err.contains("outside the project folder"), "{err}");
        }
        let err = scope.check("export", &json!({ "code": "cube(1);", "output": "../out.stl" })).unwrap_err();
        assert!(err.contains("may only write"), "{err}");
    }

    #[test]
    fn the_library_is_read_only() {
        let folders = Folders::new("library");
        let scope = folders.scope();
        let led = folders.root.join("library/electronics/led.c2f");
        assert!(scope.check("check_part", &json!({ "path": led })).is_ok(), "read");
        let named = scope.check("part_pins", &json!({ "path": "electronics/led.c2f" })).unwrap();
        assert_eq!(named["path"], resolve(&led).to_string_lossy().as_ref(), "a library name, found there");
        assert!(scope.check("part_pins", &json!({ "path": "../../etc/passwd" })).is_err());
        let output = folders.root.join("library/led.stl");
        let err = scope.check("export", &json!({ "path": led, "output": output })).unwrap_err();
        assert!(err.contains("may only write"), "{err}");
        assert!(scope.check("save_brief", &json!({ "path": folders.root.join("library/brief.md") })).is_err());
    }

    #[test]
    fn a_relative_name_is_never_left_to_the_current_directory() {
        // A file the process's current directory has, in neither folder:
        // the guard used to pass the name on as is, and the tool found it
        // there.
        let folders = Folders::new("cwd");
        let scope = folders.scope();
        let cwd_file = "Cargo.toml";
        assert!(Path::new(cwd_file).is_file(), "the crate's folder is the tests' current directory");
        let err = scope.check("check_part", &json!({ "path": cwd_file })).unwrap_err();
        assert!(err.contains("neither in the project folder"), "{err}");
        let tools = Guarded::registry(Registry::builtin(), Policy { scope: Some(scope), ..Policy::default() });
        let err = tools.call("part_pins", &json!({ "path": cwd_file })).unwrap().unwrap_err();
        assert!(err.contains("neither in the project folder"), "{err}");
    }

    #[test]
    fn tools_turned_off_are_refused() {
        let policy = Policy { export: false, save_part: false, ..Policy::default() };
        let tools = Guarded::registry(Registry::builtin(), policy);
        assert_eq!(tools.names(), Registry::builtin().names(), "still listed");
        let err = tools.call("export", &json!({ "code": "cube(1);", "output": "x.stl" })).unwrap().unwrap_err();
        assert!(err.contains("`export` is not allowed") && err.contains("settings"), "{err}");
        let err = tools.call("save_part", &json!({ "name": "x", "code": "" })).unwrap().unwrap_err();
        assert!(err.contains("not allowed"), "{err}");
        assert!(tools.call("render", &json!({ "code": "cube(1);" })).unwrap().is_ok());
        assert!(tools.call("nothing", &json!({})).is_none());
        let policy = Policy { new_files: false, electrify: false, ..Policy::default() };
        let tools = Guarded::registry(Registry::builtin(), policy);
        for name in ["save_brief", "electrify"] {
            assert!(tools.call(name, &json!({ "path": "x" })).unwrap().unwrap_err().contains("not allowed"));
        }
    }

    #[test]
    fn ask_first_refuses_the_tools_that_write() {
        let tools = Guarded::registry(Registry::builtin(), Policy { ask_first: true, ..Policy::default() });
        for name in WRITING_TOOLS {
            let err = tools.call(name, &json!({ "path": "x.c2f", "output": "x.stl", "name": "x", "code": "" })).unwrap().unwrap_err();
            assert!(err.contains("Apply directly") && err.contains("Settings › Agent"), "{name}: {err}");
        }
        assert!(tools.call("render", &json!({ "code": "cube(1);" })).unwrap().is_ok(), "reading is not asked");
    }

    #[test]
    fn electrify_refuses_a_design_open_in_the_studio() {
        let folders = Folders::new("open");
        let design = resolve(&folders.root.join("project/box.c2f"));
        let open = design.clone();
        let tools = Guarded::new(Registry::builtin(), Policy { scope: Some(folders.scope()), ..Policy::default() })
            .open_files(Arc::new(move |path: &Path| path == open))
            .into_registry();
        let err = tools.call("electrify", &json!({ "path": "box.c2f" })).unwrap().unwrap_err();
        assert!(err.contains("open in the studio") && err.contains("close its tab"), "{err}");
        assert_eq!(std::fs::read_to_string(&design).unwrap(), "cube(10);", "left alone");
        // Its circuit open, by default beside it or as named.
        for (circuit, args) in [("box.c2fc", json!({ "path": "box.c2f" })), ("wiring.c2fc", json!({ "path": "box.c2f", "circuit": "wiring.c2fc" }))] {
            let circuit = resolve(&folders.root.join("project")).join(circuit);
            let tools = Guarded::new(Registry::builtin(), Policy { scope: Some(folders.scope()), ..Policy::default() })
                .open_files(Arc::new(move |path: &Path| path == circuit))
                .into_registry();
            let err = tools.call("electrify", &args).unwrap().unwrap_err();
            assert!(err.contains("c2fc is open in the studio"), "{err}");
        }
    }

    #[test]
    fn scoped_passes_the_scope_on() {
        let from = json!({ "code": "x", SCOPE_KEY: ["/project"] });
        assert_eq!(scoped(json!({ "path": "a.c2f" }), &from), json!({ "path": "a.c2f", SCOPE_KEY: ["/project"] }));
        assert_eq!(scoped(json!({ "path": "a.c2f" }), &json!({})), json!({ "path": "a.c2f" }), "none to pass");
    }

    #[test]
    fn a_guarded_registry_checks_the_paths() {
        let folders = Folders::new("registry");
        let policy = Policy { scope: Some(folders.scope()), ..Policy::default() };
        let tools = Guarded::registry(Registry::builtin(), policy);
        assert!(tools.call("render", &json!({ "path": "box.c2f" })).unwrap().is_ok(), "relative to the folder");
        let err = tools.call("render", &json!({ "path": folders.root.join("secret.scad") })).unwrap().unwrap_err();
        assert!(err.contains("outside"), "{err}");
    }

    #[test]
    fn the_program_s_own_files_stay_in_the_scope() {
        let folders = Folders::new("program");
        std::fs::write(folders.root.join("secret.txt"), "SECRET LINE: hunter2\n").unwrap();
        let secret = folders.root.join("secret.txt");
        let render = |tools: &Registry, code: String| tools.call("render", &json!({ "code": code, "working_dir": folders.root.join("project") })).unwrap();
        let include = format!("include <{}>\ncube(1);", secret.display());
        // Unguarded, as plain `c2f mcp`: read, and the error quotes it.
        let plain = render(&Registry::builtin(), include.clone()).unwrap_err();
        assert!(plain.contains("SECRET"), "{plain}");
        let tools = Guarded::registry(Registry::builtin(), Policy { scope: Some(folders.scope()), ..Policy::default() });
        let report = render(&tools, include).unwrap();
        let text = report[0]["text"].as_str().unwrap();
        assert!(text.contains("outside the folders") && !text.contains("SECRET"), "{text}");
        // The agent cannot widen the scope itself.
        let widened = json!({ "code": format!("include <{}>", secret.display()), "_scope": ["/"] });
        let report = tools.call("render", &widened).unwrap();
        let text = format!("{report:?}");
        assert!(!text.contains("SECRET"), "{text}");
        // A library the scope reads.
        let library = format!("include <{}>\ncube(1);", folders.root.join("library/electronics/led.c2f").display());
        let report = render(&tools, library).unwrap();
        assert!(!report[0]["text"].as_str().unwrap().contains("outside"));
    }

    #[cfg(unix)]
    #[test]
    fn a_closed_studio_says_so_rather_than_unknown_tool() {
        let socket = std::env::temp_dir().join(format!("c2f-guard-none-{}", std::process::id())).join("s.sock");
        let tools = Registry::builtin().with(Box::new(crate::studio_link::StudioProxy::at(socket))).unwrap();
        let tools = Guarded::registry(tools, Policy::default());
        let err = tools.call("edit_code", &json!({})).unwrap().unwrap_err();
        assert_eq!(err, crate::studio_link::NOT_OPEN);
    }

    #[test]
    fn calls_do_not_list_the_tools_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counted(Arc<AtomicUsize>);
        impl Tools for Counted {
            fn list(&self) -> Vec<Value> {
                self.0.fetch_add(1, Ordering::SeqCst);
                vec![json!({ "name": "counted" })]
            }
            fn call(&self, name: &str, _: &Value) -> Option<Result<Vec<Value>, String>> {
                (name == "counted").then(|| Ok(Vec::new()))
            }
        }
        let lists = Arc::new(AtomicUsize::new(0));
        let tools = Guarded::registry(Registry::new().with(Box::new(Counted(lists.clone()))).unwrap(), Policy::default());
        let before = lists.load(Ordering::SeqCst);
        for _ in 0..3 {
            assert!(tools.call("counted", &json!({})).unwrap().is_ok());
        }
        assert!(tools.call("nothing", &json!({})).is_none());
        assert_eq!(lists.load(Ordering::SeqCst), before);
    }

    #[test]
    fn the_settings_file_sets_the_policy() {
        // An absolute path on every system.
        let libs = std::env::temp_dir().join("my-libs");
        let text = format!(
            "agent-permission = ask\nagent-allow-export = false\nagent-allow-save-part = maybe\n\
             library = {}\nlibrary = relative\nother = 1\n",
            libs.display()
        );
        let policy = Policy::from_settings(&text, Some(Path::new("/project")));
        assert!(policy.ask_first && !policy.export && policy.save_part && policy.new_files && policy.electrify);
        let scope = policy.scope.unwrap();
        assert_eq!(scope.folder, Path::new("/project"));
        assert_eq!(scope.read_only[0], libs);
        assert!(!scope.read_only.contains(&PathBuf::from("relative")));
        assert_eq!(Policy::from_settings_file(Some(Path::new("/no/such/settings.txt")), None), Policy::default());
    }
}
