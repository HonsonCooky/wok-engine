//! The loaded scene: the active scene tab's authored data, held app-side and reconciled to the model.
//!
//! This is filesystem residency, not part of the pure [`Model`](crate::model::Model): opening a scene
//! reads its `scene.json` and chunk files off disk, which the model is deliberately free of (it stays
//! unit-testable without a window and renders the same live and in the snapshot test). So the editor
//! holds an `Option<LoadedScene>` alongside the model and [`reconcile`]s it to the active tab each
//! frame - the same shape the prior build used for its GPU/filesystem residency.
//!
//! What it holds: the active scene's identity (the project root and the scene name - what is loaded),
//! its chunks, and a flat placement list derived from them. The chunks are the authoritative store -
//! what [`save`](LoadedScene::save) writes back, each with its coord and streaming metadata intact - so
//! an edit mutates a placement in its chunk. The flat [`placements`](LoadedScene::placements) list is a
//! derived, id-sorted view of every chunk's placements (the read model the Instances view and the
//! inspector consume), rebuilt after each mutation so it never drifts; it is cached rather than
//! recomputed per call because the view reads it every frame while edits are rare. A [`dirty`] flag
//! tracks unsaved edits.
//!
//! Editing goes through the mutators here ([`rename`](LoadedScene::rename) and
//! [`set_transform`](LoadedScene::set_transform)), called by the single writer `crate::action::handle`.
//! The mutators are in-memory and filesystem-free; the disk write is [`save`](LoadedScene::save),
//! driven by the frame loop (Ctrl+S, via `Handled`) and by [`reconcile`] (auto-save on a tab switch).
//! The 3D viewport and its picking are later bites.
//!
//! [`dirty`]: LoadedScene::dirty
//!
//! Reconcile policy. [`reconcile`] reloads only when the active scene tab's identity changes
//! (reload-on-tab-change). Switching tabs, opening or closing the active one, or changing project all
//! re-derive the identity and reload; an unchanged identity is a cheap no-op (no disk touch). Before it
//! replaces a dirty outgoing scene it auto-saves it, so a tab switch never silently drops unsaved edits
//! (a confirm prompt is a later refinement). Disk hot reload - re-reading when the files themselves
//! change under an open scene - is a later bite. A load failure (a missing or malformed `scene.json`, a
//! corrupt chunk) is noted on the struct and degrades to an empty placement list; it never crashes the
//! editor, and surfacing the note (the status bar, integrity) is a later bite.

use std::path::{Path, PathBuf};

use wok_scene::{Chunk, ContentLayout, InstanceId, LoadError, Placement, SaveError, Transform};

use crate::model::{Model, Tab};

/// The active scene tab's loaded authored data. Built by [`load`](Self::load) from a project root and
/// a scene name, and reconciled to the active tab by [`reconcile`]. Holds the identity (so [`reconcile`]
/// can tell whether a reload is needed), the scene's chunks (the authoritative store [`save`](Self::save)
/// writes back) and a derived flat placement list, and a [`dirty`](Self::dirty) flag; a load error is
/// noted rather than thrown, leaving the chunk and placement lists empty.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedScene {
    /// The project root the scene was loaded from. Half of the load identity.
    root: PathBuf,
    /// The scene name (its `assets/scenes/<name>` folder). The other half of the identity, and what
    /// the tab and the editor well show.
    name: String,
    /// The scene's chunks, in coord order, as loaded from disk - the authoritative placement store.
    /// An edit mutates a placement in place here; [`save`](Self::save) writes each chunk back whole
    /// (coord + streaming metadata), so it is the chunks, not the flat list, that persist.
    chunks: Vec<Chunk>,
    /// Every placement across all chunks, sorted by instance id - a derived read model for the flat
    /// Instances list, deterministic regardless of chunk or file order. Rebuilt from `chunks` after
    /// each mutation ([`reindex`](Self::reindex)), so it never drifts from the chunks it mirrors.
    placements: Vec<Placement>,
    /// Whether the scene has unsaved edits. Set by the mutators, cleared by a successful
    /// [`save`](Self::save). Drives the status-bar save dot and the auto-save in [`reconcile`].
    dirty: bool,
    /// The load error, if the scene failed to load (the chunk and placement lists are then empty).
    /// Noted, not thrown; a later bite surfaces it (the status bar, the integrity view).
    error: Option<String>,
}

