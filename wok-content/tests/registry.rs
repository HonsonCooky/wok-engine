//! Registry tests for plan section 7.1. Covers #1-#8 and #10 (step 2 + step 5 scope per
//! plan section 11). Test #9 (read-view after rename) is Phase B per plan section 11
//! step 14.

#![allow(clippy::similar_names)]
// The fixture types ("cube" vs "cue", "real_cube" vs "real_cue") look similar enough that
// clippy::similar_names fires across the file. Renames would be artificial; suppress here.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use pantry::math::{Aabb, Vec3};
use wok_scene::{
    AudioCueId, Chunk, ChunkCoord, ChunkEagerness, ChunkMetadata, LightStateRef, MeshId, Prefab,
    PrefabId, PrefabState, RegionMarker, RegionPurpose, Scene, SceneId, ShapePrimitive, Slug,
};
use wok_content::{AssetStatus, Registry, UsageSite};

fn slug(s: &str) -> Slug {
    Slug::new(s).expect("valid slug literal")
}

fn cube_primitive() -> ShapePrimitive {
    ShapePrimitive::Cube {
        half_extents: Vec3::new(0.5, 0.5, 0.5),
    }
}

// §7.1 #1: Empty registry → save → load → equal.
#[test]
fn t01_empty_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("registry.json");
    let reg = Registry::empty();
    reg.save(&path).expect("save empty registry");
    let back = Registry::load(&path).expect("load empty registry");
    assert_registry_equal(&reg, &back);
}

// §7.1 #2: Register mesh → returned MeshId resolves via `mesh()`.
#[test]
fn t02_register_and_lookup() {
    let mut reg = Registry::empty();
    let id = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect("register");
    let entry = reg.mesh(&id).expect("lookup");
    assert_eq!(entry.slug, slug("primitive-cube"));
    assert_eq!(entry.status, AssetStatus::Placeholder);
    assert_eq!(entry.primitive, Some(cube_primitive()));
    assert_eq!(entry.source_path, None);
}

// §7.1 #3: Register two meshes with same slug → second errors with SlugCollision.
#[test]
fn t03_slug_collision_on_register() {
    let mut reg = Registry::empty();
    let _ = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect("first registration");
    let err = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect_err("second registration must fail");
    match err {
        wok_content::RegistryError::SlugCollision {
            kind,
            slug: s,
            existing,
        } => {
            assert_eq!(kind, wok_content::AssetKind::Mesh);
            assert_eq!(s, slug("primitive-cube"));
            assert_eq!(existing, 0);
        }
        other => panic!("expected SlugCollision, got {other:?}"),
    }
}

// §7.1 #4: Rename mesh → mesh(old_id) still resolves (serial-based); mesh_by_slug(new) works.
#[test]
fn t04_rename_preserves_serial() {
    let mut reg = Registry::empty();
    let old_id = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect("register");
    reg.rename_mesh(&old_id, slug("placeholder-cube"))
        .expect("rename");
    // Serial-based equality: the old id still resolves.
    let entry = reg.mesh(&old_id).expect("serial lookup still resolves");
    assert_eq!(entry.slug, slug("placeholder-cube"));
    // New slug lookup yields a MeshId with the same serial.
    let new_id = reg
        .mesh_by_slug(&slug("placeholder-cube"))
        .expect("new slug resolves");
    assert_eq!(new_id.serial(), old_id.serial());
    // Old slug no longer resolves.
    assert!(reg.mesh_by_slug(&slug("primitive-cube")).is_none());
}

// §7.1 #5: Rename to slug already in use by a different serial → SlugCollision.
#[test]
fn t05_rename_to_taken_slug_errors() {
    let mut reg = Registry::empty();
    let a = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect("register a");
    let _b = reg
        .register_mesh(slug("primitive-capsule"), Some(cube_primitive()))
        .expect("register b");
    let err = reg
        .rename_mesh(&a, slug("primitive-capsule"))
        .expect_err("rename onto held slug must fail");
    match err {
        wok_content::RegistryError::SlugCollision {
            kind,
            slug: s,
            existing,
        } => {
            assert_eq!(kind, wok_content::AssetKind::Mesh);
            assert_eq!(s, slug("primitive-capsule"));
            assert_eq!(existing, 1);
        }
        other => panic!("expected SlugCollision, got {other:?}"),
    }
    // Renamer's entry slug is unchanged.
    let entry = reg.mesh(&a).expect("a still resolves");
    assert_eq!(entry.slug, slug("primitive-cube"));
}

