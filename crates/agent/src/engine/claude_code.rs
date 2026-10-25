//! The user's own Claude Code as the agent's engine: the studio runs the
//! `claude` CLI they installed and logged in with their subscription, in
//! its non-interactive streaming mode (`claude -p` with stream-json in and
//! out), and gives it the Code2Form tools through an MCP server it starts.
//!
//! One process lives across the turns of a conversation: each message is
//! one JSON line on its standard input, and a reader thread turns the JSON
//! lines it prints into [`Event`]s. Stopping a turn sends the CLI's
//! interrupt control request, and kills the process when that does not end
//! the turn within a second; the next message then starts a new process
//! that resumes the same session.
//!
//! The engine never logs in nor sees a credential: the user's CLI does.
//! Driving one's own Claude Code install this way is fine for personal
//! use; distributing an app that offers others a claude.ai login needs
//! Anthropic's approval, so check Anthropic's current terms (the Consumer
//! Terms and the Claude Code legal and compliance page) before shipping it
//! to other people.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::conversation::{Attachment, Engine, Event, Usage, UserMessage};

/// The MCP server's name in the CLI's configuration: its tools are named
/// `mcp__code2form__<tool>`.
const SERVER: &str = "code2form";

/// How long an interrupted turn may take to end before the process is
/// killed.
const STOP_GRACE: Duration = Duration::from_millis(800);

/// What the agent is told on top of Claude Code's own system prompt.
const SYSTEM_PROMPT: &str = "You run inside the Code2Form studio, where parts are programs in the .scad \
language: the user sees the open document, its code and its 3D view. Prefer the code2form tools: the live ones \
(`studio_state` to read the open document and what is selected, `edit_code` to change it, and the others the \
studio offers) act on what the user sees; `render`, `image` and the checks verify your work. Change the open \
document through those tools rather than by writing files, unless the user asks for a file. Keep your replies \
short: the user reads them beside the model.";

/// Where Claude Code is, or `None` when it is not installed: the path the
/// user `configured`, then the `PATH`, then the places its installers put
/// it. An app started from the macOS Finder or Dock does not get the
/// shell's `PATH`, hence the usual places.
pub fn find_claude(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured {
        return path.is_file().then(|| path.to_path_buf());
    }
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(usual_dirs())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The folders Claude Code's installers (native, Homebrew, npm) put it in.
fn usual_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(home) = &home {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".volta/bin"));
        dirs.push(home.join(".bun/bin"));
        // nvm keeps one folder per Node version.
        if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            dirs.extend(versions.flatten().map(|version| version.path().join("bin")));
        }
    }
    if let Some(prefix) = std::env::var_os("NPM_CONFIG_PREFIX") {
        let prefix = PathBuf::from(prefix);
        dirs.push(if cfg!(windows) { prefix.clone() } else { prefix.join("bin") });
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    } else {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
        dirs.push("/usr/bin".into());
    }
    dirs
}

/// The `PATH` Claude Code runs with: its own folder, the inherited `PATH`,
/// then the usual folders, so that an npm install finds `node` even when
/// the studio was started without the shell's `PATH`.
fn search_path(claude: &Path) -> OsString {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let all = claude.parent().map(Path::to_path_buf).into_iter().chain(std::env::split_paths(&inherited)).chain(usual_dirs());
    for dir in all {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or(inherited)
}

/// The oldest Claude Code with the flags this engine passes
/// (`--restricted`, `--permission-prompts`). 2.1.310 has them; no
/// changelog was at hand to tell the first release that did, so this is a
/// floor below it, which a newer finding may raise.
pub const MIN_VERSION: &str = "2.1.300";

/// What the studio can tell the user about their Claude Code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeCodeStatus {
    /// No `claude` there, or it does not run.
    Missing,
    /// It runs, but is older than [`MIN_VERSION`]: it would refuse the
    /// command line.
    TooOld { version: String, needed: String },
    /// It runs, but says nothing of its login: a turn tells.
    Found { version: String },
    /// It runs and is logged out: `claude` then `/login` in a terminal.
    NotLoggedIn { version: String },
    /// It runs and is logged in.
    Ready { version: String },
}

impl ClaudeCodeStatus {
    /// What to tell the user, in a sentence.
    pub fn message(&self) -> String {
        match self {
            ClaudeCodeStatus::Missing => "Claude Code — not found".into(),
            ClaudeCodeStatus::TooOld { version, needed } => {
                format!("Claude Code {version} — too old: {needed} or newer is needed (run `claude update`)")
            }
            ClaudeCodeStatus::Found { version } => format!("Claude Code {version} — found"),
            ClaudeCodeStatus::NotLoggedIn { version } => {
                format!("Claude Code {version} — not logged in: run `claude`, then /login")
            }
            ClaudeCodeStatus::Ready { version } => format!("Claude Code {version} — ready"),
        }
    }
}

impl std::fmt::Display for ClaudeCodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Whether the version `version` ("2.1.310") is older than `needed`;
/// `None` when either does not read as numbers.
fn older(version: &str, needed: &str) -> Option<bool> {
    let numbers = |v: &str| v.split('.').map(|n| n.parse::<u64>().ok()).collect::<Option<Vec<u64>>>();
    Some(numbers(version)? < numbers(needed)?)
}

