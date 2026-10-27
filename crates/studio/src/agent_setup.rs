//! The agent's engine and what it may do: its settings, kept with the
//! others in `settings.txt`, the check of which engines this computer has
//! (made off the UI thread), the choice of one, and its making.
//!
//! The API key is never kept here nor shown: it stays in the system's
//! secure store ([`c2f_agent::keys`]), read when an engine is made.

use std::fmt;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use c2f_agent::conversation::{Engine, Event, UserMessage};
use c2f_agent::engine::api::{self, ApiEngine, ApiOptions};
use c2f_agent::engine::claude_code::{self, ClaudeCodeEngine, ClaudeCodeOptions, ClaudeCodeStatus, Permission};
use c2f_agent::guard::{Guarded, Policy, Scope};
use c2f_agent::{Registry, Tools, keys};
use eframe::egui::{self, RichText};

use crate::agent_panel::{Entry, Transcript};
use crate::i18n::tr;
use crate::theme;

/// The tool rounds an answer may take, and the default.
pub const MAX_TOOL_ROUNDS: RangeInclusive<u32> = 1..=100;
const TOOL_ROUNDS: u32 = 25;

/// Which engine runs the agent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineChoice {
    /// Claude Code when it is there and logged in, else the API when a key
    /// is stored.
    #[default]
    Automatic,
    ClaudeCode,
    Api,
}

impl EngineChoice {
    pub const ALL: [EngineChoice; 3] = [EngineChoice::Automatic, EngineChoice::ClaudeCode, EngineChoice::Api];

    fn id(self) -> &'static str {
        match self {
            EngineChoice::Automatic => "auto",
            EngineChoice::ClaudeCode => "claude-code",
            EngineChoice::Api => "api",
        }
    }

    fn name(self) -> &'static str {
        match self {
            EngineChoice::Automatic => tr("Automatic", "Automatique"),
            EngineChoice::ClaudeCode => "Claude Code",
            EngineChoice::Api => tr("Anthropic API", "API Anthropic"),
        }
    }
}

/// The agent's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSettings {
    pub engine: EngineChoice,
    /// Where `claude` is, when not where it is looked for.
    pub claude_path: Option<PathBuf>,
    /// The API's model, an id from [`api::models`].
    pub api_model: String,
    /// Claude Code's model; its own default when empty.
    pub claude_model: String,
    /// Each change the agent proposes waits for the user's yes.
    pub ask_first: bool,
    pub allow_new_files: bool,
    pub allow_export: bool,
    pub allow_save_part: bool,
    pub allow_electrify: bool,
    /// The agent may search the web and read pages.
    pub allow_web: bool,
    /// The rounds of tools an answer may take.
    pub max_tool_rounds: u32,
}

impl Default for AgentSettings {
    fn default() -> AgentSettings {
        AgentSettings {
            engine: EngineChoice::Automatic,
            claude_path: None,
            api_model: api::DEFAULT_MODEL.into(),
            claude_model: String::new(),
            ask_first: false,
            allow_new_files: true,
            allow_export: true,
            allow_save_part: true,
            allow_electrify: true,
            allow_web: true,
            max_tool_rounds: TOOL_ROUNDS,
        }
    }
}

impl AgentSettings {
    /// Takes in the line `key = value` of `settings.txt`; whether the key is
    /// the agent's. A bad value leaves the setting as it was.
    pub fn read(&mut self, key: &str, value: &str) -> bool {
        let flag = |field: &mut bool| {
            if let Ok(on) = value.parse() {
                *field = on;
            }
        };
        match key {
            "agent-engine" => {
                if let Some(engine) = EngineChoice::ALL.into_iter().find(|e| e.id() == value) {
                    self.engine = engine;
                }
            }
            "agent-claude" => {
                if Path::new(value).is_absolute() {
                    self.claude_path = Some(PathBuf::from(value));
                }
            }
            "agent-api-model" => {
                if api::models().iter().any(|(id, _)| *id == value) {
                    self.api_model = value.into();
                }
            }
            "agent-claude-model" => self.claude_model = value.into(),
            "agent-permission" => match value {
                "apply" => self.ask_first = false,
                "ask" => self.ask_first = true,
                _ => {}
            },
            "agent-allow-new-files" => flag(&mut self.allow_new_files),
            "agent-allow-export" => flag(&mut self.allow_export),
            "agent-allow-save-part" => flag(&mut self.allow_save_part),
            "agent-allow-electrify" => flag(&mut self.allow_electrify),
            "agent-allow-web" => flag(&mut self.allow_web),
            "agent-max-tool-rounds" => {
                if let Ok(rounds) = value.parse::<u32>() {
                    self.max_tool_rounds = rounds.clamp(*MAX_TOOL_ROUNDS.start(), *MAX_TOOL_ROUNDS.end());
                }
            }
            _ => return false,
        }
        true
    }

    /// Adds the agent's lines to `settings.txt`'s `text`.
    pub fn write(&self, text: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(text, "agent-engine = {}", self.engine.id());
        if let Some(path) = self.claude_path.as_deref().and_then(Path::to_str).filter(|p| !p.contains(['\n', '\r'])) {
            let _ = writeln!(text, "agent-claude = {path}");
        }
        let _ = writeln!(text, "agent-api-model = {}", self.api_model);
        if !self.claude_model.contains(['\n', '\r']) {
            let _ = writeln!(text, "agent-claude-model = {}", self.claude_model.trim());
        }
        let _ = writeln!(text, "agent-permission = {}", if self.ask_first { "ask" } else { "apply" });
        let _ = writeln!(text, "agent-allow-new-files = {}", self.allow_new_files);
        let _ = writeln!(text, "agent-allow-export = {}", self.allow_export);
        let _ = writeln!(text, "agent-allow-save-part = {}", self.allow_save_part);
        let _ = writeln!(text, "agent-allow-electrify = {}", self.allow_electrify);
        let _ = writeln!(text, "agent-allow-web = {}", self.allow_web);
        let _ = writeln!(text, "agent-max-tool-rounds = {}", self.max_tool_rounds);
    }

