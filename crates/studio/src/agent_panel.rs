//! The Agent panel: a conversation with the agent, docked on the right of
//! the window beside the code and the view.
//!
//! The user's messages go to an [`Engine`], which streams back what
//! happens: the reply, delta by delta, and the tools the agent calls. The
//! panel reduces those events into a [`Transcript`], plain data apart
//! from egui, which it draws, and which it keeps for each file in the
//! user's settings folder, to resume the conversation later.
//!
//! The transcripts are kept there rather than beside the file, as the
//! assembly progress is: they hold the user's chat and the pictures the
//! tools drew, which have no place in a project folder that may be shared
//! or committed, and they would litter it with a file per design.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use c2f_agent::conversation::{Attachment, Engine, Event, Usage, UserMessage};
use eframe::egui::{
    self, Align, Button, Color32, ColorImage, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, RichText, Sense,
    Stroke, TextFormat, TextureHandle, TextureOptions, Vec2, text::LayoutJob,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_setup::{self, AgentSettings, Built, EngineRequest, KeyChange, Prober};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme;

/// What the conversation shows, in order, and what its turns took.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub entries: Vec<Entry>,
    /// The engine's session, to resume the conversation where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The tokens of every turn, added up, and their cost when the engine
    /// says.
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub cost_usd: f64,
}

/// A step of the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    /// What the user sent; `shown` is what the panel shows of it instead
    /// of its text, for a prompt sent by its button.
    User {
        message: UserMessage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shown: Option<String>,
    },
    /// The assistant's reply, in Markdown.
    Assistant { text: String },
    Tool(ToolCall),
    /// What the panel says itself: the turn stopped, or failed.
    Notice { text: String, error: bool },
}

/// A tool the agent called, and what it gave back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub status: Status,
    /// MCP content items: text, or images.
    #[serde(default)]
    pub content: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Ok,
    Error,
    /// The turn ended before the tool gave anything back.
    Stopped,
}

impl Transcript {
    pub fn push_user(&mut self, message: UserMessage, shown: Option<String>) {
        self.entries.push(Entry::User { message, shown });
    }

    /// Takes in what the engine streamed; whether it ends the turn.
    pub fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Text { delta } => {
                match self.entries.last_mut() {
                    Some(Entry::Assistant { text }) => text.push_str(&delta),
                    _ => self.entries.push(Entry::Assistant { text: delta }),
                }
                false
            }
            Event::ToolCall { id, name, input } => {
                self.entries.push(Entry::Tool(ToolCall { id, name, input, status: Status::Running, content: Vec::new() }));
                false
            }
            Event::ToolResult { id, content, is_error } => {
                let call = self.entries.iter_mut().rev().find_map(|entry| match entry {
                    Entry::Tool(call) if call.id == id => Some(call),
                    _ => None,
                });
                if let Some(call) = call {
                    call.status = if is_error { Status::Error } else { Status::Ok };
                    call.content = content;
                }
                false
            }
            Event::Done { usage, cost_usd, session_id } => {
                if let Some(usage) = usage {
                    self.usage.input_tokens += usage.input_tokens;
                    self.usage.output_tokens += usage.output_tokens;
                }
                self.cost_usd += cost_usd.unwrap_or(0.0);
                if session_id.is_some() {
                    self.session_id = session_id;
                }
                self.settle();
                true
            }
            Event::Error { message } => {
                self.settle();
                self.entries.push(Entry::Notice { text: message, error: true });
                true
            }
        }
    }

    /// The turn was stopped: the tools still running stop with it, and a
    /// note says so.
    pub fn stopped(&mut self) {
        self.settle();
        self.entries.push(Entry::Notice { text: tr("Stopped.", "Arrêté.").into(), error: false });
    }

    /// The tools still running at the end of a turn never will finish.
    fn settle(&mut self) {
        for entry in &mut self.entries {
            if let Entry::Tool(call) = entry {
                if call.status == Status::Running {
                    call.status = Status::Stopped;
                }
            }
        }
    }
}

/// The line (from 1) an `edit_code` call changed, once it succeeded: its
/// input's `line` or `start_line`, else its result's.
pub fn edited_line(call: &ToolCall) -> Option<usize> {
    if call.name != "edit_code" || call.status != Status::Ok {
        return None;
    }
    let line = |value: &Value| {
        ["line", "start_line"].iter().find_map(|key| value.get(key)?.as_u64()).map(|n| n as usize)
    };
    line(&call.input).or_else(|| {
        call.content.iter().find_map(|item| line(&serde_json::from_str(item.get("text")?.as_str()?).ok()?))
    })
}

/// A tool's input on one line: `key: value` for each argument, the
/// strings cut to their first line, the whole cut to `max` characters.
pub fn summary(input: &Value, max: usize) -> String {
    let short = |value: &Value| match value {
        Value::String(s) => s.lines().next().unwrap_or("").to_string(),
        other => other.to_string(),
    };
    let text = match input {
        Value::Object(map) => map.iter().map(|(key, value)| format!("{key}: {}", short(value))).collect::<Vec<_>>().join(", "),
        other => short(other),
    };
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max.saturating_sub(1)).collect::<String>())
    } else {
        text
    }
}

/// The byte offset where `line` (from 1) of `text` starts; the end of the
/// text for a line past it.
pub fn line_offset(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    text.match_indices('\n').nth(line - 2).map_or(text.len(), |(at, _)| at + 1)
}

// ---- Prompts ----

/// The buttons that send one of the agent's prompts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Plan,
    Design,
    Electronics,
}

impl Shortcut {
    const ALL: [Shortcut; 3] = [Shortcut::Plan, Shortcut::Design, Shortcut::Electronics];

