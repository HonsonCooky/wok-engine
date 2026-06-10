//! The fixed-timestep accumulator: variable frame time in, whole simulation steps out.
//!
//! Rendering runs as fast as the platform delivers frames; the simulation advances only in fixed
//! `dt` steps (the determinism contract's "time passed as dt", never a wall-clock delta inside the
//! step). The accumulator is the standard bridge between the two rates: each frame's elapsed time is
//! banked, and the loop withdraws as many whole steps as the bank covers, carrying the remainder so
//! no time is lost to rounding. A clamp bounds the withdrawal per frame: after a long stall the bank
//! holds more debt than can be repaid without causing the next stall, so the excess is forgiven and
//! the game slows down instead of spiraling (each catch-up burst creating more debt than it clears).
//!
//! Pure state-plus-arithmetic, no clock reads: the caller supplies the frame time, so the unit tests
//! drive it with exact numbers.

/// Banks frame time and pays it out in whole fixed steps.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock {
    /// The fixed step size, in seconds.
    dt: f32,
    /// Most steps one call to [`FixedClock::advance`] may return.
    max_steps: u32,
    /// Banked time not yet consumed by a step, in seconds; always in `0.0..dt` between calls
    /// (except right after a clamp, when it is exactly zero).
    accumulator: f32,
}

impl FixedClock {
    pub fn new(dt: f32, max_steps: u32) -> Self {
        FixedClock { dt, max_steps, accumulator: 0.0 }
    }

    /// How far the banked time has progressed through the next fixed step, in `0.0..1.0`: the
    /// render-side interpolation factor between the previous and current simulation states. Read it
    /// after [`FixedClock::advance`], whose contract keeps the bank below one step.
    pub fn alpha(&self) -> f32 {
        self.accumulator / self.dt
    }

    /// Bank `frame_dt` seconds and return how many fixed steps the simulation should run now.
    ///
    /// At most `max_steps` are returned; when the bank holds more than that, the excess is dropped
    /// (the anti-spiral clamp). A negative `frame_dt` is treated as zero: time does not run
    /// backwards, and the platform's first frame can report a degenerate delta.
    pub fn advance(&mut self, frame_dt: f32) -> u32 {
        self.accumulator += frame_dt.max(0.0);
        let steps = (self.accumulator / self.dt) as u32;
        if steps > self.max_steps {
            self.accumulator = 0.0;
            self.max_steps
        } else {
            self.accumulator -= steps as f32 * self.dt;
            steps
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    /// A clock with binary-exact numbers (dt 0.25 and frame times that are sums of powers of two),
    /// so every test below is exact arithmetic rather than float luck. The demo's 1/60 dt changes
    /// the values, not the behavior under test.
    fn clock() -> FixedClock {
        FixedClock::new(0.25, 4)
    }

    #[test]
    fn one_full_frame_buys_one_step() {
        let mut c = clock();
        assert_eq!(c.advance(0.25), 1);
    }

    #[test]
    fn short_frames_accumulate_until_a_step_is_covered() {
        // Two half-step frames: the first banks, the second completes the step.
        let mut c = clock();
        assert_eq!(c.advance(0.125), 0);
        assert_eq!(c.advance(0.125), 1);
    }

    #[test]
    fn a_long_frame_buys_several_steps_and_carries_the_remainder() {
        // 3.5 steps of time: three steps now, the half step stays banked and one more half-step
        // frame completes the fourth.
        let mut c = clock();
        assert_eq!(c.advance(0.875), 3);
        assert_eq!(c.advance(0.125), 1);
    }

    #[test]
    fn a_stall_clamps_to_max_steps_and_forgives_the_debt() {
        // A 40-step stall: the clamp pays 4 and drops the rest, so the very next ordinary frame is
        // back to one step, not another burst.
        let mut c = clock();
        assert_eq!(c.advance(10.0), 4);
        assert_eq!(c.advance(0.25), 1);
    }

    #[test]
    fn exactly_max_steps_is_paid_in_full_not_clamped() {
        // Exactly max_steps of banked time is payable without the clamp; nothing is forgiven.
        let mut c = clock();
        assert_eq!(c.advance(1.0), 4);
        assert_eq!(c.advance(0.25), 1);
    }

    #[test]
    fn alpha_is_the_banked_fraction_of_the_next_step() {
        let mut c = clock();
        assert_eq!(c.alpha(), 0.0, "a fresh clock has banked nothing");
        c.advance(0.125);
        assert_eq!(c.alpha(), 0.5, "half a step banked is alpha one half");
        c.advance(0.25);
        assert_eq!(c.alpha(), 0.5, "consuming the whole step leaves the same remainder");
    }

    #[test]
    fn alpha_resets_with_the_clamp() {
        // The clamp forgives the debt, so the draw lands exactly on the last computed state.
        let mut c = clock();
        c.advance(10.0);
        assert_eq!(c.alpha(), 0.0);
    }

    #[test]
    fn negative_frame_time_is_ignored() {
        let mut c = clock();
        assert_eq!(c.advance(-1.0), 0);
        assert_eq!(c.advance(0.25), 1);
    }
}
