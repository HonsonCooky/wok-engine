//! One-time sample content generation: the scene the editor creates when the content directory is
//! empty, so a first run has something to draw and a second run exercises the real load path.
//!
//! Everything authored here flows through the engine's own save paths (wok-scene and wok-light
//! file IO); the editor invents no formats. The build is deterministic - fixed tables, fixed
//! arithmetic, no clock and no RNG - so two generations produce identical files, which is what the
//! determinism test pins.
//!
//! Placement uses wok-physics terrain functions per the brief: each prefab's authored bounds are
//! reduced with `world_aabb`, dropped below the terrain floor, and lifted with `resolve_heightmap`
//! (lift-only), so every placeholder rests exactly on the surface rather than sinking into a hill
//! or floating over a dip.

use std::error::Error;

use glam::{Quat, Vec3};
use wok_light::{CelParams, Fog, LightState, SkyGradient, Sun};
use wok_physics::{resolve_heightmap, world_aabb};
use wok_scene::{
    Aabb, CHUNK_GRID_DIM, CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, Eagerness, HEIGHT_MIN_M,
    Heightmap, Placement, Prefab, PrefabRef, PrefabState, Primitive, Scene, Shape, StreamingDefaults,
    SurfaceTag, Transform,
};

use crate::content::ContentPaths;

pub const SCENE_NAME: &str = "sample";
pub const LIGHT_NAME: &str = "default";

/// The sample prefabs: a slug, the unit primitive, the shape's local scale in metres, and a
/// surface tag (the editor colors placeholders by tag). Each is one solid placeholder shape.
const PREFABS: [(&str, Primitive, Vec3, &str); 4] = [
    ("crate", Primitive::Cube, Vec3::new(1.0, 1.0, 1.0), "wood"),
    ("boulder", Primitive::Ellipsoid, Vec3::new(2.6, 1.8, 2.2), "stone"),
    ("pillar", Primitive::Cylinder, Vec3::new(1.2, 5.0, 1.2), "stone"),
    ("marker", Primitive::Capsule, Vec3::new(0.8, 2.0, 0.8), "metal"),
];

/// The sample placements: prefab slug, chunk-local x/z in metres, yaw in degrees, uniform scale.
/// Spread around the chunk's middle so the spawn camera sees them against the hills.
const PLACEMENTS: [(&str, f32, f32, f32, f32); 8] = [
    ("crate", 52.0, 60.0, 15.0, 1.5),
    ("crate", 54.5, 61.8, 40.0, 1.0),
    ("crate", 66.0, 55.0, 70.0, 2.0),
    ("boulder", 70.0, 48.0, 0.0, 1.0),
    ("boulder", 45.0, 75.0, 110.0, 1.4),
    ("pillar", 60.0, 70.0, 0.0, 1.0),
    ("pillar", 64.0, 70.0, 0.0, 1.0),
    ("marker", 75.0, 64.0, 0.0, 1.0),
];

/// Everything one generation produces, in authored form.
pub struct SampleContent {
    pub scene: Scene,
    pub chunk: Chunk,
    pub heightmap: Heightmap,
    pub prefabs: Vec<(PrefabRef, Prefab)>,
    pub light: LightState,
}

/// Build the sample content in memory. Deterministic; the file writing lives in [`generate`].
pub fn build() -> SampleContent {
    let heightmap = sample_heightmap();
    let prefabs: Vec<(PrefabRef, Prefab)> = PREFABS
        .iter()
        .map(|&(slug, primitive, scale, surface)| (PrefabRef::new(slug), solid_prefab(primitive, scale, surface)))
        .collect();

    let mut scene = Scene {
        name: SCENE_NAME.to_string(),
        default_lighting: wok_scene::LightStateRef::new(LIGHT_NAME),
        regions: vec![],
        default_streaming: StreamingDefaults { load_radius: 2, default_eagerness: Eagerness::Eager },
        next_instance_id: wok_scene::InstanceId(0),
    };

    let mut placements = Vec::with_capacity(PLACEMENTS.len());
    for &(slug, x, z, yaw_deg, scale) in &PLACEMENTS {
        let prefab = &prefabs.iter().find(|(r, _)| r.as_str() == slug).expect("placement table names a prefab").1;
        let floating = Transform {
            translation: Vec3::new(x, 0.0, z),
            rotation: Quat::from_rotation_y(yaw_deg.to_radians()),
            scale: Vec3::splat(scale),
        };
        placements.push(Placement {
            prefab: PrefabRef::new(slug),
            instance_id: scene.allocate_instance_id(),
            transform: rest_on_terrain(prefab, floating, &heightmap),
            state: None,
        });
    }

    let chunk = Chunk { coord: ChunkCoord::new(0, 0), placements, streaming: ChunkStreaming::default() };
    SampleContent { scene, chunk, heightmap, prefabs, light: sample_light() }
}

