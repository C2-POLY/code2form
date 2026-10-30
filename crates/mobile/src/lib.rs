//! Code2Form for the iPad app: the engine, called from Swift. UniFFI
//! writes the Swift side; `scripts/ios-xcframework.sh` builds this crate
//! for iOS and wraps it in `Code2FormCore.xcframework`.
//!
//! An [`Engine`] lists a program's customizer parameters, renders it into
//! a mesh to draw and exports it as STL or 3MF. It reads the fonts and
//! libraries the app ships, from the folders it is given: an app on iOS
//! has neither a home library folder nor the macOS app's `Resources`.
//!
//! Each engine does its work on a thread of its own, one request at a
//! time, as the studio's worker does: the geometry recurses once per
//! nesting level of the model, deeper than the 512 KiB stack of the
//! threads Swift calls from.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use c2f_eval::customizer::{HIDDEN_GROUP, parse_typed};

uniffi::setup_scaffolding!();

/// The stack of an engine's thread, reserved rather than committed: about
/// 12 KiB a nesting level, so some twenty thousand levels. The evaluator
/// and the parser run on big-stack threads of their own.
const STACK: usize = 1 << 28;

/// The engine of one document. A render makes the one before it stale,
/// so give each open document its own.
#[derive(uniffi::Object)]
pub struct Engine {
    jobs: mpsc::Sender<Job>,
    /// The generation of the latest render; an older one is stale.
    latest: Arc<AtomicU64>,
}

type Job = Box<dyn FnOnce(&[PathBuf]) + Send>;

/// A program to render or export: its text, the path of its file (whose
/// folder `include`, `use` and `import()` read from) and the values given
/// to its parameters.
#[derive(uniffi::Record)]
pub struct Request {
    pub source: String,
    pub path: String,
    pub parameters: Vec<ParameterValue>,
}

/// A parameter's value, written as in the program: `12`, `true`,
/// `"text"`, `[1, 2, 3]`.
#[derive(uniffi::Record)]
pub struct ParameterValue {
    pub name: String,
    pub value: String,
}

/// A customizer parameter, as the studio's panel shows it.
#[derive(uniffi::Record)]
pub struct Parameter {
    pub name: String,
    pub group: String,
    /// The `//` comment on the line above.
    pub description: Option<String>,
    /// The value written in the file, as written.
    pub value: String,
    pub widget: Widget,
}

/// How a parameter is edited.
#[derive(uniffi::Enum)]
pub enum Widget {
    Checkbox,
    Slider { min: f64, max: f64, step: f64 },
    Spinbox { step: f64 },
    Dropdown { options: Vec<Choice> },
    /// One to four numbers.
    Vector { step: f64 },
    Text { max_length: Option<u32> },
}

/// One option of a dropdown: its value, written as in the program, and
/// its label.
#[derive(uniffi::Record)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

/// What a render gives: the mesh to draw, the model's size, and what the
/// program printed, warned about or stopped on.
#[derive(uniffi::Record)]
pub struct Rendering {
    pub mesh: Mesh,
    pub is_2d: bool,
    /// `None` when the model is empty.
    pub bounds: Option<Bounds>,
    pub triangles: u64,
    pub vertices: u64,
    /// The volume in mm³, or the area in mm² of a 2D model.
    pub volume: f64,
    pub echo: Vec<String>,
    /// The program's warnings, and the values given to parameters that no
    /// longer apply (the program changed since), which are left out.
    pub warnings: Vec<String>,
    /// What stopped the program; the mesh is then empty.
    pub error: Option<Problem>,
}

/// Flat-shaded triangles, three vertices each, as little-endian `f32`s
/// ready for a GPU buffer: x, y, z per position and normal, r, g, b, a
/// per color.
#[derive(uniffi::Record)]
pub struct Mesh {
    pub positions: Vec<u8>,
    pub normals: Vec<u8>,
    pub colors: Vec<u8>,
}

