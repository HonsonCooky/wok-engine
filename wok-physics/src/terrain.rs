//! Terrain collision: rest a body on a chunk [`Heightmap`].
//!
//! Two queries, both lift-only and both deliberately coarse: keep a body from sinking through the
//! ground, and report whether it is resting on walkable ground. Precise swept contact against the
//! triangulated surface (and sliding along terrain slopes) is a later refinement; not sinking, plus
//! a grounded signal, is the goal now and is enough for basic locomotion on gentle ground.
//!
//! - [`resolve_heightmap`] rests an AABB (part 1's player box).
//! - [`rest_on_heightmap`] rests a [`Capsule`] and also reports `grounded`.
//!
//! Both run in the heightmap's frame: x and z are chunk-local metres and up is `+Y`. The correction
//! is purely vertical, so the result is invariant to the chunk's horizontal world offset once the
//! game has mapped world coordinates into chunk-local space, which is what honours the determinism
//! contract's position-independence for terrain.

use glam::Vec3;
use wok_scene::{Aabb, Heightmap};

use crate::bounds::{aabb_center, aabb_translated};
use crate::capsule::Capsule;

/// Rest the player box on the terrain beneath it, returning the corrected box.
///
/// **Footprint sampling.** Terrain height is read at five points of the box's footprint: its four
/// bottom corners and its centre. The player is rested on the highest of those samples, so no
/// sampled corner is left underground. This is deliberately coarse: a sharp peak that falls between
/// the five samples can still poke through, which denser sampling would catch, but for 1m-resolution
/// terrain and a roughly player-sized box it keeps every corner on or above the surface, which is
/// the goal for this step.
///
/// **Lift only.** If the box already sits on or above the sampled ground it is returned unchanged. A
/// falling body is stopped by the surface but never pulled down onto it; sticking a body to the
/// ground is a game decision, not physics, so it is not done here.
///
/// **Coordinates.** `x` and `z` are read as chunk-local metres, the frame the heightmap samples in
/// (out-of-range values clamp to the chunk edge, per [`Heightmap::height_at`]). The correction is
/// purely vertical, so the result is invariant to the chunk's horizontal world offset once the game
/// has mapped world coordinates into chunk-local space before the call. That mapping is what honours
/// the determinism contract's position-independence requirement for terrain.
pub fn resolve_heightmap(player: Aabb, terrain: &Heightmap) -> Aabb {
    let c = aabb_center(&player);
    let samples = [
        (player.min.x, player.min.z),
        (player.max.x, player.min.z),
        (player.min.x, player.max.z),
        (player.max.x, player.max.z),
        (c.x, c.z),
    ];
    let mut ground = f32::NEG_INFINITY;
    for (x, z) in samples {
        ground = ground.max(terrain.height_at(x, z));
    }

    if player.min.y < ground {
        aabb_translated(&player, Vec3::new(0.0, ground - player.min.y, 0.0))
    } else {
        player
    }
}

/// The outcome of resting a capsule on terrain: the resolved capsule-centre position and whether it
/// is standing on walkable ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainRest {
    /// Resolved capsule-centre position ([`Capsule::center`]) after the lift.
    pub position: Vec3,
    /// True when the capsule is resting on the surface and that surface is no steeper than the
    /// walkable threshold.
    pub grounded: bool,
}

/// Lift a [`Capsule`] so its base stays on (not below) the terrain, and report whether it is
/// grounded.
///
/// **Footprint sampling.** The ground height is the highest of five samples under the base: its
/// centre and four points a `radius` out along +/-x and +/-z. Taking the max keeps the rounded
/// bottom from dipping below the surface where it overhangs a rise; like part 1's box rest this is
/// coarse (a peak between samples can still poke through) but right for 1m terrain and a
/// player-sized capsule.
///
/// **Lift only.** A capsule resting on or above the sampled ground is left where it is; a sinking
/// one is raised straight up until its base meets the ground. A falling body is stopped by the
/// surface but never pulled down onto it - sticking to the ground is a game decision, not physics.
///
/// **Grounded.** True when the base is at or below the sampled ground (so the capsule is resting,
/// not airborne) *and* the terrain normal under the base ([`Heightmap::normal_at`]) is within the
/// walkable-slope threshold of straight up: `normal.dot(Y) >= walkable_cos`. Pass
/// `walkable_cos = cos(max_slope_angle)`, the same limit [`crate::collide_and_slide`] takes.
pub fn rest_on_heightmap(capsule: Capsule, terrain: &Heightmap, walkable_cos: f32) -> TerrainRest {
    let base = capsule.base();
    let r = capsule.radius;
    let samples = [
        (base.x, base.z),
        (base.x - r, base.z),
        (base.x + r, base.z),
        (base.x, base.z - r),
        (base.x, base.z + r),
    ];
    let mut ground = f32::NEG_INFINITY;
    for (x, z) in samples {
        ground = ground.max(terrain.height_at(x, z));
    }

    let resting = base.y <= ground + GROUND_EPS;
    let rested = if base.y < ground {
        capsule.translated(Vec3::new(0.0, ground - base.y, 0.0))
    } else {
        capsule
    };
    let walkable = terrain.normal_at(base.x, base.z).dot(Vec3::Y) >= walkable_cos;

    TerrainRest { position: rested.center(), grounded: resting && walkable }
}

