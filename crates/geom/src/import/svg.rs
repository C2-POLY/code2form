//! SVG, imported as the reference implementation (2021.01) does with its
//! libsvg (behaviour probed against that release):
//!
//! - shapes: `path`, `rect` (rounded too), `circle`, `ellipse`, `line`,
//!   `polyline`, `polygon`; `text` and `use` give nothing; everything
//!   under `defs` is left out, other containers are transparent. Fill and
//!   display properties are ignored.
//! - circles and ellipses have 40 segments and Bézier curves 20, whatever
//!   `$fn`; an arc of angle θ has `trunc(10 θ / π) + 4`.
//! - open shapes (`line`, `polyline`, and a `path`'s open subpaths before
//!   its first `Z`) are stroked with the element's own `stroke-width`
//!   (1 when missing or not a plain positive number), square caps and
//!   mitered joins unless `stroke-linecap` / `stroke-linejoin` say
//!   otherwise; other subpaths are filled. Each shape's outlines combine
//!   even-odd, whatever `fill-rule`; shapes are united.
//! - units: user units are millimetres unless a `viewBox` maps them onto
//!   the page size (unitless sizes count 72 per inch, `px` 96). The y axis
//!   flips about the page height: `y = height - y_user`; with a viewBox the
//!   offset is the viewBox *plus* y (a libsvg sign slip, kept).
//! - the last `<svg>` element of the document gives the page; the root's
//!   own transform is ignored.

use std::f64::consts::PI;

use manifold_rust::cross_section::CrossSection;
use manifold_rust::types::OpType;

use super::xml::{self, Element};

/// `import()`'s default `dpi`.
const DPI: f64 = 72.0;
const MM_PER_INCH: f64 = 25.4;
const CIRCLE_SEGMENTS: u32 = 40;
const CURVE_SEGMENTS: u32 = 20;

pub(super) fn read(data: &[u8], name: &str, warnings: &mut Vec<String>) -> Option<CrossSection> {
    let root = match xml::parse(&String::from_utf8_lossy(data)) {
        Ok(root) if root.name == "svg" => root,
        Ok(_) => {
            warnings.push(format!("'{name}' is not an SVG file"));
            return None;
        }
        Err(e) => {
            warnings.push(format!("Error parsing file '{name}': {e}"));
            return None;
        }
    };
    let (shapes, page) = walk(&root, warnings);
    let place = Placement::of(page);
    let sections: Vec<CrossSection> = shapes
        .iter()
        .map(|outlines| {
            let placed: Vec<Vec<[f64; 2]>> = outlines.iter().map(|o| o.iter().map(|&p| place.apply(p)).collect()).collect();
            super::even_odd(&placed)
        })
        .filter(|s| !s.is_empty())
        .collect();
    let united = match sections.len() {
        0 => return None,
        1 => sections.into_iter().next()?,
        _ => CrossSection::batch_boolean(&sections, OpType::Add),
    };
    Some(super::without_collinear(united))
}

/// An affine map `[a, b, c, d, e, f]`: x' = a x + c y + e, y' = b x + d y + f.
type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn multiply(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

fn map(m: &Matrix, [x, y]: [f64; 2]) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

/// Each shape's outlines in the root's user units, in document order, and
/// the last `<svg>` element.
fn walk<'a>(root: &'a Element, warnings: &mut Vec<String>) -> (Vec<Vec<Vec<[f64; 2]>>>, &'a Element) {
    let mut shapes = Vec::new();
    let mut page = root;
    // Element, its parent's transform, whether under a `defs`.
    let mut stack = vec![(root, IDENTITY, false)];
    while let Some((e, parent, excluded)) = stack.pop() {
        if e.name == "svg" {
            page = e;
        }
        let m = match e.attr("transform") {
            Some(t) if !std::ptr::eq(e, root) => multiply(&parent, &transform(t, warnings)),
            _ => parent,
        };
        let excluded = excluded || e.name == "defs";
        if !excluded {
            let outlines = shape(e);
            if !outlines.is_empty() {
                shapes.push(outlines.iter().map(|o| o.iter().map(|&p| map(&m, p)).collect()).collect());
            }
        }
        stack.extend(e.children.iter().rev().map(|child| (child, m, excluded)));
    }
    (shapes, page)
}

