//! The editor's action vocabulary and its handler - the one seam every menu choice and keybind
//! routes through.
//!
//! The view emits [`Action`]s into a per-frame buffer rather than mutating state inside its egui
//! closures; the frame loop (`crate::main`) drains the buffer and applies each through [`handle`], so
//! the editor has a single writer. This is the action layer the editor grows on
//! (designs/editor-design.md): one vocabulary, one apply point, which is what later makes undo and
//! redo possible. [`handle`] writes the pure [`Model`] and, for the editing actions, the active
//! [`LoadedScene`] (passed by `&mut`, plain data - the residency the model cannot hold). It is free of
//! egui and the filesystem so the routing is unit testable without a window: a scene edit mutates the
//! loaded scene in memory and the disk write is the frame loop's, signalled through [`Handled`].
//!
//! Effects channel. [`handle`] returns [`Handled`], the parts the pure model cannot do itself and the
//! frame loop must carry out: persisting the recent-projects list, and saving the open scene to disk.
//! handle stays filesystem-free, so it flags the intent (a renamed scene is dirtied in memory; Ctrl+S
//! sets [`Handled::save`]) and the frame loop performs the I/O.
//!
//! Open is a pure state change. Opening a project has no gate and no filesystem touch (HLD "content
//! conventions and integrity"): any picked folder becomes the project, so [`Action::OpenProject`] just
//! sets the project and records the recent, applied through [`handle`] like every other action. wok
//! writes only inside `assets/`, and only on save (a later bite), so there is nothing to validate at
//! open and no validation step in the frame loop.

use std::path::PathBuf;

use wok_scene::{InstanceId, Transform};

use crate::loaded::LoadedScene;
use crate::model::{InstanceSort, Model, NavSide, NavView, Tab};
use crate::project::Project;

/// A menu choice, keybind, or chrome interaction, emitted by the view and applied by [`handle`].
///
/// `PartialEq` only, not `Eq`: [`SetInstanceTransform`](Action::SetInstanceTransform) carries a
/// [`Transform`] of floats, which are not `Eq`. Nothing keys actions in a set or map, so `PartialEq`
/// is all the vocabulary needs.
#[derive(Debug, Clone, PartialEq)]
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
    /// click-to-select is a later workflow). The id is set as given; the view resolves it against the
    /// loaded scene when it reads, so an id with no matching placement resolves to nothing rather than
    /// erroring.
    Select(InstanceId),
    /// Clear the selection. Emitted by Esc, and applied by the frame loop when it switches to a
    /// different scene (the per-scene id no longer applies).
    Deselect,

    // ---- editing ----
    /// Set the display name of the placement with this instance id, renaming it in the loaded scene.
    /// Emitted by the inspector's Name field on commit (blur or Enter). An empty string clears the
    /// name (the inverse of the inspector's blank-when-unnamed display), restoring the
    /// `{prefab} #{id}` fallback; handle maps it to `None`. A stale id or an unchanged value is a clean
    /// no-op (no dirty).
    SetInstanceName(InstanceId, String),
    /// Set the whole transform (position, rotation, scale) of the placement with this instance id,
    /// editing it in the loaded scene. Emitted by the inspector's Pos / Rot / Scale `DragValue`s on
    /// change - the precise authoring path, exact values with no grid snap (the 1m / 5deg keyboard snap
    /// is a later viewport bite). The whole transform travels each time (the field that changed already
    /// folded into it), so one variant covers all three rows; per-component actions are a later
    /// refinement if that proves too coarse. A stale id or a transform equal to the one stored is a
    /// clean no-op (no dirty), the same as a rename.
    SetInstanceTransform(InstanceId, Transform),
    /// Save the open scene to disk (Ctrl+S, or the status-bar save dot). The handler flags the write
    /// on [`Handled`] only when a scene is loaded and dirty; the frame loop calls
    /// [`LoadedScene::save`], which writes the chunks and clears the dirty flag on success.
    Save,

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
/// itself. Empty by default; an action sets only the field it needs. Today: persisting the
/// recent-projects list, and saving the open scene to disk.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Handled {
    /// The recent-projects list changed; the frame loop should write it to disk (`crate::recent`).
    pub save_recents: bool,
    /// The open scene should be saved; the frame loop calls [`LoadedScene::save`] (which writes the
    /// chunks and clears the dirty flag on success). Set only when a scene is loaded and dirty, so a
    /// Ctrl+S with nothing to save does no disk work.
    pub save: bool,
}

