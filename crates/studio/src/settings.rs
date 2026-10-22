//! The user's settings, kept between launches in `settings.txt` in the
//! settings folder: one `key = value` per line, a key repeated for each
//! folder of a list. Unknown keys and bad values are ignored, so that an
//! older or newer app reads what it understands.

use std::path::{Path, PathBuf};

use crate::app_icon::Tint;

/// The range and default of the code's size, in points.
pub const CODE_SIZES: std::ops::RangeInclusive<f32> = 10.0..=24.0;
const CODE_SIZE: f32 = 13.5;
/// The range and default of the pause in typing before a render, in ms.
pub const RENDER_DELAYS: std::ops::RangeInclusive<u32> = 100..=3000;
const RENDER_DELAY: u32 = 400;
/// The range and default of the layer view's layer height, in mm.
pub const LAYER_HEIGHTS: std::ops::RangeInclusive<f32> = 0.05..=1.0;
const LAYER_HEIGHT: f32 = 0.2;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub icon: Tint,
    /// Searched for `include` and `use` before the default folders.
    pub library_folders: Vec<PathBuf>,
    /// Searched for `text()`'s fonts before the bundled and system fonts.
    pub font_folders: Vec<PathBuf>,
    /// Renders once typing pauses for `render_delay` ms, and as soon as a
    /// customizer value changes; otherwise only on Render (F5).
    pub render_while_typing: bool,
    pub render_delay: u32,
    /// The editor's and the console's text size, in points.
    pub code_size: f32,
    /// The view as last left.
    pub show_grid: bool,
    pub show_axes: bool,
    pub layer_height: f32,
    /// The code editor shows; off, the code-free mode leaves the view,
    /// the tree and the panels around it.
    pub show_code: bool,
    /// The tree of the program shows, between the editor and the view.
    pub show_tree: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            icon: Tint::default(),
            library_folders: Vec::new(),
            font_folders: Vec::new(),
            render_while_typing: true,
            render_delay: RENDER_DELAY,
            code_size: CODE_SIZE,
            show_grid: true,
            show_axes: true,
            layer_height: LAYER_HEIGHT,
            show_code: true,
            show_tree: true,
        }
    }
}

impl Settings {
    /// Reads the settings from `store`; a missing or unreadable file gives
    /// the defaults. The icon picked before this file existed is kept.
    pub fn load(store: Option<&Path>) -> Self {
        let Some(store) = store else { return Settings::default() };
        match std::fs::read_to_string(store) {
            Ok(text) => Settings::parse(&text),
            Err(_) => {
                let old = store.with_file_name("app-icon.txt");
                let icon = std::fs::read_to_string(old).ok().and_then(|id| Tint::from_id(&id)).unwrap_or_default();
                Settings { icon, ..Settings::default() }
            }
        }
    }

    pub fn save(&self, store: Option<&Path>) -> std::io::Result<()> {
        let Some(store) = store else { return Ok(()) };
        if let Some(dir) = store.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(store, self.format())
    }

