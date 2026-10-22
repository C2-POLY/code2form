//! Electronics annotations: the `//` comments in which a part declares
//! where its pins, mount holes and ports are, in its own coordinates.
//!
//! ```text
//! // @pin D3  [2.54, 15.2, 1.6] up io 3.3V "GPIO4"
//! // @pin GND [15.24, 15.2, 1.6] ground
//! // @mount_hole [1.5, 1.5] d=2
//! // @port usb_c [9, 0, 2] -y [9, 3.2]
//! ```
//!
//! The reference implementation reads them as plain comments, so an
//! annotated file parses, evaluates and renders as it did without them.
//!
//! Each annotation is a `//` comment whose text starts with its keyword. A
//! name and a position `[x, y, z]` come first (a mount hole has no name and
//! may give `[x, y]`, at z = 0); the other fields follow in any order, each
//! told apart by its form:
//!
//! - direction: `up`, `down`, `+x`, `-x`, `+y`, `-y`, `+z`, `-z`, or a
//!   vector `[x, y, z]`, normalized. Pins and ports face `up` and mount
//!   holes `down` unless told otherwise.
//! - pin type: one of [`PinKind`]'s names, `passive` when left out.
//! - voltage: a number followed by `V`, the most a pin takes (what it
//!   supplies for `power_out`).
//! - label: a `"string"`.
//! - a mount hole's diameter `d=2`; a port's opening `[width, height]` or a
//!   round one `d=3`.
//!
//! Positions and sizes are numbers only: they do not depend on the file's
//! variables.

use std::fmt;

use crate::Diagnostic;
use crate::customizer::parse_number;
use crate::dump::{number, string};
use crate::span::Span;

/// What a part declares for wiring and mounting, in source order within
/// each kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Annotations {
    pub pins: Vec<Pin>,
    pub mount_holes: Vec<MountHole>,
    pub ports: Vec<Port>,
}

impl Annotations {
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty() && self.mount_holes.is_empty() && self.ports.is_empty()
    }
}

/// `// @pin name [x, y, z] ...`: where a wire is soldered or plugged.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    pub position: [f64; 3],
    /// Unit vector, the way a wire leaves the pin.
    pub direction: [f64; 3],
    pub kind: PinKind,
    /// The most the pin takes, in volts; what it supplies for `power_out`.
    pub voltage: Option<f64>,
    pub label: Option<String>,
    /// The annotation's text, after `//`.
    pub span: Span,
}

/// `// @mount_hole [x, y] d=2`: a hole a screw or a peg goes through.
#[derive(Clone, Debug, PartialEq)]
pub struct MountHole {
    pub position: [f64; 3],
    pub diameter: f64,
    /// Unit vector, the side the part rests on.
    pub direction: [f64; 3],
    pub span: Span,
}

/// `// @port name [x, y, z] -y [w, h]`: an opening the enclosure must leave,
/// such as a connector or a microphone's hole.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    /// The center of the opening.
    pub position: [f64; 3],
    /// Unit vector, the way the opening faces.
    pub direction: [f64; 3],
    pub size: PortSize,
    pub label: Option<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PortSize {
    /// Width and height, across the direction.
    Rect([f64; 2]),
    /// A round opening's diameter.
    Round(f64),
}

/// What a pin does electrically, for the wiring checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    Io,
    Input,
    Output,
    PowerIn,
    PowerOut,
    Ground,
    Passive,
    /// Not connected: nothing may be wired to it.
    Nc,
}

