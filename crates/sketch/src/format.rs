//! The `// @sketch` lines: the whole sketch written as comments inside its
//! generated module, so that the design stays one readable file and the
//! sketch reopens from it with its constraints.
//!
//! ```text
//! // @sketch v1 plane xy
//! // @sketch point p0 0 0
//! // @sketch point p1 40 0
//! // @sketch line l0 p0 p1
//! // @sketch arc a1 p2 p3 p4 construction
//! // @sketch circle c2 p5 4
//! // @sketch fixed p0
//! // @sketch horizontal l0
//! // @sketch length l0 = bracket_width 40
//! // @sketch radius c2 = 4
//! ```
//!
//! One line per item: the version and plane, then the points, the
//! entities (an arc by its center, start and end) and the constraints, in
//! the sketch's order. A dimension ends with `= value`, or `= name value`
//! when it is bound to a variable: the value is the one it had when the
//! code was written, and the variable's value in the code wins when the
//! sketch is read back. Names are free (`p`, `l`, `c` and `a` and the
//! index when written); a line that does not fit is an error, so that a
//! damaged sketch is not silently half read. Other `// @...` comments
//! belong to someone else (see [`c2f_lang::annotations`]), which ignores
//! these.

use std::collections::HashMap;

use c2f_lang::ast::{ExprKind, Stmt, StmtKind, UnaryOp};
use c2f_lang::scope::Scope;

use crate::model::{Constraint, Dimension, EntityId, Geometry, Plane, PointId, Sketch, SketchError};

pub(crate) const TAG: &str = "@sketch";
const VERSION: &str = "v1";

/// A number as the code writes it: rounded to a billionth of a
/// millimeter, without an exponent or a trailing `.0`.
pub(crate) fn num(x: f64) -> String {
    let rounded = (x * 1e9).round() / 1e9;
    if rounded == 0.0 { "0".into() } else { format!("{rounded}") }
}

fn entity_name(sketch: &Sketch, e: EntityId) -> String {
    let letter = match sketch.entities[e.0].geometry {
        Geometry::Line { .. } => 'l',
        Geometry::Circle { .. } => 'c',
        Geometry::Arc { .. } => 'a',
    };
    format!("{letter}{}", e.0)
}

/// The sketch as `// @sketch` lines, each starting with `indent`.
pub(crate) fn write_block(sketch: &Sketch, indent: &str) -> String {
    let p = |id: &PointId| format!("p{}", id.0);
    let e = |id: &EntityId| entity_name(sketch, *id);
    let dim = |d: &Dimension| match &d.name {
        Some(name) => format!("= {name} {}", num(d.value)),
        None => format!("= {}", num(d.value)),
    };
    let mut lines = Vec::new();
    lines.push(match sketch.plane {
        Plane::Xy => format!("{VERSION} plane xy"),
        Plane::Xz => format!("{VERSION} plane xz"),
        Plane::Yz => format!("{VERSION} plane yz"),
        Plane::Face { origin, x_axis, normal } => {
            let all: Vec<String> = [origin, x_axis, normal].iter().flatten().map(|v| num(*v)).collect();
            format!("{VERSION} plane face {}", all.join(" "))
        }
    });
    for (i, point) in sketch.points.iter().enumerate() {
        lines.push(format!("point p{i} {} {}", num(point.x), num(point.y)));
    }
    for (i, entity) in sketch.entities.iter().enumerate() {
        let name = e(&EntityId(i));
        let mut line = match entity.geometry {
            Geometry::Line { a, b } => format!("line {name} {} {}", p(&a), p(&b)),
            Geometry::Circle { center, radius } => format!("circle {name} {} {}", p(&center), num(radius)),
            Geometry::Arc { center, start, end } => format!("arc {name} {} {} {}", p(&center), p(&start), p(&end)),
        };
        if entity.construction {
            line.push_str(" construction");
        }
        lines.push(line);
    }
    for constraint in &sketch.constraints {
        lines.push(match constraint {
            Constraint::Coincident(a, b) => format!("coincident {} {}", p(a), p(b)),
            Constraint::Horizontal(l) => format!("horizontal {}", e(l)),
            Constraint::Vertical(l) => format!("vertical {}", e(l)),
            Constraint::Parallel(a, b) => format!("parallel {} {}", e(a), e(b)),
            Constraint::Perpendicular(a, b) => format!("perpendicular {} {}", e(a), e(b)),
            Constraint::Tangent(a, b) => format!("tangent {} {}", e(a), e(b)),
            Constraint::Equal(a, b) => format!("equal {} {}", e(a), e(b)),
            Constraint::Distance(a, b, d) => format!("distance {} {} {}", p(a), p(b), dim(d)),
            Constraint::Length(l, d) => format!("length {} {}", e(l), dim(d)),
            Constraint::Radius(c, d) => format!("radius {} {}", e(c), dim(d)),
            Constraint::Diameter(c, d) => format!("diameter {} {}", e(c), dim(d)),
            Constraint::Angle(a, b, d) => format!("angle {} {} {}", e(a), e(b), dim(d)),
            Constraint::PointOnLine(q, l) => format!("on_line {} {}", p(q), e(l)),
            Constraint::PointOnCircle(q, c) => format!("on_circle {} {}", p(q), e(c)),
            Constraint::Midpoint(q, l) => format!("midpoint {} {}", p(q), e(l)),
            Constraint::Fixed(q) => format!("fixed {}", p(q)),
        });
    }
    lines.iter().fold(String::new(), |mut out, line| {
        out.push_str(&format!("{indent}// {TAG} {line}\n"));
        out
    })
}

