//! Prefab data: stateful shape lists with optional mesh replacement.
//!
//! A prefab is a reusable authored entity (a tree, a barrel, a door). It carries one or more
//! named states (`default`, `open`, `destroyed`); each state is a list of shapes plus an
//! optional `MeshRef` that stands in for the visible shapes at render time when present.
//! Hitbox shapes always provide collision, even when a mesh is supplying the visible surface.
//!
//! The three meaningful shape flag combinations (HLD principle: placeholder-first authoring):
//!
//! - `is_hitbox = true, is_visible = true` - solid placeholder.
//! - `is_hitbox = true, is_visible = false` - trigger volume.
//! - `is_hitbox = false, is_visible = true` - visual-only placeholder.
//!
//! `is_hitbox = false, is_visible = false` is degenerate but not forbidden here; tooling in
//! `wok-shell` is the place to warn about it.

use serde::{Deserialize, Serialize};

use crate::math::Transform;
use crate::refs::{MeshRef, SurfaceTag};

/// Unit-shape primitives. Size and placement come from the parent `Shape`'s transform; these
/// variants are dimensionless.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Primitive {
    Cube,
    Ellipsoid,
    Cylinder,
    Capsule,
    Plane,
}

/// A single shape within a prefab state.
///
/// `is_hitbox` and `is_visible` independently gate the shape's participation in collision and
/// rendering. See module docs for the three meaningful combinations.
///
/// `surface` is an optional material tag ("grass", "stone"); the engine only carries it and
/// the game maps it to behavior. `None` means untagged, which is valid - older shapes authored
/// before surface tags existed deserialize as untagged via the `serde(default)`. The newtype
/// makes `Shape` no longer `Copy` (it now owns a `String`); callers clone where they used to
/// copy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub primitive: Primitive,
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<SurfaceTag>,
    pub is_hitbox: bool,
    pub is_visible: bool,
}

/// One named state of a prefab.
///
/// When `mesh` is `Some`, that mesh stands in for the state's visible shapes at render time;
/// the visible shapes are still meaningful at authoring time (block-out, dimensions) but are
/// not drawn. Hitbox shapes are always active for collision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrefabState {
    pub name: String,
    pub shapes: Vec<Shape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshRef>,
}

/// A prefab: a set of named states with a chosen default.
///
/// `default_state` is the name of the state to use when a `Placement` does not specify one;
/// it must match one of the entries in `states`. Mismatch is a load-time validation error
/// (see `crate::io::load_prefab` and `crate::LoadError::Validation`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prefab {
    pub states: Vec<PrefabState>,
    pub default_state: String,
}

impl Prefab {
    /// True if `default_state` names one of the prefab's states.
    pub fn default_state_is_valid(&self) -> bool {
        self.states.iter().any(|s| s.name == self.default_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Transform;
    use glam::Vec3;

    fn sample_shape() -> Shape {
        Shape {
            primitive: Primitive::Cube,
            transform: Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                ..Transform::IDENTITY
            },
            surface: None,
            is_hitbox: true,
            is_visible: true,
        }
    }

    // ---- Primitive ----

    #[test]
    fn primitive_round_trips_for_every_variant() {
        for p in [
            Primitive::Cube,
            Primitive::Ellipsoid,
            Primitive::Cylinder,
            Primitive::Capsule,
            Primitive::Plane,
        ] {
            let json = serde_json::to_string(&p).unwrap();
            let back: Primitive = serde_json::from_str(&json).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn primitive_serializes_as_bare_string() {
        let json = serde_json::to_string(&Primitive::Capsule).unwrap();
        assert_eq!(json, r#""Capsule""#);
    }

    // ---- Shape ----

    #[test]
    fn shape_round_trips_without_surface() {
        let s = sample_shape();
        assert!(s.surface.is_none());
        let json = serde_json::to_string(&s).unwrap();
        // An untagged shape emits no `surface` key, so files predating surface tags stay valid.
        assert!(!json.contains("surface"));
        let back: Shape = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn shape_round_trips_with_surface() {
        let s = Shape {
            surface: Some(SurfaceTag::new("stone")),
            ..sample_shape()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""surface":"stone""#));
        let back: Shape = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn shape_deserializes_legacy_json_without_surface_field() {
        // The serde(default) is what lets a shape authored before surface tags load cleanly.
        let json = r#"{"primitive":"Cube","transform":{"pos":[0.0,0.0,0.0]},"is_hitbox":true,"is_visible":false}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.surface, None);
        assert_eq!(s.primitive, Primitive::Cube);
        assert!(s.is_hitbox && !s.is_visible);
    }

    #[test]
    fn shape_rejects_unknown_fields() {
        let json = r#"{"primitive":"Cube","transform":{"pos":[0,0,0]},"is_hitbox":true,"is_visible":true,"bogus":1}"#;
        assert!(serde_json::from_str::<Shape>(json).is_err());
    }

    // ---- PrefabState ----

    #[test]
    fn prefab_state_round_trips_without_mesh() {
        let s = PrefabState {
            name: "default".into(),
            shapes: vec![sample_shape()],
            mesh: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        // Missing mesh field when None.
        assert!(!json.contains("mesh"));
        let back: PrefabState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn prefab_state_round_trips_with_mesh() {
        let s = PrefabState {
            name: "default".into(),
            shapes: vec![],
            mesh: Some(MeshRef::new("oak_tree_lod0")),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: PrefabState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ---- Prefab ----

    #[test]
    fn prefab_round_trips() {
        let p = Prefab {
            states: vec![
                PrefabState {
                    name: "default".into(),
                    shapes: vec![sample_shape()],
                    mesh: None,
                },
                PrefabState {
                    name: "destroyed".into(),
                    shapes: vec![],
                    mesh: None,
                },
            ],
            default_state: "default".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Prefab = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn default_state_is_valid_when_named() {
        let p = Prefab {
            states: vec![PrefabState {
                name: "default".into(),
                shapes: vec![],
                mesh: None,
            }],
            default_state: "default".into(),
        };
        assert!(p.default_state_is_valid());
    }

    #[test]
    fn default_state_is_invalid_when_not_named() {
        let p = Prefab {
            states: vec![PrefabState {
                name: "default".into(),
                shapes: vec![],
                mesh: None,
            }],
            default_state: "open".into(),
        };
        assert!(!p.default_state_is_valid());
    }
}
