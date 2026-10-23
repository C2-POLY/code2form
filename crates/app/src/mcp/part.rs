//! The MCP tools that help an agent write a part: `check_part` checks its
//! annotations against its shape, and `save_part` puts it in the user's
//! parts folder, where the catalogue and circuits find it. The agent reads
//! the supplier's product page and datasheet itself; these tools make sure
//! what it wrote from them holds together.

use std::path::{Path, PathBuf};

use c2f_circuit::check::Severity;
use c2f_lang::annotations;
use serde_json::{Value, json};

use super::{Tool, pretty, schema_of, text};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "check_part",
            "title": "Check a part",
            "description": "Reads a part's annotations and renders it with its default parameters, then checks one against \
                            the other. Errors (`ok` false), with their line and column: a badly written annotation, a part \
                            that does not render, a pin or a mount hole outside the part's bounding box (0.5 mm slack), two \
                            pins at the same place, a port whose direction points into the part instead of out of it, a \
                            power pin without its voltage. Warnings: no `// @description`, no `// @source`. Also gives the \
                            bounding box and the number of pins, mount holes, ports, specs and sources. Run it after each \
                            change to a part, before `save_part`.",
            "inputSchema": schema_of(json!({
                "code": { "type": "string", "description": "The part's text (.c2f). Give this or `path`." },
                "path": { "type": "string", "description": "The part's file, as a circuit names it (`electronics/inmp441.c2f`) or a path." },
                "working_dir": { "type": "string", "description": "With `code`: the folder its `use`s resolve from; with `path`, a folder it is read from before the library." },
            }), &[]),
        }),
        json!({
            "name": "save_part",
            "title": "Save a part in the user's library",
            "description": "Writes a part into the user's parts folder (`~/Documents/Code2Form/libraries/electronics`, \
                            where the app's catalogue lists it and a circuit names it `electronics/NAME.c2f`), then \
                            checks it as `check_part` does and returns its path and that report. Write the part from the \
                            supplier's product page and datasheet: a module NAME() that draws it, `// @pin`, \
                            `// @mount_hole` and `// @port` annotations in its millimeters, and its sheet: \
                            `// @description` (what it is and does, a sentence or two), `// @spec key: value` lines (supply, \
                            current, interface, size...: only figures the sources give) and a `// @source URL label` for \
                            each page used. Refuses a file that does not parse, whose annotations have errors, or that \
                            lacks a `@description` or a `@source`; refuses to replace a part unless `overwrite` is true.",
            "inputSchema": schema_of(json!({
                "name": { "type": "string", "description": "The part's file name without `.c2f`, and its module's name: letters, digits and `_`, not starting with a digit." },
                "code": { "type": "string", "description": "The part's text." },
                "overwrite": { "type": "boolean", "default": false, "description": "Replace the part of that name if there is one." },
            }), &["name", "code"]),
        }),
    ]
}

/// The tool named `name`, if it is one of these.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "check_part" => check_part,
        "save_part" => save_part,
        _ => return None,
    })
}

fn check_part(args: &Value) -> Result<Vec<Value>, String> {
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
    let report = report(&src, file.as_deref(), string("working_dir"));
    if report["ok"] == json!(true) { Ok(vec![text(pretty(&report))]) } else { Err(pretty(&report)) }
}

