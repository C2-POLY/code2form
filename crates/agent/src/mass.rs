//! What a design or a circuit weighs printed, where its center of gravity
//! is and how far it leans before it tips: the settings the MCP tool
//! `mass_properties` and `c2f mass` take, and the report as JSON. A circuit
//! weighs its enclosure's parts, printed, and the parts it places, as their
//! sheets say (`// @spec mass: 3.3 g`).

use std::path::Path;

use c2f_circuit::Circuit;
use c2f_geom::{Component, Extra, MassOptions, MassReport, Material, Print};
use serde_json::{Value, json};

use crate::circuit::assemble;

/// The settings the tool's arguments give: `material`, `infill`, `wall`,
/// `parts` ({name: {material, infill, wall}}) and `extra` ([{name,
/// mass_g, at}]).
pub fn options(args: &Value) -> Result<MassOptions, String> {
    let print = print(args, &Print::default(), "")?;
    let mut options = MassOptions { print: print.clone(), ..MassOptions::default() };
    match args.get("parts") {
        None | Some(Value::Null) => {}
        Some(Value::Object(parts)) => {
            for (name, settings) in parts {
                let settings = settings.as_object().ok_or_else(|| format!("`parts.{name}` is an object: {{material, infill, wall}}"))?;
                if let Some(key) = settings.keys().find(|key| !["material", "infill", "wall"].contains(&key.as_str())) {
                    return Err(format!("`parts.{name}.{key}`: a part takes `material`, `infill` and `wall`"));
                }
                options.parts.push((name.clone(), self::print(&Value::Object(settings.clone()), &print, &format!("parts.{name}."))?));
            }
        }
        Some(_) => return Err("`parts` is an object of part names and their {material, infill, wall}".into()),
    }
    match args.get("extra") {
        None | Some(Value::Null) => {}
        Some(Value::Array(extra)) => {
            for (i, item) in extra.iter().enumerate() {
                let what = format!("`extra[{i}]`");
                let name = match item.get("name") {
                    None => format!("extra {}", i + 1),
                    Some(name) => name.as_str().ok_or(format!("{what}.name is a string"))?.to_string(),
                };
                let mass = item
                    .get("mass_g")
                    .and_then(Value::as_f64)
                    .filter(|g| g.is_finite() && *g >= 0.0)
                    .ok_or(format!("{what}.mass_g is its mass, a number of grams, 0 or more"))?;
                let at = item
                    .get("at")
                    .and_then(Value::as_array)
                    .filter(|at| at.len() == 3)
                    .and_then(|at| at.iter().map(|c| c.as_f64().filter(|c| c.is_finite())).collect::<Option<Vec<f64>>>())
                    .ok_or(format!("{what}.at is where it is, [x, y, z] in millimeters"))?;
                options.extra.push(Extra { name, mass, at: [at[0], at[1], at[2]] });
            }
        }
        Some(_) => return Err("`extra` is a list of masses: [{\"name\": \"watch\", \"mass_g\": 150, \"at\": [0, 0, 60]}]".into()),
    }
    Ok(options)
}

/// `base` with the `material`, `infill` and `wall` of `args`, named in
/// errors after `prefix`.
fn print(args: &Value, base: &Print, prefix: &str) -> Result<Print, String> {
    let mut print = base.clone();
    match args.get("material") {
        None | Some(Value::Null) => {}
        Some(Value::String(name)) => print.material = Material::named(name).map_err(|why| format!("`{prefix}material`: {why}"))?,
        Some(Value::Number(n)) => {
            print.material = Material::density(n.as_f64().unwrap_or(f64::NAN)).map_err(|why| format!("`{prefix}material`: {why}"))?
        }
        Some(_) => return Err(format!("`{prefix}material` is a name (PLA, PETG...) or a density in g/cm³")),
    }
    for key in ["infill", "wall"] {
        if let Some(value) = args.get(key).filter(|v| !v.is_null()) {
            let value = value.as_f64().ok_or(format!("`{prefix}{key}` is a number"))?;
            if key == "infill" { print.infill = value } else { print.wall = value }
        }
    }
    print.check().map_err(|why| match prefix {
        "" => why,
        part => format!("`{}`: {why}", part.trim_end_matches('.')),
    })?;
    Ok(print)
}

