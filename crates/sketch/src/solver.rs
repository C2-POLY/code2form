//! The constraint solver: moves the points (and the circles' radii) as
//! little as it can so that every constraint holds, then tells which
//! points are still free to move and which constraints are too many.
//!
//! # How it solves
//!
//! 1. Constraints that only say two unknowns are equal (coincident points,
//!    horizontal and vertical lines, circles of equal radius) and fixed
//!    points are not solved: they merge unknowns, with a union-find over
//!    the coordinates. A merged unknown starts at the mean of its members,
//!    and a fixed point's coordinates are constants. A rectangle drawn as
//!    four lines thus has four unknowns left, not eight.
//! 2. Every other constraint becomes one or two equations `f(x) = 0` in
//!    the unknowns left, in millimeters: the distance from a point to a
//!    line, the length of a segment less its dimension, and for directions
//!    (parallel, perpendicular, angle) the sine of the angle error times
//!    the lines' mean length, so that every equation weighs alike. An
//!    arc adds the equations that keep its two ends on its circle.
//!    Derivatives are exact, by forward differentiation (dual numbers):
//!    each equation reads at most 8 unknowns.
//! 3. Damped Gauss-Newton (Levenberg-Marquardt) in its minimum-norm form:
//!    each step is the smallest move that solves the equations linearized
//!    at the current geometry, `dx = Jᵀ (J Jᵀ + λI)⁻¹ (-f)`. Starting
//!    from the geometry as drawn, the solution found is the one next to
//!    it, so an under-constrained sketch keeps its shape where nothing
//!    says otherwise, and dragging a point only moves what must follow.
//!    λ shrinks after a step that lowers the residual and grows after one
//!    that does not.
//!    Within tolerance, two more steps polish the last digits.
//! 4. At the solution, the rows of the Jacobian are orthogonalized in the
//!    constraints' order (Gram-Schmidt). A row that adds nothing to the
//!    rows before it belongs to a redundant constraint: it already held.
//!    The degrees of freedom are the unknowns less the independent rows;
//!    an unknown is determined when its own direction lies in the rows'
//!    span, that is when no motion left free moves it.
//! 5. When the solve fails, the constraint to blame is the first one, in
//!    the sketch's order, that cannot be met with those before it: the
//!    editor's last addition, as a rule. Bisection over the list finds it
//!    in a few more solves.
//!
//! Where a line and an arc, or two arcs, are tangent and meet at a shared
//! end, tangency is written at that end (the radius square to the line,
//! the centers aligned with it): the distance between center and line
//! would only hold to second order there, and leave the touching point
//! sliding by the square root of the tolerance.
//!
//! Sketches are small (a few hundred unknowns at most), so the linear
//! algebra is dense and hand-written; `J Jᵀ` is built from the Jacobian's
//! sparse rows.

use crate::model::{ConstraintId, Constraint, EntityId, Geometry, PointId, Sketch};

/// The most unknowns one equation reads: two lines' four points.
const SLOTS: usize = 8;
/// The weight of a held point's coordinates: smaller moves less.
const HELD_WEIGHT: f64 = 1e-4;

/// How to solve.
#[derive(Clone, Debug)]
pub struct SolveOptions {
    /// Points to keep where they are when the constraints allow it, such
    /// as the one being dragged: the others move first.
    pub hold: Vec<PointId>,
    pub max_iterations: usize,
}

impl Default for SolveOptions {
    fn default() -> Self {
        SolveOptions { hold: Vec::new(), max_iterations: 100 }
    }
}

/// Whether the constraints could all be met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every constraint holds; the sketch has been moved to the solution.
    Converged,
    /// Some constraints could not be met: they conflict, or the geometry
    /// is too far from any solution. The sketch is left as it was.
    Failed,
}

/// What a solve found.
#[derive(Clone, Debug, PartialEq)]
pub struct SolveReport {
    pub status: Status,
    pub iterations: usize,
    /// The largest error left in an equation, in millimeters.
    pub residual: f64,
    /// Degrees of freedom left: 0 when the sketch is fully constrained.
    pub dof: usize,
    /// For each point, whether the constraints pin it down: drawn black
    /// when true, blue when it can still move.
    pub point_constrained: Vec<bool>,
    /// For each entity, whether its points (and a circle's radius) are all
    /// pinned down.
    pub entity_constrained: Vec<bool>,
    /// Constraints that add nothing: they already held from the others.
    pub redundant: Vec<ConstraintId>,
    /// Constraints that contradict the ones before them.
    pub conflicting: Vec<ConstraintId>,
    /// Constraints left unmet when the solve failed.
    pub unsatisfied: Vec<ConstraintId>,
    /// Constraints whose arguments do not fit them (see [`Sketch::check`]),
    /// left out of the solve.
    pub invalid: Vec<ConstraintId>,
}

