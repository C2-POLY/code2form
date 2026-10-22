//! Built-in modules for an enclosure made for a circuit, not in the
//! reference: a file that names its circuit (`// @circuit robot.c2fc`)
//! gets standoffs, screw holes and cutouts where the circuit's parts
//! declare their mount holes and ports, and the parts themselves. They are
//! in the circuit's coordinates, so moving a part in the circuit moves them
//! at the next render.
//!
//! ```text
//! difference() { plate(); cutout_for("xiao", "usb_c", clearance = 0.5); }
//! standoffs_for("driver", height = 4, screw = "M2");
//! ```

use c2f_circuit::Instance;
use c2f_lang::annotations::{MountHole, PortSize};

use crate::interp::Interp;
use crate::modules::Params;
use crate::node::{IDENTITY, Matrix, Node, NodeKind, Segments};
use crate::value::Value;

/// Screws a standoff or a hole is sized for: (name, pilot hole a screw
/// cuts its thread into in plastic, clearance hole it passes through,
/// standoff diameter).
const SCREWS: [(&str, f64, f64, f64); 3] = [("M2", 1.6, 2.4, 4.5), ("M2.5", 2.1, 2.9, 5.5), ("M3", 2.5, 3.4, 6.0)];

/// How far behind its opening a cutout starts, so that it cuts a wall
/// touching the part cleanly.
const BEHIND: f64 = 1.0;

/// The part the call names, placed in the circuit; `None` after a warning
/// when there is no circuit or no such part.
fn instance<'a>(interp: &mut Interp<'a>, module: &str, p: &Params) -> Option<&'a Instance> {
    let program = interp.program();
    let Some(circuit) = &program.circuit else {
        let why = match &program.circuit_path {
            Some(path) => format!("the circuit {} could not be read", path.display()),
            None => "the file names no circuit: add a comment such as `// @circuit robot.c2fc`".to_string(),
        };
        interp.warn(format!("Ignoring {module}(): {why}"));
        return None;
    };
    let name = match p.get("part") {
        Value::Str(name) => name.to_string(),
        other => {
            interp.warn(format!("Ignoring {module}(): part = {} is not the name of a part", other.repr()));
            return None;
        }
    };
    let found = circuit.parts.iter().find(|part| part.name == name);
    if found.is_none() {
        let names: Vec<&str> = circuit.parts.iter().map(|part| part.name.as_str()).collect();
        interp.warn(format!("Ignoring {module}(): the circuit has no part named \"{name}\" ({})", names.join(", ")));
    }
    found
}

/// `standoffs_for(part, height, d, hole, screw)`: a tube under each mount
/// hole of the part, from the hole along its direction (down, the side the
/// part rests on, unless the part says otherwise), `height` long, `d`
/// across with a `hole` for the screw. `screw = "M2"`, `"M2.5"` or `"M3"`
/// sizes both for a screw cutting its thread in the plastic; without it,
/// they follow the part's hole.
pub(crate) fn standoffs(interp: &mut Interp, p: &Params) -> Option<Node> {
    let part = instance(interp, "standoffs_for", p)?;
    let height = positive(interp, "standoffs_for", p, "height", None)?;
    let screw = screw(interp, "standoffs_for", p);
    let tubes = holes(interp, "standoffs_for", part)?
        .iter()
        .map(|hole| {
            let d = p.number("d").unwrap_or(screw.map_or(hole.diameter * 2.25, |s| s.3));
            let bore = p.number("hole").unwrap_or(screw.map_or(hole.diameter * 0.8, |s| s.1));
            let mut tube = cylinder(height, d, 0.0, p.segments(true));
            if bore > 0.0 {
                let bore = cylinder(height + 2.0, bore, -1.0, p.segments(true));
                tube = Node::new(NodeKind::Difference, vec![tube, bore]);
            }
            transform(frame(hole.position, hole.direction), tube)
        })
        .collect();
    Some(transform(part.matrix(), Node::group(tubes)))
}

