# tests-integration/fixtures

Shared on-disk content fixtures for workspace integration tests. Plan section 4.4 layout:

```
fixtures/
+- registry.json
+- prefabs/
|  +- player-capsule.json    one capsule prefab, hitbox + visible
|  +- room-floor.json        plane prefab
|  +- room-walls.json        cube prefabs assembled into a room
|  +- interactable-box.json  one box, hitbox + visible
|  +- trigger-pad.json       hitbox-only volume with trigger_id
+- scenes/
   +- room/
      +- scene.json
      +- 0_0.json
      +- 0_0.heightmap.bin   sibling binary per wok-scene v0.2 terrain
```

The heightmap binary `0_0.heightmap.bin` is NOT checked in. Integration tests copy the
JSON files into a tempdir and generate a flat heightmap there before loading the scene
via `ContentSystem::load_scene`. The plan section 4.4 calls for the binary in the on-disk
layout; the test scaffolding keeps it out of source control so diffs stay readable and
regenerating is one helper call.

The committed Phase-A integration test lives in `wok-content/tests/fixtures.rs` and
exercises plan section 11 step 10's verification list:

- Scene loads.
- Each chunk reaches Resident through the LoopbackWorker.
- Transition to Vista works.
- Registry populates correctly (placeholder entry for the unknown light state slug).
- Terrain mesh appears in the slot's GPU handles when the chunk carries terrain data.

When other crates (`wok-render`, `wok-physics`, etc.) participate in workspace integration
tests, the `tests-integration/` crate skeleton lands alongside this directory.
