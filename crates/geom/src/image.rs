//! Pictures of a model without a window or a GPU: a small software
//! renderer (a z-buffer of flat-shaded triangles, lit and framed as the
//! app's viewer does) and a PNG encoder. The same model and view always
//! give the same file.

use crate::export::crc32;
use crate::mesh::{RenderMesh, Rendered};

/// The views the app's presets look from: name, then the camera's angle
/// around Z from +X and its elevation, in degrees.
pub const VIEWS: &[(&str, f32, f32)] = &[
    ("diagonal", -60.0, 28.0),
    ("top", -90.0, 90.0),
    ("bottom", -90.0, -90.0),
    ("front", -90.0, 0.0),
    ("back", 90.0, 0.0),
    ("left", 180.0, 0.0),
    ("right", 0.0, 0.0),
];

/// The viewer's field of view and steepest elevation.
const FOV_Y: f32 = 35.0;
const MAX_PITCH: f32 = 89.5;
/// Samples per pixel along each axis, averaged to smooth the edges.
const SUPERSAMPLE: usize = 2;
/// Outlines: where neighboring samples' depths differ by this fraction, or
/// their faces turn by more than 30 degrees (this cosine); they are this
/// much darker.
const DEPTH_JUMP: f32 = 0.02;
const CREASE: f32 = 0.866;
const EDGE_SHADE: f32 = 0.45;
const BACKGROUND_TOP: [f32; 3] = [0.215, 0.240, 0.290];
const BACKGROUND_BOTTOM: [f32; 3] = [0.078, 0.086, 0.106];

impl Rendered {
    /// A PNG picture of the model, `width` by `height` pixels, seen from
    /// `yaw` and `pitch` (degrees, as in [`VIEWS`]) and framed to fit.
    /// Parts marked `#` and `%` are drawn translucent, and a 2D model as
    /// the slab the app shows. `None` when there is nothing to draw.
    pub fn png(&self, yaw: f32, pitch: f32, width: usize, height: usize) -> Option<Vec<u8>> {
        let (width, height) = (width.max(1), height.max(1));
        Some(encode_png(width, height, &self.rgb(yaw, pitch, width, height)?))
    }

    /// The same picture as [`Rendered::png`], as RGB bytes, row by row from
    /// the top: for a window to show without decoding.
    pub fn rgb(&self, yaw: f32, pitch: f32, width: usize, height: usize) -> Option<Vec<u8>> {
        let meshes = [&self.solid, &self.highlights, &self.backgrounds];
        let (lo, hi) = meshes.iter().filter_map(|m| m.bounds()).reduce(|(lo, hi), (l, h)| {
            ([lo[0].min(l[0]), lo[1].min(l[1]), lo[2].min(l[2])], [hi[0].max(h[0]), hi[1].max(h[1]), hi[2].max(h[2])])
        })?;
        let (width, height) = (width.max(1), height.max(1));
        let camera = Camera::new(lo, hi, yaw, pitch, width * SUPERSAMPLE, height * SUPERSAMPLE);
        let mut canvas = Canvas::new(width * SUPERSAMPLE, height * SUPERSAMPLE);
        canvas.draw(&camera, &self.solid);
        canvas.outline();
        canvas.draw(&camera, &self.highlights);
        canvas.draw(&camera, &self.backgrounds);
        Some(canvas.downsample())
    }
}

/// A perspective camera on a sphere around the box, as the viewer frames
/// a model: looking at its center, from far enough to see all of it.
struct Camera {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    /// Pixels per unit of `x / depth`, and the center of the image.
    scale: f32,
    center: [f32; 2],
}

impl Camera {
    fn new(lo: [f32; 3], hi: [f32; 3], yaw: f32, pitch: f32, width: usize, height: usize) -> Camera {
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.clamp(-MAX_PITCH, MAX_PITCH).to_radians().sin_cos();
        let back = [cp * cy, cp * sy, sp];
        let forward = scale(back, -1.0);
        let right = normalize(cross(forward, [0.0, 0.0, 1.0]));
        let up = cross(right, forward);

        let target = scale(add(lo, hi), 0.5);
        let radius = (length(sub(hi, lo)) * 0.5).max(0.5);
        let half_y = (FOV_Y * 0.5).to_radians();
        let aspect = width as f32 / height as f32;
        let half_x = (half_y.tan() * aspect).atan();
        let distance = radius / half_y.min(half_x).sin() * 1.08;
        Camera {
            eye: add(target, scale(back, distance)),
            right,
            up,
            forward,
            scale: height as f32 * 0.5 / half_y.tan(),
            center: [width as f32 * 0.5, height as f32 * 0.5],
        }
    }

