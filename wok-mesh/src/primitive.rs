//! The default tessellation and the [`primitive_mesh`] dispatcher that turns a [`Primitive`] into its
//! unit mesh.
//!
//! The unit-primitive convention - the unit half-extent and each primitive's unit bounds - is owned
//! by wok-scene (`wok_scene::UNIT_HALF_EXTENT` and `Primitive::unit_aabb`; see
//! `designs/high-level-design.md`, wok-scene section). The per-shape generators read that one
//! constant for vertex placement, so there is no second copy to drift. The cross-convention test
//! below is the structural tie: a generated unit mesh's bounds must equal `Primitive::unit_aabb` for
//! the same primitive - drawn mesh and collider checked against one source.

use wok_scene::Primitive;

use crate::capsule::capsule;
use crate::cube::{cube, plane};
use crate::cylinder::cylinder;
use crate::ellipsoid::ellipsoid;
use crate::mesh::MeshCpu;

/// Default longitude divisions for the round primitives: smooth enough for cel shading at the humble
/// vertex counts the engine targets, and divisible by 4 so the cardinal points land exactly on the
/// `+/-0.5` faces (the unit half-extent).
pub const DEFAULT_SEGMENTS: usize = 24;

/// Default latitude stacks for the ellipsoid; the capsule uses half this per hemisphere.
pub const DEFAULT_RINGS: usize = 16;

/// Generate the unit mesh for `primitive` at the default tessellation. The per-shape generators
/// ([`cube`], [`plane`], [`ellipsoid`], [`cylinder`], [`capsule`]) are public for callers that want
/// to pick their own tessellation; this is the convenience entry point.
pub fn primitive_mesh(primitive: Primitive) -> MeshCpu {
    match primitive {
        Primitive::Cube => cube(),
        Primitive::Plane => plane(),
        Primitive::Ellipsoid => ellipsoid(DEFAULT_SEGMENTS, DEFAULT_RINGS),
        Primitive::Cylinder => cylinder(DEFAULT_SEGMENTS),
        Primitive::Capsule => capsule(DEFAULT_SEGMENTS, DEFAULT_RINGS / 2),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const PRIMITIVES: [Primitive; 5] = [
        Primitive::Cube,
        Primitive::Ellipsoid,
        Primitive::Cylinder,
        Primitive::Capsule,
        Primitive::Plane,
    ];

    #[test]
    fn every_primitive_matches_the_scene_unit_aabb() {
        // The crate's reason to exist: drawn mesh bounds equal collision bounds for every unit shape.
        // Both sides now read wok-scene's canonical `Primitive::unit_aabb`, so the agreement is
        // structural - asserted against one source - rather than a local replica of the convention
        // kept in lockstep by hand.
        for p in PRIMITIVES {
            let mesh_bounds = primitive_mesh(p).bounds();
            let convention = p.unit_aabb();
            assert!(
                (mesh_bounds.min - convention.min).length() < 1e-6
                    && (mesh_bounds.max - convention.max).length() < 1e-6,
                "{p:?}: mesh bounds {mesh_bounds:?} != convention {convention:?}",
            );
        }
    }

    #[test]
    fn every_primitive_regenerates_bitwise() {
        for p in PRIMITIVES {
            assert_eq!(primitive_mesh(p), primitive_mesh(p), "{p:?} not deterministic");
        }
    }

    #[test]
    fn dispatcher_picks_the_matching_generator() {
        assert_eq!(primitive_mesh(Primitive::Cube), cube());
        assert_eq!(primitive_mesh(Primitive::Plane), plane());
        assert_eq!(primitive_mesh(Primitive::Ellipsoid), ellipsoid(DEFAULT_SEGMENTS, DEFAULT_RINGS));
        assert_eq!(primitive_mesh(Primitive::Cylinder), cylinder(DEFAULT_SEGMENTS));
        assert_eq!(primitive_mesh(Primitive::Capsule), capsule(DEFAULT_SEGMENTS, DEFAULT_RINGS / 2));
    }
}
