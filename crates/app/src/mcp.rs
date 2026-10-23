//! `c2f mcp`: a Model Context Protocol server, so that an AI agent can
//! write a model, render it, look at it and export it. JSON-RPC 2.0, one
//! message per line on stdin and stdout; logs go to stderr.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use c2f_eval::customizer::{HIDDEN_GROUP, Literal, Widget, parse_typed};
use serde_json::{Map, Value, json};

use crate::Export;

mod brief;
mod circuit;
mod mass;
mod loads;
mod part;
mod prompts;

/// The protocol versions this server speaks, the latest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "\
Code2Form models parts as programs in the .scad language. Give a tool the \
program's text (`code`) or a file (`path`). `render` reports errors with \
their line, echo output, warnings, the triangle count, the volume and the \
bounding box; `image` returns a picture of the model from a view, or with \
view `sheet` four views at one scale with its size, to show for approval; \
`parameters` lists the customizer parameters, which every tool can set \
with `parameters`; `export` writes STL or 3MF (or SVG and DXF for 2D), \
glTF (.glb) for game engines such as Godot and Unity, OBJ, STEP for CAD \
programs, URDF for robots (part(\"name\", joint = \"revolute\", axis = \
[0, 1, 0], limits = [-90, 90]) joins a part to its parent) or the parts \
as JSON. `import()` reads STL, OBJ, 3MF, OFF, AMF, DXF and SVG, and STEP \
when FreeCAD is installed. \
`pause(z, message);` pauses a 3D print once it is done up to height `z`, \
to drop magnets or nuts into closed cavities: the 3MF carries the pauses \
for PrusaSlicer. For electronics, `library_parts` lists the parts (a `query` \
such as `micro I2S` ranks them) and `part_pins` their pins; to add a \
part from its supplier's product page and datasheet, write it with its \
annotations and sheet, `check_part` it, then `save_part` it into the \
user's library; a circuit (`.c2fc`, TOML) places parts and wires \
their pins: `check_circuit` checks it (voltages, grounds, reserved pins), \
`circuit_image` pictures it with its wires, `circuit_wires` and \
`circuit_bom` list its wires in order and its parts, `fit` says how its \
parts fit in their enclosure. `mass_properties` weighs a design or a \
circuit as printed, with its center of gravity and the angle it tips at. \
`loads` gives what turns a shaft: \
the torque a motor must give a shaft, what its supports take and the \
margins, to decide on a bearing, a reduction or another motor. To go from an \
idea to a printed object with its electronics, follow the prompts \
`plan_object` (questions, a mechanical review and a brief the user \
approves, written with `save_brief` and read with `read_brief`), \
`design_object` (the model, held to the brief and approved by the user) then \
`add_electronics` (`electrify` makes the approved design ready for its \
circuit). To turn a mesh into \
a design, `section` cuts it with a plane: its outlines give the sizes to \
write it again with.";

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
        "prompts/list" => Ok(prompts::list()),
        "prompts/get" => prompts::get(&params),
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
        "capabilities": { "tools": {}, "prompts": {} },
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
                            best seen from `top`. `sheet` is a sheet of views to show someone for approval: diagonal, \
                            front, right and top, titled, at one scale, with the size in mm (width × depth × height) \
                            and a scale bar; `section` adds the model cut through its middle (y = the box's center, \
                            seen from the front, the cut hatched) to show inside a hollow part. A sheet has its own \
                            size: `width` and `height` are ignored.",
            "inputSchema": schema(json!({
                "view": { "type": "string", "enum": sheet_views(), "default": "diagonal", "description": "Where the camera looks from, or `sheet`." },
                "section": { "type": "boolean", "default": false, "description": "With `view` = `sheet`: add a view cut through the middle." },
                "width": { "type": "integer", "minimum": 1, "maximum": MAX_IMAGE_SIZE, "default": IMAGE_SIZE.0, "description": "In pixels." },
                "height": { "type": "integer", "minimum": 1, "maximum": MAX_IMAGE_SIZE, "default": IMAGE_SIZE.1, "description": "In pixels." },
            }), &[]),
        },
        {
            "name": "electrify",
            "title": "Make a design ready for its electronics",
            "description": "Prepares an approved design (`path`, a .c2f file) for its electronics without \
                            rewriting it: creates its circuit file beside it (`circuit`, by default the design's \
                            name with .c2fc), whose enclosure is the design; names the circuit in the design \
                            (// @circuit); moves the design's drawing, unchanged, into a module design(); and adds \
                            empty modules electronics_cuts() (subtracted: cutout_for, holes_for, channel_for) and \
                            electronics_mounts() (added: standoffs_for). A design made of pieces (part()) keeps \
                            them: only the piece that holds the electronics (`piece`, needed when there are \
                            several) takes the cuts and mounts, where the design draws it. Then place the parts and wires in the \
                            circuit, and fill the two modules. Reports the volume before and after, which match.",
            "inputSchema": schema_of(json!({
                "path": { "type": "string", "description": "The design's .c2f file; it is rewritten." },
                "circuit": { "type": "string", "description": "The circuit file's name, beside the design (default: <design>.c2fc)." },
                "piece": { "type": "string", "description": "In a design made of pieces, the part() that holds the electronics." },
            }), &["path"]),
        },
        {
            "name": "section",
            "title": "Cut a model",
            "description": "Cuts the solid with a plane across `axis` at `at` (mm) and returns its outline at its real \
                            size: each contour's points in the plane's two axes (x = at gives y, z; y = at gives x, z; \
                            z = at gives x, y), outer ones counter-clockwise, holes clockwise, with its area and bounds. \
                            Use it to measure a mesh (import(\"part.3mf\"), or one object of it with object = \"name\") \
                            before writing it again as a parametric design: a cut through the axis of a turned part is \
                            its profile, for rotate_extrude(); cuts at several heights show where a prism changes.",
            "inputSchema": schema(json!({
                "axis": { "type": "string", "enum": ["x", "y", "z"], "default": "z", "description": "The axis the plane is across." },
                "at": { "type": "number", "default": 0, "description": "Where along the axis, in mm." },
                "tolerance": {
                    "type": "number",
                    "minimum": 0,
                    "default": 0.01,
                    "description": "Points this close (mm) to the line through their neighbours are left out; 0 keeps them all.",
                },
            }), &[]),
        },
        {
            "name": "export",
            "title": "Export a model",
            "description": "Renders a model and writes it to a file: a solid as STL or 3MF (in millimeters), as \
                            glTF 2.0 binary (.glb, in meters with Y up, a material per color, for Godot and Unity), \
                            as OBJ (an object per part(), its colors in a .mtl beside it), as STEP (faceted solids \
                            for CAD programs), as URDF (a robot: a link per part(), joined by part(joint = ...), \
                            its meshes in a folder beside it) or its parts as JSON; a 2D model as SVG or DXF, and \
                            a 3D model's outline seen from `view` as DXF. Returns the render report and the size written.",
            "inputSchema": schema(json!({
                "output": { "type": "string", "description": "The file to write, absolute or relative to the server's current directory; it is replaced." },
                "format": {
                    "type": "string",
                    "enum": ["stl", "3mf", "glb", "obj", "step", "urdf", "json", "svg", "dxf"],
                    "description": "Default: the output's extension.",
                },
                "view": {
                    "type": "string",
                    "enum": views,
                    "default": "top",
                    "description": "dxf of a 3D model: where its outline is seen from, at its real size.",
                },
                "density": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "default": 1240,
                    "description": "urdf: kilograms per cubic meter, for the links' masses (PLA's by default).",
                },
                "scale": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "default": 0.001,
                    "description": "glb: meters per model unit. 0.001 keeps a model in millimeters at its size; 1 makes each unit a meter.",
                },
                "duration": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "default": 2,
                    "description": "glb of a model that reads $t: the seconds its part() nodes take to move from $t = 0 to 1.",
                },
                "fps": { "type": "number", "exclusiveMinimum": 0, "default": 30, "description": "glb: animation frames sampled a second." },
                "animation": { "type": "string", "description": "glb: the animation's name (default: the output file's name)." },
                "loop": {
                    "type": "boolean",
                    "default": true,
                    "description": "glb: the animation loops; its name then ends in -loop, which Godot reads. false plays it once.",
                },
                "animate": { "type": "boolean", "default": true, "description": "glb: false writes the model at $t = 0, without animation." },
            }), &["output"]),
        },
    ]);
    if let Some(tools) = tools.as_array_mut() {
        tools.extend(circuit::tools());
        // Writing a part from its supplier's pages.
        tools.extend(part::tools());
        // The plan's brief, and what measures a design against it.
        tools.extend(brief::tools());
        tools.extend(mass::tools());
        tools.extend(loads::tools());
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
                            (`{\"width\": 30, \"label\": \"A\", \"size\": [10, 20]}`); the `parameters` tool lists them. \
                            `$t` sets the animation's time, from 0 to 1 (`{\"$t\": 0.25}`), which is 0 otherwise.",
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
        "section" => section_tool,
        "electrify" => electrify_tool,
        _ => circuit::tool(name).or_else(|| part::tool(name)).or_else(|| brief::tool(name)).or_else(|| mass::tool(name)).or_else(|| loads::tool(name)).ok_or((-32602, format!("unknown tool: {name}")))?,
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
    let (rendered, report) = render(args)?;
    let png = picture(&rendered, args)?.ok_or_else(|| format!("the model is empty: nothing to draw\n{}", pretty(&report)))?;
    Ok(vec![json!({ "type": "image", "data": base64(&png), "mimeType": "image/png" }), text(pretty(&report))])
}

