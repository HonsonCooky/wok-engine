# wok-scene

The bedrock data-model crate of the Wok engine. Defines every authored type that lives on
disk as JSON, the runtime-array types they get sliced into, the pure slicing function that
transforms one into the other, file IO with a format-version header, and a debounced
filesystem watcher.

## Modules

- `ids` - `Slug`, content IDs (`PrefabId`, `SceneId`, `TriggerId`, `ChunkCoord`), and asset
  IDs (`MeshId`, `AudioCueId`, `AnimationId`, `VoiceLineId`, `LightStateRef`).
- `authored` - on-disk types: `Prefab`, `PrefabState`, `Shape`, `ShapePrimitive`,
  `PrefabPlacement`, `Chunk`, `ChunkMetadata`, `ChunkEagerness`, `RegionMarker`,
  `RegionPurpose`, `Scene`.
- `runtime` - per-system runtime arrays: `VisibleShape`, `PhysicalHitbox`, `TriggerVolume`,
  `RuntimeRegion`, `ChunkRuntime`.
- `slice` - `slice_chunk` plus the `PrefabLookup` trait.
- `load` / `save` - JSON file IO with the `_format: 1` header.
- `watcher` - debounced filesystem watcher producing typed `FileEvent`s.

## Non-responsibilities

- No GPU work. Runtime arrays are pure data.
- No simulation. Slicing is a pure transformation.
- No threads owned by wok-scene. The watcher's background thread is owned by
  notify-debouncer-full and torn down when `FileWatcher` is dropped.
- No runtime-state persistence. Snapshotting belongs to `wok-content`, not here.
- No asset registry. Asset IDs are constructed here; the "every ID points to something"
  invariant is `wok-content`'s registry to enforce.

## Cross-platform status

Verified on Windows 11. Linux and macOS watcher tests are outstanding; the rest of the
crate is platform-independent.