// §7.1 #6: Populate from scene → expected MeshIds present, with UsageSite lists populated.
#[test]
fn t06_populate_records_usage() {
    let mut reg = Registry::empty();
    // Pre-register the mesh and light entries the fixture references; populate should find
    // them by slug and record usage rather than allocating new placeholders.
    let cube = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect("register cube");
    let lights = reg
        .register_light_state(slug("lights-default"), "lights/default.json".into())
        .expect("register lights");
    let crate_cue = reg
        .register_audio(slug("crate-impact"), "audio/crate-impact.ogg".into())
        .expect("register audio");

    let scene = make_scene(slug("act1"), &lights);
    let prefabs = make_prefab_set(&cube, &crate_cue);
    let chunks = make_chunks(&lights);

    reg.populate_from_scene(&scene, &prefabs, &chunks)
        .expect("populate");

    let cube_entry = reg.mesh(&cube).expect("cube still present");
    assert!(
        cube_entry.usage.iter().any(|u| matches!(
            u,
            UsageSite::PrefabState { prefab, state }
                if prefab == &PrefabId(slug("crate"))
                && state == "closed"
        )),
        "cube mesh missing prefab-state usage: {:?}",
        cube_entry.usage
    );

    let lights_entry = reg.light_state(&lights).expect("lights still present");
    let mut has_scene_default = false;
    let mut has_chunk_light = false;
    let mut has_chunk_region = false;
    for u in &lights_entry.usage {
        match u {
            UsageSite::SceneDefaultLightState { scene: s } if s == &SceneId(slug("act1")) => {
                has_scene_default = true;
            }
            UsageSite::ChunkLightState { coord, .. } if *coord == ChunkCoord::new(0, 0) => {
                has_chunk_light = true;
            }
            UsageSite::ChunkRegion {
                region_name, ..
            } if region_name == "fog-room" => {
                has_chunk_region = true;
            }
            _ => {}
        }
    }
    assert!(has_scene_default, "scene default usage missing: {:?}", lights_entry.usage);
    assert!(has_chunk_light, "chunk light usage missing: {:?}", lights_entry.usage);
    assert!(has_chunk_region, "chunk region usage missing: {:?}", lights_entry.usage);

    let cue_entry = reg.audio(&crate_cue).expect("audio still present");
    assert!(
        cue_entry.usage.iter().any(|u| matches!(
            u,
            UsageSite::PrefabAudioCue { cue_name, .. } if cue_name == "impact"
        )),
        "audio cue missing usage: {:?}",
        cue_entry.usage
    );
}

// §7.1 #7: Re-populate from same scene → no duplicate entries.
#[test]
fn t07_repopulate_is_idempotent() {
    let mut reg = Registry::empty();
    let cube = reg
        .register_mesh(slug("primitive-cube"), Some(cube_primitive()))
        .expect("register cube");
    let lights = reg
        .register_light_state(slug("lights-default"), "lights/default.json".into())
        .expect("register lights");
    let crate_cue = reg
        .register_audio(slug("crate-impact"), "audio/crate-impact.ogg".into())
        .expect("register audio");

    let scene = make_scene(slug("act1"), &lights);
    let prefabs = make_prefab_set(&cube, &crate_cue);
    let chunks = make_chunks(&lights);

    reg.populate_from_scene(&scene, &prefabs, &chunks)
        .expect("populate 1");
    let usage1: Vec<_> = reg.mesh(&cube).unwrap().usage.clone();
    reg.populate_from_scene(&scene, &prefabs, &chunks)
        .expect("populate 2");
    let usage2: Vec<_> = reg.mesh(&cube).unwrap().usage.clone();
    assert_eq!(
        usage1, usage2,
        "re-populate produced different usage lists"
    );

    // No duplicate entries: counts of live entries match what we registered + nothing more.
    // We registered: 1 mesh, 0 audio_placeholders (the cue was pre-registered too), 0 light
    // placeholders. The view has live entries we can count via the read view.
    let view = reg.read_view();
    let live_meshes = view.meshes.iter().filter(|s| s.is_some()).count();
    let live_lights = view.light_states.iter().filter(|s| s.is_some()).count();
    let live_audio = view.audio.iter().filter(|s| s.is_some()).count();
    assert_eq!(live_meshes, 1, "extra mesh entries: {:?}", view.meshes);
    assert_eq!(live_lights, 1, "extra light entries: {:?}", view.light_states);
    assert_eq!(live_audio, 1, "extra audio entries: {:?}", view.audio);
}

