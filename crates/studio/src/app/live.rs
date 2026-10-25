//! The agent's live tools: they read and edit the documents open in the
//! studio, not the files on disk, so that the agent's edits show at once,
//! keep the user's unsaved edits, and each undo as one step.
//!
//! [`LiveTools`] provides them to an agent from any thread but the UI's:
//! a call sends a [`Request`] to the UI thread and waits for its answer.
//! The app drains the requests each frame ([`StudioApp::serve_live`]),
//! and the socket of [`c2f_agent::studio_link`] serves the same tools to
//! `c2f mcp --studio`. The heavy work (pictures from a preset view, the
//! parts of a design) is done on the caller's thread, from a snapshot of
//! the document.

use std::fmt::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use c2f_agent::Tools;
use c2f_eval::customizer::Literal;
use eframe::egui;
use serde_json::{Map, Value, json};

use super::{StudioApp, Tab, find_hit};
use crate::console::Level;
use crate::pick::Hit;

/// How long a call waits for the UI thread to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(30);
/// How long an edit waits for its answer: the user may review it first.
const REVIEW_WAIT: Duration = Duration::from_secs(15 * 60);
/// How long an edit waits for its render.
const RENDER_WAIT: Duration = Duration::from_secs(30);
/// The longest side of a picture of the view, in pixels.
const MAX_SHOT: usize = 1200;
/// The console rows `studio_state` gives, the latest.
const CONSOLE_ROWS: usize = 40;

/// The live tools, for an agent: cheap to clone, usable from any thread
/// but the UI's, which answers them.
#[derive(Clone)]
pub struct LiveTools {
    requests: Sender<Request>,
    ctx: egui::Context,
    /// The parts of the design last edited, with the text and values they
    /// were read from, to tell which parts the next edit moved.
    parts: PartsSeen,
}

/// The parts of the design last edited, and what they were read from.
type PartsSeen = Arc<Mutex<Option<(Value, Vec<PartBox>)>>>;

/// A part of a design and the corners of its box.
pub(super) type PartBox = (String, [f32; 3], [f32; 3]);

/// A call on its way to the UI thread, and where its answer goes.
pub struct Request {
    op: Op,
    reply: Sender<Result<Value, String>>,
}

/// What the UI thread is asked to do: each quick, the waiting is done by
/// the caller.
enum Op {
    /// The state of the studio; `tab`'s text, numbered or not.
    State { tab: Option<usize>, numbered: bool },
    /// A tab's text, file and parameter values.
    Snapshot { tab: Option<usize> },
    /// Edits a tab's text: `{tab_id, generation, changed_lines}`.
    Edit { tab: Option<usize>, edit: CodeEdit },
    /// Sets parameters: `{tab_id, generation}`.
    SetParameters { tab: Option<usize>, values: Map<String, Value> },
    /// The render report of tab `id` once `generation` is rendered;
    /// `null` until then.
    Rendered { id: u64, generation: u64 },
    /// A picture of the view as the user sees it, answered a few frames
    /// later.
    Screenshot,
    /// Selects a circuit's part, pin or wire by `name`, shows `line` in
    /// the code and frames `bounds` in the view.
    Focus { name: Option<String>, line: Option<usize>, bounds: Option<([f32; 3], [f32; 3])> },
    /// Opens a file in a tab.
    Open { path: String },
    /// Flashes the box `lo`..`hi` in the view of tab `id`: the part an
    /// edit touched.
    Flash { id: u64, lo: [f32; 3], hi: [f32; 3] },
}

/// The studio's side of the live tools: the requests to answer, and the
/// picture of the view asked for.
pub struct Live {
    tools: LiveTools,
    requests: Receiver<Request>,
    shot: Option<Sender<Result<Value, String>>>,
    /// The agent's change waiting for the user's review.
    pub(super) review: Option<super::review::Pending>,
}

/// The mark of the screenshots the live tools ask for.
struct Shot;

impl Live {
    /// The live tools of the app drawn in `ctx`, and their requests.
    pub fn new(ctx: &egui::Context) -> Live {
        let (requests, received) = mpsc::channel();
        let tools = LiveTools { requests, ctx: ctx.clone(), parts: Arc::default() };
        Live { tools, requests: received, shot: None, review: None }
    }

    /// The tools, to give an agent.
    pub fn tools(&self) -> LiveTools {
        self.tools.clone()
    }
}

/// A change to a document's text, as `edit_code` takes it.
#[derive(Clone, Debug, PartialEq)]
pub enum CodeEdit {
    /// Each `old` text, found exactly once, replaced by its `new` one, in
    /// order.
    Replace(Vec<(String, String)>),
    /// Lines `start` to `end` (from 1, both included) replaced by `new`;
    /// with `end` = `start` - 1, `new` goes before line `start`.
    Lines { start: usize, end: usize, new: String },
}

/// `text` with `edit` made, and the lines it changed in the new text (from
/// 0, the end excluded); the error says which edit cannot be made, and
/// then nothing is.
pub fn apply(text: &str, edit: &CodeEdit) -> Result<(String, Vec<Range<usize>>), String> {
    let mut text = text.to_string();
    // Byte ranges of the new text, kept in step with each edit.
    let mut marks: Vec<Range<usize>> = Vec::new();
    let mut replace = |text: &mut String, at: Range<usize>, new: &str| {
        let shift = new.len() as isize - at.len() as isize;
        let moved = |offset: usize| (offset as isize + shift) as usize;
        let mut mark = at.start..at.start + new.len();
        marks.retain_mut(|m| {
            if m.start >= at.end && !(m.start == at.end && at.is_empty()) {
                *m = moved(m.start)..moved(m.end);
                true
            } else if m.end <= at.start {
                true
            } else {
                // Overlapping marks become one.
                mark = mark.start.min(m.start)..mark.end.max(if m.end > at.end { moved(m.end) } else { mark.end });
                false
            }
        });
        marks.push(mark);
        text.replace_range(at, new);
    };
    match edit {
        CodeEdit::Replace(edits) => {
            if edits.is_empty() {
                return Err("`edits` is empty: give at least one {old, new}".into());
            }
            for (i, (old, new)) in edits.iter().enumerate() {
                let which = if edits.len() > 1 { format!("edit {} of {}: ", i + 1, edits.len()) } else { String::new() };
                if old.is_empty() {
                    return Err(format!("{which}`old` is empty: give text that occurs once, or use `range` to insert lines"));
                }
                let found: Vec<usize> = text.match_indices(old.as_str()).map(|(at, _)| at).collect();
                match found.as_slice() {
                    [at] => replace(&mut text, *at..*at + old.len(), new),
                    [] => {
                        return Err(format!(
                            "{which}`old` occurs 0 times in the text{}: read it again with studio_state and copy `old` exactly, without the line numbers",
                            if i > 0 { " as the edits before it left it" } else { "" }
                        ));
                    }
                    many => {
                        return Err(format!(
                            "{which}`old` occurs {} times: add the lines around it so that it occurs once",
                            many.len()
                        ));
                    }
                }
            }
        }
        CodeEdit::Lines { start, end, new } => {
            let starts = line_starts(&text);
            let count = starts.len() - 1;
            if *start < 1 || *start > count + 1 || *end + 1 < *start || *end > count {
                return Err(format!(
                    "lines {start} to {end} are not in the text, which has {count} lines: give 1 ≤ start_line ≤ end_line ≤ {count}, or end_line = start_line - 1 to insert before start_line"
                ));
            }
            let at = starts[start - 1]..starts[*end];
            let mut new = new.clone();
            let appending = at.start == text.len() && !text.is_empty() && !text.ends_with('\n');
            if appending && !new.is_empty() {
                // The last line ends before the new ones start.
                text.push('\n');
                replace(&mut text, at.start + 1..at.start + 1, &new);
                return Ok(finish(text, marks));
            }
            if !new.is_empty() && !new.ends_with('\n') && (at.end < text.len() || text.ends_with('\n') || at.is_empty() && at.start < text.len()) {
                new.push('\n');
            }
            replace(&mut text, at, &new);
        }
    }
    Ok(finish(text, marks))
}

