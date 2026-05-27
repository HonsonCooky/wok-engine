# `wok-scene` — Detailed Crate Plan

The bedrock data-model crate. Defines every authored type that lives
on disk as JSON, the runtime-array types those get sliced into, the
slicing function itself, save/load, and a file watcher for hot reload.
No GPU, no simulation, no logic. Pure data and pure transformations
between two shapes of data.

Depends only on `pantry` (which re-exports `glam`, `serde`,
`serde_json`) plus `notify` and `notify-debouncer-full` as direct
dependencies. No other `wok-*` crate.

This document covers wok-scene through v0.2.0, which adds terrain
support. v0.1.0 surface (everything except terrain) is shipped at
the `v0.1.0-wok-scene-baseline` git tag. v0.2.0 surface adds the
terrain authored type, runtime type, sampling functions, sibling
binary file format, and associated tests.

---

## 1. Design Rules

### Pure transformations only

`wok-scene` owns pure transformations on authored data. Downstream
crates orchestrate.

The layer that defines a data type also owns the pure functions over
it. Crates that need side effects (GPU, threads, registries, the
network) are orchestrators that call those pure functions and layer
the side effects on top.

Practical consequence here: `wok-scene::slice_chunk` is a pure function
from authored chunks to runtime arrays.
`wok-content::transform_chunk` calls it, then resolves asset references
through its registry, coordinates GPU uploads, and manages the worker
thread.

### One-way data flow, no exceptions

Authored data is the source of truth. Runtime arrays are derived
data. The arrows go authored→disk and authored→runtime only.

- No runtime→authored path. Game state never writes back into prefab
  or scene files.
- No per-chunk snapshots. A chunk that unloads and reloads comes back
  in its authored state, period.
- Snapshots exist at the whole-world level (save/load and multiplayer
  join-in-progress) and are `wok-content`'s concern.

Games that want persistent world state — dropped items, solved
puzzles, partially-cleared enemy camps — own that problem game-side.
The canonical pattern is a sidecar file the game maintains, queried
during chunk load to override authored placements and to gate
spawner-trigger firing. This works cleanly with the existing
architecture because actors live in `wok-physics`'s pool, not in
`ChunkRuntime` — they persist across chunk unload/reload until
explicitly killed, so per-NPC state is naturally separable from
chunk geometry.

This crate has no API for snapshotting, restoring, or persisting
runtime state.

---

## 2. Crate Layout

```
wok-scene/
├── Cargo.toml
└── src/
    ├── lib.rs              # Re-exports
    ├── error.rs            # LoadError, SaveError, SliceError
    ├── ids/
    │   ├── mod.rs
    │   ├── slug.rs         # Slug, validation
    │   ├── content.rs      # PrefabId, SceneId, TriggerId
    │   ├── assets.rs       # MeshId, AudioCueId, AnimationId, VoiceLineId, LightStateRef
    │   └── chunk.rs        # ChunkCoord
    ├── authored/
    │   ├── mod.rs
    │   ├── shape.rs        # ShapePrimitive, Shape
    │   ├── prefab.rs       # Prefab, PrefabState
    │   ├── chunk.rs        # Chunk, PrefabPlacement, RegionMarker, RegionPurpose
    │   ├── scene.rs        # Scene (manifest)
    │   ├── streaming.rs    # ChunkMetadata, ChunkEagerness
    │   └── terrain.rs      # TerrainData (v0.2.0)
    ├── runtime/
    │   ├── mod.rs
    │   ├── shape.rs        # VisibleShape, PhysicalHitbox, TriggerVolume
    │   ├── region.rs       # RuntimeRegion
    │   ├── chunk.rs        # ChunkRuntime
    │   └── terrain.rs      # RuntimeTerrain (v0.2.0)
    ├── slice.rs            # slice_chunk
    ├── sampling.rs         # height_at, normal_at, surface_at (v0.2.0)
    ├── serde_format.rs     # Format version, custom (de)serializers
    ├── load.rs             # load_prefab, load_scene_manifest, load_chunk
    ├── save.rs             # save_prefab, save_scene_manifest, save_chunk
    └── watcher.rs          # FileWatcher, FileEvent
```

Target < 400 lines per file.

---

## 3. Public API Surface

### Slug

```rust
// Validated path-safe identifier. Lowercase alphanumeric + '_' + '-' + '/'.
// Cloned cheaply via Arc.
pub struct Slug(Arc<str>);

impl Slug {
    pub fn new(s: &str) -> Result<Self, InvalidSlug>;
    pub fn as_str(&self) -> &str;
}
```

Conventions: hyphens between words (`wooden-crate`), forward slash for
namespacing (`enemies/grunt`). Numbers allowed anywhere including the
end (`version-2`).

### Content identifiers

```rust
pub struct PrefabId(Slug);
pub struct SceneId(Slug);
pub struct TriggerId(String);       // game-defined, not slug-validated
pub struct ChunkCoord { pub x: i32, pub z: i32 }

impl ChunkCoord {
    /// Chunk extent along each horizontal axis, in meters.
    /// Chunks are square; world coordinates within a chunk lie in
    /// `[0, CHUNK_SIZE_METERS]` for both x and z.
    pub const CHUNK_SIZE_METERS: f32 = 128.0;

    pub fn to_world_offset(self) -> Vec3 {
        Vec3::new(
            self.x as f32 * Self::CHUNK_SIZE_METERS,
            0.0,
            self.z as f32 * Self::CHUNK_SIZE_METERS,
        )
    }
}
```

### Asset identifiers

Per-kind nominal types, all with the same `{ serial, slug }` shape:

```rust
pub struct MeshId         { serial: u32, slug: Slug }
pub struct AudioCueId     { serial: u32, slug: Slug }
pub struct AnimationId    { serial: u32, slug: Slug }
pub struct VoiceLineId    { serial: u32, slug: Slug }
pub struct LightStateRef  { serial: u32, slug: Slug }

// Implementation note: define these via a small declarative macro
// (`define_asset_id!`) in ids/assets.rs to keep the boilerplate honest.

impl MeshId {
    pub fn new(slug: Slug, serial: u32) -> Self;
    pub fn serial(&self) -> u32;
    pub fn slug(&self) -> &Slug;
}

// PartialEq, Eq, Hash, PartialOrd, Ord — implemented manually,
// based on `serial` only. Slug is debug surface, NOT identity.
//
// Two MeshIds with serial=42 are equal even if their slugs differ.
// This means the same registry entry retrieved twice over a rename
// still compares equal.

impl Debug for MeshId {
    // prints as "MeshId(wooden-crate-42)"
}
impl Display for MeshId {
    // prints as "wooden-crate-42"
}
```

Serial allocation lives in `wok-content`'s registry; `wok-scene` does
not enforce that constructed IDs correspond to registered assets. The
constructor is public so tests and the editor can build IDs; the
"every ID points to something" invariant is `wok-content`'s
responsibility.

Per-kind serial counters: `mesh-1` and `audio-1` are different things,
distinguished by the Rust type, not by the serial.

### Authored types

