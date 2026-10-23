//! The CSG tree produced by evaluation, and its text form as the reference
//! implementation (release 2021.01) writes it with `-o out.csg`.

use c2f_lang::dump::string;

use crate::value::number;

pub type Matrix = [[f64; 4]; 4];

pub const IDENTITY: Matrix = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    /// `#`: shown highlighted in previews.
    pub highlight: bool,
    /// `%`: shown transparent in previews and left out of renders.
    pub background: bool,
    pub children: Vec<Node>,
}

/// `$fn`, `$fa`, `$fs` as seen by a primitive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segments {
    pub fn_: f64,
    pub fa: f64,
    pub fs: f64,
}

/// A 2D import from a file, still accepted by the extrusions.
#[derive(Clone, Debug, PartialEq)]
pub struct FileSource {
    pub file: String,
    pub layer: String,
    pub origin: [f64; 2],
    pub scale: f64,
    pub timestamp: i64,
}

/// How a `part()` moves against the part it is written in, for a robot
/// (URDF): `part("elbow", joint = "revolute", axis = [0, 1, 0],
/// limits = [-90, 90])`.
#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub kind: JointKind,
    /// In the part's own frame, of length 1.
    pub axis: [f64; 3],
    /// Lowest and highest position: degrees for a revolute joint,
    /// millimeters for a prismatic one.
    pub limits: Option<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    /// Turns about its axis, between its limits.
    Revolute,
    /// Turns about its axis without limits, as a wheel does.
    Continuous,
    /// Slides along its axis.
    Prismatic,
    Fixed,
}

impl JointKind {
    /// The kind a `joint = "…"` names, as URDF names it.
    pub fn named(name: &str) -> Option<JointKind> {
        match name {
            "revolute" => Some(JointKind::Revolute),
            "continuous" => Some(JointKind::Continuous),
            "prismatic" => Some(JointKind::Prismatic),
            "fixed" => Some(JointKind::Fixed),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            JointKind::Revolute => "revolute",
            JointKind::Continuous => "continuous",
            JointKind::Prismatic => "prismatic",
            JointKind::Fixed => "fixed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Group,
    Union,
    Difference,
    Intersection,
    Hull,
    Minkowski { convexity: u32 },
    Render { convexity: i64 },
    Projection { cut: bool, convexity: i32 },

    Cube { size: [f64; 3], center: bool },
    Sphere { r: f64, segments: Segments },
    Cylinder { h: f64, r1: f64, r2: f64, center: bool, segments: Segments },
    Polyhedron { points: Points, faces: Points, convexity: u32 },
    Square { size: [f64; 2], center: bool },
    Circle { r: f64, segments: Segments },
    Polygon { points: Points, paths: Points, convexity: u32 },
    Text(TextParams),
    Surface { file: String, center: bool, invert: bool, timestamp: i64 },
    /// `object` (not in the reference) keeps one object, by its name, of
    /// a 3MF or OBJ file; empty for all.
    Import {
        file: String,
        layer: String,
        origin: [f64; 2],
        scale: f64,
        convexity: u32,
        segments: Segments,
        timestamp: i64,
        object: String,
    },

    Transform(Matrix),
    Color([f64; 4]),
    /// Not in the reference: a part of the model kept apart from the rest,
    /// which a glTF export writes as a node of its own to move and animate.
    /// Other exports unite it with the rest, as a group. A URDF export
    /// makes it a link, which its joint moves.
    Part { name: String, joint: Option<Joint> },
    Offset { op: OffsetOp, segments: Segments },
    Resize { size: [f64; 3], auto: [bool; 3], convexity: u32 },
    LinearExtrude {
        source: Option<FileSource>,
        height: f64,
        center: bool,
        convexity: u32,
        twist: f64,
        slices: Option<u32>,
        scale: [f64; 2],
        segments: Segments,
    },
    RotateExtrude { source: Option<FileSource>, angle: f64, convexity: u32, segments: Segments },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OffsetOp {
    Round { r: f64 },
    Delta { delta: f64, chamfer: bool },
}

/// Points, faces or paths: kept as given (the reference prints them
/// verbatim) together with their numbers when they are well formed.
#[derive(Clone, Debug, PartialEq)]
pub struct Points {
    pub repr: String,
    pub rows: Option<Vec<Vec<f64>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextParams {
    pub text: String,
    pub size: f64,
    pub spacing: f64,
    pub font: String,
    pub direction: String,
    pub language: String,
    pub script: Option<String>,
    pub halign: String,
    pub valign: String,
    pub segments: Segments,
}

impl Node {
    pub fn new(kind: NodeKind, children: Vec<Node>) -> Self {
        Node { kind, highlight: false, background: false, children }
    }

    pub fn group(children: Vec<Node>) -> Self {
        Node::new(NodeKind::Group, children)
    }
}

impl Segments {
    fn csg(&self) -> String {
        format!("$fn = {}, $fa = {}, $fs = {}", c_g(self.fn_), c_g(self.fa), c_g(self.fs))
    }
}

impl FileSource {
    fn csg(&self) -> String {
        format!(
            "file = {}, layer = {}, origin = [{}, {}], ",
            string(&self.file),
            string(&self.layer),
            c_g(self.origin[0]),
            c_g(self.origin[1])
        )
    }
}

/// `text()` writes its strings quoted but unescaped, unlike every other
/// node.
fn string_raw(s: &str) -> String {
    format!("\"{s}\"")
}

/// Node parameters are written with C++ streams: `%g`, unlike values.
fn vector(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|&v| c_g(v)).collect();
    format!("[{}]", parts.join(", "))
}

/// Matrices are written like values (`echo` formatting).
fn value_vector(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|&v| number(v)).collect();
    format!("[{}]", parts.join(", "))
}

/// Eigen's stream format, which `resize` uses: `[1,2,3]` with C `%g`.
fn eigen_vector(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|&v| c_g(v)).collect();
    format!("[{}]", parts.join(","))
}

/// C's `%g` with six significant digits: exponent notation below 1e-4 or
/// from 1e6 up, at least two exponent digits, ties to even, `-0` kept.
fn c_g(v: f64) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !v.is_finite() {
        return number(v);
    }
    let sci = format!("{:.5e}", v.abs());
    let (mantissa, exponent) = sci.split_once('e').expect("{:e} has an exponent");
    let exponent: i32 = exponent.parse().expect("integer exponent");
    let sign = if v < 0.0 { "-" } else { "" };
    if !(-4..6).contains(&exponent) {
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{mantissa}e{exp_sign}{:02}", exponent.abs());
    }
    let decimals = (5 - exponent).max(0) as usize;
    let fixed = format!("{:.*}", decimals, v.abs());
    let fixed = if fixed.contains('.') { fixed.trim_end_matches('0').trim_end_matches('.') } else { &fixed };
    format!("{sign}{fixed}")
}

