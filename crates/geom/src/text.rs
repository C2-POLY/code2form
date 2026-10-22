//! `text()`: glyph outlines laid out the way the reference implementation
//! (2021.01) does it with FreeType 2.10, HarfBuzz and Clipper, so that
//! vertices land where the reference puts them.
//!
//! What the reference does, and what is reproduced here:
//! - FreeType renders every glyph at one fixed character size, 100000 in
//!   26.6 points at 100 dpi (2170 pixels per em: integer ppem fonts such as
//!   Liberation round the 2170.14). Outline points are integer 26.6
//!   values, divided by 1e5 and multiplied by `size`: one em is about
//!   `size / 0.72`.
//! - HarfBuzz shapes the text (kerning, right-to-left, scripts) with
//!   FreeType's unhinted, unrounded advances. The reference multiplies its
//!   positions by 1/1.024e5 instead of 1/1e5, so glyphs sit 2.4% closer
//!   than the outlines' own scale, a mismatch the reference kept.
//! - Curves become `max(floor(n / 8) + 1, 2)` straight segments each,
//!   where `n` is the circle fragment count for `r = size` from `$fn`,
//!   `$fa`, `$fs` (4 by default).
//! - `halign` uses the total advance width; `valign` uses the largest
//!   glyph ascent and descent of the control boxes rounded out to whole
//!   pixels, also multiplied by 1/1.024e5.
//! - Points go to Clipper truncated to 1/65536 and the glyphs are united
//!   with the non-zero rule; each contour is then cleaned of points
//!   within 1.415/65536 of their neighbors or of the line through them.

mod fonts;
mod outline;

pub use fonts::set_folders as set_font_folders;

use clipper2_rust::{FillRule, Path64, Paths64, Point64, union_64};
use manifold_rust::cross_section::CrossSection;
use manifold_rust::linalg::Vec2;
use c2f_eval::node::TextParams;
use ttf_parser::{Face, GlyphId, Tag};

use outline::{Loader, Outline, Seg, div_fix, mul_div, mul_fix, pix_ceil, pix_floor};

use crate::primitives::fragments;

/// The character size the reference gives FreeType (26.6 points, at
/// 100 dpi), which outline points are also divided by.
const CHAR_SIZE: i64 = 100_000;
const SCALE: f64 = 1e5;
/// What the reference multiplies HarfBuzz positions and glyph boxes by.
const UNSCALE: f64 = 1.0 / 1.024e5;
/// The reference's fixed-point scale for Clipper (doubles are truncated).
const CLIPPER_SCALE: f64 = 65536.0;
/// Clipper 1's default `CleanPolygon` distance, in Clipper units.
const CLEAN_DISTANCE: f64 = 1.415;

/// The outline of `text()`, or `None` when it is empty; warnings go to
/// `warnings`.
pub(crate) fn render(params: &TextParams, warnings: &mut Vec<String>) -> Option<CrossSection> {
    let polygons = glyph_polygons(params, warnings)?;
    let mut paths: Vec<Paths64> = polygons.iter().map(to_clipper).collect();
    if paths.is_empty() {
        return None;
    }
    // The first glyph is Clipper's subject, all others its clip, both
    // non-zero.
    let subject = paths.remove(0);
    let clip: Paths64 = paths.into_iter().flatten().collect();
    let united = union_64(&subject, &clip, FillRule::NonZero);
    let contours: Vec<Vec<Vec2>> = united
        .iter()
        .map(|path| clean_polygon(path, CLEAN_DISTANCE))
        .filter(|path| path.len() >= 3)
        .map(|path| path.iter().map(|p| Vec2::new(p.x as f64 / CLIPPER_SCALE, p.y as f64 / CLIPPER_SCALE)).collect())
        .collect();
    if contours.is_empty() {
        return None;
    }
    // Clipper's results have counter-clockwise outer contours, which the
    // cross-section's positive fill rule keeps.
    let section = CrossSection::new(contours);
    (!section.is_empty()).then_some(section)
}

/// One glyph's contours, in model units.
type Polygon = Vec<Vec<[f64; 2]>>;

