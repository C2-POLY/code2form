//! The exploded view of a model: its `part()`s drawn apart, so that one
//! sees how they fit together, in two complementary ways ([`Explode`]):
//!
//! - **Radial**: each part carries on along the line from the center of
//!   gravity of what it is written in (the whole model, or its parent part)
//!   to its own, so that a part far from the center goes further. A part
//!   at the center still moves a quarter of the size of that, along its
//!   own Z axis when it has no line to follow.
//! - **Axis**: each part slides along its own Z axis, the axis it is built
//!   and turns about, the way that takes it away from the center of what
//!   it is written in, by half the size of that.
//!
//! Either way, a part in a part moves with its parent, then away from it,
//! [`CHILD_FACTOR`] times further at each level, so that each level comes
//! apart from the one above; a part that fills its parent stays. What is
//! outside any part stays in place.

use std::ops::Range;
use std::path::Path;

use c2f_eval::node::{Matrix, Node};

use crate::build::render_in;
use crate::mesh::RenderMesh;
use crate::parts::{self, multiply};

/// How much further a part moves from its parent than its parent from its
/// own, at each level down.
const CHILD_FACTOR: f32 = 1.5;

/// How the parts come apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Explode {
    /// Along the line from the center of gravity of their parent.
    #[default]
    Radial,
    /// Along their own Z axis.
    Axis,
}

/// A model's parts, drawn each on its own, and how far each moves when the
/// view is all the way exploded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exploded {
    /// Every part where the model puts it, the root first.
    pub mesh: RenderMesh,
    /// Each part's vertices in `mesh`, and its move all the way exploded,
    /// radially and along its axis.
    pieces: Vec<(Range<usize>, [[f32; 3]; 2])>,
    /// Each part's name, as `part("name")` gives it, the root excluded.
    names: Vec<String>,
}

impl Exploded {
    /// The parts moved `amount` of the way, from 0 (assembled) to 1, as
    /// `how` says.
    pub fn at(&self, amount: f32, how: Explode) -> RenderMesh {
        let mut mesh = self.mesh.clone();
        for (range, moves) in &self.pieces {
            let offset = moves[how as usize];
            for p in &mut mesh.positions[range.clone()] {
                *p = [p[0] + amount * offset[0], p[1] + amount * offset[1], p[2] + amount * offset[2]];
            }
        }
        mesh
    }

    /// How far each part moves all the way exploded, as `how` says, in the
    /// order the model writes them.
    pub fn offsets(&self, how: Explode) -> Vec<[f32; 3]> {
        self.pieces.iter().skip(1).map(|(_, moves)| moves[how as usize]).collect()
    }

    /// The name of the part the vertex at `index` of `mesh` belongs to;
    /// `None` for the root, outside any part.
    pub fn part_at(&self, index: usize) -> Option<&str> {
        let piece = self.pieces.iter().position(|(range, _)| range.contains(&index))?;
        self.names.get(piece.checked_sub(1)?).map(String::as_str)
    }
}

type Box3 = Option<([f32; 3], [f32; 3])>;

/// A volume and its first moment (the volume times its center of gravity).
#[derive(Clone, Copy, Default)]
struct Mass {
    volume: f64,
    moment: [f64; 3],
}

impl Mass {
    fn of(mesh: &RenderMesh) -> Mass {
        let mut mass = Mass::default();
        for t in mesh.positions.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|p| p.map(f64::from));
            // The tetrahedron from the origin to the triangle.
            let v = (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
            mass.volume += v;
            for k in 0..3 {
                mass.moment[k] += v * (a[k] + b[k] + c[k]) / 4.0;
            }
        }
        mass
    }

    fn plus(self, other: Mass) -> Mass {
        Mass { volume: self.volume + other.volume, moment: [0, 1, 2].map(|k| self.moment[k] + other.moment[k]) }
    }

    /// The center of gravity; the middle of `fallback` for what has no
    /// volume (a flat or empty part).
    fn center(&self, fallback: Box3) -> Option<[f32; 3]> {
        if self.volume.abs() > 1e-9 {
            return Some(self.moment.map(|m| (m / self.volume) as f32));
        }
        fallback.map(|(lo, hi)| [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0))
    }
}