/// The views `image` and `circuit_image` take: the presets, and the sheet.
fn sheet_views() -> Vec<&'static str> {
    c2f_geom::VIEWS.iter().map(|(name, _, _)| *name).chain(["isometric", "sheet"]).collect()
}

/// The picture the arguments ask for: the sheet of views for `view` =
/// `sheet` (with a cut for `section`), else one view; `None` when there is
/// nothing to draw.
fn picture(rendered: &c2f_geom::Rendered, args: &Value) -> Result<Option<Vec<u8>>, String> {
    let section = match args.get("section") {
        None => false,
        Some(v) => v.as_bool().ok_or("`section` must be true or false")?,
    };
    if args.get("view").and_then(Value::as_str) == Some("sheet") {
        return Ok(rendered.sheet(section));
    }
    if section {
        return Err("`section` goes with `view` = `sheet`".into());
    }
    let (yaw, pitch, width, height) = view_and_size(args)?;
    Ok(rendered.png(yaw, pitch, width, height))
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

fn electrify_tool(args: &Value) -> Result<Vec<Value>, String> {
    let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("`path` names the design's .c2f file")?);
    let src = std::fs::read_to_string(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let design_file = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("`path` names no file")?;
    let circuit_file = match args.get("circuit").and_then(Value::as_str) {
        Some(name) if name.ends_with(".c2fc") && !name.contains(['/', '\\']) => name.to_string(),
        Some(name) => return Err(format!("`circuit` must be a file name ending in .c2fc, beside the design: {name}")),
        None => format!("{}.c2fc", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
    };
    let circuit_path = path.with_file_name(&circuit_file);
    if circuit_path.exists() {
        return Err(format!("{} exists already: name another with `circuit`", circuit_path.display()));
    }
    let before = render(&json!({ "path": path }))?.1;
    let piece = args.get("piece").and_then(Value::as_str);
    let done = c2f_circuit::electrify::electrify(&src, &design_file, &circuit_file, piece)?;
    std::fs::write(&circuit_path, &done.circuit).map_err(|err| format!("cannot write {}: {err}", circuit_path.display()))?;
    // Anything that fails from here leaves the design as it was, and no circuit.
    let undo = |why: String| {
        let _ = std::fs::write(&path, &src);
        let _ = std::fs::remove_file(&circuit_path);
        why
    };
    std::fs::write(&path, &done.design).map_err(|err| undo(format!("cannot write {}: {err}", path.display())))?;
    let after = render(&json!({ "path": path })).map_err(undo)?.1;
    let report = json!({
        "ok": true,
        "design": path.display().to_string(),
        "circuit": circuit_path.display().to_string(),
        "volume_before": before.get("volume"),
        "volume_after": after.get("volume"),
        "warnings": after.get("warnings"),
        "next": "Place the parts and wires in the circuit file, check them with check_circuit and fit, \
                 then fill electronics_cuts() and electronics_mounts() in the design.",
    });
    Ok(vec![text(pretty(&report))])
}

fn section_tool(args: &Value) -> Result<Vec<Value>, String> {
    let axis = match args.get("axis").and_then(Value::as_str) {
        None => c2f_geom::Axis::Z,
        Some(name) => c2f_geom::Axis::named(name).ok_or_else(|| format!("unknown axis `{name}`: give x, y or z"))?,
    };
    let number = |key: &str, default: f64| match args.get(key) {
        None => Ok(default),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or(format!("`{key}` must be a number")),
    };
    let (at, tolerance) = (number("at", 0.0)?, number("tolerance", 0.01)?.max(0.0));
    let (rendered, mut report) = render(args)?;
    let cut = rendered.section(axis, at).ok_or_else(|| format!("the model is empty or 2D: nothing to cut\n{}", pretty(&report)))?;
    report["section"] = cut.json(axis, at, tolerance);
    Ok(vec![text(pretty(&report))])
}

fn export_tool(args: &Value) -> Result<Vec<Value>, String> {
    let output = args.get("output").and_then(Value::as_str).ok_or("`output` names the file to write")?;
    let format = match args.get("format").and_then(Value::as_str) {
        Some(format) => format,
        None => Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or(""),
    };
    let export = Export::named(format)
        .ok_or_else(|| format!("unknown format `{format}`: give `format` (stl, 3mf, glb, obj, step, urdf, json, svg or dxf)"))?;
    match export {
        Export::Glb => return export_gltf(args, output),
        Export::Obj | Export::Urdf | Export::Json => return export_parts(args, output, export),
        _ => {}
    }
    let view = match args.get("view").and_then(Value::as_str) {
        Some(view) => {
            let &(_, yaw, pitch) =
                c2f_geom::VIEWS.iter().find(|(name, _, _)| *name == view).ok_or_else(|| format!("unknown view `{view}`"))?;
            Some((f64::from(yaw), f64::from(pitch)))
        }
        None => None,
    };
    let (rendered, mut report) = render(args)?;
    let name = Path::new(output).file_stem().map_or_else(|| "model".into(), |s| s.to_string_lossy().into_owned());
    let bytes = export.write(&rendered, &name, view).map_err(|why| format!("{why}\n{}", pretty(&report)))?;
    std::fs::write(output, &bytes).map_err(|err| format!("cannot write {output}: {err}"))?;
    report["written"] = json!({ "path": output, "format": export.name(), "bytes": bytes.len() });
    Ok(vec![text(pretty(&report))])
}

/// Writes the model's parts to `output`: as OBJ and its `.mtl`, as a URDF
/// and its meshes, or as JSON.
fn export_parts(args: &Value, output: &str, export: Export) -> Result<Vec<Value>, String> {
    let (program, src) = load(args)?;
    let evaluated = c2f_eval::evaluate(&program);
    let mut report = json!({ "ok": evaluated.error.is_none(), "echo": evaluated.echo, "warnings": evaluated.warnings });
    if let Some(error) = &evaluated.error {
        report["error"] = json!(error);
        return Err(pretty(&report));
    }
    let base_dir = program.files[0].path.parent().unwrap_or(Path::new(""));
    let target = Path::new(output);
    let name = target.file_stem().map_or_else(|| "model".into(), |s| s.to_string_lossy().into_owned());
    let write = |path: &Path, bytes: &[u8]| std::fs::write(path, bytes).map_err(|err| format!("cannot write {}: {err}", path.display()));
    let fail = |why: String, report: &Value| format!("{why}\n{}", pretty(report));
    let (warnings, mut written) = match export {
        Export::Obj => {
            let obj = c2f_geom::obj(&evaluated.nodes, base_dir, &c2f_geom::ObjOptions { name: name.clone(), ..Default::default() })
                .map_err(|why| fail(why, &report))?;
            let mtl = target.with_file_name(&obj.mtl_file);
            write(&mtl, obj.mtl.as_bytes())?;
            write(target, obj.obj.as_bytes())?;
            report["triangles"] = json!(obj.triangles);
            (obj.warnings, json!({ "path": output, "materials": mtl.display().to_string(), "bytes": obj.obj.len() }))
        }
        Export::Urdf => {
            let density = match args.get("density") {
                None => 1240.0,
                Some(v) => v.as_f64().filter(|d| d.is_finite() && *d > 0.0).ok_or("`density` must be a positive number")?,
            };
            let options = c2f_geom::UrdfOptions { name: name.clone(), mesh_folder: format!("{name}_meshes"), density, ..Default::default() };
            let urdf = c2f_geom::urdf(&evaluated.nodes, base_dir, &options).map_err(|why| fail(why, &report))?;
            let dir = target.parent().unwrap_or(Path::new(""));
            for (file, bytes) in &urdf.meshes {
                let path = dir.join(file);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
                }
                write(&path, bytes)?;
            }
            write(target, urdf.urdf.as_bytes())?;
            report["links"] = json!(urdf.links);
            report["moving_joints"] = json!(urdf.moving);
            let meshes: Vec<&String> = urdf.meshes.iter().map(|(file, _)| file).collect();
            (urdf.warnings, json!({ "path": output, "meshes": meshes, "bytes": urdf.urdf.len() }))
        }
        _ => {
            let parameters = program.parameters(&src).into_iter()
                .filter(|p| p.group != c2f_eval::customizer::HIDDEN_GROUP)
                .map(|p| (p.name, p.default)).collect();
            let options = c2f_geom::PartsOptions { name: name.clone(), parameters, ..Default::default() };
            let (json, warnings) = c2f_geom::parts_json(&evaluated.nodes, base_dir, &options).map_err(|why| fail(why, &report))?;
            write(target, json.as_bytes())?;
            (warnings, json!({ "path": output, "bytes": json.len() }))
        }
    };
    if let Some(list) = report["warnings"].as_array_mut() {
        list.extend(warnings.iter().map(|w| json!(w)));
    }
    written["format"] = json!(export.name());
    report["written"] = written;
    Ok(vec![text(pretty(&report))])
}

/// Writes the model as a `.glb` to `output`.
fn export_gltf(args: &Value, output: &str) -> Result<Vec<Value>, String> {
    let mut options = c2f_geom::GltfOptions::default();
    if let Some(scale) = args.get("scale") {
        options.scale = scale.as_f64().filter(|s| s.is_finite() && *s > 0.0).ok_or("`scale` must be a positive number")?;
    }
    if let Some(stem) = Path::new(output).file_stem() {
        options.name = stem.to_string_lossy().into_owned();
    }
    let positive = |key: &str| match args.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|v| v.is_finite() && *v > 0.0).map(Some).ok_or(format!("`{key}` must be a positive number")),
    };
    let mut animation = c2f_geom::GltfAnimation { name: options.name.clone(), ..Default::default() };
    if let Some(duration) = positive("duration")? {
        animation.duration = duration;
    }
    if let Some(fps) = positive("fps")? {
        animation.fps = fps;
    }
    if let Some(name) = args.get("animation").and_then(Value::as_str) {
        animation.name = name.to_string();
    }
    animation.looped = args.get("loop").and_then(Value::as_bool).unwrap_or(true);
    let animate = args.get("animate").and_then(Value::as_bool).unwrap_or(true);
    options.animation = animate.then_some(animation);
    let (mut program, evaluated) = evaluate(args)?;
    let mut report = json!({ "ok": true, "echo": evaluated.echo, "warnings": evaluated.warnings });
    let glb = c2f_geom::program_glb(&mut program, &options).map_err(|why| format!("{why}\n{}", pretty(&report)))?;
    if let Some(warnings) = report["warnings"].as_array_mut() {
        warnings.extend(glb.warnings.iter().map(|w| json!(w)));
    }
    report["model"] = json!("3D");
    report["triangles"] = json!(glb.triangles);
    report["vertices"] = json!(glb.vertices);
    if glb.frames > 0 {
        report["animation"] = json!({ "name": options.animation.map(|a| a.written_name()), "frames": glb.frames });
    }
    std::fs::write(output, &glb.bytes).map_err(|err| format!("cannot write {output}: {err}"))?;
    report["written"] = json!({ "path": output, "format": Export::Glb.name(), "bytes": glb.bytes.len() });
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
        if name == "$t" {
            program.t = value.as_f64().filter(|t| t.is_finite()).ok_or("`$t` must be a number, the time from 0 to 1")?;
            continue;
        }
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

/// Loads and evaluates the model. An error that stops the program is the
/// report, as an error.
fn evaluate(args: &Value) -> Result<(c2f_eval::Program, c2f_eval::Output), String> {
    let (program, _) = load(args)?;
    let output = c2f_eval::evaluate(&program);
    if let Some(error) = &output.error {
        let report = json!({ "ok": false, "error": error, "echo": output.echo, "warnings": output.warnings });
        return Err(pretty(&report));
    }
    Ok((program, output))
}

/// Loads, evaluates and renders the model: its rendering and the report
/// the tools return. An error that stops the program is the report, as an
/// error.
fn render(args: &Value) -> Result<(c2f_geom::Rendered, Value), String> {
    let (program, output) = evaluate(args)?;
    let mut report = Map::new();
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
