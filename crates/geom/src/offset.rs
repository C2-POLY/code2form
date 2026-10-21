//! `offset(r = …)`: round offsets as the reference implementation (2021.01)
//! computes them with Clipper 1's `ClipperOffset`, so that arc points land
//! where the reference puts them.
//!
//! What the reference does, and what is reproduced here:
//! - Points go to Clipper's integer grid, scaled by 65536 and truncated.
//! - The arc tolerance comes from the fragment count `n` for `|r|`
//!   (`$fn`, `$fa`, `$fs`): `|r| * (1 - cos(180° / n))`, capped by Clipper
//!   at `|r| / 4`, which gives `n` steps per full turn (about 4.35 when the
//!   cap applies, for `n` of 3 or 4).
//! - Each convex corner turning by `a` gets `round(n * a / 2π)` segments
//!   (at least one): steps of exactly `2π / n` from the incoming edge's
//!   normal, then a last one to the outgoing edge's normal with whatever
//!   is left of the turn. Clipper2, which Manifold's own offset uses,
//!   takes `ceil(n * a / 2π)` segments instead, so it puts an
//!   extra point on every corner whose fraction of a step is under one
//!   half (a right angle with n = 5, the fewest `$fa`/`$fs` give: two
//!   segments instead of one), and it also skips the `|r| / 4` cap.
//! - Concave corners get the two offset points with the corner itself
//!   between them; the offset contours are then united with the positive
//!   fill rule, the points it adds rounded to the nearest grid point
//!   (Clipper2 truncates them).

use clipper2_rust::{FillRule, Path64, Paths64, Point64, union_subjects_64};
use manifold_rust::cross_section::CrossSection;
use manifold_rust::linalg::Vec2;

use std::f64::consts::PI;

/// The reference's fixed-point scale for Clipper.
const CLIPPER_SCALE: f64 = 65536.0;
/// Clipper 1's default (and largest) arc tolerance, as a fraction of `|delta|`.
const DEF_ARC_TOLERANCE: f64 = 0.25;
/// How much finer than Clipper's grid the final union computes.
const FINE: i64 = 256;

/// `offset(r)` of `section` with `n` fragments per full circle. An
/// infinite or NaN `r` gives nothing, as in the reference.
pub(crate) fn round(section: &CrossSection, r: f64, n: u32) -> CrossSection {
    if !r.is_finite() {
        return CrossSection::default();
    }
    let paths: Paths64 = section.to_polygons().iter().map(|contour| contour.iter().map(|p| to_grid(*p)).collect()).collect();
    let n = f64::from(n);
    let arc_tolerance = r.abs() * (1.0 - cos_degrees(180.0 / n));
    let offset = ClipperOffset::new(&paths, arc_tolerance * CLIPPER_SCALE);
    let united = offset.execute(r * CLIPPER_SCALE);
    let contours: Vec<Vec<Vec2>> = united
        .iter()
        .map(|path| path.iter().map(|p| Vec2::new(p.x as f64 / CLIPPER_SCALE, p.y as f64 / CLIPPER_SCALE)).collect())
        .collect();
    CrossSection::new(contours)
}

/// A point on Clipper's grid. The reference truncates; points that are
/// already on the grid (up to the cross-section's 1e-8 precision) are
/// rounded to it instead, so that truncation does not move them a step.
fn to_grid(p: Vec2) -> Point64 {
    let snap = |v: f64| {
        let scaled = v * CLIPPER_SCALE;
        let nearest = scaled.round();
        (if (scaled - nearest).abs() < 1e-3 { nearest } else { scaled.trunc() }) as i64
    };
    Point64::new(snap(p.x), snap(p.y))
}

/// The reference's `cos_degrees`: `sin_degrees(x + 90)`, whose sines of
/// 30°, 45° and 60° are exact constants.
fn cos_degrees(x: f64) -> f64 {
    sin_degrees(x + 90.0)
}