#[derive(uniffi::Record)]
pub struct Bounds {
    pub min: Vec<f64>,
    pub max: Vec<f64>,
}

/// An error in a program. A syntax error or a missing file has its file,
/// line and column (from 1); an error met while running (an `assert`, a
/// recursion) has its message only.
#[derive(Debug, uniffi::Record)]
pub struct Problem {
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Debug, uniffi::Error)]
pub enum EngineError {
    /// A newer render, or [`Engine::cancel`], made this one stale.
    Cancelled,
    /// The program does not load, or stops while it runs.
    Program { problem: Problem },
    /// Nothing to export, or the file cannot be written.
    Export { message: String },
    /// The engine failed: its thread could not start, or it panicked.
    Internal { message: String },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Cancelled => write!(f, "cancelled by a newer render"),
            EngineError::Program { problem } => match (&problem.file, problem.line) {
                (Some(file), Some(line)) => write!(f, "{file}:{line}: {}", problem.message),
                _ => write!(f, "{}", problem.message),
            },
            EngineError::Export { message } => write!(f, "{message}"),
            EngineError::Internal { message } => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

#[derive(uniffi::Enum)]
pub enum ExportFormat {
    /// Binary STL.
    Stl,
    /// 3MF, with the print's pauses for PrusaSlicer.
    ThreeMf,
}

#[uniffi::export]
impl Engine {
    /// An engine that finds fonts in `fonts` (before the system's) and
    /// libraries in `libraries`, in that order. The fonts are shared by
    /// every engine: the last ones given win.
    #[uniffi::constructor]
    pub fn new(fonts: Vec<String>, libraries: Vec<String>) -> Result<Arc<Self>, EngineError> {
        c2f_geom::set_font_folders(fonts.into_iter().map(PathBuf::from).collect());
        let libraries: Vec<PathBuf> = libraries.into_iter().map(PathBuf::from).collect();
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("c2f-engine".into())
            .stack_size(STACK)
            // Ends once the engine is dropped and its queue is done.
            .spawn(move || queue.into_iter().for_each(|job| job(&libraries)))
            .map_err(|err| EngineError::Internal { message: format!("cannot start the engine's thread: {err}") })?;
        Ok(Arc::new(Engine { jobs, latest: Arc::new(AtomicU64::new(0)) }))
    }

    /// The program's customizer parameters, without the hidden ones.
    pub fn parameters(&self, source: String, path: String) -> Result<Vec<Parameter>, EngineError> {
        self.run(move |libraries| parameters(libraries, &source, &path))?
    }

    /// Renders the program. Blocks until done: call it off the main
    /// thread. A render asked for meanwhile, or [`Engine::cancel`], makes
    /// it give up with [`EngineError::Cancelled`] at its next step (before
    /// it starts, once loaded, evaluated, then built).
    pub fn render(&self, request: Request) -> Result<Rendering, EngineError> {
        let generation = self.latest.fetch_add(1, Ordering::AcqRel) + 1;
        let latest = Arc::clone(&self.latest);
        let current = move || match latest.load(Ordering::Acquire) == generation {
            true => Ok(()),
            false => Err(EngineError::Cancelled),
        };
        match self.run(move |libraries| render(libraries, &request, current)) {
            Ok(result) => result,
            // What a panic leaves is shown as the program's error.
            Err(EngineError::Internal { message }) => {
                let problem = Problem { message: format!("internal error: {message}"), file: None, line: None, column: None };
                Ok(Rendering::failed(problem, Vec::new(), Vec::new()))
            }
            Err(err) => Err(err),
        }
    }

    /// Makes the render in progress give up at its next step.
    pub fn cancel(&self) {
        self.latest.fetch_add(1, Ordering::AcqRel);
    }

