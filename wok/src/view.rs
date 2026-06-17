//! The chrome composition root: the one place the editor's regions are ordered, and the single entry
//! both the live frame loop (`crate::app`) and the snapshot test render through. Sharing this seam is
//! what makes the snapshot a real regression guard - the PNG shows exactly what the app draws,
//! because both go through `chrome`.
//!
//! Like the regions it calls, this layer only reads the model (and the open project's content
//! summary) and emits actions; the handler (`crate::action`) is the single writer. The 3D scene is
//! drawn behind this chrome by the GPU render pass (`crate::render`), not here; what egui draws is
//! the frame around the viewport.

use crate::action::Action;
use crate::menu;
use crate::mode::Mode;
use crate::model::Model;
use crate::scene::ContentView;
use crate::workspace;

/// Render the full editor chrome for one frame. Order is layout order: the status bar claims the
/// bottom edge, then the workspace (navigation panel with the content browser, the tab bar with the
/// app-menu at its left, and the editor area) fills what is left. `content` is the open project's
/// content summary (or `None` when no project is open), and `mode` is the interaction mode the status
/// bar shows.
///
/// `open_error` is the last project-open failure's message (or `None`), shown in the status bar.
///
/// Returns the editor-area rect (egui points) the chrome settled into this frame. The frame loop
/// keeps it so the GPU pass can confine the 3D to that rect (`crate::render`); it depends on the
/// live layout (nav-panel dock and visibility, the window size), so it is read fresh each frame.
pub fn chrome(
    ctx: &egui::Context,
    model: &Model,
    content: Option<ContentView>,
    mode: Mode,
    open_error: Option<&str>,
    actions: &mut Vec<Action>,
) -> egui::Rect {
    menu::status_bar(ctx, &model.project, mode, open_error);
    workspace::ui(ctx, model, content, actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use crate::model::Model;
    use crate::project::Project;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::sync::{Mutex, MutexGuard};

    /// Serializes the wgpu snapshot tests below. egui_kittest builds a fresh headless wgpu device per
    /// harness, and creating or tearing several down concurrently crashes on some Windows drivers - a
    /// wgpu teardown race, not a fault in the chrome. Each GPU test holds this lock for its lifetime,
    /// so only one device is ever alive at a time; the pure-logic tests elsewhere stay parallel. Poison
    /// is ignored, so a failed snapshot assert does not cascade into the rest.
    static GPU_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the GPU-test lock, recovering from a poisoned mutex (a prior test's panic) so the lock
    /// still serializes rather than failing every later test.
    fn gpu_guard() -> MutexGuard<'static, ()> {
        GPU_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A stand-in content summary for the snapshots, with a couple of prefabs and one lighting state,
    /// matching what the sample project lists. Held by the caller so a `ContentView` can borrow it.
    fn demo_listings() -> (Vec<String>, Vec<String>) {
        let prefabs = ["boulder", "crate", "marker", "pillar"].iter().map(|s| (*s).to_string()).collect();
        let lights = vec!["default".to_string()];
        (prefabs, lights)
    }

    /// Build a harness that renders the chrome at `size` under `theme`, with the editor surface
    /// filled behind the transparent editor area (standing in for the in-app GPU scene render), so the
    /// snapshot reads as it does live in that theme. Forcing the theme keeps each snapshot
    /// deterministic regardless of the host's OS setting.
    fn chrome_harness<'a>(
        model: &'a Model,
        content: Option<ContentView<'a>>,
        mode: Mode,
        theme: egui::ThemePreference,
        size: egui::Vec2,
    ) -> Harness<'a> {
        Harness::builder().with_size(size).wgpu().build(move |ctx| {
            crate::theme::apply(ctx);
            ctx.set_theme(theme);
            let editor_bg = crate::theme::palette(ctx).editor_bg;
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, editor_bg);
            let mut actions = Vec::new();
            chrome(ctx, model, content, mode, None, &mut actions);
        })
    }

    /// Render the chrome in a representative state - a project open with its scene tab, the content
    /// browser populated, the navigation panel shown - to `tests/snapshots/chrome.png`. Both the
    /// editor's eyes and a regression guard: the same view functions the app calls render here,
    /// through `chrome`, so the PNG tracks the real chrome. Refresh after an intended look change with
    /// `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    #[test]
    fn chrome_snapshot() {
        let _gpu = gpu_guard();
        let (prefabs, lights) = demo_listings();
        let content = ContentView { scene_name: "sample", prefabs: &prefabs, lights: &lights };
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_or_focus_scene();
        let mut harness =
            chrome_harness(&model, Some(content), Mode::Object, egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0));
        harness.run();
        harness.snapshot("chrome");
    }

    /// The same chrome in light mode, guarding that the light palette mirrors the dark one's
    /// structure and reads cleanly.
    #[test]
    fn chrome_light_snapshot() {
        let _gpu = gpu_guard();
        let (prefabs, lights) = demo_listings();
        let content = ContentView { scene_name: "sample", prefabs: &prefabs, lights: &lights };
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_or_focus_scene();
        let mut harness =
            chrome_harness(&model, Some(content), Mode::Object, egui::ThemePreference::Light, egui::vec2(1100.0, 700.0));
        harness.run();
        harness.snapshot("chrome_light");
    }

    /// Open the app-menu (the hamburger), then its View submenu, and snapshot it. Confirms the
    /// hamburger menu and that items size to content: the long "Show Navigation Panel" item and its
    /// Ctrl+B hint sit on one line, not wrapped. The nav panel is hidden so the menu opens at the
    /// window's left edge, keeping the canvas focused.
    #[test]
    fn view_menu_open_snapshot() {
        let _gpu = gpu_guard();
        let (prefabs, lights) = demo_listings();
        let content = ContentView { scene_name: "sample", prefabs: &prefabs, lights: &lights };
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.toggle_nav();
        let mut harness =
            chrome_harness(&model, Some(content), Mode::Object, egui::ThemePreference::Dark, egui::vec2(520.0, 320.0));
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        // egui opens a submenu on pointer hover, not on an accesskit click - so hover View.
        harness.get_by_label("View").hover();
        harness.run();
        harness.snapshot("view_menu_open");
    }

    /// Open the app-menu, the File submenu, then the Open Recent submenu, and snapshot the cascade.
    /// Guards the project-lifecycle surface: the recent projects listed most-recent first with Clear
    /// Recent below them, and (in the File column behind) the Close Project entry, enabled because a
    /// project is open. The nav panel is hidden so the cascade has room to open rightward from the
    /// window's left edge.
    #[test]
    fn open_recent_menu_snapshot() {
        let _gpu = gpu_guard();
        let (prefabs, lights) = demo_listings();
        let content = ContentView { scene_name: "sample", prefabs: &prefabs, lights: &lights };
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.toggle_nav();
        model.recents.push("games/unstitched");
        model.recents.push("demos/taste");
        let mut harness =
            chrome_harness(&model, Some(content), Mode::Object, egui::ThemePreference::Dark, egui::vec2(760.0, 380.0));
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        // egui opens a submenu on pointer hover, not on an accesskit click - so hover to descend.
        harness.get_by_label("File").hover();
        harness.run();
        harness.get_by_label("Open Recent").hover();
        harness.run();
        harness.snapshot("open_recent_menu");
    }

    /// Hover the active tab's close button and snapshot it, guarding the icon buttons' hover
    /// affordance: a hovered icon button shows a filled background. (The pointing-hand cursor is the
    /// OS's, not in the image.)
    #[test]
    fn tab_close_hover_snapshot() {
        let _gpu = gpu_guard();
        let (prefabs, lights) = demo_listings();
        let content = ContentView { scene_name: "sample", prefabs: &prefabs, lights: &lights };
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_or_focus_scene();
        model.shell.toggle_nav();
        let mut harness =
            chrome_harness(&model, Some(content), Mode::Object, egui::ThemePreference::Dark, egui::vec2(460.0, 180.0));
        harness.run();
        harness.get_by_label("x").hover();
        harness.run();
        harness.snapshot("tab_close_hover");
    }
}