/// The text edited, and the lines its byte ranges `marks` cover.
fn finish(text: String, mut marks: Vec<Range<usize>>) -> (String, Vec<Range<usize>>) {
    marks.sort_by_key(|m| m.start);
    let lines = marks
        .iter()
        .map(|m| {
            let first = line_of(&text, m.start);
            let last = if m.is_empty() { first } else { line_of(&text, m.end - 1) };
            first..last + 1
        })
        .collect();
    (text, lines)
}

/// The byte where each line starts, and the text's length last: a final
/// line break starts no line.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(at, _)| at + 1).filter(|&at| at < text.len()));
    if text.is_empty() {
        starts.clear();
    }
    starts.push(text.len());
    starts
}

/// The line (from 0) of byte `offset`.
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|&&b| b == b'\n').count()
}

/// The byte where line `line` (from 0) starts, or the text's end.
fn line_offset(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n').nth(line - 1).map_or(text.len(), |(at, _)| at + 1)
}

/// The byte where `part("name"` is written in `text`, spaces allowed.
fn part_call(text: &str, name: &str) -> Option<usize> {
    part_calls(text).find(|(_, n)| *n == name).map(|(at, _)| at)
}

/// Each `part("name"` written in `text`, spaces allowed: the byte where
/// it starts, and the name.
pub(super) fn part_calls(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.match_indices("part").filter_map(|(at, _)| {
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            return None;
        }
        let args = text[at + 4..].trim_start().strip_prefix('(')?.trim_start().strip_prefix('"')?;
        Some((at, &args[..args.find('"')?]))
    })
}

/// Why an edit cannot be made, for the agent: the text may have changed
/// under it, as the user types too.
pub(super) fn refused(why: String) -> String {
    if why.contains("studio_state") {
        why
    } else {
        format!("{why}. The code may have changed since you read it (the user types too): call studio_state again before editing")
    }
}

/// A customizer value as JSON.
fn literal(value: &Literal) -> Value {
    match value {
        Literal::Undef => Value::Null,
        Literal::Bool(b) => json!(b),
        Literal::Number(n) => json!(n),
        Literal::Str(s) => json!(s),
        Literal::Range { begin, step, end } => json!({ "begin": begin, "step": step.unwrap_or(1.0), "end": end }),
        Literal::Vector(items) => Value::Array(items.iter().map(literal).collect()),
    }
}

fn text(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The `tab` argument: a tab's index, from 0.
fn tab_arg(args: &Value) -> Result<Option<usize>, String> {
    match args.get("tab") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(|n| Some(n as usize)).ok_or_else(|| "`tab` must be a tab's index, from 0, as studio_state lists them".into()),
    }
}

fn tab_schema() -> Value {
    json!({ "type": "integer", "minimum": 0, "description": "The tab, by its index in studio_state's `tabs` (default: the active tab)." })
}

