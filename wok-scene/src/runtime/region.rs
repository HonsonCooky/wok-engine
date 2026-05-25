use pantry::math::Aabb;

use crate::authored::RegionPurpose;

/// Runtime form of a `RegionMarker`. Bounds are chunk-local; the authored data already
/// clipped them to the chunk's extents.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeRegion {
    pub name: String,
    pub local_bounds: Aabb,
    pub purpose: RegionPurpose,
}
