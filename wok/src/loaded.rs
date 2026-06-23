//! The loaded scene: the active scene tab's authored data, held app-side and reconciled to the model.
//!
//! This is filesystem residency, not part of the pure [`Model`](crate::model::Model): opening a scene
//! reads its `scene.json` and chunk files off disk, which the model is deliberately free of (it stays
//! unit-testable without a window and renders the same live and in the snapshot test). So the editor
//! holds an `Option<LoadedScene>` alongside the model and [`reconcile`]s it to the active tab each
//! frame - the same shape the prior build used for its GPU/filesystem residency.
//!
//! What it holds: the active scene's identity (the project root and the scene name - what is loaded)
//! and its placements, collected from every chunk in the scene and ordered deterministically. This
//! bite lists those placements flat in the Instances nav view; the 3D viewport, selection, and the
//! richer per-placement data are later bites that grow this struct.
//!
//! Reconcile policy. [`reconcile`] reloads only when the active scene tab's identity changes
//! (reload-on-tab-change). Switching tabs, opening or closing the active one, or changing project all
//! re-derive the identity and reload; an unchanged identity is a cheap no-op (no disk touch). Disk hot
//! reload - re-reading when the files themselves change under an open scene - is a later bite. A load
//! failure (a missing or malformed `scene.json`, a corrupt chunk) is noted on the struct and degrades
//! to an empty placement list; it never crashes the editor, and surfacing the note (the status bar,
//! integrity) is a later bite.

use std::path::{Path, PathBuf};

use wok_scene::{ContentLayout, InstanceId, LoadError, Placement};

use crate::model::{Model, Tab};

/// The active scene tab's loaded authored data. Built by [`load`](Self::load) from a project root and
/// a scene name, and reconciled to the active tab by [`reconcile`]. Holds the identity (so [`reconcile`]
/// can tell whether a reload is needed) and the scene's placements; a load error is noted rather than
/// thrown, leaving the placement list empty.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedScene {
    /// The project root the scene was loaded from. Half of the load identity.
    root: PathBuf,
    /// The scene name (its `assets/scenes/<name>` folder). The other half of the identity, and what
    /// the tab and the editor well show.
    name: String,
    /// Every placement in the scene, gathered from all its chunks and sorted by instance id, so the
    /// flat Instances list is deterministic regardless of chunk or file order.
    placements: Vec<Placement>,
    /// The load error, if the scene failed to load (the placement list is then empty). Noted, not
    /// thrown; a later bite surfaces it (the status bar, the integrity view).
    error: Option<String>,
}