```rust
pub enum ShapePrimitive {
    Cube      { half_extents: Vec3 },
    Ellipsoid { radii: Vec3 },
    Cylinder  { radius: f32, half_height: f32 },
    Capsule   { radius: f32, half_height: f32 },
    Plane     { half_extents: Vec2 },
}

pub struct Shape {
    pub primitive: ShapePrimitive,
    pub transform: Transform,           // prefab-local
    pub is_hitbox: bool,
    pub is_visible: bool,
    pub trigger_id: Option<TriggerId>,  // required iff hitbox && !visible
    pub surface_tag: Option<String>,
    pub visual_color: Option<[f32; 3]>,
}

pub struct PrefabState {
    pub name: String,
    pub shapes: Vec<Shape>,
    pub mesh_override: Option<MeshId>,
    pub audio_cues: BTreeMap<String, AudioCueId>,
}

pub struct Prefab {
    pub id: PrefabId,
    pub states: Vec<PrefabState>,
    pub default_state: String,
}

pub struct PrefabPlacement {
    pub prefab: PrefabId,
    pub transform: Transform,           // chunk-local
    pub state: String,
    pub instance_tag: Option<String>,
}

// Environmental zone within a chunk.
// NOT a game-event trigger — prefab trigger volumes handle game events.
pub enum RegionPurpose {
    Fog      { color: [f32; 3], density: f32, distance: f32 },
    Lighting { state: LightStateRef },
    Ambient  { color: [f32; 3] },
}

pub struct RegionMarker {
    pub name: String,
    pub bounds: Aabb,                   // chunk-local; clipped to chunk extents
    pub purpose: RegionPurpose,
}

pub enum ChunkEagerness {
    Eager,    // loads when in radius
    Lazy,     // loads only on explicit request
    Vista,    // loaded but partially active (see §8)
}

pub struct ChunkMetadata {
    pub eagerness: ChunkEagerness,
    pub neighbors: Vec<ChunkCoord>,
    pub interlocks: Vec<ChunkCoord>,
}

// v0.2.0: Per-chunk terrain heightmap. Stored as a sibling binary
// file on disk (see §4) and referenced from the chunk JSON. Optional
// — not every chunk has terrain. Indoor chunks, scripted scenes, and
// prefab-only environments may omit it entirely.
//
// Authored resolution is fixed at 1m × 1m per cell. With chunks at
// 128m × 128m, that's 129 × 129 cells (shared-edge convention; see
// 'Shared-edge convention' gotcha in §9).
//
// Heights are u16-quantized to millimeter precision over a fixed
// vertical range of ±32m. Value 0 represents -32m; value 65535
// represents +32m. The quantization step is 64m / 65535 ≈ 0.977mm —
// submillimeter for all practical purposes.
//
// Surface indices are u16 references into a per-chunk authored
// surface table; the slicer merges this table with the chunk's
// prefab surface tags into ChunkRuntime.surface_tag_table.
pub struct TerrainData {
    /// Relative path to the sibling binary file holding the
    /// heightmap and surface-index arrays (e.g., "0_0.heightmap.bin").
    /// Resolved against the chunk file's directory at load time.
    /// Custom serde emits only this field in the JSON object;
    /// `heights`, `surface_indices`, `surface_tags`, and
    /// `vertical_range_meters` come from the sibling binary.
    /// Rejected at load: absolute paths and paths containing
    /// directory components (`..`, `/`). Terrain references must
    /// be local to the chunk's scene directory.
    pub heightmap_file: PathBuf,

    /// Heightmap: 129 * 129 = 16641 entries per the shared-edge
    /// convention. Indexing convention: heights[z * 129 + x].
    /// Origin at the chunk's local (0, 0); +x and +z run along the
    /// chunk's primary axes.
    pub heights: Box<[u16]>,

    /// Surface tag indices, one per cell, same length as `heights`
    /// (16641 entries). Indices reference the authored
    /// `surface_tags` table.
    pub surface_indices: Box<[u16]>,

    /// Per-chunk surface tag table for authored terrain surfaces.
    /// Strings are sorted alphabetically at save time. Indices in
    /// surface_indices reference positions in this table.
    pub surface_tags: Vec<String>,

    /// Vertical range in meters. Typically 32.0 for "±32m around
    /// authored zero." Stored explicitly so future authored data
    /// with different ranges can be loaded without ambiguity.
    /// Editor should default to 32.0 and warn on values outside
    /// the [4.0, 256.0] range as likely authoring errors.
    pub vertical_range_meters: f32,
}

pub struct Chunk {
    pub coord: ChunkCoord,
    pub metadata: ChunkMetadata,
    pub placements: Vec<PrefabPlacement>,
    pub regions: Vec<RegionMarker>,
    pub light_state: LightStateRef,

    // v0.2.0: Authored terrain heightmap. None when the chunk has
    // no terrain (indoor, scripted, prefab-only environments).
    // Serialized as a JSON reference to a sibling binary file;
    // see §4 'Sibling binary file format'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<TerrainData>,
}

// Scene is the manifest. Chunk data lives in per-coord files
// loaded separately.
pub struct Scene {
    pub id: SceneId,
    pub default_load_radius_meters: f32,
    pub default_eagerness: ChunkEagerness,
    pub default_light_state: LightStateRef,
    pub chunks: Vec<ChunkCoord>,        // existence only
}
```

The `#[serde(skip_serializing_if = "Option::is_none")]` on `Chunk.terrain`
ensures byte-identical save round-trip for chunks authored before
v0.2.0: a chunk loaded with `terrain: None` re-saves without the
field, preserving `v0.1.0-wok-scene-baseline` file bytes.

`PrefabState.audio_cues` is `BTreeMap<String, AudioCueId>` rather than
`Vec<(String, AudioCueId)>` — JSON keyed-object representation with
deterministic alphabetical ordering on save. Spec deviation from the
original v0.1.0 plan made during implementation; canonicalized here.

### Runtime arrays

All transforms in runtime arrays are **chunk-local**. To get world
coords, compose with `chunk.coord.to_world_offset()`.

```rust
pub struct VisibleShape {
    pub primitive: ShapePrimitive,
    pub local_transform: Mat4,
    pub color: [f32; 3],
    pub source_placement: u32,
}

pub struct PhysicalHitbox {
    pub primitive: ShapePrimitive,
    pub local_transform: Mat4,
    pub surface_tag: u32,               // index into runtime's surface_tag_table
    pub source_placement: u32,
}

pub struct TriggerVolume {
    pub primitive: ShapePrimitive,
    pub local_transform: Mat4,
    pub trigger_id: TriggerId,
    pub source_placement: u32,
}

pub struct RuntimeRegion {
    pub name: String,
    pub local_bounds: Aabb,
    pub purpose: RegionPurpose,
}

// v0.2.0: Runtime terrain attached to a ChunkRuntime. Produced by
// slice_chunk when authored TerrainData is present. Position-
// independent: the runtime data is byte-identical regardless of the
// chunk's ChunkCoord. Consumers add ChunkCoord::to_world_offset() at
// query time when they need world coordinates.
pub struct RuntimeTerrain {
    /// Heightmap, copied unchanged from TerrainData.heights.
    /// 16641 entries (129 × 129 per the shared-edge convention).
    pub heights: Box<[u16]>,

    /// Surface tag indices, referencing the merged
    /// ChunkRuntime.surface_tag_table (not the authored
    /// TerrainData.surface_tags). The slicer rewrites indices
    /// during merge — see §5.
    pub surface_indices: Box<[u16]>,

    /// Cell count along each axis. Always 129 under the locked
    /// shared-edge convention. Stored for sampler bounds checks
    /// and to support potential future authoring resolutions.
    pub width: u32,

    /// Vertical range, copied from TerrainData. Used by the
    /// sampling functions to dequantize heights.
    pub vertical_range_meters: f32,
}

pub struct ChunkRuntime {
    pub coord: ChunkCoord,
    pub eagerness: ChunkEagerness,      // carried through; downstream consults it
    pub visible: Vec<VisibleShape>,
    pub hitboxes: Vec<PhysicalHitbox>,
    pub triggers: Vec<TriggerVolume>,
    pub regions: Vec<RuntimeRegion>,
    pub light_state: LightStateRef,
    pub surface_tag_table: Vec<String>,

    // v0.2.0: Runtime terrain, present when the source chunk had
    // TerrainData. None means the chunk has no terrain. Consistent
    // with the other runtime arrays as a public field.
    pub terrain: Option<RuntimeTerrain>,
}
```

`eagerness` is carried into `ChunkRuntime` so downstream crates can
consult it cheaply without round-tripping back to authored data. The
slicer treats all three eagerness values identically; the tag is
metadata only at this layer.

### Top-level functions

