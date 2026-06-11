//! The editor's in-memory model: the authored forms it is the writer of, the runtime store they
//! transform into, selection, and the edit operations.
//!
//! v1 retains the authored chunks in memory (v0 consumed them into the store at startup): the
//! editor mutates the authored form and re-transforms the affected chunk, so data still flows
//! authored -> runtime only, never back. The runtime arrays carry no placement identity (a sliced
//! `Hitbox` has no `InstanceId` by design), so everything identity-shaped here - selection, the
//! scene tree, picking - reads the authored placements.
//!
//! Every operation that touches a chunk's placements re-transforms that chunk through the store
//! immediately (release, then load from a clone of the authored form), so the viewport always
//! draws what the authored data says. Dirty tracking is per chunk plus a scene flag (the
//! instance-id counter advances on place); `crate::sync` owns save and external-change
//! application.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::Vec3;
use wok_content::{ChunkState, ChunkStore, StoreError};
use wok_scene::{
    CHUNK_GRID_DIM, Chunk, ChunkCoord, Heightmap, InstanceId, Placement, Prefab, PrefabRef, Scene,
    Transform,
};

use crate::place;

/// Chunk side in metres, derived from the heightmap grid (128 one-metre cells; the 129th sample
/// is the shared edge). wok-scene deliberately does not bake the chunk size into ChunkCoord.
pub const CHUNK_SIZE_M: f32 = (CHUNK_GRID_DIM - 1) as f32;

/// World-space origin of a chunk: its grid coordinate times the chunk size.
pub fn chunk_origin(coord: ChunkCoord) -> Vec3 {
    Vec3::new(coord.x as f32 * CHUNK_SIZE_M, 0.0, coord.z as f32 * CHUNK_SIZE_M)
}

/// The chunk cell a world-space point falls in.
pub fn chunk_at(world: Vec3) -> ChunkCoord {
    ChunkCoord::new(
        (world.x / CHUNK_SIZE_M).floor() as i32,
        (world.z / CHUNK_SIZE_M).floor() as i32,
    )
}

/// World-space bounds of everything the loaded chunks hold - the shadow region the frame call
/// passes (caller policy per the render contract). Falls back to a small box around the origin
/// when nothing is loaded. Recomputed per frame because edits and hot reload can change the store
/// between any two frames; the scan is a few thousand min/max ops per chunk, frame-state-cheap.
pub fn scene_bounds(store: &ChunkStore) -> wok_scene::Aabb {
    use wok_scene::{Aabb, VisibleItem};
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut grow = |b: Aabb| {
        min = min.min(b.min);
        max = max.max(b.max);
    };
    for (coord, runtime) in store.iter_loaded() {
        let origin = chunk_origin(coord);
        let origin_mat = glam::Mat4::from_translation(origin);
        if let Some(mesh) = runtime.terrain_mesh.as_ref() {
            let b = mesh.bounds();
            grow(Aabb::new(b.min + origin, b.max + origin));
        }
        for item in &runtime.visible {
            if let VisibleItem::Primitive { primitive, transform, .. } = item {
                grow(wok_physics::world_aabb(*primitive, origin_mat * *transform));
            }
        }
        for hitbox in &runtime.hitboxes {
            grow(wok_physics::world_aabb(hitbox.primitive, origin_mat * hitbox.transform));
        }
    }
    if min.x > max.x {
        return Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
    }
    Aabb::new(min, max)
}

/// The selected placement: its chunk and its scene-stable instance id. The id alone would do
/// (ids are scene-unique), but carrying the chunk makes every lookup direct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub coord: ChunkCoord,
    pub id: InstanceId,
}

/// The editor's whole mutable model. Fields are read freely by the UI; mutation goes through the
/// operations below so the authored form, the store, and the dirty flags never drift apart.
pub struct EditorModel {
    pub scene: Scene,
    pub prefabs: HashMap<PrefabRef, Prefab>,
    pub chunks: BTreeMap<ChunkCoord, Chunk>,
    /// Authored heightmaps, kept so an edited chunk can re-transform without re-reading disk.
    /// Absent entry = chunk without terrain.
    pub heightmaps: BTreeMap<ChunkCoord, Heightmap>,
    pub store: ChunkStore,
    pub selection: Option<Selection>,
    pub dirty_chunks: BTreeSet<ChunkCoord>,
    /// The manifest changed (the instance-id counter advances on place).
    pub scene_dirty: bool,
}

