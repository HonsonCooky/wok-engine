//! Gameplay tuning constants: the numbers that make taste feel like taste.
//!
//! These are gameplay policy, not engine values (HLD principle 5: the engine provides the math, the
//! game owns the numbers). Everything a designer would reach for to retune the demo lives here, in
//! one place: the fixed simulation rate, gravity, locomotion speeds, the player capsule's shape, and
//! the follow camera's geometry and easing rates. The sanity tests at the bottom pin the structural
//! relationships between them (a capsule taller than its own sphere, a jump that clears something, a
//! boom longer than its probe), so a retune that breaks the demo's assumptions fails in `cargo test`
//! rather than in play.

use glam::Vec3;

// ---- simulation ----

/// The fixed simulation timestep: 60 steps per second of game time, never a wall-clock delta. The
/// fixed dt is the day-one decision behind deterministic scripted-input replay.
pub const SIM_DT: f32 = 1.0 / 60.0;

/// Most fixed steps one rendered frame may consume. A long stall (a debugger pause, a window drag)
/// otherwise turns into a catch-up burst that itself takes too long, which accumulates more debt: the
/// spiral of death. Past the clamp the leftover time is dropped and the game slows down instead.
pub const MAX_STEPS_PER_FRAME: u32 = 8;

/// Constant downward acceleration in m/s^2. Deliberately stronger than Earth's 9.8: game jumps read
/// floaty at physical gravity, and a fast fall makes landing feel intentional.
pub const GRAVITY: Vec3 = Vec3::new(0.0, -25.0, 0.0);

/// cos(45 degrees): the steepest slope that still counts as walkable ground, passed to both the
/// slide and the terrain rest so the two grounded signals agree.
pub const WALKABLE_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;

// ---- player ----

/// Player capsule total height (tip to tip) and radius, in metres. The segment length follows from
/// these via `Capsule::upright`: half-segment = height / 2 - radius. A squat, wide bean rather than
/// a tall pill: play-testing read the character better low and round under a third-person camera.
pub const PLAYER_HEIGHT: f32 = 1.1;
pub const PLAYER_RADIUS: f32 = 0.45;

/// Horizontal locomotion speed in m/s, a brisk run.
pub const MOVE_SPEED: f32 = 6.0;

/// Upward velocity granted by a jump, in m/s. With this gravity it clears about 1.3m at the apex.
pub const JUMP_VELOCITY: f32 = 8.0;

/// How far above the terrain surface the player spawns, in metres: high enough that the opening
/// moments show gravity and the landing.
pub const SPAWN_HEIGHT: f32 = 10.0;

/// Ground glue, in metres: walking downhill, the surface falls away faster than one step of
/// gravity can follow, so without glue a grounded walk flickers airborne every step. If the player
/// was grounded, did not jump, and the support is within this distance below the foot after the
/// move, the foot snaps to the support and stays grounded. Genuine drops (a ledge taller than
/// this) still go airborne, and a jump always leaves the ground. Game policy, not physics: the
/// engine's terrain rest is lift-only by design.
pub const SNAP_DOWN_DISTANCE: f32 = 0.3;

/// The placeholder ellipsoid's flat base color (linear RGB): a warm signal orange, distinct from
/// every surface-tag color the terrain and prefabs use.
pub const PLAYER_COLOR: Vec3 = Vec3::new(0.90, 0.35, 0.15);

// ---- camera ----

/// Unobstructed boom length from the look target out to the camera, in metres. Scaled to the bean:
/// at the original 6m a 1.1m character read as a speck against the hills.
pub const CAMERA_DISTANCE: f32 = 5.0;

/// Radius of the sphere the spring arm sweeps along the boom. It doubles as the standoff: the
/// camera rides the sphere's centre, so it stops this far in front of whatever the sweep hits.
pub const CAMERA_PROBE_RADIUS: f32 = 0.3;

/// The look target sits this far above the capsule centre, just under the bean's crown (centre is
/// height/2 over the ground at rest), so the camera frames the player rather than its waist. Must
/// stay inside the body: a target floating over the head reads as the camera staring at nothing.
pub const CAMERA_TARGET_LIFT: f32 = 0.35;

/// Minimum height the camera keeps above the terrain surface under it, in metres.
pub const CAMERA_TERRAIN_MARGIN: f32 = 0.4;

/// Half-life of the spring-arm length easing, in seconds: how quickly the boom recovers after an
/// obstruction pulls it in. Short, so walls feel solid rather than spongy.
pub const CAMERA_ARM_HALF_LIFE: f32 = 0.08;

/// Half-life of the camera-position easing, in seconds: the follow lag. Shorter than the arm's so
/// the camera never visibly trails through geometry the arm already cleared.
pub const CAMERA_POS_HALF_LIFE: f32 = 0.05;

/// Mouse-look sensitivity, radians of orbit per pixel of raw motion.
pub const LOOK_SENSITIVITY: f32 = 0.0035;

/// Look inversion toggles, applied to the mouse and the right stick alike. With both false (the
/// third-person default): dragging or pushing right turns the view right (the boom swings the
/// opposite way around the player), and pushing forward raises the camera to look down on the
/// player. Set one to true to flip that axis; feel is a constant here, not a code hunt through the
/// input mapping.
pub const LOOK_INVERT_X: bool = false;
pub const LOOK_INVERT_Y: bool = false;

// ---- gamepad ----

/// Radial stick deadzone, as a fraction of full deflection. Below it a stick reads zero (resting
/// sticks drift); past it the magnitude rescales from zero so analog control stays continuous
/// rather than jumping to the deadzone's edge value.
pub const STICK_DEADZONE: f32 = 0.15;

