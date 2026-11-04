//! What the board tools ask of the program they run in: the studio asks
//! the user before a firmware goes onto the board, and shares its serial
//! monitor's port. `c2f mcp` sets no host: its client asks the user before
//! a tool that changes something, as it does for any tool.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

/// A program the board tools run in, with a board of its own.
pub trait BoardHost: Send + Sync {
    /// Asks the user whether the firmware in `project` may go onto the
    /// board at `port`; true once they agree. The host frees the port
    /// meanwhile (its serial monitor closes).
    fn approve_upload(&self, project: &Path, port: &str) -> bool;
    /// The upload [`BoardHost::approve_upload`] allowed is over.
    fn upload_done(&self, port: &str, success: bool);
    /// The lines of `port` for `duration`, when the host reads it already
    /// (its serial monitor is open on it); `None` otherwise, and the tool
    /// opens the port itself.
    fn read_serial(&self, port: &str, duration: Duration, reset: bool, send: Option<&str>) -> Option<Vec<c2f_board::Line>>;
}

static HOST: OnceLock<Box<dyn BoardHost>> = OnceLock::new();

/// Sets the program's board host, once.
pub fn set_host(host: Box<dyn BoardHost>) {
    let _ = HOST.set(host);
}

pub fn host() -> Option<&'static dyn BoardHost> {
    HOST.get().map(|h| h.as_ref())
}

/// The names the studio answers the board's questions under, over its
/// socket, for an agent's tools served by another process (`code2form mcp
/// --studio`): not listed among its tools.
pub const APPROVE_UPLOAD: &str = "_board_approve_upload";
pub const UPLOAD_DONE: &str = "_board_upload_done";
pub const READ_SERIAL: &str = "_board_read_serial";

/// The studio that is open, as the host of the board tools of a process it
/// started (`mcp --studio`): it asks the user there, and shares its
/// monitor. A studio that does not answer refuses the upload.
pub struct StudioHost {
    studio: crate::studio_link::StudioProxy,
}

impl StudioHost {
    pub fn new() -> StudioHost {
        StudioHost { studio: crate::studio_link::StudioProxy::new() }
    }

    /// The studio serving on the socket at `path`.
    pub fn at(path: impl Into<std::path::PathBuf>) -> StudioHost {
        StudioHost { studio: crate::studio_link::StudioProxy::at(path) }
    }

    fn ask(&self, name: &str, args: serde_json::Value) -> Option<serde_json::Value> {
        use crate::Tools;
        let content = self.studio.call(name, &args)?.ok()?;
        serde_json::from_str(content.first()?.get("text")?.as_str()?).ok()
    }
}

impl Default for StudioHost {
    fn default() -> StudioHost {
        StudioHost::new()
    }
}

impl BoardHost for StudioHost {
    fn approve_upload(&self, project: &Path, port: &str) -> bool {
        let args = serde_json::json!({ "project": project, "port": port });
        self.ask(APPROVE_UPLOAD, args).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    fn upload_done(&self, port: &str, success: bool) {
        let _ = self.ask(UPLOAD_DONE, serde_json::json!({ "port": port, "success": success }));
    }

    fn read_serial(&self, port: &str, duration: Duration, reset: bool, send: Option<&str>) -> Option<Vec<c2f_board::Line>> {
        let args = serde_json::json!({ "port": port, "seconds": duration.as_secs_f64(), "reset": reset, "send": send });
        let lines = self.ask(READ_SERIAL, args)?;
        let line = |v: &serde_json::Value| {
            Some(c2f_board::Line { at: Duration::from_secs_f64(v.get("at")?.as_f64()?), text: v.get("text")?.as_str()?.to_string() })
        };
        lines.as_array().map(|lines| lines.iter().filter_map(line).collect())
    }
}

/// The studio's side of [`StudioHost`]: the answer to a call named
/// [`APPROVE_UPLOAD`], [`UPLOAD_DONE`] or [`READ_SERIAL`], from this
/// process's host, as one JSON text; `None` for another name.
pub fn answer(name: &str, args: &serde_json::Value) -> Option<Result<Vec<serde_json::Value>, String>> {
    use serde_json::{Value, json};
    let host = host();
    let text = |value: Value| Ok(vec![json!({ "type": "text", "text": value.to_string() })]);
    let port = args.get("port").and_then(Value::as_str).unwrap_or_default();
    Some(match name {
        APPROVE_UPLOAD => {
            let project = args.get("project").and_then(Value::as_str).unwrap_or_default();
            text(json!(host.is_some_and(|h| h.approve_upload(Path::new(project), port))))
        }
        UPLOAD_DONE => {
            if let Some(h) = host {
                h.upload_done(port, args.get("success").and_then(Value::as_bool).unwrap_or(false));
            }
            text(Value::Null)
        }
        READ_SERIAL => {
            let seconds = args.get("seconds").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 60.0);
            let reset = args.get("reset").and_then(Value::as_bool).unwrap_or(false);
            let send = args.get("send").and_then(Value::as_str);
            let lines = host.and_then(|h| h.read_serial(port, Duration::from_secs_f64(seconds), reset, send));
            let lines = lines.map(|l| l.iter().map(|l| json!({ "at": l.at.as_secs_f64(), "text": l.text })).collect::<Vec<_>>());
            text(json!(lines))
        }
        _ => return None,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;

    use serde_json::Value;

    use super::*;
    use crate::Tools;

    /// A studio's live tools, the board's questions alone.
    struct Studio;

    impl Tools for Studio {
        fn list(&self) -> Vec<Value> {
            Vec::new()
        }

        fn call(&self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
            answer(name, args)
        }
    }

    #[test]
    fn an_upload_is_refused_unless_the_studio_agrees() {
        // No studio open: refused.
        let nowhere = StudioHost::at(std::env::temp_dir().join("c2f-no-studio/studio.sock"));
        assert!(!nowhere.approve_upload(Path::new("/p"), "/dev/x"));
        // A studio without a board host of its own (this test's process):
        // its answer goes over the socket, and refuses too; nor does it
        // read the port for the tool.
        let dir = std::env::temp_dir().join(format!("c2f-board-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        let socket = dir.join("studio.sock");
        crate::studio_link::serve_at(&socket, Arc::new(Studio)).unwrap();
        let studio = StudioHost::at(&socket);
        assert!(!studio.approve_upload(Path::new("/p"), "/dev/x"));
        assert_eq!(studio.read_serial("/dev/x", Duration::from_millis(1), false, None), None);
        studio.upload_done("/dev/x", true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
