//! The render residency: the runtime form of the open scene the 3D viewport draws, derived from the
//! authored scene and reconciled to it.
//!
//! This is separate from the authored [`LoadedScene`](crate::loaded::LoadedScene), which holds the
//! editable placements and writes them back to disk. `RenderScene` is the read-only runtime side: it
//! loads the ancillary data the authored model does not keep - the prefab library, the per-chunk
//! heightmaps, and the scene's light state - and transforms the authored chunks through wok-content
//! into the runtime arrays (`visible`, terrain) that wok-render draws. Composition only, per the HLD's
//! application layer; nothing here writes to disk.
//!
//! Derived from the authored scene so edits show. [`reconcile`] rebuilds the whole residency when the
//! open scene's identity (root + name) changes, and otherwise re-derives the visible items from the
//! authored chunks whenever they differ from what the store was last built from - so moving an
//! instance in the inspector updates the 3D the same frame. The re-derivation transforms with no
//! heightmap (terrain cannot change without a scene reload in this bite), so the cached terrain GPU
//! meshes (`crate::render::Gpu`) stay put and only the cheap slice re-runs; the per-frame
//! [`scene_bounds`](RenderScene::scene_bounds) leans on the terrain bounds cached at build time.
//!
//! Chunk-origin composition (runtime arrays are chunk-local; the viewport draws in world space) is
//! application policy the engine leaves to the caller, so [`chunk_origin`] and [`scene_bounds`] live
//! here - the same constants taste and the renderer derive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use glam::{Mat4, Vec3};
use wok_content::ChunkStore;
use wok_light::LightState;
use wok_scene::{
    Aabb, CHUNK_GRID_DIM, Chunk, ChunkCoord, ContentLayout, Heightmap, InstanceId, Prefab, PrefabRef,
    Primitive, Scene, VisibleItem,
};

use crate::camera::FlyCamera;
use crate::loaded::LoadedScene;
use crate::render::Gpu;

/// Chunk side in metres, derived from the heightmap grid (128 one-metre cells; the 129th sample is
/// the shared edge). wok-scene deliberately does not bake the chunk size into ChunkCoord, so this
/// composition is application policy (the same constant taste derives).
pub const CHUNK_SIZE_M: f32 = (CHUNK_GRID_DIM - 1) as f32;

/// Render distance used when a scene's manifest cannot be read (the streaming extent is unknown): a
/// few chunks out, enough to frame the in-memory content the viewport still draws.
const FALLBACK_RENDER_DISTANCE: f32 = CHUNK_SIZE_M * 3.0;

/// World-space origin of a chunk: its grid coordinate times the chunk size.
pub fn chunk_origin(coord: ChunkCoord) -> Vec3 {
    Vec3::new(coord.x as f32 * CHUNK_SIZE_M, 0.0, coord.z as f32 * CHUNK_SIZE_M)
}

/// The open scene's runtime residency for the viewport: the prefab library and light state loaded
/// from disk, the runtime chunk store derived from the authored chunks, the terrain bounds cached for
/// the shadow region, and the authored chunks the store was last built from (the edit-detection key).
/// Built and kept by [`reconcile`]; held as an `Option` on the app for as long as a scene tab is open.
pub struct RenderScene {
    root: PathBuf,
    name: String,
    prefabs: HashMap<PrefabRef, Prefab>,
    /// The scene's light state; the renderer reads it each frame. Loaded at build (a missing or
    /// malformed state falls back to a neutral default rather than failing the viewport).
    pub light: LightState,
    /// The scene's render distance in metres - its streaming extent (`load_radius` chunks), read
    /// from the manifest at build. The far plane sits here (see [`far_plane`](Self::far_plane)); it
    /// no longer derives from fog, which may be off. Persists across a re-derive (the manifest does
    /// not change without a scene reload).
    render_distance: f32,
    /// The runtime arrays the viewport draws: visible items (re-derived on edit) and, at build time,
    /// the terrain mesh per chunk (consumed once to upload the GPU terrain, then dropped on a
    /// re-derive). The renderer reads `visible` from here and the terrain from the GPU cache.
    pub store: ChunkStore,
    /// The world-space bounds of the terrain, cached at build (terrain does not change without a
    /// scene reload, and a re-derive drops the store's terrain meshes), unioned with the live visible
    /// items in [`scene_bounds`](Self::scene_bounds). `None` for a scene with no terrain.
    terrain_bounds: Option<Aabb>,
    /// The per-chunk heightmaps, cached at build keyed by chunk coord. Cached here because a re-derive
    /// rebuilds the store with no heightmap (the GPU terrain is cached separately), which would
    /// otherwise drop them after the first edit; terrain does not change without a scene reload, so a
    /// build-time copy stays valid. The surface query samples these to rest an instance on the ground
    /// ([`surface_ray`](Self::surface_ray) -> [`terrain_height_at`](Self::terrain_height_at)); parked
    /// with that query until brief 2's drag-and-drop move calls it.
    #[allow(dead_code)]
    heightmaps: HashMap<ChunkCoord, Heightmap>,
    /// The authored chunks the store was last derived from. [`reconcile`] re-derives when the live
    /// authored chunks differ from this, so an inspector edit shows in the 3D without disk I/O.
    source_chunks: Vec<Chunk>,
}

