//! Asset references, surface tags, and instance identity.
//!
//! References here are bare names. The engine does not maintain a registry mapping name to
//! file path or to a content id; the content folder is the source of truth, and resolution
//! is done by scanning. `wok` owns the tooling that surfaces dead, missing, or orphan
//! references; this crate only carries the names.
//!
//! Each reference newtype serializes transparently as its underlying string, so authored JSON
//! sees `"oak_tree"` rather than `{"name":"oak_tree"}`. The reference type system stays useful
//! at the Rust level (`PrefabRef` cannot be silently swapped for `MeshRef`) without leaking
//! into the file format.
//!
//! `SurfaceTag` is the same shape (a transparent string newtype) but names a material rather
//! than a file: the engine carries it on shapes and per terrain cell, and the game maps it to
//! behavior. It is not resolved against the content folder.
//!
//! `InstanceId` is the only generated identity in the data model: a per-scene monotonic u32
//! stamped on each `Placement`. The counter lives on the scene manifest; deleting a placement
//! never frees its id, which keeps editor labels and trigger references stable across edits.

use serde::{Deserialize, Serialize};

/// Reference to a prefab by its file slug (e.g. `"oak_tree"`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrefabRef(pub String);

impl PrefabRef {
    pub fn new(name: impl Into<String>) -> Self {
        PrefabRef(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reference to a mesh asset by its file slug.
///
/// When a `PrefabState` carries a `MeshRef`, the runtime renderer uses that mesh in place of
/// the state's visible shapes; hitbox shapes are still active for collision.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MeshRef(pub String);

impl MeshRef {
    pub fn new(name: impl Into<String>) -> Self {
        MeshRef(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reference to a lighting state by name. Lighting states themselves live in `wok-light` and
/// carry fog and sky parameters; this crate only references them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LightStateRef(pub String);

impl LightStateRef {
    pub fn new(name: impl Into<String>) -> Self {
        LightStateRef(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A free-form surface material name (e.g. `"grass"`, `"stone"`).
///
/// Carried by the engine, mapped to behavior or material by the game. Used in two places:
/// optionally on a `Shape` (see `crate::prefab::Shape`), and per cell in a `Heightmap`'s
/// interned surface table (see `crate::heightmap`). Like the reference newtypes it serializes
/// transparently as a bare string and is never resolved against the content folder.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceTag(pub String);

impl SurfaceTag {
    pub fn new(name: impl Into<String>) -> Self {
        SurfaceTag(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Per-scene placement identity. Monotonic within a scene; never reused.
///
/// Stamped on every `Placement` and allocated from the `Scene` manifest's counter. The counter
/// only advances - deleting a placement does not return its id. That property is what makes
/// editor labels like `oak_tree_42` and trigger references stable across edits, and is the
/// reason the data model has no separate id registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceId(pub u32);

impl InstanceId {
    pub const ZERO: InstanceId = InstanceId(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Reference newtypes: transparent string serde ----

    #[test]
    fn prefab_ref_serializes_as_bare_string() {
        let r = PrefabRef::new("oak_tree");
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""oak_tree""#);
        let back: PrefabRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn mesh_ref_serializes_as_bare_string() {
        let r = MeshRef::new("oak_tree_lod0");
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""oak_tree_lod0""#);
        let back: MeshRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn light_state_ref_serializes_as_bare_string() {
        let r = LightStateRef::new("dawn");
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#""dawn""#);
        let back: LightStateRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn surface_tag_serializes_as_bare_string() {
        let t = SurfaceTag::new("grass");
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#""grass""#);
        let back: SurfaceTag = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn reference_types_are_distinct() {
        // Compile-time check that you can't pass a MeshRef where a PrefabRef is expected.
        // This is the value of the newtypes despite the transparent serde representation.
        fn accept_prefab(_: PrefabRef) {}
        accept_prefab(PrefabRef::new("x"));
    }

    // ---- InstanceId: transparent u32 serde ----

    #[test]
    fn instance_id_serializes_as_bare_number() {
        let id = InstanceId(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "42");
        let back: InstanceId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn instance_id_zero_constant() {
        assert_eq!(InstanceId::ZERO, InstanceId(0));
    }
}
