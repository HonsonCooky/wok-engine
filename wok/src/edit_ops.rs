//! v2 editing operations: duplicate, rename, and the selection's world bounds.
//!
//! A second `impl EditorModel` block beside `crate::model` (which keeps construction, place,
//! delete, and edit), split by file purely for the size target. The same rules hold: every
//! placement mutation goes through the authored form, marks dirty, and re-transforms the chunk
//! when the runtime arrays are affected. Rename deliberately skips the re-transform - the display
//! name is pure annotation, and the sliced arrays carry no placement identity by design - so a
//! rename is a dirty flag and nothing else.

use glam::Vec3;
use wok_content::StoreError;
use wok_scene::Aabb;

use crate::model::{EditorModel, Selection, chunk_origin};
use crate::place;

/// Chunk-local offset a duplicate is placed at, so the copy is visibly beside its original
/// rather than z-fighting inside it. One metre diagonally: clearly a second object at placeholder
/// scale, small enough to stay in visual context.
pub const DUPLICATE_OFFSET_M: Vec3 = Vec3::new(1.0, 0.0, 1.0);

impl EditorModel {
    /// Duplicate a placement: an identical authored copy (transform offset by
    /// [`DUPLICATE_OFFSET_M`], name and state carried over) under a fresh instance id from the
    /// scene's monotonic counter, selected so the user is immediately manipulating the copy.
    /// Returns `None` when the selection no longer resolves.
    pub fn duplicate(&mut self, sel: Selection) -> Result<Option<Selection>, StoreError> {
        let Some(original) = self.placement(sel) else { return Ok(None) };
        let mut copy = original.clone();
        copy.instance_id = self.scene.allocate_instance_id();
        copy.transform.translation += DUPLICATE_OFFSET_M;
        self.scene_dirty = true;

        let id = copy.instance_id;
        let chunk = self.chunks.get_mut(&sel.coord).expect("placement(sel) found this chunk");
        chunk.placements.push(copy);
        self.dirty_chunks.insert(sel.coord);
        self.retransform(sel.coord)?;

        let selection = Selection { coord: sel.coord, id };
        self.selection.replace(selection);
        Ok(Some(selection))
    }

    /// Duplicate every selected placement and reselect the copies in order (primary last), so the
    /// user is immediately manipulating the new group rather than the originals. One pass over a
    /// snapshot of the set; the single checkpoint that makes it one undo step is taken by the
    /// writer before this runs. A member that no longer resolves is skipped.
    pub fn duplicate_selection(&mut self) -> Result<(), StoreError> {
        let targets: Vec<Selection> = self.selection.iter().collect();
        let mut copies: Vec<Selection> = Vec::with_capacity(targets.len());
        for sel in targets {
            if let Some(copy) = self.duplicate(sel)? {
                copies.push(copy);
            }
        }
        // `duplicate` left the selection on the last copy alone; widen it to the whole group.
        self.selection.extend(copies, false);
        Ok(())
    }

    /// Set or clear a placement's display name: trimmed, and an empty result clears back to
    /// `None` (the file omits the field again). Returns whether the placement existed. No
    /// re-transform: names never reach the runtime arrays.
    pub fn rename(&mut self, sel: Selection, name: &str) -> bool {
        let Some(chunk) = self.chunks.get_mut(&sel.coord) else { return false };
        let Some(placement) = chunk.placements.iter_mut().find(|p| p.instance_id == sel.id) else {
            return false;
        };
        let trimmed = name.trim();
        placement.name = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
        self.dirty_chunks.insert(sel.coord);
        true
    }