impl RenderScene {
    /// Build the residency for the scene `name` under `root`, deriving from `chunks` (the authored
    /// placements, the live editable source). Loads the prefab library, the per-chunk heightmaps, and
    /// the scene's light state from disk, then transforms each authored chunk (with its heightmap)
    /// into the store. Tolerant: a missing or malformed prefab, heightmap, or light state degrades to
    /// a skip or a default rather than failing the viewport (the editor must not crash on partial
    /// content). Terrain bounds are cached for the shadow region.
    pub fn build(root: &Path, name: &str, chunks: &[Chunk]) -> RenderScene {
        let layout = ContentLayout::new(root);
        let prefabs = load_prefabs(&layout);
        // Read the manifest once for the two things the renderer needs from it - the default
        // lighting name and the streaming extent (the render distance) - tolerating its absence.
        let manifest = load_manifest(&layout, name);
        let light = load_light(&layout, manifest.as_ref());
        let render_distance = manifest
            .as_ref()
            .map_or(FALLBACK_RENDER_DISTANCE, |m| m.default_streaming.render_distance());

        let mut heightmaps = HashMap::new();
        let mut store = ChunkStore::new();
        for chunk in chunks {
            let heightmap = heightmap_for(&layout, name, chunk.coord);
            // Cache the heightmap before it moves into the store, so the surface query can sample
            // terrain even after a re-derive drops the store's copy.
            if let Some(h) = &heightmap {
                heightmaps.insert(chunk.coord, h.clone());
            }
            if let Err(err) = store.load(chunk.clone(), heightmap, &prefabs) {
                eprintln!("wok: chunk {}_{} did not load for render: {err}", chunk.coord.x, chunk.coord.z);
            }
        }
        let terrain_bounds = terrain_bounds(&store);

        RenderScene {
            root: root.to_path_buf(),
            name: name.to_owned(),
            prefabs,
            light,
            render_distance,
            store,
            terrain_bounds,
            heightmaps,
            source_chunks: chunks.to_vec(),
        }
    }

    /// Re-derive the visible items from the authored `chunks` after an in-memory edit: rebuild the
    /// store transforming with no heightmap (terrain is unchanged this bite, and its GPU meshes are
    /// cached separately), so a moved or renamed-and-restated instance updates in the 3D. The prefab
    /// library and light state are unchanged, so they are reused.
    fn rederive(&mut self, chunks: &[Chunk]) {
        let mut store = ChunkStore::new();
        for chunk in chunks {
            if let Err(err) = store.load(chunk.clone(), None, &self.prefabs) {
                eprintln!("wok: chunk {}_{} did not re-derive for render: {err}", chunk.coord.x, chunk.coord.z);
            }
        }
        self.store = store;
        self.source_chunks = chunks.to_vec();
    }

    /// Spawn the god-cam over the first loaded chunk, mid-south looking north across it, a little above
    /// the terrain there (or above the origin plane when the chunk has no terrain). Called right after
    /// a fresh [`build`](Self::build), where the store still carries the chunks' heightmaps.
    pub fn spawn_camera(&self) -> FlyCamera {
        let half = CHUNK_SIZE_M * 0.5;
        let south = CHUNK_SIZE_M * 0.8;
        let (origin, ground) = self.store.iter_loaded().next().map_or((Vec3::ZERO, 0.0), |(coord, runtime)| {
            let ground = runtime.heightmap.as_ref().map_or(0.0, |h| h.height_at(half, south));
            (chunk_origin(coord), ground)
        });
        FlyCamera { position: origin + Vec3::new(half, ground + 12.0, south), yaw: 0.0, pitch: -0.15 }
    }

    /// The far clip distance: the scene's streaming extent (`load_radius` chunks, the farthest
    /// anything loads), floored so a degenerate zero-radius scene still yields a sane projection.
    /// Independent of fog now (HLD): a fog-off scene gets a clean cut here, and a fog-on scene
    /// saturates before it just as it did when the plane was fog-derived.
    pub fn far_plane(&self) -> f32 {
        self.render_distance.max(50.0)
    }

