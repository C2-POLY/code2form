//! The agent's live tools for the circuit open in the active tab: place a
//! part, wire two pins, take a wire away, check the circuit as it is on
//! screen, and guide the user through its assembly wire by wire.
//!
//! Each edit is made in the circuit's text by the spans it was read with,
//! as the mouse rewires it ([`crate::rewire`], [`crate::part_edit`]), so
//! that the file keeps its comments and its order. Before it is made, the
//! wiring checks run on the circuit as the edit would leave it: an error
//! it would bring (a short, a voltage a pin cannot take...) refuses it,
//! with the checker's words, for the agent to correct. The tools are
//! served by [`super::live`]: their calls go to the UI thread as
//! [`CircuitOp`]s.

use std::ops::Range;
use std::path::{Path, PathBuf};

use c2f_agent::Tools;
use c2f_circuit::check::{self, Severity};
use c2f_circuit::{Circuit, End};
use c2f_lang::Span;
use eframe::egui;
use serde_json::{Map, Value, json};

use super::live::LiveTools;
use super::{StudioApp, find_hit};
use crate::console::Level;
use crate::mount;
use crate::pick::Hit;
use crate::{catalogue, circuit, part_edit, rewire};

/// A change to the circuit's parts or wires, as the agent asks for it.
#[derive(Clone, Debug, PartialEq)]
pub enum CircuitEdit {
    /// A new part `name`, or the part of that name moved and turned.
    Place {
        name: String,
        part: Option<String>,
        at: Option<[f64; 3]>,
        rotate: Option<[f64; 3]>,
        params: Option<Map<String, Value>>,
        color: Option<String>,
    },
    /// A new wire, or wire `index` (from 1) changed.
    Connect { from: String, to: String, color: Option<String>, via: Option<Vec<[f64; 3]>>, index: Option<usize> },
    /// Wire `index` (from 1) taken away, or the wire between two pins.
    Disconnect { index: Option<usize>, ends: Option<(String, String)> },
}

/// What the UI thread is asked to do for these tools.
pub enum CircuitOp {
    Edit(CircuitEdit),
    /// Renders the circuit if its text changed since: the text, for the
    /// checks, and the render to wait for.
    Refresh,
    /// Selects something of the circuit, named as `focus` names it.
    Highlight(String),
    /// `start`, `next` or `stop`.
    Assembly(String),
}

/// An edit made: the new text, where the cursor goes (a char index), what
/// was done, and the circuit as the new text reads.
#[derive(Debug)]
pub struct Edited {
    pub text: String,
    pub cursor: usize,
    pub done: String,
    pub circuit: Circuit,
}

/// The circuit's text `src`, the file at `path`, with `edit` made; or why
/// it is refused, and then nothing is changed.
pub fn edit_circuit(src: &str, path: &Path, libraries: &[PathBuf], edit: &CircuitEdit) -> Result<Edited, String> {
    let before = read(src, path, libraries)
        .map_err(|why| format!("the circuit does not read as it is, fix it first (edit_code): {why}"))?;
    let (text, cursor, done) = match edit {
        CircuitEdit::Place { name, part, at, rotate, params, color } => {
            place(src, &before, name, part.as_deref(), *at, *rotate, params.as_ref(), color.as_deref())?
        }
        CircuitEdit::Connect { from, to, color, via, index } => {
            connect(src, &before, from, to, color.as_deref(), via.as_deref(), *index)?
        }
        CircuitEdit::Disconnect { index, ends } => disconnect(src, &before, *index, ends.as_ref())?,
    };
    let after = read(&text, path, libraries).map_err(|why| format!("refused, nothing was changed: {why}"))?;
    let errors = rewire::new_errors(&before, &after);
    if !errors.is_empty() {
        return Err(format!(
            "refused, nothing was changed: the wiring checks find, on the circuit as this would leave it: {}",
            errors.join("; ")
        ));
    }
    Ok(Edited { text, cursor, done, circuit: after })
}

/// The circuit `src` reads as, or its problems with their lines.
fn read(src: &str, path: &Path, libraries: &[PathBuf]) -> Result<Circuit, String> {
    c2f_circuit::parse(src, path, libraries).map_err(|errors| {
        let problems: Vec<String> = errors
            .iter()
            .map(|e| {
                let line = line_at(&e.src, e.diagnostic.span.start as usize);
                if e.path == path {
                    format!("line {line}: {}", e.diagnostic.message)
                } else {
                    format!("{} line {line}: {}", e.path.display(), e.diagnostic.message)
                }
            })
            .collect();
        problems.join("; ")
    })
}

