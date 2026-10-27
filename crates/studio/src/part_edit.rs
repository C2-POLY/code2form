//! Moving, turning and removing a circuit's parts with the mouse: a part
//! dragged flat at its own height, snapped to a grid; a quarter turn about
//! z; a part removed with its wires. Each becomes an edit of the circuit
//! file's text by the spans it was read with, so that the file keeps its
//! comments and layout.

use std::ops::Range;

use c2f_circuit::{Circuit, Instance};
use glam::Vec3;

use crate::catalogue::coordinate;
use crate::i18n::tr;
use crate::pick::{self, Hit, Ray};
use crate::rewire::{self, Side};
use crate::wiring::Wiring;

/// The grid a dragged part snaps to, in millimeters, and with Shift.
pub const SNAP: f64 = 0.5;
pub const FINE_SNAP: f64 = 0.1;

/// What a press in the view takes hold of, pins first, then a wire's end,
/// then a part: the smaller sit on the larger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Pin(c2f_circuit::End),
    WireEnd(usize, Side),
    Part(usize),
}

pub fn grab(wiring: &Wiring, part_boxes: &[Option<(Vec3, Vec3)>], ray: &Ray) -> Option<Grab> {
    match pick::pick(wiring, part_boxes, ray) {
        Some(Hit::Pin(end)) => return Some(Grab::Pin(end)),
        Some(Hit::Wire(index)) => {
            if let Some(side) = pick::wire_side(wiring, index, ray) {
                return Some(Grab::WireEnd(index, side));
            }
        }
        Some(Hit::Part(index)) => return Some(Grab::Part(index)),
        None => return None,
    }
    // The middle of a wire: the part behind it, if any.
    let no_wires = Wiring { paths: Vec::new(), ..wiring.clone() };
    match pick::pick(&no_wires, part_boxes, ray) {
        Some(Hit::Part(index)) => Some(Grab::Part(index)),
        _ => None,
    }
}

/// Where the ray meets the level `z`, in front of the eye.
pub fn on_level(ray: &Ray, z: f64) -> Option<[f64; 2]> {
    let dz = f64::from(ray.direction.z);
    if dz.abs() < 1e-6 {
        return None;
    }
    let t = (z - f64::from(ray.origin.z)) / dz;
    (t > 0.0).then(|| {
        let p = ray.origin.as_dvec3() + ray.direction.as_dvec3() * t;
        [p.x, p.y]
    })
}

/// `v` to the nearest multiple of `step`.
pub fn snap(v: f64, step: f64) -> f64 {
    let snapped = (v / step).round() * step;
    // Rid the float of its noise: 0.30000000000000004 reads 0.3.
    (snapped * 1e6).round() / 1e6
}

/// A part being dragged.
#[derive(Clone, Debug)]
pub struct Drag {
    pub part: usize,
    /// From where the ray met the part's level to its `at`.
    pub offset: [f64; 2],
    /// Its `at` before the drag, and now.
    pub from: [f64; 3],
    pub at: [f64; 3],
    /// Quarter turns asked for (R) during the drag, written on release.
    pub turns: u32,
}

impl Drag {
    /// Takes hold of `part` where `ray` meets its level.
    pub fn start(circuit: &Circuit, part: usize, ray: &Ray) -> Option<Drag> {
        let at = circuit.parts.get(part)?.at;
        let [x, y] = on_level(ray, at[2])?;
        Some(Drag { part, offset: [at[0] - x, at[1] - y], from: at, at, turns: 0 })
    }

    /// Follows the ray: the part's new `at`, snapped to `step`; whether it
    /// changed.
    pub fn follow(&mut self, ray: &Ray, step: f64) -> bool {
        let Some([x, y]) = on_level(ray, self.from[2]) else { return false };
        let at = [snap(x + self.offset[0], step), snap(y + self.offset[1], step), self.from[2]];
        let changed = at != self.at;
        self.at = at;
        changed
    }

    pub fn moved(&self) -> bool {
        self.at != self.from
    }
}

/// `[x, y, z]` as a circuit writes it.
fn vector(v: [f64; 3]) -> String {
    format!("[{}, {}, {}]", coordinate(v[0]), coordinate(v[1]), coordinate(v[2]))
}

fn changed() -> String {
    tr(
        "the circuit changed since it was read: try again once it is rendered",
        "le circuit a changé depuis sa lecture : réessayez une fois le rendu fini",
    )
    .to_string()
}

