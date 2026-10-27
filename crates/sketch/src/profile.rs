//! Profiles: the closed loops the sketch's lines, arcs and circles make,
//! as polygons, which is what `polygon(points, paths)` fills.
//!
//! Construction geometry is left out. Ends meet when they are at the same
//! place, whether they share a point or are held together by a
//! [`crate::Constraint::Coincident`]. A loop is a chain of lines and arcs
//! where each end meets exactly one other; a full circle is a loop of its
//! own. Lines and arcs that do not close a loop, or that meet more than
//! one other at an end, are listed as open and left out.
//!
//! Loops inside an odd number of others are holes. The polygon is filled
//! the even-odd way, as `polygon()` fills its paths, so a hole in a hole
//! is filled again. Outer loops turn counterclockwise and holes
//! clockwise.

use std::f64::consts::TAU;

use crate::model::{EntityId, Geometry, Point, PointId, Sketch};

/// Segments per full turn when arcs and circles become polygons, as `$fn`
/// gives them.
pub const DEFAULT_SEGMENTS: u32 = 64;

/// A closed loop, as the corners of a polygon.
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub points: Vec<Point>,
    /// The lines, arcs or circle it goes through, in order.
    pub entities: Vec<EntityId>,
    /// How many loops it is inside: odd for a hole.
    pub depth: usize,
}

impl Loop {
    pub fn is_hole(&self) -> bool {
        self.depth % 2 == 1
    }

    /// The signed area: positive counterclockwise.
    pub fn signed_area(&self) -> f64 {
        signed_area(&self.points)
    }
}

/// The loops of a sketch, and what could not be closed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub loops: Vec<Loop>,
    /// Lines and arcs that are not part of a loop.
    pub open: Vec<EntityId>,
}