/// The checks of a part: its annotations read, then rendered from `file`,
/// or as `code` whose `use`s resolve from `working_dir`.
fn report(src: &str, file: Option<&Path>, working_dir: Option<&str>) -> Value {
    let lines = c2f_lang::span::LineIndex::new(src);
    let at = |message: &str, offset: usize| {
        let (line, column) = lines.line_col(src, offset.min(src.len()));
        json!({ "message": message, "line": line + 1, "column": column + 1 })
    };
    let (annotations, annotation_errors) = annotations::read(src);
    let mut errors: Vec<Value> = annotation_errors.iter().map(|e| at(&e.message, e.span.start as usize)).collect();
    let mut warnings: Vec<Value> = Vec::new();
    let mut render_args = match file {
        Some(file) => json!({ "path": file }),
        None => json!({ "code": src }),
    };
    if let (None, Some(dir)) = (file, working_dir) {
        render_args["working_dir"] = json!(dir);
    }
    let mut report = json!({});
    let (bounds, rendered) = match super::render(&render_args) {
        Ok((rendered, rendered_report)) => {
            warnings.extend(rendered_report["warnings"].as_array().into_iter().flatten().map(|w| json!({ "message": w })));
            if let Some(bounding_box) = rendered_report.get("bounding_box") {
                report["bounding_box"] = bounding_box.clone();
            }
            (rendered.bounds(), true)
        }
        Err(failure) => {
            // The render's own report says where it stopped.
            let failure: Value = serde_json::from_str(&failure).unwrap_or_else(|_| json!({ "error": failure }));
            let message = format!("the part does not render: {}", failure["error"].as_str().unwrap_or("unknown error"));
            let mut error = json!({ "message": message });
            for key in ["line", "column"] {
                if let Some(value) = failure.get(key) {
                    error[key] = value.clone();
                }
            }
            errors.push(error);
            (None, false)
        }
    };
    for finding in c2f_circuit::part::check(&annotations, bounds) {
        // A part that failed to render is reported as such, not as empty.
        if finding.rule == "empty" && !rendered {
            continue;
        }
        let value = match finding.span.end {
            0 => json!({ "message": finding.message }),
            _ => at(&finding.message, finding.span.start as usize),
        };
        match finding.severity {
            Severity::Error => errors.push(value),
            Severity::Warning => warnings.push(value),
        }
    }
    report["ok"] = json!(errors.is_empty());
    report["pins"] = json!(annotations.pins.len());
    report["mount_holes"] = json!(annotations.mount_holes.len());
    report["ports"] = json!(annotations.ports.len());
    report["specs"] = json!(annotations.specs.len());
    report["sources"] = json!(annotations.sources.len());
    report["errors"] = json!(errors);
    report["warnings"] = json!(warnings);
    if let Some(file) = file {
        report["file"] = json!(file.display().to_string());
    }
    report
}

fn save_part(args: &Value) -> Result<Vec<Value>, String> {
    let name = args.get("name").and_then(Value::as_str).ok_or("`name` is the part's file name, without `.c2f`")?;
    let name = name.strip_suffix(".c2f").unwrap_or(name);
    let code = args.get("code").and_then(Value::as_str).ok_or("`code` is the part's text")?;
    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
    if !c2f_circuit::part::valid_name(name) {
        return Err(format!(
            "`{name}` cannot name a part: use letters, digits and `_`, not starting with a digit, as `bme280_breakout`"
        ));
    }
    let lines = c2f_lang::span::LineIndex::new(code);
    let at = |message: &str, offset: usize| {
        let (line, column) = lines.line_col(code, offset.min(code.len()));
        json!({ "message": message, "line": line + 1, "column": column + 1 })
    };
    if let Err(error) = c2f_lang::parse(code) {
        let errors = [at(&error.message, error.span.start as usize)];
        return Err(pretty(&json!({ "saved": false, "why": "the part does not parse", "errors": errors })));
    }
    let (annotations, errors) = annotations::read(code);
    if !errors.is_empty() {
        let errors: Vec<Value> = errors.iter().map(|e| at(&e.message, e.span.start as usize)).collect();
        return Err(pretty(&json!({ "saved": false, "why": "its annotations have errors", "errors": errors })));
    }
    let mut missing = Vec::new();
    if annotations.description.is_none() {
        missing.push("a `// @description` line saying what the part is and does");
    }
    if annotations.sources.is_empty() {
        missing.push("a `// @source URL label` line for each page its figures come from");
    }
    if !missing.is_empty() {
        return Err(format!("the part needs its sheet before it is saved: {}", missing.join("; ")));
    }
    let folder = c2f_eval::user_parts_folder().ok_or("no home folder: cannot find the user's library")?;
    let path = folder.join(format!("{name}.c2f"));
    if path.exists() && !overwrite {
        return Err(format!("{} already exists: pass `overwrite: true` to replace it", path.display()));
    }
    std::fs::create_dir_all(&folder).map_err(|err| format!("cannot make {}: {err}", folder.display()))?;
    std::fs::write(&path, code).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    let check = report(code, Some(&path), None);
    let saved = json!({
        "saved": true,
        "path": path.display().to_string(),
        "part": format!("electronics/{name}.c2f"),
        "check": check,
    });
    Ok(vec![text(pretty(&saved))])
}
