//! `c2f loads scenario.json`: the static loads on a turning shaft, the
//! torque its motor must give and the margins against the motor's and the
//! bearings' sheets ([`c2f_circuit::loads`]), from the JSON the MCP
//! `loads` tool takes.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use c2f_agent::loads::evaluate;
use c2f_circuit::loads::{Report, num};
use serde_json::Value;

use crate::circuit::text_table;

/// `c2f loads [--json] <scenario.json | ->`.
pub fn run(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let files: Vec<&String> = args.iter().filter(|arg| *arg != "--json").collect();
    let [file] = files.as_slice() else {
        eprintln!("usage: c2f loads [--json] <scenario.json>   (- reads it from stdin)");
        return ExitCode::from(2);
    };
    let (text, dir) = if file.as_str() == "-" {
        let mut text = String::new();
        if let Err(err) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("error: cannot read stdin: {err}");
            return ExitCode::FAILURE;
        }
        (text, None)
    } else {
        match std::fs::read_to_string(file) {
            Ok(text) => (text, Path::new(file).parent().map(Path::to_path_buf)),
            Err(err) => {
                eprintln!("error: cannot read {file}: {err}");
                return ExitCode::FAILURE;
            }
        }
    };
    let report = serde_json::from_str::<Value>(&text)
        .map_err(|err| format!("{file} is not JSON: {err}"))
        .and_then(|args| evaluate(&args, dir.as_deref()));
    let report = match report {
        Ok(report) => report,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    } else {
        print!("{}", text_report(&report));
    }
    for warning in &report.warnings {
        eprintln!("WARNING: {warning}");
    }
    ExitCode::SUCCESS
}

/// The torque, a row per support and per load, then the summary.
fn text_report(report: &Report) -> String {
    let torque = &report.torque;
    let mut out = format!(
        "torque needed at the shaft: {} mN·m ({} static in the worst position of a turn, {} here, {} friction)\n\n",
        num(torque.needed_mnm),
        num(torque.static_mnm),
        num(torque.here_mnm),
        num(torque.friction_mnm)
    );
    let mut rows = vec![["support", "kind", "at", "radial", "axial", "moment", "bearing load", "friction"].map(String::from).to_vec()];
    for support in &report.supports {
        let kind = serde_json::to_value(support.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        rows.push(vec![
            support.name.clone(),
            if support.carries { kind } else { format!("{kind} (coupling)") },
            format!("{} mm", num(support.at_mm)),
            format!("{} N", num(support.radial_n)),
            format!("{} N", num(support.axial_n)),
            format!("{} N·mm", num(support.moment_nmm)),
            format!("{} N", num(support.bearing_load_n)),
            format!("{} mN·m", num(support.friction_mnm)),
        ]);
    }
    out.push_str(&text_table(&rows, &[2, 3, 4, 5, 6, 7]));
    out.push('\n');
    let mut rows = vec![["load", "mass", "at", "offset", "deflection"].map(String::from).to_vec()];
    for load in &report.loads {
        rows.push(vec![
            load.name.clone(),
            format!("{} g", num(load.mass_g)),
            format!("{} mm", num(load.at_mm)),
            format!("{} mm", num(load.offset_mm)),
            format!("{} mm", num(load.deflection_mm)),
        ]);
    }
    out.push_str(&text_table(&rows, &[1, 2, 3, 4]));
    out.push('\n');
    for sentence in &report.summary {
        out.push_str(sentence);
        out.push('\n');
    }
    out
}
