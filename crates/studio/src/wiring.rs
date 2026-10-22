//! The wires of a circuit in the 3D view: a tube along each wire's route,
//! in its color, a dot on each pin, and the legend of the colors.
//!
//! Wires and pins come from the circuit file alone, so they show as soon
//! as it is read, before the parts render. They are drawn, never modeled:
//! they never reach the geometry kernel or an export.

use std::collections::HashSet;
use std::sync::Arc;

use c2f_circuit::{Circuit, End, route};
use c2f_geom::RenderMesh;
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::circuit::parse_color;
use crate::theme;

/// The radius of a wire's tube, in millimeters.
pub const WIRE_RADIUS: f64 = 0.4;
/// Sides of a wire's tube.
const SIDES: usize = 8;
/// The radius of a pin's dot: a wired pin shows larger.
pub const PIN_RADIUS: f32 = 0.45;
pub const WIRED_PIN_RADIUS: f32 = 0.75;

/// A wire without a color.
const DEFAULT_WIRE: [f32; 4] = [0.78, 0.80, 0.84, 1.0];
const PIN: [f32; 4] = [0.42, 0.44, 0.48, 1.0];
const WIRED_PIN: [f32; 4] = [0.96, 0.96, 0.92, 1.0];
const ERROR_PIN: [f32; 4] = [1.0, 0.22, 0.20, 1.0];

/// How a wire or a pin shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Emphasis {
    #[default]
    Normal,
    /// Picked out: brighter and thicker.
    Strong,
    /// Greyed out, behind what is picked out.
    Dim,
}

/// How each wire and pin shows: by default, all as they are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// By wire index; missing ones are normal.
    pub wires: Vec<Emphasis>,
    /// Pins picked out; the others are dimmed when `dim_pins` is set.
    pub strong_pins: HashSet<End>,
    pub dim_pins: bool,
    /// The strong pins pulse: 0 to 1, the share of their extra size.
    pub pulse: Option<f32>,
    /// By part index; missing ones are normal.
    pub parts: Vec<Emphasis>,
    /// Pins shown in red, picked out: a drop there would break a check.
    pub error_pins: HashSet<End>,
}

impl Look {
    fn wire(&self, index: usize) -> Emphasis {
        self.wires.get(index).copied().unwrap_or_default()
    }

    fn pin(&self, end: End) -> Emphasis {
        if self.strong_pins.contains(&end) {
            Emphasis::Strong
        } else if self.dim_pins {
            Emphasis::Dim
        } else {
            Emphasis::Normal
        }
    }
}

/// A circuit's wires, laid out once per read of the circuit.
#[derive(Clone)]
pub struct Wiring {
    pub circuit: Arc<Circuit>,
    /// The curve of each wire, in the order of the circuit's wires.
    pub paths: Vec<Vec<[f64; 3]>>,
    pub colors: Vec<[f32; 4]>,
    /// The length of each curve, in millimeters.
    pub lengths: Vec<f64>,
    /// Pins that have a wire.
    pub wired: HashSet<End>,
    /// Each wire's name for the assembly's progress.
    pub names: Vec<String>,
}

impl Wiring {
    pub fn new(circuit: Arc<Circuit>) -> Self {
        let paths = route::paths(&circuit);
        let colors = circuit.wires.iter().map(|w| wire_color(w.color.as_deref())).collect();
        let wired = circuit.wires.iter().flat_map(|w| [w.from, w.to]).collect();
        let lengths = paths.iter().map(|path| route::length_of(path)).collect();
        let names = crate::mount::wire_names(&circuit);
        Wiring { circuit, paths, colors, lengths, wired, names }
    }

