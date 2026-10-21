//! Compares our geometry with the reference implementation's: the
//! volume of the STL it exports for each example and library file
//! bundled with its desktop app. Ignored by default (it needs the app and
//! takes a while):
//! `cargo test --release -p c2f-geom -- --include-ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Relative volume difference tolerated (the STL rounds coordinates).
const TOLERANCE: f64 = 1e-4;

fn reference() -> String {
    std::env::var("C2F_REFERENCE").unwrap_or_else(|_| "/Applications/OpenSCAD-2021.01.app/Contents/MacOS/OpenSCAD".into())
}

/// Signed volume of an ASCII or binary STL.
fn stl_volume(bytes: &[u8]) -> Option<f64> {
    let triangles: Vec<[[f64; 3]; 3]> = if bytes.starts_with(b"solid") && bytes.windows(5).any(|w| w == b"facet") {
        let text = String::from_utf8_lossy(bytes);
        let vertices: Vec<[f64; 3]> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("vertex"))
            .map(|rest| {
                let v: Vec<f64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                [v[0], v[1], v[2]]
            })
            .collect();
        vertices.chunks(3).filter(|c| c.len() == 3).map(|c| [c[0], c[1], c[2]]).collect()
    } else {
        let count = u32::from_le_bytes(bytes.get(80..84)?.try_into().ok()?) as usize;
        (0..count)
            .filter_map(|i| {
                let base = 84 + i * 50 + 12;
                let f = |o: usize| f32::from_le_bytes(bytes[base + o..base + o + 4].try_into().unwrap()) as f64;
                bytes.get(base + 36)?;
                Some([[f(0), f(4), f(8)], [f(12), f(16), f(20)], [f(24), f(28), f(32)]])
            })
            .collect()
    };
    if triangles.is_empty() {
        return None;
    }
    Some(
        triangles
            .iter()
            .map(|[a, b, c]| {
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum(),
    )
}

fn ours(scad: &Path) -> Result<(f64, Vec<String>), String> {
    let src = c2f_eval::read_source(scad).map_err(|e| e.to_string())?;
    let program = c2f_eval::Program::load(&src, scad, &c2f_eval::default_library_paths()).map_err(|e| e.render())?;
    let output = c2f_eval::evaluate(&program);
    let rendered = c2f_geom::render_in(&output.nodes, program.files[0].path.parent().unwrap_or(Path::new("")));
    if rendered.is_2d {
        return Err("2D".into());
    }
    Ok((rendered.stats.volume, rendered.warnings))
}

/// The reference's volume, or `None` when it exports nothing (2D,
/// empty, slow).
fn oracle(scad: &Path, out: &Path) -> Option<f64> {
    let stl = out.join("o.stl");
    let _ = std::fs::remove_file(&stl);
    let mut child = Command::new(reference())
        .arg("-o")
        .arg(&stl)
        .arg(scad.file_name()?)
        .current_dir(scad.parent()?)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // CGAL renders can take minutes; give up after a while.
    for _ in 0..600 {
        if child.try_wait().ok()?.is_some() {
            return stl_volume(&std::fs::read(&stl).ok()?);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    None
}

#[test]
#[ignore = "needs the reference's desktop app; set C2F_REFERENCE to override its path"]
fn live_volumes() {
    let resources = Path::new(&reference()).parent().unwrap().join("../Resources");
    let mut scads = Vec::new();
    collect(&resources.join("examples"), &mut scads);
    collect(&resources.join("libraries"), &mut scads);
    let out = std::env::temp_dir().join(format!("c2f-geom-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let (mut compared, mut skipped, mut failures) = (0, Vec::new(), Vec::new());
    for scad in &scads {
        let name = scad.strip_prefix(&resources).unwrap_or(scad).display().to_string();
        let Some(expected) = oracle(scad, &out) else {
            skipped.push(format!("{name}: no STL from the reference"));
            continue;
        };
        match ours(scad) {
            Err(why) => failures.push(format!("{name}: ours failed ({why}); the reference's volume is {expected}")),
            Ok((actual, warnings)) => {
                compared += 1;
                let relative = (actual - expected).abs() / expected.abs().max(1e-9);
                if relative > TOLERANCE {
                    failures.push(format!(
                        "{name}: volume {actual:.6} vs the reference's {expected:.6} ({relative:.2e}){}",
                        if warnings.is_empty() { String::new() } else { format!(" warnings: {}", warnings.join("; ")) }
                    ));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&out);
    eprintln!("{compared} compared, {} skipped:\n  {}", skipped.len(), skipped.join("\n  "));
    assert!(failures.is_empty(), "{} files differ:\n  {}", failures.len(), failures.join("\n  "));
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "scad") {
            out.push(path);
        }
    }
}