/// The state of the Claude Code at `path`, from `claude --version` and
/// `claude auth status --json`; neither reaches the network nor costs a
/// thing, but each starts a process, so call it off the UI thread.
pub fn status(path: &Path) -> ClaudeCodeStatus {
    let run = |args: &[&str]| {
        Command::new(path)
            .args(args)
            .env("PATH", search_path(path))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
    };
    let Some(output) = run(&["--version"]).filter(|output| output.status.success()) else {
        return ClaudeCodeStatus::Missing;
    };
    // "2.1.310 (Claude Code)"
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.split_whitespace().next().unwrap_or("").to_string();
    if older(&version, MIN_VERSION) == Some(true) {
        return ClaudeCodeStatus::TooOld { version, needed: MIN_VERSION.into() };
    }
    // `auth status` exits 1 when logged out, with the JSON all the same.
    let logged_in = run(&["auth", "status", "--json"])
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok())
        .and_then(|status| status.get("loggedIn").and_then(Value::as_bool));
    match logged_in {
        Some(true) => ClaudeCodeStatus::Ready { version },
        Some(false) => ClaudeCodeStatus::NotLoggedIn { version },
        None => ClaudeCodeStatus::Found { version },
    }
}

/// What Claude Code may do on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permission {
    /// The Code2Form tools, and reading and editing the files of the
    /// project folder, without asking.
    #[default]
    ApplyDirectly,
    /// The Code2Form tools and reading the project's files only: changes go
    /// through the studio's own tools, which ask the user in the studio.
    AskFirst,
}

/// How to run Claude Code.
#[derive(Clone, Debug)]
pub struct ClaudeCodeOptions {
    /// The `claude` executable, as [`find_claude`] finds it.
    pub claude: PathBuf,
    /// The project folder: Claude Code works in it, and its file tools
    /// reach nothing outside it.
    pub cwd: PathBuf,
    /// How Claude Code starts the Code2Form MCP server: the program and
    /// its arguments, such as the studio's own executable with `["mcp",
    /// "--studio"]`, or `c2f` with `["mcp"]`.
    pub mcp_command: (PathBuf, Vec<String>),
    /// The model, an alias (`opus`, `sonnet`) or a full name; the CLI's
    /// own default when `None`.
    pub model: Option<String>,
    /// The session to continue, as a turn's [`Event::Done`] gave it.
    pub resume: Option<String>,
    pub permission: Permission,
    /// With [`Permission::ApplyDirectly`], Claude Code may write new
    /// files in the project folder (its `Write` tool), not only edit them.
    pub write_files: bool,
    /// The most agentic turns an answer may take (`--max-turns`); the
    /// CLI's own limit when `None`.
    pub max_turns: Option<u32>,
}

/// The command line, after the executable: print mode with stream-json in
/// and out, the Code2Form server alone, the tools allowed and the session
/// to resume, if any.
fn arguments(options: &ClaudeCodeOptions, resume: Option<&str>) -> Vec<String> {
    let (program, args) = &options.mcp_command;
    let mcp = json!({ "mcpServers": { SERVER: { "type": "stdio", "command": program, "args": args } } });
    let (tools, mode) = match options.permission {
        Permission::ApplyDirectly if options.write_files => ("Read,Glob,Grep,Edit,Write", "acceptEdits"),
        Permission::ApplyDirectly => ("Read,Glob,Grep,Edit", "acceptEdits"),
        Permission::AskFirst => ("Read,Glob,Grep", "dontAsk"),
    };
    let mut arguments: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--restricted",
        "--strict-mcp-config",
        "--mcp-config",
        &mcp.to_string(),
        "--tools",
        tools,
        "--allowedTools",
        &format!("mcp__{SERVER}__*,{tools}"),
        "--permission-mode",
        mode,
        "--permission-prompts",
        "none",
        "--append-system-prompt",
        SYSTEM_PROMPT,
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    if let Some(model) = &options.model {
        arguments.extend(["--model".into(), model.clone()]);
    }
    if let Some(turns) = options.max_turns {
        arguments.extend(["--max-turns".into(), turns.to_string()]);
    }
    if let Some(session) = resume {
        arguments.extend(["--resume".into(), session.to_string()]);
    }
    arguments
}

/// A user message as a stream-json input line: its text, then its
/// attachments, pictures as base64 PNG and selections as text.
fn user_line(message: &UserMessage) -> String {
    let mut content = vec![json!({ "type": "text", "text": message.text })];
    for attachment in &message.attachments {
        content.push(match attachment {
            Attachment::Image { png } => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": "image/png", "data": crate::mcp::base64(png) },
            }),
            Attachment::Context { label, text } => json!({ "type": "text", "text": format!("{label}:\n{text}") }),
        });
    }
    json!({ "type": "user", "message": { "role": "user", "content": content } }).to_string()
}

/// Turns the CLI's stream-json output lines into [`Event`]s.
#[derive(Debug, Default)]
struct Translator {
    /// The session the CLI runs, from any line that names it.
    session: Option<String>,
    /// Whether the message under way came as partial text, so that its
    /// full copy is not shown twice.
    streamed: bool,
    /// What went wrong, said before the result that ends the turn.
    error: Option<String>,
}