/// A `transform` attribute: operations applied right to left.
fn transform(text: &str, warnings: &mut Vec<String>) -> Matrix {
    let mut m = IDENTITY;
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if rest.is_empty() {
            return m;
        }
        let Some(open) = rest.find('(') else { return m };
        let Some(close) = rest.find(')') else { return m };
        let op = rest[..open].trim();
        let args = numbers(&rest[open + 1..close.max(open + 1)]);
        rest = &rest[close + 1..];
        let arg = |i: usize, default: f64| args.get(i).copied().unwrap_or(default);
        let step = match op {
            "matrix" if args.len() == 6 => [args[0], args[1], args[2], args[3], args[4], args[5]],
            "translate" => [1.0, 0.0, 0.0, 1.0, arg(0, 0.0), arg(1, 0.0)],
            "scale" => [arg(0, 1.0), 0.0, 0.0, arg(1, arg(0, 1.0)), 0.0, 0.0],
            "rotate" => {
                let (sin, cos) = arg(0, 0.0).to_radians().sin_cos();
                let (cx, cy) = (arg(1, 0.0), arg(2, 0.0));
                // translate(cx, cy) rotate(a) translate(-cx, -cy)
                [cos, sin, -sin, cos, cx - cos * cx + sin * cy, cy - sin * cx - cos * cy]
            }
            "skewX" => [1.0, 0.0, arg(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0],
            "skewY" => [1.0, arg(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            _ => {
                warnings.push(format!("unknown transform op '{op}' in SVG"));
                continue;
            }
        };
        m = multiply(&m, &step);
    }
}

/// The page: how user units map to millimetres.
struct Placement {
    scale: [f64; 2],
    viewbox: [f64; 2],
    offset: [f64; 2],
    height: f64,
}

impl Placement {
    fn of(page: &Element) -> Placement {
        let viewbox = page.attr("viewBox").map(numbers).filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0);
        let size = |attr: &str, k: usize| length_mm(page.attr(attr), viewbox.as_ref().map(|v| v[k]));
        let (width, height) = (size("width", 2), size("height", 3));
        let mut placement = Placement { scale: [1.0, 1.0], viewbox: [0.0, 0.0], offset: [0.0, 0.0], height };
        if let Some(v) = viewbox {
            placement.viewbox = [v[0], v[1]];
            if width > 0.0 && height > 0.0 {
                let (sx, sy) = (width / v[2], height / v[3]);
                let (align, slice) = aspect(page.attr("preserveAspectRatio").unwrap_or(""));
                placement.scale = match align {
                    None => [sx, sy],
                    Some(_) if slice => [sx.max(sy); 2],
                    Some(_) => [sx.min(sy); 2],
                };
                if let Some([ax, ay]) = align {
                    placement.offset = [
                        ax * (width - placement.scale[0] * v[2]) / 2.0,
                        ay * (height - placement.scale[1] * v[3]) / 2.0,
                    ];
                }
            }
        }
        placement
    }

    fn apply(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [
            self.scale[0] * (x - self.viewbox[0]) + self.offset[0],
            self.height - self.offset[1] - self.scale[1] * (y + self.viewbox[1]),
        ]
    }
}

/// A page size in millimetres; 0 when unknown.
fn length_mm(text: Option<&str>, viewbox: Option<f64>) -> f64 {
    let Some(text) = text.map(str::trim) else {
        // Missing: the viewBox size, in unitless pixels.
        return viewbox.map_or(0.0, |v| v * MM_PER_INCH / DPI);
    };
    let mut scanner = Scanner::new(text);
    let Some(value) = scanner.number() else { return 0.0 };
    match text[scanner.at..].trim() {
        "" => value * MM_PER_INCH / DPI,
        "px" => value * MM_PER_INCH / 96.0,
        "pt" => value * MM_PER_INCH / 72.0,
        "pc" => value * MM_PER_INCH / 6.0,
        "in" => value * MM_PER_INCH,
        "cm" => value * 10.0,
        "mm" => value,
        "%" => viewbox.map_or(0.0, |v| value * v / 100.0),
        _ => 0.0,
    }
}

/// `preserveAspectRatio`: the alignment factors (0 min, 1 mid, 2 max)
/// unless `none`, and whether to slice rather than meet.
fn aspect(text: &str) -> (Option<[f64; 2]>, bool) {
    let mut words = text.split_whitespace();
    let align = words.next().unwrap_or("xMidYMid");
    let slice = words.next() == Some("slice");
    if align == "none" {
        return (None, slice);
    }
    let factor = |part: &str| match part {
        "Min" => 0.0,
        "Max" => 2.0,
        _ => 1.0,
    };
    let (x, y) = match (align.len(), align.get(1..4), align.get(5..8)) {
        (8, Some(x), Some(y)) => (x, y),
        _ => ("Mid", "Mid"),
    };
    (Some([factor(x), factor(y)]), slice)
}

/// Numbers in a list separated by spaces and/or commas, as in `points`,
/// `viewBox` and transforms.
fn numbers(text: &str) -> Vec<f64> {
    let mut scanner = Scanner::new(text);
    let mut out = Vec::new();
    while let Some(v) = scanner.number() {
        out.push(v);
    }
    out
}

/// A geometry attribute: a plain number, else 0.
fn attr_number(e: &Element, name: &str) -> f64 {
    e.attr(name).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0)
}

