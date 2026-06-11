//! The player's collide-and-slide, with the supported-ground flat resolve: the edge-drift fix.
//!
//! The engine's `collide_and_slide` resolves every contact against its true normal - correct
//! geometry, and unchanged. The feel problem is policy: standing supported on walkable-but-tilted
//! geometry (a boulder's flank near the apex, a gently tilted platform), each step's gravity
//! probe projects onto the tilted contact plane and bleeds a horizontal component, so the player
//! creeps off. Measured before the fix: 0.14m over two seconds on a boulder flank 0.2m off the
//! apex, 0.13m on a 10-degree tilted platform, both while grounded the whole time. Flat box tops
//! never drifted (with the axis inside the footprint the contact normal is already +Y); the drift
//! is the rounded-bottom-on-tilted-surface case the slipperiness reports describe.
//!
//! The policy, not the shape: this taste-owned slide runs the engine's loop (same skin, same
//! iteration cap, the engine's public sweep, so wall and airborne behavior is bit-identical) but
//! resolves a contact as GROUND when two things hold at the moment of contact: the normal grades
//! as ground (`normal.y >= WALKABLE_NORMAL_Y`) and the player is genuinely supported there
//! (`landing::supported_below`, the same weight-line test the landing policy trusts). A ground
//! resolve projects in two stages:
//!
//! - flat (+Y) first: the vertical part of the leftover motion and velocity dies in the contact
//!   exactly as it would on flat ground - this is what removes the drift, because gravity's
//!   downward step no longer rotates into a sideways one;
//! - then the true plane: whatever horizontal motion survives is projected onto the actual
//!   surface, so walking on a supported flank or tilted face still follows the surface instead
//!   of re-colliding with it every iteration and stalling at the cap. On an exactly flat top the
//!   second projection is a bitwise no-op.
//!
//! Unsupported resolutions with a ground-grade normal - airborne motion, corner grazes, the body
//! past an edge - keep true normals, so deflections around round shapes and genuine edge
//! departures are untouched. Terrain is not in scope at all: it grounds through its own rest path
//! in `sim::step`.
//!
//! Wall-grade contacts (normal below the ground threshold) carry the second policy, the incidence
//! stop: when the horizontal motion points within `WALL_STOP_DEG` of straight into the wall, the
//! tangential redirect is killed - the horizontal motion and velocity die in the contact, and the
//! player stops at the wall instead of skating along it (the play verdict: running at a wall
//! should read as running INTO it). Vertical motion is exempt - it is projected onto the true
//! plane as always, so gravity still slides a body down a wall - and contacts outside the window
//! (glancing approaches) slide exactly as the engine resolves them, bit for bit. A flat ceiling
//! has no horizontal normal to measure incidence against, so it always takes the engine path.
//!
//! Deterministic: a fixed iteration cap, the engine sweep's slice-order contract, the support
//! probe's fixed arithmetic; no RNG, no state.

use glam::Vec3;
use wok_physics::{Capsule, Collider, SlideResult, sweep_capsule_colliders};

use crate::constants::{WALKABLE_COS, WALKABLE_NORMAL_Y, WALL_STOP_DEG};
use crate::landing::supported_below;

/// Small separation kept between the capsule and surfaces while sliding: the engine slide's value,
/// kept identical so the wall paths reproduce its results bitwise.
const SKIN: f32 = 1e-3;

/// Below this squared length the leftover motion is negligible and the slide stops (engine value).
const MIN_MOVE_SQ: f32 = 1e-10;

/// Cap on slide iterations (engine value): floor plus two walls fits, leftover past it is dropped.
const MAX_ITERS: usize = 4;