impl SolveReport {
    pub fn converged(&self) -> bool {
        self.status == Status::Converged
    }

    /// Converged with no freedom left and nothing redundant.
    pub fn is_fully_constrained(&self) -> bool {
        self.converged() && self.dof == 0
    }

    /// The points that can still move.
    pub fn free_points(&self) -> Vec<PointId> {
        (0..self.point_constrained.len()).filter(|&i| !self.point_constrained[i]).map(PointId).collect()
    }
}

/// Solves the sketch's constraints, moving its geometry as little as it
/// can, and reports what is left free.
pub fn solve(sketch: &mut Sketch) -> SolveReport {
    solve_with(sketch, &SolveOptions::default())
}

/// [`solve`], with options.
pub fn solve_with(sketch: &mut Sketch, options: &SolveOptions) -> SolveReport {
    let mut system = System::build(sketch, options);
    let (status, iterations, residual) = system.run(options.max_iterations);
    let analysis = system.analyze();

    let converged = status == Status::Converged && system.conflicting.is_empty();
    let status = if converged { Status::Converged } else { Status::Failed };
    let mut conflicting = Vec::new();
    let mut redundant = system.redundant.clone();
    let mut unsatisfied = Vec::new();
    if converged {
        redundant.extend(analysis.dependent.iter().filter_map(|&row| system.equations[row].source));
    } else {
        // The first constraint, in the sketch's order, that cannot be met
        // with those before it: the shortest failing list of constraints,
        // by bisection. Equations made dependent by merges further down
        // the list would blame the wrong ones.
        let (mut good, mut bad) = (0, sketch.constraints.len());
        while bad - good > 1 {
            let middle = (good + bad) / 2;
            let mut prefix = sketch.clone();
            prefix.constraints.truncate(middle);
            let mut trial = System::build(&prefix, options);
            let solved = trial.run(options.max_iterations).0 == Status::Converged && trial.conflicting.is_empty();
            if solved { good = middle } else { bad = middle }
        }
        if bad > 0 {
            conflicting.push(ConstraintId(bad - 1));
        }
        let tolerance = system.tolerance;
        for (equation, value) in system.equations.iter().zip(&system.residuals(&system.x)) {
            if let Some(id) = equation.source {
                if value.abs() > tolerance {
                    unsatisfied.push(id);
                }
            }
        }
    }
    for list in [&mut conflicting, &mut redundant, &mut unsatisfied] {
        list.sort();
        list.dedup();
    }

    let determined = |node: usize| match system.unknowns.slot(node) {
        Slot::Const(_) => true,
        Slot::Var(j) => analysis.determined[j],
    };
    let point_constrained: Vec<bool> =
        (0..sketch.points.len()).map(|i| determined(2 * i) && determined(2 * i + 1)).collect();
    let entity_constrained = (0..sketch.entities.len())
        .map(|e| {
            let points = sketch.entity_points(EntityId(e)).iter().all(|p| point_constrained[p.0]);
            let radius = match sketch.entities[e].geometry {
                Geometry::Circle { .. } => system.radius_node[e].is_none_or(determined),
                _ => true,
            };
            points && radius
        })
        .collect();

    if converged {
        system.write_back(sketch);
    }
    SolveReport {
        status,
        iterations,
        residual,
        dof: analysis.dof,
        point_constrained,
        entity_constrained,
        redundant,
        conflicting,
        unsatisfied,
        invalid: system.invalid.clone(),
    }
}

/// An unknown of the system, or a constant (a fixed point's coordinate).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Var(usize),
    Const(f64),
}

/// The coordinates and radii, merged by the constraints that equate them.
/// Node `2i` and `2i + 1` are point `i`'s x and y; radii follow.
struct Unknowns {
    parent: Vec<usize>,
    constant: Vec<Option<f64>>,
    /// Each root's unknown, once numbered.
    var: Vec<Option<usize>>,
}

