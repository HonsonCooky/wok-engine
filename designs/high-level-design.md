# Engine Design — High-Level

A modular monolith Rust workspace for building 3D action, platformer, and
open-world games at humble vertex counts. Target hardware: Steam Deck and
developer laptops. Built primarily by AI from per-crate detailed plans
(separate documents).

Three layers, each reducing the design space of the one below:

- **Pantry** — cross-platform substrate (window, GPU, audio, input, frame
  loop, OS theme).
- **Wok** — opinionated 3D engine; nine library crates.
- **Game** (Unstitched first) — one specific game per crate. Owns the
  main loop and composes Wok's tools.

Each layer may only depend on layers below it.

---

## Principles

Underlying philosophy: **fun through simplicity.** Simpler systems (boxes
and spheres for collision, primitives for geometry, clear data flow)
produce games a small team can ship bug-free and players can pick up and
enjoy in limited time.

The five principles below are consequences. Every Wok feature must answer
to all five. Pantry is unopinionated by definition.

**1. Placeholder-first authoring.** Games are built entirely on primitive
shapes with stub audio and minimal animations. Real meshes, voice, music,
and animations plug in later via an automatic asset registry that
populates from scene contents. Terrain is authored as heightmap data per
chunk at 1m × 1m resolution; the heightmap is the data, not a placeholder.
Prefab geometry decorates and overhangs the base terrain layout.

**2. Earn its place.** Every subsystem justifies its complexity against
the engine's scope and modest hardware. No nanite, no research rendering,
no PhD-grade techniques. When two designs deliver the same outcome, the
simpler one wins.

**3. Authoring versus runtime separation.** Editing data structures and
runtime data structures are different. Authoring optimizes for human
comprehension; runtime optimizes for system access patterns.
Transformations happen at load/save time, never per frame.

**4. Built for one target.** The renderer has exactly one configuration.
Higher-end machines run the same game with thermal/framerate headroom,
not visual richness. Hardware adaptation that doesn't change what's
rendered (resolution, framerate cap, audio output device) is allowed.

**5. Primitives, not features.** Wok provides primitives that games
compose into features. Features with well-understood game-side patterns
(save/load UI, settings menus, HUD, input remapping, networking,
localization, achievements, dialog trees, inventory, quest tracking,
mod support, tutorials, audio routing policy) belong to game code.

---

## 1. Constituent Crates

A crate earns its boundary by being substantial in scope or genuinely
independent in shape from its siblings. Sub-thousand-line concerns become
modules within a larger crate.

### Pantry

- **`pantry`** — Cross-platform plumbing: window creation and surface,
  GPU device acquisition (wgpu), input event polling, gamepad handling
  (gilrs), audio output (cpal), frame loop driver, OS theme signaling.
  Re-exports wgpu / winit / glam / bytemuck / cpal / gilrs so consumers
  pin one source of truth.

### Wok

Nine library crates, in dependency order.

- **`wok-scene`** — Scene and prefab data: chunked scene representation,
  prefab definitions, region markers for fog and lighting zones, JSON
  serialization for both, file-watcher for hot-reload. Chunks are
  128m × 128m on the horizontal plane (engine constant). Scenes carry
  streaming metadata: default load radius (typically tracking fog
  distance), default eagerness, per-chunk overrides for eagerness,
  neighbor lists, and additional chunks to always load alongside (for
  portals, teleporters, interlocks). Prefabs are stateful — each prefab
  has one or more named states (default, open, destroyed), and each
  state is a list of shapes. A shape is a primitive (cube, ellipsoid,
  cylinder, capsule, plane) with a transform and two flags: `is_hitbox`
  and `is_visible`. The three meaningful combinations are solid
  placeholder (both flags), trigger volume (hitbox only), and
  visual-only placeholder (visible only). At chunk load, authored
  shapes are sliced into separate arrays per system (visible shapes,
  physical hitboxes, trigger volumes) so each system iterates only
  what it cares about. Asset replacement happens at the state level: a
  real mesh replaces all visual shapes in a state at once. Pure data
  and serialization; no runtime logic, no GPU concerns.

  **Terrain.** Each chunk carries a heightmap authored at 1m × 1m
  resolution (128 × 128 height values per chunk, ±32m vertical range,
  u16-quantized to millimeter precision). Heightmap data lives in a
  sibling binary file (`0_0.heightmap.bin` alongside `0_0.json`).
  Per-cell surface tags use the same intern-table pattern as shape
  surface tags. Sampling functions — `height_at(x, z)`,
  `normal_at(x, z)`, `surface_at(x, z)` — live here per the design rule
  that pure functions live with the data type. The 1m × 1m resolution
  matches the gameplay-relevant scale (climbable step heights,
  perceptible terrain features); finer resolution would 4× or 16×
  storage and triangle counts for marginal visual gain given the
  cel-shading band quantization and mid-distance third-person camera
  framing this engine targets.

