//! What a sketch is made of: points, the lines, circles and arcs drawn
//! between them, and the constraints and dimensions that hold them.
//!
//! Entities refer to their points by id, so that two lines meeting at a
//! corner can share the corner's point: they then meet by construction,
//! with no [`Constraint::Coincident`] to solve. Ids are indices into the
//! sketch's lists, handed out in order and never reused while the sketch
//! lives; [`crate::from_code`] numbers them afresh.

use std::fmt;

/// A point of the sketch, by its index in [`Sketch::points`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PointId(pub usize);

/// A line, circle or arc, by its index in [`Sketch::entities`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub usize);

/// A constraint, by its index in [`Sketch::constraints`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstraintId(pub usize);

/// A position in the sketch's plane, in millimeters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn distance(self, other: Point) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn midpoint(self, other: Point) -> Point {
        Point::new((self.x + other.x) / 2.0, (self.y + other.y) / 2.0)
    }
}

/// The shape of an entity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geometry {
    /// A segment from `a` to `b`.
    Line { a: PointId, b: PointId },
    /// A full circle. Its radius is solved along with the points.
    Circle { center: PointId, radius: f64 },
    /// An arc of a circle around `center`, counterclockwise from `start`
    /// to `end`. Its radius is the distance from the center to `start`,
    /// and the solver keeps `end` at the same distance.
    Arc { center: PointId, start: PointId, end: PointId },
}

/// A line, circle or arc. Construction geometry guides the drawing (an
/// axis, a center line) and is left out of the profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entity {
    pub geometry: Geometry,
    pub construction: bool,
}

/// The value of a dimension, and the variable it is bound to: a named
/// dimension becomes a top-level parameter of the generated code, which
/// the Customizer shows and which reopening the sketch reads back.
/// Several dimensions bound to one name share its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    /// Millimeters, or degrees for an angle.
    pub value: f64,
    pub name: Option<String>,
}

impl Dimension {
    /// An unnamed dimension: a number in the code.
    pub fn new(value: f64) -> Self {
        Dimension { value, name: None }
    }

    /// A dimension bound to the variable `name`.
    pub fn named(name: &str, value: f64) -> Self {
        Dimension { value, name: Some(name.to_string()) }
    }
}

impl From<f64> for Dimension {
    fn from(value: f64) -> Self {
        Dimension::new(value)
    }
}

/// What holds the geometry. Constraints between two lines or two round
/// entities read their arguments in order; tangency and equality take
/// any mix of their kinds, as listed.
#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    /// Two points at the same place.
    Coincident(PointId, PointId),
    /// A line parallel to the x axis.
    Horizontal(EntityId),
    /// A line parallel to the y axis.
    Vertical(EntityId),
    Parallel(EntityId, EntityId),
    Perpendicular(EntityId, EntityId),
    /// A line and a circle or arc, or two circles or arcs, touching
    /// without crossing. Which side (or, for two round entities, whether
    /// one is inside the other) is the one the geometry is closest to
    /// when the solver starts.
    Tangent(EntityId, EntityId),
    /// Two lines of the same length, or two circles or arcs of the same
    /// radius.
    Equal(EntityId, EntityId),
    /// The distance between two points.
    Distance(PointId, PointId, Dimension),
    /// The length of a line.
    Length(EntityId, Dimension),
    /// The radius of a circle or arc.
    Radius(EntityId, Dimension),
    /// The diameter of a circle or arc.
    Diameter(EntityId, Dimension),
    /// The angle in degrees from the first line's direction (`a` to `b`)
    /// to the second's, counterclockwise.
    Angle(EntityId, EntityId, Dimension),
    /// A point on a line, extended past its ends.
    PointOnLine(PointId, EntityId),
    /// A point on a circle, or on the full circle of an arc.
    PointOnCircle(PointId, EntityId),
    /// A point halfway along a line.
    Midpoint(PointId, EntityId),
    /// A point that stays where it is.
    Fixed(PointId),
}

impl Constraint {
    /// The dimension this constraint sets, if it is one.
    pub fn dimension(&self) -> Option<&Dimension> {
        match self {
            Constraint::Distance(_, _, d)
            | Constraint::Length(_, d)
            | Constraint::Radius(_, d)
            | Constraint::Diameter(_, d)
            | Constraint::Angle(_, _, d) => Some(d),
            _ => None,
        }
    }

