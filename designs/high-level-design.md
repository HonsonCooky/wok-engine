# wok-engine - High-Level Design

## Approach

The bet is programming-first. Most engines exist to take code out of the pipeline: you assemble components, wire nodes, fill in variable sheets, and the game becomes a configuration. wok-engine takes the opposite trade. It is a toolbox, not a framework that removes programming: it owns the wheels no one should reinvent (windowing, the GPU, collision, scene data, audio, lighting), and the game is written on top as code. Development is asset-decoupled: the game runs against placeholder shapes and a scan of the project yields the list of assets still to be made, so content and code proceed in parallel.

## Shape

wok-engine is a 3D engine for one class of game: action, platformer, and open-world at modest fidelity. It is not general and does not do 2D. The narrow surface follows from a precise mission, but the target is a class of games rather than a single one: a substrate wider than one title and narrower than a general engine.

wok-engine is a Rust workspace, a modular monolith of small crates that form one engine. It targets developer laptops, with the Steam Deck as a secondary target; PS2-era vertex counts are a complexity-reduction choice, not a hardware ceiling.

Two layers, and the dependency points one way only:

- **wok-engine** - the toolbox: a set of crates with the platform substrate (window, GPU, audio, input, the frame loop) at the bottom and the 3D libraries above it. Reusable across games.
- **applications** - separate consumers that compose engine primitives; the engine never depends on any of them, so each is replaceable. Three exist today: the wok editor, which authors content and is co-developed in the workspace as the reference application; taste, the workspace's playable demo and feel laboratory, where movement, camera, and collision verdicts are tuned before the game inherits them; and the game, which runs the content and is its own downstream project.

The one hard rule: the engine never reaches up into an application. Everything else about how the crates relate falls out of the principles.

## Principles

**1. Placeholder-first.** Games are built on primitive shapes; real meshes, voice, music, and animation plug in later, referenced by name and resolved from the content folder when present. Terrain is heightmap data per chunk at 1m x 1m resolution; the heightmap is the data, not a placeholder.

**2. Earn its place.** Every subsystem justifies its complexity against scope and modest hardware. No nanite, no research rendering, no PhD-grade techniques. When two designs reach the same outcome, the simpler one wins. The engine does the few things its kind of game needs, and refuses the rest.

**3. Authoring and runtime are separate.** The data you edit and the data you run are different shapes. Transformations happen at load and save time, never per frame.

**4. One target.** The renderer has a single configuration. Hardware adaptation that does not change what is rendered (resolution, framerate cap, audio device) is allowed; a stronger machine runs the same game with headroom, not richer visuals.

**5. The engine provides tools; the game owns state and loops.** The engine provides pure math (collision queries, pose blending, lighting), data models, and infrastructure (file I/O, GPU upload, asset-name resolution, chunk lifecycle, hot reload). Entity state and the loops that drive it (actors, game cameras, animation drivers, save format, multiplayer transport) live in the game. Engine pools exist only for engine-resource budgets (the dynamic light pool, the voice pool); entity pools live game-side. Anything with a well-known game-side pattern belongs to the game: save and load UI, settings, HUD, input remapping, networking, localization, achievements, dialog trees, inventory, quest tracking, mod support, tutorials, audio routing.

**6. Determinism is a contract.** Every engine crate honors the determinism contract in `project-canon.md`, load-bearing for replay, save/load, and multiplayer.

**7. Built to be implemented by Claude Code.** The engine is built primarily by Claude Code from prose plans, so its boundaries are explicit: small contracts at crate edges, data flow on the surface rather than buried in coupling, error types that surface failure where it happens, and a target of under 400 lines per file.

## Crates

A crate earns its boundary by being substantial in scope or independent in shape. Listed in dependency order, substrate first. The built crates carry real detail; for the unbuilt ones the responsibility and concepts are fixed here, and the tuning and mechanism are settled in each crate's plan when it is built.

