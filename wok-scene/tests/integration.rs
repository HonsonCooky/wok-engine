//! End-to-end workflow tests. The unit suites cover each component in isolation; these
//! tests exercise the composition - save writes the format that load reads, load produces
//! the shape that slice consumes, the watcher notices a save in the right directory and
//! emits an event the consumer can re-load on. If any pair of pieces is subtly mismatched
//! in a way the unit tests miss, this is where it surfaces.

use std::collections::BTreeMap;
use std::time::Duration;

use tempfile::tempdir;
use wok_scene::pantry::math::{Transform, Vec3};
use wok_scene::{
    Chunk, ChunkCoord, ChunkEagerness, ChunkMetadata, FileEvent, FileWatcher, LightStateRef,
    MeshId, Prefab, PrefabId, PrefabPlacement, PrefabState, Scene, SceneId, Shape, ShapePrimitive,
    Slug, TerrainData, TriggerId, height_at, load_chunk, load_prefab_dir, save_chunk, save_prefab,
    save_scene_manifest, slice_chunk, surface_at,
};

fn slug(s: &str) -> Slug {
    Slug::new(s).unwrap()
}

/// A two-state prefab with all three valid flag combinations in the default state and a
/// single solid in the destroyed state. Mirrors the realistic shape an authored prefab
/// would have once the engine is in real use.
fn realistic_prefab() -> Prefab {
    Prefab {
        id: PrefabId(slug("crate-wooden")),
        default_state: "default".to_string(),
        states: vec![
            PrefabState {
                name: "default".to_string(),
                shapes: vec![
                    // (T, T) - solid wood placeholder.
                    Shape {
                        primitive: ShapePrimitive::Cube {
                            half_extents: Vec3::new(0.5, 0.5, 0.5),
                        },
                        transform: Transform::IDENTITY,
                        is_hitbox: true,
                        is_visible: true,
                        trigger_id: None,
                        surface_tag: Some("wood".to_string()),
                        visual_color: Some([0.55, 0.35, 0.2]),
                    },
                    // (T, F) - interaction trigger.
                    Shape {
                        primitive: ShapePrimitive::Cube {
                            half_extents: Vec3::new(0.75, 0.75, 0.75),
                        },
                        transform: Transform::IDENTITY,
                        is_hitbox: true,
                        is_visible: false,
                        trigger_id: Some(TriggerId("interact".to_string())),
                        surface_tag: None,
                        visual_color: None,
                    },
                    // (F, T) - decorative banding, no collision.
                    Shape {
                        primitive: ShapePrimitive::Cube {
                            half_extents: Vec3::new(0.55, 0.05, 0.55),
                        },
                        transform: Transform {
                            translation: Vec3::new(0.0, 0.4, 0.0),
                            ..Transform::IDENTITY
                        },
                        is_hitbox: false,
                        is_visible: true,
                        trigger_id: None,
                        surface_tag: None,
                        visual_color: Some([0.3, 0.2, 0.1]),
                    },
                ],
                mesh_override: Some(MeshId::new(slug("wooden-crate-mesh"), 1)),
                audio_cues: BTreeMap::new(),
            },
            PrefabState {
                name: "destroyed".to_string(),
                shapes: vec![Shape {
                    primitive: ShapePrimitive::Cube {
                        half_extents: Vec3::new(0.5, 0.1, 0.5),
                    },
                    transform: Transform::IDENTITY,
                    is_hitbox: true,
                    is_visible: true,
                    trigger_id: None,
                    surface_tag: Some("wood-debris".to_string()),
                    visual_color: Some([0.4, 0.2, 0.05]),
                }],
                mesh_override: None,
                audio_cues: BTreeMap::new(),
            },
        ],
    }
}

