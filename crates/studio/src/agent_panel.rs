//! The agent's chat: a conversation with the agent, on a card floating
//! over the 3D view, another way of editing the model. While the code is
//! hidden, each edit shows in the conversation as a compact diff, which
//! shows its line in the code when clicked.
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

use crate::agent_markdown::markdown;
use crate::agent_setup::{self, AgentSettings, Built, EngineRequest, KeyChange, Prober};
use crate::app::{overlay, review};
use crate::app::selection_context::{self, Suggestion};
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
        #[serde(with = "stored_message")]
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

/// A message as a transcript keeps it: its pictures as base64 text rather
/// than arrays of numbers, four times as long; the arrays older
/// transcripts kept still read.
mod stored_message {
    use c2f_agent::conversation::{Attachment, UserMessage};
    use serde::de::Error as _;
    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(message: &UserMessage, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(message).map_err(S::Error::custom)?;
        let items = value.get_mut("attachments").and_then(Value::as_array_mut).into_iter().flatten();
        for (item, attachment) in items.zip(&message.attachments) {
            if let Attachment::Image { png } = attachment {
                item["png"] = c2f_agent::image_content(png)["data"].clone();
            }
        }
        serde::Serialize::serialize(&value, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<UserMessage, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        for item in value.get_mut("attachments").and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(text) = item.get("png").and_then(Value::as_str) {
                let png = super::base64_decode(text).ok_or_else(|| D::Error::custom("a picture is not base64"))?;
                item["png"] = Value::from(png);
            }
        }
        serde_json::from_value(value).map_err(D::Error::custom)
    }
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

/// The line (from 1) an `edit_code` call changed, once it succeeded: the
/// first of its result's `changed_lines` (`[[first, last], ...]`, from 1),
/// else its input's `range.start_line`.
pub fn edited_line(call: &ToolCall) -> Option<usize> {
    if !is_tool(&call.name, "edit_code") || call.status != Status::Ok {
        return None;
    }
    let changed = call.content.iter().find_map(|item| {
        let result: Value = serde_json::from_str(item.get("text")?.as_str()?).ok()?;
        result["changed_lines"].get(0)?.get(0)?.as_u64()
    });
    let asked = || call.input.get("range")?.get("start_line")?.as_u64();
    changed.or_else(asked).filter(|&n| n >= 1).map(|n| n as usize)
}

/// Whether a call's `name` is the tool `tool`: as the studio names it, or
/// as Claude Code does, prefixed with its server (`mcp__studio__edit_code`).
fn is_tool(name: &str, tool: &str) -> bool {
    name == tool || name.strip_suffix(tool).is_some_and(|prefix| prefix.ends_with("__"))
}

/// The most lines a diff card shows.
const DIFF_LINES: usize = 6;

/// An edit of the agent's, as the conversation shows it while the code
/// is hidden.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffCard {
    /// The first line (from 1) it changed, to show in the code.
    pub line: Option<usize>,
    /// The lines removed and added, the first [`DIFF_LINES`] of them.
    pub lines: Vec<(review::Change, String)>,
    /// How many more there are.
    pub more: usize,
    /// What a circuit's edit did, as its tool says.
    pub done: Option<String>,
}

/// The diff card of a call that edited the code or the circuit, once it
/// succeeded: `edit_code`'s `edits` (`[{old, new}]`) diffed line by line,
/// or the lines its `range` was replaced with; the circuit tools' `done`.
pub fn diff_card(call: &ToolCall) -> Option<DiffCard> {
    if call.status != Status::Ok {
        return None;
    }
    let result = call
        .content
        .iter()
        .find_map(|item| serde_json::from_str::<Value>(item.get("text")?.as_str()?).ok())
        .unwrap_or(Value::Null);
    if is_tool(&call.name, "edit_code") {
        let mut lines = Vec::new();
        if let Some(edits) = call.input.get("edits").and_then(Value::as_array) {
            for edit in edits {
                let (old, new) = (edit["old"].as_str().unwrap_or(""), edit["new"].as_str().unwrap_or(""));
                let changed = review::diff_lines(old, new).into_iter().filter(|l| l.change != review::Change::Same);
                lines.extend(changed.map(|l| (l.change, l.text)));
            }
        } else if let Some(new) = call.input.get("new").and_then(Value::as_str) {
            lines.extend(new.lines().map(|l| (review::Change::Added, l.to_string())));
        }
        let more = lines.len().saturating_sub(DIFF_LINES);
        lines.truncate(DIFF_LINES);
        // Without the indentation they share, which would take the width.
        let indent = lines.iter().filter(|(_, l)| !l.trim().is_empty()).map(|(_, l)| l.len() - l.trim_start().len()).min().unwrap_or(0);
        for (_, line) in &mut lines {
            *line = line.get(indent..).unwrap_or(line.trim_start()).to_string();
        }
        return Some(DiffCard { line: edited_line(call), lines, more, done: None });
    }
    let circuit = ["place_component", "connect", "disconnect"].iter().any(|tool| is_tool(&call.name, tool));
    let done = result.get("done").and_then(Value::as_str).filter(|_| circuit)?;
    Some(DiffCard { line: None, lines: Vec::new(), more: 0, done: Some(done.to_string()) })
}

// ---- Suggestions ----

/// A suggestion of what to ask, offered while the conversation is empty:
/// what its chip says, and the message it sends.
#[derive(Clone, Debug, PartialEq)]
pub struct Starter {
    pub label: String,
    pub message: String,
}

/// What the active tab holds, for the suggestions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Subject<'a> {
    /// Nothing written yet.
    Empty,
    /// A design, and the first of its parameters that is a number.
    Design { parameter: Option<&'a str> },
    Circuit,
}

