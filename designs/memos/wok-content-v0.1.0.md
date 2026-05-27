# wok-content v0.1.0 - Plan vs Reality (Phase A)

Implementation memo for the Phase A foundation of `wok-content`. Branched from
`v0.2.0-wok-scene`; ten steps landed as ten commits; all 39 tests pass; clippy clean under
`-D warnings` across all targets.

This memo follows §11 Phase A step ordering (1 through 10). For each step it states what
the plan specified, what landed, and any drift. Pinned `§9` guardrails get a separate
"Pinned guardrails" section at the end.

---

## Step 1 - error.rs

**Plan:** all error enums with `thiserror`-style narrow shape. `LoadError`,
`RegistryError`, `SnapshotError`, `TransitionError`, `SaveError`. No tests.

**Implemented:** matches plan. Notable deliberate deviations:

- **No `thiserror` crate.** wok-scene shipped with manual `impl Display + Error` on its
  error types; matched that pattern here. The plan phrase "thiserror-style narrow enums"
  refers to the shape, not the crate. Avoiding a new workspace dep matches the user's
  CLAUDE.md rule "never install new programs, packages, or dependencies without asking
  first." Confirmed with user before starting.
- **`TransitionError::NotResident` carries `state_label: &'static str` rather than the
  plan's `state: SlotState`.** `SlotState` carries non-`Clone` GPU handles (the `Failed`
  variant carries `Arc<LoadError>`; the `Resident` / `Unloading` variants carry
  `ResidentChunk` with `MeshGpu`). Including a `SlotState` clone in the error breaks the
  `Clone + PartialEq + Eq` bounds tests expect. The label string is sufficient for the
  diagnostic surface and keeps `TransitionError` lightweight.
- **`LoadError::AssetMissing.slug` is `Option<Slug>`** rather than `Slug`. Phase A's
  not-in-registry path can hit the error without having decoded a slug (e.g., the
  "scene-not-loaded" synthetic-Failed sentinel uses serial `u32::MAX` with no slug). The
  optional form covers both.

---

## Step 2 - registry/ (identity half)

**Plan:** `KindTable<S, E>`, five `KindTable`s, `MeshEntry` / `AudioEntry` /
`AnimationEntry` / `VoiceEntry` / `LightEntry`, `register_*` / `rename_*` /
`set_mesh_source`, on-disk format, `RegistryReadView` `Arc`-rebuild on every mutation.
Tests §7.1 except #6-#8 (population) and #9 (read-view-after-rename, Phase B).

**Implemented:** matches plan. Deliberate deviations:

- **`rename_*` and `set_mesh_source` take `&MeshId` / `&AudioCueId` / etc. by reference**
  rather than the plan's pass-by-value `id: MeshId`. The rename body only reads
  `id.serial()`; consuming the caller's id adds nothing. Pass-by-reference also matches
  `Registry::mesh(&MeshId)` so rename and lookup APIs line up. Plan's pass-by-value
  signature triggered clippy's `needless_pass_by_value`; the deviation is small and
  preserves ergonomics.
- **`register_audio_placeholder` / `register_animation_placeholder` /
  `register_voice_placeholder` / `register_light_state_placeholder`** added alongside the
  plan's shipped-only `register_audio(slug, source: PathBuf)` etc. Populate-from-scene
  needs a placeholder constructor for kinds beyond mesh; the plan's signature didn't have
  one. The shipped variant remains the canonical entry point; placeholder variants exist
  for the populate path.
