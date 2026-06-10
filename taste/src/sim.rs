//! The fixed-step player simulation: state, the per-step composition, and spawn.
//!
//! This is the loop body the engine deliberately does not own (HLD principle 5): taste holds the
//! player's [`Motion`] and grounded flag, and each fixed step sequences wok-physics's pure pieces in
//! the composition the Level 2 locomotion harness proved out (wok-physics/tests/locomotion_replay):
//!
//!     set the horizontal velocity from intent     (and the jump impulse, the game's policy)
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
    GRAVITY, JUMP_VELOCITY, MOVE_SPEED, PLAYER_HEIGHT, PLAYER_RADIUS, SIM_DT, SNAP_DOWN_DISTANCE, SPAWN_HEIGHT,
    WALKABLE_COS,
};
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

/// Advance the player by one fixed step. Pure: identical player, input, and world give an identical
/// next player, bit for bit.
pub fn step(player: Player, input: StepInput, world: &World) -> Player {
    // Direct-control locomotion: intent sets the horizontal velocity outright; vertical velocity
    // carries the gravity the body has accumulated, plus the jump impulse when one was asked for
    // and the body has ground to push off.
    let mut m = player.motion;
    m.velocity.x = input.move_dir.x * MOVE_SPEED;
    m.velocity.z = input.move_dir.z * MOVE_SPEED;
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

    // Landing policy (the harness's): grounding on box geometry or being lifted by the terrain
    // spends the downward fall.
    let mut grounded = slid.grounded || rested_grounded;
    let mut velocity = slid.velocity;
    let mut position = rested_position;
    if slid.grounded || rested_position.y > slid.position.y {
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
}