/// The model's parts drawn apart; `None` when it has no `part()`. Data
/// files are read relative to `base_dir`.
pub fn exploded(nodes: &[Node], base_dir: &Path) -> Option<Exploded> {
    let split = parts::split(nodes);
    if split.parts.is_empty() {
        return None;
    }
    // Where each part is in the model: its parents' matrices, then its own.
    let mut world: Vec<Matrix> = Vec::with_capacity(split.parts.len());
    for part in &split.parts {
        let parent = part.parent.map(|p| world[p]);
        world.push(parent.map_or(part.matrix, |m| multiply(&m, &part.matrix)));
    }
    let mut exploded = Exploded { names: split.parts.iter().map(|p| p.name.clone()).collect(), ..Exploded::default() };
    let mut boxes: Vec<Box3> = Vec::new();
    let mut masses: Vec<Mass> = Vec::new();
    let identity = c2f_eval::node::IDENTITY;
    let pieces = std::iter::once((&split.root, &identity)).chain(split.parts.iter().map(|p| &p.nodes).zip(&world));
    for (nodes, matrix) in pieces {
        let solid = render_in(nodes, base_dir).solid;
        let start = exploded.mesh.positions.len();
        let placed = placed(&solid, matrix);
        boxes.push(placed.bounds());
        masses.push(Mass::of(&placed));
        exploded.mesh.append(placed);
        exploded.pieces.push((start..exploded.mesh.positions.len(), [[0.0; 3]; 2]));
    }
    // The box and the mass of each part with the parts in it, deepest
    // first; the root's become the whole model's.
    let (mut around, mut weighs) = (boxes, masses);
    for (i, part) in split.parts.iter().enumerate().rev() {
        let parent = part.parent.map_or(0, |p| p + 1);
        around[parent] = union(around[parent], around[i + 1]);
        weighs[parent] = weighs[parent].plus(weighs[i + 1]);
    }
    let mut depth: Vec<i32> = Vec::with_capacity(split.parts.len());
    for (i, part) in split.parts.iter().enumerate() {
        depth.push(part.parent.map_or(0, |p| depth[p] + 1));
        let parent = part.parent.map_or(0, |p| p + 1);
        let (own, from) = ((around[i + 1], weighs[i + 1]), (around[parent], weighs[parent]));
        let axis = [world[i][0][2], world[i][1][2], world[i][2][2]].map(|v| v as f32);
        let level = CHILD_FACTOR.powi(depth[i]);
        let moves = [radial_move(own, from, axis), axis_move(own.0, from.0, axis)].map(|m| m.map(|v| v * level));
        let carried = exploded.pieces[parent].1;
        exploded.pieces[i + 1].1 =
            [0, 1].map(|how| [0, 1, 2].map(|k| if part.parent.is_some() { carried[how][k] } else { 0.0 } + moves[how][k]));
    }
    Some(exploded)
}

/// The length of the box's diagonal, and whether `part` fills `from`: then
/// it is all there is, and stays.
fn size_and_fills(part: Box3, from: Box3) -> Option<(f32, bool)> {
    let ((lo, hi), (flo, fhi)) = (part?, from?);
    let size = ((fhi[0] - flo[0]).powi(2) + (fhi[1] - flo[1]).powi(2) + (fhi[2] - flo[2]).powi(2)).sqrt();
    let same = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() <= 1e-4 * size);
    Some((size, same(lo, flo) && same(hi, fhi)))
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 1e-6).then(|| v.map(|c| c / length))
}

