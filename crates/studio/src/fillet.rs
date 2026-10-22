//! Rounded and chamfered primitives for the properties panel: the call
//! that replaces a selected `cube()` or `cylinder()` with its edges
//! rounded (filleted) or chamfered, keeping its bounding box and its
//! placement. The calls go to the modules of the library's
//! `shapes/rounded.c2f` ([`LIBRARY`]), which [`apply`] `use`s at the top
//! of the source.
//!
//! A library call rather than the geometry written out (a `hull()` of
//! spheres): it stays one line that reads as what it does, its radius
//! stays one number to edit by hand or from the panel again, and the
//! module can place its curves exactly on the box. See
//! `docs/research/conges-chanfreins.md`.
//!
//! The box stays the same, but for a cone whose widest rim is rounded or
//! chamfered: that rim is its widest circle, so it gets narrower (not
//! shorter). The parameters are numbers: a caller that has `cube([w, d,
//! h])` passes the evaluated values.

// The properties panel that calls these is not written yet.
#![allow(dead_code)]

use std::ops::Range;

/// The library file the generated calls need, as `use` names it.
pub const LIBRARY: &str = "shapes/rounded.c2f";

/// Which edges of a cube to round or chamfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubeEdges {
    /// The twelve edges.
    All,
    /// The four vertical edges.
    Vertical,
    /// The four edges around the top face; the vertical edges stay sharp.
    Top,
    /// The four edges around the bottom face.
    Bottom,
}

impl CubeEdges {
    fn name(self) -> &'static str {
        match self {
            CubeEdges::All => "all",
            CubeEdges::Vertical => "vertical",
            CubeEdges::Top => "top",
            CubeEdges::Bottom => "bottom",
        }
    }
}

/// Which rims of a cylinder to round or chamfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rims {
    Top,
    Bottom,
    Both,
}

impl Rims {
    fn name(self) -> &'static str {
        match self {
            Rims::Top => "top",
            Rims::Bottom => "bottom",
            Rims::Both => "both",
        }
    }
}

/// `cube(size, center)` with its `edges` rounded to `radius`:
/// `rounded_cube([30, 20, 10], fillet = 2, edges = "vertical")`.
pub fn rounded_cube(size: [f64; 3], center: bool, radius: f64, edges: CubeEdges) -> Result<String, String> {
    check_cube(size, radius, edges, "Le rayon du congé")?;
    Ok(cube_call("rounded_cube", size, center, "fillet", radius, edges))
}

/// `cube(size, center)` with its `edges` cut at 45 degrees, `distance`
/// back from each edge: `chamfered_cube([30, 20, 10], chamfer = 1)`.
pub fn chamfered_cube(size: [f64; 3], center: bool, distance: f64, edges: CubeEdges) -> Result<String, String> {
    check_cube(size, distance, edges, "Le chanfrein")?;
    Ok(cube_call("chamfered_cube", size, center, "chamfer", distance, edges))
}

/// `cylinder(h, r1, r2, center)` with its `rims` rounded to `radius`,
/// tangent to the cap and to the side:
/// `rounded_cylinder(h = 20, r = 10, fillet = 2, rims = "top")`.
pub fn rounded_cylinder(h: f64, r1: f64, r2: f64, center: bool, radius: f64, rims: Rims) -> Result<String, String> {
    check_cylinder(h, r1, r2, radius, rims, true)?;
    Ok(cylinder_call("rounded_cylinder", h, r1, r2, center, "fillet", radius, rims))
}

/// `cylinder(h, r1, r2, center)` with its `rims` cut `distance` back from
/// the rim on the cap and on the side.
pub fn chamfered_cylinder(h: f64, r1: f64, r2: f64, center: bool, distance: f64, rims: Rims) -> Result<String, String> {
    check_cylinder(h, r1, r2, distance, rims, false)?;
    Ok(cylinder_call("chamfered_cylinder", h, r1, r2, center, "chamfer", distance, rims))
}

/// Replaces the bytes `call` of `src` (a primitive's call, without its
/// `;`) by `replacement`, and adds `use <shapes/rounded.c2f>` on the first
/// line unless the source already has it.
pub fn apply(src: &str, call: Range<usize>, replacement: &str) -> String {
    let mut out = String::with_capacity(src.len() + replacement.len() + 32);
    let line = format!("use <{LIBRARY}>");
    if !src.lines().any(|l| l.trim() == line) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&src[..call.start]);
    out.push_str(replacement);
    out.push_str(&src[call.end..]);
    out
}

