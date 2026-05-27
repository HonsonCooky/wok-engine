//! Workspace integration test driven by the on-disk fixtures at
//! `wok-engine/tests-integration/fixtures/`. Plan section 11 step 10:
//!
//! > Phase A test cases verify wok-content-only behavior: scene loads, chunk reaches
//! > Resident, transition to Vista works, registry populates correctly, terrain mesh
//! > appears in slot's GPU handles when chunk has terrain data.
//!
//! The committed fixture has no heightmap binary (see `fixtures/README.md`); this test
//! copies the JSON files into a tempdir and writes a flat 129x129 heightmap binary so the
//! load path exercises the terrain pipeline. Future integration tests in the
//! `tests-integration/` crate can reuse the same setup.

#![allow(clippy::similar_names)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use pantry::serde_json;
use wok_scene::{ChunkCoord, ChunkEagerness, Slug};
use wok_content::{
    AssetStatus, ContentConfig, ContentEvent, ContentSystem, SlotState,
};

/// Path to the committed fixture directory, relative to the wok-content crate's manifest.
fn fixtures_source_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests-integration")
        .join("fixtures")
}

/// Copy the committed fixture tree into `dest`, then generate the heightmap binary at
/// `dest/scenes/room/0_0.heightmap.bin`. Returns the absolute scene_dir path.
fn install_fixtures(dest: &Path) -> PathBuf {
    let src = fixtures_source_dir();
    copy_dir_recursive(&src, dest).expect("copy fixtures");
    write_flat_heightmap(&dest.join("scenes").join("room").join("0_0.heightmap.bin"))
        .expect("write heightmap");
    dest.join("scenes").join("room")
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Build a flat 129x129 heightmap binary at `path`. Layout matches wok-scene's
/// `authored::terrain::write_sibling` (header + 1 surface tag "grass" + cells of height
/// 32768 and surface index 0). The mid-range u16 height maps to 0m given a 32m vertical
/// range.
fn write_flat_heightmap(path: &Path) -> std::io::Result<()> {
    let cells_per_axis: u32 = 129;
    let cell_count = (cells_per_axis * cells_per_axis) as usize;
    let surface_tag = b"grass";
    let vertical_range_mm: u32 = 32_000;

    let mut bytes: Vec<u8> = Vec::with_capacity(16 + 2 + surface_tag.len() + cell_count * 4);
    // Magic "WTRN"
    bytes.extend_from_slice(b"WTRN");
    // Format version 1
    bytes.extend_from_slice(&1u16.to_le_bytes());
    // Width / height in cells (129 each)
    bytes.extend_from_slice(&(cells_per_axis as u16).to_le_bytes());
    bytes.extend_from_slice(&(cells_per_axis as u16).to_le_bytes());
    // surface_tag_count
    bytes.extend_from_slice(&1u16.to_le_bytes());
    // vertical_range_mm (LE u32)
    bytes.extend_from_slice(&vertical_range_mm.to_le_bytes());
    // Surface tag entry: 2-byte length + bytes
    bytes.extend_from_slice(&(surface_tag.len() as u16).to_le_bytes());
    bytes.extend_from_slice(surface_tag);
    // Heights: u16 = 32768 = mid-range = 0 meters
    for _ in 0..cell_count {
        bytes.extend_from_slice(&32_768u16.to_le_bytes());
    }
    // Surface indices: all 0 (grass)
    for _ in 0..cell_count {
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    fs::write(path, bytes)
}

fn boot_system(content_root: &Path) -> ContentSystem {
    let (device, queue) = common::init_gpu();
    ContentSystem::new(
        device,
        queue,
        content_root.to_owned(),
        ContentConfig::default(),
    )
    .expect("ContentSystem::new")
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

#[test]
fn fixtures_integration_runs_phase_a_load_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scene_dir = install_fixtures(tmp.path());
    let mut system = boot_system(tmp.path());

    // 1. Scene loads.
    system.load_scene(&scene_dir).expect("load_scene");
    let scene_loaded_events = system.poll();
    assert!(
        scene_loaded_events
            .iter()
            .any(|e| matches!(e, ContentEvent::SceneLoaded(_))),
        "expected SceneLoaded event: {scene_loaded_events:?}"
    );

    // 2. Registry populates correctly.
    //
    // The fixture's registry.json has one light state entry ("ambient" at serial 0). The
    // scene/chunk reference the "ambient-0" token which decodes to slug "ambient", serial
    // 0. populate_from_scene looked up by slug ("ambient"), found the existing entry, and
    // recorded SceneDefaultLightState + ChunkLightState usage.
    let reg = system.registry();
    let ambient = reg
        .light_state_by_slug(&Slug::new("ambient").unwrap())
        .expect("ambient light state in registry");
    let entry = reg.light_state(&ambient).expect("entry");
    assert_eq!(entry.status, AssetStatus::Placeholder);
    assert!(
        entry.usage.iter().any(|u| matches!(
            u,
            wok_content::UsageSite::SceneDefaultLightState { .. }
        )),
        "scene default usage recorded"
    );
    assert!(
        entry.usage.iter().any(|u| matches!(
            u,
            wok_content::UsageSite::ChunkLightState { .. }
        )),
        "chunk light usage recorded"
    );

    // 3. Chunk reaches Resident.
    let coord = ChunkCoord::new(0, 0);
    drain_to_resident(&mut system, coord);
    let slot = system.slot(coord).expect("slot");
    let SlotState::Resident(rc) = &slot.state else {
        panic!("expected Resident, got {}", slot.state.label())
    };

    // 4. Terrain mesh appears in the slot's GPU handles (chunk has terrain data).
    assert!(
        rc.gpu.terrain.is_some(),
        "terrain mesh must be present on chunk with TerrainData"
    );
    // The flat heightmap produces a 129x129 grid. Index count = 128 * 128 * 6.
    let expected_indices = 128u32 * 128 * 6;
    assert_eq!(
        rc.gpu.terrain.as_ref().unwrap().index_count,
        expected_indices,
        "terrain mesh index count"
    );

    // 5. Visible meshes for placements (player, floor, walls, box, plus the trigger-pad
    // contributes no visible shape because its only shape is hitbox-only).
    let visible_count = rc.gpu.visible.len();
    let runtime_visible = rc.runtime.visible.len();
    assert_eq!(visible_count, runtime_visible);
    assert!(
        runtime_visible >= 7,
        "expected at least 7 visible shapes (floor + 4 walls + capsule + box), got {runtime_visible}"
    );
    let trigger_count = rc.runtime.triggers.len();
    assert_eq!(trigger_count, 1, "exactly one trigger volume from trigger-pad");

    // 6. Transition to Vista works.
    system
        .transition_chunk(coord, ChunkEagerness::Vista)
        .expect("transition_chunk");
    let events = system.poll();
    assert!(
        events.iter().any(|e| matches!(
            e,
            ContentEvent::ChunkTransitioned { coord: c, to, .. }
                if *c == coord && *to == ChunkEagerness::Vista
        )),
        "ChunkTransitioned event missing: {events:?}"
    );
    // active_slots excludes; vista_slots includes.
    assert!(system.active_slots().count() == 0);
    assert!(system.vista_slots().count() == 1);
}

fn _unused() {
    // Force the serde_json import to count as used so future expansions don't trip an
    // unused-import lint.
    let _ = serde_json::Value::Null;
}
