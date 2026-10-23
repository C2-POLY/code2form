//! `c2f obj`, `step`, `urdf`, `json` and `dxf --view`: the model for
//! other programs.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A folder of its own for each test.
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("c2f-exchange-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `c2f` with `args` on `src`, saved in `dir` as `design.c2f`.
fn c2f(dir: &std::path::Path, args: &[&str], src: &str) -> Output {
    let path = dir.join("design.c2f");
    std::fs::write(&path, src).unwrap();
    Command::new(env!("CARGO_BIN_EXE_c2f")).args(args).arg(&path).output().expect("running c2f")
}

const ARM: &str = "cube(10); translate([0, 0, 10]) part(\"elbow\", joint = \"revolute\", limits = [-90, 90]) cube([2, 2, 20]);";

#[test]
fn obj_goes_with_its_materials() {
    let dir = dir("obj");
    let target = dir.join("my arm.obj");
    let output = c2f(&dir, &["obj", "-o", target.to_str().unwrap()], ARM);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    // `mtllib` takes no spaces.
    assert!(std::fs::read_to_string(&target).unwrap().starts_with("# Code2Form\nmtllib my_arm.mtl\n"));
    assert!(dir.join("my_arm.mtl").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn step_and_json_print() {
    let dir = dir("print");
    let step = c2f(&dir, &["step"], ARM);
    assert!(String::from_utf8_lossy(&step.stdout).contains("MANIFOLD_SOLID_BREP"));
    let json = c2f(&dir, &["json"], "width = 4; // [1:10]\ncube(width);");
    let parts: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(parts["parameters"]["width"], 4.0);
    assert_eq!(parts["parts"][0]["name"], "design");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn urdf_needs_a_file_for_its_meshes() {
    let dir = dir("urdf");
    let output = c2f(&dir, &["urdf"], ARM);
    assert_eq!(output.status.code(), Some(2));
    let target = dir.join("arm.urdf");
    let output = c2f(&dir, &["urdf", "-o", target.to_str().unwrap()], ARM);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "2 links, 1 moving joints\n");
    // The root is named after the file written.
    assert!(dir.join("arm_meshes/arm.stl").exists() && dir.join("arm_meshes/elbow.stl").exists());
    // A file that cannot be written fails.
    let nowhere = dir.join("missing/arm.urdf");
    std::fs::write(dir.join("missing"), "a file, not a folder").unwrap();
    let output = c2f(&dir, &["urdf", "-o", nowhere.to_str().unwrap()], ARM);
    assert_eq!(output.status.code(), Some(1), "{}", String::from_utf8_lossy(&output.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dxf_of_a_solid_is_its_outline_from_a_view() {
    let dir = dir("dxf");
    let front = c2f(&dir, &["dxf", "--view", "front"], "cube([10, 20, 30]);");
    let text = String::from_utf8_lossy(&front.stdout);
    assert_eq!(text.matches("\nLINE\n").count(), 4, "{text}");
    assert!(text.contains("\n30.0\n"), "{text}");
    let angles = c2f(&dir, &["dxf", "--view", "-90,90"], "cube([10, 20, 30]);");
    assert!(String::from_utf8_lossy(&angles.stdout).contains("\n20.0\n"));
    let _ = std::fs::remove_dir_all(&dir);
}
