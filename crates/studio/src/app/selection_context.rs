//! What the user selects goes to the agent with the message: a part, a
//! pin or a wire of a circuit, a face of a design and its `part()`, or
//! lines of code, each as the text the agent needs to act without asking
//! (what it is, its file and lines, the code itself, and where it is in
//! millimeters); and a picture of the view, taken when the message is
//! sent.
//!
//! The Agent panel shows the selection as a chip above its field, sent
//! with the next message unless the user removes it: a [`Suggestion`]
//! follows the selection until then. A right-click in the view asks the
//! agent about what is under the pointer.

use std::path::Path;

use c2f_agent::conversation::Attachment;
use c2f_circuit::{Circuit, End};
use eframe::egui;

use super::{Arrangement, StudioApp, live};
use crate::i18n::tr;
use crate::pause_edit::number;
use crate::pick::{self, Hit};
use crate::viewer::FaceHit;

/// The most lines of code a context quotes.
const MAX_LINES: usize = 12;

/// What is selected, offered to go with the next message: shown until the
/// user dismisses it, which holds until the selection changes; sent once.
#[derive(Debug, Default)]
pub struct Suggestion {
    current: Option<Attachment>,
    dismissed: bool,
}

impl Suggestion {
    /// Follows the selection, `now`, every frame: another selection (told
    /// apart by its label) is offered again.
    pub fn follow(&mut self, now: Option<Attachment>) {
        if self.current.as_ref().map(label) != now.as_ref().map(label) {
            self.dismissed = false;
        }
        self.current = now;
    }

    /// What the chip shows, unless dismissed.
    pub fn shown(&self) -> Option<&Attachment> {
        self.current.as_ref().filter(|_| !self.dismissed)
    }

    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// What goes with the message being sent; the chip goes with it.
    pub fn take(&mut self) -> Option<Attachment> {
        let shown = self.shown().cloned();
        self.dismissed = true;
        shown
    }
}

/// The label of an attachment: what its chip says.
pub fn label(attachment: &Attachment) -> &str {
    match attachment {
        Attachment::Context { label, .. } => label,
        Attachment::Image { .. } => "",
    }
}

// ---- The contexts ----

/// The file, as the agent's tools take it.
fn file(path: Option<&Path>) -> String {
    path.map_or_else(|| "the untitled file open in the studio".into(), |p| p.display().to_string())
}

