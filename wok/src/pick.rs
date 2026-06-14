//! Viewport picking: a small-radius sphere swept along the camera ray, built entirely from
//! wok-physics machinery.
//!
//! The probe is a zero-length capsule (`a == b`, the documented sphere degenerate) swept by
//! `sweep_capsule_colliders` against each placement's classified colliders - the identical
//! `classify_collider` reduction the game's simulation uses, so the editor picks exactly the
//! surface the player would hit. The nearest time of impact wins; ties keep the first placement
//! in iteration order (chunks in coordinate order, placements in authored order), which is
//! deterministic.
//!
//! Terrain has no swept query in scope, so it is sampled instead: a fixed-step march along the
//! ray over `height_at`, bisected at the first crossing. Editor policy, not engine math - the
//! engine supplies the sampling function. A terrain hit in front of every placement hit resolves
//! to no selection (clicking the ground deselects, v1); the same march gives place mode its
//! clicked terrain point.

use std::collections::{BTreeMap, HashMap};

use glam::{Mat4, Vec2, Vec3};
use wok_physics::{Capsule, Collider, classify_collider, sweep_capsule_colliders};
use wok_scene::{Chunk, ChunkCoord, Heightmap, Placement, Prefab, PrefabRef};

use crate::model::{EditorModel, Selection, chunk_at, chunk_origin};

/// Probe sphere radius in metres: forgiving enough to land thin shapes (a plane's edge), small
/// enough that adjacent placements stay separately clickable.
const PICK_RADIUS: f32 = 0.05;

/// Terrain march step in metres. Half a metre against a 1m-resolution heightmap cannot skip a
/// cell-scale feature wholesale; the bisection then localizes the crossing.
const TERRAIN_STEP_M: f32 = 0.5;

/// Bisection iterations after the march finds a crossing: 24 halvings of a 0.5m bracket is far
/// below millimetre precision.
const TERRAIN_BISECT_ITERS: usize = 24;

/// Where a ray met the terrain: the fraction of the ray's full range, and the world-space point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainHit {
    pub t: f32,
    pub point: Vec3,
}

/// The world-space ray direction through a cursor position, from the frame's camera. Inverting
/// the view-projection keeps the projection parameters (fov, aspect, near) in one place - the
/// camera's matrix - instead of restating them here. `None` for a degenerate target size or a
/// non-invertible matrix.
pub fn cursor_ray(view_proj: Mat4, eye: Vec3, cursor: Vec2, size: Vec2) -> Option<Vec3> {
    if size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let ndc = Vec2::new(2.0 * cursor.x / size.x - 1.0, 1.0 - 2.0 * cursor.y / size.y);
    let inverse = view_proj.inverse();
    // Any depth strictly inside wgpu's 0..1 clip range works; the ray passes through the
    // unprojected point regardless of which depth was picked.
    let through = inverse.project_point3(Vec3::new(ndc.x, ndc.y, 0.5));
    let dir = (through - eye).normalize_or_zero();
    if dir == Vec3::ZERO || !dir.is_finite() { None } else { Some(dir) }
}

/// A placement's classified colliders in world space: every shape of its resolved state that
/// manifests in the world (solid, trigger, or visual-only), classified chunk-locally and lifted
/// by translation - the order `classify_collider` documents as exactly commuting. Visual-only
/// shapes are included on purpose: an editor must be able to select what it draws, and a trigger
/// cage is only inspectable if clicking it works.
pub fn placement_colliders(prefab: &Prefab, placement: &Placement, origin: Vec3) -> Vec<Collider> {
    let state_name = placement.state.as_deref().unwrap_or(prefab.default_state.as_str());
    let Some(state) = prefab.states.iter().find(|s| s.name == state_name) else { return vec![] };
    let placement_mat = placement.transform.to_mat4();
    state
        .shapes
        .iter()
        .filter(|shape| shape.is_hitbox || shape.is_visible)
        .map(|shape| {
            classify_collider(shape.primitive, placement_mat * shape.transform.to_mat4())
                .translated(origin)
        })
        .collect()
}

/// Every loaded placement's colliders except the current selection's, in world space: the static
/// set the Object-mode camera boom springs against (`crate::orbit`). The selection is excluded
/// because the camera frames it - its colliders sit at the orbit pivot and would otherwise collapse
/// the boom straight onto it. Built from the same [`placement_colliders`] the picker uses, so the
/// camera springs off exactly the surfaces the player and the picker see.
pub fn camera_statics(model: &EditorModel) -> Vec<Collider> {
    let mut out = Vec::new();
    for (&coord, chunk) in &model.chunks {
        let origin = chunk_origin(coord);
        for placement in &chunk.placements {
            if model.selection.contains(Selection { coord, id: placement.instance_id }) {
                continue;
            }
            if let Some(prefab) = model.prefabs.get(&placement.prefab) {
                out.extend(placement_colliders(prefab, placement, origin));
            }
        }
    }
    out
}

