//! `PointLight`: a single dynamic point light's data.
//!
//! This is the data type only. The engine-owned pool that budgets and manages a set of these at
//! runtime comes later (alongside wok-render), as does any falloff or shadow behaviour. Here a
//! point light is just its authored parameters: where it is, what colour it is, how far it reaches,
//! and how bright it is. `color` is linear RGB and serializes as a bare `[r, g, b]` array, like
//! every other colour in this crate.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// A point light's parameters. `radius` is the reach in metres beyond which the light contributes
/// nothing; `intensity` is a scalar multiplier on `color`. The exact falloff curve is a render
/// decision deferred to the pool, not fixed by this type.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointLight {
    #[serde(with = "crate::serde_vec3")]
    pub position: Vec3,
    #[serde(with = "crate::serde_vec3")]
    pub color: Vec3,
    pub radius: f32,
    pub intensity: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trips() {
        let p = PointLight {
            position: Vec3::new(1.0, 2.0, 3.0),
            color: Vec3::new(1.0, 0.5, 0.25),
            radius: 8.0,
            intensity: 2.5,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PointLight = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn serde_shape_uses_bare_color_arrays() {
        let p = PointLight {
            position: Vec3::new(0.0, 0.0, 0.0),
            color: Vec3::new(1.0, 1.0, 1.0),
            radius: 1.0,
            intensity: 1.0,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"{"position":[0.0,0.0,0.0],"color":[1.0,1.0,1.0],"radius":1.0,"intensity":1.0}"#
        );
    }

    #[test]
    fn serde_rejects_unknown_fields() {
        let json = r#"{"position":[0,0,0],"color":[1,1,1],"radius":1,"intensity":1,"x":0}"#;
        assert!(serde_json::from_str::<PointLight>(json).is_err());
    }
}
