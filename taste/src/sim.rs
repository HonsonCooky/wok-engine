//! The fixed-step player simulation: state, the per-step composition, and spawn.
//!
//! This is the loop body the engine deliberately does not own (HLD principle 5): taste holds the
//! player's [`Motion`] and grounded flag, and each fixed step sequences wok-physics's pure pieces in
//! the composition the Level 2 locomotion harness proved out (wok-physics/tests/locomotion_replay):
//!
//!     accelerate the horizontal velocity toward intent  (and the jump impulse, the game's policy)
//!     -> integrate one fixed step under gravity   (wok-physics: integrate)
//!     -> collide-and-slide against static AABBs   (wok-physics: collide_and_slide)
//!     -> rest the slid capsule on the terrain     (wok-physics: rest_on_heightmap, chunk-local)
//!
//! Two extensions over the harness. World space: statics were lifted to world space when the
//! [`World`] was built, and the terrain rest maps the capsule into the local frame of the chunk it
//! is over and back (a pure translation each way). Downhill snap-down: a grounded, non-jumping
//! step whose support fell away by at most `SNAP_DOWN_DISTANCE` is glued back to the surface
//! instead of flickering airborne (game policy over the engine's lift-only rest). Landing policy
//! is also the harness's: when the slide grounded the body on box geometry, or the terrain lifted
//! it back onto the surface, the downward fall is spent.
//!
//! Everything here is deterministic: pure functions of the inputs and `SIM_DT`, no clocks, no RNG,
//! which is what the replay test (`crate::replay`) pins bitwise.

use glam::Vec3;
use wok_physics::{Capsule, Motion, boom_direction, collide_and_slide, integrate, rest_on_heightmap};

use crate::constants::{
    AIR_CONTROL, GRAVITY, GROUND_ACCEL, GROUND_FRICTION, JUMP_VELOCITY, MOVE_SPEED, PLAYER_HEIGHT, PLAYER_RADIUS,
    SIM_DT, SNAP_DOWN_DISTANCE, SPAWN_HEIGHT, WALKABLE_COS,
};
use crate::landing::supported_below;
use crate::world::{CHUNK_SIZE_M, World};

/// The player: a capsule-shaped body the game owns between steps. `motion.position` is the capsule
/// centre (wok-physics's reference point for every resolve).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub motion: Motion,
    pub grounded: bool,
}

/// One fixed step's input, already resolved against the camera: a world-space horizontal move
/// direction of length at most one, and whether a jump was asked for.
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
pub fn spawn(world: &World) -> Player {
    let half = CHUNK_SIZE_M * 0.5;
    let position = world.terrains.first().map_or(Vec3::new(0.0, SPAWN_HEIGHT, 0.0), |t| {
        let ground = t.heightmap.height_at(half, half);
        t.origin + Vec3::new(half, ground + SPAWN_HEIGHT, half)
    });
    Player { motion: Motion { position, velocity: Vec3::ZERO }, grounded: false }
}

/// Move `current` toward `target` by at most `max_delta`, arriving exactly: the constant-rate
/// approach locomotion is built on. Unlike an exponential ease it has no asymptote, so a decaying
/// velocity reaches a true zero (and a run reaches exactly top speed) in finite steps - which is
/// also what makes the analytic time-to-speed in the tests exact rather than approximate.
fn approach(current: Vec3, target: Vec3, max_delta: f32) -> Vec3 {
    let gap = target - current;
    let len = gap.length();
    if len <= max_delta { target } else { current + gap * (max_delta / len) }
}

