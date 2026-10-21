//! Glyph outlines as FreeType 2.10 hands them to the reference
//! implementation: font units scaled to integer 26.6 coordinates with
//! FreeType's fixed-point rounding, then decomposed into lines and curves
//! by `FT_Outline_Decompose`.
//!
//! TrueType (`glyf`) glyphs are read here point by point, since where
//! FreeType rounds (each point, implied on-curve midpoints computed from
//! rounded neighbors, composite offsets) shows in the output. CFF glyphs
//! go through ttf-parser and are rounded once.

use ttf_parser::{Face, GlyphId, OutlineBuilder, Tag};

/// A 26.6 fixed-point point.
pub type Pt = (i64, i64);

/// `FT_Outline_Decompose`'s callbacks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(Pt),
    Line(Pt),
    Conic(Pt, Pt),
    Cubic(Pt, Pt, Pt),
}

#[derive(Default)]
pub struct Outline {
    pub segs: Vec<Seg>,
    /// The control box (all points, off-curve ones included):
    /// `[x_min, y_min, x_max, y_max]`, zero for an empty glyph.
    pub cbox: [i64; 4],
}

/// `FT_MulFix`: `a * b / 65536`, rounded half away from zero.
pub fn mul_fix(a: i64, b: i64) -> i64 {
    let c = (a.abs() * b.abs() + 0x8000) >> 16;
    if (a < 0) != (b < 0) { -c } else { c }
}

/// `FT_DivFix`: `a * 65536 / b`, rounded half away from zero.
pub fn div_fix(a: i64, b: i64) -> i64 {
    if b == 0 {
        return 0x7FFF_FFFF;
    }
    let q = ((a.abs() << 16) + (b.abs() >> 1)) / b.abs();
    if (a < 0) != (b < 0) { -q } else { q }
}

/// `FT_MulDiv`: `a * b / c`, rounded half away from zero.
pub fn mul_div(a: i64, b: i64, c: i64) -> i64 {
    if c == 0 {
        return 0x7FFF_FFFF;
    }
    let d = (a.abs() * b.abs() + (c.abs() >> 1)) / c.abs();
    if ((a < 0) != (b < 0)) != (c < 0) { -d } else { d }
}

pub fn pix_floor(x: i64) -> i64 {
    x & !63
}

pub fn pix_ceil(x: i64) -> i64 {
    (x + 63) & !63
}

/// Points, on-curve flags and contour ends: an `FT_Outline`.
#[derive(Default)]
struct Points {
    pts: Vec<Pt>,
    on: Vec<bool>,
    /// One past the last point of each contour.
    ends: Vec<usize>,
}

// Composite glyph flags.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

/// Reads glyph outlines of one face.
pub struct Loader<'a> {
    face: &'a Face<'a>,
    glyf: Option<&'a [u8]>,
    loca: Option<&'a [u8]>,
    long_loca: bool,
}

impl<'a> Loader<'a> {
    pub fn new(face: &'a Face<'a>) -> Self {
        let raw = face.raw_face();
        let glyf = raw.table(Tag::from_bytes(b"glyf"));
        let loca = raw.table(Tag::from_bytes(b"loca"));
        let long_loca = raw.table(Tag::from_bytes(b"head")).and_then(|h| be16(h, 50)).is_some_and(|f| f != 0);
        Loader { face, glyf: glyf.filter(|_| loca.is_some()), loca, long_loca }
    }

    pub fn is_truetype(&self) -> bool {
        self.glyf.is_some()
    }

