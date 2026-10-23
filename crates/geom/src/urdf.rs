//! URDF export: a robot for ROS, RViz, Gazebo, MuJoCo, PyBullet or Isaac
//! Sim. The root (what is outside any `part()`) and each `part()` are
//! links; each part is joined to the part it is written in by the joint
//! its `part(joint = …)` gives, fixed without one. A joint sits at the
//! origin of its part's frame, so a part turns about the point its
//! `translate()` and `rotate()` put it at, as it does in an animation.
//!
//! Each link's geometry is a binary STL in its own frame, in meters, in a
//! folder beside the `.urdf`, named relative to it (PyBullet, MuJoCo and
//! urdf viewers read it as it is; ROS wants a `package://` path); its mass, center of mass and inertia come
//! from its volume and a density.

use std::fmt::Write;
use std::path::Path;

use c2f_eval::node::{JointKind, Matrix, Node};

use crate::build::{CUT_COLOR, DEFAULT_COLOR, Model};
use crate::export::colored_mesh;
use crate::pieces::pieces;

/// Meters per millimeter: a design counts in millimeters, URDF in meters.
const METERS: f64 = 0.001;

/// How a model is written to URDF.
#[derive(Clone, Debug, PartialEq)]
pub struct UrdfOptions {
    /// The robot's name, and its root link's.
    pub name: String,
    /// Where the meshes go, relative to the `.urdf`: `meshes/`.
    pub mesh_folder: String,
    /// In kilograms per cubic meter: PLA, 1240, by default.
    pub density: f64,
    /// The color of what has no `color()`, in sRGB.
    pub color: [f32; 3],
}

impl Default for UrdfOptions {
    fn default() -> Self {
        let [r, g, b, _] = DEFAULT_COLOR;
        UrdfOptions { name: "robot".into(), mesh_folder: "meshes".into(), density: 1240.0, color: [r, g, b] }
    }
}

/// A `.urdf` file and the meshes it names.
#[derive(Clone, Debug, Default)]
pub struct Urdf {
    pub urdf: String,
    /// Each mesh's path relative to the `.urdf`, and its bytes.
    pub meshes: Vec<(String, Vec<u8>)>,
    pub warnings: Vec<String>,
    pub links: usize,
    /// The joints that move, not the fixed ones.
    pub moving: usize,
}

