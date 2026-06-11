//! One-time sample content generation: the scene the editor creates when the content directory is
//! empty, so a first run has something to draw and a second run exercises the real load path.
//!
//! Everything authored here flows through the engine's own save paths (wok-scene and wok-light
//! file IO); the editor invents no formats. The build is deterministic - fixed tables, fixed
//! arithmetic, no clock and no RNG - so two generations produce identical files, which is what the
//! determinism test pins.
//!
//! Placement rests each prefab on the terrain by a per-prefab policy (a code-level mapping in the
//! prefab table; placement policy is application-side by design). Boxy prefabs reduce their
//! authored bounds with `world_aabb` and rest by wok-physics's `resolve_heightmap` corner rest, so
//! no corner hovers or sinks. Round or organic prefabs rest their bounds' bottom on the terrain
//! sample under the placement centre (`height_at`), optionally sunk a few centimetres: corner rest
//! would perch a curved underside on its highest bounds corner and float it on any slope.

use std::error::Error;

use glam::{Quat, Vec3};
use wok_light::{CelParams, Fog, LightState, SkyGradient, Sun};
use wok_scene::{
    CHUNK_GRID_DIM, CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, Eagerness, Heightmap,
    Placement, Prefab, PrefabRef, PrefabState, Primitive, Scene, Shape, StreamingDefaults,
    SurfaceTag, Transform,
};

use crate::content::ContentPaths;
use crate::place::{Rest, rest_on_terrain};

pub const SCENE_NAME: &str = "sample";
pub const LIGHT_NAME: &str = "default";

/// The sample prefabs: a slug, the unit primitive, the shape's local scale in metres, a surface
/// tag (the editor colors placeholders by tag), and the placement rest policy. Each is one solid
/// placeholder shape.
const PREFABS: [(&str, Primitive, Vec3, &str, Rest); 4] = [
    ("crate", Primitive::Cube, Vec3::new(1.0, 1.0, 1.0), "wood", Rest::Corner),
    // Uniform on purpose: a uniformly scaled ellipsoid is exactly a sphere, so the boulder
    // classifies as a Sphere collider instead of its conservative box (the old 2.6 x 1.8 x 2.2
    // was AABB-grade). 2.2 is the old axes' geometric mean, so the volume in the scene reads the
    // same. The sink grew with the curvature: the sphere's underside is rounder than the squashed
    // ellipsoid's (bottom curvature radius 1.1 vs ~1.6), so matching the old contact patch
    // (sqrt(2 * R * sink) ~ 0.4m) needs ~0.08 rather than 0.05.
    ("boulder", Primitive::Ellipsoid, Vec3::splat(2.2), "stone", Rest::Center { sink_m: 0.08 }),
    ("pillar", Primitive::Cylinder, Vec3::new(1.2, 5.0, 1.2), "stone", Rest::Corner),
    ("marker", Primitive::Capsule, Vec3::new(0.8, 2.0, 0.8), "metal", Rest::Center { sink_m: 0.02 }),
];

