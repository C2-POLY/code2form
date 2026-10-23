//! The parts of a model, each one built in its own frame and placed in
//! the model's: what the exports that keep parts apart without a scene of
//! their own write (OBJ, URDF, the parts' JSON).

use std::path::Path;

use c2f_eval::node::{IDENTITY, Joint, Matrix, Node};

use crate::build::{Model, render_in};
use crate::parts::{self, multiply};

/// The root (what is outside any `part()`) or a `part()`.
pub(crate) struct Piece {
    /// Unique among the pieces: `lid`, then `lid 2`.
    pub name: String,
    /// The piece it is written in; `None` for the root, the first piece.
    pub parent: Option<usize>,
    /// From its own frame to its parent's.
    pub matrix: Matrix,
    /// From its own frame to the model's.
    pub world: Matrix,
    /// How it moves against its parent, for a robot; `None` is fixed.
    pub joint: Option<Joint>,
    /// Its geometry in its own frame, without the pieces inside it.
    pub model: Model,
}

pub(crate) struct Pieces {
    /// The root first, then the parts, each after its parent.
    pub pieces: Vec<Piece>,
    pub warnings: Vec<String>,
}

/// Splits `nodes` into their parts and builds each one, its data files
/// read relative to `base_dir`. The root is named `root`.
pub(crate) fn pieces(nodes: &[Node], base_dir: &Path, root: &str) -> Pieces {
    let parts::Parts { root: root_nodes, parts, mut warnings } = parts::split(nodes);
    let names = crate::gltf::unique(std::iter::once(root).chain(parts.iter().map(|p| p.name.as_str())));
    let mut pieces: Vec<Piece> = Vec::with_capacity(parts.len() + 1);
    let geometry = std::iter::once((None, IDENTITY, None, &root_nodes))
        .chain(parts.iter().map(|p| (Some(p.parent.map_or(0, |i| i + 1)), p.matrix, p.joint.clone(), &p.nodes)));
    for (name, (parent, matrix, joint, nodes)) in names.into_iter().zip(geometry) {
        let rendered = render_in(nodes, base_dir);
        for warning in rendered.warnings {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        let world = match parent {
            Some(parent) => multiply(&pieces[parent].world, &matrix),
            None => IDENTITY,
        };
        pieces.push(Piece { name, parent, matrix, world, joint, model: rendered.model });
    }
    Pieces { pieces, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_piece_is_placed_in_the_model() {
        let program = c2f_eval::Program::load(
            "cube(1);
            translate([10, 0, 0]) part(\"arm\") { cube(2); translate([0, 5, 0]) part(\"arm\") cube(3); }",
            Path::new("t.scad"),
            &[],
        )
        .unwrap();
        let Pieces { pieces, warnings } = pieces(&c2f_eval::evaluate(&program).nodes, Path::new(""), "base");
        assert!(warnings.is_empty());
        let names: Vec<(&str, Option<usize>)> = pieces.iter().map(|p| (p.name.as_str(), p.parent)).collect();
        assert_eq!(names, [("base", None), ("arm", Some(0)), ("arm 2", Some(1))]);
        assert_eq!([pieces[2].world[0][3], pieces[2].world[1][3]], [10.0, 5.0]);
        let Model::Solid(hand) = &pieces[2].model else { panic!("a solid") };
        assert!((hand.volume() - 27.0).abs() < 1e-9);
    }
}