/// The line (from 1) of byte `offset`.
fn line_at(src: &str, offset: usize) -> usize {
    src.as_bytes()[..offset.min(src.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// A number as a circuit writes it, in full.
fn number(v: f64) -> String {
    if v == 0.0 { "0".into() } else { v.to_string() }
}

fn vector(v: [f64; 3]) -> String {
    format!("[{}, {}, {}]", number(v[0]), number(v[1]), number(v[2]))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// A parameter's value as TOML writes it.
fn toml_value(value: &Value) -> Result<String, String> {
    Ok(match value {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => n.to_string(),
        Value::String(s) => quoted(s),
        Value::Array(items) => format!("[{}]", items.iter().map(toml_value).collect::<Result<Vec<_>, _>>()?.join(", ")),
        other => return Err(format!("a parameter is a boolean, a number, a string or a list of them, not {other}")),
    })
}

/// A wire's color, checked.
fn color_of(color: &str) -> Result<String, String> {
    match circuit::parse_color(color) {
        Some(_) => Ok(quoted(color)),
        None => Err(format!("`{color}` is not a color: give a name (red, black, green, blue...) or #rrggbb")),
    }
}

/// The pin `part.pin` of `circuit`.
fn end(circuit: &Circuit, name: &str) -> Result<End, String> {
    let Some((part_name, pin_name)) = name.split_once('.') else {
        return Err(format!("expected `part.pin`, as `xiao.D3`, not `{name}`"));
    };
    let Some(part) = circuit.parts.iter().position(|p| p.name == part_name) else {
        let names: Vec<&str> = circuit.parts.iter().map(|p| p.name.as_str()).collect();
        return Err(format!("there is no part named `{part_name}`; the parts: {}", names.join(", ")));
    };
    let instance = &circuit.parts[part];
    let pin = instance.pin(pin_name).ok_or_else(|| {
        let pins: Vec<&str> = instance.annotations.pins.iter().map(|p| p.name.as_str()).collect();
        format!("`{part_name}` ({}) has no pin `{pin_name}`; its pins: {}", instance.part, pins.join(", "))
    })?;
    Ok(End { part, pin })
}

/// Wire `index`, from 1.
fn wire_index(circuit: &Circuit, index: usize) -> Result<usize, String> {
    index
        .checked_sub(1)
        .filter(|&i| i < circuit.wires.len())
        .ok_or_else(|| format!("there is no wire {index}: the circuit has {} (from 1)", circuit.wires.len()))
}

type Change = (String, usize, String);

#[allow(clippy::too_many_arguments)]
fn place(
    src: &str,
    circuit: &Circuit,
    name: &str,
    part: Option<&str>,
    at: Option<[f64; 3]>,
    rotate: Option<[f64; 3]>,
    params: Option<&Map<String, Value>>,
    color: Option<&str>,
) -> Result<Change, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("`{name}` is not a part name: letters, digits, `_` and `-` only, as `button`"));
    }
    if let Some(instance) = circuit.parts.iter().find(|p| p.name == name) {
        if part.is_some_and(|part| part != instance.part) {
            return Err(format!(
                "`{name}` is a `{}` already: give another name for a new part, or leave `part` out to move it",
                instance.part
            ));
        }
        if params.is_some() || color.is_some() {
            return Err(format!(
                "`{name}` is placed already: place_component only moves and turns it (`at`, `rotate`); change its params or color with edit_code"
            ));
        }
        if at.is_none() && rotate.is_none() {
            return Err(format!("`{name}` is placed already: give `at` or `rotate` to move or turn it"));
        }
        let (text, cursor) = part_edit::set_placement(src, instance, at.map(vector), rotate.map(vector))?;
        let mut done = Vec::new();
        if let Some(at) = at {
            done.push(format!("moved `{name}` to {}", vector(at)));
        }
        if let Some(rotate) = rotate {
            done.push(format!("turned `{name}` to {}", vector(rotate)));
        }
        return Ok((text, cursor, done.join(", ")));
    }
    let part = part.ok_or_else(|| format!("there is no part `{name}` yet: give its `part` file to add it"))?;
    if part.trim().is_empty() || part.contains(['"', '\\', '\n']) {
        return Err(format!("`{part}` is not a part file: give a library path, as `electronics/xiao_esp32s3.c2f`"));
    }
    let at = at.ok_or("a new part needs `at`, where it goes: [x, y, z] in millimeters")?;
    let mut table = format!("[parts.{name}]\npart = {}\nat = {}\n", quoted(part), vector(at));
    if let Some(rotate) = rotate {
        table.push_str(&format!("rotate = {}\n", vector(rotate)));
    }
    if let Some(params) = params.filter(|p| !p.is_empty()) {
        let mut values = Vec::new();
        for (key, value) in params {
            let mut chars = key.chars();
            let identifier = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !identifier {
                return Err(format!("`{key}` is not a parameter name"));
            }
            values.push(format!("{key} = {}", toml_value(value)?));
        }
        table.push_str(&format!("params = {{ {} }}\n", values.join(", ")));
    }
    if let Some(color) = color {
        table.push_str(&format!("color = {}\n", color_of(color)?));
    }
    let (text, cursor) = catalogue::insert_table(src, &table);
    Ok((text, cursor, format!("added `{name}` ({part}) at {}", vector(at))))
}

fn connect(
    src: &str,
    circuit: &Circuit,
    from: &str,
    to: &str,
    color: Option<&str>,
    via: Option<&[[f64; 3]]>,
    index: Option<usize>,
) -> Result<Change, String> {
    let (from_end, to_end) = (end(circuit, from)?, end(circuit, to)?);
    if from_end == to_end {
        return Err(format!("a wire cannot go from `{from}` to itself"));
    }
    let color = color.map(color_of).transpose()?;
    if via.is_some_and(|points| points.iter().flatten().any(|n| !n.is_finite())) {
        return Err("`via` is a list of points of three finite numbers".into());
    }
    let via = via.map(|points| format!("[{}]", points.iter().map(|p| vector(*p)).collect::<Vec<_>>().join(", ")));
    let (from_name, to_name) = (circuit.end_name(from_end), circuit.end_name(to_end));
    let Some(index) = index else {
        let mut keys = format!("from = {}\nto = {}\n", quoted(&from_name), quoted(&to_name));
        if let Some(color) = &color {
            keys.push_str(&format!("color = {color}\n"));
        }
        if let Some(via) = &via {
            keys.push_str(&format!("via = {via}\n"));
        }
        let (text, cursor) = rewire::append_wire(src, &keys);
        return Ok((text, cursor, format!("added a wire: {from_name} ➡ {to_name}")));
    };
    let i = wire_index(circuit, index)?;
    let wire = &circuit.wires[i];
    let table = rewire::table_of(src, wire.span.start as usize, |h| h == "[[wires]]")
        .ok_or("the wire's table was not found: read the circuit again (studio_state)")?;
    let range = |s: Span| s.start as usize..s.end as usize;
    let mut edits: Vec<(Range<usize>, String)> =
        vec![(range(wire.span), quoted(&from_name)), (range(wire.to_span), quoted(&to_name))];
    let mut added = String::new();
    for (key, value, span) in [("color", color, wire.color_span), ("via", via, wire.via_span)] {
        match (value, span) {
            (Some(value), Some(span)) => edits.push((range(span), value)),
            (Some(value), None) => added.push_str(&format!("{key} = {value}\n")),
            (None, _) => {}
        }
    }
    if !added.is_empty() {
        if !src[..table.end].ends_with('\n') {
            added.insert(0, '\n');
        }
        edits.push((table.end..table.end, added));
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut text = src.to_string();
    for (r, new) in edits {
        text.replace_range(r, &new);
    }
    let cursor = src[..table.start].chars().count();
    Ok((text, cursor, format!("changed wire {index}: {from_name} ➡ {to_name}")))
}

fn disconnect(src: &str, circuit: &Circuit, index: Option<usize>, ends: Option<&(String, String)>) -> Result<Change, String> {
    let wire = match (index, ends) {
        (Some(index), None) => wire_index(circuit, index)?,
        (None, Some((from, to))) => {
            let (a, b) = (end(circuit, from)?, end(circuit, to)?);
            circuit
                .wires
                .iter()
                .position(|w| (w.from, w.to) == (a, b) || (w.from, w.to) == (b, a))
                .ok_or_else(|| format!("no wire runs between `{from}` and `{to}`"))?
        }
        _ => return Err("give either `index` (a wire, from 1) or `from` and `to`".into()),
    };
    let w = &circuit.wires[wire];
    let done = format!("removed wire {}: {} ➡ {}", wire + 1, circuit.end_name(w.from), circuit.end_name(w.to));
    let (text, cursor) = rewire::apply(src, circuit, rewire::Edit::Remove { wire })?;
    Ok((text, cursor, done))
}

/// What a finding at `span` is about, as `focus` names it: a wire as
/// `wire:N`, or a part.
fn focus_of(circuit: &Circuit, span: Span) -> Option<String> {
    if let Some(i) = circuit.wires.iter().position(|w| w.span == span || w.to_span == span) {
        return Some(format!("wire:{}", i + 1));
    }
    circuit.parts.iter().find(|p| p.span == span).map(|p| p.name.clone())
}

/// The wiring checks of `circuit`, read from `src`: errors and warnings
/// with their line and what they are about.
pub fn summary(circuit: &Circuit, src: &str) -> Value {
    let (mut errors, mut warnings) = (Vec::new(), Vec::new());
    for finding in check::check(circuit) {
        let value = json!({
            "message": finding.message,
            "line": line_at(src, finding.span.start as usize),
            "focus": focus_of(circuit, finding.span),
        });
        match finding.severity {
            Severity::Error => errors.push(value),
            Severity::Warning => warnings.push(value),
        }
    }
    json!({ "ok": errors.is_empty(), "parts": circuit.parts.len(), "wires": circuit.wires.len(), "errors": errors, "warnings": warnings })
}

/// What to point the user at first among `findings` of check_live:
/// errors before warnings.
fn first_focus(findings: &[Value]) -> Option<String> {
    let of = |severity: &str| {
        findings.iter().filter(|f| f["severity"] == severity).find_map(|f| f["focus"].as_str().map(str::to_string))
    };
    of("error").or_else(|| of("warning"))
}

/// The findings of check_live, each with what it is about: the wiring
/// checks of the circuit `src` reads as, then the warnings of `fit`.
fn findings(src: &str, path: &Path, libraries: &[PathBuf], fit: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    if let Ok(circuit) = c2f_circuit::parse(src, path, libraries) {
        for finding in check::check(&circuit) {
            found.push(json!({
                "severity": finding.severity.name(),
                "source": "wiring",
                "message": finding.message,
                "line": line_at(src, finding.span.start as usize),
                "focus": focus_of(&circuit, finding.span),
            }));
        }
    }
    for part in fit["parts"].as_array().into_iter().flatten() {
        for warning in part["warnings"].as_array().into_iter().flatten() {
            found.push(json!({ "severity": "warning", "source": "fit", "message": warning, "focus": part["name"] }));
        }
    }
    for wire in fit["wires"].as_array().into_iter().flatten() {
        let focus = wire["wire"].as_u64().map(|n| format!("wire:{n}"));
        found.push(json!({ "severity": "warning", "source": "fit", "message": wire["message"], "focus": focus }));
    }
    found
}

fn text(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The point `key` of `args`, if given.
fn point(args: &Value, key: &str) -> Result<Option<[f64; 3]>, String> {
    let Some(value) = args.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let numbers: Option<Vec<f64>> = value.as_array().map(|v| v.iter().filter_map(Value::as_f64).collect());
    match numbers.as_deref() {
        Some(&[x, y, z]) if value.as_array().is_some_and(|v| v.len() == 3) => Ok(Some([x, y, z])),
        _ => Err(format!("`{key}` is three numbers, [x, y, z]")),
    }
}

fn string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn index(args: &Value) -> Result<Option<usize>, String> {
    match args.get("index") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().filter(|&n| n >= 1).map(|n| Some(n as usize)).ok_or_else(|| "`index` is a wire's number, from 1".into()),
    }
}

/// The circuit tools, for [`super::live`]'s list.
pub fn tools() -> Vec<Value> {
    let point = |what: &str| json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": what });
    vec![
        json!({
            "name": "place_component",
            "title": "Place a part in the open circuit",
            "description": "Adds a part to the circuit (.c2fc) open in the active tab, unsaved edits kept, as one step the \
                            user can undo: a `[parts.<name>]` table for the `part` file (a library path as library_parts \
                            lists it, as `electronics/xiao_esp32s3.c2f`, or a file next to the circuit), at `at`, turned \
                            by `rotate`, with its `params` and `color`. When `name` is placed already, moves it to `at` \
                            and turns it to `rotate`, only those keys changed. An unknown part file, or an edit the \
                            wiring checks find an error in, is refused and nothing changes. Waits for the render; the \
                            result gives the circuit's checks (errors and warnings, each with `focus`) and the render.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The part's name in the circuit, as `button`: letters, digits, `_`, `-`." },
                    "part": { "type": "string", "description": "Its file, as `electronics/xiao_esp32s3.c2f`. Needed for a new part." },
                    "at": point("Where it goes, [x, y, z] in millimeters. Needed for a new part."),
                    "rotate": point("Degrees about x, then y, then z."),
                    "params": { "type": "object", "description": "A new part's parameters, as {\"headers\": true}." },
                    "color": { "type": "string", "description": "A new part's color: a name or #rrggbb." },
                },
                "required": ["name"],
            },
        }),
        json!({
            "name": "connect",
            "title": "Wire two pins of the open circuit",
            "description": "Adds a wire from pin `from` to pin `to` (`part.pin`, as `xiao.D3`) to the circuit open in the \
                            active tab, last in the assembly order, with its `color` and the `via` points it passes \
                            through; with `index` (a wire's number from 1, as `wire:N`), changes that wire's ends, color \
                            and via instead, its other keys and comments kept. Before it is made, the wiring checks run \
                            on the circuit as it would be: an error it would bring (a short, a supply above what a pin \
                            takes, two supplies tied...) refuses it with the checker's explanation, and nothing changes: \
                            correct it and try again. Warnings are applied and reported. One undo step; waits for the \
                            render and gives the circuit's checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "A pin, as `xiao.D3`." },
                    "to": { "type": "string", "description": "A pin, as `button.1`." },
                    "color": { "type": "string", "description": "The wire's color: a name (red, black, green...) or #rrggbb." },
                    "via": { "type": "array", "items": point("A point"), "description": "Points the wire passes through, [[x, y, z], ...]." },
                    "index": { "type": "integer", "minimum": 1, "description": "The wire to change, from 1, instead of adding one." },
                },
                "required": ["from", "to"],
            },
        }),
        json!({
            "name": "disconnect",
            "title": "Remove a wire of the open circuit",
            "description": "Removes a wire of the circuit open in the active tab, by `index` (from 1, as `wire:N`) or by its \
                            two pins `from` and `to`, comments kept, as one undo step. Refused, with the checker's \
                            explanation, when the wiring checks would then find an error (a part's supply or ground left \
                            open). Waits for the render and gives the circuit's checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "index": { "type": "integer", "minimum": 1, "description": "The wire, from 1." },
                    "from": { "type": "string", "description": "A pin of the wire, as `xiao.D3`." },
                    "to": { "type": "string", "description": "Its other pin." },
                },
            },
        }),
        json!({
            "name": "check_live",
            "title": "Check the open circuit",
            "description": "Checks the circuit open in the active tab as it is on screen, unsaved edits included: renders \
                            it if it changed, then runs check_circuit (wiring and parts) and fit (clearances, collisions, \
                            covered ports, wires through the enclosure) on its text. Returns both reports and `findings`, \
                            each with its severity and `focus` (a part's name or `wire:N`, which the focus tool takes), \
                            and selects the first one in the view, errors first, so the user sees it; colliding parts \
                            show red or amber.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "assembly",
            "title": "Guide the assembly",
            "description": "Steps the user through wiring the circuit open in the active tab (saved), one wire at a time in \
                            the file's order: `start` turns the assembly mode on, picks out the current wire and turns \
                            the view to its pins; `next` checks it off and goes to the next; `stop` leaves the mode. \
                            Returns the current wire (step from 1, its ends, color, label), the instruction to read out, \
                            and how many are done.",
            "inputSchema": {
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["start", "next", "stop"] } },
                "required": ["action"],
            },
        }),
    ]
}

