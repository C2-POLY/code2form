//! Building a PlatformIO project and flashing it on the board, with what
//! the compiler said at its line, the memory the program takes, and what
//! to do when the board does not answer.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::process::{Ended, stream};
use crate::toolchain::Toolchain;

/// What to do with the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Build,
    /// Build, then flash on the board at `port`, or the one PlatformIO
    /// finds.
    Upload { port: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

/// A message of the compiler about a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// As the compiler names it: relative to the project (`src/main.cpp`)
    /// for its own files.
    pub file: PathBuf,
    /// From 1.
    pub line: usize,
    /// From 1; 0 when the compiler gives none.
    pub column: usize,
    pub severity: Severity,
    pub message: String,
}

/// How much of a memory the program takes, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

/// The memories the program takes: its code and constants (flash), and
/// its variables (RAM).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub flash: Option<Usage>,
    pub ram: Option<Usage>,
}

/// Why flashing failed, when it is a known reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadProblem {
    /// No board is plugged in, or none PlatformIO knows.
    NoBoard,
    /// Another program holds the port, such as a serial monitor.
    PortBusy,
    /// The board does not answer: it needs to be put in its download mode.
    NoAnswer,
}

impl UploadProblem {
    /// What to do, in a sentence.
    pub fn describe(self) -> &'static str {
        match self {
            UploadProblem::NoBoard => "No board was found: plug it in with a USB cable that carries data, then try again.",
            UploadProblem::PortBusy => {
                "The board's port is taken by another program, such as a serial monitor: close it, then try again."
            }
            UploadProblem::NoAnswer => {
                "The board does not answer: hold its BOOT button, press and release RESET, release BOOT, then try again."
            }
        }
    }
}

/// How a build or an upload went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub success: bool,
    pub cancelled: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub size: Size,
    pub problem: Option<UploadProblem>,
    /// The last lines printed, for a failure the diagnostics do not
    /// explain (a linker error, a missing library).
    pub tail: Vec<String>,
}

impl Outcome {
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error)
    }
}

/// How many of the last lines an outcome keeps.
const TAIL: usize = 30;

/// Builds or flashes the PlatformIO project in `project` with `toolchain`,
/// handing each line printed to `on_line`; `cancel` stops it.
pub fn run(toolchain: &Toolchain, project: &Path, target: &Target, cancel: &AtomicBool, mut on_line: impl FnMut(&str)) -> Outcome {
    let mut args = vec!["run".into(), "-d".into(), project.as_os_str().to_owned()];
    if let Target::Upload { port } = target {
        args.extend(["-t".into(), "upload".into()]);
        if let Some(port) = port {
            args.extend(["--upload-port".into(), port.into()]);
        }
    }
    let mut outcome = Outcome::default();
    let mut lines = Vec::new();
    let ended = stream(toolchain.command(args), cancel, |line| {
        on_line(line);
        read(&mut outcome, line);
        lines.push(line.to_string());
    });
    match ended {
        Ok(ended) => {
            outcome.success = ended == Ended::Success;
            outcome.cancelled = ended == Ended::Cancelled;
        }
        Err(error) => lines.push(error),
    }
    if !outcome.success && matches!(target, Target::Upload { .. }) && outcome.errors().next().is_none() {
        outcome.problem = problem(&lines);
    }
    outcome.tail = lines.split_off(lines.len().saturating_sub(TAIL));
    outcome
}

/// Takes what a line says into `outcome`.
fn read(outcome: &mut Outcome, line: &str) {
    if let Some(diagnostic) = parse_diagnostic(line) {
        if !outcome.diagnostics.contains(&diagnostic) {
            outcome.diagnostics.push(diagnostic);
        }
    } else if let Some((flash, usage)) = parse_usage(line) {
        if flash {
            outcome.size.flash = Some(usage);
        } else {
            outcome.size.ram = Some(usage);
        }
    }
}

