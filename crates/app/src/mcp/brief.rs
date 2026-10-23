//! The MCP tools of a design's brief (see `c2f_circuit::brief`): the plan
//! ends by writing it with `save_brief`, the next steps, or the next
//! session, start by reading it with `read_brief`, and `check_brief`
//! measures the design and its circuit against its requirements.

use std::path::{Path, PathBuf};

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
            "name": "check_brief",
            "title": "Check a design against its brief",
            "description": "Measures a design (`design`, a .c2f file) and its circuit (`circuit`, a .c2fc file; by default the \
                            one the design names with `// @circuit`) against the requirements of their brief (`brief`). \
                            Each requirement `holds`, `fails` or is `unmeasured`, with the value measured and why: the \
                            pieces the design draws (`part \"Name\"`), the components the circuit uses (`component`), \
                            each piece's size against the printer (`size <= X x Y x Z mm`, the piece turned to fit), \
                            and the mass, tipping and margins measured by `mass_properties` and `loads` from the brief's \
                            ```json mass``` and ```json loads``` blocks. `ok` is false when one fails. Run it before \
                            showing a design or a build to the user.",
            "inputSchema": schema_of(json!({
                "brief": { "type": "string", "description": "The brief's .md file." },
                "design": { "type": "string", "description": "The design's .c2f file." },
                "circuit": { "type": "string", "description": "Its circuit's .c2fc file, if not the one the design names." },
                "parameters": { "type": "object", "description": "Customizer values for the design, as the other tools take them." },
            }), &["brief"]),
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
        "check_brief" => check_brief,
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

/// How a requirement stands against the design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Holds,
    Fails,
    Unmeasured,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Holds => "holds",
            Status::Fails => "fails",
            Status::Unmeasured => "unmeasured",
        }
    }
}

/// What the design and its circuit give to measure the requirements with,
/// and why what could not be measured was not.
struct Measured {
    /// Each piece's name and size, in millimeters.
    pieces: Option<Vec<(String, [f64; 3])>>,
    /// The part files the circuit names, as it names them.
    components: Option<Vec<String>>,
    /// The total mass in grams and the smallest tipping angle in degrees,
    /// from `mass_properties`.
    mass: Result<(f64, Option<f64>), String>,
    /// The torque margin and the motor's radial load margin, from `loads`.
    margins: Result<(Option<f64>, Option<f64>), String>,
}

