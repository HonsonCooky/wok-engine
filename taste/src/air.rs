//! Airborne steering: redirection, not acceleration-through-zero.
//!
//! The old air model scaled the grounded approach by an air-control factor: airborne input accelerated the
//! horizontal velocity toward the intended velocity along a straight line. Reversing direction
//! mid-jump therefore braked through a dead stop - the velocity passed through zero - which is the
//! opposite of the BFBB / Ratchet & Clank air authority the demo wants, where a jump's heading can
//! be turned around while the body keeps moving.
//!
//! The fix splits direction from speed. With input, [`steer`] rotates the horizontal velocity's
//! DIRECTION toward the stick at `AIR_TURN_RATE` radians per second (about the vertical axis,
//! taking the shorter way around), while the speed MAGNITUDE approaches the intended speed at
//! `AIR_ACCEL` - its own airborne rate, decoupled from the grounded acceleration so a ground
//! crispness retune can never silently change the air feel. A redirect never passes through a
//! stop, but gaining or losing speed in air stays gradual. With no input the velocity is returned
//! untouched: ballistic, the third-mechanism fix that stands from the feel arc.
//!
//! From rest there is no direction to rotate, so steering degrades to the old straight approach
//! along the stick - which is also exactly what the previous model did from rest, keeping the
//! air-control feel of a vertical jump unchanged.
//!
//! Deterministic: fixed arithmetic of the inputs (atan2 and sin/cos are deterministic for a given
//! build, the canon's bar), no RNG, no state.

use glam::Vec3;

use crate::constants::{AIR_ACCEL, AIR_TURN_RATE, MOVE_SPEED};

/// Below this squared speed the velocity has no usable heading; steering starts from rest along
/// the stick instead of rotating noise.
const NO_HEADING_SQ: f32 = 1e-8;

/// One fixed step of airborne steering: the horizontal velocity (`y` must be zero; the caller
/// splits it off) under `move_dir` (the world-space intent, length at most one) over `dt` seconds.
/// No input returns the velocity unchanged - ballistic.
pub fn steer(horizontal: Vec3, move_dir: Vec3, dt: f32) -> Vec3 {
    if move_dir == Vec3::ZERO {
        return horizontal;
    }
    let target = Vec3::new(move_dir.x, 0.0, move_dir.z) * MOVE_SPEED;
    let accel = AIR_ACCEL * dt;
    let speed = horizontal.length();
    if speed * speed <= NO_HEADING_SQ {
        // From rest: accelerate straight along the stick (the rotation has nothing to turn).
        let gap = target - horizontal;
        let len = gap.length();
        return if len <= accel { target } else { horizontal + gap * (accel / len) };
    }

    // Direction: rotate the current heading toward the stick's by at most AIR_TURN_RATE * dt,
    // the shorter way around the vertical axis.
    let current = horizontal.z.atan2(horizontal.x);
    let wanted = target.z.atan2(target.x);
    let diff = wrap_angle(wanted - current);
    let turn = diff.clamp(-AIR_TURN_RATE * dt, AIR_TURN_RATE * dt);
    let heading = current + turn;

    // Speed: the same constant-rate approach locomotion uses, toward the intended speed.
    let target_speed = target.length();
    let gap = target_speed - speed;
    let new_speed = if gap.abs() <= accel { target_speed } else { speed + accel * gap.signum() };

    Vec3::new(heading.cos(), 0.0, heading.sin()) * new_speed
}

/// Wrap an angle difference into `[-pi, pi]`, so the rotation takes the shorter way. An exact
/// half-circle comes out at one end of the range - a fixed, deterministic tie.
fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let r = a.rem_euclid(TAU);
    if r > PI { r - TAU } else { r }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::constants::SIM_DT;

    fn speed(v: Vec3) -> f32 {
        v.length()
    }

    #[test]
    fn no_input_is_ballistic() {
        let v = Vec3::new(5.0, 0.0, -2.0);
        assert_eq!(steer(v, Vec3::ZERO, SIM_DT), v, "airborne with no input must not touch velocity");
    }

    #[test]
    fn a_reversal_rotates_through_the_turn_without_speed_collapse() {
        // Entry at full speed +x, stick held -x: the old model braked through zero; the redirect
        // must turn the heading half a circle while the speed never drops below 80% of entry.
        // (At matched entry and intent speeds the magnitude approach is nearly idle, so the floor
        // is really pinning that direction change stopped being a brake.)
        let entry = MOVE_SPEED;
        let mut v = Vec3::new(entry, 0.0, 0.0);
        let steps = (std::f32::consts::PI / AIR_TURN_RATE / SIM_DT).ceil() as usize;
        for i in 0..steps {
            v = steer(v, Vec3::NEG_X, SIM_DT);
            assert!(
                speed(v) >= 0.8 * entry,
                "step {i}: speed collapsed to {} during the turn (the brake-through-zero model)",
                speed(v)
            );
        }
        let dir = v / speed(v);
        assert!(dir.dot(Vec3::NEG_X) > 0.99, "should have finished the reversal: {dir:?}");
    }

    #[test]
    fn a_quarter_turn_takes_the_shorter_way_at_the_turn_rate() {
        // Heading +x, stick +z: after exactly half the quarter-turn time the heading has rotated
        // half the way (the rotation is rate-limited, not snapped).
        let mut v = Vec3::new(MOVE_SPEED, 0.0, 0.0);
        let quarter_steps = (std::f32::consts::FRAC_PI_2 / AIR_TURN_RATE / SIM_DT).round() as usize;
        for _ in 0..quarter_steps / 2 {
            v = steer(v, Vec3::Z, SIM_DT);
        }
        let angle = v.z.atan2(v.x);
        let expected = AIR_TURN_RATE * SIM_DT * (quarter_steps / 2) as f32;
        assert!((angle - expected).abs() < 1e-4, "angle {angle} vs rate-limited {expected}");
    }

    #[test]
    fn speed_still_approaches_intent_at_the_air_accel_rate() {
        // Entry slower than intent, heading already on the stick: one step gains exactly
        // AIR_ACCEL * dt - the magnitude half of the model runs at the decoupled airborne rate.
        let v = steer(Vec3::new(2.0, 0.0, 0.0), Vec3::X, SIM_DT);
        assert!((speed(v) - (2.0 + AIR_ACCEL * SIM_DT)).abs() < 1e-5, "speed {}", speed(v));
        assert!(v.z.abs() < 1e-6, "heading must not drift when it already matches");
    }

    #[test]
    fn from_rest_steering_accelerates_straight_along_the_stick() {
        let v = steer(Vec3::ZERO, Vec3::X, SIM_DT);
        assert!((v.x - AIR_ACCEL * SIM_DT).abs() < 1e-6, "got {v:?}");
        assert_eq!(v.z, 0.0);
    }

    #[test]
    fn steering_is_deterministic() {
        let v = Vec3::new(3.0, 0.0, 4.0);
        let d = Vec3::new(-0.6, 0.0, 0.8);
        assert_eq!(steer(v, d, SIM_DT), steer(v, d, SIM_DT));
    }
}
