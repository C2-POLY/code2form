//! The sketch mode's logic without its window (C2F-42): where the
//! sketch's plane lies in space and how the camera faces it, the drawing
//! tools as a state machine fed with clicks, what the pointer is over,
//! the constraints and dimensions the palette makes of a selection, and
//! the code a finished sketch writes into the design.
//!
//! The sketch itself, its solver and its code generator live in the
//! `c2f-sketch` crate; `app/sketch.rs` draws all this over the 3D view
//! and turns the pointer and the keys into calls here.
//!
//! Coordinates in the sketch are millimeters in its plane (`Point`, f64);
//! in space they are the renderer's `Vec3` (f32).

use std::ops::Range;

use c2f_sketch::{
    Code, Constraint, Dimension, EntityId, Geometry, Plane, Point, PointId, Sketch, Snap, SnapKind, insert_parameters,
    module_name, update_source,
};
use glam::Vec3;

use crate::camera::Camera;
use crate::code_edit::{self, Place};

/// How far from the horizontal or the vertical, as a slope, a line drawn
/// still counts as horizontal or vertical, and is held so: about 3°.
const AUTO_AXIS_SLOPE: f64 = 0.055;

/// The steepest the camera looks down: the orbit camera's limit, as a
/// view straight down the Z axis has no "up" to keep.
const MAX_PITCH: f32 = 89.5;

// ---- the plane in space ----

/// A sketch's plane in space: its origin, its x and y axes and its
/// normal, all unit vectors, `y = normal × x`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneFrame {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub normal: Vec3,
}

impl PlaneFrame {
    /// The frame of a sketch plane, as the generated code places it (see
    /// `c2f_sketch::placement`).
    pub fn of(plane: &Plane) -> Self {
        let frame = |origin: Vec3, x: Vec3, normal: Vec3| PlaneFrame { origin, x, y: normal.cross(x), normal };
        match *plane {
            Plane::Xy => frame(Vec3::ZERO, Vec3::X, Vec3::Z),
            Plane::Xz => frame(Vec3::ZERO, Vec3::X, -Vec3::Y),
            Plane::Yz => frame(Vec3::ZERO, Vec3::Y, Vec3::X),
            Plane::Face { origin, x_axis, normal } => frame(vec3(origin), vec3(x_axis), vec3(normal)),
        }
    }

    /// Where a point of the sketch is in space.
    pub fn to_world(self, p: Point) -> Vec3 {
        self.origin + self.x * p.x as f32 + self.y * p.y as f32
    }

    /// The point of the sketch a point in space projects onto.
    pub fn to_plane(self, v: Vec3) -> Point {
        let d = v - self.origin;
        Point::new(f64::from(d.dot(self.x)), f64::from(d.dot(self.y)))
    }

    /// Where a ray (as the pointer casts it) crosses the plane; `None`
    /// when it runs along it or crosses behind its origin.
    pub fn intersect(&self, origin: Vec3, direction: Vec3) -> Option<Point> {
        let across = direction.dot(self.normal);
        if across.abs() < 1e-6 {
            return None;
        }
        let t = (self.origin - origin).dot(self.normal) / across;
        (t > 0.0).then(|| self.to_plane(origin + direction * t))
    }

    /// The camera looking at the plane face on from the side its normal
    /// points to, the sketch's x axis to the right, centered on `center`
    /// of the sketch, from `distance`.
    pub fn face_on(&self, center: Point, distance: f32, fov_y: f32) -> Camera {
        let (yaw, pitch) = yaw_pitch(self.normal);
        Camera { target: self.to_world(center), distance, yaw, pitch, fov_y }
    }

    /// The camera seeing the plane from three quarters, as extruding
    /// wants: tilted down 35° and turned 25° from face on.
    pub fn three_quarters(&self, center: Point, distance: f32, fov_y: f32) -> Camera {
        let (s, c) = 35f32.to_radians().sin_cos();
        let (st, ct) = 25f32.to_radians().sin_cos();
        let back = (self.normal * c - self.y * s) * ct + self.x * st;
        let (yaw, pitch) = yaw_pitch(back.normalize_or(self.normal));
        Camera { target: self.to_world(center), distance, yaw, pitch, fov_y }
    }
}

fn vec3(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32)
}

/// The orbit camera's angles for a camera looking back along `back`.
/// Straight up or down, the yaw puts +X to the right of the screen.
fn yaw_pitch(back: Vec3) -> (f32, f32) {
    let pitch = back.z.clamp(-1.0, 1.0).asin().to_degrees().clamp(-MAX_PITCH, MAX_PITCH);
    let flat = back.x.hypot(back.y);
    let yaw = if flat < 1e-4 { -90.0 } else { back.y.atan2(back.x).to_degrees() };
    (yaw.to_radians(), pitch.to_radians())
}

/// The plane of a flat face, at `point` with this `normal` (facing the
/// eye, as the 3D view gives it): its x axis is the screen's right when
/// the camera faces it, as [`PlaneFrame::face_on`] turns it. Values a hair
/// off a round number are rounded, so that the code reads `[0, 0, 1]`.
pub fn face_plane(point: Vec3, normal: Vec3) -> Plane {
    let normal = clean_axis(normal.normalize_or(Vec3::Z));
    let x = if normal.x.hypot(normal.y) < 1e-4 { Vec3::X } else { clean_axis((-normal).cross(Vec3::Z).normalize()) };
    let round = |v: f32| f64::from((v * 1000.0).round() / 1000.0);
    let wide = |v: Vec3| [f64::from(v.x), f64::from(v.y), f64::from(v.z)];
    Plane::Face { origin: [round(point.x), round(point.y), round(point.z)], x_axis: wide(x), normal: wide(normal) }
}

/// A unit vector with components close to 0 or ±1 made exact.
fn clean_axis(v: Vec3) -> Vec3 {
    let snap = |c: f32| {
        if c.abs() < 1e-4 {
            0.0
        } else if (c.abs() - 1.0).abs() < 1e-4 {
            c.signum()
        } else {
            c
        }
    };
    Vec3::new(snap(v.x), snap(v.y), snap(v.z)).normalize_or(v)
}

// ---- what the pointer is over ----

/// Something of the sketch a click selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Point(PointId),
    Entity(EntityId),
}

/// The distance from `at` to an entity, as drawn: to a segment, a circle,
/// or an arc between its ends.
pub fn distance_to(sketch: &Sketch, entity: EntityId, at: Point) -> f64 {
    match sketch.entity(entity).geometry {
        Geometry::Line { a, b } => segment_distance(sketch.point(a), sketch.point(b), at),
        Geometry::Circle { center, radius } => (sketch.point(center).distance(at) - radius).abs(),
        Geometry::Arc { center, start, end } => {
            let (c, s, e) = (sketch.point(center), sketch.point(start), sketch.point(end));
            let r = c.distance(s);
            let angle = |p: Point| (p.y - c.y).atan2(p.x - c.x);
            let sweep = ccw(angle(e) - angle(s));
            if ccw(angle(at) - angle(s)) <= sweep {
                (c.distance(at) - r).abs()
            } else {
                s.distance(at).min(e.distance(at))
            }
        }
    }
}

