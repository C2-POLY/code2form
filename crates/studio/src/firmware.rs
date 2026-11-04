//! A circuit's firmware in the studio: its project's files, edited in the
//! Firmware tab beside the circuit and saved as they are typed; the
//! Firmware module (PlatformIO), installed on demand; builds and uploads,
//! run in the background; and the serial monitor. The agent's board tools
//! reach the user through the [`Host`]: they ask before flashing the board,
//! and read the monitor's port when it is open.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use c2f_board::{Line, Monitor, Outcome, Port, Target, Toolchain};
use eframe::egui;

use crate::editor::Editor;
use crate::highlight::Syntax;

/// How long after the last key a file is saved.
const SAVE_DELAY: Duration = Duration::from_millis(800);
/// How often the project's files are looked at on disk, for the agent's
/// edits, and the serial ports listed, while on screen.
pub const DISK_POLL: Duration = Duration::from_secs(1);
const PORTS_POLL: Duration = Duration::from_secs(2);
/// How long the agent's upload waits for the user's answer.
const APPROVE_WAIT: Duration = Duration::from_secs(600);
/// The installer's lines shown, the latest.
const INSTALL_LINES: usize = 200;

/// The firmware's project of a circuit: its project's, or `firmware/`
/// beside it, once it has a platformio.ini.
pub fn project_dir(circuit: &Path) -> Option<PathBuf> {
    let dir = c2f_project::firmware_dir_of(circuit);
    dir.join("platformio.ini").is_file().then_some(dir)
}

/// A file of the project, in its editor.
pub struct File {
    pub path: PathBuf,
    /// Its path in the project, with `/`: `src/main.cpp`.
    pub name: String,
    pub text: String,
    /// The text as last read or written.
    pub saved: String,
    mtime: Option<SystemTime>,
    pub editor: Editor,
    /// `include/pins.h`, which the circuit writes, or a file that is not
    /// text.
    pub read_only: bool,
    save_at: Option<Instant>,
    /// It changed on disk under edits of the user's: they are saved over
    /// it only when asked (Save, Build).
    pub conflict: bool,
}

impl File {
    fn open(path: PathBuf, name: String, tab: u64) -> File {
        // A file that is not UTF-8 text shows empty and stays as it is.
        let read = std::fs::read_to_string(&path);
        let unreadable = read.is_err();
        let text = read.unwrap_or_default();
        let mut editor = Editor::with_id(egui::Id::new(("c2f-firmware", tab, &name)));
        editor.set_syntax(if name.ends_with(".ini") { Syntax::Circuit } else { Syntax::Cpp });
        File {
            mtime: modified(&path),
            read_only: unreadable || name == c2f_circuit::firmware::PINS_H,
            path,
            name,
            saved: text.clone(),
            text,
            editor,
            save_at: None,
            conflict: false,
        }
    }

    /// Whether it is C or C++ code, where a pin's constant can be used.
    pub fn is_code(&self) -> bool {
        [".cpp", ".c", ".h", ".hpp", ".ino"].iter().any(|e| self.name.ends_with(e))
    }

    /// The file name alone: `main.cpp`.
    pub fn short_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }

    /// Typed in: saved once the typing pauses.
    pub fn edited(&mut self) {
        self.save_at = Some(Instant::now() + SAVE_DELAY);
    }

    /// Writes the edits; over a change made on disk since it was read only
    /// when `force`d.
    fn save(&mut self, force: bool) -> Result<(), String> {
        self.save_at = None;
        if self.text == self.saved || self.read_only {
            return Ok(());
        }
        if !force && modified(&self.path) != self.mtime {
            if self.conflict {
                return Ok(());
            }
            self.conflict = true;
            return Err(format!(
                "{} changed on disk since it was opened: your edits stay in the editor, and Save writes them over it",
                self.path.display()
            ));
        }
        self.conflict = false;
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&self.path, &self.text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.text.clone();
        self.mtime = modified(&self.path);
        Ok(())
    }
}