// §7.1 #8: Populate from scene that references unknown mesh slug → new entry created as
// placeholder.
#[test]
fn t08_unknown_slug_creates_placeholder() {
    let mut reg = Registry::empty();
    // We do NOT pre-register the cube or lights or audio cue. Populate must auto-create
    // placeholders for every referenced slug.
    let placeholder_cube = MeshId::new(slug("primitive-cube"), 42);
    let placeholder_lights = LightStateRef::new(slug("lights-default"), 99);
    let placeholder_cue = AudioCueId::new(slug("crate-impact"), 7);

    let scene = make_scene(slug("act1"), &placeholder_lights);
    let prefabs = make_prefab_set(&placeholder_cube, &placeholder_cue);
    let chunks = make_chunks(&placeholder_lights);

    reg.populate_from_scene(&scene, &prefabs, &chunks)
        .expect("populate");

    // The references quoted made-up serials; populate routed by slug and allocated fresh
    // serials. Look up by slug to find the placeholders.
    let real_cube = reg
        .mesh_by_slug(&slug("primitive-cube"))
        .expect("placeholder cube created");
    let cube_entry = reg.mesh(&real_cube).expect("cube live");
    assert_eq!(cube_entry.status, AssetStatus::Placeholder);
    assert_eq!(cube_entry.primitive, None);
    assert!(
        !cube_entry.usage.is_empty(),
        "placeholder cube should have at least one usage site"
    );

    let real_lights = reg
        .light_state_by_slug(&slug("lights-default"))
        .expect("placeholder lights created");
    let lights_entry = reg
        .light_state(&real_lights)
        .expect("lights live");
    assert_eq!(lights_entry.status, AssetStatus::Placeholder);
    assert!(
        lights_entry.usage.len() >= 2,
        "placeholder lights should have scene-default + chunk-light usage at minimum: {:?}",
        lights_entry.usage
    );

    let real_cue = reg
        .audio_by_slug(&slug("crate-impact"))
        .expect("placeholder cue created");
    let cue_entry = reg.audio(&real_cue).expect("cue live");
    assert_eq!(cue_entry.status, AssetStatus::Placeholder);
}

// §7.1 #10: Tombstone of deleted entry → round-trip preserves the tombstone slot.
//
// Phase A has no public delete API; tombstones originate from on-disk data. This test
// constructs a JSON file with a tombstone entry, loads, saves, and verifies the tombstone
// slot survives.
#[test]
fn t10_tombstone_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("registry.json");
    let json = r#"{
        "_format": 1,
        "meshes": {
            "next_serial": 3,
            "entries": [
                { "serial": 0, "slug": "primitive-cube", "status": "placeholder", "primitive": { "kind": "cube", "half_extents": [0.5, 0.5, 0.5] } },
                { "serial": 1, "slug": "_deleted_1", "status": "deleted" },
                { "serial": 2, "slug": "primitive-capsule", "status": "placeholder", "primitive": { "kind": "capsule", "radius": 0.4, "half_height": 0.9 } }
            ]
        },
        "audio": { "next_serial": 0, "entries": [] },
        "animations": { "next_serial": 0, "entries": [] },
        "voice": { "next_serial": 0, "entries": [] },
        "light_states": { "next_serial": 0, "entries": [] }
    }"#;
    std::fs::write(&path, json).expect("seed registry");
    let reg = Registry::load(&path).expect("load tombstoned registry");

    // Live entries resolve; tombstone at serial 1 does not resolve as a live mesh.
    let cube = reg
        .mesh_by_slug(&slug("primitive-cube"))
        .expect("cube resolves");
    assert_eq!(cube.serial(), 0);
    let capsule = reg
        .mesh_by_slug(&slug("primitive-capsule"))
        .expect("capsule resolves");
    assert_eq!(capsule.serial(), 2);
    // The tombstone slug is synthetic and does not own a slot in the slug map (the tombstone
    // is a serial-only marker).
    assert!(reg.mesh_by_slug(&slug("_deleted_1")).is_none());
    let unknown = MeshId::new(slug("_anything_"), 1);
    assert!(reg.mesh(&unknown).is_none());

    // Save and re-load; the tombstone must survive.
    let path2 = dir.path().join("registry-2.json");
    reg.save(&path2).expect("re-save");
    let back = Registry::load(&path2).expect("re-load");
    // The mesh table's next_serial preserves the saved value.
    let raw = std::fs::read_to_string(&path2).expect("re-read");
    assert!(raw.contains("\"_deleted_1\""), "tombstone slug present");
    assert!(raw.contains("\"deleted\""), "deleted status present");
    assert_registry_equal(&reg, &back);
}

