//! The editor's egui styling: a flat, dark, Zed-flavored chrome.
//!
//! The editor is dark by design. The 3D viewport clears to a fixed dark surface ([`EDITOR_BG`], used
//! by `app.rs`), so the chrome around it is dark to match - a light chrome framing a dark viewport
//! would read as broken. `apply` therefore forces the dark theme rather than following the OS, and
//! paints egui's surfaces, text, borders, and one accent from the palette below, read off the Zed
//! reference. Geometry stays tight and flat: no shadows, small uniform rounding, hairline borders.

use egui::{Color32, CornerRadius, Margin, Stroke, Visuals, vec2};

// ---- palette (sRGB), read off the Zed reference ----

/// The editor/viewport surface: the darkest tone. The 3D view clears to this (`app.rs`), and the
/// active tab borrows it so a selected tab reads as continuous with the editor below it.
pub const EDITOR_BG: Color32 = Color32::from_rgb(0x18, 0x1a, 0x1f);
/// Panel and header surface: a touch lighter than the editor, so the chrome frames the viewport.
const SURFACE: Color32 = Color32::from_rgb(0x20, 0x23, 0x2a);
/// Menus and floating windows: a step lighter again, so a popover lifts off the chrome.
const FLOATING: Color32 = Color32::from_rgb(0x26, 0x2a, 0x32);
/// Hairline borders and separators: low contrast, just enough to part two surfaces.
const BORDER: Color32 = Color32::from_rgb(0x2e, 0x33, 0x3c);
/// Subtle hover fill for buttons and menu items.
const HOVER: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3a);
/// Pressed and open fill, a step up from hover.
const PRESSED: Color32 = Color32::from_rgb(0x34, 0x3a, 0x45);
/// Primary text: soft off-white, easy on a dark surface.
pub const TEXT: Color32 = Color32::from_rgb(0xc6, 0xca, 0xd2);
/// Secondary text: dimmer, for inactive tabs, hints, and the idle status line.
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7d, 0x85, 0x92);
/// Brighter text, for hovered and active controls.
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xe4, 0xe7, 0xec);
/// The one accent: a muted blue marking the active tab and selections.
pub const ACCENT: Color32 = Color32::from_rgb(0x4a, 0x86, 0xd8);

/// Apply the editor look. Forces the dark theme (see module docs) and paints the dark palette over
/// egui's dark base, leaving untouched fields at their dark defaults.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        // Spacing: comfortable and even, so rows read as a list rather than a cramped form.
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(8.0, 4.0);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.window_margin = Margin::same(8);
        style.spacing.indent = 16.0;
        palette(&mut style.visuals);
    });
}

/// Paint the palette onto a dark `Visuals`: surfaces, the accent, then per-widget-state text, fills,
/// and borders. Only the fields the chrome shows are set; the rest keep egui's dark defaults.
fn palette(v: &mut Visuals) {
    // Surfaces, darkest to lightest: the editor well, the chrome panels, then floating popovers.
    v.panel_fill = SURFACE;
    v.window_fill = FLOATING;
    v.extreme_bg_color = EDITOR_BG;
    v.code_bg_color = EDITOR_BG;
    v.faint_bg_color = HOVER;

    // Flat: no shadows, small uniform rounding, one hairline border for floating windows.
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_stroke = Stroke::new(1.0, BORDER);

    // The single accent: a translucent fill for selections, the accent hue for links.
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0x4a, 0x86, 0xd8, 0x4d);
    v.selection.stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.hyperlink_color = ACCENT;

    let w = &mut v.widgets;
    // Noninteractive: labels, headings, separators, and the hairline borders panels draw.
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    // Inactive: buttons and menu items at rest read flat - no fill, no border, primary text.
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    // Hovered: a subtle fill and brighter text mark the control under the pointer.
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, BORDER);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    // Active (pressed) and open (an open menu button): a stronger fill, brightest text.
    for s in [&mut w.active, &mut w.open] {
        s.weak_bg_fill = PRESSED;
        s.bg_fill = PRESSED;
        s.bg_stroke = Stroke::new(1.0, BORDER);
        s.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    }
    // One small, uniform rounding across every widget state.
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = CornerRadius::same(4);
    }
}