    /// The MCP prompt it sends.
    fn prompt(self) -> &'static str {
        match self {
            Shortcut::Plan => "plan_object",
            Shortcut::Design => "design_object",
            Shortcut::Electronics => "add_electronics",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Shortcut::Plan => tr("Plan", "Planifier"),
            Shortcut::Design => tr("Design", "Concevoir"),
            Shortcut::Electronics => tr("Electronics", "Électronique"),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Shortcut::Plan => tr(
                "Plan an object before drawing it: the agent asks what it must do and writes its brief",
                "Planifier un objet avant de le dessiner : l'agent demande ce qu'il doit faire et en écrit le cahier des charges",
            ),
            Shortcut::Design => tr(
                "Design an object as a parametric design, checked from every side",
                "Concevoir un objet en design paramétrique, vérifié sous tous les angles",
            ),
            Shortcut::Electronics => tr(
                "Give this design its electronics: parts placed, wired and checked",
                "Donner à ce design son électronique : composants placés, câblés et vérifiés",
            ),
        }
    }

    /// What the inline field asks for.
    fn question(self) -> &'static str {
        match self {
            Shortcut::Plan => tr("The idea, in a sentence or two", "L'idée, en une phrase ou deux"),
            Shortcut::Design => tr("What the object is and does", "Ce qu'est l'objet et ce qu'il fait"),
            Shortcut::Electronics => tr("The components: links or library parts", "Les composants : liens ou pièces de la bibliothèque"),
        }
    }

    /// The prompt's arguments: the user's `answer`, and the active file and
    /// its folder; why it cannot be sent.
    fn arguments(self, answer: &str, file: Option<&Path>, folder: Option<&Path>) -> Result<Value, String> {
        let folder = folder.map(|f| f.display().to_string());
        Ok(match self {
            Shortcut::Plan => json!({ "idea": answer, "folder": folder }),
            Shortcut::Design => json!({ "description": answer, "folder": folder }),
            Shortcut::Electronics => {
                let Some(design) = file else {
                    return Err(tr(
                        "Save the design first: its electronics go with its file",
                        "Enregistrez d'abord le design : son électronique accompagne son fichier",
                    )
                    .into());
                };
                json!({ "design": design.display().to_string(), "components": answer })
            }
        })
    }
}

