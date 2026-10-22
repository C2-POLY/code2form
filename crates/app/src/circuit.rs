//! `c2f circuit`: reads a `.c2fc` circuit and prints what building it
//! takes: its problems (its parts' errors and its wiring's), its wiring list in the order the wires are laid,
//! its bill of materials, or its parts assembled as one STL or 3MF.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use c2f_circuit::check::{self, Severity};
use c2f_circuit::{Circuit, Error, Instance, route};
use c2f_eval::{Node, NodeKind};
use c2f_lang::Diagnostic;

use crate::Export;

const COMMANDS: [&str; 5] = ["check", "wires", "bom", "stl", "3mf"];

/// What `c2f circuit wires` adds to each wire's routed length by default,
/// in millimeters: two stripped ends of 5 mm, as the app does.
pub const DEFAULT_MARGIN: f64 = 10.0;

/// `c2f circuit <command> [--csv] [--margin <mm>] <file.c2fc>`.
pub fn run(args: &[String]) -> ExitCode {
    let csv = args.iter().any(|arg| arg == "--csv");
    let mut margin = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut args_left = args.iter().map(String::as_str);
    while let Some(arg) = args_left.next() {
        let value = match arg.strip_prefix("--margin") {
            Some("") => args_left.next(),
            Some(inline) if inline.starts_with('=') => Some(&inline[1..]),
            _ => {
                if arg != "--csv" {
                    rest.push(arg);
                }
                continue;
            }
        };
        match value.and_then(|v| v.parse::<f64>().ok()).filter(|mm| mm.is_finite() && *mm >= 0.0) {
            Some(mm) => margin = Some(mm),
            None => {
                eprintln!("error: --margin takes a length in millimeters, 0 or more");
                return ExitCode::from(2);
            }
        }
    }
    if margin.is_some() && rest.first() != Some(&"wires") {
        eprintln!("error: --margin goes with `c2f circuit wires`");
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
            crate::print_export(&rendered, export)
        }
    }
}