    pub fn dimension_mut(&mut self) -> Option<&mut Dimension> {
        match self {
            Constraint::Distance(_, _, d)
            | Constraint::Length(_, d)
            | Constraint::Radius(_, d)
            | Constraint::Diameter(_, d)
            | Constraint::Angle(_, _, d) => Some(d),
            _ => None,
        }
    }
}

/// The plane a sketch is drawn on. The sketch's x and y map to the
/// plane's axes and its normal is the direction extrusions go.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Plane {
    /// x to x, y to y; extrusions go up (+z).
    #[default]
    Xy,
    /// x to x, y to z; extrusions go toward -y, as `rotate([90, 0, 0])`
    /// turns the z axis.
    Xz,
    /// x to y, y to z; extrusions go toward +x.
    Yz,
    /// A planar face: its origin, the direction of the sketch's x axis and
    /// the face's normal (both unit vectors, perpendicular). The y axis is
    /// `normal × x_axis`.
    Face { origin: [f64; 3], x_axis: [f64; 3], normal: [f64; 3] },
}

/// A sketch: its plane, its points, entities and constraints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sketch {
    pub plane: Plane,
    pub points: Vec<Point>,
    pub entities: Vec<Entity>,
    pub constraints: Vec<Constraint>,
}

impl Sketch {
    pub fn new(plane: Plane) -> Self {
        Sketch { plane, ..Sketch::default() }
    }

    pub fn add_point(&mut self, x: f64, y: f64) -> PointId {
        self.points.push(Point::new(x, y));
        PointId(self.points.len() - 1)
    }

    pub fn add_entity(&mut self, geometry: Geometry) -> EntityId {
        self.entities.push(Entity { geometry, construction: false });
        EntityId(self.entities.len() - 1)
    }

    pub fn add_line(&mut self, a: PointId, b: PointId) -> EntityId {
        self.add_entity(Geometry::Line { a, b })
    }

    pub fn add_circle(&mut self, center: PointId, radius: f64) -> EntityId {
        self.add_entity(Geometry::Circle { center, radius })
    }

    pub fn add_arc(&mut self, center: PointId, start: PointId, end: PointId) -> EntityId {
        self.add_entity(Geometry::Arc { center, start, end })
    }

    pub fn add_constraint(&mut self, constraint: Constraint) -> ConstraintId {
        self.constraints.push(constraint);
        ConstraintId(self.constraints.len() - 1)
    }

    /// Lines through `corners` in order, each sharing its end point with
    /// the next line's start; `closed` adds the line back to the first.
    pub fn add_polyline(&mut self, corners: &[Point], closed: bool) -> Vec<EntityId> {
        let ids: Vec<PointId> = corners.iter().map(|p| self.add_point(p.x, p.y)).collect();
        let mut lines: Vec<EntityId> = ids.windows(2).map(|w| self.add_line(w[0], w[1])).collect();
        if closed && ids.len() > 2 {
            lines.push(self.add_line(ids[ids.len() - 1], ids[0]));
        }
        lines
    }