/// A shaped glyph: its outline, and HarfBuzz's position in 26.6 units.
struct Glyph {
    outline: Outline,
    x_advance: i64,
    y_advance: i64,
    x_offset: i64,
    y_offset: i64,
}

/// The glyphs of the text as the reference's `FreetypeRenderer` and
/// `DrawingCallback` draw them, one polygon per glyph with an outline.
fn glyph_polygons(params: &TextParams, warnings: &mut Vec<String>) -> Option<Vec<Polygon>> {
    if params.text.is_empty() {
        return None;
    }
    let font = fonts::load(&params.font, warnings)?;
    let face = match Face::parse(&font.data, font.index) {
        Ok(face) => face,
        Err(err) => {
            warnings.push(format!("Can't read font \"{}\": {err}", params.font));
            return None;
        }
    };
    let glyphs = shape(&face, params);
    let direction = direction(&params.direction);
    let horizontal = direction.is_none_or(|d| matches!(d, rustybuzz::Direction::LeftToRight | rustybuzz::Direction::RightToLeft));

    let (mut width, mut ascend, mut descend) = (0.0f64, 0.0f64, 0.0f64);
    for glyph in &glyphs {
        let [x_min, y_min, x_max, y_max] = glyph.outline.cbox;
        let (x_min, y_min, x_max, y_max) = (pix_floor(x_min), pix_floor(y_min), pix_ceil(x_max), pix_ceil(y_max));
        if horizontal {
            width += glyph.x_advance as f64 * UNSCALE * params.spacing;
            ascend = ascend.max(0.0f64.max(y_max as f64 * UNSCALE));
            descend = descend.max(0.0f64.max(-y_min as f64 * UNSCALE));
        } else {
            width = width.max((x_max - x_min) as f64 * UNSCALE);
            ascend += glyph.y_advance as f64 * UNSCALE * params.spacing;
        }
    }
    let x_offset = match params.halign.as_str() {
        "right" => -width,
        "center" => -width / 2.0,
        "left" => 0.0,
        other => {
            warnings.push(format!("Unknown value for the halign parameter (use \"left\", \"right\" or \"center\"): '{other}'"));
            0.0
        }
    };
    let y_offset = match params.valign.as_str() {
        "top" => -ascend,
        "center" => descend / 2.0 - ascend / 2.0,
        "bottom" => descend,
        "baseline" => 0.0,
        other => {
            warnings.push(format!(
                "Unknown value for the valign parameter (use \"baseline\", \"bottom\", \"top\" or \"center\"): '{other}'"
            ));
            0.0
        }
    };

    let segments = text_segments(params);
    let mut pen = Pen { size: params.size, fn_: segments, offset: [0.0; 2], advance: [0.0; 2], pen: [0.0; 2] };
    let mut polygons = Vec::new();
    for glyph in &glyphs {
        pen.offset = [x_offset + glyph.x_offset as f64 * UNSCALE, y_offset + glyph.y_offset as f64 * UNSCALE];
        let polygon = pen.draw(&glyph.outline.segs);
        let adv_x = glyph.x_advance as f64 * UNSCALE * params.spacing;
        let adv_y = glyph.y_advance as f64 * UNSCALE * params.spacing;
        pen.advance = [pen.advance[0] + adv_x, pen.advance[1] + adv_y];
        if !polygon.is_empty() {
            polygons.push(polygon);
        }
    }
    Some(polygons)
}

/// Segments per curve: the reference takes the fragments of a circle of
/// radius `size`, divided by 8, plus 1, and at least 2.
fn text_segments(params: &TextParams) -> u32 {
    let s = &params.segments;
    (fragments(params.size, s.fn_, s.fs, s.fa) / 8 + 1).max(2)
}

/// HarfBuzz's `hb_direction_from_string`: only the first letter counts.
fn direction(name: &str) -> Option<rustybuzz::Direction> {
    use rustybuzz::Direction::*;
    match name.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('l') => Some(LeftToRight),
        Some('r') => Some(RightToLeft),
        Some('t') => Some(TopToBottom),
        Some('b') => Some(BottomToTop),
        _ => None,
    }
}

