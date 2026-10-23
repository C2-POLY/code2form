//! The app icon: a tile and two rounded squares on it, each smaller one
//! turned a little further, lit from above with the squares' shadows on
//! the tile, in one of ten tints the user picks in the settings. Graphite
//! is the default and the icon the app bundle ships.
//!
//! Drawn here rather than kept as pictures, so that the bundle's icon, the
//! Dock and the settings all come from one drawing, at any size.

/// The tints of the icon, in the order the settings show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tint {
    #[default]
    Graphite,
    Green,
    Red,
    Orange,
    Amber,
    Teal,
    Cyan,
    Indigo,
    Violet,
    Pink,
}

/// The tile and the two squares, darkest to lightest.
struct Palette {
    tile: [u8; 3],
    outer: [u8; 3],
    inner: [u8; 3],
}

const fn hex(rgb: u32) -> [u8; 3] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

const fn palette(tile: u32, outer: u32, inner: u32) -> Palette {
    Palette { tile: hex(tile), outer: hex(outer), inner: hex(inner) }
}

impl Tint {
    pub const ALL: [Tint; 10] = [
        Tint::Graphite,
        Tint::Green,
        Tint::Red,
        Tint::Orange,
        Tint::Amber,
        Tint::Teal,
        Tint::Cyan,
        Tint::Indigo,
        Tint::Violet,
        Tint::Pink,
    ];

    // `name`, the tint as the settings list it, is in `icon_setting.rs`, in
    // the interface's language: the `app-icon` example builds this file on
    // its own, without the app's translations.

    /// The name kept in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Tint::Graphite => "graphite",
            Tint::Green => "green",
            Tint::Red => "red",
            Tint::Orange => "orange",
            Tint::Amber => "amber",
            Tint::Teal => "teal",
            Tint::Cyan => "cyan",
            Tint::Indigo => "indigo",
            Tint::Violet => "violet",
            Tint::Pink => "pink",
        }
    }

    pub fn from_id(id: &str) -> Option<Tint> {
        Tint::ALL.into_iter().find(|t| t.id() == id.trim())
    }

    /// The tile and the two squares, darkest to lightest.
    pub fn colors(self) -> [[u8; 3]; 3] {
        let Palette { tile, outer, inner } = self.palette();
        [tile, outer, inner]
    }

    fn palette(self) -> Palette {
        match self {
            Tint::Graphite => palette(0x464952, 0x8a909c, 0xe6e8ec),
            Tint::Green => palette(0x146334, 0x22c55e, 0xa7f3c4),
            Tint::Red => palette(0x851f25, 0xef3b45, 0xffb3b8),
            Tint::Orange => palette(0x8a4011, 0xff7a1a, 0xffc79a),
            Tint::Amber => palette(0x856206, 0xf5b800, 0xffe58a),
            Tint::Teal => palette(0x0f695f, 0x14b8a6, 0x9ef0e4),
            Tint::Cyan => palette(0x0c5a83, 0x0ea5e9, 0xa5e4ff),
            Tint::Indigo => palette(0x373493, 0x6366f1, 0xc4c6ff),
            Tint::Violet => palette(0x562c92, 0x9b5cf6, 0xdcc6ff),
            Tint::Pink => palette(0x862359, 0xec4899, 0xffc1e0),
        }
    }
}

/// How the tile sits in the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The tile fills the square, for pictures inside the app.
    Full,
    /// The tile on macOS's icon grid: 824 of 1024 with a margin around it,
    /// as the Dock and the Finder expect.
    MacIcon,
}

/// An RGBA picture, its colors premultiplied by alpha.
pub struct Image {
    pub size: usize,
    pub premultiplied: Vec<u8>,
}

impl Image {
    /// The colors divided back by alpha, as most image formats store them.
    pub fn unmultiplied(&self) -> Vec<u8> {
        let mut rgba = self.premultiplied.clone();
        for pixel in rgba.chunks_exact_mut(4) {
            let a = pixel[3];
            if a != 0 && a != 255 {
                for c in &mut pixel[..3] {
                    *c = ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                }
            }
        }
        rgba
    }
}

