//! From sketch to code and back: the generated code evaluates to the
//! profile, follows its dimensions, extrudes and revolves as expected,
//! and reopens as the sketch it came from.

use std::path::Path;

use c2f_sketch::{
    Constraint, Dimension, EntityId, ExtrudeMode, Plane, Point, PointId, Sketch, SnapKind, extrude_code, from_code,
    profile, revolve_code, sketches_in, snap, solve, to_code, update_source,
};

/// Renders a design: its area in 2D or volume in 3D, and its bounds.
fn render(src: &str) -> (f64, [f64; 3], [f64; 3]) {
    let program = c2f_eval::Program::load(src, Path::new("t.c2f"), &[]).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    let output = c2f_eval::evaluate(&program);
    assert!(output.warnings.is_empty(), "{:?}\n{src}", output.warnings);
    let rendered = c2f_geom::render(&output.nodes);
    let (lo, hi) = rendered.bounds().unwrap_or_else(|| panic!("empty model\n{src}"));
    (rendered.stats.volume, lo, hi)
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// An L-shaped bracket profile, drawn roughly: 40 wide, 30 high, 5 thick.
///
/// ```text
/// p5 +--+ p4
///    |  |
///    |  + p3 ------+ p2
///    |             |
/// p0 +-------------+ p1
/// ```
fn l_bracket() -> Sketch {
    let mut sketch = Sketch::new(Plane::Xy);
    let corners = [(0.0, 0.0), (38.0, 0.0), (38.0, 6.0), (6.0, 6.0), (6.0, 28.0), (0.0, 28.0)];
    let lines = sketch.add_polyline(&corners.map(|(x, y)| Point::new(x, y)), true);
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    for (i, &line) in lines.iter().enumerate() {
        sketch.add_constraint(if i % 2 == 0 { Constraint::Horizontal(line) } else { Constraint::Vertical(line) });
    }
    sketch.add_constraint(Constraint::Length(lines[0], Dimension::named("l_bracket_width", 40.0)));
    sketch.add_constraint(Constraint::Length(lines[5], Dimension::named("l_bracket_height", 30.0)));
    sketch.add_constraint(Constraint::Length(lines[1], Dimension::named("l_bracket_thickness", 5.0)));
    sketch.add_constraint(Constraint::Length(lines[4], Dimension::named("l_bracket_thickness", 5.0)));
    sketch
}

/// The L-bracket's area for a width, height and thickness.
fn bracket_area(w: f64, h: f64, t: f64) -> f64 {
    w * t + t * (h - t)
}

#[test]
fn an_l_bracket_from_sketch_to_solid() {
    let mut sketch = l_bracket();
    let report = solve(&mut sketch);
    assert!(report.is_fully_constrained(), "{report:?}");
    assert!((profile(&sketch, 64).area() - bracket_area(40.0, 30.0, 5.0)).abs() < 1e-9);

    let code = to_code(&sketch, "l_bracket");
    // Dimensions as Customizer parameters, points as their expressions.
    assert!(code.contains("l_bracket_width = 40; // 0.1\n"), "{code}");
    assert!(code.contains("l_bracket_thickness = 5; // 0.1\n"), "{code}");
    assert_eq!(code.matches("l_bracket_thickness =").count(), 1, "one parameter per variable");
    assert!(code.contains("module sketch_l_bracket() {"), "{code}");
    for point in ["[0, 0]", "[l_bracket_width, 0]", "[l_bracket_width, l_bracket_thickness]"] {
        assert!(code.contains(point), "{point} missing from\n{code}");
    }
    assert!(code.contains("// @sketch length l0 = l_bracket_width 40"), "{code}");

    let program = c2f_eval::Program::load(&code, Path::new("t.c2f"), &[]).unwrap();
    let parameters: Vec<String> = program.parameters(&code).into_iter().map(|p| p.name).collect();
    assert_eq!(parameters, ["l_bracket_width", "l_bracket_height", "l_bracket_thickness"]);

    let (area, lo, hi) = render(&format!("{code}sketch_l_bracket();\n"));
    assert!(near(area, 325.0, 1e-6), "{area}");
    assert_eq!((lo[0], lo[1], hi[0], hi[1]), (0.0, 0.0, 40.0, 30.0));

    let extrusion = extrude_code("l_bracket", &sketch.plane, &Dimension::named("l_bracket_depth", 12.0), &ExtrudeMode::Add);
    let (volume, _, hi) = render(&format!("{}{code}{}", extrusion.parameters[0].text(), extrusion.body));
    assert!(near(volume, 325.0 * 12.0, 1e-6), "{volume}");
    assert_eq!(hi, [40.0, 30.0, 12.0]);
}

#[test]
fn the_profile_follows_a_changed_dimension() {
    let mut sketch = l_bracket();
    solve(&mut sketch);
    let code = to_code(&sketch, "l_bracket");

    // In the sketch: set the variable and solve again.
    let mut wider = sketch.clone();
    assert!(wider.set_variable("l_bracket_width", 60.0));
    assert!(solve(&mut wider).is_fully_constrained());
    assert!((profile(&wider, 64).area() - bracket_area(60.0, 30.0, 5.0)).abs() < 1e-9);
    assert_eq!(wider.point(PointId(2)), Point::new(60.0, 5.0));
    // Both thickness dimensions share their variable.
    assert!(wider.set_variable("l_bracket_thickness", 8.0));
    assert!(solve(&mut wider).converged());
    assert!((profile(&wider, 64).area() - bracket_area(60.0, 30.0, 8.0)).abs() < 1e-9);

    // In the code, as the Customizer does: the polygon follows at once.
    let edited = code.replace("l_bracket_width = 40;", "l_bracket_width = 60;");
    let (area, _, hi) = render(&format!("{edited}sketch_l_bracket();\n"));
    assert!(near(area, bracket_area(60.0, 30.0, 5.0), 1e-6), "{area}");
    assert_eq!(hi[0], 60.0);
    let mut program = c2f_eval::Program::load(&code, Path::new("t.c2f"), &[]).unwrap();
    program.set_parameter("l_bracket_thickness", &c2f_eval::customizer::Literal::Number(8.0));
    let output = c2f_eval::evaluate(&program);
    // `sketch_l_bracket()` is not called in `code`: call it.
    assert!(output.nodes.is_empty());

    // Reopened, the sketch takes the code's value and solves to the same.
    let mut reopened = from_code(&edited, "sketch_l_bracket").unwrap();
    assert_eq!(reopened.variables()[0], ("l_bracket_width".to_string(), 60.0));
    assert!(solve(&mut reopened).is_fully_constrained());
    let mut wider = sketch.clone();
    wider.set_variable("l_bracket_width", 60.0);
    solve(&mut wider);
    assert_eq!(to_code(&reopened, "l_bracket"), to_code(&wider, "l_bracket"));
}

#[test]
fn a_sketch_reopens_as_it_was_written() {
    let mut sketch = l_bracket();
    // Some of everything the format writes.
    let axis = sketch.add_polyline(&[Point::new(-10.0, -5.0), Point::new(-10.0, 40.0)], false)[0];
    sketch.set_construction(axis, true);
    let center = sketch.add_point(20.0, 18.0);
    let hole = sketch.add_circle(center, 3.0);
    let (s, e) = (sketch.add_point(25.0, 18.0), sketch.add_point(20.0, 23.0));
    let arc = sketch.add_arc(center, s, e);
    sketch.set_construction(arc, true);
    sketch.add_constraint(Constraint::Vertical(axis));
    sketch.add_constraint(Constraint::Diameter(hole, Dimension::named("hole_d", 6.0)));
    sketch.add_constraint(Constraint::Distance(PointId(0), center, Dimension::new(26.9072)));
    sketch.add_constraint(Constraint::Equal(hole, arc));
    sketch.add_constraint(Constraint::PointOnLine(center, EntityId(0)));
    sketch.constraints.pop();
    sketch.add_constraint(Constraint::Angle(EntityId(0), axis, Dimension::new(90.0)));
    sketch.plane = Plane::Face { origin: [1.0, 2.0, 3.0], x_axis: [1.0, 0.0, 0.0], normal: [0.0, -1.0, 0.0] };
    solve(&mut sketch);

    let code = to_code(&sketch, "l_bracket");
    let reopened = from_code(&code, "l_bracket").unwrap();
    assert_eq!(reopened.plane, sketch.plane);
    assert_eq!(reopened.entities, sketch.entities);
    assert_eq!(reopened.constraints, sketch.constraints);
    for (a, b) in reopened.points.iter().zip(&sketch.points) {
        assert!(a.distance(*b) < 1e-8);
    }
    assert_eq!(to_code(&reopened, "l_bracket"), code);
    assert_eq!(sketches_in(&code), ["sketch_l_bracket"]);

    // A damaged line is an error, not half a sketch.
    let damaged = code.replace("// @sketch horizontal l0", "// @sketch horizontal l99");
    let error = from_code(&damaged, "l_bracket").unwrap_err();
    assert!(error.0.contains("no entity `l99`"), "{error}");
    assert!(from_code(&code, "nothing").is_err());
}

#[test]
fn a_profile_with_a_hole() {
    let mut sketch = Sketch::new(Plane::Xy);
    let [bottom, right, ..] = sketch.add_rectangle(Point::new(0.0, 0.0), Point::new(30.0, 20.0));
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    sketch.add_constraint(Constraint::Length(bottom, Dimension::named("plate_w", 30.0)));
    sketch.add_constraint(Constraint::Length(right, Dimension::named("plate_h", 20.0)));
    // A hole in the middle, on a construction diagonal.
    let diagonal = sketch.add_line(PointId(0), PointId(2));
    sketch.set_construction(diagonal, true);
    let center = sketch.add_point(14.0, 11.0);
    sketch.add_constraint(Constraint::Midpoint(center, diagonal));
    let hole = sketch.add_circle(center, 4.0);
    sketch.add_constraint(Constraint::Radius(hole, Dimension::named("plate_r", 4.0)));
    // A rounded slot: two half circles and two lines.
    let p = [(5.0, 3.0), (25.0, 3.0), (25.0, 5.0), (5.0, 5.0), (25.0, 4.0), (5.0, 4.0)].map(|(x, y)| sketch.add_point(x, y));
    let low = sketch.add_line(p[0], p[1]);
    sketch.add_arc(p[4], p[1], p[2]);
    let high = sketch.add_line(p[2], p[3]);
    sketch.add_arc(p[5], p[3], p[0]);
    sketch.add_constraint(Constraint::Horizontal(low));
    sketch.add_constraint(Constraint::Horizontal(high));
    let report = solve(&mut sketch);
    assert!(report.converged());

    let shape = profile(&sketch, 64);
    assert!(shape.open.is_empty());
    assert_eq!(shape.loops.len(), 3);
    assert!(!shape.loops[0].is_hole() && shape.loops[1].is_hole() && shape.loops[2].is_hole());
    assert!(shape.loops[0].signed_area() > 0.0 && shape.loops[1].signed_area() < 0.0);
    let polygon = |r: f64, n: f64| n / 2.0 * r * r * (std::f64::consts::TAU / n).sin();
    let expected = 600.0 - polygon(4.0, 64.0) - (20.0 * 2.0 + polygon(1.0, 64.0));
    assert!((shape.area() - expected).abs() < 1e-9, "{} {expected}", shape.area());

    let code = to_code(&sketch, "plate");
    assert!(code.contains("paths = ["), "{code}");
    // The circle's points follow its radius and the plate's size.
    assert!(code.contains("plate_w / 2 + plate_r"), "{code}");
    let (area, _, _) = render(&format!("{code}sketch_plate();\n"));
    assert!(near(area, expected, 1e-6), "{area} {expected}");
}

#[test]
fn an_angle_applies_when_the_sketch_is_reopened() {
    // A wedge: its slanted side at an angle cannot follow in the code.
    let mut sketch = Sketch::new(Plane::Xy);
    let lines = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(0.0, 11.0)], true);
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    sketch.add_constraint(Constraint::Horizontal(lines[0]));
    sketch.add_constraint(Constraint::Vertical(lines[2]));
    sketch.add_constraint(Constraint::Length(lines[0], Dimension::named("wedge_base", 20.0)));
    sketch.add_constraint(Constraint::Angle(lines[0], lines[1], Dimension::named("wedge_angle", 150.0)));
    assert!(solve(&mut sketch).is_fully_constrained());
    let code = to_code(&sketch, "wedge");
    assert!(code.contains("// Sketch wedge: angle in degrees, applied when the sketch is reopened\n"), "{code}");
    assert!(code.contains("// Sketch wedge: length\nwedge_base = 20;"), "{code}");
    // The base follows; the height, set by the angle, is a number
    // proportional to the base.
    assert!(code.contains("[wedge_base, 0]"), "{code}");
    assert!(code.contains("[0, 0.577350269 * wedge_base]"), "{code}");
}

