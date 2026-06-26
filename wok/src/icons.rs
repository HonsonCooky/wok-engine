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

/// The target ink height for a chrome icon, in points: every glyph is scaled so the height of its
/// actual ink (not its em box) equals this, then centred on that ink. MDI glyphs fill the em by
/// different amounts - a cube or layers fills it, a list or the hamburger is wide-and-short - so one
/// font size renders them at visibly different sizes; normalizing the ink height makes the row read as
/// one set. Kept small so the icons sit at the weight of the surrounding UI text. (This supersedes the
/// earlier one-uniform-font-size approach noted in sharp-edges; it is automatic, not per-glyph hand
/// tuning, so it does not reintroduce the fiddliness that one was reverted for.)
pub const SIZE: f32 = 12.0;

/// The reference font size the glyph's ink is measured at before rescaling to [`SIZE`]. Ink scales
/// linearly with font size, so the exact value does not matter; a larger one just measures over more
/// pixels. The measured-then-rescaled galleys are both stable cache keys, so egui caches them.
const MEASURE_SIZE: f32 = 24.0;

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
/// `nf-md-folder` - a collapsed prefab group row in the Instances tree. The closed folder is the
/// group's collapsed state; [`FOLDER_OPEN`] is its expanded state - the folder glyph is the disclosure
/// now (the chevron was dropped), so the whole group row toggles between the two.
pub const FOLDER: char = '\u{f024b}';
/// `nf-md-folder-open` - an expanded prefab group row in the Instances tree (see [`FOLDER`]).
pub const FOLDER_OPEN: char = '\u{f0770}';
/// `nf-md-cube` - an instance (placement) row in the Instances tree. The filled cube, distinct from
/// the Prefabs nav view's [`CUBE_OUTLINE`], so a placed instance reads as solid against the outline
/// the library uses.
pub const CUBE: char = '\u{f01a6}';
/// `nf-md-refresh` - the inspector's per-row reset affordance (Rotation -> identity, Scale -> one). A
/// circular-arrow mark reads as "reset to default"; the row's tooltip names which.
pub const RESET: char = '\u{f0450}';

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

/// The font size to render a glyph at so its ink stands [`SIZE`] tall, given its ink measured at
/// [`MEASURE_SIZE`]. Factored out so the painter and the uniformity test apply one rule.
fn font_for_ink_height(ink_at_measure: egui::Vec2) -> f32 {
    MEASURE_SIZE * SIZE / ink_at_measure.y.max(1.0)
}

/// Paint an icon glyph centred in `rect`, in `color`, normalized to the chrome icon [`SIZE`]: measure
/// the glyph's tight ink, lay it out again scaled so its ink height is `SIZE`, and centre on the ink
/// (not the em) so a glyph whose ink sits high or low still lands centred. Positions are rounded to the
/// pixel grid so the mark stays crisp.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, glyph: char, color: egui::Color32) {
    let probe = painter.layout_no_wrap(glyph.to_string(), egui::FontId::proportional(MEASURE_SIZE), color);
    let font = font_for_ink_height(probe.mesh_bounds.size());
    let galley = painter.layout_no_wrap(glyph.to_string(), egui::FontId::proportional(font), color);
    let pos = (rect.center() - galley.mesh_bounds.center().to_vec2()).round();
    painter.galley(pos, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "same size" guard: every glyph the chrome paints through [`paint`] - the hamburger, the
    /// four nav-bar icons, and the Instances tree's folder pair and cube - normalizes to one ink
    /// height, regardless of how much of the em its shape fills. Measures each glyph's ink at the
    /// rescaled font and asserts it lands on `SIZE` within a pixel of rounding/hinting slack. A wrong
    /// codepoint (a glyph the bundled font lacks) renders as zero-ink tofu and trips this assert, so it
    /// doubles as a codepoint-exists check. The tab close glyph is excluded: it renders as a sized
    /// label, not through `paint`.
    #[test]
    fn painted_glyphs_share_one_ink_height() {
        let ctx = egui::Context::default();
        install_font(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| {
                for glyph in [MENU, LAYERS, CUBE_OUTLINE, LIST_BULLETED, WEATHER_SUNNY, FOLDER, FOLDER_OPEN, CUBE, RESET] {
                    let probe = fonts.layout_no_wrap(glyph.to_string(), egui::FontId::proportional(MEASURE_SIZE), egui::Color32::WHITE);
                    let font = font_for_ink_height(probe.mesh_bounds.size());
                    let galley = fonts.layout_no_wrap(glyph.to_string(), egui::FontId::proportional(font), egui::Color32::WHITE);
                    let height = galley.mesh_bounds.size().y;
                    assert!((height - SIZE).abs() <= 1.0, "glyph {glyph:?} ink height {height:.2} is not ~{SIZE}");
                }
            });
        });
    }
}