- **`wok-platform`** - the substrate at the bottom of the engine: window and surface, GPU device (wgpu), input polling, gamepads (gilrs), audio device discovery (cpal; playback lands with wok-audio), the frame loop. It re-exports wgpu, winit, bytemuck, cpal, and gilrs so every consumer pins one source of truth. The scope rule: window, GPU, audio, input, and the frame loop are needed by every interactive application, so they belong here; scene graphs, game models, mesh tools, and lighting are needed by only some, so they do not. (OS theme is detected by the editor's egui directly; the substrate carries no theme plumbing.)

- **`wok-scene`** - scene and prefab data: chunked scenes, prefabs, region markers (fog and lighting zones), JSON serde, the project content layout (the `assets/` folder convention: path resolution plus a tolerant discovery scan; see "content conventions and integrity" below), and a file watcher for hot reload. Asset references are by name, resolved to files at load; the only generated identity is a per-scene monotonic instance id on each placement, which game logic and triggers point at. A placement carries spatial and physical data and identity only - transform, asset reference by name, the instance id, and an optional author name - and never gameplay fields; the game owns interpretation and binds any per-instance logic or configuration to a placement by id or name, appending that configuration on its own side, so the scene format stays independent of any game's schema. There is no serial registry: content is the source of truth, and dead or missing references are found by scanning. Chunks are 128m x 128m. Prefabs have named states (default, open, destroyed), each a list of shapes; a shape is a primitive (cube, ellipsoid, cylinder, capsule, plane) with a transform, an optional surface tag, and is_hitbox / is_visible flags, giving solid, trigger-only, or visual-only placeholders. Primitives are unit-sized and positioned by the transform (the unit cube spans +/-0.5m, so scale reads as metres); wok-scene owns the canonical definition of this convention (the unit half-extent and a `Primitive`'s `unit_aabb`), and wok-physics bounds and wok-mesh meshes reference it so colliders and drawn geometry agree. Capsule and cylinder are placeholder-grade at unit scale. At chunk load, shapes are sliced into per-system arrays (visible, hitbox, trigger). Pure data and serde; no runtime logic, no GPU.

  Terrain is a per-chunk heightmap at 1m x 1m resolution (129 x 129 cells), +/-32m vertical, u16-quantized to millimetre precision, stored in a sibling binary. The heightmap is optional per chunk: a chunk with no heightmap (interiors, platforms, constructed levels) is placements-only, and the terrain consumers (mesh, collision, sampling, render) treat its absence as no terrain, the level resting on prefab geometry. Per-cell surface tags use the same intern-table pattern as shape tags. Sampling functions (`height_at`, `normal_at`, `surface_at`) live here.