/// The text of the prompt `shortcut` sends, filled in with `answer`, the
/// active `file` and its `folder`; why it cannot be sent.
pub fn prompt_text(shortcut: Shortcut, answer: &str, file: Option<&Path>, folder: Option<&Path>) -> Result<String, String> {
    let mut arguments = shortcut.arguments(answer.trim(), file, folder)?;
    // The prompts take strings only: no folder is no argument.
    if let Value::Object(map) = &mut arguments {
        map.retain(|_, value| !value.is_null());
    }
    let prompt = c2f_agent::prompt(shortcut.prompt(), &arguments)?;
    let texts: Vec<&str> = prompt["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message["content"]["text"].as_str())
        .collect();
    Ok(texts.join("\n\n"))
}

// ---- Engines ----

/// Stands in for an engine in screenshot runs and tests: it says no
/// engine is configured, and gives the message back.
pub struct EchoEngine;

impl Engine for EchoEngine {
    fn send(&mut self, message: UserMessage, events: mpsc::Sender<Event>) -> Result<(), String> {
        let said = message.text.lines().next().unwrap_or("").to_string();
        let delta = tr!(
            "No engine configured yet: no agent can answer. You said: {said}",
            "Aucun moteur configuré pour l'instant : aucun agent ne peut répondre. Vous avez dit : {said}"
        );
        // A panel gone stops nothing.
        let _ = events.send(Event::Text { delta });
        let _ = events.send(Event::Done { usage: None, cost_usd: None, session_id: None });
        Ok(())
    }

    fn stop(&mut self) {}

    fn name(&self) -> &str {
        "echo"
    }
}

// ---- Persistence ----

/// The folder the conversations are kept in, in the settings folder, but
/// in screenshot runs, which neither read nor change them.
pub fn store() -> Option<PathBuf> {
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    Some(crate::recent::settings_dir()?.join("conversations"))
}

/// The file keeping the conversation about `file`: its name, for whoever
/// looks into the folder, and a hash of its path, which sets apart two
/// files of the same name.
fn conversation_path(store: &Path, file: &Path) -> PathBuf {
    // FNV-1a: the same hash from one build to the next.
    let hash = file.to_string_lossy().bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let stem = file.file_stem().map_or_else(|| "untitled".into(), |s| s.to_string_lossy().into_owned());
    store.join(format!("{stem}-{hash:016x}.json"))
}

// ---- The panel ----

/// The id of the message field.
fn input_id() -> Id {
    Id::new("agent-input")
}

/// The id of the inline field of a prompt's button.
fn answer_id() -> Id {
    Id::new("agent-prompt-answer")
}

pub struct AgentPanel {
    /// Shown on the right of the window.
    pub open: bool,
    /// Makes an engine: the first, a fresh one for each new conversation,
    /// which forgets the last one's session, and one carrying the
    /// conversation on when the settings change or Resume brings it back.
    make_engine: Box<dyn FnMut(&EngineRequest) -> Built>,
    engine: Box<dyn Engine>,
    /// The engine's name and model, and why none can answer.
    engine_label: String,
    engine_problem: Option<String>,
    /// The agent's settings and the library folders the engine was made
    /// with, and the check of the engines it saw.
    agent: AgentSettings,
    libraries: Vec<PathBuf>,
    built_for: Option<(AgentSettings, Vec<PathBuf>, u64)>,
    /// Checks which engines this computer has, once the panel or the
    /// settings are first shown, and the `claude` path it checked.
    prober: Prober,
    probed: Option<Option<PathBuf>>,
    /// The API key typed in the settings, until saved.
    key_input: String,
    /// The user asked for the Agent settings.
    pub open_settings: bool,
    transcript: Transcript,
    /// The events of the turn under way.
    events: Option<Receiver<Event>>,
    /// The file the conversation is about, which keeps it.
    owner: Option<PathBuf>,
    /// The active tab's file and folder.
    file: Option<PathBuf>,
    folder: Option<PathBuf>,
    /// Where the conversations are kept; `None` for this run only.
    store: Option<PathBuf>,
    input: String,
    attachments: Vec<Attachment>,
    /// The prompt whose argument the inline field asks for, and the answer
    /// typed so far.
    asking: Option<(Shortcut, String)>,
    /// Why the last thing asked could not be done.
    problem: Option<String>,
    /// The tool calls unfolded, by id.
    unfolded: HashSet<String>,
    /// The images of the tools' results, by call id and place; `None` for
    /// one that could not be read.
    thumbnails: HashMap<(String, usize), Option<TextureHandle>>,
    focus_input: bool,
}

impl AgentPanel {
    /// A closed panel, its engines made by `make_engine`, its
    /// conversations kept in `store`.
    pub fn new(make_engine: impl FnMut(&EngineRequest) -> Built + 'static, store: Option<PathBuf>) -> Self {
        let mut panel = AgentPanel {
            open: false,
            make_engine: Box::new(make_engine),
            engine: Box::new(EchoEngine),
            engine_label: String::new(),
            engine_problem: None,
            agent: AgentSettings::default(),
            libraries: Vec::new(),
            built_for: None,
            prober: Prober::default(),
            probed: None,
            key_input: String::new(),
            open_settings: false,
            transcript: Transcript::default(),
            events: None,
            owner: None,
            file: None,
            folder: None,
            store,
            input: String::new(),
            attachments: Vec::new(),
            asking: None,
            problem: None,
            unfolded: HashSet::new(),
            thumbnails: HashMap::new(),
            focus_input: false,
        };
        panel.build();
        panel
    }

    /// Makes the engine for the settings, the engines found and the
    /// conversation as it is; what the user should know of it goes into
    /// the conversation.
    fn build(&mut self) {
        let (generation, probe) = self.prober.current();
        let request = EngineRequest {
            agent: &self.agent,
            libraries: &self.libraries,
            probe: probe.as_ref(),
            folder: self.folder.as_deref(),
            transcript: &self.transcript,
        };
        let built = (self.make_engine)(&request);
        self.engine = built.engine;
        self.engine_label = built.label;
        self.engine_problem = built.problem;
        if let Some(notice) = built.notice.filter(|_| !self.transcript.entries.is_empty()) {
            self.transcript.entries.push(Entry::Notice { text: notice, error: false });
        }
        self.built_for = Some((self.agent.clone(), self.libraries.clone(), generation));
    }

    /// Takes the agent's settings and the user's library folders, every
    /// frame: the engines are checked once the panel shows, and again
    /// when the path to `claude` changes; the engine is made again, the
    /// conversation carried on, when they or the engines found change.
    pub fn configure(&mut self, ctx: &egui::Context, agent: &AgentSettings, libraries: &[PathBuf]) {
        if self.agent != *agent || self.libraries != libraries {
            self.agent = agent.clone();
            self.libraries = libraries.to_vec();
        }
        let checked = self.probed.as_ref().is_some_and(|path| *path == self.agent.claude_path);
        if (self.open || self.probed.is_some()) && !checked {
            self.probed = Some(self.agent.claude_path.clone());
            self.prober.check(ctx, self.agent.claude_path.clone(), KeyChange::Keep);
        }
        let now = (self.agent.clone(), self.libraries.clone(), self.prober.current().0);
        if self.built_for.as_ref() != Some(&now) && !self.running() {
            self.build();
        }
    }

    /// The Agent page of the settings.
    pub fn settings_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, agent: &mut AgentSettings) {
        if self.probed.is_none() {
            self.probed = Some(agent.claude_path.clone());
        }
        agent_setup::page(ui, ctx, agent, &self.prober, &mut self.key_input);
    }

    /// What the conversation holds so far, for the live tools to read.
    #[allow(dead_code, reason = "the live tools read it once they are wired")]
    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    /// Whether a turn is under way.
    pub fn running(&self) -> bool {
        self.events.is_some()
    }

    /// Shows the panel, its message field ready, or hides it.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus_input = self.open;
    }

    /// Adds something to send with the next message: a picture of the
    /// view, or what the user selected.
    #[allow(dead_code, reason = "the view's selection is attached once it is wired")]
    pub fn attach(&mut self, attachment: Attachment) {
        if !self.attachments.contains(&attachment) {
            self.attachments.push(attachment);
        }
    }

    /// Follows the active tab, its `file` and `folder`, every frame. A
    /// conversation about another file is put away once its turn is over,
    /// for Resume to bring back; one begun in an untitled tab goes with
    /// the file it is saved as.
    pub fn set_tab(&mut self, file: Option<&Path>, folder: Option<PathBuf>) {
        self.folder = folder;
        if self.file.as_deref() != file {
            self.file = file.map(Path::to_path_buf);
        }
        if self.owner.as_deref() == file || self.running() {
            return;
        }
        if self.owner.is_none() && !self.transcript.entries.is_empty() {
            self.owner = self.file.clone();
            self.save();
        } else {
            self.fresh(self.file.clone());
        }
    }

    /// Sends the user's message, with the attachments, and starts a turn.
    pub fn send(&mut self, text: &str) {
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        let message = UserMessage { text: text.trim().to_string(), attachments: std::mem::take(&mut self.attachments) };
        self.start(message, None);
    }

    /// Starts a turn with `message`, `shown` in its place if given.
    fn start(&mut self, message: UserMessage, shown: Option<String>) {
        if self.running() {
            return;
        }
        if self.transcript.entries.is_empty() {
            self.owner = self.file.clone();
        }
        self.problem = None;
        let (sender, receiver) = mpsc::channel();
        self.transcript.push_user(message.clone(), shown);
        match self.engine.send(message, sender) {
            Ok(()) => self.events = Some(receiver),
            Err(why) => self.transcript.entries.push(Entry::Notice { text: why, error: true }),
        }
        self.save();
    }

    /// Takes in what the engine streamed since the last frame; whether
    /// anything came.
    pub fn poll(&mut self) -> bool {
        let Some(events) = &self.events else { return false };
        let mut changed = false;
        let mut over = false;
        loop {
            match events.try_recv() {
                Ok(event) => {
                    changed = true;
                    if self.transcript.apply(event) {
                        over = true;
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                // The engine let go without a word: the turn is over.
                Err(TryRecvError::Disconnected) => {
                    self.transcript.apply(Event::Done { usage: None, cost_usd: None, session_id: None });
                    changed = true;
                    over = true;
                    break;
                }
            }
        }
        if over {
            self.events = None;
            self.save();
        }
        changed
    }

    /// Stops the turn under way at once: the engine is told, and what it
    /// still sends is dropped.
    pub fn stop(&mut self) {
        if !self.running() {
            return;
        }
        self.engine.stop();
        self.events = None;
        self.transcript.stopped();
        self.save();
    }

    /// Starts over, the last conversation kept for Resume until the first
    /// message of this one.
    pub fn new_conversation(&mut self) {
        self.stop();
        self.fresh(self.file.clone());
    }

    /// An empty conversation about `owner`, with a fresh engine.
    fn fresh(&mut self, owner: Option<PathBuf>) {
        self.transcript = Transcript::default();
        self.build();
        self.owner = owner;
        self.problem = None;
        self.unfolded.clear();
        self.thumbnails.clear();
    }

    /// Where the conversation about the active file is kept.
    fn saved_path(&self) -> Option<PathBuf> {
        Some(conversation_path(self.store.as_deref()?, self.file.as_deref()?))
    }

    /// Whether Resume has a conversation to bring back.
    pub fn can_resume(&self) -> bool {
        !self.running() && self.transcript.entries.is_empty() && self.saved_path().is_some_and(|p| p.is_file())
    }

    /// Brings back the last conversation about the active file.
    pub fn resume(&mut self) {
        let Some(path) = self.saved_path().filter(|_| !self.running()) else { return };
        let read = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
            serde_json::from_slice::<Transcript>(&bytes).map_err(|e| e.to_string())
        });
        match read {
            Ok(transcript) => {
                self.fresh(self.file.clone());
                self.transcript = transcript;
                // Its session resumed, or its texts carried on.
                self.build();
            }
            Err(err) => {
                self.problem = Some(tr!("Cannot read the last conversation: {err}", "Impossible de lire la dernière conversation : {err}"));
            }
        }
    }

    /// Keeps the conversation about its file; an empty one keeps nothing,
    /// leaving the last for Resume.
    fn save(&mut self) {
        let (Some(store), Some(owner)) = (&self.store, &self.owner) else { return };
        if self.transcript.entries.is_empty() {
            return;
        }
        let path = conversation_path(store, owner);
        let written = std::fs::create_dir_all(store)
            .and_then(|()| std::fs::write(&path, serde_json::to_vec(&self.transcript).unwrap_or_default()));
        if let Err(err) = written {
            self.problem = Some(tr!("Cannot save the conversation: {err}", "Impossible d'enregistrer la conversation : {err}"));
        }
    }

    /// Sends the prompt asked for in the inline field.
    fn send_prompt(&mut self, shortcut: Shortcut, answer: &str) {
        if answer.trim().is_empty() {
            return;
        }
        match prompt_text(shortcut, answer, self.file.as_deref(), self.folder.as_deref()) {
            Ok(text) => {
                self.asking = None;
                let message = UserMessage { text, attachments: std::mem::take(&mut self.attachments) };
                self.start(message, Some(format!("{} · {}", shortcut.label(), answer.trim())));
            }
            Err(why) => self.problem = Some(why),
        }
    }

    // ---- Drawing ----

    /// Takes in the engine's events, and shows the panel when open; the
    /// line (from 1) of the code to show, when an edit's link is clicked.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<usize> {
        self.poll();
        if self.running() {
            // The events come from another thread, which cannot wake egui.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if !self.open {
            return None;
        }
        let palette = theme::palette();
        let width = ctx.screen_rect().width();
        let frame = Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0, palette.border))
            .inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 8 });
        let mut go_to = None;
        egui::SidePanel::right("agent")
            .frame(frame)
            .resizable(true)
            .default_width(380.0)
            .width_range(300.0..=(width * 0.5).max(320.0))
            .show(ctx, |ui| {
                self.header(ui);
                ui.add_space(4.0);
                self.prompt_row(ui);
                egui::TopBottomPanel::bottom("agent-composer")
                    .frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| self.composer(ui));
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(palette.editor).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10)))
                    .show_inside(ui, |ui| go_to = self.conversation(ui));
            });
        go_to
    }

    /// The title, the engine's name, and New, Resume and Close.
    fn header(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            icons::paint(ui.painter(), rect, Icon::Agent, palette.text_dim);
            ui.label(RichText::new("Agent").strong().size(13.5).color(palette.text));
            ui.label(RichText::new(&self.engine_label).size(11.5).color(palette.text_faint));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let close = ui.add(Button::new(RichText::new("×").size(16.0).color(palette.text_dim)).frame(false));
                if close.on_hover_text(tr("Close the panel", "Fermer le panneau")).clicked() {
                    self.open = false;
                }
                let resume = ui.add_enabled(self.can_resume(), Button::new(RichText::new(tr("Resume last", "Reprendre")).size(12.0)));
                let hint = tr("Bring back the last conversation about this file", "Reprendre la dernière conversation sur ce fichier");
                if resume.on_hover_text(hint).clicked() {
                    self.resume();
                }
                let started = !self.transcript.entries.is_empty();
                let new = ui.add_enabled(started, Button::new(RichText::new(tr("New", "Nouvelle")).size(12.0)));
                if new.on_hover_text(tr("Start a new conversation", "Commencer une nouvelle conversation")).clicked() {
                    self.new_conversation();
                }
            });
        });
        if let Some(problem) = &self.engine_problem {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(problem).size(12.0).color(palette.text_dim));
                if ui.small_button(tr("Open settings", "Ouvrir les réglages")).clicked() {
                    self.open_settings = true;
                }
            });
        }
    }

    /// The prompts' buttons, and the field one of them asks with.
    fn prompt_row(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(tr("Start:", "Démarrer :")).size(12.0).color(palette.text_dim));
            for shortcut in Shortcut::ALL {
                let on = self.asking.as_ref().is_some_and(|(s, _)| *s == shortcut);
                let button = Button::selectable(on, RichText::new(shortcut.label()).size(12.0));
                if ui.add_enabled(!self.running(), button).on_hover_text(shortcut.hint()).clicked() {
                    self.asking = if on { None } else { Some((shortcut, String::new())) };
                    self.problem = None;
                    ui.memory_mut(|m| m.request_focus(answer_id()));
                }
            }
        });
        let Some((shortcut, mut answer)) = self.asking.take() else { return };
        let mut send = false;
        let mut cancel = false;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = (ui.available_width() - 120.0).max(80.0);
            let field = egui::TextEdit::singleline(&mut answer).id(answer_id()).hint_text(shortcut.question()).desired_width(width);
            let response = ui.add(field);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                send = true;
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            send |= ui.add_enabled(!answer.trim().is_empty(), Button::new(tr("Send", "Envoyer"))).clicked();
            cancel |= ui.button(tr("Cancel", "Annuler")).clicked();
        });
        if !cancel {
            self.asking = Some((shortcut, answer.clone()));
        }
        if send {
            self.send_prompt(shortcut, &answer);
        }
    }

    /// The attachments, the message field, and Send or Stop.
    fn composer(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        if let Some(problem) = &self.problem {
            ui.label(RichText::new(problem).size(12.0).color(palette.error));
        }
        if !self.attachments.is_empty() {
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (index, attachment) in self.attachments.iter().enumerate() {
                    if chip(ui, attachment, true) {
                        remove = Some(index);
                    }
                }
            });
            if let Some(index) = remove {
                self.attachments.remove(index);
            }
            ui.add_space(4.0);
        }
        // Enter sends; Shift+Enter goes to the field, a new line.
        let focused = ui.memory(|m| m.has_focus(input_id()));
        let enter = focused
            && ui.input_mut(|i| {
                let before = i.events.len();
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift)
                });
                i.events.len() != before
            });
        let field = egui::TextEdit::multiline(&mut self.input)
            .id(input_id())
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text(tr("Ask the agent… (Enter sends, Shift+Enter: new line)", "Demandez à l'agent… (Entrée envoie, Maj+Entrée : nouvelle ligne)"));
        let response = ui.add(field);
        if std::mem::take(&mut self.focus_input) {
            response.request_focus();
        }
        ui.add_space(4.0);
        let mut send = enter && !self.running();
        ui.horizontal(|ui| {
            if self.running() {
                ui.add(egui::Spinner::new().size(12.0).color(palette.accent_strong));
                ui.label(RichText::new(tr("Working…", "En cours…")).size(12.0).color(palette.accent_strong));
            } else if self.transcript.usage != Usage::default() {
                let usage = self.transcript.usage;
                let mut text = tr!("{} in · {} out", "{} entrés · {} sortis", usage.input_tokens, usage.output_tokens);
                if self.transcript.cost_usd > 0.0 {
                    text.push_str(&format!(" · ${:.2}", self.transcript.cost_usd));
                }
                ui.label(RichText::new(text).size(11.5).color(palette.text_faint)).on_hover_text(tr("Tokens of the conversation", "Jetons de la conversation"));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.running() {
                    if ui.button(tr("Stop", "Arrêter")).on_hover_text(tr("Stop the agent's turn", "Arrêter le tour de l'agent")).clicked() {
                        self.stop();
                    }
                } else {
                    let ready = !self.input.trim().is_empty() || !self.attachments.is_empty();
                    let button = Button::new(RichText::new(tr("Send", "Envoyer")).color(palette.on_accent).strong())
                        .fill(palette.accent)
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(6));
                    send |= ui.add_enabled(ready, button).clicked();
                }
            });
        });
        if send {
            let text = std::mem::take(&mut self.input);
            self.send(&text);
            if !self.running() && self.transcript.entries.is_empty() {
                // Nothing was sent: the text stays.
                self.input = text;
            }
            response.request_focus();
        }
    }

    /// The conversation, scrolled to its end as it grows.
    fn conversation(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        let palette = theme::palette();
        if self.transcript.entries.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr(
                    "Ask the agent to design, change or check the model: it reads and edits the code beside you. The buttons above start from an idea.",
                    "Demandez à l'agent de concevoir, modifier ou vérifier le modèle : il lit et modifie le code à côté de vous. Les boutons ci-dessus partent d'une idée.",
                ))
                .size(12.5)
                .color(palette.text_faint),
            );
            return None;
        }
        let running = self.running();
        let AgentPanel { transcript, unfolded, thumbnails, .. } = self;
        let mut go_to = None;
        egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for entry in &transcript.entries {
                match entry {
                    Entry::User { message, shown } => user_message(ui, message, shown.as_deref()),
                    Entry::Assistant { text } => markdown(ui, text),
                    Entry::Tool(call) => go_to = go_to.or(tool_row(ui, call, unfolded, thumbnails)),
                    Entry::Notice { text, error } => {
                        let color = if *error { palette.error } else { palette.text_faint };
                        ui.label(RichText::new(text).size(12.0).color(color));
                    }
                }
            }
            let waiting = !transcript.entries.iter().any(|e| matches!(e, Entry::Tool(c) if c.status == Status::Running));
            if running && waiting {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0).color(palette.text_dim));
                    ui.label(RichText::new(tr("Thinking…", "Réflexion…")).size(12.0).color(palette.text_dim));
                });
            }
        });
        go_to
    }
}