impl EditorModel {
    /// Build the model from loaded content, transforming every chunk into the store.
    pub fn new(
        scene: Scene,
        prefabs: HashMap<PrefabRef, Prefab>,
        chunks: Vec<(Chunk, Option<Heightmap>)>,
    ) -> Result<EditorModel, StoreError> {
        let mut model = EditorModel {
            scene,
            prefabs,
            chunks: BTreeMap::new(),
            heightmaps: BTreeMap::new(),
            store: ChunkStore::new(),
            selection: None,
            dirty_chunks: BTreeSet::new(),
            scene_dirty: false,
        };
        for (chunk, heightmap) in chunks {
            let coord = chunk.coord;
            model.chunks.insert(coord, chunk);
            if let Some(hm) = heightmap {
                model.heightmaps.insert(coord, hm);
            }
            model.retransform(coord)?;
        }
        Ok(model)
    }

    pub fn is_dirty(&self) -> bool {
        self.scene_dirty || !self.dirty_chunks.is_empty()
    }

    pub fn placement_count(&self) -> usize {
        self.chunks.values().map(|c| c.placements.len()).sum()
    }

    /// The placement a selection points at, if it still exists.
    pub fn placement(&self, sel: Selection) -> Option<&Placement> {
        self.chunks
            .get(&sel.coord)?
            .placements
            .iter()
            .find(|p| p.instance_id == sel.id)
    }

    /// Drop the selection if its placement no longer exists (deleted, or removed by an external
    /// reload). Call after any operation that can remove placements.
    pub fn validate_selection(&mut self) {
        if let Some(sel) = self.selection
            && self.placement(sel).is_none()
        {
            self.selection = None;
        }
    }

    /// Re-transform one chunk from its authored form: release the old runtime arrays and load
    /// fresh ones from a clone. The authored-to-runtime direction of the HLD data flow, run
    /// eagerly so the viewport tracks every edit.
    pub fn retransform(&mut self, coord: ChunkCoord) -> Result<(), StoreError> {
        if self.store.state(coord) == ChunkState::Loaded {
            self.store.release(coord)?;
        }
        let Some(chunk) = self.chunks.get(&coord) else { return Ok(()) };
        self.store.load(chunk.clone(), self.heightmaps.get(&coord).cloned(), &self.prefabs)?;
        Ok(())
    }

    /// Apply an inspector edit: replace the placement's transform and state on the authored form
    /// and re-transform its chunk.
    pub fn edit_placement(
        &mut self,
        sel: Selection,
        transform: Transform,
        state: Option<String>,
    ) -> Result<(), StoreError> {
        let Some(chunk) = self.chunks.get_mut(&sel.coord) else { return Ok(()) };
        let Some(placement) = chunk.placements.iter_mut().find(|p| p.instance_id == sel.id) else {
            return Ok(());
        };
        placement.transform = transform;
        placement.state = state;
        self.dirty_chunks.insert(sel.coord);
        self.retransform(sel.coord)
    }

    /// Place a prefab at a world-space terrain point: rest it on the chunk's heightmap per its
    /// shape-derived policy, stamp a fresh instance id from the scene's monotonic counter, and
    /// select it. Returns `None` (placing nothing) when the point falls outside every authored
    /// chunk or the prefab is unknown.
    pub fn place(
        &mut self,
        prefab_ref: &PrefabRef,
        world_point: Vec3,
    ) -> Result<Option<Selection>, StoreError> {
        let coord = chunk_at(world_point);
        if !self.chunks.contains_key(&coord) {
            return Ok(None);
        }
        let Some(prefab) = self.prefabs.get(prefab_ref) else { return Ok(None) };

        let local = world_point - chunk_origin(coord);
        let floating = Transform { translation: local, ..Transform::IDENTITY };
        let transform = match self.heightmaps.get(&coord) {
            Some(heightmap) => {
                place::rest_on_terrain(prefab, place::rest_for_prefab(prefab), floating, heightmap)
            }
            None => floating,
        };

        let id = self.scene.allocate_instance_id();
        self.scene_dirty = true;
        let chunk = self.chunks.get_mut(&coord).expect("checked above");
        chunk.placements.push(Placement {
            prefab: prefab_ref.clone(),
            instance_id: id,
            name: None,
            transform,
            state: None,
        });
        self.dirty_chunks.insert(coord);
        self.retransform(coord)?;

        let selection = Selection { coord, id };
        self.selection = Some(selection);
        Ok(Some(selection))
    }

