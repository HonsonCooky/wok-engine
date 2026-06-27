//! Movement constants the build decides: the fixed step, the player body, the spawn, the
//! vertical-stillness guards, and the placeholder color.
//!
//! The feel numbers - the run speed, gravity, the jump - live in `crate::tuning`, the
//! hot-reloadable record, because those are what a play-test verdict retunes live. What stays here
//! is what changing mid-play would make a different game, not a different feel: the simulation rate
//! (the determinism contract's day-one decision), the player's dimensions (the body the collider
//! and the bean mesh share), the spawn height, the velocity threshold and dwell the jump reset
//! triggers on (numerical guards against the apex, not feel numbers), and the body's color. The
//! tests kept here pin only the relationships among these remaining constants.

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

/// How far above the terrain surface the player spawns, in metres: high enough that the opening
/// moments show gravity and the landing. Where play begins, not a feel value, so it stays here.
pub const SPAWN_HEIGHT: f32 = 10.0;

/// How close to zero the vertical velocity must be to count as still, in m/s, for the jump-reset
/// timer (`crate::sim`). Resting on the ground or a surface pins the vertical velocity to exactly
/// zero, so any positive value registers a rest; the only thing this margin must reject is a jump's
/// apex, where the vertical velocity grazes zero for a single step before the fall. A numerical
/// tolerance, not feel tuning, so it stays a constant.
pub const STILL_VY: f32 = 0.05;

/// How long the body must stay vertically still before the jump counter refills, in seconds: more
/// than one fixed step, the smallest dwell that rejects a jump's apex (still for a single step as
/// the vertical velocity crosses zero) while a real landing - still for as long as the player rests
/// - refills the jumps within a couple of hundredths of a second, no perceptible wait. This is a
/// robustness guard against the apex, NOT a cooldown, so it stays a constant rather than feel
/// tuning. Reading stillness instead of a grounded flag is what keeps complex terrain from ever
/// stranding the player without a jump.
pub const JUMP_RESET_DWELL: f32 = 1.5 * SIM_DT;

/// The steepest descending slope (as a gradient, rise/run) the body stays glued to: the ground-snap
/// reaches `GROUND_SNAP_GRADE * move_speed * SIM_DT` below the feet - the terrain a step down the
/// steepest glued slope falls away - and pulls a grounded, non-rising body onto a surface within that
/// reach (`crate::sim`), so walking downhill follows the ground instead of floating off it and
/// free-falling to catch up (the jitter). Beyond the reach the body falls, so a real drop or a ledge
/// is still a fall, never a downhill glide off a cliff. 2.0 (~63 degrees) clears the 60-degree
/// walkable limit with headroom. Scaled by the per-step distance, not an absolute band, so it tracks
/// a retuned run speed. A geometric guard against the descending-slope float, not feel tuning, so it
/// stays a constant.
pub const GROUND_SNAP_GRADE: f32 = 2.0;

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
    fn the_spawn_is_above_the_ground() {
        assert!(SPAWN_HEIGHT > 0.0);
    }

    #[test]
    fn the_stillness_threshold_is_a_small_positive_margin() {
        // Positive so a true rest (vertical velocity pinned to zero) registers, and well under the
        // run speed so it is unmistakably a vertical-only "barely moving" test.
        assert!(STILL_VY > 0.0);
        assert!(STILL_VY < 1.0, "the stillness margin should be a sliver, not a real speed");
    }
}