/// A message of the user's, in its bubble, with what went with it.
fn user_message(ui: &mut egui::Ui, message: &UserMessage, shown: Option<&str>) {
    let palette = theme::palette();
    Frame::new()
        .fill(palette.raised)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr("You", "Vous")).size(11.0).color(palette.text_dim));
            ui.label(RichText::new(shown.unwrap_or(&message.text)).color(palette.text));
            if !message.attachments.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for attachment in &message.attachments {
                        chip(ui, attachment, false);
                    }
                });
            }
        });
}

/// An attachment as a chip, with a remove button when `removable`;
/// whether that was clicked.
fn chip(ui: &mut egui::Ui, attachment: &Attachment, removable: bool) -> bool {
    let palette = theme::palette();
    let (label, hint) = match attachment {
        Attachment::Context { label, text } => (label.clone(), text.chars().take(400).collect::<String>()),
        Attachment::Image { png } => (
            tr!("View · {} KB", "Vue · {} Ko", png.len().div_ceil(1024)),
            tr("A picture of the view", "Une image de la vue").to_string(),
        ),
    };
    let mut removed = false;
    Frame::new()
        .fill(palette.accent_muted)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin { left: 8, right: if removable { 4 } else { 8 }, top: 1, bottom: 1 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.label(RichText::new(label).size(11.5).color(icons::on_muted())).on_hover_text(hint);
                if removable {
                    let remove = ui.add(Button::new(RichText::new("×").size(12.0).color(icons::on_muted())).frame(false));
                    removed = remove.on_hover_text(tr("Do not send it", "Ne pas l'envoyer")).clicked();
                }
            });
        });
    removed
}