/// The line (from 1) of the byte at `offset`.
fn line_of(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset.min(source.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// Lines `first` to `last` (from 1) of `source`, numbered, in a fence; at
/// most [`MAX_LINES`] of them.
fn snippet(source: &str, first: usize, last: usize) -> String {
    let mut text = String::from("```\n");
    let count = last.saturating_sub(first) + 1;
    for (index, line) in source.lines().enumerate().skip(first.saturating_sub(1)).take(count.min(MAX_LINES)) {
        text.push_str(&format!("{:>4} | {line}\n", index + 1));
    }
    if count > MAX_LINES {
        text.push_str("   … |\n");
    }
    text.push_str("```");
    text
}

/// `first–last`, or `first` alone.
fn range(first: usize, last: usize) -> String {
    if first == last { first.to_string() } else { format!("{first}–{last}") }
}

/// `[x, y, z]`, rounded.
fn vector(v: [f64; 3]) -> String {
    format!("[{}, {}, {}]", number(v[0] as f32), number(v[1] as f32), number(v[2] as f32))
}

/// Lines `first` to `last` (from 1) of the code selected in the editor.
pub fn lines_context(path: Option<&Path>, source: &str, first: usize, last: usize) -> Attachment {
    let lines = range(first, last);
    let label = if first == last { tr!("Line {lines}", "Ligne {lines}") } else { tr!("Lines {lines}", "Lignes {lines}") };
    let text = format!(
        "The user selected line{} {lines} of {} in the code editor:\n{}",
        if first == last { "" } else { "s" },
        file(path),
        snippet(source, first, last)
    );
    Attachment::Context { label, text }
}

/// The lines (from 1) of the TOML table holding the byte at `offset`: from
/// its header to its last line before the next header, without the blank
/// lines and comments that come before that.
fn table_lines(source: &str, offset: usize) -> (usize, usize) {
    let lines: Vec<&str> = source.lines().collect();
    let at = line_of(source, offset).min(lines.len().max(1));
    let header = |line: &&str| line.trim_start().starts_with('[');
    let first = (1..=at).rev().find(|&n| lines.get(n - 1).is_some_and(header)).unwrap_or(at);
    let next = (at + 1..=lines.len()).find(|&n| header(&lines[n - 1])).unwrap_or(lines.len() + 1);
    let mut last = next - 1;
    while last > at && lines.get(last - 1).is_none_or(|l| l.trim().is_empty() || l.trim_start().starts_with('#')) {
        last -= 1;
    }
    (first, last)
}

/// The table of the circuit file a byte is in, as a line of the context.
fn written(source: &str, offset: usize, circuit_file: &str) -> String {
    let (first, last) = table_lines(source, offset);
    format!("- Written at lines {} of {circuit_file}:\n{}", range(first, last), snippet(source, first, last))
}

/// A part, a pin or a wire of a circuit, selected in the view; `source`
/// is the circuit file's text.
pub fn circuit_context(circuit: &Circuit, hit: Hit, path: Option<&Path>, source: &str) -> Attachment {
    let circuit_file = file(path);
    let mut lines = Vec::new();
    let label = match hit {
        Hit::Part(index) => {
            let part = &circuit.parts[index];
            lines.push(format!("The user selected the part `{}` of the circuit {circuit_file}, in the 3D view.", part.name));
            lines.push(format!("- Part file: {} (`part = \"{}\"`)", part.path.display(), part.part));
            lines.push(format!("- Placed at {} mm, rotated {}°", vector(part.at), vector(part.rotate)));
            let defines = part.defines();
            if !defines.is_empty() {
                lines.push(format!("- Parameters: {}", defines.join(", ")));
            }
            let pins: Vec<&str> = part.annotations.pins.iter().map(|p| p.name.as_str()).collect();
            if !pins.is_empty() {
                lines.push(format!("- Pins: {}", pins.join(", ")));
            }
            lines.push(written(source, part.span.start as usize, &circuit_file));
            tr!("Selection: {}", "Sélection : {}", part.name)
        }
        Hit::Pin(end) => {
            let pin = circuit.pin(end);
            let part = &circuit.parts[end.part];
            let named = pin.label.as_ref().map_or_else(String::new, |l| format!(" ({l})"));
            lines.push(format!(
                "The user selected the pin `{}`{named} of the part `{}` in the circuit {circuit_file}, in the 3D view.",
                circuit.end_name(end),
                part.name
            ));
            let mut kind = format!("- Kind: {}", pin.kind.name());
            if let Some(volts) = pin.voltage {
                kind.push_str(&format!(", {} V max", number(volts as f32)));
            }
            if pin.reserved {
                kind.push_str(", reserved (better left unwired)");
            }
            lines.push(kind);
            let placed = circuit.place(end);
            lines.push(format!("- At {} mm in the circuit, a wire leaving it toward {}", vector(placed.position), vector(placed.direction)));
            lines.push(format!("- Part file: {} (`part = \"{}\"`)", part.path.display(), part.part));
            let wires: Vec<String> = circuit
                .wires
                .iter()
                .enumerate()
                .filter(|(_, w)| w.from == end || w.to == end)
                .map(|(i, w)| {
                    format!("wire {i} ({} -> {}, line {})", circuit.end_name(w.from), circuit.end_name(w.to), line_of(source, w.span.start as usize))
                })
                .collect();
            lines.push(if wires.is_empty() { "- Not wired".into() } else { format!("- Wired by {}", wires.join(", ")) });
            lines.push(written(source, part.span.start as usize, &circuit_file));
            tr!("Selection: {}", "Sélection : {}", circuit.end_name(end))
        }
        Hit::Wire(index) => {
            let wire = &circuit.wires[index];
            let end = |end: End| {
                let label = circuit.pin(end).label.as_ref().map_or_else(String::new, |l| format!(" ({l})"));
                format!("`{}`{label}", circuit.end_name(end))
            };
            lines.push(format!(
                "The user selected wire {index} (the [[wires]] table number {} in the file) of the circuit {circuit_file}, in the 3D view: from {} to {}.",
                index + 1,
                end(wire.from),
                end(wire.to)
            ));
            if let Some(color) = &wire.color {
                lines.push(format!("- Color: {color}"));
            }
            if let Some(label) = &wire.label {
                lines.push(format!("- Label: {label}"));
            }
            for (name, at) in [("from", wire.from), ("to", wire.to)] {
                lines.push(format!("- `{name}` end at {} mm in the circuit", vector(circuit.place(at).position)));
            }
            lines.push(written(source, wire.span.start as usize, &circuit_file));
            tr!(
                "Selection: {} ➡ {}",
                "Sélection : {} ➡ {}",
                circuit.end_name(wire.from),
                circuit.end_name(wire.to)
            )
        }
    };
    Attachment::Context { label, text: lines.join("\n") }
}

/// The lines (from 1) where `part("name")` is written in `source`.
fn part_lines(source: &str, name: &str) -> Vec<usize> {
    let quoted = format!("\"{name}\"");
    source
        .match_indices("part(")
        .filter(|(at, call)| {
            // Not the end of a longer name, such as `subpart(`.
            let before = source[..*at].chars().next_back();
            let after = source[at + call.len()..].trim_start();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && after.starts_with(&quoted)
        })
        .map(|(at, _)| line_of(source, at))
        .collect()
}

/// Where a `part("name")` of a design is written, as lines of a context.
fn part_written(source: &str, name: &str, design: &str) -> Vec<String> {
    let lines = part_lines(source, name);
    let Some(&first) = lines.first() else {
        return vec![format!("- `part(\"{name}\")` is not written in {design} itself: in a file it includes or uses")];
    };
    let mut text = vec![format!("- `part(\"{name}\")` is written at line {first} of {design}")];
    if lines.len() > 1 {
        let others: Vec<String> = lines[1..].iter().map(usize::to_string).collect();
        text[0].push_str(&format!(" (and line {})", others.join(", ")));
    }
    text.push(snippet(source, first, first + 2));
    text
}

/// A face of a design picked in the view, and the `part()` it is in.
pub fn face_context(path: Option<&Path>, source: &str, face: &FaceHit) -> Attachment {
    let design = file(path);
    let point = face.point.map(f64::from);
    let normal = face.normal.map(f64::from);
    let of = face.part.as_ref().map_or_else(String::new, |p| format!(" of the part `{p}`"));
    let mut lines = vec![
        format!("The user picked a face{of} of the design {design}, in the 3D view."),
        format!("- Point on the face: {} mm, in the model's coordinates", vector(point)),
        format!("- Normal, out of the solid: {}", vector(normal)),
    ];
    let label = match &face.part {
        Some(part) => {
            lines.extend(part_written(source, part, &design));
            tr!("Face of {part}", "Face de {part}")
        }
        None => tr("Face", "Face").to_string(),
    };
    Attachment::Context { label, text: lines.join("\n") }
}

// ---- In the app ----

/// The mark of the pictures of the view taken for a message.
struct AgentShot;

/// What a right-click in the view asks the agent about.
fn target_id() -> egui::Id {
    egui::Id::new("agent-view-target")
}

impl StudioApp {
    /// Tells the Agent panel what is selected in the view and in the code.
    pub(super) fn suggest_selection(&mut self) {
        if !self.agent.open {
            return;
        }
        let doc = &self.doc;
        let path = doc.path.as_deref();
        let view = match (&doc.wiring, doc.selection) {
            (Some(wiring), Some(hit)) => Some(circuit_context(&wiring.circuit, hit, path, &doc.text)),
            _ => None,
        };
        let code = if self.arrangement == Arrangement::View { None } else { doc.editor.selected_lines(&doc.text) };
        let code = code.map(|(first, last)| lines_context(path, &doc.text, first, last));
        self.agent.suggest(view, code);
    }

    /// What lies under `pos` in the view, as a context: a part, a pin or a
    /// wire of a circuit, or a face of a design.
    fn view_target(&self, rect: egui::Rect, pos: egui::Pos2) -> Option<Attachment> {
        let doc = &self.doc;
        let ray = doc.viewer.ray(rect, pos);
        if let Some(wiring) = &doc.wiring {
            let hit = pick::pick(wiring, &doc.part_boxes, &ray)?;
            return Some(circuit_context(&wiring.circuit, hit, doc.path.as_deref(), &doc.text));
        }
        let face = doc.viewer.pick_face(&ray)?;
        Some(face_context(doc.path.as_deref(), &doc.text, &face))
    }

    /// The view's menu on a right-click over something: Ask the agent.
    pub(super) fn view_menu(&mut self, ctx: &egui::Context, response: &egui::Response) {
        if response.secondary_clicked() {
            let target = response.interact_pointer_pos().and_then(|pos| self.view_target(response.rect, pos));
            ctx.data_mut(|d| d.insert_temp(target_id(), target));
        }
        let Some(target) = ctx.data(|d| d.get_temp::<Option<Attachment>>(target_id())).flatten() else { return };
        response.context_menu(|ui| {
            ui.label(egui::RichText::new(label(&target)).size(11.5).color(crate::theme::palette().text_dim));
            let ask = ui.button(tr("Ask the agent…", "Demander à l'agent…"));
            if ask.on_hover_text(tr("Open the Agent panel with this attached", "Ouvrir le panneau Agent avec ceci joint")).clicked() {
                self.agent.ask_about(target.clone());
                ui.close();
            }
        });
    }

    /// Takes the picture of the view a message waits for: asks for the
    /// window's, then cuts the view from it once it comes.
    pub(super) fn agent_view_shot(&mut self, ctx: &egui::Context) {
        if self.agent.wants_view() {
            if self.arrangement == Arrangement::Code || self.view_rect.width() < 1.0 {
                self.agent.send_with_view(None);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(AgentShot)));
                ctx.request_repaint();
            }
            return;
        }
        if !self.agent.awaits_view() {
            return;
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. }
                    if user_data.data.as_ref().is_some_and(|d| d.downcast_ref::<AgentShot>().is_some()) =>
                {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        if let Some(image) = image {
            self.agent.send_with_view(live::view_png(&image, self.view_rect, ctx.pixels_per_point()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn example(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/nightlight").join(name)
    }

    fn nightlight() -> (Circuit, String, PathBuf) {
        let path = example("nightlight.c2fc");
        let source = std::fs::read_to_string(&path).unwrap();
        let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
        let circuit = c2f_circuit::parse(&source, &path, &[library]).unwrap();
        (circuit, source, path)
    }

    fn parts_of(attachment: &Attachment) -> (&str, &str) {
        match attachment {
            Attachment::Context { label, text } => (label, text),
            Attachment::Image { .. } => panic!("not a context"),
        }
    }

    #[test]
    fn a_circuit_part_says_its_file_place_and_table() {
        let (circuit, source, path) = nightlight();
        let ring = circuit.parts.iter().position(|p| p.name == "ring").unwrap();
        let context = circuit_context(&circuit, Hit::Part(ring), Some(&path), &source);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Selection: ring");
        assert!(text.contains("the part `ring`") && text.contains("nightlight.c2fc"), "{text}");
        assert!(text.contains("neopixel_ring_12.c2f") && text.contains("Placed at [0, 0, 23.8] mm, rotated [0, 0, 180]°"), "{text}");
        assert!(text.contains("- Pins: ") && text.contains("DIN"), "{text}");
        // Its table, from the header to its last value, without the
        // comment of the next one.
        assert!(text.contains("Written at lines 20–23"), "{text}");
        assert!(text.contains("  20 | [parts.ring]") && text.contains("  23 | rotate = [0, 0, 180]"), "{text}");
        assert!(!text.contains("Copper side up"), "{text}");

        let touch = circuit.parts.iter().position(|p| p.name == "touch").unwrap();
        let context = circuit_context(&circuit, Hit::Part(touch), Some(&path), &source);
        let (_, text) = parts_of(&context);
        assert!(text.contains("- Parameters: size=10"), "{text}");
    }

    #[test]
    fn a_pin_says_its_kind_place_and_wires() {
        let (circuit, source, path) = nightlight();
        let xiao = circuit.parts.iter().position(|p| p.name == "xiao").unwrap();
        let d3 = circuit.parts[xiao].pin("D3").unwrap();
        let context = circuit_context(&circuit, Hit::Pin(End { part: xiao, pin: d3 }), Some(&path), &source);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Selection: xiao.D3");
        assert!(text.contains("the pin `xiao.D3`") && text.contains("- Kind: "), "{text}");
        assert!(text.contains(" mm in the circuit, a wire leaving it toward ["), "{text}");
        assert!(text.contains("- Wired by wire 2 (xiao.D3 -> ring.DIN, line 46)"), "{text}");
        assert!(text.contains("  14 | [parts.xiao]"), "{text}");
    }

    #[test]
    fn a_wire_says_its_ends_and_table() {
        let (circuit, source, path) = nightlight();
        let context = circuit_context(&circuit, Hit::Wire(2), Some(&path), &source);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Selection: xiao.D3 ➡ ring.DIN");
        assert!(text.contains("wire 2 (the [[wires]] table number 3 in the file)"), "{text}");
        assert!(text.contains("from `xiao.D3`") && text.contains("to `ring.DIN`"), "{text}");
        assert!(text.contains("- Color: green") && text.contains("- Label: LED data"), "{text}");
        assert!(text.contains("Written at lines 45–49"), "{text}");
        assert!(text.contains("  45 | [[wires]]") && text.contains("  49 | label = \"LED data\""), "{text}");
    }

    #[test]
    fn a_face_says_its_point_normal_and_part() {
        let path = example("nightlight.c2f");
        let source = std::fs::read_to_string(&path).unwrap();
        let face = FaceHit { point: [1.25, -2.0, 30.0], normal: [0.0, 0.0, 1.0], part: Some("Top".into()) };
        let context = face_context(Some(&path), &source, &face);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Face of Top");
        assert!(text.contains("a face of the part `Top` of the design ") && text.contains("nightlight.c2f"), "{text}");
        assert!(text.contains("- Point on the face: [1.25, -2, 30] mm"), "{text}");
        assert!(text.contains("- Normal, out of the solid: [0, 0, 1]"), "{text}");
        assert!(text.contains("`part(\"Top\")` is written at line 50 of ") && text.contains("(and line 55)"), "{text}");
        assert!(text.contains("  50 | ") && text.contains("  52 | "), "{text}");

        // A face outside any part, in an untitled design.
        let face = FaceHit { part: None, ..face };
        let context = face_context(None, "cube(10);", &face);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Face");
        assert!(text.contains("of the design the untitled file") && !text.contains("part("), "{text}");
    }

    #[test]
    fn a_part_is_found_by_its_call() {
        let source = "part(\"a\") cube(1);\nsubpart(\"a\") x();\n  part( \"a\" ) y();\npart(\"ab\") z();";
        assert_eq!(part_lines(source, "a"), [1, 3]);
        assert_eq!(part_lines(source, "ab"), [4]);
        let lines = part_written(source, "c", "box.c2f");
        assert!(lines[0].contains("not written in box.c2f"), "{lines:?}");
    }

    #[test]
    fn selected_lines_quote_the_code() {
        let path = example("nightlight.c2f");
        let source = std::fs::read_to_string(&path).unwrap();
        let context = lines_context(Some(&path), &source, 49, 50);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Lines 49–50");
        assert!(text.starts_with("The user selected lines 49–50 of ") && text.contains("in the code editor"), "{text}");
        assert!(text.contains("  49 |     part(\"Base\")") && text.contains("  50 | "), "{text}");
        assert!(!text.contains("  51 | "), "{text}");
        let context = lines_context(None, "a\nb\n", 2, 2);
        let (label, text) = parts_of(&context);
        assert_eq!(label, "Line 2");
        assert!(text.contains("line 2 of the untitled") && text.ends_with("   2 | b\n```"), "{text}");
        // A long selection is cut.
        let long = lines_context(None, &"x\n".repeat(40), 1, 40);
        assert!(text_of(&long).contains("  12 | x\n   … |") && !text_of(&long).contains("  13 |"));
    }

    fn text_of(attachment: &Attachment) -> &str {
        parts_of(attachment).1
    }

    #[test]
    fn a_suggestion_follows_the_selection_until_sent() {
        let context = |label: &str| Attachment::Context { label: label.into(), text: format!("{label}…") };
        let mut s = Suggestion::default();
        assert_eq!(s.shown(), None);
        s.follow(Some(context("Selection: ring")));
        assert_eq!(s.shown(), Some(&context("Selection: ring")));
        // It updates with the selection.
        s.follow(Some(context("Selection: xiao")));
        assert_eq!(s.shown().map(label), Some("Selection: xiao"));
        // Dismissed, it stays so while the selection stays, its text
        // updated or not.
        s.dismiss();
        s.follow(Some(Attachment::Context { label: "Selection: xiao".into(), text: "edited".into() }));
        assert_eq!(s.shown(), None);
        // Another selection is offered again.
        s.follow(Some(context("Selection: ring")));
        assert_eq!(s.shown().map(label), Some("Selection: ring"));
        // Sent, it is cleared until the selection changes.
        assert_eq!(s.take().as_ref().map(label), Some("Selection: ring"));
        assert_eq!(s.shown(), None);
        assert_eq!(s.take(), None, "sent once");
        s.follow(None);
        s.follow(Some(context("Selection: ring")));
        assert!(s.shown().is_some(), "selected again");
    }
}