enum Merge {
    Done,
    Redundant,
    Conflict,
}

impl Unknowns {
    fn new(nodes: usize) -> Self {
        Unknowns { parent: (0..nodes).collect(), constant: vec![None; nodes], var: vec![None; nodes] }
    }

    fn root(&self, mut node: usize) -> usize {
        while self.parent[node] != node {
            node = self.parent[node];
        }
        node
    }

    fn merge(&mut self, a: usize, b: usize, tolerance: f64) -> Merge {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return Merge::Redundant;
        }
        let constant = match (self.constant[ra], self.constant[rb]) {
            (Some(x), Some(y)) if (x - y).abs() > tolerance => return Merge::Conflict,
            (x, y) => x.or(y),
        };
        self.parent[rb] = ra;
        self.constant[ra] = constant;
        Merge::Done
    }

    fn fix(&mut self, node: usize, value: f64, tolerance: f64) -> Merge {
        let root = self.root(node);
        match self.constant[root] {
            Some(c) if (c - value).abs() > tolerance => Merge::Conflict,
            Some(_) => Merge::Redundant,
            None => {
                self.constant[root] = Some(value);
                Merge::Done
            }
        }
    }

    fn slot(&self, node: usize) -> Slot {
        let root = self.root(node);
        match self.constant[root] {
            Some(c) => Slot::Const(c),
            None => Slot::Var(self.var[root].expect("numbered")),
        }
    }
}

/// What an equation computes from its slots.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// p, q: `|p - q| - d`.
    Distance(f64),
    /// p, a, b: the signed distance from p to the line through a and b.
    OnLine,
    /// p, c, r: `|p - c| - r`.
    OnCircle,
    /// p, a, b (one coordinate each): `p - (a + b) / 2`.
    Mid,
    /// a, b, c, d: two lines' directions, `sin(angle from ab to cd - θ)`,
    /// given as (cos θ, sin θ), times the lines' mean length.
    Direction(f64, f64),
    /// a, b, c, d: `|b - a| - |d - c|`.
    EqualLength,
    /// x: `x - v`.
    Value(f64),
    /// a, b, c, r: the signed distance from c to the line ab, on the side
    /// given by the sign, less r.
    TangentLine(f64),
    /// c1, r1, c2, r2: `|c1 - c2| - (r1 + r2)` outside each other, or
    /// `|c1 - c2| - s (r1 - r2)` one inside the other.
    TangentRound(Option<f64>),
    /// a, b, s, c: a line and an arc tangent where they meet at s: the
    /// arc's radius at s square to the line, `(b - a)·(s - c) / |b - a|`.
    NormalAt,
    /// c1, s, c2: two arcs tangent where they meet at s: the second
    /// center on the line through the first and s, as a distance.
    Collinear,
}

struct Equation {
    kind: Kind,
    slots: Vec<Slot>,
    /// The constraint it comes from; none for an arc's own equations.
    source: Option<ConstraintId>,
}

struct System {
    unknowns: Unknowns,
    /// The circles' and arcs' radius nodes, by entity.
    radius_node: Vec<Option<usize>>,
    equations: Vec<Equation>,
    x: Vec<f64>,
    /// Each unknown's weight in the step's norm: held points move less.
    weight: Vec<f64>,
    tolerance: f64,
    redundant: Vec<ConstraintId>,
    conflicting: Vec<ConstraintId>,
    invalid: Vec<ConstraintId>,
}

