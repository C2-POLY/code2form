//! The Anthropic API engine: the agent with nothing to install but an API
//! key. Its loop runs in-process, on a thread of its own: it streams a
//! reply from the Messages API, runs the tools the reply calls through a
//! [`Registry`], sends their results back and goes on until the reply
//! ends its turn.
//!
//! The model, prices and request shapes follow Anthropic's Claude API
//! reference as of October 2026.

use std::fmt;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::Registry;
use crate::conversation::{Attachment, Engine, Event, Usage, UserMessage};
use crate::mcp::base64;

/// The model the engine runs when the settings name none: the most capable
/// of the current Opus line.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

/// Where the Messages API is.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The models a user can choose, as `(id, label)`, the default first.
pub fn models() -> &'static [(&'static str, &'static str)] {
    &[
        ("claude-opus-5-5", "Claude Opus 5.5"),
        ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
        ("claude-haiku-5-5", "Claude Haiku 5.5"),
        ("claude-fable-5-1", "Claude Fable 5.1"),
    ]
}

/// What the agent is told it runs in, after the [`crate::instructions`].
const STUDIO: &str = "\
You run inside the Code2Form studio, beside the document the user has open. \
Prefer the studio's live tools, which act on that open document and show their \
changes in its view, to the built-in tools that take code or a file: change the \
document the user sees rather than a copy of it. The user may send a picture of \
the view, or the part or the lines of code they have selected, with a message: \
that is what they are pointing at.";

/// The longest reply, in tokens. Streaming keeps a long one from timing out.
const MAX_TOKENS: u64 = 64_000;

/// How often the loop looks whether it was stopped while it waits.
const POLL: Duration = Duration::from_millis(100);

/// How long a stream may stay silent before the engine gives up on it: the
/// API sends pings to a stream that has nothing else to say.
const QUIET: Duration = Duration::from_secs(120);

/// How many times a request refused by a busy API is sent again.
const RETRIES: u32 = 2;

/// The longest wait before sending again; an API that asks for more is
/// reported instead.
const LONGEST_WAIT: u64 = 30;

/// How the engine talks to the API.
#[derive(Clone)]
pub struct ApiOptions {
    /// The Anthropic API key; [`fmt::Debug`] never shows it.
    pub key: String,
    /// A model id, as [`models`] gives them.
    pub model: String,
    /// The tools the agent calls: the built-in ones and the studio's.
    pub registry: Arc<Registry>,
    /// The system prompt.
    pub system: String,
    /// How many rounds of tool calls a turn may take before it stops.
    pub max_tool_rounds: u32,
    /// The API's address, without the `/v1/messages`.
    pub base_url: String,
    /// The agent may search the web and read pages, with Anthropic's
    /// server-side web search and web fetch tools: they run on Anthropic's
    /// side, and their results come back in the reply.
    pub web: bool,
}

impl ApiOptions {
    /// The defaults for `key` and the tools of `registry`: the
    /// [`DEFAULT_MODEL`], the [`default_system`] prompt, 25 rounds of tools
    /// and the Anthropic API itself, without the web.
    pub fn new(key: impl Into<String>, registry: Arc<Registry>) -> ApiOptions {
        ApiOptions {
            key: key.into(),
            model: DEFAULT_MODEL.into(),
            registry,
            system: default_system(),
            max_tool_rounds: 25,
            base_url: DEFAULT_BASE_URL.into(),
            web: false,
        }
    }
}

impl fmt::Debug for ApiOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiOptions")
            .field("key", &"<redacted>")
            .field("model", &self.model)
            .field("tools", &self.registry.names().len())
            .field("system", &format_args!("{} bytes", self.system.len()))
            .field("max_tool_rounds", &self.max_tool_rounds)
            .field("base_url", &self.base_url)
            .field("web", &self.web)
            .finish()
    }
}

/// The system prompt: the agent's [`crate::instructions`], and a note that
/// it runs in the studio.
pub fn default_system() -> String {
    format!("{}\n\n{STUDIO}", crate::instructions())
}

/// The agent over the Anthropic API. It keeps the conversation from one
/// [`Engine::send`] to the next.
pub struct ApiEngine {
    options: Arc<ApiOptions>,
    agent: ureq::Agent,
    history: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    turn: Option<JoinHandle<()>>,
}

impl ApiEngine {
    pub fn new(options: ApiOptions) -> ApiEngine {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(QUIET))
            .build()
            .into();
        ApiEngine {
            options: Arc::new(options),
            agent,
            history: Arc::default(),
            stop: Arc::default(),
            turn: None,
        }
    }

    /// This engine with an earlier conversation as its context: `turns`,
    /// the user's (`true`) and the assistant's texts in order. Consecutive
    /// turns of one side join, and the assistant's before the first of
    /// the user's are left out, so that the roles alternate from the user.
    pub fn with_history(self, turns: impl IntoIterator<Item = (bool, String)>) -> ApiEngine {
        {
            let mut history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
            for (user, text) in turns {
                let role = if user { "user" } else { "assistant" };
                if text.trim().is_empty() || (history.is_empty() && !user) {
                    continue;
                }
                match history.last_mut() {
                    Some(last) if last["role"] == role => {
                        if let Some(blocks) = last["content"].as_array_mut() {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                    }
                    _ => history.push(json!({ "role": role, "content": [{ "type": "text", "text": text }] })),
                }
            }
        }
        self
    }

    /// Whether a turn is under way.
    fn busy(&self) -> bool {
        self.turn.as_ref().is_some_and(|turn| !turn.is_finished())
    }
}

