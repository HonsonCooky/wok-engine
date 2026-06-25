//! wok-scene: authored scene and prefab data.
//!
//! This crate is the bedrock data layer of the engine: pure authored types and their JSON
//! serialization. No runtime logic, no GPU, no scheduling. Higher layers (`wok-content`,
//! `wok-mesh`, `wok-render`, the editor) consume these types and transform them into the
//! runtime arrays and per-frame state described in `designs/high-level-design.md`.
//!
//! Part 1: math primitives, reference newtypes, instance identity, the authored types for
//! prefabs and chunked scenes, and per-file JSON load/save.
//! Part 2: surface tags, the per-chunk terrain `Heightmap` with its binary load/save and pure
//! sampling functions (`height_at`, `normal_at`, `surface_at`).
//! Part 3 (this revision): chunk slicing into per-system runtime arrays (`slice_chunk`), and the
//! `Watcher` file watcher for hot reload.
//!
//! The crate has no internal asset registry: references are bare names, resolved by scanning
//! the content folder. `layout::ContentLayout` is the engine-owned definition of that folder
//! convention - path resolution plus a tolerant discovery scan. Identity is the per-`Scene`
//! monotonic `InstanceId`, allocated through `Scene::allocate_instance_id` and stamped on every
//! `Placement`.

pub mod chunk;
pub mod error;
pub mod heightmap;
pub mod heightmap_io;
pub mod io;
pub mod layout;
pub mod math;
pub mod prefab;
pub mod refs;
pub mod scene;
pub mod slice;
pub mod watch;

pub use chunk::{Chunk, ChunkCoord, ChunkStreaming, Eagerness, Placement};
pub use error::{LoadError, SaveError};
pub use heightmap::{
    CHUNK_GRID_DIM, CHUNK_GRID_LEN, CHUNK_SIZE_M, HEIGHT_MAX_M, HEIGHT_MIN_M, Heightmap,
    HeightmapError,
};
pub use heightmap_io::{load_heightmap, save_heightmap};
pub use io::{load_chunk, load_prefab, load_scene, save_chunk, save_prefab, save_scene};
pub use layout::ContentLayout;
pub use math::{Aabb, Mat4, Transform};
pub use prefab::{Prefab, PrefabState, Primitive, Shape, UNIT_HALF_EXTENT};
pub use refs::{InstanceId, LightStateRef, MeshRef, PrefabRef, SurfaceTag};
pub use scene::{Region, Scene, StreamingDefaults};
pub use slice::{Hitbox, SliceError, SlicedChunk, Trigger, VisibleItem, slice_chunk};
pub use watch::{WatchError, Watcher};
