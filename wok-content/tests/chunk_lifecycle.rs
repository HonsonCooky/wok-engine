//! Chunk lifecycle tests for plan section 7.3 #1-#7. Tests use the synchronous
//! `LoopbackWorker` so each "tick" is one `poll()` call.
//!
//! Each test builds a tempdir fixture on disk (scene manifest + chunk JSON files + prefab
//! files + registry.json) and drives `ContentSystem` through it. File-based load is the
//! real Phase A path; reading from disk also exercises the wok-scene loader.

#![allow(clippy::needless_collect)]
#![allow(clippy::similar_names)]
// scene_dir / scenes_dir / content_root form a similar-named cluster that clippy flags;
// renames would lose the wok-scene mapping convention.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use pantry::math::{Transform, Vec3};
use pantry::serde_json;
use wok_scene::{ChunkCoord, MeshId, Slug};
use wok_content::{ContentConfig, ContentEvent, ContentSystem, LoadError, SlotState};

fn slug(s: &str) -> Slug {
    Slug::new(s).expect("slug literal valid")
}

/// Write the integer-tag JSON file at `path/_format-tagged.json`. The wok-scene loader
/// requires the file open with `{ "_format": 1, ... }`.
fn write_versioned(path: &Path, body: &serde_json::Value) {
    let mut top = serde_json::Map::new();
    top.insert(
        "_format".to_string(),
        serde_json::Value::from(1u32),
    );
    if let serde_json::Value::Object(map) = body {
        for (k, v) in map {
            top.insert(k.clone(), v.clone());
        }
    }
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(top)).expect("encode");
    fs::write(path, json).expect("write file");
}

/// Set up the standard fixture layout under `content_root`:
/// - `<content_root>/prefabs/crate.json` - one prefab with two states; "closed" state has a
///   single cube shape (visible + hitbox), "open" same.
/// - `<content_root>/scenes/room/scene.json` - one chunk (0, 0).
/// - `<content_root>/scenes/room/0_0.json` - one placement of the crate.
/// - `<content_root>/registry.json` - empty registry (populate will fill).
///
/// Returns the scene_dir (absolute) so tests can pass it to `load_scene`.
fn setup_fixture(content_root: &Path) -> std::path::PathBuf {
    let prefabs_dir = content_root.join("prefabs");
    fs::create_dir_all(&prefabs_dir).expect("mkdir prefabs");
    let scenes_dir = content_root.join("scenes");
    let scene_dir = scenes_dir.join("room");
    fs::create_dir_all(&scene_dir).expect("mkdir scene");

    let prefab_body = serde_json::json!({
        "id": "crate",
        "default_state": "closed",
        "states": [
            {
                "name": "closed",
                "shapes": [
                    {
                        "primitive": { "kind": "cube", "half_extents": [0.5, 0.5, 0.5] },
                        "transform": { "pos": [0.0, 0.5, 0.0] },
                        "is_hitbox": true,
                        "is_visible": true
                    }
                ]
            },
            {
                "name": "open",
                "shapes": [
                    {
                        "primitive": { "kind": "cube", "half_extents": [0.5, 0.5, 0.5] },
                        "transform": { "pos": [0.0, 0.5, 0.0] },
                        "is_hitbox": true,
                        "is_visible": true
                    }
                ]
            }
        ]
    });
    write_versioned(&prefabs_dir.join("crate.json"), &prefab_body);

    let scene_body = serde_json::json!({
        "id": "room",
        "default_load_radius_meters": 128.0,
        "default_eagerness": "eager",
        "default_light_state": "ambient-l-0",
        "chunks": [[0, 0]]
    });
    write_versioned(&scene_dir.join("scene.json"), &scene_body);

    let chunk_body = serde_json::json!({
        "coord": [0, 0],
        "metadata": {
            "eagerness": "eager",
            "neighbors": [],
            "interlocks": []
        },
        "light_state": "ambient-l-0",
        "placements": [
            {
                "prefab": "crate",
                "transform": { "pos": [5.0, 0.0, 5.0] },
                "state": "closed"
            }
        ],
        "regions": []
    });
    write_versioned(&scene_dir.join("0_0.json"), &chunk_body);

    // registry.json: empty. populate will fill with placeholder entries for the light state.
    let registry_body = serde_json::json!({
        "_format": 1,
        "meshes": { "next_serial": 0, "entries": [] },
        "audio": { "next_serial": 0, "entries": [] },
        "animations": { "next_serial": 0, "entries": [] },
        "voice": { "next_serial": 0, "entries": [] },
        "light_states": { "next_serial": 0, "entries": [] }
    });
    fs::write(
        content_root.join("registry.json"),
        serde_json::to_string_pretty(&registry_body).expect("encode registry"),
    )
    .expect("write registry");

    scene_dir
}