/// `holes_for(part, depth, hole, screw)`: a cylinder at each mount hole of
/// the part, from 1 mm behind it along its direction, `depth` long, to
/// subtract from what the part is screwed to. `screw` sizes it for the
/// screw to pass; without it, it is the part's hole and 0.4 mm.
pub(crate) fn screw_holes(interp: &mut Interp, p: &Params) -> Option<Node> {
    let part = instance(interp, "holes_for", p)?;
    let depth = positive(interp, "holes_for", p, "depth", Some(10.0))?;
    let screw = screw(interp, "holes_for", p);
    let cylinders = holes(interp, "holes_for", part)?
        .iter()
        .map(|hole| {
            let d = p.number("hole").unwrap_or(screw.map_or(hole.diameter + 0.4, |s| s.2));
            transform(frame(hole.position, hole.direction), cylinder(depth + BEHIND, d, -BEHIND, p.segments(true)))
        })
        .collect();
    Some(transform(part.matrix(), Node::group(cylinders)))
}

/// `cutout_for(part, port, clearance, depth)`: a prism of the port's
/// opening grown by `clearance` on every side (0.5 mm unless given), from
/// 1 mm behind the opening along the way it faces, `depth` long (10 mm
/// unless given), to subtract from a wall. A port `[width, height]` facing
/// sideways is `width` across and `height` up; facing up or down, `width`
/// is along the part's x and `height` along its y.
pub(crate) fn cutout(interp: &mut Interp, p: &Params) -> Option<Node> {
    let part = instance(interp, "cutout_for", p)?;
    let name = match p.get("port") {
        Value::Str(name) => name.to_string(),
        other => {
            interp.warn(format!("Ignoring cutout_for(): port = {} is not the name of a port", other.repr()));
            return None;
        }
    };
    let Some(port) = part.annotations.ports.iter().find(|port| port.name == name) else {
        let names: Vec<&str> = part.annotations.ports.iter().map(|port| port.name.as_str()).collect();
        let known = if names.is_empty() { "it has none".to_string() } else { names.join(", ") };
        let (part, file) = (&part.name, &part.part);
        interp.warn(format!("Ignoring cutout_for(): {part} ({file}) has no port named \"{name}\" ({known})"));
        return None;
    };
    let clearance = match p.number("clearance") {
        Some(c) if c.is_finite() && c >= 0.0 => c,
        None if p.get("clearance").is_undef() => 0.5,
        _ => {
            interp.warn(format!("cutout_for(): clearance = {} is not a size, 0.5 is used", p.get("clearance").repr()));
            0.5
        }
    };
    let depth = positive(interp, "cutout_for", p, "depth", Some(10.0))?;
    let length = depth + BEHIND;
    let prism = match port.size {
        PortSize::Rect([w, h]) => {
            let size = [w + 2.0 * clearance, h + 2.0 * clearance, length];
            let cube = Node::new(NodeKind::Cube { size, center: false }, Vec::new());
            transform(translation([-size[0] / 2.0, -size[1] / 2.0, -BEHIND]), cube)
        }
        PortSize::Round(d) => cylinder(length, d + 2.0 * clearance, -BEHIND, p.segments(true)),
    };
    Some(transform(part.matrix(), transform(frame(port.position, port.direction), prism)))
}

/// `circuit_part(part)`: the part itself, with its parameters, where the
/// circuit places it: to see it in the enclosure (`%circuit_part("xiao")`)
/// or to derive a seat from it (`projection() circuit_part("xiao")`).
pub(crate) fn part(interp: &mut Interp, p: &Params) -> Option<Node> {
    let part = instance(interp, "circuit_part", p)?;
    let nodes = interp.circuit_part(part);
    Some(transform(part.matrix(), Node::group(nodes)))
}

/// The part's mount holes, or `None` after a warning when it has none.
fn holes<'a>(interp: &mut Interp, module: &str, part: &'a Instance) -> Option<&'a [MountHole]> {
    let holes = &part.annotations.mount_holes;
    if holes.is_empty() {
        interp.warn(format!("Ignoring {module}(): {} ({}) declares no mount hole", part.name, part.part));
        return None;
    }
    Some(holes)
}

