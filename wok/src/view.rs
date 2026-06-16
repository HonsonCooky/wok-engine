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

/// Render the full editor chrome for one frame. Order is layout order: the top bar and the status bar
/// claim the edges, then the workspace (navigation panel, tab bar, editor area) fills what is left.
pub fn chrome(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    menu::menu_bar(ctx, &model.shell, actions);
    menu::status_bar(ctx, &model.project);
    workspace::ui(ctx, &model.shell, actions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use crate::project::Project;
    use egui_kittest::Harness;

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
            let mut actions = Vec::new();
            chrome(ctx, &model, &mut actions);
        });
        harness.run();
        harness.snapshot("chrome");
    }
}