/// An angle brought into 0..2π, a full turn when it is 0 (an arc from a
/// point back to it).
fn ccw(angle: f64) -> f64 {
    let a = angle.rem_euclid(std::f64::consts::TAU);
    if a <= 1e-12 { std::f64::consts::TAU } else { a }
}

fn segment_distance(a: Point, b: Point, p: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length2 = dx * dx + dy * dy;
    if length2 == 0.0 {
        return a.distance(p);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length2).clamp(0.0, 1.0);
    Point::new(a.x + t * dx, a.y + t * dy).distance(p)
}

/// What a click at `at` selects: the nearest point within `tolerance`,
/// or else the nearest entity within it.
pub fn hit(sketch: &Sketch, at: Point, tolerance: f64) -> Option<Item> {
    let nearest = |items: Vec<(Item, f64)>| {
        items.into_iter().filter(|(_, d)| *d <= tolerance).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(item, _)| item)
    };
    let points = (0..sketch.points.len()).map(|i| (Item::Point(PointId(i)), sketch.points[i].distance(at))).collect();
    nearest(points).or_else(|| {
        let entities = (0..sketch.entities.len())
            .map(|e| (Item::Entity(EntityId(e)), distance_to(sketch, EntityId(e), at)))
            .collect();
        nearest(entities)
    })
}

// ---- drawing ----

/// The drawing tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Selects points and entities, and drags them.
    Select,
    /// Lines end to end, each starting where the last one ended, until a
    /// click lands on a point already drawn (closing the outline), a
    /// double-click, Enter or Esc.
    Line,
    /// Two opposite corners.
    Rectangle,
    /// The center, then a point on the circle.
    Circle,
    /// The center, the start, then the end, counterclockwise.
    Arc,
}

impl Tool {
    pub const ALL: [Tool; 5] = [Tool::Select, Tool::Line, Tool::Rectangle, Tool::Circle, Tool::Arc];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Arc => "Arc",
        }
    }

    /// What the next click does, for the hint under the view.
    pub fn hint(self, clicks: usize) -> &'static str {
        match (self, clicks) {
            (Tool::Select, _) => "Click to select, Shift-click to add · drag a point or a line to move it",
            (Tool::Line, 0) => "Click the start of the line",
            (Tool::Line, _) => "Click the next corner · click a point to close · double-click, Enter or Esc to stop",
            (Tool::Rectangle, 0) => "Click a corner",
            (Tool::Rectangle, _) => "Click the opposite corner",
            (Tool::Circle, 0) => "Click the center",
            (Tool::Circle, _) => "Click a point on the circle",
            (Tool::Arc, 0) => "Click the center",
            (Tool::Arc, 1) => "Click the start of the arc",
            (Tool::Arc, _) => "Click the end of the arc (counterclockwise)",
        }
    }
}

/// Where a click lands: the point in the plane and what it snapped to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub at: Point,
    pub snap: Option<Target>,
}

/// What a click snapped to: a point, center or midpoint of the sketch
/// (see `c2f_sketch::snap`), or the plane's origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Sketch(Snap),
    Origin,
}

/// The pick at `at`: the snap within `tolerance` if any, the origin when
/// nothing of the sketch is in reach.
pub fn pick(sketch: &Sketch, at: Point, tolerance: f64) -> Pick {
    match c2f_sketch::snap(sketch, at, tolerance) {
        Some(snap) => Pick { at: snap.at, snap: Some(Target::Sketch(snap)) },
        None if at.distance(Point::default()) <= tolerance => Pick { at: Point::default(), snap: Some(Target::Origin) },
        None => Pick { at, snap: None },
    }
}

/// A point a tool has placed, before the entity it belongs to is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    /// A point of the sketch, shared.
    Existing(PointId),
    /// A new point, free.
    Free(Point),
    /// A new point held halfway along a line, or (on an arc) just there.
    Midpoint(Point, EntityId),
    /// A new point fixed at the plane's origin.
    Origin,
}

impl Anchor {
    fn of(pick: &Pick) -> Self {
        match pick.snap {
            Some(Target::Sketch(Snap { point: Some(p), kind: SnapKind::Endpoint | SnapKind::Center, .. })) => {
                Anchor::Existing(p)
            }
            Some(Target::Sketch(Snap { kind: SnapKind::Midpoint, entity: Some(e), at, .. })) => Anchor::Midpoint(at, e),
            Some(Target::Origin) => Anchor::Origin,
            _ => Anchor::Free(pick.at),
        }
    }

    fn at(&self, sketch: &Sketch) -> Point {
        match *self {
            Anchor::Existing(p) => sketch.point(p),
            Anchor::Free(at) | Anchor::Midpoint(at, _) => at,
            Anchor::Origin => Point::default(),
        }
    }

    /// The anchor as a point of the sketch, adding it (and what holds it)
    /// when it is new.
    fn place(&self, sketch: &mut Sketch) -> PointId {
        match *self {
            Anchor::Existing(p) => p,
            Anchor::Free(at) => sketch.add_point(at.x, at.y),
            Anchor::Midpoint(at, e) => {
                let p = sketch.add_point(at.x, at.y);
                if matches!(sketch.entity(e).geometry, Geometry::Line { .. }) {
                    sketch.add_constraint(Constraint::Midpoint(p, e));
                }
                p
            }
            Anchor::Origin => {
                let p = sketch.add_point(0.0, 0.0);
                sketch.add_constraint(Constraint::Fixed(p));
                p
            }
        }
    }

    /// Holds the new point `p` where the anchor says, for shapes that make
    /// their own points (a rectangle's corners).
    fn attach(&self, sketch: &mut Sketch, p: PointId) {
        match *self {
            Anchor::Existing(q) => {
                sketch.add_constraint(Constraint::Coincident(q, p));
            }
            Anchor::Midpoint(_, e) if matches!(sketch.entity(e).geometry, Geometry::Line { .. }) => {
                sketch.add_constraint(Constraint::Midpoint(p, e));
            }
            Anchor::Origin => {
                sketch.add_constraint(Constraint::Fixed(p));
            }
            _ => {}
        }
    }
}

/// A shape drawn while a tool waits for its next click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Preview {
    Line(Point, Point),
    Circle(Point, f64),
    /// Center, start and end, counterclockwise.
    Arc(Point, Point, Point),
}