/// Renders `nodes`, data files read relative to `base_dir`, and writes
/// them as a robot. An empty or 2D model has none.
pub fn urdf(nodes: &[Node], base_dir: &Path, options: &UrdfOptions) -> Result<Urdf, String> {
    let split = pieces(nodes, base_dir, &options.name);
    let mut out = Urdf { warnings: split.warnings, ..Urdf::default() };
    // Unique once cleaned, and so are their meshes' files.
    let names = crate::gltf::unique(split.pieces.iter().map(|p| xml_name(&p.name)).collect::<Vec<_>>().iter().map(String::as_str));
    let files = crate::gltf::unique(names.iter().map(|n| file_name(n)).collect::<Vec<_>>().iter().map(String::as_str));
    let files: Vec<String> = files.iter().map(|f| file_name(f)).collect();
    let mut xml = String::new();
    let _ = writeln!(xml, "<?xml version=\"1.0\"?>\n<!-- Code2Form -->\n<robot name=\"{}\">", names[0]);
    let mut solid = false;
    let mut flat = false;
    for ((piece, name), stem) in split.pieces.iter().zip(&names).zip(&files) {
        let _ = writeln!(xml, "  <link name=\"{name}\">");
        match &piece.model {
            Model::Solid(manifold) if !manifold.is_empty() => {
                solid = true;
                let mesh = colored_mesh(manifold);
                let file = format!("{}/{}.stl", options.mesh_folder.trim_end_matches('/'), stem);
                out.meshes.push((file.clone(), stl(&mesh.vertices, &mesh.triangles)));
                let [r, g, b, a] = main_color(&mesh.colors, options.color);
                let mass = Mass::of(&mesh.vertices, &mesh.triangles, options.density);
                let [cx, cy, cz] = mass.center;
                let [[ixx, ixy, ixz], [_, iyy, iyz], [_, _, izz]] = mass.inertia;
                let _ = write!(
                    xml,
                    "    <inertial>\n      <origin xyz=\"{} {} {}\" rpy=\"0 0 0\"/>\n      <mass value=\"{}\"/>\n      \
                     <inertia ixx=\"{}\" ixy=\"{}\" ixz=\"{}\" iyy=\"{}\" iyz=\"{}\" izz=\"{}\"/>\n    </inertial>\n",
                    num(cx),
                    num(cy),
                    num(cz),
                    num(mass.mass),
                    num(ixx),
                    num(ixy),
                    num(ixz),
                    num(iyy),
                    num(iyz),
                    num(izz),
                );
                for element in ["visual", "collision"] {
                    let _ = writeln!(xml, "    <{element}>\n      <geometry>\n        <mesh filename=\"{file}\"/>\n      </geometry>");
                    if element == "visual" {
                        let _ = writeln!(
                            xml,
                            "      <material name=\"{name}_color\">\n        <color rgba=\"{} {} {} {}\"/>\n      </material>",
                            num(f64::from(r)),
                            num(f64::from(g)),
                            num(f64::from(b)),
                            num(f64::from(a)),
                        );
                    }
                    let _ = writeln!(xml, "    </{element}>");
                }
            }
            Model::Flat(_) => flat = true,
            _ => {}
        }
        xml.push_str("  </link>\n");
    }
    if !solid {
        return Err(if flat { "the model is 2D; URDF holds solids" } else { "the model is empty; no URDF written" }.into());
    }
    if flat {
        out.warnings.push("The 2D shapes of the model are left out: URDF holds solids".into());
    }
    for (index, piece) in split.pieces.iter().enumerate().skip(1) {
        let parent = piece.parent.unwrap_or(0);
        let (xyz, rpy, scaled) = origin(&piece.matrix);
        if scaled {
            out.warnings.push(format!(
                "part(\"{}\") is scaled, mirrored or sheared, which a URDF joint cannot hold: it is written with the nearest rotation",
                piece.name
            ));
        }
        let joint = piece.joint.as_ref();
        let kind = joint.map_or(JointKind::Fixed, |j| j.kind);
        let _ = writeln!(xml, "  <joint name=\"{}_joint\" type=\"{}\">", names[index], kind.name());
        let _ = writeln!(xml, "    <parent link=\"{}\"/>\n    <child link=\"{}\"/>", names[parent], names[index]);
        let _ = writeln!(
            xml,
            "    <origin xyz=\"{} {} {}\" rpy=\"{} {} {}\"/>",
            num(xyz[0]),
            num(xyz[1]),
            num(xyz[2]),
            num(rpy[0]),
            num(rpy[1]),
            num(rpy[2])
        );
        if let Some(joint) = joint.filter(|j| j.kind != JointKind::Fixed) {
            out.moving += 1;
            let [x, y, z] = joint.axis;
            let _ = writeln!(xml, "    <axis xyz=\"{} {} {}\"/>", num(x), num(y), num(z));
            let limits = match (joint.kind, joint.limits) {
                (JointKind::Revolute, Some([lo, hi])) => Some([lo.to_radians(), hi.to_radians()]),
                (JointKind::Revolute, None) => {
                    out.warnings.push(format!(
                        "part(\"{}\") turns without limits: it is given -180 to 180 degrees; say joint = \"continuous\" for a wheel",
                        piece.name
                    ));
                    Some([-std::f64::consts::PI, std::f64::consts::PI])
                }
                (JointKind::Prismatic, Some([lo, hi])) => Some([lo * METERS, hi * METERS]),
                (JointKind::Prismatic, None) => {
                    out.warnings.push(format!(
                        "part(\"{}\") slides without limits: it is given 0 to 0 mm; give limits = [lowest, highest]",
                        piece.name
                    ));
                    Some([0.0, 0.0])
                }
                _ => None,
            };
            // URDF asks for the effort and speed of a joint with limits;
            // a design does not say them.
            if let Some([lo, hi]) = limits {
                let _ = writeln!(xml, "    <limit lower=\"{}\" upper=\"{}\" effort=\"10\" velocity=\"1\"/>", num(lo), num(hi));
            }
        }
        xml.push_str("  </joint>\n");
    }
    xml.push_str("</robot>\n");
    out.links = split.pieces.len();
    out.urdf = xml;
    Ok(out)
}

