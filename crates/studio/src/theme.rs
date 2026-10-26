//! The light and the dark theme: colors, spacing and text sizes. The
//! colors of the theme shown are `palette()`'s, which `follow` sets at the
//! start of each frame from the appearance the settings ask for.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::app_icon::Tint;
use crate::i18n::tr;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Vec2, Visuals,
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The theme the settings ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// The system's, light or dark, as it changes.
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    /// As `settings.txt` writes it.
    pub fn id(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Appearance::ALL.into_iter().find(|a| a.id() == id.trim())
    }

    pub fn name(self) -> &'static str {
        match self {
            Appearance::System => tr("System", "Système"),
            Appearance::Light => tr("Light", "Clair"),
            Appearance::Dark => tr("Dark", "Sombre"),
        }
    }
}

/// The colors of a theme.
#[derive(Debug, Clone)]
pub struct Palette {
    pub dark: bool,
    pub accent: Color32,
    pub accent_strong: Color32,
    pub accent_muted: Color32,
    /// Text on `accent`, as on the Render button.
    pub on_accent: Color32,

    /// Window and toolbar background.
    pub bg: Color32,
    /// Side panels and the console.
    pub panel: Color32,
    /// The code editor.
    pub editor: Color32,
    /// The editor's line-number gutter.
    pub gutter: Color32,
    pub current_line: Color32,
    /// Find matches in the editor, and the current one: `warning`'s amber
    /// mixed into `editor`, faint and strong.
    pub find_match: Color32,
    pub find_current: Color32,
    /// Buttons and fields.
    pub raised: Color32,
    /// Dialogs, menus and tooltips.
    pub window: Color32,
    pub hover: Color32,
    pub border: Color32,
    /// Controls hovered over a raised surface.
    pub border_strong: Color32,
    pub faint_bg: Color32,

    pub text: Color32,
    /// Hovered and pressed controls' text.
    pub text_strong: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,

    pub error: Color32,
    pub warning: Color32,
    pub success: Color32,

    pub code_text: Color32,
    pub code_keyword: Color32,
    pub code_module: Color32,
    pub code_function: Color32,
    pub code_number: Color32,
    pub code_string: Color32,
    pub code_comment: Color32,
    pub code_special: Color32,
    pub code_operator: Color32,
    pub code_definition: Color32,

    /// Text on the cards over the 3D view: the selection, the legend, the
    /// bubbles and the assembly bar.
    pub overlay_text: Color32,
    pub overlay_text_dim: Color32,
    /// The card shown when there is nothing to draw, and the status badges.
    pub card: Color32,
    pub badge: Color32,
    /// The pauses' labels, the cyan of their planes.
    pub pause: Color32,
    /// The mouse help in the view's corner.
    pub view_hint: Color32,

    /// The 3D view's gradient, top and bottom, and its grid's major and
    /// minor lines.
    pub view_top: [f32; 3],
    pub view_bottom: [f32; 3],
    /// How much the view's corners darken, and the noise that breaks the
    /// gradient's bands.
    pub view_vignette: f32,
    pub view_dither: f32,
    /// The disc behind the axes' compass.
    pub gizmo: Color32,
    pub grid_major: [f32; 4],
    pub grid_minor: [f32; 4],
    /// What greyed-out parts and wires fade towards.
    pub dim: [f32; 4],

    window_shadow: u8,
    popup_shadow: u8,
}

impl Palette {
    /// The fill of the cards over the 3D view, `alpha` as in the dark
    /// theme: they read the same whatever is behind them.
    pub fn veil(&self, alpha: u8) -> Color32 {
        if self.dark { Color32::from_black_alpha(alpha) } else { Color32::from_white_alpha(alpha / 2 + 128) }
    }

    /// A faint line or fill, `alpha` as in the dark theme: light on the
    /// dark theme, dark on the light one.
    pub fn line(&self, alpha: u8) -> Color32 {
        if self.dark { Color32::from_white_alpha(alpha) } else { Color32::from_black_alpha(alpha) }
    }
}

