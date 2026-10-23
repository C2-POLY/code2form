//! A cut through a solid: its outline in a plane across one axis, at its
//! real size. What an agent measures a mesh with before drawing it again
//! as a design: a cut through the axis of a turned part is its profile.

use manifold_rust::linalg::{Mat3x4, Vec3};

use crate::build::Model;
use crate::mesh::Rendered;

/// The axis a cut is across, and the plane's own two axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// The plane x = at, read as (y, z).
    X,
    /// The plane y = at, read as (x, z).
    Y,
    /// The plane z = at, read as (x, y).
    Z,
}

impl Axis {
    pub fn named(name: &str) -> Option<Axis> {
        match name {
            "x" | "X" => Some(Axis::X),
            "y" | "Y" => Some(Axis::Y),
            "z" | "Z" => Some(Axis::Z),
            _ => None,
        }
    }

    /// The names of the plane's two axes.
    pub fn plane(self) -> [&'static str; 2] {
        match self {
            Axis::X => ["y", "z"],
            Axis::Y => ["x", "z"],
            Axis::Z => ["x", "y"],
        }
    }
}

/// The outline a cut leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// Each contour, its points in the plane's two axes: outer ones
    /// counter-clockwise, holes clockwise.
    pub outlines: Vec<Vec<[f64; 2]>>,
    pub area: f64,
    /// Smallest and largest point, in the plane's two axes.
    pub bounds: Option<([f64; 2], [f64; 2])>,
}

impl Rendered {
    /// The solid cut across `axis` at `at`. `None` for a 2D or empty model.
    pub fn section(&self, axis: Axis, at: f64) -> Option<Section> {
        let Model::Solid(manifold) = &self.model else { return None };
        // A cyclic turn of the axes (no mirror), so that the cut is at z.
        let col = |x: f64, y: f64, z: f64| Vec3::new(x, y, z);
        let turn = match axis {
            // (x, y, z) -> (y, z, x)
            Axis::X => Mat3x4::from_cols(col(0.0, 0.0, 1.0), col(1.0, 0.0, 0.0), col(0.0, 1.0, 0.0), col(0.0, 0.0, 0.0)),
            // (x, y, z) -> (z, x, y): read back as (x, z)
            Axis::Y => Mat3x4::from_cols(col(0.0, 1.0, 0.0), col(0.0, 0.0, 1.0), col(1.0, 0.0, 0.0), col(0.0, 0.0, 0.0)),
            Axis::Z => Mat3x4::from_cols(col(1.0, 0.0, 0.0), col(0.0, 1.0, 0.0), col(0.0, 0.0, 1.0), col(0.0, 0.0, 0.0)),
        };
        let cut = manifold.transform(&turn).slice(at);
        let swap = axis == Axis::Y;
        let mut outlines: Vec<Vec<[f64; 2]>> = cut
            .to_polygons()
            .into_iter()
            .filter(|c| c.len() >= 3)
            .map(|c| c.into_iter().map(|p| if swap { [p.y + 0.0, p.x + 0.0] } else { [p.x + 0.0, p.y + 0.0] }).collect())
            .collect();
        // Swapping the axes runs every contour the other way: back.
        if swap {
            outlines.iter_mut().for_each(|c| c.reverse());
        }
        let bounds = outlines.iter().flatten().fold(None, |b: Option<([f64; 2], [f64; 2])>, p| {
            let (lo, hi) = b.unwrap_or((*p, *p));
            Some(([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]))
        });
        Some(Section { area: cut.area(), outlines, bounds })
    }
}

impl Section {
    /// The outline with the points that lie within `tolerance` of the
    /// line through their neighbours left out (Douglas-Peucker), as JSON:
    /// what an agent reads, short enough to read whole.
    pub fn json(&self, axis: Axis, at: f64, tolerance: f64) -> serde_json::Value {
        let round = |v: f64| (v * 1000.0).round() / 1000.0 + 0.0;
        let outlines: Vec<Vec<[f64; 2]>> = self
            .outlines
            .iter()
            .map(|c| simplify(c, tolerance).into_iter().map(|[u, v]| [round(u), round(v)]).collect())
            .collect();
        let [u, v] = axis.plane();
        serde_json::json!({
            "plane": format!("{} = {at}", match axis { Axis::X => "x", Axis::Y => "y", Axis::Z => "z" }),
            "axes": [u, v],
            "area": round(self.area),
            "bounds": self.bounds.map(|(lo, hi)| serde_json::json!({ "min": lo.map(round), "max": hi.map(round) })),
            "outlines": outlines,
        })
    }
}

