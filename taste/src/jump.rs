//! The jump latch: a frame-edge press delivered to the fixed steps without loss.
//!
//! A jump press is an edge raised on exactly one rendered frame, but the simulation only looks at
//! input inside fixed steps, and the two rates do not line up. A frame whose banked time covers zero
//! fixed steps (`crate::clock`) runs no step at all, so an edge raised on it would vanish before any
//! step could read it; the latch holds the press until the next step runs and takes it.
//!
//! Consuming clears the latch, so one press is exactly one jump even across a multi-step catch-up
//! frame: the first step takes the press, the rest see nothing. The step itself decides whether a
//! jump is actually available (its counter, `Player::jumps_remaining`); the latch's only job is to
//! carry the press across the frame/step seam. There is no buffering - a press that reaches a step
//! with no jump left is simply spent, the deliberately simple model.

/// A pending jump press, or nothing pending.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JumpLatch {
    pending: bool,
}

impl JumpLatch {
    pub fn new() -> JumpLatch {
        JumpLatch::default()
    }

    /// Record this frame's press edge. A second press before a step consumes the first is the same
    /// single jump asked for again.
    pub fn press(&mut self) {
        self.pending = true;
    }

    /// Take the pending press, once per fixed step: return whether one was waiting and clear it. A
    /// press raised on a zero-step frame still reaches the next step this way, and one press yields
    /// at most one jump across a catch-up burst.
    pub fn consume(&mut self) -> bool {
        let pending = self.pending;
        self.pending = false;
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_press_means_no_jump() {
        let mut latch = JumpLatch::new();
        assert!(!latch.consume());
    }

    #[test]
    fn a_press_on_a_zero_step_frame_fires_on_the_next_step() {
        // The frame that raised the edge ran zero fixed steps (nothing consumed); the next frame's
        // first step must still see the press.
        let mut latch = JumpLatch::new();
        latch.press();
        assert!(latch.consume(), "the latched press must fire on the first step that runs");
    }

    #[test]
    fn one_press_never_produces_two_jumps() {
        // The catch-up-burst guarantee: however many steps one frame runs, the press fires on the
        // first and is spent.
        let mut latch = JumpLatch::new();
        latch.press();
        assert!(latch.consume());
        for _ in 0..8 {
            assert!(!latch.consume(), "a consumed press must stay consumed");
        }
    }

    #[test]
    fn a_second_press_jumps_again() {
        // Consuming clears the press, not the latch: the next edge works as the first did.
        let mut latch = JumpLatch::new();
        latch.press();
        assert!(latch.consume());
        latch.press();
        assert!(latch.consume());
    }
}
