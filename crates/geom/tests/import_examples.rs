//! The examples bundled with the reference implementation (2021.01) that
//! read data files (DXF, STL, DAT, PNG), rendered and compared with the
//! volumes the reference gives them (binary STL exports). They are read
//! from the folder `C2F_REFERENCE_EXAMPLES` names, and skipped without it.

use std::path::{Path, PathBuf};

/// Example, and the volume (area for 2D) of its render by the reference
/// release (2021.01).
const CASES: &[(&str, f64)] = &[
    ("Old/example007.scad", 3439.969797),
    ("Old/example008.scad", 43145.833333),
    ("Old/example009.scad", 5723.980534),
    ("Old/example010.scad", 13750.540617),
    ("Old/example012.scad", 25589.133907),
    ("Old/example013.scad", 61333.333333),
    ("Old/example015.scad", 2390.644286),
    ("Old/example016.scad", 38359.167246),
    ("Basics/projection.scad", 5115.495070),
    ("Advanced/surface_image.scad", 4352.583508),
];

fn examples() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("C2F_REFERENCE_EXAMPLES").map(PathBuf::from).filter(|d| d.is_dir()) else {
        eprintln!("skipped: set C2F_REFERENCE_EXAMPLES to the examples folder of the reference release (2021.01)");
        return None;
    };
    Some(dir)
}

fn render(path: &Path) -> c2f_geom::Rendered {
    let src = c2f_eval::read_source(path).expect("readable example");
    let program = c2f_eval::Program::load(&src, path, &c2f_eval::default_library_paths()).expect("example loads");
    let output = c2f_eval::evaluate(&program);
    assert!(output.error.is_none(), "{}: {:?}", path.display(), output.error);
    c2f_geom::render_in(&output.nodes, path.parent().unwrap())
}

#[test]
fn bundled_examples_match_reference_volumes() {
    let Some(dir) = examples() else { return };
    let mut failures = Vec::new();
    for &(example, expected) in CASES {
        let rendered = render(&dir.join(example));
        let volume = rendered.stats.volume;
        let relative = (volume - expected).abs() / expected;
        // Projection's own outline differs slightly from CGAL's.
        let tolerance = if example.contains("projection") { 1e-6 } else { 3e-7 };
        println!("{example}: {volume:.6} (the reference's {expected:.6}, {relative:.1e}) {:?}", rendered.warnings);
        if relative > tolerance || !rendered.warnings.is_empty() {
            failures.push(format!("{example}: volume {volume} instead of {expected}, warnings {:?}", rendered.warnings));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn extrusions_read_dxf_files() {
    use c2f_eval::node::{FileSource, NodeKind, Segments};
    let Some(dir) = examples() else { return };
    let source = |layer: &str, origin: [f64; 2], scale: f64| {
        Some(FileSource { file: "example009.dxf".into(), layer: layer.into(), origin, scale, timestamp: 0 })
    };
    let segments = |fn_: f64| Segments { fn_, fa: 12.0, fs: 2.0 };
    // linear_extrude(file = "example009.dxf", layer = "body", height = 10, scale = 2):
    // the drawing is scaled by 2 too.
    let linear = NodeKind::LinearExtrude {
        source: source("body", [0.0, 0.0], 1.0),
        height: 10.0,
        center: false,
        convexity: 1,
        twist: 0.0,
        slices: None,
        scale: [2.0, 2.0],
        segments: segments(0.0),
    };
    // rotate_extrude(file = "example009.dxf", layer = "fan_side", origin = [0, -40], scale = 0.5, $fn = 30)
    let rotate = NodeKind::RotateExtrude { source: source("fan_side", [0.0, -40.0], 0.5), angle: 360.0, convexity: 1, segments: segments(30.0) };
    // Volumes from the reference release (2021.01).
    for (kind, expected) in [(linear, 28720.587343), (rotate, 2777.510548)] {
        let rendered = c2f_geom::render_in(&[c2f_eval::Node::new(kind, Vec::new())], &dir.join("Old"));
        assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);
        let relative = (rendered.stats.volume - expected).abs() / expected;
        assert!(relative < 1e-7, "volume {} instead of {expected}", rendered.stats.volume);
    }
}

#[test]
fn files_resolve_against_the_base_directory() {
    let Some(dir) = examples() else { return };
    let node = |file: &str| {
        c2f_eval::Node::new(
            c2f_eval::NodeKind::Surface { file: file.into(), center: true, invert: false, timestamp: 0 },
            Vec::new(),
        )
    };
    let old = dir.join("Old");
    let relative = c2f_geom::render_in(&[node("example010.dat")], &old);
    let absolute = c2f_geom::render(&[node(old.join("example010.dat").to_str().unwrap())]);
    assert!(relative.warnings.is_empty(), "{:?}", relative.warnings);
    assert!(relative.stats.volume > 0.0);
    assert_eq!(relative.stats.volume, absolute.stats.volume);
    // Not found from elsewhere: a warning and nothing else.
    let missing = c2f_geom::render_in(&[node("example010.dat")], &dir);
    assert_eq!(missing.stats.volume, 0.0);
    assert_eq!(missing.warnings.len(), 1);
}
