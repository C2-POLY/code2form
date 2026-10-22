//! The files behind "Open Recent", kept between launches in a small text
//! file, one path per line, most recent first.

use std::path::{Path, PathBuf};

/// How many files the menu lists.
pub const MAX: usize = 10;

#[derive(Debug, Default)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
    /// Where the list is kept; `None` keeps it for this run only.
    store: Option<PathBuf>,
}

impl RecentFiles {
    /// Reads the list from `store`; a missing or unreadable file gives an
    /// empty list.
    pub fn load(store: Option<PathBuf>) -> Self {
        let text = store.as_deref().and_then(|s| std::fs::read_to_string(s).ok()).unwrap_or_default();
        RecentFiles { paths: parse(&text), store }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Puts `path` first, moving it there if already listed.
    pub fn push(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.paths.retain(|p| *p != path);
        self.paths.insert(0, path);
        self.paths.truncate(MAX);
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Writes the list to its store.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(store) = &self.store else { return Ok(()) };
        if let Some(dir) = store.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(store, format(&self.paths))
    }
}

/// `CODE2FORM_RECENT_FILES` when set. Otherwise the per-user settings
/// folder, except in screenshot runs, which neither read nor change the
/// list.
pub fn store() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODE2FORM_RECENT_FILES") {
        return Some(PathBuf::from(path));
    }
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    Some(settings_dir()?.join("recent-files.txt"))
}

#[cfg(target_os = "macos")]
pub fn settings_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Code2Form"))
}

#[cfg(windows)]
pub fn settings_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Code2Form"))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn settings_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?;
    Some(config.join("code2form"))
}

fn parse(text: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let path = PathBuf::from(line);
        if path.is_absolute() && !paths.contains(&path) && paths.len() < MAX {
            paths.push(path);
        }
    }
    paths
}

/// Paths that a line cannot hold (a line break, or not UTF-8) are left out.
fn format(paths: &[PathBuf]) -> String {
    let mut text = String::new();
    for path in paths.iter().filter_map(|p| p.to_str()).filter(|p| !p.contains(['\n', '\r'])) {
        text.push_str(path);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join("code2form-recent-tests-missing").join(name)
    }

    #[test]
    fn most_recent_first_without_duplicates() {
        let mut recent = RecentFiles::default();
        recent.push(&path("a.scad"));
        recent.push(&path("b.scad"));
        recent.push(&path("a.scad"));
        assert_eq!(recent.paths(), [path("a.scad"), path("b.scad")]);
    }

    #[test]
    fn keeps_the_last_ten() {
        let mut recent = RecentFiles::default();
        for i in 0..12 {
            recent.push(&path(&format!("{i}.scad")));
        }
        assert_eq!(recent.paths().len(), MAX);
        assert_eq!(recent.paths()[0], path("11.scad"));
        assert_eq!(recent.paths()[MAX - 1], path("2.scad"));
    }

    #[test]
    fn removes_and_clears() {
        let mut recent = RecentFiles::default();
        recent.push(&path("a.scad"));
        recent.push(&path("b.scad"));
        recent.remove(&path("a.scad"));
        assert_eq!(recent.paths(), [path("b.scad")]);
        recent.clear();
        assert!(recent.is_empty());
    }

    #[test]
    fn stores_canonical_paths() {
        let dir = std::env::temp_dir().join(format!("code2form-recent-canonical-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("part.scad");
        std::fs::write(&file, "cube();").unwrap();
        let mut recent = RecentFiles::default();
        recent.push(&dir.join(".").join("part.scad"));
        recent.push(&file);
        assert_eq!(recent.paths(), [std::fs::canonicalize(&file).unwrap()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn survives_a_save_and_load() {
        let dir = std::env::temp_dir().join(format!("code2form-recent-store-{}", std::process::id()));
        let store = dir.join("settings").join("recent-files.txt");
        let mut recent = RecentFiles::load(Some(store.clone()));
        assert!(recent.is_empty(), "no file yet");
        recent.push(&path("a.scad"));
        recent.push(&path("with space.scad"));
        recent.push(&path("bad\nname.scad"));
        recent.save().unwrap();
        let loaded = RecentFiles::load(Some(store));
        assert_eq!(loaded.paths(), [path("with space.scad"), path("a.scad")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ignores_bad_lines() {
        let a = path("a.scad");
        let text = format!("{}\n\nrelative.scad\n{}\n", a.display(), a.display());
        assert_eq!(parse(&text), [a]);
    }
}
