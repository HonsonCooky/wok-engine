//! The fixed-step player simulation: state, the per-step composition, and spawn.
//!
//! This is the loop body the engine deliberately does not own (HLD principle 5): taste holds the
//! player's [`Motion`] and grounded flag, and each fixed step sequences wok-physics's pure pieces in
//! the composition the Level 2 locomotion harness proved out (wok-physics/tests/locomotion_replay):
//!
//!     steer the horizontal velocity toward intent (grounded approach or air redirection, plus
//!                                                  the jump impulses, coyote grace, and the
//!                                                  variable-height cut - the game's policy)
//!     -> integrate one fixed step under gravity   (wok-physics: integrate; ascent gravity while
//!                                                  rising, the heavier fall gravity otherwise)
//!     -> collide-and-slide against the statics    (crate::slide over wok-physics's sweep:
//!                                                  supported ground contacts resolve flat)
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
use wok_physics::{Capsule, Motion, boom_direction, integrate, rest_on_heightmap};

use crate::air;
use crate::slide::slide_player;
use crate::constants::{
    AIR_JUMP_SCALE, AIR_JUMPS, ASCENT_GRAVITY, COYOTE_S, FALL_GRAVITY, GROUND_ACCEL, GROUND_FRICTION,
    JUMP_CUT_FACTOR, JUMP_VELOCITY, MOVE_SPEED, PLAYER_HEIGHT, PLAYER_RADIUS, SIM_DT, SNAP_DOWN_DISTANCE,
    SPAWN_HEIGHT, WALKABLE_COS,
};
use crate::landing::supported_below;
use crate::world::{CHUNK_SIZE_M, World};

/// The player: a capsule-shaped body the game owns between steps. `motion.position` is the capsule
/// centre (wok-physics's reference point for every resolve).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub motion: Motion,
    pub grounded: bool,
    /// Air jumps still available: spent by jumping airborne, restored to `AIR_JUMPS` on any
    /// grounding. Part of the stepped state, so replay covers it like position and velocity.
    pub air_jumps: u32,
    /// Coyote grace remaining, in seconds of simulation time: refreshed to `COYOTE_S` while
    /// grounded, burned down one `SIM_DT` per airborne step, zeroed by any jump. While positive an
    /// airborne press still fires a full ground jump without spending the air jump - the
    /// walked-off-a-ledge forgiveness. Stepped state, so replay covers it.
    pub coyote: f32,
    /// Whether this ascent still has its jump cut available: armed by every jump (ground, coyote,
    /// or air), spent by the cut, cleared on grounding. The once-per-jump guarantee behind
    /// variable jump height.
    pub cut_armed: bool,
}

impl Player {
    /// Does this player, at step entry, have a jump to give: grounded, inside the coyote window,
    /// or holding an air jump? The single definition the jump latch and the step's own jump check
    /// both read, so the latch can never fire a press the step would drop.
    pub fn can_jump(&self) -> bool {
        self.grounded || self.coyote > 0.0 || self.air_jumps > 0
    }
}

/// One fixed step's input, already resolved against the camera: a world-space horizontal move
/// direction of length at most one, whether a jump was asked for, and whether the jump control is
/// down at all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepInput {
    pub move_dir: Vec3,
    pub jump: bool,
    /// Is the jump control held this step? A level, not an edge: the jump cut reads the release
    /// as this going low while the body still rises. A level survives zero-step frames without a
    /// latch (the state persists until a step samples it), and it is the only release signal both
    /// devices can supply - the platform exposes no gamepad button-release edge.
    pub jump_held: bool,
}

