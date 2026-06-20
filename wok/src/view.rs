//! The chrome composition root: the one place the editor's regions are ordered, and the single entry
//! both the live frame loop (`crate::main`) and the snapshot test render through. Sharing this seam is
//! what makes the snapshot a real regression guard - the PNG shows exactly what the app draws, because
//! both go through `chrome`.
//!
//! The regions read the [`Model`](crate::model::Model) and emit [`Action`](crate::action::Action)s;
//! they never mutate the model themselves. `chrome` collects the frame's actions into a buffer and
//! returns it, and the caller applies each through `crate::action::handle` - the single writer. The
//! snapshot test renders the same `chrome` over a model it builds directly, so the PNG tracks exactly
//! what the app draws.
//!
//! Region order is layout order and load-bearing (see the README shell layout): the navigation panel
//! is shown first so it claims the full-height left strip, then the view column - the status bar at the
//! bottom, the tab bar at the top, the editor well filling the rest - spans only the remaining width,
//! so the status bar never runs under the nav. The status bar, tab bar, and editor well are still
//! static placeholders for this slice; only the navigation panel reads the model and emits actions.

use crate::action::Action;
use crate::menu;
use crate::model::Model;
use crate::workspace;

/// Render the full editor chrome for one frame: the navigation panel first (full height on the left),
/// then the view column's status bar, tab bar, and editor well. Returns the actions the regions
/// emitted this frame, for the caller to apply through `crate::action::handle`.
pub fn chrome(ctx: &egui::Context, model: &Model) -> Vec<Action> {
    let mut actions = Vec::new();
    workspace::nav_panel(ctx, model, &mut actions);
    menu::status_bar(ctx);
    workspace::tab_bar(ctx);
    workspace::editor_area(ctx);
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NavView;
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

    /// A model with `active` as the navigation view, built directly (not by faking a click) so each
    /// snapshot pins the chrome in a specific state - the icon-bar accent, the header label, and the
    /// placeholder body all track this view (sharp-edges 3: construct states by building the model).
    fn model_with(active: NavView) -> Model {
        let mut model = Model::default();
        model.shell.select_nav(active);
        model
    }

    /// Build a harness that renders the chrome at `size` under `theme` over `model`, with the editor
    /// surface filled behind the transparent editor well (standing in for the in-app GPU clear), so the
    /// snapshot reads as it does live in that theme. Forcing the theme keeps each snapshot deterministic
    /// regardless of the host's OS setting. The collected actions are discarded - a static render emits
    /// none.
    fn chrome_harness(theme: egui::ThemePreference, size: egui::Vec2, model: Model) -> Harness<'static> {
        Harness::builder().with_size(size).wgpu().build(move |ctx| {
            crate::theme::apply(ctx);
            ctx.set_theme(theme);
            let editor_bg = crate::theme::palette(ctx).editor_bg;
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, editor_bg);
            let _ = chrome(ctx, &model);
        })
    }

    /// Render the chrome to a PNG under `tests/snapshots`, through the composition root the app renders
    /// through, so the PNG tracks the real chrome. Refresh after an intended look change with
    /// `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    fn snapshot(name: &str, theme: egui::ThemePreference, active: NavView) {
        let _gpu = gpu_guard();
        let mut harness = chrome_harness(theme, egui::vec2(1100.0, 700.0), model_with(active));
        harness.run();
        harness.snapshot(name);
    }

    /// The default landing state - the Instances view active - in dark and light. The pair guards that
    /// the light palette mirrors the dark one's structure and reads cleanly.
    #[test]
    fn chrome_snapshot() {
        snapshot("chrome", egui::ThemePreference::Dark, NavView::Instances);
    }

    #[test]
    fn chrome_light_snapshot() {
        snapshot("chrome_light", egui::ThemePreference::Light, NavView::Instances);
    }

    /// A second view active (Scenes, the project group's first), in dark and light: the header label,
    /// the placeholder body, and the active-icon accent all move off Instances and onto Scenes, so the
    /// pair guards that the chrome tracks `model.shell.active_nav` rather than a fixed view.
    #[test]
    fn chrome_scenes_snapshot() {
        snapshot("chrome_scenes", egui::ThemePreference::Dark, NavView::Scenes);
    }

    #[test]
    fn chrome_scenes_light_snapshot() {
        snapshot("chrome_scenes_light", egui::ThemePreference::Light, NavView::Scenes);
    }
}