/// A presentation property, from `style` first, then the attribute.
fn property<'e>(e: &'e Element, name: &str) -> Option<&'e str> {
    let from_style = e.attr("style").and_then(|style| {
        style.split(';').find_map(|decl| {
            let (key, value) = decl.split_once(':')?;
            (key.trim() == name).then_some(value)
        })
    });
    from_style.or_else(|| e.attr(name))
}

/// The outlines of one element, in its user units: closed contours, with
/// open parts already stroked.
fn shape(e: &Element) -> Vec<Vec<[f64; 2]>> {
    let num = |name: &str| attr_number(e, name);
    match e.name.as_str() {
        "rect" => {
            let (x, y, w, h) = (num("x"), num("y"), num("width"), num("height"));
            let (mut rx, mut ry) = (num("rx").max(0.0), num("ry").max(0.0));
            if rx == 0.0 && ry == 0.0 {
                return vec![vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]]];
            }
            if rx == 0.0 {
                rx = ry;
            }
            if ry == 0.0 {
                ry = rx;
            }
            rx = rx.min(w / 2.0);
            ry = ry.min(h / 2.0);
            // libsvg draws the rounded rectangle as this path.
            let d = format!(
                "M {},{} H {} A {rx},{ry} 0 0,1 {},{} V {} A {rx},{ry} 0 0,1 {},{} H {} A {rx},{ry} 0 0,1 {},{} V {} A {rx},{ry} 0 0,1 {},{} z",
                x + rx,
                y,
                x + w - rx,
                x + w,
                y + ry,
                y + h - ry,
                x + w - rx,
                y + h,
                x + rx,
                x,
                y + h - ry,
                y + ry,
                x + rx,
                y
            );
            path(&d, &Stroke::of(e))
        }
        "circle" => vec![ellipse(num("cx"), num("cy"), num("r"), num("r"))],
        "ellipse" => vec![ellipse(num("cx"), num("cy"), num("rx"), num("ry"))],
        "line" => Stroke::of(e).apply(&[[num("x1"), num("y1")], [num("x2"), num("y2")]]),
        "polyline" => Stroke::of(e).apply(&pairs(e.attr("points").unwrap_or(""))),
        "polygon" => {
            let mut points = pairs(e.attr("points").unwrap_or(""));
            if let Some(&first) = points.first() {
                points.push(first);
            }
            vec![points]
        }
        "path" => path(e.attr("d").unwrap_or(""), &Stroke::of(e)),
        _ => Vec::new(),
    }
}

fn pairs(text: &str) -> Vec<[f64; 2]> {
    numbers(text).chunks_exact(2).map(|p| [p[0], p[1]]).collect()
}

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Vec<[f64; 2]> {
    (1..=CIRCLE_SEGMENTS)
        .map(|i| {
            let a = i as f64 * 360.0 / CIRCLE_SEGMENTS as f64;
            [rx * sin_degrees(a) + cx, ry * cos_degrees(a) + cy]
        })
        .collect()
}

/// Exact at multiples of 90 degrees.
fn sin_degrees(a: f64) -> f64 {
    match a.rem_euclid(360.0) {
        0.0 | 180.0 => 0.0,
        90.0 => 1.0,
        270.0 => -1.0,
        a => a.to_radians().sin(),
    }
}

fn cos_degrees(a: f64) -> f64 {
    sin_degrees(a + 90.0)
}

/// A reader of SVG numbers and flags.
struct Scanner<'t> {
    text: &'t [u8],
    at: usize,
}

impl<'t> Scanner<'t> {
    fn new(text: &'t str) -> Self {
        Scanner { text: text.as_bytes(), at: 0 }
    }