/// FreeType's scales for the reference's character size, in 16.16
/// 26.6-units per font unit.
struct Scales {
    upem: i64,
    /// The size's own scale: what FreeType's advances (HarfBuzz) use.
    metrics: i64,
    /// The scale of hinted outlines: fonts whose `head` asks for integer
    /// ppem round 2170.14 down to 2170.
    outline: i64,
    /// HarfBuzz's font scale (26.6 units per em).
    hb: i64,
}

impl Scales {
    fn new(face: &Face) -> Self {
        let upem = i64::from(face.units_per_em()).max(1);
        // FT_REQUEST_HEIGHT: points to 26.6 pixels at 100 dpi.
        let pixels = (CHAR_SIZE * 100 + 36) / 72;
        let metrics = div_fix(pixels, upem);
        let integer_ppem = face
            .raw_face()
            .table(Tag::from_bytes(b"head"))
            .and_then(|h| h.get(16..18))
            .is_some_and(|f| f[1] & 8 != 0);
        let truetype = face.raw_face().table(Tag::from_bytes(b"glyf")).is_some();
        let outline = if integer_ppem && truetype { div_fix(((pixels + 32) >> 6) << 6, upem) } else { metrics };
        let hb = (metrics * upem + (1 << 15)) >> 16;
        Scales { upem, metrics, outline, hb }
    }

    /// HarfBuzz's `em_scale`: font units to 26.6, rounded half away from
    /// zero.
    fn em(&self, v: i64) -> i64 {
        let scaled = v * self.hb;
        let scaled = scaled + if scaled >= 0 { self.upem / 2 } else { -(self.upem / 2) };
        scaled / self.upem
    }

    /// hb-ft's advance: FreeType's 16.16 linear advance, rounded to 26.6.
    fn advance(&self, units: i64) -> i64 {
        (mul_div(units, self.metrics, 64) + 512) >> 10
    }
}

/// Shapes the text with rustybuzz and converts its font-unit positions to
/// the 26.6 values HarfBuzz computes on a FreeType font.
fn shape(face: &Face, params: &TextParams) -> Vec<Glyph> {
    let scales = Scales::new(face);
    let hb_face = rustybuzz::Face::from_face(face.clone());
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(&params.text);
    let direction = direction(&params.direction);
    if let Some(direction) = direction {
        buffer.set_direction(direction);
    }
    if let Some(script) = params.script.as_deref().and_then(script) {
        buffer.set_script(script);
    }
    if let Ok(language) = params.language.parse::<rustybuzz::Language>() {
        buffer.set_language(language);
    }
    let shaped = rustybuzz::shape(&hb_face, &[], buffer);
    let vertical = matches!(direction, Some(rustybuzz::Direction::TopToBottom | rustybuzz::Direction::BottomToTop));
    let loader = Loader::new(face);
    shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(info, pos)| {
            let id = info.glyph_id as u16;
            let outline = loader.load(id, scales.outline);
            let h_units = i64::from(face.glyph_hor_advance(GlyphId(id)).unwrap_or(0));
            if vertical {
                // HarfBuzz moves each glyph by minus its vertical origin.
                let (x_origin, y_origin, y_advance) = vertical_metrics(face, &loader, &scales, id, h_units);
                Glyph { outline, x_advance: 0, y_advance, x_offset: -x_origin, y_offset: -y_origin }
            } else {
                let x_advance = match pos.x_advance {
                    0 => 0,
                    units => scales.advance(h_units) + scales.em(i64::from(units) - h_units),
                };
                Glyph {
                    outline,
                    x_advance,
                    y_advance: scales.em(pos.y_advance.into()),
                    x_offset: scales.em(pos.x_offset.into()),
                    y_offset: scales.em(pos.y_offset.into()),
                }
            }
        })
        .collect()
}

