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
/// averages; the camera speed and the counts are live, read fresh each frame, so a scroll shows
/// in the readout on the very next frame.
pub struct Stats {
    pub fps: f32,
    pub frame_ms: f32,
    /// The fly camera's current movement speed in metres per second (scroll-adjusted).
    pub cam_speed: f32,
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
                ui.label(RichText::new(right_text(stats)).weak().small());
            });
        });
    });
}

/// The bar's right-side text: camera speed first (the one number the user steers directly, by
/// scroll), then the frame averages and the counts. One decimal on the speed: the scroll steps
/// are multiplicative (factor 1.3 from 1 to 200 m/s), so whole numbers would make small speeds
/// read as stuck while a decimal tracks every notch.
fn right_text(stats: &Stats) -> String {
    format!(
        "cam {:.1} m/s    {:.0} fps  {:.2} ms    {} placements  {} draws",
        stats.cam_speed, stats.fps, stats.frame_ms, stats.placement_count, stats.draw_items
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(cam_speed: f32) -> Stats {
        Stats { fps: 60.4, frame_ms: 16.55, cam_speed, placement_count: 8, draw_items: 12 }
    }

    #[test]
    fn the_camera_speed_leads_the_bar_text_at_one_decimal() {
        assert!(right_text(&stats(12.5)).starts_with("cam 12.5 m/s"));
        // A whole-number speed still shows the decimal place, so the field never changes width
        // class as the user scrolls through it.
        assert!(right_text(&stats(16.0)).starts_with("cam 16.0 m/s"));
        // The clamp ends of the scroll range format cleanly.
        assert!(right_text(&stats(1.0)).starts_with("cam 1.0 m/s"));
        assert!(right_text(&stats(200.0)).starts_with("cam 200.0 m/s"));
    }

    #[test]
    fn the_bar_text_keeps_the_frame_stats_after_the_speed() {
        let text = right_text(&stats(13.0));
        assert_eq!(text, "cam 13.0 m/s    60 fps  16.55 ms    8 placements  12 draws");
    }
}