impl LoadedScene {
    /// Load the scene named `name` under project `root`: the manifest first, then every chunk. The
    /// flat placement list is derived from the chunks (sorted by instance id). A load failure is
    /// captured on [`error`] with empty chunk and placement lists rather than propagated - the editor
    /// must not crash because a scene on disk is missing or malformed. A freshly loaded scene is clean.
    ///
    /// [`error`]: Self::error
    pub fn load(root: &Path, name: &str) -> LoadedScene {
        let (chunks, error) = match collect_chunks(root, name) {
            Ok(chunks) => (chunks, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let placements = flatten_sorted(&chunks);
        LoadedScene { root: root.to_path_buf(), name: name.to_owned(), chunks, placements, dirty: false, error }
    }

    /// The project root this scene was loaded from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The loaded scene's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The scene's chunks, the authoritative placement store - the live source the render residency
    /// ([`RenderScene`](crate::render_scene::RenderScene)) derives the viewport's runtime arrays from,
    /// so an in-memory edit shows in the 3D without a disk reload. Empty when the scene failed to load.
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// The scene's placements, sorted by instance id. Empty when the scene has none, or when it failed
    /// to load (check [`error`](Self::error) to tell the two apart). This is the derived read cache; an
    /// edit rebuilds it, so it always mirrors the chunks.
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

    /// Whether the scene has unsaved edits. The status bar reads this for the save dot, and
    /// [`reconcile`] reads it to auto-save before switching scenes.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// The load error, or `None` when the scene loaded cleanly. Distinguishes "the scene has no
    /// placements" (`None`, empty list) from "the scene could not be read" (`Some`, also an empty
    /// list).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Set (or clear, with `None`) the display name of the placement with `id`, marking the scene dirty
    /// and rebuilding the flat list. Returns whether anything changed: a stale id (no matching
    /// placement) or a rename to the value already held is a no-op that leaves the scene clean, so
    /// reselecting and re-committing an unchanged name never spuriously dirties it. In-memory only -
    /// the disk write is [`save`](Self::save).
    pub fn rename(&mut self, id: InstanceId, name: Option<String>) -> bool {
        let Some(placement) = self.find_mut(id) else { return false };
        if placement.name == name {
            return false;
        }
        placement.name = name;
        self.dirty = true;
        self.reindex();
        true
    }

    /// Set the transform (position, rotation, scale) of the placement with `id`, marking the scene
    /// dirty and rebuilding the flat list. Returns whether anything changed: a stale id (no matching
    /// placement) or a set to the transform already held is a no-op that leaves the scene clean, so
    /// re-committing an unchanged transform never spuriously dirties it. The flat list caches a clone
    /// of each placement, so the reindex is what carries the new transform into the read model the
    /// inspector reads back. In-memory only - the disk write is [`save`](Self::save). Mirrors
    /// [`rename`](Self::rename); the inspector's Pos / Rot / Scale fields drive it through the action
    /// layer.
    pub fn set_transform(&mut self, id: InstanceId, transform: Transform) -> bool {
        let Some(placement) = self.find_mut(id) else { return false };
        if placement.transform == transform {
            return false;
        }
        placement.transform = transform;
        self.dirty = true;
        self.reindex();
        true
    }

    /// Write every chunk back to disk under the scene's folder (`assets/scenes/<name>/{x}_{z}.json`),
    /// clearing [`dirty`](Self::dirty) on success. Each chunk is written whole, so its coord and
    /// streaming metadata survive a save that touched only a placement. Stops at the first I/O or
    /// serialize error with `dirty` still set, so a partial failure stays flagged (the save dot stays
    /// lit) rather than reading as saved. All chunks are rewritten, not just changed ones; per-chunk
    /// dirty tracking is a later optimization.
    pub fn save(&mut self) -> Result<(), SaveError> {
        let layout = ContentLayout::new(&self.root);
        for chunk in &self.chunks {
            wok_scene::save_chunk(chunk, layout.chunk(&self.name, chunk.coord))?;
        }
        self.dirty = false;
        Ok(())
    }

    /// The placement with `id` across all chunks, mutably. Used by the mutators; a linear scan is ample
    /// at editor scene scale.
    fn find_mut(&mut self, id: InstanceId) -> Option<&mut Placement> {
        self.chunks.iter_mut().flat_map(|c| c.placements.iter_mut()).find(|p| p.instance_id == id)
    }

    /// Rebuild the derived flat placement list from the chunks - called after a mutation so the view's
    /// read model never drifts from the authoritative chunks.
    fn reindex(&mut self) {
        self.placements = flatten_sorted(&self.chunks);
    }
}

/// Reconcile the loaded scene to the model's active tab. Reloads only when the active scene's identity
/// (the project root and scene name) differs from what is loaded, so an unchanged active tab is a
/// no-op with no disk touch; no active scene tab (or no open project) clears it to `None`. Before it
/// replaces a dirty outgoing scene it auto-saves it, so switching tabs never silently drops unsaved
/// edits (a confirm prompt is a later refinement; a failed auto-save is best-effort and surfaces in the
/// same later bite as load errors). Called once per frame, before the chrome is built, so the Instances
/// view reflects the current tab this frame.
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
    // Identity is changing. Persist the outgoing scene's unsaved edits before it is replaced, so a tab
    // switch never silently drops them. Best-effort: a failed auto-save leaves `dirty` set, and
    // surfacing it is the same later bite as load errors.
    if let Some(outgoing) = loaded.as_mut() {
        if outgoing.dirty() {
            let _ = outgoing.save();
        }
    }
    // (Re)load, or clear when there is no active scene tab to load.
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

/// Load a scene's chunks from disk: the manifest first, then every chunk. The manifest load is the
/// gate - a scene whose `scene.json` is missing or malformed is a load error, not an empty scene (and
/// later bites read its regions, default lighting, and id counter); only its chunks carry placements.
/// Coordinates come from [`ContentLayout::chunk_coords`] (sorted), so the chunks come back in a
/// deterministic order.
fn collect_chunks(root: &Path, name: &str) -> Result<Vec<Chunk>, LoadError> {
    let layout = ContentLayout::new(root);
    wok_scene::load_scene(layout.scene_json(name))?;
    let mut chunks = Vec::new();
    for coord in layout.chunk_coords(name) {
        chunks.push(wok_scene::load_chunk(layout.chunk(name, coord))?);
    }
    Ok(chunks)
}

/// Flatten every chunk's placements into one list sorted by instance id - the derived read model. The
/// instance id is unique and monotonic per scene, so this is a total order with no ties, independent of
/// which chunk or file each placement came from.
fn flatten_sorted(chunks: &[Chunk]) -> Vec<Placement> {
    let mut placements: Vec<Placement> = chunks.iter().flat_map(|c| c.placements.iter().cloned()).collect();
    placements.sort_by_key(|p| p.instance_id);
    placements
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
    // one chunk, id 1 (named) in the other. Chunk (1,0) carries non-default streaming, so the save
    // round-trip can prove Save writes the whole chunk back (coord + streaming), not just placements.
    fn seed_scene(root: &Path, name: &str) {
        let layout = ContentLayout::new(root);
        std::fs::create_dir_all(layout.scene_dir(name)).unwrap();
        save_scene(&sample_scene(name), layout.scene_json(name)).unwrap();
        let placements = vec![placement("rock", 2, None), placement("oak_tree", 0, None)];
        save_chunk(&chunk(ChunkCoord::new(0, 0), placements), layout.chunk(name, ChunkCoord::new(0, 0))).unwrap();
        let named = Chunk {
            coord: ChunkCoord::new(1, 0),
            placements: vec![placement("barrel", 1, Some("the landmark oak"))],
            streaming: ChunkStreaming {
                eagerness: Some(Eagerness::Lazy),
                neighbors: vec![ChunkCoord::new(0, 0)],
                always_load_with: vec![],
            },
        };
        save_chunk(&named, layout.chunk(name, ChunkCoord::new(1, 0))).unwrap();
    }

    #[test]
    fn load_collects_every_chunk_sorted_by_instance_id() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");

        let loaded = LoadedScene::load(&root, "village");
        assert!(loaded.error().is_none(), "a well-formed scene loads cleanly");
        assert!(!loaded.dirty(), "a freshly loaded scene is clean");
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
    fn rename_sets_the_name_dirties_and_rebuilds_the_flat_list() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        let mut loaded = LoadedScene::load(&root, "village");

        assert!(loaded.rename(InstanceId(0), Some("first oak".to_owned())), "renaming a present id mutates");
        assert!(loaded.dirty(), "a rename dirties the scene");
        // The mutation lands on the chunk store and the derived flat list the view reads.
        assert_eq!(loaded.placement(InstanceId(0)).unwrap().name.as_deref(), Some("first oak"));
        let from_list = loaded.placements().iter().find(|p| p.instance_id == InstanceId(0)).unwrap();
        assert_eq!(from_list.name.as_deref(), Some("first oak"), "the flat list reflects the edit");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_can_clear_a_name_to_none() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        let mut loaded = LoadedScene::load(&root, "village");

        // id 1 is "the landmark oak"; clearing it restores the unnamed state.
        assert!(loaded.rename(InstanceId(1), None), "clearing a set name mutates");
        assert_eq!(loaded.placement(InstanceId(1)).unwrap().name, None);
        assert!(loaded.dirty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_of_a_missing_id_or_to_the_same_name_is_a_clean_no_op() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        let mut loaded = LoadedScene::load(&root, "village");

        assert!(!loaded.rename(InstanceId(999), Some("ghost".to_owned())), "no placement, no rename");
        assert!(!loaded.dirty(), "a missing-id rename leaves the scene clean");
        // id 1 already holds this exact name, so re-committing it changes nothing and must not dirty.
        assert!(!loaded.rename(InstanceId(1), Some("the landmark oak".to_owned())), "an identical rename is a no-op");
        assert!(!loaded.dirty(), "an unchanged name leaves the scene clean");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_transform_sets_the_transform_dirties_and_rebuilds_the_flat_list() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        let mut loaded = LoadedScene::load(&root, "village");

        let moved = Transform {
            translation: glam::Vec3::new(4.0, 1.0, -2.0),
            rotation: glam::Quat::from_rotation_y(30.0_f32.to_radians()),
            scale: glam::Vec3::splat(2.0),
        };
        assert!(loaded.set_transform(InstanceId(0), moved), "setting a present id's transform mutates");
        assert!(loaded.dirty(), "a transform edit dirties the scene");
        // The mutation lands on the chunk store and the derived flat list the inspector reads back.
        assert_eq!(loaded.placement(InstanceId(0)).unwrap().transform, moved);
        let from_list = loaded.placements().iter().find(|p| p.instance_id == InstanceId(0)).unwrap();
        assert_eq!(from_list.transform, moved, "the flat list reflects the edit");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_transform_of_a_missing_id_or_an_unchanged_value_is_a_clean_no_op() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        let mut loaded = LoadedScene::load(&root, "village");

        assert!(!loaded.set_transform(InstanceId(999), Transform::IDENTITY), "no placement, no edit");
        assert!(!loaded.dirty(), "a missing-id set leaves the scene clean");
        // id 0 was seeded at IDENTITY, so re-setting that exact transform changes nothing and must not
        // dirty (the same no-op rule the inspector leans on for an untouched commit).
        assert!(!loaded.set_transform(InstanceId(0), Transform::IDENTITY), "an identical transform is a no-op");
        assert!(!loaded.dirty(), "an unchanged transform leaves the scene clean");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_persists_an_edited_transform_round_trip() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");

        let mut loaded = LoadedScene::load(&root, "village");
        let moved = Transform {
            translation: glam::Vec3::new(7.0, 0.0, 3.5),
            rotation: glam::Quat::from_rotation_y(90.0_f32.to_radians()),
            scale: glam::Vec3::splat(1.5),
        };
        assert!(loaded.set_transform(InstanceId(0), moved));
        loaded.save().expect("the scene saves to disk");
        assert!(!loaded.dirty(), "a successful save clears dirty");

        // Reload from disk: the edited transform persisted through the chunk write (Save writes the
        // whole chunk, transform and all).
        let reloaded = LoadedScene::load(&root, "village");
        assert_eq!(reloaded.placement(InstanceId(0)).unwrap().transform, moved, "the edit survived the round trip");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_writes_chunks_back_round_trip_and_clears_dirty() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");

        let mut loaded = LoadedScene::load(&root, "village");
        assert!(loaded.rename(InstanceId(1), Some("renamed oak".to_owned())));
        assert!(loaded.dirty());

        loaded.save().expect("the scene saves to disk");
        assert!(!loaded.dirty(), "a successful save clears dirty");

        // Reload from disk: the rename persisted, and chunk (1,0)'s streaming survived the write - Save
        // writes the whole chunk, not just its placements.
        let reloaded = LoadedScene::load(&root, "village");
        assert_eq!(reloaded.placement(InstanceId(1)).unwrap().name.as_deref(), Some("renamed oak"));
        let saved_chunk =
            wok_scene::load_chunk(ContentLayout::new(&root).chunk("village", ChunkCoord::new(1, 0))).unwrap();
        assert_eq!(saved_chunk.streaming.eagerness, Some(Eagerness::Lazy), "chunk streaming survives a save");
        assert_eq!(saved_chunk.streaming.neighbors, vec![ChunkCoord::new(0, 0)]);

        let _ = std::fs::remove_dir_all(&root);
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
    fn reconcile_auto_saves_a_dirty_scene_before_switching_tabs() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_scene(&root, "village");
        seed_scene(&root, "dungeon");

        let mut model = Model { project: Some(Project::new(&root)), ..Model::default() };
        model.shell.open_tab(Tab::Scene("village".to_string()));

        let mut loaded = None;
        reconcile(&mut loaded, &model); // loads village
        loaded.as_mut().unwrap().rename(InstanceId(1), Some("renamed in village".to_owned()));
        assert!(loaded.as_ref().unwrap().dirty(), "the edit dirtied village");

        // Open (and focus) dungeon: the active scene identity changes, so reconcile must persist
        // village's unsaved edit before loading dungeon - no silent loss on a tab switch.
        model.shell.open_tab(Tab::Scene("dungeon".to_string()));
        assert!(reconcile(&mut loaded, &model), "switching scenes is a (re)load");
        assert_eq!(loaded.as_ref().unwrap().name(), "dungeon", "the new scene is now loaded");

        // village's edit reached disk via the auto-save (a fresh load off disk shows it).
        let village = LoadedScene::load(&root, "village");
        assert_eq!(village.placement(InstanceId(1)).unwrap().name.as_deref(), Some("renamed in village"));

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
