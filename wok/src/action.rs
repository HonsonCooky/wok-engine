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
    /// Open Project: set the current project to the folder at this path, and record it at the front of
    /// the recent list. The same action both the folder picker and an Open Recent entry emit, so
    /// reopening a recent is just opening its path - one obvious way, not a parallel code path.
    OpenProject(PathBuf),
    /// Close Project: return to the no-project state and clear the (project-scoped) open tabs.
    CloseProject,
    /// Clear Recent: empty the recent-projects list.
    ClearRecent,
    /// Quit: request a clean shutdown after this frame.
    Quit,

    // ---- shell layout ----
    /// Open the project's scene as a Scene tab and make it active, or focus the existing one (there
    /// is one scene per project). Emitted by the content browser's scene entry and by the auto-open
    /// when a project loads.
    OpenScene,
    /// Close this tab, activating a sensible neighbour if it was the active one.
    CloseTab(TabId),
    /// Make this tab active.
    SelectTab(TabId),
    /// Show or hide the navigation panel.
    ToggleNav,
    /// Dock the navigation panel to this side.
    SetNavSide(Side),
}

/// What [`handle`] asks the frame loop to carry out - the effects the pure model cannot perform
/// itself: closing the window, and persisting the recent-projects list. Empty by default; an action
/// sets only the field it needs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Handled {
    /// Close the window and exit after this frame.
    pub quit: bool,
    /// The recent-projects list changed; the frame loop should write it to disk (`crate::recent`).
    pub save_recents: bool,
}

/// Apply one action to the model, returning the frame-loop effects it implies. The single point
/// where a menu choice or keybind changes editor state.
pub fn handle(action: Action, model: &mut Model) -> Handled {
    match action {
        Action::OpenProject(root) => {
            model.project = Project::open(root.clone());
            model.recents.push(root);
            Handled { save_recents: true, ..Handled::default() }
        }
        Action::CloseProject => {
            model.project = Project::None;
            model.shell.close_all_tabs();
            Handled::default()
        }
        Action::ClearRecent => {
            model.recents.clear();
            Handled { save_recents: true, ..Handled::default() }
        }
        Action::Quit => Handled { quit: true, ..Handled::default() },
        // Stub: New Project has no effect until project creation is built. Kept in the vocabulary so
        // the menu routes through this seam today; the behavior drops in here when it returns.
        Action::NewProject => Handled::default(),
        Action::OpenScene => {
            model.shell.open_or_focus_scene();
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

    // ---- recent projects ----

    #[test]
    fn open_project_records_it_in_recents_and_asks_to_save() {
        let mut model = Model::default();
        let handled = handle(Action::OpenProject(PathBuf::from("games/unstitched")), &mut model);
        assert_eq!(model.recents.paths(), &[PathBuf::from("games/unstitched")]);
        assert!(handled.save_recents, "opening a project changes recents, so it should persist");
    }

    #[test]
    fn opening_projects_lists_them_most_recent_first() {
        let mut model = Model::default();
        handle(Action::OpenProject(PathBuf::from("a")), &mut model);
        handle(Action::OpenProject(PathBuf::from("b")), &mut model);
        assert_eq!(model.recents.paths(), &[PathBuf::from("b"), PathBuf::from("a")]);
    }

    #[test]
    fn reopening_a_project_dedups_to_a_single_front_entry() {
        let mut model = Model::default();
        handle(Action::OpenProject(PathBuf::from("a")), &mut model);
        handle(Action::OpenProject(PathBuf::from("b")), &mut model);
        handle(Action::OpenProject(PathBuf::from("a")), &mut model);
        // Reopening "a" moves it to the front without duplicating it.
        assert_eq!(model.recents.paths(), &[PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn clear_recent_empties_the_list_and_asks_to_save() {
        let mut model = Model::default();
        handle(Action::OpenProject(PathBuf::from("a")), &mut model);
        let handled = handle(Action::ClearRecent, &mut model);
        assert!(model.recents.paths().is_empty());
        assert!(handled.save_recents);
    }

    // ---- close project ----

    #[test]
    fn close_project_returns_to_no_project_and_clears_tabs() {
        let mut model = Model::new(Project::open("games/unstitched"));
        handle(Action::OpenScene, &mut model);
        let handled = handle(Action::CloseProject, &mut model);
        assert_eq!(model.project, Project::None);
        assert!(model.shell.tabs().is_empty(), "tabs are project-scoped and clear with the project");
        assert_eq!(model.shell.active(), None);
        assert!(!handled.quit);
    }

    #[test]
    fn close_project_keeps_recents_and_the_panel_layout() {
        let mut model = Model::default();
        handle(Action::OpenProject(PathBuf::from("games/unstitched")), &mut model);
        handle(Action::SetNavSide(Side::Right), &mut model);
        handle(Action::CloseProject, &mut model);
        assert_eq!(model.project, Project::None);
        // The closed project stays in recents (it was just opened), and the dock side is a workspace
        // preference, not project content, so the close leaves it be.
        assert_eq!(model.recents.paths(), &[PathBuf::from("games/unstitched")]);
        assert_eq!(model.shell.nav_side(), Side::Right);
    }

    // ---- tabs ----
    // The tab-strip mechanics (open / close-neighbour / select / close-all) are tested directly
    // against `Shell` in `crate::model`, where they live. Here we cover only the action seams: the
    // Scene tab open and the per-tab close/select the chrome emits.

    #[test]
    fn open_scene_opens_the_scene_tab_and_focuses_it_on_a_repeat() {
        let mut model = Model::new(Project::open("games/unstitched"));
        handle(Action::OpenScene, &mut model);
        assert_eq!(model.shell.tabs().len(), 1);
        let scene = model.shell.tabs()[0].id;
        assert_eq!(model.shell.active(), Some(scene));
        // A second open-scene (the content browser clicked again) focuses the same tab.
        handle(Action::OpenScene, &mut model);
        assert_eq!(model.shell.tabs().len(), 1, "one scene per project, never a duplicate");
        assert_eq!(model.shell.active(), Some(scene));
    }

    #[test]
    fn close_tab_then_open_scene_reopens_it() {
        let mut model = Model::new(Project::open("games/unstitched"));
        handle(Action::OpenScene, &mut model);
        let scene = model.shell.tabs()[0].id;
        handle(Action::CloseTab(scene), &mut model);
        assert!(model.shell.tabs().is_empty());
        // The content browser can reopen the scene after its tab was closed.
        handle(Action::OpenScene, &mut model);
        assert_eq!(model.shell.tabs().len(), 1);
        assert_eq!(model.shell.active(), model.shell.tabs().first().map(|t| t.id));
    }

    #[test]
    fn select_tab_makes_it_active() {
        let mut model = Model::new(Project::open("games/unstitched"));
        handle(Action::OpenScene, &mut model);
        let scene = model.shell.tabs()[0].id;
        handle(Action::SelectTab(scene), &mut model);
        assert_eq!(model.shell.active(), Some(scene));
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
