//! The app icon: three rounded squares, each smaller one turned a little
//! further, on a dark tile, in one of ten tints the user picks in the
//! settings. Graphite is the default and the icon the app bundle ships.
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

/// The tile and the three squares, darkest to lightest.
struct Palette {
    tile: [u8; 3],
    outer: [u8; 3],
    middle: [u8; 3],
    inner: [u8; 3],
}

const fn hex(rgb: u32) -> [u8; 3] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

const fn palette(tile: u32, outer: u32, middle: u32, inner: u32) -> Palette {
    Palette { tile: hex(tile), outer: hex(outer), middle: hex(middle), inner: hex(inner) }
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

    pub fn name(self) -> &'static str {
        match self {
            Tint::Graphite => "Graphite",
            Tint::Green => "Green",
            Tint::Red => "Red",
            Tint::Orange => "Orange",
            Tint::Amber => "Amber",
            Tint::Teal => "Teal",
            Tint::Cyan => "Cyan",
            Tint::Indigo => "Indigo",
            Tint::Violet => "Violet",
            Tint::Pink => "Pink",
        }
    }

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

    fn palette(self) -> Palette {
        match self {
            Tint::Graphite => palette(0x101113, 0x2b2e35, 0x8a909c, 0xe6e8ec),
            Tint::Green => palette(0x07140d, 0x0f3d24, 0x22c55e, 0xa7f3c4),
            Tint::Red => palette(0x1a0809, 0x5c1418, 0xef3b45, 0xffb3b8),
            Tint::Orange => palette(0x1a0d05, 0x5c2a0e, 0xff7a1a, 0xffc79a),
            Tint::Amber => palette(0x171204, 0x5a4108, 0xf5b800, 0xffe58a),
            Tint::Teal => palette(0x041513, 0x0d4a44, 0x14b8a6, 0x9ef0e4),
            Tint::Cyan => palette(0x04121a, 0x0b3d5c, 0x0ea5e9, 0xa5e4ff),
            Tint::Indigo => palette(0x0c0b22, 0x26216e, 0x6366f1, 0xc4c6ff),
            Tint::Violet => palette(0x120a1f, 0x3b1a6b, 0x9b5cf6, 0xdcc6ff),
            Tint::Pink => palette(0x1a0712, 0x5e1440, 0xec4899, 0xffc1e0),
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
const SQUARES: [(f32, f32, f32); 3] = [(720.0, 86.0, 0.0), (528.0, 64.0, 15.0), (336.0, 41.0, 30.0)];

/// Draws the icon `size` pixels square.
pub fn render(tint: Tint, size: usize, layout: Layout) -> Image {
    let palette = tint.palette();
    let colors = [palette.outer, palette.middle, palette.inner];
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
            let tile = coverage(rounded_square(p, 1024.0, TILE_RADIUS, 0.0), scale);
            over(&mut layer, palette.tile, tile);
            for (&(edge, radius, turn), color) in SQUARES.iter().zip(colors) {
                over(&mut layer, color, coverage(rounded_square(p, edge, radius, turn), scale));
            }
            let i = (y * size + x) * 4;
            for c in 0..4 {
                premultiplied[i + c] = (layer[c] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image { size, premultiplied }
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
        // The tile, near a corner but inside its rounding.
        assert_eq!(pixel(&image, 32, 2)[..3], palette.tile);
        // The lightest square at the center.
        assert_eq!(pixel(&image, 32, 32)[..3], palette.inner);
        // Outside the rounded corner.
        assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn leaves_the_mac_margin_clear() {
        let image = render(Tint::Graphite, 1024, Layout::MacIcon);
        // The margin is 100 of 1024.
        assert_eq!(pixel(&image, 512, 50)[3], 0);
        assert_eq!(pixel(&image, 512, 110)[3], 255);
        assert_eq!(pixel(&image, 512, 512)[..3], Tint::Graphite.palette().inner);
    }

    #[test]
    fn unmultiplies_edges() {
        let image = Image { size: 1, premultiplied: vec![64, 32, 0, 128] };
        assert_eq!(image.unmultiplied(), [128, 64, 0, 128]);
    }
}
