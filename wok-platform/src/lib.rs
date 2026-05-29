//! Platform substrate (layer 1) of the wok-engine: window, GPU, audio, input, frame loop, and
//! OS theme. Provides the cross-platform foundation that the wok-* crates and the game build on.

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
