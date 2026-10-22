//! The MCP tools for electronics: the library's parts and their pins, then
//! a circuit (`.c2fc`) checked, listed and pictured with its wires.

use std::path::{Path, PathBuf};

use c2f_circuit::check::{self, Severity};
use c2f_circuit::{Circuit, End, route};
use c2f_lang::annotations::{self, PortSize};
use serde_json::{Value, json};

use super::{IMAGE_SIZE, Tool, base64, pretty, schema_of, text, view_and_size};
use crate::circuit::{assemble, bom, wires};

/// The library folder parts are listed from.
const ELECTRONICS: &str = "electronics";
/// A wire's radius in pictures, in millimeters, thicker in a highlighted
/// net, and the sides of its tube.
const WIRE_RADIUS: f64 = 0.5;
const HIGHLIGHT_RADIUS: f64 = 0.8;
const WIRE_SIDES: usize = 8;
/// Wires without a color, and those left out of a highlighted net.
const WIRE_COLOR: [f32; 4] = [0.85, 0.85, 0.8, 1.0];
const GREYED: [f32; 4] = [0.4, 0.4, 0.42, 1.0];

pub fn tools() -> Vec<Value> {
    let circuit = || {
        json!({
            "code": { "type": "string", "description": "The circuit's text (TOML, a .c2fc file). Give this or `path`." },
            "path": { "type": "string", "description": "A .c2fc file to read instead of `code`; its parts resolve from its folder, then the library." },
            "working_dir": {
                "type": "string",
                "description": "With `code`: the folder its part files resolve from before the library (default: the server's current directory).",
            },
        })
    };
    let views: Vec<&str> = c2f_geom::VIEWS.iter().map(|(name, _, _)| *name).chain(["isometric"]).collect();
    vec![
        json!({
            "name": "library_parts",
            "title": "List the electronics parts",
            "description": "Lists the parts in the `electronics/` folders of the library paths: the name a circuit gives as \
                            `part` (as `electronics/xiao_esp32s3.c2f`), the file, and its first comment line.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "part_pins",
            "title": "A part's pins",
            "description": "The pins a part declares in `// @pin` comments (name, label, type, voltage, reserved, position \
                            and direction, in the part's millimeters), its mount holes and its ports, with the annotations \
                            badly written. A `path` is read as is, else from `working_dir`, else from the library paths.",
            "inputSchema": schema_of(json!({
                "code": { "type": "string", "description": "The part's text. Give this or `path`." },
                "path": { "type": "string", "description": "The part's file, as a circuit names it (`electronics/inmp441.c2f`) or a path." },
                "working_dir": { "type": "string", "description": "A folder `path` is read from before the library." },
            }), &[]),
        }),
        json!({
            "name": "check_circuit",
            "title": "Check a circuit",
            "description": "Reads a circuit (`.c2fc`: parts placed with `at` and `rotate`, `params`, and `[[wires]]` from \
                            `part.pin` to `part.pin` with a `color` and a `label`), evaluates its parts and checks its \
                            wiring. Errors (`ok` false) and warnings come with their file, line and column: a supply above \
                            what a pin takes, supplies tied together or to ground, a supply or ground left open, a wired nc \
                            pin; no common ground, a wired reserved pin, two outputs on one net.",
            "inputSchema": schema_of(circuit(), &[]),
        }),
        json!({
            "name": "circuit_image",
            "title": "Picture of a circuit",
            "description": "A PNG picture of a circuit: its parts where it places them and its wires as tubes in their \
                            colors, from a view as `image` takes. `net` (a pin, as `xiao.GND`) highlights the wires of its \
                            net and greys the others.",
            "inputSchema": schema_of(super::merge(circuit(), json!({
                "view": { "type": "string", "enum": views, "default": "diagonal", "description": "Where the camera looks from." },
                "width": { "type": "integer", "minimum": 1, "maximum": super::MAX_IMAGE_SIZE, "default": IMAGE_SIZE.0, "description": "In pixels." },
                "height": { "type": "integer", "minimum": 1, "maximum": super::MAX_IMAGE_SIZE, "default": IMAGE_SIZE.1, "description": "In pixels." },
                "net": { "type": "string", "description": "A pin, as `part.pin`, whose net to highlight." },
            })), &[]),
        }),
        json!({
            "name": "circuit_wires",
            "title": "Wiring list",
            "description": "The wires in the order to lay them: step, each pin and its label, color, label, and the length \
                            of the curve drawn, in millimeters.",
            "inputSchema": schema_of(circuit(), &[]),
        }),
        json!({
            "name": "circuit_bom",
            "title": "Bill of materials",
            "description": "The parts by file and params: name, quantity, file, params and the instances' names.",
            "inputSchema": schema_of(circuit(), &[]),
        }),
    ]
}