/// How far a part moves radially from `from` (its parent with its parts,
/// or the whole model): along the line between their centers of gravity,
/// as far again as it is, and at least a quarter of the size of `from`.
fn radial_move(part: (Box3, Mass), from: (Box3, Mass), axis: [f32; 3]) -> [f32; 3] {
    let Some((size, fills)) = size_and_fills(part.0, from.0) else { return [0.0; 3] };
    let (Some(c), Some(f)) = (part.1.center(part.0), from.1.center(from.0)) else { return [0.0; 3] };
    if fills {
        return [0.0; 3];
    }
    let line = [c[0] - f[0], c[1] - f[1], c[2] - f[2]];
    let distance = (line[0] * line[0] + line[1] * line[1] + line[2] * line[2]).sqrt();
    let direction = unit(line).filter(|_| distance > 1e-4 * size).or_else(|| unit(axis)).unwrap_or([0.0, 0.0, 1.0]);
    direction.map(|v| v * distance.max(size / 4.0))
}

/// How far a part in box `part` moves away from `from` along `axis` (its
/// Z): half the size of `from`, the way that takes its center away from
/// the center of `from`, or along +`axis` when neither way does.
fn axis_move(part: Box3, from: Box3, axis: [f32; 3]) -> [f32; 3] {
    let Some((size, fills)) = size_and_fills(part, from) else { return [0.0; 3] };
    let ((lo, hi), (flo, fhi)) = (part.expect("a box"), from.expect("a box"));
    if fills {
        return [0.0; 3];
    }
    let center = |lo: [f32; 3], hi: [f32; 3]| [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let (c, f) = (center(lo, hi), center(flo, fhi));
    // A part scaled flat along its Z has no axis to slide on: up, then.
    let axis = unit(axis).unwrap_or([0.0, 0.0, 1.0]);
    let along = (0..3).map(|k| (c[k] - f[k]) * axis[k]).sum::<f32>();
    let sign = if along < -1e-4 * size { -1.0 } else { 1.0 };
    axis.map(|v| sign * v * size / 2.0)
}

fn union(a: Option<([f32; 3], [f32; 3])>, b: Option<([f32; 3], [f32; 3])>) -> Option<([f32; 3], [f32; 3])> {
    match (a, b) {
        (Some((alo, ahi)), Some((blo, bhi))) => Some((
            [alo[0].min(blo[0]), alo[1].min(blo[1]), alo[2].min(blo[2])],
            [ahi[0].max(bhi[0]), ahi[1].max(bhi[1]), ahi[2].max(bhi[2])],
        )),
        (a, None) => a,
        (None, b) => b,
    }
}

/// `mesh` moved by `matrix`; a mirror turns its triangles over, so that
/// they still face out.
fn placed(mesh: &RenderMesh, matrix: &Matrix) -> RenderMesh {
    let m = matrix.map(|row| row.map(|v| v as f32));
    let apply = |p: [f32; 3]| {
        let row = |r: [f32; 4]| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3];
        [row(m[0]), row(m[1]), row(m[2])]
    };
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let mut out = RenderMesh::default();
    for (corners, colors) in mesh.positions.chunks(3).zip(mesh.colors.chunks(3)) {
        let [a, b, c] = [apply(corners[0]), apply(corners[1]), apply(corners[2])];
        if det < 0.0 {
            out.push_triangle([a, c, b], [colors[0], colors[2], colors[1]]);
        } else {
            out.push_triangle([a, b, c], [colors[0], colors[1], colors[2]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exploded_src(src: &str) -> Option<Exploded> {
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
        exploded(&c2f_eval::evaluate(&program).nodes, Path::new(""))
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4)
    }

    #[test]
    fn a_model_without_parts_has_no_exploded_view() {
        assert_eq!(exploded_src("cube(10); translate([20, 0, 0]) cube(5);"), None);
    }

    #[test]
    fn assembled_the_parts_are_where_the_model_puts_them() {
        let exploded = exploded_src("cube(10); translate([20, 0, 0]) rotate([0, 0, 90]) part(\"side\") cube([4, 2, 1]);").unwrap();
        assert_eq!(exploded.at(0.0, Explode::Radial), exploded.mesh);
        assert_eq!(exploded.at(0.0, Explode::Axis), exploded.mesh);
        // The root's cube, then the part, turned a quarter and moved.
        let side = &exploded.mesh.positions[36..];
        let lo = side.iter().fold([f32::MAX; 3], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].min(p[2])]);
        assert!(close(lo, [18.0, 0.0, 0.0]), "{lo:?}");
    }

    #[test]
    fn a_vertex_tells_its_part() {
        let exploded = exploded_src("cube(10); translate([20, 0, 0]) part(\"side\") cube([4, 2, 1]);").unwrap();
        assert_eq!(exploded.part_at(0), None, "the root");
        assert_eq!(exploded.part_at(36), Some("side"));
        assert_eq!(exploded.part_at(exploded.mesh.positions.len()), None);
    }

    #[test]
    fn along_their_axis_parts_move_by_half_the_size_of_their_parent() {
        // A box 10 wide, a lid on it, a knob on the lid.
        let exploded = exploded_src(
            "cube([10, 10, 8]);
            translate([0, 0, 8]) part(\"lid\") {
                cube([10, 10, 2]);
                translate([5, 5, 2]) part(\"knob\") cylinder(r = 1, h = 2, $fn = 4);
            }",
        )
        .unwrap();
        let [lid, knob] = [exploded.offsets(Explode::Axis)[0], exploded.offsets(Explode::Axis)[1]];
        // The lid and its knob are above the model's center: the lid goes up
        // by half the model's diagonal.
        let whole = (10f32.powi(2) * 2.0 + 12f32.powi(2)).sqrt();
        assert!(lid[2] > 0.0 && (lid.iter().map(|v| v * v).sum::<f32>().sqrt() - whole / 2.0).abs() < 1e-3, "{lid:?}");
        // The knob moves with the lid, then up from it by half the lid's
        // size with its knob, half as far again, a level down.
        let lid_size = (10f32.powi(2) * 2.0 + 4f32.powi(2)).sqrt();
        let own = [knob[0] - lid[0], knob[1] - lid[1], knob[2] - lid[2]];
        assert!(close(own, [0.0, 0.0, 1.5 * lid_size / 2.0]), "{own:?}");
        // Halfway, half as far; the root stays.
        let half = exploded.at(0.5, Explode::Axis);
        assert!(close(half.positions[0], exploded.mesh.positions[0]));
    }

    #[test]
    fn a_part_centered_on_its_parent_moves_up() {
        let exploded = exploded_src("cube(4, center = true); part(\"a\") cube(2, center = true);").unwrap();
        for how in [Explode::Radial, Explode::Axis] {
            let offset = exploded.offsets(how)[0];
            assert!(offset[0] == 0.0 && offset[1] == 0.0 && offset[2] > 0.0, "{how:?}: {offset:?}");
        }
        // Radially, a quarter of the model's size.
        let size = (3.0 * 16f32).sqrt();
        assert!((exploded.offsets(Explode::Radial)[0][2] - size / 4.0).abs() < 1e-4);
    }

    #[test]
    fn radially_a_part_carries_on_from_the_center_of_gravity() {
        // A plate at the origin, two parts on it, one twice as far out.
        let exploded = exploded_src(
            "cube([40, 40, 2], center = true);
            translate([6, 8, 0]) part(\"near\") cube(2, center = true);
            translate([12, 16, 0]) part(\"far\") cube(2, center = true);",
        )
        .unwrap();
        // The model's center of gravity, the plate and both cubes weighed.
        let volume = 40.0 * 40.0 * 2.0 + 8.0 + 8.0;
        let center = [8.0 * (6.0 + 12.0) / volume, 8.0 * (8.0 + 16.0) / volume, 0.0];
        let [near, far] = [exploded.offsets(Explode::Radial)[0], exploded.offsets(Explode::Radial)[1]];
        // Each goes as far again along the line from that center.
        let size = (40f32.powi(2) * 2.0 + 4.0).sqrt();
        let line = |p: [f32; 3]| [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
        let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        // The near cube is closer than a quarter of the model: it moves that far.
        let near_line = line([6.0, 8.0, 0.0]);
        assert!(close(near, near_line.map(|v| v / length(near_line) * size / 4.0)), "{near:?}");
        assert!(close(far, line([12.0, 16.0, 0.0])), "{far:?}");
    }

    #[test]
    fn radially_a_child_moves_further_from_its_parent() {
        let exploded = exploded_src(
            "cube([40, 40, 2], center = true);
            translate([15, 0, 0]) part(\"arm\") {
                cube([10, 2, 2], center = true);
                translate([4, 0, 0]) part(\"hand\") cube([2, 2, 2], center = true);
            }",
        )
        .unwrap();
        let [arm, hand] = [exploded.offsets(Explode::Radial)[0], exploded.offsets(Explode::Radial)[1]];
        // The hand goes out along +X with the arm, then on from the arm's
        // center of gravity (its cube and the hand weighed), 1.5 times as
        // far as it is, a level down.
        let own = [hand[0] - arm[0], hand[1] - arm[1], hand[2] - arm[2]];
        let arm_center = (40.0 * 15.0 + 8.0 * 19.0) / 48.0;
        assert!(close(own, [1.5 * (19.0 - arm_center), 0.0, 0.0]), "{own:?}");
    }

    #[test]
    fn a_part_slides_along_its_own_axis() {
        // Its Z turned to the model's X, its center off to the side in Y
        // and ahead in X: it slides along X, away from the center.
        let exploded = exploded_src(
            "cube([20, 20, 2], center = true);
            translate([6, 8, 0]) rotate([0, 90, 0]) part(\"roller\") cylinder(r = 2, h = 4, center = true, $fn = 4);",
        )
        .unwrap();
        let roller = exploded.offsets(Explode::Axis)[0];
        assert!(roller[0] > 0.0 && roller[1].abs() < 1e-4 && roller[2].abs() < 1e-4, "{roller:?}");
        // Behind the center, it slides the other way.
        let exploded = exploded_src(
            "cube([20, 20, 2], center = true);
            translate([-6, 8, 0]) rotate([0, 90, 0]) part(\"roller\") cylinder(r = 2, h = 4, center = true, $fn = 4);",
        )
        .unwrap();
        assert!(exploded.offsets(Explode::Axis)[0][0] < 0.0, "{:?}", exploded.offsets(Explode::Axis));
    }

    #[test]
    fn a_part_that_is_the_whole_model_stays() {
        // The hips carry the rest of a figure: they stay, the leg moves.
        let exploded = exploded_src("part(\"hips\") { cube(4); translate([0, 0, -6]) part(\"leg\") cube([1, 1, 6]); }").unwrap();
        for how in [Explode::Radial, Explode::Axis] {
            let [hips, leg] = [exploded.offsets(how)[0], exploded.offsets(how)[1]];
            assert_eq!(hips, [0.0; 3], "{how:?}");
            assert!(leg[2] < 0.0, "{how:?}: {leg:?}");
        }
    }

    #[test]
    fn a_mirrored_part_still_faces_out() {
        let exploded = exploded_src("mirror([1, 0, 0]) part(\"m\") cube(1);").unwrap();
        // Every normal points away from the cube's center.
        let center = [-0.5, 0.5, 0.5];
        for (p, n) in exploded.mesh.positions.iter().zip(&exploded.mesh.normals) {
            let out = (0..3).map(|k| (p[k] - center[k]) * n[k]).sum::<f32>();
            assert!(out > 0.0, "{p:?} {n:?}");
        }
    }
}