pub static DARK: Palette = Palette {
    dark: true,
    accent: rgb(0x4c, 0x8d, 0xff),
    accent_strong: rgb(0x6a, 0xa2, 0xff),
    accent_muted: rgb(0x26, 0x4a, 0x8c),
    on_accent: Color32::WHITE,

    bg: rgb(0x16, 0x18, 0x1d),
    panel: rgb(0x1b, 0x1e, 0x24),
    editor: rgb(0x1e, 0x21, 0x28),
    gutter: rgb(0x1a, 0x1d, 0x23),
    current_line: rgb(0x25, 0x29, 0x32),
    find_match: rgb(0x4a, 0x40, 0x31),
    find_current: rgb(0x8c, 0x6c, 0x3a),
    raised: rgb(0x26, 0x2a, 0x33),
    window: rgb(0x26, 0x2a, 0x33),
    hover: rgb(0x30, 0x35, 0x40),
    border: rgb(0x2c, 0x31, 0x3b),
    border_strong: rgb(0x3d, 0x44, 0x52),
    faint_bg: rgb(0x20, 0x23, 0x2a),

    text: rgb(0xd8, 0xdc, 0xe4),
    text_strong: Color32::WHITE,
    text_dim: rgb(0x8b, 0x94, 0xa5),
    text_faint: rgb(0x5c, 0x64, 0x73),

    error: rgb(0xff, 0x6b, 0x6b),
    warning: rgb(0xf2, 0xb5, 0x5b),
    success: rgb(0x5f, 0xd0, 0x8a),

    code_text: rgb(0xd4, 0xd8, 0xe0),
    code_keyword: rgb(0xc6, 0x8a, 0xf5),
    code_module: rgb(0x5f, 0xb1, 0xff),
    code_function: rgb(0x4f, 0xd1, 0xc5),
    code_number: rgb(0xf0, 0x9e, 0x5f),
    code_string: rgb(0x9b, 0xd1, 0x6c),
    code_comment: rgb(0x6b, 0x74, 0x85),
    code_special: rgb(0xff, 0x7a, 0x93),
    code_operator: rgb(0x9a, 0xa4, 0xb5),
    code_definition: rgb(0xf5, 0xd0, 0x6c),

    overlay_text: rgb(0xec, 0xee, 0xf2),
    overlay_text_dim: rgb(0xa8, 0xb0, 0xbe),
    card: Color32::from_rgba_premultiplied(19, 22, 27, 225),
    badge: Color32::from_rgba_premultiplied(15, 17, 21, 215),
    pause: rgb(115, 230, 255),
    view_hint: Color32::from_rgba_premultiplied(70, 70, 70, 70),

    view_top: [0.215, 0.240, 0.290],
    view_bottom: [0.078, 0.086, 0.106],
    view_vignette: 0.35,
    view_dither: 0.0,
    gizmo: Color32::from_black_alpha(36),
    grid_major: [0.62, 0.68, 0.80, 0.30],
    grid_minor: [0.55, 0.60, 0.70, 0.11],
    dim: [0.30, 0.32, 0.36, 1.0],

    window_shadow: 140,
    popup_shadow: 120,
};