    fn skip_separators(&mut self) {
        while self.at < self.text.len() && (self.text[self.at].is_ascii_whitespace() || self.text[self.at] == b',') {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_separators();
        self.text.get(self.at).copied()
    }

    /// `[sign] digits [. digits] [e [sign] digits]`; a second `.` starts
    /// the next number.
    fn number(&mut self) -> Option<f64> {
        self.skip_separators();
        let start = self.at;
        let t = self.text;
        let mut i = start;
        if i < t.len() && (t[i] == b'+' || t[i] == b'-') {
            i += 1;
        }
        let digits = |i: &mut usize| {
            let from = *i;
            while *i < t.len() && t[*i].is_ascii_digit() {
                *i += 1;
            }
            *i > from
        };
        let mut any = digits(&mut i);
        if i < t.len() && t[i] == b'.' {
            i += 1;
            any |= digits(&mut i);
        }
        if !any {
            return None;
        }
        if i < t.len() && (t[i] == b'e' || t[i] == b'E') {
            let mut j = i + 1;
            if j < t.len() && (t[j] == b'+' || t[j] == b'-') {
                j += 1;
            }
            if digits(&mut j) {
                i = j;
            }
        }
        self.at = i;
        std::str::from_utf8(&t[start..i]).ok()?.parse().ok()
    }

    /// An arc flag: one `0` or `1`, which needs no separator after it.
    fn flag(&mut self) -> Option<bool> {
        match self.peek()? {
            b'0' => {
                self.at += 1;
                Some(false)
            }
            b'1' => {
                self.at += 1;
                Some(true)
            }
            _ => None,
        }
    }

    fn point(&mut self) -> Option<[f64; 2]> {
        let x = self.number()?;
        Some([x, self.number()?])
    }
}

/// Path data: closed subpaths and filled ones as they are, open ones
/// stroked when no `Z` came before them.
fn path(d: &str, stroke: &Stroke) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let mut scanner = Scanner::new(d);
    let mut sub: Vec<[f64; 2]> = Vec::new();
    let mut closed_before = false;
    let (mut current, mut start) = ([0.0, 0.0], [0.0, 0.0]);
    // The last control point and whether it came from a cubic (true) or a
    // quadratic (false) command, for S and T.
    let mut last_control: Option<([f64; 2], bool)> = None;
    let mut command = 0u8;
    let finish = |sub: &mut Vec<[f64; 2]>, out: &mut Vec<Vec<[f64; 2]>>, closed_before: bool| {
        if sub.is_empty() {
            return;
        }
        let points = std::mem::take(sub);
        if !closed_before && is_open(&points) {
            out.extend(stroke.apply(&points));
        } else {
            out.push(points);
        }
    };
    while let Some(c) = scanner.peek() {
        if c.is_ascii_alphabetic() {
            command = c;
            scanner.at += 1;
        } else if command == 0 {
            break;
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative { current } else { [0.0, 0.0] };
        let add = |p: [f64; 2]| [p[0] + base[0], p[1] + base[1]];
        let mut control = None;
        match command.to_ascii_uppercase() {
            b'M' => {
                let Some(p) = scanner.point() else { break };
                finish(&mut sub, &mut out, closed_before);
                current = add(p);
                start = current;
                sub.push(current);
                // Further pairs are line-tos.
                command = if relative { b'l' } else { b'L' };
            }
            b'L' => {
                let Some(p) = scanner.point() else { break };
                current = add(p);
                sub.push(current);
            }
            b'H' => {
                let Some(x) = scanner.number() else { break };
                current = [x + base[0], current[1]];
                sub.push(current);
            }
            b'V' => {
                let Some(y) = scanner.number() else { break };
                current = [current[0], y + base[1]];
                sub.push(current);
            }
            b'C' | b'S' => {
                let first = if command.eq_ignore_ascii_case(&b'C') {
                    let Some(p) = scanner.point() else { break };
                    add(p)
                } else {
                    match last_control {
                        Some((c, true)) => [2.0 * current[0] - c[0], 2.0 * current[1] - c[1]],
                        _ => current,
                    }
                };
                let (Some(second), Some(end)) = (scanner.point(), scanner.point()) else { break };
                let (second, end) = (add(second), add(end));
                for i in 1..=CURVE_SEGMENTS {
                    let t = i as f64 / CURVE_SEGMENTS as f64;
                    let u = 1.0 - t;
                    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                    sub.push([
                        w[0] * current[0] + w[1] * first[0] + w[2] * second[0] + w[3] * end[0],
                        w[0] * current[1] + w[1] * first[1] + w[2] * second[1] + w[3] * end[1],
                    ]);
                }
                control = Some((second, true));
                current = end;
            }
            b'Q' | b'T' => {
                let ctrl = if command.eq_ignore_ascii_case(&b'Q') {
                    let Some(p) = scanner.point() else { break };
                    add(p)
                } else {
                    match last_control {
                        Some((c, false)) => [2.0 * current[0] - c[0], 2.0 * current[1] - c[1]],
                        _ => current,
                    }
                };
                let Some(end) = scanner.point() else { break };
                let end = add(end);
                for i in 1..=CURVE_SEGMENTS {
                    let t = i as f64 / CURVE_SEGMENTS as f64;
                    let u = 1.0 - t;
                    let w = [u * u, 2.0 * u * t, t * t];
                    sub.push([
                        w[0] * current[0] + w[1] * ctrl[0] + w[2] * end[0],
                        w[0] * current[1] + w[1] * ctrl[1] + w[2] * end[1],
                    ]);
                }
                control = Some((ctrl, false));
                current = end;
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(angle)) = (scanner.number(), scanner.number(), scanner.number()) else { break };
                let (Some(large), Some(sweep)) = (scanner.flag(), scanner.flag()) else { break };
                let Some(end) = scanner.point() else { break };
                let end = add(end);
                arc(&mut sub, current, rx, ry, angle, large, sweep, end);
                current = end;
            }
            b'Z' => {
                if !sub.is_empty() {
                    sub.push(sub[0]);
                    out.push(std::mem::take(&mut sub));
                }
                closed_before = true;
                current = start;
                // Z takes no numbers; a number after it is an error.
                command = 0;
            }
            _ => break,
        }
        last_control = control;
    }
    finish(&mut sub, &mut out, closed_before);
    out
}

