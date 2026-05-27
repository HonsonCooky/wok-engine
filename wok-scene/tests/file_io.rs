use std::collections::BTreeMap;

use tempfile::tempdir;
use wok_scene::pantry::math::{Aabb, Transform, Vec3};
use wok_scene::{
    AudioCueId, Chunk, ChunkCoord, ChunkEagerness, ChunkMetadata, LightStateRef, LoadError,
    MeshId, Prefab, PrefabId, PrefabPlacement, PrefabState, RegionMarker, RegionPurpose, Scene,
    SceneId, Shape, ShapePrimitive, Slug, TerrainData, load_chunk, load_prefab, load_prefab_dir,
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
        terrain: None,
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

// ---- v0.2.0 terrain (sibling binary) ----

/// A flat-ish heightmap with a known surface table. Surface indices for the first half of
/// cells reference the first authored tag, the back half references the second. The tag list
/// is deliberately not in alphabetical order so save-time sorting becomes visible.
fn fixture_terrain() -> TerrainData {
    let mut heights = vec![0u16; TerrainData::CELL_COUNT];
    // Slight slope so heights are non-uniform: each cell carries (z * CELLS + x) mod 65535.
    for z in 0..TerrainData::CELLS_PER_AXIS as usize {
        for x in 0..TerrainData::CELLS_PER_AXIS as usize {
            heights[z * (TerrainData::CELLS_PER_AXIS as usize) + x] =
                ((z * 31 + x * 17) % 65535) as u16;
        }
    }
    let mut surface_indices = vec![0u16; TerrainData::CELL_COUNT];
    for (idx, slot) in surface_indices.iter_mut().enumerate() {
        // First half references authored idx 1 ("grass"); back half references idx 0 ("wood").
        *slot = u16::from(idx < TerrainData::CELL_COUNT / 2);
    }
    TerrainData {
        heights: heights.into_boxed_slice(),
        surface_indices: surface_indices.into_boxed_slice(),
        // Intentionally unsorted; save must sort and remap indices through the permutation.
        surface_tags: vec!["wood".to_string(), "grass".to_string()],
        vertical_range_meters: 32.0,
        heightmap_file: "0_0.heightmap.bin".to_string(),
    }
}

fn make_chunk_with_terrain(coord: ChunkCoord, terrain: TerrainData) -> Chunk {
    Chunk {
        coord,
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("warehouse-day"), 3),
        placements: Vec::new(),
        regions: Vec::new(),
        terrain: Some(terrain),
    }
}

#[test]
fn terrain_chunk_round_trips_and_byte_identical_files() {
    // Plan section 7 v0.2.0: construct chunk with terrain, save, load, save again, compare
    // bytes. Both JSON and sibling binary byte-equal across the round trip.
    let dir1 = tempdir().unwrap();
    let dir2 = tempdir().unwrap();
    let coord = ChunkCoord::new(0, 0);
    let chunk = make_chunk_with_terrain(coord, fixture_terrain());

    save_chunk(dir1.path(), &chunk).unwrap();
    assert!(dir1.path().join("0_0.json").exists());
    assert!(dir1.path().join("0_0.heightmap.bin").exists());

    let loaded = load_chunk(dir1.path(), coord).unwrap();
    save_chunk(dir2.path(), &loaded).unwrap();

    let json1 = std::fs::read(dir1.path().join("0_0.json")).unwrap();
    let json2 = std::fs::read(dir2.path().join("0_0.json")).unwrap();
    assert_eq!(json1, json2, "chunk JSON byte-identical across round trip");

    let bin1 = std::fs::read(dir1.path().join("0_0.heightmap.bin")).unwrap();
    let bin2 = std::fs::read(dir2.path().join("0_0.heightmap.bin")).unwrap();
    assert_eq!(bin1, bin2, "sibling binary byte-identical across round trip");
}