- **`wok-physics`** — Math and simulation for moving things through 3D
  space. Three concerns share this crate (same math toolbox, change
  together): physics primitives (capsule, AABB, ellipsoid, heightmap;
  swept queries with slide resolution, gravity integrator), actor pool
  with stable handles and deterministic integration step (player,
  enemies, projectiles), and camera primitives (orbit for authoring,
  spring-arm for play). Capsule-vs-heightmap collision uses wok-scene's
  sampling functions for height and slope queries; max-slope clamping
  for ground-following is part of the integration step. Terrain and
  prefab hitboxes contribute independent contact streams; the actor's
  final position is constrained by both.

- **`wok-anim`** — Animation, authoring and runtime: pose data, blend
  graphs, event markers (authoring), plus playback — blending between
  named poses (idle / walk / shoot), layered animation (upper-body
  shoots while lower-body walks), animation events at marked timestamps
  (footstep sound, swing hits), eventual skeletal animation when real
  meshes arrive. Animation state is separate from physics state —
  animation affects appearance, not movement, so it doesn't feed back
  into simulation. Follows the same pattern as wok-light and
  wok-sequence: each domain-specific crate owns its own authoring
  data, and wok-scene holds typed references by ID. Implementation
  deferred until placeholder animation needs more than transform
  tweens.

- **`wok-content`** — Asset pipeline and registry: placeholder mesh
  primitives, terrain mesh generation, GLTF loader (when real meshes
  arrive), GPU upload coordination, asset registry. The registry is the
  manifest of every asset the game needs — meshes, audio cues,
  animation poses, voice lines — tracking where each is used and
  whether it has shipped content or is still placeholder. Population
  is automatic: placing a prefab or hitbox in a scene registers the
  assets it depends on. Artists pull from the registry; completed
  assets replace placeholders in-place. wok-content tracks identity
  for all asset kinds; concrete buffer data lives in the relevant
  domain crate (mesh buffers in wok-content, audio buffers in
  wok-audio, animation data in wok-anim, light data in wok-light).
  Asset and chunk loading run on a dedicated background worker thread
  (`std::thread`, fed by an `mpsc` channel from a priority queue).
  Data-parallel work inside Wok operations elsewhere uses `rayon`.
  Streaming algorithm is engine-owned and declarative: scenes describe
  topology via chunk metadata; engine computes the desired loaded set
  each tick from that data plus player position. Eager chunks within
  radius load automatically; Lazy chunks load only when explicitly
  requested by game code; Vista chunks are loaded and rendered but
  excluded from simulation, collision, and trigger evaluation (for
  distant scenery the player sees but cannot interact with). Eagerness
  transitions are flag flips on already-loaded chunks, not reloads.
  Hard cap of 32 chunks loaded at any time (engine constant); eviction
  is LRU with priority weighting.

  Terrain meshes are generated from wok-scene's heightmap data with
  smooth interpolated normals from the heightmap gradient (faceted
  geometry, smooth shading) and per-vertex color variation derived from
  surface tags. Same procedural-mesh-from-data pattern as primitive
  shape meshes; same GPU upload path.

