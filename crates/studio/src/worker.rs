//! The render worker: parse, evaluate and build the geometry on a thread
//! with a large stack, so the UI never waits on a script.
//!
//! The geometry (`c2f_geom::Rendered`) stays on the worker thread; the UI
//! gets plain triangles back, and asks the worker for exports of the model
//! it shows: STL or 3MF for a solid, SVG or DXF for a 2D model.
//!
//! A circuit (`.c2fc`) is read first, and the circuit goes back at once so
//! that its wires show; then its parts render, several at a time, into one
//! scene.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;
use c2f_eval::customizer::{Literal, Parameter};
use c2f_geom::{Model, RenderMesh, Rendered, Stats};

use crate::circuit::{self, PartCache, PartRender};
use crate::console::{Level, Line};

/// Reserved, not committed: deeply recursive scripts need a deep stack.
pub const STACK: usize = 1 << 30;

/// What the viewer draws, in plain data.
pub struct Scene {
    pub solid: RenderMesh,
    pub highlights: RenderMesh,
    pub backgrounds: RenderMesh,
    pub is_2d: bool,
    pub stats: Stats,
    /// The formats the model exports to: none when it is empty.
    pub formats: &'static [Format],
    /// The print's pauses, lowest first, within the solid's height.
    pub pauses: Vec<c2f_eval::Pause>,
    /// The text rendered, where the pauses' sources point.
    pub source: String,
    /// For a circuit, the vertices of each of its parts in `solid`, in the
    /// order of the circuit's parts.
    pub parts: Vec<std::ops::Range<usize>>,
    /// For a circuit, the vertices of its enclosure in `solid`, after the
    /// parts': see-through.
    pub enclosure: std::ops::Range<usize>,
    /// What a click in the view picks, for a design (not a circuit).
    pub picking: Option<Arc<crate::select::Picking>>,
}

impl Scene {
    /// The corners of the box around everything drawn.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        [&self.solid, &self.highlights, &self.backgrounds].into_iter().filter_map(RenderMesh::bounds).reduce(
            |(lo, hi), (l, h)| {
                (
                    [lo[0].min(l[0]), lo[1].min(l[1]), lo[2].min(l[2])],
                    [hi[0].max(h[0]), hi[1].max(h[1]), hi[2].max(h[2])],
                )
            },
        )
    }

    pub fn is_empty(&self) -> bool {
        self.solid.is_empty() && self.highlights.is_empty() && self.backgrounds.is_empty()
    }
}

pub struct RenderOutcome {
    pub generation: u64,
    pub elapsed: Duration,
    /// Echo lines, warnings and errors, in order.
    pub console: Vec<Line>,
    /// The new scene, or `Err` with the line of a syntax error in the main
    /// file (0-based) when the render failed.
    pub result: Result<Scene, Option<usize>>,
    /// The customizer parameters of the file; `None` when it did not parse.
    pub parameters: Option<Vec<Parameter>>,
    /// The line (0-based) of the first badly written electronics annotation
    /// of the file, marked like an error while the model still renders.
    pub annotation_error: Option<usize>,
    /// For a design, the files it was rendered from besides its own, to
    /// render again when one changes: the circuit its `// @circuit` names
    /// and that circuit's parts.
    pub watch: Vec<PathBuf>,
}

pub enum Reply {
    Rendered(Box<RenderOutcome>),
    /// A circuit as read, before its parts render: the reply to the same
    /// generation follows with them.
    Circuit { generation: u64, circuit: Arc<c2f_circuit::Circuit> },
    Exported { target: PathBuf, result: Result<String, String> },
}

enum Request {
    Render { generation: u64, source: String, path: PathBuf, values: Vec<(String, Literal)> },
    Export { target: PathBuf, format: Format, binary_stl: bool },
    Libraries(Vec<PathBuf>),
}

