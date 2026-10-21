//! Compares the parser with the reference implementation.
//!
//! `goldens` checks the committed corpus in `tests/oracle/cases` (regenerate
//! with `scripts/oracle.sh`). `live_examples` runs the installed reference on
//! the examples and libraries shipped with it; it is ignored by default:
//! `cargo test -p c2f-lang -- --include-ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

use c2f_lang::dump::ast_dump;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/oracle/cases")
}

/// Line-by-line report of the first differences.
fn diff(expected: &str, actual: &str) -> String {
    let (e, a): (Vec<_>, Vec<_>) = (expected.lines().collect(), actual.lines().collect());
    let mut report = String::new();
    for i in 0..e.len().max(a.len()) {
        let (el, al) = (e.get(i).copied().unwrap_or("<missing>"), a.get(i).copied().unwrap_or("<missing>"));
        if el != al {
            report.push_str(&format!("  line {}:\n    reference: {el}\n    ours:      {al}\n", i + 1));
            if report.lines().count() > 15 {
                break;
            }
        }
    }
    report
}

/// Splices includes from disk, relative to the including file, refusing
/// files already open (as the reference does).
struct DiskIncludes {
    paths: Vec<PathBuf>,
    open: Vec<PathBuf>,
}

impl c2f_lang::Includes for DiskIncludes {
    fn open(&mut self, name: &str, from: u32) -> Option<(u32, String)> {
        let dir = self.paths[from as usize].parent()?.to_path_buf();
        let path = dir.join(name).canonicalize().ok()?;
        if self.open.contains(&path) {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        self.paths.push(path.clone());
        self.open.push(path);
        Some((self.paths.len() as u32 - 1, text))
    }

    fn close(&mut self, file: u32) {
        let path = &self.paths[file as usize];
        self.open.retain(|p| p != path);
    }
}

/// Returns a failure description, or None when the parser agrees.
fn check(scad: &Path, expected_ast: Option<&str>) -> Option<String> {
    let src = std::fs::read_to_string(scad).unwrap();
    let name = scad.file_name().unwrap().to_string_lossy();
    let mut includes = DiskIncludes { paths: vec![scad.to_path_buf()], open: Vec::new() };
    match (c2f_lang::parse_with_includes(&src, &mut includes), expected_ast) {
        (Ok(file), Some(expected)) => {
            let actual = ast_dump(&file, &src);
            (actual != expected).then(|| format!("{name}: AST differs\n{}", diff(expected, &actual)))
        }
        (Ok(_), None) => Some(format!("{name}: the reference rejects it, we accepted it")),
        (Err(err), Some(_)) => Some(format!("{name}: the reference accepts it, we failed:\n{}", err.render(&src, &name))),
        (Err(_), None) => None,
    }
}

#[test]
fn goldens() {
    let mut scads: Vec<PathBuf> = std::fs::read_dir(cases_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "scad"))
        .collect();
    scads.sort();
    assert!(!scads.is_empty(), "no oracle cases found");

    let failures: Vec<String> = scads
        .iter()
        .filter_map(|scad| {
            let ast = scad.with_extension("ast");
            let err = scad.with_extension("err");
            assert!(ast.exists() || err.exists(), "{} has no golden, run scripts/oracle.sh", scad.display());
            check(scad, std::fs::read_to_string(ast).ok().as_deref())
        })
        .collect();
    assert!(failures.is_empty(), "{} of {} cases disagree with the reference:\n\n{}", failures.len(), scads.len(), failures.join("\n"));
}

#[test]
#[ignore = "needs the desktop app of the reference release (2021.01); set C2F_REFERENCE to override its path"]
fn live_examples() {
    let reference = std::env::var("C2F_REFERENCE")
        .unwrap_or_else(|_| "/Applications/OpenSCAD-2021.01.app/Contents/MacOS/OpenSCAD".into());
    let resources = Path::new(&reference).parent().expect("C2F_REFERENCE is a file path").join("../Resources");
    let mut scads = Vec::new();
    for dir in ["examples", "libraries"] {
        collect_scad(&resources.join(dir), &mut scads);
    }
    assert!(!scads.is_empty(), "no bundled .scad files under {}", resources.display());

    let out_dir = std::env::temp_dir().join(format!("scad-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut failures = Vec::new();
    let mut compared = 0;
    for scad in &scads {
        let ast = out_dir.join("out.ast");
        let _ = std::fs::remove_file(&ast);
        Command::new(&reference).arg("-o").arg(&ast).arg(scad).output().expect("the reference runs");
        compared += 1;
        if let Some(failure) = check(scad, std::fs::read_to_string(&ast).ok().as_deref()) {
            failures.push(format!("{}\n{failure}", scad.display()));
        }
    }
    let _ = std::fs::remove_dir_all(&out_dir);
    assert!(failures.is_empty(), "{} of {compared} files disagree with the reference:\n\n{}", failures.len(), failures.join("\n"));
    eprintln!("{compared} bundled files match the reference");
}

fn collect_scad(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_scad(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "scad") {
            out.push(path);
        }
    }
}
