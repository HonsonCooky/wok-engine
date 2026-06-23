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
//! Open is a pure state change. Opening a project has no gate and no filesystem touch (HLD "content
//! conventions and integrity"): any picked folder becomes the project, so [`Action::OpenProject`] just
//! sets the project and records the recent, applied through [`handle`] like every other action. wok
//! writes only inside `assets/`, and only on save (a later bite), so there is nothing to validate at
//! open and no validation step in the frame loop.

use std::path::PathBuf;

use wok_scene::InstanceId;

use crate::model::{InstanceSort, Model, NavSide, NavView, Tab};
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
    /// Set how the Instances view orders its placements (group-by-prefab or flat A-Z). Emitted by the
    /// panel header's sort control while the Instances view is active.
    SetInstanceSort(InstanceSort),

    // ---- tabs ----
    /// Open the named scene as a tab over the editor area, focusing it if already open (no
    /// duplicate). Emitted by a Scenes nav row. Scene-specific for now; opening prefab or lighting
    /// contexts as tabs is a later bite.
    OpenScene(String),
    /// Make the tab at this index active. Emitted by clicking a tab.
    SelectTab(usize),
    /// Close the tab at this index. Emitted by a tab's close affordance; the model then picks the next
    /// active tab ([`Shell::close_tab`](crate::model::Shell::close_tab)).
    CloseTab(usize),

    // ---- selection ----
    /// Select the placement with this instance id. Emitted by clicking an Instances-tree row (viewport
    /// picking is a later bite). The id is set as given; the view resolves it against the loaded scene
    /// when it reads, so an id with no matching placement resolves to nothing rather than erroring.
    Select(InstanceId),
    /// Clear the selection. Emitted by Esc or a click on empty space, and applied by the frame loop
    /// when it switches to a different scene (the per-scene id no longer applies).
    Deselect,

    // ---- project lifecycle ----
    /// Open the project rooted at this path, and record it at the front of the recent list. The same
    /// action the folder picker and an Open Recent entry both emit, so reopening a recent is just
    /// opening its path - one obvious way, not a parallel code path. Opening has no gate (HLD content
    /// conventions): any folder opens, so this sets the project directly with no filesystem check.
    OpenProject(PathBuf),
    /// Close the open project, returning to the no-project state. Emitted by the File menu.
    CloseProject,
    /// Empty the recent-projects list. Emitted by the File -> Open Recent submenu's Clear item; the
    /// handler clears the list and the frame loop persists the now-empty file, so it stays cleared.
    ClearRecents,
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
        Action::SetInstanceSort(sort) => {
            model.shell.set_instance_sort(sort);
            Handled::default()
        }
        Action::OpenScene(name) => {
            model.shell.open_tab(Tab::Scene(name));
            Handled::default()
        }
        Action::SelectTab(index) => {
            model.shell.select_tab(index);
            Handled::default()
        }
        Action::CloseTab(index) => {
            model.shell.close_tab(index);
            Handled::default()
        }
        Action::Select(id) => {
            model.shell.select(id);
            Handled::default()
        }
        Action::Deselect => {
            model.shell.deselect();
            Handled::default()
        }
        Action::OpenProject(root) => {
            // No gate: any picked folder becomes the project. Wrap it (no filesystem touch) and record
            // it at the front of recents; the frame loop persists the changed list.
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
        Action::ClearRecents => {
            // Empty the MRU list; the frame loop persists the now-empty list so it stays cleared across
            // runs. The open project (if any) and the shell layout are untouched.
            model.recents.clear();
            Handled { save_recents: true }
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

    #[test]
    fn set_instance_sort_flips_the_instances_ordering() {
        let mut model = Model::default();
        assert_eq!(model.shell.instance_sort(), InstanceSort::Group, "group-by-prefab is the default");
        handle(&mut model, Action::SetInstanceSort(InstanceSort::Flat));
        assert_eq!(model.shell.instance_sort(), InstanceSort::Flat);
        handle(&mut model, Action::SetInstanceSort(InstanceSort::Group));
        assert_eq!(model.shell.instance_sort(), InstanceSort::Group);
    }

    // ---- tabs ----

    #[test]
    fn open_scene_opens_a_scene_tab_and_focuses_it() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenScene("village".into()));
        assert_eq!(model.shell.tabs(), &[Tab::Scene("village".into())]);
        assert_eq!(model.shell.active_tab(), Some(0));
    }

    #[test]
    fn open_scene_focuses_rather_than_duplicates_an_open_scene() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenScene("village".into()));
        handle(&mut model, Action::OpenScene("dungeon".into()));
        handle(&mut model, Action::OpenScene("village".into()));
        assert_eq!(model.shell.tabs().len(), 2, "re-opening focuses, it does not duplicate");
        assert_eq!(model.shell.active_tab(), Some(0), "the re-opened scene is focused");
    }

    #[test]
    fn select_tab_focuses_the_indexed_tab() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenScene("a".into()));
        handle(&mut model, Action::OpenScene("b".into()));
        handle(&mut model, Action::SelectTab(0));
        assert_eq!(model.shell.active_tab(), Some(0));
    }

    #[test]
    fn close_tab_removes_it_and_reassigns_the_active_tab() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenScene("a".into()));
        handle(&mut model, Action::OpenScene("b".into()));
        handle(&mut model, Action::CloseTab(1)); // close b, the active tab
        assert_eq!(model.shell.tabs(), &[Tab::Scene("a".into())]);
        assert_eq!(model.shell.active_tab(), Some(0));
        handle(&mut model, Action::CloseTab(0)); // close the last remaining tab
        assert!(model.shell.tabs().is_empty());
        assert_eq!(model.shell.active_tab(), None);
    }

    // ---- selection ----

    #[test]
    fn select_sets_the_selection_and_deselect_clears_it() {
        let mut model = Model::default();
        handle(&mut model, Action::Select(InstanceId(4)));
        assert_eq!(model.shell.selection(), Some(InstanceId(4)));
        handle(&mut model, Action::Deselect);
        assert_eq!(model.shell.selection(), None);
    }

    #[test]
    fn select_replaces_the_prior_selection() {
        // Single-select this bite: selecting another instance moves the selection rather than adding.
        let mut model = Model::default();
        handle(&mut model, Action::Select(InstanceId(1)));
        handle(&mut model, Action::Select(InstanceId(2)));
        assert_eq!(model.shell.selection(), Some(InstanceId(2)));
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
    fn open_project_does_not_gate_on_folder_contents() {
        // No gate (HLD content conventions): a folder that is not a wok project - no scene.json, or one
        // that does not even exist - opens just the same as any other. handle is pure and the frame
        // loop no longer validates, so opening only sets the project and records the recent.
        let mut model = Model::default();
        let handled = handle(&mut model, Action::OpenProject(PathBuf::from("some/empty/folder")));
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("some/empty/folder")));
        assert!(handled.save_recents, "opening any folder records the recent");
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

    #[test]
    fn clear_recents_empties_the_list_and_signals_persist() {
        let mut model = Model::default();
        handle(&mut model, Action::OpenProject(PathBuf::from("a")));
        handle(&mut model, Action::OpenProject(PathBuf::from("b")));
        assert!(!model.recents.is_empty());
        let handled = handle(&mut model, Action::ClearRecents);
        assert!(model.recents.is_empty(), "clearing empties the recent list");
        assert!(handled.save_recents, "clearing changes the list, so the now-empty list must persist");
    }

    #[test]
    fn clear_recents_leaves_the_open_project_alone() {
        // Clearing the MRU list is independent of what is open: the project (and the shell layout)
        // stay; only the remembered history is dropped.
        let mut model = Model::default();
        handle(&mut model, Action::OpenProject(PathBuf::from("games/unstitched")));
        handle(&mut model, Action::ClearRecents);
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("games/unstitched")));
        assert!(model.recents.is_empty());
    }
}