pub static LIGHT: Palette = Palette {
    dark: false,
    accent: rgb(0x25, 0x63, 0xd6),
    accent_strong: rgb(0x1a, 0x56, 0xc4),
    accent_muted: rgb(0xcf, 0xdf, 0xfa),
    on_accent: Color32::WHITE,

    bg: rgb(0xe8, 0xeb, 0xef),
    panel: rgb(0xf3, 0xf4, 0xf6),
    editor: rgb(0xff, 0xff, 0xff),
    gutter: rgb(0xf6, 0xf7, 0xf9),
    current_line: rgb(0xee, 0xf2, 0xf8),
    find_match: rgb(0xfd, 0xf0, 0xd0),
    find_current: rgb(0xfb, 0xe2, 0xa6),
    raised: rgb(0xdd, 0xe1, 0xe7),
    window: rgb(0xfa, 0xfb, 0xfc),
    hover: rgb(0xd2, 0xd7, 0xdf),
    border: rgb(0xcf, 0xd4, 0xdb),
    border_strong: rgb(0xb3, 0xba, 0xc5),
    faint_bg: rgb(0xee, 0xf0, 0xf3),

    text: rgb(0x1f, 0x23, 0x28),
    text_strong: rgb(0x0b, 0x0e, 0x12),
    text_dim: rgb(0x4f, 0x58, 0x64),
    text_faint: rgb(0x62, 0x6b, 0x77),

    error: rgb(0xbd, 0x2f, 0x29),
    warning: rgb(0x8f, 0x5a, 0x00),
    success: rgb(0x1a, 0x73, 0x36),

    code_text: rgb(0x1f, 0x23, 0x28),
    code_keyword: rgb(0x7a, 0x3f, 0xc8),
    code_module: rgb(0x0a, 0x59, 0xb0),
    code_function: rgb(0x08, 0x70, 0x67),
    code_number: rgb(0xa3, 0x4a, 0x00),
    code_string: rgb(0x2b, 0x73, 0x19),
    code_comment: rgb(0x5b, 0x64, 0x70),
    code_special: rgb(0xbb, 0x26, 0x50),
    code_operator: rgb(0x4f, 0x58, 0x64),
    code_definition: rgb(0x8a, 0x5a, 0x00),

    overlay_text: rgb(0x1f, 0x23, 0x28),
    overlay_text_dim: rgb(0x4f, 0x58, 0x64),
    card: Color32::from_rgba_premultiplied(230, 231, 232, 235),
    badge: Color32::from_rgba_premultiplied(221, 221, 222, 225),
    pause: rgb(0x00, 0x70, 0x8c),
    view_hint: Color32::from_black_alpha(140),

    view_top: [0.933, 0.945, 0.961],
    view_bottom: [0.827, 0.847, 0.878],
    view_vignette: 0.15,
    view_dither: 1.0,
    gizmo: Color32::from_black_alpha(12),
    grid_major: [0.32, 0.37, 0.46, 0.38],
    grid_minor: [0.36, 0.41, 0.50, 0.14],
    dim: [0.80, 0.82, 0.85, 1.0],

    window_shadow: 40,
    popup_shadow: 36,
};

/// Whether `palette()` is the light theme's.
static SHOWS_LIGHT: AtomicBool = AtomicBool::new(false);

/// The accent shown: 0 for blue, else the icon tint's, from 1.
static ACCENT: AtomicUsize = AtomicUsize::new(0);

/// Each accent's dark and light palettes: blue's, then each tint's.
fn palettes() -> &'static [[Palette; 2]] {
    static PALETTES: OnceLock<Vec<[Palette; 2]>> = OnceLock::new();
    PALETTES.get_or_init(|| {
        let blue = [DARK.clone(), LIGHT.clone()];
        let tints = Tint::ALL.iter().map(|&tint| [accented(&DARK, tint), accented(&LIGHT, tint)]);
        std::iter::once(blue).chain(tints).collect()
    })
}

/// The colors of the theme shown.
/// The cards floating over the 3D view: its tools, the parameters, the
/// animation bar, the chat with the agent.
pub fn card_frame() -> egui::Frame {
    let palette = palette();
    egui::Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(10))
        .shadow(Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if palette.dark { 90 } else { 30 }) })
}

pub fn palette() -> &'static Palette {
    &palettes()[ACCENT.load(Ordering::Relaxed)][usize::from(SHOWS_LIGHT.load(Ordering::Relaxed))]
}

