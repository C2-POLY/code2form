//! Every drawn triangle knows the statement that made it, through the
//! booleans, hulls and extrusions: what a click in the 3D view reads.

use std::collections::BTreeSet;
use std::path::Path;

struct Case {
    src: &'static str,
    output: c2f_eval::Output,
    rendered: c2f_geom::Rendered,
}

fn render(src: &'static str) -> Case {
    let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
    let output = c2f_eval::evaluate(&program);
    let rendered = c2f_geom::render(&output.nodes);
    Case { src, output, rendered }
}

impl Case {
    /// The statements the triangles of `mesh` come from.
    fn statements(&self, mesh: &c2f_geom::RenderMesh) -> BTreeSet<&'static str> {
        assert_eq!(mesh.origins.len(), mesh.triangle_count());
        (0..mesh.triangle_count())
            .map(|t| {
                let origin = mesh.origin(t).unwrap_or_else(|| panic!("triangle {t} has no origin"));
                &self.src[self.output.origins[origin as usize].span.range()]
            })
            .collect()
    }

    /// The statements of the triangles of the top face (`z` = `top`) that
    /// cover the point (`x`, `y`).
    fn at(&self, x: f32, y: f32, top: f32) -> BTreeSet<&'static str> {
        let mesh = &self.rendered.solid;
        (0..mesh.triangle_count())
            .filter(|&t| {
                let p = &mesh.positions[t * 3..t * 3 + 3];
                let side = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
                let s = [side(p[0], p[1]), side(p[1], p[2]), side(p[2], p[0])];
                p.iter().all(|c| (c[2] - top).abs() < 1e-4)
                    && (s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0))
            })
            .map(|t| &self.src[self.output.origins[mesh.origin(t).unwrap() as usize].span.range()])
            .collect()
    }
}

#[test]
fn union_keeps_each_part_apart() {
    let case = render("cube(10);\ntranslate([20, 0, 0]) sphere(3);");
    assert_eq!(case.statements(&case.rendered.solid), BTreeSet::from(["cube(10);", "sphere(3);"]));
}

#[test]
fn the_walls_of_a_hole_come_from_the_shape_that_cuts_it() {
    let case = render("difference() {\n    cube(10);\n    translate([5, 5, -1]) cylinder(h = 12, r = 2, $fn = 24);\n}");
    assert_eq!(
        case.statements(&case.rendered.solid),
        BTreeSet::from(["cube(10);", "cylinder(h = 12, r = 2, $fn = 24);"])
    );
    // The top face away from the hole is the cube's.
    assert_eq!(case.at(1.0, 1.0, 10.0), BTreeSet::from(["cube(10);"]));
    // The cutter itself is not drawn, but can be picked.
    assert_eq!(case.statements(&case.rendered.cutters), BTreeSet::from(["cylinder(h = 12, r = 2, $fn = 24);"]));
}

#[test]
fn shapes_made_without_properties_take_their_statement() {
    let case = render("hull() { cube(1); translate([5, 0, 0]) cube(1); }\ntranslate([0, 10, 0]) linear_extrude(3) circle(2);");
    assert_eq!(
        case.statements(&case.rendered.solid),
        BTreeSet::from(["hull() { cube(1); translate([5, 0, 0]) cube(1); }", "linear_extrude(3) circle(2);"])
    );
}

#[test]
fn backgrounds_and_highlights_keep_their_origins() {
    let case = render("%cube(2);\n#translate([5, 0, 0]) sphere(1);");
    assert_eq!(case.statements(&case.rendered.backgrounds), BTreeSet::from(["%cube(2);"]));
    // The `#` is on the translate, but the triangles are the sphere's.
    assert_eq!(case.statements(&case.rendered.highlights), BTreeSet::from(["sphere(1);"]));
}

/// The lowest and highest corner of `mesh`.
fn bounds(mesh: &c2f_geom::RenderMesh) -> ([f32; 3], [f32; 3]) {
    assert!(mesh.triangle_count() > 0, "nothing drawn");
    mesh.positions.iter().fold(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]), |(lo, hi), p| {
        (std::array::from_fn(|i| lo[i].min(p[i])), std::array::from_fn(|i| hi[i].max(p[i])))
    })
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn cutters_move_with_the_transforms_around_their_difference() {
    let case = render(
        "translate([50, 0, 0]) difference() {\n    cube(20);\n    translate([10, 10, -1]) cylinder(h = 30, r = 3, $fn = 4);\n}",
    );
    let (lo, hi) = bounds(&case.rendered.cutters);
    assert!(near(lo, [57.0, 7.0, -1.0]) && near(hi, [63.0, 13.0, 29.0]), "cutter at {lo:?}..{hi:?}");
}

#[test]
fn highlights_and_backgrounds_move_with_the_transforms_around_them() {
    let case = render("translate([50, 0, 0]) #cube(5);\nrotate([0, 0, 90]) %cube(2);\nresize([10, 0, 0], auto = true) translate([1, 0, 0]) #cube(1);");
    let (lo, hi) = bounds(&case.rendered.highlights);
    assert!(near(lo, [10.0, 0.0, 0.0]) && near(hi, [55.0, 10.0, 10.0]), "highlights at {lo:?}..{hi:?}");
    let (lo, hi) = bounds(&case.rendered.backgrounds);
    assert!(near(lo, [-2.0, 0.0, 0.0]) && near(hi, [0.0, 2.0, 2.0]), "background at {lo:?}..{hi:?}");
}

#[test]
fn the_shapes_of_a_flat_design_keep_their_statements() {
    let case = render("square(4);\ntranslate([4, 2]) circle(2);\ntranslate([10, 0]) square(1);");
    assert_eq!(
        case.statements(&case.rendered.solid),
        BTreeSet::from(["square(4);", "translate([4, 2]) circle(2);", "translate([10, 0]) square(1);"])
    );
}