/// The circuit's text with part `instance` moved to `at` (when given) and
/// turned `turns` quarter turns about z: `at` and `rotate` rewritten in
/// place, or added after its `part` line. One edit, and where to put the
/// cursor, as a char index.
pub fn place(src: &str, instance: &Instance, at: Option<[f64; 3]>, turns: u32) -> Result<(String, usize), String> {
    let turned = (turns > 0).then(|| {
        let [x, y, z] = instance.rotate;
        vector([x, y, (z + 90.0 * f64::from(turns)).rem_euclid(360.0)])
    });
    set_placement(src, instance, at.map(vector), turned)
}

/// The circuit's text with part `instance`'s `at` and `rotate` set to the
/// vectors written `at` and `rotate` (when given): rewritten in place, or
/// added after its `part` line. One edit, and where to put the cursor, as
/// a char index.
pub fn set_placement(src: &str, instance: &Instance, at: Option<String>, rotate: Option<String>) -> Result<(String, usize), String> {
    let range = |s: c2f_lang::Span| s.start as usize..s.end as usize;
    // The part line must still be where it was read.
    let part = range(instance.part_span);
    if src.get(part.clone()).is_none_or(|text| text.trim_matches(['"', '\'']) != instance.part) {
        return Err(changed());
    }
    let line_end = src[part.end..].find('\n').map_or(src.len(), |i| part.end + i + 1);
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut added = String::new();
    if !src[..line_end].ends_with('\n') {
        added.push('\n');
    }
    if let Some(at) = at {
        match instance.at_span {
            Some(span) => edits.push((range(span), at)),
            None => added.push_str(&format!("at = {at}\n")),
        }
    }
    if let Some(rotate) = rotate {
        match instance.rotate_span {
            Some(span) => edits.push((range(span), rotate)),
            None => added.push_str(&format!("rotate = {rotate}\n")),
        }
    }
    if added.trim().is_empty() {
        added.clear();
    } else {
        edits.push((line_end..line_end, added));
    }
    if edits.iter().any(|(r, _)| src.get(r.clone()).is_none()) {
        return Err(changed());
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = src.to_string();
    for (r, text) in &edits {
        out.replace_range(r.clone(), text);
    }
    let cursor = out[..line_end.min(out.len())].chars().count();
    Ok((out, cursor.saturating_sub(1)))
}

/// The circuit's text without part `index`'s table and the tables of the
/// wires to it; and how many wires went.
pub fn remove(src: &str, circuit: &Circuit, index: usize) -> Result<(String, usize, usize), String> {
    let instance = circuit.parts.get(index).ok_or_else(changed)?;
    let mut ranges = vec![
        rewire::table_of(src, instance.span.start as usize, |h| h.starts_with("[parts.")).ok_or_else(changed)?,
    ];
    let wires: Vec<_> = circuit.wires.iter().filter(|w| w.from.part == index || w.to.part == index).collect();
    for wire in &wires {
        ranges.push(rewire::table_of(src, wire.span.start as usize, |h| h == "[[wires]]").ok_or_else(changed)?);
    }
    ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut out = src.to_string();
    let mut last_start = usize::MAX;
    for r in &ranges {
        // Tables never overlap; a range past the one after it would.
        if r.end > last_start {
            return Err(changed());
        }
        out.replace_range(r.clone(), "");
        last_start = r.start;
    }
    let cursor = src[..ranges.last().map_or(0, |r| r.start)].chars().count();
    Ok((out, cursor, wires.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use c2f_circuit::End;

    const SRC: &str = r#"version = 1

# The first board.
[parts.a]
part = "board.c2f"

[parts.b]
part = "board.c2f"   # the second
at = [0, 30, 0]
rotate = [0, 0, 270]

[[wires]]
from = "a.A"
to = "b.A"

# Signals.

[[wires]]
from = "b.B"
to = "b.C"

[[wires]]
from = "a.B"
to = "b.B"
"#;

    fn read(src: &str) -> Circuit {
        let path = crate::wiring::tests::circuit().parts[0].path.with_file_name("test.c2fc");
        c2f_circuit::parse(src, &path, &[]).unwrap()
    }

    #[test]
    fn at_and_rotate_are_rewritten_or_added() {
        let c = read(SRC);
        // b has both: rewritten in place, its comment kept; 270 + 90 is 0.
        let (text, _) = place(SRC, &c.parts[1], Some([12.5, -3.0, 0.0]), 1).unwrap();
        assert_eq!(
            text,
            SRC.replace("at = [0, 30, 0]\nrotate = [0, 0, 270]", "at = [12.5, -3, 0]\nrotate = [0, 0, 0]")
        );
        assert_eq!(read(&text).parts[1].at, [12.5, -3.0, 0.0]);
        // a has neither: added after its part line.
        let (text, _) = place(SRC, &c.parts[0], Some([1.0, 2.0, 0.0]), 1).unwrap();
        assert!(text.contains("[parts.a]\npart = \"board.c2f\"\nat = [1, 2, 0]\nrotate = [0, 0, 90]\n\n[parts.b]"), "{text}");
        let again = read(&text);
        assert_eq!((again.parts[0].at, again.parts[0].rotate), ([1.0, 2.0, 0.0], [0.0, 0.0, 90.0]));
        // Turned alone, three times over.
        let (text, _) = place(SRC, &c.parts[0], None, 3).unwrap();
        assert!(text.contains("part = \"board.c2f\"\nrotate = [0, 0, 270]\n\n"), "{text}");
        // A text that moved on is not edited.
        assert!(place(&SRC.replacen("board.c2f", "other.c2f", 1), &c.parts[0], Some([0.0; 3]), 0).is_err());
    }

    #[test]
    fn removing_a_part_takes_its_wires_and_leaves_the_comments() {
        let c = read(SRC);
        let (text, _, wires) = remove(SRC, &c, 1).unwrap();
        assert_eq!(wires, 3);
        assert_eq!(text, "version = 1\n\n# The first board.\n[parts.a]\npart = \"board.c2f\"\n\n# Signals.\n");
        let (text, _, wires) = remove(SRC, &c, 0).unwrap();
        assert_eq!(wires, 2);
        assert!(text.contains("# Signals.\n\n[[wires]]\nfrom = \"b.B\"\nto = \"b.C\"\n"), "{text}");
        assert!(!text.contains("[parts.a]") && text.contains("# The first board.\n"), "{text}");
        let left = read(&text);
        assert_eq!((left.parts.len(), left.wires.len()), (1, 1));
    }

    #[test]
    fn snapping_keeps_to_the_grid() {
        assert_eq!(snap(12.26, SNAP), 12.5);
        assert_eq!(snap(12.24, SNAP), 12.0);
        assert_eq!(snap(-0.26, SNAP), -0.5);
        assert_eq!(snap(0.29, FINE_SNAP), 0.3);
        assert_eq!(snap(1.04, FINE_SNAP), 1.0);
    }

    #[test]
    fn a_drag_keeps_the_grab_offset_at_the_parts_level() {
        let mut c = read(SRC);
        c.parts[1].at = [10.0, 30.0, 5.0];
        let down = |x: f32, y: f32| Ray {
            origin: Vec3::new(x, y + 40.0, 45.0),
            direction: Vec3::new(0.0, -40.0, -40.0).normalize(),
            slack: 0.0,
        };
        // The ray meets z = 5 at (x, y): grabbed 2 mm right and 1 mm
        // behind the part's origin.
        let mut drag = Drag::start(&c, 1, &down(12.0, 31.0)).unwrap();
        assert!(!drag.moved());
        assert!(drag.follow(&down(22.2, 21.0), SNAP));
        assert_eq!(drag.at, [20.0, 20.0, 5.0]);
        drag.follow(&down(22.33, 21.0), FINE_SNAP);
        assert_eq!(drag.at, [20.3, 20.0, 5.0]);
        assert!(drag.moved());
    }

    #[test]
    fn a_press_takes_a_pin_then_a_wire_end_then_a_part() {
        let wiring = Wiring::new(crate::wiring::tests::circuit());
        let boxes = [Some((Vec3::ZERO, Vec3::new(10.0, 5.0, 1.0))), Some((Vec3::new(0.0, 30.0, 0.0), Vec3::new(10.0, 35.0, 1.0)))];
        let ray = |from: [f32; 3], to: [f32; 3]| {
            let origin = Vec3::from(from);
            Ray { origin, direction: (Vec3::from(to) - origin).normalize(), slack: 0.0 }
        };
        let down = |x: f32, y: f32| grab(&wiring, &boxes, &ray([x, y, 50.0], [x, y, 0.0]));
        assert_eq!(down(0.0, 0.0), Some(Grab::Pin(End { part: 0, pin: 0 })));
        assert_eq!(down(7.0, 4.0), Some(Grab::Part(0)));
        let path = &wiring.paths[2];
        let near = path[path.len() / 6].map(|c| c as f32);
        assert_eq!(grab(&wiring, &boxes, &ray([near[0], near[1], near[2] + 60.0], near)), Some(Grab::WireEnd(2, Side::From)));
        assert_eq!(down(60.0, 60.0), None);
    }
}
