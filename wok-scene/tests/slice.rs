use std::collections::{BTreeMap, HashMap};

use wok_scene::pantry::math::{Aabb, Quat, Transform, Vec3};
use wok_scene::{
    Chunk, ChunkCoord, ChunkEagerness, ChunkMetadata, LightStateRef, Prefab, PrefabId,
    PrefabPlacement, PrefabState, RegionMarker, RegionPurpose, Shape, ShapePrimitive, SliceError,
    Slug, TriggerId, slice_chunk,
};

// ---- Fixture and helpers ----

fn slug(s: &str) -> Slug {
    Slug::new(s).unwrap()
}

// Plan section 7 fixture: one prefab with two states, one chunk with three placements in
// known order. The "default" state has all three valid flag combinations (solid, trigger,
// visual-only). The "destroyed" state has a single solid shape with a different surface
// tag, so state selection produces visibly different output.
fn fixture_prefabs() -> HashMap<PrefabId, Prefab> {
    let prefab = Prefab {
        id: PrefabId(slug("crate-wooden")),
        default_state: "default".to_string(),
        states: vec![
            PrefabState {
                name: "default".to_string(),
                shapes: vec![
                    Shape {
                        primitive: ShapePrimitive::Cube {
                            half_extents: Vec3::new(0.5, 0.5, 0.5),
                        },
                        transform: Transform::IDENTITY,
                        is_hitbox: true,
                        is_visible: true,
                        trigger_id: None,
                        surface_tag: Some("wood".to_string()),
                        visual_color: Some([0.5, 0.3, 0.1]),
                    },
                    Shape {
                        primitive: ShapePrimitive::Cube {
                            half_extents: Vec3::new(1.0, 1.0, 1.0),
                        },
                        transform: Transform::IDENTITY,
                        is_hitbox: true,
                        is_visible: false,
                        trigger_id: Some(TriggerId("interact".to_string())),
                        surface_tag: None,
                        visual_color: None,
                    },
                    Shape {
                        primitive: ShapePrimitive::Cube {
                            half_extents: Vec3::new(0.4, 0.4, 0.4),
                        },
                        transform: Transform::IDENTITY,
                        is_hitbox: false,
                        is_visible: true,
                        trigger_id: None,
                        surface_tag: None,
                        visual_color: Some([0.8, 0.6, 0.2]),
                    },
                ],
                mesh_override: None,
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
    };
    let mut prefabs = HashMap::new();
    prefabs.insert(prefab.id.clone(), prefab);
    prefabs
}

fn fixture_chunk(coord: ChunkCoord, eagerness: ChunkEagerness) -> Chunk {
    Chunk {
        coord,
        metadata: ChunkMetadata {
            eagerness,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("warehouse-day"), 3),
        placements: vec![
            PrefabPlacement {
                prefab: PrefabId(slug("crate-wooden")),
                transform: Transform {
                    translation: Vec3::new(1.0, 0.0, 0.0),
                    ..Transform::IDENTITY
                },
                state: "default".to_string(),
                instance_tag: None,
            },
            PrefabPlacement {
                prefab: PrefabId(slug("crate-wooden")),
                transform: Transform {
                    translation: Vec3::new(2.0, 0.0, 5.0),
                    ..Transform::IDENTITY
                },
                state: "destroyed".to_string(),
                instance_tag: None,
            },
            PrefabPlacement {
                prefab: PrefabId(slug("crate-wooden")),
                transform: Transform {
                    translation: Vec3::new(3.0, 0.0, 10.0),
                    ..Transform::IDENTITY
                },
                state: "default".to_string(),
                instance_tag: None,
            },
        ],
        regions: Vec::new(),
    }
}

fn make_prefab_with_one_shape(
    is_hitbox: bool,
    is_visible: bool,
    trigger_id: Option<TriggerId>,
) -> HashMap<PrefabId, Prefab> {
    let prefab = Prefab {
        id: PrefabId(slug("test")),
        default_state: "s".to_string(),
        states: vec![PrefabState {
            name: "s".to_string(),
            shapes: vec![Shape {
                primitive: ShapePrimitive::Cube {
                    half_extents: Vec3::new(0.5, 0.5, 0.5),
                },
                transform: Transform::IDENTITY,
                is_hitbox,
                is_visible,
                trigger_id,
                surface_tag: None,
                visual_color: None,
            }],
            mesh_override: None,
            audio_cues: BTreeMap::new(),
        }],
    };
    let mut prefabs = HashMap::new();
    prefabs.insert(prefab.id.clone(), prefab);
    prefabs
}

fn make_chunk_with_one_placement() -> Chunk {
    Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("l"), 1),
        placements: vec![PrefabPlacement {
            prefab: PrefabId(slug("test")),
            transform: Transform::IDENTITY,
            state: "s".to_string(),
            instance_tag: None,
        }],
        regions: Vec::new(),
    }
}