impl LiveTools {
    /// Sends `op` to the UI thread and waits for its answer.
    fn ask(&self, op: Op) -> Result<Value, String> {
        let wait = if matches!(op, Op::Edit { .. }) { REVIEW_WAIT } else { ANSWER_WAIT };
        let (reply, answer) = mpsc::channel();
        self.requests.send(Request { op, reply }).map_err(|_| "the studio is closing".to_string())?;
        self.ctx.request_repaint();
        match answer.recv_timeout(wait) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                Err("the studio did not answer within 30 s (busy, or a dialog is open): try again".into())
            }
            Err(RecvTimeoutError::Disconnected) => Err("the studio dropped the request: try again".into()),
        }
    }

    /// Waits for the render of tab `id` numbered `generation`: its report,
    /// or one saying it is still running.
    fn wait_render(&self, id: u64, generation: u64) -> Result<Value, String> {
        let deadline = Instant::now() + RENDER_WAIT;
        loop {
            let report = self.ask(Op::Rendered { id, generation })?;
            if !report.is_null() {
                return Ok(report);
            }
            if Instant::now() >= deadline {
                return Ok(json!({ "rendering": true, "note": "still rendering after 30 s: call studio_state later for the outcome" }));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn state(&self, args: &Value) -> Result<Vec<Value>, String> {
        let numbered = args.get("line_numbers").and_then(Value::as_bool).unwrap_or(true);
        let state = self.ask(Op::State { tab: tab_arg(args)?, numbered })?;
        Ok(vec![text(pretty(&state))])
    }

    fn edit(&self, args: &Value) -> Result<Vec<Value>, String> {
        let edit = match (args.get("edits"), args.get("range")) {
            (Some(edits), None) => {
                let edits = edits.as_array().ok_or("`edits` must be a list of {old, new}")?;
                let pairs = edits.iter().map(|edit| {
                    let field = |key: &str| edit.get(key).and_then(Value::as_str).map(str::to_string);
                    field("old").zip(field("new")).ok_or_else(|| format!("each edit needs `old` and `new`, as text: {edit}"))
                });
                CodeEdit::Replace(pairs.collect::<Result<_, _>>()?)
            }
            (None, Some(range)) => {
                let line = |key: &str| range.get(key).and_then(Value::as_u64).map(|n| n as usize);
                let (Some(start), Some(end)) = (line("start_line"), line("end_line")) else {
                    return Err("`range` needs `start_line` and `end_line`, line numbers from 1".into());
                };
                let new = args.get("new").and_then(Value::as_str).ok_or("with `range`, `new` holds the lines that replace them")?;
                CodeEdit::Lines { start, end, new: new.to_string() }
            }
            _ => return Err("give either `edits` (a list of {old, new}) or `range` with `new`".into()),
        };
        let done = self.ask(Op::Edit { tab: tab_arg(args)?, edit })?;
        let report = self.wait_render(done["tab_id"].as_u64().unwrap_or(0), done["generation"].as_u64().unwrap_or(0))?;
        let mut result = json!({ "edited": true, "changed_lines": done["changed_lines"], "render": report });
        if let Some(part) = done["part"].as_str().filter(|_| report["ok"] == true) {
            result["part"] = json!(part);
            if let Some(changed) = self.touched_parts(&done, part) {
                result["parts_changed"] = changed;
            }
        }
        Ok(vec![text(pretty(&result))])
    }

    /// Once an edit inside part `part` rendered: flashes the part in the
    /// view, and gives the parts whose box the edit changed, when the
    /// parts before it are known from the last edit.
    fn touched_parts(&self, done: &Value, part: &str) -> Option<Value> {
        let snapshot = self.ask(Op::Snapshot { tab: done["tab"].as_u64().map(|n| n as usize) }).ok()?;
        let after = design_parts(&snapshot).ok()?;
        let key = |text: &Value| json!([text, snapshot["values"]]);
        let before = {
            let mut cache = self.parts.lock().ok()?;
            let before = cache.take().filter(|(k, _)| *k == key(&done["text_before"])).map(|(_, parts)| parts);
            *cache = Some((key(&snapshot["text"]), after.clone()));
            before
        };
        if let Some((_, lo, hi)) = after.iter().find(|p| p.0 == part) {
            let _ = self.ask(Op::Flash { id: done["tab_id"].as_u64().unwrap_or(0), lo: *lo, hi: *hi });
        }
        Some(json!(changed_parts(&before?, &after)))
    }

    fn set_parameters(&self, args: &Value) -> Result<Vec<Value>, String> {
        let values = args.get("values").and_then(Value::as_object).ok_or("`values` must be an object of names and values")?;
        let done = self.ask(Op::SetParameters { tab: tab_arg(args)?, values: values.clone() })?;
        let report = self.wait_render(done["tab_id"].as_u64().unwrap_or(0), done["generation"].as_u64().unwrap_or(0))?;
        Ok(vec![text(pretty(&json!({ "set": true, "parameters": done["parameters"], "render": report })))])
    }

    fn view_image(&self, args: &Value) -> Result<Vec<Value>, String> {
        let Some(view) = args.get("view").and_then(Value::as_str).filter(|v| *v != "current") else {
            return Ok(vec![self.ask(Op::Screenshot)?]);
        };
        // A preset view is drawn here, from the document as it is in the
        // studio, as the built-in tools draw it.
        let snapshot = self.ask(Op::Snapshot { tab: None })?;
        let mut call = json!({ "code": snapshot["text"], "working_dir": snapshot["dir"], "view": view });
        for key in ["width", "height", "section"] {
            if let Some(value) = args.get(key) {
                call[key] = value.clone();
            }
        }
        let tool = if snapshot["circuit"] == true {
            "circuit_image"
        } else {
            call["parameters"] = snapshot["values"].clone();
            "image"
        };
        c2f_agent::Builtin.call(tool, &call).unwrap_or_else(|| Err(format!("no built-in `{tool}` tool")))
    }

    fn focus(&self, args: &Value) -> Result<Vec<Value>, String> {
        let name = args.get("part").and_then(Value::as_str).map(str::to_string);
        let line = match args.get("line") {
            None => None,
            Some(v) => Some(v.as_u64().filter(|&n| n >= 1).ok_or("`line` must be a line number, from 1")? as usize - 1),
        };
        if name.is_none() && line.is_none() {
            return Err("give `part` (a part's name) or `line` (a line of the code)".into());
        }
        let snapshot = self.ask(Op::Snapshot { tab: None })?;
        if snapshot["circuit"] == true {
            let done = self.ask(Op::Focus { name, line, bounds: None })?;
            return Ok(vec![text(pretty(&done))]);
        }
        let mut bounds = None;
        let mut line = line;
        if let Some(name) = &name {
            let source = snapshot["text"].as_str().unwrap_or_default();
            let at = part_call(source, name);
            line = line.or(at.map(|at| line_of(source, at)));
            bounds = Some(part_bounds(&snapshot, name)?);
        }
        let done = self.ask(Op::Focus { name: None, line, bounds })?;
        Ok(vec![text(pretty(&done))])
    }

    fn open_file(&self, args: &Value) -> Result<Vec<Value>, String> {
        let path = args.get("path").and_then(Value::as_str).ok_or("`path` names the file to open")?;
        let done = self.ask(Op::Open { path: path.to_string() })?;
        Ok(vec![text(pretty(&done))])
    }
}

/// The parts whose box differs between `before` and `after`, those added
/// and those gone, with their boxes.
fn changed_parts(before: &[PartBox], after: &[PartBox]) -> Vec<Value> {
    let corners = |p: Option<&PartBox>| p.map(|(_, lo, hi)| json!({ "min": lo, "max": hi }));
    let mut names: Vec<&str> = Vec::new();
    for (name, _, _) in before.iter().chain(after) {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .filter_map(|name| {
            let (old, new) = (before.iter().find(|p| p.0 == name), after.iter().find(|p| p.0 == name));
            let close = |a: &[f32; 3], b: &[f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4);
            let same = matches!((old, new), (Some(a), Some(b)) if close(&a.1, &b.1) && close(&a.2, &b.2));
            (!same).then(|| json!({ "part": name, "before": corners(old), "after": corners(new) }))
        })
        .collect()
}

/// The box around the part `name` of the design in `snapshot`, rendered
/// with its parameter values.
fn part_bounds(snapshot: &Value, name: &str) -> Result<([f32; 3], [f32; 3]), String> {
    let parts = design_parts(snapshot)?;
    let Some((_, lo, hi)) = parts.iter().find(|p| p.0 == name) else {
        let names: Vec<&str> = parts.iter().map(|p| p.0.as_str()).collect();
        return Err(format!("no part(\"{name}\") in the design; its parts: {}", if names.is_empty() { "none".into() } else { names.join(", ") }));
    };
    Ok((*lo, *hi))
}

/// The parts of the design in `snapshot`, rendered with its parameter
/// values, with their boxes: those with nothing in them left out.
pub(super) fn design_parts(snapshot: &Value) -> Result<Vec<PartBox>, String> {
    let source = snapshot["text"].as_str().unwrap_or_default();
    let path = PathBuf::from(snapshot["path"].as_str().unwrap_or_default());
    let mut program = c2f_eval::Program::load(source, &path, &c2f_eval::default_library_paths())
        .map_err(|err| format!("the design does not read: {}", err.diagnostic.message))?;
    let parameters = program.parameters(source);
    for (key, value) in snapshot["values"].as_object().into_iter().flatten() {
        let Some(parameter) = parameters.iter().find(|p| p.name == *key) else { continue };
        let raw = value.as_str().map_or_else(|| value.to_string(), str::to_string);
        if let Ok(Some(value)) = c2f_eval::customizer::parse_typed(&parameter.default, &raw) {
            program.set_parameter(key, &value);
        }
    }
    let output = c2f_eval::evaluate(&program);
    if let Some(error) = &output.error {
        return Err(format!("the design does not render: {error}"));
    }
    let dir = path.parent().unwrap_or(Path::new(""));
    let options = c2f_geom::PartsOptions { name: "(root)".into(), ..Default::default() };
    let (json, _) = c2f_geom::parts_json(&output.nodes, dir, &options)?;
    let parts: Value = serde_json::from_str(&json).map_err(|err| err.to_string())?;
    let parts = parts["parts"].as_array().cloned().unwrap_or_default();
    let boxes = parts.iter().filter(|p| p["name"] != "(root)").filter_map(|part| {
        let corner = |key: &str| -> Option<[f32; 3]> {
            let v = part["bounds"][key].as_array()?;
            Some([v.first()?.as_f64()? as f32, v.get(1)?.as_f64()? as f32, v.get(2)?.as_f64()? as f32])
        };
        Some((part["name"].as_str()?.to_string(), corner("min")?, corner("max")?))
    });
    Ok(boxes.collect())
}

impl Tools for LiveTools {
    fn list(&self) -> Vec<Value> {
        let views: Vec<&str> = ["current"].into_iter().chain(c2f_geom::VIEWS.iter().map(|(name, _, _)| *name)).chain(["isometric", "sheet"]).collect();
        vec![
            json!({
                "name": "studio_state",
                "title": "What the studio shows",
                "description": "Reads the Code2Form studio the user has open, as it is on screen, unsaved edits included: \
                                the open tabs (index, file, whether unsaved, design or circuit), the active tab's code with \
                                line numbers (`N<tab>` before each line: leave them out of edit_code's `old`), its customizer \
                                parameters with their current values, what is selected (a circuit's part, pin or wire, the \
                                cursor's line), the last render (errors with their line, warnings, volume, bounding box in mm), \
                                the lines the agent last changed, and the console's latest lines. Read it before editing, and \
                                again when an edit is refused.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tab": tab_schema(),
                        "line_numbers": { "type": "boolean", "default": true, "description": "Number the code's lines." },
                    },
                },
            }),
            json!({
                "name": "edit_code",
                "title": "Edit the open code",
                "description": "Edits the code of a tab open in the studio, unsaved edits kept, as one step the user can undo, \
                                then renders it and waits for the render: the result says whether it rendered, with the \
                                error and its line, the warnings, the volume and the bounding box, so you can fix your own \
                                errors. Either `edits`, a list of {old, new}: each `old` must occur exactly once in the text \
                                (copied exactly, without line numbers), else nothing is changed and the error says how many \
                                times it occurs; the edits apply in order, all or none. Or `range` with `new`: lines \
                                start_line to end_line (from 1, both included) become `new`; end_line = start_line - 1 \
                                inserts `new` before start_line. The file is not saved: the user saves it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tab": tab_schema(),
                        "edits": {
                            "type": "array",
                            "minItems": 1,
                            "description": "Replacements, applied in order.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "old": { "type": "string", "description": "Text that occurs exactly once." },
                                    "new": { "type": "string", "description": "What replaces it." },
                                },
                                "required": ["old", "new"],
                            },
                        },
                        "range": {
                            "type": "object",
                            "description": "Lines to replace, from 1, both included.",
                            "properties": {
                                "start_line": { "type": "integer", "minimum": 1 },
                                "end_line": { "type": "integer", "minimum": 0 },
                            },
                            "required": ["start_line", "end_line"],
                        },
                        "new": { "type": "string", "description": "With `range`: the lines that replace them." },
                    },
                },
            }),
            json!({
                "name": "set_parameters",
                "title": "Set customizer parameters",
                "description": "Sets customizer parameters of the open design as the user would in its Parameters card (the \
                                code is not changed), renders it and waits for the render: the result gives the values and \
                                the render's outcome. Each value is read as the type of the parameter's default: a number, \
                                a boolean, a string or a list of numbers. studio_state lists the parameters.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tab": tab_schema(),
                        "values": { "type": "object", "description": "Parameter names and their new values, as {\"width\": 40}." },
                    },
                    "required": ["values"],
                },
            }),
            json!({
                "name": "view_image",
                "title": "Picture of the open view",
                "description": "A PNG picture of the active tab's model. `current` (the default) is the 3D view exactly as \
                                the user sees it, with the camera, section and selection they set. A preset view (front, \
                                top, diagonal...) or `sheet` draws the document as it is in the studio, unsaved edits and \
                                parameter values included, framed to fit, with the render report.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "view": { "type": "string", "enum": views, "default": "current", "description": "The user's view, or where the camera looks from." },
                        "width": { "type": "integer", "minimum": 1, "maximum": 1600, "description": "With a preset view: in pixels." },
                        "height": { "type": "integer", "minimum": 1, "maximum": 1600, "description": "With a preset view: in pixels." },
                        "section": { "type": "boolean", "description": "With `view` = `sheet`: add a view cut through the middle." },
                    },
                },
            }),
            json!({
                "name": "focus",
                "title": "Show the user a part",
                "description": "Points the user at something of the active tab: `part` selects it and frames the view on it \
                                (a design's part(\"name\"); in a circuit a part's name, a pin as `part.pin`, or a wire as \
                                `wire:3`), and shows its line in the code; `line` shows that line of the code.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "part": { "type": "string", "description": "A part's name, a circuit's pin (`part.pin`) or wire (`wire:N`)." },
                        "line": { "type": "integer", "minimum": 1, "description": "A line of the code, from 1." },
                    },
                },
            }),
            json!({
                "name": "open_file",
                "title": "Open a file in the studio",
                "description": "Opens a file in a tab of the studio and shows it (or shows its tab when it is open already): a \
                                design (.c2f, .scad), a circuit (.c2fc) or a mesh. `path` is in the active file's folder \
                                or under it, relative to it or absolute.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "The file, in the active file's folder." } },
                    "required": ["path"],
                },
            }),
        ]
    }

    fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
        Some(match name {
            "studio_state" => self.state(args),
            "edit_code" => self.edit(args),
            "set_parameters" => self.set_parameters(args),
            "view_image" => self.view_image(args),
            "focus" => self.focus(args),
            "open_file" => self.open_file(args),
            _ => return None,
        })
    }
}

