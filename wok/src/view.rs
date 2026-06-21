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
//! Region order is layout order and load-bearing (see the README shell layout, sharp-edges 2): the
//! navigation panel is shown first so it claims its full-height strip, then the view column - the
//! status bar at the bottom, the tab bar at the top, the editor well filling the rest - spans only the
//! remaining width, so the status bar never runs under the nav. This holds whichever side the panel
//! docks to: it is added before the view column on either side. When the panel is hidden the view
//! column spans the full width. The status bar now reads the open project (its name, or that none is
//! open); the editor well is still a static placeholder; the nav panel, the status bar, and the tab
//! bar's hamburger (now with a wired File menu) read the model and emit actions.

use crate::action::Action;
use crate::menu;
use crate::model::Model;
use crate::workspace;

/// Render the full editor chrome for one frame: the navigation panel first (full height on its docked
/// side, and only when visible), then the view column's status bar, tab bar, and editor well. Returns
/// the actions the regions emitted this frame, for the caller to apply through `crate::action::handle`.
pub fn chrome(ctx: &egui::Context, model: &Model) -> Vec<Action> {
    let mut actions = Vec::new();
    // Region order is load-bearing (sharp-edges 2): the nav panel is added first on whichever side it
    // docks, so it claims its full-height strip and the view column fills the rest - the status bar
    // never runs under the nav. Hidden, the view column spans the full width.
    if model.shell.nav_visible() {
        workspace::nav_panel(ctx, model, &mut actions);
    }
    menu::status_bar(ctx, model.project.as_ref());
    workspace::tab_bar(ctx, model, &mut actions);
    workspace::editor_area(ctx);
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NavSide, NavView};
    use crate::project::Project;
    use crate::recent::Recents;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::path::PathBuf;
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

    /// Render `model`'s chrome to a PNG under `tests/snapshots`, through the composition root the app
    /// renders through, so the PNG tracks the real chrome. Refresh after an intended look change with
    /// `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    fn snapshot_of(name: &str, theme: egui::ThemePreference, model: Model) {
        let _gpu = gpu_guard();
        let mut harness = chrome_harness(theme, egui::vec2(1100.0, 700.0), model);
        harness.run();
        harness.snapshot(name);
    }

    /// Snapshot the chrome with `active` as the navigation view - the common case where only the active
    /// view varies.
    fn snapshot(name: &str, theme: egui::ThemePreference, active: NavView) {
        snapshot_of(name, theme, model_with(active));
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

    /// The navigation panel hidden: the view column - tab bar, editor well, status bar - spans the full
    /// width with no nav strip. Dark alone is enough; this is a layout state, not a palette one (the
    /// themes are guarded by the pairs above).
    #[test]
    fn chrome_nav_hidden_snapshot() {
        let mut model = Model::default();
        model.shell.toggle_nav();
        snapshot_of("chrome_nav_hidden", egui::ThemePreference::Dark, model);
    }

    /// The navigation panel docked right: the nav strip on the right edge, the view column (with its
    /// status bar) confined to the remaining left width - the region-order rule holding on the right
    /// side, not just the left.
    #[test]
    fn chrome_nav_docked_right_snapshot() {
        let mut model = Model::default();
        model.shell.set_nav_side(NavSide::Right);
        snapshot_of("chrome_nav_docked_right", egui::ThemePreference::Dark, model);
    }

    /// The app-menu open at its View submenu, driven by clicking the hamburger then hovering View (the
    /// top button opens on an accesskit click, but egui opens a submenu on pointer hover - sharp-edges
    /// 3). Guards that the menu renders and the View items are present: Hide Navigation Panel (the label
    /// tracks the default visible state) and the Dock Left / Dock Right radios with Left marked.
    #[test]
    fn chrome_view_menu_snapshot() {
        let _gpu = gpu_guard();
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), Model::default());
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        harness.get_by_label("View").hover();
        harness.run();
        harness.snapshot("chrome_view_menu");
    }

    /// A project open: the status bar's left shows the project's name (its folder leaf) rather than
    /// "No project open" - the in-window confirmation that Open took effect, mirroring the window
    /// title. Dark alone is enough; this is a content state, not a palette one.
    #[test]
    fn chrome_project_open_snapshot() {
        let model = Model { project: Some(Project::new("C:/games/MyGame")), ..Default::default() };
        snapshot_of("chrome_project_open", egui::ThemePreference::Dark, model);
    }

    /// The app-menu open at its File submenu, driven by clicking the hamburger then hovering File
    /// (egui opens a submenu on hover, not on an accesskit click - sharp-edges 3). Built over a model
    /// with a project open and a couple of recents, so every File item is live: Open Project..., Open
    /// Recent (enabled, with entries), and Close Project (enabled because a project is open).
    #[test]
    fn chrome_file_menu_snapshot() {
        let _gpu = gpu_guard();
        let model = Model {
            project: Some(Project::new("C:/games/MyGame")),
            recents: Recents::from_paths(["C:/games/MyGame", "C:/games/Other"].iter().map(PathBuf::from)),
            ..Default::default()
        };
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model);
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        harness.get_by_label("File").hover();
        harness.run();
        harness.snapshot("chrome_file_menu");
    }

    /// The Open Recent submenu open, descended Menu -> File -> Open Recent (each level opens on hover -
    /// sharp-edges 3). Built over a model with two recents, so the submenu lists them and the new Clear
    /// Recently Opened item sits enabled at the foot below a separator. Guards the Clear affordance and
    /// the submenu contents.
    #[test]
    fn chrome_open_recent_menu_snapshot() {
        let _gpu = gpu_guard();
        let model = Model {
            recents: Recents::from_paths(["C:/games/MyGame", "C:/games/Other"].iter().map(PathBuf::from)),
            ..Default::default()
        };
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model);
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        harness.get_by_label("File").hover();
        harness.run();
        harness.get_by_label("Open Recent").hover();
        harness.run();
        harness.snapshot("chrome_open_recent_menu");
    }
}