/// Reads `// @sketch` lines back (each line's text after the tag).
pub(crate) fn read_block<'a>(lines: impl IntoIterator<Item = &'a str>) -> Result<Sketch, SketchError> {
    let mut reader = Reader { sketch: Sketch::default(), points: HashMap::new(), entities: HashMap::new() };
    let mut seen_header = false;
    for (n, line) in lines.into_iter().enumerate() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let result = if seen_header { reader.item(&words) } else { reader.header(&words) };
        seen_header = true;
        result.map_err(|why| SketchError(format!("@sketch line {}: {why}: `{}`", n + 1, line.trim())))?;
    }
    if !seen_header {
        return Err(SketchError("no @sketch lines".into()));
    }
    Ok(reader.sketch)
}

struct Reader {
    sketch: Sketch,
    points: HashMap<String, PointId>,
    entities: HashMap<String, EntityId>,
}

fn number(word: &str) -> Result<f64, String> {
    word.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("`{word}` is not a number"))
}

fn dimension(rest: &[&str]) -> Result<Dimension, String> {
    match rest {
        ["=", value] => Ok(Dimension::new(number(value)?)),
        ["=", name, value] if crate::model::is_identifier(name) => Ok(Dimension::named(name, number(value)?)),
        _ => Err("expected `= value` or `= name value`".into()),
    }
}

impl Reader {
    fn header(&mut self, words: &[&str]) -> Result<(), String> {
        let (version, args) = words.split_first().expect("not empty");
        if *version != VERSION {
            return Err(format!("expected `{VERSION}` first"));
        }
        self.sketch.plane = match args {
            [] | ["plane", "xy"] => Plane::Xy,
            ["plane", "xz"] => Plane::Xz,
            ["plane", "yz"] => Plane::Yz,
            ["plane", "face", v @ ..] if v.len() == 9 => {
                let v: Vec<f64> = v.iter().map(|w| number(w)).collect::<Result<_, _>>()?;
                Plane::Face { origin: [v[0], v[1], v[2]], x_axis: [v[3], v[4], v[5]], normal: [v[6], v[7], v[8]] }
            }
            _ => return Err("unknown plane".into()),
        };
        Ok(())
    }

    fn point(&self, word: &str) -> Result<PointId, String> {
        self.points.get(word).copied().ok_or_else(|| format!("no point `{word}`"))
    }

    fn entity(&self, word: &str) -> Result<EntityId, String> {
        self.entities.get(word).copied().ok_or_else(|| format!("no entity `{word}`"))
    }

    fn add_entity(&mut self, name: &str, geometry: Geometry, construction: bool) -> Result<(), String> {
        let id = self.sketch.add_entity(geometry);
        self.sketch.set_construction(id, construction);
        match self.entities.insert(name.to_string(), id) {
            Some(_) => Err(format!("`{name}` named twice")),
            None => Ok(()),
        }
    }