/// The file formats a model exports to: STL and 3MF for a solid, SVG and
/// DXF for a 2D model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Stl,
    ThreeMf,
    Svg,
    Dxf,
}

impl Format {
    pub const SOLID: &'static [Format] = &[Format::Stl, Format::ThreeMf];
    pub const FLAT: &'static [Format] = &[Format::Svg, Format::Dxf];

    pub fn name(self) -> &'static str {
        match self {
            Format::Stl => "STL",
            Format::ThreeMf => "3MF",
            Format::Svg => "SVG",
            Format::Dxf => "DXF",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Stl => "stl",
            Format::ThreeMf => "3mf",
            Format::Svg => "svg",
            Format::Dxf => "dxf",
        }
    }

    /// The formats `model` exports to.
    fn of(model: &Model, stats: &Stats) -> &'static [Format] {
        match model {
            Model::Solid(_) if stats.triangles > 0 => Format::SOLID,
            Model::Flat(_) if stats.vertices > 0 => Format::FLAT,
            _ => &[],
        }
    }
}

pub struct Worker {
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    /// The newest render requested; the worker drops older ones.
    latest: Arc<AtomicU64>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (requests, request_rx) = mpsc::channel();
        let (reply_tx, replies) = mpsc::channel();
        let latest = Arc::new(AtomicU64::new(0));
        let shared = latest.clone();
        std::thread::Builder::new()
            .name("c2f-render".into())
            .stack_size(STACK)
            .spawn(move || run(&request_rx, &reply_tx, &shared, &ctx))
            .expect("spawning the render thread");
        Worker { requests, replies, latest }
    }

    /// Queues a render of `source`, located at `path` for its imports, with
    /// customizer `values` for its parameters, and returns its generation.
    pub fn render(&self, source: String, path: PathBuf, values: Vec<(String, Literal)>) -> u64 {
        let generation = self.latest.load(Ordering::Acquire) + 1;
        self.latest.store(generation, Ordering::Release);
        let _ = self.requests.send(Request::Render { generation, source, path, values });
        generation
    }

    /// Searches `folders` for `include` and `use` before the default
    /// library folders, from the next render on.
    pub fn set_libraries(&self, folders: Vec<PathBuf>) {
        let _ = self.requests.send(Request::Libraries(folders));
    }

    /// Writes the last good model to `target` in `format`; an STL in
    /// binary with `binary_stl`, otherwise in ASCII.
    pub fn export(&self, target: PathBuf, format: Format, binary_stl: bool) {
        let _ = self.requests.send(Request::Export { target, format, binary_stl });
    }

    pub fn latest(&self) -> u64 {
        self.latest.load(Ordering::Acquire)
    }

    pub fn poll(&self) -> Vec<Reply> {
        self.replies.try_iter().collect()
    }
}

