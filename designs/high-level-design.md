# Engine Design — High-Level

A modular monolith Rust workspace for 3D action, platformer, and
open-world games at humble vertex counts. Target hardware: Steam
Deck and developer laptops.

Three layers, each may depend only on layers below:

- **Pantry** — cross-platform substrate (window, GPU, audio, input,
  frame loop, OS theme).
- **Wok** — opinionated 3D engine; ten library crates.
- **Game** (`<game>`) — owns the main loop, entity state, and
  simulation loops; composes Wok's primitives.

---

## Principles

**1. Placeholder-first authoring.** Games built on primitive shapes;
real meshes, voice, music, animations plug in later via an
automatic asset registry populated from scene contents. Terrain is
heightmap data per chunk at 1m × 1m resolution; the heightmap is
the data, not a placeholder.

**2. Earn its place.** Every subsystem justifies complexity against
scope and modest hardware. No nanite, no research rendering, no
PhD-grade techniques. When two designs deliver the same outcome,
the simpler wins.

**3. Authoring versus runtime separation.** Editing data structures
and runtime data structures are different. Transformations happen
at load/save time, never per frame.

**4. Built for one target.** The renderer has exactly one
configuration. Higher-end machines run the same game with
thermal/framerate headroom, not visual richness. Hardware
adaptation that doesn't change what's rendered (resolution,
framerate cap, audio output device) is allowed.

**5. Math and infrastructure in the engine; state and loops in the
game.** Engine provides pure math primitives (collision queries,
pose blending, lighting), data models, and infrastructure (file
I/O, GPU upload, asset registry, chunk lifecycle, hot reload,
authoring shell). Entity state (actors, game cameras, animation
playback drivers, save format, multiplayer transport) lives in
game code. Engine pools exist only for engine-resource budgets
(dynamic light pool for renderer limits, voice pool for mixer
limits); entity pools (actors) live game-side. Features with
well-known game-side patterns belong to game code: save/load UI,
settings menus, HUD, input remapping, networking, localization,
achievements, dialog trees, inventory, quest tracking, mod support,
tutorials, audio routing policy.

**6. Determinism is a contract.** Every engine crate honors the
determinism contract specified in `project-canon.md`. Load-bearing
for replay, save/load, and multiplayer.

**7. Code structured for AI implementation.** Built primarily by
Claude Code from prose plans. Small contracts at crate boundaries,
explicit data flow (no buried coupling), error types that surface
failure modes locally, target < 400 lines per file. Implicit
assumptions are where AI implementation drifts; explicit boundaries
prevent it.

---

## 1. Constituent Crates

A crate earns its boundary by being substantial in scope or
independent in shape.

### Pantry

- **`pantry`** — Window + surface, GPU device (wgpu), input event
  polling, gamepads (gilrs), audio output (cpal), frame loop driver,
  OS theme. Re-exports wgpu / winit / glam / bytemuck / cpal / gilrs
  so consumers pin one source of truth.

### Wok

Ten library crates, in dependency order.

- **`wok-scene`** — Scene and prefab data: chunked scenes, prefabs,
  region markers (fog and lighting zones), JSON serde, file watcher
  for hot reload. Defines all opaque handle types: `MeshId`,
  `AudioCueId`, `AnimationId`, `VoiceLineId`, `LightStateRef`,
  `ActorHandle`. Chunks are 128m × 128m (engine constant). Prefabs
  have named states (default, open, destroyed); each state is a list
  of shapes. A shape is a primitive (cube, ellipsoid, cylinder,
  capsule, plane) with transform plus `is_hitbox` and `is_visible`
  flags — three meaningful combinations: solid placeholder (both),
  trigger volume (hitbox only), visual-only placeholder (visible
  only). At chunk load, shapes are sliced into per-system arrays
  (visible, hitbox, trigger). Asset replacement happens at the state
  level. Pure data and serde; no runtime logic, no GPU.

  **Terrain.** Per-chunk heightmap, 1m × 1m resolution (129 × 129
  cells, shared-edge convention), ±32m vertical, u16-quantized to
  millimeter precision. Heightmap data lives in a sibling binary
  (`0_0.heightmap.bin` alongside `0_0.json`). Per-cell surface tags
  use the same intern-table pattern as shape surface tags. Sampling
  functions (`height_at`, `normal_at`, `surface_at`) live here —
  pure functions live with their data type.