fn sin_degrees(x: f64) -> f64 {
    let mut x = if (0.0..360.0).contains(&x) { x } else { x.rem_euclid(360.0) };
    let oppose = x >= 180.0;
    if oppose {
        x -= 180.0;
    }
    if x > 90.0 {
        x = 180.0 - x;
    }
    let s = if x < 45.0 {
        if x == 30.0 { 0.5 } else { (x * PI / 180.0).sin() }
    } else if x == 45.0 {
        std::f64::consts::FRAC_1_SQRT_2
    } else if x == 60.0 {
        0.866_025_403_784_438_6
    } else {
        ((90.0 - x) * PI / 180.0).cos()
    };
    if oppose { -s } else { s }
}

/// Clipper 1's `Round`: half away from zero.
fn round_half_away(v: f64) -> i64 {
    if v < 0.0 { (v - 0.5) as i64 } else { (v + 0.5) as i64 }
}

/// Clipper 1's `ClipperOffset` for closed polygons with round joins.
struct ClipperOffset {
    contours: Vec<Vec<(i64, i64)>>,
    arc_tolerance: f64,
}

impl ClipperOffset {
    /// `AddPaths` then `FixOrientations`: duplicate points are dropped,
    /// contours of fewer than three points too, and every contour is
    /// reversed when the one with Clipper's "lowest" point (largest y, then
    /// smallest x: the topmost one here, y being up) is clockwise.
    fn new(paths: &Paths64, arc_tolerance: f64) -> Self {
        let mut contours = Vec::new();
        let mut lowest: Option<(usize, usize)> = None;
        for path in paths {
            let pts: Vec<(i64, i64)> = path.iter().map(|p| (p.x, p.y)).collect();
            let Some(&first) = pts.first() else { continue };
            let mut high = pts.len() - 1;
            while high > 0 && pts[high] == first {
                high -= 1;
            }
            let mut contour = vec![first];
            let mut k = 0;
            for &p in &pts[1..=high] {
                if *contour.last().unwrap() != p {
                    contour.push(p);
                    let low = contour[k];
                    if p.1 > low.1 || (p.1 == low.1 && p.0 < low.0) {
                        k = contour.len() - 1;
                    }
                }
            }
            if contour.len() < 3 {
                continue;
            }
            let candidate = contour[k];
            contours.push(contour);
            let index = contours.len() - 1;
            lowest = match lowest {
                Some((i, j)) => {
                    let ip = contours[i][j];
                    if candidate.1 > ip.1 || (candidate.1 == ip.1 && candidate.0 < ip.0) { Some((index, k)) } else { Some((i, j)) }
                }
                None => Some((index, k)),
            };
        }
        if lowest.is_some_and(|(i, _)| area(&contours[i]) < 0.0) {
            for contour in &mut contours {
                contour.reverse();
            }
        }
        ClipperOffset { contours, arc_tolerance }
    }

    /// `Execute(solution, delta)`: offsets every contour, then unites the
    /// results with the positive fill rule (what Clipper 1's handling of
    /// negative offsets amounts to as well).
    fn execute(&self, delta: f64) -> Paths64 {
        if delta.abs() < 1e-20 {
            let paths: Paths64 = self.contours.iter().map(|c| c.iter().map(|&(x, y)| Point64::new(x, y)).collect()).collect();
            return unite(&paths);
        }
        let y = if self.arc_tolerance <= 0.0 {
            DEF_ARC_TOLERANCE
        } else if self.arc_tolerance > delta.abs() * DEF_ARC_TOLERANCE {
            delta.abs() * DEF_ARC_TOLERANCE
        } else {
            self.arc_tolerance
        };
        let mut steps = PI / (1.0 - y / delta.abs()).acos();
        if steps > delta.abs() * PI {
            steps = delta.abs() * PI;
        }
        let mut join = Join {
            delta,
            sin: (2.0 * PI / steps).sin(),
            cos: (2.0 * PI / steps).cos(),
            steps_per_rad: steps / (2.0 * PI),
            sin_a: 0.0,
        };
        if delta < 0.0 {
            join.sin = -join.sin;
        }
        let mut offsets: Paths64 = Vec::with_capacity(self.contours.len());
        for src in &self.contours {
            let len = src.len();
            let normals: Vec<(f64, f64)> = (0..len).map(|j| unit_normal(src[j], src[(j + 1) % len])).collect();
            let mut dest = Path64::new();
            let mut k = len - 1;
            for j in 0..len {
                join.offset_point(src, &normals, j, &mut k, &mut dest);
            }
            offsets.push(dest);
        }
        unite(&offsets)
    }
}