impl PinKind {
    pub const ALL: [PinKind; 8] = [
        PinKind::Io,
        PinKind::Input,
        PinKind::Output,
        PinKind::PowerIn,
        PinKind::PowerOut,
        PinKind::Ground,
        PinKind::Passive,
        PinKind::Nc,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PinKind::Io => "io",
            PinKind::Input => "input",
            PinKind::Output => "output",
            PinKind::PowerIn => "power_in",
            PinKind::PowerOut => "power_out",
            PinKind::Ground => "ground",
            PinKind::Passive => "passive",
            PinKind::Nc => "nc",
        }
    }

    pub fn named(name: &str) -> Option<PinKind> {
        PinKind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

const UP: [f64; 3] = [0.0, 0.0, 1.0];
const DOWN: [f64; 3] = [0.0, 0.0, -1.0];

/// The words for the axes, `up` and `down` first so they print that way.
const AXES: [(&str, [f64; 3]); 8] = [
    ("up", UP),
    ("down", DOWN),
    ("+x", [1.0, 0.0, 0.0]),
    ("-x", [-1.0, 0.0, 0.0]),
    ("+y", [0.0, 1.0, 0.0]),
    ("-y", [0.0, -1.0, 0.0]),
    ("+z", UP),
    ("-z", DOWN),
];

/// Reads the annotations of a file's own text, with an error for each one
/// that is badly written; those are left out.
pub fn read(src: &str) -> (Annotations, Vec<Diagnostic>) {
    let mut reader = Reader { annotations: Annotations::default(), errors: Vec::new() };
    for (start, text) in line_comments(src) {
        let body = text.trim_start();
        let Some(rest) = body.strip_prefix('@') else { continue };
        let keyword = &rest[..rest.find(|c: char| c.is_whitespace() || c == '[' || c == '"').unwrap_or(rest.len())];
        let fields_start = start + (text.len() - body.len()) + 1 + keyword.len();
        let span = Span::new(start + (text.len() - body.len()), start + text.trim_end().len());
        let fields = &src[fields_start..start + text.len()];
        let result = match keyword {
            "pin" => lex(fields, fields_start).and_then(|items| reader.pin(items, span)),
            "mount_hole" => lex(fields, fields_start).and_then(|items| reader.mount_hole(items, span)),
            "port" => lex(fields, fields_start).and_then(|items| reader.port(items, span)),
            // Other `@words` belong to someone else.
            _ => continue,
        };
        if let Err(error) = result {
            reader.errors.push(error);
        }
    }
    (reader.annotations, reader.errors)
}

/// The `//` comments of `src` that sit outside strings and block comments:
/// (offset of the text after `//`, that text without its line break).
fn line_comments(src: &str) -> Vec<(usize, &str)> {
    let bytes = src.as_bytes();
    let mut comments = Vec::new();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_string => i += 1,
            b'"' => in_string = !in_string,
            b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(bytes.len(), |n| i + n);
                let text = &src[i + 2..end];
                comments.push((i + 2, text.strip_suffix('\r').unwrap_or(text)));
                i = end;
            }
            b'/' if !in_string && bytes.get(i + 1) == Some(&b'*') => {
                i = src[i + 2..].find("*/").map_or(bytes.len(), |n| i + 2 + n + 1);
            }
            _ => {}
        }
        i += 1;
    }
    comments
}

#[derive(Clone, Debug, PartialEq)]
enum Token<'a> {
    Vector(Vec<f64>),
    Str(String),
    Word(&'a str),
}

struct Item<'a> {
    token: Token<'a>,
    span: Span,
}

/// Splits an annotation's fields into vectors of numbers, strings and words.
fn lex(text: &str, base: usize) -> Result<Vec<Item<'_>>, Diagnostic> {
    let mut items = Vec::new();
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        let token = match c {
            '[' => {
                let close = text[i..].find(']').ok_or_else(|| {
                    Diagnostic::new("unclosed `[`", Span::new(base + i, base + text.trim_end().len()))
                })?;
                let inner = &text[i + 1..i + close];
                i += close + 1;
                let numbers: Option<Vec<f64>> = inner.split(',').map(|n| finite(n.trim())).collect();
                let numbers = numbers.ok_or_else(|| {
                    Diagnostic::new("expected numbers separated by commas", Span::new(base + start, base + i))
                })?;
                Token::Vector(numbers)
            }
            '"' => {
                let mut value = String::new();
                let mut chars = text[i + 1..].char_indices();
                loop {
                    match chars.next() {
                        None => {
                            return Err(Diagnostic::new(
                                "unterminated string",
                                Span::new(base + i, base + text.trim_end().len()),
                            ));
                        }
                        Some((end, '"')) => {
                            i += 1 + end + 1;
                            break;
                        }
                        Some((_, '\\')) => value.extend(chars.next().map(|(_, c)| c)),
                        Some((_, c)) => value.push(c),
                    }
                }
                Token::Str(value)
            }
            _ => {
                let len = text[i..].find(|c: char| c.is_whitespace() || c == '[' || c == '"').unwrap_or(text.len() - i);
                i += len;
                Token::Word(&text[start..i])
            }
        };
        items.push(Item { token, span: Span::new(base + start, base + i) });
    }
    Ok(items)
}