    /// Delete a placement from the authored form. The instance id is not reused (the scene
    /// counter never decrements). Returns whether anything was removed.
    pub fn delete(&mut self, sel: Selection) -> Result<bool, StoreError> {
        let Some(chunk) = self.chunks.get_mut(&sel.coord) else { return Ok(false) };
        let before = chunk.placements.len();
        chunk.placements.retain(|p| p.instance_id != sel.id);
        if chunk.placements.len() == before {
            return Ok(false);
        }
        self.dirty_chunks.insert(sel.coord);
        self.retransform(sel.coord)?;
        self.validate_selection();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;
    use glam::Quat;

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    #[test]
    fn place_then_delete_then_place_stays_monotonic() {
        let mut model = sample_model();
        let next = model.scene.next_instance_id.0;
        let at = chunk_origin(ChunkCoord::new(0, 0)) + Vec3::new(40.0, 0.0, 40.0);

        let first = model.place(&PrefabRef::new("crate"), at).unwrap().expect("placed");
        assert_eq!(first.id, InstanceId(next));
        assert!(model.delete(first).unwrap());
        let second = model.place(&PrefabRef::new("crate"), at).unwrap().expect("placed");
        assert_eq!(second.id, InstanceId(next + 1), "a deleted id is never reused");
        assert_eq!(model.scene.next_instance_id.0, next + 2);
        assert!(model.scene_dirty && model.dirty_chunks.contains(&ChunkCoord::new(0, 0)));
    }

    #[test]
    fn place_outside_every_chunk_places_nothing() {
        let mut model = sample_model();
        let counter = model.scene.next_instance_id;
        let far = Vec3::new(-500.0, 0.0, -500.0);
        assert_eq!(model.place(&PrefabRef::new("crate"), far).unwrap(), None);
        assert_eq!(model.scene.next_instance_id, counter, "no id burned on a miss");
        assert!(!model.is_dirty());
    }

    #[test]
    fn placed_prefab_rests_on_the_terrain() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let at = chunk_origin(coord) + Vec3::new(30.0, 0.0, 30.0);
        let sel = model.place(&PrefabRef::new("crate"), at).unwrap().expect("placed");
        let placement = model.placement(sel).unwrap();
        let prefab = &model.prefabs[&placement.prefab];
        let bounds = crate::place::prefab_bounds(prefab, &placement.transform);
        let heightmap = &model.heightmaps[&coord];
        // Corner rest: the bottom sits on the highest of the five resolve samples; on these
        // gentle hills that is within the cell's height range around the centre sample.
        let ground = heightmap.height_at(30.0, 30.0);
        assert!((bounds.min.y - ground).abs() < 0.5, "bottom {} vs ground {}", bounds.min.y, ground);
        assert!(bounds.min.y >= ground - 1e-3, "corner rest never sinks");
    }

    #[test]
    fn edit_round_trips_through_the_authored_form_and_the_re_slice() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let sel = Selection { coord, id: InstanceId(0) };
        let edited = Transform {
            translation: Vec3::new(12.0, 7.0, 34.0),
            rotation: Quat::from_rotation_y(0.5),
            scale: Vec3::splat(2.0),
        };
        model.edit_placement(sel, edited, Some("default".to_string())).unwrap();

        // The authored form holds exactly the edit.
        let placement = model.placement(sel).unwrap();
        assert_eq!(placement.transform, edited);
        assert_eq!(placement.state.as_deref(), Some("default"));

        // The re-slice reflects it: the first visible item is placement * shape, recomputed.
        let runtime = model.store.get(coord).expect("chunk re-transformed");
        let chunk = &model.chunks[&coord];
        let direct = wok_scene::slice_chunk(chunk, &model.prefabs).unwrap();
        assert_eq!(runtime.visible, direct.visible);
        assert_eq!(runtime.hitboxes, direct.hitboxes);
        // The file round trip of the edited form is covered by crate::sync's save tests, which
        // run the real save and load paths.
    }

    #[test]
    fn deleting_the_selected_placement_clears_the_selection() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        model.selection = Some(sel);
        assert!(model.delete(sel).unwrap());
        assert_eq!(model.selection, None);
        assert!(model.placement(sel).is_none());
        assert_eq!(model.placement_count(), 7);
    }

    #[test]
    fn chunk_at_floors_into_the_grid() {
        assert_eq!(chunk_at(Vec3::new(5.0, 0.0, 5.0)), ChunkCoord::new(0, 0));
        assert_eq!(chunk_at(Vec3::new(-0.1, 0.0, 130.0)), ChunkCoord::new(-1, 1));
    }
}
