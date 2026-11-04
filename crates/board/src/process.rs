//! Running a program to its end, its output line by line as it comes, and
//! stopping it when asked.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// How a program ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Success,
    Failure,
    /// `cancel` was set: the program was stopped.
    Cancelled,
}

/// Runs `command`, handing each line it prints, on its output or its
/// errors, to `on_line`; stops it once `cancel` is set. An error when it
/// cannot start.
pub fn stream(mut command: Command, cancel: &AtomicBool, mut on_line: impl FnMut(&str)) -> Result<Ended, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    // In a group of its own, to stop the programs it starts with it: the
    // compilers under PlatformIO.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    // No console window flashing from the app.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let (send, lines) = mpsc::channel::<String>();
    let readers: Vec<_> = [child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>), child.stderr.take().map(|e| Box::new(e) as _)]
        .into_iter()
        .flatten()
        .map(|out| {
            let send = send.clone();
            std::thread::spawn(move || {
                let mut out = BufReader::new(out);
                let mut line = Vec::new();
                // Progress bars end their lines with `\r`: split there too.
                loop {
                    line.clear();
                    match read_line(&mut out, &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&line);
                            let text = text.trim_end_matches(['\n', '\r']);
                            if !text.trim().is_empty() && send.send(text.to_string()).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
        })
        .collect();
    drop(send);
    let mut cancelled = false;
    // When the program ended: one it started may keep the output open
    // after it, a little while at most.
    let mut exited: Option<std::time::Instant> = None;
    loop {
        match lines.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => on_line(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if !cancelled && cancel.load(Ordering::Relaxed) {
            cancelled = true;
            kill(&mut child);
        }
        // A program it started may keep the output open: stopped, it is
        // done all the same.
        if cancelled && child.try_wait().ok().flatten().is_some() {
            break;
        }
        if exited.is_none() && child.try_wait().ok().flatten().is_some() {
            exited = Some(std::time::Instant::now());
        }
        if exited.is_some_and(|at| at.elapsed() > Duration::from_secs(2)) {
            break;
        }
    }
    if !cancelled && exited.is_none_or(|at| at.elapsed() <= Duration::from_secs(2)) {
        for reader in readers {
            let _ = reader.join();
        }
    }
    let status = child.wait().map_err(|e| format!("{program}: {e}"))?;
    Ok(if cancelled {
        Ended::Cancelled
    } else if status.success() {
        Ended::Success
    } else {
        Ended::Failure
    })
}

/// Stops `child` and the programs it started.
fn kill(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    #[cfg(unix)]
    let _ = Command::new("kill").args(["-KILL", &format!("-{pid}")]).status();
    #[cfg(windows)]
    let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid]).status();
    let _ = child.kill();
}

/// Reads up to a `\n` or a `\r`, which it keeps.
fn read_line(out: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut read = 0;
    loop {
        let buffer = out.fill_buf()?;
        if buffer.is_empty() {
            return Ok(read);
        }
        if let Some(at) = buffer.iter().position(|&b| b == b'\n' || b == b'\r') {
            line.extend_from_slice(&buffer[..=at]);
            out.consume(at + 1);
            return Ok(read + at + 1);
        }
        let n = buffer.len();
        line.extend_from_slice(buffer);
        out.consume(n);
        read += n;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_lines_come_from_both_outputs_and_the_end_says_how_it_went() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo one; echo two >&2; printf 'three\\rfour\\n'; exit 3"]);
        let mut lines = Vec::new();
        let ended = stream(command, &AtomicBool::new(false), |l| lines.push(l.to_string())).unwrap();
        assert_eq!(ended, Ended::Failure);
        lines.sort();
        assert_eq!(lines, ["four", "one", "three", "two"]);
    }

    #[test]
    fn a_cancelled_program_is_stopped() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo started; sleep 30"]);
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let ended = stream(command, &cancel, |_| cancel.store(true, Ordering::Relaxed)).unwrap();
        assert_eq!(ended, Ended::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_missing_program_says_so() {
        let error = stream(Command::new("/no/such/program"), &AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(error.contains("/no/such/program"), "{error}");
    }
}
