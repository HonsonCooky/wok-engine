//! The unit `Capsule`: a radius-[`UNIT_HALF_EXTENT`] capsule inscribed in the unit cube.
//!
//! ## Why the unit capsule is a sphere
//!
//! A capsule is a cylinder of radius `r` and body height `b` capped by a hemisphere of radius `r` at
//! each end, so its total height is `b + 2r` and its width is `2r`. To match the unit-primitive
//! convention the shape must fill the unit cube: width `2r = 1`, so `r =` [`UNIT_HALF_EXTENT`]`= 0.5`,
//! and total height `b + 2r = 1`, so the body height `b = 1 - 2r = 0`. A zero-height body leaves just
//! the two hemispheres meeting at the equator: a sphere of radius 0.5. This is forced by the
//! convention, not a shortcut, and it is what keeps the mesh's bounds equal to the cube
//! `world_aabb` wok-physics uses for a `Capsule` hitbox (any non-zero body would push past `+/-0.5`
//! in `y`, and a smaller radius would fall short in `x`/`z`, either way disagreeing with collision).
//!
//! So the unit capsule mesh coincides with the unit ellipsoid mesh. They are kept as distinct
//! generators because they are distinct primitives the convention happens to collapse at unit size.
//! When a capsule needs a visible body, that is a different contract entirely: [`capsule_mesh`]
//! below, parameterized in metres and deliberately outside the unit-primitive convention.

use std::f32::consts::{FRAC_PI_2, TAU};

use glam::Vec3;
use wok_scene::UNIT_HALF_EXTENT;

use crate::mesh::{MeshCpu, Vertex};
use crate::surface::{bottom_fan, connect_rings, top_fan, uv_sphere};
use crate::{DEFAULT_RINGS, DEFAULT_SEGMENTS};

/// The unit capsule (a radius-[`UNIT_HALF_EXTENT`] sphere; see the module docs for why the body
/// height is zero). `segments` is the longitude division count and `rings` the latitude stacks *per
/// hemisphere*, so the sphere is built with `2 * rings` stacks; both clamp up to a sane minimum. The
/// dispatcher's defaults are [`crate::DEFAULT_SEGMENTS`] and [`crate::DEFAULT_RINGS`]` / 2`.
pub fn capsule(segments: usize, rings: usize) -> MeshCpu {
    uv_sphere(segments, rings.max(1) * 2, UNIT_HALF_EXTENT)
}