#[test]
fn chunk_without_terrain_omits_field_in_json() {
    // Plan section 7 v0.2.0: chunk without terrain saves without the field, loads to
    // `terrain: None`, and re-saves with no terrain field. Preserves byte-identity with the
    // v0.1.0 baseline file shape.
    let dir = tempdir().unwrap();
    let chunk = make_chunk(ChunkCoord::new(0, 0));
    assert!(chunk.terrain.is_none());
    save_chunk(dir.path(), &chunk).unwrap();
    let json = std::fs::read_to_string(dir.path().join("0_0.json")).unwrap();
    assert!(
        !json.contains("terrain"),
        "terrain field must be omitted when None: {json}"
    );
    let loaded = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap();
    assert!(loaded.terrain.is_none());

    let dir2 = tempdir().unwrap();
    save_chunk(dir2.path(), &loaded).unwrap();
    let json2 = std::fs::read(dir2.path().join("0_0.json")).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("0_0.json")).unwrap(),
        json2,
        "chunk-without-terrain JSON byte-identical across the round trip"
    );
}

#[test]
fn terrain_surface_tags_sorted_on_save() {
    // Plan section 7 v0.2.0: surface tags in non-alphabetical order at construction time are
    // alphabetically sorted in the saved sibling binary. Surface indices are remapped through
    // the resulting permutation, so a cell that originally referenced "wood" (authored idx 0)
    // ends up referencing the sorted position of "wood" (which is index 1 after sort, because
    // "grass" sorts first).
    let dir = tempdir().unwrap();
    let coord = ChunkCoord::new(0, 0);
    let chunk = make_chunk_with_terrain(coord, fixture_terrain());
    save_chunk(dir.path(), &chunk).unwrap();

    let loaded = load_chunk(dir.path(), coord).unwrap();
    let terrain = loaded.terrain.as_ref().unwrap();
    assert_eq!(
        terrain.surface_tags,
        vec!["grass".to_string(), "wood".to_string()],
        "tags must be alphabetically sorted on save"
    );
    // First half of cells originally referenced authored idx 1 ("grass"); after sort, grass
    // is at index 0, so the loaded indices for the first half are 0.
    assert_eq!(terrain.surface_indices[0], 0);
    assert_eq!(terrain.surface_indices[TerrainData::CELL_COUNT / 2 - 1], 0);
    // Second half originally referenced authored idx 0 ("wood"); after sort, wood is at
    // index 1.
    assert_eq!(terrain.surface_indices[TerrainData::CELL_COUNT / 2], 1);
    assert_eq!(terrain.surface_indices[TerrainData::CELL_COUNT - 1], 1);
}

#[test]
fn terrain_chunk_json_references_sibling_by_filename() {
    // Spot-check the on-disk JSON shape: a single `heightmap_file` key under `terrain`. Locks
    // the wire shape so a future serde tweak does not silently change the reference layout.
    let dir = tempdir().unwrap();
    let coord = ChunkCoord::new(2, -3);
    let mut terrain = fixture_terrain();
    terrain.heightmap_file = "2_-3.heightmap.bin".to_string();
    let chunk = make_chunk_with_terrain(coord, terrain);
    save_chunk(dir.path(), &chunk).unwrap();
    let json = std::fs::read_to_string(dir.path().join("2_-3.json")).unwrap();
    assert!(
        json.contains(r#""heightmap_file": "2_-3.heightmap.bin""#),
        "expected heightmap_file reference in JSON: {json}"
    );
    // Heightmap bytes do NOT live in the JSON.
    assert!(
        !json.contains("heights"),
        "raw heightmap data must not be embedded in JSON: {json}"
    );
}

#[test]
fn terrain_load_with_missing_sibling_returns_terrain_sibling_missing() {
    // Author a chunk JSON that names a heightmap file we never write. The loader must
    // surface this specifically as TerrainSiblingMissing, not a generic IO error.
    let dir = tempdir().unwrap();
    let chunk_path = dir.path().join("0_0.json");
    std::fs::write(
        &chunk_path,
        r#"{
  "_format": 1,
  "coord": [0, 0],
  "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
  "light_state": "warehouse-day-3",
  "placements": [],
  "regions": [],
  "terrain": { "heightmap_file": "0_0.heightmap.bin" }
}"#,
    )
    .unwrap();
    let err = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap_err();
    assert!(
        matches!(err, LoadError::TerrainSiblingMissing { .. }),
        "expected TerrainSiblingMissing, got {err:?}"
    );
}

