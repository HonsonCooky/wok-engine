//! Math primitives used across the authored data: `Transform` and `Aabb`.
//!
//! Relocated from `wok_platform::math` because they describe scene-authored data, not the platform
//! substrate; wok-platform is the cross-platform window/GPU/audio/input layer and should not own
//! types that only the engine's data layer cares about.

pub use glam::{Mat4, Quat, Vec2, Vec3, Vec4, vec2, vec3, vec4};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Affine transform: scale, then rotate, then translate. No shear.
///
/// `to_mat4` returns `T * R * S` so that composing `parent.to_mat4() * child.to_mat4()`
/// applies the child's transform first in its parent's local space. The wok-scene part-2
/// slicer relies on this convention: it computes `placement.to_mat4() * shape.to_mat4()`
/// to lift a prefab-local shape into world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

// Serde shape: { "pos": [x, y, z], "rot": [x, y, z, w], "scale": [x, y, z] }.
// `rot` and `scale` are skipped when they equal the identity defaults, so a translation-only
// transform serializes as just { "pos": [...] }. `pos` is always present.

const IDENTITY_QUAT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const ONE_VEC3: [f32; 3] = [1.0, 1.0, 1.0];

fn is_identity_quat(v: &[f32; 4]) -> bool {
    *v == IDENTITY_QUAT
}

fn is_one_vec3(v: &[f32; 3]) -> bool {
    *v == ONE_VEC3
}

fn default_rot() -> [f32; 4] {
    IDENTITY_QUAT
}

fn default_scale() -> [f32; 3] {
    ONE_VEC3
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformRepr {
    pos: [f32; 3],
    #[serde(default = "default_rot", skip_serializing_if = "is_identity_quat")]
    rot: [f32; 4],
    #[serde(default = "default_scale", skip_serializing_if = "is_one_vec3")]
    scale: [f32; 3],
}

impl Serialize for Transform {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TransformRepr {
            pos: self.translation.to_array(),
            rot: self.rotation.to_array(),
            scale: self.scale.to_array(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Transform {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = TransformRepr::deserialize(deserializer)?;
        Ok(Transform {
            translation: Vec3::from_array(repr.pos),
            rotation: Quat::from_array(repr.rot),
            scale: Vec3::from_array(repr.scale),
        })
    }
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Aabb { min, max }
    }

    pub fn from_center_extents(center: Vec3, half_extents: Vec3) -> Self {
        Aabb {
            min: center - half_extents,
            max: center + half_extents,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AabbRepr {
    min: [f32; 3],
    max: [f32; 3],
}

impl Serialize for Aabb {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AabbRepr {
            min: self.min.to_array(),
            max: self.max.to_array(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Aabb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let r = AabbRepr::deserialize(deserializer)?;
        Ok(Aabb {
            min: Vec3::from_array(r.min),
            max: Vec3::from_array(r.max),
        })
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // ---- Transform: matrix construction ----

    #[test]
    fn identity_to_mat4_is_mat4_identity() {
        assert_eq!(Transform::IDENTITY.to_mat4(), Mat4::IDENTITY);
    }

    #[test]
    fn default_is_identity() {
        assert_eq!(Transform::default(), Transform::IDENTITY);
    }

    #[test]
    fn translation_only_lives_in_rightmost_column() {
        let t = Transform {
            translation: Vec3::new(3.0, -2.5, 7.0),
            ..Transform::IDENTITY
        };
        let m = t.to_mat4();
        let cols = m.to_cols_array_2d();
        // Column-major: cols[3] is the rightmost column (translation).
        assert_eq!(cols[3][0], 3.0);
        assert_eq!(cols[3][1], -2.5);
        assert_eq!(cols[3][2], 7.0);
        assert_eq!(cols[3][3], 1.0);
        // Upper-left 3x3 should be identity (no rotation, unit scale).
        assert_eq!(cols[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(cols[1], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(cols[2], [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn composition_is_t_times_r_times_s() {
        // A pure-scale transform of 2x, applied to the unit X axis, lands at (2,0,0).
        // A pure-translation of (1,0,0) on top moves it to (3,0,0). T*R*S order.
        let t = Transform {
            translation: Vec3::new(1.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(2.0),
        };
        let m = t.to_mat4();
        let p = m.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(p, Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn rotation_after_scale_before_translation() {
        // Scale (1,1,1), 90 degree yaw, translate (5,0,0). Point (1,0,0) -> rotate to (0,0,-1)
        // -> translate to (5,0,-1). Verifies R is applied between S and T.
        let t = Transform {
            translation: Vec3::new(5.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let p = t.to_mat4().transform_point3(Vec3::new(1.0, 0.0, 0.0));
        let eps = 1e-5;
        assert!((p.x - 5.0).abs() < eps, "x was {}", p.x);
        assert!(p.y.abs() < eps, "y was {}", p.y);
        assert!((p.z + 1.0).abs() < eps, "z was {}", p.z);
    }

    // ---- Transform: serde ----

    #[test]
    fn serde_identity_emits_only_pos() {
        let json = serde_json::to_string(&Transform::IDENTITY).unwrap();
        assert_eq!(json, r#"{"pos":[0.0,0.0,0.0]}"#);
        let back: Transform = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Transform::IDENTITY);
    }

    #[test]
    fn serde_translation_only_emits_only_pos() {
        let t = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            ..Transform::IDENTITY
        };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#"{"pos":[1.0,2.0,3.0]}"#);
        let back: Transform = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn serde_full_transform_round_trips() {
        let t = Transform {
            translation: Vec3::new(1.5, -2.0, 3.25),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_3),
            scale: Vec3::new(2.0, 1.0, 0.5),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Transform = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn serde_deserialize_accepts_missing_rot_and_scale() {
        let json = r#"{"pos":[4.0,5.0,6.0]}"#;
        let t: Transform = serde_json::from_str(json).unwrap();
        assert_eq!(
            t,
            Transform {
                translation: Vec3::new(4.0, 5.0, 6.0),
                ..Transform::IDENTITY
            }
        );
    }

    #[test]
    fn serde_deserialize_rejects_unknown_fields() {
        let json = r#"{"pos":[0.0,0.0,0.0],"bogus":1}"#;
        assert!(serde_json::from_str::<Transform>(json).is_err());
    }

    // ---- Aabb ----

    #[test]
    fn aabb_new_sets_fields() {
        let a = Aabb::new(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(a.min, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(a.max, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn aabb_from_center_extents() {
        let a = Aabb::from_center_extents(Vec3::new(10.0, 0.0, 5.0), Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(a.min, Vec3::new(8.0, -3.0, 1.0));
        assert_eq!(a.max, Vec3::new(12.0, 3.0, 9.0));
    }

    #[test]
    fn aabb_serde_round_trip() {
        let a = Aabb::new(Vec3::new(0.0, 1.0, 2.0), Vec3::new(3.0, 4.0, 5.0));
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, r#"{"min":[0.0,1.0,2.0],"max":[3.0,4.0,5.0]}"#);
        let back: Aabb = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn aabb_serde_rejects_unknown_fields() {
        let json = r#"{"min":[0,0,0],"max":[1,1,1],"bogus":true}"#;
        assert!(serde_json::from_str::<Aabb>(json).is_err());
    }
}
