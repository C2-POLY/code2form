//! The exploded view of a model: its `part()`s drawn apart, so that one
//! sees how they fit together, as they come apart. Each part slides along
//! its own Z axis, the axis it is built and turns about, the way that takes
//! it away from the center of what it is written in (the whole model, or
//! its parent part), by half the size of that. A part in a part moves with
//! its parent, then away from it, [`CHILD_FACTOR`] times further at each
//! level, so that each level comes apart from the one above. What is
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

/// A model's parts, drawn each on its own, and how far each moves when the
/// view is all the way exploded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exploded {
    /// Every part where the model puts it, the root first.
    pub mesh: RenderMesh,
    /// Each part's vertices in `mesh`, and its move all the way exploded.
    pieces: Vec<(Range<usize>, [f32; 3])>,
}

impl Exploded {
    /// The parts moved `amount` of the way, from 0 (assembled) to 1.
    pub fn at(&self, amount: f32) -> RenderMesh {
        let mut mesh = self.mesh.clone();
        for (range, offset) in &self.pieces {
            for p in &mut mesh.positions[range.clone()] {
                *p = [p[0] + amount * offset[0], p[1] + amount * offset[1], p[2] + amount * offset[2]];
            }
        }
        mesh
    }

    /// How far each part moves all the way exploded, in the order the
    /// model writes them.
    pub fn offsets(&self) -> Vec<[f32; 3]> {
        self.pieces.iter().skip(1).map(|(_, offset)| *offset).collect()
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
    let mut exploded = Exploded::default();
    let mut boxes: Vec<Option<([f32; 3], [f32; 3])>> = Vec::new();
    let identity = c2f_eval::node::IDENTITY;
    let pieces = std::iter::once((&split.root, &identity)).chain(split.parts.iter().map(|p| &p.nodes).zip(&world));
    for (nodes, matrix) in pieces {
        let solid = render_in(nodes, base_dir).solid;
        let start = exploded.mesh.positions.len();
        let placed = placed(&solid, matrix);
        boxes.push(placed.bounds());
        exploded.mesh.append(placed);
        exploded.pieces.push((start..exploded.mesh.positions.len(), [0.0; 3]));
    }
    // The box around each part with the parts in it, deepest first.
    let mut around = boxes.clone();
    for (i, part) in split.parts.iter().enumerate().rev() {
        let parent = part.parent.map_or(0, |p| p + 1);
        around[parent] = union(around[parent], around[i + 1]);
    }
    // The root's box is the whole model's.
    let whole = around[1..].iter().fold(around[0], |a, &b| union(a, b));
    let mut depth: Vec<i32> = Vec::with_capacity(split.parts.len());
    for (i, part) in split.parts.iter().enumerate() {
        depth.push(part.parent.map_or(0, |p| depth[p] + 1));
        let from = match part.parent {
            Some(p) => around[p + 1],
            None => whole,
        };
        let axis = [world[i][0][2], world[i][1][2], world[i][2][2]].map(|v| v as f32);
        let own = exploded_move(around[i + 1], from, axis).map(|v| v * CHILD_FACTOR.powi(depth[i]));
        let carried = part.parent.map_or([0.0; 3], |p| exploded.pieces[p + 1].1);
        exploded.pieces[i + 1].1 = [carried[0] + own[0], carried[1] + own[1], carried[2] + own[2]];
    }
    Some(exploded)
}

/// How far a part in box `part` moves away from `from`: half the size of
/// `from`, along `axis` (the part's Z), the way that takes its center away
/// from the center of `from`, or along +`axis` when neither way does. A
/// part that fills `from` is all there is: it stays.
fn exploded_move(part: Option<([f32; 3], [f32; 3])>, from: Option<([f32; 3], [f32; 3])>, axis: [f32; 3]) -> [f32; 3] {
    let (Some((lo, hi)), Some((flo, fhi))) = (part, from) else { return [0.0; 3] };
    let size = ((fhi[0] - flo[0]).powi(2) + (fhi[1] - flo[1]).powi(2) + (fhi[2] - flo[2]).powi(2)).sqrt();
    let same = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() <= 1e-4 * size);
    if same(lo, flo) && same(hi, fhi) {
        return [0.0; 3];
    }
    let center = |lo: [f32; 3], hi: [f32; 3]| [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let (c, f) = (center(lo, hi), center(flo, fhi));
    let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    // A part scaled flat along its Z has no axis to slide on: up, then.
    let axis = if length > 1e-6 { axis.map(|v| v / length) } else { [0.0, 0.0, 1.0] };
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
        assert_eq!(exploded.at(0.0), exploded.mesh);
        // The root's cube, then the part, turned a quarter and moved.
        let side = &exploded.mesh.positions[36..];
        let lo = side.iter().fold([f32::MAX; 3], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].min(p[2])]);
        assert!(close(lo, [18.0, 0.0, 0.0]), "{lo:?}");
    }

    #[test]
    fn parts_move_away_from_the_center_by_half_its_size() {
        // A box 10 wide, a lid on it, a knob on the lid.
        let exploded = exploded_src(
            "cube([10, 10, 8]);
            translate([0, 0, 8]) part(\"lid\") {
                cube([10, 10, 2]);
                translate([5, 5, 2]) part(\"knob\") cylinder(r = 1, h = 2, $fn = 4);
            }",
        )
        .unwrap();
        let [lid, knob] = [exploded.offsets()[0], exploded.offsets()[1]];
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
        let half = exploded.at(0.5);
        assert!(close(half.positions[0], exploded.mesh.positions[0]));
    }

    #[test]
    fn a_part_centered_on_its_parent_moves_up() {
        let exploded = exploded_src("cube(4, center = true); part(\"a\") cube(2, center = true);").unwrap();
        let offset = exploded.offsets()[0];
        assert!(offset[0] == 0.0 && offset[1] == 0.0 && offset[2] > 0.0, "{offset:?}");
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
        let roller = exploded.offsets()[0];
        assert!(roller[0] > 0.0 && roller[1].abs() < 1e-4 && roller[2].abs() < 1e-4, "{roller:?}");
        // Behind the center, it slides the other way.
        let exploded = exploded_src(
            "cube([20, 20, 2], center = true);
            translate([-6, 8, 0]) rotate([0, 90, 0]) part(\"roller\") cylinder(r = 2, h = 4, center = true, $fn = 4);",
        )
        .unwrap();
        assert!(exploded.offsets()[0][0] < 0.0, "{:?}", exploded.offsets());
    }

    #[test]
    fn a_part_that_is_the_whole_model_stays() {
        // The hips carry the rest of a figure: they stay, the leg moves.
        let exploded = exploded_src("part(\"hips\") { cube(4); translate([0, 0, -6]) part(\"leg\") cube([1, 1, 6]); }").unwrap();
        let [hips, leg] = [exploded.offsets()[0], exploded.offsets()[1]];
        assert_eq!(hips, [0.0; 3]);
        assert!(leg[2] < 0.0, "{leg:?}");
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
