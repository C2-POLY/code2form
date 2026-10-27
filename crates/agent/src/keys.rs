//! The Anthropic API key, kept where the system keeps secrets: the
//! Keychain on macOS, the Credential Manager on Windows. On Linux, where
//! the secret service would need libdbus to build, it is a file in the
//! configuration directory that only its owner can read.
//!
//! Nothing here prints or logs the key.

/// The service and account the key is saved under.
const SERVICE: &str = "Code2Form";
const ACCOUNT: &str = "anthropic-api-key";

/// The saved key, or else the one in `ANTHROPIC_API_KEY`; none when
/// neither is set.
pub fn load_api_key() -> Option<String> {
    stored().or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()).and_then(usable)
}

/// Saves `key`, in place of the one saved before; the error says why it
/// could not be.
pub fn save_api_key(key: &str) -> Result<(), String> {
    let key = usable(key.to_string()).ok_or("the API key is empty")?;
    store(&key)
}

/// Forgets the saved key; the error says why it could not. No key saved
/// is no error.
pub fn delete_api_key() -> Result<(), String> {
    forget()
}

/// The key without the blanks pasted around it; none when nothing is left.
fn usable(key: String) -> Option<String> {
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|err| format!("cannot open the system's secure store: {err}"))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn stored() -> Option<String> {
    entry().ok()?.get_password().ok()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn store(key: &str) -> Result<(), String> {
    entry()?.set_password(key).map_err(|err| format!("cannot save the API key in the system's secure store: {err}"))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn forget() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(format!("cannot remove the API key from the system's secure store: {err}")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod file {
    use std::path::PathBuf;

    use super::{ACCOUNT, SERVICE};

    /// `$XDG_CONFIG_HOME/Code2Form/anthropic-api-key`, or under
    /// `~/.config` when that is not set.
    pub fn path() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(config.join(SERVICE).join(ACCOUNT))
    }

    pub fn read(path: &std::path::Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    /// Writes `key` to `path`, readable and writable by its owner alone.
    pub fn write(path: &std::path::Path, key: &str) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let failed = |err: std::io::Error| format!("cannot save the API key in {}: {err}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).map_err(failed)?;
        // A file made before with wider rights keeps them otherwise.
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(failed)?;
        file.write_all(key.as_bytes()).map_err(failed)
    }

    pub fn remove(path: &std::path::Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("cannot remove the API key at {}: {err}", path.display()))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn stored() -> Option<String> {
    file::read(&file::path()?)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn store(key: &str) -> Result<(), String> {
    file::write(&file::path().ok_or("no configuration directory: HOME is not set")?, key)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn forget() -> Result<(), String> {
    file::path().map_or(Ok(()), |path| file::remove(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_trimmed_and_an_empty_one_is_none() {
        assert_eq!(usable("  sk-ant-test\n".into()).as_deref(), Some("sk-ant-test"));
        assert_eq!(usable(" \n".into()), None);
        assert!(save_api_key("   ").is_err());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn the_file_is_its_owner_s_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("c2f-keys-{}", std::process::id()));
        let path = dir.join(SERVICE).join(ACCOUNT);
        file::write(&path, "sk-ant-test").unwrap();
        assert_eq!(file::read(&path).as_deref(), Some("sk-ant-test"));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        file::remove(&path).unwrap();
        assert_eq!(file::read(&path), None);
        file::remove(&path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