/// A number as the source writes it: up to nine decimals, no trailing
/// zeros.
fn number(x: f64) -> String {
    let text = format!("{:.9}", x);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

fn cube_call(module: &str, size: [f64; 3], center: bool, key: &str, d: f64, edges: CubeEdges) -> String {
    let mut call = format!("{module}([{}, {}, {}], {key} = {}", number(size[0]), number(size[1]), number(size[2]), number(d));
    if edges != CubeEdges::All {
        call += &format!(", edges = \"{}\"", edges.name());
    }
    if center {
        call += ", center = true";
    }
    call + ")"
}

#[allow(clippy::too_many_arguments)]
fn cylinder_call(module: &str, h: f64, r1: f64, r2: f64, center: bool, key: &str, d: f64, rims: Rims) -> String {
    let radii = if r1 == r2 { format!("r = {}", number(r1)) } else { format!("r1 = {}, r2 = {}", number(r1), number(r2)) };
    let mut call = format!("{module}(h = {}, {radii}, {key} = {}", number(h), number(d));
    if rims != Rims::Both {
        call += &format!(", rims = \"{}\"", rims.name());
    }
    if center {
        call += ", center = true";
    }
    call + ")"
}

fn check_cube(size: [f64; 3], d: f64, edges: CubeEdges, what: &str) -> Result<(), String> {
    if size.iter().any(|s| !s.is_finite() || *s <= 0.0) {
        return Err("Le cube doit avoir trois dimensions positives.".into());
    }
    if !d.is_finite() || d <= 0.0 {
        return Err(format!("{what} doit être positif."));
    }
    let [x, y, z] = size;
    let (limit, words) = match edges {
        CubeEdges::All => (x.min(y).min(z) / 2.0, "la moitié de la plus petite dimension"),
        CubeEdges::Vertical => (x.min(y) / 2.0, "la moitié de la largeur ou de la profondeur"),
        // A rounded top may take the whole height.
        CubeEdges::Top | CubeEdges::Bottom if z < x.min(y) / 2.0 => (z, "la hauteur"),
        CubeEdges::Top | CubeEdges::Bottom => (x.min(y) / 2.0, "la moitié de la largeur ou de la profondeur"),
    };
    if d > limit * (1.0 + 1e-9) {
        return Err(format!("{what} ne peut dépasser {words} ({}).", number(limit)));
    }
    Ok(())
}

/// How far back from a rim a fillet or a chamfer starts, along the cap
/// and along the side: the same for a chamfer; `d / tan(angle / 2)` for a
/// fillet, `angle` being the corner's.
fn setback(d: f64, angle: f64, round: bool) -> f64 {
    if round { d / (angle / 2.0).tan() } else { d }
}

fn check_cylinder(h: f64, r1: f64, r2: f64, d: f64, rims: Rims, round: bool) -> Result<(), String> {
    let what = if round { "Le rayon du congé" } else { "Le chanfrein" };
    if !h.is_finite() || h <= 0.0 {
        return Err("Le cylindre doit avoir une hauteur positive.".into());
    }
    if !r1.is_finite() || !r2.is_finite() || r1 < 0.0 || r2 < 0.0 || r1 + r2 <= 0.0 {
        return Err("Le cylindre doit avoir un rayon positif.".into());
    }
    if !d.is_finite() || d <= 0.0 {
        return Err(format!("{what} doit être positif."));
    }
    let (bottom, top) = (rims != Rims::Top, rims != Rims::Bottom);
    if (bottom && r1 == 0.0) || (top && r2 == 0.0) {
        return Err("La pointe d'un cône n'a pas de bord à arrondir.".into());
    }
    // The angle of the profile's corner at the bottom rim, between the cap
    // and the side; the top one is its supplement.
    let side = ((r1 - r2).powi(2) + h * h).sqrt();
    let bottom_angle = ((r1 - r2) / side).acos();
    let top_angle = std::f64::consts::PI - bottom_angle;
    let at_bottom = if bottom { setback(d, bottom_angle, round) } else { 0.0 };
    let at_top = if top { setback(d, top_angle, round) } else { 0.0 };
    let tolerance = 1.0 + 1e-9;
    if at_bottom > r1 * tolerance || at_top > r2 * tolerance {
        return Err(format!("{what} est trop grand pour le rayon du cylindre."));
    }
    if at_bottom + at_top > side * tolerance {
        return Err(format!("{what} est trop grand pour la hauteur du cylindre."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// The model `body` makes, with fine curves and the library in reach:
    /// its volume and bounding box.
    fn render(body: &str) -> (f64, [f64; 3], [f64; 3]) {
        let src = format!("$fn = 96;\nuse <{LIBRARY}>\n{body};\n");
        let program = c2f_eval::Program::load(&src, std::path::Path::new("t.scad"), &c2f_eval::default_library_paths())
            .unwrap_or_else(|e| panic!("{}", e.render()));
        let output = c2f_eval::evaluate(&program);
        assert!(output.error.is_none(), "{body}: {:?}", output.error);
        assert!(output.warnings.is_empty(), "{body}: {:?}", output.warnings);
        let rendered = c2f_geom::render(&output.nodes);
        let (lo, hi) = rendered.bounds().expect("a solid");
        (rendered.stats.volume, lo, hi)
    }

    /// Renders the replacement and the original: same box (only the same
    /// height unless `same_width`), and the volume `expected` within 0.3 %.
    fn check(original: &str, replacement: &str, expected: f64) {
        check_box(original, replacement, expected, true);
    }

    fn check_box(original: &str, replacement: &str, expected: f64, same_width: bool) {
        let (volume, lo, hi) = render(replacement);
        let (_, olo, ohi) = render(original);
        for i in if same_width { 0..3 } else { 2..3 } {
            assert!((lo[i] - olo[i]).abs() < 1e-6 && (hi[i] - ohi[i]).abs() < 1e-6, "{replacement}: {lo:?} {hi:?} != {olo:?} {ohi:?}");
        }
        assert!((volume - expected).abs() / expected < 3e-3, "{replacement}: volume {volume}, expected {expected}");
    }

    #[test]
    fn fillet_calls_read_as_the_primitive() {
        assert_eq!(rounded_cube([30.0, 20.0, 10.0], false, 2.0, CubeEdges::All).unwrap(), "rounded_cube([30, 20, 10], fillet = 2)");
        assert_eq!(
            chamfered_cube([30.0, 20.0, 10.5], true, 1.0, CubeEdges::Vertical).unwrap(),
            "chamfered_cube([30, 20, 10.5], chamfer = 1, edges = \"vertical\", center = true)"
        );
        assert_eq!(rounded_cylinder(20.0, 10.0, 10.0, false, 2.0, Rims::Top).unwrap(), "rounded_cylinder(h = 20, r = 10, fillet = 2, rims = \"top\")");
        assert_eq!(chamfered_cylinder(20.0, 10.0, 6.0, true, 1.0, Rims::Both).unwrap(), "chamfered_cylinder(h = 20, r1 = 10, r2 = 6, chamfer = 1, center = true)");
    }

    #[test]
    fn fillet_rounded_cubes_keep_their_box_and_volume() {
        let (x, y, z, r): (f64, f64, f64, f64) = (30.0, 20.0, 10.0, 2.0);
        let (a, b, c) = (x - 2.0 * r, y - 2.0 * r, z - 2.0 * r);
        let box_volume = x * y * z;
        // The inner box grown by a ball (Steiner's formula).
        let all = a * b * c + 2.0 * r * (a * b + b * c + a * c) + PI * r * r * (a + b + c) + 4.0 / 3.0 * PI * r.powi(3);
        let vertical = z * (x * y - (4.0 - PI) * r * r);
        // Each edge takes (1 - π/4) r² along its length, and each corner
        // gives back (5/3 - π/2) r³ where two of them cross.
        let top = box_volume - (1.0 - PI / 4.0) * r * r * 2.0 * (x + y) + 4.0 * (5.0 / 3.0 - PI / 2.0) * r.powi(3);
        for center in [false, true] {
            let original = format!("cube([{x}, {y}, {z}], center = {center})");
            for (edges, expected) in [(CubeEdges::All, all), (CubeEdges::Vertical, vertical), (CubeEdges::Top, top), (CubeEdges::Bottom, top)] {
                check(&original, &rounded_cube([x, y, z], center, r, edges).unwrap(), expected);
            }
        }
    }

    #[test]
    fn fillet_chamfered_cubes_keep_their_box_and_volume() {
        let (x, y, z, d): (f64, f64, f64, f64) = (30.0, 20.0, 10.0, 2.0);
        let (a, b, c) = (x - 2.0 * d, y - 2.0 * d, z - 2.0 * d);
        // The inner box grown by an octahedron.
        let all = a * b * c + 2.0 * d * (a * b + b * c + a * c) + 2.0 * d * d * (a + b + c) + 4.0 / 3.0 * d.powi(3);
        let vertical = z * (x * y - 2.0 * d * d);
        let top = x * y * z - d * d * (x + y) + 4.0 / 3.0 * d.powi(3);
        for center in [false, true] {
            let original = format!("cube([{x}, {y}, {z}], center = {center})");
            for (edges, expected) in [(CubeEdges::All, all), (CubeEdges::Vertical, vertical), (CubeEdges::Top, top), (CubeEdges::Bottom, top)] {
                check(&original, &chamfered_cube([x, y, z], center, d, edges).unwrap(), expected);
            }
        }
    }

    /// Pappus: the solid a profile makes turning around Z is 2π times its
    /// area times the distance of its centroid from the axis. The profile
    /// is `cylinder()`'s trapezoid with its corners cut by `corner`
    /// (sampled finely here, independently of the library).
    fn turned_volume(h: f64, r1: f64, r2: f64, d: f64, rims: Rims, round: bool) -> f64 {
        let mut points = vec![[0.0, 0.0]];
        let mut corner = |a: [f64; 2], v: [f64; 2], b: [f64; 2], on: bool| {
            if !on {
                points.push(v);
                return;
            }
            let unit = |p: [f64; 2]| {
                let n = (p[0] * p[0] + p[1] * p[1]).sqrt();
                [p[0] / n, p[1] / n]
            };
            let u = unit([a[0] - v[0], a[1] - v[1]]);
            let w = unit([b[0] - v[0], b[1] - v[1]]);
            let angle = (u[0] * w[0] + u[1] * w[1]).acos();
            let t = setback(d, angle, round);
            let p = [v[0] + u[0] * t, v[1] + u[1] * t];
            let q = [v[0] + w[0] * t, v[1] + w[1] * t];
            if !round {
                points.extend([p, q]);
                return;
            }
            let m = unit([u[0] + w[0], u[1] + w[1]]);
            let k = d / (angle / 2.0).sin();
            let c = [v[0] + m[0] * k, v[1] + m[1] * k];
            let from = (p[1] - c[1]).atan2(p[0] - c[0]);
            let turn = PI - angle;
            for i in 0..=10_000 {
                let phi = from + turn * i as f64 / 10_000.0;
                points.push([c[0] + d * phi.cos(), c[1] + d * phi.sin()]);
            }
        };
        corner([0.0, 0.0], [r1, 0.0], [r2, h], rims != Rims::Top);
        corner([r1, 0.0], [r2, h], [0.0, h], rims != Rims::Bottom);
        points.push([0.0, h]);
        // Σ over edges of the integral of x over the area (shoelace).
        let mut moment = 0.0;
        for i in 0..points.len() {
            let ([x0, y0], [x1, y1]) = (points[i], points[(i + 1) % points.len()]);
            let cross = x0 * y1 - x1 * y0;
            moment += (x0 + x1) * cross / 6.0;
        }
        2.0 * PI * moment
    }

    #[test]
    fn fillet_cylinders_keep_their_box_and_volume() {
        for (r1, r2) in [(10.0, 10.0), (10.0, 6.0), (5.0, 9.0)] {
            for center in [false, true] {
                let original = format!("cylinder(h = 20, r1 = {r1}, r2 = {r2}, center = {center})");
                for rims in [Rims::Top, Rims::Bottom, Rims::Both] {
                    // A cone's widest circle is a rim: rounding that rim
                    // makes it narrower.
                    let widest = if r1 > r2 { Rims::Bottom } else { Rims::Top };
                    let same_width = r1 == r2 || (rims != widest && rims != Rims::Both);
                    let rounded = rounded_cylinder(20.0, r1, r2, center, 2.0, rims).unwrap();
                    check_box(&original, &rounded, turned_volume(20.0, r1, r2, 2.0, rims, true), same_width);
                    let chamfered = chamfered_cylinder(20.0, r1, r2, center, 1.5, rims).unwrap();
                    check_box(&original, &chamfered, turned_volume(20.0, r1, r2, 1.5, rims, false), same_width);
                }
            }
        }
    }

    #[test]
    fn fillet_straight_cylinder_matches_the_closed_form() {
        // A rim takes the corner of the profile outside a quarter circle,
        // (1 - π/4) r² whose centroid is (10 - 3π) / (12 - 3π) r in.
        let (h, big, r) = (20.0, 10.0, 2.0);
        let k = (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
        let rim = 2.0 * PI * (big - k * r) * (1.0 - PI / 4.0) * r * r;
        assert!((turned_volume(h, big, big, r, Rims::Both, true) - (PI * big * big * h - 2.0 * rim)).abs() < 1e-3);
    }

    #[test]
    fn fillet_too_large_is_refused() {
        assert!(rounded_cube([30.0, 20.0, 10.0], false, 5.0, CubeEdges::All).is_ok());
        assert_eq!(
            rounded_cube([30.0, 20.0, 10.0], false, 5.5, CubeEdges::All).unwrap_err(),
            "Le rayon du congé ne peut dépasser la moitié de la plus petite dimension (5)."
        );
        assert!(rounded_cube([30.0, 20.0, 10.0], false, 10.0, CubeEdges::Vertical).is_ok());
        assert!(rounded_cube([30.0, 20.0, 4.0], false, 4.0, CubeEdges::Top).is_ok());
        assert!(rounded_cube([30.0, 20.0, 4.0], false, 4.5, CubeEdges::Top).is_err());
        assert!(chamfered_cube([30.0, 20.0, 10.0], false, 0.0, CubeEdges::All).is_err());
        assert!(chamfered_cube([30.0, -1.0, 10.0], false, 1.0, CubeEdges::All).is_err());
        assert!(rounded_cylinder(20.0, 10.0, 10.0, false, 10.0, Rims::Top).is_ok());
        assert!(rounded_cylinder(20.0, 10.0, 10.0, false, 10.5, Rims::Top).is_err());
        assert!(rounded_cylinder(4.0, 10.0, 10.0, false, 2.5, Rims::Both).is_err());
        assert!(rounded_cylinder(4.0, 10.0, 10.0, false, 2.5, Rims::Top).is_ok());
        assert_eq!(chamfered_cylinder(20.0, 10.0, 0.0, false, 1.0, Rims::Both).unwrap_err(), "La pointe d'un cône n'a pas de bord à arrondir.");
        assert!(chamfered_cylinder(20.0, 10.0, 0.0, false, 1.0, Rims::Bottom).is_ok());
    }

    #[test]
    fn fillet_a_cone_point_turns_without_a_rim() {
        let call = chamfered_cylinder(20.0, 10.0, 0.0, false, 1.0, Rims::Bottom).unwrap();
        check_box("cylinder(h = 20, r1 = 10, r2 = 0)", &call, turned_volume(20.0, 10.0, 0.0, 1.0, Rims::Bottom, false), false);
    }

    #[test]
    fn fillet_apply_adds_the_library_once() {
        let src = "difference() {\n    cube([30, 20, 10]);\n}\n";
        let start = src.find("cube").unwrap();
        let end = src.find(";").unwrap();
        let call = rounded_cube([30.0, 20.0, 10.0], false, 2.0, CubeEdges::All).unwrap();
        let once = apply(src, start..end, &call);
        assert_eq!(once, "use <shapes/rounded.c2f>\ndifference() {\n    rounded_cube([30, 20, 10], fillet = 2);\n}\n");
        let start = once.find("rounded_cube").unwrap();
        let end = once[start..].find(';').unwrap() + start;
        let twice = apply(&once, start..end, "rounded_cube([30, 20, 10], fillet = 3)");
        assert_eq!(twice.matches("use <").count(), 1);
        assert!(twice.contains("fillet = 3);"));
    }
}
