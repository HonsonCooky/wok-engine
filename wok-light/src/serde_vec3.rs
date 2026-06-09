//! Serde glue for `glam::Vec3` as a bare `[f32; 3]` JSON array.
//!
//! wok-scene deliberately does not enable glam's `serde` feature; it hand-rolls every vector as a
//! `[f32; 3]` repr so the on-disk shape is owned by the engine and not by a glam version. wok-light
//! follows the same convention. Colours and the sun direction are all `Vec3`, so rather than wrap
//! each in its own repr struct (as wok-scene does for `Transform` and `Aabb`), this module exposes
//! `serialize`/`deserialize` functions usable through `#[serde(with = "crate::serde_vec3")]` on any
//! `Vec3` field. The wire form is `[x, y, z]`, identical to wok-scene's vectors.

use glam::Vec3;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S: Serializer>(v: &Vec3, serializer: S) -> Result<S::Ok, S::Error> {
    v.to_array().serialize(serializer)
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec3, D::Error> {
    <[f32; 3]>::deserialize(deserializer).map(Vec3::from_array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "super")]
        v: Vec3,
    }

    #[test]
    fn vec3_serializes_as_bare_array() {
        let h = Holder { v: Vec3::new(1.0, 2.0, 3.0) };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, r#"{"v":[1.0,2.0,3.0]}"#);
        let back: Holder = serde_json::from_str(&json).unwrap();
        assert_eq!(back, h);
    }
}