/// Orbit turn rate at full right-stick deflection, radians per second. A stick is a rate device
/// (deflection held over time), unlike the mouse (a displacement device), so it gets its own
/// sensitivity in rate units and is integrated by the frame dt.
pub const STICK_LOOK_RATE: f32 = 2.5;

// ---- diagnostics ----

/// Draw the ground-truth marker: a bright quad at the sampled terrain height under the player,
/// composed through the same chunk-origin path as the terrain mesh. For the floating-at-rest
/// diagnosis: if the marker lies on the rendered terrain while the bean floats, the gap is in the
/// rest math; if the marker itself disagrees with the rendered terrain, sampling and mesh disagree.
pub const DEBUG_GROUND_MARKER: bool = true;

/// Orbit pitch limits, radians. Positive pitch raises the camera (wok-physics boom convention);
/// the floor allows a slight under-shoulder look and the ceiling stops short of straight overhead.
pub const PITCH_MIN: f32 = -0.20;
pub const PITCH_MAX: f32 = 1.35;

/// Starting orbit pitch: a little above the shoulder, looking gently down.
pub const PITCH_DEFAULT: f32 = 0.35;

/// Vertical field of view and near plane for the projection. The far plane is per-frame data (fog
/// distance sets render distance, per the HLD), so it is a parameter, not a constant.
pub const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
pub const NEAR_PLANE: f32 = 0.1;

#[cfg(test)]
// Asserting on constants is this module's entire purpose: the tests pin relationships between
// tuning values so a retune that breaks an assumption fails loudly. The lint assumes a constant
// assertion is an accident; here it is the point. Exact float comparison is likewise intended:
// these are declared values, not computed ones.
#[allow(clippy::assertions_on_constants, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn the_simulation_rate_is_a_real_fixed_step() {
        assert!(SIM_DT > 0.0, "a non-positive dt advances nothing");
        assert!(MAX_STEPS_PER_FRAME >= 1, "a frame must be able to consume at least one step");
        // The clamp must cover at least a couple of ordinary frames of debt, or normal jitter stalls.
        assert!(MAX_STEPS_PER_FRAME as f32 * SIM_DT >= 2.0 / 60.0);
    }

    #[test]
    fn gravity_points_down_and_only_down() {
        assert!(GRAVITY.y < 0.0);
        assert_eq!(GRAVITY.x, 0.0);
        assert_eq!(GRAVITY.z, 0.0);
    }

    #[test]
    fn the_player_capsule_is_taller_than_its_own_sphere() {
        // Below 2 * radius the upright capsule degrades to a sphere and the segment is gone.
        assert!(PLAYER_HEIGHT > 2.0 * PLAYER_RADIUS);
        assert!(PLAYER_RADIUS > 0.0);
    }

    #[test]
    fn a_jump_clears_something_worth_jumping() {
        // Apex height under constant gravity: v^2 / 2g. It should clear at least a crate edge but
        // not a pillar, or the demo's prefab walls stop being obstacles.
        let apex = JUMP_VELOCITY * JUMP_VELOCITY / (2.0 * -GRAVITY.y);
        assert!(apex > 0.5, "apex {apex} too low to feel like a jump");
        assert!(apex < 4.0, "apex {apex} clears the demo's tall prefabs");
    }

    #[test]
    fn the_walkable_threshold_is_a_real_slope_limit() {
        // cos of an angle strictly between flat (1.0) and vertical (0.0).
        assert!(WALKABLE_COS > 0.0 && WALKABLE_COS < 1.0);
    }

    #[test]
    fn the_boom_outreaches_its_probe_and_the_easing_is_live() {
        assert!(CAMERA_DISTANCE > CAMERA_PROBE_RADIUS, "a boom shorter than its probe never extends");
        assert!(CAMERA_PROBE_RADIUS > 0.0);
        assert!(CAMERA_ARM_HALF_LIFE > 0.0 && CAMERA_POS_HALF_LIFE > 0.0);
        assert!(CAMERA_POS_HALF_LIFE <= CAMERA_ARM_HALF_LIFE, "position easing should not trail the arm");
    }

    #[test]
    fn the_pitch_range_contains_its_default() {
        assert!(PITCH_MIN < PITCH_MAX);
        assert!((PITCH_MIN..=PITCH_MAX).contains(&PITCH_DEFAULT));
    }

    #[test]
    fn the_snap_distance_covers_a_full_speed_step_down_the_steepest_walkable_slope() {
        // One step of full-speed walking descends at most MOVE_SPEED * SIM_DT * tan(max slope);
        // with WALKABLE_COS = cos(45 deg) that gradient is 1. The glue must cover it, or a fast
        // downhill walk outruns the snap and flickers airborne - the exact bug the glue removes.
        let max_walkable_gradient = (1.0 - WALKABLE_COS * WALKABLE_COS).sqrt() / WALKABLE_COS;
        assert!(SNAP_DOWN_DISTANCE >= MOVE_SPEED * SIM_DT * max_walkable_gradient);
        // And it must stay a glue, not a teleport: well under the player's own height.
        assert!(SNAP_DOWN_DISTANCE < PLAYER_HEIGHT * 0.5);
    }

    #[test]
    fn locomotion_and_spawn_are_positive() {
        assert!(MOVE_SPEED > 0.0);
        assert!(JUMP_VELOCITY > 0.0);
        assert!(SPAWN_HEIGHT > 0.0);
        assert!(LOOK_SENSITIVITY > 0.0);
    }

    #[test]
    fn the_stick_deadzone_leaves_a_live_range() {
        // A deadzone of 1.0 or more silences the stick entirely; the rescale divides by its
        // complement, so it must also stay strictly below 1.
        assert!((0.0..1.0).contains(&STICK_DEADZONE));
        assert!(STICK_LOOK_RATE > 0.0);
    }
}
