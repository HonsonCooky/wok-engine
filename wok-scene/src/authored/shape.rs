use pantry::math::{Transform, Vec2, Vec3};
use pantry::serde::{Deserialize, Serialize};

use crate::ids::TriggerId;

/// Primitive geometry used as both visible placeholder and collision/trigger volume. The
/// JSON tag `kind` and `snake_case` variant names match the format documented in the plan.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    crate = "pantry::serde",
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ShapePrimitive {
    Cube {
        #[serde(with = "vec3_array")]
        half_extents: Vec3,
    },
    Ellipsoid {
        #[serde(with = "vec3_array")]
        radii: Vec3,
    },
    Cylinder {
        radius: f32,
        half_height: f32,
    },
    Capsule {
        radius: f32,
        half_height: f32,
    },
    Plane {
        #[serde(with = "vec2_array")]
        half_extents: Vec2,
    },
}

/// Authored shape: a primitive with a prefab-local transform and flags that decide which
/// runtime arrays it slices into. The `(is_hitbox, is_visible)` pair is the discriminator:
///
/// - `(true,  true)`  -> solid placeholder, slices into visible + hitbox arrays.
/// - `(true,  false)` -> trigger volume, slices into triggers; `trigger_id` is required.
/// - `(false, true)`  -> visual-only placeholder.
/// - `(false, false)` -> error at slice time (accepted in authored data so the editor can
///   transiently produce this; the slicer rejects it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct Shape {
    pub primitive: ShapePrimitive,
    pub transform: Transform,
    pub is_hitbox: bool,
    pub is_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<TriggerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_color: Option<[f32; 3]>,
}

mod vec3_array {
    use pantry::math::Vec3;
    use pantry::serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec3, serializer: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec3, D::Error> {
        let arr = <[f32; 3]>::deserialize(deserializer)?;
        Ok(Vec3::from_array(arr))
    }
}

mod vec2_array {
    use pantry::math::Vec2;
    use pantry::serde::{Deserialize, Deserializer, Serialize, Serializer};

    // Serde's `with` contract requires `serialize` to take `&T`; clippy flags Vec2 as small
    // enough to pass by value, but changing the signature would break the integration.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S: Serializer>(v: &Vec2, serializer: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec2, D::Error> {
        let arr = <[f32; 2]>::deserialize(deserializer)?;
        Ok(Vec2::from_array(arr))
    }
}
