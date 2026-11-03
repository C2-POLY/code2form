//! A model posed: each `part(joint = …)` turned about its axis or slid
//! along it, by an angle or a distance, and the parts in it carried along.
//! A joint sits at the origin of its part's frame and its axis is in that
//! frame, as in the URDF export: a part's place in the model becomes its
//! parent's, then the transformations written between them, then its
//! joint's motion.

use c2f_eval::node::{IDENTITY, Joint, JointKind, Matrix};

use crate::parts::multiply;

/// A joint that moves, and how far it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct JointControl {
    /// The part's name, as `part("name")` gives it.
    pub name: String,
    /// The part's index among the model's parts, in the order they are
    /// written; a pose gives one position per part.
    pub part: usize,
    pub kind: JointKind,
    /// Lowest and highest position: degrees for a turning joint,
    /// millimeters for a sliding one.
    pub range: [f64; 2],
}

/// How far a joint goes: its limits; a full turn for one that turns
/// without them; as far as the part is long, each way, for one that
/// slides without them. `None` for a fixed joint.
pub fn range(joint: &Joint, size: f64) -> Option<[f64; 2]> {
    match (joint.kind, joint.limits) {
        (JointKind::Fixed, _) => None,
        (JointKind::Continuous, _) => Some([-180.0, 180.0]),
        (JointKind::Revolute, limits) => Some(limits.unwrap_or([-180.0, 180.0])),
        (JointKind::Prismatic, limits) => Some(limits.unwrap_or([-size, size])),
    }
}

/// Where a joint rests: at 0, the pose the model is written in, or at
/// the nearest of its limits when 0 is outside them.
pub fn rest(joint: &Joint) -> f64 {
    joint.limits.map_or(0.0, |[lo, hi]| 0f64.clamp(lo, hi))
}

