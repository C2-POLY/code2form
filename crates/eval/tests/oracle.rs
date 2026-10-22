//! Compares evaluation with the reference implementation itself: the CSG
//! tree (`-o x.csg`) and the ECHO output.
//!
//! `goldens` checks the committed corpus in `tests/oracle/eval`, and
//! `parameter_sets` the customizer sets in `tests/oracle/params`
//! (regenerate both with `scripts/oracle.sh`). `live_examples` runs the
//! installed reference on the examples and libraries shipped with it; it is
//! ignored by default: `cargo test -p c2f-eval -- --include-ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn diff(label: &str, expected: &str, actual: &str) -> Option<String> {
    if expected == actual {
        return None;
    }
    let (e, a): (Vec<_>, Vec<_>) = (expected.lines().collect(), actual.lines().collect());
    let mut report = format!("  {label} differs:\n");
    let mut shown = 0;
    for i in 0..e.len().max(a.len()) {
        let (el, al) = (e.get(i).copied().unwrap_or("<missing>"), a.get(i).copied().unwrap_or("<missing>"));
        if el != al {
            report.push_str(&format!("    line {}:\n      reference: {el}\n      ours:      {al}\n", i + 1));
            shown += 1;
            if shown == 6 {
                break;
            }
        }
    }
    if shown == 0 {
        report.push_str("    (only trailing whitespace differs)\n");
    }
    Some(report)
}

/// Runs our evaluator on a file; returns (csg, echo).
fn ours(scad: &Path) -> Result<(String, String), String> {
    let src = c2f_eval::read_source(scad).map_err(|e| e.to_string())?;
    let program = c2f_eval::Program::load(&src, scad, &c2f_eval::default_library_paths()).map_err(|e| e.render())?;
    let output = c2f_eval::evaluate(&program);
    let mut echo = output.echo.join("\n");
    if !echo.is_empty() {
        echo.push('\n');
    }
    Ok((output.csg(), echo))
}

/// Blanks the `timestamp = <mtime>` of data files, which a fresh checkout
/// changes.
fn without_timestamps(csg: &str) -> String {
    let mut out = String::with_capacity(csg.len());
    let mut rest = csg;
    while let Some(at) = rest.find("timestamp = ") {
        let (head, tail) = rest.split_at(at + "timestamp = ".len());
        out.push_str(head);
        out.push('_');
        rest = tail.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
    }
    out.push_str(rest);
    out
}

fn check(scad: &Path, expected_csg: &str, expected_echo: &str) -> Option<String> {
    let name = scad.file_name().unwrap().to_string_lossy().to_string();
    match ours(scad) {
        Err(err) => Some(format!("{name}: failed to load\n{err}")),
        Ok((csg, echo)) => {
            let (expected_csg, csg) = (without_timestamps(expected_csg), without_timestamps(&csg));
            let report: String =
                [diff("csg", &expected_csg, &csg), diff("echo", expected_echo, &echo)].into_iter().flatten().collect();
            (!report.is_empty()).then(|| format!("{name}:\n{report}"))
        }
    }
}

#[test]
fn goldens() {
    let dir = root().join("tests/oracle/eval");
    let mut scads: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "scad"))
        .collect();
    scads.sort();
    assert!(!scads.is_empty(), "no eval oracle cases");
    let failures: Vec<String> = scads
        .iter()
        .filter_map(|scad| {
            let read = |ext| {
                std::fs::read_to_string(scad.with_extension(ext))
                    .unwrap_or_else(|_| panic!("{} has no .{ext} golden, run scripts/oracle.sh", scad.display()))
            };
            check(scad, &read("csg"), &read("echo"))
        })
        .collect();
    assert!(failures.is_empty(), "{} of {} cases disagree with the reference:\n\n{}", failures.len(), scads.len(), failures.join("\n"));
}

/// Each set of `case.json` applied to `case.scad` (`-p case.json -P set`),
/// against `case.N.csg` and `case.N.echo`.
#[test]
fn parameter_sets() {
    let dir = root().join("tests/oracle/params");
    let mut scads: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "scad"))
        .collect();
    scads.sort();
    assert!(!scads.is_empty(), "no parameter set cases");
    let mut failures = Vec::new();
    let mut count = 0;
    for scad in &scads {
        let json = std::fs::read_to_string(scad.with_extension("json")).unwrap();
        let sets = c2f_eval::customizer::ParameterSets::parse(&json).unwrap();
        let src = c2f_eval::read_source(scad).unwrap();
        for (n, (name, set)) in sets.sets.iter().enumerate() {
            count += 1;
            let golden = |ext: &str| {
                let path = scad.with_extension(format!("{}.{ext}", n + 1));
                std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no {}, run scripts/oracle.sh", path.display()))
            };
            let mut program = c2f_eval::Program::load(&src, scad, &[]).unwrap();
            // A set that cannot be read is applied up to the bad value.
            let _ = program.apply_parameter_set(set);
            let output = c2f_eval::evaluate(&program);
            let mut echo = output.echo.join("\n");
            if !echo.is_empty() {
                echo.push('\n');
            }
            let report: String =
                [diff("csg", &golden("csg"), &output.csg()), diff("echo", &golden("echo"), &echo)].into_iter().flatten().collect();
            if !report.is_empty() {
                failures.push(format!("{} with set {name:?}:\n{report}", scad.display()));
            }
        }
    }
    assert!(failures.is_empty(), "{} of {count} sets disagree with the reference:\n\n{}", failures.len(), failures.join("\n"));
}

#[test]
#[ignore = "needs the reference desktop app; set C2F_REFERENCE to its executable"]
fn live_examples() {
    let reference = std::env::var("C2F_REFERENCE").expect("C2F_REFERENCE names the reference's executable");
    let resources = Path::new(&reference).parent().expect("C2F_REFERENCE is a file path").join("../Resources");
    let mut scads = Vec::new();
    for dir in ["examples", "libraries"] {
        collect_scad(&resources.join(dir), &mut scads);
    }
    assert!(!scads.is_empty(), "no bundled .scad files under {}", resources.display());

    let out = std::env::temp_dir().join(format!("c2f-eval-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let awk = root().join("scripts/echo-lines.awk");
    let mut failures = Vec::new();
    for scad in &scads {
        let csg = out.join("out.csg");
        let _ = std::fs::remove_file(&csg);
        let run = Command::new(&reference)
            .arg("-o")
            .arg(&csg)
            .arg(scad.file_name().unwrap())
            .current_dir(scad.parent().unwrap())
            .output()
            .expect("the reference runs");
        std::fs::write(out.join("log"), &run.stderr).unwrap();
        let echo = Command::new("awk").arg("-f").arg(&awk).arg(out.join("log")).output().unwrap();
        let expected_csg = std::fs::read_to_string(&csg).unwrap_or_default();
        if let Some(failure) = check(scad, &expected_csg, &String::from_utf8_lossy(&echo.stdout)) {
            failures.push(format!("{}\n{failure}", scad.display()));
        }
    }
    let _ = std::fs::remove_dir_all(&out);
    assert!(failures.is_empty(), "{} of {} files disagree with the reference:\n\n{}", failures.len(), scads.len(), failures.join("\n"));
    eprintln!("{} bundled files match the reference", scads.len());
}

fn collect_scad(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_scad(&path, out);
        } else if path.extension().is_some_and(|e| e == "scad") {
            out.push(path);
        }
    }
}
