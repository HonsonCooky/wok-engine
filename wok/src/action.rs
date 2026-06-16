//! The editor's action vocabulary and its handler - the one seam every menu choice and keybind
//! routes through.
//!
//! The view emits [`Action`]s into a per-frame buffer rather than mutating state inside its
//! closures; the frame loop drains the buffer and applies each through [`handle`], so the model has
//! a single writer. This is the action layer the editor grows on (editor-design.md): one vocabulary,
//! one apply point, which is what makes undo and redo possible later. [`handle`] is pure over the
//! [`Model`] - it takes the action and the current model, mutates the model, and returns
//! [`Handled`], the part the frame loop must carry out itself (closing the window) which the pure
//! state cannot do. Free of egui so the routing is unit testable without a window.

use std::path::PathBuf;

use crate::model::{Model, Side, TabId};
use crate::project::Project;

/// A menu choice or keybind, emitted by the view and applied by [`handle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // ---- project lifecycle ----
    /// New Project: create and open a fresh project. A stub until the scene/save piece returns; it
    /// is in the vocabulary now so the menu wiring and the handler seam are exercised and tested.
    NewProject,
    /// Open Project: set the current project to the folder at this path.
    OpenProject(PathBuf),
    /// Quit: request a clean shutdown after this frame.
    Quit,

    // ---- shell layout ----
    /// Open a new untitled placeholder tab and make it active.
    OpenTab,
    /// Close this tab, activating a sensible neighbour if it was the active one.
    CloseTab(TabId),
    /// Make this tab active.
    SelectTab(TabId),
    /// Show or hide the navigation panel.
    ToggleNav,
    /// Dock the navigation panel to this side.
    SetNavSide(Side),
}

/// What [`handle`] asks the frame loop to carry out - the effect the pure model cannot perform
/// itself: closing the window. Empty by default; an action sets only the field it needs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Handled {
    /// Close the window and exit after this frame.
    pub quit: bool,
}