#[test]
fn extrusions_and_revolutions_land_where_the_sketch_is() {
    let mut sketch = Sketch::new(Plane::Xz);
    let [bottom, right, ..] = sketch.add_rectangle(Point::new(0.0, 0.0), Point::new(10.0, 4.0));
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    sketch.add_constraint(Constraint::Length(bottom, 10.0.into()));
    sketch.add_constraint(Constraint::Length(right, 4.0.into()));
    // A construction axis along x, 5 below the rectangle.
    let axis = sketch.add_polyline(&[Point::new(-3.0, -5.0), Point::new(12.0, -5.0)], false)[0];
    sketch.set_construction(axis, true);
    assert!(solve(&mut sketch).converged());
    let code = to_code(&sketch, "block");

    // On the XZ plane: x stays, y goes up as z, extrusions go toward -y.
    let up = extrude_code("block", &sketch.plane, &Dimension::new(3.0), &ExtrudeMode::Add);
    let (volume, lo, hi) = render(&format!("{code}{}", up.body));
    assert!(near(volume, 120.0, 1e-6));
    assert_eq!((lo, hi), ([0.0, -3.0, 0.0], [10.0, 0.0, 4.0]));
    let down = extrude_code("block", &sketch.plane, &Dimension::new(-3.0), &ExtrudeMode::Add);
    let (_, lo, hi) = render(&format!("{code}{}", down.body));
    assert_eq!((lo, hi), ([0.0, 0.0, 0.0], [10.0, 3.0, 4.0]));

    // Cut through a slab.
    let slab = "translate([-5, -2, -5]) cube([20, 1, 20]);".to_string();
    let cut = extrude_code("block", &sketch.plane, &Dimension::new(3.0), &ExtrudeMode::Subtract { from: slab });
    let (volume, _, _) = render(&format!("{code}{}", cut.body));
    assert!(near(volume, 400.0 - 40.0, 1e-6), "{volume}");

    // Around the axis: a tube 10 long, from radius 5 to 9, around the
    // line y = -5, z = 0 of the XZ plane... which is z = -5 in space.
    let tube = revolve_code(&sketch, "block", axis, &Dimension::new(360.0), &ExtrudeMode::Add).unwrap();
    let (volume, lo, hi) = render(&format!("$fn = 96;\n{code}{}", tube.body));
    let exact = std::f64::consts::PI * (81.0 - 25.0) * 10.0;
    assert!(near(volume, exact, 0.005 * exact), "{volume} {exact}\n{}", tube.body);
    for (got, want) in lo.iter().chain(&hi).zip([0.0, -9.0, -14.0, 10.0, 9.0, 4.0]) {
        assert!(near(*got, want, 0.1), "bounds {lo:?} {hi:?}\n{}", tube.body);
    }

    // The profile must lie on one side of the axis.
    let first = sketch.entity_points(axis)[0].0;
    sketch.points[first] = Point::new(-3.0, 2.0);
    sketch.points[first + 1] = Point::new(12.0, 2.0);
    assert!(revolve_code(&sketch, "block", axis, &Dimension::new(360.0), &ExtrudeMode::Add).is_err());
}