/// Apply one action, returning the frame-loop effects it implies. The single point where a chrome
/// interaction changes editor state. Most actions touch only the [`Model`]; the editing actions also
/// mutate the active scene, passed in as `loaded` (`None` when no scene tab is open). handle never
/// loads or clears the residency - that stays [`reconcile`](crate::loaded::reconcile)'s job - so it
/// takes `Option<&mut LoadedScene>`, not `&mut Option<_>`.
pub fn handle(model: &mut Model, loaded: Option<&mut LoadedScene>, action: Action) -> Handled {
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
        Action::SetInstanceName(id, name) => {
            // Empty input means "no name" - the inverse of the inspector's blank-when-unnamed display,
            // so clearing the field restores the {prefab} #{id} fallback. The rename is a no-op (no
            // dirty) when the id is stale or the value is unchanged; with no scene loaded it does
            // nothing.
            let name = if name.is_empty() { None } else { Some(name) };
            if let Some(scene) = loaded {
                scene.rename(id, name);
            }
            Handled::default()
        }
        Action::SetInstanceTransform(id, transform) => {
            // The inspector's Pos / Rot / Scale fields commit here. Like a rename, it is a no-op (no
            // dirty) when the id is stale or the transform equals the one already stored, and with no
            // scene loaded it does nothing.
            if let Some(scene) = loaded {
                scene.set_transform(id, transform);
            }
            Handled::default()
        }
        Action::Save => {
            // Flag the write for the frame loop (handle is filesystem-free), only when there is an
            // unsaved scene, so a Ctrl+S with nothing dirty does no disk work. The frame loop calls
            // LoadedScene::save, which writes the chunks and clears dirty on success.
            Handled { save: loaded.is_some_and(|s| s.dirty()), ..Handled::default() }
        }
        Action::OpenProject(root) => {
            // No gate: any picked folder becomes the project. Wrap it (no filesystem touch) and record
            // it at the front of recents; the frame loop persists the changed list.
            model.project = Some(Project::new(root.clone()));
            model.recents.push(root);
            Handled { save_recents: true, ..Handled::default() }
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
            Handled { save_recents: true, ..Handled::default() }
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
        handle(&mut model, None, Action::SelectNavView(NavView::Prefabs));
        assert_eq!(model.shell.active_nav(), NavView::Prefabs);
    }

    #[test]
    fn select_nav_view_to_the_same_view_is_idempotent() {
        let mut model = Model::default();
        handle(&mut model, None, Action::SelectNavView(NavView::Lighting));
        handle(&mut model, None, Action::SelectNavView(NavView::Lighting));
        assert_eq!(model.shell.active_nav(), NavView::Lighting);
    }

    #[test]
    fn select_nav_view_reaches_every_view() {
        let mut model = Model::default();
        for view in [NavView::Scenes, NavView::Prefabs, NavView::Instances, NavView::Lighting] {
            handle(&mut model, None, Action::SelectNavView(view));
            assert_eq!(model.shell.active_nav(), view);
        }
    }

    #[test]
    fn toggle_nav_panel_flips_visibility() {
        let mut model = Model::default();
        assert!(model.shell.nav_visible());
        handle(&mut model, None, Action::ToggleNavPanel);
        assert!(!model.shell.nav_visible());
        handle(&mut model, None, Action::ToggleNavPanel);
        assert!(model.shell.nav_visible());
    }

    #[test]
    fn set_nav_side_docks_the_panel() {
        let mut model = Model::default();
        handle(&mut model, None, Action::SetNavSide(NavSide::Right));
        assert_eq!(model.shell.nav_side(), NavSide::Right);
        handle(&mut model, None, Action::SetNavSide(NavSide::Left));
        assert_eq!(model.shell.nav_side(), NavSide::Left);
    }

    #[test]
    fn set_instance_sort_flips_the_instances_ordering() {
        let mut model = Model::default();
        assert_eq!(model.shell.instance_sort(), InstanceSort::Group, "group-by-prefab is the default");
        handle(&mut model, None, Action::SetInstanceSort(InstanceSort::Flat));
        assert_eq!(model.shell.instance_sort(), InstanceSort::Flat);
        handle(&mut model, None, Action::SetInstanceSort(InstanceSort::Group));
        assert_eq!(model.shell.instance_sort(), InstanceSort::Group);
    }

    // ---- tabs ----

    #[test]
    fn open_scene_opens_a_scene_tab_and_focuses_it() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenScene("village".into()));
        assert_eq!(model.shell.tabs(), &[Tab::Scene("village".into())]);
        assert_eq!(model.shell.active_tab(), Some(0));
    }

    #[test]
    fn open_scene_focuses_rather_than_duplicates_an_open_scene() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenScene("village".into()));
        handle(&mut model, None, Action::OpenScene("dungeon".into()));
        handle(&mut model, None, Action::OpenScene("village".into()));
        assert_eq!(model.shell.tabs().len(), 2, "re-opening focuses, it does not duplicate");
        assert_eq!(model.shell.active_tab(), Some(0), "the re-opened scene is focused");
    }

    #[test]
    fn select_tab_focuses_the_indexed_tab() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenScene("a".into()));
        handle(&mut model, None, Action::OpenScene("b".into()));
        handle(&mut model, None, Action::SelectTab(0));
        assert_eq!(model.shell.active_tab(), Some(0));
    }

    #[test]
    fn close_tab_removes_it_and_reassigns_the_active_tab() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenScene("a".into()));
        handle(&mut model, None, Action::OpenScene("b".into()));
        handle(&mut model, None, Action::CloseTab(1)); // close b, the active tab
        assert_eq!(model.shell.tabs(), &[Tab::Scene("a".into())]);
        assert_eq!(model.shell.active_tab(), Some(0));
        handle(&mut model, None, Action::CloseTab(0)); // close the last remaining tab
        assert!(model.shell.tabs().is_empty());
        assert_eq!(model.shell.active_tab(), None);
    }

    // ---- selection ----

    #[test]
    fn select_sets_the_selection_and_deselect_clears_it() {
        let mut model = Model::default();
        handle(&mut model, None, Action::Select(InstanceId(4)));
        assert_eq!(model.shell.selection(), Some(InstanceId(4)));
        handle(&mut model, None, Action::Deselect);
        assert_eq!(model.shell.selection(), None);
    }

    #[test]
    fn select_replaces_the_prior_selection() {
        // Single-select this bite: selecting another instance moves the selection rather than adding.
        let mut model = Model::default();
        handle(&mut model, None, Action::Select(InstanceId(1)));
        handle(&mut model, None, Action::Select(InstanceId(2)));
        assert_eq!(model.shell.selection(), Some(InstanceId(2)));
    }

    // ---- project lifecycle ----

    #[test]
    fn open_project_sets_the_project_and_records_a_recent() {
        let mut model = Model::default();
        let handled = handle(&mut model, None, Action::OpenProject(PathBuf::from("games/unstitched")));
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
        let handled = handle(&mut model, None, Action::OpenProject(PathBuf::from("some/empty/folder")));
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("some/empty/folder")));
        assert!(handled.save_recents, "opening any folder records the recent");
    }

    #[test]
    fn opening_projects_lists_them_most_recent_first() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenProject(PathBuf::from("a")));
        handle(&mut model, None, Action::OpenProject(PathBuf::from("b")));
        assert_eq!(model.recents.paths(), &[PathBuf::from("b"), PathBuf::from("a")]);
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("b")));
    }

    #[test]
    fn reopening_a_project_dedups_to_a_single_front_entry() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenProject(PathBuf::from("a")));
        handle(&mut model, None, Action::OpenProject(PathBuf::from("b")));
        handle(&mut model, None, Action::OpenProject(PathBuf::from("a")));
        // Reopening "a" moves it to the front without duplicating it.
        assert_eq!(model.recents.paths(), &[PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn close_project_returns_to_no_project_and_keeps_recents_and_layout() {
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenProject(PathBuf::from("games/unstitched")));
        handle(&mut model, None, Action::SetNavSide(NavSide::Right));
        let handled = handle(&mut model, None, Action::CloseProject);
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
        handle(&mut model, None, Action::OpenProject(PathBuf::from("a")));
        handle(&mut model, None, Action::OpenProject(PathBuf::from("b")));
        assert!(!model.recents.is_empty());
        let handled = handle(&mut model, None, Action::ClearRecents);
        assert!(model.recents.is_empty(), "clearing empties the recent list");
        assert!(handled.save_recents, "clearing changes the list, so the now-empty list must persist");
    }

    #[test]
    fn clear_recents_leaves_the_open_project_alone() {
        // Clearing the MRU list is independent of what is open: the project (and the shell layout)
        // stay; only the remembered history is dropped.
        let mut model = Model::default();
        handle(&mut model, None, Action::OpenProject(PathBuf::from("games/unstitched")));
        handle(&mut model, None, Action::ClearRecents);
        assert_eq!(model.project.as_ref().map(Project::root), Some(Path::new("games/unstitched")));
        assert!(model.recents.is_empty());
    }

    // ---- editing ----

    // A unique temp root per editing test, on the residency's pattern (pid + atomic counter). Not
    // created here; each test seeds it and clears the tree at both ends.
    fn editing_temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-editor-action-{pid}-{n}"))
    }

    // Seed a one-chunk "village" with a single named placement (id 1, "the old well") under `root`, and
    // load it - the residency the editing actions mutate. The caller clears `root` at both ends.
    fn seed_named_scene(root: &Path) -> LoadedScene {
        use wok_scene::{
            Chunk, ChunkCoord, ChunkStreaming, ContentLayout, Eagerness, LightStateRef, Placement,
            PrefabRef, Scene, StreamingDefaults, Transform, save_chunk, save_scene,
        };
        let layout = ContentLayout::new(root);
        std::fs::create_dir_all(layout.scene_dir("village")).unwrap();
        let scene = Scene {
            name: "village".to_string(),
            default_lighting: LightStateRef::new("noon"),
            regions: vec![],
            default_streaming: StreamingDefaults { load_radius: 3, default_eagerness: Eagerness::Eager },
            next_instance_id: InstanceId(5),
        };
        save_scene(&scene, layout.scene_json("village")).unwrap();
        let chunk = Chunk {
            coord: ChunkCoord::new(0, 0),
            placements: vec![Placement {
                prefab: PrefabRef::new("well"),
                instance_id: InstanceId(1),
                name: Some("the old well".to_owned()),
                transform: Transform::IDENTITY,
                state: None,
            }],
            streaming: ChunkStreaming::default(),
        };
        save_chunk(&chunk, layout.chunk("village", ChunkCoord::new(0, 0))).unwrap();
        LoadedScene::load(root, "village")
    }

    #[test]
    fn set_instance_name_routes_to_the_loaded_scene_and_dirties_it() {
        let root = editing_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let mut loaded = seed_named_scene(&root);
        let mut model = Model::default();

        let handled =
            handle(&mut model, Some(&mut loaded), Action::SetInstanceName(InstanceId(1), "renamed".to_owned()));
        assert_eq!(handled, Handled::default(), "a rename needs no frame-loop effect of its own");
        assert_eq!(loaded.placement(InstanceId(1)).unwrap().name.as_deref(), Some("renamed"));
        assert!(loaded.dirty(), "the edit dirtied the scene");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_instance_name_with_an_empty_string_clears_the_name() {
        let root = editing_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let mut loaded = seed_named_scene(&root);
        let mut model = Model::default();

        handle(&mut model, Some(&mut loaded), Action::SetInstanceName(InstanceId(1), String::new()));
        assert_eq!(loaded.placement(InstanceId(1)).unwrap().name, None, "empty input clears the name");
        assert!(loaded.dirty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_instance_name_with_no_loaded_scene_is_a_safe_no_op() {
        // Reachable if a rename action ever outlives its scene; it must route to nothing, not panic.
        let mut model = Model::default();
        let handled = handle(&mut model, None, Action::SetInstanceName(InstanceId(1), "x".to_owned()));
        assert_eq!(handled, Handled::default());
    }

    #[test]
    fn save_flags_the_write_only_when_a_scene_is_dirty() {
        let root = editing_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let mut loaded = seed_named_scene(&root);
        let mut model = Model::default();

        // A freshly loaded scene is clean, so Ctrl+S asks for no write.
        let clean = handle(&mut model, Some(&mut loaded), Action::Save);
        assert!(!clean.save, "nothing dirty -> no save effect");

        // After an edit, Ctrl+S flags the write for the frame loop to perform.
        loaded.rename(InstanceId(1), Some("renamed".to_owned()));
        let dirty = handle(&mut model, Some(&mut loaded), Action::Save);
        assert!(dirty.save, "a dirty scene flags the save effect");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_with_no_loaded_scene_flags_nothing() {
        let mut model = Model::default();
        let handled = handle(&mut model, None, Action::Save);
        assert!(!handled.save, "no scene -> nothing to save");
    }

    #[test]
    fn set_instance_transform_routes_to_the_loaded_scene_and_dirties_it() {
        let root = editing_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let mut loaded = seed_named_scene(&root);
        let mut model = Model::default();

        let moved = Transform { translation: glam::Vec3::new(1.0, 2.0, 3.0), ..Transform::IDENTITY };
        let handled = handle(&mut model, Some(&mut loaded), Action::SetInstanceTransform(InstanceId(1), moved));
        assert_eq!(handled, Handled::default(), "a transform edit needs no frame-loop effect of its own");
        assert_eq!(loaded.placement(InstanceId(1)).unwrap().transform, moved);
        assert!(loaded.dirty(), "the edit dirtied the scene");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_instance_transform_of_a_stale_id_is_a_clean_no_op() {
        let root = editing_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let mut loaded = seed_named_scene(&root);
        let mut model = Model::default();

        // id 99 is not in the seeded scene, so the edit finds nothing and must not dirty.
        let moved = Transform { translation: glam::Vec3::new(5.0, 0.0, 0.0), ..Transform::IDENTITY };
        handle(&mut model, Some(&mut loaded), Action::SetInstanceTransform(InstanceId(99), moved));
        assert!(!loaded.dirty(), "a stale-id transform edit leaves the scene clean");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_instance_transform_with_no_loaded_scene_is_a_safe_no_op() {
        // Reachable if a transform action ever outlives its scene; it must route to nothing, not panic.
        let mut model = Model::default();
        let handled = handle(&mut model, None, Action::SetInstanceTransform(InstanceId(1), Transform::IDENTITY));
        assert_eq!(handled, Handled::default());
    }
}