/// The active tool and the points it has placed so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub tool: Tool,
    anchors: Vec<Anchor>,
}

impl Default for Drawing {
    fn default() -> Self {
        Drawing { tool: Tool::Select, anchors: Vec::new() }
    }
}

impl Drawing {
    /// Switches tools, dropping what the last one had placed.
    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.anchors.clear();
    }

    /// The clicks placed so far in the shape being drawn.
    pub fn clicks(&self) -> usize {
        self.anchors.len()
    }

    /// Drops the shape being drawn; whether there was one.
    pub fn cancel(&mut self) -> bool {
        !std::mem::take(&mut self.anchors).is_empty()
    }

    /// A click of the tool at `pick`: adds to the sketch the entities it
    /// completes, with the constraints that come with them (shared or
    /// coincident points where it snapped, horizontal and vertical lines
    /// where they nearly are), and returns them.
    pub fn click(&mut self, sketch: &mut Sketch, pick: &Pick) -> Vec<EntityId> {
        let anchor = Anchor::of(pick);
        match (self.tool, self.anchors.as_slice()) {
            (Tool::Select, _) => Vec::new(),
            (Tool::Line, &[start]) => {
                let (from, mut anchor) = (start.at(sketch), anchor);
                let to = anchor.at(sketch);
                if from.distance(to) < 1e-9 {
                    return Vec::new();
                }
                let axis = auto_axis(from, to);
                // A free end lands exactly on the axis it is held to.
                if let Anchor::Free(p) = &mut anchor {
                    match axis {
                        Some(Axis::Horizontal) => p.y = from.y,
                        Some(Axis::Vertical) => p.x = from.x,
                        None => {}
                    }
                }
                let a = start.place(sketch);
                let b = anchor.place(sketch);
                let line = sketch.add_line(a, b);
                match axis {
                    Some(Axis::Horizontal) => sketch.add_constraint(Constraint::Horizontal(line)),
                    Some(Axis::Vertical) => sketch.add_constraint(Constraint::Vertical(line)),
                    None => c2f_sketch::ConstraintId(0),
                };
                // Landing on a point drawn before closes the outline.
                self.anchors = match anchor {
                    Anchor::Existing(_) => Vec::new(),
                    _ => vec![Anchor::Existing(b)],
                };
                vec![line]
            }
            (Tool::Rectangle, &[first]) => {
                let (a, b) = (first.at(sketch), anchor.at(sketch));
                if (a.x - b.x).abs() < 1e-9 || (a.y - b.y).abs() < 1e-9 {
                    return Vec::new();
                }
                let base = sketch.points.len();
                let lines = sketch.add_rectangle(a, b);
                first.attach(sketch, PointId(base));
                anchor.attach(sketch, PointId(base + 2));
                self.anchors.clear();
                lines.to_vec()
            }
            (Tool::Circle, &[center]) => {
                let radius = center.at(sketch).distance(anchor.at(sketch));
                if radius < 1e-9 {
                    return Vec::new();
                }
                let c = center.place(sketch);
                let circle = sketch.add_circle(c, radius);
                if let Anchor::Existing(p) = anchor {
                    sketch.add_constraint(Constraint::PointOnCircle(p, circle));
                }
                self.anchors.clear();
                vec![circle]
            }
            (Tool::Arc, &[center, start]) => {
                let (c, s) = (center.at(sketch), start.at(sketch));
                let r = c.distance(s);
                let at = anchor.at(sketch);
                if r < 1e-9 || c.distance(at) < 1e-9 {
                    return Vec::new();
                }
                // A free end goes on the circle, at the click's angle.
                let anchor = match anchor {
                    Anchor::Free(p) => {
                        let angle = (p.y - c.y).atan2(p.x - c.x);
                        Anchor::Free(Point::new(c.x + r * angle.cos(), c.y + r * angle.sin()))
                    }
                    other => other,
                };
                if anchor.at(sketch).distance(s) < 1e-9 {
                    return Vec::new();
                }
                let ids = [center.place(sketch), start.place(sketch), anchor.place(sketch)];
                let arc = sketch.add_arc(ids[0], ids[1], ids[2]);
                self.anchors.clear();
                vec![arc]
            }
            (_, _) => {
                self.anchors.push(anchor);
                Vec::new()
            }
        }
    }

    /// What the shape being drawn looks like with its next click at `at`.
    pub fn preview(&self, sketch: &Sketch, at: Point) -> Option<Preview> {
        let points: Vec<Point> = self.anchors.iter().map(|a| a.at(sketch)).collect();
        match (self.tool, points.as_slice()) {
            (Tool::Line, &[from]) => {
                let to = match auto_axis(from, at) {
                    Some(Axis::Horizontal) => Point::new(at.x, from.y),
                    Some(Axis::Vertical) => Point::new(from.x, at.y),
                    None => at,
                };
                Some(Preview::Line(from, to))
            }
            (Tool::Rectangle, &[a]) => Some(Preview::Line(a, at)),
            (Tool::Circle, &[c]) => Some(Preview::Circle(c, c.distance(at))),
            (Tool::Arc, &[c]) => Some(Preview::Line(c, at)),
            (Tool::Arc, &[c, s]) => {
                let r = c.distance(s);
                let angle = (at.y - c.y).atan2(at.x - c.x);
                Some(Preview::Arc(c, s, Point::new(c.x + r * angle.cos(), c.y + r * angle.sin())))
            }
            _ => None,
        }
    }
}

/// The axis a line from `a` to `b` nearly follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

pub fn auto_axis(a: Point, b: Point) -> Option<Axis> {
    let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
    if dx == 0.0 && dy == 0.0 {
        None
    } else if dy <= AUTO_AXIS_SLOPE * dx {
        Some(Axis::Horizontal)
    } else if dx <= AUTO_AXIS_SLOPE * dy {
        Some(Axis::Vertical)
    } else {
        None
    }
}

// ---- constraints and dimensions from a selection ----

/// The constraints of the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Coincident,
    Tangent,
    Equal,
    /// Points that stay where they are.
    Fix,
}

impl Rule {
    pub const ALL: [Rule; 8] = [
        Rule::Horizontal,
        Rule::Vertical,
        Rule::Parallel,
        Rule::Perpendicular,
        Rule::Coincident,
        Rule::Tangent,
        Rule::Equal,
        Rule::Fix,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Rule::Horizontal => "Horizontal",
            Rule::Vertical => "Vertical",
            Rule::Parallel => "Parallel",
            Rule::Perpendicular => "Perpendicular",
            Rule::Coincident => "Coincident",
            Rule::Tangent => "Tangent",
            Rule::Equal => "Equal",
            Rule::Fix => "Fix",
        }
    }

    /// What to select for it.
    pub fn help(self) -> &'static str {
        match self {
            Rule::Horizontal | Rule::Vertical => "Select one or more lines",
            Rule::Parallel | Rule::Perpendicular => "Select two lines",
            Rule::Coincident => "Select two points, or a point and a line, circle or arc",
            Rule::Tangent => "Select a line and a circle or arc, or two circles or arcs",
            Rule::Equal => "Select lines, or circles and arcs",
            Rule::Fix => "Select points, or entities to fix their points",
        }
    }
}