- **`wok-physics`** — Pure math for moving things through 3D space.
  Collision primitives (capsule, AABB, ellipsoid, heightmap; swept
  queries with slide resolution), gravity integration as a function,
  camera math (orbit transform, spring-arm calculation). All pure:
  inputs → outputs. No actor pool, no integration loop. The game
  holds actors and calls these functions; the editor calls the same
  functions for placement, snap-to-surface, picking, overlap
  detection.

- **`wok-anim`** — Pure animation math: pose blending, layered
  blending, timeline evaluation, animation event resolution. Game
  owns the playback driver. Animation affects appearance, not
  movement — no feedback into simulation. Implementation deferred
  until placeholder animation needs more than transform tweens.

- **`wok-registry`** — Asset identity: slug↔serial map per asset
  kind, usage tracking (which scenes/prefabs reference which
  assets), JSON serde. Pure data + serde; no runtime state, no GPU.
  Populated automatically from scene contents. Identity only;
  concrete buffer data lives in domain crates (meshes in wok-mesh,
  audio in wok-audio, animation in wok-anim, light in wok-light).

- **`wok-mesh`** — Mesh runtime data: `MeshCpu` / `MeshGpu` types,
  primitive mesh generation (cube, sphere, cylinder, capsule,
  plane), terrain mesh generation from wok-scene's heightmap with
  smooth normals from heightmap gradients and per-vertex color
  variation from surface tags, GPU upload paths, GLTF loader (when
  real meshes arrive). Resolves `MeshId` from wok-registry.

- **`wok-content`** — Chunk lifecycle and streaming. Chunk loading
  state machine (pending → loading → loaded → unloading); streaming
  algorithm (engine-owned, declarative — scenes describe topology
  via chunk metadata, engine computes desired loaded set from that
  plus player position); background worker thread (`std::thread` +
  `mpsc`) for chunk transformations; Vista enforcement (chunks
  loaded for rendering but excluded from simulation, collision,
  trigger evaluation); hard cap of 32 chunks (engine constant); LRU
  eviction with priority weighting. Calls wok-scene's slicer,
  wok-mesh for terrain mesh, wok-registry for ID resolution.
  Eagerness modes: Eager (loads in radius), Lazy (loads on request),
  Vista (loaded but partially active). Eagerness transitions are
  flag flips, not reloads. Data-parallel internal work uses `rayon`.

  Triggers: wok-content owns volume data per chunk; wok-physics owns
  overlap math; game owns event routing.

- **`wok-audio`** — Audio playback. Voice pool with stable handles
  (engine-owned because it manages a mixer resource budget),
  distance attenuation and stereo panning for 3D positioning,
  category buses (master, music, sfx, voice — categories configured
  by game), audio buffer storage. Consumes wok-registry for
  AudioCueId → source path resolution. Audio positioning takes
  `Vec3`; game composes actor positions and passes them in. Reverb
  zones, environmental effects, music sequencing, audio routing
  policy are game-side.

