//! The user's settings, kept between launches in `settings.txt` in the
//! settings folder: one `key = value` per line, a key repeated for each
//! folder of a list. Unknown keys and bad values are ignored, so that an
//! older or newer app reads what it understands.

use std::path::{Path, PathBuf};

use crate::agent_setup::AgentSettings;
use crate::app_icon::Tint;
use crate::i18n::Language;
use crate::shortcuts::Shortcuts;
use crate::theme::Appearance;
use crate::worker::Format;

/// The range and default of the code's size, in points.
pub const CODE_SIZES: std::ops::RangeInclusive<f32> = 10.0..=24.0;
const CODE_SIZE: f32 = 13.5;
/// The range and default of the pause in typing before a render, in ms.
pub const RENDER_DELAYS: std::ops::RangeInclusive<u32> = 100..=3000;
const RENDER_DELAY: u32 = 400;
/// The range and default of the layer view's layer height, in mm.
pub const LAYER_HEIGHTS: std::ops::RangeInclusive<f32> = 0.05..=1.0;
const LAYER_HEIGHT: f32 = 0.2;
/// The range of the interface's size, as a zoom factor (1 is 100 %).
pub const INTERFACE_SIZES: std::ops::RangeInclusive<f32> = 0.8..=1.5;
/// The indent widths the editor offers, in spaces, and the default.
pub const INDENT_WIDTHS: [usize; 3] = [2, 4, 8];
const INDENT_WIDTH: usize = 4;
/// The color of the parts without `color()`, as sRGB bytes: the
/// reference's yellow (`c2f_geom::DEFAULT_COLOR`).
pub const MODEL_COLOR: [u8; 3] = [249, 215, 44];
/// The window's smallest and largest content size, in points.
/// The glTF scales offered: a model in millimeters at its size, or a unit
/// to a meter.
pub const GLTF_SCALES: [f64; 2] = [0.001, 1.0];
/// The seconds a glTF animation may last, and its frames a second.
pub const GLTF_DURATIONS: std::ops::RangeInclusive<f64> = 0.1..=600.0;
pub const GLTF_FPS: std::ops::RangeInclusive<u32> = 1..=120;

pub const MIN_WINDOW: [f32; 2] = [760.0, 480.0];
const MAX_WINDOW: f32 = 100_000.0;

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
    /// STL exports are binary rather than ASCII.
    pub binary_stl: bool,
    /// The folder Export… opens in; `None` for the design's own folder.
    pub export_folder: Option<PathBuf>,
    /// Meters per model unit in glTF exports.
    pub gltf_scale: f64,
    /// The animation of a glTF export of a design that reads `$t`: the
    /// seconds from 0 to 1, the frames a second, and whether it loops.
    pub gltf_duration: f64,
    pub gltf_fps: u32,
    pub gltf_loop: bool,
    /// The color of the parts without `color()`.
    pub model_color: [u8; 3],
    /// The interface's zoom factor, which ⌘+ and ⌘− change too.
    pub interface_size: f32,
    /// The spaces Tab indents by.
    pub indent_width: usize,
    /// The window as last left; `None` the first time.
    pub window: Option<Placement>,
    pub window_maximized: bool,
    /// Light, dark, or as the system.
    pub appearance: Appearance,
    /// Each action's keys.
    pub shortcuts: Shortcuts,
    /// The interface's language.
    pub language: Language,
    /// The tabs open when the app was last closed come back at launch.
    pub reopen_tabs: bool,
    /// Files with a name are saved as soon as typing pauses.
    pub autosave: bool,
    /// The format the Export shortcut writes a 3D model in; a 2D model
    /// goes to SVG.
    pub export_format: Format,
    /// The Finder (or the file manager) shows each file exported.
    pub reveal_export: bool,
    /// Buttons and highlights take the app icon's color rather than blue.
    pub accent_from_icon: bool,
    /// The agent's engine, model and permissions.
    pub agent: AgentSettings,
}

/// Where the window is, in the screen's points: the top left corner of its
/// frame, where winit places a window, and the size of its content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: [f32; 2],
    pub size: [f32; 2],
}