/// What the last render of `tab` gave: whether it failed, on which line,
/// its errors and warnings, and the model's size.
fn render_report(tab: &Tab) -> Value {
    let mut report = Map::new();
    report.insert("rendering".into(), json!(tab.busy()));
    report.insert("ok".into(), json!(!tab.failed));
    if let Some(line) = tab.editor.error_line {
        report.insert("error_line".into(), json!(line + 1));
    }
    let printed = |level: Level| -> Vec<&str> { tab.console.rows().filter(|r| r.2 && r.0 == level).map(|r| r.1).collect() };
    report.insert("errors".into(), json!(printed(Level::Error)));
    report.insert("warnings".into(), json!(printed(Level::Warning)));
    let echo = printed(Level::Echo);
    if !echo.is_empty() {
        report.insert("echo".into(), json!(echo));
    }
    if let Some(model) = tab.model.as_ref().filter(|_| !tab.failed) {
        let kind = if model.empty { "empty" } else if model.is_2d { "2D" } else { "3D" };
        report.insert("model".into(), json!(kind));
        if !model.empty {
            report.insert((if model.is_2d { "area" } else { "volume" }).into(), json!(model.stats.volume));
            report.insert("triangles".into(), json!(model.stats.triangles));
        }
        if let Some((lo, hi)) = model.bounds {
            let size: Vec<f32> = (0..3).map(|i| hi[i] - lo[i]).collect();
            report.insert("bounding_box".into(), json!({ "min": lo, "max": hi, "size": size }));
        }
        if tab.is_circuit() {
            report.insert("parts".into(), json!(model.parts));
        }
    }
    if let Some(elapsed) = tab.elapsed {
        report.insert("milliseconds".into(), json!(elapsed.as_millis() as u64));
    }
    Value::Object(report)
}

/// Agent marks as line numbers from 1, both ends included.
pub(super) fn marks_json(marks: &[Range<usize>]) -> Value {
    json!(marks.iter().map(|m| [m.start + 1, m.end]).collect::<Vec<_>>())
}

