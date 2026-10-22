//! `c2f glb`: the model as glTF, for game engines.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Runs `c2f glb` with `args` on `src`, saved to a file of its own.
fn c2f(args: &[&str], name: &str, src: &str) -> Output {
    let path: PathBuf = std::env::temp_dir().join(format!("c2f-gltf-{}-{name}.scad", std::process::id()));
    std::fs::write(&path, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_c2f")).arg("glb").args(args).arg(&path).output().expect("running c2f");
    let _ = std::fs::remove_file(&path);
    output
}

/// The JSON chunk of a `.glb`.
fn json(glb: &[u8]) -> serde_json::Value {
    assert_eq!(&glb[..4], b"glTF");
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&glb[20..20 + length]).unwrap()
}

/// The largest height (glTF's Y) of the first mesh's first primitive.
fn top(gltf: &serde_json::Value) -> f64 {
    let position = gltf["meshes"][0]["primitives"][0]["attributes"]["POSITION"].as_u64().unwrap() as usize;
    gltf["accessors"][position]["max"][1].as_f64().unwrap()
}

#[test]
fn glb_is_in_meters_and_named_after_the_file() {
    let output = c2f(&[], "box", "cube([10, 10, 20]);");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gltf = json(&output.stdout);
    assert!((top(&gltf) - 0.02).abs() < 1e-6, "{gltf}");
    assert!(gltf["nodes"][0]["name"].as_str().unwrap().ends_with("-box"));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "12 triangles, 24 vertices\n");
}

#[test]
fn scale_sets_meters_per_unit() {
    let output = c2f(&["--scale", "1"], "unit", "cube([10, 10, 20]);");
    assert!(output.status.success());
    assert_eq!(top(&json(&output.stdout)), 20.0);
    for bad in ["0", "-1", "x"] {
        assert_eq!(c2f(&["--scale", bad], "bad", "cube(1);").status.code(), Some(2), "--scale {bad}");
    }
}

#[test]
fn a_flat_model_writes_nothing() {
    let output = c2f(&[], "flat", "square(10);");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("2D"));
}

const TURRET: &str = "cube([40, 40, 5]); translate([20, 20, 5]) rotate([0, 0, 360 * $t]) part(\"turret\") cylinder(h = 10, r = 12);";

#[test]
fn a_model_that_reads_t_is_animated() {
    let output = c2f(&[], "turret", TURRET);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gltf = json(&output.stdout);
    assert!(gltf["animations"][0]["name"].as_str().unwrap().ends_with("-turret-loop"), "{}", gltf["animations"]);
    assert!(String::from_utf8_lossy(&output.stderr).ends_with(", 61 frames\n"));

    let output = c2f(&["--duration", "1", "--fps", "10", "--animation", "spin", "--no-loop"], "turret", TURRET);
    let gltf = json(&output.stdout);
    assert_eq!(gltf["animations"][0]["name"], "spin");
    assert!(String::from_utf8_lossy(&output.stderr).ends_with(", 11 frames\n"));

    let output = c2f(&["--still"], "turret", TURRET);
    assert!(json(&output.stdout).get("animations").is_none());
    assert_eq!(c2f(&["--fps", "0"], "bad", TURRET).status.code(), Some(2));
}

#[test]
fn an_error_writes_no_gltf() {
    let output = c2f(&[], "error", "assert(false); cube(1);");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("ERROR: Assertion"), "{stderr}");
}