/// The drawing, in a 1024-unit square, y down: the tile's corner radius,
/// then each square's edge, corner radius and turn in degrees.
const TILE_RADIUS: f32 = 230.0;
const SQUARES: [(f32, f32, f32); 2] = [(660.0, 80.0, 15.0), (420.0, 52.0, 30.0)];

/// The light from above: how far each shape goes towards white at its top
/// and towards black at its bottom, the tile first.
const LIGHT: [(f32, f32); 3] = [(0.18, 0.30), (0.15, 0.12), (0.35, 0.06)];
/// The white rim along the tile's top edge: its alpha and width.
const RIM: (f32, f32) = (0.30, 10.0);
/// The squares' shadows on what is under them: how far down, how soft
/// and how dark.
const SHADOW_DROP: f32 = 18.0;
const SHADOW_BLUR: f32 = 34.0;
const SHADOW_ALPHA: f32 = 0.45;

/// Draws the icon `size` pixels square.
pub fn render(tint: Tint, size: usize, layout: Layout) -> Image {
    let palette = tint.palette();
    let colors = [palette.outer, palette.inner];
    // Design units per pixel, and where pixel (0, 0) falls in the design.
    let (scale, origin) = match layout {
        Layout::Full => (1024.0 / size as f32, 0.0),
        Layout::MacIcon => (1024.0 * 1024.0 / 824.0 / size as f32, -100.0 * 1024.0 / 824.0),
    };
    let mut premultiplied = vec![0u8; size * size * 4];
    let mut layer = [0.0f32; 4];
    for y in 0..size {
        for x in 0..size {
            let p = (origin + (x as f32 + 0.5) * scale, origin + (y as f32 + 0.5) * scale);
            // Premultiplied color and alpha, built up shape over shape.
            layer.fill(0.0);
            let tile_distance = rounded_square(p, 1024.0, TILE_RADIUS, 0.0);
            let tile = coverage(tile_distance, scale);
            if tile > 0.0 {
                let (light, dark) = LIGHT[0];
                over(&mut layer, lit(palette.tile, light, dark, p.1 / 1024.0), tile);
                let rim = RIM.0 * top_rim(p, (1024.0, TILE_RADIUS, 0.0), tile_distance, RIM.1, scale);
                over(&mut layer, [255; 3], rim);
                for ((&square, color), &(light, dark)) in SQUARES.iter().zip(colors).zip(&LIGHT[1..]) {
                    let (edge, radius, turn) = square;
                    let shadow = rounded_square((p.0, p.1 - SHADOW_DROP), edge, radius, turn);
                    let shadow = SHADOW_ALPHA * (1.0 - smoothstep(-SHADOW_BLUR, SHADOW_BLUR, shadow));
                    over(&mut layer, [0; 3], shadow * tile);
                    // The gradient spans a little more than the square, so
                    // that its turned corners are not pure white or black.
                    let top = 512.0 - edge * 0.6;
                    let color = lit(color, light, dark, (p.1 - top) / (edge * 1.2));
                    over(&mut layer, color, coverage(rounded_square(p, edge, radius, turn), scale));
                }
            }
            let i = (y * size + x) * 4;
            for c in 0..4 {
                premultiplied[i + c] = (layer[c] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image { size, premultiplied }
}

/// `color` as the light from above leaves it `t` of the way down a shape,
/// 0 at its top and 1 at its bottom: `light` of the way to white at the
/// top, `dark` of the way to black at the bottom.
fn lit(color: [u8; 3], light: f32, dark: f32, t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    color.map(|c| {
        let c = c as f32;
        let top = c + (255.0 - c) * light;
        let bottom = c * (1.0 - dark);
        (top + (bottom - top) * t).round() as u8
    })
}

/// How much of a rim `width` units wide, inside a rounded square at
/// signed distance `distance`, covers the pixel at `p`: all of it where
/// the edge faces up, none where it faces down.
fn top_rim(p: (f32, f32), (edge, radius, turn): (f32, f32, f32), distance: f32, width: f32, scale: f32) -> f32 {
    let inside = coverage(distance, scale) * (1.0 - smoothstep(0.0, width, -distance));
    // Where the edge faces up, the shape moved down is further away.
    let below = rounded_square((p.0, p.1 + 4.0), edge, radius, turn);
    inside * ((below - distance) / 4.0).clamp(0.0, 1.0)
}

/// 0 below `from`, 1 above `to`, and a smooth step between.
fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Paints `color` with alpha `a` over `dst`, all premultiplied, 0 to 1.
fn over(dst: &mut [f32; 4], color: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    for c in 0..3 {
        dst[c] = color[c] as f32 / 255.0 * a + dst[c] * (1.0 - a);
    }
    dst[3] = a + dst[3] * (1.0 - a);
}

/// How much of a pixel `scale` design units wide a shape covers, from the
/// signed distance to its edge at the pixel's center.
fn coverage(distance: f32, scale: f32) -> f32 {
    (0.5 - distance / scale).clamp(0.0, 1.0)
}

/// Signed distance from `p` to a square centered in the design, `edge`
/// long, its corners rounded by `radius`, turned by `turn` degrees.
fn rounded_square(p: (f32, f32), edge: f32, radius: f32, turn: f32) -> f32 {
    let (sin, cos) = (-turn.to_radians()).sin_cos();
    let (dx, dy) = (p.0 - 512.0, p.1 - 512.0);
    let (x, y) = (dx * cos - dy * sin, dx * sin + dy * cos);
    let inner = edge / 2.0 - radius;
    let (qx, qy) = (x.abs() - inner, y.abs() - inner);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    outside + qx.max(qy).min(0.0) - radius
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &Image, x: usize, y: usize) -> [u8; 4] {
        let i = (y * image.size + x) * 4;
        image.premultiplied[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn ids_round_trip() {
        for tint in Tint::ALL {
            assert_eq!(Tint::from_id(tint.id()), Some(tint));
        }
        assert_eq!(Tint::from_id("graphite\n"), Some(Tint::Graphite));
        assert_eq!(Tint::from_id("plaid"), None);
    }

    #[test]
    fn draws_the_tile_and_the_center_square() {
        let image = render(Tint::Green, 64, Layout::Full);
        let palette = Tint::Green.palette();
        // The tile, near the top but under its rim, as lit there: pixel 2
        // is design units 32 to 48.
        let (light, dark) = LIGHT[0];
        assert_eq!(pixel(&image, 32, 2)[..3], lit(palette.tile, light, dark, 40.0 / 1024.0));
        // The lightest square at the center, 504 units of gradient from
        // its top at 260.
        let (light, dark) = LIGHT[2];
        assert_eq!(pixel(&image, 32, 32)[..3], lit(palette.inner, light, dark, (520.0 - 260.0) / 504.0));
        // Outside the rounded corner.
        assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn lights_from_above() {
        let image = render(Tint::Amber, 1024, Layout::Full);
        let luma = |[r, g, b, _]: [u8; 4]| 2126 * r as u32 + 7152 * g as u32 + 722 * b as u32;
        // The tile, and the outer square at its left corner, lighter at
        // the top than at the bottom.
        assert!(luma(pixel(&image, 512, 60)) > luma(pixel(&image, 512, 960)));
        assert!(luma(pixel(&image, 300, 300)) > luma(pixel(&image, 300, 700)));
        // The rim along the tile's top edge.
        assert!(luma(pixel(&image, 512, 4)) > luma(pixel(&image, 512, 60)));
        // The outer square's shadow, just below it, against the tile at
        // the same height away from it.
        assert!(luma(pixel(&image, 512, 866)) < luma(pixel(&image, 60, 866)));
    }

    #[test]
    fn leaves_the_mac_margin_clear() {
        let image = render(Tint::Graphite, 1024, Layout::MacIcon);
        // The margin is 100 of 1024.
        assert_eq!(pixel(&image, 512, 50)[3], 0);
        assert_eq!(pixel(&image, 512, 110)[3], 255);
        // The center, 512 of the design, at 512 pixels as well.
        let (light, dark) = LIGHT[2];
        let palette = Tint::Graphite.palette();
        let center = -100.0 * 1024.0 / 824.0 + 512.5 * 1024.0 / 824.0;
        assert_eq!(pixel(&image, 512, 512)[..3], lit(palette.inner, light, dark, (center - 260.0) / 504.0));
    }

    #[test]
    fn unmultiplies_edges() {
        let image = Image { size: 1, premultiplied: vec![64, 32, 0, 128] };
        assert_eq!(image.unmultiplied(), [128, 64, 0, 128]);
    }
}