- **`wok-audio`** — Audio playback engine. Voice pool with stable
  handles, distance attenuation and stereo panning for 3D positioning,
  category buses (master, music, sfx, voice — categories configured by
  the game), audio buffer storage (loaded WAV/OGG data). Receives play
  commands from game logic, consumes wok-content's registry for
  AudioCueId → source path resolution, uses pantry's cpal output. The
  storage half parallels wok-anim's animation data and wok-light's
  light state data: wok-content tracks identity; wok-audio holds the
  loaded data. A transparent passthrough to pantry's cpal stream exists
  for cases the engine's mid-level API doesn't anticipate. Reverb
  zones, environmental effects, music sequencing, and audio routing
  policy are game-side.

- **`wok-light`** — Lighting data model and animation curves (lighting
  state is animatable over time and switchable at runtime), offline
  static-light bake (precomputes baked light and ambient occlusion
  contribution into scene vertex data or lightmaps, including terrain
  heightmap surfaces queried via wok-scene's sampling functions), and
  budgeted dynamic light pool (importance-weighted with reserved player
  slots, for muzzle flashes, projectile exhaust, other moving lights).

- **`wok-sequence`** — Cutscene and scripted-sequence machinery:
  timeline data, camera path data, animation playback driver, dialog
  and audio cue timing. Drives a scene through scripted states for a
  bounded duration, then returns control to gameplay. Authoring UI
  lives in wok-shell.

- **`wok-render`** — Renderer: forward pipeline, single shadow map
  pass, lighting passes (consuming wok-light's baked + dynamic data),
  parametric gradient sky, fog, post-process. Consumes wok-scene,
  wok-light, and wok-content to know what to draw. Renderer
  commitments:

  - **Cel shading as the one visual style.** Lighting values quantized
    into discrete bands with smoothstep transitions, plus rim light for
    silhouette readability. Per-scene tunable parameters: band count
    (range 2–8, default 4), transition softness, rim intensity, ambient
    floor color, fog parameters. Terrain meshes use the same cel
    shading pipeline as primitive geometry; smooth normals from
    heightmap gradients produce smooth shading despite faceted 1m × 1m
    geometry.
  - **Alpha cutout transparency only.** No sorted blending, no OIT.
    Water, glass, smoke, magic effects use stylized cutout.
  - **Fog is always on.** Parameters animated per lighting state; fog
    distance determines render distance. Fog color drives sky's horizon
    color so distant geometry blends into the background.
  - **Parametric gradient sky.** Vertical gradient from horizon color
    to zenith color, driven by lighting state parameters; sun disc
    billboard at sun direction; optional stars layer that fades in at
    night; optional animated cloud plane. No sky textures.
  - **One shadow map render per frame.** Scene or lighting state
    declares the active shadow source (sun, flashlight, key spotlight —
    game's choice). Single-cascade projection sized to active chunk
    set; default 2048×2048, tunable. PCF soft edges. Baked lights bake
    their shadows into the lightmap at offline bake time. Dynamic
    lights from the pool don't cast shadows.

- **`wok-shell`** — Authoring shell: egui integration, panel layout,
  viewport management, command palette, terrain painting tools (sculpt
  height, paint surface tags, cross-chunk paint operations per the
  existing region-painting convention), light/dark theme switching
  with OS-aware live switching, dockable panels. Designed for the
  engineer who lives in it: left-hand-keyboard (ZSA Voyager-style
  split) and right-hand-mouse workflow, operations placed to minimize
  right-hand-keyboard travel, iteration loop tuned for enjoyment
  across long sessions. Contains the editor binary's entry point at
  `wok-shell/src/bin/wok.rs`, producing the `wok` executable.

### Game

- **`unstitched`** — All game-specific runtime logic: physics tuning,
  actor configuration, player controller, input mapping, trigger
  orchestration, effect routing through wok-light's pool, audio cue
  routing through wok-audio, HUD, menus. Internal modules give
  structure. Binary entry at `unstitched/src/main.rs` produces the
  shipped game executable.

Game content (scenes, prefab definitions, lighting curves, audio cue
tables) lives as data files under `unstitched/content/`, produced by
the Wok editor and consumed by the Unstitched runtime.

---

## 2. Dependency Graph

Topological order, lowest first. Each crate may depend only on the
crates listed for it.

- **`pantry`** — no internal dependencies.
- **`wok-scene`** — `pantry`. Bedrock data model; defines opaque
  references for lights and assets that other crates resolve.
- **`wok-physics`** — `pantry`, `wok-scene`.
- **`wok-anim`** — `pantry`, `wok-scene`, `wok-physics` (needs actor
  handles).
- **`wok-content`** — `pantry`, `wok-scene`.
- **`wok-audio`** — `pantry`, `wok-scene`, `wok-content` (needs
  registry for AudioCueId → source path).
- **`wok-light`** — `pantry`, `wok-scene`, `wok-physics` (bake step
  queries geometry via physics raycasts).
- **`wok-sequence`** — `pantry`, `wok-scene`, `wok-physics`, `wok-anim`,
  `wok-light`, `wok-content`, `wok-audio`.
- **`wok-render`** — `pantry`, `wok-scene`, `wok-physics`, `wok-anim`,
  `wok-light`, `wok-content`.
- **`wok-shell`** — `pantry` and all `wok-*` library crates.
- **`unstitched`** — `pantry` and all `wok-*` library crates.

### Forbidden edges

First three are Cargo-enforced; remainder are review-enforced.

1. No `wok-*` or `unstitched` depends on `pantry` internals beyond its
   public API.
2. No `wok-*` depends on `unstitched`.
3. `wok-scene` depends on no other `wok-*` crate.
4. `wok-render` does not depend on `wok-sequence` or `wok-shell`.
5. `wok-content` does not depend on `wok-render`, `wok-physics`,
   `wok-light`, `wok-sequence`, or `wok-audio`.
6. `wok-audio` does not depend on `wok-physics`. (Audio positioning
   takes `Vec3` arguments; game composes actor positions from
   wok-physics and passes them in.)
7. No cycles. The graph remains a DAG.

---

## 3. Data Flow from Authoring to Runtime

Wok is a library, not a runtime. The game owns the main loop and calls
into Wok's functions to load chunks, step simulation, render, etc.

Every piece of content passes through four states:

1. **Authored on disk.** JSON files in the project's content directory.
   Prefabs as stateful shape lists with flags, scenes as chunked prefab
   placements with lighting and region metadata, lighting states as
   animation curves, terrain as sibling binary heightmap files.
2. **Authored in memory.** Deserialized into Rust types by wok-scene's
   loader functions. Unified shape lists, lighting curves with control
   points, full asset metadata, heightmap arrays. Editor mutates this;
   game reads it.
3. **Runtime arrays.** Produced by wok-content's transformation
   functions when a chunk loads. Shapes sliced into per-system arrays
   (visible, physical hitbox, trigger). Lighting curves resolved.
   Asset references resolved to concrete handles. Terrain mesh
   generated. Authored form is no longer referenced.
4. **Per-frame system state.** Computed each frame from runtime arrays.
   Frustum culling produces visible set; collision produces contact
   list; lighting produces interpolated state. Not durable; computed
   and discarded.

Data flows downward only. Runtime changes don't write back to authored
data.

### Transitions

| Transition | Crate | Triggered by |
|---|---|---|
| Disk → Authored memory | `wok-scene::load_*` | Game calls when needed |
| Authored memory → Disk | `wok-scene::save_*` | Editor save |
| Authored memory → Runtime arrays | `wok-content::transform_chunk` | Game requests chunk load |
| Runtime arrays → Per-frame state | Each system's frame function | Game's main loop |
| Runtime arrays → Disposed | `wok-content::release_chunk` | Game requests chunk unload |

The transformation step at chunk load can run synchronously or on the
background worker thread. wok-content provides both.

### Snapshots

A snapshot is whole-world runtime state — all loaded chunks' runtime
arrays plus per-system state needed to resume simulation. Snapshots
exclude authored data (reloaded from disk deterministically), bounding
their size.

Save: game requests a snapshot, writes to disk. Load: game reads
snapshot, requests chunk loads referenced by it, applies snapshot to
loaded arrays. Same mechanism for multiplayer join-in-progress.

Snapshots are whole-world, not chunk-scoped. An individual chunk that
unloads and reloads always rebuilds from authored data — no chunk-level
state preservation. If a game wants persistence across unload/reload
(dropped items stay where they were, defeated enemies stay defeated),
it manages a sidecar file alongside authored data and applies that
state to runtime arrays after chunk load. The engine has no opinion
on this; it's a game-side concern, consistent with "primitives, not
features."

### Hot reload

wok-scene's file watcher detects authored-data changes. The game polls
each frame and decides how to respond (typically by requesting affected
chunks be re-transformed). Hot reload flows authored → runtime only,
never back. Editor's "Run" preview shows runtime; permanent changes
require stopping preview and editing authored data.

---

## 4. Validation Strategy

Three levels. Each crate exposes the surface needed at each level.

### Level 1: Unit tests (`cargo test`)

Per-crate test commitments:

- **wok-scene** — JSON round-trip for every authored type; shape-slicing
  transformation produces known runtime arrays from known authored
  scenes; heightmap serialization round-trip; sampling-function
  fixtures.
- **wok-physics** — every primitive query against fixture geometry
  (capsule-vs-AABB, capsule-vs-ellipsoid, capsule-vs-heightmap, swept
  queries); actor integration determinism (same intent stream → same
  position trace).
- **wok-light** — bake determinism (same scene → same baked data); light
  pool eviction against known scenarios.
- **wok-content** — registry population from synthetic scenes; chunk
  loading state machine; terrain mesh generation determinism.
- **wok-audio** — voice pool allocation/eviction; mixing math against
  fixture inputs; cue resolution via registry.
- **wok-sequence** — timeline evaluation (at time T, sequence produces
  these outputs).
- **wok-render** — pipeline assembly and shader compilation. Visual
  correctness validated at Level 3.
- **wok-shell** — panel layout and command palette routing.
- **wok-anim** — pose blending math and timeline evaluation.
- **pantry** — mockable platform abstractions (input event parsing,
  frame timing math). Hardware paths validated by running.

### Level 2: Deterministic replay harness

Workspace-level harness in `wok-engine/tests-integration/`. Steps:

1. Load known authored scene.
2. Spawn actors at known positions.
3. Drive scripted input sequence over N simulation steps.
4. Dump observable state at each step (actor positions, velocities,
   trigger overlap sets, light pool state, chunk membership).
5. Compare dump against stored expected dump.

Determinism requirements (centralized in project-canon's determinism
contract; summary here):

- Simulation never reads wall-clock time; `dt` is a parameter.
- Simulation never reads random sources without a seeded RNG; seeds
  are inputs.
- Asset loading does not affect simulation timing; chunk transformations
  produce identical arrays for identical inputs.
- No parallel reductions in physics inner loops (order affects results).
  Parallelism in rendering, content processing, and baking is fine —
  outputs are not part of simulation state.

Each game maintains its own replay scenarios. Wok ships a baseline set
covering crate-level behaviors via the workspace integration tests.

### Level 3: Screenshot diff

Workspace-level harness alongside the deterministic replay. Steps:

1. Load known scene with known lighting.
2. Place camera at known position.
3. Render one frame.
4. Compare against stored reference image, pixel-by-pixel with small
   tolerance for GPU vendor variance.

Catches shader and renderer regressions that unit tests miss. Baseline
set covers each rendering pass (shadow, lit, sky, fog, post-process).

### Out of scope

- **Hardware-specific GPU bugs** — require running on affected hardware
  (Steam Deck, dev laptops); not caught in CI unless CI runs there.
- **Visual quality judgments** — "looks good" is a human call.
  Screenshot diff catches regression from known-good, not whether
  known-good is itself good.
- **Performance regressions** — deferred until real performance
  baselines exist. Separate benchmarking harness will be added then.