#[test]
fn full_workflow_load_and_slice() {
    let tempdir = tempdir().unwrap();
    let content_root = tempdir.path();
    let prefabs_dir = content_root.join("prefabs");
    let scene_dir = content_root.join("scenes").join("test-scene");
    std::fs::create_dir_all(&prefabs_dir).unwrap();
    std::fs::create_dir_all(&scene_dir).unwrap();

    // 1. Write the prefab, scene manifest, and chunk through the real save_* path.
    let prefab = realistic_prefab();
    save_prefab(&prefab, &prefabs_dir.join("crate-wooden.json")).unwrap();

    let scene = Scene {
        id: SceneId(slug("test-scene")),
        default_load_radius_meters: 200.0,
        default_eagerness: ChunkEagerness::Eager,
        default_light_state: LightStateRef::new(slug("test-light"), 1),
        chunks: vec![ChunkCoord::new(0, 0)],
    };
    save_scene_manifest(&scene, &scene_dir).unwrap();

    let chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("test-light"), 1),
        placements: vec![
            // Placement 0: default state at (5, 0, 7).
            PrefabPlacement {
                prefab: PrefabId(slug("crate-wooden")),
                transform: Transform {
                    translation: Vec3::new(5.0, 0.0, 7.0),
                    ..Transform::IDENTITY
                },
                state: "default".to_string(),
                instance_tag: None,
            },
            // Placement 1: destroyed state at (15, 0, 25).
            PrefabPlacement {
                prefab: PrefabId(slug("crate-wooden")),
                transform: Transform {
                    translation: Vec3::new(15.0, 0.0, 25.0),
                    ..Transform::IDENTITY
                },
                state: "destroyed".to_string(),
                instance_tag: None,
            },
        ],
        regions: Vec::new(),
        terrain: None,
    };
    save_chunk(&scene_dir, &chunk).unwrap();

    // 2. Load through the real load_* path. Verify the round-trip property at the
    //    integration level: loaded == original.
    let prefab_map = load_prefab_dir(&prefabs_dir).unwrap();
    assert_eq!(prefab_map.len(), 1);
    assert_eq!(prefab_map.get(&prefab.id).unwrap(), &prefab);

    let loaded_chunk = load_chunk(&scene_dir, ChunkCoord::new(0, 0)).unwrap();
    assert_eq!(loaded_chunk, chunk);

    // 3. Slice through the real slice path.
    let rt = slice_chunk(&loaded_chunk, &prefab_map).unwrap();

    // Placement 0 (default, 3 shapes): solid (T,T) + trigger (T,F) + visual (F,T) ->
    //   visible += 2, hitboxes += 1, triggers += 1
    // Placement 1 (destroyed, 1 shape): solid (T,T) ->
    //   visible += 1, hitboxes += 1
    // Totals: visible=3, hitboxes=2, triggers=1.
    assert_eq!(rt.visible.len(), 3);
    assert_eq!(rt.hitboxes.len(), 2);
    assert_eq!(rt.triggers.len(), 1);

    // Surface tag table is first-appearance order across hitbox-bearing shapes.
    assert_eq!(rt.surface_tag_table, vec!["wood", "wood-debris"]);

    // Visible[0]: P0/shape0 (solid wood). source_placement=0, translation = placement.
    assert_eq!(rt.visible[0].source_placement, 0);
    assert_eq!(
        rt.visible[0].local_transform.w_axis.truncate(),
        Vec3::new(5.0, 0.0, 7.0)
    );
    // Visible[1]: P0/shape2 (decorative banding). source_placement=0; shape's transform
    // adds (0, 0.4, 0) on top of the placement's (5, 0, 7).
    assert_eq!(rt.visible[1].source_placement, 0);
    assert_eq!(
        rt.visible[1].local_transform.w_axis.truncate(),
        Vec3::new(5.0, 0.4, 7.0)
    );
    // Visible[2]: P1/shape0 (debris). source_placement=1, translation = placement.
    assert_eq!(rt.visible[2].source_placement, 1);
    assert_eq!(
        rt.visible[2].local_transform.w_axis.truncate(),
        Vec3::new(15.0, 0.0, 25.0)
    );

    // Hitbox[0]: P0/shape0 wood. surface_tag index 0.
    assert_eq!(rt.hitboxes[0].source_placement, 0);
    assert_eq!(rt.hitboxes[0].surface_tag, 0);
    // Hitbox[1]: P1/shape0 debris. surface_tag index 1.
    assert_eq!(rt.hitboxes[1].source_placement, 1);
    assert_eq!(rt.hitboxes[1].surface_tag, 1);

    // Trigger[0]: P0/shape1 interact volume.
    assert_eq!(rt.triggers[0].source_placement, 0);
    assert_eq!(
        rt.triggers[0].trigger_id,
        TriggerId("interact".to_string())
    );
}