/// The tool named `name`, if it is one of these.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "library_parts" => library_parts,
        "part_pins" => part_pins,
        "check_circuit" => check_circuit,
        "circuit_image" => circuit_image,
        "circuit_wires" => circuit_wires,
        "circuit_bom" => circuit_bom,
        _ => return None,
    })
}

fn library_parts(_: &Value) -> Result<Vec<Value>, String> {
    let mut parts: Vec<(String, PathBuf)> = Vec::new();
    for library in c2f_eval::default_library_paths() {
        let mut files = Vec::new();
        walk(&library.join(ELECTRONICS), &mut files);
        for file in files {
            let Ok(name) = file.strip_prefix(&library) else { continue };
            let name = name.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            // The first library that has a name is the one a circuit reads.
            if !parts.iter().any(|(n, _)| *n == name) {
                parts.push((name, file));
            }
        }
    }
    parts.sort();
    let parts: Vec<Value> = parts
        .iter()
        .map(|(name, file)| {
            let description = std::fs::read_to_string(file).ok().and_then(|src| first_comment(&src));
            json!({ "part": name, "file": file.display().to_string(), "description": description })
        })
        .collect();
    Ok(vec![text(pretty(&json!({ "parts": parts })))])
}

/// The `.c2f` and `.scad` files under `dir`.
fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().is_some_and(|e| e == "c2f" || e == "scad") {
            files.push(path);
        }
    }
}

/// The text of the first `//` comment line, if the file starts with one.
fn first_comment(src: &str) -> Option<String> {
    let line = src.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.strip_prefix("//")?.trim().to_string())
}

fn part_pins(args: &Value) -> Result<Vec<Value>, String> {
    let string = |key: &str| args.get(key).and_then(Value::as_str);
    let (src, file) = match (string("code"), string("path")) {
        (Some(code), None) => (code.to_string(), None),
        (None, Some(path)) => {
            let dirs = string("working_dir").map(PathBuf::from).into_iter().chain(c2f_eval::default_library_paths());
            let file = std::iter::once(PathBuf::from(path))
                .chain(dirs.map(|dir| dir.join(path)))
                .find(|file| file.is_file())
                .ok_or_else(|| format!("cannot find `{path}`, as is, in `working_dir` or in the library folders"))?;
            let src = c2f_eval::read_source(&file).map_err(|err| format!("cannot read {}: {err}", file.display()))?;
            (src, Some(file))
        }
        _ => return Err("give either `code` (the part's text) or `path` (its file)".into()),
    };
    let (annotations, errors) = annotations::read(&src);
    let lines = c2f_lang::span::LineIndex::new(&src);
    let line = |span: c2f_lang::Span| lines.line(span.start as usize) + 1;
    let pins: Vec<Value> = annotations
        .pins
        .iter()
        .map(|pin| {
            json!({
                "name": pin.name,
                "label": pin.label,
                "type": pin.kind.name(),
                "voltage": pin.voltage,
                "reserved": pin.reserved,
                "position": pin.position,
                "direction": pin.direction,
                "line": line(pin.span),
            })
        })
        .collect();
    let mount_holes: Vec<Value> = annotations
        .mount_holes
        .iter()
        .map(|hole| json!({ "position": hole.position, "diameter": hole.diameter, "direction": hole.direction }))
        .collect();
    let ports: Vec<Value> = annotations
        .ports
        .iter()
        .map(|port| {
            let mut value = json!({ "name": port.name, "label": port.label, "position": port.position, "direction": port.direction });
            match port.size {
                PortSize::Rect(size) => value["size"] = json!(size),
                PortSize::Round(d) => value["diameter"] = json!(d),
            }
            value
        })
        .collect();
    let errors: Vec<Value> = errors
        .iter()
        .map(|e| {
            let (line, column) = lines.line_col(&src, (e.span.start as usize).min(src.len()));
            json!({ "message": e.message, "line": line + 1, "column": column + 1 })
        })
        .collect();
    let mut report = json!({ "pins": pins, "mount_holes": mount_holes, "ports": ports, "errors": errors });
    if let Some(file) = file {
        report["file"] = json!(file.display().to_string());
    }
    Ok(vec![text(pretty(&report))])
}

