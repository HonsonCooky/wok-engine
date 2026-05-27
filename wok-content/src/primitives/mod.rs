//! Procedural mesh generation for the five `ShapePrimitive` variants. Each module yields a
//! `MeshCpu` from a primitive's authored parameters plus a tessellation count from
//! `ContentConfig`. The outputs are deterministic: same primitive + same config => byte-
//! identical `MeshCpu` (plan section 7.2 test #2 - the property the multiplayer-model
//! determinism story rides on at the primitive level).
//!
//! `DEFAULT_COLOR` is the engine's placeholder visual color: a flat off-white that reads as
//! "I'm a placeholder, swap me for a shipped mesh" without distracting from the geometry.
//! Authored `Shape.visual_color` overrides this at the slice site, not here; primitives are
//! pure geometry + their own bake-time color choice. (The slicer's runtime `VisibleShape`
//! carries the authored color separately; renderers can multiply or replace as needed.)

use wok_scene::ShapePrimitive;

use crate::config::ContentConfig;
use crate::storage::MeshCpu;

pub mod capsule;
pub mod cube;
pub mod cylinder;
pub mod ellipsoid;
pub mod plane;

/// The placeholder color baked into primitive `MeshVertex.color`. Cel rendering will read
/// this as one of the cel bands; chosen as a mid-gray-toned beige so cel quantization
/// remains discriminable.
pub const PLACEHOLDER_COLOR: [f32; 3] = [0.80, 0.78, 0.72];

/// Generate a `MeshCpu` from a `ShapePrimitive`, dispatching to the per-variant builder.
/// Tessellation parameters come from `config`; primitives that need none (cube, plane)
/// ignore the relevant fields.
pub fn generate(primitive: &ShapePrimitive, config: &ContentConfig) -> MeshCpu {
    match primitive {
        ShapePrimitive::Cube { half_extents } => cube::build(*half_extents),
        ShapePrimitive::Ellipsoid { radii } => ellipsoid::build(*radii, config.ellipsoid_subdivisions),
        ShapePrimitive::Cylinder { radius, half_height } => {
            cylinder::build(*radius, *half_height, config.cylinder_segments)
        }
        ShapePrimitive::Capsule { radius, half_height } => capsule::build(
            *radius,
            *half_height,
            config.cylinder_segments,
            config.ellipsoid_subdivisions,
        ),
        ShapePrimitive::Plane { half_extents } => plane::build(*half_extents),
    }
}