/// A circuit's firmware project, open in its tab.
pub struct Project {
    pub dir: PathBuf,
    pub files: Vec<File>,
    /// The file in the editor.
    pub current: usize,
    /// How the last build or upload went, for its status and the
    /// problems it lists.
    pub last: Option<(JobKind, Outcome)>,
    tab: u64,
    polled: Instant,
}

impl Project {
    pub fn open(dir: PathBuf, tab: u64) -> Project {
        let mut project = Project { dir, files: Vec::new(), current: 0, last: None, tab, polled: Instant::now() };
        project.scan();
        project
    }

    /// Takes in the files added since, and leaves out those removed: the
    /// sources and headers, then platformio.ini.
    fn scan(&mut self) {
        let mut found = Vec::new();
        // A library's own files are not the project's to edit.
        for folder in ["src", "include"] {
            walk(&self.dir.join(folder), &mut found);
        }
        found.retain(|p| p.extension().is_some_and(|e| ["cpp", "c", "h", "hpp", "ino"].iter().any(|x| e == *x)));
        found.sort_by_key(|p| (p.file_name().is_none_or(|n| n != "main.cpp"), !p.starts_with(self.dir.join("src")), p.clone()));
        found.push(self.dir.join(c2f_circuit::firmware::PLATFORMIO_INI));
        let current = self.files.get(self.current).map(|f| f.path.clone());
        let mut files: Vec<File> = Vec::new();
        // A file gone from disk stays while the user's edits to it wait.
        for file in &self.files {
            if file.text != file.saved && !found.contains(&file.path) {
                found.push(file.path.clone());
            }
        }
        for path in found {
            let name = path.strip_prefix(&self.dir).unwrap_or(&path).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            match self.files.iter().position(|f| f.path == path) {
                Some(i) => files.push(self.files.swap_remove(i)),
                None => files.push(File::open(path, name, self.tab)),
            }
        }
        self.files = files;
        self.current = current.and_then(|c| self.files.iter().position(|f| f.path == c)).unwrap_or(0);
    }

    /// Saves the files whose typing paused; the errors, if any.
    pub fn save_due(&mut self) -> Vec<String> {
        let now = Instant::now();
        self.files.iter_mut().filter(|f| f.save_at.is_some_and(|at| at <= now)).filter_map(|f| f.save(false).err()).collect()
    }

    /// Saves every file edited: before a build, which reads them from disk.
    pub fn save_all(&mut self) -> Vec<String> {
        self.files.iter_mut().filter_map(|f| f.save(true).err()).collect()
    }

    /// The next time a file is to be saved.
    pub fn next_save(&self) -> Option<Instant> {
        self.files.iter().filter_map(|f| f.save_at).min()
    }

    /// Reads again the files changed on disk (by the agent, by another
    /// editor) that have no edits of the user's waiting, and the files
    /// added or removed, once a second.
    pub fn poll_disk(&mut self) {
        if self.polled.elapsed() < DISK_POLL {
            return;
        }
        self.polled = Instant::now();
        self.scan();
        for file in &mut self.files {
            let mtime = modified(&file.path);
            if mtime != file.mtime && file.text == file.saved {
                file.reload_now(mtime);
            }
        }
    }

    /// Reads `name` again now: `include/pins.h` once the circuit wrote it.
    pub fn reload(&mut self, name: &str) {
        if let Some(file) = self.files.iter_mut().find(|f| f.name == name) {
            let mtime = modified(&file.path);
            file.reload_now(mtime);
        }
    }

    pub fn file(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|f| f.name == name)
    }

    /// The file a compiler's message names: by its path in the project,
    /// or its full path inside it.
    pub fn index_of(&self, file: &Path) -> Option<usize> {
        let relative = file.strip_prefix(&self.dir).unwrap_or(file);
        let name: String = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        self.files.iter().position(|f| f.name == name)
    }
}