#[test]
fn workflow_hot_reload() {
    let tempdir = tempdir().unwrap();
    let content_root = tempdir.path();
    let prefabs_dir = content_root.join("prefabs");
    let scene_dir = content_root.join("scenes").join("test-scene");
    std::fs::create_dir_all(&prefabs_dir).unwrap();
    std::fs::create_dir_all(&scene_dir).unwrap();
    std::fs::create_dir_all(content_root.join("lights")).unwrap();

    // 1. Initial state on disk.
    let prefab = realistic_prefab();
    save_prefab(&prefab, &prefabs_dir.join("crate-wooden.json")).unwrap();
    let scene = Scene {
        id: SceneId(slug("test-scene")),
        default_load_radius_meters: 200.0,
        default_eagerness: ChunkEagerness::Eager,
        default_light_state: LightStateRef::new(slug("test-light"), 1),
        chunks: vec![ChunkCoord::new(0, 0)],
    };
    save_scene_manifest(&scene, &scene_dir).unwrap();
    let mut chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("test-light"), 1),
        placements: vec![PrefabPlacement {
            prefab: PrefabId(slug("crate-wooden")),
            transform: Transform {
                translation: Vec3::new(5.0, 0.0, 7.0),
                ..Transform::IDENTITY
            },
            state: "default".to_string(),
            instance_tag: None,
        }],
        regions: Vec::new(),
        terrain: None,
    };
    save_chunk(&scene_dir, &chunk).unwrap();

    // 2. Install the watcher AFTER initial state is on disk, then settle and drain any
    //    startup ghosts so the post-modify poll sees only the change we cause.
    std::thread::sleep(Duration::from_millis(50));
    let mut watcher = FileWatcher::new(content_root).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    let _ = watcher.poll();

    // 3. Initial load + slice. One placement, default state -> 2 visible (solid + banding)
    //    + 1 hitbox + 1 trigger.
    let prefab_map = load_prefab_dir(&prefabs_dir).unwrap();
    let rt_initial = slice_chunk(
        &load_chunk(&scene_dir, ChunkCoord::new(0, 0)).unwrap(),
        &prefab_map,
    )
    .unwrap();
    assert_eq!(rt_initial.visible.len(), 2);
    assert_eq!(rt_initial.hitboxes.len(), 1);
    assert_eq!(rt_initial.triggers.len(), 1);

    // 4. Modify the chunk on disk: add a destroyed-state placement at (20, 0, 30).
    chunk.placements.push(PrefabPlacement {
        prefab: PrefabId(slug("crate-wooden")),
        transform: Transform {
            translation: Vec3::new(20.0, 0.0, 30.0),
            ..Transform::IDENTITY
        },
        state: "destroyed".to_string(),
        instance_tag: None,
    });
    save_chunk(&scene_dir, &chunk).unwrap();

    // 5. Wait for the debouncer and poll. Expect ChunkChanged at (0, 0).
    std::thread::sleep(Duration::from_millis(250));
    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            FileEvent::ChunkChanged { coord, .. } if *coord == ChunkCoord::new(0, 0)
        )),
        "expected ChunkChanged at (0, 0); got {events:?}"
    );

    // 6. Re-load and re-slice. New placement adds 1 visible + 1 hitbox; the destroyed
    //    state contributes "wood-debris" as a new surface tag.
    let rt_reloaded = slice_chunk(
        &load_chunk(&scene_dir, ChunkCoord::new(0, 0)).unwrap(),
        &prefab_map,
    )
    .unwrap();
    assert_eq!(rt_reloaded.visible.len(), 3);
    assert_eq!(rt_reloaded.hitboxes.len(), 2);
    assert_eq!(rt_reloaded.triggers.len(), 1);
    assert_eq!(
        rt_reloaded.surface_tag_table,
        vec!["wood", "wood-debris"],
        "second placement's surface tag should join the table"
    );

    // The new placement's visible shape should appear with source_placement=1 at (20, 0, 30).
    let new_visible = rt_reloaded
        .visible
        .iter()
        .find(|v| v.source_placement == 1)
        .expect("expected a visible shape from placement index 1 after hot reload");
    assert_eq!(
        new_visible.local_transform.w_axis.truncate(),
        Vec3::new(20.0, 0.0, 30.0)
    );
}

// ---- v0.2.0 terrain integration (plan §7 v0.2.0 integration cases) ----

const TERRAIN_VR: f32 = 32.0;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quantize_height(height_m: f32) -> u16 {
    let t = ((height_m + TERRAIN_VR) / (2.0 * TERRAIN_VR)).clamp(0.0, 1.0);
    (t * f32::from(u16::MAX)).round() as u16
}

/// A terrain whose cells are all at `height_m`, except for the cell at integer (5, 7) which
/// is bumped to `bumped_height_m`. The bump gives the sampler a position to verify a known
/// value at and lets the hot-reload test confirm that a modification flows through.
#[allow(clippy::cast_possible_truncation)]
fn fixture_terrain_with_bump(
    heightmap_file: &str,
    base_height_m: f32,
    bumped_height_m: f32,
) -> TerrainData {
    let mut heights = vec![quantize_height(base_height_m); TerrainData::CELL_COUNT];
    let w = TerrainData::CELLS_PER_AXIS as usize;
    heights[7 * w + 5] = quantize_height(bumped_height_m);
    TerrainData {
        heights: heights.into_boxed_slice(),
        surface_indices: vec![0u16; TerrainData::CELL_COUNT].into_boxed_slice(),
        surface_tags: vec!["grass".to_string()],
        vertical_range_meters: TERRAIN_VR,
        heightmap_file: heightmap_file.to_string(),
    }
}