impl LoadedScene {
    /// Load the scene named `name` under project `root`: the manifest first, then every chunk's
    /// placements, collected and sorted by instance id. A load failure is captured on [`error`] with
    /// an empty placement list rather than propagated - the editor must not crash because a scene on
    /// disk is missing or malformed.
    ///
    /// [`error`]: Self::error
    pub fn load(root: &Path, name: &str) -> LoadedScene {
        let (placements, error) = match collect_placements(root, name) {
            Ok(placements) => (placements, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        LoadedScene { root: root.to_path_buf(), name: name.to_owned(), placements, error }
    }

    /// The project root this scene was loaded from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The loaded scene's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The scene's placements, sorted by instance id. Empty when the scene has none, or when it failed
    /// to load (check [`error`](Self::error) to tell the two apart).
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// The placement with this instance id, or `None` when none matches - the resolution the model's
    /// [`selection`](crate::model::Shell::selection) goes through, since the model holds only the id.
    /// A missing id (a stale selection, or one from a different scene) resolves to `None`, so the
    /// selection becomes a no-op rather than an error: the highlight and the inspector simply do not
    /// show. A linear scan is ample at editor scene scale (the same call site that lists every row).
    pub fn placement(&self, id: InstanceId) -> Option<&Placement> {
        self.placements.iter().find(|p| p.instance_id == id)
    }

    /// The load error, or `None` when the scene loaded cleanly. Distinguishes "the scene has no
    /// placements" (`None`, empty list) from "the scene could not be read" (`Some`, also an empty
    /// list).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Reconcile the loaded scene to the model's active tab. Reloads only when the active scene's identity
/// (the project root and scene name) differs from what is loaded, so an unchanged active tab is a
/// no-op with no disk touch; no active scene tab (or no open project) clears it to `None`. Called once
/// per frame, before the chrome is built, so the Instances view reflects the current tab this frame.
///
/// Returns whether the active scene changed (a (re)load or a clear happened). The frame loop uses this
/// to drop a selection when the scene under it changes: an [`InstanceId`] is per-scene, so a selection
/// made in one scene must not carry onto the next. An unchanged identity returns `false`, so a standing
/// selection survives every frame the scene stays put.
pub fn reconcile(loaded: &mut Option<LoadedScene>, model: &Model) -> bool {
    let desired = desired_scene(model);
    // The identity already loaded, if any. Compared by (root, name) - the reload trigger.
    let current = loaded.as_ref().map(|l| (l.root(), l.name()));
    if current == desired {
        return false;
    }
    // Identity changed (a different tab, project, or none at all): (re)load, or clear when there is no
    // active scene tab to load.
    *loaded = desired.map(|(root, name)| LoadedScene::load(root, name));
    true
}

/// The scene the active tab wants loaded: the active tab's scene name under the open project's root,
/// or `None` when no project is open or no scene tab is active. Borrows the model, so the comparison in
/// [`reconcile`] is against the live model with no allocation.
fn desired_scene(model: &Model) -> Option<(&Path, &str)> {
    let root = model.project.as_ref()?.root();
    let active = model.shell.active_tab()?;
    match model.shell.tabs().get(active)? {
        Tab::Scene(name) => Some((root, name.as_str())),
    }
}

/// Load a scene's placements from disk: the manifest first, then every chunk. The manifest load is the
/// gate - a scene whose `scene.json` is missing or malformed is a load error, not an empty instance
/// list (and later bites read its regions, default lighting, and id counter); only its chunks carry
/// placements. Coordinates come from [`ContentLayout::chunk_coords`] (sorted), and the gathered
/// placements are sorted by instance id, which is unique and monotonic per scene - a total order with
/// no ties, independent of which chunk or file each placement came from.
fn collect_placements(root: &Path, name: &str) -> Result<Vec<Placement>, LoadError> {
    let layout = ContentLayout::new(root);
    wok_scene::load_scene(layout.scene_json(name))?;
    let mut placements = Vec::new();
    for coord in layout.chunk_coords(name) {
        let chunk = wok_scene::load_chunk(layout.chunk(name, coord))?;
        placements.extend(chunk.placements);
    }
    placements.sort_by_key(|p| p.instance_id);
    Ok(placements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    use wok_scene::{
        Chunk, ChunkCoord, ChunkStreaming, Eagerness, InstanceId, LightStateRef, PrefabRef, Scene,
        StreamingDefaults, Transform, save_chunk, save_scene,
    };

    use crate::project::Project;

    // A unique temp root per test (pid + atomic counter, no wall-clock), on wok-scene's pattern. Not
    // created here; each test seeds what it needs and clears the tree at both ends so a crashed run
    // never leaks into the next.
    fn unique_temp_root() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-editor-loaded-{pid}-{n}"))
    }

    fn sample_scene(name: &str) -> Scene {
        Scene {
            name: name.to_string(),
            default_lighting: LightStateRef::new("noon"),
            regions: vec![],
            default_streaming: StreamingDefaults { load_radius: 3, default_eagerness: Eagerness::Eager },
            next_instance_id: InstanceId(10),
        }
    }

    fn placement(prefab: &str, id: u32, name: Option<&str>) -> Placement {
        Placement {
            prefab: PrefabRef::new(prefab),
            instance_id: InstanceId(id),
            name: name.map(str::to_owned),
            transform: Transform::IDENTITY,
            state: None,
        }
    }

    fn chunk(coord: ChunkCoord, placements: Vec<Placement>) -> Chunk {
        Chunk { coord, placements, streaming: ChunkStreaming::default() }
    }

    // Seed a two-chunk scene whose placements are written out of instance-id order, across both
    // chunks, so a passing collect proves the sort rather than incidental file order: ids 2 and 0 in
    // one chunk, id 1 (named) in the other.
    fn seed_scene(root: &Path, name: &str) {
        let layout = ContentLayout::new(root);
        std::fs::create_dir_all(layout.scene_dir(name)).unwrap();
        save_scene(&sample_scene(name), layout.scene_json(name)).unwrap();
        let placements = vec![placement("rock", 2, None), placement("oak_tree", 0, None)];
        save_chunk(&chunk(ChunkCoord::new(0, 0), placements), layout.chunk(name, ChunkCoord::new(0, 0))).unwrap();
        let named = vec![placement("barrel", 1, Some("the landmark oak"))];
        save_chunk(&chunk(ChunkCoord::new(1, 0), named), layout.chunk(name, ChunkCoord::new(1, 0))).unwrap();
    }

    #[test]
    fn load_collects_every_chunk_sorted_by_instance_id() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");

        let loaded = LoadedScene::load(&root, "village");
        assert!(loaded.error().is_none(), "a well-formed scene loads cleanly");
        assert_eq!(loaded.root(), root);
        assert_eq!(loaded.name(), "village");
        // Gathered from both chunks and sorted by id, regardless of the order they were written in.
        let ids: Vec<u32> = loaded.placements().iter().map(|p| p.instance_id.0).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        // The fields the flat list labels each row from survive the round trip.
        assert_eq!(loaded.placements()[0].prefab.as_str(), "oak_tree");
        assert_eq!(loaded.placements()[1].name.as_deref(), Some("the landmark oak"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn placement_resolves_a_present_id_and_is_none_for_a_missing_one() {
        // The resolution the selection goes through: a present id returns its placement; a missing id
        // (a stale selection, or one from another scene) returns None, so the selection is a no-op
        // rather than an error.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        let loaded = LoadedScene::load(&root, "village");

        let found = loaded.placement(InstanceId(1)).expect("id 1 is in the seeded scene");
        assert_eq!(found.name.as_deref(), Some("the landmark oak"));
        assert_eq!(loaded.placement(InstanceId(999)), None, "an absent id resolves to nothing");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_notes_the_error_and_stays_empty_when_the_scene_is_missing() {
        // A scene folder with no scene.json: load_scene fails, the error is noted, and the placement
        // list is empty - the editor degrades rather than crashing.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let loaded = LoadedScene::load(&root, "does_not_exist");
        assert!(loaded.placements().is_empty());
        assert!(loaded.error().is_some(), "a missing scene.json is a noted load error");
    }

    #[test]
    fn reconcile_loads_the_active_scene_tab_then_clears_when_it_closes() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");

        let mut model = Model { project: Some(Project::new(&root)), ..Model::default() };
        model.shell.open_tab(Tab::Scene("village".to_string()));

        // An active scene tab loads its scene, and reconcile reports the (re)load.
        let mut loaded = None;
        assert!(reconcile(&mut loaded, &model), "the first load is a change");
        let scene = loaded.as_ref().expect("the active scene tab is loaded");
        assert_eq!(scene.name(), "village");
        assert_eq!(scene.placements().len(), 3);

        // The identity is unchanged the next frame, so reconcile is a no-op and reports no change (the
        // signal the frame loop relies on to keep a standing selection rather than dropping it).
        assert!(!reconcile(&mut loaded, &model), "an unchanged active scene is not a change");

        // Closing the only tab leaves no active scene, so the loaded scene clears - and that is a change.
        model.shell.close_tab(0);
        assert!(reconcile(&mut loaded, &model), "clearing the scene is a change");
        assert!(loaded.is_none(), "no active scene tab -> nothing loaded");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reconcile_is_a_no_op_without_a_project_or_a_tab() {
        // A scene tab but no project (not reachable in the UI, but the guard holds): nothing loads.
        let mut model = Model::default();
        model.shell.open_tab(Tab::Scene("village".to_string()));
        let mut loaded = None;
        reconcile(&mut loaded, &model);
        assert!(loaded.is_none(), "a tab with no open project loads nothing");
    }
}
