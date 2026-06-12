//! Gameplay tuning constants: the numbers that make taste feel like taste.
//!
//! These are gameplay policy, not engine values (HLD principle 5: the engine provides the math, the
//! game owns the numbers). Everything a designer would reach for to retune the demo lives here, in
//! one namespace, split by the domain a retune verdict talks about: [`movement`] (the fixed step,
//! the body, locomotion, and the jump), [`camera`] (the follow camera's geometry, easing, and look
//! input), and [`debug`] (the display toggles). The split is file organization only - every
//! constant re-exports flat, so consumers keep reading `crate::constants::X`. Each domain's sanity
//! tests pin the structural relationships between its values (a body taller than it is wide, a
//! jump that clears something, a boom longer than its probe), so a retune that breaks the demo's
//! assumptions fails in `cargo test` rather than in play.

mod camera;
mod debug;
mod movement;

pub use camera::*;
pub use debug::*;
pub use movement::*;
