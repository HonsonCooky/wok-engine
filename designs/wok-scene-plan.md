# `wok-scene` — Detailed Crate Plan

The bedrock data-model crate. Defines every authored type that lives
on disk as JSON, the runtime-array types those get sliced into, the
slicing function itself, save/load, and a file watcher for hot reload.
No GPU, no simulation, no logic. Pure data and pure transformations
between two shapes of data.

Depends only on `pantry` (which re-exports `glam`, `serde`,
`serde_json`) plus `notify` as a direct dependency. No other `wok-*`
crate.

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
    │   └── streaming.rs    # ChunkMetadata, ChunkEagerness
    ├── runtime/
    │   ├── mod.rs
    │   ├── shape.rs        # VisibleShape, PhysicalHitbox, TriggerVolume
    │   ├── region.rs       # RuntimeRegion
    │   └── chunk.rs        # ChunkRuntime
    ├── slice.rs            # slice_chunk
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
    pub fn to_world_offset(self) -> Vec3 {
        Vec3::new(self.x as f32 * 128.0, 0.0, self.z as f32 * 128.0)
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
    pub audio_cues: Vec<(String, AudioCueId)>,
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

pub struct Chunk {
    pub coord: ChunkCoord,
    pub metadata: ChunkMetadata,
    pub placements: Vec<PrefabPlacement>,
    pub regions: Vec<RegionMarker>,
    pub light_state: LightStateRef,
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

pub struct ChunkRuntime {
    pub coord: ChunkCoord,
    pub eagerness: ChunkEagerness,      // carried through; downstream consults it
    pub visible: Vec<VisibleShape>,
    pub hitboxes: Vec<PhysicalHitbox>,
    pub triggers: Vec<TriggerVolume>,
    pub regions: Vec<RuntimeRegion>,
    pub light_state: LightStateRef,
    pub surface_tag_table: Vec<String>,
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
```

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
smoke-test/content/
├── prefabs/
│   ├── crate-wooden.json
│   └── door-simple.json
├── scenes/
│   └── act1-warehouse/
│       ├── scene.json        # manifest
│       ├── 0_0.json          # chunk at (0,0)
│       ├── 1_0.json
│       └── 0_1.json
└── lights/
    └── warehouse-day.json
```

Chunk file names use `_` not `-` as the coord separator
(`{i}_{j}.json`) to avoid confusion with asset ID parsing. Filenames
that don't parse as `{i}_{j}.json` with `i, j` as `i32` surface as
`FileEvent::Error` rather than silent drop.

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
      "mesh_override": "wooden-crate-mesh-267"
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
  ]
}
```

### Round-trip determinism requirement

`load(save(load(x))) == load(x)`, byte-identical. Anything serialized
from a `HashMap` is sorted on save. Tested explicitly (see §7).

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

    Ok(ChunkRuntime {
        coord: chunk.coord,
        eagerness: chunk.metadata.eagerness,
        visible, hitboxes, triggers, regions,
        light_state: chunk.light_state.clone(),
        surface_tag_table: surface_intern.into_vec(),
    })
```

### Properties

1. **Deterministic.** Same inputs → bit-identical output.
2. **Single allocation per output vector** (counting pass or
   conservative `with_capacity`).
3. **Pure.** No I/O, no global state.
4. **Order-preserving.** Placement order, then shape-within-state order.
5. **Position-independent.** Output is identical regardless of
   `chunk.coord` except for the `coord` field itself. This is the
   determinism property the parallel-worlds multiplayer model depends
   on: the same authored chunk file produces the same runtime arrays
   on every client, regardless of where in the world the chunk sits.
6. **Eagerness-neutral.** Slicing produces identical arrays for Eager,
   Lazy, and Vista chunks. The tag is carried through to
   `ChunkRuntime.eagerness` for downstream consumers.

### Performance posture

10–500 placements per chunk, 1–20 shapes per placement. Sub-millisecond
on commodity hardware. No internal parallelism — `wok-content` can
`rayon`-parallelize across chunks if profiles ever justify it.

---

## 6. File-Watcher Behavior

### Lifecycle

- `FileWatcher::new(content_root)` spawns a debounced `notify` watcher
  rooted at `content_root` (~100ms debounce).
- Background thread classifies FS events into `FileEvent`s and pushes
  to an `mpsc` queue.
- `poll()` drains the queue.

### Classification

Path relative to `content_root`:

- `prefabs/{slug}.json` → `PrefabChanged` / `PrefabRemoved`
- `scenes/{scene}/scene.json` → `SceneManifestChanged`
- `scenes/{scene}/{i}_{j}.json` → `ChunkChanged` / `ChunkRemoved`
- `lights/{slug}.json` → `LightStateChanged`
- Anything else → ignored

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

### Validation (`tests/validate.rs`)

1. Invalid slug (uppercase, space, leading slash) → construction error.
2. Unknown `_format` → `LoadError::UnsupportedVersion`.
3. Missing required field → `LoadError::Parse` with field name in
   message.
4. Both shape flags false in authored data — *accepted* (editor can
   produce this transiently); *rejected* at slice time.

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

### `_format` versions are not forward-compatible

Loader rejects unknown versions. When we bump, we provide a one-shot
migration via `load_*_legacy`. Authoring tools and data are versioned
together.

### `notify` crate has platform quirks

Atomic-save patterns (write-to-temp, rename) produce different event
sequences on Linux, macOS, and Windows. The debouncer absorbs most of
this; the watcher test suite should run on at least Linux and macOS
before declaring done.

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

---

## 11. Order of Implementation

1. **`ids/`** — Slug, content IDs, asset IDs, chunk coords. Tests:
   construction, equality (serial-only for assets), slug rules,
   slug-serial parsing edge cases.
2. **`authored/shape.rs`, `authored/prefab.rs`** — Prefab and
   dependencies. Tests: round-trip.
3. **`authored/chunk.rs`, `authored/streaming.rs`, `authored/scene.rs`**
   — chunks and manifest. Tests: round-trip; manifest-references-chunks.
4. **`runtime/`** — types only, no tests yet.
5. **`slice.rs`** — the slicer. Tests: all of §7's slicing tests. This
   is the keystone.
6. **`load.rs`, `save.rs`** — serde_json wrappers. Tests: end-to-end
   load-modify-save-reload.
7. **`watcher.rs`** — file watcher. Tests last because flakiest.
