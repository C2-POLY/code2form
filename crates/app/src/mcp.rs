//! `c2f mcp`: a Model Context Protocol server, so that an AI agent can
//! write a model, render it, look at it and export it. JSON-RPC 2.0, one
//! message per line on stdin and stdout; logs go to stderr.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use c2f_eval::customizer::{HIDDEN_GROUP, Literal, Widget, parse_typed};
use serde_json::{Map, Value, json};

use crate::Export;

mod circuit;

/// The protocol versions this server speaks, the latest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "\
Code2Form models parts as programs in the .scad language. Give a tool the \
program's text (`code`) or a file (`path`). `render` reports errors with \
their line, echo output, warnings, the triangle count, the volume and the \
bounding box; `image` returns a picture of the model from a view; \
`parameters` lists the customizer parameters, which every tool can set \
with `parameters`; `export` writes STL or 3MF (or SVG and DXF for 2D). \
`pause(z, message);` pauses a 3D print once it is done up to height `z`, \
to drop magnets or nuts into closed cavities: the 3MF carries the pauses \
for PrusaSlicer. For electronics, `library_parts` lists the parts and \
`part_pins` their pins; a circuit (`.c2fc`, TOML) places parts and wires \
their pins: `check_circuit` checks it (voltages, grounds, reserved pins), \
`circuit_image` pictures it with its wires, `circuit_wires` and \
`circuit_bom` list its wires in order and its parts.";

/// The picture's size when the agent gives none, and the largest it can ask.
const IMAGE_SIZE: (u64, u64) = (800, 600);
const MAX_IMAGE_SIZE: u64 = 1600;

pub fn serve() -> ExitCode {
    eprintln!("c2f mcp: Code2Form {} serving on stdin and stdout", env!("CARGO_PKG_VERSION"));
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("c2f mcp: cannot read stdin: {err}");
                return ExitCode::FAILURE;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = handle(&line) else { continue };
        if let Err(err) = writeln!(stdout, "{reply}").and_then(|()| stdout.flush()) {
            eprintln!("c2f mcp: cannot write stdout: {err}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// The reply to one message; none for notifications and responses.
fn handle(line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(err) => return Some(failure(Value::Null, -32700, format!("parse error: {err}"))),
    };
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // This server sends no requests, so it expects no responses.
        if message.get("result").is_some() || message.get("error").is_some() {
            return None;
        }
        return Some(failure(id.unwrap_or(Value::Null), -32600, "invalid request: no method".into()));
    };
    let Some(id) = id else {
        // Notifications (`notifications/initialized`, `.../cancelled`)
        // need no answer.
        return None;
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(&params),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => failure(id, code, message),
    })
}