/// The circuit the arguments name: its text and path. The error is the
/// report the agent reads.
fn load(args: &Value) -> Result<(Circuit, String, PathBuf), String> {
    let string = |key: &str| args.get(key).and_then(Value::as_str);
    let (src, path) = match (string("code"), string("path")) {
        (Some(code), None) => {
            let dir = match string("working_dir") {
                Some(dir) => PathBuf::from(dir),
                None => std::env::current_dir().unwrap_or_default(),
            };
            (code.to_string(), dir.join("untitled.c2fc"))
        }
        (None, Some(path)) => {
            let src = std::fs::read_to_string(path).map_err(|err| format!("cannot read {path}: {err}"))?;
            (src, PathBuf::from(path))
        }
        _ => return Err("give either `code` (the circuit's text) or `path` (a .c2fc file)".into()),
    };
    match c2f_circuit::parse(&src, &path, &c2f_eval::default_library_paths()) {
        Ok(circuit) => Ok((circuit, src, path)),
        Err(errors) => {
            let errors: Vec<Value> = errors.iter().map(|e| problem(&e.path, &e.src, &e.diagnostic)).collect();
            Err(pretty(&json!({ "ok": false, "errors": errors, "warnings": [] })))
        }
    }
}

/// An error or warning with its file, line and column.
fn problem(path: &Path, src: &str, diagnostic: &c2f_lang::Diagnostic) -> Value {
    let offset = (diagnostic.span.start as usize).min(src.len());
    let (line, column) = c2f_lang::span::LineIndex::new(src).line_col(src, offset);
    json!({ "message": diagnostic.message, "file": path.display().to_string(), "line": line + 1, "column": column + 1 })
}

fn check_circuit(args: &Value) -> Result<Vec<Value>, String> {
    let (circuit, src, path) = load(args)?;
    let assembly = assemble(&circuit, &src, &path);
    let mut errors: Vec<Value> = assembly.errors.iter().map(|e| problem(&e.path, &e.src, &e.diagnostic)).collect();
    let mut warnings: Vec<Value> = assembly.warnings.iter().map(|w| json!({ "message": w })).collect();
    for finding in check::check(&circuit) {
        let value = problem(&path, &src, &c2f_lang::Diagnostic::new(finding.message, finding.span));
        match finding.severity {
            Severity::Error => errors.push(value),
            Severity::Warning => warnings.push(value),
        }
    }
    let report = json!({
        "ok": errors.is_empty(),
        "parts": circuit.parts.len(),
        "wires": circuit.wires.len(),
        "errors": errors,
        "warnings": warnings,
    });
    if errors.is_empty() { Ok(vec![text(pretty(&report))]) } else { Err(pretty(&report)) }
}

fn circuit_wires(args: &Value) -> Result<Vec<Value>, String> {
    let (circuit, _, _) = load(args)?;
    Ok(vec![text(pretty(&json!({ "wires": objects(&wires(&circuit)) })))])
}

fn circuit_bom(args: &Value) -> Result<Vec<Value>, String> {
    let (circuit, _, _) = load(args)?;
    Ok(vec![text(pretty(&json!({ "parts": objects(&bom(&circuit)) })))])
}