/// The sample placements: prefab slug, chunk-local x/z in metres, yaw in degrees, uniform scale.
/// Spread around the chunk's middle so the spawn camera sees them against the hills.
///
/// Solid boxes stay axis-aligned: a yawed cube collides as its conservative world AABB, which
/// reaches past the drawn faces and gives the player an invisible standable shelf (the
/// phantom-shelf finding). An oriented-box collider is parked until an authored scene wants a
/// rotated solid box; yaw on round prefabs is free (a sphere or vertical cylinder spun about Y is
/// itself).
const PLACEMENTS: [(&str, f32, f32, f32, f32); 8] = [
    ("crate", 52.0, 60.0, 0.0, 1.5),
    ("crate", 54.5, 61.8, 0.0, 1.0),
    ("crate", 66.0, 55.0, 0.0, 2.0),
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
        .map(|&(slug, primitive, scale, surface, _)| (PrefabRef::new(slug), solid_prefab(primitive, scale, surface)))
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
        let rest = rest_policy(slug);
        let floating = Transform {
            translation: Vec3::new(x, 0.0, z),
            rotation: Quat::from_rotation_y(yaw_deg.to_radians()),
            scale: Vec3::splat(scale),
        };
        placements.push(Placement {
            prefab: PrefabRef::new(slug),
            instance_id: scene.allocate_instance_id(),
            transform: rest_on_terrain(prefab, rest, floating, &heightmap),
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
        cel: CelParams { band_count: 32, transition_softness: 0.08, rim_intensity: 0.35 },
    }
}

/// The rest policy for a prefab slug, from the prefab table.
fn rest_policy(slug: &str) -> Rest {
    PREFABS.iter().find(|p| p.0 == slug).expect("placement table names a prefab").4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::prefab_bounds;
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
    fn placements_rest_on_the_surface_per_their_policy() {
        let content = build();
        let prefabs: std::collections::HashMap<_, _> = content.prefabs.iter().cloned().collect();
        for placement in &content.chunk.placements {
            let prefab = &prefabs[&placement.prefab];
            let bounds = prefab_bounds(prefab, &placement.transform);
            let expected = match rest_policy(placement.prefab.as_str()) {
                // Corner rest puts the bounds' bottom exactly at the highest footprint sample,
                // the same five points resolve_heightmap reads: four bottom corners plus the
                // center.
                Rest::Corner => {
                    let center = (bounds.min + bounds.max) * 0.5;
                    let samples = [
                        (bounds.min.x, bounds.min.z),
                        (bounds.max.x, bounds.min.z),
                        (bounds.min.x, bounds.max.z),
                        (bounds.max.x, bounds.max.z),
                        (center.x, center.z),
                    ];
                    samples
                        .iter()
                        .map(|&(x, z)| content.heightmap.height_at(x, z))
                        .fold(f32::NEG_INFINITY, f32::max)
                }
                // Center rest puts it at the sample under the placement centre, minus the sink.
                Rest::Center { sink_m } => {
                    let t = placement.transform.translation;
                    content.heightmap.height_at(t.x, t.z) - sink_m
                }
            };
            assert!(
                (bounds.min.y - expected).abs() < 1e-3,
                "{:?} bottom {} should rest at {}",
                placement.prefab,
                bounds.min.y,
                expected
            );
        }
    }

    #[test]
    fn sliced_sample_hitboxes_classify_into_their_true_collider_shapes() {
        // The same store-to-world reduction the game runs over loaded chunks: slice, then classify
        // each hitbox. The boulder's uniform scale is the point - it is what makes it a Sphere
        // collider here instead of its conservative box - and the upright pillar comes out a true
        // vertical cylinder. Cube and capsule placeholders stay boxes.
        use wok_physics::{Collider, classify_collider};
        let content = build();
        let prefabs: std::collections::HashMap<_, _> = content.prefabs.iter().cloned().collect();
        let sliced = wok_scene::slice_chunk(&content.chunk, &prefabs).expect("the sample chunk slices");
        assert_eq!(sliced.hitboxes.len(), content.chunk.placements.len());
        for hitbox in &sliced.hitboxes {
            let collider = classify_collider(hitbox.primitive, hitbox.transform);
            match hitbox.primitive {
                Primitive::Ellipsoid => {
                    assert!(matches!(collider, Collider::Sphere { .. }), "boulder: {collider:?}");
                }
                Primitive::Cylinder => {
                    assert!(matches!(collider, Collider::VertCylinder { .. }), "pillar: {collider:?}");
                }
                _ => assert!(matches!(collider, Collider::Aabb(_)), "{:?}: {collider:?}", hitbox.primitive),
            }
        }
    }

    #[test]
    fn box_collided_placements_stay_axis_aligned() {
        // The phantom-shelf guard: prefabs that collide as conservative AABBs (the cube crates and
        // the capsule marker) must not be yawed, or the AABB outgrows the drawn shape and the
        // player can stand on the invisible margin. Round prefabs may yaw freely.
        let content = build();
        for placement in &content.chunk.placements {
            let aabb_grade = matches!(placement.prefab.as_str(), "crate" | "marker");
            if aabb_grade {
                assert_eq!(
                    placement.transform.rotation,
                    Quat::IDENTITY,
                    "{:?} collides as its AABB and must stay axis-aligned",
                    placement.prefab
                );
            }
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
