//! A circuit read for an agent finds its parts and its enclosure in the
//! agent's folders alone, and never quotes a file outside them.

use std::path::PathBuf;

use c2f_circuit::loads::{Scenario, find_part_in, with_sheets_in};
use c2f_lang::FileScope;

struct Folders(PathBuf);

impl Folders {
    fn new(name: &str) -> Folders {
        let root = std::env::temp_dir().join(format!("c2f-circuit-scope-{name}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("project")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        let part = "// @pin 1 [0, 0, 0] down passive\n// @spec static load: 0.33 kN\ncube(1);\n";
        std::fs::write(root.join("project/led.c2f"), part).unwrap();
        std::fs::write(root.join("outside/secret.c2f"), format!("// SECRET LINE\n{part}")).unwrap();
        Folders(root)
    }

    fn scope(&self) -> FileScope {
        FileScope::new([self.0.join("project")])
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parts_and_enclosures_outside_the_scope_are_refused() {
    let folders = Folders::new("parts");
    let path = folders.0.join("project/robot.c2fc");
    let secret = folders.0.join("outside/secret.c2f");
    let inside = "[parts.led]\npart = \"led.c2f\"\n";
    let circuit = c2f_circuit::parse_in(inside, &path, &[], Some(&folders.scope())).unwrap();
    assert_eq!(circuit.scope, Some(folders.scope()), "the circuit carries its scope");

    for src in [
        format!("[parts.led]\npart = \"{}\"\n", secret.display()),
        "[parts.led]\npart = \"../outside/secret.c2f\"\n".to_string(),
        format!("enclosure = \"{}\"\n", secret.display()),
    ] {
        // Unscoped, as `c2f circuit` reads it: found.
        assert!(c2f_circuit::parse(&src, &path, &[]).is_ok(), "{src}");
        let errors = c2f_circuit::parse_in(&src, &path, &[], Some(&folders.scope())).unwrap_err();
        let shown: String = errors.iter().map(c2f_circuit::Error::render).collect();
        assert!(shown.contains("outside the folders"), "{shown}");
        assert!(!shown.contains("SECRET"), "{shown}");
    }
    // The circuit file itself.
    std::fs::write(folders.0.join("outside/x.c2fc"), "SECRET LINE = \n").unwrap();
    let errors = c2f_circuit::load_in(&folders.0.join("outside/x.c2fc"), &[], Some(&folders.scope())).unwrap_err();
    assert!(!errors.iter().map(c2f_circuit::Error::render).collect::<String>().contains("SECRET"));
}

#[test]
fn a_scoped_load_finds_parts_in_the_scope_only_never_from_the_current_directory() {
    let folders = Folders::new("loads");
    let scope = folders.scope();
    let dirs = [folders.0.join("project")];
    assert!(find_part_in("led.c2f", &dirs, Some(&scope)).is_ok());
    let secret = folders.0.join("outside/secret.c2f");
    let err = find_part_in(&secret.display().to_string(), &dirs, Some(&scope)).unwrap_err();
    assert!(err.contains("outside the folders"), "{err}");
    assert!(find_part_in(&secret.display().to_string(), &dirs, None).is_ok(), "unscoped: as is");
    // A name relative to the current directory is not looked up there.
    let cwd = std::env::current_dir().unwrap();
    let relative = pathdiff(&secret, &cwd);
    assert!(find_part_in(&relative, &[], None).is_ok(), "unscoped: from the current directory");
    assert!(find_part_in(&relative, &[], Some(&scope)).is_err());

    let scenario: Scenario = serde_json::from_value(serde_json::json!({
        "axis": { "at": [0, 0, 0], "direction": [1, 0, 0] },
        "supports": [{ "name": "motor", "at_mm": 0, "kind": "motor", "part": secret.display().to_string() }],
    }))
    .unwrap();
    let err = with_sheets_in(&scenario, &dirs, Some(&scope)).unwrap_err();
    assert!(err.contains("outside the folders") && !err.contains("SECRET"), "{err}");
    assert!(with_sheets_in(&scenario, &dirs, None).is_ok());
}

/// `path` relative to `base`, through `..`.
fn pathdiff(path: &std::path::Path, base: &std::path::Path) -> String {
    let path = path.canonicalize().unwrap();
    let base = base.canonicalize().unwrap();
    let common = path.components().zip(base.components()).take_while(|(a, b)| a == b).count();
    let ups = base.components().count() - common;
    let rest: PathBuf = path.components().skip(common).collect();
    let mut out = PathBuf::new();
    for _ in 0..ups {
        out.push("..");
    }
    out.join(rest).display().to_string()
}