fn run(requests: &Receiver<Request>, replies: &Sender<Reply>, latest: &AtomicU64, ctx: &egui::Context) {
    let defaults = c2f_eval::default_library_paths();
    let mut library_paths = defaults.clone();
    // The geometry of the model on screen, for exports.
    let mut shown: Option<Rendered> = None;
    let mut parts = PartCache::default();
    while let Ok(first) = requests.recv() {
        // Only the newest render matters; exports are all honored.
        let mut render = None;
        let mut exports = Vec::new();
        for request in std::iter::once(first).chain(requests.try_iter()) {
            match request {
                Request::Render { generation, source, path, values } => render = Some((generation, source, path, values)),
                Request::Export { target, format, binary_stl } => exports.push((target, format, binary_stl)),
                Request::Libraries(folders) => {
                    library_paths = folders.into_iter().chain(defaults.iter().cloned()).collect()
                }
            }
        }
        if let Some((generation, source, path, values)) = render {
            if generation == latest.load(Ordering::Acquire) && circuit::is_circuit(&path) {
                let current = || generation == latest.load(Ordering::Acquire);
                let read = |circuit: Arc<c2f_circuit::Circuit>| {
                    if current() {
                        let _ = replies.send(Reply::Circuit { generation, circuit });
                        ctx.request_repaint();
                    }
                };
                let outcome = render_circuit(generation, &source, &path, &library_paths, &mut parts, read);
                if current() {
                    // A circuit is not exported.
                    shown = None;
                    if replies.send(Reply::Rendered(Box::new(outcome))).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            } else if generation == latest.load(Ordering::Acquire) {
                let (outcome, rendered) = render_source(generation, &source, &path, &values, &library_paths);
                // A newer request arrived meanwhile: this result is stale.
                if generation == latest.load(Ordering::Acquire) {
                    if rendered.is_some() {
                        shown = rendered;
                    }
                    if replies.send(Reply::Rendered(Box::new(outcome))).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        }
        for (target, format, binary_stl) in exports {
            let result = export(shown.as_ref(), &target, format, binary_stl);
            if replies.send(Reply::Exported { target, result }).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

fn render_source(
    generation: u64,
    source: &str,
    path: &Path,
    values: &[(String, Literal)],
    library_paths: &[PathBuf],
) -> (RenderOutcome, Option<Rendered>) {
    let start = Instant::now();
    let mut console = Vec::new();
    let mut parameters = None;
    let mut annotation_error = None;
    let result = catch_unwind(AssertUnwindSafe(|| {
        pipeline(source, path, values, library_paths, &mut console, &mut parameters, &mut annotation_error)
    }))
    .unwrap_or_else(
        |panic| {
            console.push(Line::new(Level::Error, format!("internal error: {}", panic_message(&*panic))));
            Err(None)
        },
    );
    let (result, rendered) = match result {
        Ok((mut rendered, origins)) => {
            let picking = crate::select::Picking::new(
                origins,
                &rendered.solid,
                &rendered.highlights,
                &rendered.backgrounds,
                &rendered.cutters,
            );
            let scene = Scene {
                solid: std::mem::take(&mut rendered.solid),
                highlights: std::mem::take(&mut rendered.highlights),
                backgrounds: std::mem::take(&mut rendered.backgrounds),
                is_2d: rendered.is_2d,
                stats: rendered.stats,
                formats: Format::of(&rendered.model, &rendered.stats),
                // Kept in the rendering too: the 3MF export writes them.
                pauses: rendered.pauses.clone(),
                source: source.to_string(),
                parts: Vec::new(),
                enclosure: 0..0,
                picking: Some(Arc::new(picking)),
            };
            (Ok(scene), Some(rendered))
        }
        Err(line) => (Err(line), None),
    };
    let watch = circuit_files(source, path, library_paths);
    (RenderOutcome { generation, elapsed: start.elapsed(), console, result, parameters, annotation_error, watch }, rendered)
}

/// The circuit a design names in `// @circuit`, and its part files.
fn circuit_files(source: &str, path: &Path, library_paths: &[PathBuf]) -> Vec<PathBuf> {
    let (annotations, _) = c2f_lang::annotations::read(source);
    let Some(link) = annotations.circuit else { return Vec::new() };
    let circuit_path = path.parent().unwrap_or(Path::new("")).join(&link.path);
    let mut files = vec![circuit_path.clone()];
    if let Ok(circuit) = c2f_circuit::load(&circuit_path, library_paths) {
        files.extend(circuit.parts.iter().map(|part| part.path.clone()));
    }
    files
}

/// Reads the circuit `source` at `path`, hands it to `read`, then renders
/// its parts.
fn render_circuit(
    generation: u64,
    source: &str,
    path: &Path,
    library_paths: &[PathBuf],
    parts: &mut PartCache,
    read: impl FnOnce(Arc<c2f_circuit::Circuit>),
) -> RenderOutcome {
    let start = Instant::now();
    let mut console = Vec::new();
    let mut annotation_error = None;
    let result = catch_unwind(AssertUnwindSafe(|| match c2f_circuit::parse(source, path, library_paths) {
        Ok(found) => {
            let found = Arc::new(found);
            read(found.clone());
            // The wiring checks: the parts render even when they fail.
            let findings = c2f_circuit::check::check(&found);
            console.extend(findings.iter().map(|finding| {
                let level = match finding.severity {
                    c2f_circuit::check::Severity::Error => Level::Error,
                    c2f_circuit::check::Severity::Warning => Level::Warning,
                };
                Line::new(level, finding.render(source, path))
            }));
            let lines = c2f_lang::span::LineIndex::new(source);
            annotation_error = findings
                .iter()
                .find(|f| f.severity == c2f_circuit::check::Severity::Error)
                .map(|f| lines.line(f.span.start as usize));
            let rendered = parts.update(&found, library_paths, render_part);
            let enclosure = parts.enclosure(&found, |text, enclosure_path| {
                render_enclosure(text, enclosure_path, path, &found, library_paths)
            });
            let collisions = parts.collisions(&found, &rendered, enclosure.as_ref());
            Ok(circuit::scene(&found, &rendered, enclosure.as_deref(), &collisions, source, &mut console))
        }
        Err(errors) => {
            console.extend(errors.iter().map(|err| Line::new(Level::Error, err.render())));
            Err(circuit::error_line(&errors, path, source))
        }
    }))
    .unwrap_or_else(|panic| {
        console.push(Line::new(Level::Error, format!("internal error: {}", panic_message(&*panic))));
        // Whatever the cache held when it broke is not to be trusted.
        *parts = PartCache::default();
        Err(None)
    });
    let watch = Vec::new();
    RenderOutcome { generation, elapsed: start.elapsed(), console, result, parameters: None, annotation_error, watch }
}

/// Renders a part of a circuit, its parameters set in `source`, as a
/// design of its own.
pub fn render_part(source: &str, path: &Path, library_paths: &[PathBuf]) -> PartRender {
    let mut console = Vec::new();
    let (mut parameters, mut annotation_error) = (None, None);
    let result = catch_unwind(AssertUnwindSafe(|| {
        pipeline(source, path, &[], library_paths, &mut console, &mut parameters, &mut annotation_error)
    }))
    .unwrap_or_else(|panic| {
        console.push(Line::new(Level::Error, format!("internal error in {}: {}", path.display(), panic_message(&*panic))));
        Err(None)
    });
    let console = circuit::label_lines(console, path);
    match result {
        Ok((rendered, _)) => PartRender {
            triangles: rendered.stats.triangles,
            solid: rendered.solid,
            highlights: rendered.highlights,
            backgrounds: rendered.backgrounds,
            console,
            model: rendered.model,
        },
        Err(_) => PartRender::failed(console),
    }
}

/// Renders the enclosure `source` at `path` as a design of its own. When
/// it names the circuit at `circuit_path` in `// @circuit`, it gets
/// `circuit`, as read from the editor, rather than the file on disk.
pub fn render_enclosure(
    source: &str,
    path: &Path,
    circuit_path: &Path,
    circuit: &c2f_circuit::Circuit,
    library_paths: &[PathBuf],
) -> PartRender {
    let mut console = Vec::new();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut program = match c2f_eval::Program::load(source, path, library_paths) {
            Ok(program) => program,
            Err(err) => {
                console.push(Line::new(Level::Error, err.render()));
                return None;
            }
        };
        let same = |a: &Path| {
            let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            canonical(a) == canonical(circuit_path)
        };
        if program.circuit_path.as_deref().is_some_and(same) {
            program.circuit = Some(circuit.clone());
            program.warnings.retain(|w| !w.starts_with("Can't read circuit"));
        }
        let output = c2f_eval::evaluate(&program);
        console.extend(output.echo.into_iter().map(|text| Line::new(Level::Echo, text)));
        console.extend(output.warnings.into_iter().map(|text| Line::new(Level::Warning, text)));
        if let Some(error) = output.error {
            console.push(Line::new(Level::Error, error));
            return None;
        }
        // Its `%` copies of the parts are the circuit's, drawn already:
        // building them would only cost time.
        let nodes: Vec<_> = output.nodes.into_iter().filter(|node| !node.background).collect();
        Some(c2f_geom::render_in(&nodes, path.parent().unwrap_or(Path::new(""))))
    }))
    .unwrap_or_else(|panic| {
        console.push(Line::new(Level::Error, format!("internal error in {}: {}", path.display(), panic_message(&*panic))));
        None
    });
    let console = circuit::label_lines(console, path);
    match result {
        // Its `%` parts are the circuit's, already shown.
        Some(rendered) => PartRender {
            triangles: rendered.stats.triangles,
            solid: rendered.solid,
            highlights: rendered.highlights,
            backgrounds: RenderMesh::default(),
            console,
            model: rendered.model,
        },
        None => PartRender::failed(console),
    }
}

fn pipeline(
    source: &str,
    path: &Path,
    values: &[(String, Literal)],
    library_paths: &[PathBuf],
    console: &mut Vec<Line>,
    parameters: &mut Option<Vec<Parameter>>,
    annotation_error: &mut Option<usize>,
) -> Result<(Rendered, Vec<c2f_eval::Origin>), Option<usize>> {
    let mut program = match c2f_eval::Program::load(source, path, library_paths) {
        Ok(program) => program,
        Err(err) => {
            console.push(Line::new(Level::Error, err.render()));
            let in_main_file = err.path == path && err.src == source;
            let offset = (err.diagnostic.span.start as usize).min(source.len());
            let line = in_main_file.then(|| source.as_bytes()[..offset].iter().filter(|&&b| b == b'\n').count());
            return Err(line);
        }
    };
    // A badly written annotation leaves the model as it is.
    let (_, errors) = c2f_lang::annotations::read(source);
    console.extend(errors.iter().map(|err| Line::new(Level::Error, err.render(source, &path.display().to_string()))));
    let lines = c2f_lang::span::LineIndex::new(source);
    *annotation_error = errors.first().map(|err| lines.line(err.span.start as usize));
    let found = program.parameters(source);
    // A value set before the file changed may no longer apply; it must
    // not reach whatever assignment now has that name.
    for (name, value) in values.iter().filter(|(name, value)| crate::customizer::applies(&found, name, value)) {
        program.set_parameter(name, value);
    }
    *parameters = Some(found);
    let output = c2f_eval::evaluate(&program);
    console.extend(output.echo.into_iter().map(|text| Line::new(Level::Echo, text)));
    console.extend(output.warnings.into_iter().map(|text| Line::new(Level::Warning, text)));
    if let Some(error) = output.error {
        console.push(Line::new(Level::Error, error));
        return Err(None);
    }
    // Data files (import(), surface()) are named relative to the main file.
    let rendered = c2f_geom::render_in(&output.nodes, program.files[0].path.parent().unwrap_or(Path::new("")))
        .with_pauses(output.pauses);
    console.extend(rendered.warnings.iter().map(|text| Line::new(Level::Warning, text.clone())));
    Ok((rendered, output.origins))
}

fn export(shown: Option<&Rendered>, target: &Path, format: Format, binary_stl: bool) -> Result<String, String> {
    let rendered = shown.ok_or("there is no model to export yet")?;
    let bytes = catch_unwind(AssertUnwindSafe(|| match format {
        Format::Stl if binary_stl => rendered.stl_binary(),
        Format::Stl => rendered.stl().map(String::into_bytes),
        Format::ThreeMf => rendered.three_mf(),
        Format::Svg => rendered.svg().map(String::into_bytes),
        Format::Dxf => rendered.dxf().map(String::into_bytes),
    }))
    .map_err(|panic| format!("internal error: {}", panic_message(&*panic)))?
    .ok_or_else(|| {
        let kind = if Format::SOLID.contains(&format) { "empty or 2D" } else { "empty or 3D" };
        format!("the model is {kind}, so there is nothing to export as {}", format.name())
    })?;
    std::fs::write(target, bytes).map_err(|err| format!("cannot write {}: {err}", target.display()))?;
    let (count, what) = if rendered.is_2d { (rendered.stats.vertices, "vertices") } else { (rendered.stats.triangles, "triangles") };
    Ok(format!("Exported {} {what} to {}", group_digits(count), target.display()))
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "the renderer panicked".to_string()
    }
}

/// `1234567` as `1,234,567`.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Waits for the reply to `generation`, collecting all replies.
    fn wait_for(worker: &Worker, generation: u64) -> Vec<Reply> {
        let start = Instant::now();
        let mut replies = Vec::new();
        while start.elapsed() < Duration::from_secs(30) {
            replies.extend(worker.poll());
            if replies.iter().any(|r| matches!(r, Reply::Rendered(o) if o.generation == generation)) {
                return replies;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no reply for generation {generation}");
    }

    fn path() -> PathBuf {
        std::env::temp_dir().join("code2form-test.scad")
    }

    #[test]
    fn only_the_newest_render_is_reported_and_kept_for_export() {
        let worker = Worker::spawn(egui::Context::default());
        worker.render("cube(1);".into(), path(), Vec::new());
        worker.render("cube(2);".into(), path(), Vec::new());
        let last = worker.render("echo(\"hi\"); sphere(3);".into(), path(), Vec::new());
        let replies = wait_for(&worker, last);
        let generations: Vec<u64> = replies
            .iter()
            .filter_map(|r| match r {
                Reply::Rendered(o) => Some(o.generation),
                Reply::Exported { .. } | Reply::Circuit { .. } => None,
            })
            .collect();
        assert!(generations.windows(2).all(|w| w[0] < w[1]), "{generations:?}");
        let Some(Reply::Rendered(outcome)) = replies.into_iter().last() else { panic!("no render") };
        assert_eq!(outcome.generation, last);
        assert!(outcome.console.iter().any(|l| l.level == Level::Echo && l.text.contains("hi")));
        let Ok(scene) = outcome.result else { panic!("the render failed") };
        assert!(!scene.is_2d && scene.formats == Format::SOLID && scene.stats.triangles > 0);
        assert_eq!(scene.solid.positions.len(), scene.solid.normals.len());

        for (format, magic) in [(Format::Stl, &b"solid"[..]), (Format::ThreeMf, &b"PK\x03\x04"[..])] {
            let (result, bytes) = export_with(&worker, format, false);
            assert!(result.is_ok(), "{result:?}");
            assert!(bytes.starts_with(magic), "{}", format.name());
        }
        let (result, bytes) = export_with(&worker, Format::Stl, true);
        assert!(result.unwrap().starts_with(&format!("Exported {} triangles to ", scene.stats.triangles)));
        assert!(!bytes.starts_with(b"solid"), "binary");
        assert_eq!(bytes.len(), 84 + 50 * scene.stats.triangles);
    }

    /// Exports the shown model in `format`: the reply, and the file written.
    /// Each call gets its own file, since tests run in parallel in one process.
    fn export_with(worker: &Worker, format: Format, binary_stl: bool) -> (Result<String, String>, Vec<u8>) {
        static CALLS: AtomicU64 = AtomicU64::new(0);
        let call = CALLS.fetch_add(1, Ordering::Relaxed);
        let name = format!("code2form-test-{}-{call}.{}", std::process::id(), format.extension());
        let target = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&target);
        worker.export(target.clone(), format, binary_stl);
        let start = Instant::now();
        let result = loop {
            if let Some(Reply::Exported { result, .. }) = worker.poll().into_iter().next() {
                break result;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "no export reply");
            std::thread::sleep(Duration::from_millis(5));
        };
        let bytes = std::fs::read(&target).unwrap_or_default();
        let _ = std::fs::remove_file(&target);
        (result, bytes)
    }

    #[test]
    fn flat_models_export_as_drawings() {
        let worker = Worker::spawn(egui::Context::default());
        let generation = worker.render("square([20, 10]);".into(), path(), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        let Ok(scene) = outcome.result else { panic!("the render failed") };
        assert!(scene.is_2d && scene.formats == Format::FLAT);

        for (format, magic) in [(Format::Svg, &b"<?xml"[..]), (Format::Dxf, &b"  0\nSECTION\n"[..])] {
            let (result, bytes) = export_with(&worker, format, false);
            let message = result.unwrap();
            assert!(message.starts_with("Exported 4 vertices to "), "{message}");
            assert!(bytes.starts_with(magic), "{}", format.name());
        }
        let (result, bytes) = export_with(&worker, Format::Stl, true);
        assert_eq!(result, Err("the model is empty or 2D, so there is nothing to export as STL".to_string()));
        assert!(bytes.is_empty());
    }

    #[test]
    fn syntax_errors_point_at_their_line() {
        let worker = Worker::spawn(egui::Context::default());
        let generation = worker.render("cube(1);\n\ncube(2;\n".into(), path(), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        assert!(matches!(outcome.result, Err(Some(2))));
        assert!(outcome.console.iter().any(|l| l.level == Level::Error));
    }

    #[test]
    fn annotation_errors_are_marked_but_the_model_renders() {
        let worker = Worker::spawn(egui::Context::default());
        let generation = worker.render("cube(1);\n// @pin A [0, 0]\n".into(), path(), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        assert!(outcome.result.is_ok());
        assert_eq!(outcome.annotation_error, Some(1));
        assert!(outcome.console.iter().any(|l| l.level == Level::Error && l.text.contains("position")));
    }

    #[test]
    fn customizer_values_reach_the_model() {
        let worker = Worker::spawn(egui::Context::default());
        let source = "size = 2; // [1:10]\ncube(size);\n";
        let values = vec![("size".to_string(), Literal::Number(5.0))];
        let generation = worker.render(source.into(), path(), values);
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        let parameters = outcome.parameters.expect("parameters");
        assert_eq!(parameters.len(), 1);
        assert_eq!((parameters[0].name.as_str(), &parameters[0].default), ("size", &Literal::Number(2.0)));
        let Ok(scene) = outcome.result else { panic!("the render failed") };
        assert_eq!(scene.bounds().map(|(_, hi)| hi), Some([5.0, 5.0, 5.0]));

        // Once `size` is no longer a parameter, the value is not applied.
        let values = vec![("size".to_string(), Literal::Number(5.0))];
        let generation = worker.render("size = 1 + 1;\ncube(size);\n".into(), path(), values);
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        let Ok(scene) = outcome.result else { panic!("the render failed") };
        assert_eq!(scene.bounds().map(|(_, hi)| hi), Some([2.0, 2.0, 2.0]));
    }

    #[test]
    fn a_circuit_comes_back_before_its_parts() {
        let dir = std::env::temp_dir().join(format!("code2form-worker-circuit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("part.c2f"), "// @pin A [0, 0, 1] up\necho(\"part\");\ncube(1);\n").unwrap();
        let source = "[parts.a]\npart = \"part.c2f\"\n\n[parts.b]\npart = \"part.c2f\"\nat = [5, 0, 0]\n";
        let worker = Worker::spawn(egui::Context::default());
        let generation = worker.render(source.into(), dir.join("test.c2fc"), Vec::new());
        let replies = wait_for(&worker, generation);
        let circuit = replies.iter().position(|r| matches!(r, Reply::Circuit { generation: g, .. } if *g == generation));
        let rendered = replies.iter().position(|r| matches!(r, Reply::Rendered(o) if o.generation == generation));
        assert!(circuit.unwrap() < rendered.unwrap());
        let Some(Reply::Rendered(outcome)) = replies.into_iter().last() else { panic!("no render") };
        // The part renders once, and says so once, with its file's name.
        let echoes: Vec<&str> = outcome.console.iter().filter(|l| l.level == Level::Echo).map(|l| l.text.as_str()).collect();
        assert_eq!(echoes, ["part.c2f: ECHO: \"part\""]);
        let scene = outcome.result.unwrap_or_else(|_| panic!("the circuit failed"));
        assert_eq!(scene.parts, [0..36, 36..72]);
        assert_eq!(scene.bounds(), Some(([0.0; 3], [6.0, 1.0, 1.0])));
        assert!(scene.formats.is_empty(), "a circuit is not exported");

        // An error in the circuit marks its line.
        let generation = worker.render("[parts.a]\npart = \"none.c2f\"\n".into(), dir.join("test.c2fc"), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else { panic!() };
        assert!(matches!(outcome.result, Err(Some(1))));
        assert!(outcome.console.iter().any(|l| l.level == Level::Error && l.text.contains("none.c2f")));

        // The wiring checks reach the console, and an error marks its wire.
        std::fs::write(dir.join("supply.c2f"), "// @pin OUT [0, 0, 1] up power_out 5V\ncube(1);\n").unwrap();
        std::fs::write(dir.join("chip.c2f"), "// @pin IN [0, 0, 1] up io 3.3V\ncube(1);\n").unwrap();
        let source = "[parts.s]\npart = \"supply.c2f\"\n[parts.c]\npart = \"chip.c2f\"\nat = [5, 0, 0]\n\n[[wires]]\nfrom = \"s.OUT\"\nto = \"c.IN\"\n";
        let generation = worker.render(source.into(), dir.join("test.c2fc"), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else { panic!() };
        assert!(outcome.result.is_ok(), "the parts still render");
        assert!(outcome.console.iter().any(|l| l.level == Level::Error && l.text.contains("c.IN")), "{:?}", outcome.console.iter().map(|l| &l.text).collect::<Vec<_>>());
        assert_eq!(outcome.annotation_error, Some(7));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_design_watches_the_circuit_it_names_and_its_parts() {
        let dir = std::env::temp_dir().join(format!("code2form-worker-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("part.c2f"), "// @pin A [0, 0, 1] up\ncube(1);\n").unwrap();
        std::fs::write(dir.join("test.c2fc"), "[parts.a]\npart = \"part.c2f\"\n").unwrap();
        let worker = Worker::spawn(egui::Context::default());
        let source = "// @circuit test.c2fc\ncube(5);\n";
        let generation = worker.render(source.into(), dir.join("box.c2f"), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else { panic!() };
        assert_eq!(outcome.watch, [dir.join("test.c2fc"), dir.join("part.c2f")]);
        let generation = worker.render("cube(5);".into(), dir.join("box.c2f"), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else { panic!() };
        assert!(outcome.watch.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn library_folders_are_searched_first() {
        let dir = std::env::temp_dir().join(format!("code2form-libraries-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c2f_test_part.scad"), "module part() cube(3);\n").unwrap();
        let source = "use <c2f_test_part.scad>\npart();\n";
        let worker = Worker::spawn(egui::Context::default());
        let generation = worker.render(source.into(), path(), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        assert!(outcome.result.is_ok_and(|scene| scene.is_empty()), "not found without the folder");

        worker.set_libraries(vec![dir.clone()]);
        let generation = worker.render(source.into(), path(), Vec::new());
        let Some(Reply::Rendered(outcome)) = wait_for(&worker, generation).into_iter().last() else {
            panic!("no render")
        };
        let Ok(scene) = outcome.result else { panic!("the render failed") };
        assert_eq!(scene.bounds().map(|(_, hi)| hi), Some([3.0, 3.0, 3.0]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn groups_digits() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
