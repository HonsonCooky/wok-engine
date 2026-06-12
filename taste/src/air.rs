//! Airborne steering: pure momentum, the stick turns the heading and nothing else.
//!
//! The air model has shed two mechanisms on play verdicts. First the grounded-style straight
//! approach (acceleration through zero): reversing direction mid-jump braked through a dead stop,
//! the opposite of the BFBB / Ratchet & Clank air authority where a jump's heading can be turned
//! around while the body keeps moving. Then the decoupled speed-magnitude approach (AIR_ACCEL)
//! that replaced its speed half: any airborne speed change lets a held stick stretch or shrink a
//! jump's reach mid-flight, and the verdict was that a jump's horizontal speed is set at launch,
//! period. What remains is the direction half alone: with input, [`steer`] rotates the horizontal
//! velocity's heading toward the stick at `AIR_TURN_RATE` radians per second (about the vertical
//! axis, taking the shorter way around); the speed magnitude is never touched in the air. With no
//! input the velocity is returned untouched: ballistic, the third-mechanism fix that stands from
//! the feel arc.
//!
//! From rest there is no heading to rotate, so a standing (zero-speed) jump is unsteerable: a
//! known consequence, deliberately accepted, pinned with its contingency in the tests below.
//!
//! Deterministic: fixed arithmetic of the inputs (atan2 and sin/cos are deterministic for a given
//! build, the canon's bar), no RNG, no state.

use glam::Vec3;

use crate::tuning::Tuning;

/// Below this squared speed the velocity has no usable heading; with nothing to rotate, steering
/// leaves it alone (the unsteerable standing jump) instead of rotating noise.
const NO_HEADING_SQ: f32 = 1e-8;

/// One fixed step of airborne steering: the horizontal velocity (`y` must be zero; the caller
/// splits it off) under `move_dir` (the world-space intent, length at most one) over `dt` seconds,
/// turning at the tuning's air turn rate. Pure momentum: only the heading changes, never the speed.
/// No input returns the velocity unchanged - ballistic.
pub fn steer(horizontal: Vec3, move_dir: Vec3, dt: f32, tuning: &Tuning) -> Vec3 {
    if move_dir == Vec3::ZERO {
        return horizontal;
    }
    let speed_sq = horizontal.length_squared();
    if speed_sq <= NO_HEADING_SQ {
        return horizontal;
    }

    // Rotate the current heading toward the stick's by at most air_turn_rate * dt, the shorter
    // way around the vertical axis; the magnitude rides through untouched.
    let current = horizontal.z.atan2(horizontal.x);
    let wanted = move_dir.z.atan2(move_dir.x);
    let diff = wrap_angle(wanted - current);
    let turn = diff.clamp(-tuning.air_turn_rate * dt, tuning.air_turn_rate * dt);
    let heading = current + turn;
    Vec3::new(heading.cos(), 0.0, heading.sin()) * speed_sq.sqrt()
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
        let t = Tuning::default();
        let v = Vec3::new(5.0, 0.0, -2.0);
        assert_eq!(steer(v, Vec3::ZERO, SIM_DT, &t), v, "airborne with no input must not touch velocity");
    }

    #[test]
    fn a_reversal_rotates_through_the_turn_with_the_speed_untouched() {
        // Entry at full speed +x, stick held -x: the heading turns half a circle while the speed
        // stays the entry speed at every step (to rotation roundoff) - pure momentum's whole
        // claim, against both the old brake-through-zero model and the retired AIR_ACCEL
        // magnitude approach.
        let t = Tuning::default();
        let entry = t.move_speed;
        let mut v = Vec3::new(entry, 0.0, 0.0);
        let steps = (std::f32::consts::PI / t.air_turn_rate / SIM_DT).ceil() as usize;
        for i in 0..steps {
            v = steer(v, Vec3::NEG_X, SIM_DT, &t);
            assert!(
                (speed(v) - entry).abs() < 1e-3,
                "step {i}: the speed changed mid-turn ({} vs entry {entry})",
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
        let t = Tuning::default();
        let mut v = Vec3::new(t.move_speed, 0.0, 0.0);
        let quarter_steps = (std::f32::consts::FRAC_PI_2 / t.air_turn_rate / SIM_DT).round() as usize;
        for _ in 0..quarter_steps / 2 {
            v = steer(v, Vec3::Z, SIM_DT, &t);
        }
        let angle = v.z.atan2(v.x);
        let expected = t.air_turn_rate * SIM_DT * (quarter_steps / 2) as f32;
        assert!((angle - expected).abs() < 1e-4, "angle {angle} vs rate-limited {expected}");
    }

    #[test]
    fn airborne_speed_never_changes_under_input() {
        // Pure momentum's other face: whether entry is slower or faster than the run speed, a
        // steering step leaves the magnitude alone - there is no airborne approach toward an
        // intended speed (AIR_ACCEL retired), and an over-speed body keeps its momentum.
        let t = Tuning::default();
        for entry in [2.0, t.move_speed, 12.0] {
            let v = steer(Vec3::new(entry, 0.0, 0.0), Vec3::X, SIM_DT, &t);
            assert!((speed(v) - entry).abs() < 1e-5, "entry {entry}: speed became {}", speed(v));
            assert!(v.z.abs() < 1e-6, "heading must not drift when it already matches");
        }
    }

    #[test]
    fn a_standing_jump_is_unsteerable() {
        // The accepted consequence of pure momentum: from rest there is no heading to rotate and
        // no other airborne mechanism, so a standing (zero-speed) jump cannot be steered at all.
        // Deliberate for now. The contingency, to be added only if play demands it, is a small
        // get-moving floor (~2.5 m/s along the stick when the airborne speed is zero) - not a
        // return of airborne acceleration.
        let t = Tuning::default();
        let v = steer(Vec3::ZERO, Vec3::X, SIM_DT, &t);
        assert_eq!(v, Vec3::ZERO, "a zero-speed body must stay put under the stick: {v:?}");
    }

    #[test]
    fn steering_is_deterministic() {
        let t = Tuning::default();
        let v = Vec3::new(3.0, 0.0, 4.0);
        let d = Vec3::new(-0.6, 0.0, 0.8);
        assert_eq!(steer(v, d, SIM_DT, &t), steer(v, d, SIM_DT, &t));
    }
}
