//! Terrain rest for the flat-bottomed cylinder: keep the disc on (not below) the heightmap.
//!
//! The capsule rest discounts each footprint sample by how far the spherical bottom has curved
//! away from it, because a round bottom really does sit lower than its rim samples. A flat disc
//! has no curve: it rests on the HIGHEST point of the terrain under it, full stop. So the
//! cylinder rest is footprint-MAX over disc samples with no profile discount - geometrically
//! correct for the shape, not a simplification - and on any non-flat ground the bearing point is
//! wherever the terrain peaks under the disc, with daylight under the rest of the bottom.
//!
//! Same conventions as [`crate::terrain`]: lift-only (a falling body is stopped, never pulled
//! down - ground glue is game policy), chunk-local frame with a purely vertical correction (the
//! determinism contract's position-independence for terrain), and the same resting epsilon.

use glam::Vec3;
use wok_scene::Heightmap;

use crate::cylinder::Cylinder;
use crate::terrain::{GROUND_EPS, TerrainRest};

/// Footprint samples on the rim circle, evenly spaced (plus one at the centre). Eight rather than
/// the capsule rest's four: with no profile discount the rim is the usual bearing feature, and on
/// a planar slope of gradient `g` the worst-case azimuth miss under-lifts by
/// `r * g * (1 - cos(pi / RIM_SAMPLES))` - under 8% of `r * g` at eight, against 30% at four.
/// Still deliberately coarse (a peak between samples can poke through), like every rest here.
const RIM_SAMPLES: usize = 8;

/// The terrain height the flat-bottomed cylinder's bottom rests on: the footprint-MAX over the disc's
/// centre and [`RIM_SAMPLES`] rim samples (a flat disc bears on the highest point under it - see the
/// module docs, no per-sample discount). This is the support [`rest_cylinder_on_heightmap`] lifts the
/// base to, exposed on its own so the game's ground-snap can read the surface height it would rest on
/// without the lift - to glue a grounded body to a surface that fell away beneath a step (walking down
/// a slope) instead of letting it float off and free-fall to catch up. Chunk-local like the rest:
/// pass a cylinder already in the terrain's frame; the returned height is in that frame.
pub fn cylinder_support_height(cylinder: &Cylinder, terrain: &Heightmap) -> f32 {
    let base = cylinder.base();
    let r = cylinder.radius;
    let mut ground = terrain.height_at(base.x, base.z);
    for i in 0..RIM_SAMPLES {
        let angle = std::f32::consts::TAU * (i as f32 / RIM_SAMPLES as f32);
        ground = ground.max(terrain.height_at(base.x + r * angle.cos(), base.z + r * angle.sin()));
    }
    ground
}