- **`EntrySlot` enum** internal to the table holds `Empty | Live(E) | Tombstone`. Plan §3.3
  shows `by_serial: Vec<Option<E>>`. The three-variant enum makes tombstones a distinct
  shape from gaps and makes the serde round-trip (test §7.1 #10) the obvious shape.

Tests #1-#5 + #10 land here. #6-#8 land in step 5; #9 lands in Phase B.

---

## Step 3 - primitives/ + CPU mesh types

**Plan:** five procedural builders (cube, ellipsoid, cylinder, capsule, plane) producing
`MeshCpu`. Tests §7.2.

**Implemented:** matches plan. The CPU storage types (`MeshCpu`, `MeshVertex`) landed in
this step because primitives produce them; `MeshGpu` and upload helpers followed in step
4. Plan's order has step 4 after step 3; the CPU types are a natural step-3 prerequisite.

Cube: 24 vertices (4 per face for crisp per-face normals), 12 triangles. Plan didn't
specify vertex count; 24 chosen so cel shading gets flat per-face shading rather than
interpolating normals across faces.

Ellipsoid / cylinder / capsule: tessellation clamped to >= 3 (segments, subdivisions)
inside each builder so a hostile or programmer-error config doesn't collapse the ring to
a degenerate disc.

Plane: 1 quad in XZ with normal +Y. Matches the conventional flat-floor primitive.

`SurfaceTagPalette` default keyed to grass / stone / dirt / sand / wood with a magenta
fallback for unknown tags (the conventional "missing texture" marker). Plan §9.9 left the
concrete palette to Phase A; this picks tones that read as distinct categories under cel
quantization.

`PLACEHOLDER_COLOR = [0.80, 0.78, 0.72]` on every primitive vertex - the
swap-me-for-a-shipped-mesh tone. Authored `Shape.visual_color` overrides at the slicer's
`VisibleShape` level, not at the primitive's `MeshVertex.color`; renderers can multiply
through.

---

## Step 4 - MeshGpu + storage::upload

**Plan:** `MeshCpu`, `MeshGpu`, upload helpers. Minimal tests here.

**Implemented:** matches plan. `storage::upload` uses
`wgpu::util::DeviceExt::create_buffer_init` to fold the upload-with-data path into one
call. Buffer usage flags include `COPY_DST` for future streaming uploads even though
Phase A never rewrites.

Test scaffolding: `tests/common/mod.rs` shares the headless wgpu device setup. Uses
`pollster::block_on` to drive wgpu 24's async `request_adapter` / `request_device`.
Pollster confirmed with user as a dev-dep before starting; already a workspace dep through
pantry.

---

## Step 5 - registry/populate.rs

**Plan:** walk scene + prefabs + chunks, record `UsageSite` per reference. Tests §7.1
#6-#8.

**Implemented:** matches plan. Notable choices:

- **Lookup is by slug**, not serial. The wok-scene asset ID token is `slug-serial`; the
  serial is debug surface per wok-scene §9.1, the slug is what stays stable across
  authored-data edits. Populate walks references, looks up the slug, and either records
  usage at the existing entry or allocates a fresh placeholder. Re-populates find the
  same entry by slug, making the walk idempotent (test #7).
- **`UsageSite::SceneDefaultLightState` and `UsageSite::PrefabAudioCue`** added to the
  plan's variant list. The plan showed `PrefabState`, `PrefabShape`, `ChunkRegion`, and
  `// ...`; the new variants cover the references the Phase A walk needs.
  `UsageSite::ChunkLightState` (also new) covers the chunk-level `light_state` field.
  `PrefabShape` and the existing `ChunkRegion::region_name` cases remain so the variant
  shape is stable across phases.
- **Internal `register_placeholder_*_internal`** helpers skip the `mutate` view-rebuild
  because populate is already inside one `mutate` call; nested `mutate` would do redundant
  view-rebuild work per allocated placeholder.

---

## Step 6 - chunk slot + load + unload

**Plan:** `ChunkSlot`, `SlotState`, `ResidentChunk`, `ChunkGpuHandles`, load /
unload pipeline, `LoopbackWorker` (synchronous). Tests §7.3 #1-#7.

**Implemented:** matches plan structure. Three deliberate deviations, all confirmed with
user before starting:

1. **`MeshGpuRef { mesh_id: MeshId, source_visible_index, local_transform }` replaced by
   `VisibleMesh { gpu: MeshGpu, source_visible_index, local_transform }`.** Phase A has no
   shipped meshes; visible placeholder primitives are slot-owned (each ResidentChunk drops
   its own visible MeshGpus). Phase B reintroduces `MeshGpuRef` with the
   `HashMap<MeshId, MeshGpu>` storage and the dedup-via-shared-set pattern from plan §5.1.
   The §9.4 immortality rule applies once shipped meshes arrive; Phase A's terrain-style
   slot ownership is the right shape until then. Test §7.3 #8 (dedup) is Phase B per
   plan §11.

2. **`SlotState::Failed` and `ContentEvent::ChunkFailed` carry `Arc<LoadError>`** rather
   than the plan's bare `LoadError`. `wok_scene::LoadError` wraps `std::io::Error` which
   is not `Clone`; the same error sat on the slot and emitted as an event needs sharing.
   `Arc` is the smallest change that preserves both sites' access without restructuring.

3. **`WorkerResult::ChunkLoaded` boxes its payload** (`Box<ChunkLoadedPayload>`).
   `ChunkRuntime` carries four `Vec`s (visible, hitboxes, triggers, regions) plus the
   surface tag table; held inline the enum's stack footprint runs into the hundreds of
   bytes. Boxing keeps the enum cheap to move across the Phase B channel boundary and
   doesn't change behavior.

Other notes:

- **`Loading` state is not externally observable in Phase A.** The LoopbackWorker drains
  synchronously inside `poll()`; between submit and integration there's no public API
  surface to observe a Loading slot. The variant exists on `SlotState` so Phase B's
  threaded worker doesn't need a struct change; until then, tests for "unload while
  Loading" (§7.3 #4) collapse to "unload while in-flight" - same observable behavior at
  the public API.
- **`pipeline.rs::run_load_chunk` checks `prefab_state.mesh_override` against the
  registry view** for the `AssetMissing` path (test §7.3 #7). Phase A has no shipped
  meshes so the override is not consumed; the check enforces that the registry covers
  every authored asset reference. When shipped meshes arrive, the same check gates the
  load-mesh-from-source path.
- **`SliceError::UnknownPrefab` maps to `LoadError::PrefabMissing`** so the variant
  matches the plan's surface; other slice errors propagate as `LoadError::Slice`.

`ContentSystem` was implemented to the minimum step-6 needed (`new`, `load_scene`,
`unload_scene`, `request_load`, `request_unload`, `poll`, `slot`, `scene`). Step 9 added
the read accessors and `mesh` / `registry` exposure.

---

## Step 7 - chunk/transition.rs

**Plan:** `transition_chunk(coord, ChunkEagerness) -> Result<(), TransitionError>`. Tests
§7.4.

**Implemented:** matches plan. The transition flips
`SlotState::Resident(rc).runtime.eagerness` (also `Unloading(rc)` - the pinned variant
symmetry from §3.2). No I/O, no worker dispatch. The `ChunkTransitioned` event is queued
on `pending_events: Vec<ContentEvent>` and emitted from the next `poll()`.

The `pending_events` queue replaced step-6's `pending_scene_event: Option<...>` slot.
Synchronous operations (load_scene, unload_scene, transition_chunk) push events here;
poll() drains them before integrating worker results. Cleaner shape than per-event-kind
stash, and ChunkTransitioned needs the same channel.

Step 7 covers tests §7.4 #1, #2, #4, #5, #7. Tests #3, #6, #8 (iteration accessors and
authored-vs-runtime comparison) need step-9 accessors and land in that step's tests. Test
#9 (snapshot persistence of runtime transitions) is Phase D.

---

## Step 8 - terrain/

**Plan:** terrain mesh generation. Sample chunk via wok-scene's `height_at`, `normal_at`,
`surface_at`. NW-SE diagonal locked (§9.18). Slot-owned, not registry-tracked (§9.17).
Tests §7.2b.

**Implemented:** matches plan. The mesh generator walks a row-major (i, j) grid. For each
vertex it samples height, normal, and surface tag through the wok-scene functions and
maps the surface tag to a color via `ContentConfig::terrain_palette`. Triangulation per
quad: `(A, B, D)` then `(A, D, C)` where the diagonal A-D runs from the NW corner
`(i, j)` to the SE corner `(i+1, j+1)`. Same winding as the cube primitive's +Y face.

`terrain::generate_mesh` panics if `chunk.terrain.is_none()`. The pipeline checks
`runtime.terrain.is_some()` before calling, so the panic flags a programmer-error
contract violation rather than authored data error. Test §7.2b #8 verifies the panic via
`std::panic::catch_unwind` - the integration point in `worker/pipeline.rs` is what carries
the None vs Some branch.

`worker/pipeline.rs` wires terrain into the load pipeline: when the sliced runtime
carries terrain, generate the mesh, upload to GPU, attach to `ChunkLoadedPayload`. GPU
upload failures surface as `LoadError::Gpu`.

Tests §7.2b #1-#8 land here. The fixtures construct `ChunkRuntime` + `RuntimeTerrain`
directly at multiple grid widths (2x2, 5x5, 9x9, 129x129) because wok-scene's authored
`TerrainData::CELLS_PER_AXIS` is fixed to 129; smaller grids are only constructable at
the runtime layer.

---

## Step 9 - ContentSystem read accessors

**Plan:** `slots / active_slots / vista_slots / mesh / registry / authored_eagerness /
streaming_eagerness`. Tests §7.4 #3, #6, #8 land via these accessors.

**Implemented:** matches plan. `mesh(&MeshId) -> Option<&MeshGpu>` always returns `None`
in Phase A because there are no registry-tracked GPU meshes (Phase A's slot-owned model
per step-6 deviation). Phase B's MeshId-keyed dedup populates the storage and the method
resolves real handles. Signature stable across phases.

Tests §7.4 #3 (iteration partition) and #6 (Vista trigger-volume placement in
`vista_slots` only) and #8 (authored vs runtime divergence + unload-reload reverts to
authored) all pass.