/// Move the player capsule by `displacement` through the static `colliders`, sliding along
/// contacts, with the supported-ground flat resolve described in the module docs. Drop-in for the
/// engine's `collide_and_slide` in the player step; `up` is fixed to +Y and the walkable limit to
/// `WALKABLE_COS` because both are this game's policy, not parameters.
pub fn slide_player(capsule: Capsule, displacement: Vec3, velocity: Vec3, colliders: &[Collider]) -> SlideResult {
    let mut cap = capsule;
    let mut remaining = displacement;
    let mut velocity = velocity;
    let mut grounded = false;

    for _ in 0..MAX_ITERS {
        if remaining.length_squared() <= MIN_MOVE_SQ {
            break;
        }
        // The engine slide sweeps with its radius inflated by the skin; the public sweep takes no
        // skin parameter, so the same inflation rides on a widened capsule. Every shape path
        // reduces to "capsule radius plus skin", so the contact is the engine's.
        let inflated = Capsule::new(cap.a, cap.b, cap.radius + SKIN);
        match sweep_capsule_colliders(&inflated, remaining, colliders) {
            None => {
                cap = cap.translated(remaining);
                break;
            }
            Some(hit) => {
                let advance = remaining * hit.toi;
                cap = cap.translated(advance);
                if hit.normal.y >= WALKABLE_COS {
                    grounded = true;
                }
                let leftover = remaining - advance;
                if hit.normal.y >= WALKABLE_NORMAL_Y && supported_below(cap.center(), colliders) {
                    // Supported ground: flat first (the vertical part dies, as on flat ground),
                    // then the true plane so surviving horizontal motion follows the surface.
                    remaining = project_on_plane(project_on_plane(leftover, Vec3::Y), hit.normal);
                    velocity = project_on_plane(project_on_plane(velocity, Vec3::Y), hit.normal);
                } else if hit.normal.y < WALKABLE_NORMAL_Y && head_on(leftover, hit.normal) {
                    // The wall stop: inside the incidence window the tangential redirect dies -
                    // only the vertical part of the motion and velocity survives, projected onto
                    // the true plane so a tilted wall is still never pushed into. On an exactly
                    // vertical wall that projection is a no-op and the body simply stops.
                    remaining = project_on_plane(Vec3::new(0.0, leftover.y, 0.0), hit.normal);
                    velocity = project_on_plane(Vec3::new(0.0, velocity.y, 0.0), hit.normal);
                } else {
                    // True-normal resolution: the engine's own projection, bit for bit.
                    remaining = project_on_plane(leftover, hit.normal);
                    velocity = project_on_plane(velocity, hit.normal);
                }
            }
        }
    }

    SlideResult { position: cap.center(), velocity, grounded }
}

/// Remove the component of `v` along `normal` (the engine slide's projection). `normal` is unit
/// length (the sweep's normals are, and so is +Y).
fn project_on_plane(v: Vec3, normal: Vec3) -> Vec3 {
    v - normal * v.dot(normal)
}

/// Is this wall contact head-on: does the horizontal part of `motion` point within `WALL_STOP_DEG`
/// of straight into the wall (the wall's inward horizontal direction, `-normal` flattened)? The
/// comparison `-h.dot(w) >= |h| * |w| * cos(window)` is the angle test without normalizing; the
/// strict `> 0.0` guard makes the degenerate cases false: no horizontal motion means no incidence
/// to measure (a pure vertical graze keeps the engine resolve), and no horizontal normal means a
/// flat ceiling or floor, which this policy never touches. Inclusive at the window's edge,
/// matching "within".
fn head_on(motion: Vec3, normal: Vec3) -> bool {
    let h = Vec3::new(motion.x, 0.0, motion.z);
    let w = Vec3::new(normal.x, 0.0, normal.z);
    let into_wall = -h.dot(w);
    into_wall > 0.0 && into_wall >= h.length() * w.length() * WALL_STOP_DEG.to_radians().cos()
}