struct Reader {
    annotations: Annotations,
    errors: Vec<Diagnostic>,
}

/// The fields after the name and position, each given at most once.
#[derive(Default)]
struct Fields {
    direction: Option<[f64; 3]>,
    kind: Option<PinKind>,
    voltage: Option<f64>,
    label: Option<String>,
    diameter: Option<f64>,
    size: Option<[f64; 2]>,
}

/// Which fields an annotation takes.
#[derive(Clone, Copy, PartialEq)]
enum What {
    Pin,
    MountHole,
    Port,
}

impl What {
    fn name(self) -> &'static str {
        match self {
            What::Pin => "pin",
            What::MountHole => "mount hole",
            What::Port => "port",
        }
    }

    fn expected(self) -> &'static str {
        match self {
            What::Pin => {
                "a direction (up, down, +x, -y, [x, y, z]), a type (io, input, output, power_in, power_out, \
                 ground, passive, nc), a voltage like 3.3V or a \"label\""
            }
            What::MountHole => "its diameter d=… or a direction (up, down, +x, -y, [x, y, z])",
            What::Port => {
                "a direction (up, down, +x, -y, [x, y, z]), its opening [width, height] or d=…, or a \"label\""
            }
        }
    }
}

impl Reader {
    fn pin(&mut self, items: Vec<Item>, span: Span) -> Result<(), Diagnostic> {
        let mut items = items.into_iter();
        let (name, name_span) = name(items.next(), What::Pin, span)?;
        if self.annotations.pins.iter().any(|pin| pin.name == name) {
            return Err(Diagnostic::new(format!("the part already has a pin named `{name}`"), name_span));
        }
        let position = position(items.next(), What::Pin, span, false)?;
        let fields = fields(items, What::Pin)?;
        self.annotations.pins.push(Pin {
            name,
            position,
            direction: fields.direction.unwrap_or(UP),
            kind: fields.kind.unwrap_or(PinKind::Passive),
            voltage: fields.voltage,
            label: fields.label,
            span,
        });
        Ok(())
    }

    fn mount_hole(&mut self, items: Vec<Item>, span: Span) -> Result<(), Diagnostic> {
        let mut items = items.into_iter();
        let position = position(items.next(), What::MountHole, span, true)?;
        let fields = fields(items, What::MountHole)?;
        let diameter = fields
            .diameter
            .ok_or_else(|| Diagnostic::new("a mount hole needs its diameter, as in d=2", span))?;
        self.annotations.mount_holes.push(MountHole {
            position,
            diameter,
            direction: fields.direction.unwrap_or(DOWN),
            span,
        });
        Ok(())
    }

    fn port(&mut self, items: Vec<Item>, span: Span) -> Result<(), Diagnostic> {
        let mut items = items.into_iter();
        let (name, name_span) = name(items.next(), What::Port, span)?;
        if self.annotations.ports.iter().any(|port| port.name == name) {
            return Err(Diagnostic::new(format!("the part already has a port named `{name}`"), name_span));
        }
        let position = position(items.next(), What::Port, span, false)?;
        let fields = fields(items, What::Port)?;
        let size = match (fields.size, fields.diameter) {
            (Some(size), None) => PortSize::Rect(size),
            (None, Some(d)) => PortSize::Round(d),
            (Some(_), Some(_)) => {
                return Err(Diagnostic::new("a port's opening is [width, height] or d=…, not both", span));
            }
            (None, None) => {
                return Err(Diagnostic::new(
                    "a port needs the size of its opening, as in [9, 3.2] or d=3",
                    span,
                ));
            }
        };
        self.annotations.ports.push(Port {
            name,
            position,
            direction: fields.direction.unwrap_or(UP),
            size,
            label: fields.label,
            span,
        });
        Ok(())
    }
}

