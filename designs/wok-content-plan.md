# `wok-content` — Detailed Crate Plan (v1)

The orchestrator crate that sits between `wok-scene` and everything
else. Owns the asset registry, the chunk lifecycle (load / unload /
transition), the streaming algorithm, the background worker thread,
placeholder mesh data and GPU upload coordination, whole-world
snapshots, and Vista enforcement.

This is the biggest crate in the engine by surface area and the first
one that genuinely orchestrates side effects. `wok-scene` was 90% data
and 10% pure transformation. `wok-content` is 30% data and 70%
coordination — threads, channels, GPU queues, lifecycle state
machines, and the failure modes that come with all of them.

Depends on `pantry` (which re-exports `wgpu`, `glam`, `serde`,
`serde_json`, `bytemuck`) and `wok-scene`. No other `wok-*` crate.
This dependency closure holds — §1.3 documents how trigger
evaluation is decomposed to keep wok-content physics-agnostic.

---

## 1. Design Rules

### 1.1 Orchestration only — no authored types redefined

`wok-scene` defines authored types. `wok-content` does not redefine
them, does not extend them, does not parallel them. When
`wok-content` needs to know what a `Shape` is, it asks `wok-scene`.

The only types `wok-content` introduces are:

- **Registry types** — `AssetEntry`, `Registry`, `RegistryDelta`. The
  identity table for the engine's assets.
- **Runtime-storage types** — `MeshGpu`, `MeshCpu`, `AudioBuffer`.
  Concrete loaded data behind asset IDs. Not authored; not on disk.
- **Lifecycle types** — `ChunkSlot`, `SlotState`, `ResidentChunk`,
  `LoadHandle`, `LoadError`, `LoadStatus`. State machine for
  in-flight and resident chunks.
- **Worker-protocol types** — `WorkerRequest`, `WorkerResult`. The
  messages that cross the channel.
- **Snapshot types** — `EngineSnapshot`, `StitchId`. The
  engine's view of world state; one stream of the game's
  composite save file.

Everything else — `Chunk`, `Prefab`, `ChunkRuntime`, `MeshId`,
`Slug`, `ChunkEagerness` — is `wok-scene`'s and `wok-content` uses
it directly.

### 1.2 One-way data flow remains absolute

The `wok-scene` rule "no runtime→authored writes, ever" extends here.
`wok-content` produces runtime arrays (via `wok-scene::slice_chunk`)
and runtime storage (mesh GPU buffers, etc.) from authored data, and
never writes anything back into authored data.

The registry has two halves and the distinction matters:

- **Identity half** (serial allocation, slug↔serial map, usage tracking)
  is mutable at runtime. The editor adds entries, renames them, edits
  usage lists. This is authoring data with its own JSON file on disk.
- **Storage half** (loaded GPU buffers, CPU mesh data, audio buffers)
  is derived data — discardable, recomputable. Lives only in memory.

These two halves are separate structs sharing a lookup key (serial).
Conflating them is the obvious mistake.

### 1.3 Trigger evaluation: three-way split

Triggers are decomposed across three crates. None of them owns the
whole concept.

- **wok-content** owns the **trigger volume data**. The `triggers:
  Vec<TriggerVolume>` array on each `ChunkRuntime` (produced by
  `wok-scene::slice_chunk`) is held resident as part of the
  `ChunkSlot` and exposed by accessor. wok-content also owns the
  per-slot `ChunkEagerness` flag.
- **wok-physics** owns the **overlap test**. It already iterates
  loaded chunks for collision and already skips Vista chunks
  (per wok-scene plan §8). It adds one query —
  `actors_overlapping_volumes(...)` — that walks actors against
  the supplied volumes using the same capsule/AABB math it
  already maintains. Vista chunks produce no overlaps because
  wok-physics skips them, identically to how it skips them for
  collision.
- **The game** owns the **event routing**. It calls wok-physics's
  overlap query with the trigger volumes from wok-content's
  loaded slots, receives `(actor_id, trigger_id)` pairs, and
  routes them to its own game-event handlers.

Vista enforcement is a chunk-state property carried by wok-content
and honored at the consumption point. wok-physics enforces it for
both collision and trigger overlap; wok-render does not enforce it
(Vista chunks render normally); wok-anim does not enforce it
(animation keeps ticking). wok-content's job is to expose the
flag truthfully, not to filter on consumers' behalf.

This is the only structurally clean answer:

- It preserves the forbidden edge as written in the high-level
  design. wok-content stays physics-agnostic, which is what lets
  it serve as the asset pipeline for non-simulation contexts (the
  editor's prefab preview, headless asset validation, future
  tools).
- The overlap test is genuinely wok-physics's job — same math as
  swept-AABB queries it already runs. Putting it in wok-content
  would mean reimplementing capsule-vs-AABB overlap there.
- It puts authored data in the crate that owns authored data
  (wok-content, via wok-scene), not in wok-physics. wok-physics
  takes a borrowed slice of volumes per query; it does not
  retain them.
- It matches the "primitives, not features" principle: the engine
  provides data + math, the game composes them into game events.

The remaining cost is that the game calls two engine functions
per tick instead of one — wok-content for the volumes and
wok-physics for the overlap test. That's bookkeeping, not
complexity, and it surfaces the dependency on actor data
explicitly rather than smuggling it into wok-content.

**Consequence for this plan**: wok-content does not have a
"trigger system" submodule. It has a trigger volume array on each
resident slot and an accessor pattern for iteration. The §2 file
layout, §7.4 Vista tests, §8.1 wok-physics interaction, and §10
scope exclusion all reflect this.

### 1.4 Background-worker contract: stateless requests

The worker thread does not own engine state. It does not hold a
`Registry`, does not hold a `ContentSystem`, does not hold any lock
the main thread also takes.

Each `WorkerRequest` is self-contained: it carries the file paths,
the parameters, and whatever borrowed-immutable data the worker
needs (typically `Arc<RegistryReadView>` — a frozen snapshot of the
serial→path map at submission time). The worker produces a
`WorkerResult` and sends it back. The main thread integrates it.

This rules out:

- Worker mutating the registry directly.
- Main thread blocking on a worker-held lock.
- Race conditions on partially-loaded chunks (the chunk doesn't
  exist in the lifecycle map until the main thread inserts it on
  result arrival).

The cost is that worker requests are slightly larger (have to carry
their context). The payoff is that the threading model has zero
shared mutable state and is trivial to reason about.

### 1.5 GPU upload boundary: wok-content holds buffers, wok-render holds pipelines

`wok-content` holds:

- `wgpu::Buffer` for each mesh (vertex + index buffers).
- `wgpu::Buffer` for any future texture / material data when shipped
  assets arrive.
- The mapping from `MeshId` → buffer handle.

`wok-render` (which depends on `wok-content`) holds:

- The render pipelines, bind group layouts, shaders.
- The per-frame command encoder.
- Knowledge of which bind groups to attach to draw the meshes
  `wok-content` provides.

The boundary: `wok-content` answers "what buffer holds this mesh?";
`wok-render` answers "how is that buffer drawn?".

For Phase 4, "mesh" is procedurally-generated primitive geometry
(cube faces, ellipsoid tessellation, etc.) baked at registry-build
time and uploaded once. No GLTF, no textures, no materials.

`wok-content` calls `wgpu::Device::create_buffer_init` itself; it
does not delegate to a "GPU helper" in pantry. pantry's role is
device acquisition and re-export. Buffer creation policy is
wok-content's domain.

---

## 2. Crate Layout

```
wok-content/
├── Cargo.toml
└── src/
    ├── lib.rs                  # Re-exports
    ├── error.rs                # All error types
    │
    ├── registry/
    │   ├── mod.rs              # Registry, public API
    │   ├── entry.rs            # AssetEntry, AssetKind, usage tracking
    │   ├── alloc.rs            # Serial allocation per-kind
    │   ├── rename.rs           # Rename operations + collision detection
    │   ├── populate.rs         # Auto-population from Scene/Prefab walk
    │   ├── view.rs             # RegistryReadView (Arc-clonable snapshot)
    │   └── serde.rs            # On-disk format for the registry file
    │
    ├── primitives/
    │   ├── mod.rs              # Procedural mesh generation
    │   ├── cube.rs
    │   ├── ellipsoid.rs
    │   ├── cylinder.rs
    │   ├── capsule.rs
    │   └── plane.rs
    │
    ├── storage/
    │   ├── mod.rs              # Storage layer over loaded assets
    │   ├── mesh.rs             # MeshGpu, MeshCpu, upload helpers
    │   └── audio.rs            # AudioBuffer (stub for Phase 4)
    │
    ├── chunk/
    │   ├── mod.rs              # ChunkSlot, ContentSystem chunk operations
    │   ├── slot.rs             # ChunkSlot state machine
    │   ├── load.rs             # load_chunk pipeline (orchestrator)
    │   ├── unload.rs           # unload_chunk
    │   └── transition.rs       # transition_chunk (Vista state flip)
    │
    ├── streaming/
    │   ├── mod.rs              # Streaming algorithm entry
    │   ├── desired.rs          # Compute desired chunk set from camera
    │   ├── hysteresis.rs       # Per-chunk load/unload radii + LRU tracking
    │   ├── interlock.rs        # Interlock resolution
    │   └── prioritize.rs       # Load-order priority queue
    │
    ├── worker/
    │   ├── mod.rs              # Worker thread lifecycle
    │   ├── request.rs          # WorkerRequest enum
    │   ├── result.rs           # WorkerResult enum
    │   ├── pipeline.rs         # The "load a chunk" pipeline on the worker
    │   └── channels.rs         # mpsc setup, priority queue feed
    │
    ├── snapshot/
    │   ├── mod.rs              # EngineSnapshot, StitchId, schema_hash
    │   ├── capture.rs          # Building an EngineSnapshot from current state
    │   ├── restore.rs          # Applying an EngineSnapshot
    │   └── format.rs           # On-disk / on-wire serialization
    │
    ├── system.rs               # ContentSystem — the public entry point
    └── config.rs               # ContentConfig — tunables for streaming etc.
```

Target < 400 lines per file. Some — `system.rs`, `streaming/mod.rs` —
will push that ceiling. `worker/pipeline.rs` is the most complex
single file; if it grows past 400 lines, split by request kind.

---

## 3. Public API Surface

This section is API skeletons with prose for what's not yet decided.

### 3.1 The system