---

## Step 10 - integration fixtures + Phase-A integration test

**Plan:** fixtures at `wok-engine/tests-integration/fixtures/` per §4.4 layout. The
`tests-integration/` crate skeleton "may be created here or later when other crates
participate".

**Implemented:** committed the JSON fixtures (registry, prefabs, scene, chunk). The
heightmap binary is NOT committed; the integration test copies the fixture tree into a
tempdir and writes a flat 129x129 WTRN-format heightmap binary at the expected sibling
path. Documented in `tests-integration/fixtures/README.md`. The `tests-integration/`
crate skeleton is deferred to when wok-render / wok-physics participate.

`wok-content/tests/fixtures.rs` exercises every step-10 acceptance criterion: scene
loads, registry populates with the correct usage sites, chunk reaches Resident, terrain
mesh appears with the expected index count, visible meshes match the runtime visible
array, transition to Vista works, iteration accessors partition correctly post-transition.

---

## Pinned guardrails

The plan §9 flagged three Pinned constraints that govern Phase A. All preserved.

1. **§9.17 Terrain meshes are slot-owned.** `MeshGpu` for terrain lives on
   `ResidentChunk.gpu.terrain: Option<MeshGpu>`. The worker generates it inline; the
   integrator hands it to the slot; drop with the slot. Not registered, not in
   `ContentSystem.meshes`, no `MeshId`. Verified by test §7.2b #8 (None-terrain runtime
   produces no terrain GPU handle).