    /// Writes the model to `output` in `format`; returns the bytes written.
    /// It renders afresh, after the requests before it, and no render
    /// cancels it. The file appears whole or not at all.
    pub fn export(&self, request: Request, format: ExportFormat, output: String) -> Result<u64, EngineError> {
        self.run(move |libraries| export(libraries, &request, format, &output))?
    }
}

impl Engine {
    /// Runs `work` on the engine's thread and waits for it; a panic comes
    /// back as [`EngineError::Internal`].
    fn run<T: Send + 'static>(&self, work: impl FnOnce(&[PathBuf]) -> T + Send + 'static) -> Result<T, EngineError> {
        let (reply, answer) = mpsc::channel();
        let job: Job = Box::new(move |libraries| {
            let result = catch_unwind(AssertUnwindSafe(|| work(libraries)))
                .map_err(|panic| EngineError::Internal { message: panic_message(&*panic) });
            let _ = reply.send(result);
        });
        let stopped = || EngineError::Internal { message: "the engine's thread has stopped".into() };
        self.jobs.send(job).map_err(|_| stopped())?;
        answer.recv().map_err(|_| stopped())?
    }
}

fn parameters(libraries: &[PathBuf], source: &str, path: &str) -> Result<Vec<Parameter>, EngineError> {
    let program = c2f_eval::Program::load(source, Path::new(path), libraries)
        .map_err(|err| EngineError::Program { problem: load_problem(&err) })?;
    Ok(program
        .parameters(source)
        .into_iter()
        .filter(|p| p.group != HIDDEN_GROUP)
        .map(|p| Parameter {
            name: p.name,
            group: p.group,
            description: p.description,
            value: p.default.to_source(),
            widget: widget(p.widget),
        })
        .collect())
}

fn render(
    libraries: &[PathBuf],
    request: &Request,
    current: impl Fn() -> Result<(), EngineError>,
) -> Result<Rendering, EngineError> {
    // A render asked for after this one, while it waited, replaces it.
    current()?;
    let loaded = load(libraries, request);
    current()?;
    let (program, mut warnings) = match loaded {
        Ok(loaded) => loaded,
        Err(problem) => return Ok(Rendering::failed(problem, Vec::new(), Vec::new())),
    };
    let output = c2f_eval::evaluate(&program);
    current()?;
    warnings.extend(output.warnings);
    if let Some(message) = output.error {
        let problem = Problem { message, file: None, line: None, column: None };
        return Ok(Rendering::failed(problem, output.echo, warnings));
    }
    let rendered = build(&program, &output.nodes, output.pauses);
    current()?;
    warnings.extend(rendered.warnings.iter().cloned());
    let floats = |values: Vec<[f32; 3]>| bytes(values.into_iter().flatten());
    Ok(Rendering {
        bounds: rendered.bounds().map(|(min, max)| Bounds { min: min.to_vec(), max: max.to_vec() }),
        is_2d: rendered.is_2d,
        triangles: rendered.stats.triangles as u64,
        vertices: rendered.stats.vertices as u64,
        volume: rendered.stats.volume,
        mesh: Mesh {
            positions: floats(rendered.solid.positions),
            normals: floats(rendered.solid.normals),
            colors: bytes(rendered.solid.colors.into_iter().flatten()),
        },
        echo: output.echo,
        warnings,
        error: None,
    })
}

fn export(libraries: &[PathBuf], request: &Request, format: ExportFormat, output: &str) -> Result<u64, EngineError> {
    let (program, _) = load(libraries, request).map_err(|problem| EngineError::Program { problem })?;
    let evaluated = c2f_eval::evaluate(&program);
    if let Some(message) = evaluated.error {
        return Err(EngineError::Program { problem: Problem { message, file: None, line: None, column: None } });
    }
    let rendered = build(&program, &evaluated.nodes, evaluated.pauses);
    let (bytes, name) = match format {
        ExportFormat::Stl => (rendered.stl_binary(), "STL"),
        ExportFormat::ThreeMf => (rendered.three_mf(), "3MF"),
    };
    let bytes = bytes.ok_or_else(|| EngineError::Export { message: format!("the model is empty or 2D; no {name} written") })?;
    // Written beside it, then renamed: never a half-written file.
    let target = Path::new(output);
    let file_name = target.file_name().map_or_else(|| "export".into(), |n| n.to_string_lossy().into_owned());
    let partial = target.with_file_name(format!(".{file_name}.partial"));
    let failed = |err: std::io::Error| EngineError::Export { message: format!("cannot write {output}: {err}") };
    std::fs::write(&partial, &bytes).map_err(failed)?;
    std::fs::rename(&partial, target).map_err(|err| {
        let _ = std::fs::remove_file(&partial);
        failed(err)
    })?;
    Ok(bytes.len() as u64)
}

