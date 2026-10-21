//! Primitive solids and outlines, tessellated exactly as the reference
//! implementation (2021.01) does, so that vertices land where the
//! reference puts them.

use std::f64::consts::PI;

/// A triangle mesh: corner positions and counter-clockwise triangles.
pub struct TriMesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// Below this radius a circle gets the minimum 3 fragments.
const GRID_FINE: f64 = 0.000_000_953_674_316_406_25;

/// How many segments approximate a circle of radius `r`, from `$fn`,
/// `$fs` and `$fa` (the reference's `get_fragments_from_r`).
pub fn fragments(r: f64, fn_: f64, fs: f64, fa: f64) -> u32 {
    if r < GRID_FINE || !r.is_finite() {
        return 3;
    }
    if fn_ > 0.0 {
        return if fn_ >= 3.0 { fn_ as u32 } else { 3 };
    }
    (360.0 / fa).min(r * 2.0 * PI / fs).max(5.0).ceil() as u32
}

/// Points of a circle, starting on +x and going counter-clockwise.
pub fn circle(r: f64, n: u32) -> Vec<[f64; 2]> {
    (0..n)
        .map(|i| {
            let phi = 2.0 * PI * i as f64 / n as f64;
            [r * phi.cos(), r * phi.sin()]
        })
        .collect()
}

pub fn cube(size: [f64; 3], center: bool) -> Option<TriMesh> {
    let [x, y, z] = size;
    if !(x > 0.0 && y > 0.0 && z > 0.0 && x.is_finite() && y.is_finite() && z.is_finite()) {
        return None;
    }
    let (x0, y0, z0) = if center { (-x / 2.0, -y / 2.0, -z / 2.0) } else { (0.0, 0.0, 0.0) };
    let (x1, y1, z1) = (x0 + x, y0 + y, z0 + z);
    let positions = vec![
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    let quads = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
    let triangles = quads.iter().flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]]).collect();
    Some(TriMesh { positions, triangles })
}

/// Rings of `fragments` points; `rings = (fragments + 1) / 2`, ring `i`
/// at polar angle `π (i + 0.5) / rings`.
pub fn sphere(r: f64, fragments: u32) -> Option<TriMesh> {
    if !(r.is_finite() && r > 0.0) {
        return None;
    }
    let n = fragments.max(3);
    let rings = (n + 1) / 2;
    let mut positions = Vec::with_capacity((rings * n) as usize);
    for i in 0..rings {
        let phi = PI * (i as f64 + 0.5) / rings as f64;
        let (ring_r, z) = (r * phi.sin(), r * phi.cos());
        positions.extend(circle(ring_r, n).into_iter().map(|[x, y]| [x, y, z]));
    }
    let at = |ring: u32, j: u32| ring * n + j % n;
    let mut triangles = Vec::new();
    // Top cap, seen from above: counter-clockwise.
    for j in 1..n - 1 {
        triangles.push([at(0, 0), at(0, j), at(0, j + 1)]);
    }
    for i in 0..rings - 1 {
        for j in 0..n {
            let (a, a1, b, b1) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            triangles.push([a, b, b1]);
            triangles.push([a, b1, a1]);
        }
    }
    let last = rings - 1;
    for j in 1..n - 1 {
        triangles.push([at(last, 0), at(last, j + 1), at(last, j)]);
    }
    Some(TriMesh { positions, triangles })
}

/// A frustum from radius `r1` at the bottom to `r2` at the top; a zero
/// radius makes a cone point.
pub fn cylinder(h: f64, r1: f64, r2: f64, center: bool, fragments: u32) -> Option<TriMesh> {
    if h.is_nan() || h <= 0.0 || r1 < 0.0 || r2 < 0.0 || (r1 <= 0.0 && r2 <= 0.0) || !(h + r1 + r2).is_finite() {
        return None;
    }
    let n = fragments.max(3);
    let (z1, z2) = if center { (-h / 2.0, h / 2.0) } else { (0.0, h) };
    let mut positions = Vec::new();
    let ring = |r: f64, z: f64, positions: &mut Vec<[f64; 3]>| -> Vec<u32> {
        if r > 0.0 {
            let start = positions.len() as u32;
            positions.extend(circle(r, n).into_iter().map(|[x, y]| [x, y, z]));
            (start..start + n).collect()
        } else {
            positions.push([0.0, 0.0, z]);
            vec![positions.len() as u32 - 1; n as usize]
        }
    };
    let bottom = ring(r1, z1, &mut positions);
    let top = ring(r2, z2, &mut positions);
    let mut triangles = Vec::new();
    let k = n as usize;
    if r1 > 0.0 {
        for j in 1..k - 1 {
            triangles.push([bottom[0], bottom[j + 1], bottom[j]]);
        }
    }
    if r2 > 0.0 {
        for j in 1..k - 1 {
            triangles.push([top[0], top[j], top[j + 1]]);
        }
    }
    for j in 0..k {
        let (b, b1, t, t1) = (bottom[j], bottom[(j + 1) % k], top[j], top[(j + 1) % k]);
        if b != b1 {
            triangles.push([b, b1, t1]);
        }
        if t != t1 {
            triangles.push([b, t1, t]);
        }
    }
    Some(TriMesh { positions, triangles })
}

