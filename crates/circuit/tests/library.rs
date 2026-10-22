//! The electronics library in `library/electronics`: every part renders
//! cleanly, with and without its headers, and declares pins, mount holes
//! and ports that lie on its geometry.

use std::path::{Path, PathBuf};

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

/// The parts: the `.c2f` files of `library/electronics` itself, not of the
/// helper folders under it.
fn parts() -> Vec<PathBuf> {
    let mut parts: Vec<PathBuf> = std::fs::read_dir(library().join("electronics"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "c2f"))
        .collect();
    parts.sort();
    parts
}

/// The bounds of a part's geometry with `defines` set, after checking it
/// evaluates and renders without an error or a warning.
fn bounds(path: &Path, src: &str, defines: &[&str]) -> ([f64; 3], [f64; 3]) {
    let mut src = src.to_string();
    if !defines.is_empty() {
        src.push_str("\n\x03\n");
        for define in defines {
            src.push_str(define);
            src.push_str(";\n");
        }
    }
    let name = format!("{} {defines:?}", path.file_name().unwrap().to_string_lossy());
    let program = c2f_eval::Program::load(&src, path, &[]).unwrap_or_else(|err| panic!("{}", err.render()));
    let output = c2f_eval::evaluate(&program);
    assert_eq!(output.error, None, "{name}");
    assert!(output.warnings.is_empty(), "{name}: {:?}", output.warnings);
    let rendered = c2f_geom::render_in(&output.nodes, path.parent().unwrap());
    assert!(rendered.warnings.is_empty(), "{name}: {:?}", rendered.warnings);
    assert!(rendered.stats.volume > 0.0, "{name} has no volume");
    rendered.bounds().unwrap()
}

#[test]
fn the_library_has_the_grogu_parts() {
    let names: Vec<String> =
        parts().iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect();
    for part in [
        "xiao_esp32s3",
        "esp32s3_devkitc1",
        "drv8833",
        "max98357a",
        "inmp441",
        "capacitor_radial",
        "capacitor_ceramic",
        "speaker",
        "dc_motor",
        "touch_plate",
    ] {
        assert!(names.iter().any(|name| name == part), "{part} is missing from {names:?}");
    }
}

#[test]
fn every_part_renders_and_its_annotations_lie_on_it() {
    const TOLERANCE: f64 = 0.5;
    for path in parts() {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (annotations, errors) = c2f_lang::annotations::read(&src);
        assert!(errors.is_empty(), "{name}: {:?}", errors.iter().map(|e| e.render(&src, &name)).collect::<Vec<_>>());
        assert!(!annotations.pins.is_empty(), "{name} declares no pin");
        // The header comment links to the manufacturer's documentation.
        assert!(src.lines().take(8).any(|line| line.starts_with("// https://")), "{name} has no link");
        let mut variants: Vec<Vec<&str>> = vec![vec![]];
        if src.contains("\nheaders = ") {
            variants.push(vec!["headers=true", "header_side=\"down\""]);
            variants.push(vec!["headers=true", "header_side=\"up\""]);
        }
        for defines in variants {
            let (min, max) = bounds(&path, &src, &defines);
            let points = annotations
                .pins
                .iter()
                .map(|pin| (format!("pin {}", pin.name), pin.position))
                .chain(annotations.mount_holes.iter().map(|hole| ("a mount hole".to_string(), hole.position)))
                .chain(annotations.ports.iter().map(|port| (format!("port {}", port.name), port.position)));
            for (what, at) in points {
                let inside = (0..3).all(|k| at[k] >= min[k] - TOLERANCE && at[k] <= max[k] + TOLERANCE);
                assert!(inside, "{name} {defines:?}: {what} at {at:?} is off the part, {min:?} to {max:?}");
            }
        }
    }
}

#[test]
fn the_esp32_s3_strapping_pins_are_reserved() {
    let reserved = |file: &str| {
        let src = std::fs::read_to_string(library().join("electronics").join(file)).unwrap();
        let (annotations, _) = c2f_lang::annotations::read(&src);
        annotations.pins.into_iter().filter(|pin| pin.reserved).map(|pin| pin.name).collect::<Vec<_>>()
    };
    assert_eq!(reserved("esp32s3_devkitc1.c2f"), ["3", "46", "0", "45"]);
    // GPIO0, GPIO45 and GPIO46 are not brought out on the XIAO.
    assert_eq!(reserved("xiao_esp32s3.c2f"), ["D2"]);
}

#[test]
fn parts_are_used_from_the_library_folders() {
    let src = "use <electronics/xiao_esp32s3.c2f>\nxiao_esp32s3(headers = true);\n";
    let dir = std::env::temp_dir().join(format!("c2f-library-{}", std::process::id()));
    let program = c2f_eval::Program::load(src, &dir.join("main.c2f"), &[library()]).unwrap();
    let output = c2f_eval::evaluate(&program);
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    let rendered = c2f_geom::render_in(&output.nodes, &dir);
    let (min, max) = rendered.bounds().unwrap();
    // The board, its USB-C 1.5 mm over its edge, with its headers' pins
    // under it; the board sits 0.04 mm up, on its pads.
    assert!((max[0] - 17.8).abs() < 1e-6 && (max[1] - 22.5).abs() < 1e-6, "{max:?}");
    assert!((min[2] + 8.46).abs() < 1e-6, "{min:?}");
    // The repository's library is on the default path, for `cargo run`.
    let library = library().canonicalize().unwrap();
    assert!(c2f_eval::default_library_paths().iter().any(|p| p.canonicalize().is_ok_and(|p| p == library)));
}
