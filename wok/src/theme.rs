//! The editor's egui styling: flat, minimal chrome, tightened spacing - the Zed flavor within
//! egui's defaults.
//!
//! Applied once at startup, per built-in theme, via `Context::style_mut_of`, so the OS light/dark
//! follow (egui's default `ThemePreference::System`, fed by winit's theme events) keeps working
//! exactly as before: both variants carry the same geometry tweaks, and color adjustments are
//! derived from each variant's own palette rather than stated absolutely. Everything else stays
//! egui's default - the point is to remove visual weight (shadows, fat rounding, loud
//! separators), not to design a palette.

use egui::{CornerRadius, Margin, vec2};

/// Apply the editor look to both built-in themes.
pub fn apply(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            // Tightened, even spacing: rows read as a list, not a form.
            style.spacing.item_spacing = vec2(6.0, 3.0);
            style.spacing.button_padding = vec2(6.0, 2.0);
            style.spacing.window_margin = Margin::same(8);
            style.spacing.menu_margin = Margin::same(6);
            style.spacing.indent = 14.0;

            let v = &mut style.visuals;
            // Flat: no drop shadows anywhere, small uniform rounding, floating windows filled
            // like the fixed chrome so the details panel reads as part of the shell.
            v.window_shadow = egui::epaint::Shadow::NONE;
            v.popup_shadow = egui::epaint::Shadow::NONE;
            v.window_corner_radius = CornerRadius::same(4);
            v.menu_corner_radius = CornerRadius::same(4);
            v.window_fill = v.panel_fill;
            v.window_stroke.color = v.window_stroke.color.gamma_multiply(0.6);
            // Subtle separators: egui's separator takes the noninteractive bg_stroke.
            v.widgets.noninteractive.bg_stroke.color =
                v.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.5);
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = CornerRadius::same(3);
            }
        });
    }
}