    /// The outline of `glyph` scaled by `scale` (16.16 26.6-units per font
    /// unit).
    ///
    /// The reference loads glyphs with hinting (`FT_LOAD_DEFAULT`), which
    /// picks the integer-ppem scale; but the control programs of Liberation
    /// (and most Microsoft-style) fonts switch instructions off at
    /// 2170 ppem, and FreeType then loads as `FT_LOAD_NO_HINTING`: no
    /// bytecode, and composite offsets and the origin are not rounded to
    /// the pixel grid.
    pub fn load(&self, glyph: u16, scale: i64) -> Outline {
        let points = if self.is_truetype() {
            let mut points = Points::default();
            if self.load_glyf(glyph, scale, 0, &mut points).is_none() {
                points = Points::default();
            }
            // FreeType moves the glyph so that its origin (phantom point
            // pp1, at x_min - lsb) is at 0.
            if let (Some(lsb), Some(x_min)) = (
                self.face.glyph_hor_side_bearing(GlyphId(glyph)),
                self.glyph_data(glyph).and_then(|d| be16(d, 2)).map(|v| v as i16),
            ) {
                let pp1 = mul_fix(i64::from(x_min) - i64::from(lsb), scale);
                if pp1 != 0 {
                    points.pts.iter_mut().for_each(|p| p.0 -= pp1);
                }
            }
            points
        } else {
            return self.load_cff(glyph, scale);
        };
        let mut cbox = [0; 4];
        if let Some(&first) = points.pts.first() {
            cbox = [first.0, first.1, first.0, first.1];
            for &(x, y) in &points.pts {
                cbox = [cbox[0].min(x), cbox[1].min(y), cbox[2].max(x), cbox[3].max(y)];
            }
        }
        Outline { segs: decompose(&points), cbox }
    }