```rust
pub struct ContentSystem {
    // Roots
    content_root: PathBuf,                   // for the watcher; for resolving scene_dir paths

    // Identity layer
    registry: Registry,

    // Storage layer
    meshes: HashMap<MeshId, MeshGpu>,        // serial-keyed, via Hash impl
    audio: HashMap<AudioCueId, AudioBuffer>, // stub for Phase 4

    // Chunk lifecycle
    slots: HashMap<ChunkCoord, ChunkSlot>,
    lru: VecDeque<ChunkCoord>,               // for eviction tie-break

    // Streaming state
    streaming: StreamingState,

    // Worker
    worker: WorkerHandle,                    // join + channels

    // Hot reload
    watcher: wok_scene::FileWatcher,         // polled from poll()

    // Loaded scene (None when no scene is loaded)
    scene: Option<LoadedScene>,
}

/// Resident scene state. Holds the manifest, every chunk's authored
/// data (eager-loaded at scene boot — see §5.2), and the prefab set
/// the scene references. `prefabs` is `Arc`-wrapped so worker
/// requests can cheaply clone the handle without copying the map.
pub struct LoadedScene {
    pub manifest: wok_scene::Scene,                    // the scene manifest
    pub scene_dir: PathBuf,                            // absolute, derived from content_root
    pub chunks_authored: HashMap<ChunkCoord, Arc<wok_scene::Chunk>>,
    pub prefabs: Arc<HashMap<PrefabId, wok_scene::Prefab>>,
}

impl ContentSystem {
    /// `content_root` is the directory containing `prefabs/`, `scenes/`,
    /// `lights/`, and `registry.json` (see §4.4 for layout). The watcher
    /// is constructed against this root and polled from `poll()`.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        content_root: PathBuf,
        config: ContentConfig,
    ) -> Result<Self, LoadError>;            // Err if registry.json read fails

    pub fn shutdown(self);

    // Scene boot — one scene loaded at a time (see §9.13). Calling
    // load_scene while a scene is already loaded unloads it first.
    // `scene_dir` is resolved relative to `content_root`. Eagerly
    // loads all chunk-authored data plus the referenced prefab set
    // (see §5.2 "Scene boot preconditions").
    pub fn load_scene(&mut self, scene_dir: &Path) -> Result<(), LoadError>;
    pub fn unload_scene(&mut self);

    // Chunk lifecycle (all game-initiated)
    pub fn request_load(&mut self, coord: ChunkCoord) -> LoadHandle;
    pub fn request_unload(&mut self, coord: ChunkCoord);
    pub fn transition_chunk(&mut self, coord: ChunkCoord, new: ChunkEagerness)
        -> Result<(), TransitionError>;

    // Streaming — game calls each tick with camera position
    pub fn tick_streaming(&mut self, camera_world: Vec3);

    // Worker integration — main-thread pump, drains results
    pub fn poll(&mut self) -> Vec<ContentEvent>;

    // Read access (handed to wok-render, wok-physics, wok-anim, game).
    // Iteration accessors yield (coord, &ResidentChunk) directly —
    // see §3.2 "Canonical iteration".
    pub fn slot(&self, coord: ChunkCoord) -> Option<&ChunkSlot>;
    pub fn slots(&self) -> impl Iterator<Item = (ChunkCoord, &ResidentChunk)>;
    pub fn active_slots(&self) -> impl Iterator<Item = (ChunkCoord, &ResidentChunk)>;
    pub fn vista_slots(&self) -> impl Iterator<Item = (ChunkCoord, &ResidentChunk)>;

    // Authored data access — for consumers that need to compare
    // authored intent against current runtime state (debug overlays,
    // game logic deciding whether to transition back, etc.). Returns
    // None if the chunk isn't part of the loaded scene at all.
    pub fn authored_eagerness(&self, coord: ChunkCoord) -> Option<ChunkEagerness>;

    // Registry access
    pub fn registry(&self) -> &Registry;
    pub fn mesh(&self, id: MeshId) -> Option<&MeshGpu>;

    // Snapshot — engine state only. Game stitches with physics
    // and game snapshots into a composite save file (see §5.3).
    pub fn capture_engine_snapshot(&self, stitch_id: StitchId) -> EngineSnapshot;
    pub fn restore_engine_snapshot(&mut self, snap: &EngineSnapshot)
        -> Result<StitchId, SnapshotError>;
}
```

`device` and `queue` arrive `Arc`-wrapped because wgpu is internally
shareable and the worker can clone+upload from its own thread.

`StitchId` is an opaque token (typically a UUID) the game generates
at capture and writes into each of its three snapshot streams. The
engine writes it into `EngineSnapshot.stitch_id` and returns it
from restore — never inspects it. Game's composition layer
compares stitch_ids across streams to verify they belong together.

### 3.2 Slots

```rust
pub struct ChunkSlot {
    pub coord: ChunkCoord,
    pub state: SlotState,
    pub touched_tick: u64,                   // for LRU
}

pub enum SlotState {
    Pending,                                 // requested, not yet on worker
    Loading,                                 // on worker
    Resident(ResidentChunk),                 // ready to use
    Unloading(ResidentChunk),                // marked for release; data still readable this frame
    Failed(LoadError),
}

pub struct ResidentChunk {
    pub runtime: ChunkRuntime,
    pub gpu: ChunkGpuHandles,
}

pub struct ChunkGpuHandles {
    pub visible_mesh_refs: Vec<MeshGpuRef>,
    // Phase 4 holds nothing more; later phases add textures, etc.
}

pub struct MeshGpuRef {
    pub mesh_id: MeshId,
    pub source_visible_index: u32,           // index into runtime.visible
    pub local_transform: Mat4,
}
```

The "if Resident, runtime is Some" invariant is encoded in the
type. There's no way to have a `Resident` variant without a
`ResidentChunk`, and no way to read `ResidentChunk` data from a
slot that isn't Resident or Unloading. Drift bugs are
unrepresentable.

**`Unloading` carries the same `ResidentChunk` payload as
`Resident`** — the unload state is "still in memory, GPU buffers
being released, consumers can still read." Keeping the variants
symmetric means iteration accessors yield both without
ceremony, and the rendering/physics frame doesn't pop the chunk
mid-frame when unload is requested. If `Unloading` ever needs
additional state (e.g., a release-tick counter), add it with a
deliberate type change at that point — don't pre-diverge.

