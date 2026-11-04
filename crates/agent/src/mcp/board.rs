//! The MCP tools for the board: build the firmware, flash it, and read what
//! the board prints, to write a program, fix it and try it on the object.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use c2f_board::{Outcome, Severity, Target, Usage};
use serde_json::{Value, json};

use super::{Tool, pretty, schema_of, text};

/// Said when PlatformIO is missing.
const NO_TOOLCHAIN: &str = "PlatformIO, which builds and flashes the firmware, is not installed. Tell the user: they can \
                            install the Firmware module in Code2Form's settings (Settings › Firmware), or PlatformIO \
                            itself (platformio.org). Do not install it yourself.";
/// The longest `serial_read` listens.
const MAX_SECONDS: f64 = 30.0;
/// The warnings a build reports, at most.
const MAX_WARNINGS: usize = 20;

pub fn tools() -> Vec<Value> {
    let project = || {
        json!({
            "path": {
                "type": "string",
                "description": "The circuit's .c2fc file (its project is firmware/ beside it), the project's folder, or its platformio.ini.",
            },
        })
    };
    let port = json!({
        "type": "string",
        "description": "The board's serial port (/dev/cu.usbmodem1101, COM3). Default: the board plugged in that Code2Form knows.",
    });
    vec![
        json!({
            "name": "firmware_build",
            "title": "Build the firmware",
            "description": "Builds a PlatformIO project, such as the one `firmware` writes, with the Firmware module's \
                            PlatformIO. Returns whether it built, the compiler's errors and warnings with their file \
                            (relative to the project), line and column, and the flash and RAM the program takes. Call it \
                            after each change to the project's code, and fix the errors it gives before flashing. The \
                            first build of a board downloads its toolchain: it takes minutes.",
            "inputSchema": schema_of(project(), &["path"]),
        }),
        json!({
            "name": "firmware_upload",
            "title": "Flash the firmware on the board",
            "description": "Builds the project, then writes it onto the board plugged in by USB. It changes what runs on \
                            the user's hardware: say what it does before, and only flash a program the user asked for. \
                            In the studio, the user is asked to agree each time. Returns whether it flashed, and when \
                            it failed why and what the user should do (plug the board in, free the port, press BOOT). \
                            Then read what the board prints with `serial_read`.",
            "inputSchema": schema_of(super::merge(project(), json!({ "port": port.clone() })), &["path"]),
            "annotations": { "destructiveHint": true, "openWorldHint": true },
        }),
        json!({
            "name": "serial_read",
            "title": "Read the board",
            "description": "Listens to the board's serial port for `seconds` and returns the lines it printed, each with \
                            the time it came, in seconds. `reset` restarts the board first, to read what it prints as it \
                            starts (the first firmware prints each part as it tries it); `send` writes a line to it once \
                            the port is open, for a program that reads commands. Without a board plugged in, lists the \
                            ports there are.",
            "inputSchema": schema_of(json!({
                "port": port,
                "seconds": { "type": "number", "exclusiveMinimum": 0, "maximum": MAX_SECONDS, "default": 5, "description": "How long to listen." },
                "baud": { "type": "integer", "default": c2f_board::BAUD, "description": "The port's speed: the program's Serial.begin()." },
                "reset": { "type": "boolean", "default": false, "description": "Restart the board before listening." },
                "send": { "type": "string", "description": "A line to send to the board." },
            }), &[]),
        }),
    ]
}

/// The tool named `name`, if it is one of these.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "firmware_build" => build_tool,
        "firmware_upload" => upload_tool,
        "serial_read" => serial_tool,
        _ => return None,
    })
}

/// The PlatformIO project `path` names: the folder of its platformio.ini,
/// or a circuit's firmware/ folder.
fn project(args: &Value) -> Result<PathBuf, String> {
    let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("give the project's `path`")?);
    let dir = if path.extension().is_some_and(|e| e == "c2fc") {
        path.parent().unwrap_or(Path::new("")).join("firmware")
    } else if path.file_name().is_some_and(|n| n == "platformio.ini") {
        path.parent().unwrap_or(Path::new("")).to_path_buf()
    } else {
        path
    };
    if dir.join("platformio.ini").is_file() {
        // PlatformIO runs the Python a project names: the agent builds
        // none, as it runs no program of its own.
        let ini = std::fs::read_to_string(dir.join("platformio.ini")).unwrap_or_default();
        if ini.lines().any(|l| l.trim_start().starts_with("extra_scripts")) {
            return Err("the project runs scripts of its own as it builds (`extra_scripts` in platformio.ini): ask the \
                        user to build it from the studio instead"
                .into());
        }
        Ok(dir)
    } else {
        Err(format!("{} has no platformio.ini: write the circuit's first firmware with `firmware`, then build it", dir.display()))
    }
}

fn build_tool(args: &Value) -> Result<Vec<Value>, String> {
    let dir = project(args)?;
    let toolchain = c2f_board::find().ok_or(NO_TOOLCHAIN)?;
    let outcome = c2f_board::run(&toolchain, &dir, &Target::Build, &AtomicBool::new(false), |_| {});
    let mut report = report(&outcome);
    report["next"] = json!(if outcome.success {
        "Flash it with firmware_upload once the user agrees, then read the board with serial_read."
    } else {
        "Fix the errors at their line, then build again."
    });
    Ok(vec![text(pretty(&report))])
}