    /// Where `p` lands in the image, in pixels, and one over its depth
    /// (which, unlike the depth, varies linearly across a triangle there).
    fn project(&self, p: [f32; 3]) -> [f32; 3] {
        let d = sub(p, self.eye);
        let z = dot(d, self.forward).max(1e-6);
        let k = self.scale / z;
        [self.center[0] + dot(d, self.right) * k, self.center[1] - dot(d, self.up) * k, 1.0 / z]
    }
}

struct Canvas {
    width: usize,
    height: usize,
    color: Vec<[f32; 3]>,
    /// One over the depth of the nearest opaque surface, 0 for none.
    depth: Vec<f32>,
    /// Its normal, turned towards the camera, in 127ths.
    normal: Vec<[i8; 3]>,
}

impl Canvas {
    /// The viewer's background: a vertical gradient, darker at the corners.
    fn new(width: usize, height: usize) -> Canvas {
        let mut color = Vec::with_capacity(width * height);
        for y in 0..height {
            let v = 1.0 - (y as f32 + 0.5) / height as f32;
            let t = v * v * (3.0 - 2.0 * v);
            let row: [f32; 3] = std::array::from_fn(|i| BACKGROUND_BOTTOM[i] + (BACKGROUND_TOP[i] - BACKGROUND_BOTTOM[i]) * t);
            for x in 0..width {
                let (qx, qy) = ((x as f32 + 0.5) / width as f32 - 0.5, v - 0.5);
                color.push(scale(row, 1.0 - 0.35 * (qx * qx + qy * qy)));
            }
        }
        Canvas { width, height, color, depth: vec![0.0; width * height], normal: vec![[0; 3]; width * height] }
    }

    fn draw(&mut self, camera: &Camera, mesh: &RenderMesh) {
        for ((corners, normals), colors) in
            mesh.positions.chunks_exact(3).zip(mesh.normals.chunks_exact(3)).zip(mesh.colors.chunks_exact(3))
        {
            // Two-sided: lit on the side that faces the camera.
            let centroid = scale(add(add(corners[0], corners[1]), corners[2]), 1.0 / 3.0);
            let normal = if dot(normals[0], sub(camera.eye, centroid)) < 0.0 { scale(normals[0], -1.0) } else { normals[0] };
            let shade = shade(camera, normal, colors[0]);
            let projected = [camera.project(corners[0]), camera.project(corners[1]), camera.project(corners[2])];
            self.fill(projected, shade, colors[0][3], normal.map(|v| (v * 127.0).round() as i8));
        }
    }

    /// Fills a projected triangle: opaque ones write the depth, translucent
    /// ones blend over what is in front of them without hiding anything.
    fn fill(&mut self, [a, b, c]: [[f32; 3]; 3], color: [f32; 3], alpha: f32, normal: [i8; 3]) {
        let edge = |p: [f32; 3], q: [f32; 3], x: f32, y: f32| (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
        let area = edge(a, b, c[0], c[1]);
        if !area.is_normal() {
            return;
        }
        let clamp = |v: f32, n: usize| (v.max(0.0) as usize).min(n);
        let (x0, x1) = (clamp(a[0].min(b[0]).min(c[0]).floor(), self.width), clamp(a[0].max(b[0]).max(c[0]).ceil(), self.width));
        let (y0, y1) = (clamp(a[1].min(b[1]).min(c[1]).floor(), self.height), clamp(a[1].max(b[1]).max(c[1]).ceil(), self.height));
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let (wa, wb) = (edge(b, c, px, py) / area, edge(c, a, px, py) / area);
                let wc = 1.0 - wa - wb;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let depth = wa * a[2] + wb * b[2] + wc * c[2];
                let i = y * self.width + x;
                if depth <= self.depth[i] {
                    continue;
                }
                if alpha >= 0.999 {
                    self.color[i] = color;
                    self.depth[i] = depth;
                    self.normal[i] = normal;
                } else {
                    let under = self.color[i];
                    self.color[i] = std::array::from_fn(|k| under[k] + (color[k] - under[k]) * alpha);
                }
            }
        }
    }

    /// Darkens the opaque surfaces' outlines, where they meet the background
    /// or jump in depth, and their creases, so that faces of one color
    /// stay apart.
    fn outline(&mut self) {
        let differ = |i: usize, j: usize| {
            let (a, b) = (self.depth[i], self.depth[j]);
            if (a > 0.0) != (b > 0.0) {
                return true;
            }
            let [n, m] = [self.normal[i], self.normal[j]].map(|n| n.map(f32::from));
            a > 0.0 && ((a - b).abs() > DEPTH_JUMP * a.max(b) || dot(n, m) < CREASE * dot(n, n).sqrt() * dot(m, m).sqrt())
        };
        let mut edges = vec![false; self.depth.len()];
        for y in 0..self.height {
            for x in 0..self.width {
                let i = y * self.width + x;
                for j in [(x + 1 < self.width).then_some(i + 1), (y + 1 < self.height).then_some(i + self.width)].into_iter().flatten() {
                    if differ(i, j) {
                        edges[i] = true;
                        edges[j] = true;
                    }
                }
            }
        }
        for (color, _) in self.color.iter_mut().zip(&edges).filter(|(_, edge)| **edge) {
            *color = scale(*color, EDGE_SHADE);
        }
    }

    /// The image at its final size, each pixel the average of its samples,
    /// as rows of RGB bytes.
    fn downsample(&self) -> Vec<u8> {
        let (width, height) = (self.width / SUPERSAMPLE, self.height / SUPERSAMPLE);
        let n = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        let mut out = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0; 3];
                for sy in 0..SUPERSAMPLE {
                    let row = (y * SUPERSAMPLE + sy) * self.width + x * SUPERSAMPLE;
                    for sample in &self.color[row..row + SUPERSAMPLE] {
                        sum = add(sum, *sample);
                    }
                }
                out.extend(sum.map(|v| (v / n * 255.0).round().clamp(0.0, 255.0) as u8));
            }
        }
        out
    }
}