impl System {
    fn build(sketch: &Sketch, options: &SolveOptions) -> System {
        let mut radius_node = vec![None; sketch.entities.len()];
        let mut start_value: Vec<f64> = sketch.points.iter().flat_map(|p| [p.x, p.y]).collect();
        for (e, entity) in sketch.entities.iter().enumerate() {
            if !matches!(entity.geometry, Geometry::Line { .. }) {
                radius_node[e] = Some(start_value.len());
                start_value.push(sketch.radius(EntityId(e)).unwrap_or(0.0));
            }
        }
        let scale = start_value.iter().fold(1.0f64, |m, v| m.max(v.abs()));
        let tolerance = 1e-9 * scale;
        let mut unknowns = Unknowns::new(start_value.len());
        let mut system = System {
            unknowns: Unknowns::new(0),
            radius_node,
            equations: Vec::new(),
            x: Vec::new(),
            weight: Vec::new(),
            tolerance,
            redundant: Vec::new(),
            conflicting: Vec::new(),
            invalid: Vec::new(),
        };

        // Merges first, in the constraints' order.
        let px = |p: &PointId| 2 * p.0;
        let py = |p: &PointId| 2 * p.0 + 1;
        let line = |e: &EntityId| match sketch.entities[e.0].geometry {
            Geometry::Line { a, b } => (a, b),
            _ => unreachable!("checked"),
        };
        let mut valid = vec![false; sketch.constraints.len()];
        for (i, constraint) in sketch.constraints.iter().enumerate() {
            let id = ConstraintId(i);
            if sketch.check(id).is_err() {
                system.invalid.push(id);
                continue;
            }
            valid[i] = true;
            let merges: Vec<Merge> = match constraint {
                Constraint::Coincident(a, b) => {
                    vec![unknowns.merge(px(a), px(b), tolerance), unknowns.merge(py(a), py(b), tolerance)]
                }
                Constraint::Horizontal(e) => {
                    let (a, b) = line(e);
                    vec![unknowns.merge(py(&a), py(&b), tolerance)]
                }
                Constraint::Vertical(e) => {
                    let (a, b) = line(e);
                    vec![unknowns.merge(px(&a), px(&b), tolerance)]
                }
                Constraint::Equal(a, b) if system.radius_node[a.0].is_some() => {
                    let (ra, rb) = (system.radius_node[a.0].unwrap(), system.radius_node[b.0].unwrap());
                    vec![unknowns.merge(ra, rb, tolerance)]
                }
                Constraint::Fixed(p) => {
                    let at = sketch.point(*p);
                    vec![unknowns.fix(px(p), at.x, tolerance), unknowns.fix(py(p), at.y, tolerance)]
                }
                _ => continue,
            };
            if merges.iter().any(|m| matches!(m, Merge::Conflict)) {
                system.conflicting.push(id);
            } else if merges.iter().all(|m| matches!(m, Merge::Redundant)) {
                system.redundant.push(id);
            }
        }

        // Number the unknowns left; each starts at its members' mean,
        // weighted toward held points.
        let held_nodes: Vec<usize> = options.hold.iter().flat_map(|p| [px(p), py(p)]).collect();
        let node_weight = |node: usize| if held_nodes.contains(&node) { 1.0 / HELD_WEIGHT } else { 1.0 };
        let mut sums: Vec<(f64, f64, bool)> = vec![(0.0, 0.0, false); start_value.len()];
        for (node, &value) in start_value.iter().enumerate() {
            let root = unknowns.root(node);
            let w = node_weight(node);
            sums[root].0 += w * value;
            sums[root].1 += w;
            sums[root].2 |= held_nodes.contains(&node);
        }
        for (node, &(sum, total, held)) in sums.iter().enumerate() {
            if unknowns.parent[node] == node && unknowns.constant[node].is_none() {
                unknowns.var[node] = Some(system.x.len());
                system.x.push(sum / total);
                system.weight.push(if held { HELD_WEIGHT } else { 1.0 });
            }
        }
        system.unknowns = unknowns;

        // Equations: the arcs' own, then the constraints'.
        let u = &system.unknowns;
        let pt = |p: &PointId| [u.slot(px(p)), u.slot(py(p))];
        let radius = |e: &EntityId| u.slot(system.radius_node[e.0].expect("round"));
        let center = |e: &EntityId| match sketch.entities[e.0].geometry {
            Geometry::Circle { center, .. } | Geometry::Arc { center, .. } => center,
            Geometry::Line { .. } => unreachable!("checked"),
        };
        // The points two entities may meet at, and whether two points are
        // one, shared or held together.
        let ends = |e: &EntityId| match sketch.entities[e.0].geometry {
            Geometry::Line { a, b } => vec![a, b],
            Geometry::Arc { start, end, .. } => vec![start, end],
            Geometry::Circle { .. } => vec![],
        };
        let same = |p: &PointId, q: &PointId| {
            p == q || (u.root(px(p)) == u.root(px(q)) && u.root(py(p)) == u.root(py(q)))
        };
        let mut equations = Vec::new();
        for (e, entity) in sketch.entities.iter().enumerate() {
            if let Geometry::Arc { center, start, end } = entity.geometry {
                for p in [start, end] {
                    let slots = [pt(&p), pt(&center)].concat().into_iter().chain([radius(&EntityId(e))]).collect();
                    equations.push(Equation { kind: Kind::OnCircle, slots, source: None });
                }
            }
        }
        let sign = |v: f64| if v < 0.0 { -1.0 } else { 1.0 };
        for (i, constraint) in sketch.constraints.iter().enumerate() {
            if !valid[i] {
                continue;
            }
            let source = Some(ConstraintId(i));
            let mut push = |kind: Kind, slots: Vec<Slot>| equations.push(Equation { kind, slots, source });
            let two_lines = |a: &EntityId, b: &EntityId| {
                let ((p, q), (r, s)) = (line(a), line(b));
                [pt(&p), pt(&q), pt(&r), pt(&s)].concat()
            };
            match constraint {
                Constraint::Coincident(..)
                | Constraint::Horizontal(_)
                | Constraint::Vertical(_)
                | Constraint::Fixed(_) => {}
                Constraint::Equal(a, b) => {
                    if system.radius_node[a.0].is_none() {
                        push(Kind::EqualLength, two_lines(a, b));
                    }
                }
                Constraint::Parallel(a, b) => push(Kind::Direction(1.0, 0.0), two_lines(a, b)),
                // At 90° or 270°: the equation's zeros are both.
                Constraint::Perpendicular(a, b) => push(Kind::Direction(0.0, 1.0), two_lines(a, b)),
                Constraint::Angle(a, b, d) => {
                    let theta = d.value.to_radians();
                    push(Kind::Direction(theta.cos(), theta.sin()), two_lines(a, b));
                }
                Constraint::Distance(p, q, d) => push(Kind::Distance(d.value), [pt(p), pt(q)].concat()),
                Constraint::Length(e, d) => {
                    let (p, q) = line(e);
                    push(Kind::Distance(d.value), [pt(&p), pt(&q)].concat());
                }
                Constraint::Radius(e, d) => push(Kind::Value(d.value), vec![radius(e)]),
                Constraint::Diameter(e, d) => push(Kind::Value(d.value / 2.0), vec![radius(e)]),
                Constraint::PointOnLine(p, e) => {
                    let (a, b) = line(e);
                    push(Kind::OnLine, [pt(p), pt(&a), pt(&b)].concat());
                }
                Constraint::PointOnCircle(p, e) => {
                    let slots = [pt(p), pt(&center(e))].concat().into_iter().chain([radius(e)]).collect();
                    push(Kind::OnCircle, slots);
                }
                Constraint::Midpoint(p, e) => {
                    let (a, b) = line(e);
                    for axis in 0..2 {
                        push(Kind::Mid, vec![pt(p)[axis], pt(&a)[axis], pt(&b)[axis]]);
                    }
                }
                Constraint::Tangent(a, b) => {
                    let (cb, rb) = (center(b), sketch.radius(*b).unwrap_or(0.0));
                    // Where the two meet end to end, the distance equations
                    // would only hold to second order (the touching point
                    // slides along the line): square radius or aligned
                    // centers at the shared end hold to first order.
                    let shared = ends(a).into_iter().find_map(|p| ends(b).into_iter().find(|q| same(&p, q)));
                    if let (Some(s), Geometry::Line { a: la, b: lb }) = (shared, sketch.entities[a.0].geometry) {
                        push(Kind::NormalAt, [pt(&la), pt(&lb), pt(&s), pt(&cb)].concat());
                    } else if let Some(s) = shared {
                        push(Kind::Collinear, [pt(&center(a)), pt(&s), pt(&cb)].concat());
                    } else if system.radius_node[a.0].is_none() {
                        let (p, q) = line(a);
                        let (d, to_center) = (sub(sketch, q, p), sub(sketch, cb, p));
                        let side = sign(d[0] * to_center[1] - d[1] * to_center[0]);
                        let slots = [pt(&p), pt(&q), pt(&cb)].concat().into_iter().chain([radius(b)]).collect();
                        push(Kind::TangentLine(side), slots);
                    } else {
                        let (ca, ra) = (center(a), sketch.radius(*a).unwrap_or(0.0));
                        let gap = sketch.point(ca).distance(sketch.point(cb));
                        let mode = if (gap - (ra + rb)).abs() <= (gap - (ra - rb).abs()).abs() {
                            None
                        } else {
                            Some(sign(ra - rb))
                        };
                        let slots = [pt(&ca).as_slice(), &[radius(a)], &pt(&cb), &[radius(b)]].concat();
                        push(Kind::TangentRound(mode), slots);
                    }
                }
            }
        }
        system.equations = equations;
        system
    }

