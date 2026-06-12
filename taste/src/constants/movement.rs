//! Movement constants the build decides: the fixed step, the player body, the step-up, the spawn,
//! and the placeholder color.
//!
//! What used to live here too - locomotion speeds, the parameterized jump, air steering, the wall
//! policies, the walkable limit, and the downhill glue - moved to `crate::tuning`, the
//! hot-reloadable feel record, because those are the numbers a play-test verdict retunes live.
//! What stays is what changing mid-play would make a different game, not a different feel: the
//! simulation rate (the determinism contract's day-one decision), the player's dimensions (the body
//! the collider and the bean mesh share), the step-up height, the spawn height, and the body's
//! color. The relationship sanity these values used to assert against the moved ones now lives in
//! `Tuning::validate` (and the derivation round-trips in `crate::tuning`'s own tests); the tests
//! kept here pin only the relationships among the constants that remain.

use glam::Vec3;

// ---- simulation ----

/// The fixed simulation timestep: 60 steps per second of game time, never a wall-clock delta. The
/// fixed dt is the day-one decision behind deterministic scripted-input replay.
pub const SIM_DT: f32 = 1.0 / 60.0;

/// Most fixed steps one rendered frame may consume. A long stall (a debugger pause, a window drag)
/// otherwise turns into a catch-up burst that itself takes too long, which accumulates more debt: the
/// spiral of death. Past the clamp the leftover time is dropped and the game slows down instead.
pub const MAX_STEPS_PER_FRAME: u32 = 8;

// ---- player ----

/// Player body total height and radius, in metres - shared by the collider and the visual. The
/// COLLIDER is a flat-bottomed vertical cylinder of exactly these dimensions
/// (`Cylinder::upright`): the flat bottom is what stands on tilted faces, overhangs ledges, and
/// does not roll off edges. The VISUAL stays the bean (the capsule mesh at the same height and
/// radius; the mismatch is documented at the draw site in `crate::app`). 1.5 over 0.9 wide is the
/// bean silhouette the play-tests settled on. The body is not feel tuning: changing it mid-play is
/// a different character, so it stays a constant rather than moving to `crate::tuning`.
pub const PLAYER_HEIGHT: f32 = 1.5;
pub const PLAYER_RADIUS: f32 = 0.45;

/// The drawn capsule's wall (cylinder segment) length the height and radius imply - what the bean
/// mesh (`capsule_mesh`) sizes its straight section from. Visual only since the collider became
/// the cylinder: the collider's straight wall is the full PLAYER_HEIGHT.
pub const PLAYER_SEGMENT: f32 = PLAYER_HEIGHT - 2.0 * PLAYER_RADIUS;

/// Step-up height, in metres: a grounded walk blocked by a wall-grade contact no taller than this
/// above the foot climbs it (lift-move-drop in `crate::slide`) instead of stopping. The flat
/// bottom needs the policy where the capsule's rounded bottom glided up small lips for free; 0.3m
/// is shin height - kerbs and stair treads climb, crates (0.5m and up) are walls. Stays a constant
/// rather than feel tuning: it is a property of the body's reach, paired with the player dimensions.
pub const STEP_HEIGHT: f32 = 0.3;

/// How far above the terrain surface the player spawns, in metres: high enough that the opening
/// moments show gravity and the landing. Where play begins, not a feel value, so it stays here.
pub const SPAWN_HEIGHT: f32 = 10.0;

/// The placeholder ellipsoid's flat base color (linear RGB): a warm signal orange, distinct from
/// every surface-tag color the terrain and prefabs use.
pub const PLAYER_COLOR: Vec3 = Vec3::new(0.90, 0.35, 0.15);

#[cfg(test)]
// Asserting on constants is this module's entire purpose: the tests pin relationships between the
// constants that stay so a retune that breaks an assumption fails loudly. The lint assumes a
// constant assertion is an accident; here it is the point. (Relationships involving the moved feel
// values are pinned by `Tuning::validate` and `crate::tuning`'s tests instead.)
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
    fn the_player_body_is_taller_than_it_is_wide_and_the_bean_keeps_its_wall() {
        // The drawn capsule needs height > 2 * radius or its straight section vanishes (the
        // mesh's silhouette); the cylinder collider shares the numbers, so this also keeps the
        // body a standing shape rather than a coin.
        assert!(PLAYER_HEIGHT > 2.0 * PLAYER_RADIUS);
        assert!(PLAYER_RADIUS > 0.0);
    }

    #[test]
    fn the_step_height_is_a_shin_not_a_climb() {
        // Zero would retire the policy; at or above half the body the "step" would swallow the
        // crates the demo treats as obstacles, and a step should never substitute for a jump. (That
        // a step stays under the jump's apex is pinned by `Tuning::validate`, since the apex is feel
        // tuning now.)
        assert!(STEP_HEIGHT > 0.0);
        assert!(STEP_HEIGHT < PLAYER_HEIGHT * 0.5, "a step is climbed by the feet, not the body");
    }

    #[test]
    fn the_spawn_is_above_the_ground() {
        assert!(SPAWN_HEIGHT > 0.0);
    }
}