fn check_brief(args: &Value) -> Result<Vec<Value>, String> {
    let string = |key: &str| args.get(key).and_then(Value::as_str);
    let brief_path = PathBuf::from(string("brief").ok_or("`brief` names the brief's .md file")?);
    let brief_text = std::fs::read_to_string(&brief_path).map_err(|err| format!("cannot read {}: {err}", brief_path.display()))?;
    let brief = read_brief(&brief_text);
    let mut measured = Measured {
        mass: Err("give the `design` to weigh it".into()),
        margins: Err("the brief has no ```json loads``` block saying what the motor turns".into()),
        pieces: None,
        components: None,
    };
    let mut notes: Vec<String> = Vec::new();
    let mass_options = crate::mass::options(brief.mass.as_ref().unwrap_or(&json!({})));
    let mut design_mass: Option<Result<c2f_geom::MassReport, String>> = None;
    let mut circuit_path = string("circuit").map(PathBuf::from);
    if let Some(design) = string("design") {
        let mut design_args = json!({ "path": design });
        if let Some(parameters) = args.get("parameters") {
            design_args["parameters"] = parameters.clone();
        }
        let (program, src) = super::load(&design_args)?;
        let evaluated = c2f_eval::evaluate(&program);
        if let Some(error) = evaluated.error {
            return Err(format!("the design does not render: {error}"));
        }
        let base_dir = program.files[0].path.parent().unwrap_or(Path::new("")).to_path_buf();
        let options = c2f_geom::PartsOptions { name: "design".into(), ..Default::default() };
        let (parts, _) = c2f_geom::parts_json(&evaluated.nodes, &base_dir, &options)?;
        let parts: Value = serde_json::from_str(&parts).map_err(|err| err.to_string())?;
        let sizes = parts["parts"].as_array().into_iter().flatten().filter_map(|part| {
            let (lo, hi) = (part["bounds"]["min"].as_array()?, part["bounds"]["max"].as_array()?);
            let size: [f64; 3] = std::array::from_fn(|i| hi[i].as_f64().unwrap_or(0.0) - lo[i].as_f64().unwrap_or(0.0));
            Some((part["name"].as_str()?.to_string(), size))
        });
        // The root holds what is outside any part(); a design of pieces
        // leaves it empty, and its name is the design's.
        let pieces: Vec<(String, [f64; 3])> = sizes.filter(|(_, size)| size.iter().any(|s| *s > 0.0)).collect();
        measured.pieces = Some(pieces);
        design_mass = Some(mass_options.clone().and_then(|options| c2f_geom::mass_properties(&evaluated.nodes, &base_dir, &options)));
        if circuit_path.is_none() {
            let (annotations, _) = c2f_lang::annotations::read(&src);
            circuit_path = annotations.circuit.map(|link| base_dir.join(link.path));
        }
    } else {
        notes.push("no `design` given: its pieces and sizes are not measured".into());
    }
    if let Some(path) = &circuit_path {
        let src = std::fs::read_to_string(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let circuit = c2f_circuit::parse(&src, path, &c2f_eval::default_library_paths())
            .map_err(|_| format!("{} does not read: check it with `check_circuit`", path.display()))?;
        measured.components = Some(circuit.parts.iter().map(|p| p.part.clone()).collect());
        // The circuit weighs its components too.
        design_mass = Some(mass_options.clone().and_then(|options| crate::mass::circuit(&circuit, &src, path, options)));
    }
    if let Some(report) = design_mass {
        measured.mass = report.map(|r| (r.total, r.stability.map(|s| s.min_tipping_deg)));
    }
    if let Some(scenario) = &brief.loads {
        let dir = brief_path.parent().filter(|p| !p.as_os_str().is_empty());
        measured.margins = crate::loads::evaluate(scenario, dir).map(|report| (report.margins.torque, report.margins.motor_radial));
    }
    let results: Vec<Value> = brief.requirements.iter().map(|r| measure(r, &measured)).collect();
    let ok = results.iter().all(|r| r["status"] != Status::Fails.name());
    let count = |status: Status| results.iter().filter(|r| r["status"] == status.name()).count();
    let report = json!({
        "ok": ok,
        "holds": count(Status::Holds),
        "fails": count(Status::Fails),
        "unmeasured": count(Status::Unmeasured),
        "requirements": results,
        "notes": notes,
    });
    if ok { Ok(vec![text(pretty(&report))]) } else { Err(pretty(&report)) }
}

/// A requirement measured: its line and text, `holds`, `fails` or
/// `unmeasured`, the value measured and why.
fn measure(requirement: &Requirement, measured: &Measured) -> Value {
    let (status, value, why): (Status, Value, String) = match &requirement.rule {
        Rule::Part(name) => match &measured.pieces {
            None => (Status::Unmeasured, Value::Null, "give the `design` to find its pieces".into()),
            Some(pieces) => {
                let names: Vec<&str> = pieces.iter().map(|(n, _)| n.as_str()).collect();
                if names.contains(&name.as_str()) {
                    (Status::Holds, json!(name), format!("the design draws part(\"{name}\")"))
                } else {
                    (Status::Fails, json!(names), format!("the design draws no part(\"{name}\"): its pieces are {}", names.join(", ")))
                }
            }
        },
        Rule::Component(file) => match &measured.components {
            None => (Status::Unmeasured, Value::Null, "the design names no circuit, and none is given".into()),
            Some(files) if files.contains(file) => (Status::Holds, json!(file), format!("the circuit uses {file}")),
            Some(files) => (Status::Fails, json!(files), format!("the circuit does not use {file}")),
        },
        Rule::Size(bed) => match &measured.pieces {
            None => (Status::Unmeasured, Value::Null, "give the `design` to measure its pieces".into()),
            Some(pieces) => {
                // A piece fits when, turned, its sides fit the bed's.
                let mut room = *bed;
                room.sort_by(f64::total_cmp);
                let too_big: Vec<String> = pieces
                    .iter()
                    .filter(|(_, size)| {
                        let mut sides = *size;
                        sides.sort_by(f64::total_cmp);
                        (0..3).any(|i| sides[i] > room[i] + 1e-6)
                    })
                    .map(|(name, size)| format!("{name} ({:.1} x {:.1} x {:.1} mm)", size[0], size[1], size[2]))
                    .collect();
                let sizes: Vec<Value> = pieces.iter().map(|(name, size)| json!({ "piece": name, "size": size })).collect();
                if too_big.is_empty() {
                    (Status::Holds, json!(sizes), "every piece fits, turned if need be".into())
                } else {
                    (Status::Fails, json!(sizes), format!("too big for the printer: {}", too_big.join(", ")))
                }
            }
        },
        Rule::Mass(op, bound) => match &measured.mass {
            Err(why) => (Status::Unmeasured, Value::Null, why.clone()),
            Ok((grams, _)) => compare(*op, *grams, *bound, "g", "the total mass, printed as the ```json mass``` block says"),
        },
        Rule::Tipping(op, bound) => match &measured.mass {
            Err(why) => (Status::Unmeasured, Value::Null, why.clone()),
            Ok((_, None)) => (Status::Unmeasured, Value::Null, "nothing rests on the ground".into()),
            Ok((_, Some(degrees))) => compare(*op, *degrees, *bound, "°", "the smallest angle it leans before it tips"),
        },
        Rule::TorqueMargin(op, bound) => match &measured.margins {
            Err(why) => (Status::Unmeasured, Value::Null, why.clone()),
            Ok((None, _)) => (Status::Unmeasured, Value::Null, "the ```json loads``` block gives no motor torque".into()),
            Ok((Some(margin), _)) => compare(*op, *margin, *bound, "x", "the motor's torque over the torque needed"),
        },
        Rule::RadialLoadMargin(op, bound) => match &measured.margins {
            Err(why) => (Status::Unmeasured, Value::Null, why.clone()),
            Ok((_, None)) => (Status::Unmeasured, Value::Null, "the ```json loads``` block gives no motor radial load limit, or the motor carries none".into()),
            Ok((_, Some(margin))) => compare(*op, *margin, *bound, "x", "the motor's largest radial load over the one it carries"),
        },
        Rule::Other => (Status::Unmeasured, Value::Null, "no tool measures it: the user checks it".into()),
    };
    json!({ "line": requirement.line, "text": requirement.text, "status": status.name(), "measured": value, "why": why })
}

/// A measure against its bound, and what it is.
fn compare(op: c2f_circuit::brief::Op, value: f64, bound: f64, unit: &str, what: &str) -> (Status, Value, String) {
    let status = if op.holds(value, bound) { Status::Holds } else { Status::Fails };
    let shown = (value * 1000.0).round() / 1000.0;
    (status, json!(shown), format!("{what}: {shown} {unit}, against {} {bound} {unit}", op.symbol()))
}
