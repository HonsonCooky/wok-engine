//! The status bar: a thin strip along the bottom with the page toggles at the left and the
//! compact frame stats at the right.
//!
//! This is where the floating stats overlay retired to: the numbers were chrome, not scene
//! content, so they belong in the shell's one fixed strip rather than floating over the viewport.
//! The page toggles live here too (the Zed shape: panel pickers in the status bar), drawn as
//! painter glyphs with tooltips - see `crate::glyphs` for why they are line art and not icon-font
//! characters.

use egui::{Align, Layout, RichText, Sense, vec2};

use crate::glyphs;
use crate::pages::{Page, PageState};

/// The numbers the right side of the bar shows. fps and frame-ms are the app's one-second-window
/// averages; the counts are live.
pub struct Stats {
    pub fps: f32,
    pub frame_ms: f32,
    pub placement_count: usize,
    pub draw_items: usize,
}

/// Bar height in points: one row of small text plus breathing room.
const BAR_HEIGHT: f32 = 24.0;

/// Page-toggle glyph size in points.
const TOGGLE_SIZE: f32 = 18.0;

/// Build the status bar. Added to the context before the side panel so it spans the full window
/// width and the panel stacks above it.
pub fn bar(ctx: &egui::Context, pages: &mut PageState, stats: &Stats) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            for page in Page::ALL {
                page_toggle(ui, pages, page);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = format!(
                    "{:.0} fps  {:.2} ms    {} placements  {} draws",
                    stats.fps, stats.frame_ms, stats.placement_count, stats.draw_items
                );
                ui.label(RichText::new(text).weak().small());
            });
        });
    });
}

/// One page toggle: a glyph button, highlighted while its page is current, disabled for the
/// reserved scan slot (`PageState::select` refuses it too; the disabled look is the courtesy).
fn page_toggle(ui: &mut egui::Ui, pages: &mut PageState, page: Page) {
    let sense = if page.enabled() { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(TOGGLE_SIZE + 6.0, TOGGLE_SIZE), sense);
    let selected = pages.current() == page;
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        ui.painter().rect_filled(rect.expand(1.0), 3.0, visuals.weak_bg_fill);
    }
    let color = if page.enabled() {
        visuals.fg_stroke.color
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    };
    glyphs::page(ui.painter(), rect, page, color);
    if response.on_hover_text(page.tooltip()).clicked() {
        pages.select(page);
    }
}
