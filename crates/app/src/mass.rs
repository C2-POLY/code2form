//! `c2f mass`: what a design or a circuit weighs printed, where its center
//! of gravity is and how far it leans before it tips, as a table; the MCP
//! tool `mass_properties` gives the same as JSON. A circuit weighs its
//! enclosure's parts, printed, and the parts it places, as their sheets
//! say (`// @spec mass: 3.3 g`).

use std::path::Path;
use std::process::ExitCode;

use c2f_agent::mass::options;
use c2f_geom::{MassOptions, MassReport};
use serde_json::{Value, json};

use crate::circuit::{plural, text_table};

/// The report as `c2f mass` prints it: a row per part, component and
/// extra mass, then the total, the center of gravity and how it stands.
pub fn text(report: &MassReport) -> String {
    // Rounded first, so that a hair below zero is not `-0.0`.
    let tenth = |x: f64| (x * 10.0).round() / 10.0 + 0.0;
    let at = |p: [f64; 3]| format!("[{:.1}, {:.1}, {:.1}]", tenth(p[0]), tenth(p[1]), tenth(p[2]));
    let mut rows = vec![["part", "material", "infill", "volume", "mass", "center"].map(String::from).to_vec()];
    for part in &report.parts {
        rows.push(vec![
            part.name.clone(),
            part.print.material.name.clone(),
            format!("{} %", part.print.infill),
            format!("{:.0} mm³", part.volume),
            format!("{:.1} g", part.mass),
            at(part.center),
        ]);
    }
    for component in &report.components {
        let mass = component.mass.map_or_else(|| "?".to_string(), |g| format!("{g:.1} g"));
        rows.push(vec![component.name.clone(), "(its sheet)".into(), String::new(), String::new(), mass, at(component.center())]);
    }
    for extra in &report.extra {
        rows.push(vec![extra.name.clone(), "(extra)".into(), String::new(), String::new(), format!("{:.1} g", extra.mass), at(extra.at)]);
    }
    let mut out = text_table(&rows, &[2, 3, 4]);
    out += &format!("total: {:.1} g", report.total);
    if let Some(center) = report.center {
        out += &format!(", center of gravity at {}", at(center));
    }
    out.push('\n');
    if let Some(s) = &report.stability {
        let toward = format!("[{:.2}, {:.2}]", s.direction[0] + 0.0, s.direction[1] + 0.0);
        out += &if s.inside {
            format!(
                "stands on {} at z = {:.1}: tips first toward {toward} when leaned {:.1}° ({:.1} mm from the edge, {:.1} mm high)\n",
                plural(s.base.len(), "corner"),
                s.base_z,
                s.min_tipping_deg,
                s.margin,
                s.height
            )
        } else {
            format!("falls over toward {toward}: the center of gravity is {:.1} mm outside its base\n", -s.margin)
        };
    }
    out
}

/// `c2f mass [--material <name>] [--infill <%>] [--wall <mm>] [-D name=value]... <file>`.
pub fn run(args: &[String]) -> ExitCode {
    let mut settings = serde_json::Map::new();
    let (mut defines, mut file) = (Vec::new(), None);
    let mut rest = args.iter();
    let usage = || {
        eprintln!("{}", crate::USAGE);
        ExitCode::from(2)
    };
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--material" | "--infill" | "--wall" => {
                let Some(value) = inline.or_else(|| rest.next().cloned()) else { return usage() };
                let value = match (flag, value.parse::<f64>()) {
                    ("--material", Err(_)) => json!(value),
                    (_, Ok(n)) => json!(n),
                    (_, Err(_)) => {
                        eprintln!("error: {flag} takes a number");
                        return ExitCode::from(2);
                    }
                };
                settings.insert(flag[2..].to_string(), value);
            }
            "-D" => match rest.next() {
                Some(define) => defines.push(define.clone()),
                None => return usage(),
            },
            _ if arg.starts_with("-D") => defines.push(arg[2..].to_string()),
            _ if file.is_none() && !arg.starts_with('-') => file = Some(arg.clone()),
            _ => return usage(),
        }
    }
    let Some(file) = file else { return usage() };
    let options = match options(&Value::Object(settings)) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("error: {why}");
            return ExitCode::from(2);
        }
    };
    let path = Path::new(&file);
    let report = if path.extension().is_some_and(|e| e == "c2fc") {
        if !defines.is_empty() {
            eprintln!("error: -D sets a design's variables; a circuit's parts take theirs in `params`");
            return ExitCode::from(2);
        }
        let (circuit, src) = match c2f_agent::circuit::load(path) {
            Ok(loaded) => loaded,
            Err(errors) => {
                for error in &errors {
                    eprint!("{}", error.render());
                }
                return ExitCode::FAILURE;
            }
        };
        c2f_agent::mass::circuit(&circuit, &src, path, options)
    } else {
        design(path, &defines, &options)
    };
    match report {
        Ok(report) => {
            for warning in &report.warnings {
                eprintln!("WARNING: {warning}");
            }
            print!("{}", text(&report));
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("error: {}", why.trim_end());
            ExitCode::FAILURE
        }
    }
}

/// What the design at `path` weighs, its variables set by `defines` as
/// `-D` sets them.
fn design(path: &Path, defines: &[String], options: &MassOptions) -> Result<MassReport, String> {
    let mut src = c2f_eval::read_source(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    // As the other commands do: after an end-of-text character, which the
    // lexer skips.
    if !defines.is_empty() {
        src.push_str("\n\x03\n");
        for define in defines {
            src.push_str(define);
            src.push_str(";\n");
        }
    }
    let program = c2f_eval::Program::load(&src, path, &c2f_eval::default_library_paths()).map_err(|err| err.render())?;
    let output = c2f_eval::evaluate(&program);
    for line in &output.echo {
        eprintln!("{line}");
    }
    if let Some(error) = &output.error {
        return Err(error.clone());
    }
    let mut report = c2f_geom::mass_properties(&output.nodes, path.parent().unwrap_or(Path::new("")), options)?;
    let mut warnings = output.warnings;
    warnings.append(&mut report.warnings);
    report.warnings = warnings;
    Ok(report)
}
