//! A scan set straight: what the phone's reconstruction gives keeps the
//! angle the object had while it was photographed. This takes off the
//! floating bits (the stand, noise), stands the scan on the flat face it
//! rests on, turns it so its footprint runs along X and Y, and puts it in
//! the middle, on z = 0.
//!
//! Object Capture knows where down is, so the scan's Z is already close:
//! only a face within [`MAX_TILT`] of facing down is taken to stand it
//! on, so that a part is never tipped over on its side.

use std::path::Path;

use c2f_eval::node::Segments;

use crate::import::{self, DxfParams, Imported};
use crate::primitives::TriMesh;

/// How far from facing straight down the face a scan stands on may be,
/// in degrees.
pub const MAX_TILT: f64 = 25.0;

/// A part of the scan with less than this share of the largest part's
/// area is a floating bit, and goes.
const ISLAND_SHARE: f64 = 0.05;

/// The face a scan stands on holds at least this share of its area;
/// under it (a ball, a figurine), down stays as the phone saw it.
const BASE_SHARE: f64 = 0.02;

/// Faces within this angle of each other, in degrees, are one flat face.
const FLAT: f64 = 3.0;

/// A scan set straight, and what was done to it.
#[derive(Debug, Clone)]
pub struct Leveled {
    pub mesh: TriMesh,
    /// The floating parts taken off.
    pub islands: usize,
    /// How far the scan was tipped to stand on its base, in degrees; 0
    /// when it has none.
    pub tilt: f64,
    /// How far it was turned about Z, in degrees.
    pub turn: f64,
    /// Whether a flat face to stand it on was found.
    pub base: bool,
    /// Its size, in mm, along X, Y and Z.
    pub size: [f64; 3],
}

/// Sets `mesh` straight: see the module.
pub fn level(mesh: &TriMesh) -> Leveled {
    let (mut mesh, islands) = without_islands(mesh);
    let (tilt, base) = match base_normal(&mesh) {
        Some(n) => {
            let r = rotation_between(n, [0.0, 0.0, -1.0]);
            rotate(&mut mesh, &r);
            ((-n[2]).clamp(-1.0, 1.0).acos().to_degrees(), true)
        }
        None => (0.0, false),
    };
    let turn = footprint_angle(&mesh);
    rotate(&mut mesh, &about_z(-turn.to_radians()));
    let (lo, hi) = bounds(&mesh);
    let shift = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, lo[2]];
    for p in &mut mesh.positions {
        for k in 0..3 {
            p[k] -= shift[k];
        }
    }
    let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    Leveled { mesh, islands, tilt, turn, base, size }
}

/// Reads the mesh in `path` (STL, OBJ, 3MF… as `import()` reads them),
/// sets it straight and writes it to `out` as a binary STL, in mm. `out`
/// may be `path`: it is replaced in one go.
pub fn level_file(path: &Path, out: &Path) -> Result<Leveled, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let params = DxfParams { layer: "", origin: [0.0, 0.0], scale: 1.0, segments: Segments { fn_: 0.0, fa: 12.0, fs: 2.0 } };
    let mut warnings = Vec::new();
    let mesh = match import::import(path, &name, &params, "", &mut warnings) {
        Some(Imported::Solid(meshes)) => merged(meshes),
        Some(Imported::Flat(_)) => return Err(format!("{name} is a 2D drawing, not a scan")),
        None => return Err(warnings.first().cloned().unwrap_or_else(|| format!("{name} does not read"))),
    };
    if mesh.triangles.is_empty() {
        return Err(format!("{name} holds no triangles"));
    }
    let leveled = level(&mesh);
    let temporary = out.with_extension("leveling.stl");
    std::fs::write(&temporary, stl(&leveled.mesh)).map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, out).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot write {}: {e}", out.display())
    })?;
    Ok(leveled)
}

fn merged(meshes: Vec<TriMesh>) -> TriMesh {
    let mut all = TriMesh { positions: Vec::new(), triangles: Vec::new() };
    for mesh in meshes {
        let base = all.positions.len() as u32;
        all.positions.extend(mesh.positions);
        all.triangles.extend(mesh.triangles.into_iter().map(|t| t.map(|i| i + base)));
    }
    all
}