The runtime eagerness lives on `ResidentChunk.runtime.eagerness`
(wok-scene's field, populated by `slice_chunk`). There is no
second eagerness field on the slot — single source of truth.
`transition_chunk` mutates that field directly via pattern match
on `SlotState`.

The authored eagerness is queried separately via
`ContentSystem::authored_eagerness(coord)` (which reads the loaded
scene's chunk metadata). Authored and runtime eagerness start
equal at load time and may diverge if the game calls
`transition_chunk`. Consumers that need to compare (debug
overlays, "should I transition this back" game logic) read both.
Most consumers only need the runtime value, which is what the
iteration accessors filter on.

### Canonical iteration: accessors first, pattern-match rare

The iteration accessors are the canonical way for consumers
(wok-render, wok-physics, wok-anim, wok-light, game code) to walk
the chunk world. They yield `(ChunkCoord, &ResidentChunk)`
directly, hiding the `SlotState` pattern match inside the
accessor's `filter_map`:

```rust
// Accessors — pattern-match happens once, inside the accessor.
pub fn slots(&self) -> impl Iterator<Item = (ChunkCoord, &ResidentChunk)>;
pub fn active_slots(&self) -> impl Iterator<Item = (ChunkCoord, &ResidentChunk)>;
pub fn vista_slots(&self) -> impl Iterator<Item = (ChunkCoord, &ResidentChunk)>;

// Consumer use — no pattern match, no Option-unwrap, no panic surface.
for (coord, chunk) in system.active_slots() {
    physics.integrate_against(&chunk.runtime.hitboxes);
}
```

`slots()` yields all `Resident` and `Unloading` slots together.
`active_slots()` further filters to non-Vista runtime eagerness.
`vista_slots()` filters to Vista runtime eagerness.

**Direct pattern-matching on `SlotState` is rare and lives in
lifecycle code only** (`chunk/load.rs`, `chunk/unload.rs`,
`chunk/transition.rs`, `snapshot/restore.rs`, and a few internal
accessor implementations). Consumer code that reaches for
`let SlotState::Resident(rc) = &slot.state` is doing something
the accessor pattern should cover; check whether a new accessor
is the right answer before adding the match.

For lifecycle inspection ("is this coord still loading?",
"did this load fail?"), use `system.slot(coord) ->
Option<&ChunkSlot>` and pattern match on `slot.state`. That's the
inspection path; it's deliberately separate from the iteration
path so consumers can't accidentally treat a `Failed` or `Loading`
slot as a renderable one.

### 3.3 The registry

```rust
pub struct Registry {
    meshes: KindTable<MeshSerial, MeshEntry>,
    audio: KindTable<AudioSerial, AudioEntry>,
    animations: KindTable<AnimationSerial, AnimationEntry>,
    voice: KindTable<VoiceSerial, VoiceEntry>,
    light_states: KindTable<LightSerial, LightEntry>,
}

pub struct KindTable<S, E> {
    next_serial: S,
    by_serial: Vec<Option<E>>,                // sparse; index by serial
    by_slug: HashMap<Slug, S>,
}

pub struct MeshEntry {
    pub slug: Slug,
    pub primitive: Option<ShapePrimitive>,    // Some for procedural placeholders
    pub source_path: Option<PathBuf>,         // Some when shipped (GLTF, etc.)
    pub usage: Vec<UsageSite>,                // populated by scan
    pub status: AssetStatus,                  // Placeholder | Shipped
}

pub enum UsageSite {
    PrefabState { prefab: PrefabId, state: String },
    PrefabShape { prefab: PrefabId, state: String, shape_index: u32 },
    ChunkRegion { scene: SceneId, coord: ChunkCoord, region_name: String },
    // ...
}
```

Per-kind serial counters live in each `KindTable`. `MeshSerial`,
`AudioSerial`, etc. are newtype wrappers around `u32` so they're not
swappable. The asset IDs from `wok-scene` (`MeshId`, etc.) contain
the raw `u32`; the registry's tables are keyed by the typed wrappers
internally.

**Public API:**

```rust
impl Registry {
    pub fn empty() -> Self;
    pub fn load(path: &Path) -> Result<Self, LoadError>;
    pub fn save(&self, path: &Path) -> Result<(), SaveError>;

    // Lookup
    pub fn mesh(&self, id: MeshId) -> Option<&MeshEntry>;
    pub fn audio(&self, id: AudioCueId) -> Option<&AudioEntry>;
    // ... per kind

    // Allocation
    pub fn register_mesh(&mut self, slug: Slug, primitive: Option<ShapePrimitive>) -> Result<MeshId, RegistryError>;
    pub fn register_audio(&mut self, slug: Slug, source: PathBuf) -> Result<AudioCueId, RegistryError>;
    // ... per kind

    // Mutation
    pub fn rename_mesh(&mut self, id: MeshId, new_slug: Slug) -> Result<(), RegistryError>;
    pub fn set_mesh_source(&mut self, id: MeshId, source: PathBuf) -> Result<(), RegistryError>; // upgrade from placeholder

    // Auto-population (called after scene/prefab load)
    pub fn populate_from_scene(&mut self, scene: &Scene, prefabs: &HashMap<PrefabId, Prefab>, chunks: &HashMap<ChunkCoord, Chunk>);

    // Snapshot for worker
    pub fn read_view(&self) -> Arc<RegistryReadView>;
}
```

`RegistryReadView` is an immutable cheap-to-clone snapshot of the
serial→path map (enough for the worker to find files). When the
main thread mutates the registry, it produces a new
`Arc<RegistryReadView>` and uses it for subsequent requests.
In-flight worker requests keep their own old `Arc` and see the
old map. This is the lock-free reader pattern.

### 3.4 Storage

```rust
pub struct MeshCpu {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub bounding_aabb: Aabb,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub _pad: f32,
}

pub struct MeshGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub bounding_aabb: Aabb,
}
```

`MeshVertex` is the engine's one and only vertex format for Phase 4.
Cel shading wants position, normal, and a vertex color (since
shipped textures haven't arrived). The `_pad` is for 16-byte
alignment. Lock this in early — adding fields later is expensive.

### 3.5 Errors

```rust
pub enum LoadError {
    Scene(wok_scene::LoadError),
    Slice(wok_scene::SliceError),
    Registry(RegistryError),
    PrefabMissing(PrefabId),
    AssetMissing { kind: AssetKind, slug: Slug, serial: u32 },
    Gpu(String),
    WorkerGone,
    Io(io::Error),
}

pub enum RegistryError {
    SlugCollision { slug: Slug, existing: u32 },
    UnknownSerial { kind: AssetKind, serial: u32 },
    InvalidRename(String),
}

pub enum SnapshotError {
    UnsupportedVersion(u32),
    SchemaMismatch { snapshot: u64, expected: u64 },
    SceneMismatch { snapshot: SceneId, loaded: Option<SceneId> },
    ChunkLoadFailed { coord: ChunkCoord, error: Box<LoadError> },
    Parse(String),
}

pub enum TransitionError {
    UnknownSlot(ChunkCoord),
    NotResident { coord: ChunkCoord, state: SlotState },
}
```

`LoadError` is broad because it surfaces upstream from many phases.
The pipeline propagates with `?` and the variant tells the game (or
the editor) where the failure happened.

`TransitionError::NotResident` exists because eagerness lives on
`ChunkRuntime`, which only exists once the slot is Resident.
Transitioning a Pending or Loading slot is undefined — the game
must wait for `ContentEvent::ChunkResident` before calling
`transition_chunk`.

### 3.6 Events

```rust
pub enum ContentEvent {
    ChunkResident(ChunkCoord),
    ChunkUnloaded(ChunkCoord),
    ChunkFailed { coord: ChunkCoord, error: LoadError },
    ChunkTransitioned { coord: ChunkCoord, from: ChunkEagerness, to: ChunkEagerness },
    SceneLoaded(SceneId),
    SceneUnloaded(SceneId),
    HotReload(HotReloadKind),  // when wok-scene's watcher signals
}
```

The game polls `system.poll()` once per tick and drains these. They
are emitted on the main thread in response to worker results being
processed.

---

## 4. Data Formats

### 4.1 The registry file

Authored on disk alongside scenes and prefabs. Maintained by the
editor; treated as authored data, included in version control,
hand-editable in extremis.

Path: `<content_root>/registry.json`.

```json
{
  "_format": 1,
  "meshes": {
    "next_serial": 268,
    "entries": [
      { "serial": 0,   "slug": "primitive-cube",     "status": "placeholder", "primitive": { "kind": "cube",   "half_extents": [0.5, 0.5, 0.5] } },
      { "serial": 1,   "slug": "primitive-capsule",  "status": "placeholder", "primitive": { "kind": "capsule", "radius": 0.4, "half_height": 0.9 } },
      { "serial": 267, "slug": "wooden-crate-mesh",  "status": "shipped",     "source": "meshes/wooden-crate.gltf" }
    ]
  },
  "audio":      { "next_serial": 0, "entries": [] },
  "animations": { "next_serial": 0, "entries": [] },
  "voice":      { "next_serial": 0, "entries": [] },
  "light_states": { "next_serial": 8, "entries": [/* ... */] }
}
```

**Usage** lists are *not* on disk — they're derived from scanning
scenes and prefabs. Persisting them is redundant work that desyncs
the moment someone deletes a prefab without rebuilding the
registry. Population runs on every `load_scene`. Cost is bounded
(linear scan of scene contents); cheap enough to not cache.

The registry's `entries` array is sorted by serial on save. Slots
removed by deletion are emitted as
`{ "serial": N, "slug": "_deleted_N", "status": "deleted" }` to
preserve serial uniqueness. Tombstones (rather than array gaps
or explicit `null` entries) are the chosen form: they're self-
describing on disk, tolerable to hand-edit, and survive being
read by tools that don't know about sparse-array conventions.

### 4.2 The engine snapshot file

One engine snapshot = wok-content's view of the world at one tick.
Used for save/load and (per the multiplayer model)
join-in-progress / anchor resync. The engine snapshot is one
stream of the game's composite save file; the others (physics,
game state) live alongside but are not the engine's concern.

```json
{
  "_format": 1,
  "engine_version": "0.2.0",
  "stitch_id": "6f4a2c1e-8b3d-4f2a-9e1b-7c5a8d3f6e0a",
  "schema_hash": "8a3c7e9d12b4f5a6",
  "scene_id": "act1-warehouse",
  "tick": 84120,
  "camera": { "pos": [10.5, 1.8, 8.0], "yaw": 0.42, "pitch": -0.15 },
  "chunks": [
    { "coord": [0, 0], "eagerness": "eager" },
    { "coord": [1, 0], "eagerness": "vista" }
  ]
}
```

**What the engine snapshot carries:**

- `_format` — engine snapshot format version (loader rejects
  unknown).
- `engine_version` — informational; the engine build that produced
  this snapshot.
- `stitch_id` — opaque token provided by the game at capture and
  written through verbatim. The engine never inspects it; it
  exists so the game can verify that engine, physics, and game
  snapshots loaded from disk belong together. Typically a UUIDv4.
- `schema_hash` — compile-time content-hash of the
  `EngineSnapshot` struct shape. Catches schema drift between
  engine builds independently of `_format`. Mismatch →
  `SchemaMismatch` error.
- `scene_id` — which scene was loaded. Mismatch on restore →
  `SceneMismatch`.
- `tick` — engine tick counter; restored as-is for deterministic
  replay continuation.
- `camera` — pose of the active camera. wok-physics's `Camera`
  serde produces this; wok-content writes the result.
- `chunks` — coord + current runtime eagerness for each Resident
  slot at capture time.

**What the engine snapshot does NOT carry** (refresher from §5.3):

- Actor pool — wok-physics's snapshot, separate stream.
- Game state (inventory, quest flags, sidecar data) — game's
  snapshot, separate stream.
- Registry — deterministic from `content/`.
- Asset storage (GPU buffers, etc.) — reconstructable.
- Trigger event history — game tracks its own.

### 4.3 Snapshot format versioning

Three layers of mismatch detection, ordered by check strictness:

- `_format` — bumped on any deliberate breaking change. Old
  snapshots are rejected with `UnsupportedVersion`. Migration is
  a one-shot tool that runs outside the engine.
- `schema_hash` — caught by the loader before fields are read.
  Different builds with different struct shapes produce different
  hashes even when humans forget to bump `_format`. Mismatch →
  `SchemaMismatch`. This is the belt-and-suspenders layer.
- `stitch_id` — game-enforced, not engine-enforced. Verifies that
  engine, physics, and game snapshots loaded from disk are from
  the same capture moment.

`engine_version` is informational only — never enforced. Useful
for debug logs when crossing major version boundaries.

### 4.4 The smoke-test directory

For the in-tree smoke-test (`wok/examples/smoke-test/`):

```
wok/examples/smoke-test/content/
├── registry.json
├── prefabs/
│   ├── player-capsule.json    # one capsule prefab, hitbox + visible
│   ├── room-floor.json        # plane prefab
│   ├── room-walls.json        # cube prefabs assembled into a room
│   ├── interactable-box.json  # one box, hitbox + visible
│   └── trigger-pad.json       # hitbox-only volume with trigger_id
└── scenes/
    └── smoke/
        ├── scene.json
        └── 0_0.json
```

Phase-4 smoke-test load path: `ContentSystem::load_scene("smoke")`
populates registry from this content, then `request_load((0, 0))`
brings the chunk resident with one player capsule, room geometry,
one interactable box, and one trigger pad. Game wires the trigger
pad's `TriggerId` to a print statement and calls it done.

---

## 5. Algorithms

This section covers four distinct concerns that the file layout
already separates: chunk load orchestration, the streaming
algorithm, the snapshot mechanism, and registry rename. Each is
small enough to specify here.

### 5.1 Chunk load orchestration

Single entry: `ContentSystem::request_load(coord)`. Returns a
`LoadHandle` that the game can poll if it cares (typically it
doesn't — it just waits for `ContentEvent::ChunkResident(coord)`
on the next `poll()`).

```text
request_load(coord):
    if slots.contains(coord) and state is Resident or Loading:
        return existing handle      // idempotent
    let scene = self.scene.as_ref().expect("scene loaded")
    slots.insert(coord, ChunkSlot { state: Pending, ... })
    worker.send(WorkerRequest::LoadChunk {
        coord,
        chunk:    Arc::clone(&scene.chunks_authored[&coord]),  // cheap refcount bump
        prefabs:  Arc::clone(&scene.prefabs),                  // resident set, Arc'd
        registry: registry.read_view(),                        // Arc<RegistryReadView>
    })
    return handle
```

The worker doesn't read from disk for the chunk authored data —
that was already loaded by `load_scene` (per §5.2). The worker
DOES still do the slice + GPU upload work off the main thread.

```text
LoadChunk:
    runtime: ChunkRuntime = wok_scene::slice_chunk(&chunk, &*prefabs)?
    gpu_uploads: Vec<MeshUploadJob> = identify_required_meshes(&runtime, &registry)
    for job in gpu_uploads:
        if mesh already uploaded (per shared atomic registry of uploaded):
            skip
        else:
            cpu_data = generate_or_load(job.id, &registry)
            upload to wgpu::Queue
            record handle
    send WorkerResult::ChunkLoaded { coord, runtime, gpu_handles, newly_uploaded_meshes }
```

Main thread integrates the result:

```text
on ChunkLoaded result:
    if !slots.contains(coord) or matches!(slot.state, SlotState::Unloading(_)):
        // game called unload between request and completion;
        // release the GPU handles, drop runtime, no event
        return
    for mesh_handle in newly_uploaded_meshes:
        storage.meshes.insert(mesh_handle.id, mesh_handle.gpu)
    slot.state = SlotState::Resident(ResidentChunk {
        runtime,
        gpu: ChunkGpuHandles { visible_mesh_refs: ... },
    })
    emit ChunkResident(coord)
```

Single assignment to `slot.state`; the type system guarantees
`runtime` and `gpu` ship together or not at all.

**Three failure modes** worth calling out:

- **Cancellation between dispatch and completion** — the worker
  result arrives after the game has already called
  `request_unload`. The main thread sees
  `matches!(slot.state, SlotState::Unloading(_))`, discards the
  result, releases buffers. No event.
- **Asset missing during load** — the worker can't find a
  mesh-by-serial in its `RegistryReadView`. Returns
  `WorkerResult::ChunkFailed { coord, error: AssetMissing }`.
  Main thread inserts `SlotState::Failed(error)`, emits
  `ChunkFailed`. Game decides whether to retry (after maybe
  fixing the registry) or give up.
- **Worker thread panic** — the worker is wrapped in
  `std::panic::catch_unwind`. A panic becomes
  `LoadError::WorkerPanic(String)`, the worker is respawned, and
  the chunk's slot enters `Failed`. The crate ships with
  documentation: workers should not panic; if they do, that's a
  bug.

**Mesh-upload deduplication.** Multiple chunks may reference the
same `MeshId`. The worker needs to avoid uploading twice. Cleanest
mechanism: an `Arc<Mutex<HashSet<MeshId>>>` shared between worker
and main thread that tracks "which mesh serials have been uploaded
or are in-flight." Workers check-and-insert under the lock before
upload. Hold time is microseconds.

This is the only shared-mutable-state in the threading model. It's
small, has a single purpose, and contention is essentially zero
(multi-second load operations between sub-microsecond critical
sections). Accept it.

### 5.2 The streaming algorithm

**Scene-boot preconditions.** Streaming reads per-chunk metadata
(eagerness, neighbors, interlocks) on every tick to decide what
to load. That metadata lives on `Chunk.metadata`, which is only
available after the chunk file is loaded from disk. The streaming
algorithm therefore cannot drive its own chunk discovery —
something else has to ensure the metadata is in memory before
streaming runs.

**`load_scene` eagerly loads every chunk's authored data** at
scene boot. `LoadedScene.chunks_authored` is a complete
`HashMap<ChunkCoord, Chunk>` for every coord in the manifest. This
keeps wok-scene's "manifest holds only coords; chunk files are
the source of truth for metadata" rule intact (the alternative
— duplicating metadata into the manifest — would create a
manifest-vs-chunk drift bug the first time someone authored a
chunk as Vista but forgot to update the manifest's mirror).

The cost: at scene boot, every chunk file is read and parsed,
even chunks the player will never approach. For discrete-level
scenes (R&C, BFBB, Sly-style — bounded in the low hundreds of
chunks, each a small JSON file with placements), this is on the
order of low single-digit megabytes and tens of milliseconds —
free, effectively. See §9.14 for the scale boundary this
assumes.

A chunk being "authored-loaded" is unrelated to its slot
lifecycle. Authored data sits in `LoadedScene.chunks_authored`
for the duration of the scene; slot state (`Pending` / `Loading`
/ `Resident` / `Unloading` / `Failed`) tracks the GPU-uploaded,
sliced form that streaming actually manages.

Inputs each tick:

- Camera world position `cam: Vec3`.
- The loaded scene's `default_load_radius_meters` (call it `R`).
- The current `slots` map.
- Per-chunk metadata from `LoadedScene.chunks_authored[coord]
  .metadata` (eagerness, neighbors, interlocks). Always in memory
  per the precondition above.
- A history map `last_seen_close: HashMap<ChunkCoord, u64>` —
  most-recent tick at which the chunk was within its unload
  radius.

Constants:

- `R_in`              = R              (Eager load radius)
- `vista_multiplier`  = 1.5            (Vista's load radius is R_in * this)
- `hysteresis_factor` = 1.25           (unload radius = load radius * this)
- `K`                 = 60             (≈1 second of hysteresis at 60 Hz)
- `MAX_LOADED`        = 32             (engine constant from high-level design)

Per-chunk:

```text
load_radius(meta):
    match meta.eagerness:
        Eager => R_in
        Vista => R_in * vista_multiplier
        Lazy  => N/A — game owns the lifecycle

unload_radius(meta):
    load_radius(meta) * hysteresis_factor
```

Algorithm:

```text
tick_streaming(cam):
    let desired = compute_desired_set(cam)
    let to_load = desired - currently_loaded
    let to_unload = compute_unloads(cam)

    // priority queue: closest desired chunks load first
    sort to_load by distance(chunk_center, cam) ascending
    enqueue all to_load on worker

    for coord in to_unload:
        request_unload(coord)

compute_desired_set(cam):
    let result = HashSet::new()
    for (coord, chunk) in &scene.chunks_authored:
        let meta = &chunk.metadata
        if meta.eagerness == Lazy: continue       // game-only
        let center = coord.to_world_offset() + chunk_center_offset
        if distance(center, cam) <= load_radius(meta):
            result.insert(*coord)
    // Lazy chunks: only added by explicit game request, tracked separately
    result |= explicit_lazy_requests
    // Interlocks: pull in everyone's partners (fixpoint)
    let mut closed = result.clone()
    loop until fixpoint:
        for coord in closed:
            for partner in &scene.chunks_authored[&coord].metadata.interlocks:
                closed.insert(*partner)
    closed

compute_unloads(cam):
    let mut out = vec![]
    for (coord, slot) in &slots:
        if !matches!(slot.state, SlotState::Resident(_)): continue
        let meta = &scene.chunks_authored[coord].metadata
        // Lazy chunks: game owns lifecycle; never auto-unload
        if meta.eagerness == Lazy: continue
        let center = coord.to_world_offset() + chunk_center_offset
        let dist = distance(center, cam)
        if dist > unload_radius(meta):
            last_seen_close.get_or_insert(*coord, NEVER)
            // unchanged
        else:
            last_seen_close.insert(*coord, current_tick)
        // unload if been out of range for K ticks
        if current_tick - last_seen_close[coord] > K:
            // but not if it's part of an interlock with an in-range chunk
            if !interlocked_with_desired(*coord):
                out.push(*coord)
    // enforce MAX_LOADED cap: if currently_loaded + to_load > MAX, evict
    // the farthest non-desired Resident chunk first
    out
```

**Vista — authored eagerness vs runtime state.** Both apply, and
they are separate concerns:

- **Authored eagerness** is read from
  `scene.chunks_authored[&coord].metadata.eagerness` (a wok-scene
  field, deserialized from the chunk file at scene boot). It
  determines *loading behavior*: Eager loads inside `R_in`;
  Vista loads inside `R_in * vista_multiplier`; Lazy never
  auto-loads.
- **Runtime eagerness** is the `ChunkRuntime.eagerness` tag on a
  resident slot. It determines *what happens once loaded*
  (physics skips Vista; render does not; etc.). The game mutates
  it via `transition_chunk`.

The two start equal at load time (the slicer copies authored
eagerness into `ChunkRuntime.eagerness`) and may diverge if the
game transitions a loaded chunk. **They are not merged into a
single state with an origin tag.** The engine reports both;
consumers query whichever they need; the game tracks its own
intent if it cares about the distinction. This follows the same
"primitives, not features" principle as §1.3 — engine reports
state, game makes policy.

Concrete consequences of the collapsed model:

- **Unload-reload reverts to authored**: an authored-Eager chunk
  that the game transitioned to Vista, then unloaded, comes back
  as Eager on next load. Runtime transitions are not persistent
  across the slot's lifetime. (Whole-world snapshot is the
  exception — it does persist runtime eagerness, per §5.3.)
- **Streaming radius follows authored, not runtime**:
  authored-Eager chunks always use `R_in`; authored-Vista chunks
  always use `R_in * vista_multiplier`. A runtime transition does
  not change which radius streaming applies to the chunk. If a
  game needs an Eager-authored-but-currently-Vista chunk to stay
  loaded outside `R_in`, that's a Lazy-style explicit hold by
  the game, not a streaming-algorithm concern.
- **No origin field**: the engine never stores "this Vista state
  came from the file" vs "this Vista state came from
  `transition_chunk`." Consumers that care (debug overlays, the
  editor, game logic) reconstruct the comparison by reading
  `system.authored_eagerness(coord)` against the runtime tag
  from `system.slots()` (e.g.,
  `system.slots().find(|(c, _)| *c == coord).map(|(_, rc)| rc.runtime.eagerness)`).

`vista_multiplier` is a `ContentConfig` field (§9.9), default
1.5, overridable by the game. Not a hardcoded constant.

### 5.3 Snapshot mechanism

Capture is synchronous, main-thread, scene-quiescent. The engine
produces only its own state — chunks, eagerness, camera, tick. No
actor data, no game data, no physics bytes wrapped opaquely. The
game stitches its own composite save file from three streams: the
engine snapshot here, a physics snapshot from
`wok_physics::ActorPool::capture(...)`, and the game's own state.

```text
capture_engine_snapshot(stitch_id: StitchId) -> EngineSnapshot:
    return EngineSnapshot {
        format: 1,
        engine_version,
        stitch_id,                        // game-provided; engine writes-and-returns
        schema_hash,                      // see below
        scene_id: scene.manifest.id,
        tick: current_tick,
        camera: serialize(camera),        // wok-physics's Camera primitive serde
        chunks: slots.iter()
            .filter_map(|(coord, slot)| match &slot.state {
                SlotState::Resident(rc) => Some(ChunkSnap {
                    coord: *coord,
                    eagerness: rc.runtime.eagerness,
                }),
                // Unloading is excluded — caller is in the process
                // of tearing down; don't capture chunks on their way out.
                _ => None,
            })
            .sorted_by_coord()
            .collect(),
    }
```

`stitch_id` is the game's compatibility primitive (§4.2). The
engine accepts it on capture, writes it into the header, returns
it unchanged on restore. The engine does *not* check it — that's
the game's job at composition time. Engine doesn't enforce because
engine doesn't know what the other two snapshots' stitch_ids are.

`schema_hash` is a content-hash of the `EngineSnapshot` struct
shape — computed at compile time from the type definition.
Catches "this engine snapshot was produced by a different engine
build with a different schema" without depending on humans
remembering to bump `_format`. If two snapshots have the same
hash, they are guaranteed parseable by the same code. If they
differ, mismatch is certain. The hash is computed by a build-time
derive macro on `EngineSnapshot`; the implementation is small
enough to defer to a later phase if Phase A doesn't need it.

What the engine snapshot does **not** carry:

- **Actor pool data.** wok-physics owns its own snapshot format.
  The game captures it separately and composes.
- **Trigger event history.** Game state; not engine state.
- **Game's own state** (inventory, quest flags, persistent NPC
  state). Game owns this.
