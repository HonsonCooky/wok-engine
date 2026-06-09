//! `LightState`: the named lighting snapshot a scene's `LightStateRef` points at.
//!
//! A `LightState` is a complete, static description of the scene's lighting environment at one
//! instant: the sun, the ambient floor, fog, the gradient sky, and the cel-shading tunables the
//! renderer quantizes into. wok-scene references these states by name (its `LightStateRef`); the
//! state's name is its file stem rather than a field inside the file (see `crate::io`), so the
//! struct itself carries no name. The renderer (wok-render, later) consumes these fields; this
//! crate only models and (de)serializes them.
//!
//! All colours are linear RGB stored as `Vec3` (not gamma-encoded, not clamped to `[0, 1]`: a
//! sun colour may exceed 1 to act as an intensity). The renderer owns tone mapping. Every `Vec3`
//! serializes as a bare `[x, y, z]` array via `crate::serde_vec3`, matching wok-scene's vectors.
//!
//! `lerp` is the building block the keyframed `LightCurve` (see `crate::curve`) samples through.
//! It is a pure function of its inputs, honouring the determinism contract.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The scene's single directional light.
///
/// `direction` is the direction the sunlight travels (from the sun toward the scene), not the
/// direction toward the sun. It is not normalized here; the renderer normalizes on use. `color`
/// is linear RGB and doubles as intensity (values above 1 brighten).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sun {
    #[serde(with = "crate::serde_vec3")]
    pub direction: Vec3,
    #[serde(with = "crate::serde_vec3")]
    pub color: Vec3,
}

/// Distance-based fog. `start` is the distance at which fog begins, `end` the distance at which
/// it fully occludes; both in metres. The HLD ties render distance to `end` and the sky horizon
/// to `color`, but those are render-side reads: here they are plain data.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    #[serde(with = "crate::serde_vec3")]
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
}

/// The parametric gradient sky: a vertical blend from `horizon` colour to `zenith` colour. Sun
/// disc, stars, and cloud plane are render commitments added when wok-render needs them, not part
/// of the lighting data model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkyGradient {
    #[serde(with = "crate::serde_vec3")]
    pub horizon: Vec3,
    #[serde(with = "crate::serde_vec3")]
    pub zenith: Vec3,
}

/// Cel-shading tunables. `band_count` is how many discrete lighting bands the renderer quantizes
/// into (the HLD pins the authored range at 2 to 8; not enforced here). `transition_softness` is
/// the width of the blend between bands, `rim_intensity` the strength of the silhouette rim light.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelParams {
    pub band_count: u32,
    pub transition_softness: f32,
    pub rim_intensity: f32,
}

/// A complete lighting environment for a scene or region at one instant.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightState {
    pub sun: Sun,
    /// The ambient floor colour: the minimum light any surface receives, linear RGB.
    #[serde(with = "crate::serde_vec3")]
    pub ambient: Vec3,
    pub fog: Fog,
    pub sky: SkyGradient,
    pub cel: CelParams,
}

impl LightState {
    /// Linearly interpolate between two states. `s` is the blend fraction, expected in `[0, 1]`
    /// (`0` returns `self`, `1` returns `other`); the curve sampler always passes a value in that
    /// range. Colours and scalar fields blend componentwise; `band_count`, being discrete, blends
    /// by rounding the linear result to the nearest whole band. Pure: identical inputs give an
    /// identical output, bitwise.
    pub fn lerp(&self, other: &LightState, s: f32) -> LightState {
        LightState {
            sun: Sun {
                direction: self.sun.direction.lerp(other.sun.direction, s),
                color: self.sun.color.lerp(other.sun.color, s),
            },
            ambient: self.ambient.lerp(other.ambient, s),
            fog: Fog {
                color: self.fog.color.lerp(other.fog.color, s),
                start: lerp_f32(self.fog.start, other.fog.start, s),
                end: lerp_f32(self.fog.end, other.fog.end, s),
            },
            sky: SkyGradient {
                horizon: self.sky.horizon.lerp(other.sky.horizon, s),
                zenith: self.sky.zenith.lerp(other.sky.zenith, s),
            },
            cel: CelParams {
                band_count: lerp_u32(self.cel.band_count, other.cel.band_count, s),
                transition_softness: lerp_f32(
                    self.cel.transition_softness,
                    other.cel.transition_softness,
                    s,
                ),
                rim_intensity: lerp_f32(self.cel.rim_intensity, other.cel.rim_intensity, s),
            },
        }
    }
}

