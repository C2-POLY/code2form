//! How the parts of a circuit fit in their enclosure: the room left
//! between each part and the enclosure or the other parts, the ports a
//! wall covers, the mount holes nothing holds up, and the wires that run
//! through the enclosure's material.
//!
//! A gap is the shortest distance between the two surfaces as rendered,
//! exact to the floating point: Manifold's `min_gap` compares the triangles
//! of one with the triangles of the other within reach, found through its
//! bounding volume hierarchy. A curved face counts as its facets, so a
//! round part's clearance is that of its polygon (`$fn`). Where two
//! surfaces come closest is located to within the size of the smaller of
//! the two triangles that come closest. What a part rests on is measured
//! apart from the rest of the enclosure, so that a board on its standoffs
//! is not too tight. Ports, mount holes and wires are probed with rays: a
//! grid across a port's opening, a ring around a hole, and a wire's own
//! curve.

use std::cell::OnceCell;

use c2f_circuit::{Circuit, Instance, route};
use c2f_lang::annotations::PortSize;
use manifold_rust::collider::distance_triangle_triangle_squared;
use manifold_rust::linalg::Vec3;
use manifold_rust::manifold::Manifold;

use crate::build::Model;
use crate::collide::{Placement, TOUCHING, place};

/// The clearance a part needs around it, unless told otherwise, in
/// millimeters.
pub const CLEARANCE: f64 = 0.5;

/// How far gaps are measured, in millimeters: solids further apart than
/// this have room enough, and their gap is not given.
pub const SEARCH: f64 = 10.0;

/// The reaches `min_gap` is asked for in turn: each costs more than the
/// one before, as it compares more triangles, and most parts sit within
/// the first.
const REACHES: [f64; 3] = [1.0, 3.0, SEARCH];

/// How far in front of a port a wall covers it, in millimeters.
const PORT_REACH: f64 = 5.0;

/// How far along a mount hole's axis the enclosure must hold it up.
const HOLE_REACH: f64 = 1.0;

/// How far past the face a part rests on what it puts through its support
/// counts as being through it, in millimeters: more than the gap a part
/// leaves above what it rests on.
const REST: f64 = 1.0;

/// How far behind a port or a mount hole its rays start: inside the part,
/// so that a wall the part touches is met.
const BEHIND: f64 = 0.5;

const IDENTITY: Placement = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// How close two solids come.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// The shortest distance between their surfaces, in millimeters: 0
    /// when they touch or overlap.
    pub distance: f64,
    /// The volume they share, in cubic millimeters: more than
    /// [`TOUCHING`] when they collide, 0 otherwise.
    pub overlap: f64,
    /// About where they come closest, or the middle of what they share.
    pub at: [f64; 3],
}

impl Gap {
    /// Whether the two solids collide, or come closer than `clearance`.
    pub fn tight(&self, clearance: f64) -> bool {
        self.overlap > TOUCHING || self.distance < clearance
    }
}

/// How one part of a circuit fits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartFit {
    /// To the enclosure around the part: `None` without one, or when it is
    /// further than [`SEARCH`]. What the part rests on is not counted,
    /// unless the part runs into it: a board touches its standoffs.
    pub enclosure: Option<Gap>,
    /// To what the part rests on: the enclosure beyond the plane of the
    /// face it rests on, the side its mount holes face (its bottom when it
    /// has none).
    pub rests: Option<Gap>,
    /// To each other part closer than [`SEARCH`], by index, nearest first.
    pub parts: Vec<(usize, Gap)>,
    /// The ports the enclosure covers, by index, and where a ray from the
    /// opening first meets it.
    pub covered_ports: Vec<(usize, [f64; 3])>,
    /// The mount holes the enclosure does not hold up, by index.
    pub loose_holes: Vec<usize>,
}

impl PartFit {
    /// The least room around the part, `None` when nothing is within
    /// [`SEARCH`].
    pub fn clearance(&self) -> Option<f64> {
        self.enclosure.iter().chain(self.parts.iter().map(|(_, gap)| gap)).map(|gap| gap.distance).min_by(f64::total_cmp)
    }