- **`wok-light`** — Lighting data model with animation curves
  (animatable over time, switchable at runtime), offline static-
  light bake (precomputes baked light and ambient occlusion into
  vertex data or lightmaps, including terrain queried via
  wok-scene's sampling functions), budgeted dynamic light pool
  (engine-owned because it manages a renderer resource budget — the
  renderer can only handle N dynamic lights per frame).
  Importance-weighted with reserved player slots. Bake step queries
  geometry via wok-physics raycasts.

- **`wok-sequence`** — Cutscene and scripted-sequence data: timeline
  data, camera path data, animation event timing, dialog and audio
  cue timing. Data model and pure evaluators here. The driver that
  ticks a timeline and pumps events is game code. Authoring UI lives
  in wok-shell.

- **`wok-render`** — Renderer. Forward pipeline, single shadow map
  pass, lighting passes (consuming wok-light's baked + dynamic
  data), parametric gradient sky, fog, post-process. Consumes
  wok-scene, wok-anim, wok-light, wok-mesh, wok-content. Game
  supplies the render list each frame; wok-render doesn't read
  actor pools. Commitments:

  - **Cel shading.** Lighting quantized into discrete bands with
    smoothstep transitions, plus rim light for silhouettes.
    Per-scene tunables: band count (2–8, default 4), transition
    softness, rim intensity, ambient floor color, fog parameters.
    Terrain uses the same pipeline; heightmap-gradient normals yield
    smooth shading on faceted geometry.
  - **Alpha cutout transparency only.** No sorted blending, no OIT.
    Water, glass, smoke, effects use stylized cutout.
  - **Fog always on.** Parameters animated per lighting state; fog
    distance sets render distance. Fog color drives sky's horizon
    color.
  - **Parametric gradient sky.** Vertical gradient horizon→zenith
    from lighting state; sun disc billboard; optional fading stars;
    optional animated cloud plane. No sky textures.
  - **One shadow map per frame.** Scene or lighting state declares
    the active shadow source. Single-cascade projection sized to
    active chunk set; default 2048×2048, tunable. PCF soft edges.
    Baked lights bake shadows into the lightmap at bake time;
    dynamic pool lights don't cast shadows. Shadow code earns
    heavier Level 3 fixture coverage across sun angles and scene
    depths — highest-gotcha surface in the renderer.

- **`wok-shell`** — Authoring shell: egui integration, panel layout,
  viewport management, command palette, terrain painting tools
  (sculpt height, paint surface tags, cross-chunk paint),
  light/dark theme switching with OS-aware live switching, dockable
  panels. Uses wok-physics primitives for editor spatial queries
  (snap-to-surface, picking, overlap detection, drag along surface)
  — same primitives the game uses, no special-casing. Binary entry
  at `wok-shell/src/bin/wok.rs` produces the `wok` executable.

### Game

- **`<game>`** — All game-specific runtime logic: actor pool and
  fixed-timestep integration loop (calls wok-physics math), game
  cameras (calls wok-physics camera math), animation playback
  drivers, save/load format and composition over per-crate
  accessors, multiplayer transport (per multiplayer-model), physics
  tuning, player controller, input mapping, trigger event routing,
  effect routing through wok-light's pool, audio cue routing through
  wok-audio, HUD, menus. Binary entry at `<game>/src/main.rs`.

Game content (scenes, prefabs, lighting curves, audio cue tables,
asset registry) lives under `<game>/content/`, produced by the Wok
editor.

---

## 2. Dependency Graph

Topological order, lowest first. Each crate may depend only on the
listed crates.

- **`pantry`** — no internal dependencies.
- **`wok-scene`** — `pantry`.
- **`wok-physics`** — `pantry`, `wok-scene`.
- **`wok-anim`** — `pantry`, `wok-scene`.
- **`wok-registry`** — `pantry`, `wok-scene`.
- **`wok-mesh`** — `pantry`, `wok-scene`, `wok-registry`.
- **`wok-content`** — `pantry`, `wok-scene`, `wok-registry`,
  `wok-mesh`.
- **`wok-audio`** — `pantry`, `wok-scene`, `wok-registry`.
- **`wok-light`** — `pantry`, `wok-scene`, `wok-physics`.
- **`wok-sequence`** — `pantry`, `wok-scene`, `wok-anim`,
  `wok-light`, `wok-registry`, `wok-audio`.
- **`wok-render`** — `pantry`, `wok-scene`, `wok-anim`, `wok-light`,
  `wok-mesh`, `wok-content`.
- **`wok-shell`** — `pantry` and all `wok-*` library crates.
- **`<game>`** — `pantry` and all `wok-*` library crates.

### Forbidden edges

First three are Cargo-enforced; remainder are review-enforced.

1. No `wok-*` or `<game>` depends on `pantry` internals beyond its
   public API.
2. No `wok-*` depends on `<game>`.
3. `wok-scene` depends on no other `wok-*`.
4. `wok-render` does not depend on `wok-sequence` or `wok-shell`.
5. `wok-content` does not depend on `wok-render`, `wok-physics`,
   `wok-light`, `wok-sequence`, or `wok-audio`.
6. `wok-audio` does not depend on `wok-physics`. Audio positioning
   takes `Vec3`; game passes actor positions in.
7. `wok-anim` does not depend on `wok-physics`. Pose math is pure;
   game owns the driver and composes with actor state.
8. `wok-registry` depends only on `wok-scene`. Pure-data identity
   layer.
9. `wok-mesh` does not depend on `wok-content`, `wok-physics`,
   `wok-light`, `wok-sequence`, or `wok-audio`.
10. No cycles.

---

## 3. Data Flow

Wok is a library. Game owns the main loop, holds entity state, and
calls into Wok's functions.

Every piece of authored content passes through four states:

1. **Authored on disk.** JSON files; prefabs as stateful shape
   lists, scenes as chunked prefab placements with lighting and
   region metadata, lighting states as animation curves, terrain as
   sibling binary heightmap files, registry as its own JSON.
2. **Authored in memory.** Deserialized via `wok-scene` and
   `wok-registry` loaders. Editor mutates; game reads.
3. **Runtime arrays.** Produced by `wok-content::transform_chunk`
   (composes wok-scene slicing, wok-registry resolution, wok-mesh
   terrain generation). Authored form no longer referenced.
4. **Per-frame state.** Computed each frame from runtime arrays
   (frustum culling, collision contacts, lighting interpolation).
   Not durable.

Data flows downward only. Runtime never writes to authored.

### Transitions

| Transition | Crate(s) | Triggered by |
|---|---|---|
| Disk → Authored memory | `wok-scene::load_*`, `wok-registry::load` | Game when needed |
| Authored memory → Disk | `wok-scene::save_*`, `wok-registry::save` | Editor save |
| Authored memory → Runtime arrays | `wok-content::transform_chunk` | Game requests chunk load |
| Runtime arrays → Per-frame state | Each system's frame function | Game's main loop |
| Runtime arrays → Disposed | `wok-content::release_chunk` | Game requests chunk unload |

Transformation can run synchronously or on the background worker.

### Persistent state

The engine has no opinion on save format, multiplayer transport, or
world persistence. The engine exposes per-crate accessors for
current state (dynamic light pool from wok-light, voice pool from
wok-audio, chunk membership from wok-content, etc.). The game
composes a save file from those plus its own state (actors,
cameras, game logic). Save format, version handling, content-
version checks are game-side.

**Across-chunk-unload persistence** (dropped items remain, defeated
enemies stay defeated): game maintains a sidecar file alongside
authored data, queried during chunk load to override placements and
gate spawner-trigger firing. Actor entities persist naturally
because they live in the game's pool, not in chunk runtime arrays.

**Multiplayer**: see `multiplayer-model.md`. Game-layer composition
over engine primitives. Wok never sees the network.

### Hot reload

wok-scene's file watcher detects authored-data changes. Game polls
each frame and decides how to respond (typically by requesting
affected chunks be re-transformed). Hot reload is authored → runtime
only.

---

## 4. Validation

Three levels.

### Level 1: Unit tests (`cargo test`)

Per-crate commitments:

- **wok-scene** — JSON round-trip per type; shape-slicing
  determinism; heightmap round-trip; sampling-function fixtures.
- **wok-physics** — every primitive query against fixture geometry;
  integration determinism; camera math fixtures.
- **wok-anim** — pose blending and timeline evaluation fixtures.
- **wok-registry** — JSON round-trip; serial allocation; usage
  tracking; rename handling.
- **wok-mesh** — primitive mesh determinism; terrain mesh
  determinism; GPU upload path with mockable device.
- **wok-content** — chunk loading state machine; streaming algorithm
  against synthetic scenes; Vista enforcement; worker protocol
  round-trip.
- **wok-audio** — voice pool allocation/eviction; mixing math
  against fixtures; cue resolution via registry.
- **wok-light** — bake determinism; light pool eviction.
- **wok-sequence** — timeline evaluation against fixtures.
- **wok-render** — pipeline assembly and shader compilation. Visual
  correctness at Level 3.
- **wok-shell** — panel layout and command palette routing.
- **pantry** — mockable platform abstractions.

### Level 2: Deterministic replay harness

Game-owned, since loops are game-side. Engine provides deterministic
primitives; each crate's Level 1 covers its own determinism.
Workspace-level integration tests in `wok-engine/tests-integration/`
exercise composition through a minimal test game loop:

1. Load known authored scene.
2. Spawn actors at known positions.
3. Drive scripted input sequence over N steps.
4. Dump observable state at each step.
5. Compare against stored expected dump.

### Level 3: Screenshot diff

1. Load known scene with known lighting.
2. Place camera at known position.
3. Render one frame.
4. Compare against stored reference, pixel-by-pixel with small
   tolerance for GPU vendor variance.

Baseline covers each rendering pass; shadow pass gets extra fixture
coverage across sun angles and scene depths.

### Out of scope

- **Hardware-specific GPU bugs.** Require running on affected
  hardware; CI doesn't catch unless CI runs there.
- **Visual quality judgments.** Screenshot diff catches regression
  from known-good, not whether known-good is good.
- **Performance regressions.** Deferred until the engine runs a real
  scene end-to-end. Then perf becomes a continuous concern
  baselined against dev hardware (laptops first, Steam Deck second);
  no separate harness.