fn failure(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Answers with the client's protocol version when this server speaks it,
/// else with its latest.
fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "code2form", "title": "Code2Form", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn tools() -> Value {
    let views: Vec<&str> = c2f_geom::VIEWS.iter().map(|(name, _, _)| *name).chain(["isometric"]).collect();
    let mut tools = json!([
        {
            "name": "render",
            "title": "Render a model",
            "description": "Evaluates and renders a model. Reports the error that stops it (with its file, line and column), \
                            the echo output, warnings, whether the model is 3D or 2D, its triangle and vertex counts, \
                            its volume (area in 2D) and its bounding box, in millimeters.",
            "inputSchema": schema(json!({}), &[]),
        },
        {
            "name": "parameters",
            "title": "List customizer parameters",
            "description": "Lists the customizer parameters the main file declares in comments (`width = 20; // [10:100]`): \
                            name, group, description, default value and widget (slider range, dropdown options...).",
            "inputSchema": model_schema(json!({}), &[]),
        },
        {
            "name": "image",
            "title": "Picture of a model",
            "description": "Renders a model and returns a PNG picture of it, framed to fit, in the app's colors, \
                            with the render report. Z is up; `front` looks along +Y, `top` down with X right and Y up, \
                            `diagonal` (or `isometric`) is the app's default view. A 2D model shows as a 1 mm slab, \
                            best seen from `top`.",
            "inputSchema": schema(json!({
                "view": { "type": "string", "enum": views, "default": "diagonal", "description": "Where the camera looks from." },
                "width": { "type": "integer", "minimum": 1, "maximum": MAX_IMAGE_SIZE, "default": IMAGE_SIZE.0, "description": "In pixels." },
                "height": { "type": "integer", "minimum": 1, "maximum": MAX_IMAGE_SIZE, "default": IMAGE_SIZE.1, "description": "In pixels." },
            }), &[]),
        },
        {
            "name": "export",
            "title": "Export a model",
            "description": "Renders a model and writes it to a file: a solid as STL or 3MF (in millimeters), \
                            a 2D model as SVG or DXF. Returns the render report and the size written.",
            "inputSchema": schema(json!({
                "output": { "type": "string", "description": "The file to write, absolute or relative to the server's current directory; it is replaced." },
                "format": { "type": "string", "enum": ["stl", "3mf", "svg", "dxf"], "description": "Default: the output's extension." },
            }), &["output"]),
        },
    ]);
    if let Some(tools) = tools.as_array_mut() {
        tools.extend(circuit::tools());
    }
    tools
}

/// The input schema of a tool that names a model, with `properties` and
/// `required` added: a program and the parameter values to use.
fn schema(properties: Value, required: &[&str]) -> Value {
    let parameters = json!({
        "parameters": {
            "type": "object",
            "description": "Customizer parameter values to use instead of the file's, by name, as JSON values \
                            (`{\"width\": 30, \"label\": \"A\", \"size\": [10, 20]}`); the `parameters` tool lists them.",
        },
    });
    model_schema(merge(parameters, properties), required)
}

fn model_schema(properties: Value, required: &[&str]) -> Value {
    let model = json!({
        "code": { "type": "string", "description": "The program's text. Give this or `path`." },
        "path": {
            "type": "string",
            "description": "A .scad or .c2f file to read instead of `code`; its includes, `use`s and imported files resolve from its folder.",
        },
        "working_dir": {
            "type": "string",
            "description": "With `code`: the folder its includes, `use`s and imported files resolve from (default: the server's current directory).",
        },
    });
    schema_of(merge(model, properties), required)
}

fn schema_of(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
    a
}

/// A tool: its content, or the error the agent reads.
type Tool = fn(&Value) -> Result<Vec<Value>, String>;

/// Runs a tool. Its failures are results marked as errors, which the agent
/// reads; only a call that names no known tool is a protocol error.
fn call(params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "tools/call needs the tool's `name`".into()))?;
    let tool: Tool = match name {
        "render" => render_tool,
        "parameters" => parameters_tool,
        "image" => image_tool,
        "export" => export_tool,
        _ => circuit::tool(name).ok_or((-32602, format!("unknown tool: {name}")))?,
    };
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !args.is_object() {
        return Err((-32602, "the tool's `arguments` must be an object".into()));
    }
    // A panic in the geometry ends this call, not the server.
    let reply = std::panic::catch_unwind(|| tool(&args))
        .unwrap_or_else(|_| Err(format!("internal error: `{name}` panicked (the server's log says where)")));
    Ok(match reply {
        Ok(content) => json!({ "content": content, "isError": false }),
        Err(message) => json!({ "content": [text(message)], "isError": true }),
    })
}

