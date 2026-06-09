//! `LightCurve`: keyframed animation over `LightState`, with linear interpolation.
//!
//! A curve drives lighting over time: a day/night cycle, a scripted dusk, a flickering storm. It
//! is a list of `(time, LightState)` keyframes plus a `looping` flag, and `sample(t)` evaluates it
//! at an arbitrary time. The evaluator is pure: `t` is an input, never a clock read, so the same
//! `t` always yields the same state (the determinism contract). Driving `t` from a wall clock is
//! the game's job, not this crate's.
//!
//! Keyframes are ordered by `time`, strictly increasing; `crate::io::load_light_curve` enforces
//! that on load. `sample` assumes it and guards against degenerate input so it never panics.
//!
//! Interpolation is linear between bracketing keyframes (see `LightState::lerp`). Outside the
//! keyframe span the behaviour depends on `looping`:
//!
//! - Not looping: `t` is clamped, so before the first keyframe yields the first state and after
//!   the last yields the last (a held value at each end).
//! - Looping: `t` wraps modulo the span `[first.time, last.time]` back to the start. The loop is
//!   seamless only if the author makes the first and last keyframes carry the same `LightState`
//!   (e.g. midnight at both ends of a day cycle); there is no synthesized segment bridging the
//!   last keyframe back to the first, because its duration is not part of the data.

use serde::{Deserialize, Serialize};

use crate::state::LightState;

/// One keyframe: a `LightState` pinned at a `time`. `time` is in whatever unit the game advances
/// `sample`'s argument in (seconds, hours of a day cycle); the curve does not interpret it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    pub time: f32,
    pub state: LightState,
}

/// A keyframed animation over `LightState`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightCurve {
    pub keyframes: Vec<Keyframe>,
    /// When true, `sample` wraps `t` into the keyframe span instead of clamping at the ends.
    #[serde(default)]
    pub looping: bool,
}

impl LightCurve {
    /// Evaluate the curve at time `t`, returning the interpolated `LightState`.
    ///
    /// Pure and total: never panics, never reads a clock. An empty curve (only reachable by direct
    /// construction; the loader rejects it) returns `LightState::default()`. A single keyframe
    /// returns that keyframe's state for every `t`.
    pub fn sample(&self, t: f32) -> LightState {
        let frames = &self.keyframes;
        match frames.len() {
            0 => LightState::default(),
            1 => frames[0].state,
            _ => {
                let te = self.effective_time(t);
                self.interpolate_at(te)
            }
        }
    }

    /// Map the requested time onto the keyframe span: clamp when not looping, wrap when looping.
    /// Assumes at least two keyframes (callers in `sample` guarantee it).
    fn effective_time(&self, t: f32) -> f32 {
        let frames = &self.keyframes;
        let first = frames[0].time;
        let last = frames[frames.len() - 1].time;
        let span = last - first;
        if span <= 0.0 {
            return first;
        }
        if self.looping {
            first + (t - first).rem_euclid(span)
        } else {
            t.clamp(first, last)
        }
    }