    /// The selection's conservative world-space bounds (the union of `world_aabb` over its
    /// resolved shapes, lifted by the chunk origin): what camera framing aims at. A shapeless
    /// (mesh-only) prefab has no bounds to union, so it frames as a unit box around the
    /// placement's position rather than an infinite one.
    pub fn world_bounds(&self, sel: Selection) -> Option<Aabb> {
        let placement = self.placement(sel)?;
        let origin = chunk_origin(sel.coord);
        let local = self
            .prefabs
            .get(&placement.prefab)
            .map(|prefab| place::prefab_bounds(prefab, &placement.transform));
        match local {
            Some(b) if b.min.is_finite() && b.max.is_finite() => {
                Some(Aabb::new(b.min + origin, b.max + origin))
            }
            _ => {
                let at = placement.transform.translation + origin;
                Some(Aabb::new(at - Vec3::splat(0.5), at + Vec3::splat(0.5)))
            }
        }
    }

    /// The selection's centroid: the mean of its members' world-bounds centres. `None` when nothing
    /// is selected (or no member still resolves). The Object-mode camera orbits this point, so it
    /// follows a drag-moved selection and sits in the middle of a multi-selection.
    pub fn selection_pivot(&self) -> Option<Vec3> {
        let mut sum = Vec3::ZERO;
        let mut n = 0u32;
        for sel in self.selection.iter() {
            if let Some(bounds) = self.world_bounds(sel) {
                sum += (bounds.min + bounds.max) * 0.5;
                n += 1;
            }
        }
        (n > 0).then(|| sum / n as f32)
    }

    /// The union of the selection's members' world bounds: what Object-mode framing fits in view.
    /// `None` when nothing is selected (or no member resolves).
    pub fn selection_bounds(&self) -> Option<Aabb> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for sel in self.selection.iter() {
            if let Some(bounds) = self.world_bounds(sel) {
                min = min.min(bounds.min);
                max = max.max(bounds.max);
                any = true;
            }
        }
        any.then(|| Aabb::new(min, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ContentPaths;
    use crate::sample;
    use crate::sync;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wok_scene::{ChunkCoord, InstanceId};

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("wok-editops-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn duplicate_allocates_a_fresh_monotonic_id_and_offsets_the_copy() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let next = model.scene.next_instance_id.0;
        let sel = Selection { coord, id: InstanceId(2) };
        model.rename(sel, "the big crate");
        let original = model.placement(sel).unwrap().clone();

        let copy_sel = model.duplicate(sel).unwrap().expect("duplicated");
        assert_eq!(copy_sel.id, InstanceId(next), "the copy takes the next monotonic id");
        assert_eq!(model.scene.next_instance_id.0, next + 1, "the counter advanced");
        assert_eq!(model.selection.primary(), Some(copy_sel), "the copy is selected");

        let copy = model.placement(copy_sel).unwrap();
        assert_eq!(
            copy.transform.translation,
            original.transform.translation + DUPLICATE_OFFSET_M,
            "visibly beside the original"
        );
        assert_eq!(copy.prefab, original.prefab);
        assert_eq!(copy.name, original.name, "the duplicate carries the authored name");
        assert_eq!(copy.transform.rotation, original.transform.rotation);
        // The original is untouched and both exist.
        assert_eq!(model.placement(sel).unwrap(), &original);
        assert!(model.scene_dirty && model.dirty_chunks.contains(&coord));
        // The runtime arrays re-transformed: one more hitbox than the sample had.
        assert_eq!(model.store.get(coord).unwrap().hitboxes.len(), 9);
    }

    #[test]
    fn duplicate_of_a_vanished_selection_does_nothing() {
        let mut model = sample_model();
        let gone = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(999) };
        assert_eq!(model.duplicate(gone).unwrap(), None);
        assert!(!model.is_dirty());
    }

    #[test]
    fn duplicate_selection_copies_every_member_and_selects_the_copies() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let a = Selection { coord, id: InstanceId(0) };
        let b = Selection { coord, id: InstanceId(3) };
        model.selection.toggle(a);
        model.selection.toggle(b);
        let count = model.placement_count();

        model.duplicate_selection().unwrap();
        assert_eq!(model.placement_count(), count + 2, "both members copied");
        assert!(model.placement(a).is_some() && model.placement(b).is_some(), "originals remain");
        // The selection now holds exactly the two copies, not the originals; each copy resolves.
        assert_eq!(model.selection.len(), 2, "the copies are selected");
        assert!(!model.selection.contains(a) && !model.selection.contains(b), "the originals are deselected");
        assert!(model.selection.iter().all(|copy| model.placement(copy).is_some()), "each copy resolves");
    }