fn boot_system(content_root: &Path) -> ContentSystem {
    let (device, queue) = common::init_gpu();
    ContentSystem::new(
        device,
        queue,
        content_root.to_owned(),
        ContentConfig::default(),
    )
    .expect("new ContentSystem")
}

/// Drain `poll()` until at least one `ChunkResident` event is seen or `max_polls` is hit.
/// Returns all events seen.
fn poll_until_resident(system: &mut ContentSystem, max_polls: usize) -> Vec<ContentEvent> {
    let mut all = Vec::new();
    for _ in 0..max_polls {
        let events = system.poll();
        let saw = events
            .iter()
            .any(|e| matches!(e, ContentEvent::ChunkResident(_)));
        all.extend(events);
        if saw {
            return all;
        }
    }
    all
}

// §7.3 #1: request_load(coord) for a known chunk → Pending → after one tick, Resident.
#[test]
fn t01_request_load_to_resident() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll(); // drain SceneLoaded event

    let coord = ChunkCoord::new(0, 0);
    let _handle = system.request_load(coord);
    assert!(matches!(
        system.slot(coord).map(|s| &s.state),
        Some(SlotState::Pending)
    ));
    let events = system.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ContentEvent::ChunkResident(c) if *c == coord)),
        "expected ChunkResident in events: {events:?}"
    );
    assert!(matches!(
        system.slot(coord).map(|s| &s.state),
        Some(SlotState::Resident(_))
    ));
}

// §7.3 #2: Same coord loaded twice → idempotent (no second dispatch).
#[test]
fn t02_idempotent_load() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let h1 = system.request_load(coord);
    let h2 = system.request_load(coord);
    assert_eq!(h1.coord(), h2.coord());
    // Drain. We expect exactly one ChunkResident.
    let events = system.poll();
    let resident_count = events
        .iter()
        .filter(|e| matches!(e, ContentEvent::ChunkResident(_)))
        .count();
    assert_eq!(resident_count, 1, "expected one ChunkResident: {events:?}");

    // A third request_load on an already-Resident slot is a no-op (no new dispatch).
    let _ = system.request_load(coord);
    let events = system.poll();
    let resident_count = events
        .iter()
        .filter(|e| matches!(e, ContentEvent::ChunkResident(_)))
        .count();
    assert_eq!(
        resident_count, 0,
        "idempotent re-load fired extra ChunkResident: {events:?}"
    );
}

// §7.3 #3: request_unload while Pending → slot removed, no event.
#[test]
fn t03_unload_while_pending() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let _ = system.request_load(coord);
    assert!(matches!(
        system.slot(coord).map(|s| &s.state),
        Some(SlotState::Pending)
    ));
    system.request_unload(coord);
    assert!(system.slot(coord).is_none(), "slot should be gone");

    // The worker still has the request in its queue; poll() runs it but the slot is missing,
    // so the result is discarded. No event.
    let events = system.poll();
    let lifecycle_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                ContentEvent::ChunkResident(_)
                    | ContentEvent::ChunkUnloaded(_)
                    | ContentEvent::ChunkFailed { .. }
            )
        })
        .collect();
    assert!(
        lifecycle_events.is_empty(),
        "expected no lifecycle events: {lifecycle_events:?}"
    );
    assert!(system.slot(coord).is_none(), "slot stays gone");
}