// ---- Plan section 7 tests ----

#[test]
fn t01_smoke_arrays_have_expected_counts() {
    // Plan #1. Fixture has 3 placements: P0 default (3 shapes: solid, trigger, visual),
    // P1 destroyed (1 solid shape), P2 default (3 shapes). After slicing:
    //   visible:  P0/0, P0/2, P1/0, P2/0, P2/2 (5)
    //   hitboxes: P0/0, P1/0, P2/0 (3)
    //   triggers: P0/1, P2/1 (2)
    let prefabs = fixture_prefabs();
    let chunk = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.visible.len(), 5);
    assert_eq!(rt.hitboxes.len(), 3);
    assert_eq!(rt.triggers.len(), 2);
    assert_eq!(rt.regions.len(), 0);
    assert_eq!(rt.coord, ChunkCoord::new(0, 0));
    assert_eq!(rt.eagerness, ChunkEagerness::Eager);
}

#[test]
fn t02_placement_transform_stays_chunk_local() {
    // Plan #2 - the canary for position-independence drift. Placement at (10, 0, 5) in
    // chunk (2, 1) must produce local transform with translation (10, 0, 5). NOT
    // (266, 0, 133), which would mean the chunk's world offset was wrongly composed.
    let prefabs = make_prefab_with_one_shape(true, true, None);
    let chunk = Chunk {
        coord: ChunkCoord::new(2, 1),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("l"), 1),
        placements: vec![PrefabPlacement {
            prefab: PrefabId(slug("test")),
            transform: Transform {
                translation: Vec3::new(10.0, 0.0, 5.0),
                ..Transform::IDENTITY
            },
            state: "s".to_string(),
            instance_tag: None,
        }],
        regions: Vec::new(),
    };
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    let translation = rt.visible[0].local_transform.w_axis.truncate();
    assert_eq!(translation, Vec3::new(10.0, 0.0, 5.0));
    assert_ne!(
        translation,
        Vec3::new(266.0, 0.0, 133.0),
        "world offset must NOT be composed into local_transform"
    );
}

#[test]
fn t03_composition_is_placement_times_shape() {
    // Plan #3. Shape at (1, 0, 0) inside a placement that yaws 90 degrees and translates
    // (10, 0, 0). The composed local_transform applied to the origin gives the world-of-
    // shape position. After yaw: (1, 0, 0) -> (0, 0, -1). After placement translation:
    // (10, 0, -1).
    let mut prefabs = HashMap::new();
    let prefab = Prefab {
        id: PrefabId(slug("test")),
        default_state: "s".to_string(),
        states: vec![PrefabState {
            name: "s".to_string(),
            shapes: vec![Shape {
                primitive: ShapePrimitive::Cube {
                    half_extents: Vec3::new(0.5, 0.5, 0.5),
                },
                transform: Transform {
                    translation: Vec3::new(1.0, 0.0, 0.0),
                    ..Transform::IDENTITY
                },
                is_hitbox: true,
                is_visible: true,
                trigger_id: None,
                surface_tag: None,
                visual_color: None,
            }],
            mesh_override: None,
            audio_cues: BTreeMap::new(),
        }],
    };
    prefabs.insert(prefab.id.clone(), prefab);
    let chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("l"), 1),
        placements: vec![PrefabPlacement {
            prefab: PrefabId(slug("test")),
            transform: Transform {
                translation: Vec3::new(10.0, 0.0, 0.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                scale: Vec3::ONE,
            },
            state: "s".to_string(),
            instance_tag: None,
        }],
        regions: Vec::new(),
    };
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    let p = rt.visible[0].local_transform.transform_point3(Vec3::ZERO);
    let eps = 1e-5;
    assert!((p.x - 10.0).abs() < eps, "x was {}", p.x);
    assert!(p.y.abs() < eps, "y was {}", p.y);
    assert!((p.z + 1.0).abs() < eps, "z was {}", p.z);
}

