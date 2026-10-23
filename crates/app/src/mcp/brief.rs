//! The MCP tools of a design's brief (see `c2f_circuit::brief`): the plan
//! ends by writing it with `save_brief`, and the next steps, or the next
//! session, start by reading it with `read_brief`.

use std::path::PathBuf;

use c2f_circuit::brief::{Brief, Requirement, Rule, read_brief};
use serde_json::{Value, json};

use super::{Tool, pretty, schema_of, text};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "save_brief",
            "title": "Save a design's brief",
            "description": "Writes the brief the plan ends on, in Markdown, beside the design it is for (`path`, a .md file): \
                            a `# Title`; the idea and its use; sizes and constraints (printer bed, material, tolerances); \
                            the pieces and how they go together; the components with their links; the mechanical review \
                            (each risk: assumption, calculation, decision, what is left to verify); and a `## Requirements` \
                            section of lines a tool measures: `mass < 400 g`, `tipping > 15°`, `torque margin >= 2`, \
                            `radial load margin >= 2`, `size <= 220 x 220 x 250 mm`, `part \"Name\"`, `component \
                            \"electronics/file.c2f\"`; other lines are kept for the reader. What the tools compute from \
                            goes in fenced blocks: ```json mass``` (mass_properties' arguments: material, infill, extra \
                            masses) and ```json loads``` (loads' arguments: shaft, loads, supports, motor). Refuses a \
                            brief without a title or without a requirement a tool can measure, or with a broken block; \
                            refuses to replace a brief unless `overwrite` is true. Returns the requirements as read.",
            "inputSchema": schema_of(json!({
                "path": { "type": "string", "description": "The brief's file, ending in .md, beside the design (as `winder/brief.md`)." },
                "text": { "type": "string", "description": "The brief, in Markdown." },
                "overwrite": { "type": "boolean", "default": false, "description": "Replace the brief there is." },
            }), &["path", "text"]),
        }),
        json!({
            "name": "read_brief",
            "title": "Read a design's brief",
            "description": "Reads a brief written by `save_brief`: its text, its requirements (each with its line, what it \
                            asks and whether a tool measures it) and its `mass` and `loads` blocks. Read it before \
                            designing or adding electronics: its decisions and requirements come first.",
            "inputSchema": schema_of(json!({
                "path": { "type": "string", "description": "The brief's file." },
            }), &["path"]),
        }),
    ]
}

/// The tool named `name`, if it is one of these.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "save_brief" => save_brief,
        "read_brief" => read_brief_tool,
        _ => return None,
    })
}

fn save_brief(args: &Value) -> Result<Vec<Value>, String> {
    let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("`path` names the brief's .md file")?);
    if path.extension().is_none_or(|e| e != "md") {
        return Err(format!("a brief is Markdown: `path` ends in .md, not {}", path.display()));
    }
    let brief_text = args.get("text").and_then(Value::as_str).ok_or("`text` is the brief, in Markdown")?;
    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
    let brief = read_brief(brief_text);
    let mut why = Vec::new();
    if brief.title.is_none() {
        why.push("it has no `# Title`".to_string());
    }
    if !brief.requirements.iter().any(Requirement::checkable) {
        why.push(
            "its `## Requirements` have no line a tool measures: give at least one, as `mass < 400 g`, \
             `tipping > 15°`, `torque margin >= 2`, `size <= 220 x 220 x 250 mm` or `part \"Name\"`"
                .to_string(),
        );
    }
    why.extend(brief.errors.iter().map(|(line, message)| format!("line {line}: {message}")));
    if !why.is_empty() {
        return Err(pretty(&json!({ "saved": false, "why": why, "requirements": requirements(&brief) })));
    }
    if path.exists() && !overwrite {
        return Err(format!("{} exists already: pass `overwrite: true` to replace it", path.display()));
    }
    if let Some(folder) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).map_err(|err| format!("cannot make {}: {err}", folder.display()))?;
    }
    std::fs::write(&path, brief_text).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    Ok(vec![text(pretty(&json!({ "saved": true, "path": path.display().to_string(), "requirements": requirements(&brief) })))])
}

fn read_brief_tool(args: &Value) -> Result<Vec<Value>, String> {
    let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("`path` names the brief's file")?);
    let brief_text = std::fs::read_to_string(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let brief = read_brief(&brief_text);
    let errors: Vec<Value> = brief.errors.iter().map(|(line, message)| json!({ "line": line, "message": message })).collect();
    let report = json!({
        "path": path.display().to_string(),
        "title": brief.title,
        "requirements": requirements(&brief),
        "mass": brief.mass,
        "loads": brief.loads,
        "errors": errors,
    });
    Ok(vec![text(brief_text), text(pretty(&report))])
}

/// The requirements as JSON: line, text, what they ask, and whether a tool
/// measures them.
pub fn requirements(brief: &Brief) -> Vec<Value> {
    brief
        .requirements
        .iter()
        .map(|r| {
            let (kind, op, value) = match &r.rule {
                Rule::Mass(op, g) => ("mass", Some(op.symbol()), json!(g)),
                Rule::Tipping(op, deg) => ("tipping", Some(op.symbol()), json!(deg)),
                Rule::TorqueMargin(op, x) => ("torque margin", Some(op.symbol()), json!(x)),
                Rule::RadialLoadMargin(op, x) => ("radial load margin", Some(op.symbol()), json!(x)),
                Rule::Size(size) => ("size", Some("<="), json!(size)),
                Rule::Part(name) => ("part", None, json!(name)),
                Rule::Component(file) => ("component", None, json!(file)),
                Rule::Other => ("other", None, Value::Null),
            };
            json!({ "line": r.line, "text": r.text, "kind": kind, "op": op, "value": value, "checkable": r.checkable() })
        })
        .collect()
}