/// Buttons and highlights in `tint`'s color, or blue; whether it changed.
pub fn set_accent(ctx: &egui::Context, tint: Option<Tint>) -> bool {
    let index = tint.and_then(|t| Tint::ALL.iter().position(|&a| a == t)).map_or(0, |i| i + 1);
    let changed = ACCENT.swap(index, Ordering::Relaxed) != index;
    if changed {
        restyle(ctx);
    }
    changed
}

/// `base` with its accent in `tint`'s colors: the icon's vivid square on
/// the dark theme, its deep tile on the light one, both legible.
fn accented(base: &Palette, tint: Tint) -> Palette {
    let [tile, outer, inner] = tint.colors().map(|[r, g, b]| rgb(r, g, b));
    let mut palette = base.clone();
    if base.dark {
        palette.accent = outer;
        palette.accent_strong = mix(outer, inner, 0.45);
        palette.accent_muted = mix(tile, base.bg, 0.35);
    } else {
        palette.accent = mix(tile, outer, 0.15);
        palette.accent_strong = mix(tile, Color32::BLACK, 0.15);
        palette.accent_muted = mix(inner, Color32::WHITE, 0.55);
    }
    // White, or the dark theme's background on a bright accent: whichever
    // reads better.
    let ratio = |text: Color32| {
        let (a, b) = (luminance(text), luminance(palette.accent));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    palette.on_accent = if ratio(DARK.bg) > ratio(Color32::WHITE) { DARK.bg } else { Color32::WHITE };
    palette
}

/// `a` and `b` mixed, `t` of the way to `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The relative luminance of an opaque color, from 0 to 1, as WCAG has it.
fn luminance(c: Color32) -> f32 {
    let channel = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// The theme `appearance` asks for: the system's, or dark when the system
/// does not say.
pub fn resolve(appearance: Appearance, system: Option<Theme>) -> Theme {
    match appearance {
        Appearance::System => system.unwrap_or(Theme::Dark),
        Appearance::Light => Theme::Light,
        Appearance::Dark => Theme::Dark,
    }
}

/// The theme to show this frame for `appearance`.
pub fn wanted(ctx: &egui::Context, appearance: Appearance) -> Theme {
    resolve(appearance, system_theme(ctx))
}

/// Shows the theme `appearance` asks for, from the start of a frame, the
/// system's following it as it changes; whether the theme changed.
pub fn follow(ctx: &egui::Context, appearance: Appearance) -> bool {
    let theme = wanted(ctx, appearance);
    if ctx.theme() != theme {
        ctx.set_theme(theme);
    }
    let light = theme == Theme::Light;
    SHOWS_LIGHT.swap(light, Ordering::Relaxed) != light
}

/// Gives the window's title bar the appearance asked for, on macOS: the
/// system's, or the one picked.
pub fn style_window(ctx: &egui::Context, appearance: Appearance) {
    if cfg!(target_os = "macos") {
        let theme = match appearance {
            Appearance::System => egui::SystemTheme::SystemDefault,
            Appearance::Light => egui::SystemTheme::Light,
            Appearance::Dark => egui::SystemTheme::Dark,
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
    }
}

/// The system's theme. On macOS it is the app's, which a theme given to
/// the window does not change: winit stops reporting the system's changes
/// once the window has one.
fn system_theme(ctx: &egui::Context) -> Option<Theme> {
    mac::system_theme().or_else(|| ctx.input(|i| i.raw.system_theme))
}

#[cfg(target_os = "macos")]
mod mac {
    use eframe::egui::Theme;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    /// The app's appearance; `None` off the main thread, as in tests.
    pub fn system_theme() -> Option<Theme> {
        let mtm = MainThreadMarker::new()?;
        let name = NSApplication::sharedApplication(mtm).effectiveAppearance().name().to_string();
        Some(if name.contains("Dark") { Theme::Dark } else { Theme::Light })
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    pub fn system_theme() -> Option<eframe::egui::Theme> {
        None
    }
}

/// The size of `code_font`, set from the settings.
static CODE_SIZE: AtomicU32 = AtomicU32::new(13.5f32.to_bits());

/// The editor and console font.
pub fn code_font() -> FontId {
    FontId::new(f32::from_bits(CODE_SIZE.load(Ordering::Relaxed)), FontFamily::Monospace)
}

/// Sets the editor's and the console's text size, in points.
pub fn set_code_size(ctx: &egui::Context, size: f32) {
    CODE_SIZE.store(size.to_bits(), Ordering::Relaxed);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.text_styles.insert(TextStyle::Monospace, code_font());
        });
    }
}

/// Gives egui both themes' colors, in the accent shown.
fn restyle(ctx: &egui::Context) {
    let [dark, light] = &palettes()[ACCENT.load(Ordering::Relaxed)];
    for (theme, palette) in [(Theme::Dark, dark), (Theme::Light, light)] {
        ctx.style_mut_of(theme, |style| style.visuals = visuals(palette));
    }
}

/// Gives egui both themes, and shows the dark one until `follow` says.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    restyle(ctx);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.text_styles = [
                (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
                (TextStyle::Monospace, code_font()),
            ]
            .into();
            let spacing = &mut style.spacing;
            spacing.item_spacing = Vec2::new(8.0, 6.0);
            spacing.button_padding = Vec2::new(10.0, 5.0);
            spacing.interact_size = Vec2::new(36.0, 26.0);
            spacing.window_margin = Margin::same(14);
            spacing.menu_margin = Margin::same(8);
            spacing.scroll.bar_width = 8.0;
            spacing.scroll.floating = true;
            style.interaction.tooltip_delay = 0.35;
        });
    }
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = None;
    v.panel_fill = p.panel;
    v.window_fill = p.window;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    let window_shadow = Color32::from_black_alpha(p.window_shadow);
    v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: window_shadow };
    let popup_shadow = Color32::from_black_alpha(p.popup_shadow);
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: popup_shadow };
    v.faint_bg_color = p.faint_bg;
    v.extreme_bg_color = p.editor;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.accent_strong;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent_muted;
    v.selection.stroke = Stroke::new(1.0, p.accent_strong);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent_strong);

    let radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_dim);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text_strong);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent_muted;
    w.active.weak_bg_fill = p.accent_muted;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text_strong);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WCAG contrast ratio of two opaque colors, from 1 to 21.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// `color` (premultiplied, as egui keeps it) laid over the opaque `under`.
    fn over(color: Color32, under: Color32) -> Color32 {
        let a = f32::from(color.a()) / 255.0;
        let mix = |c: u8, u: u8| (f32::from(c) + f32::from(u) * (1.0 - a)).round() as u8;
        rgb(mix(color.r(), under.r()), mix(color.g(), under.g()), mix(color.b(), under.b()))
    }

    fn view(c: [f32; 3]) -> Color32 {
        let [r, g, b] = c.map(|v| (v * 255.0).round() as u8);
        rgb(r, g, b)
    }

    /// Each text color and what it is read on, with a name for each.
    fn pairs(p: &Palette) -> Vec<(String, f32)> {
        let mut pairs = Vec::new();
        let mut check = |fg: (&str, Color32), bgs: &[(&str, Color32)]| {
            for &(name, bg) in bgs {
                pairs.push((format!("{} on {name}", fg.0), contrast(fg.1, bg)));
            }
        };
        let surfaces = [("bg", p.bg), ("panel", p.panel), ("editor", p.editor), ("window", p.window)];
        for fg in [("text", p.text), ("text_dim", p.text_dim)] {
            check(fg, &surfaces);
            check(fg, &[("raised", p.raised)]);
        }
        check(("text_strong", p.text_strong), &[("hover", p.hover), ("accent_muted", p.accent_muted)]);
        check(("text_faint", p.text_faint), &[("panel", p.panel), ("editor", p.editor), ("gutter", p.gutter)]);
        // The console's levels, the status bar's and the find bar's.
        for fg in
            [("error", p.error), ("warning", p.warning), ("success", p.success), ("accent_strong", p.accent_strong)]
        {
            check(fg, &surfaces);
        }
        check(("accent_strong", p.accent_strong), &[("accent_muted", p.accent_muted)]);
        check(("on_accent", p.on_accent), &[("accent", p.accent)]);
        let code = [
            ("code_text", p.code_text),
            ("code_keyword", p.code_keyword),
            ("code_module", p.code_module),
            ("code_function", p.code_function),
            ("code_number", p.code_number),
            ("code_string", p.code_string),
            ("code_comment", p.code_comment),
            ("code_special", p.code_special),
            ("code_operator", p.code_operator),
            ("code_definition", p.code_definition),
        ];
        let lines =
            [("editor", p.editor), ("current_line", p.current_line), ("find_match", over(p.find_match, p.editor))];
        for fg in code {
            check(fg, &lines);
            check(fg, &[("find_current", p.find_current)]);
        }
        // The cards over the 3D view, over its lightest and darkest colors.
        for (end, under) in [("view_top", view(p.view_top)), ("view_bottom", view(p.view_bottom))] {
            let cards = [
                (format!("card over {end}"), over(p.card, under)),
                (format!("badge over {end}"), over(p.badge, under)),
                (format!("veil(160) over {end}"), over(p.veil(160), under)),
            ];
            for (name, card) in &cards {
                check(("overlay_text", p.overlay_text), &[(name, *card)]);
                check(("overlay_text_dim", p.overlay_text_dim), &[(name, *card)]);
                check(("pause", p.pause), &[(name, *card)]);
            }
        }
        pairs
    }

    /// Prints the ratios: `cargo test -p code2form contrast -- --nocapture`.
    fn report(name: &str, pairs: &[(String, f32)]) {
        for (pair, ratio) in pairs {
            println!("{name}: {pair}: {ratio:.2}");
        }
    }

    #[test]
    fn the_light_theme_reads_well() {
        let pairs = pairs(&LIGHT);
        report("light", &pairs);
        let poor: Vec<_> = pairs.iter().filter(|(_, ratio)| *ratio < 4.5).collect();
        assert!(poor.is_empty(), "under 4.5:1: {poor:?}");
    }

    /// Each icon's accent reads as well as blue: the light theme's pairs
    /// all at 4.5:1, the dark theme's but the accent's own fills.
    #[test]
    fn every_accent_reads_well() {
        let accent = |pair: &str| pair.contains("accent");
        for tint in Tint::ALL {
            let light = pairs(&accented(&LIGHT, tint));
            let poor: Vec<_> = light.iter().filter(|(pair, ratio)| accent(pair) && *ratio < 4.5).collect();
            assert!(poor.is_empty(), "{tint:?}, light, under 4.5:1: {poor:?}");
            let dark = pairs(&accented(&DARK, tint));
            let known = |pair: &str| pair.starts_with("on_accent ") || pair == "accent_strong on accent_muted";
            let poor: Vec<_> =
                dark.iter().filter(|(pair, ratio)| accent(pair) && !known(pair) && *ratio < 4.5).collect();
            assert!(poor.is_empty(), "{tint:?}, dark, under 4.5:1: {poor:?}");
            // Its own fills still read, if less than the rest.
            let fills: Vec<_> = dark.iter().filter(|(pair, ratio)| known(pair) && *ratio < 3.0).collect();
            assert!(fills.is_empty(), "{tint:?}, dark, under 3:1: {fills:?}");
        }
    }

    /// The dark theme keeps its colors: these few were under 4.5:1 before
    /// the light theme, and stay so; nothing else may fall under.
    #[test]
    fn the_dark_theme_reads_as_before() {
        let pairs = pairs(&DARK);
        report("dark", &pairs);
        let known = |pair: &str| {
            pair.starts_with("text_faint ")
                || pair.starts_with("on_accent ")
                || pair == "accent_strong on accent_muted"
                || pair == "code_comment on editor"
                || pair == "code_comment on current_line"
                || pair.ends_with(" on find_match")
                || pair.ends_with(" on find_current")
        };
        let poor: Vec<_> = pairs.iter().filter(|(pair, ratio)| *ratio < 4.5 && !known(pair)).collect();
        assert!(poor.is_empty(), "under 4.5:1: {poor:?}");
    }

    #[test]
    fn the_dark_theme_keeps_its_colors() {
        assert_eq!(DARK.bg, rgb(0x16, 0x18, 0x1d));
        assert_eq!(DARK.text, rgb(0xd8, 0xdc, 0xe4));
        assert_eq!(DARK.code_keyword, rgb(0xc6, 0x8a, 0xf5));
        assert_eq!(DARK.veil(170), Color32::from_black_alpha(170));
        assert_eq!(DARK.line(30), Color32::from_white_alpha(30));
        assert_eq!(DARK.view_top, [0.215, 0.240, 0.290]);
        assert_eq!(DARK.card, Color32::from_rgba_unmultiplied(22, 25, 31, 225));
        assert_eq!(DARK.badge, Color32::from_rgba_unmultiplied(18, 20, 25, 215));
        assert_eq!(DARK.view_hint, Color32::from_white_alpha(70));
        assert_eq!(LIGHT.card, Color32::from_rgba_unmultiplied(250, 251, 252, 235));
        assert_eq!(LIGHT.badge, Color32::from_rgba_unmultiplied(250, 251, 252, 225));
        assert_eq!(visuals(&DARK).widgets.hovered.fg_stroke.color, Color32::WHITE);
    }

    #[test]
    fn contrast_is_wcags() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-3);
        assert!((contrast(rgb(0x77, 0x77, 0x77), Color32::WHITE) - 4.48).abs() < 0.01);
        assert_eq!(contrast(DARK.bg, DARK.bg), 1.0);
    }

    #[test]
    fn appearances_have_ids() {
        for appearance in Appearance::ALL {
            assert_eq!(Appearance::from_id(appearance.id()), Some(appearance));
        }
        assert_eq!(Appearance::from_id("sepia"), None);
    }

    /// One frame of `ctx`, the system's theme `system`: the theme shown.
    fn frame(ctx: &egui::Context, appearance: Appearance, system: Option<Theme>) -> Theme {
        let input = egui::RawInput { system_theme: system, ..Default::default() };
        let mut shown = None;
        let _ = ctx.run(input, |ctx| shown = Some(wanted(ctx, appearance)));
        shown.unwrap()
    }

    #[test]
    fn system_follows_the_system() {
        let ctx = egui::Context::default();
        assert_eq!(frame(&ctx, Appearance::System, Some(Theme::Dark)), Theme::Dark);
        assert_eq!(frame(&ctx, Appearance::System, Some(Theme::Light)), Theme::Light, "the system turned light");
        assert_eq!(frame(&ctx, Appearance::System, Some(Theme::Dark)), Theme::Dark, "and dark again");
        assert_eq!(frame(&ctx, Appearance::System, None), Theme::Dark, "the system does not say");
        assert_eq!(frame(&ctx, Appearance::Light, Some(Theme::Dark)), Theme::Light);
        assert_eq!(frame(&ctx, Appearance::Dark, Some(Theme::Light)), Theme::Dark);
    }

    #[test]
    fn both_themes_get_their_visuals() {
        let ctx = egui::Context::default();
        apply(&ctx);
        for (theme, palette) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
            let style = ctx.style_of(theme);
            assert_eq!(style.visuals.panel_fill, palette.panel);
            assert_eq!(style.visuals.dark_mode, palette.dark);
            assert!(style.text_styles.contains_key(&TextStyle::Monospace));
        }
    }
}
