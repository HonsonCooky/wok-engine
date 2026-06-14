//! The editor's interaction mode: which keyboard-and-mouse grammar is live.
//!
//! The editor is a modal manipulator, not a flight simulator (see designs/editor-design.md, Input):
//! the selection is the cursor, and most work is acting on it. Two modes share one key set so a
//! one-handed scheme can cover everything, and the same physical keys mean different things in each:
//!
//! - [`Mode::Object`] (the default): the camera locks to and orbits the selection, and the home row
//!   operates on it (the object verbs are the next slice; the row is inert for now).
//! - [`Mode::FreeFly`]: a first-person fly to get around, where the home row drives the camera and
//!   the mouse looks - today's camera, now gated behind this mode.
//!
//! Only the camera and the home-row keys are modal; mouse selection (click, Ctrl+click, marquee,
//! drag-to-reposition) works the same in both. The mode is interaction state, never authored data:
//! it lives in `UiState` and is toggled in place by the input routing, not through the action layer.

/// Which interaction grammar the editor is in. `Object` is the default - selection-centric, the
/// camera framing the selection; `FreeFly` is the camera-centric roam toggled on demand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Selection-centric. The camera locks to the selection (frames it, then orbits the centroid),
    /// and the home row will act on it (next slice). The editor's resting state.
    #[default]
    Object,
    /// Camera-centric free flight. The home row flies and the mouse looks, to get around; toggled
    /// back to [`Mode::Object`] when the roam is done.
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