/// A positive size: `default` when left out (a warning when there is none),
/// `None` after a warning when it is not a positive number.
fn positive(interp: &mut Interp, module: &str, p: &Params, name: &str, default: Option<f64>) -> Option<f64> {
    match (p.number(name), default) {
        (Some(n), _) if n.is_finite() && n > 0.0 => Some(n),
        (None, Some(default)) if p.get(name).is_undef() => Some(default),
        _ => {
            interp.warn(format!("Ignoring {module}(): {name} = {} is not a positive size", p.get(name).repr()));
            None
        }
    }
}

/// The screw `screw = "M2"` names, if any, after a warning for one not
/// known.
fn screw(interp: &mut Interp, module: &str, p: &Params) -> Option<(&'static str, f64, f64, f64)> {
    let given = p.get("screw");
    if given.is_undef() {
        return None;
    }
    let found = given.as_str().and_then(|name| SCREWS.iter().find(|s| s.0.eq_ignore_ascii_case(name)));
    if found.is_none() {
        let known: Vec<&str> = SCREWS.iter().map(|s| s.0).collect();
        interp.warn(format!("{module}(): unknown screw = {}, expected one of {}", given.repr(), known.join(", ")));
    }
    found.copied()
}

/// A cylinder along z from `z` up, `d` across.
fn cylinder(h: f64, d: f64, z: f64, segments: Segments) -> Node {
    let r = d / 2.0;
    let node = Node::new(NodeKind::Cylinder { h, r1: r, r2: r, center: false, segments }, Vec::new());
    if z == 0.0 { node } else { transform(translation([0.0, 0.0, z]), node) }
}

fn transform(m: Matrix, child: Node) -> Node {
    Node::new(NodeKind::Transform(m), vec![child])
}

fn translation(v: [f64; 3]) -> Matrix {
    let mut m = IDENTITY;
    for (row, value) in v.into_iter().enumerate() {
        m[row][3] = value;
    }
    m
}

/// The matrix that takes z to the unit vector `n` and the origin to `at`:
/// x across `n` and horizontal (the x axis when `n` is vertical), y the
/// third axis, so that x, y and `n` turn as x, y and z do.
fn frame(at: [f64; 3], n: [f64; 3]) -> Matrix {
    let x = if n[0].abs() < 1e-9 && n[1].abs() < 1e-9 {
        [1.0, 0.0, 0.0]
    } else {
        // up × n, normalized.
        let x = [-n[1], n[0], 0.0];
        let length = (x[0] * x[0] + x[1] * x[1]).sqrt();
        [x[0] / length, x[1] / length, 0.0]
    };
    let y = [n[1] * x[2] - n[2] * x[1], n[2] * x[0] - n[0] * x[2], n[0] * x[1] - n[1] * x[0]];
    let mut m = IDENTITY;
    for row in 0..3 {
        m[row] = [x[row], y[row], n[row], at[row]];
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|r| (0..3).map(|c| m[r][c] * v[c]).sum::<f64>() + m[r][3])
    }

    #[test]
    fn a_frame_turns_z_to_its_direction() {
        for n in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.6, 0.0, 0.8]] {
            let m = frame([1.0, 2.0, 3.0], n);
            let z = apply(&m, [0.0, 0.0, 1.0]);
            assert!((0..3).all(|k| (z[k] - [1.0, 2.0, 3.0][k] - n[k]).abs() < 1e-12), "{n:?}");
            // Right-handed: x × y = z.
            let (x, y) = ([m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]]);
            let cross = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
            assert!((0..3).all(|k| (cross[k] - n[k]).abs() < 1e-12), "{n:?}");
        }
        // Facing sideways, the width is horizontal and the height up.
        let m = frame([0.0; 3], [0.0, -1.0, 0.0]);
        assert_eq!(apply(&m, [1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
        assert_eq!(apply(&m, [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }
}