fn name(item: Option<Item>, what: What, span: Span) -> Result<(String, Span), Diagnostic> {
    let expected = || format!("expected the {}'s name", what.name());
    let item = item.ok_or_else(|| Diagnostic::new(expected(), span))?;
    let Token::Word(word) = item.token else { return Err(Diagnostic::new(expected(), item.span)) };
    // A circuit names a pin `part.pin`.
    if let Some(c) = word.chars().find(|c| ".,=".contains(*c)) {
        return Err(Diagnostic::new(format!("a {} name cannot contain `{c}`", what.name()), item.span));
    }
    Ok((word.to_string(), item.span))
}

fn position(item: Option<Item>, what: What, span: Span, flat: bool) -> Result<[f64; 3], Diagnostic> {
    let expected = if flat { "[x, y] or [x, y, z]" } else { "[x, y, z]" };
    let error = |span| Diagnostic::new(format!("expected the {}'s position, {expected}", what.name()), span);
    let item = item.ok_or_else(|| error(span))?;
    match item.token {
        Token::Vector(v) if v.len() == 3 => Ok([v[0], v[1], v[2]]),
        Token::Vector(v) if flat && v.len() == 2 => Ok([v[0], v[1], 0.0]),
        _ => Err(error(item.span)),
    }
}

fn fields<'a>(items: impl Iterator<Item = Item<'a>>, what: What) -> Result<Fields, Diagnostic> {
    let mut fields = Fields::default();
    for item in items {
        let span = item.span;
        let unknown = |text: &str| {
            Diagnostic::new(format!("unknown {} field `{text}`: expected {}", what.name(), what.expected()), span)
        };
        let twice = |field: &str| Diagnostic::new(format!("the {} already has {field}", what.name()), span);
        match item.token {
            Token::Vector(v) if v.len() == 3 => {
                if fields.direction.is_some() {
                    return Err(twice("a direction"));
                }
                fields.direction = Some(normalize([v[0], v[1], v[2]]).ok_or_else(|| {
                    Diagnostic::new("a direction cannot be [0, 0, 0]", span)
                })?);
            }
            Token::Vector(v) if v.len() == 2 && what == What::Port => {
                if fields.size.is_some() {
                    return Err(twice("an opening"));
                }
                if v.iter().any(|&n| n <= 0.0) {
                    return Err(Diagnostic::new("an opening's width and height must be positive", span));
                }
                fields.size = Some([v[0], v[1]]);
            }
            Token::Vector(_) => {
                let text = match what {
                    What::Port => "expected [width, height] or a direction [x, y, z]",
                    _ => "expected a direction [x, y, z]",
                };
                return Err(Diagnostic::new(text, span));
            }
            Token::Str(label) if what != What::MountHole => {
                if fields.label.is_some() {
                    return Err(twice("a label"));
                }
                fields.label = Some(label);
            }
            Token::Str(label) => return Err(unknown(&string(&label))),
            Token::Word(word) => {
                if let Some((_, direction)) = AXES.iter().find(|(name, _)| *name == word) {
                    if fields.direction.is_some() {
                        return Err(twice("a direction"));
                    }
                    fields.direction = Some(*direction);
                } else if let Some(d) = word.strip_prefix("d=").filter(|_| what != What::Pin) {
                    if fields.diameter.is_some() {
                        return Err(twice("a diameter"));
                    }
                    let d = finite(d)
                        .filter(|&d| d > 0.0)
                        .ok_or_else(|| Diagnostic::new("a diameter must be a positive number", span))?;
                    fields.diameter = Some(d);
                } else if let Some(kind) = PinKind::named(word).filter(|_| what == What::Pin) {
                    if fields.kind.is_some() {
                        return Err(twice("a type"));
                    }
                    fields.kind = Some(kind);
                } else if let Some(volts) =
                    word.strip_suffix(['V', 'v']).and_then(finite).filter(|_| what == What::Pin)
                {
                    if fields.voltage.is_some() {
                        return Err(twice("a voltage"));
                    }
                    fields.voltage = Some(volts);
                } else {
                    return Err(unknown(word));
                }
            }
        }
    }
    Ok(fields)
}

/// A number in .scad syntax that is not too large for an `f64`.
fn finite(text: &str) -> Option<f64> {
    parse_number(text).filter(|n| n.is_finite())
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 0.0).then(|| v.map(|n| n / length))
}