#[test]
fn terrain_load_rejects_absolute_heightmap_path() {
    // Absolute paths in `heightmap_file` are rejected at parse time. The exact error variant
    // is Parse (the rejection comes from the serde custom deserializer); the point of this
    // test is that the load does not succeed with an absolute path.
    let dir = tempdir().unwrap();
    let chunk_path = dir.path().join("0_0.json");
    let payload = if cfg!(windows) {
        r#"{
  "_format": 1,
  "coord": [0, 0],
  "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
  "light_state": "warehouse-day-3",
  "placements": [],
  "regions": [],
  "terrain": { "heightmap_file": "C:\\evil\\heightmap.bin" }
}"#
    } else {
        r#"{
  "_format": 1,
  "coord": [0, 0],
  "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
  "light_state": "warehouse-day-3",
  "placements": [],
  "regions": [],
  "terrain": { "heightmap_file": "/etc/passwd" }
}"#
    };
    std::fs::write(&chunk_path, payload).unwrap();
    let err = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap_err();
    assert!(
        matches!(err, LoadError::Parse { .. }),
        "expected Parse rejecting absolute path, got {err:?}"
    );
}

#[test]
fn terrain_load_rejects_directory_component_in_heightmap_file() {
    // Same posture as the absolute-path rejection: heightmap_file must be a bare filename.
    let dir = tempdir().unwrap();
    let chunk_path = dir.path().join("0_0.json");
    std::fs::write(
        &chunk_path,
        r#"{
  "_format": 1,
  "coord": [0, 0],
  "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
  "light_state": "warehouse-day-3",
  "placements": [],
  "regions": [],
  "terrain": { "heightmap_file": "subdir/0_0.heightmap.bin" }
}"#,
    )
    .unwrap();
    let err = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap_err();
    assert!(
        matches!(err, LoadError::Parse { .. }),
        "expected Parse rejecting directory component, got {err:?}"
    );
}

#[test]
fn terrain_load_rejects_bad_magic() {
    // Sibling binary with wrong magic bytes surfaces as TerrainMalformed, not IO.
    let dir = tempdir().unwrap();
    save_chunk(
        dir.path(),
        &make_chunk_with_terrain(ChunkCoord::new(0, 0), fixture_terrain()),
    )
    .unwrap();
    // Corrupt the magic.
    let bin_path = dir.path().join("0_0.heightmap.bin");
    let mut bytes = std::fs::read(&bin_path).unwrap();
    bytes[0..4].copy_from_slice(b"XXXX");
    std::fs::write(&bin_path, bytes).unwrap();
    let err = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap_err();
    assert!(
        matches!(err, LoadError::TerrainMalformed { .. }),
        "expected TerrainMalformed for bad magic, got {err:?}"
    );
}

#[test]
fn terrain_load_rejects_truncated_binary() {
    // A binary that ends mid-header surfaces as TerrainMalformed.
    let dir = tempdir().unwrap();
    let bin_path = dir.path().join("0_0.heightmap.bin");
    std::fs::write(&bin_path, b"WTRN\x01\x00").unwrap();
    // Also need a chunk JSON pointing at it.
    std::fs::write(
        dir.path().join("0_0.json"),
        r#"{
  "_format": 1,
  "coord": [0, 0],
  "metadata": { "eagerness": "eager", "neighbors": [], "interlocks": [] },
  "light_state": "warehouse-day-3",
  "placements": [],
  "regions": [],
  "terrain": { "heightmap_file": "0_0.heightmap.bin" }
}"#,
    )
    .unwrap();
    let err = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap_err();
    assert!(
        matches!(err, LoadError::TerrainMalformed { .. }),
        "expected TerrainMalformed for truncated binary, got {err:?}"
    );
}

#[test]
fn terrain_load_rejects_unsupported_binary_version() {
    // Magic OK, format_version = 99 (unsupported). Surfaces as TerrainMalformed.
    let dir = tempdir().unwrap();
    save_chunk(
        dir.path(),
        &make_chunk_with_terrain(ChunkCoord::new(0, 0), fixture_terrain()),
    )
    .unwrap();
    let bin_path = dir.path().join("0_0.heightmap.bin");
    let mut bytes = std::fs::read(&bin_path).unwrap();
    bytes[4..6].copy_from_slice(&99u16.to_le_bytes());
    std::fs::write(&bin_path, bytes).unwrap();
    let err = load_chunk(dir.path(), ChunkCoord::new(0, 0)).unwrap_err();
    assert!(
        matches!(err, LoadError::TerrainMalformed { .. }),
        "expected TerrainMalformed for unsupported version, got {err:?}"
    );
}