- **Sidecar state** (dropped items, defeated enemies). Game-
  managed per the engine's "primitives, not features" rule.
- **Registry.** Deterministic from `content/`; not snapshotted.
- **Asset storage** (GPU buffers, mesh CPU data). Reconstructable
  from registry + authored data.

Restore is the inverse, synchronous, blocking:

```text
restore_engine_snapshot(snap: &EngineSnapshot)
    -> Result<StitchId, SnapshotError>:

    if snap.format != current EngineSnapshot format:
        return Err(UnsupportedVersion)
    if snap.schema_hash != EngineSnapshot::SCHEMA_HASH:
        return Err(SchemaMismatch)
    if snap.scene_id != scene.manifest.id:
        return Err(SceneMismatch)

    for chunk_snap in snap.chunks:
        if !slots.contains(chunk_snap.coord):
            request_load(chunk_snap.coord)
    // pump worker until all loads complete
    pump_until_resident(snap.chunks.iter().map(|c| c.coord))

    for chunk_snap in snap.chunks:
        match &mut slots[chunk_snap.coord].state:
            SlotState::Resident(rc) => rc.runtime.eagerness = chunk_snap.eagerness,
            _ => unreachable!("pump_until_resident guarantees Resident"),

    apply camera from snap.camera
    set tick to snap.tick

    Ok(snap.stitch_id)                    // game compares against its other streams
```

