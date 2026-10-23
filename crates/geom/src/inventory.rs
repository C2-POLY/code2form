//! The parts of a design as JSON: for a bill of materials, a web
//! configurator or an agent that reads a model without running it. Each
//! `part()` (and the root, what is outside any) gives its name, the part
//! it is written in, its place, its joint, its size, volume and colors;
//! the design's parameters come with them.
//!
//! The format is `code2form-parts`, version 1, in millimeters:
//!
//! ```json
//! {
//!   "format": "code2form-parts", "version": 1, "units": "mm",
//!   "name": "arm", "parameters": { "length": 30 },
//!   "bounds": { "min": [0, 0, 0], "max": [40, 40, 50] }, "volume": 17000.5,
//!   "parts": [
//!     { "name": "arm", "parent": null, "matrix": [[1, 0, 0, 0], …], "world": …,
//!       "joint": null, "bounds": …, "volume": 16000, "triangles": 12,
//!       "colors": ["#f9d72c"] },
//!     { "name": "elbow", "parent": "arm",
//!       "joint": { "type": "revolute", "axis": [0, 1, 0], "limits": [-90, 45] }, … }
//!   ]
//! }
//! ```
//!
//! `matrix` places a part in its parent's frame and `world` in the
//! model's, row by row; `bounds` are in the model's frame. A part's volume
//! and triangles are its own, without the parts inside it.

use std::path::Path;

use c2f_eval::customizer::Literal;
use c2f_eval::node::{Matrix, Node};
use serde_json::{Map, Value, json};

use crate::build::{CUT_COLOR, DEFAULT_COLOR, Model, transform};
use crate::pieces::pieces;

/// What the JSON says beside the parts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartsOptions {
    /// The design's name, and its root part's.
    pub name: String,
    /// The color of what has no `color()`, in sRGB.
    pub color: [f32; 3],
    /// The design's parameters and their values.
    pub parameters: Vec<(String, Literal)>,
}

/// Renders `nodes`, data files read relative to `base_dir`, and describes
/// their parts. An empty model has none.
pub fn parts_json(nodes: &[Node], base_dir: &Path, options: &PartsOptions) -> Result<(String, Vec<String>), String> {
    let split = pieces(nodes, base_dir, &options.name);
    let [r, g, b] = options.color;
    let mut parts = Vec::new();
    let mut total_volume = 0.0;
    let mut total: Option<([f64; 3], [f64; 3])> = None;
    for piece in &split.pieces {
        let placed = transform(piece.model.clone(), &piece.world);
        let (bounds, volume, triangles, colors) = match (&piece.model, &placed) {
            (Model::Solid(own), Model::Solid(placed)) if !own.is_empty() => {
                let extent = placed.bounding_box();
                let mesh = crate::export::colored_mesh(own);
                let mut colors: Vec<String> = Vec::new();
                for &c in &mesh.colors {
                    let c = if c == DEFAULT_COLOR || c == CUT_COLOR { [r, g, b, 1.0] } else { c };
                    let hex = hex(c);
                    if !colors.contains(&hex) {
                        colors.push(hex);
                    }
                }
                (Some(([extent.min.x, extent.min.y, extent.min.z], [extent.max.x, extent.max.y, extent.max.z])), own.volume(), own.num_tri(), colors)
            }
            (_, Model::Flat(section)) if !section.is_empty() => {
                let b = section.bounds();
                (Some(([b.min.x, b.min.y, 0.0], [b.max.x, b.max.y, 0.0])), 0.0, 0, Vec::new())
            }
            _ => (None, 0.0, 0, Vec::new()),
        };
        total_volume += volume;
        if let Some((lo, hi)) = bounds {
            let (mut min, mut max) = total.unwrap_or((lo, hi));
            for k in 0..3 {
                min[k] = min[k].min(lo[k]);
                max[k] = max[k].max(hi[k]);
            }
            total = Some((min, max));
        }
        let joint = piece.joint.as_ref().map(|joint| {
            json!({ "type": joint.kind.name(), "axis": joint.axis, "limits": joint.limits })
        });
        parts.push(json!({
            "name": piece.name,
            "parent": piece.parent.map(|p| split.pieces[p].name.clone()),
            "matrix": rows(&piece.matrix),
            "world": rows(&piece.world),
            "joint": joint,
            "bounds": bounds.map(|(lo, hi)| json!({ "min": lo, "max": hi })),
            "volume": volume,
            "triangles": triangles,
            "colors": colors,
        }));
    }
    if total.is_none() {
        return Err("the model is empty; no parts written".into());
    }
    let parameters: Map<String, Value> = options.parameters.iter().map(|(name, value)| (name.clone(), literal(value))).collect();
    let document = json!({
        "format": "code2form-parts",
        "version": 1,
        "units": "mm",
        "name": options.name,
        "parameters": parameters,
        "bounds": total.map(|(lo, hi)| json!({ "min": lo, "max": hi })),
        "volume": total_volume,
        "parts": parts,
    });
    let mut text = serde_json::to_string_pretty(&document).map_err(|err| err.to_string())?;
    text.push('\n');
    Ok((text, split.warnings))
}

