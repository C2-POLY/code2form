//! PlatformIO, which builds and flashes the firmware: the Firmware module's
//! own, installed on demand in Code2Form's folder with its boards'
//! toolchains, or one the user already has.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use crate::process::{Ended, stream};

/// PlatformIO's official installer, which sets PlatformIO up in a Python
/// environment of its own under `PLATFORMIO_CORE_DIR`.
const INSTALLER: &str = "https://raw.githubusercontent.com/platformio/platformio-core-installer/master/get-platformio.py";

/// Marks a module's folder while it installs: a folder that still has it
/// (the app quit during the installation) is no installation.
const INSTALLING: &str = ".installing";

/// A PlatformIO to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolchain {
    /// Its `pio` program.
    pub pio: PathBuf,
    /// The Firmware module's folder when it is the module's: its
    /// platforms and libraries go there too, and uninstalling it frees
    /// them. `None` for the user's own.
    pub module: Option<PathBuf>,
}

impl Toolchain {
    /// `pio` with `args`, set up to put what it downloads in the module's
    /// folder, and to print no progress bars.
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(&self.pio);
        command.args(args).env("PLATFORMIO_DISABLE_PROGRESSBAR", "true").env("PYTHONUNBUFFERED", "1");
        if let Some(dir) = &self.module {
            command.env("PLATFORMIO_CORE_DIR", dir);
        }
        command
    }

    /// Its version, as `pio --version` gives it: `6.1.18`.
    pub fn version(&self) -> Option<String> {
        let out = self.command(["--version"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let version = text.trim().rsplit(' ').next()?.to_string();
        (out.status.success() && !version.is_empty()).then_some(version)
    }
}

/// The Firmware module's folder, `firmware` beside the studio's settings,
/// or `CODE2FORM_FIRMWARE_DIR`; `None` without a home folder.
pub fn module_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODE2FORM_FIRMWARE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let dir = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support/Code2Form")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("APPDATA"))?).join("Code2Form")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| Some(home()?.join(".local/share")))?
            .join("code2form")
    };
    Some(dir.join("firmware"))
}

/// The PlatformIO to use: the Firmware module's, else the user's own.
/// `CODE2FORM_PLATFORMIO` names the user's `pio` instead of looking for
/// it, and set empty, keeps to the module's.
pub fn find() -> Option<Toolchain> {
    let module = module_dir();
    match std::env::var_os("CODE2FORM_PLATFORMIO") {
        Some(pio) => module
            .and_then(|dir| find_in(Some(&dir)).filter(|t| t.module.is_some()))
            .or_else(|| (!pio.is_empty()).then(|| Toolchain { pio: PathBuf::from(pio), module: None })),
        None => find_in(module.as_deref()),
    }
}

/// [`find`], with the module in `module`.
pub fn find_in(module: Option<&Path>) -> Option<Toolchain> {
    if let Some(dir) = module {
        let pio = pio_in(dir);
        if pio.is_file() && !dir.join(INSTALLING).exists() {
            return Some(Toolchain { pio, module: Some(dir.to_path_buf()) });
        }
    }
    // The user's own: PlatformIO's installer puts it in ~/.platformio; a
    // package manager, on the PATH. An app opened from the Finder has a
    // short PATH: Homebrew's folders are looked in too.
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = home.iter().map(|h| pio_in(&h.join(".platformio"))).collect();
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if cfg!(unix) {
        folders.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        folders.extend(home.iter().map(|h| h.join(".local/bin")));
    }
    for folder in folders {
        for name in ["pio", "platformio"] {
            candidates.push(folder.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }));
        }
    }
    candidates.into_iter().find(|p| p.is_file()).map(|pio| Toolchain { pio, module: None })
}

/// Where PlatformIO's installer puts `pio` under `core`.
fn pio_in(core: &Path) -> PathBuf {
    if cfg!(windows) { core.join("penv").join("Scripts").join("pio.exe") } else { core.join("penv").join("bin").join("pio") }
}

