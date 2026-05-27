//! Transition tests for plan section 7.4. Step 7 covers tests #1, #2, #4, #5, #7. Tests
//! #3, #6, #8 (iteration accessors and authored-vs-runtime comparison) land in step 9 when
//! `active_slots` / `vista_slots` / `slots` / `authored_eagerness` are exposed. Test #9
//! (snapshot persistence) is Phase D.

#![allow(clippy::similar_names)]

mod common;

use std::fs;
use std::path::Path;

use pantry::serde_json;
use wok_scene::{ChunkCoord, ChunkEagerness};
use wok_content::{
    ContentConfig, ContentEvent, ContentSystem, SlotState, TransitionError,
};

fn write_versioned(path: &Path, body: &serde_json::Value) {
    let mut top = serde_json::Map::new();
    top.insert("_format".to_string(), serde_json::Value::from(1u32));
    if let serde_json::Value::Object(map) = body {
        for (k, v) in map {
            top.insert(k.clone(), v.clone());
        }
    }
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(top)).expect("encode");
    fs::write(path, json).expect("write");
}

/// Build a fixture similar to `chunk_lifecycle::setup_fixture`, but the chunk's authored
/// eagerness is parameterised (Eager or Vista).
fn setup_fixture(content_root: &Path, eagerness: ChunkEagerness) -> std::path::PathBuf {
    let prefabs_dir = content_root.join("prefabs");
    fs::create_dir_all(&prefabs_dir).expect("mkdir prefabs");
    let scene_dir = content_root.join("scenes").join("room");
    fs::create_dir_all(&scene_dir).expect("mkdir scene");

    let prefab = serde_json::json!({
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
                    },
                    {
                        "primitive": { "kind": "cube", "half_extents": [1.0, 1.0, 1.0] },
                        "transform": { "pos": [3.0, 0.0, 0.0] },
                        "is_hitbox": true,
                        "is_visible": false,
                        "trigger_id": "crate-trigger"
                    }
                ]
            }
        ]
    });
    write_versioned(&prefabs_dir.join("crate.json"), &prefab);

    let scene_body = serde_json::json!({
        "id": "room",
        "default_load_radius_meters": 128.0,
        "default_eagerness": "eager",
        "default_light_state": "ambient-l-0",
        "chunks": [[0, 0]]
    });
    write_versioned(&scene_dir.join("scene.json"), &scene_body);

    let eagerness_str = match eagerness {
        ChunkEagerness::Eager => "eager",
        ChunkEagerness::Vista => "vista",
        ChunkEagerness::Lazy => "lazy",
    };
    let chunk_body = serde_json::json!({
        "coord": [0, 0],
        "metadata": {
            "eagerness": eagerness_str,
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

fn boot(content_root: &Path) -> ContentSystem {
    let (device, queue) = common::init_gpu();
    ContentSystem::new(device, queue, content_root.to_owned(), ContentConfig::default())
        .expect("boot")
}

fn drain_to_resident(system: &mut ContentSystem, coord: ChunkCoord) {
    let _ = system.poll();
    let _ = system.request_load(coord);
    for _ in 0..3 {
        let events = system.poll();
        if events
            .iter()
            .any(|e| matches!(e, ContentEvent::ChunkResident(c) if *c == coord))
        {
            return;
        }
    }
    panic!("chunk never reached Resident");
}

fn snapshot_runtime_arrays(
    system: &ContentSystem,
    coord: ChunkCoord,
) -> (
    Vec<wok_scene::VisibleShape>,
    Vec<wok_scene::PhysicalHitbox>,
    Vec<wok_scene::TriggerVolume>,
    Vec<wok_scene::RuntimeRegion>,
) {
    let slot = system.slot(coord).expect("slot exists");
    match &slot.state {
        SlotState::Resident(rc) | SlotState::Unloading(rc) => (
            rc.runtime.visible.clone(),
            rc.runtime.hitboxes.clone(),
            rc.runtime.triggers.clone(),
            rc.runtime.regions.clone(),
        ),
        _ => panic!("slot not Resident"),
    }
}

fn slot_eagerness(system: &ContentSystem, coord: ChunkCoord) -> ChunkEagerness {
    let slot = system.slot(coord).expect("slot exists");
    match &slot.state {
        SlotState::Resident(rc) | SlotState::Unloading(rc) => rc.runtime.eagerness,
        _ => panic!("slot not Resident"),
    }
}

// §7.4 #1: transition_chunk(coord, Vista) on Eager Resident → eagerness flips; runtime
// arrays unchanged.
#[test]
fn t01_transition_to_vista_preserves_arrays() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path(), ChunkEagerness::Eager);
    let mut system = boot(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let coord = ChunkCoord::new(0, 0);
    drain_to_resident(&mut system, coord);

    let before = snapshot_runtime_arrays(&system, coord);
    assert_eq!(slot_eagerness(&system, coord), ChunkEagerness::Eager);

    system
        .transition_chunk(coord, ChunkEagerness::Vista)
        .expect("transition");
    assert_eq!(slot_eagerness(&system, coord), ChunkEagerness::Vista);
    let after = snapshot_runtime_arrays(&system, coord);
    assert_eq!(before, after, "runtime arrays must be byte-identical");

    let events = system.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            ContentEvent::ChunkTransitioned { coord: c, from, to }
                if *c == coord && *from == ChunkEagerness::Eager && *to == ChunkEagerness::Vista
        )),
        "expected ChunkTransitioned event: {events:?}"
    );
}

