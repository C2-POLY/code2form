//! What building a `.c2fc` circuit takes, for `c2f circuit` and the
//! electronics tools: its parts evaluated and placed, how they fit in their
//! enclosure, its wiring list in the order the wires are laid and its bill
//! of materials.

use std::path::{Path, PathBuf};

use c2f_circuit::{Circuit, Error, Instance, route};
use c2f_eval::{Node, NodeKind};
use c2f_geom::{Crossing, Model, PartFit};
use c2f_lang::Diagnostic;

/// What `c2f circuit wires` adds to each wire's routed length by default,
/// in millimeters: two stripped ends of 5 mm, as the app does.
pub const DEFAULT_MARGIN: f64 = 10.0;

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

const IDENTITY: c2f_geom::Placement =
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// The solids of an assembly's parts, placed already, and the circuit's
/// enclosure if it names one; the error when the enclosure does not load.
fn solids(circuit: &Circuit, assembly: &Assembly) -> Result<(Vec<Model>, Option<Model>), String> {
    let models = assembly.nodes.iter().map(|node| c2f_geom::render(std::slice::from_ref(node)).model).collect();
    Ok((models, enclosure(circuit)?.map(|rendered| rendered.model)))
}

/// The circuit's enclosure rendered, if it names one; the error when it
/// does not load.
pub fn enclosure(circuit: &Circuit) -> Result<Option<c2f_geom::Rendered>, String> {
    let Some(enclosure) = &circuit.enclosure else { return Ok(None) };
    let library_paths = c2f_eval::default_library_paths();
    let program = c2f_eval::read_source(enclosure)
        .ok()
        .and_then(|src| c2f_eval::Program::load(&src, enclosure, &library_paths).ok())
        .ok_or_else(|| format!("cannot load the enclosure {}", enclosure.display()))?;
    let output = c2f_eval::evaluate(&program);
    // `%` copies of the parts, there to be seen through, are not the
    // enclosure: building them would only cost time.
    let nodes: Vec<_> = output.nodes.into_iter().filter(|node| !node.background).collect();
    Ok(Some(c2f_geom::render_in(&nodes, enclosure.parent().unwrap_or(Path::new("")))))
}

/// Where the parts run into each other or into the enclosure, and where
/// wires run into the enclosure, one line each: `` collision: `touch`
/// runs into the enclosure by 120.00 mm³ ``.
pub fn placed_warnings(circuit: &Circuit, assembly: &Assembly) -> Vec<String> {
    let (models, enclosure) = match solids(circuit, assembly) {
        Ok(solids) => solids,
        Err(err) => return vec![format!("collisions: {err}")],
    };
    let mut solids: Vec<_> = models.iter().map(|model| (model, IDENTITY)).collect();
    solids.extend(enclosure.iter().map(|model| (model, IDENTITY)));
    let name = |i: usize| format!("`{}`", circuit.parts[i].name);
    let mut warnings: Vec<String> = c2f_geom::collisions(&solids)
        .into_iter()
        .map(|(a, b, volume)| match b < circuit.parts.len() {
            true => format!("collision: {} and {} overlap by {volume:.2} mm³", name(a), name(b)),
            false => format!("collision: {} runs into the enclosure by {volume:.2} mm³", name(a)),
        })
        .collect();
    if let Some(enclosure) = &enclosure {
        warnings.extend(c2f_geom::crossings(circuit, enclosure).iter().map(|crossing| crossing.message(circuit)));
    }
    warnings
}

/// How the parts of a circuit fit in its enclosure.
pub struct Fitted {
    /// In the order of the circuit's parts.
    pub parts: Vec<PartFit>,
    /// The wires that run into the enclosure.
    pub crossings: Vec<Crossing>,
    /// Whether the circuit has an enclosure.
    pub enclosure: bool,
}

impl Fitted {
    /// Every warning, part after part, then the wires'.
    pub fn warnings(&self, circuit: &Circuit, clearance: f64) -> Vec<String> {
        let parts = self.parts.iter().enumerate().flat_map(|(i, fit)| fit.warnings(circuit, i, clearance, true));
        parts.chain(self.crossings.iter().map(|crossing| crossing.message(circuit))).collect()
    }
}

/// How the parts of `circuit`, evaluated in `assembly` without errors,
/// fit; the error when the enclosure does not load.
pub fn fit(circuit: &Circuit, assembly: &Assembly) -> Result<Fitted, String> {
    let (models, enclosure) = solids(circuit, assembly)?;
    let parts: Vec<_> = models.iter().map(|model| (model, IDENTITY)).collect();
    Ok(Fitted {
        parts: c2f_geom::fit(circuit, &parts, enclosure.as_ref()),
        crossings: enclosure.as_ref().map_or_else(Vec::new, |enclosure| c2f_geom::crossings(circuit, enclosure)),
        enclosure: enclosure.is_some(),
    })
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
