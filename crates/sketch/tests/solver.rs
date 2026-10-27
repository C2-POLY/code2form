//! The solver on cases worked out by hand: where the geometry ends up,
//! the freedom left, and the constraints that are too many.

use c2f_sketch::{
    Constraint, ConstraintId, Dimension, EntityId, Geometry, Plane, Point, PointId, SolveOptions, Sketch, Status,
    solve, solve_with,
};

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-7
}

fn assert_at(sketch: &Sketch, id: usize, x: f64, y: f64) {
    let p = sketch.point(PointId(id));
    assert!(close(p, Point::new(x, y)), "p{id} at ({}, {}), expected ({x}, {y})", p.x, p.y);
}

#[test]
fn a_dimensioned_rectangle_is_fully_constrained_once_its_corner_is_fixed() {
    let mut sketch = Sketch::new(Plane::Xy);
    let [bottom, right, ..] = sketch.add_rectangle(Point::new(1.0, 1.0), Point::new(35.0, 18.0));
    sketch.add_constraint(Constraint::Length(bottom, Dimension::new(40.0)));
    sketch.add_constraint(Constraint::Length(right, 20.0.into()));

    // Free to slide around: two degrees of freedom, all blue.
    let report = solve(&mut sketch);
    assert_eq!(report.status, Status::Converged);
    assert_eq!(report.dof, 2);
    assert!(report.entity_constrained.iter().all(|c| !c));
    assert_eq!(report.free_points().len(), 4);

    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    let report = solve(&mut sketch);
    assert!(report.is_fully_constrained(), "{report:?}");
    assert!(report.point_constrained.iter().all(|c| *c));
    assert!(report.entity_constrained.iter().all(|c| *c));
    assert!(report.redundant.is_empty() && report.conflicting.is_empty());
    assert!(report.residual < 1e-9);
    // The first solve grew the rectangle from its middle, and it stayed.
    for (i, (x, y)) in [(-2.0, -0.5), (38.0, -0.5), (38.0, 19.5), (-2.0, 19.5)].into_iter().enumerate() {
        assert_at(&sketch, i, x, y);
    }
}

#[test]
fn a_triangle_from_its_three_sides() {
    let mut sketch = Sketch::new(Plane::Xy);
    let sides = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(3.5, 0.4), Point::new(3.2, 3.6)], true);
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    for (line, length) in sides.iter().zip([4.0, 3.0, 5.0]) {
        sketch.add_constraint(Constraint::Length(*line, length.into()));
    }
    // It can still turn around its fixed corner.
    let report = solve(&mut sketch);
    assert!(report.converged());
    assert_eq!(report.dof, 1);
    assert_eq!(report.free_points(), [PointId(1), PointId(2)]);
    assert!(report.point_constrained[0]);

    sketch.add_constraint(Constraint::Horizontal(sides[0]));
    let report = solve(&mut sketch);
    assert!(report.is_fully_constrained(), "{report:?}");
    // A 3-4-5 right triangle, still on the side it was drawn on.
    assert_at(&sketch, 0, 0.0, 0.0);
    assert_at(&sketch, 1, 4.0, 0.0);
    assert_at(&sketch, 2, 4.0, 3.0);
}

#[test]
fn a_tangent_arc_joins_two_lines() {
    let mut sketch = Sketch::new(Plane::Xy);
    let p = [(0.0, 0.0), (9.5, 0.3), (10.4, 4.5), (14.6, 5.4), (15.3, 14.2)].map(|(x, y)| sketch.add_point(x, y));
    let first = sketch.add_line(p[0], p[1]);
    let arc = sketch.add_arc(p[2], p[1], p[3]);
    let second = sketch.add_line(p[3], p[4]);
    for c in [
        Constraint::Fixed(p[0]),
        Constraint::Horizontal(first),
        Constraint::Vertical(second),
        Constraint::Tangent(first, arc),
        Constraint::Tangent(second, arc),
        Constraint::Radius(arc, 5.0.into()),
        Constraint::Length(first, 10.0.into()),
        Constraint::Length(second, 10.0.into()),
    ] {
        sketch.add_constraint(c);
    }
    let report = solve(&mut sketch);
    assert!(report.is_fully_constrained(), "{report:?}");
    assert_at(&sketch, 1, 10.0, 0.0);
    assert_at(&sketch, 2, 10.0, 5.0);
    assert_at(&sketch, 3, 15.0, 5.0);
    assert_at(&sketch, 4, 15.0, 15.0);
    assert!((sketch.radius(arc).unwrap() - 5.0).abs() < 1e-9);
}

