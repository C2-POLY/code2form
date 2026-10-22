//! Evaluator: runs a parsed .scad program and produces its CSG tree,
//! the lines it echoes and its warnings. [`node::csg`] prints the tree in
//! the format of the reference implementation's `-o x.csg` export.

mod color;
pub mod customizer;
mod dxf;
mod env;
mod functions;
mod interp;
mod math;
mod modules;
pub mod node;
pub mod program;
mod value;

pub use node::{Node, NodeKind};
pub use program::{LoadError, Program, app_resources, default_library_paths, read_source};

/// The result of running a program.
pub struct Output {
    /// Top-level nodes; just the `!` node when there is one. Empty when
    /// evaluation was aborted, as the reference then exports nothing.
    pub nodes: Vec<Node>,
    /// `ECHO: ...` lines, in order.
    pub echo: Vec<String>,
    pub warnings: Vec<String>,
    /// The error that stopped evaluation, if any.
    pub error: Option<String>,
    /// The pauses `pause()` asks of a 3D print, in the order it was called.
    pub pauses: Vec<Pause>,
}

/// A pause in a 3D print, made once the print is done up to height `z`
/// (in the model's coordinates, in millimeters) and before the layer that
/// covers it: to drop magnets, nuts or weights into closed cavities.
#[derive(Clone, Debug, PartialEq)]
pub struct Pause {
    pub z: f64,
    /// Shown on the printer while it waits; may be empty.
    pub message: String,
    /// Where the call is written, when it is in the main file.
    pub source: Option<PauseSource>,
}

/// Where a `pause()` call is written in the main file, so that an editor
/// can move or remove it.
#[derive(Clone, Debug, PartialEq)]
pub struct PauseSource {
    /// Byte offset of the call's name.
    pub call: usize,
    /// The bytes of the height, when it is written as a plain number.
    pub number: Option<std::ops::Range<usize>>,
}

impl Output {
    /// The CSG file. The reference ends it with a blank line, even when
    /// empty.
    pub fn csg(&self) -> String {
        node::csg(&self.nodes) + "\n"
    }
}

/// Reserved, not committed: deep recursion in scripts needs a deep stack.
pub(crate) const EVAL_STACK: usize = 1 << 30;

/// Evaluates the program on a thread with a large stack.
pub fn evaluate(program: &Program) -> Output {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("c2f-eval".into())
            .stack_size(EVAL_STACK)
            .spawn_scoped(scope, || evaluate_here(program))
            .expect("spawning the evaluator thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn evaluate_here(program: &Program) -> Output {
    let mut interp = interp::Interp::new(program);
    let result = interp.run();
    let mut warnings = program.warnings.clone();
    warnings.append(&mut interp.warnings);
    let (nodes, error) = match result {
        Ok(nodes) => (interp.root.take().map_or(nodes, |root| vec![root]), None),
        Err(interp::Abort(message)) => (Vec::new(), Some(message)),
    };
    Output { nodes, echo: std::mem::take(&mut interp.echo), warnings, error, pauses: std::mem::take(&mut interp.pauses) }
}
