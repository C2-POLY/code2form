//! The render worker: parse, evaluate and build the geometry on a thread
//! with a large stack, so the UI never waits on a script.
//!
//! The geometry (`c2f_geom::Rendered`) stays on the worker thread; the UI
//! gets plain triangles back, and asks the worker for STL exports of the
//! model it shows.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;
use c2f_eval::customizer::{Literal, Parameter};
use c2f_geom::{Model, RenderMesh, Rendered, Stats};

use crate::console::{Level, Line};

/// Reserved, not committed: deeply recursive scripts need a deep stack.
const STACK: usize = 1 << 30;

/// What the viewer draws, in plain data.
pub struct Scene {
    pub solid: RenderMesh,
    pub highlights: RenderMesh,
    pub backgrounds: RenderMesh,
    pub is_2d: bool,
    pub stats: Stats,
    /// The model is a non-empty solid, so it can be exported as STL.
    pub exportable: bool,
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
}

pub enum Reply {
    Rendered(Box<RenderOutcome>),
    Exported { target: PathBuf, result: Result<String, String> },
}

enum Request {
    Render { generation: u64, source: String, path: PathBuf, values: Vec<(String, Literal)> },
    ExportStl { target: PathBuf },
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

    /// Writes the last good model to `target` as an ASCII STL.
    pub fn export_stl(&self, target: PathBuf) {
        let _ = self.requests.send(Request::ExportStl { target });
    }

    pub fn latest(&self) -> u64 {
        self.latest.load(Ordering::Acquire)
    }

    pub fn poll(&self) -> Vec<Reply> {
        self.replies.try_iter().collect()
    }
}

fn run(requests: &Receiver<Request>, replies: &Sender<Reply>, latest: &AtomicU64, ctx: &egui::Context) {
    let library_paths = c2f_eval::default_library_paths();
    // The geometry of the model on screen, for exports.
    let mut shown: Option<Rendered> = None;
    while let Ok(first) = requests.recv() {
        // Only the newest render matters; exports are all honored.
        let mut render = None;
        let mut exports = Vec::new();
        for request in std::iter::once(first).chain(requests.try_iter()) {
            match request {
                Request::Render { generation, source, path, values } => render = Some((generation, source, path, values)),
                Request::ExportStl { target } => exports.push(target),
            }
        }
        if let Some((generation, source, path, values)) = render {
            if generation == latest.load(Ordering::Acquire) {
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
        for target in exports {
            let result = export_stl(shown.as_ref(), &target);
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
    let result = catch_unwind(AssertUnwindSafe(|| {
        pipeline(source, path, values, library_paths, &mut console, &mut parameters)
    }))
    .unwrap_or_else(
        |panic| {
            console.push(Line::new(Level::Error, format!("internal error: {}", panic_message(&*panic))));
            Err(None)
        },
    );
    let (result, rendered) = match result {
        Ok(mut rendered) => {
            let scene = Scene {
                solid: std::mem::take(&mut rendered.solid),
                highlights: std::mem::take(&mut rendered.highlights),
                backgrounds: std::mem::take(&mut rendered.backgrounds),
                is_2d: rendered.is_2d,
                stats: rendered.stats,
                exportable: matches!(rendered.model, Model::Solid(_)) && rendered.stats.triangles > 0,
            };
            (Ok(scene), Some(rendered))
        }
        Err(line) => (Err(line), None),
    };
    (RenderOutcome { generation, elapsed: start.elapsed(), console, result, parameters }, rendered)
}

fn pipeline(
    source: &str,
    path: &Path,
    values: &[(String, Literal)],
    library_paths: &[PathBuf],
    console: &mut Vec<Line>,
    parameters: &mut Option<Vec<Parameter>>,
) -> Result<Rendered, Option<usize>> {
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
    let rendered = c2f_geom::render_in(&output.nodes, program.files[0].path.parent().unwrap_or(Path::new("")));
    console.extend(rendered.warnings.iter().map(|text| Line::new(Level::Warning, text.clone())));
    Ok(rendered)
}

fn export_stl(shown: Option<&Rendered>, target: &Path) -> Result<String, String> {
    let rendered = shown.ok_or("there is no model to export yet")?;
    let stl = catch_unwind(AssertUnwindSafe(|| rendered.stl()))
        .map_err(|panic| format!("internal error: {}", panic_message(&*panic)))?
        .ok_or("the model is empty or 2D, so there is nothing to export as STL")?;
    std::fs::write(target, stl).map_err(|err| format!("cannot write {}: {err}", target.display()))?;
    Ok(format!("Exported {} triangles to {}", group_digits(rendered.stats.triangles), target.display()))
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
                Reply::Exported { .. } => None,
            })
            .collect();
        assert!(generations.windows(2).all(|w| w[0] < w[1]), "{generations:?}");
        let Some(Reply::Rendered(outcome)) = replies.into_iter().last() else { panic!("no render") };
        assert_eq!(outcome.generation, last);
        assert!(outcome.console.iter().any(|l| l.level == Level::Echo && l.text.contains("hi")));
        let Ok(scene) = outcome.result else { panic!("the render failed") };
        assert!(!scene.is_2d && scene.exportable && scene.stats.triangles > 0);
        assert_eq!(scene.solid.positions.len(), scene.solid.normals.len());

        let target = std::env::temp_dir().join(format!("code2form-test-{}.stl", std::process::id()));
        worker.export_stl(target.clone());
        let start = Instant::now();
        let result = loop {
            if let Some(Reply::Exported { result, .. }) = worker.poll().into_iter().next() {
                break result;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "no export reply");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(result.is_ok(), "{result:?}");
        let stl = std::fs::read_to_string(&target).unwrap();
        let _ = std::fs::remove_file(&target);
        assert!(stl.starts_with("solid") && stl.contains("facet normal"));
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
    fn groups_digits() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