impl StudioApp {
    /// The live tools, to give an agent: call them from another thread
    /// than the UI's, which answers them in [`StudioApp::serve_live`].
    pub fn live_tools(&self) -> LiveTools {
        self.live.tools()
    }

    /// Answers the live tools' requests, each frame.
    pub(super) fn serve_live(&mut self, ctx: &egui::Context) {
        self.take_live_shot(ctx);
        while let Ok(Request { op, reply }) = self.live.requests.try_recv() {
            if let Op::Screenshot = op {
                match self.ask_live_shot(ctx) {
                    Ok(()) => self.live.shot = Some(reply),
                    Err(why) => {
                        let _ = reply.send(Err(why));
                    }
                }
                continue;
            }
            // Reviewed first, when the settings ask: the answer waits for
            // the user's.
            let op = match op {
                Op::Edit { tab, edit } if self.reviews_agent_edits() => {
                    let index = tab.unwrap_or(self.shelf.active());
                    if index < self.shelf.len() {
                        self.in_tab(index, |app| app.review_agent_edit(ctx, edit, Some(reply)));
                    } else {
                        let _ = reply.send(Err(format!("no tab {index}: there are {} (studio_state lists them)", self.shelf.len())));
                    }
                    continue;
                }
                op => op,
            };
            let answer = self.answer_live(ctx, op);
            // A caller that gave up waiting is gone.
            let _ = reply.send(answer);
        }
    }

    fn answer_live(&mut self, ctx: &egui::Context, op: Op) -> Result<Value, String> {
        match op {
            Op::State { tab, numbered } => self.on_tab(tab, |app, _| Ok(app.live_state(numbered))),
            Op::Snapshot { tab } => self.on_tab(tab, |app, index| {
                let doc = &app.doc;
                let path = doc.source_path();
                let values: Map<String, Value> = doc.customizer.values().iter().map(|(n, v)| (n.clone(), literal(v))).collect();
                Ok(json!({
                    "index": index,
                    "text": doc.text,
                    "path": path.display().to_string(),
                    "dir": path.parent().unwrap_or(Path::new("")).display().to_string(),
                    "circuit": doc.is_circuit(),
                    "values": values,
                }))
            }),
            Op::Edit { tab, edit } => self.on_tab(tab, |app, index| {
                let (text, lines) = apply(&app.doc.text, &edit).map_err(refused)?;
                let mut done = app.land_agent_edit(ctx, text, lines);
                done["tab"] = json!(index);
                Ok(done)
            }),
            Op::Flash { id, lo, hi } => {
                self.flash_part(id, lo, hi);
                ctx.request_repaint();
                Ok(Value::Null)
            }
            Op::SetParameters { tab, values } => self.on_tab(tab, |app, _| {
                if app.doc.is_circuit() {
                    return Err("a circuit has no customizer parameters".into());
                }
                if values.is_empty() {
                    return Err("`values` is empty".into());
                }
                for (name, value) in &values {
                    let raw = value.as_str().map_or_else(|| value.to_string(), str::to_string);
                    app.doc.customizer.set_typed(name, &raw).map_err(|why| format!("{why}; nothing was set (studio_state lists the parameters)"))?;
                }
                let set: Vec<String> = values.iter().map(|(n, v)| format!("{n} = {v}")).collect();
                app.doc.console.log(Level::Info, tr!("The agent set {}", "L'agent a réglé {}", set.join(", ")));
                app.render_now();
                let parameters: Map<String, Value> =
                    app.doc.customizer.shown().map(|(p, v)| (p.name.clone(), literal(v))).collect();
                Ok(json!({ "tab_id": app.doc.id, "generation": app.doc.sent, "parameters": parameters }))
            }),
            Op::Rendered { id, generation } => {
                let index = self.tab_index(id).ok_or("the tab was closed")?;
                let tab = self.shelf.iter(&self.doc).nth(index).ok_or("the tab was closed")?;
                if tab.finished >= generation {
                    return Ok(render_report(tab));
                }
                // Another tab's render took its place: it renders once shown.
                let lost = index != self.shelf.active() && tab.busy() && tab.sent != self.worker.latest();
                if lost {
                    return Ok(json!({ "rendering": false, "note": "the tab is not shown: it renders once the user shows it" }));
                }
                Ok(Value::Null)
            }
            Op::Screenshot => Err("asked in serve_live".into()),
            Op::Focus { name, line, bounds } => {
                let mut done = Map::new();
                let mut line = line;
                if let Some(name) = &name {
                    let wiring = self.doc.wiring.as_ref().ok_or("the circuit is not read yet: try again once it renders")?;
                    let circuit = wiring.circuit.clone();
                    let hit = find_hit(&circuit, name).ok_or_else(|| {
                        let names: Vec<&str> = circuit.parts.iter().map(|p| p.name.as_str()).collect();
                        format!("nothing named `{name}` in the circuit; its parts: {}", names.join(", "))
                    })?;
                    self.select(ctx, Some(hit));
                    let span = match hit {
                        Hit::Part(part) => {
                            if let Some(Some((lo, hi))) = self.doc.part_boxes.get(part).copied() {
                                self.frame_live(lo.to_array(), hi.to_array());
                                done.insert("framed".into(), json!(true));
                            }
                            Some(circuit.parts[part].span)
                        }
                        Hit::Wire(wire) => Some(circuit.wires[wire].span),
                        Hit::Pin(end) => Some(circuit.parts[end.part].span),
                    };
                    line = line.or(span.map(|s| line_of(&self.doc.text, s.start as usize)));
                    done.insert("selected".into(), json!(name));
                }
                if let Some((lo, hi)) = bounds {
                    self.frame_live(lo, hi);
                    done.insert("framed".into(), json!(true));
                }
                if let Some(line) = line {
                    let offset = line_offset(&self.doc.text, line);
                    self.doc.editor.go_to(ctx, &self.doc.text, offset);
                    done.insert("line".into(), json!(line + 1));
                }
                Ok(Value::Object(done))
            }
            Op::Open { path } => {
                let dir = self.doc.directory().ok_or("the active tab has no folder")?;
                let asked = Path::new(&path);
                let full = if asked.is_absolute() { asked.to_path_buf() } else { dir.join(asked) };
                let full = full.canonicalize().map_err(|err| format!("cannot open {}: {err}", full.display()))?;
                let inside = dir.canonicalize().is_ok_and(|dir| full.starts_with(dir));
                if !inside || !full.is_file() {
                    return Err(format!("{} is not a file in the active file's folder, {}", full.display(), dir.display()));
                }
                self.open(ctx, full.clone());
                let index = self.tab_of(&full).ok_or_else(|| format!("cannot open {} (the console says why)", full.display()))?;
                Ok(json!({ "opened": full.display().to_string(), "tab": index }))
            }
        }
    }

    /// Runs `f` on tab `tab` (by default the active one), as if it were
    /// the active tab, with its index.
    fn on_tab(&mut self, tab: Option<usize>, f: impl FnOnce(&mut Self, usize) -> Result<Value, String>) -> Result<Value, String> {
        let index = tab.unwrap_or(self.shelf.active());
        if index >= self.shelf.len() {
            return Err(format!("no tab {index}: there are {} (studio_state lists them)", self.shelf.len()));
        }
        let mut result = Err(String::new());
        self.in_tab(index, |app| result = f(app, index));
        result
    }

