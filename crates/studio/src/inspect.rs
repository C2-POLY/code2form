//! What the view says about a pin, a wire or a part of a circuit: the
//! bubble under the pointer, and the card of the selection.

use c2f_circuit::{Circuit, End};
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::pick::{Hit, net};
use crate::wiring::{TEXT, TEXT_DIM};

/// `xiao · D3 · GPIO4, A3`: the part, the pin and its label.
pub fn pin_name(circuit: &Circuit, end: End) -> String {
    let pin = circuit.pin(end);
    let mut text = format!("{} · {}", circuit.parts[end.part].name, pin.name);
    if let Some(label) = &pin.label {
        text.push_str(" · ");
        text.push_str(label);
    }
    text
}

/// `xiao D3 (GPIO4, A3)`: a wire's end, as an instruction gives it.
///
/// Arrows between ends are `➡`: egui's default fonts have no `→`.
pub fn end_text(circuit: &Circuit, end: End) -> String {
    let pin = circuit.pin(end);
    let mut text = format!("{} {}", circuit.parts[end.part].name, pin.name);
    if let Some(label) = &pin.label {
        text.push_str(&format!(" ({label})"));
    }
    text
}

/// `Blue wire` or `Wire`, then its label, if any.
pub fn wire_title(circuit: &Circuit, index: usize) -> String {
    let wire = &circuit.wires[index];
    let mut text = match &wire.color {
        Some(color) => format!("{} wire", capitalize(color)),
        None => "Wire".to_string(),
    };
    if let Some(label) = &wire.label {
        text.push_str(" · ");
        text.push_str(label);
    }
    text
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// How much wire to cut: the length routed and the margin, which
/// [`Lengths::of`] adds up.
#[derive(Clone, Copy, Debug)]
pub struct Lengths<'a> {
    /// Each wire's routed length, in millimeters.
    pub routed: &'a [f64],
    pub margin: f64,
}

impl Lengths<'_> {
    pub fn of(&self, wire: usize) -> f64 {
        self.routed.get(wire).copied().unwrap_or(0.0) + self.margin
    }
}

/// The one line shown next to the pointer.
pub fn bubble(circuit: &Circuit, hit: Hit, lengths: Lengths) -> String {
    match hit {
        Hit::Pin(end) => pin_name(circuit, end),
        Hit::Wire(index) => {
            let wire = &circuit.wires[index];
            let (title, from, to) = (wire_title(circuit, index), circuit.end_name(wire.from), circuit.end_name(wire.to));
            format!("{title}: {from} ➡ {to} · {:.0} mm", lengths.of(index))
        }
        Hit::Part(index) => {
            let part = &circuit.parts[index];
            format!("{} · {}", part.name, part.part)
        }
    }
}

/// The lines of the selection's card: a title, then details. `source` is
/// the circuit's text, for the line a wire is written on.
pub fn card(circuit: &Circuit, hit: Hit, source: &str, lengths: Lengths) -> (String, Vec<String>) {
    match hit {
        Hit::Pin(end) => {
            let pin = circuit.pin(end);
            let mut kind = pin.kind.name().to_string();
            if let Some(volts) = pin.voltage {
                kind.push_str(&format!(" · {volts} V max"));
            }
            if pin.reserved {
                kind.push_str(" · reserved");
            }
            let (pins, wires) = net(circuit, end);
            let connected = if wires.is_empty() {
                "Not wired".to_string()
            } else {
                let mut others: Vec<String> =
                    pins.iter().filter(|&&e| e != end).map(|&e| circuit.end_name(e)).collect();
                others.sort();
                format!("Net: {} ({} wire{})", others.join(", "), wires.len(), if wires.len() == 1 { "" } else { "s" })
            };
            (pin_name(circuit, end), vec![kind, connected])
        }
        Hit::Wire(index) => {
            let wire = &circuit.wires[index];
            let line = source.as_bytes()[..(wire.span.start as usize).min(source.len())]
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            let ends = format!("{} ➡ {}", end_text(circuit, wire.from), end_text(circuit, wire.to));
            let length = format!(
                "About {:.0} mm of wire: {:.0} mm routed + {} mm margin",
                lengths.of(index),
                lengths.routed.get(index).copied().unwrap_or(0.0),
                crate::pause_edit::number(lengths.margin as f32)
            );
            (wire_title(circuit, index), vec![ends, length, format!("Line {} of the circuit", line + 1)])
        }
        Hit::Part(index) => {
            let part = &circuit.parts[index];
            let defines = part.defines();
            let params = if defines.is_empty() { "No parameters".to_string() } else { defines.join(", ") };
            let file = part.path.file_name().map_or_else(|| part.part.clone(), |n| n.to_string_lossy().into_owned());
            (format!("{} · {}", part.name, part.part), vec![params, format!("Double-click to open {file}")])
        }
    }
}