impl NodeKind {
    /// `name(arguments)` as in the CSG file.
    pub fn header(&self) -> String {
        match self {
            // The reference has no parts: it reads a group back.
            NodeKind::Group | NodeKind::Part { .. } => "group()".into(),
            NodeKind::Union => "union()".into(),
            NodeKind::Difference => "difference()".into(),
            NodeKind::Intersection => "intersection()".into(),
            NodeKind::Hull => "hull()".into(),
            NodeKind::Minkowski { convexity } => format!("minkowski(convexity = {convexity})"),
            NodeKind::Render { convexity } => format!("render(convexity = {convexity})"),
            NodeKind::Projection { cut, convexity } => format!("projection(cut = {cut}, convexity = {convexity})"),
            NodeKind::Cube { size, center } => format!("cube(size = {}, center = {center})", vector(size)),
            NodeKind::Sphere { r, segments } => format!("sphere({}, r = {})", segments.csg(), c_g(*r)),
            NodeKind::Cylinder { h, r1, r2, center, segments } => format!(
                "cylinder({}, h = {}, r1 = {}, r2 = {}, center = {center})",
                segments.csg(),
                c_g(*h),
                c_g(*r1),
                c_g(*r2)
            ),
            NodeKind::Polyhedron { points, faces, convexity } => {
                format!("polyhedron(points = {}, faces = {}, convexity = {convexity})", points.repr, faces.repr)
            }
            NodeKind::Square { size, center } => format!("square(size = {}, center = {center})", vector(size)),
            NodeKind::Circle { r, segments } => format!("circle({}, r = {})", segments.csg(), c_g(*r)),
            NodeKind::Polygon { points, paths, convexity } => {
                format!("polygon(points = {}, paths = {}, convexity = {convexity})", points.repr, paths.repr)
            }
            NodeKind::Text(t) => {
                let script = t.script.as_ref().map_or(String::new(), |s| format!("script = {}, ", string_raw(s)));
                format!(
                    "text(text = {}, size = {}, spacing = {}, font = {}, direction = {}, language = {}, {script}halign = {}, valign = {}, {})",
                    string_raw(&t.text),
                    c_g(t.size),
                    c_g(t.spacing),
                    string_raw(&t.font),
                    string_raw(&t.direction),
                    string_raw(&t.language),
                    string_raw(&t.halign),
                    string_raw(&t.valign),
                    t.segments.csg()
                )
            }
            NodeKind::Surface { file, center, invert, timestamp } => format!(
                "surface(file = {}, center = {center}, invert = {invert}, timestamp = {timestamp})",
                string(file)
            ),
            NodeKind::Import { file, layer, origin, scale, convexity, segments, timestamp, object } => format!(
                "import(file = {}, layer = {}, origin = {}, scale = {}, convexity = {convexity}, {}, {}timestamp = {timestamp})",
                string(file),
                string(layer),
                vector(origin),
                c_g(*scale),
                segments.csg(),
                if object.is_empty() { String::new() } else { format!("object = {}, ", string(object)) }
            ),
            NodeKind::Transform(m) => {
                let rows: Vec<String> = m.iter().map(|row| value_vector(row)).collect();
                format!("multmatrix([{}])", rows.join(", "))
            }
            NodeKind::Color(c) => format!("color({})", vector(c)),
            NodeKind::Offset { op: OffsetOp::Round { r }, segments } => {
                format!("offset(r = {}, {})", c_g(*r), segments.csg())
            }
            NodeKind::Offset { op: OffsetOp::Delta { delta, chamfer }, segments } => {
                format!("offset(delta = {}, chamfer = {chamfer}, {})", c_g(*delta), segments.csg())
            }
            NodeKind::Resize { size, auto, convexity } => {
                let auto: Vec<f64> = auto.iter().map(|&a| if a { 1.0 } else { 0.0 }).collect();
                format!("resize(newsize = {}, auto = {}, convexity = {convexity})", eigen_vector(size), eigen_vector(&auto))
            }
            NodeKind::LinearExtrude { source, height, center, convexity, twist, slices, scale, segments } => {
                let mut out = String::from("linear_extrude(");
                if let Some(source) = source {
                    out.push_str(&source.csg());
                    out.push_str(&format!("timestamp = {}, ", source.timestamp));
                }
                out.push_str(&format!("height = {}, center = {center}, convexity = {convexity}, ", c_g(*height)));
                if *twist != 0.0 {
                    out.push_str(&format!("twist = {}, ", c_g(*twist)));
                }
                if let Some(slices) = slices {
                    out.push_str(&format!("slices = {slices}, "));
                }
                out.push_str(&format!("scale = {}", vector(scale)));
                if slices.is_none() {
                    out.push_str(&format!(", {}", segments.csg()));
                }
                out.push(')');
                out
            }
            NodeKind::RotateExtrude { source, angle, convexity, segments } => {
                let mut out = String::from("rotate_extrude(");
                if let Some(source) = source {
                    out.push_str(&source.csg());
                    out.push_str(&format!("scale = {}, timestamp = {}, ", c_g(source.scale), source.timestamp));
                }
                out.push_str(&format!("angle = {}, convexity = {convexity}, {})", c_g(*angle), segments.csg()));
                out
            }
        }
    }
}