    /// What the agent's tools may do: the tools allowed, always in
    /// `folder` (an untitled tab's is a scratch folder of its own, never
    /// "anywhere"), the `libraries` (the user's and the default ones)
    /// readable besides.
    pub fn policy(&self, folder: &Path, libraries: &[PathBuf]) -> Policy {
        let read_only = libraries.iter().cloned().chain(c2f_eval::default_library_paths()).collect();
        Policy {
            new_files: self.allow_new_files,
            export: self.allow_export,
            save_part: self.allow_save_part,
            electrify: self.allow_electrify,
            ask_first: self.ask_first,
            scope: Some(Scope { folder: folder.to_path_buf(), read_only }),
        }
    }

    /// The settings the `mcp --studio` server needs to guard the tools,
    /// and the user's `libraries`, as one line of JSON: the engine gives
    /// it to the server it starts, which thus never reads a `settings.txt`
    /// older than the engine.
    pub fn policy_json(&self, libraries: &[PathBuf]) -> String {
        serde_json::json!({
            "ask_first": self.ask_first,
            "new_files": self.allow_new_files,
            "export": self.allow_export,
            "save_part": self.allow_save_part,
            "electrify": self.allow_electrify,
            "libraries": libraries,
        })
        .to_string()
    }

    /// The settings and the libraries [`AgentSettings::policy_json`]
    /// wrote; `None` when `text` is not such JSON. A flag left out is as by
    /// default.
    pub fn from_policy_json(text: &str) -> Option<(AgentSettings, Vec<PathBuf>)> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let mut agent = AgentSettings::default();
        let flag = |key: &str, field: &mut bool| {
            if let Some(on) = value.get(key).and_then(serde_json::Value::as_bool) {
                *field = on;
            }
        };
        flag("ask_first", &mut agent.ask_first);
        flag("new_files", &mut agent.allow_new_files);
        flag("export", &mut agent.allow_export);
        flag("save_part", &mut agent.allow_save_part);
        flag("electrify", &mut agent.allow_electrify);
        let libraries = value.get("libraries")?.as_array()?;
        let libraries = libraries.iter().filter_map(serde_json::Value::as_str).map(PathBuf::from).filter(|p| p.is_absolute());
        Some((agent, libraries.collect()))
    }

    fn claude_model(&self) -> Option<String> {
        Some(self.claude_model.trim().to_string()).filter(|m| !m.is_empty())
    }
}

// ---- What this computer has ----

/// The engines this computer has, as last checked.
#[derive(Clone, PartialEq)]
pub struct Probe {
    /// Where Claude Code is, and its state.
    pub claude: Option<PathBuf>,
    pub status: ClaudeCodeStatus,
    /// The API key; never shown nor logged.
    pub key: Option<String>,
    /// The key comes from `ANTHROPIC_API_KEY` rather than the store.
    pub key_from_env: bool,
}

impl fmt::Debug for Probe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Probe")
            .field("claude", &self.claude)
            .field("status", &self.status)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What to do with the key before the check.
pub enum KeyChange {
    Keep,
    Save(String),
    Remove,
}

#[derive(Default)]
struct ProbeState {
    /// Counts the checks done, to tell a new outcome.
    generation: u64,
    probe: Option<Probe>,
    checking: bool,
    /// What the last change of the key gave.
    message: Option<(String, bool)>,
}

/// Checks the engines on a thread of its own.
#[derive(Clone, Default)]
pub struct Prober {
    state: Arc<Mutex<ProbeState>>,
}

impl Prober {
    /// Checks again, after changing the key as `change` says; the check
    /// done, `ctx` repaints.
    pub fn check(&self, ctx: &egui::Context, claude_path: Option<PathBuf>, change: KeyChange) {
        self.lock().checking = true;
        let (state, ctx) = (self.state.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("c2f-agent-probe".into()).spawn(move || {
            let message = match change {
                KeyChange::Keep => None,
                KeyChange::Save(key) => Some(match keys::save_api_key(&key) {
                    Ok(()) => (tr("The API key is saved.", "La clé d'API est enregistrée.").to_string(), false),
                    Err(err) => (err, true),
                }),
                KeyChange::Remove => Some(match keys::delete_api_key() {
                    Ok(()) => (tr("The API key is removed.", "La clé d'API est supprimée.").to_string(), false),
                    Err(err) => (err, true),
                }),
            };
            let claude = claude_code::find_claude(claude_path.as_deref());
            let status = claude.as_deref().map_or(ClaudeCodeStatus::Missing, claude_code::status);
            let key = keys::load_api_key();
            let env = std::env::var("ANTHROPIC_API_KEY").ok().map(|k| k.trim().to_string());
            let key_from_env = key.is_some() && key == env;
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.generation += 1;
            state.probe = Some(Probe { claude, status, key, key_from_env });
            state.checking = false;
            if message.is_some() {
                state.message = message;
            }
            drop(state);
            ctx.request_repaint();
        });
        if spawned.is_err() {
            self.lock().checking = false;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ProbeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The last check's outcome, `None` before the first, and its number.
    pub fn current(&self) -> (u64, Option<Probe>) {
        let state = self.lock();
        (state.generation, state.probe.clone())
    }

    pub fn checking(&self) -> bool {
        self.lock().checking
    }

    /// What the last change of the key gave, and whether it failed.
    fn message(&self) -> Option<(String, bool)> {
        self.lock().message.clone()
    }

    /// Sets the outcome, for tests.
    #[cfg(test)]
    pub fn set(&self, probe: Probe) {
        let mut state = self.lock();
        state.generation += 1;
        state.probe = Some(probe);
    }
}

// ---- The choice ----

/// The engine picked.
#[derive(Clone, PartialEq, Eq)]
pub enum Pick {
    ClaudeCode(PathBuf),
    /// With its key.
    Api(String),
}

impl fmt::Debug for Pick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pick::ClaudeCode(path) => f.debug_tuple("ClaudeCode").field(path).finish(),
            Pick::Api(_) => f.debug_tuple("Api").field(&"<redacted>").finish(),
        }
    }
}

