use std::collections::BTreeMap;

use tempfile::tempdir;
use wok_scene::pantry::math::{Aabb, Transform, Vec3};
use wok_scene::{
    AudioCueId, Chunk, ChunkCoord, ChunkEagerness, ChunkMetadata, LightStateRef, LoadError,
    MeshId, Prefab, PrefabId, PrefabPlacement, PrefabState, RegionMarker, RegionPurpose, Scene,
    SceneId, Shape, ShapePrimitive, Slug, load_chunk, load_prefab, load_prefab_dir,
    load_scene_manifest, save_chunk, save_prefab, save_scene_manifest,
};

// ---- Helpers ----

fn slug(s: &str) -> Slug {
    Slug::new(s).unwrap()
}

fn make_prefab(id: &str) -> Prefab {
    Prefab {
        id: PrefabId(slug(id)),
        default_state: "default".to_string(),
        states: vec![PrefabState {
            name: "default".to_string(),
            shapes: vec![Shape {
                primitive: ShapePrimitive::Cube {
                    half_extents: Vec3::new(0.5, 0.5, 0.5),
                },
                transform: Transform {
                    translation: Vec3::new(0.0, 0.5, 0.0),
                    ..Transform::IDENTITY
                },
                is_hitbox: true,
                is_visible: true,
                trigger_id: None,
                surface_tag: Some("wood".to_string()),
                visual_color: Some([0.55, 0.35, 0.2]),
            }],
            mesh_override: Some(MeshId::new(slug("wooden-crate-mesh"), 267)),
            audio_cues: BTreeMap::from([(
                "impact".to_string(),
                AudioCueId::new(slug("wood-impact"), 12),
            )]),
        }],
    }
}

fn make_chunk(coord: ChunkCoord) -> Chunk {
    Chunk {
        coord,
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: vec![ChunkCoord::new(coord.x + 1, coord.z)],
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("warehouse-day"), 3),
        placements: vec![PrefabPlacement {
            prefab: PrefabId(slug("crate-wooden")),
            transform: Transform {
                translation: Vec3::new(12.0, 0.0, 8.5),
                ..Transform::IDENTITY
            },
            state: "default".to_string(),
            instance_tag: None,
        }],
        regions: vec![RegionMarker {
            name: "office-interior".to_string(),
            bounds: Aabb::new(Vec3::new(30.0, 0.0, 30.0), Vec3::new(40.0, 5.0, 40.0)),
            purpose: RegionPurpose::Lighting {
                state: LightStateRef::new(slug("warehouse-office"), 7),
            },
        }],
    }
}

fn make_scene() -> Scene {
    Scene {
        id: SceneId(slug("act1-warehouse")),
        default_load_radius_meters: 200.0,
        default_eagerness: ChunkEagerness::Eager,
        default_light_state: LightStateRef::new(slug("warehouse-day"), 3),
        chunks: vec![
            ChunkCoord::new(0, 0),
            ChunkCoord::new(1, 0),
            ChunkCoord::new(0, 1),
        ],
    }
}

// ---- _format version handling ----

#[test]
fn load_rejects_unsupported_format_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    std::fs::write(
        &path,
        r#"{"_format":0,"id":"x","default_state":"d","states":[]}"#,
    )
    .unwrap();
    let err = load_prefab(&path).unwrap_err();
    let LoadError::UnsupportedVersion { found, .. } = err else {
        panic!("expected UnsupportedVersion, got {err:?}");
    };
    assert_eq!(found, 0);
}

#[test]
fn load_rejects_future_format_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    std::fs::write(
        &path,
        r#"{"_format":99,"id":"x","default_state":"d","states":[]}"#,
    )
    .unwrap();
    let err = load_prefab(&path).unwrap_err();
    assert!(matches!(
        err,
        LoadError::UnsupportedVersion { found: 99, .. }
    ));
}

#[test]
fn load_rejects_missing_format_field() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    std::fs::write(&path, r#"{"id":"x","default_state":"d","states":[]}"#).unwrap();
    let err = load_prefab(&path).unwrap_err();
    assert!(matches!(err, LoadError::MissingFormat { .. }), "got {err:?}");
}