#[test]
fn t04_state_selection_produces_correct_shapes() {
    // Plan #4. Two placements use "default" (surface_tag "wood"), one uses "destroyed"
    // (surface_tag "wood-debris"). The intern table and hitbox indices verify state-aware
    // shape selection.
    let prefabs = fixture_prefabs();
    let chunk = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.surface_tag_table, vec!["wood", "wood-debris"]);
    // P0 (default) solid hitbox -> "wood" (index 0).
    assert_eq!(rt.hitboxes[0].surface_tag, 0);
    // P1 (destroyed) solid hitbox -> "wood-debris" (index 1).
    assert_eq!(rt.hitboxes[1].surface_tag, 1);
    // P2 (default) solid hitbox -> "wood" again (reused index 0).
    assert_eq!(rt.hitboxes[2].surface_tag, 0);
}

#[test]
fn t05a_solid_flag_produces_visible_and_hitbox() {
    // Plan #5 (true, true).
    let prefabs = make_prefab_with_one_shape(true, true, None);
    let chunk = make_chunk_with_one_placement();
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.visible.len(), 1);
    assert_eq!(rt.hitboxes.len(), 1);
    assert_eq!(rt.triggers.len(), 0);
}

#[test]
fn t05b_trigger_flag_produces_trigger_only() {
    // Plan #5 (true, false).
    let prefabs = make_prefab_with_one_shape(true, false, Some(TriggerId("t".to_string())));
    let chunk = make_chunk_with_one_placement();
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.visible.len(), 0);
    assert_eq!(rt.hitboxes.len(), 0);
    assert_eq!(rt.triggers.len(), 1);
    assert_eq!(rt.triggers[0].trigger_id, TriggerId("t".to_string()));
}

#[test]
fn t05c_visual_only_flag_produces_visible_only() {
    // Plan #5 (false, true).
    let prefabs = make_prefab_with_one_shape(false, true, None);
    let chunk = make_chunk_with_one_placement();
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.visible.len(), 1);
    assert_eq!(rt.hitboxes.len(), 0);
    assert_eq!(rt.triggers.len(), 0);
}

#[test]
fn t05d_both_flags_false_errors() {
    // Plan #5 (false, false) - accepted in authored, rejected at slice time per plan §7
    // validate test #4.
    let prefabs = make_prefab_with_one_shape(false, false, None);
    let chunk = make_chunk_with_one_placement();
    let err = slice_chunk(&chunk, &prefabs).unwrap_err();
    let SliceError::InvalidShape {
        placement_index,
        shape_index,
        ..
    } = err
    else {
        panic!("expected InvalidShape, got {err:?}");
    };
    assert_eq!(placement_index, 0);
    assert_eq!(shape_index, 0);
}

#[test]
fn t05e_trigger_without_id_errors() {
    // Sibling of plan #5d. Plan §3 Shape comment: trigger_id required iff hitbox &&
    // !visible. The slicer rejects this just like the (false, false) case.
    let prefabs = make_prefab_with_one_shape(true, false, None);
    let chunk = make_chunk_with_one_placement();
    let err = slice_chunk(&chunk, &prefabs).unwrap_err();
    assert!(matches!(err, SliceError::InvalidShape { .. }));
}

