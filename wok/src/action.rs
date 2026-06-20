//! The editor's action vocabulary and its handler - the one seam every menu choice and keybind
//! routes through.
//!
//! The view emits [`Action`]s into a per-frame buffer rather than mutating state inside its egui
//! closures; the frame loop (`crate::main`) drains the buffer and applies each through [`handle`], so
//! the [`Model`] has a single writer. This is the action layer the editor grows on
//! (designs/editor-design.md): one vocabulary, one apply point, which is what later makes undo and
//! redo possible. [`handle`] is free of egui so the routing is unit testable without a window.
//!
//! This slice carries one action - switching the active navigation view. The vocabulary and the
//! `Handled`-style effect channel grow in later slices as the actions that need them return.

use crate::model::{Model, NavView};

/// A menu choice, keybind, or chrome interaction, emitted by the view and applied by [`handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Switch the navigation panel to show this view. Emitted by the bottom icon bar.
    SelectNavView(NavView),
}

/// Apply one action to the model. The single point where a chrome interaction changes editor state.
pub fn handle(model: &mut Model, action: Action) {
    match action {
        Action::SelectNavView(view) => model.shell.select_nav(view),
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
}