    fn glyph_data(&self, glyph: u16) -> Option<&'a [u8]> {
        let (glyf, loca) = (self.glyf?, self.loca?);
        let i = glyph as usize;
        let (start, end) = if self.long_loca {
            (be32(loca, 4 * i)? as usize, be32(loca, 4 * i + 4)? as usize)
        } else {
            (2 * be16(loca, 2 * i)? as usize, 2 * be16(loca, 2 * i + 2)? as usize)
        };
        if end <= start {
            return Some(&[]);
        }
        glyf.get(start..end)
    }

    /// Appends the scaled points of `glyph` (FreeType's
    /// `load_truetype_glyph`).
    fn load_glyf(&self, glyph: u16, scale: i64, depth: u32, out: &mut Points) -> Option<()> {
        if depth > 16 {
            return None;
        }
        let data = self.glyph_data(glyph)?;
        if data.is_empty() {
            return Some(());
        }
        let contours = be16(data, 0)? as i16;
        if contours >= 0 {
            return simple_glyph(data, contours as usize, scale, out);
        }
        let mut at = 10;
        // Point matching counts the composite's points from here.
        let start = out.pts.len();
        loop {
            let flags = be16(data, at)?;
            let component = be16(data, at + 2)?;
            at += 4;
            let (arg1, arg2) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                at += 4;
                if flags & ARGS_ARE_XY_VALUES != 0 {
                    (i64::from(be16(data, at - 4)? as i16), i64::from(be16(data, at - 2)? as i16))
                } else {
                    (i64::from(be16(data, at - 4)?), i64::from(be16(data, at - 2)?))
                }
            } else {
                at += 2;
                let (a, b) = (*data.get(at - 2)?, *data.get(at - 1)?);
                if flags & ARGS_ARE_XY_VALUES != 0 {
                    (i64::from(a as i8), i64::from(b as i8))
                } else {
                    (i64::from(a), i64::from(b))
                }
            };
            // 16.16 matrix [xx xy; yx yy] from F2Dot14 values.
            let f2dot14 = |at: usize| be16(data, at).map(|v| i64::from(v as i16) * 4);
            let mut matrix = None;
            if flags & WE_HAVE_A_SCALE != 0 {
                let s = f2dot14(at)?;
                matrix = Some([s, 0, 0, s]);
                at += 2;
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                matrix = Some([f2dot14(at)?, 0, 0, f2dot14(at + 2)?]);
                at += 4;
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                // Stored as xx, yx, xy, yy.
                matrix = Some([f2dot14(at)?, f2dot14(at + 4)?, f2dot14(at + 2)?, f2dot14(at + 6)?]);
                at += 8;
            }
            let base = out.pts.len();
            self.load_glyf(component, scale, depth + 1, out)?;
            if let Some([xx, xy, yx, yy]) = matrix {
                for p in &mut out.pts[base..] {
                    *p = (mul_fix(p.0, xx) + mul_fix(p.1, xy), mul_fix(p.0, yx) + mul_fix(p.1, yy));
                }
            }
            let (dx, dy) = if flags & ARGS_ARE_XY_VALUES == 0 {
                // Point matching: the component's point `arg2` onto the
                // earlier components' point `arg1`.
                let k = start + arg1 as usize;
                let l = base + arg2 as usize;
                if k >= base {
                    return None;
                }
                let (Some(p1), Some(p2)) = (out.pts.get(k), out.pts.get(l)) else { return None };
                (p1.0 - p2.0, p1.1 - p2.1)
            } else {
                let (mut x, mut y) = (arg1, arg2);
                if let Some([xx, xy, yx, yy]) = matrix {
                    if flags & SCALED_COMPONENT_OFFSET != 0 {
                        let hypot = |a: i64, b: i64| ((a as f64).hypot(b as f64)).round() as i64;
                        x = mul_fix(x, hypot(xx, xy));
                        y = mul_fix(y, hypot(yy, yx));
                    }
                }
                // ROUND_XY_TO_GRID only applies when hinting.
                (mul_fix(x, scale), mul_fix(y, scale))
            };
            if dx != 0 || dy != 0 {
                for p in &mut out.pts[base..] {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
            if flags & MORE_COMPONENTS == 0 {
                return Some(());
            }
        }
    }

    /// CFF outlines through ttf-parser, rounded to 26.6.
    fn load_cff(&self, glyph: u16, scale: i64) -> Outline {
        struct Builder {
            k: f64,
            segs: Vec<Seg>,
            start: Pt,
            points: Vec<Pt>,
        }
        impl Builder {
            fn pt(&mut self, x: f32, y: f32) -> Pt {
                let p = ((f64::from(x) * self.k).round() as i64, (f64::from(y) * self.k).round() as i64);
                self.points.push(p);
                p
            }
        }
        impl OutlineBuilder for Builder {
            fn move_to(&mut self, x: f32, y: f32) {
                let p = self.pt(x, y);
                self.start = p;
                self.segs.push(Seg::Move(p));
            }
            fn line_to(&mut self, x: f32, y: f32) {
                let p = self.pt(x, y);
                self.segs.push(Seg::Line(p));
            }
            fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
                let c = self.pt(x1, y1);
                let p = self.pt(x, y);
                self.segs.push(Seg::Conic(c, p));
            }
            fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
                let c1 = self.pt(x1, y1);
                let c2 = self.pt(x2, y2);
                let p = self.pt(x, y);
                self.segs.push(Seg::Cubic(c1, c2, p));
            }
            fn close(&mut self) {
                self.segs.push(Seg::Line(self.start));
            }
        }
        let mut builder = Builder { k: scale as f64 / 65536.0, segs: Vec::new(), start: (0, 0), points: Vec::new() };
        self.face.outline_glyph(GlyphId(glyph), &mut builder);
        let mut cbox = [0; 4];
        if let Some(&first) = builder.points.first() {
            cbox = [first.0, first.1, first.0, first.1];
            for &(x, y) in &builder.points {
                cbox = [cbox[0].min(x), cbox[1].min(y), cbox[2].max(x), cbox[3].max(y)];
            }
        }
        Outline { segs: builder.segs, cbox }
    }
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A simple glyph's points, each scaled with `FT_MulFix`.
fn simple_glyph(data: &[u8], contours: usize, scale: i64, out: &mut Points) -> Option<()> {
    let mut ends = Vec::with_capacity(contours);
    for i in 0..contours {
        ends.push(be16(data, 10 + 2 * i)? as usize + 1);
    }
    let count = ends.last().copied().unwrap_or(0);
    if ends.windows(2).any(|w| w[1] < w[0]) {
        return None;
    }
    let instructions = be16(data, 10 + 2 * contours)? as usize;
    let mut at = 12 + 2 * contours + instructions;
    let mut flags = Vec::with_capacity(count);
    while flags.len() < count {
        let flag = *data.get(at)?;
        at += 1;
        flags.push(flag);
        if flag & 8 != 0 {
            let repeat = *data.get(at)?;
            at += 1;
            flags.extend(std::iter::repeat_n(flag, repeat as usize));
        }
    }
    flags.truncate(count);
    let mut coords = |short: u8, same: u8| -> Option<Vec<i64>> {
        let mut values = Vec::with_capacity(count);
        let mut v = 0i64;
        for &flag in &flags {
            if flag & short != 0 {
                let d = i64::from(*data.get(at)?);
                at += 1;
                v += if flag & same != 0 { d } else { -d };
            } else if flag & same == 0 {
                v += i64::from(be16(data, at)? as i16);
                at += 2;
            }
            values.push(v);
        }
        Some(values)
    };
    let xs = coords(2, 16)?;
    let ys = coords(4, 32)?;
    let base = out.pts.len();
    for i in 0..count {
        out.pts.push((mul_fix(xs[i], scale), mul_fix(ys[i], scale)));
        out.on.push(flags[i] & 1 != 0);
    }
    out.ends.extend(ends.iter().map(|e| e + base));
    Some(())
}