    /// Each equation's value at `x`.
    fn residuals(&self, x: &[f64]) -> Vec<f64> {
        self.equations.iter().map(|e| evaluate(e, x, false).v).collect()
    }

    /// The residuals and the Jacobian's rows, as (unknown, derivative)
    /// pairs with each unknown once.
    fn linearize(&self, x: &[f64]) -> (Vec<f64>, Vec<Vec<(usize, f64)>>) {
        let mut f = Vec::with_capacity(self.equations.len());
        let mut rows = Vec::with_capacity(self.equations.len());
        for equation in &self.equations {
            let value = evaluate(equation, x, true);
            let mut row: Vec<(usize, f64)> = Vec::with_capacity(SLOTS);
            for (slot, &d) in equation.slots.iter().zip(&value.d) {
                if let Slot::Var(j) = *slot {
                    match row.iter_mut().find(|(k, _)| *k == j) {
                        Some(entry) => entry.1 += d,
                        None => row.push((j, d)),
                    }
                }
            }
            f.push(value.v);
            rows.push(row);
        }
        (f, rows)
    }

    /// Levenberg-Marquardt from the current unknowns: the status, the
    /// iterations taken and the largest residual left.
    fn run(&mut self, max_iterations: usize) -> (Status, usize, f64) {
        let max_abs = |f: &[f64]| f.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let norm2 = |f: &[f64]| f.iter().map(|v| v * v).sum::<f64>();
        let m = self.equations.len();
        let mut lambda = 1e-10;
        let mut iterations = 0;
        // Once within tolerance, a couple more steps polish the last
        // digits, which converge quadratically.
        let mut polish = 0;
        loop {
            let (f, rows) = self.linearize(&self.x);
            let residual = max_abs(&f);
            if residual <= self.tolerance {
                if residual <= 1e-6 * self.tolerance || polish == 2 {
                    return (Status::Converged, iterations, residual);
                }
                polish += 1;
            }
            if iterations >= max_iterations {
                let status = if residual <= self.tolerance { Status::Converged } else { Status::Failed };
                return (status, iterations, residual);
            }
            iterations += 1;

            // A = J W Jᵀ, from the sparse rows.
            let mut a = vec![0.0; m * m];
            let mut dense = vec![0.0; self.x.len()];
            for i in 0..m {
                for &(j, d) in &rows[i] {
                    dense[j] = d * self.weight[j];
                }
                for k in i..m {
                    let dot: f64 = rows[k].iter().map(|&(j, d)| dense[j] * d).sum();
                    a[i * m + k] = dot;
                    a[k * m + i] = dot;
                }
                for &(j, _) in &rows[i] {
                    dense[j] = 0.0;
                }
            }
            let diagonal = (0..m).map(|i| a[i * m + i]).fold(0.0f64, f64::max).max(1e-12);

            let error = norm2(&f);
            let mut improved = false;
            while lambda < 1e8 {
                let mut damped = a.clone();
                for i in 0..m {
                    damped[i * m + i] += lambda * diagonal;
                }
                let mut y: Vec<f64> = f.iter().map(|v| -v).collect();
                if !cholesky_solve(&mut damped, m, &mut y) {
                    lambda *= 10.0;
                    continue;
                }
                let mut trial = self.x.clone();
                for (row, yi) in rows.iter().zip(&y) {
                    for &(j, d) in row {
                        trial[j] += self.weight[j] * d * yi;
                    }
                }
                let trial_error = norm2(&self.residuals(&trial));
                if trial_error < error {
                    self.x = trial;
                    lambda = (lambda / 10.0).max(1e-12);
                    improved = true;
                    break;
                }
                lambda *= 10.0;
            }
            if !improved {
                let residual = max_abs(&self.residuals(&self.x));
                let status = if residual <= self.tolerance { Status::Converged } else { Status::Failed };
                return (status, iterations, residual);
            }
        }
    }

