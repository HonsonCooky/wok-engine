//! Platform substrate (layer 1) of the wok-engine: window, GPU, audio, input, frame loop, and
//! OS theme. Provides the cross-platform foundation that the wok-* crates and the game build on.
//!
//! Testing boundary: the window/GPU/audio shell (`platform`, `gfx`, `audio`) is exempt from unit
//! testing by nature - its behavior IS the OS, driver, and device interaction, which only a real
//! window and a live adapter exercise, and the applications smoke it on every run. Unit tests
//! cover the pure logic the crate does have: the input collector's per-frame edge/held set
//! transitions (`input`). The gamepad sets share that transition shape but their ids can only be
//! minted by a live gilrs session, so they are exercised through the applications as well.

pub mod audio;
pub mod gfx;
pub mod input;
pub mod platform;

pub use platform::{App, Desc, FrameCtx, Platform, RumbleRequest, run};

/// Common imports for consumers.
pub mod prelude {
    pub use crate::gfx;
    pub use crate::input::{GamepadState, InputState};
    pub use crate::platform::{App, Desc, FrameCtx, Platform};
    pub use crate::run;
    pub use winit::event::MouseButton;
    pub use winit::keyboard::{Key, NamedKey};
}

pub use bytemuck;
pub use cpal;
pub use gilrs;
pub use wgpu;
pub use winit;