/// The engine `agent` picks among those of `probe`; the error says, in a
/// line, why none can run.
pub fn choose(agent: &AgentSettings, probe: &Probe) -> Result<Pick, String> {
    let claude = || -> Result<Pick, String> {
        let path = probe.claude.clone();
        match (&probe.status, path) {
            (ClaudeCodeStatus::Ready { .. } | ClaudeCodeStatus::Found { .. }, Some(path)) => Ok(Pick::ClaudeCode(path)),
            (ClaudeCodeStatus::NotLoggedIn { .. }, _) => Err(tr(
                "Claude Code is not logged in: run `claude` then /login in a terminal.",
                "Claude Code n'est pas connecté : lancez `claude` puis /login dans un terminal.",
            )
            .into()),
            (ClaudeCodeStatus::TooOld { .. }, _) => Err(probe.status.message()),
            _ => Err(tr("Claude Code is not found: install it, or give its path in the settings.", "Claude Code est introuvable : installez-le, ou indiquez son chemin dans les réglages.").into()),
        }
    };
    let api = || -> Result<Pick, String> {
        probe.key.clone().map(Pick::Api).ok_or_else(|| {
            tr("No Anthropic API key: add one in the settings.", "Aucune clé d'API Anthropic : ajoutez-en une dans les réglages.").into()
        })
    };
    match agent.engine {
        EngineChoice::ClaudeCode => claude(),
        EngineChoice::Api => api(),
        EngineChoice::Automatic => match (&probe.status, &probe.claude) {
            (ClaudeCodeStatus::Ready { .. }, Some(path)) => Ok(Pick::ClaudeCode(path.clone())),
            _ => api().map_err(|_| match probe.status {
                ClaudeCodeStatus::NotLoggedIn { .. } => tr(
                    "No engine: log Claude Code in (`claude` then /login in a terminal), or add an API key in the settings.",
                    "Aucun moteur : connectez Claude Code (`claude` puis /login dans un terminal), ou ajoutez une clé d'API dans les réglages.",
                )
                .into(),
                _ => tr(
                    "No engine: install Claude Code, or add an Anthropic API key in the settings.",
                    "Aucun moteur : installez Claude Code, ou ajoutez une clé d'API Anthropic dans les réglages.",
                )
                .into(),
            }),
        },
    }
}

// ---- The making ----

/// What an engine is made from.
pub struct EngineRequest<'a> {
    pub agent: &'a AgentSettings,
    /// The user's library folders.
    pub libraries: &'a [PathBuf],
    /// The engines this computer has; `None` while they are checked.
    pub probe: Option<&'a Probe>,
    /// The active file's folder.
    pub folder: Option<&'a Path>,
    /// The conversation to carry on, empty for a new one.
    pub transcript: &'a Transcript,
}

/// An engine made, and what the panel says of it.
pub struct Built {
    pub engine: Box<dyn Engine>,
    /// Its name and model, for the panel's header.
    pub label: String,
    /// Why no engine can answer; the engine then refuses each message.
    pub problem: Option<String>,
    /// What the user should know of the conversation carried on.
    pub notice: Option<String>,
}

impl Built {
    /// `engine`, called `label`.
    pub fn ready(engine: Box<dyn Engine>, label: impl Into<String>) -> Built {
        Built { engine, label: label.into(), problem: None, notice: None }
    }

    /// No engine, for the reason `why`.
    pub fn unavailable(why: impl Into<String>) -> Built {
        let why = why.into();
        Built { engine: Box::new(Unavailable(why.clone())), label: tr("no engine", "aucun moteur").into(), problem: Some(why), notice: None }
    }
}

/// Stands for an engine that cannot run: each message gets the reason.
struct Unavailable(String);

impl Engine for Unavailable {
    fn send(&mut self, _: UserMessage, _: mpsc::Sender<Event>) -> Result<(), String> {
        Err(self.0.clone())
    }

    fn stop(&mut self) {}

    fn name(&self) -> &str {
        "none"
    }
}

/// The folder the agent works in for a tab without a file, `name` setting
/// it apart from the others': an empty folder of its own in the settings
/// folder, made when an engine is, never the home folder.
pub fn scratch_folder(name: &str) -> PathBuf {
    let settings = crate::recent::settings_dir().unwrap_or_else(|| std::env::temp_dir().join("code2form"));
    settings.join("agent-scratch").join(name)
}

/// The files open in the studio, each with whether it has unsaved edits,
/// kept up to date by the app: `electrify` leaves alone an open file
/// ([`Guarded::open_files`]), the other tools that write a file one with
/// unsaved edits ([`KeepUnsaved`]).
pub type TabFiles = Arc<Mutex<Vec<(PathBuf, bool)>>>;