    #[test]
    fn rename_round_trips_through_save_and_load_and_clears_to_none_on_empty() {
        let dir = unique_temp_dir();
        let paths = ContentPaths::new(dir.clone());
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let sel = Selection { coord, id: InstanceId(5) };

        assert!(model.rename(sel, "  watchtower  "), "rename finds the placement");
        assert_eq!(model.placement(sel).unwrap().name.as_deref(), Some("watchtower"), "trimmed");
        sync::save(&mut model, &paths).unwrap();
        let loaded = wok_scene::load_chunk(paths.chunk(coord)).unwrap();
        let on_disk = loaded.placements.iter().find(|p| p.instance_id == sel.id).unwrap();
        assert_eq!(on_disk.name.as_deref(), Some("watchtower"), "the name survives the file");

        // Empty (or whitespace) clears back to None, and the saved file omits the key again.
        assert!(model.rename(sel, "   "));
        assert_eq!(model.placement(sel).unwrap().name, None);
        sync::save(&mut model, &paths).unwrap();
        let json = std::fs::read_to_string(paths.chunk(coord)).unwrap();
        assert!(!json.contains("watchtower"), "the cleared name is gone from the file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_of_a_vanished_selection_reports_false() {
        let mut model = sample_model();
        let gone = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(999) };
        assert!(!model.rename(gone, "ghost"));
        assert!(!model.is_dirty());
    }

    #[test]
    fn world_bounds_lift_the_prefab_bounds_by_the_chunk_origin() {
        let model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let bounds = model.world_bounds(sel).expect("selection resolves");
        let placement = model.placement(sel).unwrap();
        let local = place::prefab_bounds(&model.prefabs[&placement.prefab], &placement.transform);
        // Chunk (0,0): origin zero, so world equals local; the lift itself is a pure add.
        assert_eq!(bounds.min, local.min);
        assert_eq!(bounds.max, local.max);
        assert!(bounds.min.is_finite() && bounds.max.is_finite());
    }

    #[test]
    fn selection_pivot_is_the_centroid_of_the_member_bounds_centres() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let a = Selection { coord, id: InstanceId(0) };
        let b = Selection { coord, id: InstanceId(3) };
        let (ba, bb) = (model.world_bounds(a).unwrap(), model.world_bounds(b).unwrap());
        let ca = (ba.min + ba.max) * 0.5;
        let cb = (bb.min + bb.max) * 0.5;

        // A single selection orbits its own bounds centre.
        model.selection.replace(a);
        assert!((model.selection_pivot().unwrap() - ca).length() < 1e-4, "one member: pivot is its centre");

        // Two members: the pivot is the mean of the two centres - the centroid the camera orbits.
        model.selection.toggle(b);
        let expected = (ca + cb) * 0.5;
        assert!((model.selection_pivot().unwrap() - expected).length() < 1e-4, "two members: the centroid");

        // The framing bounds are the union of the members' bounds (so a frame fits both).
        let union = model.selection_bounds().unwrap();
        assert_eq!(union.min, ba.min.min(bb.min));
        assert_eq!(union.max, ba.max.max(bb.max));
    }

    #[test]
    fn an_empty_selection_has_no_pivot_or_bounds() {
        let model = sample_model();
        assert!(model.selection.is_empty());
        assert_eq!(model.selection_pivot(), None);
        assert_eq!(model.selection_bounds(), None);
    }
}
