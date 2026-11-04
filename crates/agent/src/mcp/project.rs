//! The MCP tools for projects: what a project holds in one call, a new
//! one, and a file put in one.

use std::path::{Path, PathBuf};

use c2f_project::{Kind, Project};
use serde_json::{Value, json};

use super::{Tool, pretty, schema_of, text};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "project_info",
            "title": "What a project holds",
            "description": "Reads an object's project: its name and folder, its brief, its designs (the first is the \
                            main one) with their size, its circuit and whether it checks, its firmware's folder and \
                            whether it is written, its scans, its exports, and the files it names that are missing. \
                            `path` is the project's .c2fproj, its folder, or any file in it. Call it first when the \
                            user works on an object: it says which files to read and where to write.",
            "inputSchema": schema_of(json!({
                "path": { "type": "string", "description": "The project's .c2fproj, its folder, or a file in it." },
            }), &["path"]),
        }),
        json!({
            "name": "project_new",
            "title": "Create a project",
            "description": "Creates an object's project in a new folder `path`: its .c2fproj, a brief.md to fill in, and \
                            its folders models/ (the designs), scans/, circuit/, firmware/ and exports/; a folder that \
                            has files already is fine, a project already is refused. Then write the \
                            brief with save_brief as its brief.md, the designs in models/, the circuit in circuit/, and \
                            name each new file in it with project_add.",
            "inputSchema": schema_of(json!({
                "path": { "type": "string", "description": "The project's folder, new or empty." },
                "name": { "type": "string", "description": "The object's name (default: the folder's)." },
            }), &["path"]),
        }),
        json!({
            "name": "project_add",
            "title": "Put a file in a project",
            "description": "Names a file of the project's folder in its .c2fproj: a design (`kind` = design, added to its \
                            designs; the first is the main one), the circuit, a scan, the brief or the firmware's \
                            folder. `kind` defaults to what the file's extension says. Call it after writing a file of \
                            the object, so that the studio and the next agent find it.",
            "inputSchema": schema_of(json!({
                "path": { "type": "string", "description": "The project's .c2fproj or folder." },
                "file": { "type": "string", "description": "The file, relative to the project's folder or absolute in it." },
                "kind": { "type": "string", "enum": ["design", "circuit", "scan", "brief", "firmware"] },
            }), &["path", "file"]),
        }),
    ]
}

/// The tool named `name`, if it is one of these.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "project_info" => info_tool,
        "project_new" => new_tool,
        "project_add" => add_tool,
        _ => return None,
    })
}

/// The project `path` names: its file, its folder, or a file in it; in
/// the agent's folder, when the guard gives it one (a file's project may
/// be found above it).
fn project(args: &Value) -> Result<Project, String> {
    let project = found(args)?;
    if let Some(scope) = crate::guard::file_scope(args) {
        let folder = scope.roots().first().cloned().unwrap_or_default();
        if !c2f_lang::file_scope::resolve(&project.dir).starts_with(&folder) {
            return Err(format!("the project {} is outside the project folder, {}", project.dir.display(), folder.display()));
        }
    }
    Ok(project)
}

fn found(args: &Value) -> Result<Project, String> {
    let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("give the project's `path`")?);
    let file_or_dir = path.extension().is_some_and(|e| e == c2f_project::EXTENSION) || (path.is_dir() && c2f_project::file_in(&path).is_some());
    if file_or_dir {
        return Project::open(&path).map_err(|problems| problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"));
    }
    Project::of(&path).ok_or_else(|| {
        format!("{} is in no project: create one with project_new, or make its folder one in the studio (File › Make a Project of This Folder)", path.display())
    })
}

fn info_tool(args: &Value) -> Result<Vec<Value>, String> {
    let project = project(args)?;
    Ok(vec![text(pretty(&info(&project, args)))])
}