    /// Rank, degrees of freedom and dependent rows at the current unknowns.
    fn analyze(&self) -> Analysis {
        let n = self.x.len();
        let (_, rows) = self.linearize(&self.x);
        let mut basis: Vec<Vec<f64>> = Vec::new();
        let mut dependent = Vec::new();
        let largest = rows.iter().flatten().fold(0.0f64, |m, (_, d)| m.max(d.abs()));
        for (r, row) in rows.iter().enumerate() {
            let mut v = vec![0.0; n];
            for &(j, d) in row {
                v[j] += d;
            }
            let norm0 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm0 <= 1e-12 * largest.max(1.0) {
                dependent.push(r);
                continue;
            }
            // Modified Gram-Schmidt, twice for accuracy.
            for _ in 0..2 {
                for q in &basis {
                    let dot: f64 = q.iter().zip(&v).map(|(a, b)| a * b).sum();
                    for (vi, qi) in v.iter_mut().zip(q) {
                        *vi -= dot * qi;
                    }
                }
            }
            let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm <= 1e-8 * norm0 {
                dependent.push(r);
            } else {
                v.iter_mut().for_each(|x| *x /= norm);
                basis.push(v);
            }
        }
        let determined =
            (0..n).map(|j| basis.iter().map(|q| q[j] * q[j]).sum::<f64>() >= 1.0 - 1e-6).collect();
        Analysis { dof: n - basis.len(), determined, dependent }
    }