#[cfg(test)]
// Exact float comparison is intended where it appears: the flat resolve's claim is that gravity
// introduces exactly zero sideways motion (the projection of a vertical vector onto the +Y plane
// is the zero vector, bit for bit), not that it stays under a tolerance.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::constants::{AIR_JUMPS, PLAYER_HEIGHT, PLAYER_RADIUS, SIM_DT};
    use crate::sim::{self, Player, StepInput};
    use crate::world::{ChunkTerrain, World};
    use wok_physics::{Motion, collide_and_slide};
    use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    // ---- the policy at the slide level ----

    fn upright(center: Vec3) -> Capsule {
        Capsule::upright(center, PLAYER_HEIGHT, PLAYER_RADIUS)
    }

    /// One step's gravity-only probe: the displacement and velocity a standing body brings to the
    /// slide (fall gravity; the magnitude only has to be a realistic resting probe).
    fn gravity_probe() -> (Vec3, Vec3) {
        let vy = -crate::constants::FALL_GRAVITY * SIM_DT;
        (Vec3::new(0.0, vy * SIM_DT, 0.0), Vec3::new(0.0, vy, 0.0))
    }

    /// A capsule centre resting tangentially on the boulder at horizontal offset `d` from the
    /// apex (the landing module's geometry), a hair above contact.
    fn rested_on_boulder(center: Vec3, sphere_radius: f32, d: f32) -> Vec3 {
        let combined = PLAYER_RADIUS + sphere_radius;
        let bottom_sphere_y = center.y + (combined * combined - d * d).sqrt();
        Vec3::new(center.x + d, bottom_sphere_y - PLAYER_RADIUS + PLAYER_HEIGHT * 0.5 + 0.002, center.z)
    }

    // ---- the wall stop's incidence window ----
    //
    // One wall along z, near face at x = 65 (inward normal -x), and a capsule a nudge from it so
    // every approach below genuinely contacts: the old bitwise wall test started 0.55m out with a
    // 0.25m motion and never touched the wall at all, so it pinned nothing.

    fn wall() -> [Collider; 1] {
        [Collider::from(Aabb::new(Vec3::new(65.0, 0.0, 0.0), Vec3::new(66.0, 6.0, 128.0)))]
    }

    fn at_the_wall() -> Capsule {
        upright(Vec3::new(64.5, 2.75, 64.0))
    }

    /// One walk-speed step's motion and velocity at `degrees` off head-on (+x is straight into the
    /// wall, the tangent runs +z).
    fn approach(degrees: f32) -> (Vec3, Vec3) {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let dir = Vec3::new(cos, 0.0, sin);
        (dir * (crate::constants::MOVE_SPEED * SIM_DT), dir * crate::constants::MOVE_SPEED)
    }

    #[test]
    fn a_head_on_wall_contact_stops_dead() {
        // The verdict's centre: running straight at a wall ends the run. The body pins at the
        // wall's face and the whole velocity dies - nothing redirects, nothing skates.
        let statics = wall();
        let cap = at_the_wall();
        let (d, v) = approach(0.0);
        let r = slide_player(cap, d, v, &statics);
        assert_eq!(r.velocity, Vec3::ZERO, "a head-on hit must kill the velocity outright");
        assert_eq!(r.position.z, cap.center().z, "no tangential drift out of a head-on hit");
        assert!(r.position.x <= 65.0 - PLAYER_RADIUS, "the body stays outside the wall");
        assert!(r.position.x > cap.center().x, "the pre-contact advance still happens");
    }

    #[test]
    fn a_30_degree_approach_stops_where_the_engine_would_skate() {
        // Inside the window but off-axis: the engine's projection would keep most of the
        // tangential motion (the skate the verdict rejected); the policy kills it. The engine
        // fixture pin alongside proves the test sits where the two genuinely diverge.
        let statics = wall();
        let cap = at_the_wall();
        let (d, v) = approach(30.0);

        let engines = collide_and_slide(cap, d, v, &statics, Vec3::Y, WALKABLE_COS);
        assert!(engines.velocity.z > 1.0, "fixture: the engine must show the skate this stops");

        let ours = slide_player(cap, d, v, &statics);
        assert_eq!(ours.velocity, Vec3::ZERO, "inside the window the redirect dies");
        assert!(
            ours.position.z < engines.position.z,
            "the stop must not travel the engine's tangential distance"
        );
    }

    #[test]
    fn a_60_degree_approach_slides_bitwise_as_the_engine() {
        // Outside the window, glancing contacts are not the policy's business: position, velocity,
        // and grounded flag must be the engine's answer bit for bit.
        let statics = wall();
        let cap = at_the_wall();
        let (d, v) = approach(60.0);
        let ours = slide_player(cap, d, v, &statics);
        let engines = collide_and_slide(cap, d, v, &statics, Vec3::Y, WALKABLE_COS);
        assert_eq!(ours, engines, "a glancing wall contact must be the engine's, unchanged");
        assert!(ours.velocity.z > 1.0, "fixture: the glancing approach really does keep sliding");
    }

    #[test]
    fn gravity_still_falls_through_a_head_on_stop() {
        // The vertical exemption: a falling body that hits the wall head-on loses its run, not
        // its fall - the wall is vertical, so the downward velocity survives untouched (bitwise:
        // projecting a vertical vector onto a vertical wall's plane is a no-op).
        let statics = wall();
        let cap = at_the_wall();
        let (mut d, mut v) = approach(0.0);
        d.y = -0.02;
        v.y = -3.0;
        let r = slide_player(cap, d, v, &statics);
        assert_eq!(r.velocity, Vec3::new(0.0, -3.0, 0.0), "the fall must survive the wall stop");
        assert!(r.position.y < cap.center().y, "the body keeps descending along the wall");
    }

    #[test]
    fn an_unsupported_flank_graze_is_bitwise_the_engines() {
        // Well down a boulder's flank the support probe says no (the weight line misses the
        // bearing surface), so the policy keeps true normals and the body sheds exactly as the
        // engine resolves it - the airborne/corner-graze behavior, untouched.
        let boulder = [Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 }];
        let cap = upright(rested_on_boulder(Vec3::new(64.0, 2.0, 64.0), 1.1, 0.8));
        let (d, v) = gravity_probe();
        let ours = slide_player(cap, d, v, &boulder);
        let engines = collide_and_slide(cap, d, v, &boulder, Vec3::Y, WALKABLE_COS);
        assert_eq!(ours, engines, "unsupported resolution must be the engine's, unchanged");
    }

    #[test]
    fn a_supported_flank_contact_resolves_flat_where_the_engine_bleeds_sideways() {
        // The fix at its seam, with the broken behavior pinned alongside: resting supported on
        // the flank 0.2 off the apex, one gravity probe through the engine slide comes out with a
        // real horizontal velocity (the drift source); through the policy slide the contact
        // resolves flat and the gravity dies with zero horizontal bleed.
        let boulder = [Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 }];
        let cap = upright(rested_on_boulder(Vec3::new(64.0, 2.0, 64.0), 1.1, 0.2));
        let (d, v) = gravity_probe();

        let engines = collide_and_slide(cap, d, v, &boulder, Vec3::Y, WALKABLE_COS);
        let engine_bleed = Vec3::new(engines.velocity.x, 0.0, engines.velocity.z).length();
        assert!(engine_bleed > 1e-3, "fixture: the engine's true-normal resolve must show the drift source");

        let ours = slide_player(cap, d, v, &boulder);
        let our_bleed = Vec3::new(ours.velocity.x, 0.0, ours.velocity.z).length();
        assert!(our_bleed == 0.0, "a supported ground contact must bleed no horizontal velocity: {our_bleed}");
        assert!(ours.grounded, "the flank contact still grades as ground");
        assert_eq!(ours.position.x, cap.center().x, "gravity must not walk the body sideways");
        assert_eq!(ours.position.z, cap.center().z);
    }

    #[test]
    fn walking_on_a_supported_flank_still_makes_progress() {
        // The stall guard: the flat resolve keeps the horizontal intent, and the second (true
        // plane) projection lets it run along the surface instead of re-colliding every iteration
        // and being dropped at the cap. A walk-sized horizontal move near the apex must come out
        // mostly applied, not eaten.
        let boulder = [Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 }];
        let cap = upright(rested_on_boulder(Vec3::new(64.0, 2.0, 64.0), 1.1, 0.1));
        let step = crate::constants::MOVE_SPEED * SIM_DT;
        let d = Vec3::new(step, -0.005, 0.0);
        let v = Vec3::new(crate::constants::MOVE_SPEED, -0.3, 0.0);
        let r = slide_player(cap, d, v, &boulder);
        let moved = r.position.x - cap.center().x;
        assert!(moved > step * 0.5, "a supported walk must keep moving along the surface: moved {moved} of {step}");
    }

    // ---- the policy through the real step ----

    fn flat_world(height_m: f32) -> World {
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }], ..World::default() }
    }

    fn standing(x: f32, z: f32, base_y: f32) -> Player {
        Player {
            motion: Motion { position: Vec3::new(x, base_y + PLAYER_HEIGHT * 0.5 + 0.02, z), velocity: Vec3::ZERO },
            grounded: false,
            air_jumps: AIR_JUMPS,
            coyote: 0.0,
        }
    }

    /// Settle, then run `steps` idle steps asserting grounded throughout; returns the horizontal
    /// drift from the settled position.
    fn idle_drift(world: &World, start: Player, steps: usize) -> Vec3 {
        let mut p = start;
        for _ in 0..30 {
            p = sim::step(p, StepInput::default(), world);
        }
        assert!(p.grounded, "fixture: must settle grounded before measuring drift");
        let settled = p.motion.position;
        for i in 0..steps {
            p = sim::step(p, StepInput::default(), world);
            assert!(p.grounded, "step {i}: lost the ground while standing still");
        }
        let d = p.motion.position - settled;
        Vec3::new(d.x, 0.0, d.z)
    }

    #[test]
    fn standing_supported_near_a_crate_edge_does_not_drift() {
        // The brief's crate-edge case. With the axis inside the footprint the box contact normal
        // is already flat, so this held BEFORE the fix too (measured zero drift): it stands as
        // the regression pin that the policy does not disturb the case that already worked.
        let mut world = flat_world(2.0);
        world.statics.push(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into());
        for x in [64.9_f32, 64.99, 65.0] {
            let drift = idle_drift(&world, standing(x, 64.0, 4.0), 120);
            assert!(drift.length() < 1e-5, "x={x}: drifted {drift:?} standing near the crate edge");
        }
    }

    #[test]
    fn standing_supported_on_a_boulder_flank_does_not_drift() {
        // The demonstrator: before the fix this drifted +0.136m over these 120 steps (grounded
        // the whole way) - the felt slipperiness. Supported flank contacts now resolve flat, so
        // the player stands exactly still inside the bearing window.
        let mut world = flat_world(2.0);
        world.statics.push(Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 });
        let drift = idle_drift(&world, standing(64.2, 64.0, 3.05), 120);
        assert!(drift.length() < 1e-4, "drifted {drift:?} standing supported on the flank");
    }

    #[test]
    fn standing_supported_on_a_gently_tilted_platform_does_not_drift() {
        // The other live case: a cube pitched 10 degrees (face normal.y ~ 0.985, well inside
        // walkable). Before the fix it drifted +0.127m over these 120 steps. The support probe
        // covers tilts up to where the bottom sphere's curve outruns its tolerance (~15 degrees
        // at this radius); steeper faces read unsupported and shed as before, by design.
        let mut world = flat_world(2.0);
        world.statics.push(Collider::Obb {
            center: Vec3::new(64.0, 3.0, 64.0),
            half_extents: Vec3::ONE,
            rotation: glam::Quat::from_rotation_x(10.0_f32.to_radians()),
        });
        let drift = idle_drift(&world, standing(64.0, 64.0, 4.1), 120);
        assert!(drift.length() < 1e-4, "drifted {drift:?} standing supported on the tilted face");
    }

    #[test]
    fn walking_deliberately_off_the_edge_still_departs() {
        // The flat resolve must never glue the player to an edge: holding input over it, the
        // axis leaves the footprint, support ends, and the body goes airborne and lands on the
        // terrain below - the genuine departure, unchanged.
        let mut world = flat_world(2.0);
        world.statics.push(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into());
        let mut p = standing(64.5, 64.0, 4.0);
        for _ in 0..30 {
            p = sim::step(p, StepInput::default(), &world);
        }
        assert!(p.grounded, "fixture: should stand on the crate top");

        let off = StepInput { move_dir: Vec3::X, jump: false };
        let mut went_airborne = false;
        for _ in 0..180 {
            p = sim::step(p, off, &world);
            went_airborne |= !p.grounded;
        }
        assert!(went_airborne, "walking off the edge must still depart");
        assert!(p.grounded, "and land on the terrain below");
        let base = p.motion.position.y - PLAYER_HEIGHT * 0.5;
        assert!((base - 2.0).abs() < 1e-2, "should end on the terrain at 2m, base = {base}");
    }
}