impl File {
    fn reload_now(&mut self, mtime: Option<SystemTime>) {
        if let Ok(text) = std::fs::read_to_string(&self.path) {
            self.text = text.clone();
            self.saved = text;
        }
        self.mtime = mtime;
    }
}

/// The byte offset where `line` (from 0) starts in `text`.
pub fn line_offset(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n').nth(line - 1).map_or(text.len(), |(i, _)| i + 1)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A build or an upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Build,
    Upload,
}

/// A build or an upload under way.
pub struct Job {
    pub kind: JobKind,
    /// The tab that started it, whose console takes its lines.
    pub tab: u64,
    pub project: PathBuf,
    pub started: Instant,
    /// The port to open the monitor on again once it is over.
    reopen: Option<String>,
    events: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
}

enum JobEvent {
    Line(String),
    Done(Outcome),
}

/// The Firmware module being installed.
pub struct Install {
    pub lines: Arc<Mutex<Vec<String>>>,
    cancel: Arc<AtomicBool>,
    done: Receiver<Result<Toolchain, String>>,
}

impl Install {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// What PlatformIO there is, as last looked for: in the background, as
/// `pio --version` takes a moment.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub toolchain: Option<Toolchain>,
    pub version: Option<String>,
    /// The bytes the module's folder takes, when it is the module's.
    pub size: Option<u64>,
}

/// What the agent's tools ask of the studio.
enum HostRequest {
    Approve { project: PathBuf, port: String, answer: Sender<bool> },
    Done { port: String },
}

/// An upload of the agent's, waiting for the user's answer.
pub struct Asking {
    pub project: PathBuf,
    pub port: String,
    answer: Sender<bool>,
}

/// What happened in the background since the last frame, for the app to
/// pass on to its tabs.
pub enum Event {
    /// A line a build or an upload printed, for the tab that started it.
    Line { tab: u64, line: String },
    /// A build or an upload ended.
    Done { tab: u64, kind: JobKind, project: PathBuf, outcome: Outcome },
    /// The Firmware module is installed, or why not.
    Installed(Result<(), String>),
    /// The Firmware module is removed, or why not.
    Uninstalled(Result<(), String>),
}

/// The board, for the whole studio: the Firmware module, the build or
/// upload under way, the serial ports and the monitor.
pub struct Board {
    pub found: Found,
    /// Looking for PlatformIO.
    finding: Option<Receiver<Found>>,
    pub install: Option<Install>,
    /// Why the last installation failed.
    pub install_error: Option<String>,
    pub job: Option<Job>,
    /// The agent's upload under way, once the user agreed: its port, and
    /// the port to open the monitor on again once it is over.
    pub agent_upload: Option<(String, Option<String>)>,
    /// Removing the Firmware module.
    pub removing: Option<Receiver<Result<(), String>>>,
    pub ports: Vec<Port>,
    ports_at: Option<Instant>,
    /// Listing the ports, which can take a moment.
    listing: Option<Receiver<Vec<Port>>>,
    /// The port picked for the uploads and the monitor; the board Code2Form
    /// knows, by default.
    pub port: Option<String>,
    /// The monitor, shared with the agent's tools that read the port.
    monitor: Arc<Mutex<Option<Arc<Monitor>>>>,
    pub baud: u32,
    /// The line typed to send.
    pub serial_input: String,
    /// The lines shown while paused.
    pub paused: Option<Vec<Line>>,
    requests: Receiver<HostRequest>,
    pub asking: Option<Asking>,
    /// A build or an upload asked for without the module: the dialog that
    /// offers to install it shows.
    pub offer_install: bool,
    /// To show what the background brings.
    ctx: egui::Context,
}

