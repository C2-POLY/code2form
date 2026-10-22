//! The code a sketch becomes: its dimension variables, at the top of the
//! file where the Customizer reads them, and a module `sketch_<name>()`
//! that draws its profile with `polygon()` and holds the whole sketch in
//! `// @sketch` lines (see [`crate::format`]).
//!
//! ```text
//! // Sketch l_bracket: length
//! l_bracket_width = 40; // 0.1
//! // Sketch l_bracket: length
//! l_bracket_thickness = 5; // 0.1
//!
//! module sketch_l_bracket() {
//!     // Drawn by the sketch editor, which reopens it from the @sketch
//!     // lines: its geometry, constraints and dimensions.
//!     // @sketch v1 plane xy
//!     // @sketch point p0 0 0
//!     // ...
//!     polygon(points = [
//!         [0, 0],
//!         [l_bracket_width, 0],
//!         [l_bracket_width, l_bracket_thickness],
//!         // ...
//!     ]);
//! }
//! ```
//!
//! # Points that follow their dimensions
//!
//! The Customizer changes a variable without the sketch editor, so the
//! polygon's points are written as expressions of the variables wherever
//! that is exact. A point's coordinates are affine in a dimension when
//! moving the dimension moves them in proportion, which is the case of
//! lengths and distances on a profile whose directions are held (a
//! rectilinear profile, or one whose slanted sides have their angles set):
//! the generator solves the sketch again with each variable nudged up and
//! down, keeps the variables whose effect is proportional, then checks
//! the expressions against a solve with all of them nudged at once. The
//! points then read `[width, thickness]` rather than `[40, 5]`.
//!
//! A variable that bends the profile instead (an angle, a radius whose
//! arc turns) cannot be written that way: its parameter is still written,
//! its description says it applies when the sketch is reopened, and the
//! points hold its effect as numbers. When the expressions do not check
//! out, every point is a number.
//!
//! # Where the parameters go
//!
//! The Customizer only reads top-level assignments of literals written
//! before the first `{` of the file (see [`c2f_lang::customizer`]), so the
//! parameters are given apart from the module: [`update_source`] puts
//! them above the file's first module or instance, and the module at the
//! end, or in place of the one it replaces.

use std::fmt;
use std::ops::Range;

use c2f_lang::ast::StmtKind;
use c2f_lang::scope::Scope;

use crate::format::{module_stmt, num, write_block};
use crate::model::{Constraint, Dimension, EntityId, Geometry, Plane, Point, Sketch, SketchError};
use crate::profile::{DEFAULT_SEGMENTS, Profile, profile};
use crate::solver::{SolveReport, solve};

/// A top-level parameter of the generated code.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub value: f64,
    /// The `//` comment above it, which the Customizer shows.
    pub description: String,
}

impl Parameter {
    /// Its lines: the description, then `name = value; // 0.1`, the
    /// trailing comment making it a Customizer spinbox stepping by 0.1.
    pub fn text(&self) -> String {
        format!("// {}\n{} = {}; // 0.1\n", self.description, self.name, num(self.value))
    }
}

/// Generated code, in its two parts: the parameters, which belong at the
/// top of the file, and the statements.
#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub parameters: Vec<Parameter>,
    pub body: String,
}

impl fmt::Display for Code {
    /// The parameters, a blank line, then the body: a file of its own.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for parameter in &self.parameters {
            f.write_str(&parameter.text())?;
        }
        if !self.parameters.is_empty() {
            f.write_str("\n")?;
        }
        f.write_str(&self.body)
    }
}

/// The module a sketch named `name` becomes: `sketch_<name>`, with any
/// character a variable name cannot hold turned into `_`.
pub fn module_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    format!("sketch_{clean}")
}

/// The sketch's code as one text, arcs cut into
/// [`DEFAULT_SEGMENTS`] per turn.
pub fn to_code(sketch: &Sketch, name: &str) -> String {
    to_code_parts(sketch, name, DEFAULT_SEGMENTS).to_string()
}

