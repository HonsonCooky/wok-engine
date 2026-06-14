//! Build-fixed constants: the numbers taste decides at compile time, not the feel a play-test
//! retunes.
//!
//! These are gameplay policy, not engine values (HLD principle 5: the engine provides the math, the
//! game owns the numbers). The numbers a feel verdict moves - the run speed, gravity, the jump, and
//! the camera's feel - live in `crate::tuning`, the hot-reloadable record. What stays here is what
//! changing mid-play would make a different game rather than a different feel, split by the domain it
//! belongs to: [`movement`] (the fixed step, the body, the spawn, the vertical-stillness threshold),
//! [`camera`] (the spring-arm probe, the framing offsets, the occlusion fade, the device policy, the
//! orbit and projection limits), and [`debug`] (the display toggles). The split is file
//! organization only - every constant re-exports flat, so consumers keep reading
//! `crate::constants::X`. Each domain's sanity tests pin the structural relationships among the
//! constants that remain; the feel relationships now live in `Tuning::validate`.

mod camera;
mod debug;
mod movement;

pub use camera::*;
pub use debug::*;
pub use movement::*;