fn is_open(points: &[[f64; 2]]) -> bool {
    match (points.first(), points.last()) {
        (Some(a), Some(b)) => (a[0] - b[0]).hypot(a[1] - b[1]) > 1e-9,
        _ => false,
    }
}

/// An elliptical arc from `from` to `to` (SVG implementation notes F.6),
/// start point included.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<[f64; 2]>, from: [f64; 2], rx: f64, ry: f64, angle: f64, large: bool, sweep: bool, to: [f64; 2]) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        out.push(to);
        return;
    }
    let (sinr, cosr) = angle.to_radians().sin_cos();
    let (dx, dy) = ((from[0] - to[0]) / 2.0, (from[1] - to[1]) / 2.0);
    let x1 = cosr * dx + sinr * dy;
    let y1 = -sinr * dx + cosr * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let (rx2, ry2) = (rx * rx, ry * ry);
    let (t1, t2) = (rx2 * y1 * y1, ry2 * x1 * x1);
    let mut c = ((rx2 * ry2 - t1 - t2) / (t1 + t2)).abs().sqrt();
    if large == sweep {
        c = -c;
    }
    let (cx1, cy1) = (c * rx * y1 / ry, -c * ry * x1 / rx);
    let cx = cosr * cx1 - sinr * cy1 + (from[0] + to[0]) / 2.0;
    let cy = sinr * cx1 + cosr * cy1 + (from[1] + to[1]) / 2.0;
    let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
    let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let theta = vector_angle(1.0, 0.0, ux, uy);
    let mut delta = vector_angle(ux, uy, vx, vy) % (2.0 * PI);
    if !sweep && delta > 0.0 {
        delta -= 2.0 * PI;
    } else if sweep && delta < 0.0 {
        delta += 2.0 * PI;
    }
    let steps = (delta.abs() * 10.0 / PI + 4.0) as u64;
    for i in 0..=steps {
        let phi = theta + delta * i as f64 / steps as f64;
        let (sin, cos) = phi.sin_cos();
        out.push([cosr * cos * rx - sinr * sin * ry + cx, sinr * cos * rx + cosr * sin * ry + cy]);
    }
}

fn vector_angle(ux: f64, uy: f64, vx: f64, vy: f64) -> f64 {
    let angle = vy.atan2(vx) - uy.atan2(ux);
    if angle < 0.0 { angle + 2.0 * PI } else { angle }
}

#[derive(Clone, Copy, PartialEq)]
enum Cap {
    Butt,
    Square,
    Round,
}

#[derive(Clone, Copy, PartialEq)]
enum Join {
    Miter,
    Square,
    Round,
}

/// How an element's open outlines are stroked.
struct Stroke {
    width: f64,
    cap: Cap,
    join: Join,
}