/// `linear_extrude` as the reference meshes it: `slices` layers, each
/// rotated clockwise by `twist / slices` and scaled linearly toward
/// `scale`, with each side quad split along its shorter diagonal, as in
/// the reference's output; on a tie, the twist direction picks it.
pub fn linear_extrude(
    contours: &[Vec<[f64; 2]>],
    cap: &[[u32; 3]],
    height: f64,
    center: bool,
    twist: f64,
    slices: u32,
    scale: [f64; 2],
) -> Option<TriMesh> {
    let slices = slices.max(1);
    let total: u32 = contours.iter().map(|c| c.len() as u32).sum();
    if total == 0 || height.is_nan() || height <= 0.0 {
        return None;
    }
    let z0 = if center { -height / 2.0 } else { 0.0 };
    let mut positions = Vec::with_capacity(((slices + 1) * total) as usize);
    for k in 0..=slices {
        let t = k as f64 / slices as f64;
        let angle = (-twist * t).to_radians();
        let (sin, cos) = angle.sin_cos();
        let (sx, sy) = (1.0 + (scale[0] - 1.0) * t, 1.0 + (scale[1] - 1.0) * t);
        for contour in contours {
            for &[x, y] in contour {
                positions.push([sx * (cos * x - sin * y), sy * (sin * x + cos * y), z0 + height * t]);
            }
        }
    }
    let at = |k: u32, i: u32| k * total + i;
    let distance = |a: u32, b: u32| {
        let (p, q) = (positions[a as usize], positions[b as usize]);
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
    };
    let mut triangles = Vec::new();
    let mut start = 0u32;
    for contour in contours {
        let n = contour.len() as u32;
        for k in 0..slices {
            for i in 0..n {
                let j = (i + 1) % n;
                let (p1, c1, p2, c2) = (at(k, start + i), at(k, start + j), at(k + 1, start + i), at(k + 1, start + j));
                let (up, down) = (distance(p1, c2), distance(p2, c1));
                let tie = (up - down).abs() <= 1e-9 * up.max(down);
                if if tie { twist >= 0.0 } else { up < down } {
                    triangles.push([p1, c1, c2]);
                    triangles.push([p1, c2, p2]);
                } else {
                    triangles.push([p1, c1, p2]);
                    triangles.push([c1, c2, p2]);
                }
            }
        }
        start += n;
    }
    for &[a, b, c] in cap {
        triangles.push([at(0, a), at(0, c), at(0, b)]);
        triangles.push([at(slices, a), at(slices, b), at(slices, c)]);
    }
    Some(TriMesh { positions, triangles })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_counts() {
        // Defaults: $fa = 12, $fs = 2.
        assert_eq!(fragments(1.0, 0.0, 2.0, 12.0), 5);
        assert_eq!(fragments(10.0, 0.0, 2.0, 12.0), 30);
        assert_eq!(fragments(5.0, 0.0, 2.0, 12.0), 16);
        assert_eq!(fragments(5.0, 7.9, 2.0, 12.0), 7);
        assert_eq!(fragments(5.0, 2.0, 2.0, 12.0), 3);
    }

    #[test]
    fn closed_meshes() {
        // Every edge used once in each direction.
        for mesh in [
            cube([1.0, 2.0, 3.0], false).unwrap(),
            sphere(1.0, 7).unwrap(),
            cylinder(2.0, 1.0, 0.0, true, 9).unwrap(),
            cylinder(2.0, 1.0, 3.0, false, 5).unwrap(),
        ] {
            let mut edges = std::collections::HashMap::new();
            for t in &mesh.triangles {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    *edges.entry((a, b)).or_insert(0) += 1;
                }
            }
            for (&(a, b), &count) in &edges {
                assert_eq!(count, 1);
                assert_eq!(edges.get(&(b, a)), Some(&1), "edge {a}-{b} has no twin");
            }
        }
    }
}