/// How far below the sampled ground the base may sit and still count as resting (1mm), absorbing
/// the quantization and float noise between a settled body and the surface it sits on.
const GROUND_EPS: f32 = 1e-3;

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::motion::{Motion, integrate};
    use wok_scene::{CHUNK_GRID_DIM, CHUNK_GRID_LEN, SurfaceTag};

    // Flat terrain at a nominal height (quantized by the heightmap; tests compare against the
    // sampled height, never the nominal metres).
    fn flat(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(
            vec![raw; CHUNK_GRID_LEN],
            vec![SurfaceTag::new("g")],
            vec![0; CHUNK_GRID_LEN],
        )
        .unwrap()
    }

    // Terrain that ramps along +x by `delta` raw units per cell, independent of z (a slope facing
    // -x). Mirrors wok-scene's own ramp fixture so the height at an integer x is exact.
    fn ramp_x(base: u16, delta: u16) -> Heightmap {
        let heights = (0..CHUNK_GRID_LEN)
            .map(|i| base + (i % CHUNK_GRID_DIM) as u16 * delta)
            .collect();
        Heightmap::new(heights, vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    #[test]
    fn player_above_ground_is_left_alone() {
        let terrain = flat(2.0);
        let player = Aabb::from_center_extents(Vec3::new(64.0, 10.0, 64.0), Vec3::splat(0.5));
        assert_eq!(resolve_heightmap(player, &terrain), player);
    }

    #[test]
    fn player_sunk_into_ground_is_lifted_to_rest_on_it() {
        let terrain = flat(2.0);
        let ground = terrain.height_at(64.0, 64.0);
        // Underside at y = -0.5, well below the ground.
        let player = Aabb::from_center_extents(Vec3::new(64.0, 0.0, 64.0), Vec3::splat(0.5));
        let r = resolve_heightmap(player, &terrain);
        assert!((r.min.y - ground).abs() < 1e-4, "min.y = {}", r.min.y);
        // Lifted straight up: the horizontal centre does not move.
        assert_eq!(aabb_center(&r).x, 64.0);
        assert_eq!(aabb_center(&r).z, 64.0);
    }

    #[test]
    fn player_on_a_slope_rests_on_its_highest_footprint_sample() {
        let terrain = ramp_x(0, 200);
        // Footprint x in [10, 12], z in [63, 65]. Height rises with x, so the +x corner is highest.
        let player = Aabb::from_center_extents(Vec3::new(11.0, -50.0, 64.0), Vec3::new(1.0, 0.5, 1.0));
        let r = resolve_heightmap(player, &terrain);

        let highest = terrain.height_at(12.0, 64.0);
        assert!((r.min.y - highest).abs() < 1e-4, "min.y = {}", r.min.y);

        // No corner of the footprint is left below the surface.
        for (x, z) in [(10.0, 63.0), (12.0, 63.0), (10.0, 65.0), (12.0, 65.0), (11.0, 64.0)] {
            assert!(
                r.min.y >= terrain.height_at(x, z) - 1e-4,
                "corner ({x}, {z}) sinks: min.y = {} vs ground {}",
                r.min.y,
                terrain.height_at(x, z),
            );
        }
    }

    #[test]
    fn player_falls_under_gravity_and_comes_to_rest_on_the_surface() {
        // The intended per-step composition: integrate under gravity, then resolve terrain. The
        // game owns this loop; here it stands in to show the pieces compose to a resting body.
        let terrain = flat(5.0);
        let ground = terrain.height_at(64.0, 64.0);
        let half = Vec3::splat(0.5);
        let gravity = Vec3::new(0.0, -9.8, 0.0);
        let dt = 1.0 / 60.0;

        let mut m = Motion { position: Vec3::new(64.0, 20.0, 64.0), velocity: Vec3::ZERO };
        for _ in 0..600 {
            m = integrate(m, gravity, dt);
            let resolved = resolve_heightmap(Aabb::from_center_extents(m.position, half), &terrain);
            let corrected = aabb_center(&resolved);
            // Landing (the box was lifted) stops the fall; this velocity clamp is the game's call.
            if corrected.y > m.position.y {
                m.velocity.y = 0.0;
            }
            m.position = corrected;
        }

        let rest = Aabb::from_center_extents(m.position, half);
        assert!((rest.min.y - ground).abs() < 1e-3, "came to rest at min.y = {}", rest.min.y);
        assert!(rest.min.y >= ground - 1e-3, "must not sink through the ground");
    }

    // ---- capsule terrain rest ----

    // cos(45 deg): the steepest slope still walkable in these tests.
    const WALKABLE_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;

    // A ramp rising steeply along +x (about 55 degrees near low x). Clamped so the far columns do
    // not overflow u16; sample only at low x, where the gradient is the full steep slope.
    fn steep_ramp_x() -> Heightmap {
        let heights = (0..CHUNK_GRID_LEN)
            .map(|i| ((i % CHUNK_GRID_DIM) as u32 * 1500).min(u16::MAX as u32) as u16)
            .collect();
        Heightmap::new(heights, vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    // An upright player capsule (2m tall, 0.5m radius) whose feet sit at `feet`.
    fn player(feet: Vec3) -> Capsule {
        Capsule::upright(feet + Vec3::new(0.0, 1.0, 0.0), 2.0, 0.5)
    }

    #[test]
    fn capsule_on_flat_ground_reads_grounded() {
        let terrain = flat(2.0);
        let ground = terrain.height_at(64.0, 64.0);
        let c = player(Vec3::new(64.0, ground, 64.0));
        let r = rest_on_heightmap(c, &terrain, WALKABLE_COS);
        assert!(r.grounded, "feet on flat ground should be grounded");
        // Already resting exactly on the surface: nothing to lift.
        assert!((r.position - c.center()).length() < 1e-4, "should not move, moved to {:?}", r.position);
    }

    #[test]
    fn capsule_on_a_steep_slope_is_not_grounded() {
        let terrain = steep_ramp_x();
        // Sunk into the ramp at low x, so it gets lifted and is resting, but the slope is too steep.
        let c = player(Vec3::new(10.0, -50.0, 64.0));
        let r = rest_on_heightmap(c, &terrain, WALKABLE_COS);
        // Confirm the slope really is steeper than the threshold.
        assert!(terrain.normal_at(10.0, 64.0).dot(Vec3::Y) < WALKABLE_COS, "fixture not steep enough");
        assert!(!r.grounded, "a slope past the walkable limit must not be grounded");
    }

    #[test]
    fn capsule_above_ground_is_left_alone_and_airborne() {
        let terrain = flat(2.0);
        let c = player(Vec3::new(64.0, 10.0, 64.0));
        let r = rest_on_heightmap(c, &terrain, WALKABLE_COS);
        assert_eq!(r.position, c.center(), "a capsule above ground is not moved");
        assert!(!r.grounded, "well above the ground is not grounded");
    }

    #[test]
    fn capsule_sunk_into_ground_is_lifted_to_rest_on_it() {
        let terrain = flat(2.0);
        let ground = terrain.height_at(64.0, 64.0);
        // Feet start at y = -5, far below the surface.
        let c = player(Vec3::new(64.0, -5.0, 64.0));
        let r = rest_on_heightmap(c, &terrain, WALKABLE_COS);
        // Centre sits one metre above the feet; resting puts the feet (base) on the ground.
        let rested_base_y = r.position.y - 1.0;
        assert!((rested_base_y - ground).abs() < 1e-4, "base rested at {} vs ground {}", rested_base_y, ground);
        assert!(r.grounded);
        // Lifted straight up: x and z are unchanged.
        assert_eq!(r.position.x, 64.0);
        assert_eq!(r.position.z, 64.0);
    }

    #[test]
    fn capsule_falls_under_gravity_and_comes_to_rest_grounded() {
        // The intended composition: integrate under gravity, then rest on terrain. Game-owned here.
        let terrain = flat(5.0);
        let ground = terrain.height_at(64.0, 64.0);
        let gravity = Vec3::new(0.0, -9.8, 0.0);
        let dt = 1.0 / 60.0;

        let mut m = Motion { position: Vec3::new(64.0, 20.0, 64.0), velocity: Vec3::ZERO };
        let mut grounded = false;
        for _ in 0..600 {
            m = integrate(m, gravity, dt);
            let rest = rest_on_heightmap(Capsule::upright(m.position, 2.0, 0.5), &terrain, WALKABLE_COS);
            // Landing (the capsule was lifted) stops the fall; this velocity clamp is the game's call.
            if rest.position.y > m.position.y {
                m.velocity.y = 0.0;
            }
            m.position = rest.position;
            grounded = rest.grounded;
        }

        let base_y = m.position.y - 1.0;
        assert!((base_y - ground).abs() < 1e-3, "came to rest with base at {} vs ground {}", base_y, ground);
        assert!(grounded, "resting on flat ground should report grounded");
    }
}