impl Engine for ApiEngine {
    fn send(&mut self, message: UserMessage, events: Sender<Event>) -> Result<(), String> {
        // A stopped turn ends within a poll, unless a tool is still running:
        // the next one waits for it on its own thread, never on the
        // caller's (the studio's UI thread).
        if self.busy() && !self.stop.load(Ordering::SeqCst) {
            return Err("the agent is still answering: stop it or wait for its reply".into());
        }
        if self.options.key.trim().is_empty() {
            return Err("no Anthropic API key: add one in the settings".into());
        }
        let stopped = self.turn.take().filter(|turn| !turn.is_finished());
        // A flag of its own, so that stopping this turn never stops the next.
        self.stop = Arc::new(AtomicBool::new(false));
        let turn = Turn {
            options: self.options.clone(),
            agent: self.agent.clone(),
            history: self.history.clone(),
            stop: self.stop.clone(),
            events,
        };
        let handle = thread::Builder::new()
            .name("c2f-agent-api".into())
            .spawn(move || {
                // The history is the stopped turn's until it ends.
                if let Some(stopped) = stopped {
                    let _ = stopped.join();
                }
                turn.run(message)
            })
            .map_err(|err| format!("cannot start the agent: {err}"))?;
        self.turn = Some(handle);
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn name(&self) -> &str {
        "Anthropic API"
    }
}

/// One turn, on its own thread.
struct Turn {
    options: Arc<ApiOptions>,
    agent: ureq::Agent,
    history: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
}

/// What a turn took, summed over its requests.
struct Totals {
    usage: Usage,
    /// None once a request ran on a model whose price is unknown.
    cost: Option<f64>,
}

/// A whole reply.
#[derive(Debug, Default)]
struct Reply {
    content: Vec<Value>,
    stop_reason: String,
    /// The tool calls whose input did not read as a JSON object.
    invalid: Vec<String>,
}

/// Why a request failed, in plain words, and how long to wait before
/// sending it again when it may be.
struct Failure {
    message: String,
    retry: Option<Duration>,
}

impl Failure {
    fn new(message: impl Into<String>) -> Failure {
        Failure { message: message.into(), retry: None }
    }
}

/// What the thread that holds the connection passes on.
enum Wire {
    Line(String),
    /// The API answered with an error status.
    Status { status: u16, retry_after: Option<u64>, body: String },
    Network(String),
    End,
}

impl Turn {
    fn run(self, message: UserMessage) {
        let mut history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
        let content = user_content(&message);
        // A turn stopped or cut short leaves the history on the user's side:
        // this message joins it, so that the roles still alternate.
        match history.last_mut() {
            Some(last) if last["role"] == "user" => {
                if let Some(blocks) = last["content"].as_array_mut() {
                    blocks.extend(content);
                }
            }
            _ => history.push(json!({ "role": "user", "content": content })),
        }
        let mut totals = Totals { usage: Usage::default(), cost: Some(0.0) };
        let end = self.converse(&mut history, &mut totals);
        drop_dangling_server_calls(&mut history);
        drop(history);
        let _ = self.events.send(match end {
            Ok(()) => Event::Done { usage: Some(totals.usage), cost_usd: totals.cost, session_id: None },
            Err(message) => Event::Error { message },
        });
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn emit(&self, event: Event) {
        // A chat panel gone stops nothing: the turn ends on its own.
        let _ = self.events.send(event);
    }

    /// Requests replies and runs their tools until a reply ends the turn.
    fn converse(&self, history: &mut Vec<Value>, totals: &mut Totals) -> Result<(), String> {
        let web = if self.options.web { web_tools(&self.options.model) } else { Vec::new() };
        let tools = api_tools(&self.options.registry, web);
        let mut rounds = 0;
        loop {
            let Some(mut reply) = self.request(history, &tools, totals)? else { return Ok(()) };
            keep_after_fallback(&mut reply.content);
            if reply.stop_reason != "tool_use" {
                // A tool call cut short has no result to go with it.
                while reply.content.last().is_some_and(|block| block["type"] == "tool_use") {
                    reply.content.pop();
                }
            }
            if !reply.content.is_empty() {
                match history.last_mut() {
                    // A paused turn's reply goes on in the same message.
                    Some(last) if last["role"] == "assistant" => {
                        if let Some(blocks) = last["content"].as_array_mut() {
                            blocks.extend(reply.content.iter().cloned());
                        }
                    }
                    _ => history.push(json!({ "role": "assistant", "content": reply.content })),
                }
            }
            match reply.stop_reason.as_str() {
                "tool_use" => {}
                // The server's tools (the web) took their rounds: sending the
                // reply back as it is lets the server go on with it.
                "pause_turn" => {
                    rounds += 1;
                    if rounds >= self.options.max_tool_rounds {
                        return Err(format!(
                            "the agent stopped after {rounds} rounds of tool calls: send a message to let it go on"
                        ));
                    }
                    continue;
                }
                "max_tokens" => return Err("the reply reached its length limit and was cut short".into()),
                "refusal" => return Err("Claude declined to answer this request".into()),
                "model_context_window_exceeded" => {
                    return Err("the conversation is too long for the model: start a new one".into());
                }
                _ => return Ok(()),
            }
            let results = self.run_tools(&reply);
            history.push(json!({ "role": "user", "content": results }));
            if self.stopped() {
                return Ok(());
            }
            rounds += 1;
            if rounds >= self.options.max_tool_rounds {
                return Err(format!(
                    "the agent stopped after {rounds} rounds of tool calls: send a message to let it go on"
                ));
            }
        }
    }

    /// Runs the tools a reply calls, in order, and gives their results back
    /// in one message's blocks; once stopped, the calls left get an error.
    fn run_tools(&self, reply: &Reply) -> Vec<Value> {
        let calls = reply.content.iter().filter(|block| block["type"] == "tool_use");
        calls
            .map(|block| {
                let id = block["id"].as_str().unwrap_or_default().to_string();
                if self.stopped() {
                    return tool_result(&id, &Err("the user stopped the agent before this tool ran".into()));
                }
                let name = block["name"].as_str().unwrap_or_default();
                let input = block["input"].clone();
                self.emit(Event::ToolCall { id: id.clone(), name: name.into(), input: input.clone() });
                let result = if reply.invalid.contains(&id) {
                    Err("the tool's input is not a JSON object: send it again, whole".into())
                } else {
                    self.options.registry.call(name, &input).unwrap_or_else(|| Err(format!("there is no tool named `{name}`")))
                };
                let (content, is_error) = match &result {
                    Ok(content) => (content.clone(), false),
                    Err(message) => (vec![json!({ "type": "text", "text": message })], true),
                };
                self.emit(Event::ToolResult { id: id.clone(), content, is_error });
                tool_result(&id, &result)
            })
            .collect()
    }

    /// The next reply, streamed; none when the turn was stopped. A busy API
    /// gets the request again, after the wait it asks for.
    fn request(&self, history: &[Value], tools: &[Value], totals: &mut Totals) -> Result<Option<Reply>, String> {
        let body = self.body(history, tools).to_string();
        let mut attempt = 0;
        loop {
            let (sender, wire) = mpsc::channel();
            let (agent, url, headers, body) = (self.agent.clone(), self.url(), self.headers(), body.clone());
            thread::Builder::new()
                .name("c2f-agent-http".into())
                .spawn(move || fetch(&agent, &url, &headers, body, &sender))
                .map_err(|err| format!("cannot start a request: {err}"))?;
            match self.read(&wire, totals) {
                Ok(reply) => return Ok(reply),
                Err(Failure { retry: Some(wait), .. }) if attempt < RETRIES => {
                    attempt += 1;
                    let until = Instant::now() + wait;
                    while Instant::now() < until {
                        if self.stopped() {
                            return Ok(None);
                        }
                        thread::sleep(POLL);
                    }
                }
                Err(failure) => return Err(failure.message),
            }
        }
    }

    fn url(&self) -> String {
        format!("{}/v1/messages", self.options.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("x-api-key", self.options.key.trim().to_string()),
            ("anthropic-version", "2023-06-01".to_string()),
            ("content-type", "application/json".to_string()),
        ];
        if falls_back(&self.options.model) {
            headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".to_string()));
        }
        headers
    }

    /// The request: the tools and the system prompt each end on a cache
    /// breakpoint, and the conversation caches up to its last block.
    fn body(&self, history: &[Value], tools: &[Value]) -> Value {
        let model = &self.options.model;
        let mut body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "cache_control": { "type": "ephemeral" },
            "output_config": { "effort": "high" },
            "messages": history,
        });
        if !self.options.system.trim().is_empty() {
            let system = if self.options.web {
                format!("{}\n\n{}", self.options.system, super::claude_code::WEB_NOTE)
            } else {
                self.options.system.clone()
            };
            body["system"] = json!([{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }]);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if falls_back(model) {
            // A request the model declines runs again on the model
            // Anthropic recommends for that kind of request.
            body["fallbacks"] = json!("default");
        }
        body
    }

