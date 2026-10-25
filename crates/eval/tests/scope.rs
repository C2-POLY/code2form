//! A program loaded for an agent reads its files in the agent's folders
//! alone: an include, a `use`, a data file or a circuit outside them is a
//! warning that names it and never quotes it. Unscoped, it reads them.

use std::path::{Path, PathBuf};

use c2f_eval::Program;
use c2f_lang::FileScope;

struct Folders(PathBuf);

impl Folders {
    fn new(name: &str) -> Folders {
        let root = std::env::temp_dir().join(format!("c2f-eval-scope-{name}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("project")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        // Not a program: an include of it fails to parse, quoting it.
        std::fs::write(root.join("outside/secret.txt"), "SECRET LINE: hunter2\n").unwrap();
        std::fs::write(root.join("outside/lib.scad"), "module secret() { cube(1); }\n").unwrap();
        std::fs::write(root.join("outside/x.c2fc"), "[parts.a]\npart = \"y.c2f\"\n").unwrap();
        std::fs::write(root.join("project/inside.scad"), "cube(2);\n").unwrap();
        Folders(root)
    }

    fn scope(&self) -> FileScope {
        FileScope::new([self.0.join("project")])
    }

    fn main(&self) -> PathBuf {
        self.0.join("project/untitled.c2f")
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn shown(result: Result<Program, c2f_eval::LoadError>) -> String {
    match result {
        Ok(program) => {
            let output = c2f_eval::evaluate(&program);
            format!("{:?} {:?}", output.warnings, output.error)
        }
        Err(err) => err.render(),
    }
}

#[test]
fn an_include_outside_the_scope_is_refused_without_quoting_it() {
    let folders = Folders::new("include");
    let secret = folders.0.join("outside/secret.txt");
    let src = format!("include <{}>\ncube(1);\n", secret.display());
    // Unscoped (`c2f`, plain `c2f mcp`): read, and its error quotes it.
    let unscoped = Program::load(&src, &folders.main(), &[]);
    assert!(unscoped.as_ref().is_err_and(|err| err.render().contains("SECRET")));
    let scoped = shown(Program::load_in(&src, &folders.main(), &[], Some(&folders.scope())));
    assert!(scoped.contains("outside the folders"), "{scoped}");
    assert!(!scoped.contains("SECRET") && !scoped.contains("hunter2"), "{scoped}");
    // Relative, through `..`.
    let src = "include <../outside/secret.txt>\ncube(1);\n";
    let scoped = shown(Program::load_in(src, &folders.main(), &[], Some(&folders.scope())));
    assert!(scoped.contains("outside the folders") && !scoped.contains("SECRET"), "{scoped}");
    // A file inside is read.
    let program = Program::load_in("include <inside.scad>\n", &folders.main(), &[], Some(&folders.scope())).unwrap();
    assert!(c2f_eval::evaluate(&program).warnings.is_empty());
}

#[test]
fn a_use_a_data_file_and_a_circuit_outside_the_scope_are_refused() {
    let folders = Folders::new("use");
    let lib = folders.0.join("outside/lib.scad");
    let stl = folders.0.join("outside/secret.txt");
    let circuit = folders.0.join("outside/x.c2fc");
    let src = format!(
        "// @circuit {}\nuse <{}>\nsecret();\nimport(\"{}\");\nsurface(file = \"{}\");\n",
        circuit.display(),
        lib.display(),
        stl.display(),
        stl.display()
    );
    let scoped = Program::load_in(&src, &folders.main(), &[], Some(&folders.scope())).unwrap();
    assert!(scoped.circuit.is_none() && scoped.circuit_path.is_none());
    let output = c2f_eval::evaluate(&scoped);
    let warnings = output.warnings.join("\n");
    assert!(warnings.contains("Can't open library: '"), "{warnings}");
    assert!(warnings.contains("Can't read circuit: '"), "{warnings}");
    assert_eq!(warnings.matches("Can't open file: '").count(), 2, "{warnings}");
    assert!(!warnings.contains("SECRET"), "{warnings}");
    // Nothing names the file for the renderer to read.
    let csg = output.csg();
    assert!(!csg.contains("secret.txt"), "{csg}");

    let unscoped = Program::load(&src, &folders.main(), &[]).unwrap();
    assert!(unscoped.circuit_path.is_some());
    let output = c2f_eval::evaluate(&unscoped);
    assert!(output.csg().contains("secret.txt"), "unscoped: the data file is named");
    assert!(!output.warnings.iter().any(|w| w.contains("Can't open library")), "unscoped: the library is used");
    assert!(Path::new(&lib).exists());
}
