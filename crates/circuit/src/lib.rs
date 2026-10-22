//! Circuits: the parts of an electronics build, where each one sits and the
//! wires between their pins, read from a `.c2fc` file.
//!
//! ```toml
//! version = 1
//!
//! [parts.xiao]
//! part = "parts/xiao_esp32s3.c2f"
//! at = [0, 0, 0]
//! rotate = [0, 0, 90]
//!
//! [parts.amp]
//! part = "parts/max98357a.c2f"
//! at = [-30, 0, 0]
//! params = { headers = true }
//!
//! [[wires]]
//! from = "xiao.D3"
//! to = "amp.BCLK"
//! color = "blue"
//! label = "speaker I2S"
//! ```
//!
//! A part is a `.c2f` file that declares its pins in `// @pin` comments
//! ([`c2f_lang::annotations`]), found next to the circuit, then in the
//! library folders. It is placed as `translate(at) rotate(rotate)` would
//! place it, and its `params` are set as `c2f -D name=value` sets them.
//! Wires are laid in the order the file lists them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use c2f_lang::annotations::{self, Annotations, Pin};
use c2f_lang::customizer::Literal;
use c2f_lang::{Diagnostic, Span};
use serde::Deserialize;
use toml::Spanned;

/// The file format's version, which `version` may give.
pub const VERSION: i64 = 1;

/// The extension of circuit files.
pub const EXTENSION: &str = "c2fc";

#[derive(Clone, Debug)]
pub struct Circuit {
    /// The `.c2f` file of the enclosure, if the circuit names one.
    pub enclosure: Option<PathBuf>,
    /// In the order the file lists them.
    pub parts: Vec<Instance>,
    /// In the order they are laid.
    pub wires: Vec<Wire>,
}

/// A part placed in the circuit.
#[derive(Clone, Debug)]
pub struct Instance {
    /// Its name in the circuit, as in `xiao.D3`.
    pub name: String,
    /// The part's `.c2f` file, as the circuit names it.
    pub part: String,
    /// The file found for it.
    pub path: PathBuf,
    /// The file's text.
    pub source: String,
    /// The values its variables take, in the order given.
    pub params: Vec<(String, Literal)>,
    pub at: [f64; 3],
    /// Degrees about x, then y, then z, as `rotate([x, y, z])`.
    pub rotate: [f64; 3],
    pub color: Option<String>,
    pub annotations: Annotations,
    /// Its name in the circuit file.
    pub span: Span,
}

/// A wire from one pin to another.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub from: End,
    pub to: End,
    pub color: Option<String>,
    pub label: Option<String>,
    /// Its `from` in the circuit file.
    pub span: Span,
}

/// A pin of a part: indices into [`Circuit::parts`] and that part's pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct End {
    pub part: usize,
    pub pin: usize,
}

/// A pin where the circuit places it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: [f64; 3],
    /// Unit vector, the way a wire leaves the pin.
    pub direction: [f64; 3],
}

impl Circuit {
    pub fn pin(&self, end: End) -> &Pin {
        &self.parts[end.part].annotations.pins[end.pin]
    }

    /// `part.pin`, as the circuit file writes it.
    pub fn end_name(&self, end: End) -> String {
        format!("{}.{}", self.parts[end.part].name, self.pin(end).name)
    }

    /// Where a pin is in the circuit's coordinates.
    pub fn place(&self, end: End) -> Placed {
        let part = &self.parts[end.part];
        let pin = self.pin(end);
        Placed { position: part.place(pin.position), direction: part.turn(pin.direction) }
    }
}

impl Instance {
    pub fn pin(&self, name: &str) -> Option<usize> {
        self.annotations.pins.iter().position(|pin| pin.name == name)
    }

    /// A point of the part in the circuit's coordinates.
    pub fn place(&self, point: [f64; 3]) -> [f64; 3] {
        let p = self.turn(point);
        [p[0] + self.at[0], p[1] + self.at[1], p[2] + self.at[2]]
    }