- **`wok-physics`** - pure math for moving things through 3D space: collision against static colliders (AABB, oriented box, sphere, vertical cylinder, heightmap terrain) classified from authored shapes, swept by a moving capsule or a moving flat-bottomed vertical cylinder (the player's collider), with slide resolution available as engine API and the moving-shape rests on terrain; gravity integration as a function; and camera math (orbit transform, spring-arm calculation). Tilted round shapes classify to conservative boxes; a true swept ellipsoid is deferred until something needs it. All pure: inputs to outputs, no actor pool, no integration loop. The game holds actors and calls these functions (taste composes its own slide policy over the raw sweeps, per principle 5); the editor calls the same functions for placement, picking, and snapping.

- **`wok-mesh`** - mesh runtime data: `MeshCpu` and `MeshGpu` types, primitive mesh generation, terrain mesh generation from wok-scene's heightmap (smooth normals from gradients, per-vertex colour from surface tags), GPU upload paths, and a glTF loader when real meshes arrive. Resolves mesh names to buffers at load.

- **`wok-anim`** - pure animation math (pose blending, timeline evaluation), consumed by a game-owned playback driver. Animation affects appearance, not movement, so it needs nothing from physics. Deferred until placeholders need more than transform tweens.

- **`wok-content`** - chunk lifecycle and streaming. Built today: the synchronous part 1 lifecycle (transform chunks by composing wok-scene's slicer and wok-mesh terrain; load, hold, release). Deferred to part 2: the desired-set computation from scene topology plus player position, the background worker, eviction, and enforcement of the three eagerness modes: Eager (loads in radius), Lazy (loads on request), Vista (loaded for rendering but excluded from simulation, collision, and triggers); the eagerness enum exists as authored data now. For triggers, wok-content owns the volume data per chunk, wok-physics owns the overlap math, and the game owns event routing.

- **`wok-audio`** - audio playback: an engine-owned voice pool, 3D positioning, and category buses configured by the game. Positioning takes a `Vec3` and the game passes actor positions in, so it needs nothing from physics. Reverb, environmental effects, music sequencing, and routing policy are game-side.

- **`wok-light`** - the lighting data model (animatable, switchable at runtime; built today) and, deferred until their consumers exist, an offline static-light bake (gated on wok-physics raycasts) and an engine-owned dynamic light pool.

- **`wok-sequence`** - cutscene and scripted-sequence data and pure evaluators; the driver that ticks a timeline is game code, and the authoring UI is in the editor.

- **`wok-render`** - the renderer: a forward pipeline, a single shadow map pass, lighting passes over wok-light's baked and dynamic data, a parametric gradient sky, fog, and post-process. It consumes wok-scene, wok-anim, wok-light, wok-mesh, and wok-content, and the game supplies the render list each frame. Visual commitments:
  - **Banded lighting, authored per scene.** Lighting quantized into discrete bands with smoothstep transitions and rim light for silhouettes. Band count is an authored per-scene value with no upper clamp: low counts (2-8) read as cel, high counts (16+) read as smooth shading. The current authored look is smooth (32 bands); the band machinery, rim, ambient floor, and fog are the mechanism at any count. Terrain uses the same pipeline.
  - **Alpha cutout transparency only.** No sorted blending; water, glass, smoke, and effects use stylized cutout.
  - **Fog is a per-scene lighting config, optionally off.** Render distance is the scene's streaming extent (`load_radius`), independent of fog; the sky horizon is the gradient sky's own colour.
  - **Parametric gradient sky.** A horizon-to-zenith gradient from the lighting state, a sun disc, optional stars and a cloud plane; no sky textures.
  - **One shadow map per frame.** The scene or lighting state declares the active shadow source; baked lights bake shadows in, dynamic lights do not cast. Shadow accuracy earns extra Level 3 coverage.

- **content conventions and integrity** - an engine concern, not the editor's. The engine fixes opinionated naming and folder conventions, and that small predictable surface is what lets references resolve by name with no registry and what any tool can scan. The layout is now built and engine-owned in wok-scene (`ContentLayout`): a project root holds an `assets/` folder, and under it `scenes/<scene>/` is one self-contained folder per scene (its `scene.json`, the `{x}_{z}.json` chunk files, and the sibling `{x}_{z}.heightmap.bin` terrain), `prefabs/<slug>.json` holds the project-shared prefabs, and `lighting/<name>.json` the project-shared light states (scene-owned lighting is still parked). Opening a project is a read-only scan of `assets/`, with no manifest, no gate, and no error: a folder with no `assets/` (or an empty one) is just an empty project, there is no default scene (a scene is opened explicitly), and the `assets/` tree is created lazily when content is first saved, never on open. wok-scene provides the path resolution and a tolerant discovery scan that lists scene, prefab, and lighting names sorted for determinism, with a missing `assets/` or subdirectory scanning to an empty list. The integrity tooling layered on top is still deferred: per-scene reference checks, a missing-assets scan (the to-build list), and a project-wide deep scan for dead references, orphans, and empty slots. The editor and anything else consume this surface; none owns it.

The applications sit above the engine, each a separate consumer:

- **the wok editor** (`wok`) - the GUI for authoring content. It composes engine primitives, including the same wok-physics queries the game uses for picking and snapping, and will surface the content scan as its missing-assets and integrity views once the scan exists (today: a disabled page slot reserved for it); it owns no engine logic. Co-developed in the workspace as the reference application (the `wok` binary), and replaceable without the engine noticing.
- **`<game>`** - all game-specific runtime logic: the actor pool and its fixed-timestep loop (calling wok-physics math), game cameras (calling wok-physics camera math), the player controller, save/load composed over per-crate accessors, multiplayer transport (per `multiplayer-model.md`), and routing of triggers, effects, and audio cues into the engine's systems. Content lives under the game's content folder, produced by the editor; asset source files live alongside, named by slug.

Demo content during co-development: the only content the workspace holds today is taste's demo, which lives in-repo under `taste/assets/` (git-tracked, following the same `assets/` convention as any project) with the editor opening `taste/` to author it; relocating taste and its content to the game's own downstream repo is deferred until the engine stabilizes (decided 2026-06-25).

## Dependencies

The dependency graph is whatever the crates that exist actually use; a crate's dependencies are not declared before the crate exists. Three rules govern it, enforced by the toolchain rather than by review:

1. The engine never depends on an application. The game is a separate project, and the editor is a binary that no library can depend on.
2. wok-scene depends on no other engine crate. It is pure data and owns its own math, so everything else can build on it without pulling the world in.
3. No cycles.

Everything else follows from the principles and the crate descriptions above. What exists today, lowest first:

- **`wok-platform`** - no internal dependencies.
- **`wok-scene`** - no internal dependencies.
- **`wok-light`** - no internal dependencies.
- **`wok-physics`** - `wok-scene`.
- **`wok-mesh`** - `wok-scene`, `wok-platform` (taken when MeshGpu landed with the renderer).
- **`wok-content`** - `wok-scene`, `wok-mesh`.
- **`wok-render`** - `wok-platform`, `wok-mesh`, `wok-light`, `wok-scene` (taken with the shadow pass, for the canonical Aabb).

As later crates land, each takes on what it genuinely uses under the three rules, and this list grows to match.

## Data flow

The engine is a library: the game owns the main loop, holds entity state, and calls into engine functions. Authored content is the starting values the editor produces, and every piece passes through four states:

1. **Authored on disk.** JSON files (prefabs, scenes, lighting states) plus sibling binary heightmaps; asset source files named by slug, referenced by name.
2. **Authored in memory.** Deserialized through wok-scene loaders. The editor mutates this form; the game reads it.
3. **Runtime arrays.** Produced by `wok-content::transform_chunk`, composing wok-scene slicing, asset-name resolution, and wok-mesh terrain generation. The authored form is no longer referenced.
4. **Per-frame state.** Computed each frame from the runtime arrays (culling, contacts, lighting interpolation). Not durable.

Data flows downward only; runtime never writes back to authored.

| Transition | Crate(s) | Triggered by |
|---|---|---|
| Disk to authored memory | `wok-scene::load_*` | Game, when needed |
| Authored memory to disk | `wok-scene::save_*` | Editor save |
| Authored memory to runtime arrays | `wok-content::transform_chunk` | Game requests chunk load |
| Runtime arrays to per-frame state | each system's frame function | Game's main loop |
| Runtime arrays to disposed | `wok-content::release_chunk` | Game requests chunk unload |

**Persistent state.** The engine has no opinion on save format or world persistence; it exposes per-crate accessors for current state, and the game composes a save from those plus its own. Cross-unload persistence (dropped items, defeated enemies) is game-side, since actors live in the game's pool rather than chunk arrays. Multiplayer is game-layer composition over engine primitives (see `multiplayer-model.md`); the engine never sees the network.

**Hot reload.** wok-scene's file watcher detects authored-data changes; the game polls each frame and decides how to respond, typically by re-transforming affected chunks. Hot reload is authored-to-runtime only.

## Validation

Three levels.

**Level 1: unit tests (`cargo test`).** Each crate covers its own determinism and core invariants: serde round-trips and slicing determinism for wok-scene, primitive queries and integration determinism for wok-physics, mesh determinism for wok-mesh, the scan's dead, missing, and orphan detection, and so on as each crate lands.

**Level 2: deterministic replay harness.** Game-owned, since the loops are game-side; the engine provides deterministic primitives and each crate's Level 1 covers its own determinism. A workspace-level integration target loads a known scene, drives a scripted input sequence over N steps, dumps observable state each step, and compares against a stored expected dump. (It lives in taste, its workspace home; wok-physics deliberately retains the original locomotion replay as the engine-side pin.)

**Level 3: screenshot diff.** Load a known scene with known lighting, place the camera at a known position, render one frame, and compare against a stored reference within a small tolerance for GPU vendor variance. The shadow pass gets extra coverage across sun angles and scene depths.

**Out of scope.** Hardware-specific GPU bugs need the affected hardware. Visual quality judgments are not testable: the diff catches regression from a known-good frame, not whether that frame is any good. Performance is deferred until the engine runs a real scene end to end, then baselined against dev hardware (laptops first, Steam Deck second).
