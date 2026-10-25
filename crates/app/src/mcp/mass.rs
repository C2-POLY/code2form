//! The MCP tool for a mechanical review: `mass_properties` weighs a design
//! or a circuit printed, finds its center of gravity and how far it leans
//! before it tips, measured on the model rather than guessed.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Tool, pretty, schema, text};

pub fn tools() -> Vec<Value> {
    let print = |what: &str| {
        json!({
            "material": {
                "type": ["string", "number"],
                "default": "PLA",
                "description": format!("{what}: PLA (1.24 g/cm³), PETG (1.27), ABS (1.04), ASA (1.07), TPU (1.21) or nylon (1.14), whatever the case, or a density in g/cm³."),
            },
            "infill": { "type": "number", "minimum": 0, "maximum": 100, "default": c2f_geom::DEFAULT_INFILL, "description": "Percent of the inside filled; 100 is solid." },
            "wall": { "type": "number", "exclusiveMinimum": 0, "default": c2f_geom::DEFAULT_WALL, "description": "The walls' thickness, top and bottom too, in mm (0.8: two perimeters of 0.4)." },
        })
    };
    let mut properties = print("What every part is printed in");
    properties["parts"] = json!({
        "type": "object",
        "description": "Settings for some parts, by the name the report gives them (`root` for what is outside any part()): \
                        `{\"lid\": {\"material\": \"PETG\", \"infill\": 100}}`; what a part leaves out comes from the settings above.",
        "additionalProperties": { "type": "object", "properties": print("What this part is printed in") },
    });
    properties["extra"] = json!({
        "type": "array",
        "description": "Masses the model carries without drawing them, as a watch of 150 g on a winder's cradle.",
        "items": {
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "mass_g": { "type": "number", "minimum": 0, "description": "In grams." },
                "at": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Where its center of mass is, [x, y, z] in mm." },
            },
            "required": ["mass_g", "at"],
        },
    });
    vec![json!({
        "name": "mass_properties",
        "title": "Masses, center of gravity and stability",
        "description": "Weighs a design (.c2f, .scad) or a circuit (.c2fc: its enclosure's parts, printed, and the parts it \
                        places, each by the `// @spec mass` of its sheet, at its bounding box's center; null with a warning \
                        without one) as printed: for each part() (`root` is what is outside any) its volume, the solid's \
                        centroid and its mass, an estimate of a slicer's: the walls (surface × `wall`) plus `infill` of the \
                        rest, at the material's density. Then the total mass and center of gravity with the `extra` masses, \
                        and the stability: the base is the convex hull of the points within 0.2 mm of the lowest; `inside` \
                        says whether the center of gravity is above it, `min_tipping_deg` how far the model leans before it \
                        tips (atan of the margin to the nearest edge over the height of the center of gravity; negative: it \
                        falls), `direction` which way, as [x, y]. Masses in grams, places in mm.",
        "inputSchema": schema(properties, &[]),
    })]
}

/// The tool named `name`, if it is this one.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "mass_properties" => mass_properties,
        _ => return None,
    })
}

fn mass_properties(args: &Value) -> Result<Vec<Value>, String> {
    let options = crate::mass::options(args)?;
    let string = |key: &str| args.get(key).and_then(Value::as_str);
    // A circuit is TOML: its parts are tables.
    let is_circuit = match (string("code"), string("path")) {
        (_, Some(path)) => path.ends_with(".c2fc"),
        (Some(code), None) => code.lines().any(|line| line.trim_start().starts_with("[parts.")),
        _ => false,
    };
    if !is_circuit {
        let (program, output) = super::evaluate(args)?;
        let base_dir = program.files[0].path.parent().unwrap_or(Path::new(""));
        let mut report = c2f_geom::mass_properties(&output.nodes, base_dir, &options)?;
        let mut warnings = output.warnings;
        warnings.append(&mut report.warnings);
        report.warnings = warnings;
        return Ok(vec![text(pretty(&crate::mass::json(&report, None)))]);
    }
    if args.get("parameters").is_some() {
        return Err("`parameters` set a design's; a circuit's parts take theirs in its `params`".into());
    }
    let (src, path) = match (string("code"), string("path")) {
        (Some(code), None) => {
            let dir = string("working_dir").map_or_else(|| std::env::current_dir().unwrap_or_default(), PathBuf::from);
            (code.to_string(), dir.join("untitled.c2fc"))
        }
        (None, Some(path)) => (std::fs::read_to_string(path).map_err(|err| format!("cannot read {path}: {err}"))?, PathBuf::from(path)),
        _ => return Err("give either `code` (the circuit's text) or `path` (a .c2fc file)".into()),
    };
    let circuit = c2f_circuit::parse(&src, &path, &c2f_eval::default_library_paths())
        .map_err(|errors| errors.iter().map(c2f_circuit::Error::render).collect::<String>())?;
    let report = crate::mass::circuit(&circuit, &src, &path, options)?;
    Ok(vec![text(pretty(&crate::mass::json(&report, Some(&circuit))))])
}
