//! The menu's action vocabulary and its handler - the one seam every File-menu choice routes
//! through.
//!
//! The menu emits [`Action`]s into a per-frame buffer rather than mutating state inside its
//! closures; the frame loop drains the buffer and applies each through [`handle`], so project state
//! has a single writer. This is the smallest form of the action layer the editor grows back into
//! (editor-design.md): one vocabulary, one apply point. [`handle`] is pure over the project - it
//! takes the action and the current project, mutates the project, and returns [`Handled`], the part
//! the frame loop must carry out itself (closing the window) which the pure state cannot do. Free
//! of egui so the routing is unit testable without a window.

use std::path::PathBuf;

use crate::project::Project;

/// A File-menu choice, emitted by the menu and applied by [`handle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// New Project: create and open a fresh project. A stub until the scene/save piece returns; it
    /// is in the vocabulary now so the menu wiring and the handler seam are exercised and tested.
    NewProject,
    /// Open Project: set the current project to the folder at this path.
    OpenProject(PathBuf),
    /// Quit: request a clean shutdown after this frame.
    Quit,
}

/// What [`handle`] asks the frame loop to carry out - effects the pure project state cannot perform
/// itself. Empty by default; an action sets only the field it needs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Handled {
    /// Close the window and exit after this frame.
    pub quit: bool,
}

/// Apply one action to the project, returning the frame-loop effects it implies. The single point
/// where a menu choice changes project state.
pub fn handle(action: Action, project: &mut Project) -> Handled {
    match action {
        Action::OpenProject(root) => {
            *project = Project::open(root);
            Handled::default()
        }
        Action::Quit => Handled { quit: true },
        // Stub: New Project has no effect until project creation is built. Kept in the vocabulary so
        // the menu routes through this seam today; the behavior drops in here when it returns.
        Action::NewProject => Handled::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_project_sets_the_current_project() {
        let mut project = Project::None;
        let handled = handle(Action::OpenProject(PathBuf::from("games/unstitched")), &mut project);
        assert_eq!(project, Project::open("games/unstitched"));
        assert!(!handled.quit);
    }

    #[test]
    fn open_project_replaces_an_already_open_one() {
        let mut project = Project::open("games/first");
        handle(Action::OpenProject(PathBuf::from("games/second")), &mut project);
        assert_eq!(project, Project::open("games/second"));
    }

    #[test]
    fn quit_requests_shutdown_and_leaves_the_project_alone() {
        let mut project = Project::open("games/unstitched");
        let handled = handle(Action::Quit, &mut project);
        assert!(handled.quit);
        assert_eq!(project, Project::open("games/unstitched"));
    }

    #[test]
    fn new_project_is_a_no_op_for_now() {
        let mut project = Project::None;
        let handled = handle(Action::NewProject, &mut project);
        assert_eq!(project, Project::None);
        assert!(!handled.quit);
    }
}