/// The CSG file for a list of top-level nodes. Iterative, so any depth
/// prints on any thread's stack.
pub fn csg(nodes: &[Node]) -> String {
    enum Step<'a> {
        Open(&'a Node, usize),
        Close(usize),
    }
    let mut out = String::new();
    let mut stack: Vec<Step> = nodes.iter().rev().map(|n| Step::Open(n, 0)).collect();
    while let Some(step) = stack.pop() {
        match step {
            Step::Open(node, depth) => {
                // The reference writes modifiers before the indentation.
                if node.background {
                    out.push('%');
                }
                if node.highlight {
                    out.push('#');
                }
                out.push_str(&"\t".repeat(depth));
                out.push_str(&node.kind.header());
                if node.children.is_empty() {
                    out.push_str(";\n");
                } else {
                    out.push_str(" {\n");
                    stack.push(Step::Close(depth));
                    stack.extend(node.children.iter().rev().map(|child| Step::Open(child, depth + 1)));
                }
            }
            Step::Close(depth) => {
                out.push_str(&"\t".repeat(depth));
                out.push_str("}\n");
            }
        }
    }
    out
}

/// Frees deep trees without recursing.
impl Drop for Node {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(mut node) = pending.pop() {
            pending.append(&mut node.children);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_style_g() {
        assert_eq!(c_g(6.328125), "6.32812");
        assert_eq!(c_g(-0.0), "-0");
        assert_eq!(c_g(0.00001), "1e-05");
        assert_eq!(c_g(1e7), "1e+07");
        assert_eq!(c_g(1e-6), "1e-06");
        assert_eq!(c_g(2.25), "2.25");
        assert_eq!(c_g(1e100), "1e+100");
    }

    #[test]
    fn deep_trees_print_and_drop_without_recursion() {
        // Deep enough to overflow a 64 KiB stack if either step recursed.
        const DEPTH: usize = 3000;
        let mut tree = Node::group(Vec::new());
        for _ in 0..DEPTH {
            tree = Node::group(vec![tree]);
        }
        let len = std::thread::Builder::new()
            .stack_size(64 << 10)
            .spawn(move || {
                let text = csg(std::slice::from_ref(&tree));
                drop(tree);
                text.len()
            })
            .unwrap()
            .join()
            .unwrap();
        // Open and close lines per level, plus the innermost `group();`.
        assert_eq!(len, 12 * DEPTH + DEPTH * (DEPTH - 1) + 9 + DEPTH);
    }

    #[test]
    fn nested_output() {
        let tree = Node::new(
            NodeKind::Transform(IDENTITY),
            vec![Node::new(NodeKind::Cube { size: [1.0, 1.0, 1.0], center: false }, vec![])],
        );
        assert_eq!(
            csg(&[tree]),
            "multmatrix([[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]) {\n\tcube(size = [1, 1, 1], center = false);\n}\n"
        );
    }
}