/// A closed contour without the points within `tolerance` of the line
/// through the points kept around them.
fn simplify(contour: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    if contour.len() <= 4 || tolerance <= 0.0 {
        return contour.to_vec();
    }
    // Split at the point farthest from the first, then simplify each half.
    let far = (1..contour.len())
        .max_by(|&a, &b| {
            let d = |i: usize| (contour[i][0] - contour[0][0]).hypot(contour[i][1] - contour[0][1]);
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(contour.len() / 2);
    let mut keep = vec![false; contour.len() + 1];
    keep[0] = true;
    keep[far] = true;
    keep[contour.len()] = true;
    let at = |i: usize| contour[i % contour.len()];
    let mut pending = vec![(0, far), (far, contour.len())];
    while let Some((a, b)) = pending.pop() {
        let (p, q) = (at(a), at(b));
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let length = dx.hypot(dy);
        let distance = |r: [f64; 2]| {
            if length == 0.0 { (r[0] - p[0]).hypot(r[1] - p[1]) } else { ((r[0] - p[0]) * dy - (r[1] - p[1]) * dx).abs() / length }
        };
        if let Some((i, d)) = (a + 1..b).map(|i| (i, distance(at(i)))).max_by(|x, y| x.1.total_cmp(&y.1)) {
            if d > tolerance {
                keep[i] = true;
                pending.push((a, i));
                pending.push((i, b));
            }
        }
    }
    (0..contour.len()).filter(|&i| keep[i]).map(|i| contour[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(src: &str, axis: Axis, at: f64) -> Section {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).unwrap();
        crate::render(&c2f_eval::evaluate(&program).nodes).section(axis, at).unwrap()
    }

    #[test]
    fn points_on_a_line_are_left_out() {
        let square = [[0.0, 0.0], [5.0, 0.0], [10.0, 0.001], [10.0, 10.0], [5.0, 10.0], [0.0, 10.0]];
        assert_eq!(simplify(&square, 0.01), [[0.0, 0.0], [10.0, 0.001], [10.0, 10.0], [0.0, 10.0]]);
        assert_eq!(simplify(&square, 0.0).len(), 6);
    }

    #[test]
    fn across_each_axis_in_the_planes_own_axes() {
        let src = "cube([10, 20, 30]);";
        assert_eq!(cut(src, Axis::Z, 5.0).bounds, Some(([0.0, 0.0], [10.0, 20.0])));
        assert_eq!(cut(src, Axis::Y, 5.0).bounds, Some(([0.0, 0.0], [10.0, 30.0])));
        assert_eq!(cut(src, Axis::X, 5.0).bounds, Some(([0.0, 0.0], [20.0, 30.0])));
        assert!((cut(src, Axis::Y, 5.0).area - 300.0).abs() < 1e-9);
        // Outside the solid: nothing.
        assert!(cut(src, Axis::Z, 40.0).outlines.is_empty());
    }

    #[test]
    fn through_the_axis_of_a_turned_part_is_its_profile() {
        // A tube: two rectangles either side of the axis, a hole between.
        let s = cut("difference() { cylinder(r = 10, h = 5, $fn = 64); translate([0, 0, -1]) cylinder(r = 6, h = 7, $fn = 64); }", Axis::Y, 0.0);
        assert_eq!(s.outlines.len(), 2);
        let (lo, hi) = s.bounds.unwrap();
        assert!((lo[0] + 10.0).abs() < 1e-6 && (hi[0] - 10.0).abs() < 1e-6 && (hi[1] - 5.0).abs() < 1e-9);
        assert!((s.area - 2.0 * 4.0 * 5.0).abs() < 1e-6);
        // Outer contours counter-clockwise.
        for c in &s.outlines {
            let a: f64 = (0..c.len()).map(|i| { let (p, q) = (c[i], c[(i + 1) % c.len()]); p[0] * q[1] - q[0] * p[1] }).sum();
            assert!(a > 0.0);
        }
    }
}