#[test]
fn t06_order_preserved_via_source_placement() {
    // Plan #6. Walk placements in order, shapes within state in order.
    // visible from: P0/0, P0/2, P1/0, P2/0, P2/2 -> source_placement: 0, 0, 1, 2, 2
    // hitboxes from: P0/0, P1/0, P2/0 -> source_placement: 0, 1, 2
    // triggers from: P0/1, P2/1 -> source_placement: 0, 2
    let prefabs = fixture_prefabs();
    let chunk = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(
        rt.visible
            .iter()
            .map(|v| v.source_placement)
            .collect::<Vec<_>>(),
        vec![0, 0, 1, 2, 2]
    );
    assert_eq!(
        rt.hitboxes
            .iter()
            .map(|h| h.source_placement)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(
        rt.triggers
            .iter()
            .map(|t| t.source_placement)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
}

#[test]
fn t07_unknown_prefab_errors() {
    let prefabs: HashMap<PrefabId, Prefab> = HashMap::new();
    let chunk = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    let err = slice_chunk(&chunk, &prefabs).unwrap_err();
    let SliceError::UnknownPrefab(id) = err else {
        panic!("expected UnknownPrefab, got {err:?}");
    };
    assert_eq!(id, PrefabId(slug("crate-wooden")));
}

#[test]
fn t08_unknown_state_errors() {
    let prefabs = fixture_prefabs();
    let mut chunk = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    chunk.placements[1].state = "nonexistent".to_string();
    let err = slice_chunk(&chunk, &prefabs).unwrap_err();
    let SliceError::UnknownState { prefab, state } = err else {
        panic!("expected UnknownState, got {err:?}");
    };
    assert_eq!(prefab, PrefabId(slug("crate-wooden")));
    assert_eq!(state, "nonexistent");
}

#[test]
fn t09_deterministic_across_repeated_slices() {
    // Plan #9. Same inputs, two calls, identical output via PartialEq on ChunkRuntime.
    let prefabs = fixture_prefabs();
    let chunk = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    let a = slice_chunk(&chunk, &prefabs).unwrap();
    let b = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(a, b);
}

#[test]
fn t10_position_independent_across_chunk_coords() {
    // Plan #10 - the multiplayer-determinism property as a unit test. Slicing the same
    // authored chunk wrapped in two different ChunkCoord values must produce outputs
    // identical in every field EXCEPT `coord`.
    let prefabs = fixture_prefabs();
    let chunk_origin = fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager);
    let chunk_far = fixture_chunk(ChunkCoord::new(42, -7), ChunkEagerness::Eager);
    let a = slice_chunk(&chunk_origin, &prefabs).unwrap();
    let b = slice_chunk(&chunk_far, &prefabs).unwrap();

    assert_eq!(a.visible, b.visible);
    assert_eq!(a.hitboxes, b.hitboxes);
    assert_eq!(a.triggers, b.triggers);
    assert_eq!(a.regions, b.regions);
    assert_eq!(a.surface_tag_table, b.surface_tag_table);
    assert_eq!(a.light_state, b.light_state);
    assert_eq!(a.eagerness, b.eagerness);

    assert_ne!(a.coord, b.coord);
    assert_eq!(a.coord, ChunkCoord::new(0, 0));
    assert_eq!(b.coord, ChunkCoord::new(42, -7));
}

#[test]
fn t11_surface_tag_interning_is_fifo_with_reuse() {
    // Plan #11. Surface tags ["wood", "metal", "wood"] across three hitbox shapes produce
    // table ["wood", "metal"] and indices [0, 1, 0].
    let mut prefabs = HashMap::new();
    let prefab = Prefab {
        id: PrefabId(slug("multi")),
        default_state: "s".to_string(),
        states: vec![PrefabState {
            name: "s".to_string(),
            shapes: vec![
                make_solid_with_tag(Some("wood")),
                make_solid_with_tag(Some("metal")),
                make_solid_with_tag(Some("wood")),
            ],
            mesh_override: None,
            audio_cues: BTreeMap::new(),
        }],
    };
    prefabs.insert(prefab.id.clone(), prefab);
    let chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("l"), 1),
        placements: vec![PrefabPlacement {
            prefab: PrefabId(slug("multi")),
            transform: Transform::IDENTITY,
            state: "s".to_string(),
            instance_tag: None,
        }],
        regions: Vec::new(),
    };
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.surface_tag_table, vec!["wood", "metal"]);
    assert_eq!(rt.hitboxes[0].surface_tag, 0);
    assert_eq!(rt.hitboxes[1].surface_tag, 1);
    assert_eq!(rt.hitboxes[2].surface_tag, 0);
}