impl Placement {
    /// From `x y width height`; the size is kept within bounds.
    fn parse(value: &str) -> Option<Self> {
        let numbers: Vec<f32> = value.split_whitespace().map(finite).collect::<Option<_>>()?;
        let &[x, y, width, height] = numbers.as_slice() else { return None };
        // Beyond any screen: the value is broken, not just off screen.
        if x.abs() > MAX_WINDOW || y.abs() > MAX_WINDOW {
            return None;
        }
        let size = [clamp(width, &(MIN_WINDOW[0]..=MAX_WINDOW)), clamp(height, &(MIN_WINDOW[1]..=MAX_WINDOW))];
        Some(Placement { position: [x, y], size })
    }
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
            binary_stl: false,
            export_folder: None,
            gltf_scale: GLTF_SCALES[0],
            gltf_duration: 2.0,
            gltf_fps: 30,
            gltf_loop: true,
            model_color: MODEL_COLOR,
            interface_size: 1.0,
            indent_width: INDENT_WIDTH,
            window: None,
            window_maximized: false,
            appearance: Appearance::default(),
            shortcuts: Shortcuts::default(),
            language: Language::default(),
            reopen_tabs: true,
            autosave: false,
            export_format: Format::Stl,
            reveal_export: false,
            accent_from_icon: false,
            agent: AgentSettings::default(),
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
                "layer-height" => set(&mut settings.layer_height, finite(value).map(|v| clamp(v, &LAYER_HEIGHTS))),
                "stl" => {
                    set(&mut settings.binary_stl, ["ascii", "binary"].iter().position(|&f| f == value).map(|i| i == 1))
                }
                "export-folder" => set(&mut settings.export_folder, absolute_folder(value).map(Some)),
                "gltf-scale" => set(&mut settings.gltf_scale, value.parse().ok().filter(|s| GLTF_SCALES.contains(s))),
                "gltf-duration" => {
                    let duration = value.parse::<f64>().ok().filter(|d| d.is_finite()).map(|d| clamp(d, &GLTF_DURATIONS));
                    set(&mut settings.gltf_duration, duration)
                }
                "gltf-fps" => set(&mut settings.gltf_fps, value.parse::<u32>().ok().map(|f| clamp(f, &GLTF_FPS))),
                "gltf-loop" => set(&mut settings.gltf_loop, value.parse().ok()),
                "model-color" => set(&mut settings.model_color, hex_color(value)),
                "interface-size" => {
                    set(&mut settings.interface_size, finite(value).map(|v| clamp(v, &INTERFACE_SIZES)))
                }
                "indent" => set(&mut settings.indent_width, value.parse().ok().filter(|w| INDENT_WIDTHS.contains(w))),
                "window" => set(&mut settings.window, Placement::parse(value).map(Some)),
                "window-maximized" => set(&mut settings.window_maximized, value.parse().ok()),
                "appearance" => set(&mut settings.appearance, Appearance::from_id(value)),
                "language" => set(&mut settings.language, Language::from_id(value)),
                "reopen-tabs" => set(&mut settings.reopen_tabs, value.parse().ok()),
                "autosave" => set(&mut settings.autosave, value.parse().ok()),
                "export-format" => {
                    let format = Format::SOLID.iter().find(|f| f.extension() == value).copied();
                    set(&mut settings.export_format, format)
                }
                "reveal-export" => set(&mut settings.reveal_export, value.parse().ok()),
                "accent-from-icon" => set(&mut settings.accent_from_icon, value.parse().ok()),
                key => {
                    if settings.agent.read(key, value) {
                        continue;
                    }
                    if let Some(action) = key.strip_prefix("shortcut-") {
                        settings.shortcuts.read(action, value);
                    }
                }
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
        text.push_str(&format!("stl = {}\n", if self.binary_stl { "binary" } else { "ascii" }));
        let export_folder = self.export_folder.as_deref().and_then(Path::to_str);
        if let Some(folder) = export_folder.filter(|f| !f.contains(['\n', '\r'])) {
            text.push_str(&format!("export-folder = {folder}\n"));
        }
        text.push_str(&format!("gltf-scale = {}\n", self.gltf_scale));
        text.push_str(&format!("gltf-duration = {}\n", self.gltf_duration));
        text.push_str(&format!("gltf-fps = {}\n", self.gltf_fps));
        text.push_str(&format!("gltf-loop = {}\n", self.gltf_loop));
        let [r, g, b] = self.model_color;
        text.push_str(&format!("model-color = #{r:02x}{g:02x}{b:02x}\n"));
        text.push_str(&format!("interface-size = {}\n", self.interface_size));
        text.push_str(&format!("indent = {}\n", self.indent_width));
        if let Some(Placement { position: [x, y], size: [width, height] }) = self.window {
            text.push_str(&format!("window = {x} {y} {width} {height}\n"));
        }
        text.push_str(&format!("window-maximized = {}\n", self.window_maximized));
        text.push_str(&format!("appearance = {}\n", self.appearance.id()));
        text.push_str(&format!("language = {}\n", self.language.id()));
        text.push_str(&format!("reopen-tabs = {}\n", self.reopen_tabs));
        text.push_str(&format!("autosave = {}\n", self.autosave));
        text.push_str(&format!("export-format = {}\n", self.export_format.extension()));
        text.push_str(&format!("reveal-export = {}\n", self.reveal_export));
        text.push_str(&format!("accent-from-icon = {}\n", self.accent_from_icon));
        self.agent.write(&mut text);
        self.shortcuts.write(&mut text);
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
    if let Some(folder) = absolute_folder(value) {
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
}

fn absolute_folder(value: &str) -> Option<PathBuf> {
    Some(PathBuf::from(value)).filter(|folder| folder.is_absolute())
}

/// `#rrggbb` as bytes.
fn hex_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    // `from_str_radix` would take a sign.
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Key, KeyboardShortcut, Modifiers};

    use super::*;
    use crate::shortcuts::Action;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("code2form-settings-{name}-{}", std::process::id()))
    }

    /// An absolute path on every system (`/opt` has no drive on Windows).
    fn absolute(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn survive_a_save_and_load() {
        let mut shortcuts = Shortcuts::default();
        let ctrl_r = KeyboardShortcut::new(Modifiers { ctrl: true, alt: true, ..Modifiers::NONE }, Key::R);
        shortcuts.set(Action::Render, ctrl_r);
        shortcuts.set(Action::ZoomIn, KeyboardShortcut::new(Modifiers::NONE, Key::F12));
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
            binary_stl: true,
            export_folder: Some(absolute("exports")),
            gltf_scale: 1.0,
            gltf_duration: 4.5,
            gltf_fps: 24,
            gltf_loop: false,
            model_color: [0x12, 0xab, 0xef],
            interface_size: 1.2,
            indent_width: 2,
            window: Some(Placement { position: [-1200.5, 40.0], size: [1440.0, 900.0] }),
            window_maximized: true,
            appearance: Appearance::Light,
            shortcuts,
            language: Language::French,
            reopen_tabs: false,
            autosave: true,
            export_format: Format::Glb,
            reveal_export: true,
            accent_from_icon: true,
            agent: AgentSettings { ask_first: true, allow_export: false, claude_model: "haiku".into(), ..AgentSettings::default() },
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
        // Only the scales offered.
        assert_eq!(Settings::parse("gltf-scale = 0.5\n").gltf_scale, 0.001);
    }

    #[test]
    fn check_the_export_view_and_window_settings() {
        let text = "stl = BINARY\nexport-folder = relative/folder\nmodel-color = #+f+f+f\ninterface-size = 3\n\
                    indent = 3\nwindow = 10 20 30\nwindow-maximized = 1\n";
        let settings = Settings::parse(text);
        assert_eq!(settings, Settings { interface_size: 1.5, ..Settings::default() }, "only the size, clamped");

        let text = "stl = binary\nmodel-color = #FF8000\ninterface-size = 0.1\nindent = 8\nwindow = 10 -20 300 9e9\n";
        let settings = Settings::parse(text);
        assert!(settings.binary_stl);
        assert_eq!(settings.model_color, [255, 128, 0]);
        assert_eq!(settings.interface_size, 0.8, "clamped");
        assert_eq!(settings.indent_width, 8);
        let window = Placement { position: [10.0, -20.0], size: [MIN_WINDOW[0], MAX_WINDOW] };
        assert_eq!(settings.window, Some(window), "sizes clamped");

        for broken in ["window = 1e9 0 800 600", "window = 0 0 800 NaN", "window = 0 0 800 600 1"] {
            assert_eq!(Settings::parse(broken).window, None, "{broken}");
        }
    }

    #[test]
    fn check_the_appearance_and_the_shortcuts() {
        let text = "appearance = sepia\nshortcut-render = F6\nshortcut-save = K\nshortcut-fly = Cmd+Y\n\
                    shortcut-find = Cmd+Hyper\n";
        let settings = Settings::parse(text);
        assert_eq!(settings.appearance, Appearance::System, "the default");
        let mut shortcuts = Shortcuts::default();
        shortcuts.set(Action::Render, KeyboardShortcut::new(Modifiers::NONE, Key::F6));
        assert_eq!(settings.shortcuts, shortcuts, "only F6: K alone types text");
        assert_eq!(Settings::parse("appearance = dark\n").appearance, Appearance::Dark);
        assert!(!Settings::default().format().contains("shortcut-"), "the default keys are not written");
    }

    #[test]
    fn check_the_language_and_the_export_format() {
        let settings = Settings::parse("language = de\nexport-format = svg\nautosave = yes\n");
        assert_eq!(settings, Settings::default(), "a language, a 2D format and a word it does not know");
        let settings = Settings::parse("language = fr\nexport-format = 3mf\n");
        assert_eq!((settings.language, settings.export_format), (Language::French, Format::ThreeMf));
    }

    #[test]
    fn the_model_color_is_the_renderers_yellow() {
        let bytes = c2f_geom::DEFAULT_COLOR.map(|c| (c * 255.0).round() as u8);
        assert_eq!(MODEL_COLOR, bytes[..3]);
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