    /// A direction of the part in the circuit's coordinates.
    pub fn turn(&self, v: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = self.rotate;
        let v = [v[0], v[1] * cos(x) - v[2] * sin(x), v[1] * sin(x) + v[2] * cos(x)];
        let v = [v[0] * cos(y) + v[2] * sin(y), v[1], -v[0] * sin(y) + v[2] * cos(y)];
        [v[0] * cos(z) - v[1] * sin(z), v[0] * sin(z) + v[1] * cos(z), v[2]]
    }

    /// `c2f -D` assignments for the part's parameters, as `headers=true`.
    pub fn defines(&self) -> Vec<String> {
        self.params.iter().map(|(name, value)| format!("{name}={}", source(value))).collect()
    }

    /// The part's text with its parameters set, appended after an
    /// end-of-text character as `c2f -D` does.
    pub fn source_with_params(&self) -> String {
        let mut src = self.source.clone();
        if !self.params.is_empty() {
            src.push_str("\n\x03\n");
            for define in self.defines() {
                src.push_str(&define);
                src.push_str(";\n");
            }
        }
        src
    }
}

/// A constant as .scad writes it, its numbers in full: the AST dump's
/// notation keeps six digits.
fn source(value: &Literal) -> String {
    match value {
        Literal::Number(n) => n.to_string(),
        Literal::Vector(items) => format!("[{}]", items.iter().map(source).collect::<Vec<_>>().join(", ")),
        other => other.to_source(),
    }
}

/// Exact at multiples of 90°, so that turned pins land where they should.
fn sin(degrees: f64) -> f64 {
    match degrees.rem_euclid(360.0) {
        0.0 | 180.0 => 0.0,
        90.0 => 1.0,
        270.0 => -1.0,
        _ => degrees.to_radians().sin(),
    }
}

fn cos(degrees: f64) -> f64 {
    sin(degrees + 90.0)
}

/// A problem in a circuit file or in one of its parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub path: PathBuf,
    /// The text of that file.
    pub src: String,
    pub diagnostic: Diagnostic,
}

impl Error {
    pub fn render(&self) -> String {
        self.diagnostic.render(&self.src, &self.path.display().to_string())
    }
}

/// Reads the circuit file at `path`.
pub fn load(path: &Path, library_paths: &[PathBuf]) -> Result<Circuit, Vec<Error>> {
    match std::fs::read_to_string(path) {
        Ok(src) => parse(&src, path, library_paths),
        Err(err) => Err(vec![Error {
            path: path.to_path_buf(),
            src: String::new(),
            diagnostic: Diagnostic::new(format!("cannot read the circuit: {err}"), Span::default()),
        }]),
    }
}