fn lerp_f32(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

fn lerp_u32(a: u32, b: u32, s: f32) -> u32 {
    lerp_f32(a as f32, b as f32, s).round() as u32
}

/// A neutral daytime default: white sun straight down, dim ambient, mid-distance grey fog, a blue
/// gradient sky, and 4 cel bands. Useful as a starting point for authoring and as the value an
/// empty `LightCurve` samples to. The renderer tunes real values per scene.
impl Default for LightState {
    fn default() -> Self {
        LightState {
            sun: Sun {
                direction: Vec3::new(0.0, -1.0, 0.0),
                color: Vec3::ONE,
            },
            ambient: Vec3::splat(0.1),
            fog: Fog {
                color: Vec3::splat(0.7),
                start: 50.0,
                end: 300.0,
            },
            sky: SkyGradient {
                horizon: Vec3::splat(0.7),
                zenith: Vec3::new(0.3, 0.5, 0.9),
            },
            cel: CelParams {
                band_count: 4,
                transition_softness: 0.05,
                rim_intensity: 0.5,
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn state_a() -> LightState {
        LightState {
            sun: Sun {
                direction: Vec3::new(0.0, -1.0, 0.0),
                color: Vec3::new(1.0, 1.0, 1.0),
            },
            ambient: Vec3::new(0.0, 0.0, 0.0),
            fog: Fog {
                color: Vec3::new(0.0, 0.0, 0.0),
                start: 0.0,
                end: 100.0,
            },
            sky: SkyGradient {
                horizon: Vec3::new(0.0, 0.0, 0.0),
                zenith: Vec3::new(0.0, 0.0, 0.0),
            },
            cel: CelParams {
                band_count: 2,
                transition_softness: 0.0,
                rim_intensity: 0.0,
            },
        }
    }

    fn state_b() -> LightState {
        LightState {
            sun: Sun {
                direction: Vec3::new(0.0, 0.0, -1.0),
                color: Vec3::new(0.0, 0.0, 0.0),
            },
            ambient: Vec3::new(1.0, 1.0, 1.0),
            fog: Fog {
                color: Vec3::new(1.0, 1.0, 1.0),
                start: 10.0,
                end: 200.0,
            },
            sky: SkyGradient {
                horizon: Vec3::new(1.0, 1.0, 1.0),
                zenith: Vec3::new(1.0, 1.0, 1.0),
            },
            cel: CelParams {
                band_count: 8,
                transition_softness: 1.0,
                rim_intensity: 1.0,
            },
        }
    }

    #[test]
    fn lerp_at_zero_returns_self() {
        assert_eq!(state_a().lerp(&state_b(), 0.0), state_a());
    }

    #[test]
    fn lerp_at_one_returns_other() {
        assert_eq!(state_a().lerp(&state_b(), 1.0), state_b());
    }

    #[test]
    fn lerp_midpoint_blends_each_field() {
        let m = state_a().lerp(&state_b(), 0.5);
        assert_eq!(m.ambient, Vec3::splat(0.5));
        assert_eq!(m.fog.start, 5.0);
        assert_eq!(m.fog.end, 150.0);
        assert_eq!(m.sky.horizon, Vec3::splat(0.5));
        assert_eq!(m.cel.transition_softness, 0.5);
        assert_eq!(m.cel.rim_intensity, 0.5);
        // band_count 2 -> 8 at 0.5 rounds to 5.
        assert_eq!(m.cel.band_count, 5);
    }

    #[test]
    fn band_count_rounds_to_nearest() {
        // 2 -> 3 at 0.4 is 2.4 -> rounds to 2; at 0.6 is 2.6 -> rounds to 3.
        let a = LightState { cel: CelParams { band_count: 2, ..state_a().cel }, ..state_a() };
        let b = LightState { cel: CelParams { band_count: 3, ..state_b().cel }, ..state_b() };
        assert_eq!(a.lerp(&b, 0.4).cel.band_count, 2);
        assert_eq!(a.lerp(&b, 0.6).cel.band_count, 3);
    }

    #[test]
    fn lerp_is_deterministic() {
        let first = state_a().lerp(&state_b(), 0.37);
        let second = state_a().lerp(&state_b(), 0.37);
        assert_eq!(first, second);
    }

    #[test]
    fn serde_round_trips() {
        let s = state_b();
        let json = serde_json::to_string(&s).unwrap();
        let back: LightState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn serde_emits_grouped_shape() {
        let json = serde_json::to_string(&LightState::default()).unwrap();
        // Sub-structs are nested objects; colours are bare arrays.
        assert!(json.contains(r#""sun":{"direction":[0.0,-1.0,0.0],"color":[1.0,1.0,1.0]}"#));
        assert!(json.contains(r#""ambient":[0.1,0.1,0.1]"#));
        assert!(json.contains(r#""cel":{"band_count":4"#));
    }

    #[test]
    fn serde_rejects_unknown_fields() {
        let json = r#"{
            "sun": {"direction":[0,0,0],"color":[0,0,0]},
            "ambient":[0,0,0],
            "fog":{"color":[0,0,0],"start":0,"end":0},
            "sky":{"horizon":[0,0,0],"zenith":[0,0,0]},
            "cel":{"band_count":2,"transition_softness":0,"rim_intensity":0},
            "bogus": 1
        }"#;
        assert!(serde_json::from_str::<LightState>(json).is_err());
    }
}
