//! `c2f circuit`: reads a `.c2fc` circuit and prints what building it
//! takes: its problems (its parts' errors and its wiring's), its wiring list in the order the wires are laid,
//! its bill of materials, how its parts fit in their enclosure, or its
//! parts assembled as one STL or 3MF.

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

use c2f_agent::Export;
use c2f_agent::circuit::{DEFAULT_MARGIN, assemble, bom, fit, load, placed_warnings, wires};
use c2f_circuit::Circuit;
use c2f_circuit::check::{self, Severity};

const COMMANDS: [&str; 6] = ["check", "wires", "bom", "fit", "stl", "3mf"];

/// `c2f circuit <command> [--csv] [--margin <mm>] [--clearance <mm>] <file.c2fc>`.
pub fn run(args: &[String]) -> ExitCode {
    let csv = args.iter().any(|arg| arg == "--csv");
    let (mut margin, mut clearance) = (None, None);
    let mut rest: Vec<&str> = Vec::new();
    let mut args_left = args.iter().map(String::as_str);
    while let Some(arg) = args_left.next() {
        let option = ["--margin", "--clearance"].into_iter().find_map(|option| Some((option, arg.strip_prefix(option)?)));
        let (option, value) = match option {
            Some((option, "")) => (option, args_left.next()),
            Some((option, inline)) if inline.starts_with('=') => (option, Some(&inline[1..])),
            _ => {
                if arg != "--csv" {
                    rest.push(arg);
                }
                continue;
            }
        };
        match value.and_then(|v| v.parse::<f64>().ok()).filter(|mm| mm.is_finite() && *mm >= 0.0) {
            Some(mm) if option == "--margin" => margin = Some(mm),
            // Gaps are measured up to SEARCH: a larger clearance could not be told.
            Some(mm) if mm <= c2f_geom::SEARCH => clearance = Some(mm),
            Some(_) => {
                eprintln!("error: --clearance takes a length in millimeters, from 0 to {}", c2f_geom::SEARCH);
                return ExitCode::from(2);
            }
            None => {
                eprintln!("error: {option} takes a length in millimeters, 0 or more");
                return ExitCode::from(2);
            }
        }
    }
    if margin.is_some() && rest.first() != Some(&"wires") {
        eprintln!("error: --margin goes with `c2f circuit wires`");
        return ExitCode::from(2);
    }
    if clearance.is_some() && rest.first() != Some(&"fit") {
        eprintln!("error: --clearance goes with `c2f circuit fit`");
        return ExitCode::from(2);
    }
    let [command, file] = rest[..] else {
        eprintln!("{}", crate::USAGE);
        return ExitCode::from(2);
    };
    if !COMMANDS.contains(&command) {
        eprintln!("unknown circuit command `{command}`\n\n{}", crate::USAGE);
        return ExitCode::from(2);
    }
    if csv && !["wires", "bom"].contains(&command) {
        eprintln!("error: --csv goes with `c2f circuit wires` and `c2f circuit bom`");
        return ExitCode::from(2);
    }
    if command == "3mf" && std::io::stdout().is_terminal() {
        eprintln!("error: a 3MF package is binary; redirect it to a file: c2f circuit 3mf {file} > circuit.3mf");
        return ExitCode::from(2);
    }
    let path = Path::new(file);
    let (circuit, src) = match load(path) {
        Ok(loaded) => loaded,
        Err(errors) => {
            for error in &errors {
                eprint!("{}", error.render());
            }
            return ExitCode::FAILURE;
        }
    };
    match command {
        "wires" => {
            let rows = wires(&circuit, margin.unwrap_or(DEFAULT_MARGIN));
            print!("{}", if csv { csv_table(&rows) } else { wires_text(&rows) });
            ExitCode::SUCCESS
        }
        "bom" => {
            let rows = bom(&circuit);
            print!("{}", if csv { csv_table(&rows) } else { text_table(&rows, &[1]) });
            ExitCode::SUCCESS
        }
        "check" => check(&circuit, &src, path),
        "fit" => fit_command(&circuit, &src, path, clearance.unwrap_or(c2f_geom::CLEARANCE)),
        // An export format.
        _ => {
            let assembly = assemble(&circuit, &src, path);
            for warning in &assembly.warnings {
                eprintln!("WARNING: {warning}");
            }
            if !assembly.errors.is_empty() {
                for error in &assembly.errors {
                    eprint!("{}", error.render());
                }
                return ExitCode::FAILURE;
            }
            let export = Export::named(command).expect("an export command");
            let rendered = assembly.render(path);
            for warning in &rendered.warnings {
                eprintln!("WARNING: {warning}");
            }
            crate::print_export(&rendered, export, "circuit", None, None)
        }
    }
}

