//! `c2f circuit` on the Grogu example, against the goldens in
//! `tests/circuit/`. `C2F_BLESS=1 cargo test -p c2f-cli --test circuit`
//! rewrites them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `c2f circuit <args>` in `dir`, so that the paths it prints are
/// relative and the goldens do not depend on where the repository is.
fn c2f(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_c2f"))
        .arg("circuit")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running c2f")
}

fn grogu(args: &[&str]) -> Output {
    let output = c2f(&manifest().join("../../examples/grogu"), args);
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    output
}

/// Compares `actual` with the golden file `name`, or rewrites it with
/// `C2F_BLESS=1`. Numbers may differ in their last digits, as volumes do
/// from one platform to another.
fn golden(name: &str, actual: &str) {
    let path = manifest().join("tests/circuit").join(name);
    if std::env::var_os("C2F_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{}: {err} (C2F_BLESS=1 writes it)", path.display()));
    let same = |a: &str, b: &str| match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    };
    let (a, e): (Vec<&str>, Vec<&str>) = (actual.split([' ', '\n']).collect(), expected.split([' ', '\n']).collect());
    let matches = a.len() == e.len() && a.iter().zip(&e).all(|(a, e)| same(a, e));
    assert!(matches, "{name} differs from its golden (C2F_BLESS=1 rewrites it):\n{actual}");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn check() {
    let output = grogu(&["check", "grogu.c2fc"]);
    golden("check.txt", &(text(&output.stderr) + &text(&output.stdout)));
}

#[test]
fn wires() {
    golden("wires.txt", &text(&grogu(&["wires", "grogu.c2fc"]).stdout));
    golden("wires.csv", &text(&grogu(&["wires", "--csv", "grogu.c2fc"]).stdout));
}

#[test]
fn bom() {
    golden("bom.txt", &text(&grogu(&["bom", "grogu.c2fc"]).stdout));
    golden("bom.csv", &text(&grogu(&["bom", "grogu.c2fc", "--csv"]).stdout));
}

#[test]
fn exports() {
    let stl = grogu(&["stl", "grogu.c2fc"]);
    assert!(text(&stl.stdout).starts_with("solid Code2Form\n"));
    golden("stl.txt", &text(&stl.stderr));
    let package = grogu(&["3mf", "grogu.c2fc"]);
    assert!(package.stdout.starts_with(b"PK\x03\x04"));
    // The same parts, so the same size.
    assert_eq!(text(&package.stderr), text(&stl.stderr));
}

/// A folder of its own holding `files`, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str, files: &[(&str, &str)]) -> Folder {
        let dir = std::env::temp_dir().join(format!("c2f-cli-circuit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (file, text) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Folder(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn errors_fail_with_their_line() {
    let folder = Folder::new(
        "errors",
        &[
            ("board.c2f", "// @pin A [0, 0, 0]\n// @pin B [1, 0, 0]\nassert(size > 1, \"too small\");\ncube(size);\n"),
            ("broken.c2f", "// @pin A [0, 0, 0]\ncube(;\n"),
            (
                "test.c2fc",
                "[parts.a]\npart = \"board.c2f\"\nparams = { size = 1 }\n\n[parts.b]\npart = \"board.c2f\"\nparams = { size = 2 }\n\n\
                 [parts.c]\npart = \"broken.c2f\"\n\n[[wires]]\nfrom = \"a.A\"\nto = \"b.B\"\n",
            ),
        ],
    );
    let output = c2f(&folder.0, &["check", "test.c2fc"]);
    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    // The part that stops, at its name in the circuit; the one that does
    // not parse, in its own file.
    assert!(stderr.contains("error: `a` (board.c2f) stops:") && stderr.contains("too small"), "{stderr}");
    assert!(stderr.contains("--> test.c2fc:1:8"), "{stderr}");
    assert!(stderr.contains("--> broken.c2f:2:6"), "{stderr}");
    assert!(stderr.ends_with("test.c2fc: 3 parts, 1 wire: 2 errors\n"), "{stderr}");
    assert!(!c2f(&folder.0, &["stl", "test.c2fc"]).status.success());

    // A circuit that does not read fails every command.
    let output = c2f(&folder.0, &["wires", "missing.c2fc"]);
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("cannot read the circuit"));
    assert_eq!(c2f(&folder.0, &["solder", "test.c2fc"]).status.code(), Some(2));
    assert_eq!(c2f(&folder.0, &["check", "--csv", "test.c2fc"]).status.code(), Some(2));
}