2. **§9.18 NW-SE triangulation locked.** Every quad splits A-D where A = (i, j),
   D = (i+1, j+1). Two CCW triangles `(A, B, D)` and `(A, D, C)`. Same `TerrainData`
   produces byte-identical `MeshCpu`. Verified by tests §7.2b #2 (determinism) and #3
   (per-quad index pattern).

3. **§9.19 Async-only public API.** `request_load(coord) -> LoadHandle` plus
   `ContentEvent::ChunkResident` on `poll()`. No `load_chunk_blocking` wrapper.
   `LoopbackWorker` covers the synchronous needs of test code and editor preview without
   exposing a blocking variant on the public API.

---

## Other drift surfaced

- **No `tracing` instrumentation.** Plan explicitly forbids; verified clean. The only hit
  for "tracing" in `wok-content/` is the word inside the "back-tracing" doc comment
  inherited via wok-scene's runtime types.
- **`anyhow` not used.** Library-crate constraint per project canon and plan §1 Design
  Rules; confirmed.
- **Single-worker shape preserved.** `LoopbackWorker` is the only `Worker` flavor in
  Phase A. Plan §6.3 ("one worker, not a pool") remains the long-term shape; Phase B
  replaces LoopbackWorker with the threaded equivalent without changing the protocol.

---

## Exit criteria

- 39 tests pass: 9 registry + 5 primitives + 1 storage_gpu + 7 chunk_lifecycle +
  8 transition + 8 terrain + 1 fixtures + 0 doc.
- `cargo clippy --all-targets -- -D warnings` clean.
- Ten commits on `feature/wok-content-v0.1.0`, one per step.
- wok-scene v0.2.0 behavior unchanged: wok-content only consumes its public API.
- Plan deviations documented above plus inline in the relevant module headers.

Plan update needed: yes. Plan §3.2 `MeshGpuRef`, §3.5 `LoadError` shape on `Failed` and
`ChunkFailed`, §3.3 `register_*` pass-by-value signatures, and §3.3 `UsageSite` enum
variant list all received Phase-A implementation choices that should pull back into the
plan at the next review. None of these changes alter the design's intent; they refine
the surface to fit the type system's Clone-bounds constraints and the slot-owned model.