Ordering matters: load all chunks first, *then* apply eagerness
overrides. The structural invariant (eagerness only exists on
`SlotState::Resident`) means writing to eagerness before
`pump_until_resident` returns would either match `_` and panic at
the `unreachable!`, or — better — wouldn't even compile if the
two-phase ordering were violated by accident in a refactor.

The game's compose-side flow is then:

```rust
// game side, save
let stitch_id = StitchId::new_v4();             // game owns generation
let game_save = GameSaveFile {
    engine:  content.capture_engine_snapshot(stitch_id),
    physics: physics.capture(stitch_id),
    game:    self.serialize_self(stitch_id),
};
write_to_disk(game_save);

// game side, load
let game_save = read_from_disk();
if game_save.engine.stitch_id != game_save.physics.stitch_id
    || game_save.engine.stitch_id != game_save.game.stitch_id {
    return Err(IncompatibleStreams);
}
content.restore_engine_snapshot(&game_save.engine)?;
physics.restore(&game_save.physics)?;
self.deserialize_self(&game_save.game)?;
```

Three streams, one stitch_id, game enforces matching. Engine's
contract is just "write what you got, return what you wrote."

**Sync vs async restore.** Phase A ships blocking
`restore_engine_snapshot` (smoke-test needs only save/load, no
loading screen). Async restore that returns a `RestoreHandle` the
game polls is required for multiplayer join-in-progress (per the
multiplayer model doc — anchor-event resync can't freeze the main
loop). It lands in the multiplayer phase; the API shape will be
parallel to the blocking version.

### 5.4 Registry rename

Atomic on the main thread. No background-worker interaction needed
because the worker uses an `Arc<RegistryReadView>` that already
captured the pre-rename state.

```text
rename_mesh(id, new_slug):
    let entry = registry.meshes.by_serial.get(id.serial())?
    if let Some(existing) = registry.meshes.by_slug.get(&new_slug):
        if existing != id.serial():
            return Err(SlugCollision)
    let old_slug = entry.slug.clone()
    registry.meshes.by_slug.remove(&old_slug)
    registry.meshes.by_slug.insert(new_slug.clone(), id.serial())
    entry.slug = new_slug
    // bump the read view so subsequent worker requests see the change
    registry.read_view = Arc::new(RegistryReadView::from(&registry))
    Ok(())
```

**Atomicity**: the rename is three statements on the main thread;
no other code observes the intermediate state. In-flight worker
requests reference the *old* `Arc<RegistryReadView>`, which is
unchanged.

**Stale-slug-in-file**: an authored chunk file might reference
`old-slug-42` after a rename to `new-slug-42`. The parser
extracts serial=42 and the lookup-by-serial succeeds. The slug
part is debug surface only (wok-scene rule). The next save of
that file re-emits with the current slug.

**Slug collision**: the only error condition. Trivial check.

**Failure during rename**: there isn't a failure mode after the
collision check passes — three HashMap updates can't fail. The
worst case is OOM in the read-view `Arc` construction, which
panics with the rest of the world.

---

## 6. Threading Model

Two threads: **main** and **worker**. No others.

```text
                                         main
   ┌──────────────────────────────────────────────────────────────┐
   │  ContentSystem (registry, slots, storage, scene, prefabs)    │
   │                                                              │
   │  request_load ─┐                       ┌── ContentEvents     │
   │                │                       │   (game polls)      │
   │  tick_streaming│                       │                     │
   │                │                       │                     │
   └────────────────┼───────────────────────┼─────────────────────┘
                    │ WorkerRequest         │ WorkerResult
                    │ (priority queue)      │ (mpsc)
                    ▼                       │
   ┌──────────────────────────────────────────────────────────────┐
   │  Worker thread                                               │
   │    - file I/O (wok-scene::load_chunk, registry source files) │
   │    - slicing (wok-scene::slice_chunk)                        │
   │    - placeholder mesh generation (procedural primitives)     │
   │    - GPU upload (wgpu::Queue::write_buffer or create_buffer) │
   │                                                              │
   │  Owns: nothing engine-persistent. Each request is closed.    │
   └──────────────────────────────────────────────────────────────┘

   Shared (atomic, lock minimally):
     - Arc<Mutex<HashSet<MeshId>>>  uploaded-or-in-flight dedup
     - Arc<wgpu::Device>, Arc<wgpu::Queue>
```

### 6.1 What runs where

Main thread:

- Polls input, runs game logic, ticks simulation.
- Calls `ContentSystem` methods (mutating).
- Drains `WorkerResult` queue via `poll()`.
- Mutates the registry, slots, storage.
- Renders (handing wok-render the `&ChunkSlot` references).

Worker thread:

- Reads JSON files from disk.
- Parses them (`serde_json`).
- Calls `wok_scene::slice_chunk` (pure, no I/O).
- Generates procedural primitive meshes (CPU side).
- Uploads to GPU via `wgpu::Queue::write_buffer` (wgpu is `Sync`
  and supports this).
- Sends results back.

Worker never:

- Mutates the registry.
- Touches any `&mut` data the main thread owns.
- Calls back into `ContentSystem`.

### 6.2 Channels

Two queues, both `crossbeam_channel`. The dep is justified —
`std::sync::mpsc` doesn't support select or bounded backpressure
cleanly, and the priority-feed pattern below depends on
crossbeam's ability to compose with a main-side `BinaryHeap`.
Crossbeam is a long-standing, widely-used crate; its API surface
for what we use (bounded `Sender<T>`, unbounded `Receiver<T>`,
`select!`) is stable and minimal.