    /// Whether the part collides, or comes closer than `clearance` to the
    /// enclosure or another part.
    pub fn tight(&self, clearance: f64) -> bool {
        self.enclosure.iter().chain(self.parts.iter().map(|(_, gap)| gap)).any(|gap| gap.tight(clearance))
    }

    /// What is wrong with part `index`, a sentence each: too little room
    /// (to another part only from the first of the two, so that a pair is
    /// told once), a port covered, a mount hole held by nothing.
    /// `collisions` false leaves out the solids that overlap, for a caller
    /// that tells of them already.
    pub fn warnings(&self, circuit: &Circuit, index: usize, clearance: f64, collisions: bool) -> Vec<String> {
        let name = |i: usize| format!("`{}`", circuit.parts[i].name);
        let mut warnings = Vec::new();
        let others = self.parts.iter().filter(|(other, _)| *other > index).map(|(other, gap)| (Some(*other), gap));
        for (other, gap) in self.enclosure.iter().map(|gap| (None, gap)).chain(others) {
            if !gap.tight(clearance) || (!collisions && gap.overlap > TOUCHING) {
                continue;
            }
            let at = point(gap.at);
            warnings.push(match other {
                None if gap.overlap > TOUCHING => {
                    format!("{} runs into the enclosure by {:.2} mm³ around {at}", name(index), gap.overlap)
                }
                Some(other) if gap.overlap > TOUCHING => {
                    format!("{} and {} overlap by {:.2} mm³ around {at}", name(index), name(other), gap.overlap)
                }
                None if gap.distance == 0.0 => format!("{} touches the enclosure at {at}", name(index)),
                Some(other) if gap.distance == 0.0 => format!("{} touches {} at {at}", name(index), name(other)),
                None => format!(
                    "{} is {:.2} mm from the enclosure at {at}, under the {clearance} mm clearance",
                    name(index),
                    gap.distance
                ),
                Some(other) => format!(
                    "{} is {:.2} mm from {} at {at}, under the {clearance} mm clearance",
                    name(index),
                    gap.distance,
                    name(other)
                ),
            });
        }
        let part = &circuit.parts[index];
        for &(port, at) in &self.covered_ports {
            let port = &part.annotations.ports[port].name;
            warnings.push(format!("no cutout_for in front of port `{}.{port}`: the enclosure covers it at {}", part.name, point(at)));
        }
        for &hole in &self.loose_holes {
            let at = point(part.place(part.annotations.mount_holes[hole].position));
            warnings.push(format!(
                "no standoff under mount hole {} of {} (at {at}): no enclosure within {HOLE_REACH} mm along its axis",
                hole + 1,
                name(index)
            ));
        }
        warnings
    }
}

/// Where a wire first runs into the enclosure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    /// The wire, by index.
    pub wire: usize,
    /// The first point of its curve in the enclosure's material: where it
    /// meets its surface, or where it starts when it starts inside.
    pub at: [f64; 3],
    /// The way the face its curve first meets faces: which wall.
    pub facing: [f64; 3],
}

impl Crossing {
    /// As `wire 3 (xiao.D0 -> driver.IN1) runs into the enclosure at [...]`.
    pub fn message(&self, circuit: &Circuit) -> String {
        let wire = &circuit.wires[self.wire];
        let n = self.wire + 1;
        format!(
            "wire {n} ({} -> {}) runs into the enclosure at {}, through a face facing {}: route it around with `via`, \
             or carve it a channel_for({n})",
            circuit.end_name(wire.from),
            circuit.end_name(wire.to),
            point(self.at),
            facing(self.facing)
        )
    }
}

