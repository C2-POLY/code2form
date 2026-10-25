//! The link between the studio and the processes that call its live
//! tools, such as `c2f mcp --studio` run by an agent: a local socket only
//! the user can open, in a folder of their own
//! (`<temp>/code2form-<uid>/studio.sock`, or the file
//! `CODE2FORM_STUDIO_SOCKET` names).
//!
//! The protocol is JSON, one message per line. A request is
//! `{"id": 1, "method": "list"}` or `{"id": 2, "method": "call", "name":
//! "edit_code", "args": {...}}`; its reply `{"id": 1, "result": ...}`,
//! the tools for `list` and the content items for `call` (`null` when the
//! studio has no such tool), or `{"id": 2, "error": "..."}`, the error the
//! agent reads.
//!
//! The studio [`serve`]s its tools; a [`StudioProxy`] is a provider of
//! tools that calls them, over a connection for each call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::Tools;

/// What a call says when no studio answers.
pub const NOT_OPEN: &str = "the Code2Form studio is not open: open it, then try again";

/// How long a call waits for the studio's answer: the studio itself waits
/// for a render up to its own timeout, and for the user's answer when it
/// asks them before a change.
#[cfg_attr(not(unix), allow(dead_code))]
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(660);

/// The socket the studio serves on: the file `CODE2FORM_STUDIO_SOCKET`
/// names, else `studio.sock` in the user's own folder of the temporary
/// directory.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CODE2FORM_STUDIO_SOCKET") {
        return PathBuf::from(path);
    }
    std::env::temp_dir().join(format!("code2form-{}", user_id())).join("studio.sock")
}

#[cfg(unix)]
fn user_id() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn user_id() -> String {
    std::env::var("USERNAME").unwrap_or_default()
}

/// Serves `tools` on [`socket_path`] from a thread of its own, each
/// connection on another; returns the socket's path. The error says why
/// the socket could not be made, such as another studio serving on it.
pub fn serve(tools: Arc<dyn Tools>) -> Result<PathBuf, String> {
    let path = socket_path();
    serve_at(&path, tools)?;
    Ok(path)
}

/// Serves `tools` on the socket at `path`: its folder is made, readable by
/// the user alone, and a socket left there by a studio that quit is
/// replaced.
#[cfg(unix)]
pub fn serve_at(path: &Path, tools: Arc<dyn Tools>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    let dir = path.parent().ok_or_else(|| format!("{} is in no folder", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|err| format!("cannot make {}: {err}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| format!("cannot keep {} to this user: {err}", dir.display()))?;
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(format!("another studio serves its tools on {}", path.display()));
        }
        std::fs::remove_file(path).map_err(|err| format!("cannot remove the old socket {}: {err}", path.display()))?;
    }
    let listener = UnixListener::bind(path).map_err(|err| format!("cannot listen on {}: {err}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("cannot keep {} to this user: {err}", path.display()))?;
    std::thread::Builder::new()
        .name("studio-link".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tools = tools.clone();
                // A connection is served on its own: a tool waiting for a
                // render holds no other caller back.
                let _ = std::thread::Builder::new().name("studio-link-call".into()).spawn(move || answer(stream, &*tools));
            }
        })
        .map_err(|err| format!("cannot start serving: {err}"))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn serve_at(_path: &Path, _tools: Arc<dyn Tools>) -> Result<(), String> {
    Err("the studio's live tools are not supported on this system yet".into())
}

/// Answers each request of a connection until it closes.
#[cfg(unix)]
fn answer(stream: std::os::unix::net::UnixStream, tools: &dyn Tools) {
    use std::io::{BufRead, BufReader, Write};

    let Ok(mut writer) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let reply = reply(&line, tools);
        if writeln!(writer, "{reply}").and_then(|()| writer.flush()).is_err() {
            return;
        }
    }
}

/// The reply to one request.
#[cfg_attr(not(unix), allow(dead_code))]
fn reply(line: &str, tools: &dyn Tools) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return json!({ "id": null, "error": format!("cannot read the request: {err}") }),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str) {
        Some("list") => json!({ "id": id, "result": tools.list() }),
        Some("call") => {
            let Some(name) = request.get("name").and_then(Value::as_str) else {
                return json!({ "id": id, "error": "a call names its tool in `name`" });
            };
            let args = request.get("args").cloned().unwrap_or_else(|| json!({}));
            let called = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tools.call(name, &args)))
                .unwrap_or_else(|_| Some(Err(format!("internal error: `{name}` panicked in the studio"))));
            match called {
                None => json!({ "id": id, "result": null }),
                Some(Ok(content)) => json!({ "id": id, "result": content }),
                Some(Err(error)) => json!({ "id": id, "error": error }),
            }
        }
        other => json!({ "id": id, "error": format!("unknown method {}", other.unwrap_or("(none)")) }),
    }
}

/// The tools of the studio that is open, called over its socket: none
/// listed while no studio is open, and each call then says so.
pub struct StudioProxy {
    path: PathBuf,
    next_id: std::sync::atomic::AtomicU64,
}

impl StudioProxy {
    /// The tools of the studio serving on [`socket_path`].
    pub fn new() -> StudioProxy {
        StudioProxy::at(socket_path())
    }

    /// The tools of the studio serving on the socket at `path`.
    pub fn at(path: impl Into<PathBuf>) -> StudioProxy {
        StudioProxy { path: path.into(), next_id: std::sync::atomic::AtomicU64::new(1) }
    }