/// The sketch's code in its two parts, arcs cut into `segments` per turn.
/// The code is that of the sketch solved, when it solves.
pub fn to_code_parts(sketch: &Sketch, name: &str, segments: u32) -> Code {
    let module = module_name(name);
    let short = &module["sketch_".len()..];
    let mut solved = sketch.clone();
    let report = solve(&mut solved);
    let sketch = if report.converged() { &solved } else { sketch };
    let shape = profile(sketch, segments);
    let expressions = if report.converged() { Expressions::find(sketch, &report, &shape, segments) } else { None };

    let parameters = sketch
        .variables()
        .into_iter()
        .map(|(variable, value)| {
            let kind = sketch
                .constraints
                .iter()
                .find(|c| c.dimension().and_then(|d| d.name.as_deref()) == Some(variable.as_str()))
                .map_or("dimension", kind_name);
            let live = expressions.as_ref().is_some_and(|e| e.names.contains(&variable));
            let later = if live { "" } else { ", applied when the sketch is reopened" };
            Parameter { name: variable, value, description: format!("Sketch {short}: {kind}{later}") }
        })
        .collect();

    let mut body = format!("module {module}() {{\n");
    body.push_str("    // Drawn by the sketch editor, which reopens it from the @sketch\n");
    body.push_str("    // lines: its geometry, constraints and dimensions.\n");
    body.push_str(&write_block(sketch, "    "));
    if shape.is_empty() {
        body.push_str("    // No closed profile yet.\n");
    } else {
        let mut points = Vec::new();
        let mut paths = Vec::new();
        for (l, lp) in shape.loops.iter().enumerate() {
            let first = points.len();
            for (k, p) in lp.points.iter().enumerate() {
                let coordinate = |axis: usize, value: f64| {
                    expressions.as_ref().and_then(|e| e.write(l, k, axis)).unwrap_or_else(|| num(value))
                };
                points.push(format!("[{}, {}]", coordinate(0, p.x), coordinate(1, p.y)));
            }
            let indices: Vec<String> = (first..points.len()).map(|i| i.to_string()).collect();
            paths.push(format!("[{}]", indices.join(", ")));
        }
        body.push_str("    polygon(points = [\n");
        for p in &points {
            body.push_str(&format!("        {p},\n"));
        }
        if shape.loops.len() == 1 {
            body.push_str("    ]);\n");
        } else {
            body.push_str(&format!("    ], paths = [\n        {}\n    ]);\n", paths.join(",\n        ")));
        }
    }
    body.push_str("}\n");
    Code { parameters, body }
}

fn kind_name(constraint: &Constraint) -> &'static str {
    match constraint {
        Constraint::Distance(..) => "distance",
        Constraint::Length(..) => "length",
        Constraint::Radius(..) => "radius",
        Constraint::Diameter(..) => "diameter",
        Constraint::Angle(..) => "angle in degrees",
        _ => "dimension",
    }
}

/// The constant and one coefficient per variable of a coordinate.
type Affine = (f64, Vec<f64>);

/// The profile's points as affine expressions of the variables.
struct Expressions {
    /// The variables the points follow.
    names: Vec<String>,
    /// For each loop whose geometry is fully constrained, for each point
    /// and axis: its expression in the variables of `names`.
    terms: Vec<Option<Vec<[Affine; 2]>>>,
}