/// How each part of `circuit` fits, in the order of its parts: `parts`
/// are their solids, each placed by its matrix, and `enclosure` is in the
/// circuit's coordinates. What is not a solid fits anywhere.
pub fn fit(circuit: &Circuit, parts: &[(&Model, Placement)], enclosure: Option<&Model>) -> Vec<PartFit> {
    let solids: Vec<Option<Solid>> = parts.iter().map(|(model, m)| place(model, m).map(Solid::new)).collect();
    let enclosure = enclosure.and_then(|model| place(model, &IDENTITY)).map(Solid::new);
    let mut fits = vec![PartFit::default(); solids.len()];
    for (i, instance) in circuit.parts.iter().enumerate().take(solids.len()) {
        let Some(a) = &solids[i] else { continue };
        if let Some(enclosure) = &enclosure {
            (fits[i].enclosure, fits[i].rests) = to_enclosure(instance, a, enclosure);
            fits[i].covered_ports = covered_ports(instance, &enclosure.manifold);
            fits[i].loose_holes = loose_holes(instance, &enclosure.manifold);
        }
        for (j, b) in solids.iter().enumerate().skip(i + 1) {
            if let Some(gap) = b.as_ref().and_then(|b| gap(a, b)) {
                fits[i].parts.push((j, gap));
                fits[j].parts.push((i, gap));
            }
        }
    }
    for fit in &mut fits {
        fit.parts.sort_by(|a, b| a.1.distance.total_cmp(&b.1.distance));
    }
    fits
}

/// The wires of `circuit` whose curve runs into `enclosure`, in the
/// circuit's coordinates, each where it first does. A wire's curve is its
/// center line: one that grazes a wall by less than its radius passes.
pub fn crossings(circuit: &Circuit, enclosure: &Model) -> Vec<Crossing> {
    let Some(enclosure) = place(enclosure, &IDENTITY) else { return Vec::new() };
    route::paths(circuit)
        .iter()
        .enumerate()
        .filter_map(|(wire, path)| {
            let (pair, (at, facing)) = path.windows(2).find_map(|pair| Some((pair, first_hit(&enclosure, pair[0], pair[1])?)))?;
            // A face the curve leaves by: it was in the material already
            // where this stretch starts, as from a pin on the surface.
            let leaving = (0..3).map(|k| (pair[1][k] - pair[0][k]) * facing[k]).sum::<f64>() > 0.0;
            Some(Crossing { wire, at: if leaving { pair[0] } else { at }, facing })
        })
        .collect()
}

type Triangle = [[f64; 3]; 3];
type Bounds = ([f64; 3], [f64; 3]);

/// A placed solid, its bounds, and its triangles once a gap is located.
struct Solid {
    manifold: Manifold,
    bounds: Bounds,
    triangles: OnceCell<Vec<Triangle>>,
}

impl Solid {
    fn new(manifold: Manifold) -> Solid {
        Solid { bounds: bounds_of(&manifold), manifold, triangles: OnceCell::new() }
    }

    fn triangles(&self) -> &[Triangle] {
        self.triangles.get_or_init(|| {
            let mesh = self.manifold.get_mesh_gl64(-1);
            let stride = (mesh.num_prop as usize).max(3);
            let vertex = |i: u64| {
                let p = &mesh.vert_properties[i as usize * stride..i as usize * stride + 3];
                [p[0], p[1], p[2]]
            };
            mesh.tri_verts.chunks_exact(3).map(|t| [vertex(t[0]), vertex(t[1]), vertex(t[2])]).collect()
        })
    }
}

/// How close `a` and `b` come, `None` when further than [`SEARCH`].
fn gap(a: &Solid, b: &Solid) -> Option<Gap> {
    overlap(a, b).or_else(|| distance(a, b))
}

/// What `a` and `b` share, when they collide.
fn overlap(a: &Solid, b: &Solid) -> Option<Gap> {
    let (box_a, box_b) = (a.bounds, b.bounds);
    if (0..3).any(|k| box_a.1[k] <= box_b.0[k] || box_b.1[k] <= box_a.0[k]) {
        return None;
    }
    let shared = a.manifold.intersection(&b.manifold);
    let overlap = shared.volume();
    if overlap <= TOUCHING {
        return None;
    }
    let (low, high) = bounds_of(&shared);
    Some(Gap { distance: 0.0, overlap, at: std::array::from_fn(|k| (low[k] + high[k]) / 2.0) })
}

