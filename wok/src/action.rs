//! The editor's action vocabulary and its handler - the one seam every menu choice and keybind
//! routes through.
//!
//! The view emits [`Action`]s into a per-frame buffer rather than mutating state inside its egui
//! closures; the frame loop (`crate::main`) drains the buffer and applies each through [`handle`], so
//! the [`Model`] has a single writer. This is the action layer the editor grows on
//! (designs/editor-design.md): one vocabulary, one apply point, which is what later makes undo and
//! redo possible. [`handle`] is free of egui so the routing is unit testable without a window.
//!
//! This slice carries the navigation actions - switching the active view, toggling the panel, and
//! setting its dock side. The vocabulary and the `Handled`-style effect channel grow in later slices
//! as the actions that need them return.

use crate::model::{Model, NavSide, NavView};

/// A menu choice, keybind, or chrome interaction, emitted by the view and applied by [`handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Switch the navigation panel to show this view. Emitted by the bottom icon bar.
    SelectNavView(NavView),
    /// Show or hide the navigation panel. Emitted by the View menu.
    ToggleNavPanel,
    /// Dock the navigation panel to this side. Emitted by the View menu.
    SetNavSide(NavSide),
}

/// Apply one action to the model. The single point where a chrome interaction changes editor state.
pub fn handle(model: &mut Model, action: Action) {
    match action {
        Action::SelectNavView(view) => model.shell.select_nav(view),
        Action::ToggleNavPanel => model.shell.toggle_nav(),
        Action::SetNavSide(side) => model.shell.set_nav_side(side),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_nav_view_switches_the_active_view() {
        let mut model = Model::default();
        handle(&mut model, Action::SelectNavView(NavView::Prefabs));
        assert_eq!(model.shell.active_nav(), NavView::Prefabs);
    }

    #[test]
    fn select_nav_view_to_the_same_view_is_idempotent() {
        let mut model = Model::default();
        handle(&mut model, Action::SelectNavView(NavView::Lighting));
        handle(&mut model, Action::SelectNavView(NavView::Lighting));
        assert_eq!(model.shell.active_nav(), NavView::Lighting);
    }

    #[test]
    fn select_nav_view_reaches_every_view() {
        let mut model = Model::default();
        for view in [NavView::Scenes, NavView::Prefabs, NavView::Instances, NavView::Lighting] {
            handle(&mut model, Action::SelectNavView(view));
            assert_eq!(model.shell.active_nav(), view);
        }
    }

    #[test]
    fn toggle_nav_panel_flips_visibility() {
        let mut model = Model::default();
        assert!(model.shell.nav_visible());
        handle(&mut model, Action::ToggleNavPanel);
        assert!(!model.shell.nav_visible());
        handle(&mut model, Action::ToggleNavPanel);
        assert!(model.shell.nav_visible());
    }

    #[test]
    fn set_nav_side_docks_the_panel() {
        let mut model = Model::default();
        handle(&mut model, Action::SetNavSide(NavSide::Right));
        assert_eq!(model.shell.nav_side(), NavSide::Right);
        handle(&mut model, Action::SetNavSide(NavSide::Left));
        assert_eq!(model.shell.nav_side(), NavSide::Left);
    }
}