/// Compare two registries by inspecting the public surface: every kind's by-serial lookup
/// agrees on slug+status+source+(primitive when mesh), and every by-slug lookup agrees.
/// HashMap iteration order is not part of the comparison.
fn assert_registry_equal(a: &Registry, b: &Registry) {
    let a_json = registry_dump(a);
    let b_json = registry_dump(b);
    assert_eq!(a_json, b_json, "registry round-trip diverged");
    // Read view is rebuilt on load; it should be observationally equal.
    let av = a.read_view();
    let bv = b.read_view();
    assert_eq!(av.meshes.len(), bv.meshes.len(), "mesh view length");
    assert_eq!(av.audio.len(), bv.audio.len(), "audio view length");
    assert_eq!(av.animations.len(), bv.animations.len(), "animations view length");
    assert_eq!(av.voice.len(), bv.voice.len(), "voice view length");
    assert_eq!(
        av.light_states.len(),
        bv.light_states.len(),
        "light view length"
    );
}

/// Serialize the registry to its on-disk JSON form via the public save path. Returns the
/// JSON text. The save path normalizes ordering (serial-sorted entries) so byte-identical
/// JSON is the strongest equality test we have without leaking private internals.
fn registry_dump(reg: &Registry) -> String {
    let dir = tempfile::tempdir().expect("dump tempdir");
    let path = dir.path().join("dump.json");
    reg.save(&path).expect("save dump");
    std::fs::read_to_string(&path).expect("read dump")
}

// Helper to make rustc happy when Path is unused above; pulled in to avoid a "warn: unused
// import" lint if a future test references it.
fn _path_marker(_p: &Path) {}

/// Build a minimal Scene manifest with one default light state reference and one chunk.
fn make_scene(scene_slug: Slug, lights: &LightStateRef) -> Scene {
    Scene {
        id: SceneId(scene_slug),
        default_load_radius_meters: 200.0,
        default_eagerness: ChunkEagerness::Eager,
        default_light_state: lights.clone(),
        chunks: vec![ChunkCoord::new(0, 0)],
    }
}

/// Build a minimal prefab set: one "crate" prefab with two states. The "closed" state has a
/// mesh_override pointing at the supplied cube mesh and a single audio cue. The "open"
/// state references the same mesh and a different cue name.
fn make_prefab_set(cube: &MeshId, cue: &AudioCueId) -> HashMap<PrefabId, Prefab> {
    let mut cues = BTreeMap::new();
    cues.insert("impact".to_string(), cue.clone());
    let closed = PrefabState {
        name: "closed".to_string(),
        shapes: Vec::new(),
        mesh_override: Some(cube.clone()),
        audio_cues: cues,
    };
    let mut cues2 = BTreeMap::new();
    cues2.insert("open".to_string(), cue.clone());
    let open = PrefabState {
        name: "open".to_string(),
        shapes: Vec::new(),
        mesh_override: Some(cube.clone()),
        audio_cues: cues2,
    };
    let prefab = Prefab {
        id: PrefabId(slug("crate")),
        default_state: "closed".to_string(),
        states: vec![closed, open],
    };
    let mut prefabs = HashMap::new();
    prefabs.insert(prefab.id.clone(), prefab);
    prefabs
}

/// Build a minimal chunk set: one chunk at (0, 0) that references the supplied light state
/// at the chunk level and inside a "fog-room" region marker with `Lighting` purpose.
fn make_chunks(lights: &LightStateRef) -> HashMap<ChunkCoord, Chunk> {
    let chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        metadata: ChunkMetadata {
            eagerness: ChunkEagerness::Eager,
            neighbors: Vec::new(),
            interlocks: Vec::new(),
        },
        light_state: lights.clone(),
        placements: Vec::new(),
        regions: vec![RegionMarker {
            name: "fog-room".to_string(),
            bounds: Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 5.0, 10.0)),
            purpose: RegionPurpose::Lighting {
                state: lights.clone(),
            },
        }],
        terrain: None,
    };
    let mut chunks = HashMap::new();
    chunks.insert(chunk.coord, chunk);
    chunks
}