/// A binary STL of `mesh`, in mm.
pub fn stl(mesh: &TriMesh) -> Vec<u8> {
    let mut out = vec![b' '; 80];
    let header = b"Code2Form scan, millimetres";
    out[..header.len()].copy_from_slice(header);
    out.extend((mesh.triangles.len() as u32).to_le_bytes());
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        let n = normalized(cross(sub(b, a), sub(c, a))).unwrap_or([0.0; 3]);
        for v in [n, a, b, c] {
            for x in v {
                out.extend((x as f32).to_le_bytes());
            }
        }
        out.extend([0, 0]);
    }
    out
}

/// `mesh` without its parts (pieces joined by shared corners) much
/// smaller than the largest one, and how many went.
fn without_islands(mesh: &TriMesh) -> (TriMesh, usize) {
    let mut parent: Vec<u32> = (0..mesh.positions.len() as u32).collect();
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            parent[i as usize] = parent[parent[i as usize] as usize];
            i = parent[i as usize];
        }
        i
    }
    for &[a, b, c] in &mesh.triangles {
        for (x, y) in [(a, b), (b, c)] {
            let (rx, ry) = (root(&mut parent, x), root(&mut parent, y));
            if rx != ry {
                parent[rx as usize] = ry;
            }
        }
    }
    let mut area = std::collections::HashMap::<u32, f64>::new();
    let roots: Vec<u32> = mesh.triangles.iter().map(|t| root(&mut parent, t[0])).collect();
    for (t, r) in mesh.triangles.iter().zip(&roots) {
        *area.entry(*r).or_default() += triangle_area(mesh, t);
    }
    let largest = area.values().copied().fold(0.0, f64::max);
    let kept: std::collections::HashSet<u32> = area.iter().filter(|(_, a)| **a >= ISLAND_SHARE * largest).map(|(r, _)| *r).collect();
    let islands = area.len() - kept.len();
    if islands == 0 {
        return (mesh.clone(), 0);
    }
    // Keep the corners the remaining triangles use, renumbered.
    let mut index = vec![u32::MAX; mesh.positions.len()];
    let mut out = TriMesh { positions: Vec::new(), triangles: Vec::new() };
    for (t, r) in mesh.triangles.iter().zip(&roots) {
        if !kept.contains(r) {
            continue;
        }
        let t = t.map(|i| {
            if index[i as usize] == u32::MAX {
                index[i as usize] = out.positions.len() as u32;
                out.positions.push(mesh.positions[i as usize]);
            }
            index[i as usize]
        });
        out.triangles.push(t);
    }
    (out, islands)
}

/// The outward normal of the flat face the scan stands on: the largest
/// set of triangles facing the same way, within [`MAX_TILT`] of down,
/// that lie on the plane the scan would rest on that way (a sloping
/// flank faces down too, but higher up).
fn base_normal(mesh: &TriMesh) -> Option<[f64; 3]> {
    let total: f64 = mesh.triangles.iter().map(|t| triangle_area(mesh, t)).sum();
    if total <= 0.0 {
        return None;
    }
    let cos_tilt = MAX_TILT.to_radians().cos();
    // Each facing-down triangle: its normal, area and centre.
    let faces: Vec<([f64; 3], f64, [f64; 3])> = mesh
        .triangles
        .iter()
        .filter_map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
            let n = normalized(cross(sub(b, a), sub(c, a)))?;
            let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
            (-n[2] >= cos_tilt).then(|| (n, triangle_area(mesh, t), centre))
        })
        .collect();
    // The normals, binned by their tilt's x and y, a bin a degree wide:
    // the fullest neighbourhoods of bins are the candidates.
    let bin = |n: [f64; 3]| ((n[0].asin().to_degrees()).round() as i32, (n[1].asin().to_degrees()).round() as i32);
    let mut bins = std::collections::HashMap::<(i32, i32), f64>::new();
    for (n, a, _) in &faces {
        *bins.entry(bin(*n)).or_default() += a;
    }
    let around = |(x, y): (i32, i32)| -> f64 { (-1..=1).flat_map(|i| (-1..=1).map(move |j| (x + i, y + j))).filter_map(|k| bins.get(&k)).sum() };
    let mut candidates: Vec<(i32, i32)> = bins.keys().copied().collect();
    candidates.sort_by(|a, b| around(*b).total_cmp(&around(*a)).then(a.cmp(b)));
    let cos_flat = (FLAT * 1.5).to_radians().cos();
    let (lo, hi) = bounds(mesh);
    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    let mut best: Option<(f64, [f64; 3])> = None;
    for peak in candidates.into_iter().take(8) {
        let guess = {
            let (x, y) = (f64::from(peak.0).to_radians().sin(), f64::from(peak.1).to_radians().sin());
            [x, y, -(1.0 - x * x - y * y).max(0.0).sqrt()]
        };
        // The faces near it, averaged by their area, for a normal finer
        // than a bin.
        let mut sum = [0.0; 3];
        for (n, a, _) in &faces {
            if dot(*n, guess) >= cos_flat {
                sum = [sum[0] + n[0] * a, sum[1] + n[1] * a, sum[2] + n[2] * a];
            }
        }
        let Some(n) = normalized(sum) else { continue };
        // The plane the scan rests on that way, and the faces on it.
        let support = mesh.positions.iter().map(|p| dot(*p, n)).fold(f64::NEG_INFINITY, f64::max);
        let near = (0.01 * span).max(0.5);
        let area: f64 = faces.iter().filter(|(f, _, c)| dot(*f, n) >= cos_flat && dot(*c, n) >= support - near).map(|(_, a, _)| a).sum();
        if best.is_none_or(|(b, _)| area > b) {
            best = Some((area, n));
        }
    }
    best.filter(|(area, _)| *area >= BASE_SHARE * total).map(|(_, n)| n)
}