impl Translator {
    /// The events of one output line; none for the lines the studio does
    /// not show (hooks, status, rate limits allowed) or does not read.
    fn line(&mut self, line: &str) -> Vec<Event> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        if let Some(session) = value.get("session_id").and_then(Value::as_str) {
            self.session = Some(session.to_string());
        }
        // A subagent's messages are its own business.
        if value.get("parent_tool_use_id").is_some_and(|parent| !parent.is_null()) {
            return Vec::new();
        }
        match value["type"].as_str().unwrap_or("") {
            "stream_event" => self.stream_event(&value["event"]),
            "assistant" => self.assistant(&value),
            "user" => tool_results(&value["message"]["content"]),
            "rate_limit_event" => {
                let info = &value["rate_limit_info"];
                if info["status"] == "rejected" && self.error.is_none() {
                    let window = info["rateLimitType"].as_str().unwrap_or("usage").replace('_', " ");
                    self.error = Some(format!("Claude Code reached its {window} limit; try again once it resets."));
                }
                Vec::new()
            }
            "result" => vec![self.result(&value)],
            _ => Vec::new(),
        }
    }

    fn stream_event(&mut self, event: &Value) -> Vec<Event> {
        match event["type"].as_str() {
            Some("message_start") => self.streamed = false,
            Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                if let Some(text) = event["delta"]["text"].as_str().filter(|text| !text.is_empty()) {
                    self.streamed = true;
                    return vec![Event::Text { delta: text.to_string() }];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn assistant(&mut self, value: &Value) -> Vec<Event> {
        let blocks = value["message"]["content"].as_array().map(Vec::as_slice).unwrap_or_default();
        // The CLI says its own errors (logged out, limits) as an assistant
        // message: they end the turn as its error, not as a reply.
        if let Some(kind) = value.get("error").and_then(Value::as_str) {
            let text: String = blocks.iter().filter_map(|block| block["text"].as_str()).collect();
            self.error = Some(error_message(kind, &text));
            return Vec::new();
        }
        let mut events = Vec::new();
        for block in blocks {
            match block["type"].as_str() {
                Some("text") if !self.streamed => {
                    if let Some(text) = block["text"].as_str().filter(|text| !text.is_empty()) {
                        events.push(Event::Text { delta: text.to_string() });
                    }
                }
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or("");
                    events.push(Event::ToolCall {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: name.strip_prefix(&format!("mcp__{SERVER}__")).unwrap_or(name).to_string(),
                        input: block["input"].clone(),
                    });
                }
                _ => {}
            }
        }
        events
    }

    /// The turn's end: [`Event::Done`], or the error that ended it.
    fn result(&mut self, value: &Value) -> Event {
        let session = value["session_id"].as_str().map(str::to_string).or_else(|| self.session.clone());
        let error = self.error.take();
        if value["is_error"] != true && value["subtype"] == "success" {
            let usage = &value["usage"];
            let count = |key: &str| usage[key].as_u64().unwrap_or(0);
            let input = count("input_tokens") + count("cache_creation_input_tokens") + count("cache_read_input_tokens");
            return Event::Done {
                usage: usage.is_object().then(|| Usage { input_tokens: input, output_tokens: count("output_tokens") }),
                cost_usd: value["total_cost_usd"].as_f64(),
                session_id: session,
            };
        }
        let said = value["result"].as_str().filter(|text| !text.is_empty()).map(|text| format!("Claude Code: {text}"));
        let listed = value["errors"].as_array().filter(|errors| !errors.is_empty()).map(|errors| {
            let errors: Vec<&str> = errors.iter().filter_map(Value::as_str).collect();
            format!("Claude Code stopped: {}", errors.join("; "))
        });
        let message = error.or(said).or(listed).unwrap_or_else(|| {
            format!("Claude Code stopped ({})", value["subtype"].as_str().unwrap_or("an unknown error"))
        });
        Event::Error { message }
    }
}

/// What the user reads for an error the CLI reports as an assistant
/// message of `kind`, saying `text`.
fn error_message(kind: &str, text: &str) -> String {
    match kind {
        "authentication_failed" => "Claude Code is not logged in: open a terminal, run `claude`, log in with /login, \
                                    then send your message again."
            .to_string(),
        "rate_limit" => format!("Claude Code reached its usage limit: {text}"),
        "billing_error" => format!("Claude Code cannot bill this request: {text}"),
        _ if text.is_empty() => format!("Claude Code failed ({kind})"),
        _ => format!("Claude Code: {text}"),
    }
}

/// The tool results a user line carries back, as MCP content items.
fn tool_results(content: &Value) -> Vec<Event> {
    let Some(blocks) = content.as_array() else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block["type"] == "tool_result")
        .map(|block| {
            let content = match &block["content"] {
                Value::String(text) => vec![json!({ "type": "text", "text": text })],
                Value::Array(items) => items.iter().map(mcp_item).collect(),
                _ => Vec::new(),
            };
            Event::ToolResult {
                id: block["tool_use_id"].as_str().unwrap_or("").to_string(),
                content,
                is_error: block["is_error"] == true,
            }
        })
        .collect()
}

/// An Anthropic content block as an MCP content item: an image's base64
/// source becomes `{"type": "image", "data", "mimeType"}`.
fn mcp_item(block: &Value) -> Value {
    if block["type"] == "image" && block["source"]["type"] == "base64" {
        return json!({ "type": "image", "data": block["source"]["data"], "mimeType": block["source"]["media_type"] });
    }
    block.clone()
}

/// What a running process and its reader thread share.
struct Shared {
    /// Where the turn under way sends its events.
    events: Mutex<Option<Sender<Event>>>,
    /// A turn is under way: sent, and not yet done.
    turn: AtomicBool,
    /// How many turns were sent, to tell the turn under way from the
    /// next one.
    turns: AtomicU64,
    /// The turn the user asked to stop (its number in `turns`), 0 for none:
    /// a stop that comes as a turn ends never stops the next one.
    stopping: AtomicU64,
    /// The process has ended.
    ended: AtomicBool,
    /// The last of what the process wrote on its standard error.
    stderr: Mutex<String>,
}

