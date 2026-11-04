//! The board's serial port, read in the background: the lines it prints,
//! the lines sent to it, its reset, and its coming back once plugged in
//! again.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serialport::SerialPort;

/// The speed of the first firmware Code2Form writes (`Serial.begin`).
pub const BAUD: u32 = 115_200;

/// How many lines a monitor keeps: the oldest go first.
pub const MAX_LINES: usize = 5_000;

/// A line the board printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// When it came, from the monitor's opening.
    pub at: Duration,
    pub text: String,
}

/// Where the monitor is with its port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Opening the port.
    Connecting,
    Connected,
    /// The port is gone (the board unplugged) or cannot be opened, and why:
    /// the monitor tries again until closed.
    Disconnected(String),
}

/// A serial port read in the background until closed or dropped.
pub struct Monitor {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    port: String,
    baud: u32,
    opened: Instant,
    lines: Mutex<VecDeque<Line>>,
    status: Mutex<Status>,
    outgoing: Mutex<Vec<u8>>,
    reset: AtomicBool,
    stop: AtomicBool,
    /// Counts the lines received, to tell new ones.
    received: AtomicU64,
}

impl Monitor {
    /// Opens `port` at `baud`, in the background.
    pub fn open(port: &str, baud: u32) -> Monitor {
        let shared = Arc::new(Shared {
            port: port.to_string(),
            baud,
            opened: Instant::now(),
            lines: Mutex::new(VecDeque::new()),
            status: Mutex::new(Status::Connecting),
            outgoing: Mutex::new(Vec::new()),
            reset: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            received: AtomicU64::new(0),
        });
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new().name("serial monitor".into()).spawn(move || watch(&shared)).ok()
        };
        Monitor { shared, thread }
    }

    pub fn port(&self) -> &str {
        &self.shared.port
    }

    pub fn baud(&self) -> u32 {
        self.shared.baud
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    /// How many lines came since it opened: changes with each new line.
    pub fn received(&self) -> u64 {
        self.shared.received.load(Ordering::Relaxed)
    }

    /// Runs `f` on the lines kept, oldest first.
    pub fn with_lines<R>(&self, f: impl FnOnce(&VecDeque<Line>) -> R) -> R {
        f(&self.shared.lines.lock().unwrap())
    }

    /// Sends `text` and a line end to the board.
    pub fn send(&self, text: &str) {
        let mut outgoing = self.shared.outgoing.lock().unwrap();
        outgoing.extend_from_slice(text.as_bytes());
        outgoing.push(b'\n');
    }

    /// Restarts the board, through the port's RTS line, as esptool does.
    pub fn reset(&self) {
        self.shared.reset.store(true, Ordering::Relaxed);
    }

    /// Lets go of the port, for every handle on the monitor: the lines
    /// kept stay readable.
    pub fn close(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        self.shared.lines.lock().unwrap().clear();
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens the port, reads it, and opens it again once lost, until stopped.
fn watch(shared: &Shared) {
    while !shared.stop.load(Ordering::Relaxed) {
        match open(&shared.port, shared.baud) {
            Ok(mut port) => {
                *shared.status.lock().unwrap() = Status::Connected;
                let lost = read(shared, port.as_mut());
                if let Some(why) = lost {
                    *shared.status.lock().unwrap() = Status::Disconnected(why);
                }
            }
            Err(why) => *shared.status.lock().unwrap() = Status::Disconnected(why),
        }
        // Wait for the board to come back.
        for _ in 0..5 {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn open(port: &str, baud: u32) -> Result<Box<dyn SerialPort>, String> {
    let mut port = serialport::new(port, baud).timeout(Duration::from_millis(50)).open().map_err(|e| e.to_string())?;
    // Lines down: a board wired for esptool's reset (EN on RTS, BOOT on
    // DTR) runs rather than staying held in reset.
    let _ = port.write_data_terminal_ready(false);
    let _ = port.write_request_to_send(false);
    Ok(port)
}

/// Reads `port` until stopped (`None`) or lost (why).
fn read(shared: &Shared, port: &mut dyn SerialPort) -> Option<String> {
    let mut pending = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            return None;
        }
        if shared.reset.swap(false, Ordering::Relaxed) {
            let _ = port.write_data_terminal_ready(false);
            let _ = port.write_request_to_send(true);
            std::thread::sleep(Duration::from_millis(100));
            let _ = port.write_request_to_send(false);
        }
        let outgoing = std::mem::take(&mut *shared.outgoing.lock().unwrap());
        if !outgoing.is_empty() {
            if let Err(e) = port.write_all(&outgoing).and_then(|()| port.flush()) {
                return Some(e.to_string());
            }
        }
        match port.read(&mut buffer) {
            Ok(0) => {}
            Ok(n) => {
                pending.extend_from_slice(&buffer[..n]);
                take_lines(&mut pending, |text| push(shared, text));
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Some(e.to_string()),
        }
    }
}

fn push(shared: &Shared, text: String) {
    let mut lines = shared.lines.lock().unwrap();
    if lines.len() == MAX_LINES {
        lines.pop_front();
    }
    lines.push_back(Line { at: shared.opened.elapsed(), text });
    shared.received.fetch_add(1, Ordering::Relaxed);
}

/// Hands each whole line of `pending` to `f`, without its line end, and
/// keeps the rest; a line too long to end is handed as it is.
fn take_lines(pending: &mut Vec<u8>, mut f: impl FnMut(String)) {
    while let Some(end) = pending.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = pending.drain(..=end).collect();
        f(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string());
    }
    if pending.len() > 4096 {
        f(String::from_utf8_lossy(pending).into_owned());
        pending.clear();
    }
}

/// Reads `port` for `duration`: restarts the board first when `reset`,
/// sends `send` once open. For the agent, which reads what the board
/// printed in one go.
pub fn read_for(port: &str, baud: u32, duration: Duration, reset: bool, send: Option<&str>) -> Result<Vec<Line>, String> {
    let monitor = Monitor::open(port, baud);
    let started = Instant::now();
    // Wait for the port to open, or to fail to.
    while monitor.status() == Status::Connecting && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Status::Disconnected(why) = monitor.status() {
        return Err(format!("cannot open {port}: {why}"));
    }
    if reset {
        monitor.reset();
    }
    if let Some(text) = send {
        monitor.send(text);
    }
    std::thread::sleep(duration);
    Ok(monitor.with_lines(|lines| lines.iter().cloned().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_lines_come_out_and_the_rest_waits() {
        let mut pending = b"amp\r\nmic: level 12\nhalf".to_vec();
        let mut lines = Vec::new();
        take_lines(&mut pending, |l| lines.push(l));
        assert_eq!(lines, ["amp", "mic: level 12"]);
        assert_eq!(pending, b"half");
        pending.extend_from_slice(b" a line\n");
        take_lines(&mut pending, |l| lines.push(l));
        assert_eq!(lines.last().unwrap(), "half a line");
    }

    #[test]
    fn a_line_without_end_comes_out_once_too_long() {
        let mut pending = vec![b'x'; 5000];
        let mut lines = Vec::new();
        take_lines(&mut pending, |l| lines.push(l));
        assert_eq!(lines.len(), 1);
        assert!(pending.is_empty());
    }

    #[test]
    fn a_missing_port_is_disconnected_and_says_why() {
        let monitor = Monitor::open("/dev/no-such-port", BAUD);
        let started = Instant::now();
        while monitor.status() == Status::Connecting && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(monitor.status(), Status::Disconnected(_)), "{:?}", monitor.status());
        let error = read_for("/dev/no-such-port", BAUD, Duration::from_millis(10), false, None).unwrap_err();
        assert!(error.contains("/dev/no-such-port"), "{error}");
    }

    #[test]
    fn the_oldest_lines_go_once_full() {
        let monitor = Monitor::open("/dev/no-such-port", BAUD);
        for i in 0..MAX_LINES + 10 {
            push(&monitor.shared, i.to_string());
        }
        monitor.with_lines(|lines| {
            assert_eq!(lines.len(), MAX_LINES);
            assert_eq!(lines.front().unwrap().text, "10");
        });
        assert_eq!(monitor.received(), (MAX_LINES + 10) as u64);
    }
}