/// Pick the placement under a ray: nearest swept-sphere impact over every placement's colliders,
/// unless the terrain is hit first, which resolves to no selection.
pub fn pick(
    chunks: &BTreeMap<ChunkCoord, Chunk>,
    prefabs: &HashMap<PrefabRef, Prefab>,
    heightmaps: &BTreeMap<ChunkCoord, Heightmap>,
    eye: Vec3,
    dir: Vec3,
    range: f32,
) -> Option<Selection> {
    let probe = Capsule::new(eye, eye, PICK_RADIUS);
    let delta = dir * range;

    let mut best: Option<(f32, Selection)> = None;
    for (&coord, chunk) in chunks {
        let origin = chunk_origin(coord);
        for placement in &chunk.placements {
            let Some(prefab) = prefabs.get(&placement.prefab) else { continue };
            let colliders = placement_colliders(prefab, placement, origin);
            if let Some(hit) = sweep_capsule_colliders(&probe, delta, &colliders)
                && best.is_none_or(|(toi, _)| hit.toi < toi)
            {
                best = Some((hit.toi, Selection { coord, id: placement.instance_id }));
            }
        }
    }

    let (toi, selection) = best?;
    match terrain_hit(heightmaps, eye, dir, range) {
        Some(terrain) if terrain.t < toi => None,
        _ => Some(selection),
    }
}

/// Every placement whose origin projects inside a screen-space rectangle: the marquee box-select.
/// The box is two opposite corners in physical pixels (any order); a placement is included when
/// its world-space origin (its chunk origin plus the placement's translation) projects to a point
/// inside the box. Center-in-rect for v1 - the origin point, not the projected bounds - so a large
/// placement whose centre sits just outside the box is missed; bounds-overlap is a later
/// refinement. Placements behind the camera are skipped (their clip `w` is non-positive, so they
/// have no real screen position). Iteration is chunk-coordinate then authored order, so the result
/// is deterministic and its last element is the primary the selection set will frame.
pub fn pick_rect(
    chunks: &BTreeMap<ChunkCoord, Chunk>,
    view_proj: Mat4,
    size: Vec2,
    corner_a: Vec2,
    corner_b: Vec2,
) -> Vec<Selection> {
    let mut out = Vec::new();
    if size.x <= 0.0 || size.y <= 0.0 {
        return out;
    }
    let lo = corner_a.min(corner_b);
    let hi = corner_a.max(corner_b);
    for (&coord, chunk) in chunks {
        let origin = chunk_origin(coord);
        for placement in &chunk.placements {
            let clip = view_proj * (origin + placement.transform.translation).extend(1.0);
            if clip.w <= 0.0 {
                continue; // behind the camera: no screen position
            }
            let ndc = clip.truncate() / clip.w;
            let screen = Vec2::new((ndc.x + 1.0) * 0.5 * size.x, (1.0 - ndc.y) * 0.5 * size.y);
            if screen.x >= lo.x && screen.x <= hi.x && screen.y >= lo.y && screen.y <= hi.y {
                out.push(Selection { coord, id: placement.instance_id });
            }
        }
    }
    out
}

/// March the ray against the loaded terrain: the first point where the ray passes below the
/// heightmap surface, bisected to precision. `None` when the ray never crosses loaded terrain
/// within `range`.
pub fn terrain_hit(
    heightmaps: &BTreeMap<ChunkCoord, Heightmap>,
    eye: Vec3,
    dir: Vec3,
    range: f32,
) -> Option<TerrainHit> {
    let below = |t: f32| -> bool {
        let p = eye + dir * t;
        ground_at(heightmaps, p).is_some_and(|ground| p.y <= ground)
    };

    let steps = (range / TERRAIN_STEP_M).ceil() as usize;
    let mut prev = 0.0_f32;
    for i in 0..=steps {
        let t = (i as f32 * TERRAIN_STEP_M).min(range);
        if below(t) {
            // Crossed between `prev` (above or off-terrain) and `t`: bisect, treating
            // off-terrain as above so the bracket stays valid across a chunk edge.
            let (mut lo, mut hi) = (prev, t);
            for _ in 0..TERRAIN_BISECT_ITERS {
                let mid = 0.5 * (lo + hi);
                if below(mid) { hi = mid } else { lo = mid }
            }
            return Some(TerrainHit { t: hi / range, point: eye + dir * hi });
        }
        prev = t;
    }
    None
}