/// The dimensions of the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    /// A line's length, or the distance between two points.
    Length,
    Radius,
    Angle,
}

impl Measure {
    pub const ALL: [Measure; 3] = [Measure::Length, Measure::Radius, Measure::Angle];

    pub fn name(self) -> &'static str {
        match self {
            Measure::Length => "Length",
            Measure::Radius => "Radius",
            Measure::Angle => "Angle",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Measure::Length => "Select a line, or two points",
            Measure::Radius => "Select a circle or an arc",
            Measure::Angle => "Select two lines",
        }
    }
}

fn is_line(sketch: &Sketch, e: EntityId) -> bool {
    matches!(sketch.entity(e).geometry, Geometry::Line { .. })
}

fn split(selection: &[Item]) -> (Vec<PointId>, Vec<EntityId>) {
    let mut points = Vec::new();
    let mut entities = Vec::new();
    for item in selection {
        match *item {
            Item::Point(p) => points.push(p),
            Item::Entity(e) => entities.push(e),
        }
    }
    (points, entities)
}

/// The constraints a rule makes of the selection, or what to select
/// instead.
pub fn constraints_for(rule: Rule, sketch: &Sketch, selection: &[Item]) -> Result<Vec<Constraint>, &'static str> {
    let (points, entities) = split(selection);
    let lines: Vec<EntityId> = entities.iter().copied().filter(|&e| is_line(sketch, e)).collect();
    let rounds: Vec<EntityId> = entities.iter().copied().filter(|&e| !is_line(sketch, e)).collect();
    let made = match rule {
        Rule::Horizontal | Rule::Vertical if points.is_empty() && rounds.is_empty() && !lines.is_empty() => lines
            .iter()
            .map(|&l| if rule == Rule::Horizontal { Constraint::Horizontal(l) } else { Constraint::Vertical(l) })
            .collect(),
        Rule::Parallel if points.is_empty() && rounds.is_empty() && lines.len() == 2 => {
            vec![Constraint::Parallel(lines[0], lines[1])]
        }
        Rule::Perpendicular if points.is_empty() && rounds.is_empty() && lines.len() == 2 => {
            vec![Constraint::Perpendicular(lines[0], lines[1])]
        }
        Rule::Coincident => match (points.as_slice(), entities.as_slice()) {
            (&[a, b], &[]) => vec![Constraint::Coincident(a, b)],
            (&[p], &[e]) if is_line(sketch, e) && !sketch.entity_points(e).contains(&p) => {
                vec![Constraint::PointOnLine(p, e)]
            }
            (&[p], &[e]) if !is_line(sketch, e) => vec![Constraint::PointOnCircle(p, e)],
            _ => return Err(rule.help()),
        },
        Rule::Tangent if points.is_empty() && entities.len() == 2 && !rounds.is_empty() => match lines.as_slice() {
            &[line] => vec![Constraint::Tangent(line, rounds[0])],
            _ => vec![Constraint::Tangent(rounds[0], rounds[1])],
        },
        Rule::Equal if points.is_empty() && entities.len() >= 2 && (lines.is_empty() || rounds.is_empty()) => {
            entities[1..].iter().map(|&e| Constraint::Equal(entities[0], e)).collect()
        }
        Rule::Fix if !selection.is_empty() => {
            let mut fixed: Vec<PointId> = points;
            for &e in &entities {
                fixed.extend(sketch.entity_points(e));
            }
            fixed.sort();
            fixed.dedup();
            fixed.into_iter().map(Constraint::Fixed).collect()
        }
        _ => return Err(rule.help()),
    };
    Ok(made)
}

/// The dimension a measure makes of the selection, at its value as drawn
/// (for the prompt to change), or what to select instead.
pub fn dimension_for(measure: Measure, sketch: &Sketch, selection: &[Item]) -> Result<Constraint, &'static str> {
    let (points, entities) = split(selection);
    match (measure, points.as_slice(), entities.as_slice()) {
        (Measure::Length, &[], &[e]) if is_line(sketch, e) => {
            let ends = sketch.entity_points(e);
            let length = sketch.point(ends[0]).distance(sketch.point(ends[1]));
            Ok(Constraint::Length(e, Dimension::new(round(length))))
        }
        (Measure::Length, &[a, b], &[]) => {
            Ok(Constraint::Distance(a, b, Dimension::new(round(sketch.point(a).distance(sketch.point(b))))))
        }
        (Measure::Radius, &[], &[e]) if !is_line(sketch, e) => {
            Ok(Constraint::Radius(e, Dimension::new(round(sketch.radius(e).unwrap_or(1.0)))))
        }
        (Measure::Angle, &[], &[a, b]) if is_line(sketch, a) && is_line(sketch, b) => {
            Ok(Constraint::Angle(a, b, Dimension::new(round(angle_between(sketch, a, b)))))
        }
        _ => Err(measure.help()),
    }
}

/// The angle in degrees from line `a`'s direction to line `b`'s,
/// counterclockwise, brought under 180° (met either way, see
/// `c2f_sketch::Constraint::Angle`).
pub fn angle_between(sketch: &Sketch, a: EntityId, b: EntityId) -> f64 {
    let direction = |e: EntityId| {
        let ends = sketch.entity_points(e);
        let (p, q) = (sketch.point(ends[0]), sketch.point(ends[1]));
        (q.y - p.y).atan2(q.x - p.x)
    };
    let degrees = (direction(b) - direction(a)).to_degrees().rem_euclid(360.0);
    if degrees >= 180.0 { degrees - 180.0 } else { degrees }
}

/// A measured value as a dimension starts: to a hundredth.
fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

// ---- removing ----