impl Expressions {
    /// Only loops whose entities are all pinned down are written as
    /// expressions: how a free point moves when a dimension changes is
    /// the solver's choice, not the sketch's.
    fn find(sketch: &Sketch, report: &SolveReport, shape: &Profile, segments: u32) -> Option<Expressions> {
        let variables = sketch.variables();
        let pinned: Vec<bool> =
            shape.loops.iter().map(|l| l.entities.iter().all(|e| report.entity_constrained[e.0])).collect();
        if variables.is_empty() || !pinned.contains(&true) {
            return None;
        }
        // The pinned loops' points, all in a row.
        let flatten = |shape: &Profile| -> Vec<Point> {
            shape.loops.iter().zip(&pinned).filter(|(_, p)| **p).flat_map(|(l, _)| l.points.iter().copied()).collect()
        };
        let base = flatten(shape);
        let scale = base.iter().fold(1.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
        let tolerance = 1e-6 * scale;
        // Those points with the variables moved by `changes`, if the
        // sketch solves and the profile keeps its shape.
        let solved = |changes: &[(usize, f64)]| -> Option<Vec<Point>> {
            let mut moved = sketch.clone();
            for &(j, delta) in changes {
                moved.set_variable(&variables[j].0, variables[j].1 + delta);
            }
            if !solve(&mut moved).converged() {
                return None;
            }
            let other = profile(&moved, segments);
            let same = other.loops.len() == shape.loops.len()
                && other.loops.iter().zip(&shape.loops).all(|(a, b)| a.points.len() == b.points.len());
            same.then(|| flatten(&other))
        };

        // Each variable's effect, kept when it is proportional.
        let mut live: Vec<(usize, f64, Vec<Point>)> = Vec::new();
        for (j, (_, value)) in variables.iter().enumerate() {
            let delta = 0.05 * value.abs().max(1.0);
            let (Some(up), Some(down)) = (solved(&[(j, delta)]), solved(&[(j, -delta)])) else { continue };
            let proportional = base.iter().zip(up.iter().zip(&down)).all(|(b, (u, d))| {
                ((u.x - b.x) + (d.x - b.x)).abs() <= tolerance && ((u.y - b.y) + (d.y - b.y)).abs() <= tolerance
            });
            if proportional {
                let slope = |u: f64, d: f64| snap((u - d) / (2.0 * delta));
                let slopes = up.iter().zip(&down).map(|(u, d)| Point::new(slope(u.x, d.x), slope(u.y, d.y)));
                live.push((j, delta, slopes.collect()));
            }
        }
        if live.is_empty() {
            return None;
        }
        // All of them at once, by different amounts.
        let changes: Vec<(usize, f64)> =
            live.iter().enumerate().map(|(i, (j, delta, _))| (*j, delta * (0.3 + 0.4 * (i % 2) as f64))).collect();
        let together = solved(&changes)?;
        for (k, (b, t)) in base.iter().zip(&together).enumerate() {
            let predicted = live.iter().zip(&changes).fold(*b, |p, ((_, _, slopes), (_, change))| {
                Point::new(p.x + slopes[k].x * change, p.y + slopes[k].y * change)
            });
            if predicted.distance(*t) > tolerance {
                return None;
            }
        }

        let mut terms = Vec::new();
        let mut k = 0;
        for (l, pinned) in shape.loops.iter().zip(&pinned) {
            if !pinned {
                terms.push(None);
                continue;
            }
            let mut loop_terms = Vec::new();
            for _ in &l.points {
                let axis = |pick: fn(&Point) -> f64| {
                    let coefficients: Vec<f64> = live.iter().map(|(_, _, s)| pick(&s[k])).collect();
                    let constant = live
                        .iter()
                        .zip(&coefficients)
                        .fold(pick(&base[k]), |c, ((j, _, _), coefficient)| c - coefficient * variables[*j].1);
                    (constant, coefficients)
                };
                loop_terms.push([axis(|p| p.x), axis(|p| p.y)]);
                k += 1;
            }
            terms.push(Some(loop_terms));
        }
        let names = live.iter().map(|(j, _, _)| variables[*j].0.clone()).collect();
        Some(Expressions { names, terms })
    }

    /// Point `k` of loop `l`'s coordinate on `axis`, as code; none when
    /// the loop is written in numbers.
    fn write(&self, l: usize, k: usize, axis: usize) -> Option<String> {
        let (constant, coefficients) = &self.terms[l].as_ref()?[k][axis];
        let mut text = String::new();
        for (name, &c) in self.names.iter().zip(coefficients) {
            if num(c) == "0" {
                continue;
            }
            let magnitude = c.abs();
            let term = if magnitude == 1.0 {
                name.clone()
            } else if magnitude == 0.5 {
                format!("{name} / 2")
            } else {
                format!("{} * {name}", num(magnitude))
            };
            text = match (text.is_empty(), c < 0.0) {
                (true, false) => term,
                (true, true) => format!("-{term}"),
                (false, false) => format!("{text} + {term}"),
                (false, true) => format!("{text} - {term}"),
            };
        }
        let constant = num(*constant);
        Some(match (text.is_empty(), constant.strip_prefix('-')) {
            (true, _) => constant,
            (false, _) if constant == "0" => text,
            (false, Some(magnitude)) => format!("{text} - {magnitude}"),
            (false, None) => format!("{text} + {constant}"),
        })
    }
}
/// A coefficient rounded to a millionth when it is that close to one:
/// the solver's last digits are noise.
fn snap(x: f64) -> f64 {
    let rounded = (x * 1e6).round() / 1e6;
    if (x - rounded).abs() < 1e-7 { rounded } else { x }
}

/// The transformation that sets a sketch's plane in space, as a prefix of
/// the statement it applies to: empty for the XY plane.
pub fn placement(plane: &Plane) -> String {
    match plane {
        Plane::Xy => String::new(),
        Plane::Xz => "rotate([90, 0, 0]) ".into(),
        Plane::Yz => "rotate([90, 0, 90]) ".into(),
        Plane::Face { origin, x_axis, normal } => {
            let y = [
                normal[1] * x_axis[2] - normal[2] * x_axis[1],
                normal[2] * x_axis[0] - normal[0] * x_axis[2],
                normal[0] * x_axis[1] - normal[1] * x_axis[0],
            ];
            let rows: Vec<String> = (0..3)
                .map(|i| format!("[{}, {}, {}, {}]", num(x_axis[i]), num(y[i]), num(normal[i]), num(origin[i])))
                .collect();
            format!("multmatrix([{}, [0, 0, 0, 1]]) ", rows.join(", "))
        }
    }
}

/// Whether an extrusion adds to the model or cuts into it.
#[derive(Clone, Debug, PartialEq)]
pub enum ExtrudeMode {
    Add,
    /// Cut out of `from`, the statement of the solid it cuts.
    Subtract { from: String },
}

/// The value of a dimension as code, and its parameter when it is named.
/// A named dimension's parameter holds its magnitude: the direction is
/// in the code around it.
fn dimension_code(d: &Dimension, description: String) -> (String, Option<Parameter>) {
    match &d.name {
        Some(name) => (name.clone(), Some(Parameter { name: name.clone(), value: d.value.abs(), description })),
        None => (num(d.value.abs()), None),
    }
}

/// `linear_extrude` of sketch `name` (as given to [`to_code`]) by
/// `height` along its plane's normal, or the other way when the height is
/// negative, as when the mouse drags it below the sketch; a named height
/// becomes a parameter.
pub fn extrude_code(name: &str, plane: &Plane, height: &Dimension, mode: &ExtrudeMode) -> Code {
    let module = module_name(name);
    let (h, parameter) = dimension_code(height, format!("Extrusion of sketch {}: height", &module[7..]));
    let flip = if height.value < 0.0 { "mirror([0, 0, 1]) " } else { "" };
    let extrusion = format!("{}{flip}linear_extrude(height = {h}) {module}();", placement(plane));
    Code { parameters: parameter.into_iter().collect(), body: combine(extrusion, mode) }
}

/// `rotate_extrude` of sketch `name` around its line `axis` (often a
/// construction line), by `angle` degrees, counterclockwise seen from the
/// axis's end. The profile must lie on one side of the axis.
pub fn revolve_code(
    sketch: &Sketch,
    name: &str,
    axis: EntityId,
    angle: &Dimension,
    mode: &ExtrudeMode,
) -> Result<Code, SketchError> {
    let Some(Geometry::Line { a, b }) = sketch.entities.get(axis.0).map(|e| e.geometry) else {
        return Err(SketchError(format!("entity {} is not a line", axis.0)));
    };
    let (a, b) = (sketch.point(a), sketch.point(b));
    let length = a.distance(b);
    if length == 0.0 {
        return Err(SketchError("the axis has no length".into()));
    }
    let u = Point::new((b.x - a.x) / length, (b.y - a.y) / length);
    let shape = profile(sketch, DEFAULT_SEGMENTS);
    let sides: Vec<f64> =
        shape.loops.iter().flat_map(|l| &l.points).map(|p| u.x * (p.y - a.y) - u.y * (p.x - a.x)).collect();
    let tolerance = 1e-9 * length.max(1.0);
    let left = sides.iter().all(|s| *s >= -tolerance);
    let right = sides.iter().all(|s| *s <= tolerance);
    if !left && !right {
        return Err(SketchError("the profile lies on both sides of the axis".into()));
    }

    // Moves the axis onto the 2D y axis with the profile on the +x side,
    // revolves, and moves the result back.
    let module = module_name(name);
    let phi = u.y.atan2(u.x).to_degrees() - 90.0;
    let rotated = phi.abs() > 1e-9;
    let moved = a.x != 0.0 || a.y != 0.0;
    let mirror = if left && !right { "mirror([1, 0, 0]) " } else { "" };
    let (degrees, parameter) = dimension_code(angle, format!("Revolution of sketch {}: angle in degrees", &module[7..]));
    let sign = if angle.value < 0.0 { "-" } else { "" };
    // Outside in: back into place after the revolution, and the profile
    // brought onto the axis before it.
    let mut back = placement(&sketch.plane);
    let mut onto = mirror.to_string();
    if moved {
        back.push_str(&format!("translate([{}, {}, 0]) ", num(a.x), num(a.y)));
    }
    if rotated {
        back.push_str(&format!("rotate([0, 0, {}]) ", num(phi)));
        onto.push_str(&format!("rotate([0, 0, {}]) ", num(-phi)));
    }
    if moved {
        onto.push_str(&format!("translate([{}, {}]) ", num(-a.x), num(-a.y)));
    }
    let revolution =
        format!("{back}{mirror}rotate([-90, 0, 0])\n    rotate_extrude(angle = {sign}{degrees}) {onto}{module}();");
    Ok(Code { parameters: parameter.into_iter().collect(), body: combine(revolution, mode) })
}

fn combine(solid: String, mode: &ExtrudeMode) -> String {
    match mode {
        ExtrudeMode::Add => format!("{solid}\n"),
        ExtrudeMode::Subtract { from } => {
            let indent = |text: &str| {
                text.lines().fold(String::new(), |mut out, line| {
                    out.push_str("    ");
                    out.push_str(line);
                    out.push('\n');
                    out
                })
            };
            format!("difference() {{\n{}{}}}\n", indent(from.trim_end()), indent(&solid))
        }
    }
}

/// Writes `parameters` into a design: a top-level assignment of the same
/// name gets the new value, keeping its comments; the others are added
/// above the file's first module, function or instance, where the
/// Customizer still reads them.
pub fn insert_parameters(src: &str, parameters: &[Parameter]) -> Result<String, SketchError> {
    let file = c2f_lang::parse(src).map_err(|d| SketchError(format!("the code does not parse: {}", d.message)))?;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let assignments = Scope::collect(&file.stmts).assignments;
    let mut added = String::new();
    for parameter in parameters {
        match assignments.iter().find(|(_, a)| a.name.name == parameter.name) {
            Some((_, a)) => edits.push((a.value.span.range(), num(parameter.value))),
            None => added.push_str(&parameter.text()),
        }
    }
    if !added.is_empty() {
        let first = file.stmts.iter().find(|s| {
            !matches!(s.kind, StmtKind::Assign(_) | StmtKind::Include(_) | StmtKind::Use(_) | StmtKind::Empty)
        });
        let at = match first {
            Some(stmt) => src[..stmt.span.start as usize].rfind('\n').map_or(0, |i| i + 1),
            None => src.len(),
        };
        if at == src.len() && !src.is_empty() && !src.ends_with('\n') {
            added.insert(0, '\n');
        }
        if first.is_some() {
            added.push('\n');
        }
        edits.push((at..at, added));
    }
    Ok(apply(src, edits))
}

/// Writes a sketch into a design: its parameters as
/// [`insert_parameters`] does, and its module in place of the one of the
/// same name, or at the end of the file.
pub fn update_source(src: &str, sketch: &Sketch, name: &str) -> Result<String, SketchError> {
    let code = to_code_parts(sketch, name, DEFAULT_SEGMENTS);
    let file = c2f_lang::parse(src).map_err(|d| SketchError(format!("the code does not parse: {}", d.message)))?;
    let module = module_name(name);
    let mut text = match module_stmt(&file, &module) {
        Some(stmt) => apply(src, vec![(stmt.span.range(), code.body.trim_end().to_string())]),
        None => {
            let gap = if src.is_empty() || src.ends_with("\n\n") {
                ""
            } else if src.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{src}{gap}{}", code.body)
        }
    };
    text = insert_parameters(&text, &code.parameters)?;
    Ok(text)
}

fn apply(src: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut text = src.to_string();
    for (range, replacement) in edits {
        text.replace_range(range, &replacement);
    }
    text
}