    fn item(&mut self, words: &[&str]) -> Result<(), String> {
        let (&keyword, args) = words.split_first().expect("not empty");
        let (args, construction) = match args.split_last() {
            Some((&"construction", rest)) => (rest, true),
            _ => (args, false),
        };
        let constraint = match (keyword, args) {
            ("point", [name, x, y]) => {
                let id = self.sketch.add_point(number(x)?, number(y)?);
                return match self.points.insert(name.to_string(), id) {
                    Some(_) => Err(format!("`{name}` named twice")),
                    None if construction => Err("only entities are construction geometry".into()),
                    None => Ok(()),
                };
            }
            ("line", [name, a, b]) => {
                let geometry = Geometry::Line { a: self.point(a)?, b: self.point(b)? };
                return self.add_entity(name, geometry, construction);
            }
            ("circle", [name, c, r]) => {
                let geometry = Geometry::Circle { center: self.point(c)?, radius: number(r)? };
                return self.add_entity(name, geometry, construction);
            }
            ("arc", [name, c, s, t]) => {
                let geometry = Geometry::Arc { center: self.point(c)?, start: self.point(s)?, end: self.point(t)? };
                return self.add_entity(name, geometry, construction);
            }
            ("coincident", [a, b]) => Constraint::Coincident(self.point(a)?, self.point(b)?),
            ("horizontal", [l]) => Constraint::Horizontal(self.entity(l)?),
            ("vertical", [l]) => Constraint::Vertical(self.entity(l)?),
            ("parallel", [a, b]) => Constraint::Parallel(self.entity(a)?, self.entity(b)?),
            ("perpendicular", [a, b]) => Constraint::Perpendicular(self.entity(a)?, self.entity(b)?),
            ("tangent", [a, b]) => Constraint::Tangent(self.entity(a)?, self.entity(b)?),
            ("equal", [a, b]) => Constraint::Equal(self.entity(a)?, self.entity(b)?),
            ("distance", [a, b, rest @ ..]) => Constraint::Distance(self.point(a)?, self.point(b)?, dimension(rest)?),
            ("length", [l, rest @ ..]) => Constraint::Length(self.entity(l)?, dimension(rest)?),
            ("radius", [c, rest @ ..]) => Constraint::Radius(self.entity(c)?, dimension(rest)?),
            ("diameter", [c, rest @ ..]) => Constraint::Diameter(self.entity(c)?, dimension(rest)?),
            ("angle", [a, b, rest @ ..]) => Constraint::Angle(self.entity(a)?, self.entity(b)?, dimension(rest)?),
            ("on_line", [q, l]) => Constraint::PointOnLine(self.point(q)?, self.entity(l)?),
            ("on_circle", [q, c]) => Constraint::PointOnCircle(self.point(q)?, self.entity(c)?),
            ("midpoint", [q, l]) => Constraint::Midpoint(self.point(q)?, self.entity(l)?),
            ("fixed", [q]) => Constraint::Fixed(self.point(q)?),
            _ => return Err("not understood".into()),
        };
        if construction {
            return Err("only entities are construction geometry".into());
        }
        let id = self.sketch.add_constraint(constraint);
        self.sketch.check(id).map_err(|e| e.0)
    }
}

/// The module named `name` among a file's statements.
fn find_module<'f>(stmts: &'f [Stmt], name: &str) -> Option<&'f Stmt> {
    stmts.iter().find_map(|stmt| match &stmt.kind {
        StmtKind::Module(m) if m.name.name == name => Some(stmt),
        StmtKind::Block(inner) => find_module(inner, name),
        _ => None,
    })
}

/// The `// @sketch` lines in `text`: what follows each tag.
fn tagged_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.trim_start().strip_prefix("//")?.trim_start().strip_prefix(TAG))
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        .collect()
}

/// The module of a sketch: `name` as given, or `sketch_<name>`.
pub(crate) fn module_stmt<'f>(file: &'f c2f_lang::ast::File, name: &str) -> Option<&'f Stmt> {
    find_module(&file.stmts, name).or_else(|| find_module(&file.stmts, &format!("sketch_{name}")))
}

/// Reads back the sketch a design holds in its module `module_name`
/// (`sketch_l_bracket`, or just `l_bracket`), from the module's
/// `// @sketch` lines. Dimensions bound to a variable take the value the
/// file gives it, when that is a plain number, as after editing it in the
/// code or the Customizer; the geometry still stands as written, and
/// follows the new values at the next [`crate::solve`].
pub fn from_code(src: &str, module_name: &str) -> Result<Sketch, SketchError> {
    let file = c2f_lang::parse(src).map_err(|d| SketchError(format!("the code does not parse: {}", d.message)))?;
    let stmt = module_stmt(&file, module_name).ok_or_else(|| SketchError(format!("no module `{module_name}`")))?;
    let mut sketch = read_block(tagged_lines(&src[stmt.span.range()]))?;
    for (name, value) in variables(&file) {
        sketch.set_variable(&name, value);
    }
    Ok(sketch)
}

/// The modules of a design that hold a sketch, by name, for the design
/// tree to list and reopen.
pub fn sketches_in(src: &str) -> Vec<String> {
    let Ok(file) = c2f_lang::parse(src) else { return Vec::new() };
    Scope::collect(&file.stmts)
        .modules
        .iter()
        .filter(|m| !tagged_lines(&src[m.body.span.range()]).is_empty())
        .map(|m| m.name.name.clone())
        .collect()
}

/// The file's top-level variables set to a plain number (a repeated name:
/// its last value, as the language keeps it).
fn variables(file: &c2f_lang::ast::File) -> Vec<(String, f64)> {
    fn number(kind: &ExprKind) -> Option<f64> {
        match kind {
            ExprKind::Number(n) => Some(*n),
            ExprKind::Paren(inner) => number(&inner.kind),
            ExprKind::Unary { op: UnaryOp::Neg, operand } => number(&operand.kind).map(|n| -n),
            ExprKind::Unary { op: UnaryOp::Plus, operand } => number(&operand.kind),
            _ => None,
        }
    }
    Scope::collect(&file.stmts)
        .assignments
        .iter()
        .filter_map(|(_, a)| Some((a.name.name.clone(), number(&a.value.kind)?)))
        .collect()
}