    /// Reads the stream on `wire` into a reply, passing its text on as it
    /// comes; none when the turn was stopped.
    fn read(&self, wire: &Receiver<Wire>, totals: &mut Totals) -> Result<Option<Reply>, Failure> {
        let mut stream = Stream::default();
        let mut heard = Instant::now();
        loop {
            if self.stopped() {
                // Dropping `wire` ends the connection's thread at its next line.
                stream.count(&self.options.model, totals);
                return Ok(None);
            }
            match wire.recv_timeout(POLL) {
                Ok(Wire::Line(line)) => {
                    heard = Instant::now();
                    if let Some(data) = line.strip_prefix("data:") {
                        stream.data(data.trim_start(), &self.events)?;
                    }
                }
                Ok(Wire::Status { status, retry_after, body }) => {
                    return Err(status_failure(status, retry_after, &body, &self.options.model));
                }
                Ok(Wire::Network(err)) => {
                    stream.count(&self.options.model, totals);
                    return Err(Failure::new(format!("the connection to the Anthropic API failed: {err}")));
                }
                Ok(Wire::End) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) if heard.elapsed() > QUIET => {
                    stream.count(&self.options.model, totals);
                    return Err(Failure::new("the Anthropic API went silent: try again"));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        stream.count(&self.options.model, totals);
        if !stream.complete {
            return Err(Failure::new("the connection closed before the reply was complete: try again"));
        }
        let blocks = stream.blocks.into_iter().filter(|block| !block.is_null()).collect();
        Ok(Some(Reply { content: blocks, stop_reason: stream.stop_reason, invalid: stream.invalid }))
    }
}

/// Sends the request and passes the response on, line by line, until it
/// ends or nobody listens any more.
fn fetch(agent: &ureq::Agent, url: &str, headers: &[(&'static str, String)], body: String, wire: &Sender<Wire>) {
    let mut request = agent.post(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = match request.send(body) {
        Ok(response) => response,
        Err(err) => {
            let _ = wire.send(Wire::Network(err.to_string()));
            return;
        }
    };
    let status = response.status().as_u16();
    if status != 200 {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok());
        let body = response.into_body().read_to_string().unwrap_or_default();
        let _ = wire.send(Wire::Status { status, retry_after, body });
        return;
    }
    for line in BufReader::new(response.into_body().into_reader()).lines() {
        let message = match line {
            Ok(line) => Wire::Line(line),
            Err(err) => Wire::Network(err.to_string()),
        };
        let failed = matches!(message, Wire::Network(_));
        if wire.send(message).is_err() || failed {
            return;
        }
    }
    let _ = wire.send(Wire::End);
}

/// A reply as its events build it.
#[derive(Default)]
struct Stream {
    /// The content blocks by index; tool calls get their input at their end.
    blocks: Vec<Value>,
    /// The tool calls' input so far, as JSON text, by index.
    inputs: Vec<String>,
    invalid: Vec<String>,
    stop_reason: String,
    model: String,
    /// Input, cache writes, cache reads and output, in tokens.
    tokens: [u64; 4],
    complete: bool,
}

impl Stream {
    /// Takes the data of one event in.
    fn data(&mut self, data: &str, events: &Sender<Event>) -> Result<(), Failure> {
        let Ok(event) = serde_json::from_str::<Value>(data) else { return Ok(()) };
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.model = event["message"]["model"].as_str().unwrap_or_default().to_string();
                self.usage(&event["message"]["usage"]);
            }
            "content_block_start" => {
                if self.blocks.len() <= index {
                    self.blocks.resize(index + 1, Value::Null);
                    self.inputs.resize(index + 1, String::new());
                }
                let block = event["content_block"].clone();
                if let Some(text) = block["text"].as_str().filter(|text| !text.is_empty()) {
                    let _ = events.send(Event::Text { delta: text.into() });
                }
                if let Some(result) = server_result(&block) {
                    let _ = events.send(result);
                }
                self.blocks[index] = block;
            }
            "content_block_delta" => {
                let Some(block) = self.blocks.get_mut(index) else { return Ok(()) };
                let delta = &event["delta"];
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or_default();
                        append(block, "text", text);
                        let _ = events.send(Event::Text { delta: text.into() });
                    }
                    "thinking_delta" => append(block, "thinking", delta["thinking"].as_str().unwrap_or_default()),
                    "signature_delta" => block["signature"] = delta["signature"].clone(),
                    "input_json_delta" => self.inputs[index].push_str(delta["partial_json"].as_str().unwrap_or_default()),
                    "citations_delta" => {
                        if !block["citations"].is_array() {
                            block["citations"] = json!([]);
                        }
                        if let Some(citations) = block["citations"].as_array_mut() {
                            citations.push(delta["citation"].clone());
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let Some(block) = self.blocks.get_mut(index) else { return Ok(()) };
                if block["type"] == "tool_use" {
                    let text = self.inputs[index].trim();
                    let input = if text.is_empty() { Ok(json!({})) } else { serde_json::from_str::<Value>(text) };
                    match input {
                        Ok(input) if input.is_object() => block["input"] = input,
                        _ => {
                            block["input"] = json!({});
                            self.invalid.push(block["id"].as_str().unwrap_or_default().to_string());
                        }
                    }
                } else if block["type"] == "server_tool_use" {
                    // A call the server runs: shown, never run here.
                    let text = self.inputs[index].trim();
                    if let Some(input) = serde_json::from_str::<Value>(text).ok().filter(Value::is_object) {
                        block["input"] = input;
                    }
                    let _ = events.send(Event::ToolCall {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        input: block["input"].clone(),
                    });
                }
            }
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = reason.to_string();
                }
                self.usage(&event["usage"]);
            }
            "message_stop" => self.complete = true,
            "error" => {
                let error = &event["error"];
                let message = error["message"].as_str().unwrap_or("no reason given");
                return Err(Failure::new(match error["type"].as_str() {
                    Some("overloaded_error") => "the Anthropic API is overloaded: try again in a moment".to_string(),
                    _ => format!("the Anthropic API stopped the reply: {message}"),
                }));
            }
            _ => {}
        }
        Ok(())
    }

    /// Takes the counts `usage` gives; each is the total so far.
    fn usage(&mut self, usage: &Value) {
        let fields = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens"];
        for (count, field) in self.tokens.iter_mut().zip(fields) {
            if let Some(tokens) = usage[field].as_u64() {
                *count = tokens;
            }
        }
    }

    /// Adds what this reply took to the turn's totals.
    fn count(&self, requested: &str, totals: &mut Totals) {
        let [input, written, read, output] = self.tokens;
        totals.usage.input_tokens += input + written + read;
        totals.usage.output_tokens += output;
        let model = if self.model.is_empty() { requested } else { &self.model };
        totals.cost = match (totals.cost, price(model, input + written + read)) {
            (Some(cost), Some((per_input, per_output, per_read))) => Some(
                cost + (input as f64 * per_input
                    + written as f64 * per_input * 1.25
                    + read as f64 * per_read
                    + output as f64 * per_output)
                    / 1e6,
            ),
            _ => None,
        };
    }
}

fn append(block: &mut Value, field: &str, text: &str) {
    match &mut block[field] {
        Value::String(so_far) => so_far.push_str(text),
        other => *other = json!(text),
    }
}

/// Dollars per million tokens of input, output and cache reads, for a
/// prompt of `prompt` tokens; a cache write costs 1.25 times the input
/// (five-minute entries). None for a model this table does not know.
fn price(model: &str, prompt: u64) -> Option<(f64, f64, f64)> {
    Some(match model {
        "claude-fable-5-1" => (10.0, 50.0, 0.25),
        "claude-opus-5-5" => (4.0, 20.0, 0.20),
        "claude-opus-5" | "claude-opus-4-8" => (5.0, 25.0, 0.50),
        "claude-sonnet-5-5" | "claude-sonnet-5" => (2.0, 10.0, 0.20),
        "claude-haiku-5-5" if prompt > 100_000 => (0.50, 2.50, 0.05),
        "claude-haiku-5-5" => (0.10, 0.50, 0.01),
        _ => return None,
    })
}

/// Whether a request declined by `model` may run again on another model,
/// server-side.
fn falls_back(model: &str) -> bool {
    matches!(model, "claude-opus-5-5" | "claude-sonnet-5-5" | "claude-fable-5-1")
}

/// After a model declined part way and another took over, the blocks
/// before the switch other than text are not sent back.
fn keep_after_fallback(content: &mut Vec<Value>) {
    let Some(switch) = content.iter().rposition(|block| block["type"] == "fallback") else { return };
    let mut index = 0;
    content.retain(|block| {
        index += 1;
        index > switch || block["type"] == "text"
    });
}

/// Anthropic's server-side web search and web fetch, in the versions
/// `model` takes: the ones that filter their results (dynamic filtering)
/// on the models that have it, the basic ones on the others.
fn web_tools(model: &str) -> Vec<Value> {
    let filtering = matches!(
        model,
        "claude-opus-5-5" | "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6"
            | "claude-sonnet-5-5" | "claude-sonnet-5" | "claude-sonnet-4-6"
    );
    let (search, fetch) = if filtering { ("web_search_20260209", "web_fetch_20260209") } else { ("web_search_20250305", "web_fetch_20250910") };
    vec![
        json!({ "type": search, "name": "web_search", "max_uses": 8 }),
        json!({ "type": fetch, "name": "web_fetch", "max_uses": 8 }),
    ]
}

/// A server tool's result block (web search, web fetch, or the code that
/// filters their results) as the [`Event::ToolResult`] the panel shows:
/// the pages found, or what went wrong. None for any other block.
fn server_result(block: &Value) -> Option<Event> {
    let kind = block["type"].as_str()?;
    if kind == "tool_result" || !kind.ends_with("_tool_result") {
        return None;
    }
    let id = block["tool_use_id"].as_str().unwrap_or_default().to_string();
    let content = &block["content"];
    // An error comes as an object of its own, not as a failed request.
    if content["type"].as_str().is_some_and(|kind| kind.ends_with("_error")) {
        let code = content["error_code"].as_str().unwrap_or("error");
        return Some(Event::ToolResult { id, content: vec![json!({ "type": "text", "text": format!("failed: {code}") })], is_error: true });
    }
    let lines: Vec<String> = match content {
        // Web search: the pages found.
        Value::Array(results) => results
            .iter()
            .filter_map(|result| {
                let url = result["url"].as_str()?;
                Some(match result["title"].as_str().filter(|title| !title.is_empty()) {
                    Some(title) => format!("{title} — {url}"),
                    None => url.to_string(),
                })
            })
            .collect(),
        // Web fetch: the page read; code: what it printed.
        Value::Object(_) => {
            let title = content["content"]["title"].as_str().filter(|title| !title.is_empty());
            match (content["url"].as_str(), content["stdout"].as_str()) {
                (Some(url), _) => vec![title.map_or_else(|| url.to_string(), |title| format!("{title} — {url}"))],
                (None, Some(out)) if !out.trim().is_empty() => vec![out.trim().to_string()],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    let content = if lines.is_empty() { Vec::new() } else { vec![json!({ "type": "text", "text": lines.join("\n") })] };
    Some(Event::ToolResult { id, content, is_error: false })
}

/// A turn stopped while the server's tools were at work leaves calls of
/// theirs without a result at the end of the history: they go, so that
/// the next request is valid.
fn drop_dangling_server_calls(history: &mut Vec<Value>) {
    let Some(last) = history.last_mut().filter(|last| last["role"] == "assistant") else { return };
    let Some(blocks) = last["content"].as_array_mut() else { return };
    let answered: Vec<String> =
        blocks.iter().filter_map(|block| block["tool_use_id"].as_str().map(str::to_string)).collect();
    blocks.retain(|block| {
        block["type"] != "server_tool_use" || block["id"].as_str().is_some_and(|id| answered.iter().any(|a| a == id))
    });
    if blocks.is_empty() {
        history.pop();
    }
}

/// The `web` tools, then the registry's tools as the API takes them; the
/// last one ends a cache breakpoint. Their input streams as it is written.
fn api_tools(registry: &Registry, web: Vec<Value>) -> Vec<Value> {
    let mut tools: Vec<Value> = web;
    tools.extend(registry
        .list()
        .iter()
        .map(|tool| {
            let mut api = json!({
                "name": tool["name"],
                "input_schema": tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                "eager_input_streaming": true,
            });
            if let Some(description) = tool["description"].as_str().or(tool["title"].as_str()) {
                api["description"] = json!(description);
            }
            api
        }));
    if let Some(last) = tools.last_mut() {
        last["cache_control"] = json!({ "type": "ephemeral" });
    }
    tools
}

/// The blocks of a user's message: pictures, then what they point at,
/// then their text.
fn user_content(message: &UserMessage) -> Vec<Value> {
    let mut content: Vec<Value> = message
        .attachments
        .iter()
        .map(|attachment| match attachment {
            Attachment::Image { png } => {
                json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": base64(png) } })
            }
            Attachment::Context { label, text } => json!({ "type": "text", "text": format!("{label}:\n{text}") }),
        })
        .collect();
    if !message.text.trim().is_empty() || content.is_empty() {
        content.push(json!({ "type": "text", "text": if message.text.trim().is_empty() { "…" } else { &message.text } }));
    }
    content
}

/// A tool's result as the API takes it: MCP text and images become text
/// and image blocks.
fn tool_result(id: &str, result: &Result<Vec<Value>, String>) -> Value {
    match result {
        Ok(items) => {
            let content: Vec<Value> = items.iter().filter_map(api_content).collect();
            if content.is_empty() {
                json!({ "type": "tool_result", "tool_use_id": id })
            } else {
                json!({ "type": "tool_result", "tool_use_id": id, "content": content })
            }
        }
        Err(message) => json!({
            "type": "tool_result",
            "tool_use_id": id,
            "content": [{ "type": "text", "text": message }],
            "is_error": true,
        }),
    }
}

/// An MCP content item as an API block; none for an empty text or an item
/// of another kind.
fn api_content(item: &Value) -> Option<Value> {
    match item["type"].as_str()? {
        "text" => {
            let text = item["text"].as_str().filter(|text| !text.is_empty())?;
            Some(json!({ "type": "text", "text": text }))
        }
        "image" => Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": item["mimeType"], "data": item["data"] },
        })),
        _ => None,
    }
}

/// What an error status means, in plain words.
fn status_failure(status: u16, retry_after: Option<u64>, body: &str, model: &str) -> Failure {
    let detail: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let said = detail["error"]["message"].as_str().unwrap_or(body).trim().to_string();
    let wait = retry_after.unwrap_or(2);
    let again = |what: &str| {
        let message = match retry_after {
            Some(seconds) => format!("{what}: try again in {seconds} s"),
            None => format!("{what}: try again in a moment"),
        };
        let retry = (wait <= LONGEST_WAIT).then(|| Duration::from_secs(wait));
        Failure { message, retry }
    };
    match status {
        401 => Failure::new("the Anthropic API key was refused: check it in the settings"),
        402 => Failure::new("the Anthropic account has a billing problem: see console.anthropic.com"),
        403 => Failure::new(format!("this API key may not use {model}: {said}")),
        404 => Failure::new(format!("the model {model} is not available to this API key")),
        413 => Failure::new("the conversation is too large to send: start a new one"),
        429 => again("this API key has reached its rate limit"),
        529 => again("the Anthropic API is overloaded"),
        500..=599 => again(&format!("the Anthropic API failed ({status})")),
        _ if said.to_lowercase().contains("credit") => Failure::new(format!("no credit left on the Anthropic account: {said}")),
        _ => Failure::new(format!("the Anthropic API refused the request ({status}): {said}")),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use crate::Tools;

    use super::*;

    /// What was asked, in order: a request's head and body, or a tool's
    /// name and arguments.
    type Log = Arc<Mutex<Vec<(String, Value)>>>;

    /// What the mock server answers a request with.
    enum Canned {
        /// A whole event stream.
        Sse(String),
        /// An error status, with its headers and body.
        Status(u16, &'static str, &'static str),
        /// The start of a stream, then nothing for a while.
        Hang(String),
    }

    /// A Messages API on 127.0.0.1 that answers each request with the next
    /// canned response, and keeps the requests' bodies.
    struct Mock {
        url: String,
        requests: Log,
    }

    fn mock(responses: Vec<Canned>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests: Log = Arc::default();
        let kept = requests.clone();
        thread::spawn(move || {
            for canned in responses {
                let (mut socket, _) = listener.accept().unwrap();
                let (head, body) = read_request(&mut socket);
                kept.lock().unwrap().push((head, serde_json::from_slice(&body).unwrap()));
                let chunk = |text: &str| format!("{:x}\r\n{text}\r\n", text.len());
                let stream_head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
                match canned {
                    Canned::Sse(events) => {
                        let _ = write!(socket, "{stream_head}{}0\r\n\r\n", chunk(&events));
                    }
                    Canned::Status(status, headers, body) => {
                        let _ = write!(
                            socket,
                            "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{headers}\r\n{body}",
                            body.len()
                        );
                    }
                    Canned::Hang(events) => {
                        let _ = write!(socket, "{stream_head}{}", chunk(&events));
                        let _ = socket.flush();
                        thread::sleep(Duration::from_secs(3));
                    }
                }
            }
        });
        Mock { url, requests }
    }

    /// The request's head and body.
    fn read_request(socket: &mut std::net::TcpStream) -> (String, Vec<u8>) {
        let mut bytes = Vec::new();
        let mut byte = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
        }
        let head = String::from_utf8(bytes).unwrap().to_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map(|length| length.trim().parse().unwrap())
            .expect("a content-length");
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        (head, body)
    }

    /// The events as the API streams them.
    fn sse(events: &[Value]) -> String {
        events.iter().fold(String::new(), |mut out, event| {
            let _ = write!(out, "event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap());
            out
        })
    }

    fn start(input: u64) -> Value {
        json!({ "type": "message_start", "message": { "id": "msg", "type": "message", "role": "assistant", "model": "claude-opus-5-5", "content": [], "usage": { "input_tokens": input, "output_tokens": 1 } } })
    }

    fn end(reason: &str, output: u64) -> [Value; 2] {
        [
            json!({ "type": "message_delta", "delta": { "stop_reason": reason }, "usage": { "output_tokens": output } }),
            json!({ "type": "message_stop" }),
        ]
    }

    fn text(index: u64, parts: &[&str]) -> Vec<Value> {
        let mut events = vec![json!({ "type": "content_block_start", "index": index, "content_block": { "type": "text", "text": "" } })];
        for part in parts {
            events.push(json!({ "type": "content_block_delta", "index": index, "delta": { "type": "text_delta", "text": part } }));
        }
        events.push(json!({ "type": "content_block_stop", "index": index }));
        events
    }

    fn tool_use(index: u64, id: &str, name: &str, json_parts: &[&str]) -> Vec<Value> {
        let mut events = vec![json!({ "type": "content_block_start", "index": index, "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} } })];
        for part in json_parts {
            events.push(json!({ "type": "content_block_delta", "index": index, "delta": { "type": "input_json_delta", "partial_json": part } }));
        }
        events.push(json!({ "type": "content_block_stop", "index": index }));
        events
    }

    fn reply(input: u64, blocks: Vec<Vec<Value>>, reason: &str, output: u64) -> Canned {
        let mut events = vec![start(input), json!({ "type": "ping" })];
        events.extend(blocks.into_iter().flatten());
        events.extend(end(reason, output));
        Canned::Sse(sse(&events))
    }

    /// A host's tools: `measure` says a size, `picture` gives an image.
    struct Fake(Log);

    impl Tools for Fake {
        fn list(&self) -> Vec<Value> {
            vec![
                json!({ "name": "measure", "description": "Measures the model.", "inputSchema": { "type": "object", "properties": { "axis": { "type": "string" } } } }),
                json!({ "name": "picture", "title": "A picture of the view", "inputSchema": { "type": "object" } }),
            ]
        }

        fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
            self.0.lock().unwrap().push((name.into(), args.clone()));
            match name {
                "measure" => Some(Ok(vec![json!({ "type": "text", "text": "42 mm" })])),
                "slow" => {
                    thread::sleep(Duration::from_millis(1500));
                    Some(Ok(vec![json!({ "type": "text", "text": "done" })]))
                }
                "picture" => Some(Ok(vec![json!({ "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" })])),
                _ => None,
            }
        }
    }

    fn engine(url: &str) -> (ApiEngine, Log) {
        let calls: Log = Arc::default();
        let registry = Registry::new().with(Box::new(Fake(calls.clone()))).unwrap();
        let mut options = ApiOptions::new("sk-ant-test-key", Arc::new(registry));
        options.base_url = url.into();
        (ApiEngine::new(options), calls)
    }

    /// The events of a turn, up to its end.
    fn turn(engine: &mut ApiEngine, message: UserMessage) -> Vec<Event> {
        let (sender, receiver) = mpsc::channel();
        engine.send(message, sender).unwrap();
        let mut events = Vec::new();
        loop {
            let event = receiver.recv_timeout(Duration::from_secs(10)).expect("the turn ends");
            let last = matches!(event, Event::Done { .. } | Event::Error { .. });
            events.push(event);
            if last {
                return events;
            }
        }
    }

    #[test]
    fn text_streams_and_the_history_carries_on() {
        let server = mock(vec![
            reply(10, vec![text(0, &["Hel", "lo"])], "end_turn", 5),
            reply(20, vec![text(0, &["Again"])], "end_turn", 3),
        ]);
        let (mut engine, _) = engine(&server.url);
        let events = turn(&mut engine, UserMessage::text("hi"));
        assert_eq!(events[0], Event::Text { delta: "Hel".into() });
        assert_eq!(events[1], Event::Text { delta: "lo".into() });
        let Event::Done { usage, cost_usd, session_id } = &events[2] else { panic!("{events:?}") };
        assert_eq!(*usage, Some(Usage { input_tokens: 10, output_tokens: 5 }));
        assert!((cost_usd.unwrap() - (10.0 * 4.0 + 5.0 * 20.0) / 1e6).abs() < 1e-12);
        assert_eq!(*session_id, None);

        turn(&mut engine, UserMessage::text("and again"));
        let requests = server.requests.lock().unwrap();
        let (head, first) = &requests[0];
        assert!(head.starts_with("post /v1/messages "), "{head}");
        assert!(head.contains("x-api-key: sk-ant-test-key"));
        assert!(head.contains("anthropic-version: 2023-06-01"));
        assert_eq!(first["model"], DEFAULT_MODEL);
        assert_eq!(first["stream"], true);
        assert_eq!(first["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(first["system"][0]["text"].as_str().unwrap().contains("Code2Form studio"));
        assert_eq!(first["tools"][0]["name"], "measure");
        assert_eq!(first["tools"][1]["description"], "A picture of the view");
        assert_eq!(first["tools"][1]["cache_control"]["type"], "ephemeral");
        assert!(first["tools"][0].get("cache_control").is_none());
        let messages = requests[1].1["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1], json!({ "role": "assistant", "content": [{ "type": "text", "text": "Hello" }] }));
        assert_eq!(messages[2]["content"][0]["text"], "and again");
    }

    #[test]
    fn an_earlier_conversation_seeds_the_history() {
        let server = mock(vec![reply(10, vec![text(0, &["Yes"])], "end_turn", 2)]);
        let (engine, _) = engine(&server.url);
        let turns = [(false, "stray"), (true, "a box"), (true, "40 mm"), (false, "Done."), (true, " ")];
        let mut engine = engine.with_history(turns.map(|(user, text)| (user, text.to_string())));
        turn(&mut engine, UserMessage::text("wider?"));
        let requests = server.requests.lock().unwrap();
        let messages = requests[0].1["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 2, "the user's two texts joined");
        assert_eq!(messages[1], json!({ "role": "assistant", "content": [{ "type": "text", "text": "Done." }] }));
        assert_eq!(messages[2]["content"][0]["text"], "wider?");
    }

    #[test]
    fn two_rounds_of_tools_then_the_answer() {
        let thinking = vec![
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "", "signature": "" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "sig" } }),
            json!({ "type": "content_block_stop", "index": 0 }),
        ];
        let server = mock(vec![
            reply(100, vec![text(0, &["Measuring."]), tool_use(1, "t1", "measure", &["{\"axis\":", " \"z\"}"])], "tool_use", 20),
            reply(200, vec![thinking, tool_use(1, "t2", "picture", &[])], "tool_use", 10),
            reply(300, vec![text(0, &["42 mm tall."])], "end_turn", 7),
        ]);
        let (mut engine, calls) = engine(&server.url);
        let events = turn(&mut engine, UserMessage::text("how tall?"));
        assert_eq!(events[0], Event::Text { delta: "Measuring.".into() });
        assert_eq!(events[1], Event::ToolCall { id: "t1".into(), name: "measure".into(), input: json!({ "axis": "z" }) });
        assert_eq!(
            events[2],
            Event::ToolResult { id: "t1".into(), content: vec![json!({ "type": "text", "text": "42 mm" })], is_error: false }
        );
        assert_eq!(events[3], Event::ToolCall { id: "t2".into(), name: "picture".into(), input: json!({}) });
        assert!(matches!(&events[4], Event::ToolResult { id, is_error: false, .. } if id == "t2"));
        assert_eq!(events[5], Event::Text { delta: "42 mm tall.".into() });
        let Event::Done { usage, .. } = &events[6] else { panic!("{events:?}") };
        assert_eq!(*usage, Some(Usage { input_tokens: 600, output_tokens: 37 }));
        assert_eq!(calls.lock().unwrap().len(), 2);

        let requests = server.requests.lock().unwrap();
        let second = requests[1].1["messages"].as_array().unwrap();
        assert_eq!(second[1]["content"][1], json!({ "type": "tool_use", "id": "t1", "name": "measure", "input": { "axis": "z" } }));
        assert_eq!(
            second[2]["content"][0],
            json!({ "type": "tool_result", "tool_use_id": "t1", "content": [{ "type": "text", "text": "42 mm" }] })
        );
        let third = requests[2].1["messages"].as_array().unwrap();
        // The thinking block goes back as it came, with its signature.
        assert_eq!(third[3]["content"][0], json!({ "type": "thinking", "thinking": "", "signature": "sig" }));
        assert_eq!(
            third[4]["content"][0],
            json!({ "type": "tool_result", "tool_use_id": "t2", "content": [{ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo=" } }] })
        );
    }

    #[test]
    fn images_and_errors_become_api_blocks() {
        let image = json!({ "type": "image", "data": "AAAA", "mimeType": "image/png" });
        let text = json!({ "type": "text", "text": "ok" });
        assert_eq!(
            tool_result("t", &Ok(vec![text, image, json!({ "type": "text", "text": "" })])),
            json!({ "type": "tool_result", "tool_use_id": "t", "content": [
                { "type": "text", "text": "ok" },
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } },
            ] })
        );
        assert_eq!(
            tool_result("t", &Err("no".into())),
            json!({ "type": "tool_result", "tool_use_id": "t", "content": [{ "type": "text", "text": "no" }], "is_error": true })
        );
        let message = UserMessage {
            text: "wider".into(),
            attachments: vec![
                Attachment::Image { png: b"Ma".to_vec() },
                Attachment::Context { label: "box.c2f:3-5".into(), text: "cube(10);".into() },
            ],
        };
        assert_eq!(
            user_content(&message),
            vec![
                json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "TWE=" } }),
                json!({ "type": "text", "text": "box.c2f:3-5:\ncube(10);" }),
                json!({ "type": "text", "text": "wider" }),
            ]
        );
    }

    #[test]
    fn a_refused_key_is_said_plainly() {
        let server = mock(vec![Canned::Status(
            401,
            "",
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        )]);
        let (mut engine, _) = engine(&server.url);
        let events = turn(&mut engine, UserMessage::text("hi"));
        let [Event::Error { message }] = &events[..] else { panic!("{events:?}") };
        assert!(message.contains("API key was refused"), "{message}");
    }

    #[test]
    fn an_overloaded_api_gets_the_request_again() {
        let server = mock(vec![
            Canned::Status(529, "retry-after: 0\r\n", r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#),
            reply(10, vec![text(0, &["Fine."])], "end_turn", 2),
        ]);
        let (mut engine, _) = engine(&server.url);
        let events = turn(&mut engine, UserMessage::text("hi"));
        assert_eq!(events[0], Event::Text { delta: "Fine.".into() });
        assert!(matches!(events[1], Event::Done { .. }));
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn stop_ends_the_turn_within_a_second() {
        let events = sse(&[start(10)].into_iter().chain(text(0, &["Thinking about"])).collect::<Vec<_>>());
        let server = mock(vec![Canned::Hang(events), reply(10, vec![text(0, &["Back."])], "end_turn", 2)]);
        let (mut engine, _) = engine(&server.url);
        let (sender, receiver) = mpsc::channel();
        engine.send(UserMessage::text("hi"), sender).unwrap();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(10)).unwrap(), Event::Text { delta: "Thinking about".into() });
        let stopped = Instant::now();
        engine.stop();
        let event = receiver.recv_timeout(Duration::from_secs(1)).expect("the turn ends");
        assert!(stopped.elapsed() < Duration::from_secs(1));
        assert!(matches!(event, Event::Done { .. }), "{event:?}");
        // The user's message stays; the next one joins it.
        let events = turn(&mut engine, UserMessage::text("still there?"));
        assert_eq!(events[0], Event::Text { delta: "Back.".into() });
        let requests = server.requests.lock().unwrap();
        let messages = requests[1].1["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"][1]["text"], "still there?");
    }

    #[test]
    fn a_message_after_a_stop_never_blocks_the_caller() {
        // The stop comes while a tool runs: the turn ends once it returns.
        let server = mock(vec![
            reply(10, vec![tool_use(0, "t1", "slow", &[])], "tool_use", 2),
            reply(10, vec![text(0, &["Back."])], "end_turn", 2),
        ]);
        let (mut engine, calls) = engine(&server.url);
        let (sender, receiver) = mpsc::channel();
        engine.send(UserMessage::text("hi"), sender).unwrap();
        let started = Instant::now();
        while calls.lock().unwrap().is_empty() && started.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        engine.stop();
        let asked = Instant::now();
        let (next, replies) = mpsc::channel();
        engine.send(UserMessage::text("still there?"), next).unwrap();
        assert!(asked.elapsed() < Duration::from_millis(200), "the caller waited {:?}", asked.elapsed());
        let ended = receiver.recv_timeout(Duration::from_secs(10)).expect("the stopped turn ends");
        assert!(receiver.iter().chain([ended]).any(|event| matches!(event, Event::Done { .. })));
        let event = replies.recv_timeout(Duration::from_secs(10)).expect("the next turn runs");
        assert_eq!(event, Event::Text { delta: "Back.".into() });
    }

    #[test]
    fn the_key_is_never_shown() {
        let options = ApiOptions::new("sk-ant-secret", Arc::new(Registry::new()));
        let shown = format!("{options:?}");
        assert!(!shown.contains("sk-ant-secret"), "{shown}");
        assert!(shown.contains("redacted"));
        assert!(models().iter().any(|(id, _)| *id == DEFAULT_MODEL));
    }

    /// A web search as the API streams it, after the shape of Anthropic's
    /// web search tool documentation.
    fn web_search(index: u64, id: &str) -> Vec<Value> {
        vec![
            json!({ "type": "content_block_start", "index": index, "content_block": { "type": "server_tool_use", "id": id, "name": "web_search", "input": {} } }),
            json!({ "type": "content_block_delta", "index": index, "delta": { "type": "input_json_delta", "partial_json": "{\"query\": \"M3 screw" } }),
            json!({ "type": "content_block_delta", "index": index, "delta": { "type": "input_json_delta", "partial_json": " head height\"}" } }),
            json!({ "type": "content_block_stop", "index": index }),
            json!({ "type": "content_block_start", "index": index + 1, "content_block": { "type": "web_search_tool_result", "tool_use_id": id, "content": [
                { "type": "web_search_result", "title": "ISO 4762 dimensions", "url": "https://example.com/iso4762", "encrypted_content": "EqgfCioIARgB", "page_age": "April 30, 2025" },
                { "type": "web_search_result", "title": "", "url": "https://example.org/m3", "encrypted_content": "EqgfCioIARgC", "page_age": null },
            ] } }),
            json!({ "type": "content_block_stop", "index": index + 1 }),
        ]
    }

    fn web_engine(url: &str) -> ApiEngine {
        let registry = Registry::new().with(Box::new(Fake(Arc::default()))).unwrap();
        let mut options = ApiOptions::new("sk-ant-test-key", Arc::new(registry));
        options.base_url = url.into();
        options.web = true;
        ApiEngine::new(options)
    }

    #[test]
    fn the_web_tools_go_only_when_allowed() {
        let server = mock(vec![reply(10, vec![text(0, &["Hi"])], "end_turn", 2), reply(10, vec![text(0, &["Hi"])], "end_turn", 2)]);
        let (mut without, _) = engine(&server.url);
        turn(&mut without, UserMessage::text("hi"));
        let mut with = web_engine(&server.url);
        turn(&mut with, UserMessage::text("hi"));
        let requests = server.requests.lock().unwrap();
        let types = |body: &Value| -> Vec<String> {
            body["tools"].as_array().unwrap().iter().filter_map(|tool| tool["type"].as_str().map(str::to_string)).collect()
        };
        assert!(types(&requests[0].1).is_empty(), "{:?}", requests[0].1["tools"]);
        assert!(!requests[0].1["system"][0]["text"].as_str().unwrap().contains("untrusted"));
        let with = &requests[1].1;
        assert_eq!(types(with), ["web_search_20260209", "web_fetch_20260209"]);
        assert_eq!(with["tools"][0]["name"], "web_search");
        assert_eq!(with["tools"][1]["name"], "web_fetch");
        // The breakpoint stays on the last tool, the registry's.
        assert_eq!(with["tools"][3]["cache_control"]["type"], "ephemeral");
        assert!(with["system"][0]["text"].as_str().unwrap().contains("untrusted"));
        // No beta header for the web tools.
        assert!(!requests[1].0.contains("web-search"), "{}", requests[1].0);
        assert_eq!(web_tools("claude-haiku-5-5")[0]["type"], "web_search_20250305");
        assert_eq!(web_tools("claude-haiku-5-5")[1]["type"], "web_fetch_20250910");
    }

    #[test]
    fn a_web_search_shows_as_a_tool_row_and_stays_in_the_history() {
        let server = mock(vec![
            reply(10, vec![text(0, &["Searching."]), web_search(1, "srvtoolu_1"), text(3, &["5.5 mm or less."])], "end_turn", 9),
            reply(20, vec![text(0, &["Yes."])], "end_turn", 2),
        ]);
        let mut engine = web_engine(&server.url);
        let events = turn(&mut engine, UserMessage::text("how tall is an M3 head?"));
        assert_eq!(events[0], Event::Text { delta: "Searching.".into() });
        assert_eq!(
            events[1],
            Event::ToolCall { id: "srvtoolu_1".into(), name: "web_search".into(), input: json!({ "query": "M3 screw head height" }) }
        );
        assert_eq!(
            events[2],
            Event::ToolResult {
                id: "srvtoolu_1".into(),
                content: vec![json!({ "type": "text", "text": "ISO 4762 dimensions — https://example.com/iso4762\nhttps://example.org/m3" })],
                is_error: false,
            }
        );
        assert_eq!(events[3], Event::Text { delta: "5.5 mm or less.".into() });
        assert!(matches!(events[4], Event::Done { .. }));

        turn(&mut engine, UserMessage::text("sure?"));
        let requests = server.requests.lock().unwrap();
        let messages = requests[1].1["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3, "user, assistant, user: no tool result of ours");
        let blocks = messages[1]["content"].as_array().unwrap();
        let kinds: Vec<&str> = blocks.iter().map(|block| block["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["text", "server_tool_use", "web_search_tool_result", "text"]);
        assert_eq!(blocks[1]["input"], json!({ "query": "M3 screw head height" }));
        assert_eq!(blocks[2]["content"][0]["encrypted_content"], "EqgfCioIARgB", "kept as it came");
    }

    #[test]
    fn a_paused_turn_goes_on_and_errors_show() {
        let error = vec![
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "server_tool_use", "id": "s2", "name": "web_fetch", "input": {} } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "input_json_delta", "partial_json": "{\"url\": \"https://example.com/a\"}" } }),
            json!({ "type": "content_block_stop", "index": 0 }),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "web_fetch_tool_result", "tool_use_id": "s2", "content": { "type": "web_fetch_tool_error", "error_code": "url_not_accessible" } } }),
            json!({ "type": "content_block_stop", "index": 1 }),
        ];
        let server = mock(vec![
            reply(10, vec![web_search(0, "s1")], "pause_turn", 3),
            reply(10, vec![error, text(2, &["Could not read it."])], "end_turn", 3),
        ]);
        let mut engine = web_engine(&server.url);
        let events = turn(&mut engine, UserMessage::text("look it up"));
        assert!(matches!(&events[1], Event::ToolResult { id, is_error: false, .. } if id == "s1"));
        assert!(matches!(&events[2], Event::ToolCall { name, input, .. } if name == "web_fetch" && input["url"] == "https://example.com/a"));
        assert_eq!(
            events[3],
            Event::ToolResult { id: "s2".into(), content: vec![json!({ "type": "text", "text": "failed: url_not_accessible" })], is_error: true }
        );
        let requests = server.requests.lock().unwrap();
        // The paused reply went back as it was, the last message.
        let messages = requests[1].1["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "server_tool_use");
    }