#[test]
fn load_rejects_format_field_typo() {
    // Typo like `_formate` is valid JSON but does not declare our header. MissingFormat,
    // not Parse, because the JSON itself is well-formed - just lacks our header.
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    std::fs::write(
        &path,
        r#"{"_formate":1,"id":"x","default_state":"d","states":[]}"#,
    )
    .unwrap();
    let err = load_prefab(&path).unwrap_err();
    assert!(matches!(err, LoadError::MissingFormat { .. }), "got {err:?}");
}

#[test]
fn load_rejects_non_integer_format_field() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    std::fs::write(
        &path,
        r#"{"_format":"one","id":"x","default_state":"d","states":[]}"#,
    )
    .unwrap();
    let err = load_prefab(&path).unwrap_err();
    assert!(matches!(err, LoadError::MissingFormat { .. }), "got {err:?}");
}

#[test]
fn saved_json_is_pretty_printed() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    let prefab = make_prefab("crate-wooden");
    save_prefab(&prefab, &path).unwrap();
    let json = std::fs::read_to_string(&path).unwrap();
    assert!(
        json.contains('\n'),
        "saved JSON should be pretty-printed (have newlines): {json}"
    );
    assert!(
        json.contains("  "),
        "saved JSON should be indented: {json}"
    );
}

// ---- Field validation ----

#[test]
fn load_rejects_missing_required_field() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    // Missing `states`.
    std::fs::write(&path, r#"{"_format":1,"id":"x","default_state":"d"}"#).unwrap();
    let err = load_prefab(&path).unwrap_err();
    let LoadError::Parse { source, .. } = err else {
        panic!("expected Parse, got {err:?}");
    };
    let msg = source.to_string();
    assert!(
        msg.contains("states") || msg.contains("missing"),
        "error message should name the missing field: {msg}"
    );
}

#[test]
fn load_rejects_unknown_field() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    std::fs::write(
        &path,
        r#"{"_format":1,"id":"x","default_state":"d","states":[],"bogus":1}"#,
    )
    .unwrap();
    let err = load_prefab(&path).unwrap_err();
    let LoadError::Parse { source, .. } = err else {
        panic!("expected Parse, got {err:?}");
    };
    let msg = source.to_string();
    assert!(
        msg.contains("bogus") || msg.contains("unknown"),
        "error message should mention the unknown field: {msg}"
    );
}

#[test]
fn load_rejects_unknown_field_inside_nested_struct() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    // Unknown field inside a nested Shape - tests that deny_unknown_fields cascades.
    std::fs::write(
        &path,
        r#"{
            "_format":1,
            "id":"x",
            "default_state":"d",
            "states":[{
                "name":"d",
                "shapes":[{
                    "primitive":{"kind":"cube","half_extents":[0.5,0.5,0.5]},
                    "transform":{"pos":[0,0,0]},
                    "is_hitbox":true,
                    "is_visible":true,
                    "extra_nested":42
                }]
            }]
        }"#,
    )
    .unwrap();
    let err = load_prefab(&path).unwrap_err();
    assert!(matches!(err, LoadError::Parse { .. }));
}

// ---- Round-trip equality and byte-identical ----

#[test]
fn save_then_load_round_trips_prefab() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crate-wooden.json");
    let prefab = make_prefab("crate-wooden");
    save_prefab(&prefab, &path).unwrap();
    let loaded = load_prefab(&path).unwrap();
    assert_eq!(prefab, loaded);
}

#[test]
fn save_then_load_round_trips_scene() {
    let dir = tempdir().unwrap();
    let scene = make_scene();
    save_scene_manifest(&scene, dir.path()).unwrap();
    assert!(dir.path().join("scene.json").exists());
    let loaded = load_scene_manifest(dir.path()).unwrap();
    assert_eq!(scene, loaded);
}

#[test]
fn save_then_load_round_trips_chunk() {
    let dir = tempdir().unwrap();
    let chunk = make_chunk(ChunkCoord::new(2, 1));
    save_chunk(dir.path(), &chunk).unwrap();
    assert!(dir.path().join("2_1.json").exists());
    let loaded = load_chunk(dir.path(), ChunkCoord::new(2, 1)).unwrap();
    assert_eq!(chunk, loaded);
}

