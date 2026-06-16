//! The chrome composition root: the one place the editor's regions are ordered, and the single entry
//! both the live frame loop (`crate::app`) and the snapshot test render through. Sharing this seam is
//! what makes the snapshot a real regression guard - the PNG shows exactly what the app draws,
//! because both go through `chrome`.
//!
//! Like the regions it calls, this layer only reads the model and emits actions; the handler
//! (`crate::action`) is the single writer.

use crate::action::Action;
use crate::menu;
use crate::model::Model;
use crate::workspace;

/// Render the full editor chrome for one frame. Order is layout order: the status bar claims the
/// bottom edge, then the workspace (navigation panel, the tab bar with the app-menu at its left, and
/// the editor area) fills what is left.
pub fn chrome(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    menu::status_bar(ctx, &model.project);
    workspace::ui(ctx, &model.shell, actions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use crate::project::Project;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// Render the chrome in a representative state - a project open, two tabs with one active, the
    /// navigation panel shown - to `tests/snapshots/chrome.png`. This is both the editor's eyes and a
    /// regression guard: the same view functions the app calls render here, through `chrome`, so the
    /// PNG tracks the real chrome. After an intended look change, refresh it with
    /// `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    ///
    /// Renders the egui chrome only - no GPU viewport clear and no 3D - so the editor area shows the
    /// test renderer's default background rather than the in-app clear. The menu, panels, tabs, and
    /// theme are what this guards.
    #[test]
    fn chrome_snapshot() {
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_tab();
        model.shell.open_tab();

        let mut harness = Harness::builder().with_size(egui::vec2(1100.0, 700.0)).wgpu().build(|ctx| {
            crate::theme::apply(ctx);
            // Stand in for the in-app GPU clear: fill the screen with the editor surface so the
            // transparent editor area reads the same dark tone here as it does live.
            ctx.layer_painter(egui::LayerId::background())
                .rect_filled(ctx.screen_rect(), 0.0, crate::theme::EDITOR_BG);
            let mut actions = Vec::new();
            chrome(ctx, &model, &mut actions);
        });
        harness.run();
        harness.snapshot("chrome");
    }

    /// Open the app-menu (the hamburger), then its View submenu, and snapshot it. Confirms the
    /// hamburger menu and that items size to content: the long "Show Navigation Panel" item and its
    /// Ctrl+B hint sit on one line, not wrapped. The nav panel is hidden so the menu opens at the
    /// window's left edge, keeping the canvas focused.
    #[test]
    fn view_menu_open_snapshot() {
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.toggle_nav();

        let mut harness = Harness::builder().with_size(egui::vec2(520.0, 320.0)).wgpu().build(|ctx| {
            crate::theme::apply(ctx);
            ctx.layer_painter(egui::LayerId::background())
                .rect_filled(ctx.screen_rect(), 0.0, crate::theme::EDITOR_BG);
            let mut actions = Vec::new();
            chrome(ctx, &model, &mut actions);
        });
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        // egui opens a submenu on pointer hover, not on an accesskit click - so hover View.
        harness.get_by_label("View").hover();
        harness.run();
        harness.snapshot("view_menu_open");
    }
}