/// Calls the circuit tool `name`, if it is one.
pub fn call(tools: &LiveTools, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
    Some(match name {
        "place_component" => parse_place(args).and_then(|edit| modify(tools, edit)),
        "connect" => parse_connect(args).and_then(|edit| modify(tools, edit)),
        "disconnect" => parse_disconnect(args).and_then(|edit| modify(tools, edit)),
        "check_live" => check_live(tools),
        "assembly" => match args.get("action").and_then(Value::as_str) {
            Some(action) => tools.ask_circuit(CircuitOp::Assembly(action.into())).map(|done| vec![text(pretty(&done))]),
            None => Err("`action` is `start`, `next` or `stop`".into()),
        },
        _ => return None,
    })
}

fn parse_place(args: &Value) -> Result<CircuitEdit, String> {
    let params = match args.get("params") {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map.clone()),
        Some(_) => return Err("`params` is an object of names and values".into()),
    };
    Ok(CircuitEdit::Place {
        name: string(args, "name").ok_or("`name` names the part in the circuit")?,
        part: string(args, "part"),
        at: point(args, "at")?,
        rotate: point(args, "rotate")?,
        params,
        color: string(args, "color"),
    })
}

fn parse_connect(args: &Value) -> Result<CircuitEdit, String> {
    let via = match args.get("via") {
        None | Some(Value::Null) => None,
        Some(Value::Array(points)) => Some(
            points
                .iter()
                .map(|p| point(&json!({ "via": p }), "via")?.ok_or_else(|| "`via` is a list of [x, y, z]".to_string()))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err("`via` is a list of [x, y, z]".into()),
    };
    Ok(CircuitEdit::Connect {
        from: string(args, "from").ok_or("`from` is a pin, as `xiao.D3`")?,
        to: string(args, "to").ok_or("`to` is a pin, as `button.1`")?,
        color: string(args, "color"),
        via,
        index: index(args)?,
    })
}

fn parse_disconnect(args: &Value) -> Result<CircuitEdit, String> {
    let ends = string(args, "from").zip(string(args, "to"));
    Ok(CircuitEdit::Disconnect { index: index(args)?, ends })
}

/// Makes `edit` in the studio and waits for its render.
fn modify(tools: &LiveTools, edit: CircuitEdit) -> Result<Vec<Value>, String> {
    let done = tools.ask_circuit(CircuitOp::Edit(edit))?;
    let report = tools.wait_render(done["tab_id"].as_u64().unwrap_or(0), done["generation"].as_u64().unwrap_or(0))?;
    Ok(vec![text(pretty(&json!({ "done": done["done"], "check": done["check"], "render": report })))])
}

fn check_live(tools: &LiveTools) -> Result<Vec<Value>, String> {
    let shown = tools.ask_circuit(CircuitOp::Refresh)?;
    let render = tools.wait_render(shown["tab_id"].as_u64().unwrap_or(0), shown["generation"].as_u64().unwrap_or(0))?;
    let call = json!({ "code": shown["text"], "working_dir": shown["dir"] });
    // A report, whether the tool succeeds or finds errors.
    let report = |tool: &str| -> Value {
        let result = c2f_agent::Builtin.call(tool, &call).unwrap_or_else(|| Err(format!("no built-in `{tool}` tool")));
        let raw = match result {
            Ok(content) => content.first().and_then(|c| c["text"].as_str()).unwrap_or_default().to_string(),
            Err(report) => report,
        };
        serde_json::from_str(&raw).unwrap_or(Value::String(raw))
    };
    let (checked, fit) = (report("check_circuit"), report("fit"));
    let src = shown["text"].as_str().unwrap_or_default();
    let libraries: Vec<PathBuf> =
        shown["libraries"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect();
    let found = findings(src, Path::new(shown["path"].as_str().unwrap_or_default()), &libraries, &fit);
    let selected = match first_focus(&found) {
        Some(name) => tools.ask_circuit(CircuitOp::Highlight(name)).ok().map(|done| done["selected"].clone()),
        None => None,
    };
    let ok = checked["ok"] == true && !found.iter().any(|f| f["severity"] == "error");
    Ok(vec![text(pretty(&json!({
        "ok": ok,
        "findings": found,
        "selected": selected,
        "check_circuit": checked,
        "fit": fit,
        "render": render,
    })))])
}

impl StudioApp {
    /// The library folders a circuit's parts are found in, as the worker
    /// reads them: the user's folders, then the defaults.
    pub(super) fn circuit_libraries(&self) -> Vec<PathBuf> {
        self.settings.library_folders.iter().cloned().chain(c2f_eval::default_library_paths()).collect()
    }

    /// Answers a circuit tool's request, on the active tab.
    pub(super) fn answer_circuit(&mut self, ctx: &egui::Context, op: CircuitOp) -> Result<Value, String> {
        if !self.doc.is_circuit() {
            return Err("the active tab is not a circuit (.c2fc): open one with open_file, or show its tab".into());
        }
        match op {
            CircuitOp::Edit(edit) => {
                let edited = edit_circuit(&self.doc.text, &self.doc.source_path(), &self.circuit_libraries(), &edit)?;
                if matches!(edit, CircuitEdit::Disconnect { .. }) {
                    self.doc.selection = None;
                }
                let check = summary(&edited.circuit, &edited.text);
                self.doc.editor.replace_text(ctx, &mut self.doc.text, edited.text, edited.cursor);
                self.doc.console.log(Level::Info, tr!("The agent {}", "L'agent : {}", edited.done));
                self.render_now();
                Ok(json!({ "tab_id": self.doc.id, "generation": self.doc.sent, "done": edited.done, "check": check }))
            }
            CircuitOp::Refresh => {
                self.render_if_changed();
                let path = self.doc.source_path();
                let libraries: Vec<String> = self.circuit_libraries().iter().map(|p| p.display().to_string()).collect();
                Ok(json!({
                    "tab_id": self.doc.id,
                    "generation": self.doc.sent,
                    "text": self.doc.text,
                    "path": path.display().to_string(),
                    "dir": path.parent().unwrap_or(Path::new("")).display().to_string(),
                    "libraries": libraries,
                }))
            }
            CircuitOp::Highlight(name) => {
                let circuit = self.doc.wiring.as_ref().ok_or("the circuit is not read yet")?.circuit.clone();
                let hit = find_hit(&circuit, &name).ok_or_else(|| format!("nothing named `{name}` in the circuit"))?;
                self.select(ctx, Some(hit));
                if let Hit::Part(part) = hit {
                    self.doc.editor.go_to(ctx, &self.doc.text, circuit.parts[part].span.start as usize);
                    if let Some(Some((lo, hi))) = self.doc.part_boxes.get(part).copied() {
                        let viewer = &mut self.doc.viewer;
                        let mut goal = viewer.camera;
                        goal.frame(lo, hi, viewer.aspect());
                        viewer.fly_to(goal);
                    }
                }
                Ok(json!({ "selected": name }))
            }
            CircuitOp::Assembly(action) => {
                match action.as_str() {
                    "start" => {
                        self.set_mount(true);
                        if !self.doc.mount.active {
                            return Err(match &self.doc.wiring {
                                None => "the circuit is not read yet: try again once it renders".into(),
                                Some(w) if w.circuit.wires.is_empty() => "the circuit has no wires to lay".into(),
                                Some(_) => "the assembly needs the circuit saved: ask the user to save it".to_string(),
                            });
                        }
                    }
                    "next" if self.doc.mount.active => self.mount_next(),
                    "next" => return Err("the assembly is not started: call assembly with `start`".into()),
                    "stop" => {
                        self.set_mount(false);
                        return Ok(json!({ "active": false }));
                    }
                    other => return Err(format!("`{other}` is not an action: `start`, `next` or `stop`")),
                }
                let wiring = self.mount_wiring().ok_or("the circuit has no wires to lay")?;
                let circuit = &wiring.circuit;
                let names = &wiring.names;
                let step = self.doc.mount.step.min(names.len() - 1);
                let wire = &circuit.wires[step];
                let done = self.doc.mount.done_count(names);
                Ok(json!({
                    "active": true,
                    "step": step + 1,
                    "wires": names.len(),
                    "done": done,
                    "finished": done == names.len(),
                    "wire": {
                        "from": circuit.end_name(wire.from),
                        "to": circuit.end_name(wire.to),
                        "color": wire.color,
                        "label": wire.label,
                    },
                    "instruction": mount::instruction(circuit, step),
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The winder's circuit, as the example has it.
    fn winder() -> (String, PathBuf) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/winder/winder.c2fc");
        (std::fs::read_to_string(&path).unwrap(), path)
    }

    fn libraries() -> Vec<PathBuf> {
        c2f_eval::default_library_paths()
    }

    fn edit(src: &str, path: &Path, edit: CircuitEdit) -> Result<Edited, String> {
        edit_circuit(src, path, &libraries(), &edit)
    }

    fn place(name: &str, part: Option<&str>, at: Option<[f64; 3]>, rotate: Option<[f64; 3]>) -> CircuitEdit {
        CircuitEdit::Place { name: name.into(), part: part.map(Into::into), at, rotate, params: None, color: None }
    }

    fn connect(from: &str, to: &str, color: Option<&str>) -> CircuitEdit {
        CircuitEdit::Connect { from: from.into(), to: to.into(), color: color.map(Into::into), via: None, index: None }
    }

    /// Every comment line of `src` is in `text`.
    fn comments_kept(src: &str, text: &str) {
        for line in src.lines().filter(|l| l.trim_start().starts_with('#')) {
            assert!(text.contains(line), "lost `{line}`");
        }
    }

    /// The library has no button or switch: the ceramic capacitor, a
    /// plain two-pin part, stands in for one.
    const BUTTON: &str = "electronics/capacitor_ceramic.c2f";

    #[test]
    fn a_part_is_added_then_moved_and_turned_in_place() {
        let (src, path) = winder();
        let added = edit(&src, &path, place("button", Some(BUTTON), Some([30.0, 0.0, 4.0]), None)).unwrap();
        let table = "[parts.button]\npart = \"electronics/capacitor_ceramic.c2f\"\nat = [30, 0, 4]\n";
        assert!(added.text.contains(&format!("rotate = [60, 0, 0]\n\n{table}\n# Power")), "{}", added.text);
        assert_eq!(added.circuit.parts.len(), 4);
        assert_eq!(added.circuit.parts[3].at, [30.0, 0.0, 4.0]);
        comments_kept(&src, &added.text);
        assert_eq!(added.done, "added `button` (electronics/capacitor_ceramic.c2f) at [30, 0, 4]");

        let moved = edit(&added.text, &path, place("button", None, Some([32.5, -1.0, 4.0]), Some([0.0, 0.0, 90.0]))).unwrap();
        assert_eq!(
            moved.text,
            added.text.replace(
                "capacitor_ceramic.c2f\"\nat = [30, 0, 4]\n",
                "capacitor_ceramic.c2f\"\nrotate = [0, 0, 90]\nat = [32.5, -1, 4]\n"
            )
        );
        let button = &moved.circuit.parts[3];
        assert_eq!((button.at, button.rotate), ([32.5, -1.0, 4.0], [0.0, 0.0, 90.0]));
        // Moved again: the keys written are rewritten, not added.
        let again = edit(&moved.text, &path, place("button", None, Some([0.0, 0.0, 4.0]), None)).unwrap();
        assert_eq!(again.text, moved.text.replace("at = [32.5, -1, 4]", "at = [0, 0, 4]"));

        // Params and a color on a new part.
        let mut params = Map::new();
        params.insert("headers".into(), json!(true));
        let colored = CircuitEdit::Place {
            name: "amp".into(),
            part: Some("electronics/max98357a.c2f".into()),
            at: Some([-30.0, 0.0, 4.0]),
            rotate: None,
            params: Some(params),
            color: Some("purple".into()),
        };
        let amp = edit(&src, &path, colored).unwrap();
        assert!(amp.text.contains("at = [-30, 0, 4]\nparams = { headers = true }\ncolor = \"purple\"\n"), "{}", amp.text);
    }

    #[test]
    fn unknown_part_files_and_bad_requests_are_refused() {
        let (src, path) = winder();
        let err = edit(&src, &path, place("x", Some("electronics/nothing.c2f"), Some([0.0; 3]), None)).unwrap_err();
        assert!(err.contains("cannot find `electronics/nothing.c2f`"), "{err}");
        assert!(edit(&src, &path, place("a.b", Some(BUTTON), Some([0.0; 3]), None)).unwrap_err().contains("not a part name"));
        assert!(edit(&src, &path, place("new", Some(BUTTON), None, None)).unwrap_err().contains("needs `at`"));
        let err = edit(&src, &path, place("xiao", Some(BUTTON), Some([0.0; 3]), None)).unwrap_err();
        assert!(err.contains("is a `electronics/xiao_esp32s3.c2f` already"), "{err}");
        let err = edit(&src, &path, connect("xiao.D99", "driver.IN1", None)).unwrap_err();
        assert!(err.contains("no pin `D99`") && err.contains("D10"), "{err}");
        assert!(edit(&src, &path, connect("xiao.D5", "driver.IN1", Some("glitter"))).unwrap_err().contains("not a color"));
    }

    #[test]
    fn a_button_is_wired_between_d3_and_ground_in_green() {
        let (src, path) = winder();
        let mut text = edit(&src, &path, place("button", Some(BUTTON), Some([30.0, 0.0, 4.0]), None)).unwrap().text;
        for (from, to) in [("button.1", "xiao.D3"), ("button.2", "xiao.GND")] {
            let done = edit(&text, &path, connect(from, to, Some("green"))).unwrap();
            assert!(done.text.ends_with(&format!("[[wires]]\nfrom = \"{from}\"\nto = \"{to}\"\ncolor = \"green\"\n")), "{}", done.text);
            assert_eq!(summary(&done.circuit, &done.text)["errors"], json!([]));
            text = done.text;
        }
        comments_kept(&src, &text);
        let circuit = c2f_circuit::parse(&text, &path, &libraries()).unwrap();
        assert_eq!(circuit.wires.len(), 13);
        let last = &circuit.wires[12];
        assert_eq!((circuit.end_name(last.from), circuit.end_name(last.to)), ("button.2".into(), "xiao.GND".into()));
        assert_eq!(last.color.as_deref(), Some("green"));

        // Changed in place: ends, color and a via added; the label kept.
        let rewired = CircuitEdit::Connect {
            from: "xiao.D5".into(),
            to: "driver.IN3".into(),
            color: Some("white".into()),
            via: Some(vec![[1.0, 2.0, 3.0]]),
            index: Some(5),
        };
        let done = edit(&text, &path, rewired).unwrap();
        assert_eq!(
            done.text,
            text.replace(
                "from = \"xiao.D3\"\nto = \"driver.IN3\"\ncolor = \"pink\"\n",
                "from = \"xiao.D5\"\nto = \"driver.IN3\"\ncolor = \"white\"\nvia = [[1, 2, 3]]\n"
            )
        );
        // The motor cable's via, rewritten where it is.
        let via = CircuitEdit::Connect {
            from: "driver.A".into(),
            to: "motor.blue".into(),
            color: None,
            via: Some(vec![[-12.0, 0.0, 25.0]]),
            index: Some(7),
        };
        let done = edit(&text, &path, via).unwrap();
        assert_eq!(done.text, text.replace("via = [[-10, 0, 25]]", "via = [[-12, 0, 25]]"));
        assert!(done.text.contains("# Around the plate's underside"));
    }

    #[test]
    fn a_wire_is_removed_by_its_pins_or_its_number() {
        let (src, path) = winder();
        let by_pins = CircuitEdit::Disconnect { index: None, ends: Some(("driver.IN4".into(), "xiao.D4".into())) };
        let done = edit(&src, &path, by_pins).unwrap();
        assert_eq!(done.text, src.replace("\n[[wires]]\nfrom = \"xiao.D4\"\nto = \"driver.IN4\"\ncolor = \"blue\"\n", ""));
        assert_eq!(done.done, "removed wire 6: xiao.D4 ➡ driver.IN4");
        comments_kept(&src, &done.text);
        let by_number = edit(&done.text, &path, CircuitEdit::Disconnect { index: Some(5), ends: None }).unwrap();
        assert_eq!(by_number.circuit.wires.len(), 9);
        assert_eq!(by_number.done, "removed wire 5: xiao.D3 ➡ driver.IN3");
        assert!(!by_number.text.contains("driver.IN3"), "{}", by_number.text);
        comments_kept(&src, &by_number.text);
        // The motor's supply taken away is refused: its other pins are wired.
        let err = edit(&src, &path, CircuitEdit::Disconnect { index: Some(11), ends: None }).unwrap_err();
        assert!(err.contains("`motor.red` is not wired"), "{err}");
        let none = CircuitEdit::Disconnect { index: None, ends: Some(("xiao.D9".into(), "xiao.D8".into())) };
        assert!(edit(&src, &path, none).unwrap_err().contains("no wire runs"));
        assert!(edit(&src, &path, CircuitEdit::Disconnect { index: Some(40), ends: None }).unwrap_err().contains("no wire 40"));
    }

    #[test]
    fn a_short_is_refused_with_the_checker_s_explanation() {
        let (src, path) = winder();
        let err = edit(&src, &path, connect("xiao.5V", "xiao.GND", Some("red"))).unwrap_err();
        assert!(err.starts_with("refused, nothing was changed"), "{err}");
        assert!(err.contains("`xiao.5V` is a supply wired to the ground") && err.contains("short circuit"), "{err}");
        // 5 V onto the XIAO's 3.3 V pin.
        let err = edit(&src, &path, connect("xiao.5V", "xiao.D5", None)).unwrap_err();
        assert!(err.contains("takes at most 3.3 V"), "{err}");
        // The driver's ground taken away leaves its supply wired alone.
        let ground = CircuitEdit::Disconnect { index: None, ends: Some(("xiao.GND".into(), "driver.GND".into())) };
        let err = edit(&src, &path, ground).unwrap_err();
        assert!(err.starts_with("refused"), "{err}");
    }

    /// The app without a window, on `file`.
    fn studio(ctx: &egui::Context, file: &Path) -> StudioApp {
        let stores = super::super::Stores { session: None, settings: None, conversations: None };
        let settings = crate::settings::Settings::load(None);
        StudioApp::with(ctx, None, stores, crate::recent::RecentFiles::default(), settings, vec![file.to_path_buf()])
    }

    /// Calls the live tool `name` from another thread, as an agent does,
    /// while the app answers it frame after frame.
    fn call(app: &mut StudioApp, ctx: &egui::Context, name: &str, args: Value) -> Result<Value, String> {
        let tools = app.live_tools();
        let name = name.to_string();
        let caller = std::thread::spawn(move || tools.call(&name, &args).expect("a live tool"));
        let start = std::time::Instant::now();
        while !caller.is_finished() {
            assert!(start.elapsed() < std::time::Duration::from_secs(120), "no answer");
            app.receive();
            app.serve_live(ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let content = caller.join().unwrap()?;
        Ok(serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap())
    }

    #[test]
    fn the_agent_wires_the_open_circuit_live() {
        let dir = std::env::temp_dir().join(format!("c2f-live-circuit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lamp.c2fc");
        let saved = "# A board alone.\n[parts.xiao]\npart = \"electronics/xiao_esp32s3.c2f\"\n";
        std::fs::write(&file, saved).unwrap();
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, &file);
        // Read and rendered once.
        let state = call(&mut app, &ctx, "check_live", json!({})).unwrap();
        assert_eq!(state["check_circuit"]["ok"], true, "{state}");

        let placed = call(&mut app, &ctx, "place_component", json!({ "name": "button", "part": BUTTON, "at": [30, 0, 0] })).unwrap();
        assert_eq!(placed["check"]["parts"], 2, "{placed}");
        assert_eq!(placed["render"]["parts"], 2, "{placed}");
        // A signal before the ground leaves the XIAO's ground open.
        let refused = call(&mut app, &ctx, "connect", json!({ "from": "button.1", "to": "xiao.D3" })).unwrap_err();
        assert!(refused.contains("ground is left open"), "{refused}");
        let wired = call(&mut app, &ctx, "connect", json!({ "from": "button.2", "to": "xiao.GND", "color": "green" })).unwrap();
        assert_eq!(wired["check"]["wires"], 1, "{wired}");
        let wired = call(&mut app, &ctx, "connect", json!({ "from": "button.1", "to": "xiao.D3", "color": "green" })).unwrap();
        assert_eq!(wired["check"]["wires"], 2, "{wired}");
        assert_eq!(wired["check"]["errors"], json!([]), "{wired}");
        let refused = call(&mut app, &ctx, "connect", json!({ "from": "xiao.5V", "to": "xiao.GND" })).unwrap_err();
        assert!(refused.contains("short circuit"), "{refused}");
        assert!(app.doc.text.contains("[[wires]]\nfrom = \"button.1\"\nto = \"xiao.D3\"\ncolor = \"green\"\n"), "{}", app.doc.text);
        assert!(app.doc.text.starts_with("# A board alone.\n"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), saved, "the file is not saved");

        // Each edit is one step of the editor's undo history.
        let mut text = app.doc.text.clone();
        assert!(app.doc.editor.undo_step(&ctx, &mut text, true));
        assert!(!text.contains("xiao.D3") && text.contains("xiao.GND"), "{text}");

        // The findings come with what they are about, picked out.
        let checked = call(&mut app, &ctx, "check_live", json!({})).unwrap();
        assert!(checked["findings"].is_array(), "{checked}");
        assert!(checked["fit"]["parts"].is_array(), "{checked}");

        // The assembly, once saved.
        let refused = call(&mut app, &ctx, "assembly", json!({ "action": "next" })).unwrap_err();
        assert!(refused.contains("not started"), "{refused}");
        std::fs::write(&file, &app.doc.text).unwrap();
        let started = call(&mut app, &ctx, "assembly", json!({ "action": "start" })).unwrap();
        assert_eq!(started["step"], 1, "{started}");
        assert_eq!(started["wire"]["from"], "button.2", "{started}");
        assert!(started["instruction"].as_str().unwrap().starts_with("Green wire"), "{started}");
        let next = call(&mut app, &ctx, "assembly", json!({ "action": "next" })).unwrap();
        assert_eq!((next["step"].as_u64(), next["done"].as_u64()), (Some(2), Some(1)), "{next}");
        assert_eq!(next["wire"]["to"], "xiao.D3", "{next}");
        let last = call(&mut app, &ctx, "assembly", json!({ "action": "next" })).unwrap();
        assert_eq!((last["done"].as_u64(), last["finished"].as_bool()), (Some(2), Some(true)), "{last}");
        assert_eq!(call(&mut app, &ctx, "assembly", json!({ "action": "stop" })).unwrap()["active"], false);
        assert!(!app.doc.mount.active);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
