//! Frame-rate-independent exponential smoothing: ease a value toward a target by a fixed fraction
//! of the remaining gap per unit time, so where it lands depends on elapsed time, not step count.
//!
//! ## Why not a plain lerp
//!
//! The naive `current += (target - current) * k` eases toward the target, but `k` is a per-*step*
//! fraction: halving `dt` (twice as many steps) eases roughly twice as fast, so the same wall-clock
//! motion settles somewhere different at 30fps than at 60fps. That breaks the determinism contract,
//! which requires the same elapsed time to give the same result however the loop sliced it.
//!
//! ## The fix: decay the gap exponentially in *time*
//!
//! Treat the gap `current - target` as decaying continuously: `gap(t) = gap(0) * 2^(-t / half_life)`.
//! A step of `dt` multiplies the gap by `2^(-dt / half_life)`, so two steps of `dt` multiply by
//! `2^(-2 dt / half_life)` - exactly what one step of `2 dt` does. The eased value is then a function
//! of elapsed time alone, so 30fps and 60fps converge to the same place (to float precision): the
//! frame-rate independence the contract wants. `half_life` is the time for the gap to halve - one
//! intuitive knob, in the same units as `dt` - and the game owns it.
//!
//! This is a first-order approach (the "spring" the camera brief names): it eases in and never
//! overshoots. A second-order spring with its own velocity and overshoot is a different tool the
//! game can build on top; this is the critically-damped-feeling primitive the follow camera rides.
//!
//! Determinism (canon contract): pure arithmetic of the inputs, no wall-clock and no stored state;
//! identical `current`, `target`, `half_life`, and `dt` give an identical result on the same build.

use std::ops::{Add, Mul, Sub};

/// Ease `current` toward `target`, closing the fraction of the remaining gap that `half_life` and
/// `dt` imply.
///
/// Generic over anything that adds, subtracts, and scales by `f32`: `f32` for an arm length or an
/// angle, `Vec3` for a follow position, so the game smooths whatever it holds with one helper.
/// (Smoothing an angle this way assumes the game has already handled wrap-around; the helper is
/// plain arithmetic and has no notion of angles.)
///
/// `half_life` is the time (same units as `dt`) for the gap to halve. Frame-rate independent: the
/// same elapsed `dt` total lands in the same place whether taken in one step or many (to float
/// precision), so a fixed-timestep replay reproduces regardless of step size.
///
/// Total over valid inputs - the degenerate cases the brief calls out are graceful, not errors:
/// `dt <= 0.0` returns `current` unchanged (no time elapsed, nothing to ease); `half_life <= 0.0`
/// returns `target` (an instant snap - zero half-life means no smoothing at all). Both guards also
/// keep the `dt / half_life` ratio clear of `0 / 0`.
pub fn smooth<T>(current: T, target: T, half_life: f32, dt: f32) -> T
where
    T: Add<Output = T> + Sub<Output = T> + Mul<f32, Output = T> + Copy,
{
    if dt <= 0.0 {
        return current;
    }
    if half_life <= 0.0 {
        return target;
    }
    // Fraction of the gap to close this step: 1 - 2^(-dt / half_life). At dt == half_life that is
    // 1 - 0.5 = half the gap, which is the definition of the half-life.
    let blend = 1.0 - (-dt / half_life).exp2();
    current + (target - current) * blend
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn one_half_life_closes_half_the_gap() {
        // dt == half_life: exactly half the remaining distance, by definition.
        let r = smooth(0.0_f32, 10.0, 0.5, 0.5);
        assert!((r - 5.0).abs() < 1e-5, "r = {r}");
    }

    #[test]
    fn frame_rate_independent_within_tolerance() {
        // The same 1.0s of elapsed time, taken as one big step or a hundred small ones, must land in
        // the same place (to float precision) - the property the determinism contract relies on.
        let half_life = 0.3;
        let one = smooth(0.0_f32, 1.0, half_life, 1.0);
        let mut many = 0.0_f32;
        for _ in 0..100 {
            many = smooth(many, 1.0, half_life, 0.01);
        }
        assert!((one - many).abs() < 1e-4, "one = {one}, many = {many}");
    }

    #[test]
    fn approaches_the_target_asymptotically() {
        let mut v = 0.0_f32;
        for _ in 0..1000 {
            v = smooth(v, 100.0, 0.1, 1.0 / 60.0);
        }
        assert!((v - 100.0).abs() < 1e-2, "v = {v}");
    }

    #[test]
    fn zero_dt_is_a_no_op() {
        assert_eq!(smooth(3.0_f32, 9.0, 0.5, 0.0), 3.0);
    }

    #[test]
    fn non_positive_half_life_snaps_to_the_target() {
        assert_eq!(smooth(3.0_f32, 9.0, 0.0, 1.0 / 60.0), 9.0);
    }

    #[test]
    fn smooths_a_vec3_per_component() {
        // Generic over Vec3: at dt == half_life each component covers half its own gap.
        let r = smooth(Vec3::ZERO, Vec3::new(10.0, -4.0, 2.0), 0.25, 0.25);
        assert!((r - Vec3::new(5.0, -2.0, 1.0)).length() < 1e-5, "r = {r:?}");
    }

    #[test]
    fn is_deterministic() {
        let a = smooth(1.0_f32, 7.5, 0.4, 1.0 / 60.0);
        let b = smooth(1.0_f32, 7.5, 0.4, 1.0 / 60.0);
        assert_eq!(a, b);
    }
}
