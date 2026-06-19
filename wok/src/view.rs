//! The chrome composition root: the one place the editor's regions are ordered, and the single entry
//! both the live frame loop (`crate::main`) and the snapshot test render through. Sharing this seam is
//! what makes the snapshot a real regression guard - the PNG shows exactly what the app draws, because
//! both go through `chrome`.
//!
//! This slice is the static shell framing: the five regions are themed placeholders with no behavior
//! and no model. Region order is layout order and load-bearing (see the README shell layout): the
//! navigation panel is shown first so it claims the full-height left strip, then the view column - the
//! status bar at the bottom, the tab bar at the top, the editor well filling the rest - spans only the
//! remaining width, so the status bar never runs under the nav. When the model + action seam returns,
//! the regions read it and emit actions here; today they render statically.

use crate::menu;
use crate::workspace;

/// Render the full editor chrome for one frame: the navigation panel first (full height on the left),
/// then the view column's status bar, tab bar, and editor well.
pub fn chrome(ctx: &egui::Context) {
    workspace::nav_panel(ctx);
    menu::status_bar(ctx);
    workspace::tab_bar(ctx);
    workspace::editor_area(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use std::sync::{Mutex, MutexGuard};

    /// Serializes the wgpu snapshot tests. egui_kittest builds a fresh headless wgpu device per
    /// harness, and creating or tearing several down concurrently crashes on some Windows drivers - a
    /// wgpu teardown race, not a fault in the chrome. Each GPU test holds this lock for its lifetime,
    /// so only one device is ever alive at a time. Poison is ignored, so a failed snapshot assert does
    /// not cascade into the rest.
    static GPU_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the GPU-test lock, recovering from a poisoned mutex (a prior test's panic) so the lock
    /// still serializes rather than failing every later test.
    fn gpu_guard() -> MutexGuard<'static, ()> {
        GPU_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Build a harness that renders the chrome at `size` under `theme`, with the editor surface filled
    /// behind the transparent editor well (standing in for the in-app GPU clear), so the snapshot reads
    /// as it does live in that theme. Forcing the theme keeps each snapshot deterministic regardless of
    /// the host's OS setting.
    fn chrome_harness(theme: egui::ThemePreference, size: egui::Vec2) -> Harness<'static> {
        Harness::builder().with_size(size).wgpu().build(move |ctx| {
            crate::theme::apply(ctx);
            ctx.set_theme(theme);
            let editor_bg = crate::theme::palette(ctx).editor_bg;
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, editor_bg);
            chrome(ctx);
        })
    }

    /// Render the full chrome in dark mode to `tests/snapshots/chrome.png`: all five regions through
    /// the composition root the app renders through, so the PNG tracks the real chrome. Refresh after
    /// an intended look change with `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    #[test]
    fn chrome_snapshot() {
        let _gpu = gpu_guard();
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0));
        harness.run();
        harness.snapshot("chrome");
    }

    /// The same chrome in light mode, guarding that the light palette mirrors the dark one's structure
    /// and reads cleanly.
    #[test]
    fn chrome_light_snapshot() {
        let _gpu = gpu_guard();
        let mut harness = chrome_harness(egui::ThemePreference::Light, egui::vec2(1100.0, 700.0));
        harness.run();
        harness.snapshot("chrome_light");
    }
}