    fn write_back(&self, sketch: &mut Sketch) {
        let value = |node: usize| match self.unknowns.slot(node) {
            Slot::Const(c) => c,
            Slot::Var(j) => self.x[j],
        };
        for (i, p) in sketch.points.iter_mut().enumerate() {
            p.x = value(2 * i);
            p.y = value(2 * i + 1);
        }
        for (e, entity) in sketch.entities.iter_mut().enumerate() {
            if let (Geometry::Circle { radius, .. }, Some(node)) = (&mut entity.geometry, self.radius_node[e]) {
                *radius = value(node);
            }
        }
    }
}

struct Analysis {
    dof: usize,
    determined: Vec<bool>,
    dependent: Vec<usize>,
}

fn sub(sketch: &Sketch, a: PointId, b: PointId) -> [f64; 2] {
    let (a, b) = (sketch.point(a), sketch.point(b));
    [a.x - b.x, a.y - b.y]
}

/// An equation's value and, when `derive`, its derivatives by slot.
fn evaluate(equation: &Equation, x: &[f64], derive: bool) -> Dual {
    let s: Vec<Dual> = equation
        .slots
        .iter()
        .enumerate()
        .map(|(i, slot)| match *slot {
            Slot::Const(c) => Dual::constant(c),
            Slot::Var(j) if derive => Dual::variable(x[j], i),
            Slot::Var(j) => Dual::constant(x[j]),
        })
        .collect();
    let vec = |i: usize| [s[i], s[i + 1]];
    match equation.kind {
        Kind::Distance(d) => length(sub2(vec(0), vec(2))) - d,
        Kind::OnLine => {
            let (p, a, b) = (vec(0), vec(2), vec(4));
            let d = sub2(b, a);
            cross(d, sub2(p, a)) / length(d)
        }
        Kind::OnCircle => length(sub2(vec(0), vec(2))) - s[4],
        Kind::Mid => s[0] - (s[1] + s[2]) * 0.5,
        Kind::Direction(cos, sin) => {
            let (d1, d2) = (sub2(vec(2), vec(0)), sub2(vec(6), vec(4)));
            let (l1, l2) = (length(d1), length(d2));
            let sine = (cross(d1, d2) * cos - dot(d1, d2) * sin) / (l1 * l2);
            sine * ((l1 + l2) * 0.5)
        }
        Kind::EqualLength => length(sub2(vec(2), vec(0))) - length(sub2(vec(6), vec(4))),
        Kind::Value(v) => s[0] - v,
        Kind::TangentLine(side) => {
            let (a, b, c) = (vec(0), vec(2), vec(4));
            let d = sub2(b, a);
            cross(d, sub2(c, a)) / length(d) * side - s[6]
        }
        Kind::TangentRound(mode) => {
            let gap = length(sub2(vec(0), vec(3)));
            match mode {
                None => gap - (s[2] + s[5]),
                Some(sign) => gap - (s[2] - s[5]) * sign,
            }
        }
        Kind::NormalAt => {
            let d = sub2(vec(2), vec(0));
            dot(d, sub2(vec(4), vec(6))) / length(d)
        }
        Kind::Collinear => {
            let radial = sub2(vec(2), vec(0));
            cross(radial, sub2(vec(4), vec(0))) / length(radial)
        }
    }
}