#[test]
fn full_workflow_load_slice_sample() {
    // Plan §7 v0.2.0 integration: write a chunk JSON + sibling binary, call load_chunk +
    // slice_chunk, then exercise the three samplers at known positions. End-to-end
    // verification that authored terrain bytes flow correctly through load -> slice ->
    // sample.
    let tempdir = tempdir().unwrap();
    let scene_dir = tempdir.path().join("scenes").join("terrain-scene");
    std::fs::create_dir_all(&scene_dir).unwrap();

    let coord = ChunkCoord::new(0, 0);
    let chunk = Chunk {
        coord,
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("test-light"), 1),
        placements: Vec::new(),
        regions: Vec::new(),
        terrain: Some(fixture_terrain_with_bump("0_0.heightmap.bin", 0.0, 5.0)),
    };
    save_chunk(&scene_dir, &chunk).unwrap();
    assert!(scene_dir.join("0_0.json").exists());
    assert!(scene_dir.join("0_0.heightmap.bin").exists());

    let loaded = load_chunk(&scene_dir, coord).unwrap();
    let prefabs: std::collections::HashMap<PrefabId, Prefab> = std::collections::HashMap::new();
    let rt = slice_chunk(&loaded, &prefabs).unwrap();

    let terrain = rt.terrain.as_ref().expect("terrain present after slice");
    assert_eq!(terrain.width, TerrainData::CELLS_PER_AXIS);
    assert_eq!(rt.surface_tag_table, vec!["grass".to_string()]);

    // height_at(5, 7) should match the bumped height. Tolerance covers the u16 quantum.
    let h = height_at(&rt, 5.0, 7.0).expect("in-domain sample");
    let quantum_m = 2.0 * TERRAIN_VR / f32::from(u16::MAX);
    assert!(
        (h - 5.0).abs() <= 2.0 * quantum_m,
        "expected ~5.0 at the bumped cell, got {h}"
    );

    // height_at at a flat cell should be ~0.0.
    let h_flat = height_at(&rt, 0.0, 0.0).expect("in-domain sample");
    assert!(
        h_flat.abs() <= quantum_m,
        "expected ~0.0 at a flat cell, got {h_flat}"
    );

    // surface_at returns the only authored tag at every in-domain cell.
    assert_eq!(surface_at(&rt, 5.0, 7.0), Some("grass"));
    assert_eq!(surface_at(&rt, 64.0, 64.0), Some("grass"));
}

#[test]
fn hot_reload_terrain_modification() {
    // Plan §7 v0.2.0 integration: install the watcher, write the initial terrain, modify the
    // heightmap binary, poll the watcher for ChunkChanged, re-load + re-slice, and verify
    // the sampled value reflects the modification.
    let tempdir = tempdir().unwrap();
    let content_root = tempdir.path();
    let scene_dir = content_root.join("scenes").join("terrain-scene");
    std::fs::create_dir_all(&scene_dir).unwrap();
    std::fs::create_dir_all(content_root.join("prefabs")).unwrap();
    std::fs::create_dir_all(content_root.join("lights")).unwrap();

    let coord = ChunkCoord::new(0, 0);
    let mut chunk = Chunk {
        coord,
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("test-light"), 1),
        placements: Vec::new(),
        regions: Vec::new(),
        terrain: Some(fixture_terrain_with_bump("0_0.heightmap.bin", 0.0, 5.0)),
    };
    save_chunk(&scene_dir, &chunk).unwrap();

    // Settle, install watcher, settle, drain startup ghosts. Same shape as the v0.1.0
    // hot-reload test.
    std::thread::sleep(Duration::from_millis(50));
    let mut watcher = FileWatcher::new(content_root).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    let _ = watcher.poll();

    // Modify the terrain in memory and re-save (which rewrites both the JSON and the
    // sibling binary).
    chunk.terrain = Some(fixture_terrain_with_bump("0_0.heightmap.bin", 0.0, 12.0));
    save_chunk(&scene_dir, &chunk).unwrap();
    std::thread::sleep(Duration::from_millis(250));

    let events = watcher.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            FileEvent::ChunkChanged { coord: c, .. } if *c == coord
        )),
        "expected ChunkChanged after terrain modification; got {events:?}"
    );

    // Re-load + re-slice + sample: the bumped height now reflects the modification.
    let prefabs: std::collections::HashMap<PrefabId, Prefab> = std::collections::HashMap::new();
    let reloaded = load_chunk(&scene_dir, coord).unwrap();
    let rt = slice_chunk(&reloaded, &prefabs).unwrap();
    let h = height_at(&rt, 5.0, 7.0).expect("in-domain sample");
    let quantum_m = 2.0 * TERRAIN_VR / f32::from(u16::MAX);
    assert!(
        (h - 12.0).abs() <= 2.0 * quantum_m,
        "expected ~12.0 after hot-reload, got {h}"
    );
}