impl Shared {
    /// Whether the turn sent `turns`-th is still under way.
    fn open(&self, turns: u64) -> bool {
        self.turn.load(Ordering::SeqCst) && self.turns.load(Ordering::SeqCst) == turns
    }

    /// Whether the user asked to stop the turn under way.
    fn stopping(&self) -> bool {
        let stopping = self.stopping.load(Ordering::SeqCst);
        stopping != 0 && stopping == self.turns.load(Ordering::SeqCst)
    }

    /// Starts the next turn: its number.
    fn begin(&self) -> u64 {
        self.stopping.store(0, Ordering::SeqCst);
        let turn = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
        self.turn.store(true, Ordering::SeqCst);
        turn
    }

    /// Marks the turn under way as stopped: the turn it is, or `None`
    /// when none is under way.
    fn request_stop(&self) -> Option<u64> {
        let turn = self.turns.load(Ordering::SeqCst);
        if !self.open(turn) {
            return None;
        }
        self.stopping.store(turn, Ordering::SeqCst);
        Some(turn)
    }

    fn send(&self, event: Event) {
        if matches!(event, Event::Done { .. } | Event::Error { .. }) {
            self.turn.store(false, Ordering::SeqCst);
            self.stopping.store(0, Ordering::SeqCst);
        }
        if let Some(events) = self.events.lock().unwrap().as_ref() {
            // A listener gone stops nothing.
            let _ = events.send(event);
        }
    }
}

/// A running `claude` process.
struct Process {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    shared: Arc<Shared>,
}

/// The engine that drives the user's Claude Code: see the [module](self).
pub struct ClaudeCodeEngine {
    options: ClaudeCodeOptions,
    /// The session to resume when a new process starts, kept up to date by
    /// the reader threads.
    session: Arc<Mutex<Option<String>>>,
    process: Option<Process>,
    requests: AtomicU64,
    /// Let Claude Code use `ANTHROPIC_API_KEY` (or `ANTHROPIC_AUTH_TOKEN`)
    /// from the environment; off, it uses the user's subscription.
    use_api_key_env: bool,
}

impl ClaudeCodeEngine {
    /// An engine with `options`; it starts Claude Code at its first
    /// message.
    pub fn new(options: ClaudeCodeOptions) -> ClaudeCodeEngine {
        let session = Arc::new(Mutex::new(options.resume.clone()));
        ClaudeCodeEngine { options, session, process: None, requests: AtomicU64::new(0), use_api_key_env: false }
    }

    /// Whether Claude Code may bill an API key the environment gives
    /// (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`) rather than the user's
    /// subscription: off by default, as an inherited key would otherwise
    /// win without the user knowing.
    pub fn use_api_key_env(mut self, use_it: bool) -> ClaudeCodeEngine {
        self.use_api_key_env = use_it;
        self
    }

    /// The command that starts Claude Code, without its arguments.
    fn command(&self) -> Command {
        let claude = &self.options.claude;
        let mut command = Command::new(claude);
        command.current_dir(&self.options.cwd).env("PATH", search_path(claude));
        if !self.use_api_key_env {
            command.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN");
        }
        command
    }

    /// The session to resume this conversation with, once a turn named it.
    pub fn session_id(&self) -> Option<String> {
        self.session.lock().unwrap().clone()
    }

