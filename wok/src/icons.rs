//! Chrome iconography: one Nerd Font family for every glyph the editor's chrome draws, so the
//! hamburger, the nav-view icons, and the tab close read as one set rather than a grab-bag of
//! hand-painted marks.
//!
//! The glyphs come from the bundled "Symbols Nerd Font Mono" (`wok/assets/`, embedded below via
//! `include_bytes!`), registered as a FALLBACK behind egui's own UI text font: ordinary text renders
//! through the default font, and the icon codepoints - which the default font lacks - fall through to
//! the symbols font. We use one icon set, Material Design Icons (`nf-md-*`), whose codepoints sit in
//! the supplementary private-use area (plane 15); the values below are from the Nerd Fonts v3.4.0
//! cheat sheet (and match the bundled font's version).
//!
//! `install_font` is called from `theme::apply`, so both the live app and the snapshot test - the one
//! styling entry point both share - register the font before the chrome renders; the snapshot would
//! show empty boxes otherwise.

use std::sync::Arc;

/// The standard chrome icon size in points (the small ~16px Zed-scale mark the handoff calls for).
/// The nav-bar icons paint at this size; the hamburger paints a touch larger so its more compact
/// glyph reads as the same visual size (see `menu::HAMBURGER_GLYPH`).
pub const SIZE: f32 = 16.0;

// The `nf-md-*` codepoints in use (Material Design Icons set). Names map to the Nerd Fonts cheat
// sheet; keep this list to only the glyphs the chrome actually draws.
/// `nf-md-menu` - the app-menu hamburger.
pub const MENU: char = '\u{f035c}';
/// `nf-md-layers` - the Scenes nav view (project group).
pub const LAYERS: char = '\u{f0328}';
/// `nf-md-cube-outline` - the Prefabs nav view (project group).
pub const CUBE_OUTLINE: char = '\u{f01a7}';
/// `nf-md-format-list-bulleted` - the Instances nav view (this-scene group). Distinct from `MENU`,
/// so the hamburger and the Instances icon never read as the same mark.
pub const LIST_BULLETED: char = '\u{f0279}';
/// `nf-md-weather-sunny` - the Lighting nav view (this-scene group).
pub const WEATHER_SUNNY: char = '\u{f0599}';
/// `nf-md-close` - the tab close affordance.
pub const CLOSE: char = '\u{f0156}';

/// The bundled icons-only symbols font, embedded so the build needs no network and the asset is
/// version-pinned (see `wok/assets/README.md`).
const FONT: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");

/// Register the symbols font as a fallback on both default families, so icon codepoints render
/// through it while everything else stays the default UI font. Replaces the font set wholesale via
/// `set_fonts` (it rebuilds the atlas), so call it once at setup, not per frame.
pub fn install_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("nerd_symbols".to_owned(), Arc::new(egui::FontData::from_static(FONT)));
    // Append (not insert at front) so it is the LAST resort: the UI font wins for text, and only the
    // glyphs it lacks - the icon codepoints - fall through to the symbols font.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("nerd_symbols".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Paint an icon glyph centred in `rect`, in `color`, at `size` points. The position is rounded to
/// the pixel grid so the mark stays crisp. `size` is per-call because glyphs differ in how much of
/// the em their ink fills: the compact `menu` glyph needs a larger size to read as the same visual
/// size as the fuller nav glyphs.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, glyph: char, color: egui::Color32, size: f32) {
    let galley = painter.layout_no_wrap(glyph.to_string(), egui::FontId::proportional(size), color);
    let pos = (rect.center() - galley.size() * 0.5).round();
    painter.galley(pos, galley, color);
}