fn upload_tool(args: &Value) -> Result<Vec<Value>, String> {
    let dir = project(args)?;
    let toolchain = c2f_board::find().ok_or(NO_TOOLCHAIN)?;
    let port = port(args)?;
    if let Some(host) = crate::board::host() {
        if !host.approve_upload(&dir, &port) {
            return Err("The user declined to flash the board. Ask them what they want before trying again.".into());
        }
    }
    let target = Target::Upload { port: Some(port.clone()) };
    let outcome = c2f_board::run(&toolchain, &dir, &target, &AtomicBool::new(false), |_| {});
    if let Some(host) = crate::board::host() {
        host.upload_done(&port, outcome.success);
    }
    let mut report = report(&outcome);
    report["port"] = json!(port);
    if let Some(problem) = outcome.problem {
        report["problem"] = json!(problem.describe());
    }
    report["next"] = json!(if outcome.success {
        "Read what the board prints with serial_read (reset: true to see it start)."
    } else if outcome.problem.is_some() {
        "Tell the user what `problem` says, then try again once they have."
    } else {
        "Fix the errors at their line, then try again."
    });
    Ok(vec![text(pretty(&report))])
}

fn serial_tool(args: &Value) -> Result<Vec<Value>, String> {
    let port = port(args)?;
    let seconds = args.get("seconds").and_then(Value::as_f64).unwrap_or(5.0).clamp(0.1, MAX_SECONDS);
    let baud = args.get("baud").and_then(Value::as_u64).map_or(c2f_board::BAUD, |b| b as u32);
    let reset = args.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let send = args.get("send").and_then(Value::as_str);
    let duration = Duration::from_secs_f64(seconds);
    let lines = match crate::board::host().and_then(|h| h.read_serial(&port, duration, reset, send)) {
        Some(lines) => lines,
        None => c2f_board::read_for(&port, baud, duration, reset, send)?,
    };
    let lines: Vec<String> = lines.iter().map(|l| format!("{:7.3}  {}", l.at.as_secs_f64(), l.text)).collect();
    let report = json!({
        "port": port,
        "seconds": seconds,
        "lines": lines,
        "note": lines.is_empty().then_some("Nothing came: check the speed (`baud`, the program's Serial.begin()), or `reset` the board to read it start."),
    });
    Ok(vec![text(pretty(&report))])
}

/// The port `args` names, or the board plugged in.
fn port(args: &Value) -> Result<String, String> {
    if let Some(port) = args.get("port").and_then(Value::as_str) {
        return Ok(port.to_string());
    }
    let ports = c2f_board::ports();
    if let Some(port) = c2f_board::preferred(&ports) {
        return Ok(port.name.clone());
    }
    let listed: Vec<String> = ports.iter().map(|p| format!("{} ({})", p.name, p.description)).collect();
    Err(if listed.is_empty() {
        "No board is plugged in: ask the user to plug it in with a USB cable that carries data.".to_string()
    } else {
        format!("No board Code2Form knows is plugged in; give `port`, one of: {}", listed.join(", "))
    })
}

/// A build's or an upload's outcome, for the agent.
fn report(outcome: &Outcome) -> Value {
    let diagnostic = |d: &c2f_board::Diagnostic| {
        json!({ "file": d.file.display().to_string(), "line": d.line, "column": d.column, "message": d.message })
    };
    let usage = |u: Option<Usage>| {
        u.map(|u| json!({ "used": u.used, "total": u.total, "percent": (u.used as f64 * 1000.0 / u.total.max(1) as f64).round() / 10.0 }))
    };
    let errors: Vec<Value> = outcome.errors().map(diagnostic).collect();
    // The project's own: those of the framework's headers are not the
    // agent's to fix.
    let warnings: Vec<Value> = outcome
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning && d.file.is_relative())
        .take(MAX_WARNINGS)
        .map(diagnostic)
        .collect();
    json!({
        "ok": outcome.success,
        "errors": errors,
        "warnings": warnings,
        "flash": usage(outcome.size.flash),
        "ram": usage(outcome.size.ram),
        // What explains a failure the messages do not: a link error, a
        // missing library.
        "output": (!outcome.success && errors.is_empty()).then_some(&outcome.tail),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_is_found_from_the_circuit_or_its_folder() {
        let dir = std::env::temp_dir().join(format!("c2f-board-tools-{}", std::process::id()));
        let firmware = dir.join("firmware");
        std::fs::create_dir_all(&firmware).unwrap();
        std::fs::write(firmware.join("platformio.ini"), "[env:x]\n").unwrap();
        for path in [dir.join("lamp.c2fc"), firmware.clone(), firmware.join("platformio.ini")] {
            assert_eq!(project(&json!({ "path": path })).unwrap(), firmware);
        }
        let err = project(&json!({ "path": dir })).unwrap_err();
        assert!(err.contains("`firmware`"), "{err}");
        std::fs::write(firmware.join("platformio.ini"), "[env:x]\nextra_scripts = pre:run.py\n").unwrap();
        let err = project(&json!({ "path": firmware })).unwrap_err();
        assert!(err.contains("extra_scripts"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_report_gives_the_errors_and_the_memories() {
        let mut outcome = Outcome::default();
        outcome.diagnostics.push(c2f_board::parse_diagnostic("src/main.cpp:4:3: error: 'X' was not declared").unwrap());
        outcome.tail = vec!["*** Error 1".into()];
        let report = report(&outcome);
        assert_eq!(report["ok"], false);
        assert_eq!(report["errors"][0]["line"], 4);
        assert_eq!(report["output"], Value::Null, "the errors say it");
        outcome.success = true;
        outcome.diagnostics.clear();
        outcome.size.flash = Some(Usage { used: 328003, total: 3342336 });
        let report = super::report(&outcome);
        assert_eq!(report["flash"]["percent"], 9.8);
        assert_eq!(report["ram"], Value::Null);
    }
}