impl Board {
    /// The studio's board, and the host of the agent's board tools.
    pub fn new(ctx: &egui::Context) -> Board {
        let (send, requests) = mpsc::channel();
        let monitor = Arc::new(Mutex::new(None));
        c2f_agent::board::set_host(Box::new(Host { requests: Mutex::new(send), monitor: monitor.clone(), ctx: ctx.clone() }));
        let mut board = Board {
            found: Found::default(),
            finding: None,
            install: None,
            install_error: None,
            job: None,
            agent_upload: None,
            removing: None,
            ports: Vec::new(),
            ports_at: None,
            listing: None,
            port: None,
            monitor,
            baud: c2f_board::BAUD,
            serial_input: String::new(),
            paused: None,
            requests,
            asking: None,
            offer_install: false,
            ctx: ctx.clone(),
        };
        board.find();
        board
    }

    /// Looks for PlatformIO again: at once, which only looks at files, then
    /// its version and size in the background.
    pub fn find(&mut self) {
        let toolchain = c2f_board::find();
        self.found = Found { toolchain: toolchain.clone(), version: None, size: None };
        let (send, receive) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let version = toolchain.as_ref().and_then(Toolchain::version);
            let size = toolchain.as_ref().and_then(|t| t.module.as_deref()).map(c2f_board::size_of);
            let _ = send.send(Found { toolchain, version, size });
            ctx.request_repaint();
        });
        self.finding = Some(receive);
    }

    /// Installs the Firmware module, in the background.
    pub fn start_install(&mut self) {
        let ctx = &self.ctx;
        let Some(dir) = c2f_board::module_dir() else {
            self.install_error = Some("no home folder to install in".into());
            return;
        };
        let lines = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, done) = mpsc::channel();
        {
            let (lines, cancel, ctx) = (lines.clone(), cancel.clone(), ctx.clone());
            std::thread::spawn(move || {
                let result = c2f_board::install(&dir, &cancel, |line| {
                    let mut lines = lines.lock().unwrap();
                    if lines.len() == INSTALL_LINES {
                        lines.remove(0);
                    }
                    lines.push(line.to_string());
                    ctx.request_repaint();
                });
                let _ = send.send(result);
                ctx.request_repaint();
            });
        }
        self.install_error = None;
        self.install = Some(Install { lines, cancel, done });
    }

    /// Removes the Firmware module, in the background: gigabytes of files.
    pub fn uninstall(&mut self) {
        let Some(dir) = self.found.toolchain.as_ref().and_then(|t| t.module.clone()) else { return };
        if self.occupied() || self.removing.is_some() {
            return;
        }
        let (send, done) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(c2f_board::uninstall(&dir));
            ctx.request_repaint();
        });
        self.removing = Some(done);
    }

    /// Builds or flashes `project`, in the background, for tab `tab`. The
    /// monitor lets go of the port while the board is flashed.
    pub fn start(&mut self, kind: JobKind, tab: u64, project: &Path) -> Result<(), String> {
        let toolchain = self.found.toolchain.clone().ok_or("the Firmware module is not installed")?;
        if self.occupied() || self.removing.is_some() {
            return Err("a build or an upload is under way".into());
        }
        let port = self.port.clone().or_else(|| c2f_board::preferred(&self.ports).map(|p| p.name.clone()));
        let mut reopen = None;
        if kind == JobKind::Upload {
            if port.is_none() {
                return Err(c2f_board::UploadProblem::NoBoard.describe().into());
            }
            if let Some(open) = self.monitor().filter(|m| Some(m.port()) == port.as_deref()) {
                reopen = Some(open.port().to_string());
                self.close_monitor();
            }
        }
        let target = match kind {
            JobKind::Build => Target::Build,
            JobKind::Upload => Target::Upload { port },
        };
        let (send, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let (dir, cancel, ctx) = (project.to_path_buf(), cancel.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let outcome = c2f_board::run(&toolchain, &dir, &target, &cancel, |line| {
                    let _ = send.send(JobEvent::Line(line.to_string()));
                    ctx.request_repaint();
                });
                let _ = send.send(JobEvent::Done(outcome));
                ctx.request_repaint();
            });
        }
        self.job = Some(Job { kind, tab, project: project.to_path_buf(), started: Instant::now(), reopen, events, cancel });
        Ok(())
    }

    pub fn cancel_job(&self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// What happened in the background since the last frame.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some(found) = self.finding.as_ref().and_then(|f| f.try_recv().ok()) {
            self.found = found;
            self.finding = None;
        }
        if let Some(result) = self.install.as_ref().and_then(|i| i.done.try_recv().ok()) {
            self.install = None;
            match result {
                Ok(_) => {
                    events.push(Event::Installed(Ok(())));
                    self.find();
                }
                Err(error) => {
                    self.install_error = Some(error.clone());
                    events.push(Event::Installed(Err(error)));
                }
            }
        }
        if let Some(result) = self.removing.as_ref().and_then(|r| r.try_recv().ok()) {
            self.removing = None;
            events.push(Event::Uninstalled(result));
            self.find();
        }
        if let Some(ports) = self.listing.as_ref().and_then(|l| l.try_recv().ok()) {
            self.listing = None;
            self.take_ports(ports);
        }
        if let Some(job) = &self.job {
            let mut done = None;
            while let Ok(event) = job.events.try_recv() {
                match event {
                    JobEvent::Line(line) => events.push(Event::Line { tab: job.tab, line }),
                    JobEvent::Done(outcome) => done = Some(outcome),
                }
            }
            if let Some(outcome) = done {
                let job = self.job.take().expect("a job");
                events.push(Event::Done { tab: job.tab, kind: job.kind, project: job.project, outcome });
                if let Some(port) = job.reopen {
                    self.open_monitor(&port);
                }
            }
        }
        while let Ok(request) = self.requests.try_recv() {
            match request {
                HostRequest::Approve { project, port, answer } => {
                    if self.asking.is_some() || self.occupied() || self.removing.is_some() {
                        // One at a time: a second waits for nothing.
                        let _ = answer.send(false);
                    } else {
                        self.asking = Some(Asking { project, port, answer });
                    }
                }
                HostRequest::Done { port } => {
                    if self.agent_upload.as_ref().is_some_and(|(p, _)| *p == port) {
                        if let Some((_, Some(reopen))) = self.agent_upload.take() {
                            self.open_monitor(&reopen);
                        }
                    }
                }
            }
        }
        events
    }

    /// The user's answer to the agent's upload: the monitor lets go of the
    /// port, when it is the one, until the upload is over.
    pub fn answer(&mut self, yes: bool) {
        let Some(asking) = self.asking.take() else { return };
        let open = self.monitor().filter(|m| m.port() == asking.port).map(|m| m.port().to_string());
        // The port is let go first: the upload starts once answered.
        if yes {
            self.close_monitor();
        }
        if asking.answer.send(yes).is_err() {
            // The agent gave up waiting: nothing uploads.
            if let Some(port) = open.filter(|_| yes) {
                self.open_monitor(&port);
            }
            return;
        }
        if yes {
            self.agent_upload = Some((asking.port, open));
        }
    }

    /// Whether a build or an upload holds the project or the board: the
    /// studio's or the agent's.
    pub fn occupied(&self) -> bool {
        self.job.is_some() || self.agent_upload.is_some()
    }

    /// Lists the serial ports again, every two seconds, or now; picks the
    /// board plugged in when none is picked.
    pub fn refresh_ports(&mut self, now: bool) {
        if !now && self.ports_at.is_some_and(|at| at.elapsed() < PORTS_POLL) {
            return;
        }
        self.ports_at = Some(Instant::now());
        if self.listing.is_some() {
            return;
        }
        let (send, listing) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(c2f_board::ports());
            ctx.request_repaint();
        });
        self.listing = Some(listing);
    }

    fn take_ports(&mut self, ports: Vec<Port>) {
        self.ports = ports;
        let gone = self.port.as_ref().is_some_and(|p| !self.ports.iter().any(|q| &q.name == p));
        if self.port.is_none() || (gone && self.monitor().is_none()) {
            self.port = c2f_board::preferred(&self.ports).or(self.ports.first()).map(|p| p.name.clone());
        }
    }

    pub fn monitor(&self) -> Option<Arc<Monitor>> {
        self.monitor.lock().unwrap().clone()
    }

    pub fn open_monitor(&mut self, port: &str) {
        self.paused = None;
        *self.monitor.lock().unwrap() = Some(Arc::new(Monitor::open(port, self.baud)));
    }

    pub fn close_monitor(&mut self) {
        // Taken out of the lock first: the agent's read may hold another
        // handle, and the port closes once both let go.
        // Closed for every handle: the port is let go even while the agent
        // reads it.
        let monitor = self.monitor.lock().unwrap().take();
        if let Some(monitor) = monitor {
            monitor.close();
        }
    }

    /// Whether anything runs in the background that needs frames.
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.install.is_some() || self.finding.is_some() || self.removing.is_some() || self.agent_upload.is_some()
    }

    pub fn installed(&self) -> bool {
        self.found.toolchain.is_some()
    }
}

