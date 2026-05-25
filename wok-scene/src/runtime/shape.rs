use pantry::math::Mat4;

use crate::authored::ShapePrimitive;
use crate::ids::TriggerId;

/// A visible shape sliced out of an authored prefab placement. Transform is chunk-local;
/// `source_placement` is the placement's index inside its chunk for back-tracing in tools.
///
/// Runtime types do not serialize. They are derived from authored data via `slice_chunk` on
/// chunk load and discarded on chunk unload. Caching to disk is a deliberate future addition,
/// not a default behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleShape {
    pub primitive: ShapePrimitive,
    pub local_transform: Mat4,
    pub color: [f32; 3],
    pub source_placement: u32,
}

/// A physical hitbox sliced out of an authored shape with `is_hitbox = true`. `surface_tag`
/// is an interned index into `ChunkRuntime::surface_tag_table`. The empty string `""` is the
/// conventional "untagged" value, produced when authored `Shape.surface_tag` is `None`;
/// physics consumers should treat it as the default surface.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicalHitbox {
    pub primitive: ShapePrimitive,
    pub local_transform: Mat4,
    pub surface_tag: u32,
    pub source_placement: u32,
}

/// A trigger volume sliced out of an authored shape with `is_hitbox = true, is_visible =
/// false`. The `trigger_id` is the game-defined identifier the volume fires under.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerVolume {
    pub primitive: ShapePrimitive,
    pub local_transform: Mat4,
    pub trigger_id: TriggerId,
    pub source_placement: u32,
}