/// A tool call as a row: its status, name and arguments, which unfolds
/// to its input and result; the pictures it gave back under it, and for
/// an edit, a link to the line; the line clicked.
fn tool_row(
    ui: &mut egui::Ui,
    call: &ToolCall,
    unfolded: &mut HashSet<String>,
    thumbnails: &mut HashMap<(String, usize), Option<TextureHandle>>,
) -> Option<usize> {
    let palette = theme::palette();
    let open = unfolded.contains(&call.id);
    let mut go_to = None;
    Frame::new()
        .fill(palette.faint_bg)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                icons::paint(ui.painter(), rect, if open { Icon::FoldDown } else { Icon::FoldRight }, palette.text_dim);
                status_mark(ui, call.status);
                ui.label(RichText::new(&call.name).font(FontId::monospace(12.0)).color(palette.text_strong));
                let summary = summary(&call.input, 120);
                ui.add(egui::Label::new(RichText::new(summary).size(12.0).color(palette.text_dim)).truncate());
            });
            let toggle = ui.interact(row.response.rect, Id::new(("agent-tool", &call.id)), Sense::click());
            if toggle.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if open {
                    unfolded.remove(&call.id);
                } else {
                    unfolded.insert(call.id.clone());
                }
            }
            if let Some(line) = edited_line(call) {
                if ui.link(tr!("Edited line {line}: show it", "Ligne {line} modifiée : l'afficher")).clicked() {
                    go_to = Some(line);
                }
            }
            let images: Vec<(usize, &Value)> =
                call.content.iter().enumerate().filter(|(_, item)| item["type"] == "image").collect();
            if !images.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (index, item) in images {
                        let key = (call.id.clone(), index);
                        let texture = thumbnails
                            .entry(key)
                            .or_insert_with(|| thumbnail(ui.ctx(), &format!("agent-{}-{index}", call.id), item));
                        if let Some(texture) = texture {
                            ui.add(egui::Image::new(&*texture).max_width(200.0).max_height(150.0).corner_radius(CornerRadius::same(4)));
                        }
                    }
                });
            }
            if open {
                ui.label(RichText::new(tr("Input", "Entrée")).size(11.0).color(palette.text_dim));
                code_block(ui, &serde_json::to_string_pretty(&call.input).unwrap_or_default());
                if !call.content.is_empty() {
                    ui.label(RichText::new(tr("Result", "Résultat")).size(11.0).color(palette.text_dim));
                    for item in call.content.iter().filter(|item| item["type"] != "image") {
                        let text = item["text"].as_str().map_or_else(|| item.to_string(), pretty);
                        code_block(ui, &text);
                    }
                }
            }
        });
    go_to
}