/// The agent's board tools' way to the user, from their thread.
struct Host {
    requests: Mutex<Sender<HostRequest>>,
    monitor: Arc<Mutex<Option<Arc<Monitor>>>>,
    ctx: egui::Context,
}

impl c2f_agent::board::BoardHost for Host {
    fn approve_upload(&self, project: &Path, port: &str) -> bool {
        let (answer, answered) = mpsc::channel();
        let request = HostRequest::Approve { project: project.to_path_buf(), port: port.to_string(), answer };
        if self.requests.lock().unwrap().send(request).is_err() {
            return false;
        }
        self.ctx.request_repaint();
        answered.recv_timeout(APPROVE_WAIT).unwrap_or(false)
    }

    fn upload_done(&self, port: &str, _success: bool) {
        let _ = self.requests.lock().unwrap().send(HostRequest::Done { port: port.to_string() });
        self.ctx.request_repaint();
    }

    fn read_serial(&self, port: &str, duration: Duration, reset: bool, send: Option<&str>) -> Option<Vec<Line>> {
        let monitor = self.monitor.lock().unwrap().clone().filter(|m| m.port() == port)?;
        let before = monitor.received();
        if reset {
            monitor.reset();
        }
        if let Some(text) = send {
            monitor.send(text);
        }
        std::thread::sleep(duration);
        let new = (monitor.received() - before) as usize;
        Some(monitor.with_lines(|lines| lines.iter().skip(lines.len().saturating_sub(new)).cloned().collect()))
    }
}