    /// Linearly interpolate at an effective time already mapped into `[first, last]`.
    fn interpolate_at(&self, te: f32) -> LightState {
        let frames = &self.keyframes;
        // Find the segment [cur, next] that brackets `te`. Linear scan: keyframe counts are small
        // (a day cycle is a handful), and a scan keeps the order explicit for determinism.
        for pair in frames.windows(2) {
            let cur = &pair[0];
            let next = &pair[1];
            if te <= next.time {
                let seg = next.time - cur.time;
                if seg <= 0.0 {
                    return cur.state;
                }
                let s = (te - cur.time) / seg;
                return cur.state.lerp(&next.state, s);
            }
        }
        // te is at or past the last keyframe (clamp/wrap keep it in range, but float rounding at
        // the upper bound can land here): hold the final state.
        frames[frames.len() - 1].state
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::state::{CelParams, Fog, SkyGradient, Sun};
    use glam::Vec3;

    // A state whose ambient colour encodes a scalar marker, so interpolation is easy to read.
    fn marker_state(v: f32) -> LightState {
        LightState {
            sun: Sun { direction: Vec3::new(0.0, -1.0, 0.0), color: Vec3::splat(v) },
            ambient: Vec3::splat(v),
            fog: Fog { color: Vec3::splat(v), start: v, end: v },
            sky: SkyGradient { horizon: Vec3::splat(v), zenith: Vec3::splat(v) },
            cel: CelParams { band_count: 4, transition_softness: v, rim_intensity: v },
        }
    }

    fn three_frame_curve(looping: bool) -> LightCurve {
        LightCurve {
            keyframes: vec![
                Keyframe { time: 0.0, state: marker_state(0.0) },
                Keyframe { time: 10.0, state: marker_state(1.0) },
                Keyframe { time: 20.0, state: marker_state(0.0) },
            ],
            looping,
        }
    }

    #[test]
    fn sample_at_start_returns_first() {
        let c = three_frame_curve(false);
        assert_eq!(c.sample(0.0).ambient, Vec3::splat(0.0));
    }

    #[test]
    fn sample_mid_segment_interpolates() {
        let c = three_frame_curve(false);
        // Halfway through the first segment (t=5 of 0..10): the marker 0 -> 1 gives 0.5 in every
        // marker-backed field (fog.start carries the marker value, not the time).
        assert_eq!(c.sample(5.0).ambient, Vec3::splat(0.5));
        assert_eq!(c.sample(5.0).fog.start, 0.5);
    }

    #[test]
    fn sample_on_keyframe_returns_that_frame() {
        let c = three_frame_curve(false);
        assert_eq!(c.sample(10.0).ambient, Vec3::splat(1.0));
    }

    #[test]
    fn sample_at_end_returns_last() {
        let c = three_frame_curve(false);
        assert_eq!(c.sample(20.0).ambient, Vec3::splat(0.0));
    }

    #[test]
    fn non_looping_clamps_below_and_above() {
        let c = three_frame_curve(false);
        assert_eq!(c.sample(-5.0).ambient, Vec3::splat(0.0)); // held at first
        assert_eq!(c.sample(100.0).ambient, Vec3::splat(0.0)); // held at last
    }

    #[test]
    fn looping_wraps_past_the_end() {
        let c = three_frame_curve(true);
        // Span is 20. t=25 wraps to 5, which is mid first segment -> 0.5.
        assert_eq!(c.sample(25.0).ambient, Vec3::splat(0.5));
        // t=20 wraps to 0 -> first state.
        assert_eq!(c.sample(20.0).ambient, Vec3::splat(0.0));
    }

    #[test]
    fn looping_wraps_negative() {
        let c = three_frame_curve(true);
        // t=-15 wraps by span 20 to 5 -> 0.5.
        assert_eq!(c.sample(-15.0).ambient, Vec3::splat(0.5));
    }

    #[test]
    fn empty_curve_samples_default() {
        let c = LightCurve { keyframes: vec![], looping: false };
        assert_eq!(c.sample(3.0), LightState::default());
    }

    #[test]
    fn single_keyframe_holds_for_all_t() {
        let c = LightCurve {
            keyframes: vec![Keyframe { time: 7.0, state: marker_state(0.3) }],
            looping: false,
        };
        assert_eq!(c.sample(-100.0).ambient, Vec3::splat(0.3));
        assert_eq!(c.sample(7.0).ambient, Vec3::splat(0.3));
        assert_eq!(c.sample(999.0).ambient, Vec3::splat(0.3));
    }

    #[test]
    fn sample_is_deterministic() {
        let c = three_frame_curve(true);
        assert_eq!(c.sample(13.7), c.sample(13.7));
    }

    #[test]
    fn serde_round_trips() {
        let c = three_frame_curve(true);
        let json = serde_json::to_string(&c).unwrap();
        let back: LightCurve = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn looping_defaults_to_false_when_absent() {
        let json = r#"{"keyframes":[{"time":0.0,"state":"PLACEHOLDER"}]}"#
            .replace("\"PLACEHOLDER\"", &serde_json::to_string(&marker_state(0.0)).unwrap());
        let c: LightCurve = serde_json::from_str(&json).unwrap();
        assert!(!c.looping);
    }
}
