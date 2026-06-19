//! The editor chrome's app-menu hamburger and status bar.
//!
//! The OS owns the title bar (`crate::main`), so the editor's menu is a single hamburger button at
//! the left of the tab-bar row - always visible, since the tab bar always is, unlike the toggleable
//! navigation panel (Zed's grammar, not a horizontal menu bar). This slice draws the hamburger as a
//! static glyph only; the menu it opens (File / View / Run / Help) is a later slice. Every colour is
//! read through `theme::palette`, so the chrome follows the OS light/dark.

use crate::theme;

/// Status-bar height in points (README shell layout): one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 26.0;

/// Size of the hamburger button cell, in points.
const HAMBURGER_CELL: egui::Vec2 = egui::vec2(30.0, 22.0);

/// Font size for the hamburger glyph - its own knob, since the `nf-md-menu` ink fills the em
/// differently from the nav glyphs. Currently the nav icons' `icons::SIZE` (16px); bump it if the
/// hamburger should grow to read as the same visual size as them.
const HAMBURGER_GLYPH: f32 = 16.0;

/// The app-menu hamburger, drawn by the caller into the tab-bar row. A static `nf-md-menu` glyph for
/// this slice; the menu it opens lands with the project-lifecycle and view actions. Painted dim, the
/// way an idle control reads on the surface.
pub fn hamburger(ui: &mut egui::Ui) {
    let (rect, _response) = ui.allocate_exact_size(HAMBURGER_CELL, egui::Sense::hover());
    let color = theme::palette(ui.ctx()).text_dim;
    crate::icons::paint(ui.painter(), rect, crate::icons::MENU, color, HAMBURGER_GLYPH);
}

/// The bottom status bar, within the view column only (the composition root shows the navigation
/// panel first, so this bottom panel spans only the width right of it, never under the nav). Reads
/// contextual diagnostics in a built editor - mode, snap, counts, framerate, save state, integrity;
/// here it holds dim placeholder text at each end (left context, right diagnostics) to exercise the
/// layout. The richer readouts join as their features land.
pub fn status_bar(ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(STATUS_BAR_HEIGHT).show(ctx, |ui| {
        let dim = theme::palette(ui.ctx()).text_dim;
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("No project open").small().color(dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("snap 1 m / 5 deg").small().color(dim));
            });
        });
    });
}