    fn start(&mut self) -> Result<Process, String> {
        let claude = &self.options.claude;
        let resume = self.session_id();
        let mut child = self
            .command()
            .args(arguments(&self.options, resume.as_deref()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("could not start Claude Code ({}): {err}", claude.display()))?;
        let stdin = child.stdin.take().expect("a piped stdin");
        let stdout = child.stdout.take().expect("a piped stdout");
        let mut stderr = child.stderr.take().expect("a piped stderr");
        let shared = Arc::new(Shared {
            events: Mutex::new(None),
            turn: AtomicBool::new(false),
            turns: AtomicU64::new(0),
            stopping: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            stderr: Mutex::new(String::new()),
        });
        let child = Arc::new(Mutex::new(child));

        let errors = shared.clone();
        let stderr_thread = thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read) = stderr.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut kept = errors.stderr.lock().unwrap();
                kept.push_str(&String::from_utf8_lossy(&buffer[..read]));
                // Only the end tells why it failed.
                if kept.len() > 8192 {
                    let mut cut = kept.len() - 4096;
                    while !kept.is_char_boundary(cut) {
                        cut += 1;
                    }
                    kept.drain(..cut);
                }
            }
        });

        let (reader, process, session) = (shared.clone(), child.clone(), self.session.clone());
        thread::spawn(move || {
            let mut translator = Translator::default();
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let events = translator.line(&line);
                if let Some(id) = &translator.session {
                    *session.lock().unwrap() = Some(id.clone());
                }
                for mut event in events {
                    // A stopped turn ends as done, whatever the CLI says.
                    if matches!(event, Event::Error { .. }) && reader.stopping() {
                        event = Event::Done { usage: None, cost_usd: None, session_id: translator.session.clone() };
                    }
                    reader.send(event);
                }
            }
            let _ = stderr_thread.join();
            reader.ended.store(true, Ordering::SeqCst);
            if !reader.turn.load(Ordering::SeqCst) {
                return;
            }
            if reader.stopping() {
                reader.send(Event::Done { usage: None, cost_usd: None, session_id: translator.session.clone() });
                return;
            }
            let code = exit_code(&process);
            let stderr = reader.stderr.lock().unwrap().trim().to_string();
            let mut message = format!("Claude Code stopped before the end of the turn ({code})");
            if !stderr.is_empty() {
                message.push_str(": ");
                message.push_str(&stderr);
            }
            reader.send(Event::Error { message });
        });
        Ok(Process { child, stdin, shared })
    }

    /// Writes `line` to the running process, starting one first when none
    /// runs.
    fn write(&mut self, line: &str, events: &Sender<Event>) -> Result<(), String> {
        if self.process.as_ref().is_none_or(|process| process.shared.ended.load(Ordering::SeqCst)) {
            self.kill();
            self.process = Some(self.start()?);
        }
        let process = self.process.as_mut().expect("a process");
        *process.shared.events.lock().unwrap() = Some(events.clone());
        process.shared.begin();
        let written = writeln!(process.stdin, "{line}").and_then(|()| process.stdin.flush());
        written.map_err(|err| {
            process.shared.turn.store(false, Ordering::SeqCst);
            format!("could not write to Claude Code: {err}")
        })
    }

    fn kill(&mut self) {
        if let Some(process) = self.process.take() {
            let mut child = process.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// How the process ended, once it has, for an error message.
fn exit_code(child: &Mutex<Child>) -> String {
    for _ in 0..20 {
        match child.lock().unwrap().try_wait() {
            Ok(Some(status)) => return status.to_string(),
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(err) => return err.to_string(),
        }
    }
    "still running".into()
}

impl Engine for ClaudeCodeEngine {
    fn send(&mut self, message: UserMessage, events: Sender<Event>) -> Result<(), String> {
        if self.process.as_ref().is_some_and(|process| process.shared.turn.load(Ordering::SeqCst)) {
            return Err("a turn is under way: stop it or wait for its end".into());
        }
        let line = user_line(&message);
        // A process that ended since its last turn is started again once.
        match self.write(&line, &events) {
            Err(_) if self.process.as_ref().is_some_and(|process| process.shared.ended.load(Ordering::SeqCst)) => {
                self.write(&line, &events)
            }
            result => result,
        }
    }

    fn stop(&mut self) {
        let Some(process) = self.process.as_mut() else { return };
        // Idle, nothing to interrupt: an interrupt now would reach the CLI
        // between turns.
        let Some(turn) = process.shared.request_stop() else { return };
        let id = self.requests.fetch_add(1, Ordering::SeqCst);
        let request = json!({ "type": "control_request", "request_id": format!("stop-{id}"), "request": { "subtype": "interrupt" } });
        let asked = writeln!(process.stdin, "{request}").and_then(|()| process.stdin.flush()).is_ok();
        // The next turn may start before the deadline: it is left alone.
        let (shared, child, session) = (process.shared.clone(), process.child.clone(), self.session.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + if asked { STOP_GRACE } else { Duration::ZERO };
            while Instant::now() < deadline {
                if !shared.open(turn) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            if !shared.open(turn) {
                return;
            }
            // The reader then sees the end of the output and ends the turn
            // as stopped, unless something the CLI started keeps the output
            // open: the turn then ends here.
            let _ = child.lock().unwrap().kill();
            thread::sleep(Duration::from_millis(150));
            if shared.open(turn) {
                shared.ended.store(true, Ordering::SeqCst);
                let session_id = session.lock().unwrap().clone();
                shared.send(Event::Done { usage: None, cost_usd: None, session_id });
            }
        });
    }

    fn name(&self) -> &str {
        "Claude Code"
    }
}

impl Drop for ClaudeCodeEngine {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENDER: &str = include_str!("../../tests/fixtures/claude_code_render.jsonl");
    const INTERRUPTED: &str = include_str!("../../tests/fixtures/claude_code_interrupted.jsonl");
    const LOGGED_OUT: &str = include_str!("../../tests/fixtures/claude_code_logged_out.jsonl");

    fn translate(lines: &str) -> Vec<Event> {
        let mut translator = Translator::default();
        lines.lines().flat_map(|line| translator.line(line)).collect()
    }

    fn text(events: &[Event]) -> String {
        events.iter().filter_map(|event| if let Event::Text { delta } = event { Some(delta.as_str()) } else { None }).collect()
    }

    #[test]
    fn a_recorded_turn_calls_render_and_answers() {
        let events = translate(RENDER);
        let Event::ToolCall { id, name, input } = &events[0] else { panic!("{events:?}") };
        assert_eq!(name, "render");
        assert_eq!(input, &json!({ "code": "cube([10,10,10]);" }));
        let Event::ToolResult { id: result, content, is_error } = &events[1] else { panic!("{events:?}") };
        assert_eq!((result, *is_error), (id, false));
        assert!(content[0]["text"].as_str().unwrap().contains("\"volume\": 1000.0"));
        // The streamed text, once: the full message after it is not repeated.
        assert_eq!(text(&events), "The cube's volume is 1000 mm³ (10 × 10 × 10 mm).");
        let Some(Event::Done { usage, cost_usd, session_id }) = events.last() else { panic!("{events:?}") };
        assert_eq!(session_id.as_deref(), Some("2ae90353-7b53-4672-b448-ce02b8b77858"));
        assert!(cost_usd.unwrap() > 0.0);
        assert_eq!(usage.unwrap(), Usage { input_tokens: 4 + 20535 + 20281, output_tokens: 93 });
        assert_eq!(events.iter().filter(|event| matches!(event, Event::Done { .. })).count(), 1);
    }

    #[test]
    fn full_messages_are_text_when_nothing_streamed() {
        let without_partials = RENDER.lines().filter(|line| !line.contains("\"stream_event\"")).collect::<Vec<_>>().join("\n");
        assert_eq!(text(&translate(&without_partials)), "The cube's volume is 1000 mm³ (10 × 10 × 10 mm).");
    }

    #[test]
    fn an_interrupted_turn_ends_with_an_error_the_engine_turns_into_done() {
        let events = translate(INTERRUPTED);
        assert!(text(&events).starts_with("# The Keeper"));
        let Some(Event::Error { message }) = events.last() else { panic!("{events:?}") };
        assert!(message.contains("Claude Code stopped"), "{message}");
    }

    #[test]
    fn a_logged_out_cli_is_a_clear_error() {
        let events = translate(LOGGED_OUT);
        assert_eq!(events.len(), 1, "{events:?}");
        let Event::Error { message } = &events[0] else { panic!("{events:?}") };
        assert!(message.contains("not logged in") && message.contains("/login"), "{message}");
    }

    #[test]
    fn tool_results_become_mcp_content() {
        let line = json!({ "type": "user", "message": { "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "t1", "content": "oops", "is_error": true },
            { "type": "tool_result", "tool_use_id": "t2", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBO" } },
            ] },
        ] } });
        let events = translate(&line.to_string());
        assert_eq!(events, vec![
            Event::ToolResult { id: "t1".into(), content: vec![json!({ "type": "text", "text": "oops" })], is_error: true },
            Event::ToolResult {
                id: "t2".into(),
                content: vec![json!({ "type": "image", "data": "iVBO", "mimeType": "image/png" })],
                is_error: false,
            },
        ]);
    }

    #[test]
    fn a_rejected_rate_limit_says_so() {
        let lines = [
            json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "rejected", "rateLimitType": "five_hour" } }),
            json!({ "type": "result", "subtype": "success", "is_error": true, "result": "" }),
        ];
        let events = translate(&lines.map(|line| line.to_string()).join("\n"));
        assert_eq!(events, vec![Event::Error { message: "Claude Code reached its five hour limit; try again once it resets.".into() }]);
    }

    #[test]
    fn messages_carry_their_attachments() {
        let message = UserMessage {
            text: "wider".into(),
            attachments: vec![
                Attachment::Image { png: b"png".to_vec() },
                Attachment::Context { label: "box.c2f:3-5".into(), text: "cube(10);".into() },
            ],
        };
        let line: Value = serde_json::from_str(&user_line(&message)).unwrap();
        assert_eq!(line["type"], "user");
        let content = &line["message"]["content"];
        assert_eq!(content[0], json!({ "type": "text", "text": "wider" }));
        assert_eq!(content[1]["source"], json!({ "type": "base64", "media_type": "image/png", "data": "cG5n" }));
        assert_eq!(content[2], json!({ "type": "text", "text": "box.c2f:3-5:\ncube(10);" }));
    }

    fn options(claude: PathBuf, permission: Permission) -> ClaudeCodeOptions {
        ClaudeCodeOptions {
            claude,
            cwd: std::env::temp_dir(),
            mcp_command: ("/bin/c2f".into(), vec!["mcp".into(), "--studio".into()]),
            model: Some("sonnet".into()),
            resume: None,
            permission,
            write_files: true,
            max_turns: None,
        }
    }

    #[test]
    fn the_command_line_confines_claude_code() {
        let args = arguments(&options("claude".into(), Permission::ApplyDirectly), Some("s1"));
        let after = |flag: &str| args[args.iter().position(|arg| arg == flag).unwrap_or_else(|| panic!("{flag}")) + 1].as_str();
        assert_eq!(after("--input-format"), "stream-json");
        assert_eq!(after("--output-format"), "stream-json");
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config") && args.iter().any(|arg| arg == "--restricted"));
        let mcp: Value = serde_json::from_str(after("--mcp-config")).unwrap();
        assert_eq!(mcp["mcpServers"]["code2form"], json!({ "type": "stdio", "command": "/bin/c2f", "args": ["mcp", "--studio"] }));
        assert_eq!(after("--allowedTools"), "mcp__code2form__*,Read,Glob,Grep,Edit,Write");
        assert_eq!(after("--permission-mode"), "acceptEdits");
        assert_eq!(after("--permission-prompts"), "none");
        assert_eq!((after("--model"), after("--resume")), ("sonnet", "s1"));
        let asking = arguments(&options("claude".into(), Permission::AskFirst), None);
        assert!(asking.contains(&"Read,Glob,Grep".to_string()) && asking.contains(&"dontAsk".to_string()));
        assert!(!asking.iter().any(|arg| arg == "--resume"));
        assert!(!asking.iter().any(|arg| arg == "--max-turns"));
        let limited = ClaudeCodeOptions { write_files: false, max_turns: Some(7), ..options("claude".into(), Permission::ApplyDirectly) };
        let limited = arguments(&limited, None);
        let after = |flag: &str| limited[limited.iter().position(|arg| arg == flag).unwrap() + 1].clone();
        assert_eq!(after("--tools"), "Read,Glob,Grep,Edit", "no Write");
        assert_eq!(after("--max-turns"), "7");
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert_eq!(older("2.1.299", "2.1.300"), Some(true));
        assert_eq!(older("2.1.310", "2.1.300"), Some(false));
        assert_eq!(older("2.10.0", "2.9.9"), Some(false));
        assert_eq!(older("2.1.300", "2.1.300"), Some(false));
        assert_eq!(older("dev", "2.1.300"), None);
    }

    #[test]
    fn the_subscription_is_used_unless_asked_otherwise() {
        let removed = |engine: &ClaudeCodeEngine| -> Vec<String> {
            let command = engine.command();
            command.get_envs().filter(|(_, value)| value.is_none()).map(|(key, _)| key.to_string_lossy().into_owned()).collect()
        };
        let engine = ClaudeCodeEngine::new(options("claude".into(), Permission::ApplyDirectly));
        let gone = removed(&engine);
        assert!(gone.iter().any(|k| k == "ANTHROPIC_API_KEY") && gone.iter().any(|k| k == "ANTHROPIC_AUTH_TOKEN"), "{gone:?}");
        let engine = ClaudeCodeEngine::new(options("claude".into(), Permission::ApplyDirectly)).use_api_key_env(true);
        assert!(removed(&engine).is_empty());
    }

    #[test]
    fn a_stop_that_comes_as_a_turn_ends_never_stops_the_next() {
        let shared = Shared {
            events: Mutex::new(None),
            turn: AtomicBool::new(false),
            turns: AtomicU64::new(0),
            stopping: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            stderr: Mutex::new(String::new()),
        };
        assert_eq!(shared.request_stop(), None, "idle: nothing to stop");
        let first = shared.begin();
        // The stop sees the turn under way...
        assert_eq!(shared.request_stop(), Some(first));
        // ...which ends meanwhile; the stop's mark lands after.
        shared.send(Event::Done { usage: None, cost_usd: None, session_id: None });
        shared.stopping.store(first, Ordering::SeqCst);
        assert!(shared.stopping(), "still the stopped turn's");
        shared.begin();
        assert!(!shared.stopping(), "the next turn is not stopped");
    }

    #[test]
    fn a_configured_path_that_is_not_there_is_not_found() {
        assert_eq!(find_claude(Some(Path::new("/no/such/claude"))), None);
        assert_eq!(status(Path::new("/no/such/claude")), ClaudeCodeStatus::Missing);
    }

    /// A fake `claude`, a shell script that plays recorded output: the
    /// tests never reach the real CLI nor spend a subscription.
    #[cfg(unix)]
    mod fake {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::mpsc;

        use super::*;

        /// A fresh folder holding a script `claude` with `body`, and the
        /// recorded turns as fixtures beside it.
        fn script(body: &str) -> PathBuf {
            static COUNT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!("c2f-fake-claude-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("render.jsonl"), RENDER).unwrap();
            std::fs::write(dir.join("logged_out.jsonl"), LOGGED_OUT).unwrap();
            let path = dir.join("claude");
            std::fs::write(&path, format!("#!/bin/sh\nDIR=\"$(dirname \"$0\")\"\n{body}")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        fn turn(receiver: &mpsc::Receiver<Event>) -> Vec<Event> {
            let mut events = Vec::new();
            loop {
                let event = receiver.recv_timeout(Duration::from_secs(10)).expect("the turn's end");
                let last = matches!(event, Event::Done { .. } | Event::Error { .. });
                events.push(event);
                if last {
                    return events;
                }
            }
        }

        /// Answers each user line with the recorded render turn, and writes
        /// down its arguments and input.
        const PLAYER: &str = r#"printf '%s\n' "$@" > "$DIR/args.txt"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$DIR/stdin.txt"
  case "$line" in
    *'"type":"user"'*) cat "$DIR/render.jsonl" ;;
  esac
done
"#;

        #[test]
        fn the_status_reads_version_and_login() {
            let ready = script("case \"$1\" in --version) echo '2.1.310 (Claude Code)';; auth) echo '{\"loggedIn\": true}';; esac\n");
            assert_eq!(status(&ready), ClaudeCodeStatus::Ready { version: "2.1.310".into() });
            let out = script("case \"$1\" in --version) echo '2.1.310 (Claude Code)';; auth) echo '{\"loggedIn\": false}'; exit 1;; esac\n");
            assert_eq!(status(&out), ClaudeCodeStatus::NotLoggedIn { version: "2.1.310".into() });
            let found = script("case \"$1\" in --version) echo '2.1.310 (Claude Code)';; *) exit 2;; esac\n");
            assert_eq!(status(&found), ClaudeCodeStatus::Found { version: "2.1.310".into() });
            assert_eq!(find_claude(Some(&found)), Some(found.clone()));
            // Older than the flags it is given: said so, not run.
            let old = script("case \"$1\" in --version) echo '1.0.0 (Claude Code)';; auth) echo '{\"loggedIn\": true}';; esac\n");
            let too_old = status(&old);
            assert_eq!(too_old, ClaudeCodeStatus::TooOld { version: "1.0.0".into(), needed: MIN_VERSION.into() });
            assert!(too_old.to_string().contains("too old") && too_old.message().contains(MIN_VERSION), "{too_old}");
        }

        #[test]
        fn a_stop_when_idle_sends_no_interrupt() {
            let claude = script(PLAYER);
            let dir = claude.parent().unwrap().to_path_buf();
            let mut engine = ClaudeCodeEngine::new(options(claude, Permission::ApplyDirectly));
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("a cube"), sender.clone()).unwrap();
            turn(&receiver);
            engine.stop();
            engine.send(UserMessage::text("again"), sender).unwrap();
            assert!(matches!(turn(&receiver).last(), Some(Event::Done { .. })));
            let stdin = std::fs::read_to_string(dir.join("stdin.txt")).unwrap();
            assert!(!stdin.contains("interrupt"), "{stdin}");
        }

        #[test]
        fn one_process_plays_several_turns() {
            let claude = script(PLAYER);
            let dir = claude.parent().unwrap().to_path_buf();
            let mut engine = ClaudeCodeEngine::new(options(claude, Permission::ApplyDirectly));
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("a cube"), sender.clone()).unwrap();
            let first = turn(&receiver);
            assert!(matches!(first[0], Event::ToolCall { .. }), "{first:?}");
            assert_eq!(engine.session_id().as_deref(), Some("2ae90353-7b53-4672-b448-ce02b8b77858"));
            engine.send(UserMessage::text("again"), sender).unwrap();
            assert_eq!(turn(&receiver), first);
            let stdin = std::fs::read_to_string(dir.join("stdin.txt")).unwrap();
            assert_eq!(stdin.lines().count(), 2, "one process read both: {stdin}");
            let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
            assert!(args.lines().any(|arg| arg == "--input-format") && !args.lines().any(|arg| arg == "--resume"));
        }

        #[test]
        fn a_logged_out_cli_ends_the_turn_with_its_error() {
            let claude = script("read -r line; cat \"$DIR/logged_out.jsonl\"; exit 1\n");
            let mut engine = ClaudeCodeEngine::new(options(claude, Permission::AskFirst));
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("hi"), sender).unwrap();
            let events = turn(&receiver);
            assert_eq!(events.len(), 1, "{events:?}");
            assert!(matches!(&events[0], Event::Error { message } if message.contains("/login")));
        }

        #[test]
        fn a_crash_says_what_the_cli_wrote_and_the_next_turn_resumes() {
            let claude = script(&format!(
                "if [ -e \"$DIR/crashed\" ]; then\n{PLAYER}else\n touch \"$DIR/crashed\"; read -r line; echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s-1\"}}'; echo 'boom: no network' >&2; exit 3\nfi\n"
            ));
            let dir = claude.parent().unwrap().to_path_buf();
            let mut engine = ClaudeCodeEngine::new(options(claude, Permission::ApplyDirectly));
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("hi"), sender.clone()).unwrap();
            let events = turn(&receiver);
            let [Event::Error { message }] = events.as_slice() else { panic!("{events:?}") };
            assert!(message.contains("boom: no network") && message.contains('3'), "{message}");
            engine.send(UserMessage::text("again"), sender).unwrap();
            assert!(matches!(turn(&receiver).last(), Some(Event::Done { .. })));
            let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
            assert!(args.contains("--resume\ns-1"), "{args}");
        }

        #[test]
        fn stop_interrupts_the_turn() {
            // Answers the interrupt as the CLI does.
            let claude = script(
                r#"while IFS= read -r line; do
  case "$line" in
    *'"interrupt"'*) echo '{"type":"control_response","response":{"subtype":"success"}}'
      echo '{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s-2","errors":["aborted"]}' ;;
    *) echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"1 2 3"}}}' ;;
  esac
