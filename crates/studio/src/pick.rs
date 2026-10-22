//! What lies under the pointer in a circuit: a pin's dot, a wire's tube or
//! a part's box, found along the ray through the pointer; and the nets the
//! wires join pins into.

use std::collections::{BTreeSet, HashSet};

use c2f_circuit::{Circuit, End};
use glam::Vec3;

use crate::rewire::Side;
use crate::wiring::{WIRE_RADIUS, Wiring};

/// Something of the circuit, under the pointer or selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Pin(End),
    /// By index in the circuit's wires.
    Wire(usize),
    /// By index in the circuit's parts.
    Part(usize),
}

/// A ray from the eye; `direction` has unit length.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
    /// How wide a few pixels are, per unit of distance along the ray: what
    /// is that close counts as under the pointer, however far it is.
    pub slack: f32,
}

impl Ray {
    /// The distance along the ray where it comes within `radius` (plus the
    /// slack) of `center`.
    pub fn sphere(&self, center: Vec3, radius: f32) -> Option<f32> {
        let along = (center - self.origin).dot(self.direction);
        if along <= 0.0 {
            return None;
        }
        let closest = self.origin + self.direction * along;
        (closest.distance(center) <= radius + self.slack * along).then_some(along)
    }

    /// The distance along the ray where it comes within `radius` (plus the
    /// slack) of the segment `a`..`b`.
    pub fn segment(&self, a: Vec3, b: Vec3, radius: f32) -> Option<f32> {
        let (along, distance) = self.closest(a, b)?;
        (distance <= radius + self.slack * along).then_some(along)
    }

    /// Where the segment `a`..`b` comes closest to the ray: the distance
    /// along the ray, and how close; `None` when it is behind the eye.
    fn closest(&self, a: Vec3, b: Vec3) -> Option<(f32, f32)> {
        // The closest points of two lines, kept within the segment.
        let (u, v, w) = (self.direction, b - a, self.origin - a);
        let (bb, c, d, e) = (u.dot(v), v.dot(v), u.dot(w), v.dot(w));
        let denominator = c - bb * bb;
        // A point, or a segment along the ray: any point of it will do.
        let degenerate = c < 1e-12 || denominator.abs() < 1e-9;
        let s = if degenerate { 0.0 } else { ((e - bb * d) / denominator).clamp(0.0, 1.0) };
        let on_segment = a + v * s;
        let along = (on_segment - self.origin).dot(u);
        if along <= 0.0 {
            return None;
        }
        Some((along, (self.origin + u * along).distance(on_segment)))
    }

    /// The distance along the ray where it enters the box.
    pub fn aabb(&self, lo: Vec3, hi: Vec3) -> Option<f32> {
        let inverse = self.direction.recip();
        let (t1, t2) = ((lo - self.origin) * inverse, (hi - self.origin) * inverse);
        let near = t1.min(t2).max_element();
        let far = t1.max(t2).min_element();
        (far >= near.max(0.0)).then_some(near.max(0.0))
    }
}

/// What the ray meets: a pin before a wire before a part, since the
/// smaller ones sit on the larger ones; the nearest of each kind.
pub fn pick(wiring: &Wiring, part_boxes: &[Option<(Vec3, Vec3)>], ray: &Ray) -> Option<Hit> {
    let nearest =
        |hits: &mut dyn Iterator<Item = (f32, Hit)>| hits.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, hit)| hit);
    let circuit = &wiring.circuit;
    let pins = &mut wiring.pins().filter_map(|end| {
        let center = Vec3::from(circuit.place(end).position.map(|c| c as f32));
        ray.sphere(center, wiring.pin_radius(end)).map(|t| (t, Hit::Pin(end)))
    });
    if let Some(hit) = nearest(pins) {
        return Some(hit);
    }
    let wires = &mut wiring.paths.iter().enumerate().filter_map(|(index, path)| {
        let points: Vec<Vec3> = path.iter().map(|p| Vec3::from(p.map(|c| c as f32))).collect();
        let along = points
            .windows(2)
            .filter_map(|pair| ray.segment(pair[0], pair[1], WIRE_RADIUS as f32))
            .min_by(f32::total_cmp)?;
        Some((along, Hit::Wire(index)))
    });
    if let Some(hit) = nearest(wires) {
        return Some(hit);
    }
    // Boxes from a render before the circuit lost parts name none.
    let parts = &mut part_boxes
        .iter()
        .take(circuit.parts.len())
        .enumerate()
        .filter_map(|(index, b)| b.and_then(|(lo, hi)| ray.aabb(lo, hi)).map(|t| (t, Hit::Part(index))));
    nearest(parts)
}

