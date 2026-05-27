use std::collections::BTreeMap;

use wok_scene::pantry::math::{Aabb, Transform, Vec2, Vec3};
use wok_scene::pantry::serde::Serialize;
use wok_scene::pantry::serde::de::DeserializeOwned;
use wok_scene::pantry::serde_json;
use wok_scene::{
    AudioCueId, Chunk, ChunkCoord, ChunkEagerness, ChunkMetadata, LightStateRef, MeshId, Prefab,
    PrefabId, PrefabPlacement, PrefabState, RegionMarker, RegionPurpose, Scene, SceneId, Shape,
    ShapePrimitive, Slug, TriggerId,
};

/// Serialize, deserialize, equality-check, re-serialize, byte-equal-check.
///
/// This is the in-memory version of the plan section 7 round-trip property. The disk version
/// of the test (save to temp file, load, re-save, byte-compare files) lands with load.rs and
/// save.rs at the next checkpoint.
fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("first serialize");
    let back: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(*value, back, "deserialize value matches original");
    let json2 = serde_json::to_string(&back).expect("second serialize");
    assert_eq!(json, json2, "re-serialize byte-identical");
}

fn slug(s: &str) -> Slug {
    Slug::new(s).unwrap()
}

// ---- ShapePrimitive, every variant ----

#[test]
fn shape_primitive_cube_round_trips() {
    round_trip(&ShapePrimitive::Cube {
        half_extents: Vec3::new(0.5, 0.5, 0.5),
    });
}

#[test]
fn shape_primitive_ellipsoid_round_trips() {
    round_trip(&ShapePrimitive::Ellipsoid {
        radii: Vec3::new(1.0, 2.0, 3.0),
    });
}

#[test]
fn shape_primitive_cylinder_round_trips() {
    round_trip(&ShapePrimitive::Cylinder {
        radius: 0.4,
        half_height: 1.25,
    });
}

#[test]
fn shape_primitive_capsule_round_trips() {
    round_trip(&ShapePrimitive::Capsule {
        radius: 0.4,
        half_height: 0.85,
    });
}

#[test]
fn shape_primitive_plane_round_trips() {
    round_trip(&ShapePrimitive::Plane {
        half_extents: Vec2::new(50.0, 50.0),
    });
}

