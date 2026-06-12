//! Build-fixed constants: the numbers taste decides at compile time, not the feel a play-test
//! retunes.
//!
//! These are gameplay policy, not engine values (HLD principle 5: the engine provides the math, the
//! game owns the numbers). The numbers a feel verdict moves live - locomotion, the jump, air
//! steering, the wall policies, the walkable limit, the downhill glue, and the camera's feel - moved
//! to `crate::tuning`, the hot-reloadable record. What stays here is what changing mid-play would
//! make a different game rather than a different feel, split by the domain it belongs to:
//! [`movement`] (the fixed step, the body, the step-up, the spawn), [`camera`] (the spring-arm
//! probe, the framing offsets, the occlusion fade, the device policy, the orbit and projection
//! limits), and [`debug`] (the display toggles). The split is file organization only - every
//! constant re-exports flat, so consumers keep reading `crate::constants::X`. Each domain's sanity
//! tests pin the structural relationships among the constants that remain; the feel relationships
//! that used to be pinned here against the moved values now live in `Tuning::validate`.

mod camera;
mod debug;
mod movement;

pub use camera::*;
pub use debug::*;
pub use movement::*;
