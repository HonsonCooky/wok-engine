//! The editor's action vocabulary and its handler - the one seam every menu choice and keybind
//! routes through.
//!
//! The view emits [`Action`]s into a per-frame buffer rather than mutating state inside its egui
//! closures; the frame loop (`crate::main`) drains the buffer and applies each through [`handle`], so
//! the [`Model`] has a single writer. This is the action layer the editor grows on
//! (designs/editor-design.md): one vocabulary, one apply point, which is what later makes undo and
//! redo possible. [`handle`] is free of egui and the filesystem so the routing is unit testable
//! without a window.
//!
//! Effects channel. [`handle`] returns [`Handled`], the part the pure model cannot do itself and the
//! frame loop must carry out - here, persisting the recent-projects list to disk. The seam flagged
//! this for "when an action first needs a frame-loop effect like quit/save"; project open is that
//! action, so the channel returns now.
//!
//! Open is split across the seam. Validating that a picked folder is a wok project is filesystem I/O,
//! so it lives in the frame loop (`crate::project::open`), never in this pure handler. The loop
//! validates first and applies [`Action::OpenProject`] only on success, so [`handle`]'s job is just
//! the pure mutation - set the project and record the recent - and a recent is recorded only for a
//! folder that really opened.

use std::path::PathBuf;

use crate::model::{Model, NavSide, NavView};
use crate::project::Project;

/// A menu choice, keybind, or chrome interaction, emitted by the view and applied by [`handle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // ---- navigation panel ----
    /// Switch the navigation panel to show this view. Emitted by the bottom icon bar.
    SelectNavView(NavView),
    /// Show or hide the navigation panel. Emitted by the View menu.
    ToggleNavPanel,
    /// Dock the navigation panel to this side. Emitted by the View menu.
    SetNavSide(NavSide),

    // ---- project lifecycle ----
    /// Open the project rooted at this path, and record it at the front of the recent list. The same
    /// action the folder picker and an Open Recent entry both emit, so reopening a recent is just
    /// opening its path - one obvious way, not a parallel code path. The frame loop validates the
    /// folder is a wok project before applying this (the I/O is the loop's, not the handler's), so it
    /// reaches [`handle`] only for a folder that really opened.
    OpenProject(PathBuf),
    /// Close the open project, returning to the no-project state. Emitted by the File menu.
    CloseProject,
}

/// What [`handle`] asks the frame loop to carry out - the effects the pure model cannot perform
/// itself. Empty by default; an action sets only the field it needs. Today the one effect is
/// persisting the recent-projects list; quit and save join as those actions return.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Handled {
    /// The recent-projects list changed; the frame loop should write it to disk (`crate::recent`).
    pub save_recents: bool,
}

/// Apply one action to the model, returning the frame-loop effects it implies. The single point where
/// a chrome interaction changes editor state.
pub fn handle(model: &mut Model, action: Action) -> Handled {
    match action {
        Action::SelectNavView(view) => {
            model.shell.select_nav(view);
            Handled::default()
        }
        Action::ToggleNavPanel => {
            model.shell.toggle_nav();
            Handled::default()
        }
        Action::SetNavSide(side) => {
            model.shell.set_nav_side(side);
            Handled::default()
        }
        Action::OpenProject(root) => {
            // The loop validated the folder is a wok project before applying this, so construct the
            // project without re-touching the filesystem and record it at the front of recents.
            model.project = Some(Project::new(root.clone()));
            model.recents.push(root);
            Handled { save_recents: true }
        }
        Action::CloseProject => {
            // The dock side and visibility are workspace preferences, not project content, so closing
            // leaves them alone; recents keeps the just-closed project too.
            model.project = None;
            Handled::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ---- navigation panel ----

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

    // ---- project lifecycle ----

    #[test]
    fn open_project_sets_the_project_and_records_a_recent() {
        let mut model = Model::default();
        let handled = handle(&mut model, Action::OpenProject(PathBuf::from("games/unstitched")));
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("games/unstitched")));
        assert_eq!(model.recents.paths(), &[PathBuf::from("games/unstitched")]);
        assert!(handled.save_recents, "opening a project changes recents, so it should persist");
    }

    #[test]
    fn opening_projects_lists_them_most_recent_first() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenProject(PathBuf::from("a")));
        handle(&mut model, Action::OpenProject(PathBuf::from("b")));
        assert_eq!(model.recents.paths(), &[PathBuf::from("b"), PathBuf::from("a")]);
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("b")));
    }

    #[test]
    fn reopening_a_project_dedups_to_a_single_front_entry() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenProject(PathBuf::from("a")));
        handle(&mut model, Action::OpenProject(PathBuf::from("b")));
        handle(&mut model, Action::OpenProject(PathBuf::from("a")));
        // Reopening "a" moves it to the front without duplicating it.
        assert_eq!(model.recents.paths(), &[PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn close_project_returns_to_no_project_and_keeps_recents_and_layout() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenProject(PathBuf::from("games/unstitched")));
        handle(&mut model, Action::SetNavSide(NavSide::Right));
        let handled = handle(&mut model, Action::CloseProject);
        assert!(model.project.is_none());
        // The closed project stays in recents (it was just opened), and the dock side is a workspace
        // preference, not project content, so the close leaves it be.
        assert_eq!(model.recents.paths(), &[PathBuf::from("games/unstitched")]);
        assert_eq!(model.shell.nav_side(), NavSide::Right);
        assert!(!handled.save_recents, "closing does not change recents");
    }
}
