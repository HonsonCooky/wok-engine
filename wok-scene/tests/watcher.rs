//! Watcher tests. These tests are inherently timing-sensitive: they touch the real
//! filesystem and depend on the OS surfacing events through notify within the ~100ms
//! debounce window. On commodity hardware the 250ms post-action sleep is generous, but
//! these tests are slightly flakier than the pure-Rust suites; that is acknowledged in
//! the plan.

use std::path::Path;
use std::time::Duration;

use tempfile::{TempDir, tempdir};
use wok_scene::{ChunkCoord, FileEvent, FileWatcher};

/// Sleep long enough for the 100ms debouncer to flush events and the OS to surface them.
fn settle() {
    std::thread::sleep(Duration::from_millis(250));
}

fn setup() -> (TempDir, FileWatcher) {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("prefabs")).unwrap();
    std::fs::create_dir_all(dir.path().join("scenes/act1")).unwrap();
    std::fs::create_dir_all(dir.path().join("lights")).unwrap();
    // Let the filesystem settle before installing the watcher so any directory-creation
    // events are not picked up.
    std::thread::sleep(Duration::from_millis(50));
    let mut watcher = FileWatcher::new(dir.path()).unwrap();
    // Drain any startup ghosts. The plan says the watcher does not emit on startup, but
    // sleeping briefly before the first test action makes the assertions robust against
    // any filesystem coalescing.
    settle();
    let _ = watcher.poll();
    (dir, watcher)
}

fn path_ends_with(path: &Path, suffix: &str) -> bool {
    path.to_string_lossy().replace('\\', "/").ends_with(suffix)
}

// ---- Plan section 7 tests ----

#[test]
fn t1_create_prefab_emits_prefab_changed() {
    let (dir, mut watcher) = setup();
    let path = dir.path().join("prefabs/crate-wooden.json");
    std::fs::write(&path, "{}").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, FileEvent::PrefabChanged(p) if path_ends_with(p, "prefabs/crate-wooden.json"))),
        "expected PrefabChanged for crate-wooden.json; got {events:?}"
    );
}

#[test]
fn t2_modify_prefab_emits_prefab_changed() {
    let (dir, mut watcher) = setup();
    let path = dir.path().join("prefabs/crate-wooden.json");
    std::fs::write(&path, "{}").unwrap();
    settle();
    let _ = watcher.poll();

    std::fs::write(&path, r#"{"modified":true}"#).unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, FileEvent::PrefabChanged(_))),
        "expected PrefabChanged after modify; got {events:?}"
    );
}

#[test]
fn t3_delete_prefab_emits_prefab_removed() {
    let (dir, mut watcher) = setup();
    let path = dir.path().join("prefabs/crate-wooden.json");
    std::fs::write(&path, "{}").unwrap();
    settle();
    let _ = watcher.poll();

    std::fs::remove_file(&path).unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, FileEvent::PrefabRemoved(_))),
        "expected PrefabRemoved; got {events:?}"
    );
}

#[test]
fn t4_chunk_change_carries_parsed_coord() {
    let (dir, mut watcher) = setup();
    let chunk_path = dir.path().join("scenes/act1/3_-2.json");
    std::fs::write(&chunk_path, "{}").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            FileEvent::ChunkChanged { coord, .. } if *coord == ChunkCoord::new(3, -2)
        )),
        "expected ChunkChanged at (3, -2); got {events:?}"
    );
}

#[test]
fn t5_chunk_file_with_unparseable_name_emits_error() {
    let (dir, mut watcher) = setup();
    let path = dir.path().join("scenes/act1/not-a-chunk.json");
    std::fs::write(&path, "{}").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(e, FileEvent::Error(_))),
        "expected Error for unparseable chunk filename; got {events:?}"
    );
}