/// What a project holds, for the agent; its circuit read in the scope
/// `args` carries.
fn info(project: &Project, args: &Value) -> Value {
    let size = |path: &Path| std::fs::metadata(path).ok().map(|m| m.len());
    let designs: Vec<Value> = project
        .designs
        .iter()
        .enumerate()
        .map(|(i, d)| json!({ "path": d, "main": i == 0, "bytes": size(&project.dir.join(d)) }))
        .collect();
    let circuit = project.circuit_path().map(|path| {
        let checked = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|src| {
            let scope = crate::guard::file_scope(args);
            c2f_circuit::parse_in(&src, &path, &c2f_eval::default_library_paths(), scope.as_ref())
                .map_err(|errors| format!("{} errors", errors.len()))
        });
        match checked {
            Ok(circuit) => {
                let problems = c2f_circuit::check::check(&circuit);
                let errors = problems.iter().filter(|p| p.severity == c2f_circuit::check::Severity::Error).count();
                json!({ "path": project.circuit, "parts": circuit.parts.len(), "wires": circuit.wires.len(), "errors": errors, "warnings": problems.len() - errors })
            }
            Err(why) => json!({ "path": project.circuit, "reads": false, "why": why }),
        }
    });
    let firmware = project.firmware_dir();
    let exports: Vec<String> = std::fs::read_dir(project.exports_dir())
        .map(|entries| entries.flatten().filter(|e| e.path().is_file()).filter_map(|e| project.relative(&e.path())).collect())
        .unwrap_or_default();
    json!({
        "name": project.name,
        "folder": project.dir.display().to_string(),
        "file": project.file.display().to_string(),
        "brief": project.brief,
        "designs": designs,
        "circuit": circuit,
        "firmware": {
            "folder": project.relative(&firmware).unwrap_or_else(|| firmware.display().to_string()),
            "written": project.has_firmware(),
        },
        "scans": project.scans,
        "exports": { "folder": project.relative(&project.exports_dir()), "files": exports },
        "missing": project.missing(),
    })
}

fn new_tool(args: &Value) -> Result<Vec<Value>, String> {
    let dir = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("give the project's folder, `path`")?);
    let name = match args.get("name").and_then(Value::as_str) {
        Some(name) => name.to_string(),
        None => dir.file_name().map_or_else(|| "Project".into(), |n| n.to_string_lossy().into_owned()),
    };
    let project = c2f_project::create(&dir, &name)?;
    let mut report = info(&project, args);
    report["next"] = json!("Write the brief as its brief.md, the designs in models/ and the circuit in circuit/, and name each with project_add.");
    Ok(vec![text(pretty(&report))])
}

fn add_tool(args: &Value) -> Result<Vec<Value>, String> {
    let mut project = project(args)?;
    let file = args.get("file").and_then(Value::as_str).ok_or("give the `file` to put in the project")?;
    let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { project.dir.join(file) };
    if !path.exists() {
        return Err(format!("{} is not there: write it first", path.display()));
    }
    let kind = match args.get("kind").and_then(Value::as_str) {
        Some(kind) => Kind::from_name(kind).ok_or_else(|| format!("`kind` = {kind} is not one of design, circuit, scan, brief, firmware"))?,
        None if path.is_dir() => Kind::Firmware,
        None => Kind::of(&path).ok_or_else(|| format!("say what {file} is with `kind`"))?,
    };
    project.add(kind, &path)?;
    project.save()?;
    Ok(vec![text(pretty(&info(&project, args)))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_created_filled_and_read() {
        let dir = std::env::temp_dir().join(format!("c2f-project-tools-{}", std::process::id())).join("Lamp");
        let _ = std::fs::remove_dir_all(&dir);
        let read = |result: Result<Vec<Value>, String>| -> Value { serde_json::from_str(result.unwrap()[0]["text"].as_str().unwrap()).unwrap() };
        let made = read(new_tool(&json!({ "path": dir, "name": "Lamp" })));
        assert_eq!(made["name"], "Lamp");
        assert_eq!(made["firmware"]["written"], false);
        std::fs::write(dir.join("models/shade.c2f"), "sphere(10);").unwrap();
        let added = read(add_tool(&json!({ "path": dir, "file": "models/shade.c2f" })));
        assert_eq!(added["designs"][0]["path"], "models/shade.c2f");
        assert_eq!(added["designs"][0]["main"], true);
        // From any file of it.
        let info = read(info_tool(&json!({ "path": dir.join("models/shade.c2f") })));
        assert_eq!(info["brief"], "brief.md");
        assert!(info["missing"].as_array().unwrap().is_empty());
        // Refused: a file not written, one outside, a kind it does not know.
        assert!(add_tool(&json!({ "path": dir, "file": "models/none.c2f" })).is_err());
        assert!(add_tool(&json!({ "path": dir, "file": std::env::temp_dir() })).is_err());
        assert!(add_tool(&json!({ "path": dir, "file": "brief.md", "kind": "photo" })).is_err());
        assert!(info_tool(&json!({ "path": std::env::temp_dir() })).is_err());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
