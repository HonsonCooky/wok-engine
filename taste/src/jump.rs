//! The jump latch: frame-edge presses delivered to fixed steps without loss.
//!
//! A jump press is an edge raised on exactly one rendered frame, but the simulation only looks at
//! input inside fixed steps, and the two rates do not line up. Two gaps eat presses without this:
//!
//! - A frame whose banked time covers zero fixed steps (`crate::clock`) runs no step at all, so an
//!   edge raised on it would vanish before any step could consume it. The latch holds the press
//!   until a step does.
//! - A press a moment before landing reaches a step that finds the player unable to jump and does
//!   nothing; the player, who timed the press against the visible landing, feels a swallowed
//!   input. The buffer keeps a press alive `JUMP_BUFFER_S` of simulation time, so it fires on the
//!   landing step instead.
//!
//! The latch asks "can this step jump", not "is this step grounded": with the double jump, an
//! airborne step with an air jump in hand consumes the press too, so a press latched on a
//! zero-step frame near the apex fires the air jump on the next step instead of waiting for
//! ground. Once the air jump is spent, presses buffer for the landing exactly as before.
//!
//! Consuming clears the latch, so one press is exactly one jump (ground or air): a multi-step
//! catch-up frame cannot bounce twice on one press (the guarantee the old first-step-only
//! delivery gave, kept). Ages advance in simulation time (one `SIM_DT` per waiting step), not
//! wall time, so the replay contract is untouched: scripted inputs through the latch reproduce
//! bitwise.

use crate::constants::{JUMP_BUFFER_S, SIM_DT};

/// A pending jump press: its age in simulation seconds, or nothing pending.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JumpLatch {
    age: Option<f32>,
}

impl JumpLatch {
    pub fn new() -> JumpLatch {
        JumpLatch::default()
    }

    /// Record this frame's press edge. A new press supersedes any older pending one; both were
    /// asking for the same single jump.
    pub fn press(&mut self) {
        self.age = Some(0.0);
    }

    /// Ask, once per fixed step, whether this step should jump. `can_jump` is whether the player
    /// at step entry has a jump to give: grounded, or airborne with an air jump remaining (the
    /// state the step's own jump check reads, so the latch and the step agree). A pending press
    /// fires and is consumed on the first step that can jump; a step that cannot ages it by
    /// `SIM_DT` and drops it once it is older than `JUMP_BUFFER_S`.
    pub fn consume(&mut self, can_jump: bool) -> bool {
        let Some(age) = self.age else { return false };
        if can_jump {
            self.age = None;
            return true;
        }
        let aged = age + SIM_DT;
        self.age = if aged <= JUMP_BUFFER_S { Some(aged) } else { None };
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Airborne steps safely inside the buffer window (80% of it: 4 at the defaults), and safely
    /// past it (150%: 9 at the defaults). Deliberately clear of the exact boundary, where float
    /// accumulation of SIM_DT decides the outcome and the test would pin luck, not behavior.
    fn steps_inside_buffer() -> u32 {
        (JUMP_BUFFER_S * 0.8 / SIM_DT) as u32
    }

    fn steps_past_buffer() -> u32 {
        (JUMP_BUFFER_S * 1.5 / SIM_DT).ceil() as u32
    }

    #[test]
    fn no_press_means_no_jump() {
        let mut latch = JumpLatch::new();
        assert!(!latch.consume(true));
        assert!(!latch.consume(false));
    }

    #[test]
    fn a_press_on_a_zero_step_frame_fires_on_the_next_step() {
        // The frame that raised the edge ran zero fixed steps (nothing consumed); the next frame's
        // first step must still see the press.
        let mut latch = JumpLatch::new();
        latch.press();
        assert!(latch.consume(true), "the latched press must fire on the first step that runs");
    }

    #[test]
    fn a_press_within_the_buffer_before_landing_fires_at_landing() {
        // Pressed while airborne with the air jump already spent (can_jump false), inside the
        // buffer window before the landing step: every waiting step ages it, the landing fires.
        let mut latch = JumpLatch::new();
        latch.press();
        for i in 0..steps_inside_buffer() {
            assert!(!latch.consume(false), "spent-air step {i} must not jump");
        }
        assert!(latch.consume(true), "the buffered press must fire on the landing step");
    }

    #[test]
    fn a_buffered_press_near_apex_fires_the_air_jump() {
        // Pressed on a frame near the apex that ran zero fixed steps: the latch holds it, the
        // next step is airborne with the air jump in hand (can_jump true), so the press fires the
        // air jump through the very same latch the ground jump uses - a zero-step frame cannot
        // eat the double jump. Driven through the real step so the wiring matches the app's.
        use crate::constants::{AIR_JUMP_SCALE, GRAVITY, JUMP_VELOCITY};
        use crate::sim::{self, Player, StepInput};
        use crate::world::{ChunkTerrain, World};
        use glam::Vec3;
        use wok_physics::Motion;
        use wok_scene::{CHUNK_GRID_LEN, Heightmap, SurfaceTag};

        let raw = Heightmap::meters_to_raw(2.0);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        let world = World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }] };
        // Near the apex of a jump: airborne, barely rising, the air jump unspent.
        let p = Player {
            motion: Motion { position: Vec3::new(64.0, 20.0, 64.0), velocity: Vec3::new(0.0, 0.2, 0.0) },
            grounded: false,
            air_jumps: crate::constants::AIR_JUMPS,
        };

        let mut latch = JumpLatch::new();
        latch.press(); // the frame that raised this edge ran zero fixed steps
        let fired = latch.consume(p.grounded || p.air_jumps > 0);
        assert!(fired, "an airborne step with an air jump in hand consumes the press");
        let next = sim::step(p, StepInput { move_dir: Vec3::ZERO, jump: fired }, &world);
        assert!(
            (next.motion.velocity.y - (JUMP_VELOCITY * AIR_JUMP_SCALE + GRAVITY.y * crate::constants::SIM_DT)).abs()
                < 1e-5,
            "the latched press must fire the air jump: vy = {}",
            next.motion.velocity.y
        );
        assert!(!latch.consume(next.grounded || next.air_jumps > 0), "the press is spent: one press, one jump");
    }

    #[test]
    fn a_press_older_than_the_buffer_does_not_fire() {
        let mut latch = JumpLatch::new();
        latch.press();
        for _ in 0..steps_past_buffer() {
            assert!(!latch.consume(false));
        }
        assert!(!latch.consume(true), "a stale press must not fire on a later landing");
    }

    #[test]
    fn one_press_never_produces_two_jumps() {
        // The catch-up-burst guarantee: however many grounded steps one frame runs, the press
        // fires on the first and is spent.
        let mut latch = JumpLatch::new();
        latch.press();
        assert!(latch.consume(true));
        for _ in 0..8 {
            assert!(!latch.consume(true), "a consumed press must stay consumed");
        }
    }

    #[test]
    fn a_second_press_jumps_again() {
        // Consuming clears the press, not the latch: the next edge works as the first did.
        let mut latch = JumpLatch::new();
        latch.press();
        assert!(latch.consume(true));
        latch.press();
        assert!(latch.consume(true));
    }
}