/// Advance the player by one fixed step. Pure: identical player, input, and world give an identical
/// next player, bit for bit.
pub fn step(player: Player, input: StepInput, world: &World) -> Player {
    // Accelerated locomotion: the horizontal velocity approaches intent * MOVE_SPEED at a constant
    // rate instead of being set outright, so starts and stops take a beat and read as weight. With
    // input the rate is GROUND_ACCEL toward the intended velocity, scaled by AIR_CONTROL when
    // airborne: steering, weaker in the air. Without input, GROUND_FRICTION toward rest - on the
    // ground only. Friction is a grounded idea: an airborne body with no input keeps its velocity
    // ballistically, which is both the momentum AIR_CONTROL promises a jump and what lets a body
    // caught on a crate's corner accumulate slide-off speed and roll free (air friction re-zeroed
    // that escape velocity every step and pinned the body hovering on the corner point - the last
    // of the mid-air halts). Top speed is unchanged - the target is the same intent * MOVE_SPEED
    // the direct set used. Vertical velocity carries the gravity the body has accumulated, plus
    // the jump impulse when one was asked for and the body has ground to push off.
    let mut m = player.motion;
    let target = Vec3::new(input.move_dir.x, 0.0, input.move_dir.z) * MOVE_SPEED;
    let rate = match (input.move_dir == Vec3::ZERO, player.grounded) {
        (false, true) => GROUND_ACCEL,
        (false, false) => GROUND_ACCEL * AIR_CONTROL,
        (true, true) => GROUND_FRICTION,
        (true, false) => 0.0,
    };
    let horizontal = approach(Vec3::new(m.velocity.x, 0.0, m.velocity.z), target, rate * SIM_DT);
    m.velocity.x = horizontal.x;
    m.velocity.z = horizontal.z;
    if input.jump && player.grounded {
        m.velocity.y = JUMP_VELOCITY;
    }

    // One fixed step under gravity, then slide the resulting move along any static geometry it meets.
    let next = integrate(m, GRAVITY, SIM_DT);
    let capsule = Capsule::upright(m.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let slid =
        collide_and_slide(capsule, next.position - m.position, next.velocity, &world.statics, Vec3::Y, WALKABLE_COS);

    // Rest the slid capsule on the terrain of the chunk beneath it, in that chunk's local frame
    // (lift-only; the slide handled walls). Off every chunk there is no ground to rest on.
    let slid_capsule = Capsule::upright(slid.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let (rested_position, rested_grounded) = match world.terrain_under(slid.position.x, slid.position.z) {
        Some(t) => {
            let rest = rest_on_heightmap(slid_capsule.translated(-t.origin), &t.heightmap, WALKABLE_COS);
            (rest.position + t.origin, rest.grounded)
        }
        None => (slid.position, false),
    };

    // Landing policy: a static-geometry landing must be genuine support, not a corner graze. The
    // slide grounds on any walkable-normal contact, and a capsule's rounded bottom grazing a
    // crate's top corner from beside the crate produces a near-vertical normal; reading that as
    // landed zeroed the fall each step and held the player hovering at box-top height for seconds
    // (the walk-off halt). Landing therefore requires all three: a walkable contact this step
    // (`slid.grounded`), the body moving downward into it (a rising jump is never landing), and a
    // bearing surface actually under the capsule's axis (`crate::landing::supported_below`).
    // Terrain landings are the rest's own signal, unchanged.
    let supported =
        slid.grounded && next.velocity.y <= 0.0 && supported_below(slid.position, &world.statics);
    let mut grounded = supported || rested_grounded;
    let mut velocity = slid.velocity;
    let mut position = rested_position;
    if supported || rested_position.y > slid.position.y {
        velocity.y = 0.0;
    }

    // Downhill snap-down, the game's ground glue: walking downhill the surface falls away faster
    // than one step of gravity follows, so a grounded walk would flicker airborne every step. If
    // the player was grounded, did not jump, and ended this step just above the terrain, probe
    // SNAP_DOWN_DISTANCE below: when the lift-only rest brings that probe back up onto walkable
    // ground (the support is within the glue distance), take its position and remain grounded.
    // A drop taller than the glue leaves the probe unlifted and ungrounded, so real ledges and
    // jumps still go airborne.
    let jumped = input.jump && player.grounded;
    if player.grounded && !jumped && !grounded
        && let Some(t) = world.terrain_under(position.x, position.z)
    {
        let probe = Capsule::upright(position - t.origin, PLAYER_HEIGHT, PLAYER_RADIUS)
            .translated(Vec3::new(0.0, -SNAP_DOWN_DISTANCE, 0.0));
        let snap = rest_on_heightmap(probe, &t.heightmap, WALKABLE_COS);
        if snap.grounded {
            position = snap.position + t.origin;
            velocity.y = 0.0;
            grounded = true;
        }
    }

    Player { motion: Motion { position, velocity }, grounded }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

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

    fn player_at(position: Vec3) -> Player {
        Player { motion: Motion { position, velocity: Vec3::ZERO }, grounded: false }
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

    // ---- acceleration ----

    use crate::world::ChunkTerrain;
    use wok_scene::{CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    fn flat_world(height_m: f32) -> World {
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }] }
    }

    /// A player standing at rest mid-chunk: capsule base exactly on the surface, grounded.
    fn at_rest(world: &World) -> Player {
        let ground = world.terrains[0].heightmap.height_at(64.0, 64.0);
        Player {
            motion: Motion { position: Vec3::new(64.0, ground + PLAYER_HEIGHT * 0.5, 64.0), velocity: Vec3::ZERO },
            grounded: true,
        }
    }

    fn horizontal_speed(p: &Player) -> f32 {
        Vec3::new(p.motion.velocity.x, 0.0, p.motion.velocity.z).length()
    }

    #[test]
    fn a_run_reaches_95_percent_of_top_speed_in_the_analytic_time() {
        // Constant-rate approach from rest: v(t) = GROUND_ACCEL * t, so 95% of top speed arrives
        // at 0.95 * MOVE_SPEED / GROUND_ACCEL seconds; the ceil grants the partial step.
        let world = flat_world(2.0);
        let run = StepInput { move_dir: Vec3::X, jump: false };
        let steps = (0.95 * MOVE_SPEED / GROUND_ACCEL / SIM_DT).ceil() as usize;
        let mut p = at_rest(&world);
        for _ in 0..steps {
            p = step(p, run, &world);
        }
        assert!(horizontal_speed(&p) >= 0.95 * MOVE_SPEED - EPS, "speed {} after {steps} steps", horizontal_speed(&p));
        // Top speed is unchanged: the approach arrives at exactly MOVE_SPEED and cruises there.
        for _ in 0..120 {
            p = step(p, run, &world);
            assert!(horizontal_speed(&p) <= MOVE_SPEED + EPS, "overshot top speed: {}", horizontal_speed(&p));
        }
        assert!((horizontal_speed(&p) - MOVE_SPEED).abs() < EPS, "should cruise at top speed: {}", horizontal_speed(&p));
    }

    #[test]
    fn friction_decays_a_full_speed_run_to_exact_rest() {
        // The approach has no asymptote: within MOVE_SPEED / GROUND_FRICTION seconds of no input
        // the horizontal velocity is exactly zero, not a lingering creep.
        let world = flat_world(2.0);
        let mut p = at_rest(&world);
        p.motion.velocity = Vec3::new(MOVE_SPEED, 0.0, 0.0);
        let steps = (MOVE_SPEED / GROUND_FRICTION / SIM_DT).ceil() as usize;
        for _ in 0..steps {
            p = step(p, StepInput::default(), &world);
        }
        assert_eq!(p.motion.velocity.x, 0.0);
        assert_eq!(p.motion.velocity.z, 0.0);
        assert!(p.grounded, "decaying to rest should never leave the ground");
    }

    #[test]
    fn air_acceleration_is_the_ground_rate_scaled_by_air_control() {
        let world = flat_world(2.0);
        let run = StepInput { move_dir: Vec3::X, jump: false };
        let ground_dv = step(at_rest(&world), run, &world).motion.velocity.x;
        let airborne = Player {
            motion: Motion { position: Vec3::new(64.0, 30.0, 64.0), velocity: Vec3::ZERO },
            grounded: false,
        };
        let air_dv = step(airborne, run, &world).motion.velocity.x;
        assert!((ground_dv - GROUND_ACCEL * SIM_DT).abs() < EPS, "one grounded step from rest gains accel * dt");
        assert!((air_dv - ground_dv * AIR_CONTROL).abs() < EPS, "air {air_dv} vs ground {ground_dv}");
    }
}