/// Evaluates every part and checks the wiring ([`check::check`]), then
/// prints the errors and warnings with their line, and a summary. Only
/// errors fail.
fn check(circuit: &Circuit, src: &str, path: &Path) -> ExitCode {
    let assembly = assemble(circuit, src, path);
    for warning in &assembly.warnings {
        eprintln!("WARNING: {warning}");
    }
    for error in &assembly.errors {
        eprint!("{}", error.render());
    }
    let findings = check::check(circuit);
    for finding in &findings {
        eprint!("{}", finding.render(src, path));
    }
    // Collisions need every part: a part that failed is reported already.
    let placed = if assembly.errors.is_empty() { placed_warnings(circuit, &assembly) } else { Vec::new() };
    for warning in &placed {
        eprintln!("WARNING: {warning}");
    }
    let count = |severity| findings.iter().filter(|f| f.severity == severity).count();
    let errors = assembly.errors.len() + count(Severity::Error);
    let warnings = count(Severity::Warning) + placed.len();
    let mut summary = format!("{}: {}, {}: ", path.display(), plural(circuit.parts.len(), "part"), plural(circuit.wires.len(), "wire"));
    summary += &match errors {
        0 => "no errors".to_string(),
        n => plural(n, "error"),
    };
    if warnings > 0 {
        summary += &format!(", {}", plural(warnings, "warning"));
    }
    if errors > 0 {
        eprintln!("{summary}");
        return ExitCode::FAILURE;
    }
    println!("{summary}");
    ExitCode::SUCCESS
}

/// Prints a row per part (its least room, to the enclosure and to the
/// nearest part), then the warnings and a summary. Only a part or an
/// enclosure that does not load fails: a tight part is a warning.
fn fit_command(circuit: &Circuit, src: &str, path: &Path, clearance: f64) -> ExitCode {
    let assembly = assemble(circuit, src, path);
    for warning in &assembly.warnings {
        eprintln!("WARNING: {warning}");
    }
    if !assembly.errors.is_empty() {
        for error in &assembly.errors {
            eprint!("{}", error.render());
        }
        return ExitCode::FAILURE;
    }
    let fitted = match fit(circuit, &assembly) {
        Ok(fitted) => fitted,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };
    let far = format!("over {} mm", c2f_geom::SEARCH);
    // `any`: whether there is something to measure to.
    let room = |gap: Option<&c2f_geom::Gap>, any: bool| match gap {
        None if any => far.clone(),
        None => "-".to_string(),
        Some(gap) if gap.overlap > c2f_geom::TOUCHING => format!("overlaps by {:.2} mm³", gap.overlap),
        Some(gap) => format!("{:.2} mm", gap.distance),
    };
    let mut rows = vec![["part", "clearance", "enclosure", "rests on", "nearest part"].map(String::from).to_vec()];
    for (instance, part) in circuit.parts.iter().zip(&fitted.parts) {
        let least = part.clearance().map_or_else(|| far.clone(), |mm| format!("{mm:.2} mm"));
        let nearest = match part.parts.first() {
            Some((other, gap)) => format!("{} {}", circuit.parts[*other].name, room(Some(gap), true)),
            None => room(None, circuit.parts.len() > 1),
        };
        let enclosure = room(part.enclosure.as_ref(), fitted.enclosure);
        rows.push(vec![instance.name.clone(), least, enclosure, room(part.rests.as_ref(), fitted.enclosure), nearest]);
    }
    print!("{}", text_table(&rows, &[]));
    let warnings = fitted.warnings(circuit, clearance);
    for warning in &warnings {
        eprintln!("WARNING: {warning}");
    }
    let summary = match warnings.len() {
        0 => format!("everything fits with {clearance} mm of clearance"),
        n => plural(n, "warning"),
    };
    println!("{}: {}, {summary}", path.display(), plural(circuit.parts.len(), "part"));
    ExitCode::SUCCESS
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// Comma-separated values, quoted where a value needs it.
pub fn csv_table(rows: &[Vec<String>]) -> String {
    let field = |value: &String| {
        if value.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.clone()
        }
    };
    rows.iter().map(|row| row.iter().map(field).collect::<Vec<_>>().join(",") + "\n").collect()
}

/// The wiring list as text: a pin's label in parentheses after it, and
/// the length, in whole millimeters, before the wire's own label.
fn wires_text(rows: &[Vec<String>]) -> String {
    let end = |name: &String, label: &String| match label.as_str() {
        "" => name.clone(),
        label => format!("{name} ({label})"),
    };
    let mut text = vec![["step", "from", "to", "color", "length", "label"].map(String::from).to_vec()];
    for row in &rows[1..] {
        let length = row[7].parse::<f64>().map_or_else(|_| row[7].clone(), |mm| format!("{mm:.0} mm"));
        text.push(vec![row[0].clone(), end(&row[1], &row[2]), end(&row[3], &row[4]), row[5].clone(), length, row[6].clone()]);
    }
    text_table(&text, &[0, 4])
}

/// Rows as aligned columns, those in `right` aligned to the right.
pub fn text_table(rows: &[Vec<String>], right: &[usize]) -> String {
    let widths: Vec<usize> =
        (0..rows[0].len()).map(|c| rows.iter().map(|row| row[c].chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| {
                let pad = " ".repeat(widths[c] - cell.chars().count());
                if right.contains(&c) { pad + cell } else { cell.clone() + &pad }
            })
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}
