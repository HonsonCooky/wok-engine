# wok-scene v0.2.0 - Plan vs Reality

Implementation memo for the v0.2.0 terrain addition. Branched from
`v0.1.0-wok-scene-baseline`; six steps landed as six commits; all 128 tests pass; clippy
clean under `-D warnings` on all targets.

This memo follows the §11 step ordering (8 through 13). For each step it states what the
plan specified, what landed, and any drift. Drift is documented in line, not deferred.

---

## Step 8 - authored/terrain.rs + sibling binary format

**Plan:** `TerrainData` type, sibling binary format (WTRN magic, header + sorted surface tag
table + heights + surface_indices), extend `load_chunk`/`save_chunk` to read/write the
sibling, add `LoadError::TerrainSiblingMissing` and `LoadError::TerrainMalformed`. Round-trip
tests.

**Implemented:** all of the above, plus:

- **`heightmap_file` is a struct field, not a derived value.** Plan §3 lists `TerrainData`'s
  fields as heights, surface_indices, surface_tags, vertical_range_meters - but the JSON shape
  is `{"heightmap_file": "..."}`, which means the filename has to live somewhere. I added it
  as a public field on `TerrainData`. Custom `Serialize`/`Deserialize` emit and parse only
  that field; the heightmap bytes live in the sibling binary and are read/written by
  `load_chunk`/`save_chunk`. The alternative - splitting `TerrainData` (in-memory) from
  `TerrainRef` (on-disk JSON) - would have forced `Chunk` to lose its derive(Serialize) since
  its terrain field type would diverge from the JSON shape. The single-type approach keeps
  Chunk's existing serde derive, which the v0.1.0 round-trip tests depend on.

- **Sorted surface tags + index remap happens in `write_sibling`, not in `save_chunk`.** The
  in-memory `TerrainData` is not mutated. The save path sorts a local copy of the tags,
  computes a permutation, rewrites surface_indices through it, and writes the result. Two
  saves of equivalent in-memory data produce byte-identical binaries regardless of authored
  tag order.