/// Reads `src` as the circuit file at `path`, whose folder its part files
/// are named from. Every problem found is reported, not just the first.
pub fn parse(src: &str, path: &Path, library_paths: &[PathBuf]) -> Result<Circuit, Vec<Error>> {
    let mut reader =
        Reader { src, path, library_paths, errors: Vec::new(), files: HashMap::new(), left_out: Vec::new() };
    let circuit = reader.circuit();
    match circuit {
        Some(circuit) if reader.errors.is_empty() => Ok(circuit),
        _ => Err(reader.errors),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    version: Option<Spanned<i64>>,
    enclosure: Option<Spanned<String>>,
    #[serde(default)]
    parts: BTreeMap<Spanned<String>, PartEntry>,
    #[serde(default)]
    wires: Vec<WireEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartEntry {
    part: Spanned<String>,
    at: Option<Spanned<Vec<f64>>>,
    rotate: Option<Spanned<Vec<f64>>>,
    #[serde(default)]
    params: BTreeMap<Spanned<String>, Spanned<toml::Value>>,
    color: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEntry {
    from: Spanned<String>,
    to: Spanned<String>,
    color: Option<String>,
    label: Option<String>,
}

/// A part file as read once, however many instances use it.
struct PartFile {
    path: PathBuf,
    source: String,
    annotations: Annotations,
}

struct Reader<'a> {
    src: &'a str,
    path: &'a Path,
    library_paths: &'a [PathBuf],
    errors: Vec<Error>,
    /// By the name the circuit gives; `None` when it was not found.
    files: HashMap<String, Option<PartFile>>,
    /// The parts whose file was not found: wires to them are not reported.
    left_out: Vec<String>,
}

fn span<T>(value: &Spanned<T>) -> Span {
    let range = value.span();
    Span::new(range.start, range.end)
}

impl Reader<'_> {
    fn error(&mut self, message: impl Into<String>, span: Span) {
        let diagnostic = Diagnostic::new(message, span);
        self.errors.push(Error { path: self.path.to_path_buf(), src: self.src.to_string(), diagnostic });
    }

    fn circuit(&mut self) -> Option<Circuit> {
        let file: FileEntry = match toml::from_str(self.src) {
            Ok(file) => file,
            Err(err) => {
                let range = err.span().unwrap_or(0..0);
                self.error(err.message().trim_end(), Span::new(range.start, range.end));
                return None;
            }
        };
        if let Some(version) = &file.version {
            if *version.get_ref() != VERSION {
                self.error(format!("unknown circuit version {}, expected {VERSION}", version.get_ref()), span(version));
            }
        }
        let enclosure = file.enclosure.as_ref().and_then(|name| {
            let found = self.find(name.get_ref());
            if found.is_none() {
                self.error(self.not_found(name.get_ref()), span(name));
            }
            found
        });
        // A table keeps its keys sorted; the file's order is the keys'.
        let mut entries: Vec<_> = file.parts.into_iter().collect();
        entries.sort_by_key(|(name, _)| name.span().start);
        let parts = entries.into_iter().filter_map(|(name, entry)| self.part(name, entry)).collect();
        let mut circuit = Circuit { enclosure, parts, wires: Vec::new() };
        circuit.wires = file.wires.into_iter().filter_map(|entry| self.wire(&circuit, entry)).collect();
        Some(circuit)
    }

    fn part(&mut self, name: Spanned<String>, entry: PartEntry) -> Option<Instance> {
        let bad_name = match name.get_ref().chars().find(|c| ".,=".contains(*c) || c.is_whitespace()) {
            Some(c) => Some(format!("a part name cannot contain `{c}`")),
            None if name.get_ref().is_empty() => Some("a part needs a name".to_string()),
            None => None,
        };
        if let Some(message) = bad_name {
            self.error(message, span(&name));
            self.left_out.push(name.get_ref().clone());
            return None;
        }
        let at = self.vector(entry.at.as_ref(), "at");
        let rotate = self.vector(entry.rotate.as_ref(), "rotate");
        let mut params = Vec::new();
        for (param, value) in &entry.params {
            if !is_identifier(param.get_ref()) {
                self.error(format!("`{}` is not a variable name", param.get_ref()), span(param));
            } else if let Some(literal) = literal(value.get_ref()) {
                params.push((param.span().start, param.get_ref().clone(), literal));
            } else {
                self.error("a parameter is a boolean, a finite number, a string or an array of them", span(value));
            }
        }
        params.sort_by_key(|(start, _, _)| *start);
        if self.part_file(&entry.part).is_none() {
            self.left_out.push(name.get_ref().clone());
            return None;
        }
        let file = self.files[entry.part.get_ref()].as_ref()?;
        Some(Instance {
            name: name.get_ref().clone(),
            part: entry.part.get_ref().clone(),
            path: file.path.clone(),
            source: file.source.clone(),
            params: params.into_iter().map(|(_, name, value)| (name, value)).collect(),
            at,
            rotate,
            color: entry.color,
            annotations: file.annotations.clone(),
            span: span(&name),
        })
    }

    /// Three numbers, `[0, 0, 0]` when left out.
    fn vector(&mut self, value: Option<&Spanned<Vec<f64>>>, field: &str) -> [f64; 3] {
        match value {
            None => [0.0; 3],
            Some(v) if v.get_ref().iter().any(|n| !n.is_finite()) => {
                self.error(format!("`{field}` must be finite numbers"), span(v));
                [0.0; 3]
            }
            Some(v) if v.get_ref().len() == 3 => [v.get_ref()[0], v.get_ref()[1], v.get_ref()[2]],
            Some(v) => {
                self.error(format!("`{field}` is three numbers, [x, y, z]"), span(v));
                [0.0; 3]
            }
        }
    }

    /// The part file a circuit names, read once.
    fn part_file(&mut self, name: &Spanned<String>) -> Option<&PartFile> {
        if !self.files.contains_key(name.get_ref()) {
            let file = self.read_part(name);
            self.files.insert(name.get_ref().clone(), file);
        }
        self.files[name.get_ref()].as_ref()
    }

    fn read_part(&mut self, name: &Spanned<String>) -> Option<PartFile> {
        let Some(path) = self.find(name.get_ref()) else {
            self.error(self.not_found(name.get_ref()), span(name));
            return None;
        };
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(err) => {
                self.error(format!("cannot read {}: {err}", path.display()), span(name));
                return None;
            }
        };
        let (annotations, errors) = annotations::read(&source);
        self.errors.extend(
            errors.into_iter().map(|diagnostic| Error { path: path.clone(), src: source.clone(), diagnostic }),
        );
        Some(PartFile { path, source, annotations })
    }

    /// A file named next to the circuit, then in the library folders.
    fn find(&self, name: &str) -> Option<PathBuf> {
        let folder = self.path.parent().unwrap_or(Path::new(""));
        std::iter::once(folder)
            .chain(self.library_paths.iter().map(PathBuf::as_path))
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    }

    fn not_found(&self, name: &str) -> String {
        format!("cannot find `{name}` next to the circuit or in the library folders")
    }

    fn wire(&mut self, circuit: &Circuit, entry: WireEntry) -> Option<Wire> {
        let from = self.end(circuit, &entry.from);
        let to = self.end(circuit, &entry.to);
        let (from, to) = (from?, to?);
        if from == to {
            self.error(format!("a wire cannot go from `{}` to itself", circuit.end_name(from)), span(&entry.to));
            return None;
        }
        Some(Wire { from, to, color: entry.color, label: entry.label, span: span(&entry.from) })
    }

    fn end(&mut self, circuit: &Circuit, end: &Spanned<String>) -> Option<End> {
        // A part left out was reported already, and a dot in its name
        // was one of the reasons.
        if self.left_out.iter().any(|name| end.get_ref().starts_with(&format!("{name}."))) {
            return None;
        }
        let Some((part_name, pin_name)) = end.get_ref().split_once('.') else {
            self.error(format!("expected `part.pin`, as in `xiao.D3`, found `{}`", end.get_ref()), span(end));
            return None;
        };
        let Some(part) = circuit.parts.iter().position(|p| p.name == part_name) else {
            self.error(format!("there is no part named `{part_name}`"), span(end));
            return None;
        };
        let instance = &circuit.parts[part];
        let Some(pin) = instance.pin(pin_name) else {
            self.error(format!("`{part_name}` ({}) has no pin named `{pin_name}`", instance.part), span(end));
            return None;
        };
        Some(End { part, pin })
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A TOML value as the .scad constant that writes it.
fn literal(value: &toml::Value) -> Option<Literal> {
    Some(match value {
        toml::Value::Boolean(b) => Literal::Bool(*b),
        toml::Value::Integer(n) => Literal::Number(*n as f64),
        toml::Value::Float(n) if n.is_finite() => Literal::Number(*n),
        toml::Value::String(s) => Literal::Str(s.clone()),
        toml::Value::Array(items) => Literal::Vector(items.iter().map(literal).collect::<Option<_>>()?),
        toml::Value::Float(_) | toml::Value::Table(_) | toml::Value::Datetime(_) => return None,
    })
}