    /// Every pin of every part.
    pub fn pins(&self) -> impl Iterator<Item = End> + '_ {
        self.circuit
            .parts
            .iter()
            .enumerate()
            .flat_map(|(part, p)| (0..p.annotations.pins.len()).map(move |pin| End { part, pin }))
    }

    /// The radius a pin's dot is drawn with, as picking finds it.
    pub fn pin_radius(&self, end: End) -> f32 {
        if self.wired.contains(&end) { WIRED_PIN_RADIUS } else { PIN_RADIUS }
    }

    /// The tubes of the wires, and the dots of the pins.
    pub fn meshes(&self, look: &Look) -> (RenderMesh, RenderMesh) {
        let mut wires = RenderMesh::default();
        for (index, path) in self.paths.iter().enumerate() {
            let emphasis = look.wire(index);
            // Thicker when picked out, thinner when greyed out: a black
            // wire picked out stays apart from the grey ones.
            let radius = match emphasis {
                Emphasis::Strong => WIRE_RADIUS * 1.6,
                Emphasis::Dim => WIRE_RADIUS * 0.6,
                Emphasis::Normal => WIRE_RADIUS,
            };
            let color = shade(self.colors[index], emphasis);
            push_indexed(&mut wires, &route::tube(path, radius, SIDES), color);
        }
        let mut pins = RenderMesh::default();
        for end in self.pins() {
            let wired = self.wired.contains(&end);
            let emphasis = look.pin(end);
            let mut radius = self.pin_radius(end);
            if emphasis == Emphasis::Strong {
                radius *= 1.5 + look.pulse.unwrap_or(0.0);
            }
            let base = if look.error_pins.contains(&end) {
                radius = self.pin_radius(end) * 1.8;
                ERROR_PIN
            } else if wired {
                WIRED_PIN
            } else {
                PIN
            };
            let position = self.circuit.place(end).position.map(|c| c as f32);
            push_sphere(&mut pins, position, radius, shade(base, emphasis));
        }
        (wires, pins)
    }

    /// The box around the wires and pins.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let points = self.paths.iter().flatten().copied().chain(self.pins().map(|end| self.circuit.place(end).position));
        points.map(|p| p.map(|c| c as f32)).fold(None, |acc, p| {
            Some(match acc {
                None => (p, p),
                Some((lo, hi)) => (std::array::from_fn(|i| lo[i].min(p[i])), std::array::from_fn(|i| hi[i].max(p[i]))),
            })
        })
    }

    /// The legend: each color in the order the wires first use it, with
    /// the labels its wires carry.
    pub fn legend(&self) -> Vec<LegendEntry> {
        let mut entries: Vec<LegendEntry> = Vec::new();
        for (wire, &color) in self.circuit.wires.iter().zip(&self.colors) {
            let name = wire.color.clone().unwrap_or_else(|| "no color".to_string());
            let at = match entries.iter().position(|e| e.name.eq_ignore_ascii_case(&name)) {
                Some(at) => at,
                None => {
                    entries.push(LegendEntry { color, name, labels: Vec::new(), wires: 0 });
                    entries.len() - 1
                }
            };
            let entry = &mut entries[at];
            entry.wires += 1;
            if let Some(label) = wire.label.as_ref().filter(|l| !entry.labels.contains(l)) {
                entry.labels.push(label.clone());
            }
        }
        entries
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntry {
    pub color: [f32; 4],
    /// As the circuit writes it.
    pub name: String,
    pub labels: Vec<String>,
    pub wires: usize,
}

/// An egui color for an RGBA color in 0..1.
pub fn color32(c: [f32; 4]) -> Color32 {
    let [r, g, b, a] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The legend, in the top-right corner of the view, `right` and `top`
/// points from its edges: a swatch per color, its name and its wires' labels, on a dark
/// card that reads the same whatever is behind it.
/// Returns the card's rectangle, for what goes under it.
pub fn paint_legend(painter: &egui::Painter, rect: Rect, right: f32, top: f32, entries: &[LegendEntry]) -> Rect {
    if entries.is_empty() {
        return Rect::from_min_size(Pos2::new(rect.right() - right, rect.top() + top), Vec2::ZERO);
    }
    let p = theme::palette();
    let font = FontId::proportional(12.0);
    let rows: Vec<_> = entries
        .iter()
        .map(|entry| {
            let name = painter.layout_no_wrap(entry.name.clone(), font.clone(), p.overlay_text);
            let labels = painter.layout_no_wrap(entry.labels.join(", "), font.clone(), p.overlay_text_dim);
            (entry, name, labels)
        })
        .collect();
    let title = painter.layout_no_wrap("Wires".to_string(), FontId::proportional(12.5), p.overlay_text);
    let name_width = rows.iter().map(|(_, n, _)| n.size().x).fold(0.0, f32::max);
    let labels_width = rows.iter().map(|(_, _, l)| l.size().x).fold(0.0, f32::max);
    let row_height = 18.0;
    let width = (22.0 + name_width + 12.0 + labels_width).max(title.size().x) + 20.0;
    let height = 30.0 + rows.len() as f32 * row_height + 6.0;
    let card = Rect::from_min_size(Pos2::new(rect.right() - right - width, rect.top() + top), Vec2::new(width, height));
    let border = Stroke::new(1.0, p.line(20));
    painter.rect(card, CornerRadius::same(8), p.veil(170), border, StrokeKind::Inside);
    painter.galley(card.min + Vec2::new(10.0, 8.0), title, p.overlay_text);
    for (i, (entry, name, labels)) in rows.into_iter().enumerate() {
        let y = card.top() + 30.0 + i as f32 * row_height + row_height / 2.0;
        let swatch = Pos2::new(card.left() + 16.0, y);
        painter.circle(swatch, 5.0, color32(entry.color), Stroke::new(1.0, p.line(110)));
        painter.galley(Pos2::new(card.left() + 28.0, y - name.size().y / 2.0), name, p.overlay_text);
        painter.galley(
            Pos2::new(card.left() + 28.0 + name_width + 12.0, y - labels.size().y / 2.0),
            labels,
            p.overlay_text_dim,
        );
    }
    card
}

/// A wire's color, or the default for none or one not understood.
pub fn wire_color(name: Option<&str>) -> [f32; 4] {
    name.and_then(parse_color).map_or(DEFAULT_WIRE, |c| [c[0], c[1], c[2], 1.0])
}

/// A color as an emphasis shows it: greyed out, it fades towards the
/// theme's grey.
pub fn shade(color: [f32; 4], emphasis: Emphasis) -> [f32; 4] {
    let grey = theme::palette().dim;
    match emphasis {
        Emphasis::Normal => color,
        // Lifts dark colors (black wires) so that they still stand out.
        Emphasis::Strong => [0, 1, 2, 3].map(|i| if i == 3 { 1.0 } else { color[i] * 0.85 + 0.15 }),
        Emphasis::Dim => [0, 1, 2, 3].map(|i| if i == 3 { 1.0 } else { color[i] * 0.25 + grey[i] * 0.75 }),
    }
}

/// Appends an indexed mesh with smooth normals as plain triangles.
fn push_indexed(out: &mut RenderMesh, tube: &route::Tube, color: [f32; 4]) {
    for &i in &tube.indices {
        out.positions.push(tube.positions[i as usize]);
        out.normals.push(tube.normals[i as usize]);
        out.colors.push(color);
    }
}

/// A small UV sphere with smooth normals.
fn push_sphere(out: &mut RenderMesh, center: [f32; 3], radius: f32, color: [f32; 4]) {
    const RINGS: usize = 6;
    const SEGMENTS: usize = 10;
    let point = |ring: usize, segment: usize| {
        let theta = ring as f32 / RINGS as f32 * std::f32::consts::PI;
        let phi = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()]
    };
    for ring in 0..RINGS {
        for segment in 0..SEGMENTS {
            let [a, b, c, d] = [(ring, segment), (ring, segment + 1), (ring + 1, segment + 1), (ring + 1, segment)]
                .map(|(r, s)| point(r, s));
            for n in [a, d, c, a, c, b] {
                out.positions.push(std::array::from_fn(|i| center[i] + n[i] * radius));
                out.normals.push(n);
                out.colors.push(color);
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Two boards side by side: wires a.A to b.A (red, "5 V"), a.A to
    /// b.B (red, "5 V"), and a.B to b.B (no color).
    pub fn circuit() -> Arc<Circuit> {
        // Written once: tests run in parallel and read it.
        static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("code2form-wiring-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let board = "// @pin A [0, 0, 1] up \"first\"\n// @pin B [5, 0, 1] up\n// @pin C [10, 0, 1] up\ncube([10, 5, 1]);\n";
            std::fs::write(dir.join("board.c2f"), board).unwrap();
            dir
        });
        let source = r#"
[parts.a]
part = "board.c2f"

[parts.b]
part = "board.c2f"
at = [0, 30, 0]

[[wires]]
from = "a.A"
to = "b.A"
color = "red"
label = "5 V"

[[wires]]
from = "a.A"
to = "b.B"
color = "Red"
label = "5 V"

[[wires]]
from = "a.B"
to = "b.B"
"#;
        Arc::new(c2f_circuit::parse(source, &dir.join("test.c2fc"), &[]).unwrap())
    }

    #[test]
    fn wires_are_tubes_in_their_color_and_pins_are_dots() {
        let wiring = Wiring::new(circuit());
        let (wires, pins) = wiring.meshes(&Look::default());
        let per_wire = wires.positions.len() / 3;
        assert_eq!(wires.positions.len(), 3 * per_wire);
        assert!(wires.colors[..per_wire].iter().all(|c| *c == [1.0, 0.0, 0.0, 1.0]));
        assert!(wires.colors[2 * per_wire..].iter().all(|c| *c == DEFAULT_WIRE));
        // Six pins, the four wired ones larger.
        let per_pin = pins.positions.len() / 6;
        let size = |i: usize| {
            let r = i * per_pin..(i + 1) * per_pin;
            pins.positions[r].iter().map(|p| p[2]).fold(f32::MIN, f32::max) - 1.0
        };
        assert!((size(0) - WIRED_PIN_RADIUS).abs() < 1e-5);
        assert!((size(2) - PIN_RADIUS).abs() < 1e-5, "C has no wire");

        // Two wires from a.A leave it apart.
        let mid = |i: usize| wiring.paths[i][wiring.paths[i].len() / 2];
        let (m0, m1) = (mid(0), mid(1));
        assert!((0..3).map(|k| (m0[k] - m1[k]).powi(2)).sum::<f64>().sqrt() > 2.0 * WIRE_RADIUS);
        let (lo, hi) = wiring.bounds().unwrap();
        assert!(lo[1] <= 0.0 && hi[1] >= 30.0);
    }

    #[test]
    fn the_legend_groups_wires_by_color() {
        let legend = Wiring::new(circuit()).legend();
        let entries: Vec<(&str, Vec<&str>, usize)> =
            legend.iter().map(|e| (e.name.as_str(), e.labels.iter().map(String::as_str).collect(), e.wires)).collect();
        assert_eq!(entries, [("red", vec!["5 V"], 2), ("no color", vec![], 1)]);
    }

    #[test]
    fn emphasis_brightens_or_greys() {
        assert_eq!(shade([1.0, 0.0, 0.0, 1.0], Emphasis::Normal), [1.0, 0.0, 0.0, 1.0]);
        let strong = shade([0.0, 0.0, 0.0, 1.0], Emphasis::Strong);
        assert!(strong[0] > 0.1, "a black wire picked out still shows");
        let dim = shade([1.0, 0.0, 0.0, 1.0], Emphasis::Dim);
        assert!(dim[0] < 0.6 && (dim[0] - dim[1]).abs() < 0.3);
        assert_eq!(wire_color(Some("#0000ff")), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(wire_color(Some("not a color")), DEFAULT_WIRE);
        assert_eq!(wire_color(None), DEFAULT_WIRE);
    }
}