    /// Turns the camera smoothly to frame the box `lo`..`hi`.
    fn frame_live(&mut self, lo: [f32; 3], hi: [f32; 3]) {
        let viewer = &mut self.doc.viewer;
        let mut goal = viewer.camera;
        goal.frame(glam::Vec3::from(lo), glam::Vec3::from(hi), viewer.aspect());
        viewer.fly_to(goal);
    }

    /// What `studio_state` gives of the active tab, put in its place.
    fn live_state(&self, numbered: bool) -> Value {
        let active = self.shelf.active();
        let doc = &self.doc;
        let tabs: Vec<Value> = self
            .shelf
            .iter(doc)
            .enumerate()
            .map(|(i, tab)| {
                json!({
                    "index": i,
                    "file": tab.path.as_ref().map(|p| p.display().to_string()),
                    "name": tab.file_name(),
                    "unsaved": tab.is_dirty(),
                    "kind": if tab.is_circuit() { "circuit" } else { "design" },
                    "active": i == active,
                })
            })
            .collect();
        let code = if numbered {
            doc.text.lines().enumerate().fold(String::new(), |mut code, (i, line)| {
                let _ = writeln!(code, "{}\t{line}", i + 1);
                code
            })
        } else {
            doc.text.clone()
        };
        let parameters: Vec<Value> = doc
            .customizer
            .shown()
            .map(|(p, value)| {
                json!({ "name": p.name, "group": p.group, "description": p.description, "value": literal(value), "default": literal(&p.default) })
            })
            .collect();
        let circuit = doc.wiring.as_ref().map(|w| &w.circuit);
        let selection = match (doc.selection, circuit) {
            (Some(Hit::Part(part)), Some(c)) => {
                json!({ "part": c.parts[part].name, "line": line_of(&doc.text, c.parts[part].span.start as usize) + 1 })
            }
            (Some(Hit::Pin(end)), Some(c)) => json!({ "pin": c.end_name(end) }),
            (Some(Hit::Wire(wire)), Some(c)) => json!({
                "wire": format!("wire:{}", wire + 1),
                "from": c.end_name(c.wires[wire].from),
                "to": c.end_name(c.wires[wire].to),
                "line": line_of(&doc.text, c.wires[wire].span.start as usize) + 1,
            }),
            _ => Value::Null,
        };
        let rows: Vec<Value> = doc
            .console
            .rows()
            .rev()
            .take(CONSOLE_ROWS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|(level, text, _)| json!({ "level": format!("{level:?}").to_lowercase(), "text": text }))
            .collect();
        json!({
            "tabs": tabs,
            "tab": active,
            "file": doc.path.as_ref().map(|p| p.display().to_string()),
            "kind": if doc.is_circuit() { "circuit" } else { "design" },
            "unsaved": doc.is_dirty(),
            "cursor_line": doc.editor.cursor.map(|(line, _)| line + 1),
            "selection": selection,
            "parameters": parameters,
            "render": render_report(doc),
            "agent_changed_lines": marks_json(&doc.agent_marks),
            "console": rows,
            "code": code,
        })
    }