```rust
// Prefabs
pub fn load_prefab(path: &Path) -> Result<Prefab, LoadError>;
pub fn save_prefab(prefab: &Prefab, path: &Path) -> Result<(), SaveError>;
pub fn load_prefab_dir(dir: &Path) -> Result<HashMap<PrefabId, Prefab>, LoadError>;

// Scenes — manifest and per-chunk
pub fn load_scene_manifest(scene_dir: &Path) -> Result<Scene, LoadError>;
pub fn save_scene_manifest(scene: &Scene, scene_dir: &Path) -> Result<(), SaveError>;
pub fn load_chunk(scene_dir: &Path, coord: ChunkCoord) -> Result<Chunk, LoadError>;
pub fn save_chunk(scene_dir: &Path, chunk: &Chunk) -> Result<(), SaveError>;

// Slicing (the keystone)
pub fn slice_chunk(
    chunk: &Chunk,
    prefabs: &dyn PrefabLookup,
) -> Result<ChunkRuntime, SliceError>;

pub trait PrefabLookup {
    fn get(&self, id: &PrefabId) -> Option<&Prefab>;
}

// Blanket impl for HashMap-stored prefabs. Generic over the hasher
// so consumers using ahash/fxhash don't need to redefine the trait.
impl<S: BuildHasher> PrefabLookup for HashMap<PrefabId, Prefab, S> { ... }
```

`load_chunk` reads two files when the chunk's JSON references a
sibling heightmap: the chunk JSON first, then the binary if
`chunk.terrain` is `Some(TerrainRef)`. Missing sibling produces
`LoadError::TerrainSiblingMissing`. See §4 for the sibling format.

`save_chunk` writes both files when terrain is present (binary
first, then JSON; see §9 'Sibling-binary save ordering' for the
ordering rationale and failure modes).

### Sampling functions (v0.2.0)

Three pure functions for sampling terrain from a chunk. All three
take `&ChunkRuntime` as their first argument — uniform call-site
shape, even though `height_at` and `normal_at` strictly need only
the terrain field. The asymmetry-vs-uniformity trade-off is
documented in §9 'Sampling signature uniformity'.

```rust
/// Sample interpolated height at a chunk-local position.
///
/// Coordinates are in chunk-local meters: x and z each in the
/// range [0, 128]. Returns None if the chunk has no terrain
/// (chunk.terrain is None) or if (x, z) is outside the valid
/// domain.
///
/// Bilinear interpolation between the four enclosing cells.
/// Heights are dequantized from u16 to meters before
/// interpolation, using the terrain's vertical_range_meters.
/// Returns chunk-local height in meters.
pub fn height_at(
    chunk: &ChunkRuntime,
    chunk_local_x: f32,
    chunk_local_z: f32,
) -> Option<f32>;

/// Sample interpolated surface normal at a chunk-local position.
///
/// Same domain and return semantics as height_at. The returned
/// Vec3 is unit-length and in chunk-local space (consumer
/// composes with chunk transform for world space).
///
/// Normal is computed from a 1-cell gradient between immediate
/// neighbor cells. See §9 'Normal computation method' for the
/// gradient-method decision and the upgrade trigger.
pub fn normal_at(
    chunk: &ChunkRuntime,
    chunk_local_x: f32,
    chunk_local_z: f32,
) -> Option<Vec3>;

/// Look up the surface tag at a chunk-local position.
///
/// Returns a borrowed string from chunk.surface_tag_table,
/// referenced by the cell containing (x, z). Lifetime tied to
/// the borrowed ChunkRuntime via elision.
///
/// Returns None if the chunk has no terrain, if (x, z) is outside
/// the valid domain, or if the cell's surface index doesn't
/// resolve in the table (which would indicate a slicer bug —
/// surfaced rather than panicked).
pub fn surface_at(
    chunk: &ChunkRuntime,
    chunk_local_x: f32,
    chunk_local_z: f32,
) -> Option<&str>;
```

The terrain-absent case (`chunk.terrain` is None) is handled
identically across all three: early return via the `?` operator on
`chunk.terrain.as_ref()`. Callers don't need to pre-check
`terrain.is_some()` before sampling.

### Errors

The crate defines three error types, one per failure domain.

```rust
pub enum LoadError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    UnsupportedVersion { found: u32 },
    MissingFormat { path: PathBuf },
    InvalidSlug { token: String, reason: String },

    // v0.2.0: A chunk's `terrain.heightmap_file` references a sibling
    // binary file that doesn't exist on disk.
    TerrainSiblingMissing {
        chunk_path: PathBuf,
        terrain_path: PathBuf,
    },

    // v0.2.0: A chunk's sibling heightmap file is malformed: wrong
    // magic bytes, unsupported format version, length mismatch with
    // declared resolution, or surface_indices referencing an
    // out-of-range entry.
    TerrainMalformed {
        terrain_path: PathBuf,
        reason: String,
    },
}

pub enum SaveError {
    Io(std::io::Error),
    Encode(serde_json::Error),
}

pub enum SliceError {
    UnknownPrefab(PrefabId),
    UnknownState { prefab: PrefabId, state: String },
    InvalidShape { placement_index: usize, shape_index: usize, reason: String },

    // v0.2.0: Merging a chunk's authored terrain surface tags with
    // its prefab surface tags produced a surface_tag_table larger
    // than u16::MAX. This should be unreachable for any realistic
    // chunk; surfaced rather than panicked so authoring tools can
    // report it.
    TerrainSurfaceTableOverflow {
        coord: ChunkCoord,
        prefab_tag_count: usize,
        terrain_tag_count: usize,
    },
}
```

All implement `std::error::Error` and `Display`. Per project-canon,
the crate uses `thiserror` for the derive boilerplate.

### File watcher

```rust
pub struct FileWatcher { /* opaque */ }

pub enum FileEvent {
    PrefabChanged(PathBuf),
    PrefabRemoved(PathBuf),
    SceneManifestChanged(PathBuf),
    ChunkChanged { scene_dir: PathBuf, coord: ChunkCoord },
    ChunkRemoved { scene_dir: PathBuf, coord: ChunkCoord },
    LightStateChanged(PathBuf),
    Error(String),
}

impl FileWatcher {
    pub fn new(content_root: &Path) -> Result<Self, std::io::Error>;
    pub fn poll(&mut self) -> Vec<FileEvent>;
}
```

Heightmap file changes coalesce with chunk changes via
`FileEvent::ChunkChanged` — no separate variant. See §6 for the
classification rules.

---

## 4. JSON Serialization Format

### Conventions

- `_format` field at top level, integer, currently `1`. Loader refuses
  unknown versions.
- `Transform` as `{ "pos": [x,y,z], "rot": [x,y,z,w], "scale": [x,y,z] }`,
  identity defaults skipped on serialization.
- Enums tagged `serde(tag = "kind")` for readable JSON.
- Asset IDs serialize as `slug-serial` strings (e.g.,
  `"wooden-crate-267"`).
- Per-chunk files. Scene file is a manifest.
- JSON output is pretty-printed (indented, multi-line).
  `serde_json::to_string_pretty` is used; byte-identical round-trip
  preserves indentation.

### Asset reference parsing rule

The reference token `slug-serial` is parsed by splitting on the **last
hyphen** and parsing the suffix as `u32`. The slug part may itself
contain hyphens; this is unambiguous.

Examples:

- `wooden-crate-267` → slug=`wooden-crate`, serial=267
- `version-2-5` → slug=`version-2`, serial=5
- `foo-0` → slug=`foo`, serial=0
- `foo` → parse error (no separator)
- `foo-bar` → parse error (suffix `bar` is not a u32)

After splitting, the slug part is validated against the `Slug` rules.
A token like `WOODEN-CRATE-267` parses to slug=`WOODEN-CRATE` then
fails slug validation on uppercase. Error messages include both the
original token and the failure reason.

### Directory layout

```
wok-engine/tests-integration/fixtures/
├── prefabs/
│   ├── crate-wooden.json
│   └── door-simple.json
├── scenes/
│   └── act1-warehouse/
│       ├── scene.json              # manifest
│       ├── 0_0.json                # chunk at (0,0)
│       ├── 0_0.heightmap.bin       # v0.2.0: sibling heightmap
│       ├── 1_0.json
│       └── 0_1.json
└── lights/
    └── warehouse-day.json
```