    #[test]
    fn server_calls_left_without_a_result_are_dropped() {
        let mut history = vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] }),
            json!({ "role": "assistant", "content": [
                { "type": "server_tool_use", "id": "a", "name": "web_search", "input": {} },
                { "type": "web_search_tool_result", "tool_use_id": "a", "content": [] },
                { "type": "server_tool_use", "id": "b", "name": "web_search", "input": {} },
            ] }),
        ];
        drop_dangling_server_calls(&mut history);
        assert_eq!(history[1]["content"].as_array().unwrap().len(), 2);
        let mut lone = vec![history[0].clone(), json!({ "role": "assistant", "content": [{ "type": "server_tool_use", "id": "c" }] })];
        drop_dangling_server_calls(&mut lone);
        assert_eq!(lone.len(), 1);
    }

    #[test]
    fn fallback_blocks_before_the_switch_are_dropped_but_the_text() {
        let mut content = vec![
            json!({ "type": "thinking", "thinking": "", "signature": "a" }),
            json!({ "type": "text", "text": "Part" }),
            json!({ "type": "fallback", "from": { "model": "x" }, "to": { "model": "y" } }),
            json!({ "type": "thinking", "thinking": "", "signature": "b" }),
        ];
        keep_after_fallback(&mut content);
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[2]["signature"], "b");
    }
}