- **Request queue**: bounded, priority-ordered. Higher priority =
  smaller distance to camera. Producer is main thread; consumer
  is worker. Bounded to prevent queue blowup if a game spams
  requests; main thread blocks on `send` (which is fine — it
  means the worker is saturated and we shouldn't pile on).
- **Result queue**: unbounded mpsc. Producer is worker; consumer
  is main thread.

A priority queue in front of the request channel lives main-side.
The worker just pulls the next request. The main thread sorts on
insertion (or uses a `BinaryHeap`).

### 6.3 Why one worker, not a pool

The high-level design specifies one worker thread. Reasons that
make this the right call:

- Chunk loads aren't parallel-friendly. Disk I/O is fine in
  parallel, but `wgpu::Queue::write_buffer` is serialized at the
  driver level anyway. Multiple workers contending on `Queue`
  buys nothing.
- Determinism is easier with one worker. Order of completion is
  predictable.
- Memory bound: 32 chunks max, each <a few MB, so total content
  budget is small. We're not waiting on tens of gigabytes of
  geometry. Loads will be sub-second.
- One worker keeps the threading model trivially explainable.

If profiling later reveals contention, the worker can `rayon`-fan
out within a single request (e.g., parallel mesh generation
across primitives in one chunk). The main-thread / worker
boundary stays single.

### 6.4 Hot reload across the boundary

The wok-scene file watcher's events surface in
`ContentSystem::poll()`'s output as `HotReload(...)` events. Main
thread decides what to do — typically requesting a chunk reload,
which proceeds through the normal path.

The watcher itself runs on its own debounce thread (owned by
wok-scene, not us). We poll it from the main thread in `poll()`.
This means hot reload has a one-frame latency from "file changes"
to "game sees event," which is fine.

### 6.5 Shutdown

```rust
impl ContentSystem {
    pub fn shutdown(mut self) {
        // signal worker
        self.worker.signal_shutdown();
        // drain remaining results (may include in-flight)
        while let Ok(_) = self.worker.try_recv_result() {}
        // join
        self.worker.join();
        // drop GPU buffers and CPU storage explicitly so they
        // release before wgpu::Device drops
    }
}
```

Drop is not enough — we have to join the worker thread before
the `Arc<wgpu::Device>` count hits zero, otherwise device cleanup
can race with worker uploads. Explicit shutdown call required.

---

## 7. Test Plan

### 7.1 Registry (`tests/registry.rs`)

1. Empty registry → save → load → equal.
2. Register mesh → returned `MeshId` resolves via `mesh()`.
3. Register two meshes with same slug → second errors with
   `SlugCollision`.
4. Rename mesh → `mesh(old_id)` still resolves (serial-based) →
   `mesh(new_id_with_new_slug)` also resolves.
5. Rename to a slug already in use by a different serial →
   `SlugCollision`.
6. Populate from scene → expected `MeshId`s present, with
   `UsageSite` lists populated correctly.
7. Re-populate from same scene → no duplicate entries.
8. Populate from scene that references unknown mesh slug → new
   entry created as placeholder.
9. `read_view()` after a rename returns a snapshot containing the
   new slug; pre-rename `read_view()` still references the old.
10. Tombstone of deleted entry → round-trip preserves the
    tombstone slot.

### 7.2 Primitive generation (`tests/primitives.rs`)

1. Each primitive (`Cube`, `Ellipsoid`, `Cylinder`, `Capsule`,
   `Plane`) generates a `MeshCpu` with non-empty vertices and
   triangle-count > 0.
2. Determinism: same `ShapePrimitive` → byte-identical `MeshCpu`.
3. Vertex normals point outward (sample a few vertices, dot with
   position − center > 0).
4. Bounding AABB encloses all vertices.
5. Tessellation parameters from `ContentConfig` honored (e.g.,
   ellipsoid subdivision = 16 → expected vertex count).

GPU upload tested separately with a headless wgpu adapter — needs
a `tests/common.rs` helper that boots one.

### 7.3 Chunk lifecycle (`tests/chunk_lifecycle.rs`)

Tests use a synchronous mode of the worker (a `LoopbackWorker`
that drains requests and produces results on the same thread).
This is the same trick that wok-scene's watcher tests use to keep
things deterministic. Async-worker behavior tested in §7.5.

1. `request_load(coord)` for a known chunk → slot enters Pending
   → after one `tick`, Resident.
2. Same coord loaded twice → idempotent (second call returns same
   handle, no second worker dispatch).
3. `request_unload(coord)` while Pending → cancellation; slot
   removed; no event.
4. `request_unload(coord)` while Loading → state goes to
   Unloading; result arrives → handles released; no event.
5. `request_unload(coord)` while Resident → state goes to
   Unloading → after tick, slot removed → event.
6. Load chunk referencing missing prefab → `ChunkFailed` with
   `PrefabMissing`.
7. Load chunk referencing missing mesh serial → `ChunkFailed`
   with `AssetMissing`.
8. Two chunks share a `MeshId` → upload deduplicated (only one
   GPU buffer in storage).
9. Eviction: load `MAX_LOADED + 1` chunks → least-recently-touched
   non-desired Resident chunk evicted before new one enters.
10. **Snapshot of a Vista chunk** — matching the slot's state
    against `SlotState::Resident(rc)` yields
    `rc.runtime.eagerness == Vista`; `rc.runtime` is byte-identical
    to what it would be at Eager (per slicer neutrality).

### 7.4 Vista enforcement (`tests/vista.rs`)

These are the behavioral tests that wok-scene's plan §8 explicitly
defers to wok-content:

1. `transition_chunk(coord, Vista)` on an Eager Resident slot →
   pattern-matching the slot's state yields
   `SlotState::Resident(rc)` with `rc.runtime.eagerness == Vista`;
   the visible / hitbox / trigger / region arrays inside
   `rc.runtime` are byte-identical to before the transition (only
   the eagerness tag changed).
2. No I/O during transition (use a `LoopbackWorker` and assert no
   requests sent).
3. `active_slots()` excludes Vista; `vista_slots()` includes only
   Vista; `slots()` includes both. (These iterators are the
   interface other crates use to enforce their Vista semantics.)
4. Round-trip: Eager → Vista → Eager. After the second
   transition, the runtime arrays AND the eagerness tag are
   byte-identical to the pre-Vista state.
5. Authored Vista chunk loads with `rc.runtime.eagerness == Vista`
   (no transition needed).
6. Trigger volumes on a Vista chunk are present in `slots()` and
   `vista_slots()` iteration; absent from `active_slots()`. This
   is the contract: wok-content carries data faithfully; consumers
   honor eagerness by choosing the right iterator. wok-physics's
   overlap test iterates `active_slots()`, so Vista trigger
   volumes never reach the overlap math.
7. `transition_chunk` on a Pending or Loading slot →
   `TransitionError::NotResident`. No state change.
8. **Authored eagerness vs runtime eagerness divergence**:
   authored-Eager chunk transitioned to Vista at runtime →
   `system.authored_eagerness(coord) == Some(Eager)` while
   iteration via `system.slots()` yields the chunk with
   `rc.runtime.eagerness == Vista`. Unload and reload → both
   equal Eager again (runtime transition was not persistent).
9. Same chunk, but inside a `capture_engine_snapshot` →
   `restore_engine_snapshot` round-trip → runtime eagerness
   restored to Vista. (This is the snapshot-vs-unload-reload
   distinction made concrete.)

### 7.5 Streaming (`tests/streaming.rs`)

Use a hand-built scene with known chunk positions. No actual file
I/O — feed a `Scene` and prefab map directly into the system.

1. Camera at chunk (0,0) origin, R_in = 200 → Eager chunks within
   200m load.
2. Camera moves outside an Eager chunk's load radius, but within
   its unload radius for K ticks → chunk stays loaded.
3. Camera outside an Eager chunk's unload radius for K+1 ticks →
   chunk unloads.
4. Hysteresis: camera jiggles around the R_in boundary → chunk
   does not churn (load/unload pair never both occur in <K ticks).
5. Lazy chunk in radius → not auto-loaded.
6. Lazy chunk explicitly requested → loaded; stays loaded even
   outside radius; never auto-unloaded by streaming (game owns
   the lifecycle).
7. Interlock: two chunks A and B linked → loading A forces B
   into desired set.
8. Interlock + hysteresis: A goes out of range but B is in range
   → A stays loaded (interlocked with a desired chunk).
9. MAX_LOADED = 32, 33rd chunk requested → farthest non-desired
   Resident chunk evicted.
10. Authored Vista chunk uses extended load radius (R_in *
    vista_multiplier); unload radius likewise extended.
11. **Runtime-Vista does not change streaming**: an authored-Eager
    chunk transitioned to runtime-Vista uses Eager's load and
    unload radii — it unloads at Eager's hysteresis boundary, not
    at Vista's extended boundary. (The streaming algorithm reads
    authored eagerness from
    `scene.chunks_authored[&coord].metadata.eagerness`, not from
    the slot's `ResidentChunk.runtime.eagerness`.)
12. `vista_multiplier = 1.0` in `ContentConfig` → Vista chunks
    behave identically to Eager for load/unload (the runtime
    state still differs; only the radius changes).

### 7.6 Snapshot (`tests/snapshot.rs`)

1. Capture → restore on the same loaded state → resident chunk
   set, eagerness tags, camera, and tick are byte-identical
   (sort before compare to absorb HashMap ordering).
2. Capture → unload one chunk → restore → engine re-loads the
   chunk and brings it Resident with the snapshot's eagerness.
3. Capture with a runtime-Vista chunk (transitioned from
   authored-Eager) → restore → chunk is Resident with
   `runtime.eagerness == Vista`. This is the snapshot's
   superpower vs. unload-reload: it persists the game's runtime
   transitions.
4. Restore against a different loaded scene → `SceneMismatch`.
5. Restore with `_format = 99` → `UnsupportedVersion`.
6. Restore with mutated `schema_hash` → `SchemaMismatch`. (Test
   constructs a snapshot then hand-edits the hash.)
7. **Stitch ID round-trip**: capture with stitch_id S → snapshot
   header contains S → restore returns Ok(S). Engine doesn't
   inspect or transform.
8. Two snapshots captured with the same scene but different
   stitch_ids are distinguishable by header alone (no body parse
   needed). This is the property the game's compose layer
   depends on.
9. Restore where a referenced chunk fails to load (e.g., missing
   prefab) → `ChunkLoadFailed { coord, error }` with the
   underlying error preserved.
10. Determinism: capture → restore → capture again → both
    snapshots are byte-identical (after sorting). Same-state
    snapshots from same engine build must hash identically; this
    is the property `schema_hash` rides on.

### 7.7 Worker integration (`tests/worker_integration.rs`)

These are the only tests that exercise the real worker thread.
Slower; gated under an integration tag.

1. End-to-end load of one chunk from on-disk content → Resident
   in <1 second.
2. Worker panic (inject via test-only request variant) → respawn
   → next request succeeds.
3. Shutdown during in-flight load → no data races, no Drop
   panic.
4. Heavy concurrent requests (load 16 chunks at once) → all
   complete; order respects priority.

### 7.8 Determinism (`tests/determinism.rs`)

The multiplayer-model commitment: same authored content + same
camera path = byte-identical observable state.

1. Load the same scene twice (with system rebuild between) →
   identical `Registry` (modulo HashMap ordering on save).
2. Same chunk loaded at two different `ChunkCoord` values (in two
   different scenes) → identical runtime arrays modulo
   `coord` field. (This is the wok-scene-plan §7 test #10
   property surfaced at the wok-content layer.)
3. Same scene, same scripted camera trajectory → identical chunk
   load/unload sequence.

---

## 8. Interactions with Other Crates (Forward References)

### 8.1 wok-physics

Consumes `ChunkSlot.runtime.hitboxes` for collision and integration.
Iterates `active_slots()` — wok-content's eagerness-aware accessor
returns only non-Vista resident slots, which is what wok-physics
needs both for collision skipping and for trigger overlap skipping.

Provides `actors_overlapping_volumes(volumes: &[TriggerVolume]) ->
Vec<(ActorId, TriggerId)>`. The game gathers volumes by iterating
`active_slots()` (Vista chunks excluded at the iteration step) and
passes them in. wok-physics does the math; the game routes the
returned pairs to its own event handlers. wok-physics never sees
wok-content's types directly — it takes a slice of `TriggerVolume`
(a wok-scene type) and returns physics-side identifiers.

Provides actor-pool snapshot serialization for whole-world
snapshots (open question §5.3 — game-stitching pattern).

### 8.2 wok-render

Consumes `ChunkSlot.runtime.visible` and `ChunkSlot.gpu` for
draw calls. Iterates `slots()` (all, including Vista — Vista
chunks render normally per the high-level design).

Asks `ContentSystem::mesh(id)` for the GPU buffer behind a
`MeshId` reference. Buffer is borrowed for the frame.

### 8.3 wok-anim

Holds animation pose data; consumes `AnimationId` from runtime
arrays. wok-content's registry tracks AnimationId entries as
metadata only (slug, serial, source path); wok-anim opens the
source path and loads its own pose data.

This is the per-domain-data pattern: wok-content tracks identity
for all kinds; only meshes (and eventually textures, audio) have
their actual data stored in wok-content's storage half.

### 8.4 wok-light

Provides lighting-state data; consumes `LightStateRef` from
`ChunkSlot.runtime` (the chunk's lighting state) and from
`RuntimeRegion` (per-region overrides). Same pattern as wok-anim:
registry tracks identity, wok-light owns the data.

The `wok-content` registry's `light_states` table is metadata
only (slug, serial, source-file path). wok-light's own loader
reads the actual animation curves from those paths.

### 8.5 wok-sequence

Consumes mostly identity (camera-path data, timeline data) which
it owns. Indirectly depends on wok-content for chunk lifecycle
(a sequence may need a chunk loaded before it can play).

### 8.6 The game

Owns the main loop. Composes wok-content's primitives:

- Calls `load_scene` at scene-transition time.
- Calls `tick_streaming` each frame with camera position.
- Calls `request_load` / `request_unload` for Lazy chunks.
- Calls `transition_chunk` to push a chunk into Vista state when
  streaming pressure demands.
- Drains `poll()`, handles events.
- Each tick, gathers trigger volumes from `active_slots()`, hands
  them to wok-physics's `actors_overlapping_volumes` query, and
  routes the returned `(actor_id, trigger_id)` pairs to its own
  game-event handlers.
- Owns the save/load UI and calls `capture_engine_snapshot` /
  `restore_engine_snapshot`.

---

## 9. Gotchas

### 9.1 Serial-based equality applies to keys

`HashMap<MeshId, MeshGpu>` works correctly because `MeshId`'s `Hash`
impl is serial-only. Two `MeshId` instances with the same serial
but different slugs hash and compare equal. This is what makes
rename safe at the registry level — the storage half doesn't
need an update.

Don't accidentally use a `HashMap<(u32, Slug), MeshGpu>` or anything
that includes slug as part of the key. The wok-scene-plan-§9 rule
"two MeshIds with same serial are equal" extends down here.

### 9.2 wgpu's `Queue::write_buffer` is `Sync`-safe but not free

Multiple workers calling `write_buffer` simultaneously is fine from
a correctness standpoint but doesn't speed up GPU upload — the
driver serializes. We chose one worker for this reason; don't
"optimize" later by spawning a pool unless profiling shows the
worker thread itself (not the GPU) is the bottleneck.

### 9.3 The registry read view must be re-`Arc`'d on every mutation

The registry's `read_view: Arc<RegistryReadView>` is the
lock-free reader pattern. When the main thread mutates the
registry, the existing `Arc` is left for in-flight workers and a
new one is built for future requests. Forgetting to rebuild it
means workers see stale data; forgetting to keep it `Arc<>`'d
means we accidentally start sharing mutable state.

A small wrapper pattern enforces this:

```rust
impl Registry {
    fn mutate<R>(&mut self, f: impl FnOnce(&mut RegistryInner) -> R) -> R {
        let r = f(&mut self.inner);
        self.view = Arc::new(RegistryReadView::from(&self.inner));
        r
    }
}
```

All public mutators go through `mutate`. The read view stays in
lockstep.

### 9.4 GPU buffer release timing

When a chunk unloads, its `MeshGpuRef`s drop. The underlying
`MeshGpu` does NOT drop — it might be referenced by other
chunks.

**Decision**: through Phase 4, meshes are immortal — uploaded
once at first use and never released. The placeholder mesh set
is small and constant (one `MeshGpu` per primitive shape ×
tessellation parameters from `ContentConfig`); memory pressure
is not a concern at this fidelity.

When shipped assets arrive (post-GLTF integration, when meshes
can be large and per-chunk), `MeshGpu` gains a refcount and is
released when count == 0. The public interface stays identical;
the storage layer changes. Pin the migration to the phase that
introduces the first non-immortal asset kind so the refcount
ships with a real test surface, not speculatively.

### 9.5 Hot reload races

When wok-scene's watcher reports `PrefabChanged` and the prefab
is referenced by an in-flight chunk load, the worker may produce
runtime arrays from the *old* prefab definition.

**Decision (Phase E)**: generation tokens. Each
`WorkerRequest::LoadChunk` carries a `Generation(u64)` snapshotted
from the prefab's current generation counter; the main thread
bumps the counter on every `PrefabChanged` event. The result's
generation is compared on receive; mismatch → discard the result,
re-dispatch with the new generation.

Stale-result was the alternative (check referenced prefabs at
receive time, discard if any changed). Generation tokens win
because the decision happens at receive, not at load-time —
which means a chunk that references three prefabs only needs one
comparison, not three lookups, and the main thread doesn't have
to consult the prefab map under any lock.

Phase 4 doesn't exercise this path (hot reload deferred to Phase
E). The decision is here so Phase E doesn't reopen it.

### 9.6 Snapshots reference scenes by ID, not content

A snapshot says `scene_id: "act1-warehouse"`. If the authored
scene has changed between snapshot capture and restore, the
restored slot eagerness values may correspond to chunks that no
longer exist or have different metadata. **The engine doesn't
detect this.**

Games that ship updates with content changes are responsible for
versioning their snapshots. The engine version stamp is
informational; the game version stamp is the game's problem.

### 9.7 `LoadHandle` is not a future

`LoadHandle` is a value the game can poll. It's not awaitable.
`wok-content` does not assume an async runtime. Games that want
async wrap this themselves; the engine is sync-with-polling.

### 9.8 Camera position is in world space

`tick_streaming(camera_world)` takes a *world-space* position.
The streaming algorithm internally computes chunk-center world
positions and compares. wok-content does not see chunk-local
data here; this is one of the few places where world coords
appear.

### 9.9 `ContentConfig` defaults are the engine's contract

```rust
pub struct ContentConfig {
    pub max_loaded_chunks: usize,            // default 32
    pub hysteresis_factor: f32,              // default 1.25
    pub hysteresis_ticks: u64,               // default 60
    pub vista_multiplier: f32,               // default 1.5
    pub ellipsoid_subdivisions: u32,         // default 16
    pub cylinder_segments: u32,              // default 24
    pub priority_queue_capacity: usize,      // default 64
}
```

Most callers use `ContentConfig::default()`. Knobs exist for
testing and unusual scenes. Defaults are the values smoke-test
exercises; deviating from them changes engine behavior in ways
test suites won't catch.

### 9.10 `_format` of the registry is independent of authored data

The registry is *its own* on-disk format. Bumping wok-scene's
authored `_format` doesn't bump the registry's. They evolve
independently.

### 9.11 Authored eagerness lives in the scene; runtime eagerness lives on the slot

These are two separate fields with two separate lifetimes. The
expected confusion: a reader sees `ChunkRuntime.eagerness` and
assumes it reflects what the chunk file says. It doesn't — it
reflects current runtime state, which may have been mutated by
`transition_chunk` after load.

To query what the chunk file said, use
`ContentSystem::authored_eagerness(coord)`. To query the current
runtime state, iterate via `system.slots()` and read
`rc.runtime.eagerness` from the yielded `&ResidentChunk` — or, if
you have a `ChunkSlot` reference for lifecycle inspection,
pattern-match `SlotState::Resident(rc) | SlotState::Unloading(rc)`.

A consequence the streaming code easily gets wrong: load/unload
decisions read **authored** eagerness, never runtime. The
algorithm cannot use the slot's runtime tag for radius decisions
because the runtime tag may diverge after a game transition, and
streaming behavior must remain deterministic against the
authored data. See §7.5 streaming test #11 — the test exists to
catch exactly this confusion in implementation.

### 9.12 No "origin" tag on Vista state

The engine does not record whether a Vista chunk became Vista
via authored eagerness or via `transition_chunk`. If you find
yourself wanting to add a `vista_origin: AuthoredOrRuntime` field
to `ChunkRuntime` or `ChunkSlot`, stop: the comparison can be
reconstructed by reading `authored_eagerness(coord)` against the
current runtime tag. Games that need to track their own
transition intent (so they know which chunks to transition back
to Eager) keep that in game state, not engine state.

### 9.13 Single-scene loading is a deliberate Phase-4 simplification

`ContentSystem` holds exactly one `Option<LoadedScene>` and one
flat `HashMap<ChunkCoord, ChunkSlot>`. Implications:

- **Cross-scene transitions hitch.** Going from scene A to scene
  B means `unload_scene()` then `load_scene(B)`, with a frame (or
  several, while chunks load) where no scene is resident.
  Renderers see an empty world; physics sees no hitboxes;
  triggers don't fire. Games handle this by gating the
  transition behind a loading screen, a cutscene (driven by
  wok-sequence), or a fade-to-black — i.e., R&C-style
  load-bordered transitions.
- **Multi-scene composition is unavailable.** Games cannot have
  two scenes loaded simultaneously (e.g., persistent home base
  alongside the current adventure scene). The whole-engine
  scene API is single-valued.

The constraint matches the engine's intended posture: discrete
levels connected by deliberate transitions, idiomatic to the
R&C-derived games this engine is shaped for. Designing for
multi-scene now would be speculative work against a future need
the first game (still unnamed) hasn't articulated.

**Migration cost, for future reference.** If this constraint
ever needs to be lifted, the changes are not localized:

- `ContentSystem.scene: Option<LoadedScene>` becomes a stack or
  registry (`Vec<LoadedScene>` or `HashMap<SceneId, LoadedScene>`).
- Chunk slot keying gains a `SceneId`:
  `HashMap<ChunkCoord, ChunkSlot>` becomes
  `HashMap<(SceneId, ChunkCoord), ChunkSlot>` or nested by scene.
- `EngineSnapshot.scene_id: SceneId` becomes
  `scenes: Vec<SceneSnap>`, with each `SceneSnap` carrying its
  own chunk list. Format bump required.
- Streaming algorithm sums desired sets across scenes; the
  `MAX_LOADED` cap becomes a shared budget; per-scene radii
  and origins need to be threaded through.

If a future game needs cross-scene atomic transitions
specifically (no-hitch door-to-door) without full multi-scene
support, a smaller intermediate option exists: a
`scene_swap(next_scene_dir)` operation that loads scene B's
first chunks before unloading scene A's last chunks, briefly
exceeding `MAX_LOADED`. That's a localized addition rather than
the full migration above. Note it; don't build it.

### 9.14 Eager-load-all-chunks is a discrete-level scale assumption

`load_scene` reads every chunk file in the scene at boot
(see §5.2). For the engine's target shape — discrete-level games
in the R&C / BFBB / Sly lineage, with chunk counts bounded in
the low hundreds — this is cost-free: a 200-chunk scene with
small JSON chunk files totals a few megabytes and tens of
milliseconds at scene-boot.

This stops being free at open-world scale. A scene with
thousands of chunks would push scene-boot times into seconds,
load megabytes of metadata that streaming will never use, and
eventually exceed reasonable memory budgets for authored data
held in standby. The engine doesn't target open worlds in
Phase 4 and shouldn't pretend to.

If a future game forces the scale change, the migration:

- **wok-scene change**: `Scene` manifest gains per-chunk
  metadata (eagerness, neighbors, interlocks copied into the
  manifest). The chunk file remains the source of truth; the
  manifest is a derived index produced at save time and verified
  on load.
- **wok-content change**: chunk authored data is lazy-loaded
  behind a feature flag — `LoadedScene.chunks_authored` becomes
  `HashMap<ChunkCoord, Option<Arc<Chunk>>>` with the option
  populated on demand. The streaming algorithm reads metadata
  from the manifest (which is now sufficient) and pulls authored
  chunk data only when actually slicing.
- **Drift mitigation**: a build-time check (or scene-load
  validation) compares manifest metadata against chunk-file
  metadata and refuses to load if they disagree. Authoring tools
  always rewrite the manifest when chunk files change.

This is a real migration but a clean one — both crates change in
the same release, the feature flag lets discrete-level games
continue with the eager-load path unchanged, and the drift bug
the eager-load avoids is replaced by an explicit validation step.

Don't pre-build any of this. Note the boundary so future-you (or
future-CC) knows what's at stake when a game design starts
sketching thousand-chunk scenes.