#[test]
fn circles_tangent_outside_and_inside() {
    let mut sketch = Sketch::new(Plane::Xy);
    let (a, b, c) = (sketch.add_point(0.0, 0.0), sketch.add_point(14.0, 1.0), sketch.add_point(1.0, 0.5));
    let big = sketch.add_circle(a, 10.0);
    let outside = sketch.add_circle(b, 3.5);
    let inside = sketch.add_circle(c, 6.0);
    for constraint in [
        Constraint::Fixed(a),
        Constraint::Radius(big, 10.0.into()),
        Constraint::Diameter(outside, 8.0.into()),
        Constraint::Equal(outside, inside),
        Constraint::Tangent(big, outside),
        Constraint::Tangent(big, inside),
    ] {
        sketch.add_constraint(constraint);
    }
    let report = solve(&mut sketch);
    assert!(report.converged(), "{report:?}");
    // Each small circle can still roll around the big one.
    assert_eq!(report.dof, 2);
    assert!(report.entity_constrained[0] && !report.entity_constrained[1]);
    let at = |p: PointId| sketch.point(p).distance(sketch.point(a));
    assert!((at(b) - 14.0).abs() < 1e-9, "{}", at(b));
    assert!((at(c) - 6.0).abs() < 1e-9, "{}", at(c));
    assert!(matches!(sketch.entity(inside).geometry, Geometry::Circle { radius, .. } if (radius - 4.0).abs() < 1e-9));
}

#[test]
fn points_on_lines_and_circles_midpoints_and_angles() {
    let mut sketch = Sketch::new(Plane::Xy);
    let base = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(20.0, 0.5)], false)[0];
    let slant = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(8.0, 9.0)], false)[0];
    let (mid, on, round) = (sketch.add_point(9.0, 1.0), sketch.add_point(3.0, 3.5), sketch.add_point(30.0, 0.0));
    let circle = sketch.add_circle(round, 5.0);
    let rim = sketch.add_point(31.0, 4.0);
    for constraint in [
        Constraint::Coincident(PointId(0), PointId(2)),
        Constraint::Fixed(PointId(0)),
        Constraint::Horizontal(base),
        Constraint::Length(base, 20.0.into()),
        Constraint::Angle(base, slant, 45.0.into()),
        Constraint::Distance(PointId(2), PointId(3), Dimension::new(2f64.sqrt() * 10.0)),
        Constraint::Midpoint(mid, base),
        Constraint::PointOnLine(on, slant),
        Constraint::PointOnCircle(rim, circle),
        Constraint::Perpendicular(base, slant),
    ] {
        sketch.add_constraint(constraint);
    }
    let report = solve(&mut sketch);
    // 45° and perpendicular cannot both hold.
    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.conflicting, [ConstraintId(9)]);

    sketch.constraints.pop();
    let report = solve(&mut sketch);
    assert!(report.converged(), "{report:?}");
    assert_at(&sketch, 1, 20.0, 0.0);
    assert_at(&sketch, 3, 10.0, 10.0);
    assert_at(&sketch, 4, 10.0, 0.0);
    let on = sketch.point(on);
    assert!((on.x - on.y).abs() < 1e-9);
    let radius = sketch.radius(circle).unwrap();
    assert!((sketch.point(rim).distance(sketch.point(round)) - radius).abs() < 1e-9);
    // Free: the point sliding on the slanted line, and the circle.
    assert!(!report.point_constrained[5] && !report.point_constrained[6] && !report.point_constrained[7]);
    assert!(report.point_constrained[4], "the midpoint is pinned down");
}

/// A fully dimensioned rectangle, then `extra`.
fn rectangle_with(extra: Constraint) -> (Sketch, [EntityId; 4]) {
    let mut sketch = Sketch::new(Plane::Xy);
    let lines = sketch.add_rectangle(Point::new(0.0, 0.0), Point::new(40.0, 20.0));
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    sketch.add_constraint(Constraint::Length(lines[0], 40.0.into()));
    sketch.add_constraint(Constraint::Length(lines[1], 20.0.into()));
    sketch.add_constraint(extra);
    (sketch, lines)
}

