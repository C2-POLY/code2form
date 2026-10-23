//! The `part()`s of a model: each one's geometry in its own frame, and
//! where it sits in its parent part's frame, for exports that keep parts
//! apart (glTF). Everything outside any part is the root.

use c2f_eval::node::{IDENTITY, Joint, Matrix, Node, NodeKind};

/// A `part("name") { … }` of the model.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Part {
    pub name: String,
    /// The part it is written in, by index; `None` for the root.
    pub parent: Option<usize>,
    /// From its own frame to its parent's: the transformations written
    /// between the two.
    pub matrix: Matrix,
    /// How it moves against its parent, as `part()` says.
    pub joint: Option<Joint>,
    /// Its geometry in its own frame, without the parts inside it, in the
    /// color it is drawn in.
    pub nodes: Vec<Node>,
}

/// The model split into its parts: the geometry outside any part, and the
/// parts in the order they are met, each one after its parent.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Parts {
    pub root: Vec<Node>,
    pub parts: Vec<Part>,
    pub warnings: Vec<String>,
}

/// Splits the top-level nodes of a model into its parts.
pub(crate) fn split(nodes: &[Node]) -> Parts {
    let mut parts = Parts::default();
    let at = Place { matrix: IDENTITY, color: None, parent: None };
    parts.root = nodes.iter().filter_map(|node| parts.strip(node, &at)).collect();
    parts
}

/// Where a node is met: the transformations since the enclosing part (or
/// the root), the color it is drawn in, and that part.
#[derive(Clone, Copy)]
struct Place {
    matrix: Matrix,
    /// The outermost `color()` above it, which wins in the reference.
    color: Option<[f64; 4]>,
    parent: Option<usize>,
}

impl Parts {
    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    /// `node` without the parts in it, which go to `self.parts`; `None`
    /// for a part itself.
    fn strip(&mut self, node: &Node, at: &Place) -> Option<Node> {
        // Shown in the preview only, as are the parts in it.
        if node.background {
            return Some(node.clone());
        }
        let inner = match &node.kind {
            NodeKind::Part { name, joint } => {
                let index = self.parts.len();
                self.parts.push(Part {
                    name: name.clone(),
                    parent: at.parent,
                    matrix: at.matrix,
                    joint: joint.clone(),
                    nodes: Vec::new(),
                });
                let inside = Place { matrix: IDENTITY, color: at.color, parent: Some(index) };
                let mut nodes: Vec<Node> = node.children.iter().filter_map(|child| self.strip(child, &inside)).collect();
                if let Some(color) = at.color {
                    nodes = vec![Node::new(NodeKind::Color(color), nodes)];
                }
                self.parts[index].nodes = nodes;
                return None;
            }
            NodeKind::Transform(m) => Place { matrix: multiply(&at.matrix, m), ..*at },
            // A color whose red is negative is `color()` without a color.
            NodeKind::Color(rgba) => Place { color: at.color.or((rgba[0] >= 0.0).then_some(*rgba)), ..*at },
            NodeKind::Group | NodeKind::Union | NodeKind::Render { .. } => *at,
            kind => {
                // A part under any other operation is shaped by its
                // siblings: it stays in the geometry it is in.
                if let Some(name) = first_part(node) {
                    let operation = kind.header();
                    let operation = operation.split('(').next().unwrap_or_default();
                    self.warn(format!(
                        "part(\"{name}\") is inside {operation}(), which it changes or is changed by: it is merged \
                         into the geometry around it"
                    ));
                }
                return Some(node.clone());
            }
        };
        let children: Vec<Node> = node.children.iter().filter_map(|child| self.strip(child, &inner)).collect();
        // What only held parts holds nothing now: its place would change
        // with theirs.
        if children.is_empty() && !node.children.is_empty() {
            return None;
        }
        Some(Node { kind: node.kind.clone(), highlight: node.highlight, background: false, children })
    }
}