### 9.15 Hot-reloaded shared resources are replaced, not mutated

`LoadedScene.prefabs` is an `Arc<HashMap<PrefabId, Prefab>>`.
When hot reload (Phase E) signals that a prefab file has
changed, the main thread builds a new `HashMap` with the updated
prefab and replaces the `Arc` wholesale — incremental in-place
mutation is not the model. Same pattern as `RegistryReadView`
(§9.3).

The trade:

- Workers holding an old `Arc` continue to see consistent old
  prefab state until their in-flight request completes.
  Determinism preserved; no half-updated state mid-slice.
- Each hot-reload edit costs O(N_prefabs) shallow HashMap
  clones. Acceptable because hot reload is developer-driven and
  infrequent.
- Read path has zero overhead — `Arc<HashMap>` derefs to
  `&HashMap` and lookup is identical to a non-Arc'd map.

The same shape applies to `LoadedScene.chunks_authored: HashMap<
ChunkCoord, Arc<Chunk>>` for chunk-file hot reload — replace the
specific `Arc<Chunk>` value, not the surrounding HashMap.
(Per-value `Arc` here, rather than per-map `Arc` like prefabs,
because chunk-file edits are the unit of change. Prefabs are
swapped together because a single prefab edit can affect any
chunk that placed it.)

