//! The fixed-step player simulation: state, the per-step composition, and spawn.
//!
//! This is the loop body the engine deliberately does not own (HLD principle 5): taste holds the
//! player's [`Motion`] and the jump state, and each fixed step sequences wok-physics's pure pieces.
//! The movement model is deliberately small - a run speed, one gravity, and a jump counter:
//!
//!     set the horizontal velocity to the move intent * move_speed (instant, ground and air alike)
//!     -> a jump press, if the counter has one, sets the vertical velocity to the launch speed
//!     -> integrate one fixed step under a single gravity   (wok-physics: integrate)
//!     -> collide-and-slide against the statics              (crate::slide: prefab collision)
//!     -> rest the slid body on the terrain beneath          (wok-physics: rest_cylinder_on_heightmap)
//!     -> refill the jump counter once the body has been vertically still for the reset dwell
//!
//! The player's collider is the flat-bottomed vertical cylinder; the drawn bean stays a capsule,
//! documented at the draw site (`crate::app`). Horizontal control is instant and identical on the
//! ground and in the air: holding a direction moves at `move_speed`, releasing stops, and the same
//! is true mid-jump (air control). Every jump - the first off the ground and the double jump alike -
//! sets the vertical velocity to the same launch speed, so a second jump acts like the first.
//!
//! The jump counter refills by vertical STILLNESS, not by ground detection: resting on the ground
//! or on a surface pins the vertical velocity to zero, so the still timer fills and, once it passes
//! a brief dwell (`JUMP_RESET_DWELL`, a step or two), the counter is restored - a landing hands the
//! jumps back effectively at once. The dwell exists only to reject a jump's apex, which grazes zero
//! for a single step; it is not a felt cooldown. Reading stillness instead of a grounded flag is
//! what keeps complex terrain - slopes, ledges, edges - from ever leaving the player unable to jump.
//!
//! Two world-space wrinkles the engine leaves to the caller: statics were lifted to world space when
//! the [`World`] was built, and the terrain rest maps the body into the local frame of the chunk it
//! is over and back (a pure vertical translation each way).
//!
//! Everything here is deterministic: pure functions of the inputs and `SIM_DT`, no clocks, no RNG,
//! which is what the replay test (`crate::replay`) pins bitwise.

use glam::Vec3;
use wok_physics::{Cylinder, Motion, boom_direction, integrate, rest_cylinder_on_heightmap};

use crate::constants::{JUMP_RESET_DWELL, PLAYER_HEIGHT, PLAYER_RADIUS, SIM_DT, SPAWN_HEIGHT, STILL_VY};
use crate::slide::slide_player;
use crate::tuning::Tuning;
use crate::world::{CHUNK_SIZE_M, World};

/// The terrain rest reports a `grounded` flag gated by a walkable-slope limit; the simple model
/// does not read it (the jump counter refills by vertical stillness, not by grounding), and the
/// lift onto the surface happens regardless of this value. Pass a permissive limit so nothing is
/// ever gated out of the lift's report.
const TERRAIN_REST_WALKABLE_COS: f32 = 0.0;

/// The player: a cylinder-bodied actor the game owns between steps. `motion.position` is the
/// cylinder centre (wok-physics's reference point for every resolve).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub motion: Motion,
    /// Jumps still available before the counter must refill. Spent by jumping, restored to
    /// `max_jumps` once `still_time` passes the reset dwell. Part of the stepped state, so replay
    /// covers it like position and velocity.
    pub jumps_remaining: u32,
    /// How long the body's vertical velocity has been still, in seconds of simulation time: grows
    /// while resting (vertical velocity at zero), zeroes the moment the body moves vertically. When
    /// it passes the tuning's `jump_reset_time` the jump counter refills. Stepped state, so replay
    /// covers it.
    pub still_time: f32,
}

/// One fixed step's input, already resolved against the camera: a world-space horizontal move
/// direction of length at most one, and whether a jump was asked for. The press is the whole
/// signal: every jump flies the full arc (the play verdict against variable height), so the
/// simulation never reads how long the control stays down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepInput {
    pub move_dir: Vec3,
    pub jump: bool,
}