// §7.3 #4: request_unload during in-flight load → result discarded, no event.
//
// LoopbackWorker collapses Pending and Loading (Loading is only observable with Phase B's
// threaded worker). This test exercises the same code path as #3 from the consumer's
// perspective: unload comes in before the worker integrates the result. The integration
// happens during poll(); the slot is removed before poll(), so integration sees a missing
// slot and drops the result. No event.
#[test]
fn t04_unload_during_in_flight() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let _ = system.request_load(coord);
    system.request_unload(coord);
    let events = system.poll();
    let lifecycle_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                ContentEvent::ChunkResident(_) | ContentEvent::ChunkUnloaded(_)
            )
        })
        .collect();
    assert!(
        lifecycle_events.is_empty(),
        "expected no lifecycle events after in-flight unload: {lifecycle_events:?}"
    );
}

// §7.3 #5: request_unload while Resident → state Unloading → next poll() removes → event.
#[test]
fn t05_unload_while_resident() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let _ = system.request_load(coord);
    let _ = poll_until_resident(&mut system, 2);
    assert!(matches!(
        system.slot(coord).map(|s| &s.state),
        Some(SlotState::Resident(_))
    ));
    system.request_unload(coord);
    assert!(
        matches!(
            system.slot(coord).map(|s| &s.state),
            Some(SlotState::Unloading(_))
        ),
        "expected Unloading state: {:?}",
        system.slot(coord).map(|s| s.state.label())
    );
    let events = system.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ContentEvent::ChunkUnloaded(c) if *c == coord)),
        "expected ChunkUnloaded: {events:?}"
    );
    assert!(system.slot(coord).is_none(), "slot removed after finalize");
}

// §7.3 #6: Load chunk referencing missing prefab → ChunkFailed with PrefabMissing.
#[test]
fn t06_missing_prefab() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Custom fixture: chunk references "missing-prefab" which the prefabs directory does
    // not contain.
    let scene_dir = setup_missing_prefab(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let _ = system.request_load(coord);
    let events = system.poll();
    let failed = events
        .iter()
        .find_map(|e| match e {
            ContentEvent::ChunkFailed { coord: c, error } if *c == coord => Some(error.clone()),
            _ => None,
        })
        .expect("ChunkFailed event missing");
    match &*failed {
        LoadError::PrefabMissing(id) => assert_eq!(id.0, slug("missing-prefab")),
        other => panic!("expected PrefabMissing, got {other:?}"),
    }
}

fn setup_missing_prefab(content_root: &Path) -> std::path::PathBuf {
    let prefabs_dir = content_root.join("prefabs");
    fs::create_dir_all(&prefabs_dir).expect("mkdir prefabs");
    let scenes_dir = content_root.join("scenes");
    let scene_dir = scenes_dir.join("room");
    fs::create_dir_all(&scene_dir).expect("mkdir scene");

    let scene_body = serde_json::json!({
        "id": "room",
        "default_load_radius_meters": 128.0,
        "default_eagerness": "eager",
        "default_light_state": "ambient-l-0",
        "chunks": [[0, 0]]
    });
    write_versioned(&scene_dir.join("scene.json"), &scene_body);
    let chunk_body = serde_json::json!({
        "coord": [0, 0],
        "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
        "light_state": "ambient-l-0",
        "placements": [
            { "prefab": "missing-prefab", "transform": { "pos": [0.0, 0.0, 0.0] }, "state": "x" }
        ],
        "regions": []
    });
    write_versioned(&scene_dir.join("0_0.json"), &chunk_body);
    let registry_body = serde_json::json!({
        "_format": 1,
        "meshes": { "next_serial": 0, "entries": [] },
        "audio": { "next_serial": 0, "entries": [] },
        "animations": { "next_serial": 0, "entries": [] },
        "voice": { "next_serial": 0, "entries": [] },
        "light_states": { "next_serial": 0, "entries": [] }
    });
    fs::write(
        content_root.join("registry.json"),
        serde_json::to_string_pretty(&registry_body).expect("encode"),
    )
    .expect("write registry");
    scene_dir
}