/// The angle, in degrees, of the smallest rectangle around the scan's
/// footprint, its long side along X once turned back by it.
fn footprint_angle(mesh: &TriMesh) -> f64 {
    let hull = convex_hull(mesh.positions.iter().map(|p| [p[0], p[1]]).collect());
    if hull.len() < 3 {
        return 0.0;
    }
    let mut best = (f64::INFINITY, 0.0, 0.0, 0.0);
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
        let (s, c) = angle.sin_cos();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in &hull {
            let q = [p[0] * c + p[1] * s, -p[0] * s + p[1] * c];
            for k in 0..2 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        if w * h < best.0 - 1e-9 {
            best = (w * h, angle, w, h);
        }
    }
    let (_, mut angle, w, h) = best;
    if h > w {
        angle += std::f64::consts::FRAC_PI_2;
    }
    // The smallest turn that lines it up: within ±90°.
    let mut degrees = angle.to_degrees();
    while degrees > 90.0 {
        degrees -= 180.0;
    }
    while degrees <= -90.0 {
        degrees += 180.0;
    }
    degrees
}

/// The convex hull of `points`, counter-clockwise (Andrew's monotone
/// chain).
fn convex_hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let turn = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<[f64; 2]> = Vec::with_capacity(points.len() + 1);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 { Box::new(points.iter()) } else { Box::new(points.iter().rev()) };
        for &p in iter {
            while hull.len() >= start + 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

type Matrix = [[f64; 3]; 3];

fn rotate(mesh: &mut TriMesh, r: &Matrix) {
    for p in &mut mesh.positions {
        let q = *p;
        *p = [dot(r[0], q), dot(r[1], q), dot(r[2], q)];
    }
}

fn about_z(angle: f64) -> Matrix {
    let (s, c) = angle.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// The rotation that takes the unit vector `from` to `to` (Rodrigues).
fn rotation_between(from: [f64; 3], to: [f64; 3]) -> Matrix {
    let v = cross(from, to);
    let c = dot(from, to);
    if c > 1.0 - 1e-12 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let k = 1.0 / (1.0 + c);
    [
        [c + v[0] * v[0] * k, v[0] * v[1] * k - v[2], v[0] * v[2] * k + v[1]],
        [v[1] * v[0] * k + v[2], c + v[1] * v[1] * k, v[1] * v[2] * k - v[0]],
        [v[2] * v[0] * k - v[1], v[2] * v[1] * k + v[0], c + v[2] * v[2] * k],
    ]
}

fn bounds(mesh: &TriMesh) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for t in &mesh.triangles {
        for &i in t {
            let p = mesh.positions[i as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    (lo, hi)
}

fn triangle_area(mesh: &TriMesh, t: &[u32; 3]) -> f64 {
    let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
    let n = cross(sub(b, a), sub(c, a));
    dot(n, n).sqrt() / 2.0
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalized(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(v, v).sqrt();
    (length > 1e-12).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed box `size` with its corner at `at`, counter-clockwise from
    /// outside.
    fn cuboid(mesh: &mut TriMesh, at: [f64; 3], size: [f64; 3]) {
        let base = mesh.positions.len() as u32;
        for i in 0..8 {
            let pick = |k: usize| at[k] + if i >> k & 1 == 1 { size[k] } else { 0.0 };
            mesh.positions.push([pick(0), pick(1), pick(2)]);
        }
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        for [a, b, c, d] in quads {
            mesh.triangles.push([base + a, base + b, base + c]);
            mesh.triangles.push([base + a, base + c, base + d]);
        }
    }

    /// A 60 × 40 × 25 mm case with a tall post off-centre (so that it has
    /// no symmetry to hide a wrong turn behind).
    fn case() -> TriMesh {
        let mut mesh = TriMesh { positions: Vec::new(), triangles: Vec::new() };
        cuboid(&mut mesh, [0.0, 0.0, 0.0], [60.0, 40.0, 25.0]);
        // Glued on top: its bottom corners are the case's top face's.
        cuboid(&mut mesh, [5.0, 5.0, 25.0], [10.0, 10.0, 15.0]);
        // One part: the post shares no corner, so tie them with a sliver.
        mesh.triangles.push([4, 8 + 0, 5]);
        mesh
    }

    fn turned(mesh: &TriMesh, r: &Matrix, shift: [f64; 3]) -> TriMesh {
        let mut m = mesh.clone();
        rotate(&mut m, r);
        for p in &mut m.positions {
            for k in 0..3 {
                p[k] += shift[k];
            }
        }
        m
    }

    fn about_x(angle: f64) -> Matrix {
        let (s, c) = angle.sin_cos();
        [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
    }

    fn times(a: &Matrix, b: &Matrix) -> Matrix {
        let mut m = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        m
    }

    #[test]
    fn a_case_turned_and_tipped_comes_back_straight() {
        let case = case();
        let r = times(&about_z(17f64.to_radians()), &about_x(4f64.to_radians()));
        let leveled = level(&turned(&case, &r, [123.0, -45.0, 67.0]));
        assert!(leveled.base);
        assert!((leveled.tilt - 4.0).abs() < 0.1, "tilt {}", leveled.tilt);
        assert!((leveled.turn.abs() - 17.0).abs() < 0.5, "turn {}", leveled.turn);
        // Straight within 1°: off by 1°, the 60 mm side would widen the
        // footprint by 0.7 mm.
        for (k, want) in [60.0, 40.0, 40.0].into_iter().enumerate() {
            assert!((leveled.size[k] - want).abs() < 0.1, "size {:?}", leveled.size);
        }
        // Standing on z = 0, centred: its bottom face is flat at 0.
        let bottom: Vec<f64> = leveled.mesh.positions.iter().map(|p| p[2]).filter(|z| *z < 1.0).collect();
        assert_eq!(bottom.len(), 4);
        assert!(bottom.iter().all(|z| z.abs() < 1e-6));
        let (lo, hi) = bounds(&leveled.mesh);
        assert!((lo[0] + hi[0]).abs() < 1e-6 && (lo[1] + hi[1]).abs() < 1e-6);
    }

    #[test]
    fn a_part_on_its_side_is_not_tipped_over() {
        // Lying on its 60 × 25 side: down is right as the phone saw it.
        let lying = turned(&case(), &about_x(90f64.to_radians()), [0.0; 3]);
        let leveled = level(&lying);
        assert!(leveled.tilt < 0.1, "tilt {}", leveled.tilt);
        assert!((leveled.size[2] - 40.0).abs() < 0.1, "size {:?}", leveled.size);
    }

    #[test]
    fn a_part_tipped_too_far_keeps_its_down() {
        // Tipped 60°: no face within 25° of down, so none is stood on.
        let tipped = turned(&case(), &about_x(60f64.to_radians()), [0.0; 3]);
        let leveled = level(&tipped);
        assert!(!leveled.base);
        assert_eq!(leveled.tilt, 0.0);
    }

    #[test]
    fn a_sloping_flank_up_high_is_not_a_base() {
        // An open shell, as a scan with no bottom: four walls of a 60 × 40
        // case, and a wide roof sloping 20° whose underside faces down.
        let mut mesh = TriMesh { positions: Vec::new(), triangles: Vec::new() };
        let (s, c) = 20f64.to_radians().sin_cos();
        let p = [[0.0, 0.0, 10.0], [60.0, 0.0, 10.0 + 60.0 * s / c], [60.0, 40.0, 10.0 + 60.0 * s / c], [0.0, 40.0, 10.0]];
        for q in p {
            mesh.positions.push(q);
        }
        // The roof, its underside facing down and out.
        mesh.triangles.push([0, 2, 1]);
        mesh.triangles.push([0, 3, 2]);
        // The walls down to z = 0, thin and open below.
        for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            let (a, b) = (p[i], p[j]);
            let base = mesh.positions.len() as u32;
            mesh.positions.extend([[a[0], a[1], 0.0], [b[0], b[1], 0.0]]);
            mesh.triangles.push([i as u32, base, base + 1]);
            mesh.triangles.push([i as u32, base + 1, j as u32]);
        }
        let leveled = level(&mesh);
        assert!(!leveled.base, "tipped {}", leveled.tilt);
        assert_eq!(leveled.tilt, 0.0);
    }

    #[test]
    fn floating_bits_go() {
        let mut mesh = case();
        for (i, at) in [[100.0, 0.0, 0.0], [0.0, 100.0, 5.0], [-50.0, -50.0, 30.0]].into_iter().enumerate() {
            cuboid(&mut mesh, at, [1.0 + i as f64, 1.0, 1.0]);
        }
        let leveled = level(&mesh);
        assert_eq!(leveled.islands, 3);
        assert!((leveled.size[0] - 60.0).abs() < 0.1 && (leveled.size[1] - 40.0).abs() < 0.1, "size {:?}", leveled.size);
    }

    #[test]
    fn a_ball_keeps_its_down() {
        // No flat face: an octahedron-like blob of small facets.
        let mut mesh = TriMesh { positions: Vec::new(), triangles: Vec::new() };
        let (rings, segments) = (24, 48);
        mesh.positions.push([0.0, 0.0, -20.0]);
        for i in 1..rings {
            let a = std::f64::consts::PI * f64::from(i) / f64::from(rings) - std::f64::consts::FRAC_PI_2;
            for j in 0..segments {
                let b = std::f64::consts::TAU * f64::from(j) / f64::from(segments);
                mesh.positions.push([20.0 * a.cos() * b.cos(), 20.0 * a.cos() * b.sin(), 20.0 * a.sin()]);
            }
        }
        mesh.positions.push([0.0, 0.0, 20.0]);
        let ring = |i: u32, j: u32| 1 + (i - 1) * segments + j % segments;
        let top = mesh.positions.len() as u32 - 1;
        for j in 0..segments {
            mesh.triangles.push([0, ring(1, j + 1), ring(1, j)]);
            mesh.triangles.push([top, ring(rings - 1, j), ring(rings - 1, j + 1)]);
            for i in 1..rings - 1 {
                mesh.triangles.push([ring(i, j), ring(i, j + 1), ring(i + 1, j + 1)]);
                mesh.triangles.push([ring(i, j), ring(i + 1, j + 1), ring(i + 1, j)]);
            }
        }
        let leveled = level(&mesh);
        assert!(!leveled.base);
        assert_eq!(leveled.tilt, 0.0);
    }

    #[test]
    fn a_file_is_leveled_in_place() {
        let dir = std::env::temp_dir().join(format!("c2f-level-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("scan.stl");
        let r = about_z(30f64.to_radians());
        std::fs::write(&file, stl(&turned(&case(), &r, [10.0, 10.0, 10.0]))).unwrap();
        let leveled = level_file(&file, &file).unwrap();
        assert!((leveled.size[0] - 60.0).abs() < 0.1);
        // Read again, it is already straight.
        let again = level_file(&file, &dir.join("again.stl")).unwrap();
        assert!(again.turn.abs() < 0.01 && again.tilt < 0.01, "{} {}", again.turn, again.tilt);
        assert!(level_file(&dir.join("none.stl"), &dir.join("x.stl")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