fn vector(v: &[f64]) -> String {
    let items: Vec<String> = v.iter().map(|&n| number(n)).collect();
    format!("[{}]", items.join(", "))
}

/// An axis's word, or the vector.
fn direction(v: [f64; 3]) -> String {
    AXES.iter().find(|(_, axis)| *axis == v).map_or_else(|| vector(&v), |(name, _)| name.to_string())
}

/// Each annotation prints as it is written in full, with its defaults.
impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "@pin {} {} {} {}", self.name, vector(&self.position), direction(self.direction), self.kind.name())?;
        if let Some(volts) = self.voltage {
            write!(f, " {}V", number(volts))?;
        }
        if let Some(label) = &self.label {
            write!(f, " {}", string(label))?;
        }
        Ok(())
    }
}

impl fmt::Display for MountHole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "@mount_hole {} d={} {}", vector(&self.position), number(self.diameter), direction(self.direction))
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let size = match self.size {
            PortSize::Rect(size) => vector(&size),
            PortSize::Round(d) => format!("d={}", number(d)),
        };
        write!(f, "@port {} {} {} {size}", self.name, vector(&self.position), direction(self.direction))?;
        if let Some(label) = &self.label {
            write!(f, " {}", string(label))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::LineIndex;

    /// The annotations as printed, and each error as `line: message`.
    fn read_text(src: &str) -> (Vec<String>, Vec<String>) {
        let (annotations, errors) = read(src);
        let mut printed: Vec<(Span, String)> = Vec::new();
        printed.extend(annotations.pins.iter().map(|a| (a.span, a.to_string())));
        printed.extend(annotations.mount_holes.iter().map(|a| (a.span, a.to_string())));
        printed.extend(annotations.ports.iter().map(|a| (a.span, a.to_string())));
        printed.sort_by_key(|(span, _)| span.start);
        let lines = LineIndex::new(src);
        let errors = errors
            .iter()
            .map(|e| format!("{}: {}", lines.line(e.span.start as usize) + 1, e.message))
            .collect();
        (printed.into_iter().map(|(_, text)| text).collect(), errors)
    }

    #[test]
    fn reads_each_kind_with_its_defaults() {
        let src = "\
// @pin D3  [2.54, 15.2, 1.6] up io 3.3V \"GPIO4\"
// @pin GND [15.24, 15.2, 1.6] ground
//@pin OUT [0, 0, 5]
// @mount_hole [1.5, 1.5] d=2
// @port usb_c [9, 0, 2] -y [9, 3.2]
// @port mic [0, 0, 1] d=1.2 \"sound hole\"
cube(10);
";
        let (printed, errors) = read_text(src);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(
            printed,
            [
                "@pin D3 [2.54, 15.2, 1.6] up io 3.3V \"GPIO4\"",
                "@pin GND [15.24, 15.2, 1.6] up ground",
                "@pin OUT [0, 0, 5] up passive",
                "@mount_hole [1.5, 1.5, 0] d=2 down",
                "@port usb_c [9, 0, 2] -y [9, 3.2]",
                "@port mic [0, 0, 1] up d=1.2 \"sound hole\"",
            ]
        );
    }

    #[test]
    fn fields_come_in_any_order() {
        let (a, _) = read("// @pin 5V [1, 2, 3] 5V \"supply\" power_in -x\n");
        let (b, _) = read("// @pin 5V [1, 2, 3] -x power_in 5V \"supply\"\n");
        assert_eq!(a.pins[0].to_string(), "@pin 5V [1, 2, 3] -x power_in 5V \"supply\"");
        assert_eq!(a.pins[0].to_string(), b.pins[0].to_string());
        let (c, _) = read("// @port jack [0, 0, 0] [6, 5] [1, 0, 1]\n");
        let d = c.ports[0].direction;
        assert!((d[0] - 0.5f64.sqrt()).abs() < 1e-12 && d[1] == 0.0 && (d[2] - 0.5f64.sqrt()).abs() < 1e-12);
        assert_eq!(c.ports[0].to_string(), "@port jack [0, 0, 0] [0.707107, 0, 0.707107] [6, 5]");
    }

    #[test]
    fn records_where_each_annotation_is() {
        let src = "cube(1);\n  // @pin A [0, 0, 0]  \r\n";
        let (annotations, _) = read(src);
        let span = annotations.pins[0].span;
        assert_eq!(&src[span.range()], "@pin A [0, 0, 0]");
        assert_eq!(LineIndex::new(src).line(span.start as usize), 1);
    }

    #[test]
    fn badly_written_annotations_give_errors_with_their_line() {
        let src = "\
// @pin
// @pin A
// @pin A [1, 2]
// @pin B [0, 0, 0] sideways
// @pin C [0, 0, 0] up down
// @pin D [0, 0, 0] [0, 0, 0]
// @pin E.1 [0, 0, 0]
// @pin F [0, 0, x]
// @pin G [0, 0, 0
// @pin H [0, 0, 0] \"open
// @mount_hole [1, 1]
// @mount_hole [1, 1] d=0
// @port usb [0, 0, 0] -y
// @port usb [0, 0, 0] [1, 2] d=3
// @pin I [0, 0, 0] d=2
// @mount_hole [1, 1] d=2 \"label\"
// @pin J [1e999, 0, 0]
//@pin[0, 0, 0]
";
        let (printed, errors) = read_text(src);
        assert_eq!(printed, Vec::<String>::new());
        assert_eq!(
            errors,
            [
                "1: expected the pin's name",
                "2: expected the pin's position, [x, y, z]",
                "3: expected the pin's position, [x, y, z]",
                "4: unknown pin field `sideways`: expected a direction (up, down, +x, -y, [x, y, z]), a type (io, input, output, power_in, power_out, ground, passive, nc), a voltage like 3.3V or a \"label\"",
                "5: the pin already has a direction",
                "6: a direction cannot be [0, 0, 0]",
                "7: a pin name cannot contain `.`",
                "8: expected numbers separated by commas",
                "9: unclosed `[`",
                "10: unterminated string",
                "11: a mount hole needs its diameter, as in d=2",
                "12: a diameter must be a positive number",
                "13: a port needs the size of its opening, as in [9, 3.2] or d=3",
                "14: a port's opening is [width, height] or d=…, not both",
                "15: unknown pin field `d=2`: expected a direction (up, down, +x, -y, [x, y, z]), a type (io, input, output, power_in, power_out, ground, passive, nc), a voltage like 3.3V or a \"label\"",
                "16: unknown mount hole field `\"label\"`: expected its diameter d=… or a direction (up, down, +x, -y, [x, y, z])",
                "17: expected numbers separated by commas",
                "18: expected the pin's name",
            ]
        );
    }

    #[test]
    fn errors_point_at_the_faulty_field() {
        let src = "// @pin B [0, 0, 0] io sideways\n";
        let (_, errors) = read(src);
        assert_eq!(&src[errors[0].span.range()], "sideways");
    }

    #[test]
    fn two_pins_with_the_same_name_are_an_error() {
        let src = "// @pin GND [0, 0, 0]\n// @pin GND [1, 0, 0]\n// @port a [0, 0, 0] d=1\n// @port a [0, 0, 0] d=1\n";
        let (printed, errors) = read_text(src);
        assert_eq!(printed.len(), 2);
        assert_eq!(errors, ["2: the part already has a pin named `GND`", "4: the part already has a port named `a`"]);
    }

    #[test]
    fn only_line_comments_outside_strings_count() {
        let src = "\
/* // @pin A [0, 0, 0] */
s = \"// @pin B [0, 0, 0]\";
/*
// @pin C [0, 0, 0]
*/
// @todo not ours
// see @pin D
x = 1; // @pin E [0, 0, 0]
";
        let (printed, errors) = read_text(src);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(printed, ["@pin E [0, 0, 0] up passive"]);
    }

    #[test]
    fn annotations_leave_the_program_unchanged() {
        let plain = "\
// A breakout board.
width = 20; // [10:40]
cube([width, 10, 1.6]);
";
        let annotated = "\
// A breakout board.
width = 20; // [10:40]
// @pin VIN [1, 1, 1.6] up power_in 5V
// @mount_hole [2, 2] d=2
cube([width, 10, 1.6]); // @port usb [0, 5, 1] -x [8, 3]
";
        let dump = |src| crate::dump::ast_dump(&crate::parse(src).unwrap(), src);
        assert_eq!(dump(annotated), dump(plain));
    }
}