/// The shortest distance between the surfaces of `a` and `b`, and about
/// where, `None` when further than [`SEARCH`].
fn distance(a: &Solid, b: &Solid) -> Option<Gap> {
    if a.manifold.is_empty() || b.manifold.is_empty() || box_distance(a.bounds, b.bounds) > SEARCH {
        return None;
    }
    let distance = REACHES.iter().map(|&reach| (reach, a.manifold.min_gap(&b.manifold, reach))).find(|&(reach, d)| d < reach)?.1;
    let at = locate(a.triangles(), b.triangles(), distance).unwrap_or_else(|| between(a.bounds, b.bounds));
    Some(Gap { distance, overlap: 0.0, at })
}

/// The gap from `part` to `enclosure` around it, and to what it rests on
/// (see [`PartFit::rests`]); a collision when it runs into the enclosure.
fn to_enclosure(instance: &Instance, part: &Solid, enclosure: &Solid) -> (Option<Gap>, Option<Gap>) {
    if let Some(collision) = overlap(part, enclosure) {
        return (Some(collision), None);
    }
    let hole = instance.annotations.mount_holes.first();
    let d = instance.turn(hole.map_or([0.0, 0.0, -1.0], |hole| hole.direction));
    let along = |p: &[f64; 3]| p[0] * d[0] + p[1] * d[1] + p[2] * d[2];
    // The plane of the face the part rests on: its mount holes', or as far
    // as the part goes downwards.
    let plane = match hole {
        Some(hole) => along(&instance.place(hole.position)),
        None => part.triangles().iter().flatten().map(along).fold(f64::MIN, f64::max),
    };
    let normal = Vec3::new(d[0], d[1], d[2]);
    let (support, around) = enclosure.manifold.split_by_plane(normal, plane);
    let support = Solid::new(support);
    // What the part puts through its support, as a motor's shaft, has its
    // own room there: measured from [`REST`] past the plane, clear of the
    // gap the part may leave above its support.
    let through = Solid::new(part.manifold.trim_by_plane(normal, plane + REST));
    let room = [distance(part, &Solid::new(around)), distance(&through, &support)];
    (room.into_iter().flatten().min_by(|a, b| a.distance.total_cmp(&b.distance)), distance(part, &support))
}

/// The middle of the place where a triangle of `a` and one of `b` come
/// within `distance` of each other, the closest found: the middle of the
/// space between their two bounding boxes.
fn locate(a: &[Triangle], b: &[Triangle], distance: f64) -> Option<[f64; 3]> {
    // What rounding may add to the distance found.
    let reach = distance + 1e-9 * (1.0 + distance);
    let all = |triangles: &[Triangle]| triangles.iter().map(bounds).reduce(|p, q| (min(p.0, q.0), max(p.1, q.1)));
    let (all_a, all_b) = (all(a)?, all(b)?);
    fn near(triangles: &[Triangle], other: Bounds, reach: f64) -> Vec<(Bounds, &Triangle)> {
        triangles.iter().map(|t| (bounds(t), t)).filter(|(b, _)| box_distance(*b, other) <= reach).collect()
    }
    let near_a = near(a, all_b, reach);
    let mut near_b = near(b, all_a, reach);
    // Sorted along x, the triangles of `b` past a triangle of `a` end the
    // search for it.
    near_b.sort_by(|p, q| p.0.0[0].total_cmp(&q.0.0[0]));
    let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
    for (box_a, ta) in &near_a {
        for (box_b, tb) in &near_b {
            if box_b.0[0] > box_a.1[0] + reach {
                break;
            }
            if box_distance(*box_a, *box_b) <= reach
                && distance_triangle_triangle_squared(ta.map(v), tb.map(v)) <= reach * reach
            {
                return Some(between(*box_a, *box_b));
            }
        }
    }
    None
}

