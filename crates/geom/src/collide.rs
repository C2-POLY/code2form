//! Collisions between solids placed in one space, as the parts of a
//! circuit and its enclosure are: the volume each pair shares.

use manifold_rust::linalg::{Mat3x4, Vec3};
use manifold_rust::manifold::Manifold;

use crate::build::Model;

/// A 4×4 matrix by rows, its last row `[0, 0, 0, 1]`.
pub type Placement = [[f64; 4]; 4];

/// Less shared volume than this, in cubic millimeters, is two surfaces
/// touching, not a collision.
pub const TOUCHING: f64 = 0.01;

/// Every pair of `solids`, each placed by its matrix, that shares more
/// than [`TOUCHING`] of volume: their indices, in order, and that volume.
/// What is not a solid collides with nothing.
pub fn collisions(solids: &[(&Model, Placement)]) -> Vec<(usize, usize, f64)> {
    let placed: Vec<Option<Manifold>> = solids.iter().map(|(model, m)| place(model, m)).collect();
    let boxes: Vec<_> = placed.iter().map(|m| m.as_ref().map(Manifold::bounding_box)).collect();
    let mut found = Vec::new();
    for i in 0..placed.len() {
        for j in i + 1..placed.len() {
            let (Some(a), Some(b), Some(box_a), Some(box_b)) = (&placed[i], &placed[j], &boxes[i], &boxes[j]) else {
                continue;
            };
            // Boxes apart (or only touching): nothing to intersect.
            let apart = box_a.max.x <= box_b.min.x
                || box_b.max.x <= box_a.min.x
                || box_a.max.y <= box_b.min.y
                || box_b.max.y <= box_a.min.y
                || box_a.max.z <= box_b.min.z
                || box_b.max.z <= box_a.min.z;
            if apart {
                continue;
            }
            let volume = a.intersection(b).volume();
            if volume > TOUCHING {
                found.push((i, j, volume));
            }
        }
    }
    found
}

/// The solid of `model` placed by `m`; `None` when it is not a solid.
pub(crate) fn place(model: &Model, m: &Placement) -> Option<Manifold> {
    match model {
        Model::Solid(solid) if !solid.is_empty() => {
            let col = |j: usize| Vec3::new(m[0][j], m[1][j], m[2][j]);
            Some(solid.transform(&Mat3x4::from_cols(col(0), col(1), col(2), col(3))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(size: f64) -> Model {
        let program = c2f_eval::Program::load(&format!("cube({size});"), "t.scad".as_ref(), &[]).unwrap();
        crate::render(&c2f_eval::evaluate(&program).nodes).model
    }

    fn at(x: f64, y: f64, z: f64) -> Placement {
        [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z], [0.0, 0.0, 0.0, 1.0]]
    }

    #[test]
    fn overlaps_count_and_touching_does_not() {
        let (a, b) = (cube(10.0), cube(2.0));
        // b half inside a's corner: 1 × 2 × 2 shared.
        let found = collisions(&[(&a, at(0.0, 0.0, 0.0)), (&b, at(9.0, 0.0, 0.0)), (&b, at(10.0, 5.0, 0.0))]);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].0, found[0].1), (0, 1));
        assert!((found[0].2 - 4.0).abs() < 1e-6, "{}", found[0].2);
        // Face to face, and apart.
        assert!(collisions(&[(&a, at(0.0, 0.0, 0.0)), (&b, at(0.0, 0.0, 10.0)), (&b, at(30.0, 0.0, 0.0))]).is_empty());
        // Turned a quarter around z, into the cube.
        let turned = [[0.0, -1.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let found = collisions(&[(&a, at(0.0, 0.0, 0.0)), (&b, turned)]);
        assert!((found[0].2 - 4.0).abs() < 1e-6, "{found:?}");
        assert!(collisions(&[(&Model::Empty, at(0.0, 0.0, 0.0)), (&a, at(0.0, 0.0, 0.0))]).is_empty());
    }
}