#[test]
fn a_revolution_around_a_slanted_axis_on_the_left() {
    // A right triangle against an axis at 45°, on its left: a cone.
    let mut sketch = Sketch::new(Plane::Xy);
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let (base, tip, rim) = (Point::new(2.0, 1.0), Point::new(2.0 + 10.0 * s, 1.0 + 10.0 * s), Point::new(2.0 - 4.0 * s, 1.0 + 4.0 * s));
    sketch.add_polyline(&[base, tip, rim], true);
    let axis = sketch.add_polyline(&[base, tip], false)[0];
    sketch.set_construction(axis, true);
    let code = to_code(&sketch, "cone");
    let cone = revolve_code(&sketch, "cone", axis, &Dimension::named("cone_turn", 180.0), &ExtrudeMode::Add).unwrap();
    assert!(cone.body.contains("mirror([1, 0, 0])"), "{}", cone.body);
    let (volume, _, _) = render(&format!("$fn = 96;\n{}{code}{}", cone.parameters[0].text(), cone.body));
    let exact = std::f64::consts::PI * 16.0 * 10.0 / 3.0 / 2.0;
    assert!(near(volume, exact, 0.005 * exact), "{volume} {exact}");
}

#[test]
fn update_source_writes_the_sketch_into_a_design() {
    let mut sketch = l_bracket();
    solve(&mut sketch);
    let design = "wall = 2; // [1:5]\n\ntranslate([-10, 0]) square(wall);\n";
    let written = update_source(design, &sketch, "l_bracket").unwrap();
    // Parameters above the first instance, the module at the end.
    let first_param = written.find("l_bracket_width = 40;").unwrap();
    assert!(first_param < written.find("square(wall);").unwrap(), "{written}");
    assert!(written.find("module sketch_l_bracket()").unwrap() > written.find("square(wall);").unwrap());
    let program = c2f_eval::Program::load(&written, Path::new("t.c2f"), &[]).unwrap();
    let names: Vec<String> = program.parameters(&written).into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["wall", "l_bracket_width", "l_bracket_height", "l_bracket_thickness"]);

    // Again after a change: the values and the module are replaced.
    let edited = written.replace("l_bracket_height = 30;", "l_bracket_height = 50;");
    let mut reopened = from_code(&edited, "l_bracket").unwrap();
    assert!(solve(&mut reopened).converged());
    reopened.set_variable("l_bracket_width", 45.0);
    solve(&mut reopened);
    let rewritten = update_source(&edited, &reopened, "l_bracket").unwrap();
    assert_eq!(rewritten.matches("module sketch_l_bracket()").count(), 1);
    assert_eq!(rewritten.matches("l_bracket_width =").count(), 1);
    assert!(rewritten.contains("l_bracket_width = 45;") && rewritten.contains("l_bracket_height = 50;"));
    let (area, _, _) = render(&format!("{rewritten}\nsketch_l_bracket();"));
    assert!(near(area, 4.0 + bracket_area(45.0, 50.0, 5.0), 1e-6), "{area}");
    // Rewriting what is already there changes nothing.
    assert_eq!(update_source(&rewritten, &reopened, "l_bracket").unwrap(), rewritten);
}

#[test]
fn snapping_prefers_ends_then_centers_then_midpoints() {
    let mut sketch = Sketch::new(Plane::Xy);
    let line = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false)[0];
    let center = sketch.add_point(5.0, 0.5);
    sketch.add_circle(center, 2.0);

    let at = |x: f64, y: f64, tolerance: f64| snap(&sketch, Point::new(x, y), tolerance);
    let hit = at(9.7, 0.2, 0.5).unwrap();
    assert_eq!((hit.kind, hit.point), (SnapKind::Endpoint, Some(PointId(1))));
    // The center wins over the line's midpoint, half a unit away.
    let hit = at(5.0, 0.2, 1.0).unwrap();
    assert_eq!((hit.kind, hit.point), (SnapKind::Center, Some(center)));
    let hit = at(5.0, -0.3, 0.4).unwrap();
    assert_eq!((hit.kind, hit.entity, hit.at), (SnapKind::Midpoint, Some(line), Point::new(5.0, 0.0)));
    assert_eq!(at(2.5, 3.0, 1.0), None);
}