/// What a joint at `position` does in its part's frame: a turn about its
/// axis, in degrees, or a slide along it, in millimeters.
pub fn motion(joint: &Joint, position: f64) -> Matrix {
    let [x, y, z] = joint.axis;
    match joint.kind {
        JointKind::Fixed => IDENTITY,
        JointKind::Prismatic => {
            let mut m = IDENTITY;
            m[0][3] = x * position;
            m[1][3] = y * position;
            m[2][3] = z * position;
            m
        }
        JointKind::Revolute | JointKind::Continuous => {
            // Rodrigues' rotation about a unit axis.
            let (s, c) = position.to_radians().sin_cos();
            let t = 1.0 - c;
            [
                [t * x * x + c, t * x * y - s * z, t * x * z + s * y, 0.0],
                [t * x * y + s * z, t * y * y + c, t * y * z - s * x, 0.0],
                [t * x * z - s * y, t * y * z + s * x, t * z * z + c, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        }
    }
}

/// A piece of a posed model: the part it is written in (by index among
/// the pieces, the root first, with no parent), its place in that part
/// and its joint.
pub(crate) struct Link<'a> {
    pub parent: Option<usize>,
    pub matrix: Matrix,
    pub joint: Option<&'a Joint>,
}

/// Where each piece is in the model at `pose`, one position per piece
/// (missing ones are 0), each after its parent.
pub(crate) fn worlds(links: &[Link], pose: &[f64]) -> Vec<Matrix> {
    let mut worlds: Vec<Matrix> = Vec::with_capacity(links.len());
    for (i, link) in links.iter().enumerate() {
        let parent = link.parent.map_or(IDENTITY, |p| worlds[p]);
        let mut world = multiply(&parent, &link.matrix);
        let position = pose.get(i).copied().unwrap_or(0.0);
        if let Some(joint) = link.joint.filter(|_| position != 0.0) {
            world = multiply(&world, &motion(joint, position));
        }
        worlds.push(world);
    }
    worlds
}

/// What moves each piece from where the model puts it to where `pose`
/// does: its place posed, after its place undone.
pub(crate) fn moves(links: &[Link], pose: &[f64]) -> Vec<Matrix> {
    let rest = worlds(links, &[]);
    let posed = worlds(links, pose);
    rest.iter().zip(&posed).map(|(rest, posed)| multiply(posed, &invert(rest))).collect()
}

/// The inverse of an affine matrix; the identity for one that flattens
/// space, which no part can be placed by.
pub(crate) fn invert(m: &Matrix) -> Matrix {
    let a = |i: usize, j: usize| m[i][j];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-12 {
        return IDENTITY;
    }
    let mut out = IDENTITY;
    for (i, row) in out.iter_mut().enumerate().take(3) {
        for (j, cell) in row.iter_mut().enumerate().take(3) {
            // The cofactor of (j, i), over the determinant.
            let (r0, r1) = match j {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let (c0, c1) = match i {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let minor = a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0);
            let sign = if (i + j) % 2 == 0 { 1.0 } else { -1.0 };
            *cell = sign * minor / det;
        }
    }
    for row in out.iter_mut().take(3) {
        row[3] = -(0..3).map(|k| row[k] * m[k][3]).sum::<f64>();
    }
    out
}

/// `p` moved by `m`.
pub(crate) fn apply(m: &Matrix, p: [f64; 3]) -> [f64; 3] {
    let row = |r: [f64; 4]| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3];
    [row(m[0]), row(m[1]), row(m[2])]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9)
    }

    fn revolute(axis: [f64; 3]) -> Joint {
        Joint { kind: JointKind::Revolute, axis, limits: Some([-90.0, 90.0]) }
    }

    #[test]
    fn a_turn_goes_the_right_way_about_its_axis() {
        // A quarter turn about Z takes X to Y.
        assert!(close(apply(&motion(&revolute([0.0, 0.0, 1.0]), 90.0), [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        let slide = Joint { kind: JointKind::Prismatic, axis: [0.0, 1.0, 0.0], limits: None };
        assert!(close(apply(&motion(&slide, 5.0), [1.0, 0.0, 0.0]), [1.0, 5.0, 0.0]));
    }

    #[test]
    fn an_inverse_undoes_its_matrix() {
        let m = multiply(&motion(&revolute([0.6, 0.0, 0.8]), 37.0), &{
            let mut t = IDENTITY;
            t[0][3] = 4.0;
            t[1][1] = 2.0;
            t[2][3] = -3.0;
            t
        });
        let p = [1.5, -2.0, 7.0];
        assert!(close(apply(&invert(&m), apply(&m, p)), p));
    }

    #[test]
    fn a_child_turns_with_its_parent_about_its_parent_joint() {
        // An arm at (10, 0, 0) turning about Z, a hand 5 further along X.
        let joint = revolute([0.0, 0.0, 1.0]);
        let mut at_arm = IDENTITY;
        at_arm[0][3] = 10.0;
        let mut at_hand = IDENTITY;
        at_hand[0][3] = 5.0;
        let links = [
            Link { parent: None, matrix: IDENTITY, joint: None },
            Link { parent: Some(0), matrix: at_arm, joint: Some(&joint) },
            Link { parent: Some(1), matrix: at_hand, joint: None },
        ];
        let moves = moves(&links, &[0.0, 90.0, 0.0]);
        // The root stays; the hand's origin, at (15, 0, 0), turns about the
        // arm's origin to (10, 5, 0).
        assert!(close(apply(&moves[0], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]));
        assert!(close(apply(&moves[2], [15.0, 0.0, 0.0]), [10.0, 5.0, 0.0]));
        // At rest, nothing moves.
        assert!(close(apply(&super::moves(&links, &[])[2], [15.0, 0.0, 0.0]), [15.0, 0.0, 0.0]));
    }

    #[test]
    fn a_joint_rests_at_zero_or_its_nearest_limit() {
        assert_eq!(rest(&revolute([0.0, 0.0, 1.0])), 0.0);
        let up = Joint { kind: JointKind::Revolute, axis: [0.0, 0.0, 1.0], limits: Some([10.0, 90.0]) };
        assert_eq!(rest(&up), 10.0);
    }

    #[test]
    fn a_joint_without_limits_goes_a_full_turn_or_its_length() {
        let free = Joint { kind: JointKind::Revolute, axis: [0.0, 0.0, 1.0], limits: None };
        assert_eq!(range(&free, 10.0), Some([-180.0, 180.0]));
        let slide = Joint { kind: JointKind::Prismatic, axis: [1.0, 0.0, 0.0], limits: None };
        assert_eq!(range(&slide, 12.0), Some([-12.0, 12.0]));
        let fixed = Joint { kind: JointKind::Fixed, axis: [0.0, 0.0, 1.0], limits: None };
        assert_eq!(range(&fixed, 1.0), None);
    }
}
