# Wok Engine History

Project log for the wok engine. Captures the load-bearing decisions, milestones, and open
items that are not readable from code or git history alone. New sessions read this file to
pick up continuity.

## Current state

As of 2026-05-26:

- pantry `v0.1.0-pantry-baseline` (HonsonCooky/pantry @ 034fbae) and wok-scene
  `v0.1.0-wok-scene-baseline` (HonsonCooky/wok-engine @ 118f9fb) are the paired baselines.
- wok-scene has 94 tests across 8 suites, `cargo clippy --all-targets -D warnings` clean.
  Tests verified on Windows 11 only; Linux and macOS watcher tests are outstanding.
- `designs/wok-content-plan.md` has been drafted but is not yet reviewed. Implementation of
  wok-content does not start until the plan is approved.
- Next work after the plan review: wok-content implementation.

## Repository layout

Three sibling git repos under `~/Projects/HonsonCooky/`:

- `pantry/` -> HonsonCooky/pantry.
  Platform substrate: window, GPU (wgpu), audio (cpal), input (winit + gilrs), math
  (glam re-exports plus `Transform` and `Aabb` with serde). No engine concerns.

- `wok/` -> HonsonCooky/wok-engine.
  Engine workspace. Hosts wok-* library crates and (eventually) `examples/smoke-test`.
  Local directory is named `wok/`; the GitHub repo is `wok-engine` for discoverability.
  Path dependency on pantry as `{ path = "../pantry" }`; the directory and repo name
  difference is harmless.

- `<game>/` -> not yet decided.
  Future first game. Each game is its own repo with path deps on both pantry and wok.

Design documents live in `wok/designs/` (this repo). Format rules and philosophy directives
are in `CLAUDE.md` and `philosophy.md` at this repo's root, copied verbatim from
`dotfiles-windowsos/`.

## Architectural decisions

Decisions that are load-bearing for future work but not obvious from reading the code.

### Three-layer dependency order

pantry -> wok-* -> game. Each layer reduces the design space of the one below. Engine
crates depend only on pantry. Games depend on pantry and wok-* crates. No layer depends
upward.

### Five engine principles

From `designs/high-level-design.md`. Every wok-* feature must answer to all five before
landing:

1. Placeholder-first authoring.
2. Earn its place (no nanite, no PhD rendering; tie goes to simpler).
3. Authoring versus runtime separation (transform at load/save, not per frame).
4. Built for one target (Steam Deck and developer laptops).
5. Primitives, not features (HUD, save UI, networking, etc. are game code).

Pantry is unopinionated by definition; the five do not apply to it.

### "unstitched" is retired

The previously-named first game ("unstitched") was tied to an earlier plan and is no longer
the committed first game. The in-tree `examples/smoke-test` (planned, not yet created)
replaces it as the end-to-end validation harness. Game references in code, docs, or commits
use `<game>` as a placeholder until a real name is committed.

### Pantry holds opinions on JSON shape

`Transform` serializes as `{ "pos": [...], "rot": [...], "scale": [...] }` with `rot` and
`scale` skipped when equal to their identity defaults. `Aabb` serializes as
`{ "min": [...], "max": [...] }`. Both are implemented as manual serde impls in pantry, so
every wok-* crate that uses these types inherits the same shape. Consumer crates do not
override.

### glam serde feature: deliberately NOT enabled

wok-scene needs `Vec3` and `Vec2` serialization in `ShapePrimitive`. Rather than enable
glam's serde feature in pantry (which would force a JSON shape on every glam type in
pantry's universe whether wanted or not), wok-scene has local `vec3_array` and `vec2_array`
helper modules in `wok-scene/src/authored/shape.rs` and uses `#[serde(with = ...)]` per
field. Hoist these helpers to `pantry::math` when a second wok-* crate needs the same
shape.

### Asymmetric `Versioned<T>` in load/save

serde's `flatten` and `deny_unknown_fields` do not cooperate (this is documented in serde).
wok-scene therefore uses:

- `Versioned<&T>` with `flatten` on the SERIALIZE side. `deny_unknown_fields` is irrelevant
  on output, so flatten works cleanly.
- A parse-to-Value, validate-`_format`, strip-`_format`, deserialize-rest pattern on the
  LOAD side. Authored types keep their `deny_unknown_fields` posture because they never see
  the `_format` field directly.

Documented in `wok-scene/src/serde_format.rs`.

### BTreeMap, not Vec, for ordered map data