/// The viewer's lighting of a flat face: a key light slightly above and
/// right of the camera, a fill light, and a highlight. The highlight takes
/// the camera's direction for the whole image, where the viewer takes it
/// per pixel: per triangle, a face's triangles would differ.
fn shade(camera: &Camera, normal: [f32; 3], color: [f32; 4]) -> [f32; 3] {
    // In the camera's frame: x right, y up, z towards the camera.
    let n = [dot(normal, camera.right), dot(normal, camera.up), -dot(normal, camera.forward)];
    let key = normalize([0.35, 0.45, 1.0]);
    let fill = normalize([-0.75, -0.35, 0.30]);
    let light = 0.30 + 0.62 * dot(n, key).max(0.0) + 0.22 * dot(n, fill).max(0.0);
    let specular = 0.16 * dot(n, normalize(add(key, [0.0, 0.0, 1.0]))).max(0.0).powi(48);
    [color[0], color[1], color[2]].map(|c| (c * light + specular).min(1.0))
}

/// An 8-bit RGB PNG of `rgb`, rows of `width` pixels from the top.
fn encode_png(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 3 + 1) * height);
    for row in rgb.chunks_exact(width * 3) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut header = Vec::with_capacity(13);
    header.extend((width as u32).to_be_bytes());
    header.extend((height as u32).to_be_bytes());
    header.extend([8, 2, 0, 0, 0]); // 8 bits, RGB, deflate, adaptive filters, not interlaced
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let idat = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    for (kind, data) in [(b"IHDR", header), (b"IDAT", idat), (b"IEND", Vec::new())] {
        let mut chunk = kind.to_vec();
        chunk.extend(&data);
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(&chunk);
        out.extend(crc32(&chunk).to_be_bytes());
    }
    out
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// `a` at unit length, or along X when it has none.
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > 1e-12 { scale(a, 1.0 / l) } else { [1.0, 0.0, 0.0] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(src: &str, view: &str) -> Vec<u8> {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        let rendered = crate::render(&c2f_eval::evaluate(&program).nodes);
        let &(_, yaw, pitch) = VIEWS.iter().find(|(name, _, _)| *name == view).unwrap();
        rendered.png(yaw, pitch, 40, 30).expect("a picture")
    }

    /// The pixels of a 40 by 30 PNG this module wrote, as RGB bytes.
    fn pixels(png: &[u8]) -> Vec<u8> {
        let length = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&png[41..41 + length]).unwrap();
        raw.chunks_exact(40 * 3 + 1).flat_map(|row| row[1..].to_vec()).collect()
    }

    #[test]
    fn png_layout() {
        let png = picture("cube(10);", "diagonal");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!((&png[16..20], &png[20..24]), (&40u32.to_be_bytes()[..], &30u32.to_be_bytes()[..]));
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        assert_eq!(pixels(&png).len(), 40 * 30 * 3);
        // The same model gives the same file.
        assert_eq!(png, picture("cube(10);", "diagonal"));
    }

    #[test]
    fn the_model_fills_the_middle_in_its_color() {
        // A red cube seen from the front: its face, lit, in the middle.
        let rgb = pixels(&picture("color(\"red\") translate([-5, -5, -5]) cube(10);", "front"));
        let middle = &rgb[(15 * 40 + 20) * 3..][..3];
        assert!(middle[0] > 150 && middle[1] < 40 && middle[2] < 40, "{middle:?}");
        // The background at the corners.
        assert!(rgb[..3].iter().all(|&c| c < 80), "{:?}", &rgb[..3]);
    }

    #[test]
    fn nothing_to_draw() {
        let program = c2f_eval::Program::load("", std::path::Path::new("t.scad"), &[]).unwrap();
        assert!(crate::render(&c2f_eval::evaluate(&program).nodes).png(0.0, 0.0, 10, 10).is_none());
    }
}
