//! What the user lets the agent do, enforced around its tools: a
//! [`Guarded`] provider refuses the tools the settings turn off, and keeps
//! the paths the tools take inside the project folder, the library
//! folders readable besides.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::{Registry, Tools};

/// The tools' arguments that name a file or a folder.
const PATH_KEYS: [&str; 6] = ["path", "working_dir", "output", "brief", "design", "circuit"];

/// The tools that look a relative `path` up in the library when it is not
/// in the project folder.
const LIBRARY_LOOKUPS: [&str; 2] = ["part_pins", "check_part"];

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
    /// Where the paths may lead; anywhere when `None`.
    pub scope: Option<Scope>,
}

impl Default for Policy {
    /// Everything allowed, anywhere.
    fn default() -> Policy {
        Policy { new_files: true, export: true, save_part: true, electrify: true, scope: None }
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
}

/// Whether the tool `name` writes the file its argument `key` names.
fn writes(name: &str, key: &str) -> bool {
    matches!((name, key), ("export", "output") | ("save_brief", "path") | ("electrify", "path" | "circuit"))
}

/// `path` without `.` and with each `..` taking off the component before
/// it, without reading the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` as the disk has it: links followed for the part that exists,
/// the rest as written.
fn resolve(path: &Path) -> PathBuf {
    let path = normalize(path);
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            return rest.iter().rev().fold(real, |full, part| full.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return path,
        }
    }
}

/// Whether a relative `path` stays where it is put: no `..`, no root.
fn plain(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

impl Scope {
    /// The arguments of the tool `name` with their relative paths made
    /// absolute in the project folder; the error says which path leads
    /// outside.
    pub fn check(&self, name: &str, args: &Value) -> Result<Value, String> {
        let folder = resolve(&self.folder);
        let mut args = args.clone();
        for key in PATH_KEYS {
            let Some(asked) = args.get(key).and_then(Value::as_str).map(str::to_string) else { continue };
            let given = Path::new(&asked);
            if given.is_relative() {
                // Beside the design, which is checked itself.
                let beside = name == "electrify" && key == "circuit";
                let in_library = LIBRARY_LOOKUPS.contains(&name) && key == "path" && !folder.join(given).exists();
                if (beside || in_library) && plain(given) {
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

/// Tools behind a [`Policy`].
pub struct Guarded {
    tools: Registry,
    policy: Policy,
}

impl Guarded {
    pub fn new(tools: Registry, policy: Policy) -> Guarded {
        Guarded { tools, policy }
    }

    /// A registry of `tools` behind `policy`.
    pub fn registry(tools: Registry, policy: Policy) -> Registry {
        Registry::new().with(Box::new(Guarded::new(tools, policy))).expect("a single provider")
    }
}

impl Tools for Guarded {
    fn list(&self) -> Vec<Value> {
        self.tools.list()
    }

    fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
        if !self.tools.names().iter().any(|n| n == name) {
            return None;
        }
        if let Some(why) = self.policy.refusal(name) {
            return Some(Err(why));
        }
        let args = match &self.policy.scope {
            Some(scope) => match scope.check(name, args) {
                Ok(args) => args,
                Err(why) => return Some(Err(why)),
            },
            None => args.clone(),
        };
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
        assert_eq!(new["output"], project.join("out/box.stl").to_string_lossy().as_ref());

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
        assert_eq!(named["path"], "electronics/led.c2f", "a library name, looked up there");
        assert!(scope.check("part_pins", &json!({ "path": "../../etc/passwd" })).is_err());
        let output = folders.root.join("library/led.stl");
        let err = scope.check("export", &json!({ "path": led, "output": output })).unwrap_err();
        assert!(err.contains("may only write"), "{err}");
        assert!(scope.check("save_brief", &json!({ "path": folders.root.join("library/brief.md") })).is_err());
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
    fn a_guarded_registry_checks_the_paths() {
        let folders = Folders::new("registry");
        let policy = Policy { scope: Some(folders.scope()), ..Policy::default() };
        let tools = Guarded::registry(Registry::builtin(), policy);
        assert!(tools.call("render", &json!({ "path": "box.c2f" })).unwrap().is_ok(), "relative to the folder");
        let err = tools.call("render", &json!({ "path": folders.root.join("secret.scad") })).unwrap().unwrap_err();
        assert!(err.contains("outside"), "{err}");
    }
}