/// The bubble next to the pointer, kept inside the view.
pub fn paint_bubble(painter: &egui::Painter, rect: Rect, pointer: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.5), TEXT);
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let inside = rect.shrink(6.0);
    let min = Pos2::new(
        (pointer.x + 14.0).min(inside.right() - size.x).max(inside.left()),
        (pointer.y + 18.0).min(inside.bottom() - size.y).max(inside.top()),
    );
    let bubble = Rect::from_min_size(min, size);
    let border = Stroke::new(1.0, Color32::from_white_alpha(40));
    painter.rect(bubble, CornerRadius::same(6), Color32::from_black_alpha(200), border, StrokeKind::Inside);
    painter.galley(bubble.min + Vec2::new(8.0, 4.0), galley, TEXT);
}

/// The selection's card, in the top-left corner of the view, under the
/// status badges: on a dark card, so that it reads whatever is behind.
pub fn paint_card(painter: &egui::Painter, rect: Rect, title: &str, lines: &[String]) {
    let title = painter.layout_no_wrap(title.to_string(), FontId::proportional(13.5), TEXT);
    let lines: Vec<_> =
        lines.iter().map(|l| painter.layout_no_wrap(l.clone(), FontId::proportional(12.0), TEXT_DIM)).collect();
    let width = lines.iter().map(|g| g.size().x).fold(title.size().x, f32::max) + 24.0;
    let height = title.size().y + lines.iter().map(|g| g.size().y + 3.0).sum::<f32>() + 22.0;
    let card = Rect::from_min_size(rect.left_top() + Vec2::new(12.0, 44.0), Vec2::new(width, height));
    let border = Stroke::new(1.0, Color32::from_white_alpha(28));
    painter.rect(card, CornerRadius::same(8), Color32::from_black_alpha(185), border, StrokeKind::Inside);
    let mut y = card.top() + 9.0;
    let title_height = title.size().y;
    painter.galley(Pos2::new(card.left() + 12.0, y), title, TEXT);
    y += title_height + 6.0;
    for line in lines {
        let h = line.size().y;
        painter.galley(Pos2::new(card.left() + 12.0, y), line, TEXT_DIM);
        y += h + 3.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::tests::circuit;

    #[test]
    fn says_what_a_pin_a_wire_and_a_part_are() {
        let circuit = circuit();
        let a = End { part: 0, pin: 0 };
        let lengths = Lengths { routed: &[40.2, 41.0, 37.6], margin: 10.0 };
        assert_eq!(bubble(&circuit, Hit::Pin(a), lengths), "a · A · first");
        assert_eq!(bubble(&circuit, Hit::Pin(End { part: 0, pin: 1 }), lengths), "a · B");
        assert_eq!(bubble(&circuit, Hit::Wire(0), lengths), "Red wire · 5 V: a.A ➡ b.A · 50 mm");
        assert_eq!(bubble(&circuit, Hit::Wire(2), lengths), "Wire: a.B ➡ b.B · 48 mm");
        assert_eq!(bubble(&circuit, Hit::Part(1), lengths), "b · board.c2f");

        let (title, lines) = card(&circuit, Hit::Pin(a), "", lengths);
        assert_eq!(title, "a · A · first");
        assert_eq!(lines, ["passive", "Net: a.B, b.A, b.B (3 wires)"]);
        let (_, lines) = card(&circuit, Hit::Pin(End { part: 0, pin: 2 }), "", lengths);
        assert_eq!(lines[1], "Not wired");
        let source = "x\ny\n\nfrom = \"a.A\"";
        let mut circuit = (*circuit).clone();
        circuit.wires[0].span = c2f_lang::Span::new(5, 9);
        let (title, lines) = card(&circuit, Hit::Wire(0), source, Lengths { margin: 12.5, ..lengths });
        assert_eq!(title, "Red wire · 5 V");
        let length = "About 53 mm of wire: 40 mm routed + 12.5 mm margin";
        assert_eq!(lines, ["a A (first) ➡ b A (first)", length, "Line 4 of the circuit"]);
        let (_, lines) = card(&circuit, Hit::Part(0), "", lengths);
        assert_eq!(lines, ["No parameters", "Double-click to open board.c2f"]);
    }
}