    /// A rectangle with sides along the axes, from one corner to the
    /// opposite one: its four lines (bottom, right, top, left when `a` is
    /// the lower left corner), held horizontal and vertical.
    pub fn add_rectangle(&mut self, a: Point, b: Point) -> [EntityId; 4] {
        let corners = [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)];
        let lines = self.add_polyline(&corners, true);
        for (i, &line) in lines.iter().enumerate() {
            self.add_constraint(if i % 2 == 0 { Constraint::Horizontal(line) } else { Constraint::Vertical(line) });
        }
        [lines[0], lines[1], lines[2], lines[3]]
    }

    pub fn point(&self, id: PointId) -> Point {
        self.points[id.0]
    }

    pub fn entity(&self, id: EntityId) -> &Entity {
        &self.entities[id.0]
    }

    pub fn set_construction(&mut self, id: EntityId, construction: bool) {
        self.entities[id.0].construction = construction;
    }

    /// The points an entity is drawn through: a line's ends, a circle's
    /// center, an arc's center, start and end.
    pub fn entity_points(&self, id: EntityId) -> Vec<PointId> {
        match self.entities[id.0].geometry {
            Geometry::Line { a, b } => vec![a, b],
            Geometry::Circle { center, .. } => vec![center],
            Geometry::Arc { center, start, end } => vec![center, start, end],
        }
    }

    /// The radius of a circle or arc.
    pub fn radius(&self, id: EntityId) -> Option<f64> {
        match self.entities[id.0].geometry {
            Geometry::Line { .. } => None,
            Geometry::Circle { radius, .. } => Some(radius),
            Geometry::Arc { center, start, .. } => Some(self.point(center).distance(self.point(start))),
        }
    }

    /// The variables the named dimensions are bound to, each once, in the
    /// order they first appear, with the value of the first dimension
    /// bound to it.
    pub fn variables(&self) -> Vec<(String, f64)> {
        let mut variables: Vec<(String, f64)> = Vec::new();
        for d in self.constraints.iter().filter_map(Constraint::dimension) {
            if let Some(name) = &d.name {
                if !variables.iter().any(|(n, _)| n == name) {
                    variables.push((name.clone(), d.value));
                }
            }
        }
        variables
    }

    /// Sets every dimension bound to `name`; whether there was one. The
    /// geometry follows at the next [`crate::solve`].
    pub fn set_variable(&mut self, name: &str, value: f64) -> bool {
        let mut found = false;
        for d in self.constraints.iter_mut().filter_map(Constraint::dimension_mut) {
            if d.name.as_deref() == Some(name) {
                d.value = value;
                found = true;
            }
        }
        found
    }

    /// What is wrong with a constraint's arguments: an id past the end, or
    /// an entity of the wrong kind. The solver skips such constraints.
    pub fn check(&self, id: ConstraintId) -> Result<(), SketchError> {
        let bad = |why: &str| Err(SketchError(format!("constraint {}: {why}", id.0)));
        let point = |p: &PointId| p.0 < self.points.len();
        let kind = |e: &EntityId| self.entities.get(e.0).map(|e| Kind::of(&e.geometry));
        let line = |e: &EntityId| kind(e) == Some(Kind::Line);
        let round = |e: &EntityId| matches!(kind(e), Some(Kind::Round));
        let ok = match &self.constraints[id.0] {
            Constraint::Coincident(a, b) | Constraint::Distance(a, b, _) => point(a) && point(b) && a != b,
            Constraint::Fixed(p) => point(p),
            Constraint::Horizontal(e) | Constraint::Vertical(e) | Constraint::Length(e, _) => line(e),
            Constraint::Parallel(a, b) | Constraint::Perpendicular(a, b) | Constraint::Angle(a, b, _) => {
                line(a) && line(b) && a != b
            }
            Constraint::Tangent(a, b) => (line(a) || round(a)) && round(b) && a != b,
            Constraint::Equal(a, b) => a != b && kind(a).is_some() && kind(a) == kind(b),
            Constraint::Radius(e, _) | Constraint::Diameter(e, _) => round(e),
            Constraint::PointOnLine(p, e) | Constraint::Midpoint(p, e) => {
                point(p) && line(e) && !self.entity_points(*e).contains(p)
            }
            // On its own center, a circle could only shrink to nothing.
            Constraint::PointOnCircle(p, e) => point(p) && round(e) && self.entity_points(*e)[0] != *p,
        };
        if !ok {
            return bad("its arguments do not fit it");
        }
        if let Some(d) = self.constraints[id.0].dimension() {
            if !d.value.is_finite() {
                return bad("its value is not a number");
            }
            if let Some(name) = &d.name {
                if !is_identifier(name) {
                    return bad(&format!("`{name}` cannot name a variable"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Line,
    Round,
}

impl Kind {
    pub(crate) fn of(geometry: &Geometry) -> Kind {
        match geometry {
            Geometry::Line { .. } => Kind::Line,
            Geometry::Circle { .. } | Geometry::Arc { .. } => Kind::Round,
        }
    }
}

/// Whether `name` can be a variable of the .scad language: plain ASCII
/// letters, digits and `_`, not starting with a digit, and read by the
/// language's own lexer as one identifier. Asking the lexer rather than
/// listing the keywords here keeps the two from drifting apart: a
/// dimension named `for` would write `for = 30;`, which does not parse.
pub fn is_identifier(name: &str) -> bool {
    use c2f_lang::lexer::{Tok, tokenize};
    let mut chars = name.chars();
    let shape = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    shape
        && tokenize(name).is_ok_and(|tokens| {
            matches!(tokens.as_slice(), [first, last] if first.tok == Tok::Ident(name.to_string()) && last.tok == Tok::Eof)
        })
}

/// A sketch that cannot be used as it is, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchError(pub String);

impl fmt::Display for SketchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SketchError {}