/// Apply one action to the model, returning the frame-loop effects it implies. The single point
/// where a menu choice or keybind changes editor state.
pub fn handle(action: Action, model: &mut Model) -> Handled {
    match action {
        Action::OpenProject(root) => {
            model.project = Project::open(root);
            Handled::default()
        }
        Action::Quit => Handled { quit: true },
        // Stub: New Project has no effect until project creation is built. Kept in the vocabulary so
        // the menu routes through this seam today; the behavior drops in here when it returns.
        Action::NewProject => Handled::default(),
        Action::OpenTab => {
            model.shell.open_tab();
            Handled::default()
        }
        Action::CloseTab(id) => {
            model.shell.close_tab(id);
            Handled::default()
        }
        Action::SelectTab(id) => {
            model.shell.select_tab(id);
            Handled::default()
        }
        Action::ToggleNav => {
            model.shell.toggle_nav();
            Handled::default()
        }
        Action::SetNavSide(side) => {
            model.shell.set_nav_side(side);
            Handled::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- project lifecycle ----

    #[test]
    fn open_project_sets_the_current_project() {
        let mut model = Model::default();
        let handled = handle(Action::OpenProject(PathBuf::from("games/unstitched")), &mut model);
        assert_eq!(model.project, Project::open("games/unstitched"));
        assert!(!handled.quit);
    }

    #[test]
    fn open_project_replaces_an_already_open_one() {
        let mut model = Model::new(Project::open("games/first"));
        handle(Action::OpenProject(PathBuf::from("games/second")), &mut model);
        assert_eq!(model.project, Project::open("games/second"));
    }

    #[test]
    fn quit_requests_shutdown_and_leaves_the_model_alone() {
        let mut model = Model::new(Project::open("games/unstitched"));
        let handled = handle(Action::Quit, &mut model);
        assert!(handled.quit);
        assert_eq!(model.project, Project::open("games/unstitched"));
    }

    #[test]
    fn new_project_is_a_no_op_for_now() {
        let mut model = Model::default();
        let handled = handle(Action::NewProject, &mut model);
        assert_eq!(model.project, Project::None);
        assert!(!handled.quit);
    }

    // ---- tabs ----

    /// The id of the tab at `index`. Panics if absent - a test-only assumption that the tab exists.
    fn tab_id(model: &Model, index: usize) -> TabId {
        model.shell.tabs()[index].id
    }

    #[test]
    fn open_tab_adds_and_activates_it() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model);
        assert_eq!(model.shell.tabs().len(), 1);
        assert_eq!(model.shell.active(), Some(tab_id(&model, 0)));
    }

    #[test]
    fn opening_a_second_tab_activates_the_new_one() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model);
        handle(Action::OpenTab, &mut model);
        assert_eq!(model.shell.tabs().len(), 2);
        assert_eq!(model.shell.active(), Some(tab_id(&model, 1)));
    }

    #[test]
    fn closing_the_active_tab_activates_its_right_neighbour() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model); // index 0
        handle(Action::OpenTab, &mut model); // index 1
        handle(Action::OpenTab, &mut model); // index 2, active
        let middle = tab_id(&model, 1);
        handle(Action::SelectTab(middle), &mut model);
        handle(Action::CloseTab(middle), &mut model);
        assert_eq!(model.shell.tabs().len(), 2);
        // The tab that was to the right (originally index 2) slid into index 1 and is now active.
        assert_eq!(model.shell.active(), Some(tab_id(&model, 1)));
    }

    #[test]
    fn closing_the_active_rightmost_tab_activates_the_new_last() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model);
        handle(Action::OpenTab, &mut model); // index 1, active (rightmost)
        let right = tab_id(&model, 1);
        handle(Action::CloseTab(right), &mut model);
        assert_eq!(model.shell.tabs().len(), 1);
        assert_eq!(model.shell.active(), Some(tab_id(&model, 0)));
    }

    #[test]
    fn closing_the_last_remaining_tab_clears_the_active_tab() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model);
        let only = tab_id(&model, 0);
        handle(Action::CloseTab(only), &mut model);
        assert!(model.shell.tabs().is_empty());
        assert_eq!(model.shell.active(), None);
    }

    #[test]
    fn closing_an_inactive_tab_leaves_the_active_tab_alone() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model); // index 0
        handle(Action::OpenTab, &mut model); // index 1, active
        let active = tab_id(&model, 1);
        let inactive = tab_id(&model, 0);
        handle(Action::CloseTab(inactive), &mut model);
        assert_eq!(model.shell.tabs().len(), 1);
        assert_eq!(model.shell.active(), Some(active));
    }

    #[test]
    fn select_tab_switches_the_active_tab() {
        let mut model = Model::default();
        handle(Action::OpenTab, &mut model); // index 0
        handle(Action::OpenTab, &mut model); // index 1, active
        let first = tab_id(&model, 0);
        handle(Action::SelectTab(first), &mut model);
        assert_eq!(model.shell.active(), Some(first));
    }

    // ---- navigation panel ----

    #[test]
    fn toggle_nav_flips_visibility() {
        let mut model = Model::default();
        assert!(model.shell.nav_visible(), "the panel starts shown");
        handle(Action::ToggleNav, &mut model);
        assert!(!model.shell.nav_visible());
        handle(Action::ToggleNav, &mut model);
        assert!(model.shell.nav_visible());
    }

    #[test]
    fn set_nav_side_docks_to_the_given_side() {
        let mut model = Model::default();
        assert_eq!(model.shell.nav_side(), Side::Left, "the panel starts on the left");
        handle(Action::SetNavSide(Side::Right), &mut model);
        assert_eq!(model.shell.nav_side(), Side::Right);
        handle(Action::SetNavSide(Side::Left), &mut model);
        assert_eq!(model.shell.nav_side(), Side::Left);
    }
}