/// The sketch without these points and entities: an entity goes with any
/// of its points, a point with the last entity drawn through it, and a
/// constraint with anything it names. What stays is numbered afresh.
pub fn remove(sketch: &Sketch, items: &[Item]) -> Sketch {
    let mut gone_points = vec![false; sketch.points.len()];
    let mut gone_entities = vec![false; sketch.entities.len()];
    for item in items {
        match *item {
            Item::Point(p) if p.0 < gone_points.len() => gone_points[p.0] = true,
            Item::Entity(e) if e.0 < gone_entities.len() => gone_entities[e.0] = true,
            _ => {}
        }
    }
    for (e, gone) in gone_entities.iter_mut().enumerate() {
        if sketch.entity_points(EntityId(e)).iter().any(|p| gone_points[p.0]) {
            *gone = true;
        }
    }
    // Points left on no entity go too.
    let mut used = vec![false; sketch.points.len()];
    for e in (0..sketch.entities.len()).filter(|&e| !gone_entities[e]) {
        for p in sketch.entity_points(EntityId(e)) {
            used[p.0] = true;
        }
    }
    for e in (0..sketch.entities.len()).filter(|&e| gone_entities[e]) {
        for p in sketch.entity_points(EntityId(e)) {
            if !used[p.0] {
                gone_points[p.0] = true;
            }
        }
    }
    let renumber = |gone: &[bool]| {
        let mut next = 0;
        gone.iter()
            .map(|&g| {
                (!g).then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect::<Vec<Option<usize>>>()
    };
    let (point_ids, entity_ids) = (renumber(&gone_points), renumber(&gone_entities));
    let p = |id: PointId| point_ids[id.0].map(PointId);
    let e = |id: EntityId| entity_ids[id.0].map(EntityId);

    let mut out = Sketch::new(sketch.plane);
    out.points = (0..sketch.points.len()).filter(|&i| !gone_points[i]).map(|i| sketch.points[i]).collect();
    for (i, entity) in sketch.entities.iter().enumerate().filter(|(i, _)| !gone_entities[*i]) {
        let geometry = match entity.geometry {
            Geometry::Line { a, b } => Geometry::Line { a: p(a).expect("kept"), b: p(b).expect("kept") },
            Geometry::Circle { center, radius } => Geometry::Circle { center: p(center).expect("kept"), radius },
            Geometry::Arc { center, start, end } => Geometry::Arc {
                center: p(center).expect("kept"),
                start: p(start).expect("kept"),
                end: p(end).expect("kept"),
            },
        };
        let id = out.add_entity(geometry);
        out.set_construction(id, sketch.entities[i].construction);
    }
    out.constraints = sketch.constraints.iter().filter_map(|c| renumbered(c, &p, &e)).collect();
    out
}

/// A constraint with its points and entities renumbered; `None` when one
/// of them is gone.
fn renumbered(
    c: &Constraint,
    p: &dyn Fn(PointId) -> Option<PointId>,
    e: &dyn Fn(EntityId) -> Option<EntityId>,
) -> Option<Constraint> {
    use Constraint as C;
    Some(match c {
        C::Coincident(a, b) => C::Coincident(p(*a)?, p(*b)?),
        C::Horizontal(l) => C::Horizontal(e(*l)?),
        C::Vertical(l) => C::Vertical(e(*l)?),
        C::Parallel(a, b) => C::Parallel(e(*a)?, e(*b)?),
        C::Perpendicular(a, b) => C::Perpendicular(e(*a)?, e(*b)?),
        C::Tangent(a, b) => C::Tangent(e(*a)?, e(*b)?),
        C::Equal(a, b) => C::Equal(e(*a)?, e(*b)?),
        C::Distance(a, b, d) => C::Distance(p(*a)?, p(*b)?, d.clone()),
        C::Length(l, d) => C::Length(e(*l)?, d.clone()),
        C::Radius(r, d) => C::Radius(e(*r)?, d.clone()),
        C::Diameter(r, d) => C::Diameter(e(*r)?, d.clone()),
        C::Angle(a, b, d) => C::Angle(e(*a)?, e(*b)?, d.clone()),
        C::PointOnLine(q, l) => C::PointOnLine(p(*q)?, e(*l)?),
        C::PointOnCircle(q, r) => C::PointOnCircle(p(*q)?, e(*r)?),
        C::Midpoint(q, l) => C::Midpoint(p(*q)?, e(*l)?),
        C::Fixed(q) => C::Fixed(p(*q)?),
    })
}

/// The points and entities a constraint names, for drawing it in red
/// when it conflicts.
pub fn constraint_items(sketch: &Sketch, c: &Constraint) -> Vec<Item> {
    use Constraint as C;
    let _ = sketch;
    match c {
        C::Coincident(a, b) | C::Distance(a, b, _) => vec![Item::Point(*a), Item::Point(*b)],
        C::Fixed(q) => vec![Item::Point(*q)],
        C::Horizontal(l) | C::Vertical(l) | C::Length(l, _) | C::Radius(l, _) | C::Diameter(l, _) => {
            vec![Item::Entity(*l)]
        }
        C::Parallel(a, b) | C::Perpendicular(a, b) | C::Tangent(a, b) | C::Equal(a, b) | C::Angle(a, b, _) => {
            vec![Item::Entity(*a), Item::Entity(*b)]
        }
        C::PointOnLine(q, l) | C::PointOnCircle(q, l) | C::Midpoint(q, l) => vec![Item::Point(*q), Item::Entity(*l)],
    }
}

// ---- the code ----

/// The module a statement calls a sketch through, as in
/// `sketch_l_bracket();` or `linear_extrude(height = 5) sketch_l_bracket();`.
pub fn sketch_called(statement: &str) -> Option<String> {
    let mut rest = statement;
    while let Some(at) = rest.find("sketch_") {
        let before = rest[..at].chars().next_back();
        let tail = &rest[at..];
        let name_len = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(tail.len());
        let after = tail[name_len..].trim_start();
        let word_start = before.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        if word_start
            && name_len > "sketch_".len()
            && after.starts_with('(')
            && after[1..].trim_start().starts_with(')')
        {
            return Some(tail[..name_len].to_string());
        }
        rest = &tail["sketch_".len()..];
    }
    None
}

/// The text, cursor (a char index) and selection of an edit, as
/// `StudioApp::apply_code_edits` takes them.
pub type Written = (String, usize, Vec<Range<usize>>);

/// What extruding or revolving a finished sketch adds to the design: the
/// code (from `c2f_sketch::extrude_code` or `revolve_code`, added rather
/// than subtracted there) and the object to cut it from, if any, as the
/// byte range of its statement in the source given to [`write`].
pub struct Feature<'a> {
    pub code: &'a Code,
    pub cut_from: Option<Range<usize>>,
}

/// Writes a sketch into the design `src`, as one edit: its parameters at
/// the top, its module (in place of the one of the same name, or at the
/// end) and, given a feature, the extrusion or revolution with its own
/// parameters, either at the end of the file or cut out of an object (a
/// `difference()`, see `code_edit::make_hole`). The selection is the
/// feature's statement; without one, nothing (the module draws nothing by
/// itself) and the cursor is on the module.
pub fn write(src: &str, sketch: &Sketch, name: &str, feature: Option<Feature>) -> Result<Written, String> {
    let (text, selected) = match &feature {
        Some(Feature { code, cut_from }) => {
            let edit = match cut_from {
                Some(target) => code_edit::make_hole(src, target.clone(), &code.body)?,
                None => code_edit::insert_shape(src, &Place::End, &code.body)?,
            };
            (edit.text, Some(edit.selection))
        }
        None => (src.to_string(), None),
    };
    let failed = |e: c2f_sketch::SketchError| format!("Cannot write the sketch: {e}");
    let mut new = update_source(&text, sketch, name).map_err(failed)?;
    if let Some(Feature { code, .. }) = &feature {
        new = insert_parameters(&new, &code.parameters).map_err(failed)?;
    }
    let selection = match selected {
        Some(range) => vec![follow(&text, range, &new).ok_or("Cannot find the new statement in the code.")?],
        None => Vec::new(),
    };
    let at = match selection.first() {
        Some(range) => range.start,
        None => new.find(&format!("module {}(", module_name(name))).unwrap_or(0),
    };
    let cursor = new[..at].chars().count();
    Ok((new, cursor, selection))
}

/// Where the text at `range` of `old` is in `new`, which only added or
/// changed text that does not hold it (the sketch's module and the
/// parameters): the same occurrence of that text, counted from the start.
fn follow(old: &str, range: Range<usize>, new: &str) -> Option<Range<usize>> {
    let needle = &old[range.clone()];
    if needle.is_empty() {
        return None;
    }
    let nth = old.match_indices(needle).take_while(|(i, _)| *i < range.start).count();
    new.match_indices(needle).nth(nth).map(|(i, _)| i..i + needle.len())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use c2f_sketch::{ExtrudeMode, extrude_code, from_code, revolve_code, solve};

    use super::*;

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn plane_mapping_round_trips() {
        let face = face_plane(Vec3::new(3.0, -2.0, 7.0), Vec3::new(0.0, 0.6, 0.8));
        for plane in [Plane::Xy, Plane::Xz, Plane::Yz, face] {
            let frame = PlaneFrame::of(&plane);
            assert!((frame.x.length() - 1.0).abs() < 1e-5 && frame.x.dot(frame.normal).abs() < 1e-5);
            for p in [Point::new(0.0, 0.0), Point::new(12.5, -4.0), Point::new(-30.0, 22.0)] {
                assert!(close(frame.to_plane(frame.to_world(p)), p), "{plane:?} {p:?}");
                // A ray from in front of the plane lands back on the point.
                let eye = frame.to_world(p) + frame.normal * 50.0 + frame.x * 5.0;
                let hit = frame.intersect(eye, (frame.to_world(p) - eye).normalize()).unwrap();
                assert!(close(hit, p), "{plane:?} {p:?} {hit:?}");
            }
            // Running along the plane, or away from it, misses.
            assert_eq!(frame.intersect(frame.origin + frame.normal, frame.x), None);
            assert_eq!(frame.intersect(frame.origin + frame.normal, frame.normal), None);
        }
        // The planes the code places them on.
        assert_eq!(PlaneFrame::of(&Plane::Xz).to_world(Point::new(1.0, 2.0)), Vec3::new(1.0, 0.0, 2.0));
        assert_eq!(PlaneFrame::of(&Plane::Yz).to_world(Point::new(1.0, 2.0)), Vec3::new(0.0, 1.0, 2.0));
    }

    #[test]
    fn the_camera_faces_the_plane_with_x_to_the_right() {
        let top = face_plane(Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(top, Plane::Face { origin: [0.0, 0.0, 10.0], x_axis: [1.0, 0.0, 0.0], normal: [0.0, 0.0, 1.0] });
        for plane in [Plane::Xy, Plane::Xz, Plane::Yz, top, face_plane(Vec3::ZERO, Vec3::new(-0.6, 0.0, -0.8))] {
            let frame = PlaneFrame::of(&plane);
            let camera = frame.face_on(Point::new(5.0, 5.0), 100.0, 0.6);
            assert!(camera.back().dot(frame.normal) > 0.999, "{plane:?}");
            // The sketch's x axis points right on screen, its y axis up.
            let view = camera.view();
            let x = view.transform_vector3(frame.x);
            let y = view.transform_vector3(frame.y);
            assert!(x.x > 0.99 && y.y > 0.99, "{plane:?} {x} {y}");
            let tilted = frame.three_quarters(Point::new(0.0, 0.0), 100.0, 0.6);
            assert!(tilted.back().dot(frame.normal) > 0.5 && tilted.back().dot(frame.normal) < 0.95, "{plane:?}");
        }
    }

    /// Clicks of the line tool, snapping as the view does.
    fn draw_lines(sketch: &mut Sketch, drawing: &mut Drawing, clicks: &[(f64, f64)]) -> Vec<EntityId> {
        let mut made = Vec::new();
        for &(x, y) in clicks {
            made.extend(drawing.click(sketch, &pick(sketch, Point::new(x, y), 0.5)));
        }
        made
    }

    /// An L-bracket profile drawn as a user would: from the origin, a
    /// little off the axes, closing on the first point.
    fn l_bracket_drawn() -> (Sketch, Vec<EntityId>) {
        let mut sketch = Sketch::new(Plane::Xy);
        let mut drawing = Drawing::default();
        drawing.set_tool(Tool::Line);
        let clicks = [(0.1, -0.1), (38.0, 0.6), (38.2, 6.0), (6.0, 6.2), (6.3, 28.0), (0.4, 28.3), (0.2, 0.3)];
        let lines = draw_lines(&mut sketch, &mut drawing, &clicks);
        assert_eq!(drawing.clicks(), 0, "closing on the first point ends the outline");
        (sketch, lines)
    }

    #[test]
    fn the_line_tool_draws_a_closed_held_outline() {
        let (sketch, lines) = l_bracket_drawn();
        assert_eq!(lines.len(), 6);
        assert_eq!(sketch.points.len(), 6, "corners are shared");
        assert_eq!(sketch.points[0], Point::new(0.0, 0.0), "snapped to the origin");
        assert_eq!(sketch.points[1].y, 0.0, "a nearly horizontal line lands on its axis");
        let mut expected = vec![Constraint::Fixed(PointId(0))];
        for (i, &line) in lines.iter().enumerate() {
            expected.push(if i % 2 == 0 { Constraint::Horizontal(line) } else { Constraint::Vertical(line) });
        }
        assert_eq!(sketch.constraints, expected);
        let Geometry::Line { a, b } = sketch.entity(lines[5]).geometry else { panic!() };
        assert_eq!((a, b), (PointId(5), PointId(0)), "the last line closes on the first point");
        assert_eq!(c2f_sketch::profile(&sketch, 64).loops.len(), 1);
    }

    #[test]
    fn the_other_tools_and_cancelling() {
        let mut sketch = Sketch::new(Plane::Xy);
        let mut drawing = Drawing::default();
        drawing.set_tool(Tool::Rectangle);
        let rect = draw_lines(&mut sketch, &mut drawing, &[(10.0, 10.0), (30.0, 20.0)]);
        assert_eq!(rect.len(), 4);
        assert_eq!(sketch.constraints.iter().filter(|c| matches!(c, Constraint::Horizontal(_))).count(), 2);

        // A circle centered on the rectangle's corner shares that point.
        drawing.set_tool(Tool::Circle);
        let circle = draw_lines(&mut sketch, &mut drawing, &[(30.0, 20.0), (34.0, 20.0)]);
        assert_eq!(sketch.entity(circle[0]).geometry, Geometry::Circle { center: PointId(2), radius: 4.0 });

        // An arc: center, start, then an end put on the circle.
        drawing.set_tool(Tool::Arc);
        let arc = draw_lines(&mut sketch, &mut drawing, &[(50.0, 0.0), (55.0, 0.0), (50.0, 9.0)]);
        let Geometry::Arc { end, .. } = sketch.entity(arc[0]).geometry else { panic!() };
        assert!(close(sketch.point(end), Point::new(50.0, 5.0)));

        // A line from the middle of the rectangle's bottom is held there.
        drawing.set_tool(Tool::Line);
        draw_lines(&mut sketch, &mut drawing, &[(20.0, 10.0), (20.0, 0.0)]);
        assert!(sketch.constraints.contains(&Constraint::Midpoint(PointId(sketch.points.len() - 2), rect[0])));
        assert_eq!(drawing.clicks(), 1, "the polyline goes on");
        assert!(drawing.cancel());
        assert!(!drawing.cancel());
        assert_eq!(drawing.preview(&sketch, Point::new(1.0, 1.0)), None);

        // Two clicks at the same place draw nothing.
        let before = sketch.clone();
        draw_lines(&mut sketch, &mut drawing, &[(70.0, 70.0), (70.0, 70.0)]);
        assert_eq!(sketch, before);
        assert_eq!(
            drawing.preview(&sketch, Point::new(80.0, 70.5)),
            Some(Preview::Line(Point::new(70.0, 70.0), Point::new(80.0, 70.0)))
        );
        assert!(solve(&mut sketch).converged());
    }

    #[test]
    fn hits_points_before_entities() {
        let mut sketch = Sketch::new(Plane::Xy);
        let [bottom, ..] = sketch.add_rectangle(Point::new(0.0, 0.0), Point::new(20.0, 10.0));
        let c = sketch.add_point(40.0, 0.0);
        let circle = sketch.add_circle(c, 5.0);
        let (center, start, end) =
            (sketch.add_point(70.0, 0.0), sketch.add_point(75.0, 0.0), sketch.add_point(70.0, 5.0));
        let arc = sketch.add_arc(center, start, end);
        assert_eq!(hit(&sketch, Point::new(0.3, 0.2), 1.0), Some(Item::Point(PointId(0))));
        assert_eq!(hit(&sketch, Point::new(10.0, 0.4), 1.0), Some(Item::Entity(bottom)));
        assert_eq!(hit(&sketch, Point::new(10.0, 5.0), 1.0), None);
        assert_eq!(hit(&sketch, Point::new(40.0, 5.5), 1.0), Some(Item::Entity(circle)));
        // On the arc's quarter, not on the rest of its circle.
        assert_eq!(hit(&sketch, Point::new(73.6, 3.6), 1.0), Some(Item::Entity(arc)));
        assert_eq!(hit(&sketch, Point::new(66.4, -3.6), 1.0), None);
    }

    #[test]
    fn the_palette_turns_a_selection_into_constraints() {
        let mut sketch = Sketch::new(Plane::Xy);
        let lines = sketch.add_polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 1.0), Point::new(11.0, 9.0)], false);
        let c = sketch.add_point(20.0, 0.0);
        let circle = sketch.add_circle(c, 3.0);
        let (l0, l1) = (Item::Entity(lines[0]), Item::Entity(lines[1]));
        assert_eq!(constraints_for(Rule::Horizontal, &sketch, &[l0, l1]).unwrap().len(), 2);
        assert_eq!(
            constraints_for(Rule::Perpendicular, &sketch, &[l0, l1]).unwrap(),
            [Constraint::Perpendicular(lines[0], lines[1])]
        );
        assert!(constraints_for(Rule::Parallel, &sketch, &[l0]).is_err());
        assert_eq!(
            constraints_for(Rule::Coincident, &sketch, &[Item::Point(c), l1]).unwrap(),
            [Constraint::PointOnLine(c, lines[1])]
        );
        assert_eq!(
            constraints_for(Rule::Tangent, &sketch, &[Item::Entity(circle), l0]).unwrap(),
            [Constraint::Tangent(lines[0], circle)]
        );
        assert!(constraints_for(Rule::Equal, &sketch, &[l0, Item::Entity(circle)]).is_err());
        assert_eq!(
            constraints_for(Rule::Fix, &sketch, &[l0]).unwrap(),
            [Constraint::Fixed(PointId(0)), Constraint::Fixed(PointId(1))]
        );

        let Constraint::Length(_, d) = dimension_for(Measure::Length, &sketch, &[l0]).unwrap() else { panic!() };
        assert_eq!(d.value, 10.05);
        assert!(matches!(
            dimension_for(Measure::Length, &sketch, &[Item::Point(PointId(0)), Item::Point(c)]),
            Ok(Constraint::Distance(..))
        ));
        assert!(
            matches!(dimension_for(Measure::Radius, &sketch, &[Item::Entity(circle)]), Ok(Constraint::Radius(_, d)) if d.value == 3.0)
        );
        let Constraint::Angle(_, _, d) = dimension_for(Measure::Angle, &sketch, &[l0, l1]).unwrap() else { panic!() };
        assert!((d.value - 77.15).abs() < 0.01, "{}", d.value);
        assert!(dimension_for(Measure::Radius, &sketch, &[l0]).is_err());
    }

    #[test]
    fn removing_renumbers_what_stays() {
        let mut sketch = Sketch::new(Plane::Xy);
        let lines = sketch.add_rectangle(Point::new(0.0, 0.0), Point::new(20.0, 10.0));
        let c = sketch.add_point(5.0, 5.0);
        let circle = sketch.add_circle(c, 2.0);
        sketch.add_constraint(Constraint::Radius(circle, Dimension::new(2.0)));
        let smaller = remove(&sketch, &[Item::Entity(lines[1])]);
        // The right side goes; its corners stay on the top and bottom.
        assert_eq!((smaller.entities.len(), smaller.points.len()), (4, 5));
        assert_eq!(smaller.constraints.len(), 4);
        assert_eq!(smaller.constraints[3], Constraint::Radius(EntityId(3), Dimension::new(2.0)));
        // A corner takes both its lines, and the circle's center the circle.
        let fewer = remove(&sketch, &[Item::Point(PointId(0)), Item::Point(c)]);
        assert_eq!((fewer.entities.len(), fewer.points.len()), (2, 3));
        assert!(solve(&mut fewer.clone()).converged());
    }

    #[test]
    fn finds_the_sketch_a_statement_calls() {
        assert_eq!(sketch_called("sketch_l_bracket();").as_deref(), Some("sketch_l_bracket"));
        assert_eq!(
            sketch_called("rotate([90, 0, 0]) linear_extrude(height = h) sketch_a ( );").as_deref(),
            Some("sketch_a")
        );
        assert_eq!(sketch_called("my_sketch_a();"), None);
        assert_eq!(sketch_called("sketch_(); cube(sketch_w);"), None);
    }

    /// Renders a design, for its volume.
    fn volume(src: &str) -> f64 {
        let program = c2f_eval::Program::load(src, Path::new("t.c2f"), &[]).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
        let output = c2f_eval::evaluate(&program);
        assert!(output.warnings.is_empty(), "{:?}\n{src}", output.warnings);
        c2f_geom::render(&output.nodes).stats.volume
    }

    /// The L-bracket drawn, then dimensioned from the palette.
    fn l_bracket() -> Sketch {
        let (mut sketch, lines) = l_bracket_drawn();
        let named = |measure, line: EntityId, name: &str, value: f64| {
            let mut c = dimension_for(measure, &sketch, &[Item::Entity(line)]).unwrap();
            *c.dimension_mut().unwrap() = Dimension::named(name, value);
            c
        };
        let dims = [
            named(Measure::Length, lines[0], "width", 40.0),
            named(Measure::Length, lines[5], "height", 30.0),
            named(Measure::Length, lines[1], "thickness", 5.0),
            named(Measure::Length, lines[4], "thickness", 5.0),
        ];
        for c in dims {
            sketch.add_constraint(c);
        }
        assert!(solve(&mut sketch).is_fully_constrained());
        sketch
    }

    #[test]
    fn sketch_to_code_to_solid_and_back() {
        let src = "// A bracket.\n$fn = 32;\n\ntranslate([100, 0, 0]) cube(1);\n";
        let mut sketch = l_bracket();

        // Finish: the module and its parameters, no selection.
        let (text, cursor, selection) = write(src, &sketch, "l_bracket", None).unwrap();
        assert!(selection.is_empty());
        assert!(text[text.char_indices().nth(cursor).unwrap().0..].starts_with("module sketch_l_bracket()"));
        assert!(text.contains("width = 40; // 0.1"), "{text}");
        assert!((volume(&text) - 1.0).abs() < 1e-9, "the module alone draws nothing");

        // Extrude: one edit, the extrusion selected.
        let height = Dimension::named("depth", 12.0);
        let code = extrude_code("l_bracket", &sketch.plane, &height, &ExtrudeMode::Add);
        let (text, _, selection) =
            write(src, &sketch, "l_bracket", Some(Feature { code: &code, cut_from: None })).unwrap();
        assert_eq!(&text[selection[0].clone()], "linear_extrude(height = depth) sketch_l_bracket();");
        assert!(text.contains("depth = 12; // 0.1"), "{text}");
        let solid = 325.0 * 12.0;
        assert!((volume(&text) - (1.0 + solid)).abs() < 1e-6, "{}", volume(&text));

        // A dimension changed: the profile follows, and so does the solid.
        assert!(sketch.set_variable("width", 60.0));
        assert!(solve(&mut sketch).is_fully_constrained());
        let (wider, _, _) = write(&text, &sketch, "l_bracket", None).unwrap();
        assert_eq!(wider.matches("module sketch_l_bracket").count(), 1, "replaced in place");
        assert!((volume(&wider) - (1.0 + (60.0 * 5.0 + 5.0 * 25.0) * 12.0)).abs() < 1e-6);

        // Reopened from the code, the sketch is the same and writes the
        // same code: the same solid.
        let mut reopened = from_code(&wider, "sketch_l_bracket").unwrap();
        assert_eq!(reopened.constraints, sketch.constraints);
        assert!(solve(&mut reopened).is_fully_constrained());
        let (again, _, _) = write(&wider, &reopened, "l_bracket", None).unwrap();
        assert_eq!(again, wider);
    }

    #[test]
    fn cutting_and_revolving() {
        // A sketch on the top face of a plate, extruded down into it.
        let src = "translate([0, 0, 0]) cube([20, 20, 10]);\n";
        let plane = face_plane(Vec3::new(10.0, 10.0, 10.0), Vec3::Z);
        let mut sketch = Sketch::new(plane);
        let mut drawing = Drawing::default();
        drawing.set_tool(Tool::Rectangle);
        draw_lines(&mut sketch, &mut drawing, &[(-2.0, -2.0), (2.0, 2.0)]);
        let code = extrude_code("pocket", &plane, &Dimension::new(-4.0), &ExtrudeMode::Add);
        let target = 0..src.trim_end().len();
        let (text, _, selection) =
            write(src, &sketch, "pocket", Some(Feature { code: &code, cut_from: Some(target) })).unwrap();
        assert!(text.contains("difference() {"), "{text}");
        assert!(text[selection[0].clone()].ends_with("sketch_pocket();"), "{text}");
        assert!((volume(&text) - (4000.0 - 16.0 * 4.0)).abs() < 1e-6, "{}", volume(&text));

        // A rectangle beside a construction axis, revolved: a tube.
        let mut sketch = Sketch::new(Plane::Xz);
        let (a, b) = (sketch.add_point(0.0, 0.0), sketch.add_point(0.0, 10.0));
        let axis = sketch.add_line(a, b);
        sketch.set_construction(axis, true);
        sketch.add_rectangle(Point::new(3.0, 0.0), Point::new(5.0, 10.0));
        let code = revolve_code(&sketch, "ring", axis, &Dimension::new(360.0), &ExtrudeMode::Add).unwrap();
        let (text, _, selection) =
            write("$fn = 256;\n", &sketch, "ring", Some(Feature { code: &code, cut_from: None })).unwrap();
        assert!(text[selection[0].clone()].contains("rotate_extrude"), "{text}");
        let tube = std::f64::consts::PI * (25.0 - 9.0) * 10.0;
        assert!((volume(&text) - tube).abs() < tube * 0.01, "{}", volume(&text));
    }
}