impl Profile {
    /// The area filled, holes taken out.
    pub fn area(&self) -> f64 {
        self.loops.iter().map(|l| if l.is_hole() { -1.0 } else { 1.0 } * l.signed_area().abs()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.loops.is_empty()
    }
}

/// An edge of the loop graph: a line or arc between two places.
struct Edge {
    entity: EntityId,
    from: usize,
    to: usize,
}

/// The sketch's profile, arcs and circles cut into `segments` per turn.
pub fn profile(sketch: &Sketch, segments: u32) -> Profile {
    let segments = segments.max(3);
    let scale = sketch.points.iter().fold(1.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
    let tolerance = 1e-7 * scale;

    // Places where ends meet, and each end's place.
    let mut places: Vec<Point> = Vec::new();
    let mut place_of = |p: PointId| {
        let at = sketch.point(p);
        match places.iter().position(|q| q.distance(at) <= tolerance) {
            Some(i) => i,
            None => {
                places.push(at);
                places.len() - 1
            }
        }
    };
    let mut edges = Vec::new();
    let mut circles = Vec::new();
    for (e, entity) in sketch.entities.iter().enumerate() {
        if entity.construction {
            continue;
        }
        let entity_id = EntityId(e);
        match entity.geometry {
            Geometry::Line { a, b } => {
                let (from, to) = (place_of(a), place_of(b));
                if from != to {
                    edges.push(Edge { entity: entity_id, from, to });
                }
            }
            Geometry::Arc { start, end, .. } => {
                let (from, to) = (place_of(start), place_of(end));
                edges.push(Edge { entity: entity_id, from, to });
            }
            Geometry::Circle { radius, .. } if radius > 0.0 => circles.push(entity_id),
            Geometry::Circle { .. } => {}
        }
    }
    let mut degree = vec![0usize; places.len()];
    for edge in &edges {
        degree[edge.from] += 1;
        degree[edge.to] += 1;
    }

    // Chains through places where exactly two ends meet.
    let mut used = vec![false; edges.len()];
    let mut found: Vec<(EntityId, Loop)> = Vec::new();
    let mut open = Vec::new();
    for first in 0..edges.len() {
        if used[first] {
            continue;
        }
        let mut chain = vec![(first, true)];
        used[first] = true;
        let mut closed = false;
        let mut at = edges[first].to;
        let mut current = first;
        loop {
            if at == edges[first].from {
                closed = degree[at] == 2;
                break;
            }
            if degree[at] != 2 {
                break;
            }
            let Some(next) = (0..edges.len())
                .find(|&k| k != current && !used[k] && (edges[k].from == at || edges[k].to == at))
            else {
                break;
            };
            used[next] = true;
            let forward = edges[next].from == at;
            chain.push((next, forward));
            at = if forward { edges[next].to } else { edges[next].from };
            current = next;
        }
        if !closed || chain.iter().any(|&(k, _)| degree[edges[k].from] != 2 || degree[edges[k].to] != 2) {
            open.extend(chain.iter().map(|&(k, _)| edges[k].entity));
            continue;
        }
        let mut points = Vec::new();
        for &(k, forward) in &chain {
            points.extend(edge_points(sketch, edges[k].entity, forward, segments));
        }
        let entities = chain.iter().map(|&(k, _)| edges[k].entity).collect();
        found.push((edges[first].entity, Loop { points, entities, depth: 0 }));
    }
    for e in circles {
        let Geometry::Circle { center, radius } = sketch.entity(e).geometry else { unreachable!() };
        let c = sketch.point(center);
        let points = (0..segments)
            .map(|k| {
                let a = TAU * k as f64 / segments as f64;
                Point::new(c.x + radius * a.cos(), c.y + radius * a.sin())
            })
            .collect();
        found.push((e, Loop { points, entities: vec![e], depth: 0 }));
    }
    open.sort();
    open.dedup();
    found.sort_by_key(|(e, _)| *e);

    let mut loops: Vec<Loop> = found.into_iter().map(|(_, l)| l).collect();
    let depths: Vec<usize> = (0..loops.len())
        .map(|i| {
            let probe = loops[i].points[0].midpoint(loops[i].points[1 % loops[i].points.len()]);
            (0..loops.len()).filter(|&j| j != i && contains(&loops[j].points, probe)).count()
        })
        .collect();
    for (l, depth) in loops.iter_mut().zip(depths) {
        l.depth = depth;
        if (l.signed_area() > 0.0) == l.is_hole() {
            l.points.reverse();
            l.entities.reverse();
        }
    }
    Profile { loops, open }
}

/// The corners an edge adds to a loop, walked forward or backward: its
/// first end, then for an arc the corners along it; the last end is the
/// next edge's first.
fn edge_points(sketch: &Sketch, entity: EntityId, forward: bool, segments: u32) -> Vec<Point> {
    match sketch.entity(entity).geometry {
        Geometry::Line { a, b } => vec![sketch.point(if forward { a } else { b })],
        Geometry::Arc { center, start, end } => {
            let (c, s, e) = (sketch.point(center), sketch.point(start), sketch.point(end));
            let r = c.distance(s);
            let a0 = (s.y - c.y).atan2(s.x - c.x);
            let mut sweep = (e.y - c.y).atan2(e.x - c.x) - a0;
            if sweep <= 1e-12 {
                sweep += TAU;
            }
            let n = ((segments as f64 * sweep / TAU) - 1e-9).ceil().max(1.0) as usize;
            let at = |k: usize| {
                let a = a0 + sweep * k as f64 / n as f64;
                Point::new(c.x + r * a.cos(), c.y + r * a.sin())
            };
            if forward {
                std::iter::once(s).chain((1..n).map(at)).collect()
            } else {
                std::iter::once(e).chain((1..n).rev().map(at)).collect()
            }
        }
        Geometry::Circle { .. } => unreachable!("circles are loops of their own"),
    }
}

pub(crate) fn signed_area(points: &[Point]) -> f64 {
    let n = points.len();
    (0..n).map(|i| {
        let (p, q) = (points[i], points[(i + 1) % n]);
        p.x * q.y - q.x * p.y
    })
    .sum::<f64>()
        / 2.0
}

/// Whether `p` is inside the polygon, by the even-odd rule.
fn contains(polygon: &[Point], p: Point) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}