/// A compiler's message, `src/main.cpp:12:5: error: 'x' was not declared`.
pub fn parse_diagnostic(line: &str) -> Option<Diagnostic> {
    let (severity, at, tag) = [("fatal error", Severity::Error), ("error", Severity::Error), ("warning", Severity::Warning), ("note", Severity::Note)]
        .into_iter()
        .find_map(|(tag, severity)| line.find(&format!(": {tag}: ")).map(|at| (severity, at, tag)))?;
    let message = line[at + tag.len() + 4..].trim().to_string();
    // `file:line:column` or `file:line`, the file maybe with a drive's `:`.
    let place = &line[..at];
    let mut numbers = Vec::new();
    let mut file = place;
    while numbers.len() < 2 {
        match file.rsplit_once(':') {
            Some((rest, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                numbers.push(n.parse::<usize>().ok()?);
                file = rest;
            }
            _ => break,
        }
    }
    let (line, column) = match numbers[..] {
        [column, line] => (line, column),
        [line] => (line, 0),
        _ => return None,
    };
    (!file.is_empty() && !file.contains(' ') || Path::new(file).exists()).then(|| Diagnostic {
        file: PathBuf::from(file),
        line,
        column,
        severity,
        message,
    })
}

/// PlatformIO's account of a memory:
/// `RAM:   [=         ]   7.4% (used 24308 bytes from 327680 bytes)`;
/// true for the flash.
pub fn parse_usage(line: &str) -> Option<(bool, Usage)> {
    let line = line.trim_start();
    let flash = if line.starts_with("Flash:") {
        true
    } else if line.starts_with("RAM:") {
        false
    } else {
        return None;
    };
    let rest = &line[line.find("(used ")? + 6..];
    let (used, rest) = rest.split_once(" bytes from ")?;
    let total = rest.split_once(" bytes")?.0;
    Some((flash, Usage { used: used.trim().parse().ok()?, total: total.trim().parse().ok()? }))
}

/// The known reason an upload failed, from what it printed.
fn problem(lines: &[String]) -> Option<UploadProblem> {
    let said = |words: &[&str]| lines.iter().any(|l| words.iter().any(|w| l.contains(w)));
    if said(&["Resource busy", "Could not exclusively lock port", "Access is denied", "PermissionError"]) {
        Some(UploadProblem::PortBusy)
    } else if said(&["Failed to connect to", "No serial data received", "Wrong boot mode", "Timed out waiting for packet"]) {
        Some(UploadProblem::NoAnswer)
    } else if said(&["Please specify `upload_port`", "could not open port", "Could not open", "No such file or directory: '/dev/"]) {
        Some(UploadProblem::NoBoard)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compiler_message_gives_its_file_line_and_column() {
        let d = parse_diagnostic("src/main.cpp:12:5: error: 'AMP_SD' was not declared in this scope").unwrap();
        assert_eq!(
            d,
            Diagnostic {
                file: "src/main.cpp".into(),
                line: 12,
                column: 5,
                severity: Severity::Error,
                message: "'AMP_SD' was not declared in this scope".into()
            }
        );
        let d = parse_diagnostic("include/pins.h:3: warning: \"X\" redefined").unwrap();
        assert_eq!((d.line, d.column, d.severity), (3, 0, Severity::Warning));
        let d = parse_diagnostic("C:\\proj\\src\\main.cpp:7:1: fatal error: Foo.h: No such file or directory").unwrap();
        assert_eq!(d.file, PathBuf::from("C:\\proj\\src\\main.cpp"));
        assert_eq!((d.line, d.severity), (7, Severity::Error));
        assert_eq!(d.message, "Foo.h: No such file or directory");
        // Lines that only mention an error are not messages about a line.
        assert_eq!(parse_diagnostic("*** [.pio/build/firmware.elf] Error 1"), None);
        assert_eq!(parse_diagnostic("Error: Please specify `upload_port`"), None);
    }

    #[test]
    fn the_memories_used_come_from_platformios_account() {
        assert_eq!(
            parse_usage("RAM:   [=         ]   7.4% (used 24308 bytes from 327680 bytes)"),
            Some((false, Usage { used: 24308, total: 327680 }))
        );
        assert_eq!(
            parse_usage("Flash: [=         ]   9.8% (used 328003 bytes from 3342336 bytes)"),
            Some((true, Usage { used: 328003, total: 3342336 }))
        );
        assert_eq!(parse_usage("Checking size .pio/build/firmware.elf"), None);
    }

    #[test]
    fn a_failed_upload_says_why_when_it_is_known() {
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(problem(&lines(&["Error: Please specify `upload_port` for environment"])), Some(UploadProblem::NoBoard));
        assert_eq!(
            problem(&lines(&["A fatal error occurred: Could not open /dev/cu.usbmodem1, the port is busy or doesn't exist.", "([Errno 16] Resource busy: '/dev/cu.usbmodem1')"])),
            Some(UploadProblem::PortBusy)
        );
        assert_eq!(problem(&lines(&["A fatal error occurred: Failed to connect to ESP32-S3: No serial data received."])), Some(UploadProblem::NoAnswer));
        assert_eq!(problem(&lines(&["something else"])), None);
    }

    #[test]
    fn the_outcome_keeps_each_message_once() {
        let mut outcome = Outcome::default();
        for line in ["src/main.cpp:3:1: error: x", "src/main.cpp:3:1: error: x", "src/main.cpp:4:1: warning: y"] {
            read(&mut outcome, line);
        }
        assert_eq!(outcome.diagnostics.len(), 2);
        assert_eq!(outcome.errors().count(), 1);
    }
}
