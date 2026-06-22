//! The editor's egui styling: a flat, Zed-flavored chrome that follows the OS light/dark theme.
//!
//! `apply` styles both built-in themes from the palettes below and sets egui to follow the OS
//! (`ThemePreference::System`), so the chrome switches with the desktop. The dark palette is read off
//! the Zed reference; the light palette mirrors it in Zed's light look. Wherever the view paints its
//! own colors - the tabs, text, the GPU viewport clear (`app.rs`) - it reads the active palette back
//! through [`palette`], so those follow the theme too. Geometry stays tight and flat: no shadows,
//! small uniform rounding, hairline borders.

use egui::{Color32, CornerRadius, Margin, Stroke, Visuals, vec2};

/// One theme's colors. There is one per built-in theme; [`palette`] returns the active one. The
/// fields the view paints with are public; the rest only feed egui's own surfaces here.
pub struct Palette {
    /// The editor/viewport surface - the GPU clear and the active tab borrow it.
    pub editor_bg: Color32,
    /// Secondary text: inactive tabs, hints, the idle status line, the hamburger glyph at rest.
    pub text_dim: Color32,
    /// Brighter text: the active tab, hovered controls.
    pub text_bright: Color32,
    /// The one accent: the active tab's line and selections.
    pub accent: Color32,
    /// Panel and header surface, a step off the editor so the chrome frames the viewport.
    surface: Color32,
    /// Menus and floating windows, a further step so a popover lifts off the chrome.
    floating: Color32,
    /// Hairline borders and separators. Public because the chrome paints them directly - the icon
    /// bar's top hairline and group divider, the panel header's underline - not only through egui's
    /// own panel separators (sharp-edges: a colour the view paints must be public).
    pub border: Color32,
    /// Hover fill for buttons and menu items. Public because the nav panel paints it directly behind a
    /// hovered clickable content row (sharp-edges: a colour the view paints must be public), the same
    /// reason `text` and `border` are - egui's own widgets read it through `hovered.bg_fill`, set here.
    pub hover: Color32,
    /// Pressed and open fill.
    pressed: Color32,
    /// Primary text: ordinary body text. Public because the nav panel paints its content-list rows
    /// directly in this colour (sharp-edges: a colour the view paints must be public), the same reason
    /// `border` is - egui's own widgets read it through `noninteractive.fg_stroke`, set from here.
    pub text: Color32,
}

/// Dark palette, read off the Zed reference.
const DARK: Palette = Palette {
    editor_bg: Color32::from_rgb(0x18, 0x1a, 0x1f),
    text_dim: Color32::from_rgb(0x7d, 0x85, 0x92),
    text_bright: Color32::from_rgb(0xe4, 0xe7, 0xec),
    accent: Color32::from_rgb(0x4a, 0x86, 0xd8),
    surface: Color32::from_rgb(0x20, 0x23, 0x2a),
    floating: Color32::from_rgb(0x26, 0x2a, 0x32),
    border: Color32::from_rgb(0x2e, 0x33, 0x3c),
    hover: Color32::from_rgb(0x2c, 0x31, 0x3a),
    pressed: Color32::from_rgb(0x34, 0x3a, 0x45),
    text: Color32::from_rgb(0xc6, 0xca, 0xd2),
};

/// Light palette, mirroring the dark one in Zed's light look: a near-white editor, panels a touch
/// darker, dark text, the same blue accent a shade deeper for contrast.
const LIGHT: Palette = Palette {
    editor_bg: Color32::from_rgb(0xfc, 0xfc, 0xfd),
    text_dim: Color32::from_rgb(0x86, 0x8d, 0x9a),
    text_bright: Color32::from_rgb(0x16, 0x18, 0x1d),
    accent: Color32::from_rgb(0x3a, 0x6f, 0xd6),
    surface: Color32::from_rgb(0xf0, 0xf1, 0xf4),
    floating: Color32::from_rgb(0xff, 0xff, 0xff),
    border: Color32::from_rgb(0xd6, 0xd9, 0xe0),
    hover: Color32::from_rgb(0xe7, 0xe9, 0xee),
    pressed: Color32::from_rgb(0xda, 0xdd, 0xe4),
    text: Color32::from_rgb(0x2b, 0x2d, 0x34),
};

/// The active theme's palette - the one the view paints its own colors from, so they follow the OS
/// light/dark like egui's own surfaces do. Keyed off the resolved theme's `dark_mode`.
pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if ctx.style().visuals.dark_mode { &DARK } else { &LIGHT }
}

/// Apply the editor look. Styles both built-in themes from their palettes, registers the chrome icon
/// font (so icon codepoints render through it), and follows the OS (`System`), so the chrome switches
/// with the desktop. The one styling entry point both the live app and the snapshot test call, so the
/// font lands on every context the chrome renders through.
pub fn apply(ctx: &egui::Context) {
    crate::icons::install_font(ctx);
    ctx.set_theme(egui::ThemePreference::System);
    style_theme(ctx, egui::Theme::Dark, &DARK);
    style_theme(ctx, egui::Theme::Light, &LIGHT);
}

/// Style one built-in theme: the shared geometry, then its palette's surfaces, text, borders, and
/// accent painted over egui's base for that theme.
fn style_theme(ctx: &egui::Context, theme: egui::Theme, p: &Palette) {
    ctx.style_mut_of(theme, |style| {
        // Spacing: comfortable and even, so rows read as a list rather than a cramped form.
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(8.0, 4.0);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.window_margin = Margin::same(8);
        style.spacing.indent = 16.0;
        paint(&mut style.visuals, p);
    });
}

/// Paint a palette onto a `Visuals` (already the right light/dark base, so `dark_mode` stays
/// correct): surfaces, the accent, then per-widget-state text, fills, and borders. Only the fields
/// the chrome shows are set; the rest keep egui's defaults.
fn paint(v: &mut Visuals, p: &Palette) {
    // Surfaces, editor outward: the editor well, the chrome panels, then floating popovers.
    v.panel_fill = p.surface;
    v.window_fill = p.floating;
    v.extreme_bg_color = p.editor_bg;
    v.code_bg_color = p.editor_bg;
    v.faint_bg_color = p.hover;

    // Flat: no shadows, small uniform rounding, one hairline border for floating windows.
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_stroke = Stroke::new(1.0, p.border);

    // The single accent: a translucent fill for selections, the accent hue for links.
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 0x4d);
    v.selection.stroke = Stroke::new(1.0, p.text_bright);
    v.hyperlink_color = p.accent;

    let w = &mut v.widgets;
    // Noninteractive: labels, headings, separators, and the hairline borders panels draw.
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    // Inactive: buttons and menu items at rest read flat - no fill, no border, primary text.
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    // Hovered: a subtle fill and brighter text mark the control under the pointer.
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text_bright);
    // Active (pressed) and open (an open menu button): a stronger fill, brightest text.
    for s in [&mut w.active, &mut w.open] {
        s.weak_bg_fill = p.pressed;
        s.bg_fill = p.pressed;
        s.bg_stroke = Stroke::new(1.0, p.border);
        s.fg_stroke = Stroke::new(1.0, p.text_bright);
    }
    // One small, uniform rounding across every widget state.
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = CornerRadius::same(4);
    }
}