/// Lift a [`Cylinder`] so its flat bottom stays on (not below) the terrain, and report whether it
/// is grounded.
///
/// **Footprint-MAX.** The support height is the maximum raw terrain height over the centre and
/// [`RIM_SAMPLES`] rim samples of the bottom disc - no per-sample discount (see the module docs).
/// The consequence on a planar slope of gradient `g`: the up-slope rim sample is the bearing
/// contact and the base rests `r * g` above the surface under the axis (the documented gap - the
/// disc bridges from the up-slope contact over the falling ground, exactly as a rigid disc on a
/// ramp does).
///
/// **Lift only.** A cylinder resting on or above the sampled support is left where it is; a
/// sinking one is raised straight up until its base height meets the support.
///
/// **Grounded.** True when the base is at or below the sampled support (resting, not airborne,
/// within the shared millimetre epsilon) *and* the terrain normal under the axis
/// ([`Heightmap::normal_at`]) is within the walkable threshold: pass
/// `walkable_cos = cos(max_slope_angle)`, the limit the game owns.
pub fn rest_cylinder_on_heightmap(cylinder: Cylinder, terrain: &Heightmap, walkable_cos: f32) -> TerrainRest {
    let base = cylinder.base();
    let ground = cylinder_support_height(&cylinder, terrain);

    let resting = base.y <= ground + GROUND_EPS;
    let rested = if base.y < ground {
        cylinder.translated(Vec3::new(0.0, ground - base.y, 0.0))
    } else {
        cylinder
    };
    let walkable = terrain.normal_at(base.x, base.z).dot(Vec3::Y) >= walkable_cos;

    TerrainRest { position: rested.center, grounded: resting && walkable }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_scene::{CHUNK_GRID_DIM, CHUNK_GRID_LEN, SurfaceTag};

    // cos(60 deg): the walkable limit taste runs at.
    const WALKABLE_COS: f32 = 0.5;

    fn flat(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    // Terrain ramping along +x by `delta` raw units per cell (the capsule rest's fixture shape).
    fn ramp_x(base: u16, delta: u16) -> Heightmap {
        let heights = (0..CHUNK_GRID_LEN)
            .map(|i| base + (i % CHUNK_GRID_DIM) as u16 * delta)
            .collect();
        Heightmap::new(heights, vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    // The player-shaped cylinder: feet at `feet`, 1.5m tall, 0.45m radius (taste's dimensions, so
    // the rest is exercised at the shape that consumes it).
    fn player(feet: Vec3) -> Cylinder {
        Cylinder::new(feet + Vec3::new(0.0, 0.75, 0.0), 0.45, 0.75)
    }

    #[test]
    fn flat_rest_is_exact_to_the_ulp() {
        // Sunk into flat ground, the lift puts the base AT the sampled height: the correction is
        // the single add `base + (ground - base)`, so the residue is one rounding of that sum -
        // ulp-grade, not tolerance-grade. A convention bug would be centimetres.
        let terrain = flat(2.0);
        let ground = terrain.height_at(64.0, 64.0);
        let r = rest_cylinder_on_heightmap(player(Vec3::new(64.0, -5.0, 64.0)), &terrain, WALKABLE_COS);
        let base_y = r.position.y - 0.75;
        assert!(
            (base_y - ground).abs() <= 2.0 * f32::EPSILON * ground.abs(),
            "base {base_y} vs ground {ground}: gap {} exceeds ulp grade",
            base_y - ground
        );
        assert!(r.grounded);
        // Lifted straight up: x and z unchanged.
        assert_eq!(r.position.x, 64.0);
        assert_eq!(r.position.z, 64.0);
    }

    #[test]
    fn a_cylinder_already_resting_on_flat_ground_is_left_alone_and_grounded() {
        let terrain = flat(2.0);
        let ground = terrain.height_at(64.0, 64.0);
        let c = player(Vec3::new(64.0, ground, 64.0));
        let r = rest_cylinder_on_heightmap(c, &terrain, WALKABLE_COS);
        assert_eq!(r.position, c.center, "already resting: nothing to lift");
        assert!(r.grounded);
    }

    #[test]
    fn a_cylinder_above_ground_is_left_alone_and_airborne() {
        let terrain = flat(2.0);
        let c = player(Vec3::new(64.0, 10.0, 64.0));
        let r = rest_cylinder_on_heightmap(c, &terrain, WALKABLE_COS);
        assert_eq!(r.position, c.center, "lift-only: never pulled down");
        assert!(!r.grounded);
    }

    #[test]
    fn ramp_rest_bears_on_the_up_slope_rim_with_the_documented_gap() {
        // The flat-bottom convention on a planar ramp of gradient g: the up-slope rim sample (at
        // +r along x) is the support, so the base rests at height(x + r) - which puts it exactly
        // r * g above the surface under the axis. The capsule rested with ZERO gap under the
        // centre (its bottom curved down to meet the ground); the disc cannot, and this gap is
        // the geometry, not an error.
        let terrain = ramp_x(0, 300);
        let g = 300.0 * (64.0 / u16::MAX as f32); // gradient in m/m, from the quantization step
        let c = player(Vec3::new(60.5, -50.0, 64.0));
        let r = rest_cylinder_on_heightmap(c, &terrain, WALKABLE_COS);

        let base_y = r.position.y - 0.75;
        let rim_ground = terrain.height_at(60.5 + 0.45, 64.0);
        let axis_ground = terrain.height_at(60.5, 64.0);
        assert!((base_y - rim_ground).abs() < 1e-4, "base {base_y} should rest on the rim sample {rim_ground}");
        assert!(
            (base_y - axis_ground - 0.45 * g).abs() < 1e-3,
            "the gap under the axis should be r * g = {}, got {}",
            0.45 * g,
            base_y - axis_ground
        );
        assert!(r.grounded, "the ramp is well inside the walkable limit");
        // No sampled point of the disc is left underground.
        for i in 0..8 {
            let a = std::f32::consts::TAU * (i as f32 / 8.0);
            let (x, z) = (60.5 + 0.45 * a.cos(), 64.0 + 0.45 * a.sin());
            assert!(base_y >= terrain.height_at(x, z) - 1e-4, "rim sample ({x}, {z}) sinks");
        }
    }

    #[test]
    fn support_height_is_the_footprint_max_the_rest_lifts_to() {
        // The exposed support is the highest sample under the disc - the up-slope rim on a ramp - and
        // is exactly the base height the lift-only rest puts a sunk body at. The game's ground-snap
        // reads this to glue a grounded body to a descended surface without the rest's lift.
        let terrain = ramp_x(0, 300);
        let c = player(Vec3::new(60.5, 5.0, 64.0)); // floating above the ramp
        let support = cylinder_support_height(&c, &terrain);
        let rim_ground = terrain.height_at(60.5 + 0.45, 64.0);
        assert!((support - rim_ground).abs() < 1e-4, "support {support} is the up-slope rim {rim_ground}");
        // A body sunk into the same column lifts to exactly this support height.
        let lifted = rest_cylinder_on_heightmap(player(Vec3::new(60.5, -50.0, 64.0)), &terrain, WALKABLE_COS);
        assert!((lifted.position.y - 0.75 - support).abs() < 1e-4, "the rest lifts the base to the support");
    }

    #[test]
    fn the_steep_guard_rests_but_does_not_ground() {
        // A ~66-degree ramp (2250 raw per cell is ~2.2 m/m): the body is lifted (it must not sink)
        // but the surface normal fails the 60-degree walkable limit, so grounded stays false.
        let heights = (0..CHUNK_GRID_LEN)
            .map(|i| ((i % CHUNK_GRID_DIM) as u32 * 2250).min(u16::MAX as u32) as u16)
            .collect();
        let terrain = Heightmap::new(heights, vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        assert!(
            terrain.normal_at(10.0, 64.0).dot(Vec3::Y) < WALKABLE_COS,
            "fixture: the ramp must be steeper than walkable"
        );
        let r = rest_cylinder_on_heightmap(player(Vec3::new(10.0, -50.0, 64.0)), &terrain, WALKABLE_COS);
        assert!(!r.grounded, "a slope past the walkable limit must not ground");
        let base_y = r.position.y - 0.75;
        assert!(base_y > -50.0 + 0.001, "the lift must still have happened");
    }

    #[test]
    fn the_rest_is_deterministic() {
        let terrain = ramp_x(100, 250);
        let c = player(Vec3::new(40.2, -3.0, 71.7));
        assert_eq!(
            rest_cylinder_on_heightmap(c, &terrain, WALKABLE_COS),
            rest_cylinder_on_heightmap(c, &terrain, WALKABLE_COS)
        );
    }
}