#[test]
fn save_twice_produces_byte_identical_files() {
    let dir = tempdir().unwrap();
    let prefab = make_prefab("crate-wooden");
    let path1 = dir.path().join("first.json");
    let path2 = dir.path().join("second.json");
    save_prefab(&prefab, &path1).unwrap();
    save_prefab(&prefab, &path2).unwrap();
    let a = std::fs::read(&path1).unwrap();
    let b = std::fs::read(&path2).unwrap();
    assert_eq!(a, b);
}

// Plan section 4: load(save(load(x))) == load(x), byte-identical.
#[test]
fn load_save_load_byte_identical_prefab() {
    let dir = tempdir().unwrap();
    let original = make_prefab("crate-wooden");
    let path1 = dir.path().join("first.json");
    save_prefab(&original, &path1).unwrap();

    let loaded = load_prefab(&path1).unwrap();
    let path2 = dir.path().join("second.json");
    save_prefab(&loaded, &path2).unwrap();

    let bytes1 = std::fs::read(&path1).unwrap();
    let bytes2 = std::fs::read(&path2).unwrap();
    assert_eq!(bytes1, bytes2);
}

#[test]
fn load_save_load_byte_identical_chunk() {
    let dir1 = tempdir().unwrap();
    let dir2 = tempdir().unwrap();
    let original = make_chunk(ChunkCoord::new(0, 0));

    save_chunk(dir1.path(), &original).unwrap();
    let loaded = load_chunk(dir1.path(), ChunkCoord::new(0, 0)).unwrap();
    save_chunk(dir2.path(), &loaded).unwrap();

    let bytes1 = std::fs::read(dir1.path().join("0_0.json")).unwrap();
    let bytes2 = std::fs::read(dir2.path().join("0_0.json")).unwrap();
    assert_eq!(bytes1, bytes2);
}

// ---- Filename construction ----

#[test]
fn chunk_filename_uses_underscore_separator() {
    let dir = tempdir().unwrap();
    let chunk = make_chunk(ChunkCoord::new(3, 4));
    save_chunk(dir.path(), &chunk).unwrap();
    assert!(dir.path().join("3_4.json").exists());
}

#[test]
fn chunk_filename_handles_negative_coords() {
    let dir = tempdir().unwrap();
    let chunk = make_chunk(ChunkCoord::new(-1, -2));
    save_chunk(dir.path(), &chunk).unwrap();
    assert!(dir.path().join("-1_-2.json").exists());
    let loaded = load_chunk(dir.path(), ChunkCoord::new(-1, -2)).unwrap();
    assert_eq!(chunk, loaded);
}

// ---- load_prefab_dir ----

#[test]
fn load_prefab_dir_collects_all_json_files() {
    let dir = tempdir().unwrap();
    let p1 = make_prefab("crate-wooden");
    let p2 = make_prefab("door-simple");
    save_prefab(&p1, &dir.path().join("crate-wooden.json")).unwrap();
    save_prefab(&p2, &dir.path().join("door-simple.json")).unwrap();
    std::fs::write(dir.path().join("README.txt"), "not a prefab").unwrap();

    let prefabs = load_prefab_dir(dir.path()).unwrap();
    assert_eq!(prefabs.len(), 2);
    assert_eq!(prefabs.get(&p1.id).unwrap(), &p1);
    assert_eq!(prefabs.get(&p2.id).unwrap(), &p2);
}

#[test]
fn load_prefab_dir_empty_directory_returns_empty_map() {
    let dir = tempdir().unwrap();
    let prefabs = load_prefab_dir(dir.path()).unwrap();
    assert!(prefabs.is_empty());
}

#[test]
fn load_prefab_dir_propagates_load_errors() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("bad.json"), "this isn't json").unwrap();
    let err = load_prefab_dir(dir.path()).unwrap_err();
    assert!(matches!(err, LoadError::Parse { .. }));
}

// ---- IO error surface ----

#[test]
fn load_prefab_on_missing_file_returns_io_error() {
    let dir = tempdir().unwrap();
    let err = load_prefab(&dir.path().join("does-not-exist.json")).unwrap_err();
    assert!(matches!(err, LoadError::Io { .. }));
}
