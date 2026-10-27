//! Snapping: the point of the sketch a cursor is close enough to land on.
//! Pure geometry, in the sketch's coordinates; the editor turns its pixel
//! tolerance into millimeters and adds the constraint the snap implies
//! (a shared point for an endpoint, [`crate::Constraint::Midpoint`],
//! [`crate::Constraint::Coincident`] with a center).

use crate::model::{EntityId, Geometry, Point, PointId, Sketch};

/// What a snap lands on, by priority: an endpoint wins over a center,
/// which wins over a midpoint, when several are within reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    /// A point of the sketch: a line's or arc's end, or a lone point.
    Endpoint,
    /// A circle's or arc's center.
    Center,
    /// Halfway along a line, or along an arc.
    Midpoint,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub kind: SnapKind,
    pub at: Point,
    /// The point snapped to, for endpoints and centers.
    pub point: Option<PointId>,
    /// The entity whose midpoint or center it is.
    pub entity: Option<EntityId>,
}

/// The snap target within `tolerance` of `cursor`: of the highest
/// priority kind in reach, the nearest. Construction geometry snaps too.
pub fn snap(sketch: &Sketch, cursor: Point, tolerance: f64) -> Option<Snap> {
    let mut targets: Vec<Snap> = Vec::new();
    let mut centers = Vec::new();
    for (e, entity) in sketch.entities.iter().enumerate() {
        let entity_id = Some(EntityId(e));
        match entity.geometry {
            Geometry::Line { a, b } => targets.push(Snap {
                kind: SnapKind::Midpoint,
                at: sketch.point(a).midpoint(sketch.point(b)),
                point: None,
                entity: entity_id,
            }),
            Geometry::Circle { center, .. } => centers.push((center, e)),
            Geometry::Arc { center, start, end } => {
                centers.push((center, e));
                let c = sketch.point(center);
                let (s, t) = (sketch.point(start), sketch.point(end));
                let a0 = (s.y - c.y).atan2(s.x - c.x);
                let mut sweep = (t.y - c.y).atan2(t.x - c.x) - a0;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                let middle = a0 + sweep / 2.0;
                let r = c.distance(s);
                targets.push(Snap {
                    kind: SnapKind::Midpoint,
                    at: Point::new(c.x + r * middle.cos(), c.y + r * middle.sin()),
                    point: None,
                    entity: entity_id,
                });
            }
        }
    }
    for (i, &at) in sketch.points.iter().enumerate() {
        let id = PointId(i);
        match centers.iter().find(|(c, _)| *c == id) {
            Some(&(_, e)) => {
                targets.push(Snap { kind: SnapKind::Center, at, point: Some(id), entity: Some(EntityId(e)) })
            }
            None => targets.push(Snap { kind: SnapKind::Endpoint, at, point: Some(id), entity: None }),
        }
    }
    targets
        .into_iter()
        .map(|t| (t, t.at.distance(cursor)))
        .filter(|(_, d)| *d <= tolerance)
        .min_by(|(a, da), (b, db)| a.kind.cmp(&b.kind).then(da.total_cmp(db)))
        .map(|(t, _)| t)
}