/// The ports of `part` that `enclosure` covers: a ray from the opening's
/// center or from near its edge, along the way it faces, meets it within
/// [`PORT_REACH`].
fn covered_ports(part: &Instance, enclosure: &Manifold) -> Vec<(usize, [f64; 3])> {
    let mut covered = Vec::new();
    for (index, port) in part.annotations.ports.iter().enumerate() {
        let (x, y) = across(port.direction);
        let (half_w, half_h, round) = match port.size {
            PortSize::Rect([w, h]) => (w / 2.0, h / 2.0, false),
            PortSize::Round(d) => (d / 2.0, d / 2.0, true),
        };
        // A grid over the opening, out to 80% of the way to its edge: a
        // cutout grown by its clearance leaves every ray free, and a grille
        // as much as it is open.
        let steps = [-0.8, -0.4, 0.0, 0.4, 0.8];
        let samples: Vec<(f64, f64)> = steps
            .iter()
            .flat_map(|&u| steps.iter().map(move |&v| (u, v)))
            .filter(|(u, v)| !round || u * u + v * v <= 0.81)
            .collect();
        let n = part.turn(port.direction);
        let hits: Vec<[f64; 3]> = samples
            .iter()
            .filter_map(|(u, v)| {
                let p = part.place(add(port.position, add(scale(x, u * half_w), scale(y, v * half_h))));
                first_hit(enclosure, add(p, scale(n, -BEHIND)), add(p, scale(n, PORT_REACH))).map(|(at, _)| at)
            })
            .collect();
        // Covered when most of the opening is.
        if hits.len() * 2 > samples.len() {
            covered.push((index, hits[0]));
        }
    }
    covered
}

/// The mount holes of `part` with nothing of `enclosure` along their axis
/// within [`HOLE_REACH`], probed on a ring around the hole, between a
/// standoff's bore and its outside.
fn loose_holes(part: &Instance, enclosure: &Manifold) -> Vec<usize> {
    let mut loose = Vec::new();
    for (index, hole) in part.annotations.mount_holes.iter().enumerate() {
        let (x, y) = across(hole.direction);
        let n = part.turn(hole.direction);
        let held = (0..8).any(|k| {
            let angle = k as f64 * std::f64::consts::TAU / 8.0;
            let ring = add(scale(x, angle.cos()), scale(y, angle.sin()));
            let p = part.place(add(hole.position, scale(ring, 0.75 * hole.diameter)));
            first_hit(enclosure, add(p, scale(n, -BEHIND)), add(p, scale(n, HOLE_REACH))).is_some()
        });
        if !held {
            loose.push(index);
        }
    }
    loose
}

/// Where the segment from `from` to `to` first meets the surface of
/// `solid`, and the way the face it meets faces.
fn first_hit(solid: &Manifold, from: [f64; 3], to: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
    let hit = solid.ray_cast(v(from), v(to)).into_iter().next()?;
    Some(([hit.position.x, hit.position.y, hit.position.z], [hit.normal.x, hit.normal.y, hit.normal.z]))
}

/// Two axes across the unit vector `n`, as `cutout_for()` lays a port's
/// width and height: x horizontal (the x axis when `n` is vertical), y the
/// third axis.
fn across(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let x = if n[0].abs() < 1e-9 && n[1].abs() < 1e-9 {
        [1.0, 0.0, 0.0]
    } else {
        let length = (n[0] * n[0] + n[1] * n[1]).sqrt();
        [-n[1] / length, n[0] / length, 0.0]
    };
    let y = [n[1] * x[2] - n[2] * x[1], n[2] * x[0] - n[0] * x[2], n[0] * x[1] - n[1] * x[0]];
    (x, y)
}