/// A matrix row by row, `-0` written `0`.
fn rows(m: &Matrix) -> Value {
    json!(m.map(|row| row.map(|c| c + 0.0)))
}

fn hex([r, g, b, a]: [f32; 4]) -> String {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a < 1.0 {
        format!("#{:02x}{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b), byte(a))
    } else {
        format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_part_with_its_place_size_and_colors() {
        let src = "length = 30; // [10:50]
            color(\"red\") cube([40, 40, 10]);
            translate([20, 20, 10]) part(\"arm\", joint = \"revolute\", axis = [0, 1, 0], limits = [-90, 45])
                cube([5, 5, length]);";
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).unwrap();
        let options = PartsOptions {
            name: "robot".into(),
            color: [0.0, 0.0, 1.0],
            parameters: program.parameters(src).into_iter().map(|p| (p.name, p.default)).collect(),
        };
        let (text, warnings) = parts_json(&c2f_eval::evaluate(&program).nodes, Path::new(""), &options).unwrap();
        assert!(warnings.is_empty());
        let json: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((json["format"].as_str(), json["version"].as_i64()), (Some("code2form-parts"), Some(1)));
        assert_eq!(json["parameters"], json!({ "length": 30.0 }));
        assert_eq!(json["bounds"], json!({ "min": [0.0, 0.0, 0.0], "max": [40.0, 40.0, 40.0] }));
        assert_eq!(json["volume"].as_f64(), Some(16000.0 + 750.0));
        let root = &json["parts"][0];
        assert_eq!((root["name"].as_str(), root["parent"].is_null()), (Some("robot"), true));
        assert_eq!(root["colors"], json!(["#ff0000"]));
        let arm = &json["parts"][1];
        assert_eq!(arm["parent"], "robot");
        assert_eq!(arm["joint"], json!({ "type": "revolute", "axis": [0.0, 1.0, 0.0], "limits": [-90.0, 45.0] }));
        assert_eq!(arm["matrix"][0], json!([1.0, 0.0, 0.0, 20.0]));
        assert_eq!(arm["bounds"], json!({ "min": [20.0, 20.0, 10.0], "max": [25.0, 25.0, 40.0] }));
        assert_eq!(arm["colors"], json!(["#0000ff"]));
        assert_eq!(arm["triangles"].as_u64(), Some(12));
    }

    #[test]
    fn an_empty_model_has_no_parts() {
        let program = c2f_eval::Program::load("echo(1);", Path::new("t.scad"), &[]).unwrap();
        assert!(parts_json(&c2f_eval::evaluate(&program).nodes, Path::new(""), &PartsOptions::default()).is_err());
    }
}