    /// Sends one request and reads its reply; the error says the studio
    /// is not open, or what went wrong on the way.
    fn request(&self, mut request: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        request["id"] = json!(id);
        let reply = exchange(&self.path, &request)?;
        if reply.get("id") != Some(&json!(id)) {
            return Err(format!("the studio answered another request: {reply}"));
        }
        match (reply.get("result"), reply.get("error")) {
            (_, Some(error)) => Err(error.as_str().map_or_else(|| error.to_string(), str::to_string)),
            (Some(result), None) => Ok(result.clone()),
            (None, None) => Err(format!("the studio's reply has no result: {reply}")),
        }
    }
}

impl Default for StudioProxy {
    fn default() -> StudioProxy {
        StudioProxy::new()
    }
}

/// Writes `request` on a new connection to the socket at `path` and reads
/// the reply.
#[cfg(unix)]
fn exchange(path: &Path, request: &Value) -> Result<Value, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(path).map_err(|_| NOT_OPEN.to_string())?;
    let lost = |err: std::io::Error| format!("the studio stopped answering: {err}");
    stream.set_read_timeout(Some(CALL_TIMEOUT)).map_err(lost)?;
    writeln!(stream, "{request}").and_then(|()| stream.flush()).map_err(lost)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(lost)?;
    if line.is_empty() {
        return Err("the studio closed the connection without answering".into());
    }
    serde_json::from_str(&line).map_err(|err| format!("cannot read the studio's reply: {err}"))
}

#[cfg(not(unix))]
fn exchange(_path: &Path, _request: &Value) -> Result<Value, String> {
    Err("the studio's live tools are not supported on this system yet".into())
}

impl Tools for StudioProxy {
    fn list(&self) -> Vec<Value> {
        match self.request(json!({ "method": "list" })) {
            Ok(Value::Array(tools)) => tools,
            _ => Vec::new(),
        }
    }

    fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
        match self.request(json!({ "method": "call", "name": name, "args": args })) {
            Ok(Value::Null) => None,
            Ok(Value::Array(content)) => Some(Ok(content)),
            Ok(other) => Some(Err(format!("the studio's reply is not content: {other}"))),
            Err(error) => Some(Err(error)),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A studio with one tool, `echo`.
    struct Echo;

    impl Tools for Echo {
        fn list(&self) -> Vec<Value> {
            vec![json!({ "name": "echo", "description": "Says its `text` back.", "inputSchema": { "type": "object" } })]
        }

        fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
            match (name, args.get("text").and_then(Value::as_str)) {
                ("echo", Some(text)) => Some(Ok(vec![json!({ "type": "text", "text": text })])),
                ("echo", None) => Some(Err("`text` is missing".into())),
                _ => None,
            }
        }
    }

    fn socket(name: &str) -> PathBuf {
        // Short: a socket's path has a length limit.
        std::env::temp_dir().join(format!("c2f-{name}-{}", std::process::id())).join("s.sock")
    }

    #[test]
    fn a_proxy_lists_and_calls_the_studio_s_tools() {
        let path = socket("link");
        serve_at(&path, Arc::new(Echo)).unwrap();
        let proxy = StudioProxy::at(&path);
        let names: Vec<Value> = proxy.list().iter().map(|tool| tool["name"].clone()).collect();
        assert_eq!(names, vec![json!("echo")]);
        let content = proxy.call("echo", &json!({ "text": "hello" })).unwrap().unwrap();
        assert_eq!(content, vec![json!({ "type": "text", "text": "hello" })]);
        assert_eq!(proxy.call("echo", &json!({})).unwrap().unwrap_err(), "`text` is missing");
        assert!(proxy.call("nothing", &json!({})).is_none());
        // The folder and the socket are the user's alone.
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(path.parent().unwrap()), mode(&path)), (0o700, 0o600));
        // A second studio does not take the socket of the first.
        assert!(serve_at(&path, Arc::new(Echo)).unwrap_err().contains("another studio"));
        // A registry takes the studio's tools after the built-in ones.
        let registry = crate::Registry::builtin().with(Box::new(StudioProxy::at(&path))).unwrap();
        assert_eq!(registry.names().last().map(String::as_str), Some("echo"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn no_studio_lists_nothing_and_calls_say_so() {
        let proxy = StudioProxy::at(socket("none"));
        assert!(proxy.list().is_empty());
        assert_eq!(proxy.call("edit_code", &json!({})).unwrap().unwrap_err(), NOT_OPEN);
    }

    #[test]
    fn a_socket_left_by_a_studio_that_quit_is_replaced() {
        let path = socket("stale");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A socket file nobody answers on. Not a listener bound then
        // dropped: on macOS a socket is made close-on-exec only after it
        // is made, so a process another test spawns in between keeps the
        // listener open, and it answers. A datagram socket never accepts
        // a connection, whoever holds it.
        drop(std::os::unix::net::UnixDatagram::bind(&path).unwrap());
        assert!(path.exists());
        serve_at(&path, Arc::new(Echo)).unwrap();
        assert_eq!(StudioProxy::at(&path).list().len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn bad_requests_get_errors() {
        assert!(reply("not json", &Echo)["error"].as_str().unwrap().contains("cannot read"));
        assert!(reply(r#"{"id": 3, "method": "nope"}"#, &Echo)["error"].as_str().unwrap().contains("unknown method"));
        assert_eq!(reply(r#"{"id": 4, "method": "call"}"#, &Echo)["id"], 4);
    }
}