/// The end of wire `index` the ray meets it near: its first or last third,
/// where a press grabs that end; `None` in the middle.
pub fn wire_side(wiring: &Wiring, index: usize, ray: &Ray) -> Option<Side> {
    let path = wiring.paths.get(index)?;
    let points: Vec<Vec3> = path.iter().map(|p| Vec3::from(p.map(|c| c as f32))).collect();
    let segments = points.len().checked_sub(1).filter(|&n| n > 0)?;
    let (nearest, _) = points
        .windows(2)
        .enumerate()
        .filter_map(|(i, pair)| ray.closest(pair[0], pair[1]).map(|(_, distance)| (i, distance)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let along = (nearest as f32 + 0.5) / segments as f32;
    if along < 1.0 / 3.0 {
        Some(Side::From)
    } else if along > 2.0 / 3.0 {
        Some(Side::To)
    } else {
        None
    }
}

/// The pins joined to `end` through wires, it among them, and those wires.
pub fn net(circuit: &Circuit, end: End) -> (HashSet<End>, BTreeSet<usize>) {
    let mut pins = HashSet::from([end]);
    let mut wires = BTreeSet::new();
    let mut queue = vec![end];
    while let Some(at) = queue.pop() {
        for (index, wire) in circuit.wires.iter().enumerate() {
            let other = if wire.from == at {
                wire.to
            } else if wire.to == at {
                wire.from
            } else {
                continue;
            };
            wires.insert(index);
            if pins.insert(other) {
                queue.push(other);
            }
        }
    }
    (pins, wires)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::tests::circuit;

    fn ray(origin: [f32; 3], toward: [f32; 3]) -> Ray {
        let origin = Vec3::from(origin);
        Ray { origin, direction: (Vec3::from(toward) - origin).normalize(), slack: 0.0 }
    }

    #[test]
    fn rays_meet_spheres_segments_and_boxes() {
        let r = ray([0.0, 0.0, 10.0], [0.0, 0.0, 0.0]);
        assert_eq!(r.sphere(Vec3::new(0.3, 0.0, 0.0), 0.5), Some(10.0));
        assert_eq!(r.sphere(Vec3::new(0.6, 0.0, 0.0), 0.5), None);
        assert_eq!(r.sphere(Vec3::new(0.0, 0.0, 20.0), 0.5), None, "behind the eye");
        // A wider slack reaches further at a distance.
        assert!(Ray { slack: 0.02, ..r }.sphere(Vec3::new(0.6, 0.0, 0.0), 0.5).is_some());

        let near = r.segment(Vec3::new(-5.0, 0.3, 2.0), Vec3::new(5.0, 0.3, 2.0), 0.4).unwrap();
        assert!((near - 8.0).abs() < 1e-5);
        assert_eq!(r.segment(Vec3::new(-5.0, 0.5, 2.0), Vec3::new(5.0, 0.5, 2.0), 0.4), None);
        assert_eq!(r.segment(Vec3::new(1.0, 0.0, 2.0), Vec3::new(5.0, 0.0, 2.0), 0.4), None, "past its end");

        assert_eq!(r.aabb(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 3.0)), Some(7.0));
        assert_eq!(r.aabb(Vec3::new(2.0, -1.0, 0.0), Vec3::new(3.0, 1.0, 3.0)), None);
    }

    #[test]
    fn pins_come_before_wires_and_wires_before_parts() {
        let wiring = Wiring::new(circuit());
        let boxes = [
            Some((Vec3::ZERO, Vec3::new(10.0, 5.0, 1.0))),
            Some((Vec3::new(0.0, 30.0, 0.0), Vec3::new(10.0, 35.0, 1.0))),
        ];
        let down = |x: f32, y: f32| pick(&wiring, &boxes, &ray([x, y, 50.0], [x, y, 0.0]));
        // Straight down on a.A, on its board.
        assert_eq!(down(0.0, 0.0), Some(Hit::Pin(End { part: 0, pin: 0 })));
        // Down on a.C, which has no wire: still a pin.
        assert_eq!(down(10.0, 0.0), Some(Hit::Pin(End { part: 0, pin: 2 })));
        // Elsewhere on the board.
        assert_eq!(down(7.0, 4.0), Some(Hit::Part(0)));
        // Down on the middle of the third wire.
        let mid = wiring.paths[2][wiring.paths[2].len() / 2].map(|c| c as f32);
        assert_eq!(pick(&wiring, &boxes, &ray([mid[0], mid[1], 80.0], mid)), Some(Hit::Wire(2)));
        assert_eq!(pick(&wiring, &boxes, &ray([50.0, 50.0, 50.0], [60.0, 60.0, 0.0])), None);
    }

    #[test]
    fn a_wire_is_grabbed_by_the_end_nearer_the_press() {
        let wiring = Wiring::new(circuit());
        let at = |t: f64| {
            let path = &wiring.paths[2];
            let p = path[((path.len() - 1) as f64 * t).round() as usize].map(|c| c as f32);
            ray([p[0], p[1], p[2] + 60.0], p)
        };
        // Just above each end's pin the pin wins; along the wire, the end.
        assert_eq!(pick(&wiring, &[], &at(0.0)), Some(Hit::Pin(End { part: 0, pin: 1 })));
        assert_eq!(wire_side(&wiring, 2, &at(0.2)), Some(Side::From));
        assert_eq!(wire_side(&wiring, 2, &at(0.5)), None);
        assert_eq!(wire_side(&wiring, 2, &at(0.8)), Some(Side::To));
        assert_eq!(wire_side(&wiring, 7, &at(0.8)), None);
    }

    #[test]
    fn boxes_of_parts_the_circuit_no_longer_has_name_nothing() {
        let wiring = Wiring::new(circuit());
        let stale = Some((Vec3::new(100.0, 100.0, 0.0), Vec3::new(110.0, 110.0, 5.0)));
        let boxes = [None, None, stale];
        assert_eq!(pick(&wiring, &boxes, &ray([105.0, 105.0, 50.0], [105.0, 105.0, 0.0])), None);
    }

    #[test]
    fn a_net_follows_wires_from_pin_to_pin() {
        let circuit = circuit();
        let a = |pin| End { part: 0, pin };
        let b = |pin| End { part: 1, pin };
        // a.A–b.A, a.A–b.B and a.B–b.B: all four in one net.
        let (pins, wires) = net(&circuit, b(0));
        assert_eq!(pins, HashSet::from([a(0), a(1), b(0), b(1)]));
        assert_eq!(wires, BTreeSet::from([0, 1, 2]));
        // A pin without wires is a net of its own.
        assert_eq!(net(&circuit, a(2)), (HashSet::from([a(2)]), BTreeSet::new()));
    }
}