Chunk file names use `_` not `-` as the coord separator
(`{i}_{j}.json`) to avoid confusion with asset ID parsing. Filenames
that don't parse as `{i}_{j}.json` with `i, j` as `i32` surface as
`FileEvent::Error` rather than silent drop. The same applies to
heightmap files: `{i}_{j}.heightmap.bin` with matching coord parse.

### Prefab file example

```json
{
  "_format": 1,
  "id": "crate-wooden",
  "default_state": "default",
  "states": [
    {
      "name": "default",
      "shapes": [
        {
          "primitive": { "kind": "cube", "half_extents": [0.5, 0.5, 0.5] },
          "transform": { "pos": [0, 0.5, 0] },
          "is_hitbox": true,
          "is_visible": true,
          "surface_tag": "wood",
          "visual_color": [0.55, 0.35, 0.2]
        }
      ],
      "mesh_override": "wooden-crate-mesh-267",
      "audio_cues": {
        "impact": "wood-impact-12",
        "footstep": "wood-step-7"
      }
    },
    {
      "name": "destroyed",
      "shapes": [
        {
          "primitive": { "kind": "cube", "half_extents": [0.5, 0.1, 0.5] },
          "transform": { "pos": [0, 0.1, 0] },
          "is_hitbox": true,
          "is_visible": true,
          "surface_tag": "wood-debris",
          "visual_color": [0.4, 0.25, 0.15]
        }
      ]
    }
  ]
}
```

### Scene manifest example

```json
{
  "_format": 1,
  "id": "act1-warehouse",
  "default_load_radius_meters": 200.0,
  "default_eagerness": "eager",
  "default_light_state": "warehouse-day-3",
  "chunks": [[0, 0], [1, 0], [0, 1]]
}
```

### Chunk file example

```json
{
  "_format": 1,
  "coord": [0, 0],
  "metadata": {
    "eagerness": "eager",
    "neighbors": [[1, 0], [0, 1]],
    "interlocks": []
  },
  "light_state": "warehouse-day-3",
  "placements": [
    {
      "prefab": "crate-wooden",
      "transform": { "pos": [12.0, 0.0, 8.5] },
      "state": "default"
    }
  ],
  "regions": [
    {
      "name": "office-interior",
      "bounds": { "min": [30, 0, 30], "max": [40, 5, 40] },
      "purpose": { "kind": "lighting", "state": "warehouse-office-7" }
    }
  ],
  "terrain": {
    "heightmap_file": "0_0.heightmap.bin"
  }
}
```

The `terrain` field is `Option<TerrainRef>`; absent means the chunk
has no terrain. When present, `heightmap_file` is a relative path
resolved against the chunk file's directory. Absolute paths are
rejected at load (`LoadError::TerrainMalformed`).

### Sibling binary file format (v0.2.0)

The terrain heightmap lives in a sibling file next to each chunk's
JSON, named to mirror the chunk: `0_0.json` has companion
`0_0.heightmap.bin`. The chunk JSON references it by relative name
through the `terrain.heightmap_file` field.

All multi-byte values are little-endian.

```
Offset  Size  Field                  Notes
------  ----  ----------------------  ----------------------------
0       4     magic                  ASCII "WTRN" (Wok Terrain)
4       2     format_version (u16)   Currently 1.
6       2     width (u16)            Cells along the x axis.
                                     Always 129 under shared-edge.
8       2     height_along_z (u16)   Cells along the z axis.
                                     Equals width (always 129).
10      2     surface_tag_count (u16) Number of authored surface
                                     tag strings that follow.
12      4     vertical_range_mm (u32) Vertical range in millimeters.
                                     Typically 32000 (= 32.0m).
16      ...   surface_tags          surface_tag_count entries:
                                     u16 length-prefixed UTF-8
                                     strings, sorted alphabetically.
...     ...   heights[]              width * height_along_z u16
                                     values, little-endian.
                                     Row-major: z varies slowest.
...     ...   surface_indices[]      Same count as heights,
                                     u16 each.
```

For the locked 129 × 129 resolution: 16641 heights + 16641 surface
indices = 66564 bytes of cell data per chunk, plus the small header
and surface tag strings. Typical chunk sibling binary is ~65-70KB.

The header explicitly carries `vertical_range_mm` to support future
authored data with non-standard ranges. The current authoring
default is 32000 (= 32.0m), but a chunk authored with a different
range loads correctly regardless.

#### Determinism

Surface tags are sorted alphabetically before write, matching the
project-canon determinism contract for HashMap-derived data. Heights
and surface indices are stored in their native row-major order — no
sort needed. Two saves of the same `TerrainData` produce
byte-identical sibling files.

#### Save ordering

`save_chunk` writes both files via plain `std::fs::write` (same
style as v0.1.0 prefab and chunk saves; no temp-file-and-rename).
The binary is written first, then the JSON. Order matters: write
binary first so a successful JSON write always references a binary
that exists on disk.

See §9 'Sibling-binary save ordering' for failure-mode details and
why atomicity isn't retrofitted here.

### Round-trip determinism requirement

`load(save(load(x))) == load(x)`, byte-identical. Anything serialized
from a `HashMap` is sorted on save. Tested explicitly (see §7).

For chunks with terrain, byte-identity applies to both the JSON file
and the sibling binary file independently. The pair round-trips as a
unit.

---

## 5. The Shape-Slicing Algorithm

### Inputs

- `chunk: &Chunk`
- `prefabs: &dyn PrefabLookup`

### Output

`Result<ChunkRuntime, SliceError>` — all transforms in chunk-local
space.

### Errors

- `SliceError::UnknownPrefab(PrefabId)`
- `SliceError::UnknownState { prefab: PrefabId, state: String }`
- `SliceError::InvalidShape { placement_index, shape_index, reason }`
- `SliceError::TerrainSurfaceTableOverflow { ... }` (v0.2.0)

Fail-fast: one error aborts the whole chunk.

### Pseudocode

```
fn slice_chunk(chunk, prefabs) -> ChunkRuntime:
    let mut visible = Vec::new()
    let mut hitboxes = Vec::new()
    let mut triggers = Vec::new()
    let mut surface_intern = StringInterner::new()

    for (placement_idx, placement) in chunk.placements.enumerate():
        let prefab = prefabs.get(&placement.prefab)
            .ok_or(UnknownPrefab(placement.prefab.clone()))?

        let state = prefab.states.iter()
            .find(|s| s.name == placement.state)
            .ok_or(UnknownState { ... })?

        // Both transforms are chunk-local; NO chunk-world offset applied.
        let placement_m = placement.transform.to_mat4()

        for (shape_idx, shape) in state.shapes.enumerate():
            validate_shape_flags(shape, placement_idx, shape_idx)?
            let local_transform = placement_m * shape.transform.to_mat4()

            match (shape.is_hitbox, shape.is_visible):
                (true, true) =>
                    visible.push(VisibleShape { local_transform, ... })
                    hitboxes.push(PhysicalHitbox {
                        local_transform,
                        surface_tag: surface_intern.intern(
                            shape.surface_tag.as_deref().unwrap_or("")
                        ),
                        ...
                    })
                (true, false) =>
                    triggers.push(TriggerVolume {
                        local_transform,
                        trigger_id: shape.trigger_id.clone().unwrap(),
                        ...
                    })
                (false, true) =>
                    visible.push(VisibleShape { local_transform, ... })
                (false, false) =>
                    return Err(InvalidShape { ... })

    let regions = chunk.regions.iter().map(|r| RuntimeRegion {
        name: r.name.clone(),
        local_bounds: r.bounds,
        purpose: r.purpose.clone(),
    }).collect()

    // v0.2.0: terrain pass. Merge terrain surface tags into the
    // surface intern table; produce RuntimeTerrain with remapped
    // surface indices.
    let runtime_terrain = match &chunk.terrain {
        None => None,
        Some(authored) => Some(slice_terrain(authored, &mut surface_intern)?),
    }

    Ok(ChunkRuntime {
        coord: chunk.coord,
        eagerness: chunk.metadata.eagerness,
        visible, hitboxes, triggers, regions,
        light_state: chunk.light_state.clone(),
        surface_tag_table: surface_intern.into_vec(),
        terrain: runtime_terrain,
    })
```