/// hb-ft's vertical origin and advance (26.6), from the metrics FreeType
/// makes up for fonts without vertical ones: the advance is the OS/2
/// typographic line height and the glyph is centered on it.
fn vertical_metrics(face: &Face, loader: &Loader, scales: &Scales, id: u16, h_units: i64) -> (i64, i64, i64) {
    let glyph = GlyphId(id);
    let unhinted = loader.load(id, scales.metrics);
    let [_, y_min, _, y_max] = unhinted.cbox;
    let lsb = i64::from(face.glyph_hor_side_bearing(glyph).unwrap_or(0));
    let x_min = face.glyph_bounding_box(glyph).map_or(lsb, |b| i64::from(b.x_min));
    let hori_advance = mul_fix(x_min - lsb + h_units, scales.metrics) - mul_fix(x_min - lsb, scales.metrics);
    let (advance, top) = match (face.glyph_ver_advance(glyph), face.glyph_ver_side_bearing(glyph)) {
        (Some(advance), Some(tsb)) => (i64::from(advance), i64::from(tsb)),
        _ => {
            let advance = match (face.typographic_ascender(), face.typographic_descender()) {
                (Some(a), Some(d)) => (i64::from(a) - i64::from(d)).abs(),
                _ => i64::from(face.ascender()) - i64::from(face.descender()),
            };
            let height = div_fix(y_max - y_min, scales.metrics);
            (advance, (advance - height) / 2)
        }
    };
    // HarfBuzz negates FreeType's downward 16.16 advance, rounding to 26.6.
    let y_advance = (-mul_div(advance, scales.metrics, 64) + 512) >> 10;
    (hori_advance / 2, y_max + mul_fix(top, scales.metrics), y_advance)
}

/// HarfBuzz's script for an ISO 15924 tag.
fn script(tag: &str) -> Option<rustybuzz::Script> {
    let bytes: Vec<u8> = tag.bytes().chain(std::iter::repeat(b' ')).take(4).collect();
    rustybuzz::Script::from_iso15924_tag(Tag::from_bytes(&bytes.try_into().ok()?))
}

/// The reference's `DrawingCallback`: flattens outlines into contours of
/// `size * (point + offset + advance)`.
struct Pen {
    size: f64,
    fn_: u32,
    offset: [f64; 2],
    advance: [f64; 2],
    pen: [f64; 2],
}

impl Pen {
    fn draw(&mut self, segs: &[Seg]) -> Polygon {
        let mut polygon = Vec::new();
        let mut contour: Vec<[f64; 2]> = Vec::new();
        let scaled = |p: outline::Pt| [p.0 as f64 / SCALE, p.1 as f64 / SCALE];
        for seg in segs {
            match *seg {
                Seg::Move(to) => {
                    if !contour.is_empty() {
                        polygon.push(std::mem::take(&mut contour));
                    }
                    let to = scaled(to);
                    contour.push(self.vertex(to));
                    self.pen = to;
                }
                Seg::Line(to) => {
                    let to = scaled(to);
                    contour.push(self.vertex(to));
                    self.pen = to;
                }
                Seg::Conic(c1, to) => {
                    let (c1, to) = (scaled(c1), scaled(to));
                    for idx in 1..=self.fn_ {
                        let a = idx as f64 * (1.0 / self.fn_ as f64);
                        let b = 1.0 - a;
                        let p = |i: usize| self.pen[i] * b.powf(2.0) + c1[i] * 2.0 * b * a + to[i] * a.powf(2.0);
                        contour.push(self.vertex([p(0), p(1)]));
                    }
                    self.pen = to;
                }
                Seg::Cubic(c1, c2, to) => {
                    let (c1, c2, to) = (scaled(c1), scaled(c2), scaled(to));
                    for idx in 1..=self.fn_ {
                        let a = idx as f64 * (1.0 / self.fn_ as f64);
                        let b = 1.0 - a;
                        let p = |i: usize| {
                            self.pen[i] * b.powf(3.0)
                                + c1[i] * 3.0 * b.powf(2.0) * a
                                + c2[i] * 3.0 * b * a.powf(2.0)
                                + to[i] * a.powf(3.0)
                        };
                        contour.push(self.vertex([p(0), p(1)]));
                    }
                    self.pen = to;
                }
            }
        }
        if !contour.is_empty() {
            polygon.push(contour);
        }
        polygon
    }

    fn vertex(&self, v: [f64; 2]) -> [f64; 2] {
        [self.size * (v[0] + self.offset[0] + self.advance[0]), self.size * (v[1] + self.offset[1] + self.advance[1])]
    }
}