    fn parse(text: &str) -> Self {
        let mut settings = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "icon" => settings.icon = Tint::from_id(value).unwrap_or(settings.icon),
                "library" => push_folder(&mut settings.library_folders, value),
                "font" => push_folder(&mut settings.font_folders, value),
                "render-while-typing" => set(&mut settings.render_while_typing, value.parse().ok()),
                "render-delay" => {
                    set(&mut settings.render_delay, value.parse::<u32>().ok().map(|v| clamp(v, &RENDER_DELAYS)))
                }
                "code-size" => set(&mut settings.code_size, finite(value).map(|v| clamp(v, &CODE_SIZES))),
                "grid" => set(&mut settings.show_grid, value.parse().ok()),
                "axes" => set(&mut settings.show_axes, value.parse().ok()),
                "code" => set(&mut settings.show_code, value.parse().ok()),
                "tree" => set(&mut settings.show_tree, value.parse().ok()),
                "layer-height" => set(&mut settings.layer_height, finite(value).map(|v| clamp(v, &LAYER_HEIGHTS))),
                _ => {}
            }
        }
        settings
    }

    fn format(&self) -> String {
        let mut text = format!("icon = {}\n", self.icon.id());
        for (key, folders) in [("library", &self.library_folders), ("font", &self.font_folders)] {
            for folder in folders.iter().filter_map(|f| f.to_str()).filter(|f| !f.contains(['\n', '\r'])) {
                text.push_str(&format!("{key} = {folder}\n"));
            }
        }
        text.push_str(&format!("render-while-typing = {}\n", self.render_while_typing));
        text.push_str(&format!("render-delay = {}\n", self.render_delay));
        text.push_str(&format!("code-size = {}\n", self.code_size));
        text.push_str(&format!("grid = {}\n", self.show_grid));
        text.push_str(&format!("axes = {}\n", self.show_axes));
        text.push_str(&format!("layer-height = {}\n", self.layer_height));
        text.push_str(&format!("code = {}\n", self.show_code));
        text.push_str(&format!("tree = {}\n", self.show_tree));
        text
    }
}

/// The per-user settings folder, except in screenshot runs, which neither
/// read nor change the settings.
pub fn store() -> Option<PathBuf> {
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    Some(crate::recent::settings_dir()?.join("settings.txt"))
}

fn set<T>(field: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *field = value;
    }
}

fn finite(value: &str) -> Option<f32> {
    value.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn clamp<T: PartialOrd + Copy>(value: T, range: &std::ops::RangeInclusive<T>) -> T {
    if value < *range.start() {
        *range.start()
    } else if value > *range.end() {
        *range.end()
    } else {
        value
    }
}

/// Adds an absolute folder once.
fn push_folder(folders: &mut Vec<PathBuf>, value: &str) {
    let folder = PathBuf::from(value);
    if folder.is_absolute() && !folders.contains(&folder) {
        folders.push(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("code2form-settings-{name}-{}", std::process::id()))
    }

    /// An absolute path on every system (`/opt` has no drive on Windows).
    fn absolute(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn survive_a_save_and_load() {
        let dir = temp("round-trip");
        let store = dir.join("settings").join("settings.txt");
        assert_eq!(Settings::load(Some(&store)), Settings::default(), "no file yet");
        let settings = Settings {
            icon: Tint::Teal,
            library_folders: vec![absolute("libs"), absolute("with space")],
            font_folders: vec![absolute("fonts")],
            render_while_typing: false,
            render_delay: 900,
            code_size: 16.0,
            show_grid: false,
            show_axes: true,
            layer_height: 0.12,
            show_code: false,
            show_tree: false,
        };
        settings.save(Some(&store)).unwrap();
        assert_eq!(Settings::load(Some(&store)), settings);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keep_what_they_understand() {
        let a = absolute("a");
        let text = format!(
            "icon = plaid\ncode-size = 99\nrender-delay = 5\nlayer-height = NaN\nlibrary = relative\n\
             library = {a}\nlibrary = {a}\ngrid = maybe\nfuture-key = 1\nno equals sign\n",
            a = a.display()
        );
        let settings = Settings::parse(&text);
        assert_eq!(settings.icon, Tint::Graphite);
        assert_eq!(settings.code_size, 24.0, "clamped");
        assert_eq!(settings.render_delay, 100, "clamped");
        assert_eq!(settings.layer_height, LAYER_HEIGHT);
        assert_eq!(settings.library_folders, [a]);
        assert!(settings.show_grid);
    }

    #[test]
    fn keep_the_icon_picked_before_this_file() {
        let dir = temp("migration");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app-icon.txt"), "violet\n").unwrap();
        let settings = Settings::load(Some(&dir.join("settings.txt")));
        assert_eq!(settings, Settings { icon: Tint::Violet, ..Settings::default() });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_store_keeps_the_defaults() {
        assert_eq!(Settings::load(None), Settings::default());
        Settings::default().save(None).unwrap();
    }
}