/// The terrain height under a world-space point, when its chunk has a heightmap.
fn ground_at(heightmaps: &BTreeMap<ChunkCoord, Heightmap>, point: Vec3) -> Option<f32> {
    let coord = chunk_at(point);
    let local = point - chunk_origin(coord);
    heightmaps.get(&coord).map(|hm| hm.height_at(local.x, local.z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::FlyCamera;
    use glam::Quat;
    use wok_scene::{
        CHUNK_GRID_LEN, ChunkStreaming, InstanceId, PrefabState, Primitive, Shape, SurfaceTag,
        Transform,
    };

    fn ball_prefab() -> Prefab {
        Prefab {
            states: vec![PrefabState {
                name: "default".to_string(),
                shapes: vec![Shape {
                    primitive: Primitive::Ellipsoid,
                    transform: Transform::IDENTITY,
                    surface: None,
                    is_hitbox: true,
                    is_visible: true,
                }],
                mesh: None,
            }],
            default_state: "default".to_string(),
        }
    }

    fn world(placements: &[(u32, Vec3)]) -> (BTreeMap<ChunkCoord, Chunk>, HashMap<PrefabRef, Prefab>) {
        let chunk = Chunk {
            coord: ChunkCoord::new(0, 0),
            placements: placements
                .iter()
                .map(|&(id, at)| Placement {
                    prefab: PrefabRef::new("ball"),
                    instance_id: InstanceId(id),
                    name: None,
                    transform: Transform { translation: at, ..Transform::IDENTITY },
                    state: None,
                })
                .collect(),
            streaming: ChunkStreaming::default(),
        };
        let mut chunks = BTreeMap::new();
        chunks.insert(chunk.coord, chunk);
        let mut prefabs = HashMap::new();
        prefabs.insert(PrefabRef::new("ball"), ball_prefab());
        (chunks, prefabs)
    }

    fn flat_heightmap(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
            .unwrap()
    }

    #[test]
    fn the_cursor_at_screen_center_rays_along_the_camera_forward() {
        let cam = FlyCamera { position: Vec3::new(3.0, 8.0, -2.0), yaw: 0.8, pitch: -0.3, speed: 1.0 };
        let size = Vec2::new(1600.0, 900.0);
        let dir = cursor_ray(cam.view_proj(size.x / size.y, 500.0), cam.position, size * 0.5, size)
            .expect("center ray exists");
        assert!((dir - cam.forward()).length() < 1e-4, "dir {dir:?} vs forward {:?}", cam.forward());
    }

    #[test]
    fn an_off_center_cursor_rays_toward_that_side() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, speed: 1.0 };
        let size = Vec2::new(1000.0, 1000.0);
        let dir = cursor_ray(cam.view_proj(1.0, 500.0), cam.position, Vec2::new(900.0, 500.0), size)
            .expect("ray exists");
        assert!(dir.x > 0.1, "cursor right of center should ray toward +x: {dir:?}");
    }

    #[test]
    fn the_nearest_of_two_overlapping_colliders_wins_in_either_order() {
        let near = Vec3::new(64.0, 10.0, 60.0);
        let far = Vec3::new(64.0, 10.0, 50.0);
        let eye = Vec3::new(64.0, 10.0, 70.0);
        let dir = Vec3::NEG_Z; // through both spheres
        let empty = BTreeMap::new();
        for order in [[(1, near), (2, far)], [(2, far), (1, near)]] {
            let (chunks, prefabs) = world(&order);
            let hit = pick(&chunks, &prefabs, &empty, eye, dir, 100.0).expect("hits");
            assert_eq!(hit.id, InstanceId(1), "the nearer sphere wins regardless of order");
        }
    }

    #[test]
    fn a_ray_into_the_sky_misses_cleanly() {
        let (chunks, prefabs) = world(&[(1, Vec3::new(64.0, 5.0, 64.0))]);
        let empty = BTreeMap::new();
        let hit = pick(&chunks, &prefabs, &empty, Vec3::new(64.0, 20.0, 64.0), Vec3::Y, 100.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn a_terrain_hit_in_front_of_a_placement_resolves_to_no_selection() {
        // Flat ground at y = 5; the ball is fully buried below it. A click down at the ground
        // sweeps into the ball, but the terrain crossing comes first, so nothing selects.
        let (chunks, prefabs) = world(&[(1, Vec3::new(64.0, 2.0, 64.0))]);
        let mut heightmaps = BTreeMap::new();
        heightmaps.insert(ChunkCoord::new(0, 0), flat_heightmap(5.0));
        let eye = Vec3::new(64.0, 30.0, 64.0);
        assert_eq!(pick(&chunks, &prefabs, &heightmaps, eye, Vec3::NEG_Y, 100.0), None);
        // Without the terrain the same click selects the buried ball: the terrain is what vetoed.
        let empty = BTreeMap::new();
        assert!(pick(&chunks, &prefabs, &empty, eye, Vec3::NEG_Y, 100.0).is_some());
    }

    #[test]
    fn terrain_hit_lands_on_the_surface() {
        let mut heightmaps = BTreeMap::new();
        heightmaps.insert(ChunkCoord::new(0, 0), flat_heightmap(4.0));
        let eye = Vec3::new(30.0, 20.0, 30.0);
        let dir = (Vec3::new(60.0, 4.0, 60.0) - eye).normalize();
        let hit = terrain_hit(&heightmaps, eye, dir, 200.0).expect("ray meets the ground");
        assert!((hit.point.y - 4.0).abs() < 1e-2, "hit at y = {}", hit.point.y);
        assert!(hit.t > 0.0 && hit.t < 1.0);
    }

    #[test]
    fn terrain_hit_misses_off_the_loaded_world() {
        let mut heightmaps = BTreeMap::new();
        heightmaps.insert(ChunkCoord::new(0, 0), flat_heightmap(4.0));
        // Marching away from the loaded chunk: never below loaded terrain.
        let hit = terrain_hit(&heightmaps, Vec3::new(-10.0, 20.0, -10.0), Vec3::NEG_Z, 100.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn placement_colliders_classify_and_lift_like_the_simulation() {
        let placement = Placement {
            prefab: PrefabRef::new("ball"),
            instance_id: InstanceId(0),
            name: None,
            transform: Transform {
                translation: Vec3::new(10.0, 2.0, 10.0),
                rotation: Quat::from_rotation_y(0.5),
                scale: Vec3::splat(3.0),
            },
            state: None,
        };
        let origin = Vec3::new(128.0, 0.0, 0.0);
        let colliders = placement_colliders(&ball_prefab(), &placement, origin);
        assert_eq!(colliders.len(), 1);
        match colliders[0] {
            Collider::Sphere { center, radius } => {
                assert!((center - Vec3::new(138.0, 2.0, 10.0)).length() < 1e-4);
                assert!((radius - 1.5).abs() < 1e-4);
            }
            ref other => panic!("a uniform ellipsoid must pick as a sphere, got {other:?}"),
        }
    }

    #[test]
    fn rect_pick_includes_whats_inside_and_skips_the_outside_and_behind() {
        // Camera at the origin looking down -Z. Two balls in front, left and right of centre; one
        // behind the camera (at +Z), which has no screen position at all.
        let (chunks, _) = world(&[
            (1, Vec3::new(-5.0, 0.0, -20.0)),
            (2, Vec3::new(5.0, 0.0, -20.0)),
            (3, Vec3::new(0.0, 0.0, 20.0)),
        ]);
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, speed: 1.0 };
        let size = Vec2::new(800.0, 600.0);
        let view_proj = cam.view_proj(size.x / size.y, 100.0);
        let ids = |sels: Vec<Selection>| sels.into_iter().map(|s| s.id).collect::<Vec<_>>();

        // The whole screen catches both balls in front and never the one behind the camera.
        let all = pick_rect(&chunks, view_proj, size, Vec2::ZERO, size);
        assert_eq!(ids(all), vec![InstanceId(1), InstanceId(2)], "both in front, the behind ball skipped");

        // The left half catches only the ball left of centre; the right half only the right one.
        let left = pick_rect(&chunks, view_proj, size, Vec2::ZERO, Vec2::new(400.0, 600.0));
        assert_eq!(ids(left), vec![InstanceId(1)], "only the left ball is inside the left half");
        let right = pick_rect(&chunks, view_proj, size, Vec2::new(400.0, 0.0), size);
        assert_eq!(ids(right), vec![InstanceId(2)], "only the right ball is inside the right half");

        // Corner order does not matter: the box is normalized to min/max internally.
        let swapped = pick_rect(&chunks, view_proj, size, size, Vec2::ZERO);
        assert_eq!(ids(swapped), vec![InstanceId(1), InstanceId(2)], "opposite corners select the same set");
    }
}