impl Stroke {
    fn of(e: &Element) -> Stroke {
        // "2mm" or "4px" do not parse, and give the default, as in libsvg.
        let width = property(e, "stroke-width").and_then(|w| w.trim().parse::<f64>().ok()).filter(|&w| w > 0.0).unwrap_or(1.0);
        let cap = match property(e, "stroke-linecap").map(str::trim) {
            Some("butt") => Cap::Butt,
            Some("round") => Cap::Round,
            _ => Cap::Square,
        };
        let join = match property(e, "stroke-linejoin").map(str::trim) {
            Some("round") => Join::Round,
            Some("bevel") => Join::Square,
            _ => Join::Miter,
        };
        Stroke { width, cap, join }
    }

    fn apply(&self, points: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
        offset::open_path(points, self.width, self.cap, self.join)
    }
}

/// Clipper 1's `ClipperOffset` on one open path, as libsvg calls it:
/// coordinates scaled by 10000 and truncated to integers, half the width
/// as the offset, miter limit 2, arc tolerance 0.25 units, then the
/// positive union of the result.
mod offset {
    use std::f64::consts::PI;

    use clipper2_rust::{FillRule, Path64, Paths64, Point, union_subjects_64};

    use super::{Cap, Join};

    const SCALE: f64 = 10000.0;

    fn round(v: f64) -> i64 {
        if v < 0.0 { (v - 0.5) as i64 } else { (v + 0.5) as i64 }
    }

    fn unit_normal(a: [i64; 2], b: [i64; 2]) -> [f64; 2] {
        if a == b {
            return [0.0, 0.0];
        }
        let (dx, dy) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
        let f = 1.0 / (dx * dx + dy * dy).sqrt();
        [dy * f, -dx * f]
    }

    struct Offsetter {
        src: Vec<[i64; 2]>,
        normals: Vec<[f64; 2]>,
        dest: Vec<[i64; 2]>,
        delta: f64,
        sin_a: f64,
        sin: f64,
        cos: f64,
        steps_per_rad: f64,
        miter_limit: f64,
        join: Join,
    }

    impl Offsetter {
        fn push(&mut self, j: usize, n: [f64; 2]) {
            let p = self.src[j];
            self.dest.push([round(p[0] as f64 + n[0] * self.delta), round(p[1] as f64 + n[1] * self.delta)]);
        }

        fn offset_point(&mut self, j: usize, k: &mut usize) {
            let (nk, nj) = (self.normals[*k], self.normals[j]);
            self.sin_a = nk[0] * nj[1] - nj[0] * nk[1];
            if (self.sin_a * self.delta).abs() < 1.0 {
                let cos_a = nk[0] * nj[0] + nj[1] * nk[1];
                if cos_a > 0.0 {
                    self.push(j, nk);
                    return;
                }
            } else {
                self.sin_a = self.sin_a.clamp(-1.0, 1.0);
            }
            if self.sin_a * self.delta < 0.0 {
                self.push(j, nk);
                self.dest.push(self.src[j]);
                self.push(j, nj);
            } else {
                match self.join {
                    Join::Miter => {
                        let r = 1.0 + (nj[0] * nk[0] + nj[1] * nk[1]);
                        if r >= self.miter_limit {
                            let q = self.delta / r;
                            let p = self.src[j];
                            self.dest.push([round(p[0] as f64 + (nk[0] + nj[0]) * q), round(p[1] as f64 + (nk[1] + nj[1]) * q)]);
                        } else {
                            self.square(j, *k);
                        }
                    }
                    Join::Square => self.square(j, *k),
                    Join::Round => self.round_join(j, *k),
                }
            }
            *k = j;
        }

        fn square(&mut self, j: usize, k: usize) {
            let (nk, nj) = (self.normals[k], self.normals[j]);
            let dx = (self.sin_a.atan2(nk[0] * nj[0] + nk[1] * nj[1]) / 4.0).tan();
            let p = self.src[j];
            let d = self.delta;
            self.dest.push([round(p[0] as f64 + d * (nk[0] - nk[1] * dx)), round(p[1] as f64 + d * (nk[1] + nk[0] * dx))]);
            self.dest.push([round(p[0] as f64 + d * (nj[0] + nj[1] * dx)), round(p[1] as f64 + d * (nj[1] - nj[0] * dx))]);
        }

        fn round_join(&mut self, j: usize, k: usize) {
            let (nk, nj) = (self.normals[k], self.normals[j]);
            let a = self.sin_a.atan2(nk[0] * nj[0] + nk[1] * nj[1]);
            let steps = (round(self.steps_per_rad * a.abs()) as i32).max(1);
            let (mut x, mut y) = (nk[0], nk[1]);
            let p = self.src[j];
            for _ in 0..steps {
                self.dest.push([round(p[0] as f64 + x * self.delta), round(p[1] as f64 + y * self.delta)]);
                let x2 = x;
                x = x * self.cos - self.sin * y;
                y = x2 * self.sin + y * self.cos;
            }
            self.push(j, nj);
        }