fn make_solid_with_tag(tag: Option<&str>) -> Shape {
    Shape {
        primitive: ShapePrimitive::Cube {
            half_extents: Vec3::new(0.5, 0.5, 0.5),
        },
        transform: Transform::IDENTITY,
        is_hitbox: true,
        is_visible: true,
        trigger_id: None,
        surface_tag: tag.map(str::to_string),
        visual_color: None,
    }
}

#[test]
fn t12_region_purposes_round_trip_into_runtime_regions() {
    // Plan #12. Each RegionPurpose variant survives the slice transformation into a
    // RuntimeRegion with bounds and name preserved.
    let prefabs: HashMap<PrefabId, Prefab> = HashMap::new();
    let chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: LightStateRef::new(slug("l"), 1),
        placements: Vec::new(),
        regions: vec![
            RegionMarker {
                name: "fog-zone".to_string(),
                bounds: Aabb::new(Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0)),
                purpose: RegionPurpose::Fog {
                    color: [0.6, 0.7, 0.8],
                    density: 0.05,
                    distance: 200.0,
                },
            },
            RegionMarker {
                name: "lit-room".to_string(),
                bounds: Aabb::new(Vec3::new(20.0, 0.0, 20.0), Vec3::new(30.0, 5.0, 30.0)),
                purpose: RegionPurpose::Lighting {
                    state: LightStateRef::new(slug("indoor"), 5),
                },
            },
            RegionMarker {
                name: "ambient-zone".to_string(),
                bounds: Aabb::new(Vec3::new(40.0, 0.0, 40.0), Vec3::new(50.0, 5.0, 50.0)),
                purpose: RegionPurpose::Ambient {
                    color: [0.1, 0.1, 0.12],
                },
            },
        ],
    };
    let rt = slice_chunk(&chunk, &prefabs).unwrap();
    assert_eq!(rt.regions.len(), 3);
    assert_eq!(rt.regions[0].name, "fog-zone");
    assert_eq!(
        rt.regions[0].local_bounds,
        Aabb::new(Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0))
    );
    assert!(matches!(rt.regions[0].purpose, RegionPurpose::Fog { .. }));
    assert_eq!(rt.regions[1].name, "lit-room");
    assert!(matches!(
        rt.regions[1].purpose,
        RegionPurpose::Lighting { .. }
    ));
    assert_eq!(rt.regions[2].name, "ambient-zone");
    assert!(matches!(
        rt.regions[2].purpose,
        RegionPurpose::Ambient { .. }
    ));
}

#[test]
fn t13_eagerness_carried_through_arrays_unchanged() {
    // Plan #13. Slicing the same chunk with each ChunkEagerness value produces identical
    // visible/hitboxes/triggers/regions/surface_tag_table. Only ChunkRuntime.eagerness
    // differs.
    let prefabs = fixture_prefabs();
    let eager = slice_chunk(
        &fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Eager),
        &prefabs,
    )
    .unwrap();
    let lazy = slice_chunk(
        &fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Lazy),
        &prefabs,
    )
    .unwrap();
    let vista = slice_chunk(
        &fixture_chunk(ChunkCoord::new(0, 0), ChunkEagerness::Vista),
        &prefabs,
    )
    .unwrap();

    assert_eq!(eager.eagerness, ChunkEagerness::Eager);
    assert_eq!(lazy.eagerness, ChunkEagerness::Lazy);
    assert_eq!(vista.eagerness, ChunkEagerness::Vista);

    assert_eq!(eager.visible, lazy.visible);
    assert_eq!(eager.hitboxes, lazy.hitboxes);
    assert_eq!(eager.triggers, lazy.triggers);
    assert_eq!(eager.regions, lazy.regions);
    assert_eq!(eager.surface_tag_table, lazy.surface_tag_table);

    assert_eq!(lazy.visible, vista.visible);
    assert_eq!(lazy.hitboxes, vista.hitboxes);
    assert_eq!(lazy.triggers, vista.triggers);
    assert_eq!(lazy.regions, vista.regions);
    assert_eq!(lazy.surface_tag_table, vista.surface_tag_table);
}