// Lock the on-disk shape so a future serde-config tweak does not silently rename a field.
#[test]
fn shape_primitive_cube_json_shape_matches_plan() {
    let json = serde_json::to_string(&ShapePrimitive::Cube {
        half_extents: Vec3::new(0.5, 0.5, 0.5),
    })
    .unwrap();
    assert_eq!(json, r#"{"kind":"cube","half_extents":[0.5,0.5,0.5]}"#);
}

#[test]
fn shape_primitive_plane_json_uses_vec2() {
    let json = serde_json::to_string(&ShapePrimitive::Plane {
        half_extents: Vec2::new(10.0, 20.0),
    })
    .unwrap();
    assert_eq!(json, r#"{"kind":"plane","half_extents":[10.0,20.0]}"#);
}

// ---- Shape ----

#[test]
fn shape_with_all_optional_fields_set_round_trips() {
    let s = Shape {
        primitive: ShapePrimitive::Cube {
            half_extents: Vec3::new(0.5, 0.5, 0.5),
        },
        transform: Transform {
            translation: Vec3::new(0.0, 0.5, 0.0),
            ..Transform::IDENTITY
        },
        is_hitbox: true,
        is_visible: true,
        trigger_id: Some(TriggerId("door-opens".to_string())),
        surface_tag: Some("wood".to_string()),
        visual_color: Some([0.55, 0.35, 0.2]),
    };
    round_trip(&s);
}

#[test]
fn shape_with_no_optional_fields_omits_them_in_json() {
    let s = Shape {
        primitive: ShapePrimitive::Cube {
            half_extents: Vec3::new(0.5, 0.5, 0.5),
        },
        transform: Transform::IDENTITY,
        is_hitbox: true,
        is_visible: true,
        trigger_id: None,
        surface_tag: None,
        visual_color: None,
    };
    let json = serde_json::to_string(&s).unwrap();
    assert!(!json.contains("trigger_id"));
    assert!(!json.contains("surface_tag"));
    assert!(!json.contains("visual_color"));
    round_trip(&s);
}

// ---- ChunkEagerness, every variant ----

#[test]
fn chunk_eagerness_eager_serializes_as_lowercase_string() {
    let json = serde_json::to_string(&ChunkEagerness::Eager).unwrap();
    assert_eq!(json, r#""eager""#);
    round_trip(&ChunkEagerness::Eager);
}

#[test]
fn chunk_eagerness_lazy_round_trips() {
    let json = serde_json::to_string(&ChunkEagerness::Lazy).unwrap();
    assert_eq!(json, r#""lazy""#);
    round_trip(&ChunkEagerness::Lazy);
}

#[test]
fn chunk_eagerness_vista_round_trips() {
    let json = serde_json::to_string(&ChunkEagerness::Vista).unwrap();
    assert_eq!(json, r#""vista""#);
    round_trip(&ChunkEagerness::Vista);
}

// ---- RegionPurpose, every variant ----

#[test]
fn region_purpose_fog_round_trips() {
    round_trip(&RegionPurpose::Fog {
        color: [0.6, 0.7, 0.8],
        density: 0.05,
        distance: 200.0,
    });
}

#[test]
fn region_purpose_lighting_round_trips_and_matches_plan_shape() {
    let lp = RegionPurpose::Lighting {
        state: LightStateRef::new(slug("warehouse-office"), 7),
    };
    let json = serde_json::to_string(&lp).unwrap();
    assert_eq!(json, r#"{"kind":"lighting","state":"warehouse-office-7"}"#);
    round_trip(&lp);
}

#[test]
fn region_purpose_ambient_round_trips() {
    round_trip(&RegionPurpose::Ambient {
        color: [0.1, 0.1, 0.12],
    });
}

// ---- Prefab ----

#[test]
fn prefab_round_trips() {
    let p = Prefab {
        id: PrefabId(slug("crate-wooden")),
        default_state: "default".to_string(),
        states: vec![
            PrefabState {
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
                audio_cues: BTreeMap::from([
                    (
                        "impact".to_string(),
                        AudioCueId::new(slug("wood-impact"), 12),
                    ),
                    (
                        "footstep".to_string(),
                        AudioCueId::new(slug("wood-step"), 7),
                    ),
                ]),
            },
            PrefabState {
                name: "destroyed".to_string(),
                shapes: vec![Shape {
                    primitive: ShapePrimitive::Cube {
                        half_extents: Vec3::new(0.5, 0.1, 0.5),
                    },
                    transform: Transform {
                        translation: Vec3::new(0.0, 0.1, 0.0),
                        ..Transform::IDENTITY
                    },
                    is_hitbox: true,
                    is_visible: true,
                    trigger_id: None,
                    surface_tag: Some("wood-debris".to_string()),
                    visual_color: Some([0.4, 0.25, 0.15]),
                }],
                mesh_override: None,
                audio_cues: BTreeMap::new(),
            },
        ],
    };
    round_trip(&p);
}

#[test]
fn prefab_state_audio_cues_serialize_alphabetically_sorted() {
    let state = PrefabState {
        name: "default".to_string(),
        shapes: Vec::new(),
        mesh_override: None,
        audio_cues: BTreeMap::from([
            (
                "impact".to_string(),
                AudioCueId::new(slug("wood-impact"), 12),
            ),
            (
                "footstep".to_string(),
                AudioCueId::new(slug("wood-step"), 7),
            ),
            ("open".to_string(), AudioCueId::new(slug("door-open"), 3)),
        ]),
    };
    let json = serde_json::to_string(&state).unwrap();
    let footstep_pos = json.find("footstep").unwrap();
    let impact_pos = json.find("impact").unwrap();
    let open_pos = json.find("open").unwrap();
    assert!(footstep_pos < impact_pos, "footstep before impact");
    assert!(impact_pos < open_pos, "impact before open");
}

// ---- Scene ----

#[test]
fn scene_round_trips() {
    let s = Scene {
        id: SceneId(slug("act1-warehouse")),
        default_load_radius_meters: 200.0,
        default_eagerness: ChunkEagerness::Eager,
        default_light_state: LightStateRef::new(slug("warehouse-day"), 3),
        chunks: vec![
            ChunkCoord::new(0, 0),
            ChunkCoord::new(1, 0),
            ChunkCoord::new(0, 1),
        ],
    };
    round_trip(&s);
}

// ---- Chunk ----

#[test]
fn chunk_round_trips() {
    let c = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: vec![ChunkCoord::new(1, 0), ChunkCoord::new(0, 1)],
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
    };
    round_trip(&c);
}

#[test]
fn chunk_with_all_eagerness_values_round_trips() {
    for eagerness in [
        ChunkEagerness::Eager,
        ChunkEagerness::Lazy,
        ChunkEagerness::Vista,
    ] {
        let c = Chunk {
            coord: ChunkCoord::new(0, 0),
            metadata: ChunkMetadata {
                eagerness,
                neighbors: Vec::new(),
                interlocks: Vec::new(),
            },
            light_state: LightStateRef::new(slug("warehouse-day"), 3),
            placements: Vec::new(),
            regions: Vec::new(),
            terrain: None,
        };
        round_trip(&c);
    }
}
