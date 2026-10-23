//! A sheet of views: one picture of a model from four sides at one scale,
//! with its size in millimeters and a scale bar, to approve a design at a
//! glance without opening the app; and on request a fifth view, cut
//! through its middle, that shows the inside of a hollow part.

use std::borrow::Cow;

use super::{Camera, Canvas, SUPERSAMPLE, VIEWS, dot, encode_png, font, frame};
use crate::mesh::RenderMesh;

/// The views a sheet shows, by their names in [`VIEWS`].
const SHEET_VIEWS: [&str; 4] = ["diagonal", "front", "right", "top"];
/// A view's size in pixels, its title band included, and the band's.
const PANEL: [usize; 2] = [480, 360];
const TITLE: usize = 30;
/// Between the views and around them, and the band above them that holds
/// the size and the scale bar.
const GAP: usize = 10;
const HEADER: usize = 76;
/// The share of its view the model fills along its longest side, in the
/// view where it is largest.
const FILL: f32 = 0.84;
/// The scale bar is about this share of a view's width.
const BAR: f32 = 0.25;
const BACKGROUND: [u8; 3] = [24, 26, 31];
const TITLE_BAND: [u8; 3] = [38, 41, 49];
const TEXT: [u8; 3] = [228, 231, 236];
const MUTED: [u8; 3] = [150, 157, 168];

/// A PNG sheet of the model `meshes` draws (the model, its `#` parts and
/// its `%` parts, as [`crate::Rendered`] holds them): four titled views at
/// one scale, the model's size and a scale bar; with `section`, a fifth
/// view seen from the front with the near half cut away, its cut hatched.
/// A 2D model has no section. `None` when there is nothing to draw.
pub fn sheet_png(meshes: [&RenderMesh; 3], is_2d: bool, section: bool) -> Option<Vec<u8>> {
    let sheet = Sheet::draw(meshes, is_2d, section)?;
    Some(encode_png(sheet.width, sheet.height, &sheet.rgb))
}

impl crate::Rendered {
    /// The sheet of views of this model: see [`sheet_png`].
    pub fn sheet(&self, section: bool) -> Option<Vec<u8>> {
        sheet_png([&self.solid, &self.highlights, &self.backgrounds], self.is_2d, section)
    }
}

struct Sheet {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    /// Pixels per millimeter, in every view.
    #[cfg_attr(not(test), allow(dead_code))]
    scale: f32,
}

/// One view of a sheet.
struct View {
    title: String,
    yaw: f32,
    pitch: f32,
    /// For the section: the solid is cut at y = this, its near half away.
    cut: Option<f32>,
}

impl Sheet {
    fn draw(meshes: [&RenderMesh; 3], is_2d: bool, section: bool) -> Option<Sheet> {
        let (lo, hi) = meshes.iter().filter_map(|m| m.bounds()).reduce(union)?;
        // The size leaves out the parts marked `%`, which are not the model's.
        let size = meshes[..2].iter().filter_map(|m| m.bounds()).reduce(union).map_or([0.0; 3], |(l, h)| {
            std::array::from_fn(|i| h[i] - l[i])
        });
        let target: [f32; 3] = std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5);
        let radius = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f32>().sqrt() * 0.5;

        let mut views: Vec<View> = SHEET_VIEWS
            .iter()
            .map(|name| {
                let &(_, yaw, pitch) = VIEWS.iter().find(|(view, _, _)| view == name).expect("a preset view");
                View { title: name.to_uppercase(), yaw, pitch, cut: None }
            })
            .collect();
        if section && !is_2d && !meshes[0].is_empty() {
            let at = target[1];
            views.push(View { title: format!("SECTION Y = {}", millimeters(at)), yaw: -90.0, pitch: 0.0, cut: Some(at) });
        }

        // The largest scale at which the box fits every view.
        let corners: [[f32; 3]; 8] =
            std::array::from_fn(|i| std::array::from_fn(|axis| if (i >> axis) & 1 == 1 { hi[axis] } else { lo[axis] }));
        let room = [PANEL[0] as f32 * FILL, (PANEL[1] - TITLE) as f32 * FILL];
        let scale = views
            .iter()
            .map(|view| {
                let [_, right, up, _] = frame(view.yaw, view.pitch);
                let extent = |axis: [f32; 3]| {
                    let along = corners.map(|c| dot(c, axis));
                    let (min, max) = along.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
                    (max - min).max(1e-3)
                };
                (room[0] / extent(right)).min(room[1] / extent(up))
            })
            .fold(f32::INFINITY, f32::min);

