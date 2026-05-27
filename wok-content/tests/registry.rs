//! Registry tests for plan section 7.1. This file covers #1-#5 and #10 (step 2 scope per
//! plan section 11). Tests #6-#8 (populate) land in step 5; test #9 (read-view after rename)
//! is Phase B per plan section 11 step 14.

use std::path::Path;

use pantry::math::Vec3;
use wok_scene::{MeshId, ShapePrimitive, Slug};
use wok_content::{AssetStatus, Registry};

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