/// The reference's conversion to Clipper: coordinates scaled and
/// truncated, contours kept as they are.
fn to_clipper(polygon: &Polygon) -> Paths64 {
    polygon
        .iter()
        .map(|contour| {
            contour.iter().map(|&[x, y]| Point64::new((x * CLIPPER_SCALE) as i64, (y * CLIPPER_SCALE) as i64)).collect()
        })
        .collect()
}

/// Clipper 1's `CleanPolygon`: drops vertices within `distance` of the
/// previous one, pairs around a vertex whose neighbors are that close,
/// and vertices within `distance` of the line through their neighbors.
fn clean_polygon(path: &Path64, distance: f64) -> Vec<Point64> {
    let n = path.len();
    if n == 0 {
        return Vec::new();
    }
    let pt: Vec<(i64, i64)> = path.iter().map(|p| (p.x, p.y)).collect();
    let mut next: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut done = vec![false; n];
    let dist_sqrd = distance * distance;
    let mut size = n;
    // Unlinks `op`, returning its predecessor, which is checked again.
    let exclude = |op: usize, next: &mut Vec<usize>, prev: &mut Vec<usize>, done: &mut Vec<bool>| {
        let result = prev[op];
        next[result] = next[op];
        prev[next[op]] = result;
        done[result] = false;
        result
    };
    let mut op = 0;
    while !done[op] && next[op] != prev[op] {
        if points_are_close(pt[op], pt[prev[op]], dist_sqrd) {
            op = exclude(op, &mut next, &mut prev, &mut done);
            size -= 1;
        } else if points_are_close(pt[prev[op]], pt[next[op]], dist_sqrd) {
            exclude(next[op], &mut next, &mut prev, &mut done);
            op = exclude(op, &mut next, &mut prev, &mut done);
            size = size.saturating_sub(2);
        } else if slopes_near_collinear(pt[prev[op]], pt[op], pt[next[op]], dist_sqrd) {
            op = exclude(op, &mut next, &mut prev, &mut done);
            size -= 1;
        } else {
            done[op] = true;
            op = next[op];
        }
    }
    if size < 3 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(size);
    for _ in 0..size {
        out.push(Point64::new(pt[op].0, pt[op].1));
        op = next[op];
    }
    out
}

fn points_are_close(a: (i64, i64), b: (i64, i64), dist_sqrd: f64) -> bool {
    let dx = a.0 as f64 - b.0 as f64;
    let dy = a.1 as f64 - b.1 as f64;
    dx * dx + dy * dy <= dist_sqrd
}

fn distance_from_line_sqrd(pt: (i64, i64), ln1: (i64, i64), ln2: (i64, i64)) -> f64 {
    let a = (ln1.1 - ln2.1) as f64;
    let b = (ln2.0 - ln1.0) as f64;
    let c = a * ln1.0 as f64 + b * ln1.1 as f64;
    let c = a * pt.0 as f64 + b * pt.1 as f64 - c;
    (c * c) / (a * a + b * b)
}

/// Tests the point lying between the other two, as Clipper does.
fn slopes_near_collinear(p1: (i64, i64), p2: (i64, i64), p3: (i64, i64), dist_sqrd: f64) -> bool {
    if (p1.0 - p2.0).abs() > (p1.1 - p2.1).abs() {
        if (p1.0 > p2.0) == (p1.0 < p3.0) {
            distance_from_line_sqrd(p1, p2, p3) < dist_sqrd
        } else if (p2.0 > p1.0) == (p2.0 < p3.0) {
            distance_from_line_sqrd(p2, p1, p3) < dist_sqrd
        } else {
            distance_from_line_sqrd(p3, p1, p2) < dist_sqrd
        }
    } else if (p1.1 > p2.1) == (p1.1 < p3.1) {
        distance_from_line_sqrd(p1, p2, p3) < dist_sqrd
    } else if (p2.1 > p1.1) == (p2.1 < p3.1) {
        distance_from_line_sqrd(p2, p1, p3) < dist_sqrd
    } else {
        distance_from_line_sqrd(p3, p1, p2) < dist_sqrd
    }
}