/// The suggestions for `subject`: things to ask of what is open, taken
/// from what it is, never claims about it.
pub fn starters(subject: Subject) -> Vec<Starter> {
    let starter = |label: String, message: String| Starter { label, message };
    match subject {
        Subject::Empty => vec![
            starter(
                tr("A box with a lid", "Une boîte avec couvercle").into(),
                tr(
                    "Make a parametric box with a lid in this file: its width, depth and height as parameters.",
                    "Fais dans ce fichier une boîte paramétrique avec couvercle : sa largeur, sa profondeur et sa hauteur en paramètres.",
                )
                .into(),
            ),
            starter(
                tr("Help me choose what to make", "Aide-moi à choisir quoi faire").into(),
                tr(
                    "Help me decide what to make: ask me what the object must do, one question at a time.",
                    "Aide-moi à décider quoi fabriquer : demande-moi ce que l'objet doit faire, une question à la fois.",
                )
                .into(),
            ),
        ],
        Subject::Design { parameter } => {
            let mut list = Vec::new();
            if let Some(name) = parameter {
                list.push(starter(
                    tr!("Make {name} bigger", "Agrandir {name}"),
                    tr!("Make `{name}` bigger, and check the design still renders.", "Augmente `{name}`, et vérifie que le design se rend toujours."),
                ));
            }
            list.push(starter(
                tr("Check it prints without supports", "Vérifier l'impression sans supports").into(),
                tr(
                    "Check whether this design prints without supports, and show me where it would need them.",
                    "Vérifie si ce design s'imprime sans supports, et montre-moi où il en faudrait.",
                )
                .into(),
            ));
            list.push(starter(
                tr("Make it lighter", "L'alléger").into(),
                tr(
                    "Make this design lighter while keeping it strong: say where you remove material, then do it.",
                    "Allège ce design en le gardant solide : dis où tu retires de la matière, puis fais-le.",
                )
                .into(),
            ));
            list.push(starter(
                tr("Explain how it is built", "Expliquer sa construction").into(),
                tr("Explain how this design is built, part by part.", "Explique comment ce design est construit, pièce par pièce.").into(),
            ));
            list.truncate(4);
            list
        }
        Subject::Circuit => vec![
            starter(
                tr("Check the wiring", "Vérifier le câblage").into(),
                tr("Check the wiring of this circuit and explain any problem.", "Vérifie le câblage de ce circuit et explique les problèmes.").into(),
            ),
            starter(
                tr("Guide me through the assembly", "Me guider dans le montage").into(),
                tr("Guide me through the assembly of this circuit, one wire at a time.", "Guide-moi dans le montage de ce circuit, un fil à la fois.").into(),
            ),
            starter(
                tr("Explain each part", "Expliquer chaque composant").into(),
                tr("Explain what each part of this circuit does.", "Explique le rôle de chaque composant de ce circuit.").into(),
            ),
        ],
    }
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

    fn icon(self) -> Icon {
        match self {
            Shortcut::Plan => Icon::Plan,
            Shortcut::Design => Icon::Design,
            Shortcut::Electronics => Icon::Electronics,
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
    prompt_messages(shortcut.prompt(), &arguments)
}

/// The text of the prompt `name` filled in with `arguments`: its messages'
/// texts, joined.
fn prompt_messages(name: &str, arguments: &Value) -> Result<String, String> {
    let prompt = c2f_agent::prompt(name, arguments)?;
    let texts: Vec<&str> = prompt["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message["content"]["text"].as_str())
        .collect();
    Ok(texts.join("\n\n"))
}

// ---- Next steps ----

/// A step the conversation leads to, offered as a chip under its last
/// turn.
#[derive(Clone, Debug, PartialEq)]
pub enum NextStep {
    /// Draw the object from the brief the plan saved.
    Design { brief: PathBuf },
    /// Give the design drawn from `brief` the components it lists.
    Electronics { design: PathBuf, brief: PathBuf, components: String },
}

impl NextStep {
    fn label(&self) -> &'static str {
        match self {
            NextStep::Design { .. } => tr("Draw it from this brief", "Dessiner à partir de ce brief"),
            NextStep::Electronics { .. } => tr("Add the electronics", "Ajouter l'électronique"),
        }
    }

    /// The prompt it sends, the design saved in `folder`.
    fn prompt(&self, folder: Option<&Path>) -> Result<String, String> {
        let shown = |path: &Path| path.display().to_string();
        let (name, arguments) = match self {
            NextStep::Design { brief } => {
                let mut arguments = json!({ "brief": shown(brief) });
                if let Some(folder) = folder {
                    arguments["folder"] = json!(shown(folder));
                }
                ("design_object", arguments)
            }
            NextStep::Electronics { design, brief, components } => {
                ("add_electronics", json!({ "design": shown(design), "brief": shown(brief), "components": components }))
            }
        };
        prompt_messages(name, &arguments)
    }
}

/// The brief a successful `save_brief` call wrote, and the components its
/// requirements list (`component "…"` lines).
fn saved_brief(call: &ToolCall) -> Option<(PathBuf, Vec<String>)> {
    if !is_tool(&call.name, "save_brief") || call.status != Status::Ok {
        return None;
    }
    call.content.iter().find_map(|item| {
        let result: Value = serde_json::from_str(item.get("text")?.as_str()?).ok()?;
        if result["saved"] != true {
            return None;
        }
        let components = result["requirements"].as_array().into_iter().flatten();
        let components = components.filter(|r| r["kind"] == "component").filter_map(|r| r["value"].as_str().map(str::to_string));
        Some((PathBuf::from(result["path"].as_str()?), components.collect()))
    })
}

/// The last brief the conversation saved, and the entry after which it
/// was saved.
fn last_brief(entries: &[Entry]) -> Option<(usize, PathBuf, Vec<String>)> {
    entries.iter().enumerate().rev().find_map(|(at, entry)| match entry {
        Entry::Tool(call) => saved_brief(call).map(|(path, components)| (at, path, components)),
        _ => None,
    })
}

/// What the user sent after entry `at` that starts with `start`: a prompt
/// of the studio's, recognised by its first words.
fn sent_after(entries: &[Entry], at: usize, start: &str) -> bool {
    entries[at..].iter().any(|entry| matches!(entry, Entry::User { message, .. } if message.text.starts_with(start)))
}

/// The steps the conversation leads to, once its turn is over: drawing the
/// object once the plan saved its brief, then, once the design was drawn
/// from it and saved as the active `file`, its electronics when the brief
/// lists components. A step already taken is not offered again.
pub fn next_steps(entries: &[Entry], file: Option<&Path>) -> Vec<NextStep> {
    let Some((at, brief, components)) = last_brief(entries) else { return Vec::new() };
    // The prompts' first words, as `c2f_agent::prompt` writes them.
    let designing = "Design this object with Code2Form";
    if !sent_after(entries, at, designing) {
        return vec![NextStep::Design { brief }];
    }
    let design = file.filter(|file| file.extension().is_some_and(|e| e == "c2f") && file.is_file());
    match design {
        Some(design) if !components.is_empty() && !sent_after(entries, at, "Add the electronics to") => {
            vec![NextStep::Electronics { design: design.to_path_buf(), brief, components: components.join(", ") }]
        }
        _ => Vec::new(),
    }
}

/// The brief the conversation last saved, when it is in an untitled tab's
/// scratch folder, which the user would not find.
pub fn brief_in_scratch(entries: &[Entry]) -> Option<PathBuf> {
    let (_, brief, _) = last_brief(entries)?;
    brief.starts_with(agent_setup::scratch_folder("")).then_some(brief)
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

/// Stands in for an engine in the screenshot of a turn under way: it
/// starts answering and holds the turn open.
#[derive(Default)]
pub struct WorkingEngine(Option<mpsc::Sender<Event>>);

impl Engine for WorkingEngine {
    fn send(&mut self, _: UserMessage, events: mpsc::Sender<Event>) -> Result<(), String> {
        let delta = tr("Making the box 40 mm wide…", "J'élargis la boîte à 40 mm…").to_string();
        let _ = events.send(Event::Text { delta });
        self.0 = Some(events);
        Ok(())
    }

    fn stop(&mut self) {
        self.0 = None;
    }

    fn name(&self) -> &str {
        "working"
    }
}

/// Stands in for an engine in the screenshot of a conversation: it plays
/// the events it was given as a turn.
pub struct ScriptedEngine(pub Vec<Event>);

impl Engine for ScriptedEngine {
    fn send(&mut self, _: UserMessage, events: mpsc::Sender<Event>) -> Result<(), String> {
        for event in std::mem::take(&mut self.0) {
            let _ = events.send(event);
        }
        Ok(())
    }

    fn stop(&mut self) {}

    fn name(&self) -> &str {
        "scripted"
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
    /// Open over the view: the app sets it each frame.
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
    /// The settings, libraries, check and folder it was made for: a new
    /// folder (an untitled tab saved) makes a new engine.
    built_for: Option<(AgentSettings, Vec<PathBuf>, u64, Option<PathBuf>)>,
    /// The turn under way was started again once without its session,
    /// which Claude Code no longer had.
    retried: bool,
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
    /// What is selected in the view, and in the code: sent with the next
    /// message unless removed.
    suggested: [Suggestion; 2],
    /// Whether a picture of the view goes with each message.
    attach_view: bool,
    /// A message waiting for the picture of the view, and whether it was
    /// asked for.
    awaiting_view: Option<(String, bool)>,
    /// The label of the action that undoes the agent's last change, while
    /// the app says it can; and whether it was clicked, for the app.
    pub undo_offer: Option<String>,
    pub undo_asked: bool,
    /// What to suggest asking while the conversation is empty, for the
    /// active tab: the app sets them each frame.
    pub starters: Vec<Starter>,
    /// The height the card's header and field took at the last frame.
    chrome: f32,
    /// The next step the user picked under the last turn.
    next_asked: Option<NextStep>,
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
            retried: false,
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
            suggested: Default::default(),
            attach_view: false,
            awaiting_view: None,
            undo_offer: None,
            undo_asked: false,
            next_asked: None,
            starters: Vec::new(),
            chrome: 170.0,
        };
        panel.build();
        panel
    }

    /// Makes the engine for the settings, the engines found and the
    /// conversation as it is; what the user should know of it goes into
    /// the conversation.
    fn build(&mut self) {
        // Claude Code keeps a session in its folder: in another folder it
        // has none to resume, and starts a new one.
        if self.built_for.as_ref().is_some_and(|built| built.3 != self.folder) {
            self.transcript.session_id = None;
        }
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
        self.built_for = Some((self.agent.clone(), self.libraries.clone(), generation, self.folder.clone()));
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
        let now = (self.agent.clone(), self.libraries.clone(), self.prober.current().0, self.folder.clone());
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

    /// What the conversation holds so far.
    #[cfg(test)]
    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    /// Adds a note of the panel's own to the conversation.
    pub fn note(&mut self, text: String, error: bool) {
        self.transcript.entries.push(Entry::Notice { text, error });
    }

    /// Whether a turn is under way.
    pub fn running(&self) -> bool {
        self.events.is_some()
    }

    /// Readies the message field, for when the panel next shows.
    pub fn focus(&mut self) {
        self.focus_input = true;
    }

    /// Adds something to send with the next message: a picture of the
    /// view, or what the user selected.
    pub fn attach(&mut self, attachment: Attachment) {
        if !self.attachments.contains(&attachment) {
            self.attachments.push(attachment);
        }
    }

    // ---- The selection ----

    /// Follows what is selected in the view and in the code, every frame.
    pub fn suggest(&mut self, view: Option<Attachment>, code: Option<Attachment>) {
        let [in_view, in_code] = &mut self.suggested;
        in_view.follow(view);
        in_code.follow(code);
    }

    /// The suggestions shown: neither dismissed nor attached already.
    fn suggestions(&self) -> impl Iterator<Item = (usize, &Attachment)> {
        let attached = |a: &Attachment| self.attachments.iter().any(|b| selection_context::label(b) == selection_context::label(a));
        self.suggested.iter().enumerate().filter_map(move |(i, s)| s.shown().filter(|a| !attached(a)).map(|a| (i, a)))
    }

    /// Readies the field, with `attachment` to send; the app shows the panel.
    pub fn ask_about(&mut self, attachment: Attachment) {
        self.focus_input = true;
        self.attach(attachment);
    }

    /// The attachments of the message being sent, the suggestions with
    /// them.
    fn outgoing(&mut self) -> Vec<Attachment> {
        let mut attachments = std::mem::take(&mut self.attachments);
        for suggestion in &mut self.suggested {
            if let Some(attachment) = suggestion.take() {
                let label = selection_context::label(&attachment);
                if !attachments.iter().any(|a| selection_context::label(a) == label) {
                    attachments.push(attachment);
                }
            }
        }
        attachments
    }

    /// Whether to ask for the picture of the view a message waits for:
    /// true once.
    pub fn wants_view(&mut self) -> bool {
        match &mut self.awaiting_view {
            Some((_, asked)) if !*asked => {
                *asked = true;
                true
            }
            _ => false,
        }
    }

    /// Whether a message waits for the picture of the view.
    pub fn awaits_view(&self) -> bool {
        self.awaiting_view.is_some()
    }

    /// Sends the message that waited for the view, with its picture,
    /// `None` when the view is hidden.
    pub fn send_with_view(&mut self, png: Option<Vec<u8>>) {
        let Some((text, _)) = self.awaiting_view.take() else { return };
        let shown = png.is_some();
        if let Some(png) = png {
            self.attach(Attachment::Image { png });
        }
        self.send(&text);
        if !shown {
            self.problem = Some(tr("The 3D view is hidden: sent without its picture", "La vue 3D est masquée : envoyé sans son image").into());
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
        let message = UserMessage { text: text.trim().to_string(), attachments: self.outgoing() };
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
        self.retried = false;
        self.transcript.push_user(message.clone(), shown);
        self.send_to_engine(message);
        self.save();
    }

    /// Sends `message` to the engine, its turn's events to come.
    fn send_to_engine(&mut self, message: UserMessage) {
        let (sender, receiver) = mpsc::channel();
        match self.engine.send(message, sender) {
            Ok(()) => self.events = Some(receiver),
            Err(why) => self.transcript.entries.push(Entry::Notice { text: why, error: true }),
        }
    }

    /// Whether the turn failed on a session Claude Code does not have
    /// (made in another folder, or cleared), and was started again without
    /// it: once a turn, from a fresh engine.
    fn retry_without_session(&mut self, event: &Event) -> bool {
        let Event::Error { message } = event else { return false };
        if self.retried || self.transcript.session_id.is_none() || !message.contains("No conversation found") {
            return false;
        }
        self.retried = true;
        self.transcript.session_id = None;
        let last = self.transcript.entries.iter().rev().find_map(|entry| match entry {
            Entry::User { message, .. } => Some(message.clone()),
            _ => None,
        });
        let Some(message) = last else { return false };
        self.events = None;
        self.build();
        self.send_to_engine(message);
        true
    }

    /// Takes in what the engine streamed since the last frame; whether
    /// anything came.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut over = false;
        while let Some(events) = &self.events {
            match events.try_recv() {
                Ok(event) => {
                    changed = true;
                    if self.retry_without_session(&event) {
                        continue;
                    }
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
        // Written beside it, then put in its place: a crash midway leaves
        // the last conversation whole.
        let partial = path.with_extension("json.partial");
        let written = std::fs::create_dir_all(store)
            .and_then(|()| std::fs::write(&partial, serde_json::to_vec(&self.transcript).unwrap_or_default()))
            .and_then(|()| std::fs::rename(&partial, &path));
        if let Err(err) = written {
            self.problem = Some(tr!("Cannot save the conversation: {err}", "Impossible d'enregistrer la conversation : {err}"));
        }
    }

    /// Starts the conversation with the plan prompt for `idea`, as the
    /// start screen of a new file asks: a note first says where the plan
    /// is written, the active tab's folder (a scratch folder until the
    /// file is saved).
    pub fn start_plan(&mut self, idea: &str) {
        if idea.trim().is_empty() || self.running() {
            return;
        }
        if !self.transcript.entries.is_empty() {
            self.new_conversation();
        }
        self.send_prompt(Shortcut::Plan, idea);
        if self.transcript.entries.is_empty() {
            return;
        }
        let folder = self.folder.as_deref().map_or_else(String::new, |f| f.display().to_string());
        let note = if self.file.is_some() {
            tr!("The agent writes the plan in {folder}.", "L'agent écrit le plan dans {folder}.")
        } else {
            tr!(
                "The agent writes the plan in {folder}, a folder of this untitled tab's: save the design later to keep it with your files.",
                "L'agent écrit le plan dans {folder}, un dossier de cet onglet sans titre : enregistrez le design plus tard pour le garder avec vos fichiers."
            )
        };
        self.transcript.entries.insert(0, Entry::Notice { text: note, error: false });
    }

    /// Talks to `engine` from now on, named `label`: for screenshots.
    pub fn use_engine(&mut self, engine: Box<dyn Engine>, label: &str) {
        self.engine = engine;
        self.engine_label = label.to_string();
        self.engine_problem = None;
    }

    /// Sends the prompt of `step`, the design saved in the active tab's
    /// folder.
    fn take_step(&mut self, step: &NextStep) {
        if self.running() {
            return;
        }
        match step.prompt(self.folder.as_deref()) {
            Ok(text) => {
                let message = UserMessage { text, attachments: self.outgoing() };
                self.start(message, Some(step.label().to_string()));
            }
            Err(why) => self.problem = Some(why),
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
                let message = UserMessage { text, attachments: self.outgoing() };
                self.start(message, Some(format!("{} · {}", shortcut.label(), answer.trim())));
            }
            Err(why) => self.problem = Some(why),
        }
    }

    // ---- Drawing ----

    /// Takes in the engine's events, every frame, shown or not.
    pub fn update(&mut self, ctx: &egui::Context) {
        self.poll();
        if self.running() {
            // The events come from another thread, which cannot wake egui.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Shows the conversation on a card floating over the view, as `place`
    /// says, or only its message field when `minimised`; what the user
    /// asked of the app.
    pub fn show_card(&mut self, ctx: &egui::Context, place: Placement, minimised: &mut bool) -> CardResponse {
        let mut out = CardResponse::default();
        let area = egui::Area::new(Id::new("agent-chat"))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(place.anchor)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                theme::card_frame().inner_margin(Margin { left: 12, right: 10, top: 8, bottom: 10 }).show(ui, |ui| {
                    ui.set_width(place.width - 22.0);
                    if *minimised {
                        self.input_bar(ui, minimised, &mut out);
                        return;
                    }
                    let top = ui.cursor().top();
                    self.header(ui, minimised, &mut out);
                    ui.add_space(6.0);
                    let shown = if self.transcript.entries.is_empty() {
                        self.empty_state(ui);
                        0.0
                    } else {
                        let max = overlay::conversation_room(self.chrome, place.cap);
                        let (go_to, height) = self.conversation(ui, max);
                        out.go_to = go_to;
                        if let Some(step) = self.next_asked.take() {
                            self.take_step(&step);
                        }
                        height
                    };
                    ui.add_space(8.0);
                    self.composer(ui, &mut out);
                    // What the card takes besides its messages, for the next
                    // frame's room for them.
                    self.chrome = ui.cursor().top() - top - shown + 18.0;
                });
            });
        out.rect = Some(area.response.rect);
        out
    }

    /// The engine's name; New, Resume, Minimise and Close.
    fn header(&mut self, ui: &mut egui::Ui, minimised: &mut bool, out: &mut CardResponse) {
        let palette = theme::palette();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
            icons::paint(ui.painter(), rect, Icon::Agent, palette.text_dim);
            ui.add(egui::Label::new(RichText::new(&self.engine_label).size(11.5).color(palette.text_faint)).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if small_icon(ui, Icon::Close, tr("Close the chat (Esc in an empty field)", "Fermer la discussion (Échap dans un champ vide)")).clicked() {
                    out.close = true;
                }
                if small_icon(ui, Icon::Minimise, tr("Fold the chat to its message field", "Replier la discussion sur son champ de message")).clicked() {
                    *minimised = true;
                }
                ui.add_space(4.0);
                let resume = ui.add_enabled(self.can_resume(), Button::new(RichText::new(tr("Resume", "Reprendre")).size(12.0)));
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

    /// What an empty conversation shows: what the agent does, suggestions
    /// for the open file, and the prompts' buttons.
    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        ui.label(
            RichText::new(tr(
                "Ask the agent to design, change or check the model: it edits the code and the view follows.",
                "Demandez à l'agent de concevoir, modifier ou vérifier le modèle : il modifie le code et la vue suit.",
            ))
            .size(12.5)
            .color(palette.text_dim),
        );
        ui.add_space(6.0);
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for (index, starter) in self.starters.iter().enumerate() {
                if suggestion_chip(ui, &starter.label).on_hover_text(&starter.message).clicked() {
                    picked = Some(index);
                }
            }
        });
        if let Some(starter) = picked.and_then(|i| self.starters.get(i).cloned()) {
            if !self.running() {
                self.attach_view_or_send(starter.message);
            }
        }
        ui.add_space(8.0);
        self.prompt_row(ui);
    }

    /// The prompts' buttons, and the field one of them asks with.
    fn prompt_row(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette();
        ui.label(RichText::new(tr("Or start from an idea:", "Ou partir d'une idée :")).size(11.5).color(palette.text_faint));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for shortcut in Shortcut::ALL {
                let on = self.asking.as_ref().is_some_and(|(s, _)| *s == shortcut);
                let clicked = ui
                    .add_enabled_ui(!self.running(), |ui| icons::labelled(ui, shortcut.icon(), shortcut.label(), on))
                    .inner
                    .on_hover_text(shortcut.hint())
                    .clicked();
                if clicked {
                    self.asking = if on { None } else { Some((shortcut, String::new())) };
                    self.problem = None;
                    ui.memory_mut(|m| m.request_focus(answer_id()));
                }
            }
        });
        let Some((shortcut, mut answer)) = self.asking.take() else { return };
        let mut send = false;
        let mut cancel = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = (ui.available_width() - 130.0).max(80.0);
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

    /// The attachments and suggestions of the selection, as chips.
    fn attachment_row(&mut self, ui: &mut egui::Ui) {
        if self.attachments.is_empty() && self.suggestions().next().is_none() {
            return;
        }
        let (mut remove, mut dismiss) = (None, None);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (index, attachment) in self.attachments.iter().enumerate() {
                if chip(ui, attachment, true) {
                    remove = Some(index);
                }
            }
            for (index, attachment) in self.suggestions() {
                if chip(ui, attachment, true) {
                    dismiss = Some(index);
                }
            }
        });
        if let Some(index) = remove {
            self.attachments.remove(index);
        }
        if let Some(index) = dismiss {
            self.suggested[index].dismiss();
        }
        ui.add_space(4.0);
    }

    /// The message field, `rows` high: Enter sends, Shift+Enter goes to the
    /// field, a new line; Escape in it, empty, asks to close the chat.
    fn field(&mut self, ui: &mut egui::Ui, rows: usize, width: f32, out: &mut CardResponse) -> (egui::Response, bool) {
        let hint = if rows > 1 {
            tr("Ask the agent… (Enter sends, Shift+Enter: new line)", "Demandez à l'agent… (Entrée envoie, Maj+Entrée : nouvelle ligne)")
        } else {
            tr("Ask the agent…", "Demandez à l'agent…")
        };
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
            .desired_rows(rows)
            .desired_width(width)
            .frame(false)
            .hint_text(hint);
        let response = field_frame(ui, |ui| ui.add(field));
        if std::mem::take(&mut self.focus_input) {
            response.request_focus();
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) && self.input.trim().is_empty() {
            out.close = true;
        }
        (response, enter)
    }

    /// The chat folded to its message field, with Unfold and Close.
    fn input_bar(&mut self, ui: &mut egui::Ui, minimised: &mut bool, out: &mut CardResponse) {
        let palette = theme::palette();
        self.attachment_row(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if self.running() || self.awaits_view() {
                ui.add(egui::Spinner::new().size(14.0).color(palette.accent_strong)).on_hover_text(tr("The agent is working", "L'agent travaille"));
            } else {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                icons::paint(ui.painter(), rect, Icon::Agent, palette.text_dim);
            }
            let width = (ui.available_width() - 2.0 * 26.0 - 8.0).max(80.0);
            let (response, enter) = self.field(ui, 1, width, out);
            if small_icon(ui, Icon::FoldUp, tr("Show the conversation", "Afficher la conversation")).clicked() {
                *minimised = false;
            }
            if small_icon(ui, Icon::Close, tr("Close the chat", "Fermer la discussion")).clicked() {
                out.close = true;
            }
            if enter && !self.running() && !self.awaits_view() {
                let text = std::mem::take(&mut self.input);
                self.attach_view_or_send(text);
                response.request_focus();
            }
        });
        if let Some(problem) = &self.problem {
            ui.label(RichText::new(problem).size(12.0).color(palette.error));
        }
    }

    /// The attachments, the message field, and Send or Stop.
    fn composer(&mut self, ui: &mut egui::Ui, out: &mut CardResponse) {
        let palette = theme::palette();
        if let Some(problem) = &self.problem {
            ui.label(RichText::new(problem).size(12.0).color(palette.error));
        }
        self.attachment_row(ui);
        let (response, enter) = self.field(ui, 2, f32::INFINITY, out);
        ui.add_space(4.0);
        let mut send = enter && !self.running() && !self.awaits_view();
        ui.horizontal(|ui| {
            if self.awaits_view() {
                ui.add(egui::Spinner::new().size(12.0).color(palette.accent_strong));
                ui.label(RichText::new(tr("Taking the view…", "Capture de la vue…")).size(12.0).color(palette.accent_strong));
            } else if self.running() {
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
                ui.spacing_mut().item_spacing.x = 4.0;
                if self.running() {
                    if ui.button(tr("Stop", "Arrêter")).on_hover_text(tr("Stop the agent's turn", "Arrêter le tour de l'agent")).clicked() {
                        self.stop();
                    }
                } else {
                    let ready = (!self.input.trim().is_empty() || !self.attachments.is_empty()) && !self.awaits_view();
                    let button = Button::new(RichText::new(tr("Send", "Envoyer")).color(palette.on_accent).strong())
                        .fill(palette.accent)
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(0.0, 26.0));
                    send |= ui.add_enabled(ready, button).clicked();
                }
                let hint = if self.attach_view {
                    tr(
                        "On: a picture of the 3D view, as you see it, goes with each message. Click to stop attaching it",
                        "Activé : une image de la vue 3D, telle que vous la voyez, part avec chaque message. Cliquer pour ne plus la joindre",
                    )
                } else {
                    tr(
                        "Off: click to attach a picture of the 3D view, as you see it, to each message",
                        "Désactivé : cliquer pour joindre à chaque message une image de la vue 3D, telle que vous la voyez",
                    )
                };
                if icons::button(ui, Icon::Camera, self.attach_view, hint).clicked() {
                    self.attach_view = !self.attach_view;
                }
            });
        });
        if send {
            let text = std::mem::take(&mut self.input);
            self.attach_view_or_send(text);
            response.request_focus();
        }
    }

    /// Sends `text`, once the app has taken the picture of the view when
    /// it goes with each message; a text nothing could send stays.
    fn attach_view_or_send(&mut self, text: String) {
        if self.attach_view && (!text.trim().is_empty() || !self.attachments.is_empty()) {
            // Sent once the app has taken the picture.
            self.awaiting_view = Some((text, false));
            return;
        }
        self.send(&text);
        if !self.running() && self.transcript.entries.is_empty() {
            self.input = text;
        }
    }

    /// The conversation, at most `max` high, scrolled to its end as it
    /// grows unless the user scrolled up; the line of the code to show, and
    /// the height it took.
    fn conversation(&mut self, ui: &mut egui::Ui, max: f32) -> (Option<usize>, f32) {
        let palette = theme::palette();
        let running = self.running();
        let file = self.owner.as_deref().or(self.file.as_deref()).and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
        let file = file.unwrap_or_else(|| tr("Untitled", "Sans titre").to_string());
        let steps = if running { Vec::new() } else { next_steps(&self.transcript.entries, self.file.as_deref()) };
        let scratch = if running { None } else { brief_in_scratch(&self.transcript.entries) };
        let AgentPanel { transcript, unfolded, thumbnails, undo_offer, undo_asked, next_asked, .. } = self;
        let mut go_to = None;
        let scroll = egui::ScrollArea::vertical().max_height(max).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for entry in &transcript.entries {
                match entry {
                    Entry::User { message, shown } => user_message(ui, message, shown.as_deref()),
                    Entry::Assistant { text } => markdown(ui, text),
                    Entry::Tool(call) => {
                        let clicked = match diff_card(call) {
                            Some(card) => diff_card_row(ui, &card, &file, &call.id),
                            None => tool_row(ui, call, unfolded, thumbnails),
                        };
                        go_to = go_to.or(clicked);
                    }
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
            if !steps.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for step in steps {
                        if suggestion_chip(ui, step.label()).clicked() {
                            *next_asked = Some(step);
                        }
                    }
                });
            }
            if let Some(brief) = &scratch {
                scratch_notice(ui, brief);
            }
            if let Some(label) = undo_offer.as_deref().filter(|_| !running) {
                let hint = tr("Put the code back as it was before the agent's answer, in one step", "Remettre le code comme avant la réponse de l'agent, en une étape");
                if icons::labelled(ui, Icon::Reset, label, false).on_hover_text(hint).clicked() {
                    *undo_asked = true;
                }
            }
        });
        (go_to, scroll.inner_rect.height())
    }
}

