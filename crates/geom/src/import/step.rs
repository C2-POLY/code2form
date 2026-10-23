//! STEP (`.step`, `.stp`): the exact surfaces of a CAD model, which a CAD
//! kernel meshes. Code2Form has none of its own, so it asks FreeCAD's
//! command-line program (`freecadcmd`) when it is installed, and reads
//! back the STL it writes. The mesh is kept in a cache, keyed on the file,
//! its size and date and the fineness asked for, so that a design renders
//! again without FreeCAD.
//!
//! FreeCAD is found through `C2F_FREECAD`, then on the `PATH`, then where
//! its installers put it.

use std::path::{Path, PathBuf};
use std::process::Command;

use c2f_eval::node::Segments;

use crate::primitives::TriMesh;

/// The FreeCAD script: the file, the STL and the fineness come through
/// the environment, which needs no quoting.
const SCRIPT: &str = r#"import os, Part, MeshPart
shape = Part.Shape()
shape.read(os.environ["C2F_STEP_IN"])
linear = float(os.environ["C2F_STEP_LINEAR"]) or max(shape.BoundBox.DiagonalLength * 1e-3, 1e-3)
mesh = MeshPart.meshFromShape(Shape=shape, LinearDeflection=linear, AngularDeflection=float(os.environ["C2F_STEP_ANGULAR"]), Relative=False)
mesh.write(os.environ["C2F_STEP_OUT"])
print("C2F_OK", mesh.CountFacets)
"#;

pub(super) fn read(path: &Path, name: &str, segments: &Segments, warnings: &mut Vec<String>) -> Option<TriMesh> {
    let Ok(metadata) = std::fs::metadata(path) else {
        warnings.push(format!("Can't open import file '{name}'"));
        return None;
    };
    // The facets turn by at most `$fa`, or a `$fn`th of a turn; their
    // distance to the surface is FreeCAD's thousandth of the size.
    let angular = if segments.fn_ >= 3.0 { 360.0 / segments.fn_ } else { segments.fa.clamp(1.0, 45.0) }.to_radians();
    let linear = 0.0;
    let modified = metadata.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = format!(
        "{}|{}|{:?}|{}|{}",
        canonical.display(),
        metadata.len(),
        modified.map(|d| d.as_nanos()),
        angular.to_bits(),
        f64::to_bits(linear)
    );
    let cache = cache_dir().join(format!("{:016x}.stl", fnv(key.as_bytes())));
    if let Ok(data) = std::fs::read(&cache) {
        return super::stl::read(&data, name, warnings);
    }
    let Some(freecad) = freecad() else {
        warnings.push(format!(
            "import(\"{name}\"): reading STEP needs FreeCAD (freecad.org), which is not installed; or set C2F_FREECAD to its freecadcmd"
        ));
        return None;
    };
    let _ = std::fs::create_dir_all(cache_dir());
    let partial = cache.with_extension(format!("{}.stl", std::process::id()));
    let script = cache.with_extension(format!("{}.py", std::process::id()));
    if std::fs::write(&script, SCRIPT).is_err() {
        warnings.push(format!("import(\"{name}\"): can't write FreeCAD's script in {}", cache_dir().display()));
        return None;
    }
    let output = Command::new(&freecad)
        .arg(&script)
        .env("C2F_STEP_IN", &canonical)
        .env("C2F_STEP_OUT", &partial)
        .env("C2F_STEP_LINEAR", linear.to_string())
        .env("C2F_STEP_ANGULAR", angular.to_string())
        .output();
    let _ = std::fs::remove_file(&script);
    let ok = match &output {
        Ok(output) => String::from_utf8_lossy(&output.stdout).contains("C2F_OK"),
        Err(_) => false,
    };
    if !ok {
        let why = match &output {
            Ok(output) => {
                let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
                text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no message").trim().to_string()
            }
            Err(err) => err.to_string(),
        };
        let _ = std::fs::remove_file(&partial);
        warnings.push(format!("import(\"{name}\"): FreeCAD could not read the STEP: {why}"));
        return None;
    }
    let data = std::fs::read(&partial).ok()?;
    // Moved in place whole, so that another render never reads half.
    let _ = std::fs::rename(&partial, &cache);
    super::stl::read(&data, name, warnings)
}

/// FreeCAD's command-line program, if it is installed.
fn freecad() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("C2F_FREECAD") {
        return Some(PathBuf::from(path));
    }
    let names: &[&str] = if cfg!(windows) { &["FreeCADCmd.exe", "freecadcmd.exe"] } else { &["freecadcmd", "FreeCADCmd"] };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let mut places: Vec<PathBuf> = vec![
        PathBuf::from("/Applications/FreeCAD.app/Contents/Resources/bin/freecadcmd"),
        PathBuf::from("/usr/bin/freecadcmd"),
        PathBuf::from("/usr/local/bin/freecadcmd"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        places.push(PathBuf::from(home).join("Applications/FreeCAD.app/Contents/Resources/bin/freecadcmd"));
    }
    // Windows installs a version per folder: `FreeCAD 1.0`.
    for root in ["C:\\Program Files", "C:\\Program Files (x86)"] {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("FreeCAD") {
                    places.push(entry.path().join("bin").join("FreeCADCmd.exe"));
                }
            }
        }
    }
    places.into_iter().find(|p| p.is_file())
}

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("code2form-step")
}

/// FNV-1a, 64 bits: a short, steady name for a cache entry.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With FreeCAD (`C2F_FREECAD`), the STEP of a cube with a hole that
    /// Code2Form writes reads back as the same solid.
    #[test]
    fn a_step_reads_back_through_freecad() {
        if std::env::var_os("C2F_FREECAD").is_none() {
            return;
        }
        let src = "difference() { cube(10); translate([5, 5, -1]) cylinder(r = 2, h = 12, $fn = 8); }";
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).unwrap();
        let rendered = crate::render(&c2f_eval::evaluate(&program).nodes);
        let dir = std::env::temp_dir().join(format!("c2f-step-in-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("part.step"), rendered.step(&crate::StepOptions::default()).unwrap()).unwrap();
        let program = c2f_eval::Program::load("import(\"part.step\");", Path::new("t.scad"), &[]).unwrap();
        let back = crate::render_in(&c2f_eval::evaluate(&program).nodes, &dir);
        // The STL holds 32-bit floats.
        assert!((back.stats.volume - rendered.stats.volume).abs() < 1e-3, "{:?} {:?}", back.stats, back.warnings);
        // From the cache, the second time.
        let again = crate::render_in(&c2f_eval::evaluate(&program).nodes, &dir);
        assert_eq!(again.stats, back.stats);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cache_names_differ_with_the_key() {
        assert_ne!(fnv(b"a|1"), fnv(b"a|2"));
    }
}