/// The text of a result, pretty-printed when it is JSON, cut when long.
fn pretty(text: &str) -> String {
    const MAX: usize = 6000;
    let text = serde_json::from_str::<Value>(text).ok().and_then(|v| serde_json::to_string_pretty(&v).ok()).unwrap_or_else(|| text.to_string());
    if text.chars().count() > MAX { format!("{}\n…", text.chars().take(MAX).collect::<String>()) } else { text }
}

/// A call's status: a spinner while it runs, then a mark in the color of
/// how it went.
fn status_mark(ui: &mut egui::Ui, status: Status) {
    let palette = theme::palette();
    if status == Status::Running {
        ui.add(egui::Spinner::new().size(12.0).color(palette.accent_strong));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    match status {
        Status::Ok => {
            let stroke = Stroke::new(1.8, palette.success);
            painter.line_segment([c + Vec2::new(-4.0, 0.0), c + Vec2::new(-1.0, 3.0)], stroke);
            painter.line_segment([c + Vec2::new(-1.0, 3.0), c + Vec2::new(4.5, -3.5)], stroke);
        }
        Status::Error => {
            let stroke = Stroke::new(1.8, palette.error);
            painter.line_segment([c + Vec2::splat(-3.5), c + Vec2::splat(3.5)], stroke);
            painter.line_segment([c + Vec2::new(-3.5, 3.5), c + Vec2::new(3.5, -3.5)], stroke);
        }
        Status::Stopped | Status::Running => {
            painter.rect_filled(egui::Rect::from_center_size(c, Vec2::splat(7.0)), CornerRadius::same(1), palette.text_faint);
        }
    }
}

/// Code or JSON on the editor's colors, wrapped.
fn code_block(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette();
    Frame::new()
        .fill(palette.faint_bg)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).font(FontId::monospace(12.0)).color(palette.code_text)).wrap());
        });
}

/// The picture of an image content item, as a texture named `name`.
fn thumbnail(ctx: &egui::Context, name: &str, item: &Value) -> Option<TextureHandle> {
    let png = base64_decode(item["data"].as_str()?)?;
    let (size, rgba) = decode_png(&png)?;
    Some(ctx.load_texture(name, ColorImage::from_rgba_unmultiplied(size, &rgba), TextureOptions::LINEAR))
}

/// Standard base64, padded or not; `None` for anything else.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for c in text.bytes().filter(|&c| c != b'=' && !c.is_ascii_whitespace()) {
        let digit = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(digit);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

/// The size and RGBA pixels of a PNG.
fn decode_png(bytes: &[u8]) -> Option<([usize; 2], Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(([info.width as usize, info.height as usize], rgba))
}

// ---- Markdown ----

/// A block of the assistant's Markdown.
#[derive(Debug, PartialEq)]
enum Block {
    Paragraph(String),
    Heading(String),
    /// A list item, its marker (`•` or `1.`) and its text.
    Item(String, String),
    Code(String),
}

/// The blocks of `text`: paragraphs, headings, list items and fenced
/// code, which runs to the end while its fence is not closed yet, as the
/// reply streams in.
fn blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    // Whether the next plain line carries on the last block.
    let mut joinable = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(lines) = &mut code {
            if trimmed.starts_with("```") {
                blocks.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if trimmed.starts_with("```") {
            code = Some(Vec::new());
            joinable = false;
        } else if trimmed.is_empty() {
            joinable = false;
        } else if let Some(heading) = trimmed.strip_prefix('#') {
            blocks.push(Block::Heading(heading.trim_start_matches('#').trim().to_string()));
            joinable = false;
        } else if let Some((marker, item)) = list_item(trimmed) {
            blocks.push(Block::Item(marker, item.to_string()));
            joinable = true;
        } else {
            match blocks.last_mut() {
                Some(Block::Paragraph(text) | Block::Item(_, text)) if joinable => {
                    text.push(' ');
                    text.push_str(trimmed.trim_end());
                }
                _ => blocks.push(Block::Paragraph(trimmed.trim_end().to_string())),
            }
            joinable = true;
        }
    }
    if let Some(lines) = code {
        blocks.push(Block::Code(lines.join("\n")));
    }
    blocks
}