/// Resolve the intent's movement axes against the camera yaw into a world-space horizontal move
/// direction, clamped to unit length so diagonals are no faster than a single axis.
///
/// "Forward" is where the camera looks: the boom points from the target out to the camera
/// (wok-physics orbit convention), so the camera's horizontal forward is the boom direction at zero
/// pitch, negated. Right is that forward crossed with up.
pub fn move_direction(camera_yaw: f32, forward: f32, right: f32) -> Vec3 {
    let f = -boom_direction(camera_yaw, 0.0);
    let r = Vec3::new(-f.z, 0.0, f.x);
    (f * forward + r * right).clamp_length_max(1.0)
}

/// Where to draw the player this frame: the position eased between the previous and current fixed
/// steps by the accumulator's progress through the next step (`FixedClock::alpha`).
///
/// Rendering at the raw current state samples the simulation at whichever step boundary the frame
/// happened to catch; when the frame rate beats against the step rate that sampling error oscillates
/// and reads as jitter. Interpolating costs one step of display latency (at most 1/60s) and makes
/// the drawn motion continuous. Render-side only: simulation state is never interpolated, so the
/// replay contract is untouched. Alpha is clamped defensively; the clock's contract already keeps it
/// in `0.0..1.0`.
pub fn lerp_position(prev: &Player, curr: &Player, alpha: f32) -> Vec3 {
    prev.motion.position.lerp(curr.motion.position, alpha.clamp(0.0, 1.0))
}

/// The player at rest in the air above the middle of the first terrain chunk (or above the world
/// origin when no chunk has terrain), `SPAWN_HEIGHT` above the surface: the opening fall.
pub fn spawn(world: &World, tuning: &Tuning) -> Player {
    let half = CHUNK_SIZE_M * 0.5;
    let position = world.terrains.first().map_or(Vec3::new(0.0, SPAWN_HEIGHT, 0.0), |t| {
        let ground = t.heightmap.height_at(half, half);
        t.origin + Vec3::new(half, ground + SPAWN_HEIGHT, half)
    });
    Player {
        motion: Motion { position, velocity: Vec3::ZERO },
        jumps_remaining: tuning.max_jumps,
        still_time: 0.0,
    }
}