/// The joint's origin from a part's matrix: its translation in meters, its
/// rotation as roll, pitch and yaw (about the fixed X, Y then Z axes), and
/// whether it also scaled, mirrored or sheared the part.
fn origin(m: &Matrix) -> ([f64; 3], [f64; 3], bool) {
    let xyz = [m[0][3], m[1][3], m[2][3]].map(|c| c * METERS);
    let column = |j: usize| [m[0][j], m[1][j], m[2][j]];
    let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let (cx, cy, cz) = (column(0), column(1), column(2));
    let det = dot(cx, [cy[1] * cz[2] - cy[2] * cz[1], cy[2] * cz[0] - cy[0] * cz[2], cy[0] * cz[1] - cy[1] * cz[0]]);
    let scaled = [cx, cy, cz].iter().any(|&c| (length(c) - 1.0).abs() > 1e-6)
        || [dot(cx, cy), dot(cy, cz), dot(cx, cz)].iter().any(|d| d.abs() > 1e-6)
        || det < 0.0;
    // The nearest rotation, by Gram-Schmidt.
    let unit = |v: [f64; 3]| {
        let l = length(v);
        if l > 0.0 { v.map(|c| c / l) } else { v }
    };
    let x = unit(cx);
    let y = unit([cy[0] - x[0] * dot(x, cy), cy[1] - x[1] * dot(x, cy), cy[2] - x[2] * dot(x, cy)]);
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    // R = Rz(yaw) · Ry(pitch) · Rx(roll), its columns x, y, z.
    let r = |i: usize, j: usize| [x, y, z][j][i];
    let pitch = (-r(2, 0)).clamp(-1.0, 1.0).asin();
    let (roll, yaw) = if r(2, 0).abs() < 1.0 - 1e-12 {
        (r(2, 1).atan2(r(2, 2)), r(1, 0).atan2(r(0, 0)))
    } else {
        // Gimbal lock: the roll is folded into the yaw.
        (0.0, (-r(0, 1)).atan2(r(1, 1)))
    };
    (xyz, [roll, pitch, yaw], scaled)
}

/// A solid's mass, center of mass and inertia tensor about it, in its own
/// frame, in kilograms and meters.
#[derive(Clone, Copy, Debug)]
struct Mass {
    mass: f64,
    center: [f64; 3],
    inertia: [[f64; 3]; 3],
}

impl Mass {
    /// Summed over the tetrahedra from the origin to each triangle.
    fn of(vertices: &[[f64; 3]], triangles: &[[usize; 3]], density: f64) -> Mass {
        let mut volume = 0.0;
        let mut moment = [0.0; 3];
        let mut covariance = [[0.0; 3]; 3];
        for t in triangles {
            let [a, b, c] = t.map(|v| vertices[v].map(|x| x * METERS));
            let v = (a[0] * (b[1] * c[2] - b[2] * c[1]) + a[1] * (b[2] * c[0] - b[0] * c[2]) + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
            volume += v;
            let sum = [a[0] + b[0] + c[0], a[1] + b[1] + c[1], a[2] + b[2] + c[2]];
            for i in 0..3 {
                moment[i] += v * sum[i] / 4.0;
                for j in 0..3 {
                    let corners = a[i] * a[j] + b[i] * b[j] + c[i] * c[j];
                    covariance[i][j] += v / 20.0 * (corners + sum[i] * sum[j]);
                }
            }
        }
        let mass = volume * density;
        let center = if volume != 0.0 { moment.map(|m| m / volume) } else { [0.0; 3] };
        // About the center of mass, then the inertia tensor from it.
        let mut c = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                c[i][j] = density * covariance[i][j] - mass * center[i] * center[j];
            }
        }
        let trace = c[0][0] + c[1][1] + c[2][2];
        let mut inertia = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                inertia[i][j] = if i == j { trace - c[i][j] } else { -c[i][j] };
            }
        }
        Mass { mass, center, inertia }
    }
}

/// The color of most of a link's triangles: an STL has none, so the link
/// is drawn in one.
fn main_color(colors: &[[f32; 4]], [r, g, b]: [f32; 3]) -> [f32; 4] {
    let mut counts: Vec<([f32; 4], usize)> = Vec::new();
    for &c in colors {
        let c = if c == DEFAULT_COLOR || c == CUT_COLOR { [r, g, b, 1.0] } else { c };
        match counts.iter_mut().find(|(color, _)| *color == c) {
            Some((_, n)) => *n += 1,
            None => counts.push((c, 1)),
        }
    }
    counts.into_iter().max_by_key(|&(_, n)| n).map_or([r, g, b, 1.0], |(c, _)| c)
}