### Slicing terrain (v0.2.0)

```
fn slice_terrain(authored: &TerrainData, intern: &mut StringInterner)
    -> Result<RuntimeTerrain, SliceError>
{
    // Merge authored terrain surface tags into the runtime intern
    // table. Build a remap from authored index -> runtime index.
    let mut remap = SurfaceIndexRemap::with_capacity(authored.surface_tags.len());

    for (authored_idx, tag) in authored.surface_tags.iter().enumerate() {
        let runtime_idx = intern.intern(tag);
        if runtime_idx > u16::MAX as usize {
            return Err(TerrainSurfaceTableOverflow { ... });
        }
        remap.set(authored_idx, runtime_idx as u16);
    }

    // Rewrite the per-cell surface indices using the remap.
    let surface_indices: Box<[u16]> = authored.surface_indices.iter()
        .map(|&authored_idx| remap.get(authored_idx))
        .collect();

    Ok(RuntimeTerrain {
        heights: authored.heights.clone(),
        surface_indices,
        width: 129,
        vertical_range_meters: authored.vertical_range_meters,
    })
}
```

#### Implementation note on the intern pattern

The pseudocode shows `intern.intern(tag)` as a single call. Inside
`StringInterner`, the natural intent is:

```
fn intern(&mut self, tag: &str) -> usize {
    self.table.iter().position(|t| t == tag)
        .unwrap_or_else(|| {
            self.table.push(tag.to_string());
            self.table.len() - 1
        })
}
```

The `unwrap_or_else` closure borrows `self.table` mutably while the
`.iter().position()` chain has an outstanding immutable borrow.
Rust's borrow checker rejects this even though the immutable borrow
is logically released before the closure runs. Real implementation
uses a `match`-and-push pattern:

```
fn intern(&mut self, tag: &str) -> usize {
    match self.table.iter().position(|t| t == tag) {
        Some(i) => i,
        None => {
            self.table.push(tag.to_string());
            self.table.len() - 1
        }
    }
}
```

Intent is identical; the pseudocode keeps the `unwrap_or_else` form
for readability of intent.

### Properties

1. **Deterministic.** Same inputs → bit-identical output.
2. **Single allocation per output vector** (counting pass or
   conservative `with_capacity`). Terrain `heights` and
   `surface_indices` are exact-size single allocations from the
   authored data.
3. **Pure.** No I/O, no global state.
4. **Order-preserving.** Placement order, then shape-within-state
   order. Terrain surface tags are appended to the runtime table in
   the order they appear in `TerrainData.surface_tags` (which is
   alphabetically sorted on save).
5. **Position-independent.** Output is identical regardless of
   `chunk.coord` except for the `coord` field itself. This is the
   determinism property the parallel-worlds multiplayer model
   depends on: the same authored chunk file produces the same
   runtime arrays on every client, regardless of where in the world
   the chunk sits. **Terrain inherits this property:** `RuntimeTerrain`
   contains nothing derived from `ChunkCoord`.
6. **Eagerness-neutral.** Slicing produces identical arrays for
   Eager, Lazy, and Vista chunks. The tag is carried through to
   `ChunkRuntime.eagerness` for downstream consumers.

### Performance posture

10–500 placements per chunk, 1–20 shapes per placement. Sub-millisecond
on commodity hardware. No internal parallelism — `wok-content` can
`rayon`-parallelize across chunks if profiles ever justify it.

Terrain slicing adds a constant cost per chunk with terrain: ~16641
surface-index remapping operations and a memcpy of ~33KB of heights.
Negligible.

---

## 6. File-Watcher Behavior

### Lifecycle

- `FileWatcher::new(content_root)` spawns a debounced `notify` watcher
  rooted at `content_root` (~100ms debounce, via
  `notify-debouncer-full`).
- Background thread classifies FS events into `FileEvent`s and pushes
  to an `mpsc` queue.
- `poll()` drains the queue.
- `content_root` is canonicalized inside `FileWatcher::new`; the
  directory must exist when the watcher is created.

### Classification

Path relative to `content_root`:

- `prefabs/{slug}.json` → `PrefabChanged` / `PrefabRemoved`
- `scenes/{scene}/scene.json` → `SceneManifestChanged`
- `scenes/{scene}/{i}_{j}.json` → `ChunkChanged` / `ChunkRemoved`
- `scenes/{scene}/{i}_{j}.heightmap.bin` → `ChunkChanged` (v0.2.0)
- `lights/{slug}.json` → `LightStateChanged`
- Anything else → ignored

Filename matching is ASCII case-insensitive: `Crate.JSON` is
recognized identically to `crate.json`. Slug content within filenames
is still validated case-sensitively by `Slug::new` at load time.

Heightmap files coalesce into `ChunkChanged` rather than introducing
a new variant. Rationale: the chunk has effectively changed; a
separate variant would add API surface for no benefit. Consumers
respond to `ChunkChanged` the same way regardless of whether the
underlying file change was in the JSON or the sibling binary.

Heightmap filenames that don't parse as `{i}_{j}.heightmap.bin` with
`{i}, {j}` as i32 surface as `FileEvent::Error`, same pattern as
unparseable chunk filenames.

### Non-responsibilities

- Doesn't re-parse files.
- Doesn't deduplicate against actual content changes.
- Doesn't emit anything on startup.

---

## 7. Test Plan

### Round-trip (`tests/round_trip.rs`)

For each authored type — `Prefab`, `Scene`, `Chunk`, every
`RegionPurpose` variant, every `ShapePrimitive` variant, every
`ChunkEagerness` variant:

1. Construct hand-built instance.
2. Save to temp file.
3. Load from that file.
4. Equality check (requires `PartialEq` on all authored types).
5. Save loaded value to second temp file.
6. Byte-equal both files.

Steps 4 and 6 both must pass.

**Terrain additions (v0.2.0):**

- `terrain_round_trip_byte_identical`: construct a chunk with
  terrain, save, load, save again, compare bytes. Both JSON and
  sibling binary byte-equal across the round trip.
- `terrain_optional_field_round_trips`: chunk without terrain
  saves without the field, loads to `terrain: None`, saves again
  with no terrain field. Byte-equal across the round trip.
  Preserves byte-identity with `v0.1.0-wok-scene-baseline` files.
- `terrain_surface_tags_sorted_on_save`: construct a chunk with
  terrain whose surface tags are in non-alphabetical insertion
  order. After save, the sibling binary has them sorted.

### Asset ID parsing (`tests/asset_ids.rs`)

1. Parse `wooden-crate-267` → serial=267, slug=`wooden-crate`.
2. Parse `version-2-5` → serial=5, slug=`version-2`.
3. Parse `foo-0` → serial=0, slug=`foo`.
4. Parse `foo` → error (no separator).
5. Parse `foo-bar` → error (non-numeric serial).
6. Parse `WOODEN-CRATE-267` → error (slug validation, after split).
7. Round-trip through JSON: construct, serialize, deserialize, compare.
8. Equality: two `MeshId`s with same serial but different slugs are
   equal (slug is debug surface).
9. Hashing: two `MeshId`s with same serial hash to the same value
   regardless of slug.

### Slicing (`tests/slice.rs`)

A small fixture: one prefab with two states, one chunk with three
placements in known order.

1. Smoke — non-empty arrays produced; expected counts.
2. **Chunk-local coords** — placement at (10, 0, 5) in chunk (2, 1)
   produces local transform with translation (10, 0, 5). Crucially,
   *not* offset by the chunk's world position.