// §7.4 #2: No I/O during transition. LoopbackWorker proxy: the worker queue length does
// not grow across a transition; the slot's runtime stays in place.
#[test]
fn t02_transition_does_no_io() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path(), ChunkEagerness::Eager);
    let mut system = boot(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let coord = ChunkCoord::new(0, 0);
    drain_to_resident(&mut system, coord);

    // Snapshot the slot's runtime address-equivalent state (the slot's GPU vec length is a
    // proxy for "nothing was uploaded"). Phase A's LoopbackWorker is fully synchronous;
    // after drain_to_resident the queue is empty. Transition must not enqueue anything.
    let pre_visible_count = match &system.slot(coord).unwrap().state {
        SlotState::Resident(rc) => rc.gpu.visible.len(),
        _ => panic!("not resident"),
    };

    system
        .transition_chunk(coord, ChunkEagerness::Vista)
        .expect("transition");

    let post_visible_count = match &system.slot(coord).unwrap().state {
        SlotState::Resident(rc) => rc.gpu.visible.len(),
        _ => panic!("not resident"),
    };
    assert_eq!(
        pre_visible_count, post_visible_count,
        "transition must not change GPU visible mesh count"
    );
    // The next poll() should emit ChunkTransitioned and nothing else (no ChunkResident or
    // ChunkUnloaded from a worker round-trip).
    let events = system.poll();
    let non_transition: Vec<_> = events
        .iter()
        .filter(|e| {
            !matches!(
                e,
                ContentEvent::ChunkTransitioned { .. } | ContentEvent::SceneLoaded(_)
            )
        })
        .collect();
    assert!(
        non_transition.is_empty(),
        "transition produced extraneous events: {non_transition:?}"
    );
}

// §7.4 #4: Round-trip Eager → Vista → Eager. Arrays + eagerness byte-identical.
#[test]
fn t04_round_trip_eager_vista_eager() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path(), ChunkEagerness::Eager);
    let mut system = boot(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let coord = ChunkCoord::new(0, 0);
    drain_to_resident(&mut system, coord);

    let before = snapshot_runtime_arrays(&system, coord);
    let eagerness_before = slot_eagerness(&system, coord);

    system
        .transition_chunk(coord, ChunkEagerness::Vista)
        .expect("to vista");
    system
        .transition_chunk(coord, ChunkEagerness::Eager)
        .expect("back to eager");

    let after = snapshot_runtime_arrays(&system, coord);
    let eagerness_after = slot_eagerness(&system, coord);
    assert_eq!(eagerness_before, eagerness_after);
    assert_eq!(before, after);
}

// §7.4 #5: Authored Vista chunk loads with runtime.eagerness == Vista.
#[test]
fn t05_authored_vista_loads_as_vista() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path(), ChunkEagerness::Vista);
    let mut system = boot(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let coord = ChunkCoord::new(0, 0);
    drain_to_resident(&mut system, coord);
    assert_eq!(slot_eagerness(&system, coord), ChunkEagerness::Vista);
}

// §7.4 #7: transition_chunk on Pending or Loading → NotResident. No state change.
#[test]
fn t07_transition_pending_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = setup_fixture(tmp.path(), ChunkEagerness::Eager);
    let mut system = boot(tmp.path());
    system.load_scene(&scene_dir).expect("load_scene");
    let _ = system.poll();

    let coord = ChunkCoord::new(0, 0);
    let _ = system.request_load(coord);
    // Slot is Pending; not Resident.
    let err = system
        .transition_chunk(coord, ChunkEagerness::Vista)
        .expect_err("transition on Pending must fail");
    match err {
        TransitionError::NotResident { coord: c, state_label } => {
            assert_eq!(c, coord);
            assert_eq!(state_label, "Pending");
        }
        other @ TransitionError::UnknownSlot(_) => panic!("expected NotResident, got {other:?}"),
    }
    // No state change.
    assert!(matches!(
        system.slot(coord).map(|s| &s.state),
        Some(SlotState::Pending)
    ));

    // After draining, slot becomes Resident; transition now succeeds. Sanity check that
    // the error path didn't break the slot.
    let _ = system.poll();
    assert!(matches!(
        system.slot(coord).map(|s| &s.state),
        Some(SlotState::Resident(_))
    ));
    system
        .transition_chunk(coord, ChunkEagerness::Vista)
        .expect("transition after Resident");

    // Unknown coord errors with UnknownSlot.
    let bad = ChunkCoord::new(99, 99);
    let err = system
        .transition_chunk(bad, ChunkEagerness::Vista)
        .expect_err("unknown coord must fail");
    match err {
        TransitionError::UnknownSlot(c) => assert_eq!(c, bad),
        other @ TransitionError::NotResident { .. } => {
            panic!("expected UnknownSlot, got {other:?}")
        }
    }
}