/// Clipper 1's positive-fill union. Clipper 1 rounds the intersections
/// it adds to the nearest grid point, where Clipper2 truncates them: the
/// union runs on a grid `FINE` times finer, and its points are rounded
/// back.
fn unite(paths: &Paths64) -> Paths64 {
    let fine: Paths64 = paths.iter().map(|path| path.iter().map(|p| Point64::new(p.x.saturating_mul(FINE), p.y.saturating_mul(FINE))).collect()).collect();
    let back = |v: i64| round_half_away(v as f64 / FINE as f64);
    union_subjects_64(&fine, FillRule::Positive)
        .iter()
        .map(|path| {
            let mut out: Path64 = path.iter().map(|p| Point64::new(back(p.x), back(p.y))).collect();
            out.dedup();
            out
        })
        .collect()
}

/// The state of one `DoOffset` pass.
struct Join {
    delta: f64,
    sin: f64,
    cos: f64,
    steps_per_rad: f64,
    sin_a: f64,
}

impl Join {
    fn at(&self, p: (i64, i64), n: (f64, f64)) -> Point64 {
        Point64::new(round_half_away(p.0 as f64 + n.0 * self.delta), round_half_away(p.1 as f64 + n.1 * self.delta))
    }

    /// Clipper 1's `OffsetPoint` with a round join.
    fn offset_point(&mut self, src: &[(i64, i64)], normals: &[(f64, f64)], j: usize, k: &mut usize, dest: &mut Path64) {
        let (nk, nj) = (normals[*k], normals[j]);
        self.sin_a = nk.0 * nj.1 - nj.0 * nk.1;
        if (self.sin_a * self.delta).abs() < 1.0 {
            let cos_a = nk.0 * nj.0 + nj.1 * nk.1;
            if cos_a > 0.0 {
                // Nearly straight on: one point, and k stays.
                dest.push(self.at(src[j], nk));
                return;
            }
        } else {
            self.sin_a = self.sin_a.clamp(-1.0, 1.0);
        }
        if self.sin_a * self.delta < 0.0 {
            dest.push(self.at(src[j], nk));
            dest.push(Point64::new(src[j].0, src[j].1));
            dest.push(self.at(src[j], nj));
        } else {
            self.round(src[j], nk, nj, dest);
        }
        *k = j;
    }

    /// Clipper 1's `DoRound`.
    fn round(&self, p: (i64, i64), nk: (f64, f64), nj: (f64, f64), dest: &mut Path64) {
        let a = self.sin_a.atan2(nk.0 * nj.0 + nk.1 * nj.1);
        let steps = (round_half_away(self.steps_per_rad * a.abs()) as i32).max(1);
        let (mut x, mut y) = nk;
        for _ in 0..steps {
            dest.push(Point64::new(round_half_away(p.0 as f64 + x * self.delta), round_half_away(p.1 as f64 + y * self.delta)));
            let x2 = x;
            x = x * self.cos - self.sin * y;
            y = x2 * self.sin + y * self.cos;
        }
        dest.push(self.at(p, nj));
    }
}

/// Clipper 1's `GetUnitNormal`: the edge's direction turned clockwise.
fn unit_normal(a: (i64, i64), b: (i64, i64)) -> (f64, f64) {
    if a == b {
        return (0.0, 0.0);
    }
    let dx = (b.0 - a.0) as f64;
    let dy = (b.1 - a.1) as f64;
    let f = 1.0 / (dx * dx + dy * dy).sqrt();
    (dy * f, -dx * f)
}

/// Twice the signed area, positive counter-clockwise (y up).
fn area(contour: &[(i64, i64)]) -> f64 {
    let n = contour.len();
    (0..n)
        .map(|i| {
            let (a, b) = (contour[i], contour[(i + 1) % n]);
            (a.0 as f64 + b.0 as f64) * (b.1 as f64 - a.1 as f64)
        })
        .sum()
}
