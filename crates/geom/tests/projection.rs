//! `projection()` without `cut` on solids with holes, against the
//! reference implementation (2021.01).
//!
//! The expected numbers were measured on the macOS build of the reference
//! release (2021.01): each snippet was exported with its `-o out.svg`, and
//! the contours and area (shoelace over the exported points) read from
//! the SVG path. The SVG prints six significant digits, hence the
//! tolerance. The GEB case needs the Liberation fonts, as `tests/text.rs`
//! does.
//!
//! The reference projects each facet and unites them, which is the
//! solid's shadow: a hole stays open only where it goes through the solid
//! along Z, as it does in all three cases. GEB itself is left out: see
//! `live_volumes` in `tests/oracle.rs`.

use std::path::Path;

use c2f_geom::Model;

/// The letters of the reference's `examples/Advanced/GEB.scad`.
const GEB_LETTERS: &str = r#"
font = "Liberation Sans";
module G() offset(0.3) text("G", size=10, halign="center", valign="center", font = font);
module E() offset(0.3) text("E", size=10, halign="center", valign="center", font = font);
module B() offset(0.5) text("B", size=10, halign="center", valign="center", font = font);
$fn=64;
"#;

struct Oracle {
    src: &'static str,
    /// Whether `src` uses GEB's letters (and their `$fn`).
    letters: bool,
    /// Contours: the outline and its holes.
    contours: usize,
    area: f64,
}

const CASES: [Oracle; 3] = [
    // A through-hole of a CGAL difference.
    Oracle {
        src: "projection() difference() { cube(10, center=true); cylinder(r=2, h=20, center=true); }",
        letters: false,
        contours: 2,
        area: 89.0544391,
    },
    // A square frame cut by a cylinder lying along Y.
    Oracle {
        src: "projection() intersection() {
                linear_extrude(height=20, center=true) difference() { square(10, center=true); square(4, center=true); }
                rotate([90,0,0]) cylinder(r=6, h=20, center=true);
              }",
        letters: false,
        contours: 2,
        area: 84.0,
    },
    // GEB's intersection with its children in B, G, E order: B's two holes
    // stay open.
    Oracle {
        src: "projection() intersection() {
                linear_extrude(height = 20, convexity = 3, center=true) B();
                rotate([90, 0, 90]) linear_extrude(height = 20, convexity = 3, center=true) G();
                rotate([90, 0, 0]) linear_extrude(height = 20, convexity = 3, center=true) E();
              }",
        letters: true,
        contours: 3,
        area: 58.1693781,
    },
];

#[test]
fn projection_keeps_through_holes_open() {
    for case in &CASES {
        let src = format!("{}{}", if case.letters { GEB_LETTERS } else { "" }, case.src);
        let program = c2f_eval::Program::load(&src, Path::new("projection.scad"), &[]).expect("parses");
        let rendered = c2f_geom::render(&c2f_eval::evaluate(&program).nodes);
        assert!(rendered.warnings.is_empty(), "{}: {:?}", case.src, rendered.warnings);
        let Model::Flat(section) = rendered.model else { panic!("{} is not 2D", case.src) };
        assert_eq!(section.num_contour(), case.contours, "{}: contours", case.src);
        let area = section.area();
        assert!((area - case.area).abs() <= 5e-6 * case.area, "{}: area {area} vs {}", case.src, case.area);
    }
}

#[test]
fn the_same_projection_twice_is_one_outline_used_twice() {
    // A fence: the outline grown by 2, less the outline grown by 1. The
    // second projection reuses the first, which a `difference` builds in
    // another context; a marked solid is still drawn highlighted.
    let render = |src: &str| {
        let program = c2f_eval::Program::load(src, Path::new("fence.scad"), &[]).expect("parses");
        c2f_geom::render(&c2f_eval::evaluate(&program).nodes)
    };
    let fence = "difference() { offset(delta = 2) projection() cube([10, 10, 5]); \
                 offset(delta = 1) projection() cube([10, 10, 5]); }";
    let Model::Flat(section) = render(fence).model else { panic!("not 2D") };
    assert!((section.area() - (14.0 * 14.0 - 12.0 * 12.0)).abs() < 1e-9, "{}", section.area());
    let marked = "projection() #cube(4); translate([10, 0]) projection() #cube(4);";
    assert!(!render(marked).highlights.is_empty());
}