/// Whether `a` and `b` are the same file, which may not exist yet.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || c2f_lang::file_scope::resolve(a) == c2f_lang::file_scope::resolve(b)
}

/// The files the tool `name` called with `args` writes, its paths made
/// absolute already; `electrify`'s are the guard's.
fn written_files(name: &str, args: &serde_json::Value) -> Vec<PathBuf> {
    let path = |key: &str| args.get(key).and_then(serde_json::Value::as_str).map(PathBuf::from);
    match name {
        "export" => path("output").into_iter().collect(),
        "save_part" => {
            let Some(part) = args.get("name").and_then(serde_json::Value::as_str) else { return Vec::new() };
            let part = part.strip_suffix(".c2f").unwrap_or(part);
            c2f_eval::user_parts_folder().map(|folder| folder.join(format!("{part}.c2f"))).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// Tools that refuse to write onto a file open in the studio with unsaved
/// edits, which the write would lose or the tab would overwrite.
struct KeepUnsaved {
    tools: Registry,
    tabs: TabFiles,
}

impl Tools for KeepUnsaved {
    fn list(&self) -> Vec<serde_json::Value> {
        self.tools.list()
    }

    fn call(&self, name: &str, args: &serde_json::Value) -> Option<Result<Vec<serde_json::Value>, String>> {
        let tabs = self.tabs.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let unsaved = |file: &Path| tabs.iter().any(|(open, dirty)| *dirty && same_file(open, file));
        if let Some(file) = written_files(name, args).into_iter().find(|file| unsaved(file)) {
            return Some(Err(format!(
                "`{name}` would write {}, which is open in the studio with unsaved edits: ask the user to save or close \
                 it first (or change it with edit_code)",
                file.display()
            )));
        }
        self.tools.call(name, args)
    }
}

/// The folder the agent of `request` works in: the active file's, else a
/// scratch one (made here), never "anywhere" nor the home folder.
fn work_folder(request: &EngineRequest) -> PathBuf {
    let folder = request.folder.map_or_else(|| scratch_folder("default"), Path::to_path_buf);
    if folder.starts_with(scratch_folder("")) {
        let _ = std::fs::create_dir_all(&folder);
    }
    folder
}

/// How to run Claude Code for `request`: in its folder, its server started
/// by `mcp_command` with the policy as the settings say now (not as
/// `settings.txt` last kept them).
fn claude_code_options(request: &EngineRequest, claude: PathBuf, mcp_command: (PathBuf, Vec<String>)) -> ClaudeCodeOptions {
    let agent = request.agent;
    let (program, mut args) = mcp_command;
    args.extend(["--policy".to_string(), agent.policy_json(request.libraries)]);
    ClaudeCodeOptions {
        claude,
        cwd: work_folder(request),
        mcp_command: (program, args),
        model: agent.claude_model(),
        resume: request.transcript.session_id.clone(),
        permission: if agent.ask_first { Permission::AskFirst } else { Permission::ApplyDirectly },
        write_files: agent.allow_new_files,
        web: agent.allow_web,
        max_turns: Some(agent.max_tool_rounds),
    }
}

/// The API engine's tools: the built-in ones and the studio's `live` ones,
/// behind `policy`, leaving alone the files open in the studio (`tabs`).
fn api_tools(live: Option<Box<dyn Tools>>, tabs: Option<&TabFiles>, policy: Policy) -> Result<Registry, String> {
    let mut tools = Registry::builtin();
    if let Some(live) = live {
        tools = tools.with(live)?;
    }
    let Some(tabs) = tabs else { return Ok(Guarded::registry(tools, policy)) };
    let kept = KeepUnsaved { tools, tabs: tabs.clone() };
    let tools = Registry::new().with(Box::new(kept)).expect("a single provider");
    let open = tabs.clone();
    let is_open = move |file: &Path| open.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|(tab, _)| same_file(tab, file));
    Ok(Guarded::new(tools, policy).open_files(Arc::new(is_open)).into_registry())
}

/// The engine `request` asks for, the studio's `live` tools among its
/// tools; `mcp_command` starts the server Claude Code reaches them by. The
/// tools of the API engine leave alone the files open in the studio
/// (`tabs`).
pub fn engine_for(
    request: &EngineRequest,
    live: Option<Box<dyn Tools>>,
    mcp_command: (PathBuf, Vec<String>),
    tabs: Option<&TabFiles>,
) -> Built {
    let Some(probe) = request.probe else {
        return Built::unavailable(tr("Checking the engines…", "Vérification des moteurs…"));
    };
    let agent = request.agent;
    let folder = work_folder(request);
    match choose(agent, probe) {
        Err(why) => Built::unavailable(why),
        Ok(Pick::ClaudeCode(claude)) => {
            let options = claude_code_options(request, claude, mcp_command);
            let notice = (options.resume.is_none() && !request.transcript.entries.is_empty()).then(|| {
                tr(
                    "The earlier messages are shown, but Claude Code starts a new context.",
                    "Les messages précédents restent affichés, mais Claude Code repart d'un contexte neuf.",
                )
                .to_string()
            });
            let model = agent.claude_model().unwrap_or_else(|| tr("default model", "modèle par défaut").into());
            Built { notice, ..Built::ready(Box::new(ClaudeCodeEngine::new(options)), format!("Claude Code · {model}")) }
        }
        Ok(Pick::Api(key)) => {
            let registry = match api_tools(live, tabs, agent.policy(&folder, request.libraries)) {
                Ok(registry) => registry,
                Err(err) => return Built::unavailable(err),
            };
            let mut options = ApiOptions::new(key, Arc::new(registry));
            options.model = agent.api_model.clone();
            options.max_tool_rounds = agent.max_tool_rounds;
            options.web = agent.allow_web;
            let history = request.transcript.entries.iter().filter_map(|entry| match entry {
                Entry::User { message, .. } => Some((true, message.text.clone())),
                Entry::Assistant { text } => Some((false, text.clone())),
                _ => None,
            });
            let engine = ApiEngine::new(options).with_history(history.collect::<Vec<_>>());
            let model = api::models().iter().find(|(id, _)| *id == agent.api_model).map_or(agent.api_model.as_str(), |m| m.1);
            Built::ready(Box::new(engine), format!("{} · {model}", EngineChoice::Api.name()))
        }
    }
}

// ---- The settings page ----

/// What the engines' state reads as.
fn claude_state(probe: Option<&Probe>, checking: bool) -> String {
    let Some(probe) = probe else {
        return tr("Claude Code: checking…", "Claude Code : vérification…").into();
    };
    let state = match &probe.status {
        ClaudeCodeStatus::Ready { version } => tr!("Claude Code {version} — ready", "Claude Code {version} — prêt"),
        ClaudeCodeStatus::Found { version } => tr!("Claude Code {version} — found", "Claude Code {version} — trouvé"),
        ClaudeCodeStatus::NotLoggedIn { version } => tr!(
            "Claude Code {version} — not logged in (run `claude` then /login in a terminal)",
            "Claude Code {version} — non connecté (lancez `claude` puis /login dans un terminal)"
        ),
        ClaudeCodeStatus::Missing => tr("Claude Code — not found", "Claude Code — introuvable").into(),
        ClaudeCodeStatus::TooOld { .. } => probe.status.message(),
    };
    if checking { tr!("{state} (checking…)", "{state} (vérification…)") } else { state }
}

fn key_state(probe: Option<&Probe>) -> &'static str {
    match probe {
        None => tr("API key: checking…", "Clé d'API : vérification…"),
        Some(Probe { key: None, .. }) => tr("API key: none", "Clé d'API : aucune"),
        Some(Probe { key_from_env: true, .. }) => tr("API key: from ANTHROPIC_API_KEY", "Clé d'API : depuis ANTHROPIC_API_KEY"),
        Some(_) if cfg!(target_os = "macos") => tr("API key: stored in the Keychain", "Clé d'API : enregistrée dans le trousseau"),
        Some(_) if cfg!(windows) => tr("API key: stored in the Credential Manager", "Clé d'API : enregistrée dans le Gestionnaire d'identification"),
        Some(_) => tr("API key: stored in a private file", "Clé d'API : enregistrée dans un fichier privé"),
    }
}

/// The Agent page of the settings: the engine and its state, the API key,
/// the model, the permissions and the limits. `key_input` is the key typed,
/// until saved.
pub fn page(ui: &mut egui::Ui, ctx: &egui::Context, agent: &mut AgentSettings, prober: &Prober, key_input: &mut String) {
    let palette = theme::palette();
    let (_, probe) = prober.current();
    let checking = prober.checking();
    if probe.is_none() && !checking {
        prober.check(ctx, agent.claude_path.clone(), KeyChange::Keep);
    }
    let dim = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).color(palette.text_dim));
    };
    heading(ui, tr("Engine", "Moteur"), tr(
        "Automatic runs Claude Code when it is installed and logged in, else the API when a key is stored.",
        "Automatique lance Claude Code s'il est installé et connecté, sinon l'API si une clé est enregistrée.",
    ));
    ui.horizontal(|ui| {
        for engine in EngineChoice::ALL {
            ui.selectable_value(&mut agent.engine, engine, engine.name());
        }
    });
    ui.add_space(4.0);
    dim(ui, &claude_state(probe.as_ref(), checking));
    dim(ui, key_state(probe.as_ref()));
    if let Some(probe) = &probe {
        let picked = match choose(agent, probe) {
            Ok(Pick::ClaudeCode(_)) => tr("The agent runs on Claude Code.", "L'agent tourne sur Claude Code.").to_string(),
            Ok(Pick::Api(_)) => tr("The agent runs on the Anthropic API.", "L'agent tourne sur l'API Anthropic.").to_string(),
            Err(why) => why,
        };
        ui.label(RichText::new(picked).color(palette.text));
    }
    if ui.add_enabled(!checking, egui::Button::new(tr("Check again", "Vérifier à nouveau"))).clicked() {
        prober.check(ctx, agent.claude_path.clone(), KeyChange::Keep);
    }

    ui.add_space(14.0);
    heading(ui, "Claude Code", tr(
        "Your own Claude Code, with your subscription. It is looked for in the usual places unless you give its path.",
        "Votre propre Claude Code, avec votre abonnement. Il est cherché aux endroits habituels, sauf si vous indiquez son chemin.",
    ));
    ui.horizontal(|ui| {
        ui.label(tr("Path", "Chemin"));
        let mut path = agent.claude_path.as_deref().map(|p| p.display().to_string()).unwrap_or_default();
        let field = egui::TextEdit::singleline(&mut path).hint_text(tr("found automatically", "trouvé automatiquement")).desired_width(260.0);
        if ui.add(field).changed() {
            let path = path.trim();
            agent.claude_path = (!path.is_empty() && Path::new(path).is_absolute()).then(|| PathBuf::from(path));
        }
        if ui.button(tr("Choose…", "Choisir…")).clicked() {
            if let Some(file) = rfd::FileDialog::new().set_title(tr("Claude Code's executable", "L'exécutable de Claude Code")).pick_file() {
                agent.claude_path = Some(file);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("Model", "Modèle"));
        let field = egui::TextEdit::singleline(&mut agent.claude_model)
            .hint_text(tr("Claude Code's default", "par défaut de Claude Code"))
            .desired_width(200.0);
        ui.add(field);
    });
    dim(ui, tr(
        "A model id or alias (opus, sonnet). Claude Code runs restricted, without your own settings: empty is its default model.",
        "Un identifiant ou un alias de modèle (opus, sonnet). Claude Code tourne restreint, sans vos propres réglages : vide, c'est son modèle par défaut.",
    ));

    ui.add_space(14.0);
    heading(ui, tr("Anthropic API", "API Anthropic"), tr(
        "Billed to your API account. The key is kept in the system's secure store, never shown.",
        "Facturée sur votre compte d'API. La clé est gardée dans le coffre sécurisé du système, jamais affichée.",
    ));
    ui.horizontal(|ui| {
        ui.label(tr("Key", "Clé"));
        let field = egui::TextEdit::singleline(key_input).password(true).hint_text("sk-ant-…").desired_width(220.0);
        ui.add(field);
        if ui.add_enabled(!key_input.trim().is_empty() && !checking, egui::Button::new(tr("Save", "Enregistrer"))).clicked() {
            prober.check(ctx, agent.claude_path.clone(), KeyChange::Save(std::mem::take(key_input)));
        }
        let stored = probe.as_ref().is_some_and(|p| p.key.is_some() && !p.key_from_env);
        if ui.add_enabled(stored && !checking, egui::Button::new(tr("Remove", "Supprimer"))).clicked() {
            prober.check(ctx, agent.claude_path.clone(), KeyChange::Remove);
        }
    });
    if let Some((message, failed)) = prober.message() {
        ui.label(RichText::new(message).color(if failed { palette.error } else { palette.text_dim }));
    }
    ui.horizontal(|ui| {
        ui.label(tr("Model", "Modèle"));
        let label = api::models().iter().find(|(id, _)| *id == agent.api_model).map_or(agent.api_model.as_str(), |m| m.1);
        egui::ComboBox::from_id_salt("agent-api-model").selected_text(label).show_ui(ui, |ui| {
            for (id, name) in api::models() {
                ui.selectable_value(&mut agent.api_model, id.to_string(), *name);
            }
        });
    });

    ui.add_space(14.0);
    heading(ui, tr("Permissions", "Autorisations"), tr(
        "The agent works only in the active file's folder; the library folders it may read.",
        "L'agent travaille seulement dans le dossier du fichier actif ; il peut lire les dossiers de bibliothèques.",
    ));
    ui.radio_value(&mut agent.ask_first, false, tr("Apply changes directly", "Appliquer les modifications directement"));
    ui.radio_value(&mut agent.ask_first, true, tr("Ask before each change", "Demander avant chaque modification"));
    ui.add_space(4.0);
    ui.checkbox(&mut agent.allow_new_files, tr("Allow writing new files", "Autoriser l'écriture de nouveaux fichiers"));
    ui.checkbox(&mut agent.allow_export, tr("Allow exporting", "Autoriser l'export"));
    ui.checkbox(&mut agent.allow_save_part, tr("Allow saving parts to the library", "Autoriser l'enregistrement de pièces dans la bibliothèque"));
    ui.checkbox(&mut agent.allow_electrify, tr("Allow electrifying a design", "Autoriser l'électrification d'un design"));
    ui.checkbox(&mut agent.allow_web, tr("Search the web", "Rechercher sur le web"));
    ui.label(RichText::new(tr(
        "Pages the agent finds are read by Claude, and treated as information, not instructions.",
        "Les pages trouvées sont lues par Claude, comme des informations et non comme des instructions.",
    ))
    .small()
    .color(theme::palette().text_dim));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tr("Tool rounds per answer, at most", "Tours d'outils par réponse, au plus"));
        ui.add(egui::Slider::new(&mut agent.max_tool_rounds, MAX_TOOL_ROUNDS).trailing_fill(true));
    });
}