/// Advance the player by one fixed step. Pure: identical player, input, and world give an identical
/// next player, bit for bit.
pub fn step(player: Player, input: StepInput, world: &World, tuning: &Tuning) -> Player {
    let mut m = player.motion;

    // Horizontal locomotion is the whole of the move model: the intent (length at most one) times
    // the run speed, set instantly, the same on the ground and in the air. No acceleration or
    // friction - releasing the stick stops the body, and holding a direction mid-jump steers it
    // (air control). The vertical velocity rides through untouched here; gravity and the jump own
    // it below.
    m.velocity.x = input.move_dir.x * tuning.move_speed;
    m.velocity.z = input.move_dir.z * tuning.move_speed;

    // The jump: a press with a jump left in the counter sets the vertical velocity to the launch
    // speed and spends one. Every jump is identical - the first off the ground and the double jump
    // launch the same - so a second jump acts like the first. The latch upstream (`crate::jump`)
    // guarantees one press is one jump.
    let mut jumps = player.jumps_remaining;
    if input.jump && jumps > 0 {
        m.velocity.y = tuning.jump_velocity;
        jumps -= 1;
    }

    // One fixed step under a single gravity, then slide the resulting move along any static geometry
    // it meets (`crate::slide`: stop at walls, slide along them, come to rest on surfaces landed on
    // from above).
    let next = integrate(m, Vec3::new(0.0, -tuning.gravity, 0.0), SIM_DT);
    let body = Cylinder::upright(m.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let slid = slide_player(body, next.position - m.position, next.velocity, &world.statics);

    // Rest the slid body on the terrain of the chunk beneath it, in that chunk's local frame
    // (lift-only). Off every chunk there is no ground to rest on. A lift means the body met the
    // surface this step, so its descent stops there.
    let slid_body = Cylinder::upright(slid.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let mut position = slid.position;
    let mut velocity = slid.velocity;
    if let Some(t) = world.terrain_under(slid.position.x, slid.position.z) {
        let rest = rest_cylinder_on_heightmap(slid_body.translated(-t.origin), &t.heightmap, TERRAIN_REST_WALKABLE_COS);
        position = rest.position + t.origin;
        if position.y > slid.position.y {
            velocity.y = 0.0;
        }
    }

    // The jump counter refills by vertical stillness, not ground detection: resting on the ground or
    // a surface pins the vertical velocity to zero (the slide projects out a landing's descent, the
    // terrain rest's lift zeroes it just above), so the still timer fills while at rest and, after a
    // brief dwell - just long enough to reject the apex's single still step - refills the counter, so
    // a landing restores the jumps at once.
    let still_time = if velocity.y.abs() <= STILL_VY { player.still_time + SIM_DT } else { 0.0 };
    let jumps_remaining = if still_time >= JUMP_RESET_DWELL { tuning.max_jumps } else { jumps };

    Player { motion: Motion { position, velocity }, jumps_remaining, still_time }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::world::ChunkTerrain;
    use wok_scene::{CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    const EPS: f32 = 1e-5;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

    // ---- camera-relative movement direction ----

    #[test]
    fn forward_at_zero_yaw_moves_toward_negative_z() {
        // Boom at +Z puts the camera behind a target facing -Z, so forward intent walks -Z.
        assert!(close(move_direction(0.0, 1.0, 0.0), Vec3::NEG_Z));
    }

    #[test]
    fn right_at_zero_yaw_moves_toward_positive_x() {
        assert!(close(move_direction(0.0, 0.0, 1.0), Vec3::X));
    }

    #[test]
    fn movement_follows_the_camera_yaw() {
        // A quarter-turn yaw swings the camera to the target's +X side, looking -X: forward is -X.
        let dir = move_direction(std::f32::consts::FRAC_PI_2, 1.0, 0.0);
        assert!(close(dir, Vec3::NEG_X), "dir = {dir:?}");
    }

    #[test]
    fn diagonals_clamp_to_unit_length() {
        let dir = move_direction(0.7, 1.0, 1.0);
        assert!(dir.length() <= 1.0 + EPS, "length = {}", dir.length());
        // And the clamp preserves direction: still a positive mix of forward and right.
        assert!(dir.length() > 0.9, "a full diagonal should still be (nearly) full speed");
    }

    #[test]
    fn move_direction_is_horizontal() {
        assert_eq!(move_direction(1.3, 1.0, -0.5).y, 0.0);
    }

    // ---- the interpolated draw position ----

    fn player_at(position: Vec3) -> Player {
        Player {
            motion: Motion { position, velocity: Vec3::ZERO },
            jumps_remaining: Tuning::default().max_jumps,
            still_time: 0.0,
        }
    }

    #[test]
    fn lerp_position_spans_the_two_states() {
        let prev = player_at(Vec3::new(0.0, 4.0, -2.0));
        let curr = player_at(Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(lerp_position(&prev, &curr, 0.0), prev.motion.position);
        assert_eq!(lerp_position(&prev, &curr, 1.0), curr.motion.position);
        assert_eq!(lerp_position(&prev, &curr, 0.5), Vec3::new(0.5, 3.0, -1.0));
    }

    #[test]
    fn lerp_position_clamps_out_of_range_alpha() {
        let prev = player_at(Vec3::ZERO);
        let curr = player_at(Vec3::X);
        assert_eq!(lerp_position(&prev, &curr, -1.0), prev.motion.position);
        assert_eq!(lerp_position(&prev, &curr, 2.0), curr.motion.position);
    }

    // ---- the movement model through the real step ----

    fn flat_world(height_m: f32) -> World {
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }], ..World::default() }
    }

    /// A player settled at rest mid-chunk: the cylinder's flat base on the surface, vertically still
    /// long enough that its jumps are full.
    fn at_rest(world: &World) -> Player {
        let ground = world.terrains[0].heightmap.height_at(64.0, 64.0);
        let t = Tuning::default();
        Player {
            motion: Motion { position: Vec3::new(64.0, ground + PLAYER_HEIGHT * 0.5, 64.0), velocity: Vec3::ZERO },
            jumps_remaining: t.max_jumps,
            still_time: JUMP_RESET_DWELL,
        }
    }

    /// Airborne high over the terrain, moving at `velocity`, jumps full and the still timer reset.
    fn airborne_at(position: Vec3, velocity: Vec3) -> Player {
        Player { motion: Motion { position, velocity }, jumps_remaining: Tuning::default().max_jumps, still_time: 0.0 }
    }

    fn horizontal_speed(p: &Player) -> f32 {
        Vec3::new(p.motion.velocity.x, 0.0, p.motion.velocity.z).length()
    }

    #[test]
    fn horizontal_velocity_is_the_run_speed_instantly_grounded_and_airborne() {
        // The whole horizontal model: one step at full stick is already move_speed, with no ramp,
        // and the air behaves identically - holding a direction mid-air moves at the run speed.
        let world = flat_world(2.0);
        let t = Tuning::default();
        let grounded = step(at_rest(&world), StepInput { move_dir: Vec3::X, jump: false }, &world, &t);
        assert!((horizontal_speed(&grounded) - t.move_speed).abs() < EPS, "grounded: {}", horizontal_speed(&grounded));
        let air = step(airborne_at(Vec3::new(64.0, 30.0, 64.0), Vec3::ZERO), StepInput { move_dir: Vec3::Z, jump: false }, &world, &t);
        assert!((horizontal_speed(&air) - t.move_speed).abs() < EPS, "airborne: {}", horizontal_speed(&air));
    }

    #[test]
    fn releasing_the_stick_stops_the_horizontal_at_once() {
        let world = flat_world(2.0);
        let t = Tuning::default();
        let mut running = at_rest(&world);
        running.motion.velocity = Vec3::new(t.move_speed, 0.0, 0.0);
        let stopped = step(running, StepInput::default(), &world, &t);
        assert_eq!(horizontal_speed(&stopped), 0.0, "no input stops the horizontal at once");
    }

    #[test]
    fn a_jump_sets_the_launch_velocity_and_spends_one() {
        let world = flat_world(2.0);
        let t = Tuning::default();
        let start = at_rest(&world);
        let jumped = step(start, StepInput { move_dir: Vec3::ZERO, jump: true }, &world, &t);
        // The impulse sets the launch speed; one step of gravity follows in the same step.
        assert!(
            (jumped.motion.velocity.y - (t.jump_velocity - t.gravity * SIM_DT)).abs() < EPS,
            "vy = {}",
            jumped.motion.velocity.y
        );
        assert_eq!(jumped.jumps_remaining, start.jumps_remaining - 1, "the jump spends one");
    }

    #[test]
    fn the_double_jump_launches_exactly_like_the_first() {
        let world = flat_world(2.0);
        let t = Tuning::default();
        let jump = StepInput { move_dir: Vec3::ZERO, jump: true };
        let first = step(at_rest(&world), jump, &world, &t);
        let second = step(first, jump, &world, &t);
        assert!(
            (second.motion.velocity.y - first.motion.velocity.y).abs() < EPS,
            "the double jump must match the first: {} vs {}",
            second.motion.velocity.y,
            first.motion.velocity.y
        );
        assert_eq!(first.jumps_remaining, t.max_jumps - 1);
        assert_eq!(second.jumps_remaining, t.max_jumps - 2);
    }

    #[test]
    fn a_jump_with_an_empty_counter_does_nothing() {
        let world = flat_world(2.0);
        let t = Tuning::default();
        let jump = StepInput { move_dir: Vec3::ZERO, jump: true };
        let mut p = step(at_rest(&world), jump, &world, &t);
        for _ in 1..t.max_jumps {
            p = step(p, jump, &world, &t);
        }
        assert_eq!(p.jumps_remaining, 0, "every jump spent");
        let vy_before = p.motion.velocity.y;
        let pressed = step(p, jump, &world, &t);
        assert!(
            (pressed.motion.velocity.y - (vy_before - t.gravity * SIM_DT)).abs() < EPS,
            "a spent counter must not relaunch: {}",
            pressed.motion.velocity.y
        );
    }

    #[test]
    fn the_jumps_do_not_refill_in_the_air() {
        // Through the rise and the apex, the counter must never climb back: the apex is still for
        // only an instant, far short of the reset dwell.
        let world = flat_world(2.0);
        let t = Tuning::default();
        let mut p = step(at_rest(&world), StepInput { move_dir: Vec3::ZERO, jump: true }, &world, &t);
        let spent = p.jumps_remaining;
        for i in 0..30 {
            p = step(p, StepInput::default(), &world, &t);
            assert!(p.jumps_remaining <= spent, "step {i}: the counter refilled in the air");
        }
    }

    #[test]
    fn the_jumps_refill_shortly_after_landing() {
        let world = flat_world(2.0);
        let t = Tuning::default();
        let jumped = step(at_rest(&world), StepInput { move_dir: Vec3::ZERO, jump: true }, &world, &t);
        assert!(jumped.jumps_remaining < t.max_jumps, "a jump was spent");
        let mut p = jumped;
        let mut refilled = false;
        for _ in 0..600 {
            p = step(p, StepInput::default(), &world, &t);
            if p.jumps_remaining == t.max_jumps {
                refilled = true;
                break;
            }
        }
        assert!(refilled, "landing and coming to rest must refill the jumps");
    }

    #[test]
    fn the_jumps_reset_while_resting_on_a_standable_prefab_slope() {
        // The user's case: spend the jumps, land on a tilted prefab face, and the counter must
        // refill - the floor-ish resolve rests the body on the slope so its vertical velocity
        // settles, the same as on flat ground.
        use glam::Quat;
        use wok_physics::Collider;
        let ramp = Collider::Obb {
            center: Vec3::new(0.0, 0.0, 0.0),
            half_extents: Vec3::splat(2.0),
            rotation: Quat::from_rotation_x(20.0_f32.to_radians()),
        };
        let world = World { statics: vec![ramp], ..World::default() };
        let t = Tuning::default();
        let mut p = airborne_at(Vec3::new(0.0, 4.0, 0.0), Vec3::ZERO);
        p.jumps_remaining = 0;
        for _ in 0..240 {
            p = step(p, StepInput::default(), &world, &t);
        }
        assert!(
            p.motion.velocity.y.abs() <= STILL_VY,
            "the body should rest on the slope, not slide: vy = {}",
            p.motion.velocity.y
        );
        assert_eq!(p.jumps_remaining, t.max_jumps, "resting on a standable prefab slope must refill the jumps");
    }

    #[test]
    fn a_falling_body_lands_and_comes_to_rest_on_the_terrain() {
        // Ground collision: the body falls, the terrain rest catches it on the surface, and the
        // vertical velocity comes to rest (which is what feeds the jump-reset stillness timer).
        let world = flat_world(2.0);
        let t = Tuning::default();
        let ground = world.terrains[0].heightmap.height_at(64.0, 64.0);
        let mut p = airborne_at(Vec3::new(64.0, ground + 5.0, 64.0), Vec3::ZERO);
        for _ in 0..600 {
            p = step(p, StepInput::default(), &world, &t);
            if p.motion.velocity.y.abs() <= STILL_VY && p.motion.position.y < ground + 5.0 {
                break;
            }
        }
        let base = p.motion.position.y - PLAYER_HEIGHT * 0.5;
        assert!((base - ground).abs() < 1e-2, "the base should rest on the surface: base {base}, ground {ground}");
        assert!(p.motion.velocity.y.abs() <= STILL_VY, "a rested body is vertically still: {}", p.motion.velocity.y);
    }
}