/// Generate the sample content on disk through the engine's save paths. The only writing v0 does.
pub fn generate(paths: &ContentPaths) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(&paths.root)?;
    std::fs::create_dir_all(paths.prefab_dir())?;
    std::fs::create_dir_all(paths.lighting_dir())?;

    let content = build();
    for (name, prefab) in &content.prefabs {
        wok_scene::save_prefab(prefab, paths.prefab(name.as_str()))?;
    }
    wok_light::save_light_state(&content.light, paths.light(LIGHT_NAME))?;
    wok_scene::save_heightmap(&content.heightmap, paths.heightmap(content.chunk.coord))?;
    wok_scene::save_chunk(&content.chunk, paths.chunk(content.chunk.coord))?;
    wok_scene::save_scene(&content.scene, paths.scene())?;
    Ok(())
}

/// Rolling hills well inside the +/-32m height range: three summed waves, low frequencies for the
/// hills and a faint higher one so slopes catch distinct cel bands.
fn sample_heightmap() -> Heightmap {
    let mut heights = Vec::with_capacity(CHUNK_GRID_LEN);
    for z in 0..CHUNK_GRID_DIM {
        for x in 0..CHUNK_GRID_DIM {
            let (xf, zf) = (x as f32, z as f32);
            let h = 4.0 * (xf * 0.06).sin() * (zf * 0.05).cos()
                + 1.8 * ((xf + zf) * 0.045).sin()
                + 0.7 * (xf * 0.15).sin() * (zf * 0.13).cos();
            heights.push(Heightmap::meters_to_raw(h));
        }
    }
    Heightmap::new(heights, vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
        .expect("sample heightmap grids have the right length by construction")
}

/// A one-state prefab holding a single solid placeholder shape (hitbox and visible).
fn solid_prefab(primitive: Primitive, scale: Vec3, surface: &str) -> Prefab {
    Prefab {
        states: vec![PrefabState {
            name: "default".to_string(),
            shapes: vec![Shape {
                primitive,
                transform: Transform { scale, ..Transform::IDENTITY },
                surface: Some(SurfaceTag::new(surface)),
                is_hitbox: true,
                is_visible: true,
            }],
            mesh: None,
        }],
        default_state: "default".to_string(),
    }
}

/// A warm afternoon default. The sky horizon matches the fog color (HLD: fog color drives the
/// sky's horizon) so distant terrain dissolves into the sky instead of meeting it at a seam.
fn sample_light() -> LightState {
    LightState {
        sun: Sun { direction: Vec3::new(-0.4, -1.0, -0.3), color: Vec3::new(1.0, 0.95, 0.85) },
        ambient: Vec3::new(0.12, 0.12, 0.16),
        fog: Fog { color: Vec3::new(0.65, 0.70, 0.80), start: 60.0, end: 260.0 },
        sky: SkyGradient { horizon: Vec3::new(0.65, 0.70, 0.80), zenith: Vec3::new(0.25, 0.45, 0.85) },
        cel: CelParams { band_count: 4, transition_softness: 0.08, rim_intensity: 0.35 },
    }
}

/// Vertically correct a placement so the prefab's bounds rest on the terrain surface.
///
/// The authored bounds are reduced to a conservative chunk-local AABB (`world_aabb` over the
/// default state's shapes), dropped below the lowest representable terrain, and lifted by the
/// lift-only `resolve_heightmap`; the lifted bottom is exactly the surface under the footprint,
/// and the difference from the original bottom is the correction.
fn rest_on_terrain(prefab: &Prefab, transform: Transform, terrain: &Heightmap) -> Transform {
    let bounds = prefab_bounds(prefab, &transform);
    let drop = (HEIGHT_MIN_M - 1.0) - bounds.min.y;
    let dropped = Aabb::new(bounds.min + Vec3::Y * drop, bounds.max + Vec3::Y * drop);
    let rested = resolve_heightmap(dropped, terrain);
    let lift = rested.min.y - bounds.min.y;
    Transform { translation: transform.translation + Vec3::Y * lift, ..transform }
}

/// Conservative AABB of a placed prefab: the union of `world_aabb` over its default state's
/// shapes, composed `placement * shape` exactly as the slicer composes them.
fn prefab_bounds(prefab: &Prefab, transform: &Transform) -> Aabb {
    let placement = transform.to_mat4();
    let state = prefab
        .states
        .iter()
        .find(|s| s.name == prefab.default_state)
        .expect("sample prefabs have a valid default state");
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for shape in &state.shapes {
        let b = world_aabb(shape.primitive, placement * shape.transform.to_mat4());
        min = min.min(b.min);
        max = max.max(b.max);
    }
    Aabb::new(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn build_is_deterministic() {
        let a = build();
        let b = build();
        assert_eq!(a.scene, b.scene);
        assert_eq!(a.chunk, b.chunk);
        assert_eq!(a.heightmap, b.heightmap);
        assert_eq!(a.prefabs, b.prefabs);
        assert_eq!(a.light, b.light);
    }

    #[test]
    fn placements_rest_on_the_surface() {
        let content = build();
        let prefabs: std::collections::HashMap<_, _> = content.prefabs.iter().cloned().collect();
        for placement in &content.chunk.placements {
            let prefab = &prefabs[&placement.prefab];
            let bounds = prefab_bounds(prefab, &placement.transform);
            // The rest puts the bounds' bottom exactly at the highest footprint sample, the same
            // five points resolve_heightmap reads: four bottom corners plus the center.
            let center = (bounds.min + bounds.max) * 0.5;
            let samples = [
                (bounds.min.x, bounds.min.z),
                (bounds.max.x, bounds.min.z),
                (bounds.min.x, bounds.max.z),
                (bounds.max.x, bounds.max.z),
                (center.x, center.z),
            ];
            let ground = samples
                .iter()
                .map(|&(x, z)| content.heightmap.height_at(x, z))
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                (bounds.min.y - ground).abs() < 1e-3,
                "{:?} bottom {} should rest at ground {}",
                placement.prefab,
                bounds.min.y,
                ground
            );
        }
    }

    #[test]
    fn instance_ids_are_unique_and_the_counter_advanced() {
        let content = build();
        let mut ids: Vec<u32> = content.chunk.placements.iter().map(|p| p.instance_id.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), content.chunk.placements.len());
        assert_eq!(content.scene.next_instance_id.0, content.chunk.placements.len() as u32);
    }

    #[test]
    fn placements_stay_inside_the_chunk() {
        let content = build();
        let extent = (CHUNK_GRID_DIM - 1) as f32;
        for placement in &content.chunk.placements {
            let t = placement.transform.translation;
            assert!(t.x >= 0.0 && t.x <= extent && t.z >= 0.0 && t.z <= extent, "{t:?}");
        }
    }

    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-sample-test-{}-{}", std::process::id(), n))
    }

    #[test]
    fn generated_files_load_back_identically() {
        // End-to-end through the real save and load paths: what generate writes, load_all reads,
        // and the authored forms survive the round trip. This is what makes the editor's second
        // run (load, no regeneration) trustworthy.
        let dir = unique_temp_dir();
        let paths = ContentPaths::new(dir.clone());
        generate(&paths).unwrap();
        let loaded = crate::content::load_all(&paths).unwrap();
        let built = build();

        assert_eq!(loaded.scene, built.scene);
        assert_eq!(loaded.chunks.len(), 1);
        assert_eq!(loaded.chunks[0].0, built.chunk);
        assert_eq!(loaded.chunks[0].1.as_ref(), Some(&built.heightmap));
        assert_eq!(loaded.prefabs.len(), built.prefabs.len());
        for (name, prefab) in &built.prefabs {
            assert_eq!(loaded.prefabs.get(name), Some(prefab));
        }
        assert_eq!(loaded.light, built.light);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
