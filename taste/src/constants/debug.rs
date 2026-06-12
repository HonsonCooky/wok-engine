//! Display toggles: the opt-in diagnostic and the permanent gameplay cue, together because both
//! are switches over the line pass rather than tuning values.

/// Draw the ground-truth marker: a bright quad at the sampled terrain height under the player,
/// composed through the same chunk-origin path as the terrain mesh. For the floating-at-rest
/// diagnosis: if the marker lies on the rendered terrain while the bean floats, the gap is in the
/// rest math; if the marker itself disagrees with the rendered terrain, sampling and mesh disagree.
/// Off by default: the shadow map carries the grounding cue in normal play now, so the marker
/// retires to an opt-in diagnostic.
pub const DEBUG_GROUND_MARKER: bool = false;

/// Draw a small cross at the camera's look-at point (the look-ahead target), through the line
/// pass, depth-tested like any world-anchored cue. The reticle is the look-target indicator - a
/// gameplay cue showing the player exactly where the view leads, which the look-ahead framing
/// makes otherwise invisible - not a tuning aid, so it stays on in normal play rather than
/// retiring with the framing work that introduced it.
pub const SHOW_RETICLE: bool = true;