    /// World-space bounds of everything the loaded chunks draw - the cached terrain bounds plus the
    /// live placed visible extents - the shadow region the render pass passes (caller policy per the
    /// render contract). Conservative AABBs are the right tool: a shadow region wants cover, not fit.
    /// Falls back to a small box around the origin when nothing is loaded, so the shadow fit stays
    /// well-formed.
    pub fn scene_bounds(&self) -> Aabb {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut grow = |b: Aabb| {
            min = min.min(b.min);
            max = max.max(b.max);
        };
        if let Some(terrain) = self.terrain_bounds {
            grow(terrain);
        }
        for (coord, runtime) in self.store.iter_loaded() {
            let origin = Mat4::from_translation(chunk_origin(coord));
            for item in &runtime.visible {
                if let VisibleItem::Primitive { primitive, transform, .. } = item {
                    grow(wok_physics::world_aabb(*primitive, origin * *transform));
                }
            }
        }
        if min.x > max.x {
            return Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        }
        Aabb::new(min, max)
    }

    /// Cast a ray (`origin`, and a normalized `dir` - the cursor ray from
    /// [`FlyCamera::cursor_ray`](crate::camera::FlyCamera::cursor_ray)) against the open scene's
    /// authored placements, returning the [`InstanceId`] of the nearest one hit, or `None` when the
    /// ray meets only terrain or empty space (the empty-space deselect). The 3D viewport's
    /// click-to-select.
    ///
    /// It runs over the authored chunks (not the sliced visible items), so every shape of a
    /// placement's active state is tested - visible placeholders and invisible hitboxes alike -
    /// against the collider [`classify_collider`](wok_physics::classify_collider) reduces it to, the
    /// same reduction the game's simulation uses, so a pick agrees with what the placement collides
    /// as. A mesh-replaced instance, whose visible shapes do not draw, is still selectable by its
    /// authored footprint. Tolerant like [`build`](Self::build) and [`rederive`](Self::rederive): a
    /// placement whose prefab or active state is absent is skipped rather than failing the pick.
    /// Terrain is not a placement, so a ray meeting only terrain returns `None`.
    pub fn pick(&self, origin: Vec3, dir: Vec3) -> Option<InstanceId> {
        let mut best: Option<(f32, InstanceId)> = None;
        for chunk in &self.source_chunks {
            let origin_mat = Mat4::from_translation(chunk_origin(chunk.coord));
            for placement in &chunk.placements {
                let Some(prefab) = self.prefabs.get(&placement.prefab) else { continue };
                let state_name = placement.state.as_deref().unwrap_or(&prefab.default_state);
                let Some(state) = prefab.states.iter().find(|s| s.name == state_name) else { continue };
                let placement_mat = origin_mat * placement.transform.to_mat4();
                for shape in &state.shapes {
                    let world = placement_mat * shape.transform.to_mat4();
                    let collider = wok_physics::classify_collider(shape.primitive, world);
                    if let Some(t) = wok_physics::ray_collider(origin, dir, &collider) {
                        if best.is_none_or(|(best_t, _)| t < best_t) {
                            best = Some((t, placement.instance_id));
                        }
                    }
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// Cast a ray against the scene's surfaces - the prefab colliders and the terrain - and return the
    /// world point where it first lands, or `None` when it meets neither (the cursor is over empty
    /// sky). The editor's drag-and-drop move (brief 2) rests the moved instance on whatever lies under
    /// the cursor; `exclude` is that instance, whose own colliders are skipped so it snaps to the ground
    /// or to other prefabs, never to itself.
    ///
    /// The nearer of two hits wins. The prefab colliders are tested exactly, the same
    /// `classify_collider` -> [`ray_collider`](wok_physics::ray_collider) path as [`pick`](Self::pick)
    /// (so a snap lands on what a placement collides as), over the authored chunks. The terrain is then
    /// marched no farther than the nearest collider - a closer solid occludes the ground - by sampling
    /// the cached heightmaps, a stepped sample precise enough for snapping (the inspector is the exact
    /// path). Returns `origin + dir * t` for the winning `t`.
    ///
    /// Parked under `#[allow(dead_code)]` with [`instance_aabb`](Self::instance_aabb) and the private
    /// surface-query helpers below: the held-key move that drove it was removed in the interaction
    /// demolition, and brief 2's drag-and-drop is its caller (designs/movement-camera-design.md).
    #[allow(dead_code)]
    pub fn surface_ray(&self, origin: Vec3, dir: Vec3, exclude: InstanceId) -> Option<Vec3> {
        // Nearest prefab-collider hit, skipping the moving instance.
        let mut best: Option<f32> = None;
        self.for_each_excluded_shape(exclude, |primitive, world| {
            let collider = wok_physics::classify_collider(primitive, world);
            if let Some(t) = wok_physics::ray_collider(origin, dir, &collider) {
                best = Some(best.map_or(t, |b| b.min(t)));
            }
        });
        // Terrain, no farther than the nearest collider. A hit within that range is necessarily the
        // nearer (or equal), so it wins outright.
        let march_to = best.unwrap_or_else(|| self.far_plane());
        if let Some(t) = ray_heightfield(origin, dir, march_to, |x, z| self.terrain_height_at(x, z)) {
            best = Some(t);
        }
        best.map(|t| origin + dir * t)
    }

    /// The world-space AABB of one instance's prefab shapes at its live transform (rotation and scale
    /// included), or `None` when the instance resolves to no shape (an unknown prefab/state, or a
    /// mesh-only state with no placeholder shapes). The drag-and-drop move (brief 2) reads this to rest
    /// the item's bottom on the surface rather than its (often centered) origin; like the shadow region
    /// it unions each shape's conservative world AABB. Y is chunk-origin-independent (a chunk origin has
    /// zero height), so `min.y` reads directly as the item's lowest point in world space. Parked with
    /// the surface query (see [`surface_ray`](Self::surface_ray)).
    #[allow(dead_code)]
    pub fn instance_aabb(&self, id: InstanceId) -> Option<Aabb> {
        let mut bounds: Option<Aabb> = None;
        self.for_each_shape(|shape_id, primitive, world| {
            if shape_id != id {
                return;
            }
            let aabb = wok_physics::world_aabb(primitive, world);
            bounds = Some(bounds.map_or(aabb, |b| Aabb::new(b.min.min(aabb.min), b.max.max(aabb.max))));
        });
        bounds
    }

    /// Visit each authored shape: its placement's instance id, primitive, and world transform. The
    /// shared traversal behind the surface queries and the per-instance bounds; each caller filters by
    /// id as it needs. (`pick` keeps its own copy: it tracks the nearest id across all instances.)
    #[allow(dead_code)]
    fn for_each_shape(&self, mut visit: impl FnMut(InstanceId, Primitive, Mat4)) {
        for chunk in &self.source_chunks {
            let origin_mat = Mat4::from_translation(chunk_origin(chunk.coord));
            for placement in &chunk.placements {
                let Some(prefab) = self.prefabs.get(&placement.prefab) else { continue };
                let state_name = placement.state.as_deref().unwrap_or(&prefab.default_state);
                let Some(state) = prefab.states.iter().find(|s| s.name == state_name) else { continue };
                let placement_mat = origin_mat * placement.transform.to_mat4();
                for shape in &state.shapes {
                    visit(placement.instance_id, shape.primitive, placement_mat * shape.transform.to_mat4());
                }
            }
        }
    }

    /// Visit each authored shape (primitive and world transform) of every placement except `exclude`,
    /// so the dragged instance never snaps to itself - the surface queries' filter over
    /// [`for_each_shape`](Self::for_each_shape).
    #[allow(dead_code)]
    fn for_each_excluded_shape(&self, exclude: InstanceId, mut visit: impl FnMut(Primitive, Mat4)) {
        self.for_each_shape(|id, primitive, world| {
            if id != exclude {
                visit(primitive, world);
            }
        });
    }

    /// World terrain height at world `(x, z)`, or `None` off the loaded terrain (no chunk there, or one
    /// with no heightmap). Resolves the chunk from the world coordinate and samples its cached
    /// heightmap in chunk-local space; the chunk origin's height is zero, so the sample is the world
    /// height. Reads the heightmaps cached at build, which survive an edit's re-derive.
    #[allow(dead_code)]
    fn terrain_height_at(&self, x: f32, z: f32) -> Option<f32> {
        let coord = ChunkCoord::new((x / CHUNK_SIZE_M).floor() as i32, (z / CHUNK_SIZE_M).floor() as i32);
        let heightmap = self.heightmaps.get(&coord)?;
        let origin = chunk_origin(coord);
        Some(heightmap.height_at(x - origin.x, z - origin.z))
    }
}

/// Reconcile the render residency to the open authored scene, uploading or dropping the terrain GPU
/// meshes as the scene changes. Returns whether a scene was newly (re)built - the caller spawns the
/// god-cam over it when so.
///
/// A scene that failed to author-load renders as the empty well, like no scene at all. When the open
/// scene's identity (root + name) changes, the whole residency is rebuilt from disk and its terrain
/// uploaded. Otherwise, when the authored chunks differ from what the store was last derived from (an
/// in-memory edit), the visible items are re-derived - no disk I/O, the cached terrain stays. Needs a
/// device for the terrain upload, so it runs from the frame loop, never a pure action.
pub fn reconcile(
    render: &mut Option<RenderScene>,
    gpu: &mut Gpu,
    platform: &wok_platform::Platform,
    loaded: Option<&LoadedScene>,
) -> bool {
    let Some(loaded) = loaded.filter(|l| l.error().is_none()) else {
        if render.is_some() {
            *render = None;
            gpu.clear_terrain();
        }
        return false;
    };

    let identity_changed = render
        .as_ref()
        .is_none_or(|r| r.root.as_path() != loaded.root() || r.name.as_str() != loaded.name());
    if identity_changed {
        let built = RenderScene::build(loaded.root(), loaded.name(), loaded.chunks());
        gpu.set_terrain(platform, &built.store);
        *render = Some(built);
        return true;
    }

    // Same scene: reflect in-memory edits by re-deriving when the authored chunks have changed.
    let residency = render.as_mut().expect("an unchanged identity implies a residency");
    if residency.source_chunks.as_slice() != loaded.chunks() {
        residency.rederive(loaded.chunks());
    }
    false
}

/// Load every prefab under `assets/prefabs/`, keyed by file-stem slug. Tolerant: the engine-owned
/// discovery is sorted and skips a missing folder; a prefab that fails to parse is skipped with a
/// note rather than failing the whole viewport.
fn load_prefabs(layout: &ContentLayout) -> HashMap<PrefabRef, Prefab> {
    let mut prefabs = HashMap::new();
    for slug in layout.prefab_slugs() {
        match wok_scene::load_prefab(layout.prefab(&slug)) {
            Ok(prefab) => {
                prefabs.insert(PrefabRef::new(&slug), prefab);
            }
            Err(err) => eprintln!("wok: prefab {slug:?} did not load, skipping: {err}"),
        }
    }
    prefabs
}

/// Read the scene manifest (`scene.json`). The build reads it once for the two things the renderer
/// needs - the default lighting name and the streaming extent (the render distance) - and tolerates
/// its absence: a missing or malformed manifest returns `None`, and the caller falls back to a
/// neutral light state and a default render distance so the viewport still opens over a content gap.
fn load_manifest(layout: &ContentLayout, scene: &str) -> Option<Scene> {
    match wok_scene::load_scene(layout.scene_json(scene)) {
        Ok(manifest) => Some(manifest),
        Err(err) => {
            eprintln!("wok: scene manifest did not load, using defaults: {err}");
            None
        }
    }
}

/// The scene's default light state: read `manifest`'s `default_lighting` name, then that state under
/// `assets/lighting/`. A missing manifest or a missing/malformed state falls back to a neutral
/// daytime default, so the viewport always has lighting and never fails to open over a content gap.
fn load_light(layout: &ContentLayout, manifest: Option<&Scene>) -> LightState {
    let Some(manifest) = manifest else {
        return LightState::default();
    };
    let name = manifest.default_lighting.as_str();
    match wok_light::load_light_state(layout.lighting(name)) {
        Ok((_, state)) => state,
        Err(err) => {
            eprintln!("wok: light state {name:?} did not load, using default: {err}");
            LightState::default()
        }
    }
}

/// One chunk's sibling heightmap, or `None` when it has no terrain (a missing file is not an error).
fn heightmap_for(layout: &ContentLayout, scene: &str, coord: ChunkCoord) -> Option<Heightmap> {
    let path = layout.heightmap(scene, coord);
    if !path.exists() {
        return None;
    }
    match wok_scene::load_heightmap(path) {
        Ok(heightmap) => Some(heightmap),
        Err(err) => {
            eprintln!("wok: heightmap {}_{} did not load: {err}", coord.x, coord.z);
            None
        }
    }
}

/// The world-space bounds of every loaded chunk's terrain, or `None` when no chunk has terrain.
fn terrain_bounds(store: &ChunkStore) -> Option<Aabb> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (coord, runtime) in store.iter_loaded() {
        if let Some(mesh) = runtime.terrain_mesh.as_ref() {
            let bounds = mesh.bounds();
            let origin = chunk_origin(coord);
            min = min.min(bounds.min + origin);
            max = max.max(bounds.max + origin);
        }
    }
    (min.x <= max.x).then(|| Aabb::new(min, max))
}

/// Step between height samples along the surface march, in metres. Coarse enough to stay cheap over a
/// scene-far ray, fine enough that the interpolated crossing snaps cleanly.
#[allow(dead_code)]
const TERRAIN_MARCH_STEP_M: f32 = 0.5;

/// March the ray `origin + t*dir` (`dir` normalized) against a height field, returning the `t` where it
/// first dips to or below the surface within `max_t`, or `None` when it never does. A stepped sample
/// (canon: fine for snapping): the surface is read every [`TERRAIN_MARCH_STEP_M`], and the bracket
/// where the ray crosses is interpolated by the signed gap on each side, so the snap is not stair-
/// stepped to the stride. `sample` returns the world height at `(x, z)`, or `None` off the terrain - a
/// gap is not a crossing, so it breaks the above-the-surface tracking rather than registering a hit.
#[allow(dead_code)]
fn ray_heightfield(origin: Vec3, dir: Vec3, max_t: f32, sample: impl Fn(f32, f32) -> Option<f32>) -> Option<f32> {
    // The previous on-terrain sample: its t and signed gap (ray height minus terrain height), kept to
    // interpolate the crossing. Cleared on a gap, so a bracket never spans terrain-less space.
    let mut prev: Option<(f32, f32)> = None;
    let mut t = 0.0;
    while t <= max_t {
        let p = origin + dir * t;
        if let Some(h) = sample(p.x, p.z) {
            let gap = p.y - h;
            if gap <= 0.0 {
                return Some(match prev {
                    // A bracket from above (gap > 0) to on/below: interpolate where the gap hits zero.
                    Some((tp, gp)) if gp > 0.0 => tp + (t - tp) * gp / (gp - gap),
                    // The march started on or under the terrain: it is already there.
                    _ => t,
                });
            }
            prev = Some((t, gap));
        } else {
            prev = None;
        }
        t += TERRAIN_MARCH_STEP_M;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    use wok_scene::{
        CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, Eagerness, Heightmap, InstanceId, LightStateRef,
        Placement, Prefab, PrefabState, Primitive, Scene, Shape, StreamingDefaults, SurfaceTag, Transform,
        save_heightmap, save_prefab, save_scene,
    };

    #[allow(clippy::float_cmp)]
    #[test]
    fn chunk_origin_scales_by_the_chunk_size() {
        assert_eq!(chunk_origin(ChunkCoord::new(0, 0)), Vec3::ZERO);
        assert_eq!(chunk_origin(ChunkCoord::new(2, -1)), Vec3::new(256.0, 0.0, -128.0));
    }

    fn unique_temp_root() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-render-scene-{}-{}", std::process::id(), n))
    }

    /// Seed the disk content `RenderScene::build` reads (the manifest for the lighting name, one
    /// resolvable prefab, and that light state); the authored chunks are passed to `build` in memory,
    /// so the chunk file itself is not needed on disk.
    fn seed_content(root: &Path, scene: &str) {
        let layout = ContentLayout::new(root);
        std::fs::create_dir_all(layout.scene_dir(scene)).unwrap();
        let manifest = Scene {
            name: scene.to_string(),
            default_lighting: LightStateRef::new("noon"),
            regions: vec![],
            default_streaming: StreamingDefaults { load_radius: 3, default_eagerness: Eagerness::Eager },
            next_instance_id: InstanceId(5),
        };
        save_scene(&manifest, layout.scene_json(scene)).unwrap();

        std::fs::create_dir_all(layout.prefabs_dir()).unwrap();
        let block = Prefab {
            states: vec![PrefabState {
                name: "default".to_string(),
                shapes: vec![Shape {
                    primitive: Primitive::Cube,
                    transform: Transform::IDENTITY,
                    surface: Some(SurfaceTag::new("stone")),
                    is_hitbox: true,
                    is_visible: true,
                }],
                mesh: None,
            }],
            default_state: "default".to_string(),
        };
        save_prefab(&block, layout.prefab("block")).unwrap();

        std::fs::create_dir_all(layout.lighting_dir()).unwrap();
        wok_light::save_light_state(&LightState::default(), layout.lighting("noon")).unwrap();
    }

    /// Write a flat heightmap (every cell at `height_m`, one surface tag) for `coord` under the scene,
    /// so `RenderScene::build` loads terrain the surface-snap move can sample.
    fn save_flat_heightmap(root: &Path, scene: &str, coord: ChunkCoord, height_m: f32) {
        let layout = ContentLayout::new(root);
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
                .unwrap();
        save_heightmap(&heightmap, layout.heightmap(scene, coord)).unwrap();
    }

    /// One chunk placing a single `block` instance at `transform`.
    fn one_block(transform: Transform) -> Chunk {
        Chunk {
            coord: ChunkCoord::new(0, 0),
            placements: vec![Placement {
                prefab: PrefabRef::new("block"),
                instance_id: InstanceId(0),
                name: None,
                transform,
                state: None,
            }],
            streaming: ChunkStreaming::default(),
        }
    }

    fn visible_transform(runtime: &wok_content::ChunkRuntime) -> Mat4 {
        match &runtime.visible[0] {
            VisibleItem::Primitive { transform, .. } => *transform,
            other => panic!("expected a primitive visible item, got {other:?}"),
        }
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn build_derives_the_visible_items_and_a_finite_shadow_region() {
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");

        let chunks = vec![one_block(Transform::IDENTITY)];
        let scene = RenderScene::build(&root, "village", &chunks);

        // The block resolved through the prefab library into one visible item (no heightmap seeded, so
        // no terrain), and the shadow region and far plane are well-formed.
        let (_, runtime) = scene.store.iter_loaded().next().expect("the one chunk is loaded");
        assert_eq!(runtime.visible.len(), 1, "the block prefab's one visible shape");
        assert!(scene.scene_bounds().min.is_finite() && scene.scene_bounds().max.is_finite());
        // The far plane is the streaming extent (seed_content's load_radius 3 x 128m), not derived
        // from fog - the decoupling.
        assert!(scene.far_plane() >= 50.0);
        assert_eq!(scene.far_plane(), 3.0 * CHUNK_SIZE_M);
        // The god-cam spawns above the scene, finite and looking down a little.
        let cam = scene.spawn_camera();
        assert!(cam.position.is_finite() && cam.pitch < 0.0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rederive_reflects_a_moved_placement() {
        // The core of the design call: an inspector edit to a placement transform shows in the 3D
        // without a disk reload. Build from the identity placement, then re-derive from a moved copy
        // (the authored chunks the inspector mutates) and confirm the runtime visible transform moved.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");

        let chunks = vec![one_block(Transform::IDENTITY)];
        let mut scene = RenderScene::build(&root, "village", &chunks);
        let before = visible_transform(scene.store.iter_loaded().next().unwrap().1);

        let mut moved = chunks.clone();
        moved[0].placements[0].transform.translation = Vec3::new(5.0, 0.0, -3.0);
        scene.rederive(&moved);

        let after = visible_transform(scene.store.iter_loaded().next().unwrap().1);
        assert_ne!(before, after, "the re-derive carries the moved placement into the runtime arrays");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn surface_ray_lands_on_the_nearest_of_prefab_and_terrain_and_skips_the_excluded() {
        // The surface-snap move's spatial query: the nearer of a prefab collider and the terrain, with
        // the moving instance excluded so it never snaps to itself.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");
        save_flat_heightmap(&root, "village", ChunkCoord::new(0, 0), 0.0);

        // A unit-cube block centred at (10, 0, 10): its top face sits at y = 0.5 over flat terrain at 0.
        let placed = Transform { translation: Vec3::new(10.0, 0.0, 10.0), ..Transform::IDENTITY };
        let scene = RenderScene::build(&root, "village", &[one_block(placed)]);

        // Straight down onto the block lands on its top face (the collider is nearer than the ground).
        let on_block =
            scene.surface_ray(Vec3::new(10.0, 10.0, 10.0), Vec3::NEG_Y, InstanceId(99)).expect("hits the block");
        assert!((on_block - Vec3::new(10.0, 0.5, 10.0)).length() < 0.05, "on the block top: {on_block:?}");

        // Excluding the block (the instance being moved) falls through to the terrain beneath it.
        let through =
            scene.surface_ray(Vec3::new(10.0, 10.0, 10.0), Vec3::NEG_Y, InstanceId(0)).expect("hits the terrain");
        assert!((through - Vec3::new(10.0, 0.0, 10.0)).length() < 0.05, "through to the ground: {through:?}");

        // A ray over empty ground lands on the terrain; a ray at the sky meets neither.
        let on_ground =
            scene.surface_ray(Vec3::new(50.0, 10.0, 50.0), Vec3::NEG_Y, InstanceId(0)).expect("hits the terrain");
        assert!((on_ground - Vec3::new(50.0, 0.0, 50.0)).length() < 0.05, "on the ground: {on_ground:?}");
        assert!(scene.surface_ray(Vec3::new(50.0, 10.0, 50.0), Vec3::Y, InstanceId(0)).is_none(), "sky is empty");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn surface_ray_under_a_floating_prefab_lands_on_the_terrain_the_aim_points_at() {
        // The under-prefab fix: G sources its height from where the cursor ray actually lands
        // (surface_ray), not the column top, so aiming at terrain beneath a prefab rests on the ground -
        // no teleport onto the prefab. A unit cube floats at (10, 3, 10); a ray angled in under it lands
        // on the terrain at its column (which target_surface then snaps to the 1m grid floor, here 0).
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");
        save_flat_heightmap(&root, "village", ChunkCoord::new(0, 0), 0.0);
        let floating = Transform { translation: Vec3::new(10.0, 3.0, 10.0), ..Transform::IDENTITY };
        let scene = RenderScene::build(&root, "village", &[one_block(floating)]);

        // From low and to the south, angled down to the ground directly under the floating cube: the ray
        // passes beneath the cube (y ~ 0 through its column) and meets the terrain, never the cube top.
        let origin = Vec3::new(10.0, 2.0, 0.0);
        let dir = (Vec3::new(10.0, 0.0, 10.0) - origin).normalize();
        let hit = scene.surface_ray(origin, dir, InstanceId(99)).expect("hits the ground under the cube");
        assert!(hit.y.abs() < 0.05, "lands on the terrain (y ~ 0), not the cube top at 3.5: {hit:?}");
        assert!((hit.x - 10.0).abs() < 0.2 && (hit.z - 10.0).abs() < 0.3, "under the cube's column: {hit:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn instance_aabb_unions_the_instance_shapes_at_the_live_scale() {
        // The bounds the surface-snap rest reads: a unit cube scaled 2x (a 2m box) spans [-1, 1] in
        // each axis, so its bottom sits 1m below the centered origin. A missing instance has no bounds.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");
        let scaled = Transform { scale: Vec3::splat(2.0), ..Transform::IDENTITY };
        let scene = RenderScene::build(&root, "village", &[one_block(scaled)]);

        let aabb = scene.instance_aabb(InstanceId(0)).expect("the block resolves to shapes");
        assert!((aabb.min.y + 1.0).abs() < 1e-4, "the 2m box's bottom is 1m below the origin: {}", aabb.min.y);
        assert!((aabb.max.y - 1.0).abs() < 1e-4, "and its top 1m above: {}", aabb.max.y);
        assert!(scene.instance_aabb(InstanceId(99)).is_none(), "an absent instance has no bounds");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn instance_aabb_reflects_rotation_in_the_vertical_extent() {
        // A unit cube tilted 45 degrees about Z points a diagonal down, so its world AABB's vertical
        // half-extent grows to sqrt(2)/2 ~ 0.707: the surface-snap rest lifts a tilted item by its
        // rotated extent, not its unrotated half-height.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");
        let tilted = Transform {
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
            ..Transform::IDENTITY
        };
        let scene = RenderScene::build(&root, "village", &[one_block(tilted)]);

        let aabb = scene.instance_aabb(InstanceId(0)).expect("resolves");
        assert!((aabb.min.y + std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "tilted bottom: {}", aabb.min.y);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn surface_ray_terrain_survives_an_edit_rederive() {
        // A re-derive rebuilds the store with no heightmap; the cached heightmaps must keep the
        // surface-snap move sampling terrain, or the first edit would break ground snapping.
        let root = unique_temp_root();
        let _ = std::fs::remove_dir_all(&root);
        seed_content(&root, "village");
        save_flat_heightmap(&root, "village", ChunkCoord::new(0, 0), 0.0);

        let mut scene = RenderScene::build(&root, "village", &[one_block(Transform::IDENTITY)]);
        // Any edit re-derives the store, rebuilding it with no heightmap. Re-derive from nothing so the
        // store is empty, proving the cached heightmaps (not the store) answer the terrain query.
        scene.rederive(&[]);

        let hit =
            scene.surface_ray(Vec3::new(50.0, 10.0, 50.0), Vec3::NEG_Y, InstanceId(0)).expect("terrain after re-derive");
        assert!((hit - Vec3::new(50.0, 0.0, 50.0)).length() < 0.05, "still snaps to the ground: {hit:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ray_heightfield_finds_the_crossing_on_flat_ground() {
        // A ray dropping from y = 10 onto flat terrain at y = 2 crosses at t = 8 (dir is -Y, unit).
        let t = ray_heightfield(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0, |_, _| Some(2.0)).expect("crosses");
        assert!((t - 8.0).abs() < 0.05, "t = {t}");
    }

    #[test]
    fn ray_heightfield_misses_when_the_ray_rises_or_the_space_is_terrain_less() {
        // Pointing up from above flat terrain never dips to it.
        assert!(ray_heightfield(Vec3::new(0.0, 5.0, 0.0), Vec3::Y, 100.0, |_, _| Some(0.0)).is_none());
        // The sampler reports no terrain anywhere (off the loaded chunks), so there is no crossing.
        assert!(ray_heightfield(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0, |_, _| None).is_none());
    }
}