/// `FT_Outline_Decompose` with no shift or delta: conic midpoints are
/// computed in integers (C division, truncating), and each contour is
/// closed with a line back to its start unless it ends on a curve.
fn decompose(points: &Points) -> Vec<Seg> {
    let mut segs = Vec::new();
    let mut first = 0;
    for &end in &points.ends {
        if end <= first {
            first = end;
            continue;
        }
        let last = end - 1;
        let pts = &points.pts;
        let on = &points.on;
        let mid = |a: Pt, b: Pt| ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
        let mut v_start = pts[first];
        let v_last = pts[last];
        let mut limit = last;
        // Index of the point before the next one to read.
        let mut point = first;
        if !on[first] {
            if on[last] {
                v_start = v_last;
                limit = last - 1;
            } else {
                v_start = mid(v_start, v_last);
            }
            // Re-read the first point as a control point.
            segs.push(Seg::Move(v_start));
            if let Some(closed) = conic_run(pts, on, first, limit, v_start, &mut segs) {
                point = closed;
            } else {
                first = end;
                continue;
            }
        } else {
            segs.push(Seg::Move(v_start));
        }
        let mut closed_on_curve = false;
        while point < limit {
            point += 1;
            if on[point] {
                segs.push(Seg::Line(pts[point]));
            } else {
                match conic_run(pts, on, point, limit, v_start, &mut segs) {
                    Some(next) => point = next,
                    None => {
                        closed_on_curve = true;
                        break;
                    }
                }
            }
        }
        if !closed_on_curve {
            segs.push(Seg::Line(v_start));
        }
        first = end;
    }
    segs
}

/// Consumes off-curve points from `point` on (FreeType's `Do_Conic`):
/// returns the index of the on-curve point that ends the run, or `None`
/// when the run reaches the end of the contour and closes on `v_start`.
fn conic_run(pts: &[Pt], on: &[bool], mut point: usize, limit: usize, v_start: Pt, segs: &mut Vec<Seg>) -> Option<usize> {
    let mut v_control = pts[point];
    loop {
        if point < limit {
            point += 1;
            let vec = pts[point];
            if on[point] {
                segs.push(Seg::Conic(v_control, vec));
                return Some(point);
            }
            let v_middle = ((v_control.0 + vec.0) / 2, (v_control.1 + vec.1) / 2);
            segs.push(Seg::Conic(v_control, v_middle));
            v_control = vec;
        } else {
            segs.push(Seg::Conic(v_control, v_start));
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_rounding_matches_freetype() {
        // 67.8125 per font unit: 168 units are 11392.5, rounded away from 0.
        assert_eq!(mul_fix(168, (67 << 16) | 0xD000), 11393);
        assert_eq!(mul_fix(-168, (67 << 16) | 0xD000), -11393);
        assert_eq!(div_fix(138889, 2048), 4444448);
        assert_eq!(mul_div(1479, 4444448, 64), 102708416);
        assert_eq!(pix_floor(-1), -64);
        assert_eq!(pix_ceil(1), 64);
    }

    #[test]
    fn decompose_closes_like_freetype() {
        // A square, then a contour that starts off-curve with an on-curve
        // last point, then one made only of off-curve points.
        let points = Points {
            pts: vec![(0, 0), (10, 0), (10, 10), (5, 20), (0, 0), (10, 10), (0, 0), (10, 0), (10, 11)],
            on: vec![true, true, true, false, true, true, false, false, false],
            ends: vec![3, 6, 9],
        };
        let segs = decompose(&points);
        assert_eq!(
            segs,
            [
                Seg::Move((0, 0)),
                Seg::Line((10, 0)),
                Seg::Line((10, 10)),
                Seg::Line((0, 0)),
                Seg::Move((10, 10)),
                Seg::Conic((5, 20), (0, 0)),
                Seg::Line((10, 10)),
                Seg::Move((5, 5)),
                Seg::Conic((0, 0), (5, 0)),
                Seg::Conic((10, 0), (10, 5)),
                Seg::Conic((10, 11), (5, 5)),
            ]
        );
    }
}
