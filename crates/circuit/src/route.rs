//! Where wires run: a smooth curve that leaves each pin the way its `@pin`
//! points, and the tube meshes the views draw them with. Wires are drawn,
//! not modeled: they never reach the geometry kernel or an export.

use crate::{Circuit, End};

/// Points along each wire's curve, in the circuit's coordinates.
const SEGMENTS: usize = 24;

/// How far apart wires that share a pin fan out, in millimeters.
const SPREAD: f64 = 1.2;

/// The curve of every wire of `circuit`, in the order of its wires.
pub fn paths(circuit: &Circuit) -> Vec<Vec<[f64; 3]>> {
    (0..circuit.wires.len()).map(|i| path(circuit, i)).collect()
}

/// The curve of wire `index`: a cubic Bézier leaving each pin along its
/// direction. Wires that share a pin, or run between the same two pins,
/// leave it further out and bow to either side, so that each stays apart.
pub fn path(circuit: &Circuit, index: usize) -> Vec<[f64; 3]> {
    let wire = &circuit.wires[index];
    let (a, b) = (circuit.place(wire.from), circuit.place(wire.to));
    let chord = sub(b.position, a.position);
    let distance = length(chord);
    let (rank_a, count_a) = rank(circuit, index, wire.from);
    let (rank_b, count_b) = rank(circuit, index, wire.to);
    let lead = (distance * 0.4).clamp(3.0, 30.0);
    let handle_a = lead + rank_a as f64 * SPREAD;
    let handle_b = lead + rank_b as f64 * SPREAD;
    // Sideways, across the chord and the way the wire leaves.
    let side = normalize(cross(a.direction, chord))
        .or_else(|| normalize(cross([0.0, 0.0, 1.0], chord)))
        .or_else(|| normalize(cross([1.0, 0.0, 0.0], a.direction)))
        .unwrap_or([1.0, 0.0, 0.0]);
    let fan = |rank: usize, count: usize| (rank as f64 - (count as f64 - 1.0) / 2.0) * SPREAD;
    let c1 = add(add(a.position, scale(a.direction, handle_a)), scale(side, fan(rank_a, count_a)));
    let c2 = add(add(b.position, scale(b.direction, handle_b)), scale(side, fan(rank_b, count_b)));
    (0..=SEGMENTS)
        .map(|i| {
            let t = i as f64 / SEGMENTS as f64;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            let p = [a.position, c1, c2, b.position];
            std::array::from_fn(|k| (0..4).map(|j| w[j] * p[j][k]).sum())
        })
        .collect()
}

/// The place of wire `index` among the wires at `end`, and how many there
/// are.
fn rank(circuit: &Circuit, index: usize, end: End) -> (usize, usize) {
    let at_end = |i: usize| circuit.wires[i].from == end || circuit.wires[i].to == end;
    let rank = (0..index).filter(|&i| at_end(i)).count();
    let count = (0..circuit.wires.len()).filter(|&i| at_end(i)).count();
    (rank, count)
}

/// The length of a curve, in millimeters.
pub fn length_of(points: &[[f64; 3]]) -> f64 {
    points.windows(2).map(|pair| length(sub(pair[1], pair[0]))).sum()
}

/// A tube mesh around a curve: `sides` vertices around each point, without
/// end caps (the ends sit in pins).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tube {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Counter-clockwise triangles, seen from outside.
    pub indices: Vec<u32>,
}

pub fn tube(points: &[[f64; 3]], radius: f64, sides: usize) -> Tube {
    let mut tube = Tube::default();
    if points.len() < 2 || sides < 3 {
        return tube;
    }
    // A frame carried along the curve, so the tube does not twist.
    let tangent = |i: usize| {
        let (p, q) = (points[i.saturating_sub(1)], points[(i + 1).min(points.len() - 1)]);
        normalize(sub(q, p)).unwrap_or([0.0, 0.0, 1.0])
    };
    let first = tangent(0);
    let mut normal = normalize(cross(first, [0.0, 0.0, 1.0]))
        .or_else(|| normalize(cross(first, [1.0, 0.0, 0.0])))
        .unwrap_or([1.0, 0.0, 0.0]);
    for (i, &point) in points.iter().enumerate() {
        let t = tangent(i);
        // Project the previous normal onto the plane across this tangent.
        normal = normalize(sub(normal, scale(t, dot(normal, t)))).unwrap_or(normal);
        let binormal = cross(t, normal);
        for k in 0..sides {
            let angle = k as f64 / sides as f64 * std::f64::consts::TAU;
            let out = add(scale(normal, angle.cos()), scale(binormal, angle.sin()));
            tube.positions.push(add(point, scale(out, radius)).map(|c| c as f32));
            tube.normals.push(out.map(|c| c as f32));
        }
    }
    let ring = sides as u32;
    for i in 0..points.len() as u32 - 1 {
        for k in 0..ring {
            let (a, b) = (i * ring + k, i * ring + (k + 1) % ring);
            let (c, d) = (a + ring, b + ring);
            tube.indices.extend([a, b, d, a, d, c]);
        }
    }
    tube
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = length(a);
    (l > 1e-12).then(|| scale(a, 1.0 / l))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tube_has_a_ring_per_point_and_faces_outward() {
        let points = [[0.0, 0.0, 0.0], [0.0, 0.0, 5.0], [0.0, 5.0, 10.0]];
        let mesh = tube(&points, 0.5, 8);
        assert_eq!(mesh.positions.len(), 24);
        assert_eq!(mesh.indices.len(), 2 * 8 * 6);
        // Each vertex sits at the radius from its point, its normal pointing out.
        for (i, (p, n)) in mesh.positions.iter().zip(&mesh.normals).enumerate() {
            let center = points[i / 8];
            let out = [p[0] as f64 - center[0], p[1] as f64 - center[1], p[2] as f64 - center[2]];
            assert!((length(out) - 0.5).abs() < 1e-5);
            assert!(dot(out, n.map(f64::from)) > 0.49);
        }
        // The first triangle turns counter-clockwise seen from outside.
        let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[mesh.indices[k] as usize].map(f64::from));
        let face = cross(sub(b, a), sub(c, a));
        assert!(dot(face, mesh.normals[mesh.indices[0] as usize].map(f64::from)) > 0.0);
    }

    #[test]
    fn lengths_add_up() {
        assert_eq!(length_of(&[[0.0, 0.0, 0.0], [3.0, 4.0, 0.0], [3.0, 4.0, 2.0]]), 7.0);
    }
}