/// The name of the first part in `node`, if any.
fn first_part(node: &Node) -> Option<&str> {
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        if let NodeKind::Part { name, .. } = &node.kind {
            return Some(name);
        }
        pending.extend(node.children.iter().rev());
    }
    None
}

pub(crate) fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_src(src: &str) -> Parts {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        split(&c2f_eval::evaluate(&program).nodes)
    }

    fn translation(m: &Matrix) -> [f64; 3] {
        [m[0][3], m[1][3], m[2][3]]
    }

    #[test]
    fn parts_leave_the_root_with_their_place() {
        let parts = split_src(
            "cube(40);
            translate([20, 20, 40]) rotate([0, 0, 90]) part(\"turret\") cylinder(r = 10, h = 5);",
        );
        assert_eq!(parts.parts.len(), 1);
        let turret = &parts.parts[0];
        assert_eq!((turret.name.as_str(), turret.parent), ("turret", None));
        assert_eq!(translation(&turret.matrix), [20.0, 20.0, 40.0]);
        // A quarter turn about Z: x goes to y.
        assert!((turret.matrix[1][0] - 1.0).abs() < 1e-12 && turret.matrix[0][0].abs() < 1e-12);
        // The cylinder stays in the part's own frame, the cube at the root.
        assert!(matches!(turret.nodes[0].kind, NodeKind::Cylinder { .. }));
        assert!(c2f_eval::node::csg(&parts.root).contains("cube"));
        assert!(!c2f_eval::node::csg(&parts.root).contains("cylinder"));
        assert!(parts.warnings.is_empty());
    }

    #[test]
    fn nested_parts_follow_their_parent() {
        let parts = split_src(
            "part(\"base\") {
                cube(10);
                translate([0, 0, 10]) part(\"arm\") {
                    cube([2, 2, 20]);
                    translate([0, 0, 20]) part(\"hand\") sphere(2);
                }
            }
            translate([50, 0, 0]) part(\"other\") cube(1);",
        );
        let names: Vec<(&str, Option<usize>)> = parts.parts.iter().map(|p| (p.name.as_str(), p.parent)).collect();
        assert_eq!(names, [("base", None), ("arm", Some(0)), ("hand", Some(1)), ("other", None)]);
        // Each relative to its parent.
        assert_eq!(translation(&parts.parts[1].matrix), [0.0, 0.0, 10.0]);
        assert_eq!(translation(&parts.parts[2].matrix), [0.0, 0.0, 20.0]);
        assert_eq!(translation(&parts.parts[3].matrix), [50.0, 0.0, 0.0]);
        // The arm's geometry holds its cube, not the hand's sphere.
        let arm = c2f_eval::node::csg(&parts.parts[1].nodes);
        assert!(arm.contains("cube(size = [2, 2, 20]") && !arm.contains("sphere"), "{arm}");
        // What only held parts is gone from the root.
        assert!(parts.root.is_empty(), "{}", c2f_eval::node::csg(&parts.root));
    }

    #[test]
    fn a_part_keeps_the_color_it_is_drawn_in() {
        let parts = split_src("color(\"red\") color(\"blue\") part(\"lid\") cube(1);");
        assert_eq!(parts.parts[0].nodes[0].kind, NodeKind::Color([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_part_in_a_boolean_is_merged_with_a_warning() {
        let parts = split_src("difference() { cube(10); part(\"hole\") cylinder(r = 2, h = 30); }");
        assert!(parts.parts.is_empty());
        assert_eq!(parts.warnings.len(), 1);
        assert!(parts.warnings[0].contains("part(\"hole\") is inside difference()"), "{:?}", parts.warnings);
    }

    #[test]
    fn background_parts_stay_in_the_preview() {
        let parts = split_src("%part(\"ghost\") cube(1); cube(2);");
        assert!(parts.parts.is_empty() && parts.warnings.is_empty());
    }
}