/// Rows under a header as JSON objects, numbers as numbers and empty
/// values as null.
fn objects(rows: &[Vec<String>]) -> Vec<Value> {
    rows[1..]
        .iter()
        .map(|row| {
            let fields = rows[0].iter().zip(row).map(|(key, value)| {
                let value = match value.parse::<f64>() {
                    _ if value.is_empty() => Value::Null,
                    Ok(n) if ["step", "quantity"].contains(&key.as_str()) => json!(n as u64),
                    Ok(n) if key.ends_with("_mm") => json!(n),
                    _ => json!(value),
                };
                (key.clone(), value)
            });
            Value::Object(fields.collect())
        })
        .collect()
}

fn circuit_image(args: &Value) -> Result<Vec<Value>, String> {
    let (yaw, pitch, width, height) = view_and_size(args)?;
    let (circuit, src, path) = load(args)?;
    let highlight = match args.get("net").and_then(Value::as_str) {
        None => None,
        Some(name) => Some(check::net_of(&circuit, end_named(&circuit, name)?)),
    };
    let assembly = assemble(&circuit, &src, &path);
    if !assembly.errors.is_empty() {
        let errors: Vec<Value> = assembly.errors.iter().map(|e| problem(&e.path, &e.src, &e.diagnostic)).collect();
        return Err(pretty(&json!({ "ok": false, "errors": errors })));
    }
    let mut rendered = assembly.render(&path);
    for (wire, points) in circuit.wires.iter().zip(route::paths(&circuit)) {
        let lit = highlight.as_ref().is_none_or(|net| net.contains(&wire.from));
        let color = match &wire.color {
            _ if !lit => GREYED,
            Some(name) => color_named(name).ok_or_else(|| format!("unknown wire color `{name}`"))?,
            None => WIRE_COLOR,
        };
        let radius = if highlight.is_some() && lit { HIGHLIGHT_RADIUS } else { WIRE_RADIUS };
        let tube = route::tube(&points, radius, WIRE_SIDES);
        for corners in tube.indices.chunks_exact(3) {
            rendered.solid.push_triangle(std::array::from_fn(|k| tube.positions[corners[k] as usize]), [color; 3]);
        }
    }
    let png = rendered.png(yaw, pitch, width, height).ok_or("the circuit is empty: nothing to draw")?;
    let mut report = json!({ "parts": circuit.parts.len(), "wires": circuit.wires.len() });
    if let Some(net) = &highlight {
        report["net"] = json!(net.iter().map(|&end| circuit.end_name(end)).collect::<Vec<_>>());
    }
    Ok(vec![json!({ "type": "image", "data": base64(&png), "mimeType": "image/png" }), text(pretty(&report))])
}

/// The pin a circuit names `part.pin`.
fn end_named(circuit: &Circuit, name: &str) -> Result<End, String> {
    let (part_name, pin_name) = name.split_once('.').ok_or_else(|| format!("`net` is a pin, as `xiao.GND`, not `{name}`"))?;
    let part = circuit.parts.iter().position(|p| p.name == part_name).ok_or_else(|| format!("there is no part named `{part_name}`"))?;
    let pin = circuit.parts[part].pin(pin_name).ok_or_else(|| format!("`{part_name}` has no pin named `{pin_name}`"))?;
    Ok(End { part, pin })
}

/// A color as `color()` reads it, a name (`red`) or `#rrggbb`: the
/// evaluator holds the table, so it evaluates a colored cube.
fn color_named(name: &str) -> Option<[f32; 4]> {
    let src = format!("color(\"{}\") cube(1);", name.replace(['"', '\\'], ""));
    let program = c2f_eval::Program::load(&src, Path::new("color.scad"), &[]).ok()?;
    let output = c2f_eval::evaluate(&program);
    match output.nodes.first()?.kind {
        c2f_eval::NodeKind::Color(rgba) if output.warnings.is_empty() => Some(rgba.map(|c| c as f32)),
        _ => None,
    }
}
