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

    /// Build a harness that renders the chrome at `size` under `theme`, with the editor surface
    /// filled behind the transparent editor area (standing in for the in-app GPU clear), so the
    /// snapshot reads as it does live in that theme. Forcing the theme keeps each snapshot
    /// deterministic regardless of the host's OS setting.
    fn chrome_harness(model: &Model, theme: egui::ThemePreference, size: egui::Vec2) -> Harness<'_> {
        Harness::builder().with_size(size).wgpu().build(move |ctx| {
            crate::theme::apply(ctx);
            ctx.set_theme(theme);
            let editor_bg = crate::theme::palette(ctx).editor_bg;
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, editor_bg);
            let mut actions = Vec::new();
            chrome(ctx, model, &mut actions);
        })
    }

    /// Render the chrome in a representative state - a project open, two tabs with one active, the
    /// navigation panel shown - to `tests/snapshots/chrome.png`. Both the editor's eyes and a
    /// regression guard: the same view functions the app calls render here, through `chrome`, so the
    /// PNG tracks the real chrome. Refresh after an intended look change with
    /// `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    #[test]
    fn chrome_snapshot() {
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_tab();
        model.shell.open_tab();
        let mut harness = chrome_harness(&model, egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0));
        harness.run();
        harness.snapshot("chrome");
    }

    /// The same chrome in light mode, guarding that the light palette mirrors the dark one's
    /// structure and reads cleanly.
    #[test]
    fn chrome_light_snapshot() {
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_tab();
        model.shell.open_tab();
        let mut harness = chrome_harness(&model, egui::ThemePreference::Light, egui::vec2(1100.0, 700.0));
        harness.run();
        harness.snapshot("chrome_light");
    }

    /// Open the app-menu (the hamburger), then its View submenu, and snapshot it. Confirms the
    /// hamburger menu and that items size to content: the long "Show Navigation Panel" item and its
    /// Ctrl+B hint sit on one line, not wrapped. The nav panel is hidden so the menu opens at the
    /// window's left edge, keeping the canvas focused.
    #[test]
    fn view_menu_open_snapshot() {
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.toggle_nav();
        let mut harness = chrome_harness(&model, egui::ThemePreference::Dark, egui::vec2(520.0, 320.0));
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        // egui opens a submenu on pointer hover, not on an accesskit click - so hover View.
        harness.get_by_label("View").hover();
        harness.run();
        harness.snapshot("view_menu_open");
    }

    /// Hover the new-tab + and snapshot it, guarding the icon buttons' hover affordance: a hovered
    /// icon button shows a filled background. (The pointing-hand cursor is the OS's, not in the image.)
    #[test]
    fn tab_row_hover_snapshot() {
        let mut model = Model::new(Project::open("wok-engine"));
        model.shell.open_tab();
        model.shell.toggle_nav();
        let mut harness = chrome_harness(&model, egui::ThemePreference::Dark, egui::vec2(460.0, 180.0));
        harness.run();
        harness.get_by_label("+").hover();
        harness.run();
        harness.snapshot("tab_row_hover");
    }
}