fn text(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn render_tool(args: &Value) -> Result<Vec<Value>, String> {
    let (_, report) = render(args)?;
    Ok(vec![text(pretty(&report))])
}

fn parameters_tool(args: &Value) -> Result<Vec<Value>, String> {
    let (program, src) = load(args)?;
    let parameters: Vec<Value> = program
        .parameters(&src)
        .into_iter()
        .filter(|p| p.group != HIDDEN_GROUP)
        .map(|p| {
            json!({
                "name": p.name,
                "group": p.group,
                "description": p.description,
                "default": literal(&p.default),
                "widget": widget(&p.widget),
            })
        })
        .collect();
    Ok(vec![text(pretty(&json!({ "parameters": parameters })))])
}

fn image_tool(args: &Value) -> Result<Vec<Value>, String> {
    let (yaw, pitch, width, height) = view_and_size(args)?;
    let (rendered, report) = render(args)?;
    let png = rendered
        .png(yaw, pitch, width, height)
        .ok_or_else(|| format!("the model is empty: nothing to draw\n{}", pretty(&report)))?;
    Ok(vec![json!({ "type": "image", "data": base64(&png), "mimeType": "image/png" }), text(pretty(&report))])
}

/// The camera's angles and the picture's size the arguments ask for.
fn view_and_size(args: &Value) -> Result<(f32, f32, usize, usize), String> {
    let view = args.get("view").and_then(Value::as_str).unwrap_or("diagonal");
    let view = if view == "isometric" { "diagonal" } else { view };
    let &(_, yaw, pitch) = c2f_geom::VIEWS
        .iter()
        .find(|(name, _, _)| *name == view)
        .ok_or_else(|| format!("unknown view `{view}`"))?;
    let size = |key: &str, default: u64| match args.get(key) {
        None => Ok(default as usize),
        Some(v) => match v.as_u64() {
            Some(n @ 1..=MAX_IMAGE_SIZE) => Ok(n as usize),
            _ => Err(format!("`{key}` must be a whole number of pixels from 1 to {MAX_IMAGE_SIZE}")),
        },
    };
    Ok((yaw, pitch, size("width", IMAGE_SIZE.0)?, size("height", IMAGE_SIZE.1)?))
}

fn export_tool(args: &Value) -> Result<Vec<Value>, String> {
    let output = args.get("output").and_then(Value::as_str).ok_or("`output` names the file to write")?;
    let format = match args.get("format").and_then(Value::as_str) {
        Some(format) => format,
        None => Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or(""),
    };
    let export = Export::named(format).ok_or_else(|| format!("unknown format `{format}`: give `format` (stl, 3mf, svg or dxf)"))?;
    let (rendered, mut report) = render(args)?;
    let bytes = export.write(&rendered).map_err(|why| format!("{why}\n{}", pretty(&report)))?;
    std::fs::write(output, &bytes).map_err(|err| format!("cannot write {output}: {err}"))?;
    report["written"] = json!({ "path": output, "format": export.name(), "bytes": bytes.len() });
    Ok(vec![text(pretty(&report))])
}

/// Loads the program the arguments name, with their parameter values set.
/// The error is the report the agent reads.
fn load(args: &Value) -> Result<(c2f_eval::Program, String), String> {
    let string = |key: &str| args.get(key).and_then(Value::as_str);
    let (src, path) = match (string("code"), string("path")) {
        (Some(code), None) => {
            let dir = match string("working_dir") {
                Some(dir) => PathBuf::from(dir),
                None => std::env::current_dir().unwrap_or_default(),
            };
            (code.to_string(), dir.join("untitled.c2f"))
        }
        (None, Some(path)) => {
            let src = c2f_eval::read_source(Path::new(path)).map_err(|err| format!("cannot read {path}: {err}"))?;
            (src, PathBuf::from(path))
        }
        _ => return Err("give either `code` (the program's text) or `path` (a file)".into()),
    };
    let mut program = c2f_eval::Program::load(&src, &path, &c2f_eval::default_library_paths()).map_err(|err| {
        let file_src = &err.src;
        let offset = (err.diagnostic.span.start as usize).min(file_src.len());
        let (line, column) = c2f_lang::span::LineIndex::new(file_src).line_col(file_src, offset);
        pretty(&json!({
            "ok": false,
            "error": err.diagnostic.message,
            "file": err.path.display().to_string(),
            "line": line + 1,
            "column": column + 1,
            "diagnostic": err.render(),
        }))
    })?;
    if let Some(values) = args.get("parameters") {
        set_parameters(&mut program, &src, values)?;
    }
    Ok((program, src))
}

/// Sets customizer parameters from JSON values, read as a parameter set
/// reads its strings: by the type of the value in the file.
fn set_parameters(program: &mut c2f_eval::Program, src: &str, values: &Value) -> Result<(), String> {
    let values = values.as_object().ok_or("`parameters` must be an object of names and values")?;
    let parameters = program.parameters(src);
    for (name, value) in values {
        let parameter = parameters
            .iter()
            .find(|p| p.name == *name)
            .ok_or_else(|| format!("no customizer parameter `{name}`; the `parameters` tool lists them"))?;
        let raw = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        match parse_typed(&parameter.default, &raw) {
            Ok(Some(literal)) => {
                program.set_parameter(name, &literal);
            }
            Ok(None) => {
                return Err(format!("`{name}`: {value} does not have the type of its default, {}", parameter.default.to_source()));
            }
            Err(why) => return Err(format!("`{name}`: {why}")),
        }
    }
    Ok(())
}

/// Loads, evaluates and renders the model: its rendering and the report
/// the tools return. An error that stops the program is the report, as an
/// error.
fn render(args: &Value) -> Result<(c2f_geom::Rendered, Value), String> {
    let (program, _) = load(args)?;
    let output = c2f_eval::evaluate(&program);
    let mut report = Map::new();
    if let Some(error) = &output.error {
        report.insert("ok".into(), json!(false));
        report.insert("error".into(), json!(error));
        report.insert("echo".into(), json!(output.echo));
        report.insert("warnings".into(), json!(output.warnings));
        return Err(pretty(&Value::Object(report)));
    }
    let rendered = crate::render(&program, &output);
    let warnings: Vec<&String> = output.warnings.iter().chain(&rendered.warnings).collect();
    let s = rendered.stats;
    report.insert("ok".into(), json!(true));
    report.insert("echo".into(), json!(output.echo));
    report.insert("warnings".into(), json!(warnings));
    match rendered.bounds() {
        None => {
            report.insert("model".into(), json!("empty"));
        }
        Some((lo, hi)) => {
            report.insert("model".into(), json!(if rendered.is_2d { "2D" } else { "3D" }));
            if !rendered.is_2d {
                report.insert("triangles".into(), json!(s.triangles));
            }
            report.insert("vertices".into(), json!(s.vertices));
            report.insert((if rendered.is_2d { "area" } else { "volume" }).into(), json!(s.volume));
            let size: Vec<f64> = (0..3).map(|i| hi[i] - lo[i]).collect();
            report.insert("bounding_box".into(), json!({ "min": lo, "max": hi, "size": size }));
        }
    }
    if !rendered.pauses.is_empty() {
        let pauses: Vec<Value> = rendered.pauses.iter().map(|p| json!({ "z": p.z, "message": p.message })).collect();
        report.insert("pauses".into(), json!(pauses));
    }
    Ok((rendered, Value::Object(report)))
}

fn literal(value: &Literal) -> Value {
    match value {
        Literal::Undef => Value::Null,
        Literal::Bool(b) => json!(b),
        Literal::Number(n) => json!(n),
        Literal::Str(s) => json!(s),
        Literal::Range { begin, step, end } => json!({ "begin": begin, "step": step.unwrap_or(1.0), "end": end }),
        Literal::Vector(items) => Value::Array(items.iter().map(literal).collect()),
    }
}

fn widget(widget: &Widget) -> Value {
    match widget {
        Widget::Checkbox => json!({ "type": "checkbox" }),
        Widget::Slider { min, max, step } => json!({ "type": "slider", "min": min, "max": max, "step": step }),
        Widget::Spinbox { step } => json!({ "type": "spinbox", "step": step }),
        Widget::Dropdown { options } => {
            let options: Vec<Value> = options.iter().map(|(v, label)| json!({ "value": literal(v), "label": label })).collect();
            json!({ "type": "dropdown", "options": options })
        }
        Widget::Vector { step } => json!({ "type": "vector", "step": step }),
        Widget::Text { max_length } => json!({ "type": "text", "max_length": max_length }),
    }
}

/// Standard base64, with padding.
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        for i in 0..4 {
            out.push(if i <= chunk.len() { DIGITS[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(&[0xff, 0xfe, 0x00, 0x01]), "//4AAQ==");
    }

    #[test]
    fn protocol_version_negotiation() {
        let version = |asked: Value| initialize(&json!({ "protocolVersion": asked }))["protocolVersion"].clone();
        assert_eq!(version(json!("2025-03-26")), "2025-03-26");
        assert_eq!(version(json!("1999-01-01")), PROTOCOL_VERSIONS[0]);
        assert_eq!(version(Value::Null), PROTOCOL_VERSIONS[0]);
    }
}