/// A parameterized true capsule, in metres: a cylindrical wall of length `segment` along the y axis
/// capped by a hemisphere of `radius` at each end, centred at the origin. Total height is
/// `segment + 2 * radius`; the cap poles sit at exactly `+/-(segment / 2 + radius)`. Counter-
/// clockwise outward winding and smooth (radial-from-the-cap-centre) normals, the crate-wide
/// [`Vertex`] contract.
///
/// This generator is deliberately OUTSIDE the unit-primitive convention. The convention forces the
/// unit [`capsule`] to collapse to a sphere (see the module docs); that limitation stands for scene
/// prefabs, which are unit shapes sized by their placement transform. `capsule_mesh` instead pairs
/// with wok-physics's parameterized `Capsule` - explicit metres, the numbers are the shape - so a
/// character drawn with it matches its collider exactly (`Capsule::upright(c, h, r)` is
/// `capsule_mesh(r, h - 2.0 * r)` translated to `c`). Size is baked into the vertices; the draw
/// transform should translate and rotate, not scale.
///
/// A non-positive `segment` clamps to zero (a sphere with a doubled equator ring); tessellation is
/// the crate default ([`DEFAULT_SEGMENTS`] longitude divisions, [`DEFAULT_RINGS`]` / 2` latitude
/// stacks per hemisphere).
pub fn capsule_mesh(radius: f32, segment: f32) -> MeshCpu {
    let segments = DEFAULT_SEGMENTS;
    let rings = DEFAULT_RINGS / 2; // latitude stacks per hemisphere
    let seg_u32 = segments as u32;
    let half = segment.max(0.0) * 0.5;

    let mut vertices = Vec::with_capacity(2 * segments * rings + 2);
    let mut indices = Vec::with_capacity(segments * (2 * rings) * 6);

    // One latitude ring of a cap hemisphere: unit directions at `lat` swept around y, pushed out by
    // `radius` from the cap's centre `(0, cy, 0)`. The direction doubles as the smooth normal, so
    // the equator rings (lat 0) come out horizontal-radial - exactly the wall's normal, which is
    // what lets the wall stitch the two equators with no seam vertex duplication.
    let cap_ring = |vertices: &mut Vec<Vertex>, lat: f32, cy: f32| -> u32 {
        let start = vertices.len() as u32;
        let (sin_lat, cos_lat) = lat.sin_cos();
        for j in 0..segments {
            let lon = TAU * (j as f32 / segments as f32);
            let (sin_lon, cos_lon) = lon.sin_cos();
            let n = Vec3::new(cos_lat * cos_lon, sin_lat, cos_lat * sin_lon);
            vertices.push(Vertex::new(Vec3::new(0.0, cy, 0.0) + n * radius, n));
        }
        start
    };

    // South pole, the lower hemisphere's rings up to its equator (lat runs -pi/2 toward 0; i =
    // rings lands on exactly 0), the upper hemisphere's rings from its equator (lat 0) up, north
    // pole. The two equator rings are distinct vertices `segment` apart; the wall spans them.
    let south = vertices.len() as u32;
    vertices.push(Vertex::new(Vec3::new(0.0, -(half + radius), 0.0), Vec3::NEG_Y));
    let mut ring_starts = Vec::with_capacity(2 * rings);
    for i in 1..=rings {
        let lat = -FRAC_PI_2 + FRAC_PI_2 * (i as f32 / rings as f32);
        ring_starts.push(cap_ring(&mut vertices, lat, -half));
    }
    for i in 0..rings {
        let lat = FRAC_PI_2 * (i as f32 / rings as f32);
        ring_starts.push(cap_ring(&mut vertices, lat, half));
    }
    let north = vertices.len() as u32;
    vertices.push(Vertex::new(Vec3::new(0.0, half + radius, 0.0), Vec3::Y));

    bottom_fan(&mut indices, south, ring_starts[0], seg_u32);
    for pair in ring_starts.windows(2) {
        // Consecutive cap rings, including the lower-equator-to-upper-equator pair: the wall.
        connect_rings(&mut indices, pair[0], pair[1], seg_u32);
    }
    top_fan(&mut indices, *ring_starts.last().unwrap(), north, seg_u32);

    MeshCpu::new(vertices, indices)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::mesh::faces_outward;
    use glam::Vec3;

    #[test]
    fn capsule_vertices_lie_on_the_radius_half_surface() {
        let mesh = capsule(24, 8);
        for v in &mesh.vertices {
            assert!((v.position.length() - 0.5).abs() < 1e-6, "off-surface vertex {:?}", v.position);
            assert!(v.normal.dot(v.position) > 0.0, "inward normal {:?}", v.normal);
        }
    }

    #[test]
    fn capsule_triangles_all_wind_outward() {
        assert!(faces_outward(&capsule(24, 8), Vec3::ZERO));
    }

    #[test]
    fn capsule_bounds_are_the_unit_cube() {
        let b = capsule(24, 8).bounds();
        assert!((b.min - Vec3::splat(-0.5)).length() < 1e-6, "min {:?}", b.min);
        assert!((b.max - Vec3::splat(0.5)).length() < 1e-6, "max {:?}", b.max);
    }

    #[test]
    fn capsule_uses_two_stacks_per_hemisphere_ring_count() {
        // rings-per-hemisphere maps to 2*rings latitude stacks: matches an ellipsoid with that many.
        assert_eq!(capsule(24, 8), crate::ellipsoid(24, 16));
    }

    #[test]
    fn capsule_regenerates_bitwise() {
        assert_eq!(capsule(24, 8), capsule(24, 8));
    }

    // ---- capsule_mesh (parameterized, outside the unit convention) ----

    #[test]
    fn capsule_mesh_bounds_equal_the_analytic_capsule_aabb() {
        // The analytic AABB of a capsule of radius r and wall length s about the origin: +/-r in x
        // and z, +/-(s/2 + r) in y. The mesh's bounds must be that box, which is also exactly what
        // wok-physics computes for the matching collider - the agreement the generator exists for.
        let (r, s) = (0.45, 0.6);
        let b = capsule_mesh(r, s).bounds();
        let expect = Vec3::new(r, s * 0.5 + r, r);
        assert!((b.min - -expect).length() < 1e-6, "min {:?}", b.min);
        assert!((b.max - expect).length() < 1e-6, "max {:?}", b.max);
    }

    #[test]
    fn capsule_mesh_cap_poles_sit_exactly_at_the_tips() {
        // The poles are constructed directly at +/-(s/2 + r), not through any trigonometry, so the
        // mesh's vertical extremes are exact - no roundoff for a caller sizing a character to a
        // collider to absorb.
        let (r, s) = (0.45, 0.6);
        let mesh = capsule_mesh(r, s);
        let top = mesh.vertices.iter().map(|v| v.position.y).fold(f32::NEG_INFINITY, f32::max);
        let bottom = mesh.vertices.iter().map(|v| v.position.y).fold(f32::INFINITY, f32::min);
        assert_eq!(top, s * 0.5 + r);
        assert_eq!(bottom, -(s * 0.5 + r));
    }

    #[test]
    fn capsule_mesh_regenerates_bitwise() {
        assert_eq!(capsule_mesh(0.45, 0.6), capsule_mesh(0.45, 0.6));
    }
}