/// The gravity in force for a body whose vertical velocity is `vy`, as the acceleration
/// `integrate` takes. Rising bodies decelerate under the jump's derived ascent gravity; everything
/// else (descent, standing) falls under the `FALL_GRAVITY_MULT`-scaled descent gravity. The split
/// is what makes the arc asymmetric: the rise keeps the tuned apex and time, the fall commits.
pub fn gravity(vy: f32) -> Vec3 {
    Vec3::new(0.0, if vy > 0.0 { -ASCENT_GRAVITY } else { -FALL_GRAVITY }, 0.0)
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
    Player {
        motion: Motion { position, velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: AIR_JUMPS,
        coyote: 0.0,
        cut_armed: false,
    }
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
    // Locomotion splits by grounding. Grounded, the horizontal velocity approaches intent *
    // MOVE_SPEED at a constant rate (GROUND_ACCEL with input, GROUND_FRICTION toward rest
    // without), so starts and stops take a beat and read as weight. Airborne, steering is
    // redirection (`crate::air`): input rotates the velocity's direction toward the stick at
    // AIR_TURN_RATE while the speed approaches the intended speed at the AIR_CONTROL-scaled
    // accel, so a mid-air do-over turns the moving body around instead of braking through a dead
    // stop; no input stays ballistic - the momentum a jump promises, and what lets a body caught
    // on a crate's corner accumulate slide-off speed and roll free.
    let mut m = player.motion;
    let horizontal = Vec3::new(m.velocity.x, 0.0, m.velocity.z);
    let horizontal = if player.grounded {
        let target = Vec3::new(input.move_dir.x, 0.0, input.move_dir.z) * MOVE_SPEED;
        let rate = if input.move_dir == Vec3::ZERO { GROUND_FRICTION } else { GROUND_ACCEL };
        approach(horizontal, target, rate * SIM_DT)
    } else {
        air::steer(horizontal, input.move_dir, SIM_DT)
    };
    m.velocity.x = horizontal.x;
    m.velocity.z = horizontal.z;

    // The jump impulse: grounded presses - and airborne presses inside the coyote window - push
    // off at full strength without touching the air jump; the window is the walked-off-a-ledge
    // grace, and the press spends it outright so it can never stack a second free jump. Past the
    // window, airborne presses spend an air jump when one remains - the R&C do-over: vertical
    // velocity is SET (not added) to the scaled launch speed, and the horizontal velocity is
    // redirected outright to the current stick direction at the current speed, so the double jump
    // is a full commitment to the new heading; with no stick held the heading is kept. The latch
    // upstream guarantees one press is one jump; this block only decides what a delivered press
    // does. Every fired jump arms the cut below.
    let mut air_jumps = player.air_jumps;
    let mut coyote = player.coyote;
    let mut cut_armed = player.cut_armed;
    let mut jumped = false;
    if input.jump {
        if player.grounded || coyote > 0.0 {
            m.velocity.y = JUMP_VELOCITY;
            jumped = true;
        } else if air_jumps > 0 {
            air_jumps -= 1;
            m.velocity.y = JUMP_VELOCITY * AIR_JUMP_SCALE;
            if input.move_dir != Vec3::ZERO {
                let speed = Vec3::new(m.velocity.x, 0.0, m.velocity.z).length();
                let dir = Vec3::new(input.move_dir.x, 0.0, input.move_dir.z).normalize();
                m.velocity.x = dir.x * speed;
                m.velocity.z = dir.z * speed;
            }
            jumped = true;
        }
        coyote = 0.0;
        cut_armed |= jumped;
    }

    // Variable jump height: the first step that finds the jump control released while the body
    // still rises scales the climb by JUMP_CUT_FACTOR and disarms - once per jump, ground or air.
    // Reading the held level (rather than a release edge) means a press-and-release that fired
    // from the buffer on the landing step cuts immediately: a tap is a short hop however the
    // press was delivered.
    if cut_armed && !input.jump_held && m.velocity.y > 0.0 {
        m.velocity.y *= JUMP_CUT_FACTOR;
        cut_armed = false;
    }

    // One fixed step under gravity - ascent gravity while rising, the heavier fall gravity
    // otherwise - then slide the resulting move along any static geometry it meets. The slide is
    // the game's policy wrapper (`crate::slide`): supported ground contacts resolve flat so
    // standing on walkable geometry never bleeds sideways, walls and airborne contacts resolve
    // exactly as the engine does.
    let next = integrate(m, gravity(m.velocity.y), SIM_DT);
    let capsule = Capsule::upright(m.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let slid = slide_player(capsule, next.position - m.position, next.velocity, &world.statics);

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

    // Any grounding restores the air jumps and refreshes the coyote grace (and retires the cut:
    // the next jump arms its own); airborne, the grace burns down one step of simulation time.
    // The double jump is per airtime, not per life.
    let air_jumps = if grounded { AIR_JUMPS } else { air_jumps };
    let coyote = if grounded { COYOTE_S } else { (coyote - SIM_DT).max(0.0) };
    let cut_armed = !grounded && cut_armed;
    Player { motion: Motion { position, velocity }, grounded, air_jumps, coyote, cut_armed }
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
        Player {
            motion: Motion { position, velocity: Vec3::ZERO },
            grounded: false,
            air_jumps: AIR_JUMPS,
            coyote: 0.0,
            cut_armed: false,
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

    // ---- acceleration ----

    use crate::world::ChunkTerrain;
    use wok_scene::{CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    fn flat_world(height_m: f32) -> World {
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }] }
    }

    /// A player standing at rest mid-chunk: capsule base exactly on the surface, grounded (with
    /// the coyote grace a grounded step would carry).
    fn at_rest(world: &World) -> Player {
        let ground = world.terrains[0].heightmap.height_at(64.0, 64.0);
        Player {
            motion: Motion { position: Vec3::new(64.0, ground + PLAYER_HEIGHT * 0.5, 64.0), velocity: Vec3::ZERO },
            grounded: true,
            air_jumps: AIR_JUMPS,
            coyote: crate::constants::COYOTE_S,
            cut_armed: false,
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
        let run = StepInput { move_dir: Vec3::X, jump: false, jump_held: false };
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
    fn air_acceleration_from_rest_is_the_ground_rate_scaled_by_air_control() {
        // The redirection model's from-rest degenerate is the old straight air accel: one
        // airborne step from rest gains AIR_CONTROL times the grounded gain, along the stick.
        use crate::constants::AIR_CONTROL;
        let world = flat_world(2.0);
        let run = StepInput { move_dir: Vec3::X, jump: false, jump_held: false };
        let ground_dv = step(at_rest(&world), run, &world).motion.velocity.x;
        let airborne = player_at(Vec3::new(64.0, 30.0, 64.0));
        let air_dv = step(airborne, run, &world).motion.velocity.x;
        assert!((ground_dv - GROUND_ACCEL * SIM_DT).abs() < EPS, "one grounded step from rest gains accel * dt");
        assert!((air_dv - ground_dv * AIR_CONTROL).abs() < EPS, "air {air_dv} vs ground {ground_dv}");
    }

}