/// Where the chat's card goes: its bottom-left corner, its width, and the
/// tallest it grows.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub anchor: egui::Pos2,
    pub width: f32,
    pub cap: f32,
}

/// What the user asked of the app on the card.
#[derive(Debug, Default)]
pub struct CardResponse {
    /// The line (from 1) of the code to show: an edit's card was clicked.
    pub go_to: Option<usize>,
    pub close: bool,
    /// Where the card is.
    pub rect: Option<egui::Rect>,
}

/// A text field drawn by `add` in a box that stands out of the card, its
/// border lit while it has the focus.
pub fn field_frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> egui::Response) -> egui::Response {
    let palette = theme::palette();
    let mut response = None;
    let frame = Frame::new()
        .fill(palette.editor)
        .stroke(Stroke::new(1.0, palette.border_strong))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(6, 4))
        .show(ui, |ui| response = Some(add(ui)));
    let rect = frame.response.rect;
    let response = response.unwrap_or(frame.response);
    if response.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
    }
    response
}

/// A small square button holding `icon`, for the card's header.
fn small_icon(ui: &mut egui::Ui, icon: Icon, hint: &str) -> egui::Response {
    let palette = theme::palette();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), palette.hover);
    }
    let color = if response.hovered() { palette.text_strong } else { palette.text_dim };
    icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0)), icon, color);
    response.on_hover_text(hint)
}