- **Path security stronger than plan.** Plan §4 says absolute paths in `heightmap_file` are
  rejected at load. I went further: also rejected any path with a directory component
  (`/` or `\`). A chunk JSON cannot reference a heightmap outside its own scene directory.
  Defense in depth; cheap to enforce. Drift, intentional adaptation.

- **Atomic save crash recovery drift.** Plan §9 "Atomic save crash recovery" says
  `save_chunk` "writes both the JSON and the sibling binary using the existing
  temp-file-and-rename pattern." No such pattern exists in v0.1.0 - the existing save path is
  straight `std::fs::write`. I matched the v0.1.0 style (straight write for both binary and
  JSON, no temp-rename). The plan §9 atomicity claim was aspirational, not how the code is.
  The ordering invariant the plan really cares about (binary first, then JSON) is preserved.
  If a future contributor wants atomicity, retrofitting temp-file-and-rename onto both writes
  is a small, separable PR.

Tests in `wok-scene/tests/file_io.rs`:
`terrain_chunk_round_trips_and_byte_identical_files` (§7 v0.2.0 round-trip),
`chunk_without_terrain_omits_field_in_json` (v0.1.0 byte-identity preserved),
`terrain_surface_tags_sorted_on_save` (with index-remap verification),
plus error-variant coverage: missing sibling, absolute path, directory component, bad magic,
truncated binary, unsupported binary version.

All existing `Chunk { ... }` literals across the test files updated to set `terrain: None`.
Mechanical change; no logic delta.

---

## Step 9 - runtime/terrain.rs + ChunkRuntime.terrain field

**Plan:** `RuntimeTerrain` type (heights, surface_indices, width, vertical_range_meters);
`ChunkRuntime` gains `terrain: Option<RuntimeTerrain>`. Types only.

**Implemented:** matches plan exactly. The slicer at this checkpoint unconditionally yields
`terrain: None`; the terrain merge pass lands in step 10. The field exists so the type shape
is stable across steps 9 and 10.

---

## Step 10 - slice.rs terrain pass + surface table merge

**Plan:** extend `slice_chunk` with a `slice_terrain` helper. Authored terrain tags appended
to the runtime intern table (after prefab tags); per-cell surface_indices rewritten through
the resulting remap. Add `SliceError::TerrainSurfaceTableOverflow`. Tests 14-18.

**Implemented:** matches plan. Some notes:

- Added a `len()` method to the private `StringInterner` so the overflow error can report
  `prefab_tag_count` (the table size before terrain merge). Trivial extension; doesn't change
  the intern semantics.

- Overflow triggers when the merged table would exceed `u16::MAX` entries. The runtime intern
  table holds exactly 65536 entries (u16 indices 0..=u16::MAX); the 65537th intern attempt is
  what fails. Test t18 constructs 65537 unique tags to exercise this. The test takes ~12s
  under debug due to the O(n^2) linear-scan intern; the plan §5 design note acknowledges this
  is acceptable because real chunks have a handful of tags.

- Out-of-range authored surface indices (corrupt in-memory `TerrainData`) pass through
  unchanged rather than threading a new error variant. The binary loader already rejects
  out-of-range indices at the I/O boundary, so this path is only reachable for
  hand-constructed in-memory data - same posture as the existing slicer's trust in
  shape-flag combinations on the placement side.

Tests in `wok-scene/tests/slice.rs`: t14 (smoke), t15 (position-independent), t16
(deterministic), t17 (surface table merge with explicit remap verification), t18 (overflow).

---

## Step 11 - sampling.rs (height_at, normal_at, surface_at)

**Plan:** three pure samplers, all taking `&ChunkRuntime` so call sites have a uniform shape
(plan §9 "Sampling signature uniformity"). 1-cell gradient for `normal_at`. Domain `[0, 128]`
closed-closed.

**Implemented:** matches plan. Notable drift:

- **Normal computation: central-difference, not the §9 errata pseudocode.** Plan §9 "Normal
  computation method" gives a pseudocode using `height_at_cell(ceil_x, ...)` (undefined
  symbol) and a `ceil_x`/`floor_x` framing that is not standard central-difference. The
  prompt explicitly directed: "Implement the correct central-difference form; flag in the
  memo." Implemented form: at sample `(x, z)`, evaluate heights at `(x-1, z)`, `(x+1, z)`,
  `(x, z-1)`, `(x, z+1)` (each through the bilinear `sample_height`), compute
  `dh_dx = (h_xp - h_xm) / (xp - xm)`, same for `dh_dz`. Normal is
  `normalize(Vec3::new(-dh_dx, 1.0, -dh_dz))`. At the chunk boundary, the neighbor that would
  fall outside `[0, 128]` is clamped to the boundary itself, producing a one-sided
  (forward or backward) difference with `dx_span = 1.0` instead of `2.0`; dividing by the
  actual span gives the correct slope either way. `normal_at`'s domain therefore equals
  `height_at`'s, no surprise narrowing.

- **`LightStateRef::default_for_test()` does not exist.** Plan §7's sampling fixture pattern
  referenced this helper; v0.1.0 doesn't define it. I substituted the working
  `LightStateRef::new(slug("l"), 1)` pattern used throughout the existing tests. Drift in the
  plan; not a code drift.

- **Quantization clamping caveat in test design.** The slope test (t `normal_at_sloped_terrain_tilts`)
  uses slope = 0.1 m/m rather than a steeper slope. Heights above `vertical_range_meters`
  clamp during quantization, which would corrupt the gradient near the clipped region. With
  VR = 32m and slope 0.1, `h(128) = 12.8` stays well within range; the central-difference at
  any sample reflects the actual slope. Documented in the test.

Tests in `wok-scene/tests/terrain_sampling.rs`: every §7 sampling case plus NaN handling,
boundary one-sided difference, surface_at floor-based indexing, and unresolvable-tag-index
returns None (locks the "surface rather than panic" contract).

---

## Step 12 - watcher.rs heightmap classification

**Plan:** classify `{i}_{j}.heightmap.bin` files inside scene directories; coalesce into
`FileEvent::ChunkChanged` (no new variant); error on unparseable filenames.

**Implemented:** matches plan exactly. Added a private `heightmap_stem(filename) ->
Option<&str>` helper that matches the suffix `.heightmap.bin` ASCII case-insensitively (same
posture as the existing `.json` matching). Even a removed heightmap binary emits
`ChunkChanged` - no separate "removed" handling - per the plan's coalescing rule.

Tests in `wok-scene/tests/watcher.rs`: t8 (modification emits ChunkChanged), t9 (unparseable
filename emits Error).

---

## Step 13 - integration tests + this memo + version bump + tag

**Plan:** end-to-end tests `full_workflow_load_slice_sample` and
`hot_reload_terrain_modification`. Memo. Bump Cargo.toml to 0.2.0. Tag `v0.2.0-wok-scene`.
Push.

**Implemented:** both integration tests in `wok-scene/tests/integration.rs`. The hot-reload
test writes the initial heightmap, installs the watcher, modifies the heightmap in memory and
re-saves, polls for `ChunkChanged`, re-loads and re-slices, then samples the modified
position to verify the change flowed through. Same timing posture (50ms install settle,
250ms post-action settle) as the v0.1.0 hot-reload test.

The memo is this document. The version bump and tag follow.

---

## Pinned guardrails

The plan §9 flagged three explicit invariants. Each is preserved as written:

1. **Shared-edge convention - no slicer-level verification** (§9, line "Pinned: future
   implementers should not add slicer-level verification."). Preserved: `slice_terrain` reads
   only the chunk's own authored data, never inspects or reaches into neighbor chunks.
   `chunk.coord` is read only for the overflow error message, never for transform composition
   or boundary checks. Boundary consistency between adjacent chunks remains the editor's
   responsibility.

2. **Don't smooth across chunk boundaries** (§9 Normal computation method, line "Don't smooth
   across chunk boundaries"). Preserved: the 1-cell central-difference implementation does
   not reach across boundaries. At `x = 0` or `x = 128`, the implementation falls back to a
   one-sided difference using only the chunk's own data. The plan notes this guardrail is
   really a 3-cell-window concern; for the 1-cell implementation in place, the constraint is
   trivially satisfied because the stencil never extends past the chunk's own cells anyway.

3. **Sampling signature uniformity - don't fragment the signatures** (§9, line "Pinned: don't
   fragment the signatures later."). Preserved: all three public samplers (`height_at`,
   `normal_at`, `surface_at`) take `&ChunkRuntime` as the first argument. The module's
   internal `sample_height(&RuntimeTerrain, ...)` helper is `fn`-private (not `pub`, not
   `pub(crate)`) and not re-exported; consumers can only reach the samplers through the
   uniform-shape public API.

---

## Other drift surfaced

- **`tracing` instrumentation check from v0.1.0 baseline.** Prompt asked to flag any existing
  `tracing` usage encountered. None found - the only hit for "tracing" in `wok-scene/` is the
  string "back-tracing" inside a doc comment on `runtime/shape.rs`, which is a comment about
  debugging utility, not instrumentation. No drift on the "zero tracing" constraint.

- **Plan §3 `TerrainRef` reference vs implementation.** Plan §3 mentions
  `chunk.terrain = Some(TerrainRef)` in the load_chunk description but the struct definition
  shows `Option<TerrainData>`. The implementation uses `Option<TerrainData>` with the
  reference fields collapsed into `TerrainData` (the `heightmap_file` field). No separate
  `TerrainRef` type. Notedeviation from the plan's intermediate prose; the structural
  definition was the source of truth.

- **Pedantic-clippy allows on test file.** `tests/terrain_sampling.rs` carries
  `#![allow(clippy::cast_precision_loss, clippy::cast_sign_loss,
  clippy::cast_possible_truncation)]` because the test does u32-to-f32 conversions for cell
  coordinates. The lib-side sampling code carries narrower per-function allows for the same
  reason. wok-scene's `Cargo.toml` `[lints.clippy]` already allows `cast_possible_truncation`
  globally; the others are local to where they're needed.

---

## Exit criteria

- 128 tests pass: 31 file_io + 22 slice + 15 terrain_sampling + 4 integration + 12 watcher +
  20 round_trip + 6 validate + 3 asset_ids + 15 lib unit + 0 doc.
- `cargo clippy --all-targets -- -D warnings` clean.
- Six commits on `feature/wok-scene-v0.2.0`, one per step (8-13 + this memo as part of 13).
- v0.1.0-wok-scene-baseline behavior unchanged: chunks without terrain produce byte-identical
  JSON, prior tests untouched in intent (only the mechanical `terrain: None` additions to
  literal constructors).