/// `bytes` for a person: `328 KB`, `3.2 MB`.
pub fn format_bytes(bytes: u64) -> String {
    let (kb, mb, gb) = (crate::i18n::tr("KB", "Ko"), crate::i18n::tr("MB", "Mo"), crate::i18n::tr("GB", "Go"));
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} {gb}", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} {mb}", b / 1e6)
    } else {
        format!("{:.0} {kb}", (b / 1e3).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("c2f-studio-firmware-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("firmware/src")).unwrap();
        std::fs::create_dir_all(dir.join("firmware/include")).unwrap();
        std::fs::write(dir.join("firmware/platformio.ini"), "[env:x]\n").unwrap();
        std::fs::write(dir.join("firmware/src/main.cpp"), "void setup() {}\n").unwrap();
        std::fs::write(dir.join("firmware/src/motor.h"), "#pragma once\n").unwrap();
        std::fs::write(dir.join("firmware/include/pins.h"), "#define A_B 1\n").unwrap();
        dir
    }

    #[test]
    fn a_project_lists_main_first_and_its_ini_last() {
        let dir = project_folder("list");
        assert_eq!(project_dir(&dir.join("lamp.c2fc")), Some(dir.join("firmware")));
        let project = Project::open(dir.join("firmware"), 1);
        let names: Vec<&str> = project.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["src/main.cpp", "src/motor.h", "include/pins.h", "platformio.ini"]);
        assert!(project.files[2].read_only && !project.files[0].read_only);
        assert_eq!(project.index_of(Path::new("src/motor.h")), Some(1));
        assert_eq!(project.index_of(&dir.join("firmware/include/pins.h")), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edit_is_not_saved_over_a_change_on_disk_unless_asked() {
        let dir = project_folder("conflict");
        let mut project = Project::open(dir.join("firmware"), 1);
        project.files[0].text = "// mine\n".into();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("firmware/src/main.cpp"), "// the agent's\n").unwrap();
        project.files[0].save_at = Some(Instant::now());
        let errors = project.save_due();
        assert_eq!(errors.len(), 1, "said once");
        assert_eq!(std::fs::read_to_string(dir.join("firmware/src/main.cpp")).unwrap(), "// the agent's\n");
        // The edits stay, even with the file gone.
        std::fs::remove_file(dir.join("firmware/src/main.cpp")).unwrap();
        project.polled = Instant::now() - DISK_POLL;
        project.poll_disk();
        assert_eq!(project.file("src/main.cpp").map(|f| f.text.as_str()), Some("// mine\n"));
        assert!(project.save_all().is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("firmware/src/main.cpp")).unwrap(), "// mine\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edit_is_saved_and_a_change_on_disk_is_read() {
        let dir = project_folder("edit");
        let mut project = Project::open(dir.join("firmware"), 1);
        project.files[0].text.push_str("void loop() {}\n");
        project.files[0].edited();
        assert!(project.next_save().is_some());
        assert!(project.save_all().is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("firmware/src/main.cpp")).unwrap(), "void setup() {}\nvoid loop() {}\n");
        // The agent writes the file, and adds one.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("firmware/src/main.cpp"), "// the agent's\n").unwrap();
        std::fs::write(dir.join("firmware/src/led.cpp"), "").unwrap();
        project.polled = Instant::now() - DISK_POLL;
        project.poll_disk();
        assert_eq!(project.files[0].text, "// the agent's\n");
        assert!(project.files.iter().any(|f| f.name == "src/led.cpp"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_agents_upload_holds_the_board_from_the_yes_to_its_end() {
        let mut board = Board::new(&egui::Context::default());
        let (send, requests) = mpsc::channel();
        board.requests = requests;
        let (answer, answered) = mpsc::channel();
        send.send(HostRequest::Approve { project: "/p".into(), port: "/dev/x".into(), answer }).unwrap();
        board.poll();
        assert!(board.asking.is_some() && !board.occupied());
        board.answer(true);
        assert!(answered.recv().unwrap());
        assert!(board.occupied(), "nothing else builds or opens the port meanwhile");
        // A second one waits for nothing.
        let (answer, refused) = mpsc::channel();
        send.send(HostRequest::Approve { project: "/p".into(), port: "/dev/x".into(), answer }).unwrap();
        board.poll();
        assert!(!refused.recv().unwrap());
        send.send(HostRequest::Done { port: "/dev/x".into() }).unwrap();
        board.poll();
        assert!(!board.occupied());
        // Declined, or answered once the agent gave up: nothing holds it.
        let (answer, answered) = mpsc::channel();
        send.send(HostRequest::Approve { project: "/p".into(), port: "/dev/x".into(), answer }).unwrap();
        board.poll();
        drop(answered);
        board.answer(true);
        assert!(!board.occupied());
    }

    #[test]
    fn a_line_starts_after_its_newline() {
        let text = "a\nbc\n\nd";
        assert_eq!([0, 1, 2, 3, 9].map(|l| line_offset(text, l)), [0, 2, 5, 6, 7]);
    }
}