3. Transform composition — rotated placement × translated shape →
   correct local transform.
4. State selection — same prefab placed twice in different states →
   shapes from each state appear correctly.
5. Flag combinations — all four. `(false, false)` errors.
6. Order preservation — verify ordering and `source_placement` indices.
7. Unknown prefab → `UnknownPrefab`.
8. Unknown state → `UnknownState`.
9. Determinism — slice the same chunk twice; byte-identical output.
10. **Position independence** — slice the same chunk wrapped in two
    different `ChunkCoord` values; outputs differ ONLY in the `coord`
    field. This is the multiplayer-determinism property as a unit test.
11. Surface tag interning — `["wood", "metal", "wood"]` produces a
    2-entry table with correct indices.
12. Region purposes — fog, lighting, ambient all round-trip through
    slicing.
13. **Eagerness round-trip** — slicing Eager, Lazy, Vista produces
    identical shape/hitbox/trigger/region arrays. The only difference
    is `ChunkRuntime.eagerness`. Vista runtime semantics are
    `wok-content`'s problem; this test verifies the tag is preserved
    and the slicing is neutral.

**Terrain additions (v0.2.0):**

14. `terrain_slice_smoke`: chunk with simple terrain slices to a
    `RuntimeTerrain` with the expected shape.
15. `terrain_slice_position_independent`: slice the same chunk with
    two different `ChunkCoord` values. The `RuntimeTerrain` is
    byte-identical between the two outputs. Extension of test #10 to
    cover terrain.
16. `terrain_slice_deterministic`: slice the same chunk twice,
    outputs byte-equal.
17. `terrain_surface_table_merge`: chunk with a prefab using surface
    tag "wood" and terrain using surface tags ["grass", "wood",
    "stone"]. The runtime table has ["wood", "grass", "stone"]
    (prefab first by existing slicer order; terrain appends new in
    authored alphabetical order). Terrain cells originally
    referencing index 1 (wood) get remapped to index 0 in the runtime.
18. `terrain_surface_table_overflow_errors`: pathological chunk with
    >65535 distinct surface tags produces
    `SliceError::TerrainSurfaceTableOverflow`. Constructed
    artificially; would never appear in real authoring.

### Sampling (`tests/terrain_sampling.rs`, v0.2.0)

All sampling tests construct `ChunkRuntime` fixtures with populated
`terrain` fields. Fixture helper:

```rust
fn fixture_chunk_with_flat_terrain(height: f32) -> ChunkRuntime {
    let surface_tag_table = vec!["grass".to_string()];
    let terrain = RuntimeTerrain {
        heights: vec![quantize(height); 129 * 129].into_boxed_slice(),
        surface_indices: vec![0u16; 129 * 129].into_boxed_slice(),
        width: 129,
        vertical_range_meters: 32.0,
    };
    ChunkRuntime { /* ... */, surface_tag_table, terrain: Some(terrain) }
}
```

Tests:

- `height_at_returns_authored_at_integer_cells`: chunk with heightmap
  of known values at each integer cell; `height_at(0.0, 0.0)`,
  `height_at(1.0, 0.0)`, ..., return the dequantized authored values
  exactly.
- `height_at_interpolates_between_cells`: chunk with two adjacent
  cells at known heights; sampling at the midpoint returns the
  average (bilinear).
- `height_at_out_of_bounds_returns_none`: `height_at(-0.1, 5.0)` and
  `height_at(128.1, 5.0)` both return `None`. Under the shared-edge
  convention, `height_at(128.0, 5.0)` is in-domain and returns a
  valid value — the boundary row is part of the chunk's data. Domain
  is `[0, 128]` closed-closed, not half-open.
- `height_at_no_terrain_returns_none`: chunk with `terrain: None`;
  any sampling returns `None`.
- `normal_at_flat_terrain_is_up`: chunk with all cells at the same
  height; `normal_at` returns `Vec3::Y` (within float tolerance).
- `normal_at_sloped_terrain_tilts`: chunk with a known slope along
  the x axis; `normal_at` returns the expected tilted normal,
  computed via 1-cell gradient.
- `normal_at_out_of_bounds_returns_none`: same domain as `height_at`.
- `normal_at_no_terrain_returns_none`.
- `surface_at_returns_borrowed_str`: chunk with known surface
  indices; `surface_at` returns the expected tag string. Compile-time
  lifetime check via the borrow checker.
- `surface_at_out_of_bounds_returns_none`.
- `surface_at_no_terrain_returns_none`.

### Watcher (`tests/watcher.rs`)

Use `tempfile::tempdir` + short sleeps for debounce.

1. Create prefab → `PrefabChanged`.
2. Modify prefab → `PrefabChanged`.
3. Delete prefab → `PrefabRemoved`.
4. Create/modify chunk file with parseable name → `ChunkChanged`
   with correct coord.
5. Chunk file with unparseable name → `FileEvent::Error`.
6. Multiple rapid writes → debounced to one event per file.
7. File outside known prefixes → no event.

**Terrain additions (v0.2.0):**

8. `heightmap_modification_emits_chunk_changed`: scene with a chunk
   + sibling heightmap; modify the heightmap, expect
   `ChunkChanged { coord }`.
9. `unparseable_heightmap_filename_emits_error`: write
   `notachunk.heightmap.bin` to a scene directory; expect
   `FileEvent::Error`.

### Validation (`tests/validate.rs`)

1. Invalid slug (uppercase, space, leading slash) → construction error.
2. Unknown `_format` → `LoadError::UnsupportedVersion`.
3. Missing required field → `LoadError::Parse` with field name in
   message.
4. Both shape flags false in authored data — *accepted* (editor can
   produce this transiently); *rejected* at slice time.

### Integration (`tests/integration.rs`)

End-to-end tests exercising the full pipeline through public API.

- `full_workflow_load_and_slice`: write prefab + scene manifest +
  chunk to tempdir, load via the public API, slice, verify output
  matches expectations.
- `workflow_hot_reload`: install `FileWatcher`, load, slice, modify a
  chunk file on disk, poll watcher for `ChunkChanged`, reload, re-slice,
  verify the new state appears.

**Terrain additions (v0.2.0):**

- `full_workflow_load_slice_sample`: write a chunk JSON + sibling
  binary with known terrain, call `load_chunk`, call `slice_chunk`,
  call the three sampling functions at known points. End-to-end
  pipeline verification.
- `hot_reload_terrain_modification`: install `FileWatcher`, load
  chunk, modify heightmap file, poll watcher, re-load, re-slice,
  verify sampled values reflect the modification.

---

## 8. Vista Semantics (Forward Reference)

`ChunkEagerness::Vista` is a state in which a chunk is loaded and
partially active. The slicer in `wok-scene` is neutral — Vista chunks
produce the same runtime arrays as Eager or Lazy. Downstream crates
honor the tag as follows:

- **`wok-render`** — iterates Vista chunks' visible-shape arrays
  normally (same culling, lighting, shadows).
- **`wok-physics`** — skips Vista chunks entirely (no collision, no
  actor integration, no swept tests).
- **`wok-content`**'s trigger system — skips Vista chunks (no trigger
  evaluation, no event firing).
- **`wok-light`** — Vista chunks' geometry does not occlude dynamic
  lights (illuminated visually but not reactive).
- **`wok-anim`** — animation state still ticks on any actors that
  happen to be in Vista chunks (cheap, animations need to keep
  moving).

Eagerness transitions are pure flag flips on the runtime tag — same
runtime arrays, no reload, no I/O. The transition operation lives in
`wok-content` (`transition_chunk(id, new_eagerness)`).

Behavior tests for these semantics land in `wok-content`, not
`wok-scene`. This crate's tests verify only that the tag round-trips
and that slicing is neutral on it.

---

## 9. Gotchas

### Asset ID equality is serial-only