/// A list item's marker and text: `- a`, `* a`, `+ a` or `1. a`.
fn list_item(line: &str) -> Option<(String, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(item) = line.strip_prefix(bullet) {
            return Some(("•".into(), item));
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = &line[digits..];
    let item = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    (digits > 0).then(|| (format!("{}.", &line[..digits]), item))
}

/// A run of inline Markdown.
#[derive(Debug, PartialEq)]
enum Span<'a> {
    Plain(&'a str),
    Bold(&'a str),
    Code(&'a str),
}

/// The runs of `text`: `**bold**`, `` `code` ``, and the plain text
/// between; a marker never closed is plain text.
fn spans(text: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let code = rest.find('`').map(|at| (at, "`"));
        let bold = rest.find("**").map(|at| (at, "**"));
        let marker = match (code, bold) {
            (Some(c), Some(b)) => Some(if c.0 <= b.0 { c } else { b }),
            (c, b) => c.or(b),
        };
        let Some((at, marker)) = marker else {
            spans.push(Span::Plain(rest));
            break;
        };
        let inner = &rest[at + marker.len()..];
        let Some(end) = inner.find(marker).filter(|&end| end > 0) else {
            spans.push(Span::Plain(rest));
            break;
        };
        if at > 0 {
            spans.push(Span::Plain(&rest[..at]));
        }
        let content = &inner[..end];
        spans.push(if marker == "`" { Span::Code(content) } else { Span::Bold(content) });
        rest = &inner[end + marker.len()..];
    }
    spans
}

/// `text` laid out in `width`, its inline Markdown styled.
fn inline(ui: &egui::Ui, text: &str, size: f32, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let palette = theme::palette();
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for span in spans(text) {
        let (text, format) = match span {
            Span::Plain(text) => (text, TextFormat { font_id: FontId::proportional(size), color, ..Default::default() }),
            Span::Bold(text) => (text, TextFormat { font_id: FontId::proportional(size), color: palette.text_strong, ..Default::default() }),
            Span::Code(text) => (
                text,
                TextFormat { font_id: FontId::monospace(size - 1.0), color: palette.code_text, background: palette.raised, ..Default::default() },
            ),
        };
        job.append(text, 0.0, format);
    }
    ui.painter().layout_job(job)
}

/// The assistant's reply, its Markdown drawn.
fn markdown(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        for block in blocks(text) {
            let width = ui.available_width();
            match block {
                Block::Paragraph(text) => {
                    ui.label(inline(ui, &text, 13.5, palette.text, width));
                }
                Block::Heading(text) => {
                    ui.label(inline(ui, &text, 15.0, palette.text_strong, width));
                }
                Block::Item(marker, text) => {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 16.0), Sense::hover());
                        let font = FontId::proportional(13.5);
                        ui.painter().text(rect.right_top(), egui::Align2::RIGHT_TOP, marker, font, palette.text_dim);
                        let width = ui.available_width();
                        ui.label(inline(ui, &text, 13.5, palette.text, width));
                    });
                }
                Block::Code(code) => code_block(ui, &code),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use c2f_agent::conversation::FakeEngine;

    use super::*;

    fn turn() -> Vec<Event> {
        vec![
            Event::Text { delta: "Here is ".into() },
            Event::Text { delta: "**a cube**.".into() },
            Event::ToolCall { id: "t1".into(), name: "render".into(), input: json!({ "code": "cube(10);\nsphere(2);" }) },
            Event::ToolResult { id: "t1".into(), content: vec![json!({ "type": "text", "text": "{\"volume\": 1000}" })], is_error: false },
            Event::ToolCall { id: "t2".into(), name: "edit_code".into(), input: json!({ "line": 3, "text": "cube(12);" }) },
            Event::ToolResult { id: "t2".into(), content: vec![json!({ "type": "text", "text": "done" })], is_error: false },
            Event::Text { delta: " Done.".into() },
            Event::Done { usage: Some(Usage { input_tokens: 120, output_tokens: 40 }), cost_usd: Some(0.01), session_id: Some("s1".into()) },
        ]
    }

    fn fake(turns: Vec<Vec<Event>>) -> AgentPanel {
        AgentPanel::new(move |_: &EngineRequest| Built::ready(Box::new(FakeEngine::new(turns.clone())), "fake"), None)
    }

    #[test]
    fn the_engine_is_made_again_when_the_settings_or_the_engines_change() {
        use c2f_agent::engine::claude_code::ClaudeCodeStatus;

        // What each engine was made from: the probe's key, the session, the
        // permission and the texts carried on.
        type Made = (bool, Option<String>, bool, usize);
        let made: Arc<Mutex<Vec<Made>>> = Arc::default();
        let log = made.clone();
        let mut panel = AgentPanel::new(
            move |request: &EngineRequest| {
                let probe = request.probe.is_some_and(|p| p.key.is_some());
                let session = request.transcript.session_id.clone();
                log.lock().unwrap().push((probe, session, request.agent.ask_first, request.transcript.entries.len()));
                Built::ready(Box::new(FakeEngine::new(vec![turn()])), "fake")
            },
            None,
        );
        let ctx = egui::Context::default();
        panel.configure(&ctx, &AgentSettings::default(), &[]);
        assert_eq!(made.lock().unwrap().len(), 1, "nothing changed");
        panel.send("a cube");
        panel.poll();

        let ask = AgentSettings { ask_first: true, ..AgentSettings::default() };
        panel.configure(&ctx, &ask, &[]);
        panel.prober.set(agent_setup::Probe {
            claude: None,
            status: ClaudeCodeStatus::Missing,
            key: Some("sk-ant-test".into()),
            key_from_env: false,
        });
        panel.configure(&ctx, &ask, &[]);
        panel.configure(&ctx, &ask, &[]);
        let made = made.lock().unwrap();
        assert_eq!(made.len(), 3, "{made:?}");
        assert_eq!(made[1], (false, Some("s1".into()), true, 5), "the settings changed: the conversation carried on");
        assert_eq!(made[2], (true, Some("s1".into()), true, 5), "the engines found");
    }

    #[test]
    fn a_conversation_reduces_into_the_transcript() {
        let mut panel = fake(vec![turn()]);
        panel.attach(Attachment::Context { label: "box.c2f:3".into(), text: "cube(10);".into() });
        panel.send("  a cube  ");
        assert!(panel.running());
        assert!(panel.poll());
        assert!(!panel.running(), "the turn is over");
        let t = panel.transcript();
        let Entry::User { message, shown: None } = &t.entries[0] else { panic!("{:?}", t.entries[0]) };
        assert_eq!(message.text, "a cube");
        assert_eq!(message.attachments.len(), 1, "the attachments go with the message");
        assert!(panel.attachments.is_empty());
        assert_eq!(t.entries[1], Entry::Assistant { text: "Here is **a cube**.".into() });
        let Entry::Tool(render) = &t.entries[2] else { panic!() };
        assert_eq!((render.name.as_str(), render.status, render.content.len()), ("render", Status::Ok, 1));
        let Entry::Tool(edit) = &t.entries[3] else { panic!() };
        assert_eq!(edited_line(edit), Some(3));
        assert_eq!(edited_line(render), None);
        assert_eq!(t.entries[4], Entry::Assistant { text: " Done.".into() });
        assert_eq!(t.usage, Usage { input_tokens: 120, output_tokens: 40 });
        assert_eq!(t.session_id.as_deref(), Some("s1"));
        assert!((t.cost_usd - 0.01).abs() < 1e-9);

        // The fake engine has no turn left: the panel says why.
        panel.send("again");
        assert!(!panel.running());
        assert!(matches!(panel.transcript().entries.last(), Some(Entry::Notice { error: true, .. })));
        // Blank text sends nothing.
        let count = panel.transcript().entries.len();
        panel.send("   ");
        assert_eq!(panel.transcript().entries.len(), count);
    }

    #[test]
    fn an_error_ends_the_turn_and_stops_its_tools() {
        let mut t = Transcript::default();
        assert!(!t.apply(Event::ToolCall { id: "1".into(), name: "image".into(), input: json!({}) }));
        assert!(!t.apply(Event::ToolResult { id: "nope".into(), content: vec![], is_error: true }), "an unknown call is ignored");
        assert!(t.apply(Event::Error { message: "rate limited".into() }));
        assert!(matches!(&t.entries[0], Entry::Tool(call) if call.status == Status::Stopped));
        assert_eq!(t.entries[1], Entry::Notice { text: "rate limited".into(), error: true });
    }

    /// An engine whose turn runs until stopped, its events sent by the test.
    struct Held {
        sender: Arc<Mutex<Option<mpsc::Sender<Event>>>>,
        stops: Arc<AtomicUsize>,
    }

    impl Engine for Held {
        fn send(&mut self, _: UserMessage, events: mpsc::Sender<Event>) -> Result<(), String> {
            *self.sender.lock().unwrap() = Some(events);
            Ok(())
        }

        fn stop(&mut self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
        }

        fn name(&self) -> &str {
            "held"
        }
    }

    #[test]
    fn stop_ends_the_turn_at_once() {
        let sender = Arc::new(Mutex::new(None));
        let stops = Arc::new(AtomicUsize::new(0));
        let (s, n) = (sender.clone(), stops.clone());
        let mut panel = AgentPanel::new(move |_: &EngineRequest| Built::ready(Box::new(Held { sender: s.clone(), stops: n.clone() }), "held"), None);
        panel.send("a long job");
        let events = sender.lock().unwrap().clone().unwrap();
        events.send(Event::Text { delta: "Working".into() }).unwrap();
        events.send(Event::ToolCall { id: "1".into(), name: "render".into(), input: json!({}) }).unwrap();
        assert!(panel.poll());
        assert!(panel.running());

        let started = Instant::now();
        panel.stop();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!panel.running());
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        let entries = &panel.transcript().entries;
        assert!(matches!(&entries[2], Entry::Tool(call) if call.status == Status::Stopped));
        assert!(matches!(entries.last(), Some(Entry::Notice { error: false, .. })));

        // What the engine still sends is dropped.
        let count = entries.len();
        let _ = events.send(Event::Text { delta: "late".into() });
        assert!(!panel.poll());
        assert_eq!(panel.transcript().entries.len(), count);
        panel.stop();
        assert_eq!(stops.load(Ordering::SeqCst), 1, "nothing to stop");
    }

    fn temp_store(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code2form-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_conversation_comes_back_for_its_file() {
        let store = temp_store("resume");
        let file = PathBuf::from("/work/box.c2f");
        let other = PathBuf::from("/work/lid.c2f");
        let turns = vec![turn(), vec![Event::Text { delta: "Hi".into() }, Event::Done { usage: None, cost_usd: None, session_id: None }]];
        let mut panel = AgentPanel::new(move |_: &EngineRequest| Built::ready(Box::new(FakeEngine::new(turns.clone())), "fake"), Some(store.clone()));
        panel.set_tab(Some(&file), Some(PathBuf::from("/work")));
        assert!(!panel.can_resume(), "nothing kept yet");
        panel.send("a cube");
        panel.poll();
        let kept = panel.transcript().clone();

        // Another tab: its own conversation, empty.
        panel.set_tab(Some(&other), None);
        assert!(panel.transcript().entries.is_empty());
        assert!(!panel.can_resume());
        // Back: Resume brings it back.
        panel.set_tab(Some(&file), None);
        assert!(panel.can_resume());
        panel.resume();
        assert_eq!(panel.transcript(), &kept);

        // A new conversation keeps the last until its first message.
        panel.new_conversation();
        assert!(panel.can_resume());
        let mut later = fake(vec![]);
        later.store = Some(store.clone());
        later.set_tab(Some(&file), None);
        later.resume();
        assert_eq!(later.transcript(), &kept, "kept across runs");
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn an_untitled_conversation_goes_with_the_file_saved() {
        let store = temp_store("untitled");
        let mut panel = AgentPanel::new(|_: &EngineRequest| Built::ready(Box::new(EchoEngine), "echo"), Some(store.clone()));
        panel.set_tab(None, None);
        panel.send("hello");
        panel.poll();
        assert!(panel.transcript().entries.len() == 2);
        let file = PathBuf::from("/work/saved.c2f");
        panel.set_tab(Some(&file), None);
        assert_eq!(panel.transcript().entries.len(), 2, "kept");
        assert!(conversation_path(&store, &file).is_file());
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn conversations_are_named_by_their_file() {
        let store = Path::new("/store");
        let a = conversation_path(store, Path::new("/a/box.c2f"));
        let b = conversation_path(store, Path::new("/b/box.c2f"));
        assert_ne!(a, b);
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("box-"));
        assert_eq!(a, conversation_path(store, Path::new("/a/box.c2f")));
    }

    #[test]
    fn the_prompts_take_the_answer_and_the_file() {
        let folder = Path::new("/work/winder");
        let plan = prompt_text(Shortcut::Plan, "a watch winder", None, Some(folder)).unwrap();
        assert!(plan.contains("a watch winder") && plan.contains("/work/winder"), "{plan}");
        let design = prompt_text(Shortcut::Design, "a lamp", None, None).unwrap();
        assert!(design.contains("a lamp"));
        assert!(prompt_text(Shortcut::Electronics, "an ESP32", None, Some(folder)).is_err(), "no design saved");
        let file = Path::new("/work/winder/winder.c2f");
        let electronics = prompt_text(Shortcut::Electronics, "an ESP32", Some(file), Some(folder)).unwrap();
        assert!(electronics.contains("winder.c2f") && electronics.contains("an ESP32"));
    }

    #[test]
    fn markdown_splits_into_blocks_and_spans() {
        let text = "# Plan\nFirst line\nsame paragraph.\n\n- one\n  more\n2. two\n```\ncube(1);\n\nsphere(2);\n```\nafter\n```\nopen";
        assert_eq!(
            blocks(text),
            [
                Block::Heading("Plan".into()),
                Block::Paragraph("First line same paragraph.".into()),
                Block::Item("•".into(), "one more".into()),
                Block::Item("2.".into(), "two".into()),
                Block::Code("cube(1);\n\nsphere(2);".into()),
                Block::Paragraph("after".into()),
                Block::Code("open".into()),
            ]
        );
        assert_eq!(
            spans("a **b** `c` d"),
            [Span::Plain("a "), Span::Bold("b"), Span::Plain(" "), Span::Code("c"), Span::Plain(" d")]
        );
        assert_eq!(spans("2 ** 3 and `x"), [Span::Plain("2 ** 3 and `x")]);
        assert_eq!(spans("`a**b`"), [Span::Code("a**b")]);
    }

    #[test]
    fn inputs_read_on_one_line() {
        assert_eq!(summary(&json!({ "code": "cube(1);\nsphere(2);", "view": "top" }), 80), "code: cube(1);, view: top");
        assert_eq!(summary(&json!({ "n": 3 }), 80), "n: 3");
        assert_eq!(summary(&json!({ "path": "abcdefghij" }), 8), "path: a…");
    }

    #[test]
    fn lines_have_offsets() {
        let text = "a\nbc\nd";
        assert_eq!(line_offset(text, 1), 0);
        assert_eq!(line_offset(text, 2), 2);
        assert_eq!(line_offset(text, 3), 5);
        assert_eq!(line_offset(text, 9), text.len());
    }

    #[test]
    fn images_decode() {
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
        assert_eq!(base64_decode("//4AAQ==").unwrap(), [0xff, 0xfe, 0x00, 0x01]);
        assert!(base64_decode("not base64!").is_none());
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        }
        assert_eq!(decode_png(&png), Some(([2, 1], vec![255, 0, 0, 255, 0, 255, 0, 255])));
    }

    #[test]
    fn transcripts_read_as_json() {
        let mut panel = fake(vec![turn()]);
        panel.send("a cube");
        panel.poll();
        let json = serde_json::to_string(panel.transcript()).unwrap();
        let back: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, panel.transcript());
    }
}