If hot reload ever needs to be fast enough that O(N_prefabs)
clones become measurable, the right move is to make each
`HashMap` value an `Arc<Prefab>` so full rebuild is also cheap.
Don't move to lock-based incremental mutation — that inverts
read/write costs and complicates the worker-snapshot
determinism story.

---

## 10. What This Crate Is Explicitly Not

- **Authored data definitions**. Lives in wok-scene.
- **The slicer**. Lives in wok-scene as `slice_chunk`. We call it.
- **Animation pose data**. Lives in wok-anim. We track AnimationId
  identity in the registry; we don't store the data.
- **Light state curves**. Lives in wok-light. Same pattern as
  animations.
- **Render pipelines / shaders / draw calls**. Lives in wok-render.
- **Physics math / actor integration / collision**. Lives in
  wok-physics. We hand it our runtime arrays; we never run the
  math.
- **Trigger overlap testing**. Lives in wok-physics. We provide
  the volumes; physics provides the overlap predicate.
- **Game-event semantics**. Lives in the game. The `TriggerId →
  game event` wiring is the game's, not ours.
- **Save/load UI**. Lives in the game.
- **Multiplayer protocol**. Doesn't exist in the engine at all.
  Snapshots are the substrate; the game uses them.
- **Sidecar persistence** (dropped items, defeated enemies). Game-
  managed per high-level-design §3 and wok-scene-plan §1.
- **Per-chunk snapshots**. Doesn't exist. Whole-world only.
- **Multi-scene composition or atomic cross-scene transitions**.
  Single-scene at a time; cross-scene transitions hitch through
  "no scene loaded." See §9.13 for the constraint and migration
  cost.
- **Async/await**. We use threads and channels. The game runs
  the loop.

---

## 11. Order of Implementation

Phased to support smoke-test as early as possible (Phase 4 of
Harrison's roadmap, currently active). Each step lands with tests.

### Phase A — Minimum viable for Phase-4 smoke-test (sync, no streaming)

Goal: load one chunk, draw one room, walk a capsule around it.

1. **`error.rs`** — all error enums. No tests.
2. **`registry/`** — full registry (without snapshot integration).
   Tests: §7.1 except #6-#8 (population from scene comes later).
3. **`primitives/`** — procedural meshes for all five primitives.
   Tests: §7.2.
4. **`storage/mesh.rs`** — `MeshCpu`, `MeshGpu`, upload helpers.
   Tests: minimal; full GPU tests under integration tag.
5. **`registry/populate.rs`** — auto-populate from scene+prefabs.
   Tests: §7.1 #6-#8.
6. **`chunk/slot.rs`, `chunk/load.rs`, `chunk/unload.rs`** — slot
   state machine and the load pipeline. Initial worker is the
   `LoopbackWorker` (synchronous). Tests: §7.3 #1-#7.
7. **`chunk/transition.rs`** — the eagerness flag flip
   (`transition_chunk`). Vista state is now carried correctly
   through slot iteration; consumers honor it at their read sites.
   Tests: §7.4.
8. **`system.rs`** — `ContentSystem` minimal API for Phase A:
   `new(device, queue, content_root, config) / shutdown /
    load_scene / unload_scene / request_load / request_unload /
    transition_chunk / poll / slot / slots / active_slots /
    vista_slots / mesh / registry / authored_eagerness`.
   `tick_streaming`, `capture_engine_snapshot`, and
   `restore_engine_snapshot` are deferred to later phases.
9. **Smoke-test crate skeleton** at `wok/examples/smoke-test/`
   with the content layout from §4.4. Verifies: scene loads,
   chunk goes Resident, capsule renders, room renders, transition
   to Vista works.

Phase A ships the crate end-to-end for Phase-4 needs.

### Phase B — Background worker

Goal: file I/O off the main thread.

10. **`worker/`** — real `std::thread`-based worker with
    crossbeam channels. `LoopbackWorker` stays as a test-only
    variant. Tests: §7.7.
11. **Mesh-upload dedup** — `Arc<Mutex<HashSet<MeshId>>>`. Tests:
    §7.3 #8.
12. **Cancellation paths** — slot transitions to Unloading while
    Loading, etc. Tests: §7.3 #3-#5 under async worker.
13. **`registry/view.rs`** — `RegistryReadView` + Arc-swap
    pattern. Tests: §7.1 #9.

### Phase C — Streaming

Goal: camera-driven chunk loading.

14. **`streaming/desired.rs`, `streaming/hysteresis.rs`** —
    desired set + hysteresis. Tests: §7.5 #1-#4.
15. **`streaming/interlock.rs`** — interlock fixpoint. Tests:
    §7.5 #7-#8.
16. **`streaming/prioritize.rs`** — distance-ordered queue.
    Tests: covered by §7.5 #9.
17. **MAX_LOADED enforcement and LRU** — `slots.lru` bookkeeping.
    Tests: §7.5 #9.
18. **Vista-multiplier radius** — authored Vista chunks load at
    extended range. Tests: §7.5 #10.

### Phase D — Snapshots

Goal: save/load.

19. **`snapshot/mod.rs`** — `EngineSnapshot`, `StitchId`, and the
    compile-time `schema_hash` derive macro. Tests: §7.6 #6, #7.
20. **`snapshot/capture.rs`, `snapshot/restore.rs`** — engine
    fields only (game stitching pattern). Blocking restore that
    pumps worker internally; async restore deferred to the
    multiplayer phase. Tests: §7.6 #1–#5, #8–#10.

### Phase E — Hot reload integration

Goal: the editor's "Run" preview reflects edits.

21. Poll wok-scene's `FileWatcher` from `ContentSystem::poll()`.
22. Re-dispatch affected chunks via `request_load` (which is
    already idempotent and replaces existing slot).
23. Generation-token policy (§9.5) — workers receive a generation
    snapshot; main thread compares on receive.

### Phase F — Determinism harness integration

Goal: ship the workspace-level deterministic replay (high-level
§4 Level 2).

24. Expose deterministic dump format for ContentSystem state
    (current slots, loaded scene, registry hash). Tests: §7.8.

---

## 12. Decisions Index

All design questions from v1 review are resolved. Cross-reference
for decisions whose reasoning lives in the body:

| Decision | Resolution | Section |
|---|---|---|
| Trigger evaluation split | Data in wok-content, overlap test in wok-physics, routing in game | §1.3 |
| Vista's authored vs runtime model | Both, separated by data location; no origin field; per-chunk load/unload radii | §5.2, §9.11, §9.12 |
| Snapshot scope and stitching | (β) — engine snapshot only; game stitches engine + physics + game; stitch_id token | §4.2, §5.3 |
| Slot state machine encoding | Enum payload (Resident/Unloading carry ResidentChunk); canonical iteration via accessors | §3.2 |
| Single-scene loading | Phase-4 simplification; cross-scene transitions hitch; multi-scene migration noted | §9.13 |
| Chunk authored-data residency | Eager-load all chunks at scene boot; manifest stays coords-only; scale boundary noted | §5.2, §9.14 |
| Restore mode | Blocking restore for Phase A; async deferred to multiplayer phase | §5.3 |
| Mesh GPU buffer lifetime | Immortal through Phase 4; refcount with first non-immortal asset kind | §9.4 |
| Hot reload race policy | Generation tokens (Phase E) | §9.5 |
| Registry deletion form | Tombstones, not sparse-array nulls | §4.1 |
| Worker concurrency | One worker thread; rayon for fan-out inside requests if profiles justify | §6.3 |
| Channel implementation | `crossbeam_channel` (bounded priority + unbounded mpsc) | §6.2 |