/// What a circuit weighs: the parts of its enclosure printed as `options`
/// says, and the parts it places, each where its bounding box's center
/// is, by the mass its sheet gives. The error when a part or the
/// enclosure does not load.
pub fn circuit(circuit: &Circuit, src: &str, path: &Path, mut options: MassOptions) -> Result<MassReport, String> {
    let assembly = assemble(circuit, src, path);
    if !assembly.errors.is_empty() {
        return Err(assembly.errors.iter().map(c2f_circuit::Error::render).collect());
    }
    let mut warnings = assembly.warnings.clone();
    let dir = path.parent().unwrap_or(Path::new(""));
    for (instance, node) in circuit.parts.iter().zip(&assembly.nodes) {
        let Some((lo, hi)) = c2f_geom::render_in(std::slice::from_ref(node), dir).bounds() else {
            warnings.push(format!("`{}` draws nothing: it has no place to weigh at", instance.name));
            continue;
        };
        let spec = instance.annotations.specs.iter().find(|s| s.key.trim().eq_ignore_ascii_case("mass"));
        if let Some(Some(Err(why))) = spec.map(c2f_circuit::quantity::figure) {
            warnings.push(format!("`{}`: {why}", instance.name));
        }
        let mass = c2f_circuit::quantity::spec_figure(&instance.annotations.specs, "mass");
        options.components.push(Component { name: instance.name.clone(), mass, bounds: (lo, hi) });
    }
    let (nodes, base_dir) = match &circuit.enclosure {
        Some(file) => {
            let program = c2f_eval::read_source(file)
                .ok()
                .and_then(|src| c2f_eval::Program::load_in(&src, file, &c2f_eval::default_library_paths(), circuit.scope.as_ref()).ok())
                .ok_or_else(|| format!("cannot load the enclosure {}", file.display()))?;
            let output = c2f_eval::evaluate(&program);
            if let Some(error) = output.error {
                return Err(format!("the enclosure {} stops: {error}", file.display()));
            }
            warnings.extend(output.warnings);
            // `%` copies of the parts are there to be seen through.
            let nodes: Vec<_> = output.nodes.into_iter().filter(|node| !node.background).collect();
            (nodes, file.parent().unwrap_or(Path::new("")).to_path_buf())
        }
        None => (Vec::new(), dir.to_path_buf()),
    };
    let mut report = c2f_geom::mass_properties(&nodes, &base_dir, &options)?;
    warnings.append(&mut report.warnings);
    report.warnings = warnings;
    Ok(report)
}

/// Rounded to a thousandth, `-0` written `0`.
fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0 + 0.0
}

fn point(p: &[f64]) -> Value {
    json!(p.iter().map(|c| round(*c)).collect::<Vec<_>>())
}

/// The report as the MCP tool gives it; `circuit` names its parts' files.
pub fn json(report: &MassReport, circuit: Option<&Circuit>) -> Value {
    let parts: Vec<Value> = report
        .parts
        .iter()
        .map(|part| {
            json!({
                "name": part.name,
                "volume_mm3": round(part.volume),
                "area_mm2": round(part.area),
                "mass_g": round(part.mass),
                "material": part.print.material.name,
                "density_g_cm3": part.print.material.density,
                "infill": part.print.infill,
                "wall": part.print.wall,
                "center": point(&part.center),
            })
        })
        .collect();
    let components: Vec<Value> = report
        .components
        .iter()
        .map(|component| {
            let file = circuit.and_then(|c| c.parts.iter().find(|p| p.name == component.name)).map(|p| p.part.clone());
            json!({
                "name": component.name,
                "part": file,
                "mass_g": component.mass.map(round),
                "center": point(&component.center()),
            })
        })
        .collect();
    let extra: Vec<Value> =
        report.extra.iter().map(|e| json!({ "name": e.name, "mass_g": round(e.mass), "at": point(&e.at) })).collect();
    let stability = report.stability.as_ref().map(|s| {
        json!({
            "inside": s.inside,
            "min_tipping_deg": round(s.min_tipping_deg),
            "direction": point(&s.direction),
            "margin_mm": round(s.margin),
            "height_mm": round(s.height),
            "base": s.base.iter().map(|p| point(p)).collect::<Vec<_>>(),
            "base_z": round(s.base_z),
        })
    });
    let mut value = json!({
        "parts": parts,
        "extra": extra,
        "total": { "mass_g": round(report.total), "center": report.center.as_ref().map(|c| point(c)) },
        "stability": stability,
        "warnings": report.warnings,
    });
    if circuit.is_some() {
        value["components"] = json!(components);
    }
    value
}