fn sub2(a: [Dual; 2], b: [Dual; 2]) -> [Dual; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: [Dual; 2], b: [Dual; 2]) -> Dual {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: [Dual; 2], b: [Dual; 2]) -> Dual {
    a[0] * b[0] + a[1] * b[1]
}

fn length(a: [Dual; 2]) -> Dual {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

/// A value and its derivatives with respect to an equation's slots.
#[derive(Clone, Copy, Debug)]
struct Dual {
    v: f64,
    d: [f64; SLOTS],
}

impl Dual {
    fn constant(v: f64) -> Self {
        Dual { v, d: [0.0; SLOTS] }
    }

    fn variable(v: f64, slot: usize) -> Self {
        let mut d = [0.0; SLOTS];
        d[slot] = 1.0;
        Dual { v, d }
    }

    fn map(self, v: f64, scale: f64) -> Self {
        Dual { v, d: self.d.map(|d| d * scale) }
    }

    /// The square root; flat at 0, where a zero-length segment has no
    /// direction to follow.
    fn sqrt(self) -> Self {
        let v = self.v.max(0.0).sqrt();
        self.map(v, if v > 1e-150 { 0.5 / v } else { 0.0 })
    }
}

impl std::ops::Add for Dual {
    type Output = Dual;
    fn add(self, o: Dual) -> Dual {
        Dual { v: self.v + o.v, d: std::array::from_fn(|i| self.d[i] + o.d[i]) }
    }
}

impl std::ops::Sub for Dual {
    type Output = Dual;
    fn sub(self, o: Dual) -> Dual {
        Dual { v: self.v - o.v, d: std::array::from_fn(|i| self.d[i] - o.d[i]) }
    }
}

impl std::ops::Sub<f64> for Dual {
    type Output = Dual;
    fn sub(self, o: f64) -> Dual {
        Dual { v: self.v - o, ..self }
    }
}

impl std::ops::Mul for Dual {
    type Output = Dual;
    /// The product rule: `(uv)' = u'v + v'u`.
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn mul(self, o: Dual) -> Dual {
        Dual { v: self.v * o.v, d: std::array::from_fn(|i| self.d[i] * o.v + o.d[i] * self.v) }
    }
}

impl std::ops::Mul<f64> for Dual {
    type Output = Dual;
    fn mul(self, o: f64) -> Dual {
        self.map(self.v * o, o)
    }
}

impl std::ops::Div for Dual {
    type Output = Dual;
    /// A quotient; flat where the divisor vanishes (a zero-length line).
    fn div(self, o: Dual) -> Dual {
        if o.v.abs() < 1e-150 {
            return Dual::constant(0.0);
        }
        let v = self.v / o.v;
        Dual { v, d: std::array::from_fn(|i| (self.d[i] - v * o.d[i]) / o.v) }
    }
}

/// Solves `a y = b` in place for a symmetric positive definite `a` (n×n,
/// row-major); false when it is not positive definite.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    for j in 0..n {
        let mut diag = a[j * n + j];
        for k in 0..j {
            diag -= a[j * n + k] * a[j * n + k];
        }
        if diag <= 0.0 || !diag.is_finite() {
            return false;
        }
        let diag = diag.sqrt();
        a[j * n + j] = diag;
        for i in j + 1..n {
            let mut v = a[i * n + j];
            for k in 0..j {
                v -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = v / diag;
        }
    }
    for i in 0..n {
        let mut v = b[i];
        for k in 0..i {
            v -= a[i * n + k] * b[k];
        }
        b[i] = v / a[i * n + i];
    }
    for i in (0..n).rev() {
        let mut v = b[i];
        for k in i + 1..n {
            v -= a[k * n + i] * b[k];
        }
        b[i] = v / a[i * n + i];
    }
    true
}