/// The program, with the request's values given to its parameters. A
/// value that no longer applies (the parameter is gone, or its default has
/// another type since the value was chosen) is left out with a warning,
/// as the studio's customizer does.
fn load(libraries: &[PathBuf], request: &Request) -> Result<(c2f_eval::Program, Vec<String>), Problem> {
    let mut program =
        c2f_eval::Program::load(&request.source, Path::new(&request.path), libraries).map_err(|err| load_problem(&err))?;
    let mut warnings = Vec::new();
    if request.parameters.is_empty() {
        return Ok((program, warnings));
    }
    let parameters = program.parameters(&request.source);
    for ParameterValue { name, value } in &request.parameters {
        let Some(parameter) = parameters.iter().find(|p| p.name == *name) else {
            warnings.push(format!("Ignoring the value of `{name}`: the program has no such parameter"));
            continue;
        };
        match parse_typed(&parameter.default, value) {
            Ok(Some(literal)) => {
                program.set_parameter(name, &literal);
            }
            Ok(None) => warnings.push(format!(
                "Ignoring the value of `{name}`: {value} does not have the type of its default, {}",
                parameter.default.to_source()
            )),
            Err(why) => warnings.push(format!("Ignoring the value of `{name}`: {why}")),
        }
    }
    Ok((program, warnings))
}

impl Rendering {
    fn failed(problem: Problem, echo: Vec<String>, warnings: Vec<String>) -> Rendering {
        Rendering {
            mesh: Mesh { positions: Vec::new(), normals: Vec::new(), colors: Vec::new() },
            is_2d: false,
            bounds: None,
            triangles: 0,
            vertices: 0,
            volume: 0.0,
            echo,
            warnings,
            error: Some(problem),
        }
    }
}

/// The geometry of an evaluated program, reading data files next to its
/// main file, with its pauses.
fn build(program: &c2f_eval::Program, nodes: &[c2f_eval::Node], pauses: Vec<c2f_eval::Pause>) -> c2f_geom::Rendered {
    c2f_geom::render_in(nodes, program.files[0].path.parent().unwrap_or(Path::new(""))).with_pauses(pauses)
}

fn bytes(values: impl Iterator<Item = f32>) -> Vec<u8> {
    values.flat_map(f32::to_le_bytes).collect()
}

fn load_problem(err: &c2f_eval::LoadError) -> Problem {
    let src = &err.src;
    let offset = (err.diagnostic.span.start as usize).min(src.len());
    let (line, column) = c2f_lang::span::LineIndex::new(src).line_col(src, offset);
    Problem {
        message: err.diagnostic.message.clone(),
        file: Some(err.path.display().to_string()),
        line: Some(line as u32 + 1),
        column: Some(column as u32 + 1),
    }
}

fn widget(widget: c2f_eval::customizer::Widget) -> Widget {
    use c2f_eval::customizer::Widget as W;
    match widget {
        W::Checkbox => Widget::Checkbox,
        W::Slider { min, max, step } => Widget::Slider { min, max, step },
        W::Spinbox { step } => Widget::Spinbox { step },
        W::Dropdown { options } => Widget::Dropdown {
            options: options.into_iter().map(|(value, label)| Choice { value: value.to_source(), label }).collect(),
        },
        W::Vector { step } => Widget::Vector { step },
        W::Text { max_length } => Widget::Text { max_length: max_length.map(|n| n as u32) },
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "the engine panicked".to_string()
    }
}