#[test]
fn constraints_too_many_are_reported() {
    // The rectangle's constraints: 4 for its sides, the fixed corner, two
    // lengths; the extra one is the 8th.
    let extra = ConstraintId(7);

    // The same length again holds already.
    let (mut sketch, lines) = rectangle_with(Constraint::Length(EntityId(2), 40.0.into()));
    let report = solve(&mut sketch);
    assert!(report.converged());
    assert_eq!(report.redundant, [extra]);
    assert_eq!(report.dof, 0);

    // As does a side already horizontal, or parallel to another.
    for constraint in [Constraint::Horizontal(lines[2]), Constraint::Parallel(lines[0], lines[2])] {
        let (mut sketch, _) = rectangle_with(constraint);
        let report = solve(&mut sketch);
        assert!(report.converged());
        assert_eq!(report.redundant, [extra]);
    }

    // A different length cannot hold: the sketch is left as it was.
    let (mut sketch, _) = rectangle_with(Constraint::Length(EntityId(2), 45.0.into()));
    let before = sketch.points.clone();
    let report = solve(&mut sketch);
    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.conflicting, [extra]);
    assert!(report.unsatisfied.contains(&extra));
    assert_eq!(sketch.points, before);

    // Nor can a corner fixed elsewhere.
    let (mut sketch, _) = rectangle_with(Constraint::Coincident(PointId(0), PointId(2)));
    let report = solve(&mut sketch);
    assert_eq!(report.status, Status::Failed);
    assert_eq!(report.conflicting, [extra]);

    // An argument of the wrong kind is left out.
    let (mut sketch, _) = rectangle_with(Constraint::Radius(EntityId(0), 3.0.into()));
    let report = solve(&mut sketch);
    assert!(report.is_fully_constrained());
    assert_eq!(report.invalid, [extra]);
}

#[test]
fn solving_moves_the_geometry_as_little_as_it_can() {
    // A free line given a length grows from both ends alike.
    let mut sketch = Sketch::new(Plane::Xy);
    let line = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false)[0];
    sketch.add_constraint(Constraint::Length(line, 12.0.into()));
    let report = solve(&mut sketch);
    assert_eq!(report.dof, 3);
    assert_at(&sketch, 0, -1.0, 0.0);
    assert_at(&sketch, 1, 11.0, 0.0);

    // Dragging an end keeps it under the mouse; the other end follows.
    sketch.points[1] = Point::new(20.0, 0.0);
    let report = solve_with(&mut sketch, &SolveOptions { hold: vec![PointId(1)], ..SolveOptions::default() });
    assert!(report.converged());
    let held = sketch.point(PointId(1));
    assert!(held.distance(Point::new(20.0, 0.0)) < 1e-3, "{held:?}");
    assert!((sketch.point(PointId(0)).distance(held) - 12.0).abs() < 1e-9);
}

#[test]
fn fifty_entities_solve_quickly() {
    // A regular 50-gon of 10 mm sides, by its lengths and all but three
    // of its angles, drawn crooked.
    let n = 50;
    let mut seed = 12345u64;
    let mut noise = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.6
    };
    let radius = 10.0 / (2.0 * (std::f64::consts::PI / n as f64).sin());
    let corners: Vec<Point> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64 - std::f64::consts::FRAC_PI_2;
            Point::new(radius * a.cos() + noise(), radius * a.sin() + radius + noise())
        })
        .collect();
    let mut sketch = Sketch::new(Plane::Xy);
    let sides = sketch.add_polyline(&corners, true);
    sketch.add_constraint(Constraint::Fixed(PointId(0)));
    sketch.add_constraint(Constraint::Horizontal(sides[0]));
    for &side in &sides {
        sketch.add_constraint(Constraint::Length(side, 10.0.into()));
    }
    for pair in sides.windows(2).take(n - 3) {
        sketch.add_constraint(Constraint::Angle(pair[0], pair[1], (360.0 / n as f64).into()));
    }
    let start = std::time::Instant::now();
    let report = solve(&mut sketch);
    let elapsed = start.elapsed();
    assert!(report.is_fully_constrained(), "{report:?}");
    for &side in &sides {
        let Geometry::Line { a, b } = sketch.entity(side).geometry else { unreachable!() };
        assert!((sketch.point(a).distance(sketch.point(b)) - 10.0).abs() < 1e-8);
    }
    // About 2 ms in release; debug builds run some 20 times slower, and
    // CI machines slower still.
    let budget = if cfg!(debug_assertions) { 300 } else { 5 };
    assert!(elapsed.as_millis() < budget, "{elapsed:?} for {} entities", sketch.entities.len());
}