        let columns = if views.len() > 4 { 3 } else { 2 };
        let width = GAP + columns * (PANEL[0] + GAP);
        let height = HEADER + views.len().div_ceil(columns) * (PANEL[1] + GAP);
        let mut rgb = BACKGROUND.repeat(width * height);
        for (i, view) in views.iter().enumerate() {
            let (x, y) = (GAP + i % columns * (PANEL[0] + GAP), HEADER + i / columns * (PANEL[1] + GAP));
            rectangle(&mut rgb, width, [x, y], [PANEL[0], TITLE], TITLE_BAND);
            font::draw(&mut rgb, width, [x + 10, y + (TITLE - 2 * font::HEIGHT) / 2], &view.title, 2, TEXT);
            let picture = view.picture(meshes, target, radius, scale);
            for (row, line) in picture.chunks_exact(PANEL[0] * 3).enumerate() {
                let start = ((y + TITLE + row) * width + x) * 3;
                rgb[start..start + line.len()].copy_from_slice(line);
            }
        }

        // The size, on the left; the scale bar, on the right.
        let (numbers, axes) = size_text(size, is_2d);
        font::draw(&mut rgb, width, [GAP + 2, 14], &numbers, 3, TEXT);
        font::draw(&mut rgb, width, [GAP + 2, 14 + 3 * font::HEIGHT + 10], axes, 2, MUTED);
        let length = bar_length(scale);
        let label = format!("{} mm", millimeters(length));
        let right = width - GAP - 2;
        let bar = ((length * scale).round() as usize).clamp(4, right);
        let left = right - bar;
        font::draw(&mut rgb, width, [right - font::width(&label, 2), 14], &label, 2, TEXT);
        rectangle(&mut rgb, width, [left, 46], [bar, 4], TEXT);
        for tick in [left, left + bar / 2 - 1, right - 2] {
            rectangle(&mut rgb, width, [tick, 40], [2, 16], TEXT);
        }
        Some(Sheet { width, height, rgb, scale })
    }
}

impl View {
    /// The view's picture, below its title band, as rows of RGB bytes.
    fn picture(&self, meshes: [&RenderMesh; 3], target: [f32; 3], radius: f32, scale: f32) -> Vec<u8> {
        let [width, height] = [PANEL[0] * SUPERSAMPLE, (PANEL[1] - TITLE) * SUPERSAMPLE];
        let camera = Camera::orthographic(target, radius, self.yaw, self.pitch, scale * SUPERSAMPLE as f32, width, height);
        let mut canvas = Canvas::new(width, height);
        let cut = |mesh| -> Cow<'_, RenderMesh> {
            match self.cut {
                Some(at) => Cow::Owned(clip(mesh, at)),
                None => Cow::Borrowed(mesh),
            }
        };
        // Through the cut, what shows first of a closed solid is the inside
        // of its far side: that is where the cut is.
        canvas.caps = self.cut.is_some();
        canvas.draw(&camera, &cut(meshes[0]));
        canvas.caps = false;
        canvas.outline();
        canvas.draw(&camera, &cut(meshes[1]));
        canvas.draw(&camera, &cut(meshes[2]));
        canvas.downsample()
    }
}

/// The triangles of `mesh`, cut at y = `at`, without what lies before it.
fn clip(mesh: &RenderMesh, at: f32) -> RenderMesh {
    let mut out = RenderMesh::default();
    for ((corners, normals), colors) in mesh.positions.chunks_exact(3).zip(mesh.normals.chunks_exact(3)).zip(mesh.colors.chunks_exact(3)) {
        // A triangle cut by a plane keeps three or four corners.
        let mut kept = [[0.0; 3]; 4];
        let mut n = 0;
        for k in 0..3 {
            let (p, q) = (corners[k], corners[(k + 1) % 3]);
            let (dp, dq) = (p[1] - at, q[1] - at);
            if dp >= 0.0 {
                kept[n] = p;
                n += 1;
            }
            if (dp >= 0.0) != (dq >= 0.0) {
                let t = dp / (dp - dq);
                kept[n] = std::array::from_fn(|i| p[i] + (q[i] - p[i]) * t);
                n += 1;
            }
        }
        for k in 1..n.saturating_sub(1) {
            out.positions.extend([kept[0], kept[k], kept[k + 1]]);
            out.normals.extend([normals[0]; 3]);
            out.colors.extend([colors[0]; 3]);
        }
    }
    out
}