Two `MeshId`s with serial=42 are equal even if their slugs read
differently. This is what makes asset rename safe: existing references
to an asset continue to work after a rename, because the loader-parsed
slug is debug surface only, not identity. The same is true under
`Hash`, `PartialOrd`, `Ord`. Don't accidentally derive these — they
must be implemented manually. The `define_asset_id!` macro is what
guarantees the implementations stay consistent across kinds; a manual
`#[derive(PartialEq)]` on any of these types is a regression.

### Asset IDs are not registry-validated by this crate

`wok-scene` constructs `MeshId { serial: 9999, slug: "nonexistent" }`
without complaint. The "every ID points to something" invariant is
`wok-content`'s registry to enforce when the reference is resolved.
This keeps `wok-scene` independent of all other `wok-*` crates.

### Slug-serial parser splits on the last hyphen

`version-2-5` parses to slug=`version-2`, serial=5. If you reach for
"split on first hyphen" or "regex match `^(\\w+)-(\\d+)$`," you'll get
wrong behavior on slugs that contain hyphens (which is most of them).

### Runtime shapes are chunk-local, not world

The single most non-obvious thing in the runtime arrays. Consumers
compose `chunk.coord.to_world_offset()` when world coords are needed:

- Renderer: per-chunk model matrix on draw calls.
- Physics: broad-phase per-chunk before narrow-phase against local
  shapes.

The benefit: slicing is position-independent, which is what the
parallel-worlds multiplayer determinism story rests on. The cost is
one vector add per cross-chunk physics query.

### Cross-chunk regions are authored per-chunk

A fog zone covering a swamp across four chunks is authored as four
`RegionMarker`s with shared parameters, each clipped to its chunk's
extents. The authoring tool offers a "paint region across chunks"
operation that emits the four. Conceptually one region; on-disk four.
No special-case load path.

### Animation authoring lives in `wok-anim`, not here

Authored animation data (named poses, blend graphs, event markers)
lives in `wok-anim` next to the playback code that consumes it. This
preserves `wok-scene`'s independence from all other `wok-*` crates.
`wok-scene` does not know what an animation pose is.

### No runtime→authored writes, ever

Game state never modifies prefab or scene data. A chunk that unloads
and reloads comes back in its authored state. Games that want
persistence own that problem game-side via sidecar files; they query
the sidecar during chunk load and override or gate accordingly.
Actors are pool-managed in `wok-physics`, not part of `ChunkRuntime`,
so per-NPC state naturally persists across chunk lifecycle without any
engine-level snapshotting. The engine has no API for persistence.

### Vista is a partial mitigation for combat-across-boundaries

If a game does want combat that crosses chunk edges without the
mid-fight enemy losing collision, the relevant chunks should be kept
Eager rather than transitioning to Vista or unloading. Vista preserves
geometry and animation but not physics or trigger evaluation. The
correct framing is design discipline (R&C-style arenas with clear
loading boundaries) rather than engine machinery.

### No per-chunk snapshots

There is no `snapshot_chunk` / `restore_chunk` operation. Snapshots
exist at the whole-world level and are `wok-content`'s concern.

### `default_state` is a name, not an index

Linear lookup in a small list. Indices break across edits.

### `HashMap` iteration order is non-deterministic

Anything serializing from `HashMap` sorts first. Anything iterating
chunks for deterministic processing sorts coords lexically.
`PrefabState.audio_cues` is `BTreeMap` rather than `HashMap`
specifically to provide alphabetical iteration ordering on save.

### `_format` versions are not forward-compatible

Loader rejects unknown versions. When we bump, we provide a one-shot
migration via `load_*_legacy`. Authoring tools and data are versioned
together.

### `notify` crate has platform quirks

Atomic-save patterns (write-to-temp, rename) produce different event
sequences on Linux, macOS, and Windows. The debouncer absorbs most of
this; the watcher test suite should run on at least Linux and macOS
before declaring done.

Current status: Windows-verified. Linux and macOS verification
outstanding.

### Shared-edge convention (v0.2.0)

Each chunk's heightmap stores 129 × 129 cells. The boundary row and
column are duplicated to the neighboring chunk's matching row and
column — chunk A's right edge (x=128) is also chunk B's left edge
(x=0), and both chunks store the same height values at those cells.

**Reasoning:** the architectural cleanliness argument outweighed the
single-source-of-truth concern:

- Sampling functions are pure over a single `ChunkRuntime`'s data.
  Querying `height_at(128.0, 5.0)` on chunk A returns a valid value
  from chunk A's own data; the sampler doesn't need to reach into
  chunk B's `RuntimeTerrain`. This preserves the design rule "pure
  functions live with their data type."
- Memory cost is trivial: 129 × 129 vs 128 × 128 is ~1.5% more cells,
  ~0.8% more total bytes per chunk's sibling binary.
- The alternative (128 × 128 with no shared edge) would either
  restrict the sampling domain to `[0, 128)` half-open (which
  surprises consumers — `height_at(128.0, ...)` returning `None`
  for a coordinate that physically exists in the world is a footgun)
  or require samplers to take multiple `ChunkRuntime` arguments to
  reach across the boundary (which breaks the function's purity
  boundary).

**Editor responsibility:** boundary consistency is the editor's job,
not the slicer's. When wok-shell's painting tools modify a cell on
chunk A's right edge (x=128, any z), they must also modify the
matching cell on chunk B's left edge (x=0, same z), and similarly for
top/bottom edges along the z axis and corner cells shared by four
chunks.

The slicer does not verify edge match. A chunk with a mismatched edge
slices cleanly and produces a `RuntimeTerrain` with whatever the
authored data contained, even if it disagrees with the neighbor. The
visible result at runtime is a seam at the chunk boundary. Same
pattern as cross-chunk regions — authoring tools maintain consistency,
slicer trusts the input.