/// Installs PlatformIO in `dir`, the module's folder, handing each line
/// the installer prints to `on_line`; `cancel` stops it. A failed or
/// stopped installation leaves no folder behind, to try again from
/// scratch.
pub fn install(dir: &Path, cancel: &AtomicBool, mut on_line: impl FnMut(&str)) -> Result<Toolchain, String> {
    let result = (|| {
        // What a stopped installation left goes first.
        if dir.join(INSTALLING).exists() {
            uninstall(dir)?;
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        std::fs::write(dir.join(INSTALLING), "").map_err(|e| format!("cannot write in {}: {e}", dir.display()))?;
        on_line("Downloading PlatformIO's installer");
        let script = dir.join("get-platformio.py");
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(120))).build().into();
        let body = agent
            .get(INSTALLER)
            .call()
            .map_err(|e| format!("cannot download PlatformIO's installer: {e}"))?
            .body_mut()
            .read_to_vec()
            .map_err(|e| format!("cannot download PlatformIO's installer: {e}"))?;
        std::fs::write(&script, body).map_err(|e| format!("cannot write {}: {e}", script.display()))?;
        let (python, first) = python().ok_or(
            "PlatformIO needs Python 3, which this computer does not have: install it (python.org, or `xcode-select \
             --install` on a Mac), then try again",
        )?;
        let mut command = Command::new(python);
        command.args(first).arg(&script).env("PLATFORMIO_CORE_DIR", dir).env("PYTHONUNBUFFERED", "1");
        let mut last = Vec::new();
        let ended = stream(command, cancel, |line| {
            on_line(line);
            last.push(line.to_string());
        })?;
        match ended {
            Ended::Cancelled => Err("the installation was stopped".to_string()),
            Ended::Failure => {
                Err(format!("PlatformIO's installer failed:\n{}", last[last.len().saturating_sub(8)..].join("\n")))
            }
            Ended::Success => {
                let _ = std::fs::remove_file(&script);
                let _ = std::fs::remove_file(dir.join(INSTALLING));
                find_in(Some(dir))
                    .filter(|t| t.module.is_some())
                    .ok_or_else(|| "PlatformIO's installer finished, but no `pio` came out of it".to_string())
            }
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(dir);
    }
    result
}

/// A Python 3 to run the installer with, and the arguments that pick it.
fn python() -> Option<(&'static str, &'static [&'static str])> {
    let candidates: &[(&str, &[&str])] =
        if cfg!(windows) { &[("py", &["-3"]), ("python", &[]), ("python3", &[])] } else { &[("python3", &[]), ("python", &[])] };
    candidates.iter().copied().find(|(program, args)| {
        Command::new(program)
            .args(*args)
            .args(["-c", "import sys; sys.exit(0 if sys.version_info >= (3, 6) else 1)"])
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Removes the Firmware module: its folder and what PlatformIO put in it.
pub fn uninstall(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", dir.display())),
    }
}

/// The bytes the files under `dir` take, links not followed.
pub fn size_of(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => size_of(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("c2f-board-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_modules_own_platformio_comes_first_and_sets_its_folder() {
        let dir = folder("module");
        let pio = pio_in(&dir);
        std::fs::create_dir_all(pio.parent().unwrap()).unwrap();
        std::fs::write(&pio, "").unwrap();
        let found = find_in(Some(&dir)).unwrap();
        assert_eq!(found, Toolchain { pio, module: Some(dir.clone()) });
        let command = found.command(["run"]);
        let core = command.get_envs().find(|(k, _)| *k == "PLATFORMIO_CORE_DIR").and_then(|(_, v)| v);
        assert_eq!(core, Some(dir.as_os_str()));
        uninstall(&dir).unwrap();
        assert!(!dir.exists());
        // Twice is fine.
        uninstall(&dir).unwrap();
    }

    #[test]
    fn an_empty_module_folder_is_not_an_installation() {
        let dir = folder("empty");
        assert!(find_in(Some(&dir)).is_none_or(|t| t.module.is_none()));
        // Nor one left mid-installation.
        let pio = pio_in(&dir);
        std::fs::create_dir_all(pio.parent().unwrap()).unwrap();
        std::fs::write(&pio, "").unwrap();
        std::fs::write(dir.join(INSTALLING), "").unwrap();
        assert!(find_in(Some(&dir)).is_none_or(|t| t.module.is_none()));
        uninstall(&dir).unwrap();
    }

    /// Downloads PlatformIO: run it by hand, with a folder to install in,
    /// `C2F_INSTALL_TEST=/tmp/pio cargo test -p c2f-board -- --ignored`.
    #[test]
    #[ignore]
    fn platformio_installs_in_the_module_folder_and_leaves() {
        let dir = PathBuf::from(std::env::var_os("C2F_INSTALL_TEST").expect("C2F_INSTALL_TEST"));
        let _ = std::fs::remove_dir_all(&dir);
        let toolchain = install(&dir, &AtomicBool::new(false), |line| println!("{line}")).unwrap();
        assert_eq!(toolchain.module.as_deref(), Some(dir.as_path()));
        println!("version {:?}, {} bytes", toolchain.version(), size_of(&dir));
        assert!(toolchain.version().is_some());
        // Stopped at once, nothing is left.
        let stopped = dir.with_extension("stopped");
        let cancel = AtomicBool::new(true);
        assert!(install(&stopped, &cancel, |_| {}).is_err());
        assert!(!stopped.exists());
    }

    #[test]
    fn the_size_counts_the_files_below() {
        let dir = folder("size");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("one"), [0; 10]).unwrap();
        std::fs::write(dir.join("a/b/two"), [0; 32]).unwrap();
        assert_eq!(size_of(&dir), 42);
        uninstall(&dir).unwrap();
    }
}