#[test]
fn t6_multiple_rapid_writes_debounce_to_one_event() {
    let (dir, mut watcher) = setup();
    let path = dir.path().join("prefabs/crate-wooden.json");
    for i in 0..5 {
        std::fs::write(&path, format!(r#"{{"iter":{i}}}"#)).unwrap();
    }
    settle();
    let events = watcher.poll();
    let count = events
        .iter()
        .filter(|e| matches!(e, FileEvent::PrefabChanged(p) if path_ends_with(p, "prefabs/crate-wooden.json")))
        .count();
    assert_eq!(
        count, 1,
        "expected exactly one debounced PrefabChanged for 5 rapid writes; got {count}: {events:?}"
    );
}

#[test]
fn t7_file_outside_known_prefixes_emits_no_event() {
    let (dir, mut watcher) = setup();
    // README at the root, JSON at the root, JSON in an unrelated directory.
    std::fs::write(dir.path().join("README.txt"), "not a prefab").unwrap();
    std::fs::write(dir.path().join("loose.json"), "{}").unwrap();
    std::fs::create_dir_all(dir.path().join("misc")).unwrap();
    std::fs::write(dir.path().join("misc/whatever.json"), "{}").unwrap();
    settle();
    let events = watcher.poll();
    assert_eq!(events.len(), 0, "expected no events; got {events:?}");
}

// ---- Extra coverage for the remaining FileEvent variants ----

#[test]
fn scene_manifest_change_emits_scene_manifest_changed() {
    let (dir, mut watcher) = setup();
    std::fs::write(dir.path().join("scenes/act1/scene.json"), "{}").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, FileEvent::SceneManifestChanged(_))),
        "expected SceneManifestChanged; got {events:?}"
    );
}

#[test]
fn light_state_change_emits_light_state_changed() {
    let (dir, mut watcher) = setup();
    std::fs::write(dir.path().join("lights/warehouse-day.json"), "{}").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, FileEvent::LightStateChanged(_))),
        "expected LightStateChanged; got {events:?}"
    );
}

#[test]
fn chunk_delete_emits_chunk_removed_with_coord() {
    let (dir, mut watcher) = setup();
    let path = dir.path().join("scenes/act1/0_0.json");
    std::fs::write(&path, "{}").unwrap();
    settle();
    let _ = watcher.poll();

    std::fs::remove_file(&path).unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            FileEvent::ChunkRemoved { coord, .. } if *coord == ChunkCoord::new(0, 0)
        )),
        "expected ChunkRemoved at (0, 0); got {events:?}"
    );
}

// ---- v0.2.0 heightmap watcher coverage (plan §7 tests 8 and 9) ----

#[test]
fn t8_heightmap_modification_emits_chunk_changed() {
    // Plan §7 #8. Writing a heightmap binary in a scene directory emits ChunkChanged with
    // the coord parsed from the filename. The point: heightmap touches coalesce with the
    // chunk JSON path - no separate ChunkHeightmapChanged variant.
    let (dir, mut watcher) = setup();
    let path = dir.path().join("scenes/act1/3_-2.heightmap.bin");
    // A minimal payload - the watcher does not parse the binary, it only classifies the
    // filename.
    std::fs::write(&path, b"WTRN\x01\x00").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            FileEvent::ChunkChanged { coord, .. } if *coord == ChunkCoord::new(3, -2)
        )),
        "expected ChunkChanged at (3, -2) for heightmap modification; got {events:?}"
    );
}

#[test]
fn t9_unparseable_heightmap_filename_emits_error() {
    // Plan §7 #9. A heightmap filename that does not parse as `{i}_{j}.heightmap.bin`
    // surfaces as FileEvent::Error, same posture as unparseable chunk-JSON names.
    let (dir, mut watcher) = setup();
    let path = dir.path().join("scenes/act1/notachunk.heightmap.bin");
    std::fs::write(&path, b"WTRN\x01\x00").unwrap();
    settle();
    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(e, FileEvent::Error(_))),
        "expected Error for unparseable heightmap filename; got {events:?}"
    );
}
