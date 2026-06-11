//! Save, and the application of watcher-reported changes to the editor model.
//!
//! Save writes every dirty chunk plus the manifest through wok-scene's save paths and marks the
//! model clean. The file watcher then reports those same writes back (it cannot tell the editor's
//! writes from anyone else's), so change application is content-based: a reported file is
//! re-read and compared against the authored form in memory, and an identical file is a no-op -
//! no store churn, no selection loss, no reload loop. This is what makes the editor's own save
//! invisible to its hot-reload path while a genuinely external edit (another tool, a text
//! editor) still lands.
//!
//! External edits win over unsaved in-memory edits to the same chunk: the editor adopts the disk
//! form and the chunk's dirty flag clears (the file now is the truth). Surfacing a merge conflict
//! is undo/redo-era work, deferred with it.

use std::error::Error;

use wok_content::ChunkState;
use wok_scene::{Chunk, ChunkCoord, Heightmap, Scene};

use crate::content::ContentPaths;
use crate::model::EditorModel;

/// What applying one watcher-reported chunk change did to the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkOutcome {
    /// Disk matches the authored form in memory (typically the editor's own save): nothing done.
    Unchanged,
    /// The disk form replaced the in-memory form and the chunk re-transformed. The caller
    /// re-uploads the chunk's terrain mesh; the heightmap may have changed.
    Updated,
    /// The chunk file is gone: authored form dropped, runtime released.
    Removed,
}

/// Write every dirty chunk and (if dirty) the scene manifest, then mark the model clean. The
/// heightmaps are never dirty in v1 (no terrain sculpting), so only JSON is written.
pub fn save(model: &mut EditorModel, paths: &ContentPaths) -> Result<(), Box<dyn Error>> {
    for &coord in &model.dirty_chunks {
        if let Some(chunk) = model.chunks.get(&coord) {
            wok_scene::save_chunk(chunk, paths.chunk(coord))?;
        }
    }
    if model.scene_dirty {
        wok_scene::save_scene(&model.scene, paths.scene())?;
    }
    model.dirty_chunks.clear();
    model.scene_dirty = false;
    Ok(())
}

/// Apply one watcher-reported chunk change: compare disk against memory, adopt on difference.
/// `on_disk` is `None` when the chunk file no longer exists.
pub fn apply_external_chunk(
    model: &mut EditorModel,
    coord: ChunkCoord,
    on_disk: Option<(Chunk, Option<Heightmap>)>,
) -> Result<ChunkOutcome, Box<dyn Error>> {
    let Some((chunk, heightmap)) = on_disk else {
        if model.chunks.remove(&coord).is_none() {
            return Ok(ChunkOutcome::Unchanged);
        }
        model.heightmaps.remove(&coord);
        model.dirty_chunks.remove(&coord);
        if model.store.state(coord) == ChunkState::Loaded {
            model.store.release(coord)?;
        }
        model.validate_selection();
        return Ok(ChunkOutcome::Removed);
    };

    let same_chunk = model.chunks.get(&coord) == Some(&chunk);
    let same_terrain = model.heightmaps.get(&coord) == heightmap.as_ref();
    if same_chunk && same_terrain {
        return Ok(ChunkOutcome::Unchanged);
    }

    model.chunks.insert(coord, chunk);
    match heightmap {
        Some(hm) => {
            model.heightmaps.insert(coord, hm);
        }
        None => {
            model.heightmaps.remove(&coord);
        }
    }
    model.dirty_chunks.remove(&coord);
    model.retransform(coord)?;
    model.validate_selection();
    Ok(ChunkOutcome::Updated)
}

/// Apply a watcher-reported manifest change. Returns whether anything was adopted (false means
/// disk already matches memory, e.g. the editor's own save).
pub fn apply_external_scene(model: &mut EditorModel, on_disk: Scene) -> bool {
    if model.scene == on_disk {
        return false;
    }
    model.scene = on_disk;
    model.scene_dirty = false;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Selection;
    use crate::sample;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use glam::Vec3;
    use wok_scene::{InstanceId, PrefabRef, Transform};

    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("wok-sync-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .unwrap()
    }

    #[test]
    fn save_writes_dirty_files_marks_clean_and_round_trips_the_edit() {
        let dir = unique_temp_dir();
        let paths = ContentPaths::new(dir.clone());
        let mut model = sample_model();
        let coord = wok_scene::ChunkCoord::new(0, 0);
        let sel = Selection { coord, id: InstanceId(2) };
        let edited = Transform { translation: Vec3::new(20.0, 6.0, 25.0), ..Transform::IDENTITY };
        model.edit_placement(sel, edited, None).unwrap();
        model.place(&PrefabRef::new("pillar"), Vec3::new(50.0, 0.0, 50.0)).unwrap();
        assert!(model.is_dirty());

        save(&mut model, &paths).unwrap();
        assert!(!model.is_dirty());

        // The real load paths read back exactly the authored forms in memory.
        let chunk = wok_scene::load_chunk(paths.chunk(coord)).unwrap();
        assert_eq!(&chunk, model.chunks.get(&coord).unwrap());
        let scene = wok_scene::load_scene(paths.scene()).unwrap();
        assert_eq!(scene, model.scene);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_of_own_save_is_a_no_op_for_editor_state() {
        let dir = unique_temp_dir();
        let paths = ContentPaths::new(dir.clone());
        let mut model = sample_model();
        let coord = wok_scene::ChunkCoord::new(0, 0);
        let sel = model.place(&PrefabRef::new("crate"), Vec3::new(45.0, 0.0, 45.0)).unwrap().unwrap();
        save(&mut model, &paths).unwrap();

        // What the watcher-driven path would do: read the reported files back and apply.
        let on_disk = crate::content::load_chunk_with_heightmap(&paths, coord).ok();
        // The save wrote no heightmap; the editor's own copy stands in, as app reload does.
        let on_disk = on_disk.map(|(c, _)| (c, model.heightmaps.get(&coord).cloned()));
        let outcome = apply_external_chunk(&mut model, coord, on_disk).unwrap();
        assert_eq!(outcome, ChunkOutcome::Unchanged);
        assert_eq!(model.selection, Some(sel), "selection survives the echo");
        assert!(!model.is_dirty(), "clean stays clean: no reload loop");

        let scene = wok_scene::load_scene(paths.scene()).unwrap();
        assert!(!apply_external_scene(&mut model, scene));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_genuinely_external_chunk_edit_is_adopted_and_re_transformed() {
        let mut model = sample_model();
        let coord = wok_scene::ChunkCoord::new(0, 0);
        let mut external = model.chunks.get(&coord).unwrap().clone();
        external.placements.truncate(3);
        let heightmap = model.heightmaps.get(&coord).cloned();
        model.selection = Some(Selection { coord, id: InstanceId(7) });

        let outcome = apply_external_chunk(&mut model, coord, Some((external, heightmap))).unwrap();
        assert_eq!(outcome, ChunkOutcome::Updated);
        assert_eq!(model.placement_count(), 3);
        assert_eq!(model.store.get(coord).unwrap().hitboxes.len(), 3, "runtime re-transformed");
        assert_eq!(model.selection, None, "selection of a vanished placement clears");
    }

    #[test]
    fn a_deleted_chunk_file_releases_the_chunk() {
        let mut model = sample_model();
        let coord = wok_scene::ChunkCoord::new(0, 0);
        let outcome = apply_external_chunk(&mut model, coord, None).unwrap();
        assert_eq!(outcome, ChunkOutcome::Removed);
        assert!(model.chunks.is_empty());
        assert_eq!(model.store.iter_loaded().count(), 0);
    }
}