/// A note that the brief is in an untitled tab's scratch folder, with a
/// button that shows it in the Finder (or opens its folder).
fn scratch_notice(ui: &mut egui::Ui, brief: &Path) {
    let palette = theme::palette();
    ui.label(
        RichText::new(tr(
            "The brief is in this untitled tab's own folder. Save the design in a folder of yours to keep them together.",
            "Le brief est dans le dossier propre à cet onglet sans titre. Enregistrez le design dans un de vos dossiers pour les garder ensemble.",
        ))
        .size(12.0)
        .color(palette.text_dim),
    );
    let label = if cfg!(target_os = "macos") { tr("Show in Finder", "Afficher dans le Finder") } else { tr("Open the folder", "Ouvrir le dossier") };
    if ui.small_button(label).on_hover_text(brief.display().to_string()).clicked() {
        let _ = crate::app::reveal(brief);
    }
}

/// A suggestion of what to ask, as a rounded chip that sends it.
fn suggestion_chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let palette = theme::palette();
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 22.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() { palette.hover } else { palette.raised };
    let stroke = Stroke::new(1.0, if response.hovered() { palette.border_strong } else { palette.border });
    ui.painter().rect(rect, CornerRadius::same(13), fill, stroke, egui::StrokeKind::Inside);
    let color = if response.hovered() { palette.text_strong } else { palette.text };
    ui.painter().galley(egui::pos2(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An edit of the agent's as a compact diff: the file and line, the lines
/// removed and added; clicked, the line it shows in the code.
fn diff_card_row(ui: &mut egui::Ui, card: &DiffCard, file: &str, id: &str) -> Option<usize> {
    let palette = theme::palette();
    let inner = Frame::new()
        .fill(palette.faint_bg)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                status_mark(ui, Status::Ok);
                let place = match card.line {
                    Some(line) => format!("{file}:{line}"),
                    None => file.to_string(),
                };
                ui.label(RichText::new(place).font(FontId::monospace(12.0)).color(palette.text_strong));
                if card.line.is_some() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(tr("Show in the code", "Voir dans le code")).size(11.5).color(palette.accent_strong));
                    });
                }
            });
            if let Some(done) = &card.done {
                ui.label(RichText::new(done).size(12.0).color(palette.text));
            }
            ui.add_space(3.0);
            for (change, text) in &card.lines {
                let (sign, color, fill) = match change {
                    review::Change::Removed => ('−', palette.error, palette.error.gamma_multiply(0.14)),
                    _ => ('+', palette.success, palette.success.gamma_multiply(0.14)),
                };
                Frame::new().fill(fill).corner_radius(CornerRadius::same(3)).inner_margin(Margin::symmetric(4, 1)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let mut job = LayoutJob::default();
                    job.wrap.max_rows = 1;
                    job.wrap.max_width = ui.available_width();
                    job.wrap.break_anywhere = true;
                    let font = FontId::monospace(12.0);
                    job.append(&format!("{sign} "), 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
                    job.append(text, 0.0, TextFormat { font_id: font, color: palette.code_text, ..Default::default() });
                    ui.label(job);
                });
            }
            if card.more > 0 {
                ui.label(RichText::new(tr!("… {} more lines", "… {} lignes de plus", card.more)).size(11.5).color(palette.text_faint));
            }
        });
    let line = card.line?;
    let response = ui.interact(inner.response.rect, Id::new(("agent-diff", id)), Sense::click());
    let hint = tr!("Show line {line} in the code", "Afficher la ligne {line} dans le code");
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hint).clicked().then_some(line)
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
            markdown(ui, shown.unwrap_or(&message.text));
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
pub(crate) fn code_block(ui: &mut egui::Ui, text: &str) {
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
            Event::ToolCall { id: "t2".into(), name: "edit_code".into(), input: json!({ "edits": [{ "old": "cube(10);", "new": "cube(12);" }] }) },
            Event::ToolResult {
                id: "t2".into(),
                content: vec![json!({ "type": "text", "text": "{\"edited\": true, \"changed_lines\": [[3, 3]], \"render\": {\"ok\": true}}" })],
                is_error: false,
            },
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
        // A range edit whose result is not read: its input's start line.
        let ranged = ToolCall {
            id: "t3".into(),
            name: "edit_code".into(),
            input: json!({ "range": { "start_line": 7, "end_line": 8 }, "new": "x", "revision": "r" }),
            status: Status::Ok,
            content: vec![],
        };
        assert_eq!(edited_line(&ranged), Some(7));
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
    fn the_selection_goes_with_the_message_once() {
        let context = |label: &str| Attachment::Context { label: label.into(), text: label.into() };
        let done = || vec![Event::Done { usage: None, cost_usd: None, session_id: None }];
        let mut panel = fake(vec![done(), done(), done()]);
        panel.suggest(Some(context("Selection: ring")), Some(context("Lines 3–4")));
        panel.suggested[1].dismiss();
        panel.send("make it bigger");
        panel.poll();
        let sent = |panel: &AgentPanel| match panel.transcript().entries.iter().rev().find(|e| matches!(e, Entry::User { .. })) {
            Some(Entry::User { message, .. }) => message.attachments.clone(),
            _ => panic!("nothing sent"),
        };
        assert_eq!(sent(&panel), [context("Selection: ring")], "the code's was removed");
        // Sent once: the same selection is not offered again.
        panel.suggest(Some(context("Selection: ring")), Some(context("Lines 3–4")));
        assert_eq!(panel.suggestions().count(), 0);

        // Asked about from the view: attached once, with a picture taken
        // when sent.
        panel.ask_about(context("Face of Top"));
        assert!(panel.focus_input);
        panel.suggest(Some(context("Face of Top")), None);
        assert_eq!(panel.suggestions().count(), 0, "attached already");
        panel.attach_view = true;
        panel.awaiting_view = Some(("this face".into(), false));
        assert!(panel.wants_view());
        assert!(!panel.wants_view(), "asked once");
        panel.send_with_view(Some(vec![1, 2, 3]));
        panel.poll();
        assert!(!panel.awaits_view());
        assert_eq!(sent(&panel), [context("Face of Top"), Attachment::Image { png: vec![1, 2, 3] }]);
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

    #[test]
    fn a_new_folder_makes_a_new_engine_without_the_old_session() {
        // The folder and the session of each engine made.
        type Made = (Option<PathBuf>, Option<String>);
        let made: Arc<Mutex<Vec<Made>>> = Arc::default();
        let log = made.clone();
        let mut panel = AgentPanel::new(
            move |request: &EngineRequest| {
                log.lock().unwrap().push((request.folder.map(Path::to_path_buf), request.transcript.session_id.clone()));
                Built::ready(Box::new(FakeEngine::new(vec![turn()])), "fake")
            },
            None,
        );
        let ctx = egui::Context::default();
        let scratch = PathBuf::from("/settings/agent-scratch/0");
        panel.set_tab(None, Some(scratch.clone()));
        panel.configure(&ctx, &AgentSettings::default(), &[]);
        panel.send("a cube");
        panel.poll();
        assert_eq!(panel.transcript().session_id.as_deref(), Some("s1"));
        // The untitled tab saved: its folder now.
        let file = PathBuf::from("/work/box.c2f");
        panel.set_tab(Some(&file), Some(PathBuf::from("/work")));
        panel.configure(&ctx, &AgentSettings::default(), &[]);
        let made = made.lock().unwrap();
        assert_eq!(made.last().unwrap(), &(Some(PathBuf::from("/work")), None), "{made:?}");
        assert!(panel.transcript().session_id.is_none(), "no --resume across folders");
    }

    #[test]
    fn a_session_claude_code_lost_is_started_again_once_without_it() {
        let lost = || vec![Event::Error { message: "Claude Code stopped before the end of the turn (exit status: 1): No conversation found with session ID: s1".into() }];
        let sessions: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let log = sessions.clone();
        let mut panel = AgentPanel::new(
            move |request: &EngineRequest| {
                log.lock().unwrap().push(request.transcript.session_id.clone());
                let turns = if request.transcript.session_id.is_some() { vec![lost(), lost()] } else { vec![turn()] };
                Built::ready(Box::new(FakeEngine::new(turns)), "fake")
            },
            None,
        );
        panel.transcript.session_id = Some("s1".into());
        panel.build();
        panel.send("a cube");
        while panel.running() {
            panel.poll();
        }
        assert_eq!(sessions.lock().unwrap().last().unwrap(), &None, "made again without the session");
        let t = panel.transcript();
        assert!(!t.entries.iter().any(|e| matches!(e, Entry::Notice { error: true, .. })), "{:?}", t.entries);
        assert!(t.entries.iter().any(|e| matches!(e, Entry::Assistant { .. })), "answered");
        assert_eq!(t.entries.iter().filter(|e| matches!(e, Entry::User { .. })).count(), 1, "sent once");
    }

    #[test]
    fn pictures_are_kept_as_base64_and_old_arrays_still_read() {
        let mut transcript = Transcript::default();
        let png = vec![0x89, b'P', b'N', b'G', 0, 255];
        let message = UserMessage { text: "this".into(), attachments: vec![Attachment::Image { png: png.clone() }] };
        transcript.push_user(message.clone(), None);
        let text = serde_json::to_string(&transcript).unwrap();
        assert!(text.contains("\"png\":\"iVBORwD/\""), "{text}");
        assert_eq!(serde_json::from_str::<Transcript>(&text).unwrap(), transcript);
        let old = r#"{"entries":[{"type":"user","message":{"text":"this","attachments":[{"type":"image","png":[137,80,78,71,0,255]}]}}]}"#;
        let read: Transcript = serde_json::from_str(old).unwrap();
        assert_eq!(read.entries[0], Entry::User { message, shown: None });
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

    fn brief_saved(path: &Path, components: &[&str]) -> Entry {
        let mut requirements = vec![json!({ "line": 5, "text": "mass < 400 g", "kind": "mass", "op": "<", "value": 400, "checkable": true })];
        requirements.extend(components.iter().map(|c| json!({ "line": 6, "text": c, "kind": "component", "op": null, "value": c, "checkable": true })));
        let result = json!({ "saved": true, "path": path.display().to_string(), "requirements": requirements, "warnings": [] });
        Entry::Tool(ToolCall {
            id: "b1".into(),
            name: "mcp__code2form__save_brief".into(),
            input: json!({ "path": path.display().to_string(), "text": "# Winder" }),
            status: Status::Ok,
            content: vec![json!({ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap() })],
        })
    }

    fn sent(text: &str) -> Entry {
        Entry::User { message: UserMessage::text(text), shown: None }
    }

    #[test]
    fn a_saved_brief_offers_to_draw_it_then_its_electronics() {
        let dir = temp_store("next-steps");
        std::fs::create_dir_all(&dir).unwrap();
        let brief = dir.join("brief.md");
        let design = dir.join("winder.c2f");
        let mut entries = vec![sent("plan a winder"), brief_saved(&brief, &["28BYJ-48.c2f"]), Entry::Assistant { text: "Saved.".into() }];
        assert_eq!(next_steps(&entries, None), [NextStep::Design { brief: brief.clone() }]);

        // A brief refused, or another tool, offers nothing.
        let Entry::Tool(call) = &entries[1] else { panic!() };
        assert_eq!(next_steps(&[Entry::Tool(ToolCall { status: Status::Error, ..call.clone() })], None), []);
        let refused = json!({ "saved": false, "why": ["it has no `# Title`"] }).to_string();
        let refused = ToolCall { content: vec![json!({ "type": "text", "text": refused })], ..call.clone() };
        assert_eq!(next_steps(&[Entry::Tool(refused)], None), []);
        assert_eq!(next_steps(&[Entry::Tool(ToolCall { name: "read_brief".into(), ..call.clone() })], None), []);

        // The chip's prompt draws from the brief, in the tab's folder.
        let step = NextStep::Design { brief: brief.clone() };
        let text = step.prompt(Some(&dir)).unwrap();
        assert!(text.starts_with("Design this object with Code2Form"), "{text}");
        assert!(text.contains(&format!("Brief: {}", brief.display())));
        assert!(text.contains(&format!("Save it in: {}", dir.display())));

        // Once drawn and saved, its electronics, with the brief's components.
        entries.push(sent(&text));
        assert_eq!(next_steps(&entries, None), [], "no design saved yet");
        assert_eq!(next_steps(&entries, Some(&design)), [], "the file is not there");
        std::fs::write(&design, "cube(10);").unwrap();
        let electronics = NextStep::Electronics { design: design.clone(), brief: brief.clone(), components: "28BYJ-48.c2f".into() };
        assert_eq!(next_steps(&entries, Some(&design)), [electronics.clone()]);
        let text = electronics.prompt(None).unwrap();
        assert!(text.starts_with(&format!("Add the electronics to the approved design {}", design.display())), "{text}");
        assert!(text.contains("Components: 28BYJ-48.c2f") && text.contains(&format!("Brief: {}", brief.display())));
        entries.push(sent(&text));
        assert_eq!(next_steps(&entries, Some(&design)), []);

        // A brief without components stops at the drawing.
        let plain = vec![brief_saved(&brief, &[]), sent("Design this object with Code2Form, …")];
        assert_eq!(next_steps(&plain, Some(&design)), []);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_brief_in_a_scratch_folder_is_pointed_out() {
        let scratch = agent_setup::scratch_folder("tab-14").join("brief.md");
        assert_eq!(brief_in_scratch(&[brief_saved(&scratch, &[])]), Some(scratch));
        assert_eq!(brief_in_scratch(&[brief_saved(Path::new("/work/winder/brief.md"), &[])]), None);
        assert_eq!(brief_in_scratch(&[]), None);
    }

    #[test]
    fn edits_show_as_diff_cards() {
        let ok = |name: &str, input: Value, result: Value| ToolCall {
            id: "e".into(),
            name: name.into(),
            input,
            status: Status::Ok,
            content: vec![json!({ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap() })],
        };
        // `edits`, as live.rs takes them, and its result's changed lines.
        let edit = ok(
            "edit_code",
            json!({ "edits": [{ "old": "w = 20;\nh = 5;", "new": "w = 30;\nh = 5;" }] }),
            json!({ "edited": true, "changed_lines": [[3, 3]], "revision": "r2", "render": { "ok": true } }),
        );
        let card = diff_card(&edit).unwrap();
        assert_eq!(card.line, Some(3));
        assert_eq!(card.lines, [(review::Change::Removed, "w = 20;".to_string()), (review::Change::Added, "w = 30;".to_string())]);
        assert_eq!(card.more, 0);
        // As Claude Code names the tool.
        let named = ToolCall { name: "mcp__code2form-studio__edit_code".into(), ..edit.clone() };
        assert_eq!(diff_card(&named), Some(card));
        // A range: the lines that replace it, cut to six.
        let new = (1..=9).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let range = ok(
            "edit_code",
            json!({ "range": { "start_line": 7, "end_line": 8 }, "new": new, "revision": "r" }),
            json!({ "edited": true, "changed_lines": [[7, 15]] }),
        );
        let card = diff_card(&range).unwrap();
        assert_eq!((card.line, card.lines.len(), card.more), (Some(7), DIFF_LINES, 3));
        assert!(card.lines.iter().all(|(change, _)| *change == review::Change::Added));
        // Failed or still running: the tool's row instead.
        assert_eq!(diff_card(&ToolCall { status: Status::Error, ..edit.clone() }), None);
        assert_eq!(diff_card(&ToolCall { status: Status::Running, ..edit }), None);
        // A circuit's edit: what it did.
        let placed = ok("place_component", json!({ "name": "button" }), json!({ "done": "placed `button`", "check": {} }));
        assert_eq!(diff_card(&placed).unwrap().done.as_deref(), Some("placed `button`"));
        assert_eq!(diff_card(&ok("render", json!({}), json!({ "done": "x" }))), None);
        assert_eq!(diff_card(&ok("my_edit_code", json!({}), json!({}))), None, "not the studio's");
    }

    #[test]
    fn suggestions_follow_the_open_file() {
        let labels = |subject| starters(subject).into_iter().map(|s| s.label).collect::<Vec<_>>();
        let design = labels(Subject::Design { parameter: Some("width") });
        assert_eq!(design.len(), 4);
        assert_eq!(design[0], "Make width bigger");
        assert!(design.contains(&"Check it prints without supports".to_string()));
        assert!(design.contains(&"Make it lighter".to_string()));
        let plain = labels(Subject::Design { parameter: None });
        assert!(!plain.iter().any(|l| l.contains("bigger")), "no parameter to name");
        let circuit = labels(Subject::Circuit);
        assert!(circuit.contains(&"Check the wiring".to_string()));
        assert!(circuit.contains(&"Guide me through the assembly".to_string()));
        assert!(!labels(Subject::Empty).is_empty());
        // Each sends a message of its own.
        let starter = &starters(Subject::Design { parameter: Some("width") })[0];
        assert!(starter.message.contains("`width`"));
    }

    #[test]
    fn the_plan_starts_with_a_note_of_where_it_is_written() {
        let mut panel = AgentPanel::new(|_: &EngineRequest| Built::ready(Box::new(EchoEngine), "echo"), None);
        let scratch = PathBuf::from("/settings/agent-scratch/tab-1");
        panel.set_tab(None, Some(scratch.clone()));
        panel.start_plan("a lamp");
        panel.poll();
        let entries = &panel.transcript().entries;
        let Entry::Notice { text, error: false } = &entries[0] else { panic!("{entries:?}") };
        assert!(text.contains("/settings/agent-scratch/tab-1") && text.contains("save"), "{text}");
        let Entry::User { message, shown } = &entries[1] else { panic!("{entries:?}") };
        assert_eq!(message.text, prompt_text(Shortcut::Plan, "a lamp", None, Some(&scratch)).unwrap());
        assert_eq!(shown.as_deref(), Some("Plan · a lamp"));
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