        fn cap(&mut self, cap: Cap, j: usize, k: usize) {
            self.sin_a = 0.0;
            match cap {
                Cap::Square => self.square(j, k),
                _ => self.round_join(j, k),
            }
        }
    }

    pub(super) fn open_path(points: &[[f64; 2]], width: f64, cap: Cap, join: Join) -> Vec<Vec<[f64; 2]>> {
        let mut src: Vec<[i64; 2]> = Vec::with_capacity(points.len());
        for p in points {
            // Truncated, as the IntPoint constructor does.
            let q = [(p[0] * SCALE) as i64, (p[1] * SCALE) as i64];
            if src.last() != Some(&q) {
                src.push(q);
            }
        }
        if src.is_empty() {
            return Vec::new();
        }
        let delta = width * SCALE / 2.0;
        let tolerance = if 0.25 > delta.abs() * 0.25 { delta.abs() * 0.25 } else { 0.25 };
        let mut steps = PI / (1.0 - tolerance / delta.abs()).acos();
        if steps > delta.abs() * PI {
            steps = delta.abs() * PI;
        }
        let mut o = Offsetter {
            normals: Vec::new(),
            dest: Vec::new(),
            delta,
            sin_a: 0.0,
            sin: (2.0 * PI / steps).sin(),
            cos: (2.0 * PI / steps).cos(),
            steps_per_rad: steps / (2.0 * PI),
            miter_limit: 0.5,
            join,
            src,
        };
        let len = o.src.len();
        if len == 1 {
            let p = o.src[0];
            if join == Join::Round {
                let (mut x, mut y) = (1.0, 0.0);
                let mut j = 1.0;
                while j <= steps {
                    o.dest.push([round(p[0] as f64 + x * delta), round(p[1] as f64 + y * delta)]);
                    let x2 = x;
                    x = x * o.cos - o.sin * y;
                    y = x2 * o.sin + y * o.cos;
                    j += 1.0;
                }
            } else {
                for [x, y] in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                    o.dest.push([round(p[0] as f64 + x * delta), round(p[1] as f64 + y * delta)]);
                }
            }
        } else {
            for j in 0..len - 1 {
                o.normals.push(unit_normal(o.src[j], o.src[j + 1]));
            }
            o.normals.push(o.normals[len - 2]);
            let mut k = 0;
            for j in 1..len - 1 {
                o.offset_point(j, &mut k);
            }
            let last = len - 1;
            if cap == Cap::Butt {
                let n = o.normals[last];
                o.push(last, n);
                o.push(last, [-n[0], -n[1]]);
            } else {
                o.normals[last] = [-o.normals[last][0], -o.normals[last][1]];
                o.cap(cap, last, len - 2);
            }
            for j in (1..len).rev() {
                o.normals[j] = [-o.normals[j - 1][0], -o.normals[j - 1][1]];
            }
            o.normals[0] = [-o.normals[1][0], -o.normals[1][1]];
            let mut k = last;
            for j in (1..last).rev() {
                o.offset_point(j, &mut k);
            }
            if cap == Cap::Butt {
                let n = o.normals[0];
                o.push(0, [-n[0], -n[1]]);
                o.push(0, n);
            } else {
                o.cap(cap, 0, 1);
            }
        }
        let polygon: Path64 = o.dest.iter().map(|p| Point::new(p[0], p[1])).collect();
        let united = union_subjects_64(&Paths64::from(vec![polygon]), FillRule::Positive);
        united
            .iter()
            .map(|path| {
                let mut outline: Vec<[f64; 2]> = path.iter().map(|p| [p.x as f64 / SCALE, p.y as f64 / SCALE]).collect();
                if let Some(&first) = outline.first() {
                    outline.push(first);
                }
                outline
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(svg: &str) -> CrossSection {
        let mut warnings = Vec::new();
        let section = read(svg.as_bytes(), "t.svg", &mut warnings).expect("geometry");
        assert!(warnings.is_empty(), "{warnings:?}");
        section
    }

    fn bounds(section: &CrossSection) -> [f64; 4] {
        let b = section.bounds();
        [b.min.x, b.min.y, b.max.x, b.max.y]
    }

    fn close(a: [f64; 4], b: [f64; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    // Expected values below come from the reference release (2021.01).

    #[test]
    fn page_units_and_flip() {
        let rect = r#"<rect x="10" y="5" width="20" height="10"/>"#;
        let s = import(&format!(r#"<svg width="100" height="50">{rect}</svg>"#));
        assert!(close(bounds(&s), [10.0, 2.63889, 30.0, 12.63889]));
        let s = import(&format!(r#"<svg width="100mm" height="50mm" viewBox="0 0 200 100">{rect}</svg>"#));
        assert!(close(bounds(&s), [5.0, 42.5, 15.0, 47.5]));
        let s = import(r#"<svg width="96px" height="96px"><rect width="1" height="1"/></svg>"#);
        assert!(close(bounds(&s), [0.0, 24.4, 1.0, 25.4]));
        // The viewBox origin's y is added, not subtracted.
        let s = import(r#"<svg width="100mm" height="100mm" viewBox="10 20 200 100"><rect x="10" y="20" width="20" height="10"/></svg>"#);
        assert!(close(bounds(&s), [0.0, 50.0, 10.0, 55.0]));
        let s = import(r#"<svg width="100mm" height="50mm" viewBox="0 0 10 10" preserveAspectRatio="xMaxYMax slice"><rect width="1" height="1"/></svg>"#);
        assert!(close(bounds(&s), [0.0, 90.0, 10.0, 100.0]));
    }

    #[test]
    fn segment_counts() {
        let count = |svg: &str| import(svg).num_vert();
        assert_eq!(count(r#"<svg><circle cx="50" cy="40" r="10"/></svg>"#), 40);
        // A cubic: 20 segments, plus the closing line.
        assert_eq!(count(r#"<svg><path d="M 0 0 C 0 100 100 100 100 0 Z"/></svg>"#), 21);
        // Half a circle: 14 segments.
        assert_eq!(count(r#"<svg><path d="M 10 50 A 20 20 0 0 1 50 50 Z"/></svg>"#), 15);
        // Rounded corners: 9 segments each.
        assert_eq!(count(r#"<svg><rect x="10" y="10" width="40" height="20" rx="5"/></svg>"#), 40);
    }

    #[test]
    fn strokes() {
        // Square caps and a mitered join, width 1 by default.
        let s = import(r#"<svg><polyline points="40,0 60,0 60,10"/></svg>"#);
        assert!((s.area() - 31.0).abs() < 1e-6, "{}", s.area());
        assert!(close(bounds(&s), [39.5, -10.5, 60.5, 0.5]));
        let s = import(r#"<svg><path d="M 10 10 L 50 10 L 50 30" stroke-width="2" stroke-linecap="butt"/></svg>"#);
        assert!(close(bounds(&s), [10.0, -30.0, 51.0, -9.0]));
        // Round caps: many points, about a full circle of area.
        let s = import(r#"<svg><path d="M 0 0 L 10 0" stroke-width="2" stroke-linecap="round"/></svg>"#);
        assert!((s.area() - (20.0 + PI)).abs() < 1e-3);
        // An open subpath after a Z is filled, not stroked.
        let s = import(r#"<svg><path d="M 0 0 L 10 0 L 10 10 Z M 20 0 L 30 0 L 30 10"/></svg>"#);
        assert!((s.area() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn transforms_and_holes() {
        let s = import(r#"<svg width="100" height="100"><g transform="translate(10,20) scale(2)"><rect width="10" height="5" transform="rotate(30)"/></g></svg>"#);
        assert!((s.area() - 200.0).abs() < 1e-6);
        assert!(close(bounds(&s), [5.0, -3.38246, 27.3205, 15.2778]));
        let s = import(r#"<svg><path fill-rule="nonzero" d="M 0 0 H 30 V 30 H 0 Z M 10 10 H 20 V 20 H 10 Z"/></svg>"#);
        assert!((s.area() - 800.0).abs() < 1e-6);
        let s = import(r#"<svg><defs><rect width="5" height="5"/></defs><rect x="10" width="5" height="5"/></svg>"#);
        assert!((s.area() - 25.0).abs() < 1e-6);
    }

    #[test]
    fn number_syntax() {
        assert_eq!(numbers("1e1,1E1 30.5.5-5,20"), [10.0, 10.0, 30.5, 0.5, -5.0, 20.0]);
        let s = import(r#"<svg><path d="M10,10h10v10h-10zM40-5l5,5,5-5z"/></svg>"#);
        assert!((s.area() - 125.0).abs() < 1e-6);
    }
}