/// A binary STL of the triangles, in meters.
fn stl(vertices: &[[f64; 3]], triangles: &[[usize; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + 50 * triangles.len());
    let mut header = [0u8; 80];
    header[..9].copy_from_slice(b"Code2Form");
    out.extend(header);
    out.extend((triangles.len() as u32).to_le_bytes());
    for t in triangles {
        let [a, b, c] = t.map(|v| vertices[v].map(|x| x * METERS));
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = if length > 0.0 { n.map(|c| c / length) } else { [0.0; 3] };
        for p in [n, a, b, c] {
            for x in p {
                out.extend((x as f32).to_le_bytes());
            }
        }
        out.extend(0u16.to_le_bytes());
    }
    out
}

/// A name XML holds in an attribute: `&`, `<`, `>` and quotes left out.
fn xml_name(name: &str) -> String {
    name.chars().filter(|c| !matches!(c, '&' | '<' | '>' | '"' | '\'')).collect()
}

/// A name a file can have on every system.
fn file_name(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// `v` to twelve significant digits, which drops the float noise of the
/// sums (`0.05000000000000001`), without a trailing `.0`; under a
/// femtometer (the noise of a sum of zeros), or `-0`, is `0`.
fn num(v: f64) -> String {
    let rounded: f64 = format!("{v:.11e}").parse().unwrap_or(v);
    let rounded = if rounded.abs() < 1e-15 { 0.0 } else { rounded };
    let text = format!("{rounded:?}");
    text.strip_suffix(".0").map_or(text.clone(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARM: &str = "cube([40, 40, 10]);
        translate([20, 20, 10]) part(\"shoulder\", joint = \"continuous\") {
            cylinder(r = 8, h = 10);
            translate([0, 0, 10]) rotate([90, 0, 0]) part(\"elbow\", joint = \"revolute\", limits = [-90, 45]) {
                color(\"red\") cube([5, 30, 5]);
                translate([0, 30, 0]) part(\"gripper\", joint = \"prismatic\", axis = [1, 0, 0], limits = [0, 15]) cube(2);
            }
        }";

    fn write(src: &str) -> Urdf {
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
        urdf(&c2f_eval::evaluate(&program).nodes, Path::new(""), &UrdfOptions::default()).unwrap()
    }

    #[test]
    fn a_link_for_each_part_and_a_joint_between() {
        let out = write(ARM);
        assert_eq!((out.links, out.moving), (4, 3));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let xml = &out.urdf;
        assert!(xml.contains("<robot name=\"robot\">"));
        for name in ["robot", "shoulder", "elbow", "gripper"] {
            assert!(xml.contains(&format!("<link name=\"{name}\">")), "{xml}");
            assert!(out.meshes.iter().any(|(file, _)| file == &format!("meshes/{name}.stl")));
        }
        assert!(xml.contains("<joint name=\"shoulder_joint\" type=\"continuous\">"));
        assert!(xml.contains("<parent link=\"elbow\"/>\n    <child link=\"gripper\"/>"));
        // The shoulder sits 20 mm across, 10 mm up, in meters.
        assert!(xml.contains("<origin xyz=\"0.02 0.02 0.01\" rpy=\"0 0 0\"/>"), "{xml}");
        // The elbow is turned a quarter about X.
        assert!(xml.contains(&format!("rpy=\"{} 0 0\"", num(std::f64::consts::FRAC_PI_2))), "{xml}");
        assert!(xml.contains(&format!("<limit lower=\"{}\" upper=\"{}\"", num(-std::f64::consts::FRAC_PI_2), num(45f64.to_radians()))));
        assert!(xml.contains("<limit lower=\"0\" upper=\"0.015\""));
        assert!(xml.contains("<axis xyz=\"1 0 0\"/>"), "{xml}");
        // The elbow's red is its color.
        assert!(xml.contains("<color rgba=\"1 0 0 1\"/>"), "{xml}");
    }

    #[test]
    fn a_meshes_triangles_in_meters() {
        let out = write("cube(10);");
        let (_, stl) = &out.meshes[0];
        assert_eq!(u32::from_le_bytes(stl[80..84].try_into().unwrap()), 12);
        assert_eq!(stl.len(), 84 + 12 * 50);
        let coordinates: Vec<f32> =
            (0..12).flat_map(|t| (0..9).map(move |k| 84 + t * 50 + 12 + k * 4)).map(|at| f32::from_le_bytes(stl[at..at + 4].try_into().unwrap())).collect();
        assert!(coordinates.iter().all(|&c| c == 0.0 || (c - 0.01).abs() < 1e-7), "{coordinates:?}");
    }

    #[test]
    fn a_cube_weighs_what_its_volume_does() {
        // 10 cm of water: a kilogram.
        let program = c2f_eval::Program::load("cube(100);", Path::new("t.scad"), &[]).unwrap();
        let options = UrdfOptions { density: 1000.0, ..UrdfOptions::default() };
        let out = urdf(&c2f_eval::evaluate(&program).nodes, Path::new(""), &options).unwrap();
        let program = c2f_eval::Program::load("cube(100);", Path::new("t.scad"), &[]).unwrap();
        let Model::Solid(cube) = crate::render(&c2f_eval::evaluate(&program).nodes).model else { panic!() };
        let mesh = colored_mesh(&cube);
        let mass = Mass::of(&mesh.vertices, &mesh.triangles, 1000.0);
        assert!((mass.mass - 1.0).abs() < 1e-9);
        assert!(mass.center.iter().all(|c| (c - 0.05).abs() < 1e-12));
        // A cube's: m (a² + a²) / 12 about each axis, none across.
        let expected = 0.02 / 12.0;
        for i in 0..3 {
            assert!((mass.inertia[i][i] - expected).abs() < 1e-12, "{:?}", mass.inertia);
            assert!(mass.inertia[i][(i + 1) % 3].abs() < 1e-12, "{:?}", mass.inertia);
        }
        assert!(out.urdf.contains("<mass value=\"1\"/>"), "{}", out.urdf);
        assert!(out.urdf.contains("<origin xyz=\"0.05 0.05 0.05\" rpy=\"0 0 0\"/>"), "{}", out.urdf);
    }

    #[test]
    fn roll_pitch_and_yaw_read_back() {
        let rotation = |roll: f64, pitch: f64, yaw: f64| {
            let (sr, cr) = roll.sin_cos();
            let (sp, cp) = pitch.sin_cos();
            let (sy, cy) = yaw.sin_cos();
            [
                [cy * cp, cy * sp * sr - sy * cr, cy * sp * cr + sy * sr, 1.0],
                [sy * cp, sy * sp * sr + cy * cr, sy * sp * cr - cy * sr, 2.0],
                [-sp, cp * sr, cp * cr, 3.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        };
        let (xyz, rpy, scaled) = origin(&rotation(0.3, -0.7, 2.0));
        assert_eq!(xyz, [0.001, 0.002, 0.003]);
        assert!(!scaled);
        for (got, want) in rpy.iter().zip([0.3, -0.7, 2.0]) {
            assert!((got - want).abs() < 1e-12, "{rpy:?}");
        }
        let mut mirrored = rotation(0.0, 0.0, 0.0);
        mirrored[0][0] = -1.0;
        assert!(origin(&mirrored).2);
    }

    #[test]
    fn names_stay_apart_once_cleaned() {
        let out = write("part(\"arm\") cube(1); part(\"arm\") cube(1); part(\"arm_2\") cube(1); part(\"a<b\") cube(1); part(\"ab\") cube(1);");
        let mut files: Vec<&String> = out.meshes.iter().map(|(file, _)| file).collect();
        let count = files.len();
        files.dedup();
        files.sort();
        files.dedup();
        assert_eq!(files.len(), count, "{files:?}");
        let mut links: Vec<&str> = out.urdf.lines().filter(|l| l.contains("<link name=")).collect();
        links.sort();
        links.dedup();
        assert_eq!(links.len(), 6, "{links:?}");
    }

    #[test]
    fn a_turn_without_limits_is_given_a_full_turn() {
        let out = write("cube(1); part(\"arm\", joint = \"revolute\") cube(1);");
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.urdf.contains(&format!("lower=\"{}\"", num(-std::f64::consts::PI))));
    }
}