/// Reads the circuit file at `path`, its parts found next to it or in the
/// library folders; also returns its text.
pub fn load(path: &Path) -> Result<(Circuit, String), Vec<Error>> {
    let src = std::fs::read_to_string(path).map_err(|err| {
        vec![Error {
            path: path.to_path_buf(),
            src: String::new(),
            diagnostic: Diagnostic::new(format!("cannot read the circuit: {err}"), c2f_lang::Span::default()),
        }]
    })?;
    let circuit = c2f_circuit::parse(&src, path, &c2f_eval::default_library_paths())?;
    Ok((circuit, src))
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
    let collisions = if assembly.errors.is_empty() { collisions(circuit, &assembly) } else { Vec::new() };
    for collision in &collisions {
        eprintln!("WARNING: {collision}");
    }
    let count = |severity| findings.iter().filter(|f| f.severity == severity).count();
    let errors = assembly.errors.len() + count(Severity::Error);
    let warnings = count(Severity::Warning) + collisions.len();
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

/// Where the parts run into each other or into the enclosure, one line
/// each: `` `touch` runs into the enclosure by 120.00 mm³ ``.
fn collisions(circuit: &Circuit, assembly: &Assembly) -> Vec<String> {
    const IDENTITY: c2f_geom::Placement =
        [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    // Each part's nodes are placed already.
    let mut models: Vec<c2f_geom::Model> =
        assembly.nodes.iter().map(|node| c2f_geom::render(std::slice::from_ref(node)).model).collect();
    if let Some(enclosure) = &circuit.enclosure {
        let library_paths = c2f_eval::default_library_paths();
        let program = c2f_eval::read_source(enclosure)
            .ok()
            .and_then(|src| c2f_eval::Program::load(&src, enclosure, &library_paths).ok());
        match program {
            Some(program) => {
                let output = c2f_eval::evaluate(&program);
                // `%` copies of the parts, there to be seen through, are
                // not the enclosure: building them would only cost time.
                let nodes: Vec<_> = output.nodes.into_iter().filter(|node| !node.background).collect();
                models.push(c2f_geom::render_in(&nodes, enclosure.parent().unwrap_or(Path::new(""))).model);
            }
            None => return vec![format!("collisions: cannot load the enclosure {}", enclosure.display())],
        }
    }
    let solids: Vec<_> = models.iter().map(|model| (model, IDENTITY)).collect();
    let name = |i: usize| format!("`{}`", circuit.parts[i].name);
    c2f_geom::collisions(&solids)
        .into_iter()
        .map(|(a, b, volume)| match b < circuit.parts.len() {
            true => format!("collision: {} and {} overlap by {volume:.2} mm³", name(a), name(b)),
            false => format!("collision: {} runs into the enclosure by {volume:.2} mm³", name(a)),
        })
        .collect()
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// The parts of a circuit evaluated, each distinct part and set of
/// parameters once, and placed where the circuit puts them.
pub struct Assembly {
    /// Each part's nodes under the matrix that places it, in the order of
    /// the circuit's parts.
    pub nodes: Vec<Node>,
    /// A part that does not load is reported in its own file; one whose
    /// evaluation stops, at its name in the circuit.
    pub errors: Vec<Error>,
    /// Evaluation warnings, after the part they come from.
    pub warnings: Vec<String>,
}

impl Assembly {
    /// The parts' geometry, together, data files read where each part
    /// names them.
    pub fn render(&self, circuit_path: &Path) -> c2f_geom::Rendered {
        c2f_geom::render_in(&self.nodes, circuit_path.parent().unwrap_or(Path::new("")))
    }
}

/// A part file and the `-D` assignments of its parameters.
type PartKey = (PathBuf, Vec<String>);

pub fn assemble(circuit: &Circuit, src: &str, path: &Path) -> Assembly {
    let library_paths = c2f_eval::default_library_paths();
    let mut assembly = Assembly { nodes: Vec::new(), errors: Vec::new(), warnings: Vec::new() };
    // By file and parameters: the part's nodes, `None` when it failed.
    let mut evaluated: Vec<(PartKey, Option<Vec<Node>>)> = Vec::new();
    for instance in &circuit.parts {
        let key = (instance.path.clone(), instance.defines());
        let index = match evaluated.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                let nodes = evaluate(instance, &library_paths, src, path, &mut assembly);
                evaluated.push((key, nodes));
                evaluated.len() - 1
            }
        };
        if let Some(nodes) = &evaluated[index].1 {
            assembly.nodes.push(Node {
                kind: NodeKind::Transform(instance.matrix()),
                highlight: false,
                background: false,
                children: nodes.clone(),
            });
        }
    }
    assembly
}

/// One part's nodes, the data files they read named from the part's
/// folder; `None` when it does not load or its evaluation stops.
fn evaluate(
    instance: &Instance,
    library_paths: &[PathBuf],
    src: &str,
    path: &Path,
    assembly: &mut Assembly,
) -> Option<Vec<Node>> {
    let what = format!("`{}` ({})", instance.name, instance.part);
    let program = match c2f_eval::Program::load(&instance.source_with_params(), &instance.path, library_paths) {
        Ok(program) => program,
        Err(err) => {
            assembly.errors.push(Error { path: err.path, src: err.src, diagnostic: err.diagnostic });
            return None;
        }
    };
    assembly.warnings.extend(program.warnings.iter().map(|w| format!("{what}: {w}")));
    let output = c2f_eval::evaluate(&program);
    assembly.warnings.extend(output.warnings.iter().map(|w| format!("{what}: {w}")));
    if let Some(error) = output.error {
        let diagnostic = Diagnostic::new(format!("{what} stops: {error}"), instance.span);
        assembly.errors.push(Error { path: path.to_path_buf(), src: src.to_string(), diagnostic });
        return None;
    }
    let dir = instance.path.parent().unwrap_or(Path::new(""));
    let mut nodes = output.nodes;
    for node in &mut nodes {
        rebase(node, dir);
    }
    Some(nodes)
}

/// Names the files `node` reads from `dir`, the folder they are relative
/// to: the parts of a circuit come from several folders, rendered as one.
fn rebase(node: &mut Node, dir: &Path) {
    let join = |file: &mut String| {
        if !file.is_empty() {
            *file = dir.join(&*file).display().to_string();
        }
    };
    match &mut node.kind {
        NodeKind::Surface { file, .. } | NodeKind::Import { file, .. } => join(file),
        NodeKind::LinearExtrude { source: Some(source), .. } | NodeKind::RotateExtrude { source: Some(source), .. } => {
            join(&mut source.file)
        }
        _ => {}
    }
    for child in &mut node.children {
        rebase(child, dir);
    }
}

/// The wiring list, a header and a row per wire in the order they are
/// laid: step, each end and its pin's label, color, label, and the length
/// to cut: the curve the views draw, plus `margin` millimeters.
pub fn wires(circuit: &Circuit, margin: f64) -> Vec<Vec<String>> {
    let header = ["step", "from", "from_label", "to", "to_label", "color", "label", "length_mm"];
    let mut rows = vec![header.map(String::from).to_vec()];
    for (i, (wire, path)) in circuit.wires.iter().zip(route::paths(circuit)).enumerate() {
        let label = |end| circuit.pin(end).label.clone().unwrap_or_default();
        rows.push(vec![
            (i + 1).to_string(),
            circuit.end_name(wire.from),
            label(wire.from),
            circuit.end_name(wire.to),
            label(wire.to),
            wire.color.clone().unwrap_or_default(),
            wire.label.clone().unwrap_or_default(),
            format!("{:.1}", route::length_of(&path) + margin),
        ]);
    }
    rows
}

/// The bill of materials, a header and a row per part file and set of
/// parameters, in the order they first appear: name, quantity, file,
/// parameters, and the instances' names.
pub fn bom(circuit: &Circuit) -> Vec<Vec<String>> {
    let mut groups: Vec<(&Instance, Vec<String>, Vec<&str>)> = Vec::new();
    for instance in &circuit.parts {
        let defines = instance.defines();
        match groups.iter_mut().find(|(first, d, _)| first.path == instance.path && *d == defines) {
            Some((_, _, names)) => names.push(&instance.name),
            None => groups.push((instance, defines, vec![&instance.name])),
        }
    }
    let header = ["name", "quantity", "file", "params", "instances"];
    let mut rows = vec![header.map(String::from).to_vec()];
    for (first, defines, names) in groups {
        let name = first.path.file_stem().map_or_else(|| first.part.clone(), |s| s.to_string_lossy().into_owned());
        rows.push(vec![name, names.len().to_string(), first.part.clone(), defines.join("; "), names.join(" ")]);
    }
    rows
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
fn text_table(rows: &[Vec<String>], right: &[usize]) -> String {
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
