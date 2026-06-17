//! The editor's interaction mode: which keyboard-and-mouse grammar is live.
//!
//! The editor is a modal manipulator, not a flight simulator (see designs/editor-design.md, Input):
//! the selection is the cursor, and most work is acting on it. Modes share one key set so a
//! one-handed scheme can cover everything, and the same physical keys mean different things in each:
//!
//! - [`Mode::Object`] (the default): the resting mode. The selection is the cursor and the left hand
//!   operates on it; the camera does not move on its own (it holds wherever free-fly last left it).
//!   The home-row verbs and the floating inspector that act on the selection return with picking.
//! - [`Mode::FreeFly`]: a god-cam to get around, where WASD pans the yaw-only heading, Q/E is a
//!   world-up elevator, and right-drag looks (`crate::camera`, `crate::input`).
//!
//! Only the camera and the home-row keys are modal; mouse selection works the same in both. The mode
//! is interaction state, never authored data: it lives on the app and is toggled in place by the
//! input routing (backtick), not through the action layer.

/// Which interaction grammar the editor is in. `Object` is the default - selection-centric and
/// camera-at-rest; `FreeFly` is the camera-centric roam toggled on demand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Selection-centric and at rest: the camera holds its pose and the home row acts on the
    /// selection (the verbs return with picking). The editor's resting state.
    #[default]
    Object,
    /// Camera-centric free flight. WASD pans, Q/E changes altitude, and right-drag looks, to get
    /// around; toggled back to [`Mode::Object`] when the roam is done.
    FreeFly,
}

impl Mode {
    /// Flip to the other mode - the whole of the toggle.
    pub fn toggled(self) -> Mode {
        match self {
            Mode::Object => Mode::FreeFly,
            Mode::FreeFly => Mode::Object,
        }
    }

    /// The mode's name for the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Object => "object",
            Mode::FreeFly => "free-fly",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_is_the_default() {
        assert_eq!(Mode::default(), Mode::Object);
    }

    #[test]
    fn toggling_round_trips_between_the_two_modes() {
        assert_eq!(Mode::Object.toggled(), Mode::FreeFly);
        assert_eq!(Mode::FreeFly.toggled(), Mode::Object);
        assert_eq!(Mode::Object.toggled().toggled(), Mode::Object, "two toggles return to the start");
    }
}