fn heading(ui: &mut egui::Ui, title: &str, about: &str) {
    ui.label(RichText::new(title).strong().color(theme::palette().text));
    ui.label(RichText::new(about).color(theme::palette().text_dim));
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(status: ClaudeCodeStatus, key: bool) -> Probe {
        let claude = (status != ClaudeCodeStatus::Missing).then(|| PathBuf::from("/opt/claude"));
        Probe { claude, status, key: key.then(|| "sk-ant-test".to_string()), key_from_env: false }
    }

    fn ready() -> ClaudeCodeStatus {
        ClaudeCodeStatus::Ready { version: "2.1.310".into() }
    }

    fn logged_out() -> ClaudeCodeStatus {
        ClaudeCodeStatus::NotLoggedIn { version: "2.1.310".into() }
    }

    #[test]
    fn automatic_prefers_claude_code_then_the_api() {
        let auto = AgentSettings::default();
        let claude = Pick::ClaudeCode("/opt/claude".into());
        assert_eq!(choose(&auto, &probe(ready(), true)), Ok(claude.clone()));
        assert_eq!(choose(&auto, &probe(logged_out(), true)), Ok(Pick::Api("sk-ant-test".into())));
        assert_eq!(choose(&auto, &probe(ClaudeCodeStatus::Missing, true)), Ok(Pick::Api("sk-ant-test".into())));
        assert!(choose(&auto, &probe(logged_out(), false)).unwrap_err().contains("/login"));
        assert!(choose(&auto, &probe(ClaudeCodeStatus::Missing, false)).unwrap_err().contains("API key"));

        let api = AgentSettings { engine: EngineChoice::Api, ..AgentSettings::default() };
        assert_eq!(choose(&api, &probe(ready(), true)), Ok(Pick::Api("sk-ant-test".into())));
        assert!(choose(&api, &probe(ready(), false)).is_err());
        let code = AgentSettings { engine: EngineChoice::ClaudeCode, ..AgentSettings::default() };
        assert_eq!(choose(&code, &probe(ready(), true)), Ok(claude));
        assert!(choose(&code, &probe(logged_out(), true)).unwrap_err().contains("not logged in"));
        assert!(choose(&code, &probe(ClaudeCodeStatus::Missing, true)).unwrap_err().contains("not found"));
    }

    fn built(agent: &AgentSettings, probe: Option<&Probe>, transcript: &Transcript) -> Built {
        let request = EngineRequest { agent, libraries: &[], probe, folder: Some(Path::new("/work")), transcript };
        engine_for(&request, None, ("/bin/code2form".into(), vec!["mcp".into(), "--studio".into()]), None)
    }

    #[test]
    fn without_a_file_the_agent_works_in_a_scratch_folder_not_anywhere() {
        let agent = AgentSettings::default();
        let transcript = Transcript::default();
        let probe = probe(ready(), true);
        let scratch = scratch_folder("test-untitled");
        let request = EngineRequest { agent: &agent, libraries: &[], probe: Some(&probe), folder: Some(&scratch), transcript: &transcript };
        let options = claude_code_options(&request, "/opt/claude".into(), ("/bin/code2form".into(), vec!["mcp".into(), "--studio".into()]));
        assert_eq!(options.cwd, scratch);
        assert!(scratch.is_dir(), "made for the engine");
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let none = EngineRequest { folder: None, ..request };
        let cwd = claude_code_options(&none, "/opt/claude".into(), ("/bin/code2form".into(), vec![])).cwd;
        assert!(cwd.ends_with("agent-scratch/default") && Some(&cwd) != home.as_ref(), "{}", cwd.display());
        // The server Claude Code starts gets the policy as it is now.
        let (_, args) = options.mcp_command;
        assert_eq!(args[..3], ["mcp", "--studio", "--policy"]);
        let (back, libraries) = AgentSettings::from_policy_json(&args[3]).unwrap();
        assert_eq!((back.ask_first, back.allow_export, libraries.len()), (agent.ask_first, agent.allow_export, 0));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_policy_json_reads_back() {
        let agent = AgentSettings { ask_first: true, allow_export: false, allow_electrify: false, ..AgentSettings::default() };
        // Absolute on every system: `/libs` has no drive on Windows.
        let libs = std::env::temp_dir().join("libs");
        let libraries = vec![libs.clone(), PathBuf::from("relative")];
        let (back, read) = AgentSettings::from_policy_json(&agent.policy_json(&libraries)).unwrap();
        assert!(back.ask_first && !back.allow_export && !back.allow_electrify && back.allow_save_part && back.allow_new_files);
        assert_eq!(read, vec![libs], "a relative folder is left out");
        assert!(AgentSettings::from_policy_json("nope").is_none());
    }

    #[test]
    fn a_tool_does_not_write_onto_a_file_open_with_unsaved_edits() {
        let dir = std::env::temp_dir().join(format!("c2f-unsaved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = dir.join("box.c2f");
        std::fs::write(&open, "cube(1);").unwrap();
        let tabs: TabFiles = Arc::new(Mutex::new(vec![(open.clone(), true)]));
        let tools = api_tools(None, Some(&tabs), Policy::default()).unwrap();
        let export = serde_json::json!({ "code": "cube(1);", "output": open });
        let err = tools.call("export", &export).unwrap().unwrap_err();
        assert!(err.contains("unsaved edits"), "{err}");
        // Another file, or the tab saved: the tool runs.
        let elsewhere = serde_json::json!({ "code": "cube(1);", "output": dir.join("box.stl") });
        assert!(tools.call("export", &elsewhere).unwrap().is_ok());
        tabs.lock().unwrap()[0].1 = false;
        assert!(!matches!(tools.call("export", &export), Some(Err(e)) if e.contains("unsaved")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_api_engine_s_tools_do_not_electrify_an_open_tab() {
        let dir = std::env::temp_dir().join(format!("c2f-open-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let design = dir.join("box.c2f");
        std::fs::write(&design, "cube(1);").unwrap();
        // Open, saved: the guard knows it from the studio's tabs.
        let tabs: TabFiles = Arc::new(Mutex::new(vec![(design.clone(), false)]));
        let policy = AgentSettings::default().policy(&dir, &[]);
        let tools = api_tools(None, Some(&tabs), policy).unwrap();
        let err = tools.call("electrify", &serde_json::json!({ "path": "box.c2f" })).unwrap().unwrap_err();
        assert!(err.contains("open in the studio") && err.contains("close its tab"), "{err}");
        assert_eq!(std::fs::read_to_string(&design).unwrap(), "cube(1);", "left alone");
        // Its circuit open instead.
        *tabs.lock().unwrap() = vec![(dir.join("box.c2fc"), true)];
        let err = tools.call("electrify", &serde_json::json!({ "path": "box.c2f" })).unwrap().unwrap_err();
        assert!(err.contains("box.c2fc is open in the studio"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_api_key_is_not_in_a_pick_s_debug() {
        let shown = format!("{:?}", Pick::Api("sk-ant-secret".into()));
        assert!(!shown.contains("sk-ant-secret") && shown.contains("redacted"), "{shown}");
    }

    #[test]
    fn the_factory_makes_the_engine_picked() {
        let empty = Transcript::default();
        let mut agent = AgentSettings::default();
        let checking = built(&agent, None, &empty);
        assert!(checking.problem.is_some());
        let (sender, _) = mpsc::channel();
        let mut engine = checking.engine;
        assert!(engine.send(UserMessage::text("hi"), sender).is_err(), "no engine answers");

        let code = built(&agent, Some(&probe(ready(), true)), &empty);
        assert_eq!((code.engine.name(), code.label.as_str()), ("Claude Code", "Claude Code · default model"));
        assert!(code.problem.is_none());

        agent.claude_model = "haiku".into();
        assert_eq!(built(&agent, Some(&probe(ready(), true)), &empty).label, "Claude Code · haiku");
        agent.api_model = "claude-haiku-5-5".into();
        let api = built(&agent, Some(&probe(logged_out(), true)), &empty);
        assert_eq!((api.engine.name(), api.label.as_str()), ("Anthropic API", "Anthropic API · Claude Haiku 5.5"));

        let none = built(&agent, Some(&probe(ClaudeCodeStatus::Missing, false)), &empty);
        assert!(none.problem.unwrap().contains("No engine"));
    }

    #[test]
    fn a_conversation_carried_on_without_a_session_says_so() {
        let mut transcript = Transcript::default();
        transcript.push_user(UserMessage::text("a box"), None);
        let agent = AgentSettings::default();
        assert!(built(&agent, Some(&probe(ready(), false)), &transcript).notice.is_some());
        transcript.session_id = Some("s1".into());
        assert!(built(&agent, Some(&probe(ready(), false)), &transcript).notice.is_none(), "resumed");
        assert!(built(&agent, Some(&probe(ClaudeCodeStatus::Missing, true)), &transcript).notice.is_none(), "the API carries the texts on");
    }

    #[test]
    fn the_settings_read_back() {
        let agent = AgentSettings {
            engine: EngineChoice::ClaudeCode,
            claude_path: Some(std::env::temp_dir().join("claude")),
            api_model: "claude-sonnet-5-5".into(),
            claude_model: "opus".into(),
            ask_first: true,
            allow_new_files: false,
            allow_export: false,
            allow_save_part: true,
            allow_electrify: false,
            allow_web: false,
            max_tool_rounds: 9,
        };
        let mut text = String::new();
        agent.write(&mut text);
        let mut back = AgentSettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').unwrap();
            assert!(back.read(key.trim(), value.trim()), "{line}");
        }
        assert_eq!(back, agent);
        let mut bad = AgentSettings::default();
        for (key, value) in [("agent-engine", "gpt"), ("agent-api-model", "nope"), ("agent-claude", "relative"), ("agent-max-tool-rounds", "0")] {
            bad.read(key, value);
        }
        assert_eq!(bad, AgentSettings { max_tool_rounds: 1, ..AgentSettings::default() });
        assert!(!bad.read("render-delay", "100"));
    }

    #[test]
    fn the_policy_follows_the_settings() {
        let agent = AgentSettings { allow_export: false, ..AgentSettings::default() };
        let policy = agent.policy(Path::new("/work"), &[PathBuf::from("/libs")]);
        assert!(!policy.export && policy.new_files && policy.save_part && policy.electrify);
        let scope = policy.scope.unwrap();
        assert_eq!(scope.folder, Path::new("/work"));
        assert_eq!(scope.read_only[0], Path::new("/libs"));
    }

    /// A real turn on the user's Claude Code, with Haiku, through the
    /// studio's own `mcp --studio` server: run by hand, with `--ignored`,
    /// once `cargo build -p code2form` made the server.
    #[test]
    #[ignore = "spends a little of the user's Claude Code subscription"]
    fn a_real_claude_code_turn_renders_through_the_studio_s_server() {
        let claude = claude_code::find_claude(None).expect("Claude Code installed");
        let status = claude_code::status(&claude);
        let probe = Probe { claude: Some(claude), status, key: None, key_from_env: false };
        let agent = AgentSettings { engine: EngineChoice::ClaudeCode, claude_model: "haiku".into(), max_tool_rounds: 4, ..AgentSettings::default() };
        let folder = std::env::temp_dir();
        let transcript = Transcript::default();
        let request = EngineRequest { agent: &agent, libraries: &[], probe: Some(&probe), folder: Some(&folder), transcript: &transcript };
        let server = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/code2form");
        let mut built = engine_for(&request, None, (server, vec!["mcp".into(), "--studio".into()]), None);
        assert!(built.problem.is_none(), "{:?}", built.problem);
        let (sender, events) = mpsc::channel();
        let ask = "Call the code2form `render` tool with code `cube(2);`, then reply with the volume alone.";
        built.engine.send(UserMessage::text(ask), sender).unwrap();
        let mut seen = Vec::new();
        loop {
            let event = events.recv_timeout(std::time::Duration::from_secs(180)).expect("the turn ends");
            let last = matches!(event, Event::Done { .. } | Event::Error { .. });
            seen.push(event);
            if last {
                break;
            }
        }
        assert!(seen.iter().any(|e| matches!(e, Event::ToolCall { name, .. } if name.ends_with("render"))), "{seen:?}");
        assert!(matches!(seen.last(), Some(Event::Done { session_id: Some(_), .. })), "{seen:?}");
    }

    #[test]
    fn the_key_is_never_shown() {
        let shown = format!("{:?}", probe(ready(), true));
        assert!(!shown.contains("sk-ant-test") && shown.contains("redacted"), "{shown}");
    }
}
