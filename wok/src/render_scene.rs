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
    Aabb, CHUNK_GRID_DIM, Chunk, ChunkCoord, ContentLayout, Heightmap, Prefab, PrefabRef, Primitive,
    VisibleItem,
};

use crate::camera::FlyCamera;
use crate::loaded::LoadedScene;
use crate::render::Gpu;

/// Chunk side in metres, derived from the heightmap grid (128 one-metre cells; the 129th sample is
/// the shared edge). wok-scene deliberately does not bake the chunk size into ChunkCoord, so this
/// composition is application policy (the same constant taste derives).
pub const CHUNK_SIZE_M: f32 = (CHUNK_GRID_DIM - 1) as f32;

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
    /// The runtime arrays the viewport draws: visible items (re-derived on edit) and, at build time,
    /// the terrain mesh per chunk (consumed once to upload the GPU terrain, then dropped on a
    /// re-derive). The renderer reads `visible` from here and the terrain from the GPU cache.
    pub store: ChunkStore,
    /// The world-space bounds of the terrain, cached at build (terrain does not change without a
    /// scene reload, and a re-derive drops the store's terrain meshes), unioned with the live visible
    /// items in [`scene_bounds`](Self::scene_bounds). `None` for a scene with no terrain.
    terrain_bounds: Option<Aabb>,
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
        let light = load_light(&layout, name);

        let mut store = ChunkStore::new();
        for chunk in chunks {
            let heightmap = heightmap_for(&layout, name, chunk.coord);
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
            store,
            terrain_bounds,
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

    /// Fog distance sets render distance (HLD); the far plane sits past full occlusion.
    pub fn far_plane(&self) -> f32 {
        (self.light.fog.end * 1.2).max(50.0)
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
                    grow(primitive_world_aabb(*primitive, origin * *transform));
                }
            }
        }
        if min.x > max.x {
            return Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        }
        Aabb::new(min, max)
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

/// Load the scene's default light state: read the manifest for its `default_lighting` name, then that
/// state under `assets/lighting/`. A missing or malformed manifest or state falls back to a neutral
/// daytime default, so the viewport always has lighting and never fails to open over a content gap.
fn load_light(layout: &ContentLayout, scene: &str) -> LightState {
    let manifest = match wok_scene::load_scene(layout.scene_json(scene)) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("wok: scene manifest did not load for lighting, using default: {err}");
            return LightState::default();
        }
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

/// The world-space AABB of a unit primitive under `world`: the eight corners of its unit box
/// transformed and min/maxed. A conservative cover for the shadow region (not a tight fit), so an
/// oriented or scaled placement still reports a box that contains it.
fn primitive_world_aabb(primitive: Primitive, world: Mat4) -> Aabb {
    let local = primitive.unit_aabb();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for &x in &[local.min.x, local.max.x] {
        for &y in &[local.min.y, local.max.y] {
            for &z in &[local.min.z, local.max.z] {
                let corner = world.transform_point3(Vec3::new(x, y, z));
                min = min.min(corner);
                max = max.max(corner);
            }
        }
    }
    Aabb::new(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    use wok_scene::{
        Chunk, ChunkCoord, ChunkStreaming, Eagerness, InstanceId, LightStateRef, Placement, Prefab,
        PrefabState, Scene, Shape, StreamingDefaults, SurfaceTag, Transform, save_prefab, save_scene,
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
        assert!(scene.far_plane() >= 50.0);
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
}