`PrefabState.audio_cues` is `BTreeMap<String, AudioCueId>`. Spec deviation from
`designs/wok-scene-plan.md` section 3 (which had `Vec<(String, AudioCueId)>`). Rationale:
BTreeMap sorts on serialize and rejects duplicate keys, so the plan section 4 "sorted on
save" determinism rule is enforced by the type rather than by manual sort calls. Any future
map-shaped authored data should default to BTreeMap for the same reasons.

### Asset ID equality is `serial`-only

`MeshId`, `AudioCueId`, `AnimationId`, `VoiceLineId`, and `LightStateRef` all have
`{ serial, slug }` shape. Equality, hashing, and ordering are based on `serial` only; the
slug is debug surface, not identity. This makes asset rename safe: an existing reference
compares equal across a rename because the serial does not change. All five are produced by
a `define_asset_id!` declarative macro in `wok-scene/src/ids/assets.rs`. Manual
`#[derive(PartialEq)]` on any of these types would be a regression.

### Slicer is position-independent

`slice_chunk` never composes `chunk.coord.to_world_offset()` into the runtime transforms.
Runtime arrays are chunk-local; consumers compose the world offset at draw or query time.
This is the property the parallel-worlds multiplayer model depends on: the same authored
chunk file produces bit-identical runtime arrays on every client, regardless of where the
chunk sits in the world. Test
`tests/slice.rs::t10_position_independent_across_chunk_coords` is the canary.

### File format header (`_format`) and version bumping

Every authored JSON file carries `_format: 1` at the top level. The loader refuses other
values via `LoadError::UnsupportedVersion`. A future bump will ship with a one-shot
`load_*_legacy` migration function; the engine commits to forward-incompatibility, not
forward-compatibility.

### Workspace lints: per-crate for now

`[workspace.lints]` is not used in `wok/Cargo.toml` yet. wok-scene has its own
`clippy::pedantic` plus a targeted allow list mirroring the noise filter from pantry. When
the second wok-* crate inherits the same allows, hoist to `[workspace.lints]`.

### Saved JSON is pretty-printed

`save_*` uses `serde_json::to_string_pretty`. Authored files are read by humans (level
designers, future-you, git diffs); compact JSON is an ergonomics regression. Round-trip
determinism still holds because pretty output is deterministic given the same input.

## Checkpoint log

### pantry baseline (HonsonCooky/pantry @ 034fbae, v0.1.0-pantry-baseline)

Realignment from previous-engine state. Removed binary-shipping CI workflow, added
`Transform` and `Aabb` with serde, re-exported serde and serde_json, copied `CLAUDE.md`
and `philosophy.md`.

Pre-existing tag preserved on remote: `v0.1.3` (older lineage; not part of the wok-engine
versioning scheme).

### wok-scene baseline (HonsonCooky/wok-engine @ 118f9fb, v0.1.0-wok-scene-baseline)

Bedrock data-model crate complete. Eight public modules (`ids`, `authored`, `runtime`,
`slice`, `load`, `save`, `watcher`) plus private `serde_format`. 94 tests across 8 suites:

- `tests/asset_ids.rs` (15)
- `tests/file_io.rs` (21)
- `tests/integration.rs` (2)
- `tests/round_trip.rs` (20)
- `tests/slice.rs` (17)
- `tests/validate.rs` (6)
- `tests/watcher.rs` (10)
- inline `src/ids/chunk.rs` (3)

`cargo clippy --all-targets -D warnings` clean.

The slicer's position-independence property (high-level-design section 5, property 5) is
the unit-test embodiment of the parallel-worlds multiplayer determinism story.

## Open items

- **Cross-platform watcher verification.** `wok-scene/tests/watcher.rs` has been run on
  Windows 11 only. Linux (inotify) and macOS (FSEvents) verification is outstanding.
  Atomic-save patterns produce different raw event sequences across platforms; the
  debouncer should absorb most of this, but verify before declaring the watcher complete.
  Documented in `wok-scene/README.md`.

- **wok-content plan review.** `designs/wok-content-plan.md` is drafted but not yet
  approved. Implementation does not start until the plan is reviewed.

- **`examples/smoke-test` crate.** Not yet created. Adds as a wok workspace member when
  there is enough engine to demonstrate end-to-end. Discipline: minimum viable
  demonstration of every subsystem, no gameplay creep, depends on the engine only through
  public APIs.

- **Workspace `[lints]` config.** Defer until a second wok-* crate proves the need.

- **glam serde helpers hoist.** `vec3_array` and `vec2_array` in
  `wok-scene/src/authored/shape.rs` should move to `pantry::math` when a second wok-*
  crate needs raw Vec3 serialization.