fn bounds_of(manifold: &Manifold) -> Bounds {
    let b = manifold.bounding_box();
    ([b.min.x, b.min.y, b.min.z], [b.max.x, b.max.y, b.max.z])
}

fn bounds(t: &Triangle) -> Bounds {
    (min(min(t[0], t[1]), t[2]), max(max(t[0], t[1]), t[2]))
}

/// How far apart two boxes are, 0 when they meet.
fn box_distance(a: Bounds, b: Bounds) -> f64 {
    (0..3).map(|k| (a.0[k] - b.1[k]).max(b.0[k] - a.1[k]).max(0.0).powi(2)).sum::<f64>().sqrt()
}

/// The middle of what two boxes share, or of the space between them.
fn between(a: Bounds, b: Bounds) -> [f64; 3] {
    std::array::from_fn(|k| (a.0[k].max(b.0[k]) + a.1[k].min(b.1[k])) / 2.0)
}

fn min(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k].min(b[k]))
}

fn max(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k].max(b[k]))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] + b[k])
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|c| c * s)
}

/// A point as `[x, y, z]`, to a tenth of a millimeter.
pub fn point(p: [f64; 3]) -> String {
    // -0.0 reads as 0.
    let n = |c: f64| format!("{:.1}", c + 0.0).replace("-0.0", "0.0");
    format!("[{}, {}, {}]", n(p[0]), n(p[1]), n(p[2]))
}

/// A face's direction as an axis (`-y`), or as a vector when it is none.
fn facing(n: [f64; 3]) -> String {
    let axis = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs())).unwrap_or(2);
    if n[axis].abs() > 0.99 {
        format!("{}{}", if n[axis] > 0.0 { "+" } else { "-" }, ["x", "y", "z"][axis])
    } else {
        format!("[{:.2}, {:.2}, {:.2}]", n[0], n[1], n[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(src: &str) -> Solid {
        let program = c2f_eval::Program::load(src, "t.scad".as_ref(), &[]).unwrap();
        let model = crate::render(&c2f_eval::evaluate(&program).nodes).model;
        Solid::new(place(&model, &IDENTITY).unwrap())
    }

    fn distance_between(a: &Solid, b: &Solid) -> f64 {
        gap(a, b).unwrap().distance
    }

    #[test]
    fn gaps_are_exact_and_located() {
        let a = solid("cube(10);");
        // 0.3 mm beside it, across the face x = 10.
        let gap_beside = gap(&a, &solid("translate([10.3, 2, 2]) cube(2);")).unwrap();
        assert!((gap_beside.distance - 0.3).abs() < 1e-12 && gap_beside.overlap == 0.0, "{gap_beside:?}");
        assert!((gap_beside.at[0] - 10.15).abs() < 1e-9, "{gap_beside:?}");
        // Corner to edge, and further, within reach.
        assert!((distance_between(&a, &solid("translate([11, 11, 0]) cube(2);")) - 2f64.sqrt()).abs() < 1e-12);
        assert!((distance_between(&a, &solid("translate([19, 0, 0]) cube(2);")) - 9.0).abs() < 1e-12);
        // Touching, then overlapping by 1 x 2 x 2, around the middle of it.
        assert_eq!(distance_between(&a, &solid("translate([10, 0, 0]) cube(2);")), 0.0);
        let shared = gap(&a, &solid("translate([9, 0, 0]) cube(2);")).unwrap();
        assert!((shared.overlap - 4.0).abs() < 1e-9 && shared.at == [9.5, 1.0, 1.0], "{shared:?}");
        // Out of reach.
        assert!(gap(&a, &solid("translate([25, 0, 0]) cube(2);")).is_none());
    }

    #[test]
    fn points_and_faces_read_plainly() {
        assert_eq!(point([-0.04, 12.345, 3.0]), "[0.0, 12.3, 3.0]");
        assert_eq!(facing([0.0, -1.0, 0.0]), "-y");
        assert_eq!(facing([0.0, 0.866, -0.5]), "[0.00, 0.87, -0.50]");
    }
}