    /// Asks for a picture of the window, to cut the view from.
    fn ask_live_shot(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.arrangement == super::Arrangement::Code || self.view_rect.width() < 1.0 {
            return Err("the 3D view is hidden: ask for a preset `view` (such as `diagonal`) instead".into());
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(Shot)));
        ctx.request_repaint();
        Ok(())
    }

    /// Answers the picture asked for once the window's arrives: the view
    /// cut from it, at most [`MAX_SHOT`] pixels on its longest side.
    fn take_live_shot(&mut self, ctx: &egui::Context) {
        if self.live.shot.is_none() {
            return;
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. }
                    if user_data.data.as_ref().is_some_and(|d| d.downcast_ref::<Shot>().is_some()) =>
                {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        let Some(image) = image else { return };
        let Some(reply) = self.live.shot.take() else { return };
        let scale = ctx.pixels_per_point();
        let [width, height] = image.size;
        let rect = self.view_rect;
        let x0 = ((rect.left() * scale).round().max(0.0) as usize).min(width);
        let y0 = ((rect.top() * scale).round().max(0.0) as usize).min(height);
        let x1 = ((rect.right() * scale).round().max(0.0) as usize).clamp(x0, width);
        let y1 = ((rect.bottom() * scale).round().max(0.0) as usize).clamp(y0, height);
        let step = (x1 - x0).max(y1 - y0).div_ceil(MAX_SHOT).max(1);
        let (w, h) = ((x1 - x0) / step, (y1 - y0) / step);
        if w == 0 || h == 0 {
            let _ = reply.send(Err("the 3D view has no size on screen".into()));
            return;
        }
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let color = image.pixels[(y0 + y * step) * width + x0 + x * step];
                rgb.extend_from_slice(&[color.r(), color.g(), color.b()]);
            }
        }
        let png = c2f_geom::encode_png(w, h, &rgb);
        let _ = reply.send(Ok(c2f_agent::image_content(&png)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(pairs: &[(&str, &str)]) -> CodeEdit {
        CodeEdit::Replace(pairs.iter().map(|(o, n)| (o.to_string(), n.to_string())).collect())
    }

    const BOX: &str = "width = 20;\ndepth = 10;\ncube([width, depth, 5]);\n";

    #[test]
    fn an_edit_replaces_text_found_once() {
        let (text, lines) = apply(BOX, &replace(&[("depth = 10;", "depth = 12;")])).unwrap();
        assert_eq!(text, "width = 20;\ndepth = 12;\ncube([width, depth, 5]);\n");
        assert_eq!(lines, vec![1..2]);
        let (text, lines) = apply(BOX, &replace(&[("cube(", "translate([0, 0, 1])\n    cube(")])).unwrap();
        assert!(text.ends_with("translate([0, 0, 1])\n    cube([width, depth, 5]);\n"));
        assert_eq!(lines, vec![2..4]);
    }

    #[test]
    fn text_found_never_or_twice_is_refused_and_nothing_changes() {
        let err = apply(BOX, &replace(&[("height", "h")])).unwrap_err();
        assert!(err.contains("0 times"), "{err}");
        let err = apply(BOX, &replace(&[("width", "w")])).unwrap_err();
        assert!(err.contains("2 times"), "{err}");
        // The second edit fails: the first is not made either.
        let err = apply(BOX, &replace(&[("width = 20", "width = 30"), ("nothing", "x")])).unwrap_err();
        assert!(err.starts_with("edit 2 of 2"), "{err}");
        assert!(apply(BOX, &replace(&[])).is_err());
        assert!(apply(BOX, &replace(&[("", "x")])).is_err());
    }

    #[test]
    fn edits_apply_in_order_and_mark_each_change() {
        let edits = replace(&[("width = 20;", "width = 30;"), ("5]", "8]"), ("width = 30;", "width = 40;")]);
        let (text, lines) = apply(BOX, &edits).unwrap();
        assert_eq!(text, "width = 40;\ndepth = 10;\ncube([width, depth, 8]);\n");
        assert_eq!(lines, vec![0..1, 2..3]);
    }

    #[test]
    fn a_range_replaces_inserts_and_deletes_lines() {
        let lines = |start, end, new: &str| CodeEdit::Lines { start, end, new: new.into() };
        let (text, marks) = apply(BOX, &lines(2, 2, "depth = 15;")).unwrap();
        assert_eq!(text, "width = 20;\ndepth = 15;\ncube([width, depth, 5]);\n");
        assert_eq!(marks, vec![1..2]);
        let (text, _) = apply(BOX, &lines(1, 0, "// A box.")).unwrap();
        assert_eq!(text, format!("// A box.\n{BOX}"));
        let (text, marks) = apply(BOX, &lines(4, 3, "sphere(1);")).unwrap();
        assert_eq!(text, format!("{BOX}sphere(1);\n"));
        assert_eq!(marks, vec![3..4]);
        let (text, _) = apply(BOX, &lines(1, 2, "")).unwrap();
        assert_eq!(text, "cube([width, depth, 5]);\n");
        let (text, _) = apply("cube(1);", &lines(2, 1, "sphere(1);")).unwrap();
        assert_eq!(text, "cube(1);\nsphere(1);");
        for (start, end) in [(0, 1), (2, 4), (3, 1), (5, 4)] {
            assert!(apply(BOX, &lines(start, end, "x")).unwrap_err().contains("3 lines"), "{start}..{end}");
        }
    }

    #[test]
    fn a_part_call_is_found_by_its_name() {
        let text = "part(\"base\") cube(10);\nmy_part(\"lid\");\npart ( \"lid\" ) sphere(2);\n";
        assert_eq!(part_call(text, "base"), Some(0));
        assert_eq!(part_call(text, "lid").map(|at| line_of(text, at)), Some(2));
        assert_eq!(part_call(text, "arm"), None);
    }

    #[test]
    fn the_bounds_of_a_design_s_part() {
        let snapshot = json!({
            "text": "size = 10;\npart(\"base\") cube(size);\ntranslate([0, 0, 20]) part(\"lid\") cube([size, size, 2]);\n",
            "path": std::env::temp_dir().join("live.c2f").display().to_string(),
            "values": { "size": 4 },
        });
        assert_eq!(part_bounds(&snapshot, "lid").unwrap(), ([0.0, 0.0, 20.0], [4.0, 4.0, 22.0]));
        let err = part_bounds(&snapshot, "arm").unwrap_err();
        assert!(err.contains("base") && err.contains("lid"), "{err}");
    }

    /// The app without a window, on `file`.
    fn studio(ctx: &egui::Context, file: &Path) -> StudioApp {
        let stores = super::super::Stores { session: None, settings: None, conversations: None };
        let settings = crate::settings::Settings::load(None);
        StudioApp::with(ctx, None, stores, crate::recent::RecentFiles::default(), settings, vec![file.to_path_buf()])
    }

    /// Calls the live tool `name` from another thread, as an agent does,
    /// while the app answers it frame after frame.
    fn call(app: &mut StudioApp, ctx: &egui::Context, name: &str, args: Value) -> Result<Vec<Value>, String> {
        let tools = app.live_tools();
        let name = name.to_string();
        let caller = std::thread::spawn(move || tools.call(&name, &args).expect("a live tool"));
        let start = Instant::now();
        while !caller.is_finished() {
            assert!(start.elapsed() < Duration::from_secs(60), "no answer");
            app.receive();
            app.serve_live(ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        caller.join().unwrap()
    }

    fn report(content: &[Value]) -> Value {
        serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn the_agent_edits_the_open_document_as_one_undo_step() {
        let dir = std::env::temp_dir().join(format!("c2f-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("box.c2f");
        let saved = "// Width.\nwidth = 20; // [1:100]\ncube([width, 10, 5]);\n";
        std::fs::write(&file, saved).unwrap();
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, &file);
        // The user's unsaved edit stays.
        app.doc.text.push_str("// mine\n");

        let state = report(&call(&mut app, &ctx, "studio_state", json!({})).unwrap());
        assert_eq!(state["unsaved"], true);
        assert!(state["code"].as_str().unwrap().starts_with("1\t// Width.\n"), "{state}");
        assert_eq!(state["parameters"][0]["name"], "width");

        let done = report(&call(&mut app, &ctx, "edit_code", json!({ "edits": [{ "old": "10, 5", "new": "30, 5" }] })).unwrap());
        assert_eq!(done["changed_lines"], json!([[3, 3]]));
        assert_eq!(done["render"]["ok"], true, "{done}");
        assert_eq!(done["render"]["volume"], 3000.0, "{done}");
        assert_eq!(app.doc.text, "// Width.\nwidth = 20; // [1:100]\ncube([width, 30, 5]);\n// mine\n");
        assert_eq!(app.doc.agent_marks, vec![2..3]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), saved, "the file is not saved");

        // A mistake comes back with its line, for the agent to fix.
        let failed = report(&call(&mut app, &ctx, "edit_code", json!({ "range": { "start_line": 3, "end_line": 3 }, "new": "cube([width, 30, 5]" })).unwrap());
        assert_eq!(failed["render"]["ok"], false, "{failed}");
        assert!(failed["render"]["error_line"].is_number(), "{failed}");
        let refused = call(&mut app, &ctx, "edit_code", json!({ "edits": [{ "old": "nothing", "new": "x" }] })).unwrap_err();
        assert!(refused.contains("0 times"), "{refused}");

        // Each edit is one step of the editor's undo history.
        let mut text = app.doc.text.clone();
        assert!(app.doc.editor.undo_step(&ctx, &mut text, true));
        assert!(text.contains("cube([width, 30, 5]);"), "{text}");
        assert!(app.doc.editor.undo_step(&ctx, &mut text, true));
        assert!(text.contains("cube([width, 10, 5]);\n// mine"), "{text}");
        app.doc.text = text;

        let set = report(&call(&mut app, &ctx, "set_parameters", json!({ "values": { "width": 2 } })).unwrap());
        assert_eq!(set["parameters"]["width"], 2.0);
        assert_eq!(set["render"]["volume"], 100.0, "{set}");
        assert!(call(&mut app, &ctx, "set_parameters", json!({ "values": { "height": 2 } })).unwrap_err().contains("height"));

        let other = dir.join("other.c2f");
        std::fs::write(&other, "sphere(1);\n").unwrap();
        let opened = report(&call(&mut app, &ctx, "open_file", json!({ "path": "other.c2f" })).unwrap());
        assert_eq!(opened["tab"], 1);
        assert!(call(&mut app, &ctx, "open_file", json!({ "path": "/etc/hosts" })).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tools_are_named_apart_from_the_built_in_ones() {
        let ctx = egui::Context::default();
        let tools = Live::new(&ctx).tools();
        let registry = c2f_agent::Registry::builtin().with(Box::new(tools)).expect("no name taken twice");
        for name in ["studio_state", "edit_code", "set_parameters", "view_image", "focus", "open_file"] {
            assert!(registry.names().iter().any(|n| n == name), "{name}");
        }
        for tool in Live::new(&ctx).tools().list() {
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
            assert!(tool["description"].as_str().is_some_and(|d| d.len() > 80), "{tool}");
        }
    }

    /// A folder of its own for a test, with `box.c2f` holding `text`.
    fn scratch(name: &str, text: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("c2f-live-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("box.c2f");
        std::fs::write(&file, text).unwrap();
        (dir, file)
    }

    const PARTS: &str = "part(\"base\") cube([20, 10, 5]);\ntranslate([0, 0, 10]) part(\"lid\") cube([20, 10, 2]);\n";

    #[test]
    fn the_user_s_typing_survives_the_agent_s_edits_landing_between_frames() {
        let (dir, file) = scratch("typing", "width = 20;\ncube([width, 10, 5]);\n// mine\n");
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, &file);
        // A frame as the app draws it: the agent's edits land first, then
        // the editor takes the keys pressed.
        let frame = |app: &mut StudioApp, events: Vec<egui::Event>| {
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                app.receive();
                app.serve_live(ctx);
                egui::CentralPanel::default().show(ctx, |ui| app.doc.editor.show(ui, &mut app.doc.text));
            });
        };
        frame(&mut app, Vec::new());
        // The user types at the start of the last line.
        let at = app.doc.text.find("// mine").unwrap();
        app.doc.editor.go_to(&ctx, &app.doc.text, at);
        app.doc.editor.focus(&ctx);
        frame(&mut app, Vec::new());

        let tools = app.live_tools();
        let edit = json!({ "edits": [{ "old": "cube([width, 10, 5]);", "new": "translate([0, 0, 1])\n    cube([width, 30, 5]);" }] });
        let caller = std::thread::spawn(move || tools.call("edit_code", &edit).expect("a live tool"));
        let start = Instant::now();
        let mut typed = "abcdefghij".chars();
        while !caller.is_finished() || typed.clone().next().is_some() {
            assert!(start.elapsed() < Duration::from_secs(60), "no answer");
            let events = typed.next().map(|c| vec![egui::Event::Text(c.to_string())]).unwrap_or_default();
            frame(&mut app, events);
            std::thread::sleep(Duration::from_millis(2));
        }
        let done = report(&caller.join().unwrap().unwrap());
        assert_eq!(done["edited"], true, "{done}");
        assert_eq!(app.doc.text, "width = 20;\ntranslate([0, 0, 1])\n    cube([width, 30, 5]);\nabcdefghij// mine\n");

        // An edit written against text the user has changed since is
        // refused, and the agent told to read it again.
        app.doc.text = app.doc.text.replace("30, 5", "31, 5");
        let refused = call(&mut app, &ctx, "edit_code", json!({ "edits": [{ "old": "cube([width, 30, 5]);", "new": "x" }] })).unwrap_err();
        assert!(refused.contains("studio_state"), "{refused}");
        let refused = call(&mut app, &ctx, "edit_code", json!({ "range": { "start_line": 9, "end_line": 9 }, "new": "x" })).unwrap_err();
        assert!(refused.contains("call studio_state again"), "{refused}");
        assert!(app.doc.text.contains("abcdefghij// mine"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_answer_s_edits_undo_as_one_step() {
        let (dir, file) = scratch("group", PARTS);
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, &file);
        let original = app.doc.text.clone();
        app.follow_agent_turn(&ctx, true);
        let first = report(&call(&mut app, &ctx, "edit_code", json!({ "edits": [{ "old": "[20, 10, 5]", "new": "[30, 10, 5]" }] })).unwrap());
        assert_eq!(first["part"], "base", "{first}");
        let second = report(&call(&mut app, &ctx, "edit_code", json!({ "edits": [{ "old": "[20, 10, 2]", "new": "[30, 10, 2]" }] })).unwrap());
        assert_eq!(second["part"], "lid", "{second}");
        // The second edit knows the parts before it: the lid grew.
        assert_eq!(second["parts_changed"], json!([{ "part": "lid", "before": { "min": [0.0, 0.0, 10.0], "max": [20.0, 10.0, 12.0] }, "after": { "min": [0.0, 0.0, 10.0], "max": [30.0, 10.0, 12.0] } }]));
        assert!(app.doc.agent_flash.is_some(), "the lid flashes");
        let marks = app.doc.agent_highlight.clone().unwrap();
        // The second edit's line tinted, both edits' lines marked.
        assert_eq!(marks.lines.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(1, 2)]);
        assert_eq!(marks.markers.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(0, 2)]);
        app.follow_agent_turn(&ctx, false);
        assert!(app.agent_undo_offer().unwrap().contains("2 edits"));

        let edited = app.doc.text.clone();
        app.undo_agent_change(&ctx);
        assert_eq!(app.doc.text, original);
        assert!(app.agent_undo_offer().is_none());
        // One step of the undo history: Cmd+Z brings the change back whole.
        let mut text = original.clone();
        assert!(app.doc.editor.undo_step(&ctx, &mut text, true));
        assert_eq!(text, edited);
        assert!(app.doc.editor.undo_step(&ctx, &mut text, false));
        assert_eq!(text, original);

        // After the user's own edit, the agent's change is not undone.
        app.follow_agent_turn(&ctx, true);
        call(&mut app, &ctx, "edit_code", json!({ "edits": [{ "old": "[20, 10, 5]", "new": "[25, 10, 5]" }] })).unwrap();
        app.follow_agent_turn(&ctx, false);
        app.doc.text.push_str("// mine\n");
        let before = app.doc.text.clone();
        assert!(app.agent_undo_offer().is_none());
        app.undo_agent_change(&ctx);
        assert_eq!(app.doc.text, before, "nothing undone");
        assert!(app.doc.console.rows().any(|r| r.1.contains("Cmd+Z")));
        assert!(app.doc.agent_highlight.as_ref().is_none_or(|h| !h.shows(&app.doc.text)), "the marks go");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reviewed_change_applies_once_accepted_and_not_when_declined() {
        let (dir, file) = scratch("review", PARTS);
        let ctx = egui::Context::default();
        let mut app = studio(&ctx, &file);
        app.settings.review_agent_edits = true;
        let original = app.doc.text.clone();
        // Calls `edit_code`, answering the review with `accept`.
        let reviewed = |app: &mut StudioApp, old: &str, new: &str, accept: bool| {
            let tools = app.live_tools();
            let args = json!({ "edits": [{ "old": old, "new": new }] });
            let caller = std::thread::spawn(move || tools.call("edit_code", &args).expect("a live tool"));
            let start = Instant::now();
            let mut asked = None;
            while !caller.is_finished() {
                assert!(start.elapsed() < Duration::from_secs(60), "no answer");
                app.receive();
                app.serve_live(&ctx);
                app.receive_review();
                if let Some(pending) = app.live.review.as_ref().filter(|p| p.settled()) {
                    asked = Some(pending.state.clone());
                    app.answer_review(&ctx, accept);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            (caller.join().unwrap(), asked.expect("a review"))
        };
        let (declined, state) = reviewed(&mut app, "[20, 10, 2]", "[20, 10, 4]", false);
        assert!(declined.unwrap_err().contains("declined"));
        assert_eq!((state.added, state.removed), (1, 1));
        assert!(state.proposed.contains("[20, 10, 4]"));
        assert_eq!(app.doc.text, original, "nothing changed");

        let (accepted, _) = reviewed(&mut app, "[20, 10, 2]", "[20, 10, 4]", true);
        let done = report(&accepted.unwrap());
        assert_eq!(done["render"]["ok"], true, "{done}");
        assert!(app.doc.text.contains("[20, 10, 4]"));
        assert!(app.live.review.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