fn union((lo, hi): ([f32; 3], [f32; 3]), (l, h): ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
    (std::array::from_fn(|i| lo[i].min(l[i])), std::array::from_fn(|i| hi[i].max(h[i])))
}

/// The model's size as the sheet writes it, and what each number is: a
/// 2D model has no height.
fn size_text([x, y, z]: [f32; 3], is_2d: bool) -> (String, &'static str) {
    if is_2d {
        (format!("{} × {} mm", millimeters(x), millimeters(y)), "WIDTH (X) × DEPTH (Y)")
    } else {
        (format!("{} × {} × {} mm", millimeters(x), millimeters(y), millimeters(z)), "WIDTH (X) × DEPTH (Y) × HEIGHT (Z)")
    }
}

/// A round length for the scale bar, 1, 2 or 5 times a power of ten
/// millimeters, about [`BAR`] of a view at `scale` pixels per millimeter.
fn bar_length(scale: f32) -> f32 {
    let target = PANEL[0] as f32 * BAR / scale;
    let power = 10f32.powf(target.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|k| k * power).find(|length| *length <= target).unwrap_or(power)
}

/// `v` with at most two decimals, and none it does not need.
fn millimeters(v: f32) -> String {
    let text = format!("{v:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.into() }
}

fn rectangle(rgb: &mut [u8], width: usize, [x, y]: [usize; 2], [w, h]: [usize; 2], color: [u8; 3]) {
    for row in y..y + h {
        for column in x..(x + w).min(width) {
            if let Some(pixel) = rgb.get_mut((row * width + column) * 3..(row * width + column) * 3 + 3) {
                pixel.copy_from_slice(&color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(src: &str, section: bool) -> Sheet {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        let rendered = crate::render(&c2f_eval::evaluate(&program).nodes);
        Sheet::draw([&rendered.solid, &rendered.highlights, &rendered.backgrounds], rendered.is_2d, section).expect("a sheet")
    }

    /// The pixels of view `i`'s picture, with their place in it.
    fn view(sheet: &Sheet, i: usize) -> impl Iterator<Item = (usize, usize, [u8; 3])> + '_ {
        let columns = if sheet.width > GAP + 2 * (PANEL[0] + GAP) { 3 } else { 2 };
        let (x0, y0) = (GAP + i % columns * (PANEL[0] + GAP), HEADER + i / columns * (PANEL[1] + GAP) + TITLE);
        (0..PANEL[1] - TITLE).flat_map(move |y| {
            (0..PANEL[0]).map(move |x| {
                let at = ((y0 + y) * sheet.width + x0 + x) * 3;
                (x, y, [sheet.rgb[at], sheet.rgb[at + 1], sheet.rgb[at + 2]])
            })
        })
    }

    /// The width and height, in pixels, of what is red in view `i`.
    fn red(sheet: &Sheet, i: usize) -> Option<[usize; 2]> {
        let reds = view(sheet, i).filter(|(_, _, [r, g, b])| *r > 90 && *g < 60 && *b < 60);
        let (lo, hi) = reds.fold(None, |b: Option<([usize; 2], [usize; 2])>, (x, y, _)| {
            let (lo, hi) = b.unwrap_or(([x, y], [x, y]));
            Some(([lo[0].min(x), lo[1].min(y)], [hi[0].max(x), hi[1].max(y)]))
        })?;
        Some([hi[0] - lo[0] + 1, hi[1] - lo[1] + 1])
    }

    fn is_cut([r, g, _]: [u8; 3]) -> bool {
        r > 120 && f32::from(r) > 1.6 * f32::from(g)
    }

    #[test]
    fn four_titled_views_at_one_scale() {
        let s = sheet("color(\"red\") cube([10, 20, 40]);", false);
        assert_eq!((s.width, s.height), (GAP + 2 * (PANEL[0] + GAP), HEADER + 2 * (PANEL[1] + GAP)));
        let [diagonal, front, right, top] = [0, 1, 2, 3].map(|i| red(&s, i).expect("a view with the model"));
        assert!(diagonal[0] > 0);
        // Front shows x and z, the right side y and z, the top x and y: at
        // one scale, each size is as long in every view it shows in.
        let close = |a: usize, b: f32| (a as f32 - b).abs() <= 2.0;
        assert!(close(front[0], 10.0 * s.scale) && close(front[1], 40.0 * s.scale), "{front:?} at {}", s.scale);
        assert!(close(right[0], 20.0 * s.scale) && close(right[1], 40.0 * s.scale), "{right:?}");
        assert!(close(top[0], 10.0 * s.scale) && close(top[1], 20.0 * s.scale), "{top:?}");
    }

    #[test]
    fn sizes_compare() {
        // Two cubes side by side: three times the side, three times the
        // footprint.
        let s = sheet("color(\"red\") cube(10); color(\"red\") translate([20, 0, 0]) cube(30);", false);
        let reds: Vec<usize> = (0..PANEL[0]).filter(|&x| view(&s, 3).any(|(px, _, [r, g, _])| px == x && r > 90 && g < 60)).collect();
        // Two runs of columns, the second three times the first.
        let gap = reds.windows(2).position(|w| w[1] > w[0] + 1).expect("two cubes apart");
        let (small, large) = (gap + 1, reds.len() - gap - 1);
        assert!((large as f32 / small as f32 - 3.0).abs() < 0.15, "{small} and {large}");
        // A model three times the size: drawn at a third of the scale, the
        // same in millimeters.
        let (one, three) = (sheet("color(\"red\") cube(10);", false), sheet("color(\"red\") cube(30);", false));
        assert!((one.scale / three.scale - 3.0).abs() < 1e-3);
        let [w1, w3] = [&one, &three].map(|s| red(s, 1).unwrap()[0] as f32 / s.scale);
        assert!((w1 - 10.0).abs() < 0.5 && (w3 - 30.0).abs() < 0.5, "{w1} {w3}");
        assert!(bar_length(one.scale) < bar_length(three.scale));
    }

    #[test]
    fn size_in_millimeters() {
        assert_eq!(size_text([10.0, 20.5, 3.126], false).0, "10 × 20.5 × 3.13 mm");
        assert_eq!(size_text([120.0, 80.0, 1.0], true), ("120 × 80 mm".into(), "WIDTH (X) × DEPTH (Y)"));
        assert_eq!((millimeters(-0.001), millimeters(0.5), millimeters(100.0)), ("0".into(), "0.5".into(), "100".into()));
        assert_eq!([0.9, 2.0, 4.5, 30.0].map(bar_length), [100.0, 50.0, 20.0, 2.0]);
        // The header is drawn: the size's numbers stand out of the background.
        let s = sheet("cube(10);", false);
        assert!(s.rgb[..HEADER * s.width * 3].chunks_exact(3).any(|p| p == TEXT));
    }

    #[test]
    fn a_section_shows_the_inside() {
        // A hollow box: through its middle, its walls are cut and its
        // cavity shows behind.
        let s = sheet("difference() { cube(30, center = true); cube(20, center = true); }", true);
        assert_eq!(s.width, GAP + 3 * (PANEL[0] + GAP));
        let pixel = |i: usize, x: usize, y: usize| view(&s, i).find(|p| p.0 == x && p.1 == y).unwrap().2;
        let middle = [PANEL[0] / 2, (PANEL[1] - TITLE) / 2];
        assert!(!is_cut(pixel(4, middle[0], middle[1])), "{:?}", pixel(4, middle[0], middle[1]));
        // The walls, 5 mm thick, around it.
        let wall = middle[0] + (12.5 * s.scale) as usize;
        assert!(is_cut(pixel(4, wall, middle[1])), "{:?}", pixel(4, wall, middle[1]));
        // Seen whole from the front, there is no cut.
        assert!(!view(&s, 1).any(|p| is_cut(p.2)));

        // A solid block is cut through and through.
        let s = sheet("cube(30, center = true);", true);
        assert!(is_cut(view(&s, 4).find(|p| p.0 == middle[0] && p.1 == middle[1]).unwrap().2));
    }

    #[test]
    fn a_2d_model_has_no_section() {
        let s = sheet("square([30, 10]);", true);
        assert_eq!(s.width, GAP + 2 * (PANEL[0] + GAP));
    }

    #[test]
    fn the_png() {
        let program = c2f_eval::Program::load("cube(10);", std::path::Path::new("t.scad"), &[]).unwrap();
        let rendered = crate::render(&c2f_eval::evaluate(&program).nodes);
        let png = rendered.sheet(false).unwrap();
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()) as usize, GAP + 2 * (PANEL[0] + GAP));
        assert_eq!(png, rendered.sheet(false).unwrap());
        let empty = c2f_eval::Program::load("", std::path::Path::new("t.scad"), &[]).unwrap();
        assert!(crate::render(&c2f_eval::evaluate(&empty).nodes).sheet(false).is_none());
    }
}
