//! Talking to a board: the toolchain that builds its firmware (PlatformIO,
//! installed on demand as the Firmware module, or the user's own), building
//! and flashing a project with the compiler's errors at their line, and the
//! board's serial port, listed and read. The studio and the agent's tools
//! share it.

mod compile;
mod monitor;
mod ports;
mod process;
mod toolchain;

pub use compile::{Diagnostic, Outcome, Severity, Size, Target, UploadProblem, Usage, parse_diagnostic, parse_usage, run};
pub use monitor::{BAUD, Line, MAX_LINES, Monitor, Status, read_for};
pub use ports::{Port, list as ports, preferred};
pub use toolchain::{Toolchain, find, find_in, install, module_dir, size_of, uninstall};