**Pinned:** future implementers should not add slicer-level
verification. If a future contributor proposes a
`verify_chunk_boundaries` pass in the slicer, the answer is no. The
slicer has no awareness of neighbor chunks (and acquiring it would
inject cross-chunk dependencies that break the slicer's purity).
Boundary consistency lives in the editor.

### Normal computation method (v0.2.0)

`normal_at` computes the surface normal from a 1-cell gradient: the
height difference between the cells immediately neighboring the
sample point, projected into a normal vector.

**Formula:** for a sample point at chunk-local `(x, z)`:

```
dh_dx = (height_at_cell(ceil_x, sample_z) - height_at_cell(floor_x, sample_z)) / 1.0
dh_dz = (height_at_cell(sample_x, ceil_z) - height_at_cell(sample_x, floor_z)) / 1.0
normal = normalize(Vec3::new(-dh_dx, 1.0, -dh_dz))
```

`cell_size` is 1.0 meter (the fixed authoring resolution). The
heights are dequantized from u16 to meters before the gradient
calculation, using the terrain's `vertical_range_meters`.

**Reasoning:** pragmatic path. 1-cell is simpler to implement,
simpler to test, simpler to reason about. The alternative (3-cell
window) smooths the normal across a wider neighborhood, which would
reduce visible per-quad discontinuities under cel-band lighting. But
we don't yet have rendering evidence that 1-cell normals produce
visible shimmer in this engine's specific cel-shading configuration.

**Upgrade trigger:** if smoke-test visual review reveals cel-band
shimmer on terrain — specifically: per-quad lighting discontinuities
amplified by the cel quantization, manifesting as visible facets or
"stepped" shading on what should appear as smooth slopes — switch
`normal_at`'s implementation to a 3-cell window.

The 3-cell window averages the gradient over a wider area: instead of
comparing `cell[x+1] - cell[x-1]`, compare `cell[x+2] - cell[x-2]`
with optional weighting toward the center. The exact weights are an
implementation detail; the smoothing effect is what matters.

**What changes under the upgrade:** only `normal_at`'s implementation
body. The signature, the tests (which assert expected normals from
known input geometry — the expected values would update), the
authored data, the runtime type, and all other sampling functions
are unaffected. CC's implementation pass could swap from 1-cell to
3-cell in a single PR if rendering review demands it.

**Don't smooth across chunk boundaries:** the 3-cell window, if
adopted, samples cells `[x-2, x-1, x, x+1, x+2]`. When `x` is near a
chunk boundary, the implementation must either clamp the window to
the chunk's domain or accept a discontinuity. Under the 129 × 129
shared-edge convention, cells 0 and 128 are duplicated in the
neighbor, so clamping at the boundary is equivalent to extending into
the neighbor's data — the gradient at the boundary is well-defined
under either interpretation.

This is a 3-cell-implementation detail; 1-cell gradient doesn't have
this concern.

### Sampling signature uniformity (v0.2.0)

All three sampling functions (`height_at`, `normal_at`, `surface_at`)
take `&ChunkRuntime` as their first argument, not `&RuntimeTerrain`.

**The asymmetry:** `height_at` and `normal_at` strictly need only the
terrain heightmap. They could take `&RuntimeTerrain` directly.
`surface_at` needs both the heightmap and the surface tag table —
which lives on `ChunkRuntime`, not on `RuntimeTerrain`, because it
contains entries for both prefab hitbox tags and terrain surface tags
merged at slice time.

Under per-function-needs signatures, call sites would mix shapes:

```rust
let h = height_at(&runtime.terrain.as_ref()?, x, z)?;
let n = normal_at(&runtime.terrain.as_ref()?, x, z)?;
let s = surface_at(&runtime.terrain.as_ref()?, &runtime.surface_tag_table, x, z)?;
```

Under uniform `&ChunkRuntime`:

```rust
let h = height_at(&runtime, x, z)?;
let n = normal_at(&runtime, x, z)?;
let s = surface_at(&runtime, x, z)?;
```

Consistent parallelism. Internal `?` on `chunk.terrain.as_ref()`
handles the terrain-absent case uniformly.

**Reasoning:** the "pure functions live with their data type" rule
still applies; the data type for these sampling functions is
`ChunkRuntime` as a whole. Sampling operates on the chunk —
heightmap data + surface table together — not on the heightmap in
isolation. Uniformity at call sites is the visible benefit; one
Option check per function is the negligible cost.

**Pinned:** don't fragment the signatures later. If a future
contributor proposes a more efficient
`height_at_terrain(&RuntimeTerrain, x, z)` that skips the
`chunk.terrain.as_ref()` indirection, the answer is no. The
indirection cost is one Option check — negligible. The API
fragmentation cost is permanent.

### Sibling-binary save ordering (v0.2.0)

`save_chunk` writes the sibling binary first, then the chunk JSON.
Both use `std::fs::write` directly — the same plain-write style as
v0.1.0 prefab and chunk saves. There is no temp-file-and-rename
atomicity; matching the shipped style was the deliberate choice
over introducing atomicity for terrain alone.

The binary-before-JSON ordering is the actual invariant. The chunk
JSON only references a heightmap file that already exists on disk;
a successful JSON write therefore points at a binary the loader can
find.

Failure modes under the plain-write style:

- **Crash between the two writes** leaves an updated binary on disk
  with an old (or absent) JSON reference. Recovery: next save
  overwrites both. No corruption; on-disk state is at worst stale.
- **Crash during the binary write itself** can leave a partially
  written sibling file. The loader's `LoadError::TerrainMalformed`
  surfaces this (magic-bytes / length-mismatch checks). The editor
  is responsible for surfacing the error and either re-saving or
  letting the user re-author.
- **Chunk JSON references a binary that doesn't exist** surfaces as
  `LoadError::TerrainSiblingMissing` rather than silent terrain-less
  load. Editor's responsibility to handle (remove the dangling
  reference or re-author the heightmap).

**Pinned: atomic save retrofit is a separable concern.** Adding
temp-file-and-rename across all of wok-scene's writes (prefab,
chunk JSON, chunk sibling binary) is a worthwhile improvement, but
it's a uniform retrofit and belongs in its own PR — not folded into
a terrain feature. The retrofit becomes more interesting when
multiplayer determinism is at stake (a corrupted save can desync
clients); until then, the plain-write style is fine for the
single-player Phase 4 milestone.

### Determinism property extension (v0.2.0)

Terrain extends, doesn't break, the slicer's existing determinism
and position-independence properties:

- **Determinism:** same `Chunk` (including `TerrainData`) sliced
  twice produces byte-identical `ChunkRuntime` (including
  `RuntimeTerrain`). Surface table merge is order-deterministic
  because authored terrain tags are sorted on save.

- **Position-independence:** `RuntimeTerrain` contains nothing
  derived from `ChunkCoord`. Slicing the same chunk authored data at
  two different coords produces `RuntimeTerrain` values that are
  byte-equal; only the surrounding `ChunkRuntime.coord` field
  differs.

These properties matter for the multiplayer-determinism story (per
project-canon's determinism contract). Tested in §7.

---

## 10. What This Crate Is Explicitly Not

- **Scene graphs / hierarchy / parenting** beyond prefab → state →
  shapes. Flat placements.
- **Components / ECS.**
- **Animation authoring data.** Lives in `wok-anim`.
- **Snapshots of any kind.** Authored save/load lives here; runtime
  state snapshotting (per-chunk or whole-world) does not.
- **Streaming logic.** Chunks carry topology metadata; the streaming
  algorithm is `wok-content`'s.
- **Asset registry.** Asset IDs are defined here; the registry that
  maps them to concrete loaded assets is `wok-content`'s.
- **Game-event triggers as environmental regions.** Game events come
  from prefab trigger volumes (hitbox-only shapes). Region markers
  are environmental zones (fog, lighting, ambient).
- **Vista runtime behavior.** Tag defined here; semantics enforced
  downstream.
- **Terrain chunk-boundary verification (v0.2.0).** Boundary
  consistency between adjacent chunks is the editor's job. The
  slicer trusts authored data and does not verify edge match.

---

## 11. Order of Implementation

### v0.1.0 (shipped at `v0.1.0-wok-scene-baseline`)

1. **`ids/`** — Slug, content IDs, asset IDs, chunk coords. Tests:
   construction, equality (serial-only for assets), slug rules,
   slug-serial parsing edge cases.
2. **`authored/shape.rs`, `authored/prefab.rs`** — Prefab and
   dependencies. Tests: round-trip.
3. **`authored/chunk.rs`, `authored/streaming.rs`, `authored/scene.rs`**
   — chunks and manifest. Tests: round-trip; manifest-references-chunks.
4. **`runtime/`** — types only, no tests yet.
5. **`slice.rs`** — the slicer. Tests: all of §7's slicing tests
   1-13. This is the keystone.
6. **`load.rs`, `save.rs`** — serde_json wrappers. Tests: end-to-end
   load-modify-save-reload.
7. **`watcher.rs`** — file watcher. Tests last because flakiest.

### v0.2.0 (terrain)

8. **`authored/terrain.rs`** — `TerrainData` type, sibling binary
   format. Load/save integration: extend `load_chunk` to read the
   sibling, extend `save_chunk` to write the sibling first then the
   JSON. New error variants. Tests: round-trip with terrain.
9. **`runtime/terrain.rs`** — `RuntimeTerrain` type. `ChunkRuntime`
   gains the `terrain: Option<RuntimeTerrain>` field. Types only;
   no tests yet.
10. **`slice.rs` extension** — terrain pass: surface table merge,
    `slice_terrain` helper. Tests: §7's slicing tests 14-18.
11. **`sampling.rs`** — `height_at`, `normal_at`, `surface_at` with
    uniform `&ChunkRuntime` signatures. 1-cell gradient for
    `normal_at`. Tests: §7's sampling tests.
12. **`watcher.rs` extension** — heightmap filename classification,
    coalescing into `ChunkChanged`. Tests: §7's watcher tests 8-9.
13. **Integration tests** — end-to-end load → slice → sample;
    hot-reload terrain modification.

After all v0.2.0 checkpoints land: bump `wok-scene` to v0.2.0, tag
`v0.2.0-wok-scene`, push to GitHub.