// §7.3 #7: Load chunk referencing missing mesh serial → ChunkFailed with AssetMissing.
//
// Fixture: prefab state has a `mesh_override: "shipped-floor-99"`. The registry does not
// contain serial 99 (only serial 0 exists, with a different slug). Load should surface
// AssetMissing.
#[test]
fn t07_missing_mesh_serial() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_missing_mesh(tmp.path());
    let mut system = boot_system(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let _ = system.request_load(coord);
    let events = system.poll();
    let failed = events
        .iter()
        .find_map(|e| match e {
            ContentEvent::ChunkFailed { coord: c, error } if *c == coord => Some(error.clone()),
            _ => None,
        })
        .expect("ChunkFailed event missing");
    match &*failed {
        LoadError::AssetMissing { serial, .. } => assert_eq!(*serial, 99),
        other => panic!("expected AssetMissing, got {other:?}"),
    }
}

fn setup_missing_mesh(content_root: &Path) -> std::path::PathBuf {
    let prefabs_dir = content_root.join("prefabs");
    fs::create_dir_all(&prefabs_dir).expect("mkdir prefabs");
    let scenes_dir = content_root.join("scenes");
    let scene_dir = scenes_dir.join("room");
    fs::create_dir_all(&scene_dir).expect("mkdir scene");

    // Prefab references shipped-floor-99 via mesh_override. The registry has shipped-floor
    // at serial 0; serial 99 does not exist.
    let prefab_body = serde_json::json!({
        "id": "floor",
        "default_state": "default",
        "states": [
            {
                "name": "default",
                "shapes": [
                    {
                        "primitive": { "kind": "cube", "half_extents": [0.5, 0.5, 0.5] },
                        "transform": { "pos": [0.0, 0.0, 0.0] },
                        "is_hitbox": true,
                        "is_visible": true
                    }
                ],
                "mesh_override": "shipped-floor-99"
            }
        ]
    });
    write_versioned(&prefabs_dir.join("floor.json"), &prefab_body);

    let scene_body = serde_json::json!({
        "id": "room",
        "default_load_radius_meters": 128.0,
        "default_eagerness": "eager",
        "default_light_state": "ambient-l-0",
        "chunks": [[0, 0]]
    });
    write_versioned(&scene_dir.join("scene.json"), &scene_body);
    let chunk_body = serde_json::json!({
        "coord": [0, 0],
        "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
        "light_state": "ambient-l-0",
        "placements": [
            { "prefab": "floor", "transform": { "pos": [0.0, 0.0, 0.0] }, "state": "default" }
        ],
        "regions": []
    });
    write_versioned(&scene_dir.join("0_0.json"), &chunk_body);

    // Registry: holds shipped-floor at serial 0; serial 99 is empty.
    let registry_body = serde_json::json!({
        "_format": 1,
        "meshes": {
            "next_serial": 1,
            "entries": [
                { "serial": 0, "slug": "shipped-floor", "status": "shipped", "source": "meshes/floor.gltf" }
            ]
        },
        "audio": { "next_serial": 0, "entries": [] },
        "animations": { "next_serial": 0, "entries": [] },
        "voice": { "next_serial": 0, "entries": [] },
        "light_states": { "next_serial": 0, "entries": [] }
    });
    fs::write(
        content_root.join("registry.json"),
        serde_json::to_string_pretty(&registry_body).expect("encode"),
    )
    .expect("write registry");
    scene_dir
}

// Suppress unused-import lint inflated by per-test compilation.
fn _unused() {
    let _ = BTreeMap::<String, MeshId>::new();
    let _ = Transform::IDENTITY;
    let _ = Vec3::ZERO;
}
