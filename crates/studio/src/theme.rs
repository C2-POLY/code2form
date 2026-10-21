//! The dark theme: colors, spacing and text sizes.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Vec2, Visuals,
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const ACCENT: Color32 = rgb(0x4c, 0x8d, 0xff);
pub const ACCENT_STRONG: Color32 = rgb(0x6a, 0xa2, 0xff);
pub const ACCENT_MUTED: Color32 = rgb(0x26, 0x4a, 0x8c);

/// Window and toolbar background.
pub const BG: Color32 = rgb(0x16, 0x18, 0x1d);
/// Side panels and the console.
pub const PANEL: Color32 = rgb(0x1b, 0x1e, 0x24);
/// The code editor.
pub const EDITOR: Color32 = rgb(0x1e, 0x21, 0x28);
/// The editor's line-number gutter.
pub const GUTTER: Color32 = rgb(0x1a, 0x1d, 0x23);
pub const CURRENT_LINE: Color32 = rgb(0x25, 0x29, 0x32);
/// Find matches in the editor, and the current one: `WARNING` amber mixed
/// into `EDITOR`, faint and strong.
pub const FIND_MATCH: Color32 = rgb(0x4a, 0x40, 0x31);
pub const FIND_CURRENT: Color32 = rgb(0x8c, 0x6c, 0x3a);
pub const RAISED: Color32 = rgb(0x26, 0x2a, 0x33);
pub const HOVER: Color32 = rgb(0x30, 0x35, 0x40);
pub const BORDER: Color32 = rgb(0x2c, 0x31, 0x3b);

pub const TEXT: Color32 = rgb(0xd8, 0xdc, 0xe4);
pub const TEXT_DIM: Color32 = rgb(0x8b, 0x94, 0xa5);
pub const TEXT_FAINT: Color32 = rgb(0x5c, 0x64, 0x73);

pub const ERROR: Color32 = rgb(0xff, 0x6b, 0x6b);
pub const WARNING: Color32 = rgb(0xf2, 0xb5, 0x5b);
pub const SUCCESS: Color32 = rgb(0x5f, 0xd0, 0x8a);

pub const CODE_TEXT: Color32 = rgb(0xd4, 0xd8, 0xe0);
pub const CODE_KEYWORD: Color32 = rgb(0xc6, 0x8a, 0xf5);
pub const CODE_MODULE: Color32 = rgb(0x5f, 0xb1, 0xff);
pub const CODE_FUNCTION: Color32 = rgb(0x4f, 0xd1, 0xc5);
pub const CODE_NUMBER: Color32 = rgb(0xf0, 0x9e, 0x5f);
pub const CODE_STRING: Color32 = rgb(0x9b, 0xd1, 0x6c);
pub const CODE_COMMENT: Color32 = rgb(0x6b, 0x74, 0x85);
pub const CODE_SPECIAL: Color32 = rgb(0xff, 0x7a, 0x93);
pub const CODE_OPERATOR: Color32 = rgb(0x9a, 0xa4, 0xb5);
pub const CODE_DEFINITION: Color32 = rgb(0xf5, 0xd0, 0x6c);

/// The editor and console font.
pub fn code_font() -> FontId {
    FontId::new(13.5, FontFamily::Monospace)
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.visuals = visuals();
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

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = None;
    v.panel_fill = PANEL;
    v.window_fill = RAISED;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(140) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(120) };
    v.faint_bg_color = rgb(0x20, 0x23, 0x2a);
    v.extreme_bg_color = EDITOR;
    v.code_bg_color = RAISED;
    v.hyperlink_color = ACCENT_STRONG;
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    v.selection.bg_fill = ACCENT_MUTED;
    v.selection.stroke = Stroke::new(1.0, ACCENT_STRONG);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT_STRONG);

    let radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, rgb(0x3d, 0x44, 0x52));
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = ACCENT_MUTED;
    w.active.weak_bg_fill = ACCENT_MUTED;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = RAISED;
    w.open.weak_bg_fill = RAISED;
    w.open.bg_stroke = Stroke::new(1.0, BORDER);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;
    v
}