done
"#,
            );
            let mut engine = ClaudeCodeEngine::new(options(claude, Permission::ApplyDirectly));
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("count"), sender.clone()).unwrap();
            assert_eq!(receiver.recv_timeout(Duration::from_secs(10)).unwrap(), Event::Text { delta: "1 2 3".into() });
            assert!(engine.send(UserMessage::text("meanwhile"), sender).is_err());
            let start = Instant::now();
            engine.stop();
            let end = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(end, Event::Done { usage: None, cost_usd: None, session_id: Some("s-2".into()) });
            assert!(start.elapsed() < Duration::from_secs(1));
            // The next turn, sent at once, outlives the stop's deadline.
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("again"), sender).unwrap();
            assert_eq!(receiver.recv_timeout(Duration::from_secs(10)).unwrap(), Event::Text { delta: "1 2 3".into() });
            thread::sleep(STOP_GRACE + Duration::from_millis(400));
            assert!(engine.process.as_ref().is_some_and(|process| !process.shared.ended.load(Ordering::SeqCst)));
            assert!(receiver.try_recv().is_err());
        }

        #[test]
        fn stop_kills_a_cli_that_does_not_answer_within_a_second() {
            let claude = script("read -r line; echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s-3\"}'; sleep 5\n");
            let mut engine = ClaudeCodeEngine::new(options(claude, Permission::ApplyDirectly));
            let (sender, receiver) = mpsc::channel();
            engine.send(UserMessage::text("hang"), sender).unwrap();
            // Once the CLI named its session, it hangs.
            let named = Instant::now();
            while engine.session_id().is_none() && named.elapsed() < Duration::from_secs(10) {
                thread::sleep(Duration::from_millis(20));
            }
            let start = Instant::now();
            engine.stop();
            let end = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(end, Event::Done { usage: None, cost_usd: None, session_id: Some("s-3".into()) });
            assert!(start.elapsed() < Duration::from_millis(1500), "{:?}", start.elapsed());
        }
    }
}
