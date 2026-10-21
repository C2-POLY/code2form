//! `text()` against the reference implementation (2021.01).
//!
//! The expected numbers were measured on the macOS build of the reference
//! release (2021.01) with its bundled Liberation 2.00.1 fonts: each
//! snippet was exported with its `-o out.svg`, and the contours,
//! vertices, area (shoelace over the exported points) and bounds read
//! from the SVG path (y negated back). The SVG prints six significant
//! digits, hence the tolerances.
//!
//! The fonts are read from the reference's application bundle (or from
//! `OPENSCAD_FONT_PATH`); these tests fail if neither has them.

use std::path::Path;

use manifold_rust::cross_section::CrossSection;
use c2f_geom::Model;

fn render(src: &str) -> (Option<CrossSection>, Vec<String>) {
    let program = c2f_eval::Program::load(src, Path::new("text.scad"), &[]).unwrap_or_else(|_| panic!("{src} loads"));
    let output = c2f_eval::evaluate(&program);
    let rendered = c2f_geom::render(&output.nodes);
    assert!(
        !rendered.warnings.iter().any(|w| w.contains("nor Liberation Sans")),
        "the Liberation fonts were not found: install the reference release (2021.01) or set OPENSCAD_FONT_PATH"
    );
    match rendered.model {
        Model::Flat(section) => (Some(section), rendered.warnings),
        Model::Empty => (None, rendered.warnings),
        Model::Solid(_) => panic!("{src} is 2D"),
    }
}

/// Six significant digits, as the SVG export prints.
fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 6e-6 * expected.abs().max(1e-3)
}

struct Oracle {
    /// Vertices per contour, in any order.
    contours: &'static [usize],
    area: f64,
    /// `[min x, min y, max x, max y]`.
    bounds: [f64; 4],
}

fn check(src: &str, expected: Oracle) {
    let (section, warnings) = render(src);
    assert!(warnings.is_empty(), "{src}: {warnings:?}");
    let section = section.unwrap_or_else(|| panic!("{src} is empty"));
    let polygons = section.to_polygons();
    let mut counts: Vec<usize> = polygons.iter().map(Vec::len).collect();
    let mut want = expected.contours.to_vec();
    counts.sort_unstable();
    want.sort_unstable();
    assert_eq!(counts, want, "{src}: vertices per contour");
    // The oracle's area comes from its rounded points.
    let area = section.area();
    assert!((area - expected.area).abs() <= 5e-6 * expected.area, "{src}: area {area} vs {}", expected.area);
    let b = section.bounds();
    let bounds = [b.min.x, b.min.y, b.max.x, b.max.y];
    for (got, want) in bounds.iter().zip(expected.bounds) {
        assert!(close(*got, want), "{src}: bounds {bounds:?} vs {:?}", expected.bounds);
    }
}

#[test]
fn default_text() {
    check(
        r#"text("OpenSCAD");"#,
        Oracle {
            contours: &[44, 32, 130, 66, 8, 13, 23, 23, 43, 17, 60, 50, 36],
            area: 219.534134,
            bounds: [0.657791, -2.88199, 70.4331, 9.69719],
        },
    );
}

#[test]
fn centered_text() {
    check(
        r#"text("Hello", size=5, halign="center", valign="center");"#,
        Oracle { contours: &[4, 4, 12, 33, 13, 21, 24], area: 24.741505, bounds: [-7.15742, -2.49123, 7.52579, 2.60826] },
    );
}

#[test]
fn serif_italic() {
    check(
        r#"text("g", font="Liberation Serif:style=Italic");"#,
        Oracle { contours: &[69, 45], area: 22.977193, bounds: [0.00679016, -2.95659, 6.53709, 6.53709] },
    );
}

#[test]
fn fn_sets_curve_segments() {
    // $fn = 64: 64 / 8 + 1 = 9 segments per curve instead of 4.
    check(
        r#"text("OpenSCAD", $fn=64);"#,
        Oracle {
            contours: &[99, 72, 290, 146, 8, 22, 48, 48, 93, 37, 121, 105, 81],
            area: 220.315639,
            bounds: [0.657791, -2.88199, 70.4331, 9.69719],
        },
    );
}

#[test]
fn outline_points_are_font_units_on_freetypes_grid() {
    // Liberation Sans "H" is (168..1312) x (0..1409) font units, with its
    // bar at 653..813: at size 1000 the reference puts them here.
    let (section, _) = render(r#"text("H", size=1000);"#);
    let polygons = section.expect("an H").to_polygons();
    assert_eq!(polygons.len(), 1);
    let mut xs: Vec<f64> = polygons[0].iter().map(|p| p.x).collect();
    let mut ys: Vec<f64> = polygons[0].iter().map(|p| p.y).collect();
    for v in [&mut xs, &mut ys] {
        v.sort_by(f64::total_cmp);
        v.dedup();
    }
    let expected_x = [113.93, 243.45, 760.18, 889.7];
    let expected_y = [0.0, 442.82, 551.32, 955.48];
    assert_eq!(xs.len(), 4);
    assert_eq!(ys.len(), 4);
    for (got, want) in xs.iter().chain(&ys).zip(expected_x.iter().chain(&expected_y)) {
        assert!(close(*got, *want), "{xs:?} {ys:?}");
    }
}

#[test]
fn kerning_and_alignment() {
    // V moves 152 font units left of A's advance.
    check(r#"text("AV", size=1000);"#, Oracle { contours: &[8, 13, 9], area: 512404.550152, bounds: [2.70999, 0.0, 1724.22, 955.48] });
    check(
        r#"text("Hg", size=1000, halign="right", valign="top");"#,
        Oracle { contours: &[12, 74, 40], area: 612862.881538, bounds: [-1619.9, -1221.32, -71.4562, 22.355] },
    );
    check(
        r#"text("Hg", size=1000, halign="center", valign="bottom");"#,
        Oracle { contours: &[12, 74, 40], area: 612859.730538, bounds: [-752.984, -6.325, 795.458, 1237.35] },
    );
}

#[test]
fn composite_glyph_offsets_are_not_grid_rounded() {
    // The grave sits 334 font units up: 22649.375 26.6 units, not rounded
    // to the 64-unit pixel grid (that would put its top at 7.14917).
    check(
        r#"text("ÀÉ", size=6, font="Liberation Serif:style=Bold");"#,
        Oracle { contours: &[6, 6, 16, 3, 30], area: 26.134733, bounds: [0.0813599, 0.0, 11.0077, 7.14876] },
    );
}

#[test]
fn vertical_text() {
    check(
        r#"text("HH", size=1000, direction="ttb");"#,
        Oracle { contours: &[12, 12], area: 607146.524560, bounds: [-375.816, -2375.58, 399.954, -147.206] },
    );
}

#[test]
fn empty_texts_and_unknown_fonts() {
    for src in [r#"text("");"#, r#"text("   ");"#, r#"text("Hg", size=0);"#] {
        let (section, warnings) = render(src);
        assert!(section.is_none() && warnings.is_empty(), "{src}");
    }
    // The reference's fontconfig falls back to Liberation Sans, silently.
    let (fallback, warnings) = render(r#"text("Hg", font="No Such Font");"#);
    let (default, _) = render(r#"text("Hg");"#);
    assert_eq!(fallback.map(|s| s.to_polygons()), default.map(|s| s.to_polygons()));
    assert_eq!(warnings, ["Can't find font \"No Such Font\", using Liberation Sans"]);
}
