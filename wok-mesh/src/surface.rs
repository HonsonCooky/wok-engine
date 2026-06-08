//! Shared construction for surfaces of revolution: ring strips, pole fans, and the UV sphere the
//! ellipsoid and capsule are both built from.
//!
//! A ring is `segments` vertices evenly spaced around the y-axis, ordered by increasing longitude
//! so that, viewed from `+Y` (above), they run counter-clockwise. Two adjacent rings are stitched
//! by [`connect_rings`]; a ring is closed off to a single apex vertex by [`top_fan`] (apex above)
//! or [`bottom_fan`] (apex below). All three emit counter-clockwise-from-outside triangles, the
//! crate-wide winding. Longitude wraps modularly (`(j + 1) % segments`), so a ring shares its seam
//! vertex rather than duplicating it.
//!
//! Determinism: every routine is a fixed sequential loop of arithmetic, no RNG and no parallelism,
//! so identical inputs build a bitwise-identical mesh.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::Vec3;

use crate::mesh::{MeshCpu, Vertex};

/// Stitch two consecutive rings into a quad strip. `lower` and `upper` are the first-vertex indices
/// of the two rings, each holding `segments` vertices; `upper` is the ring at greater `y`. Emits two
/// counter-clockwise-from-outside triangles per longitude step.
pub(crate) fn connect_rings(indices: &mut Vec<u32>, lower: u32, upper: u32, segments: u32) {
    for j in 0..segments {
        let jn = (j + 1) % segments;
        indices.extend_from_slice(&[
            lower + j, upper + j, upper + jn,
            lower + j, upper + jn, lower + jn,
        ]);
    }
}

/// Close the top of a ring to a single `apex` vertex sitting above it (the north pole, or a flat
/// `+Y` cap centre). `ring` is the ring's first-vertex index. Triangles face outward / `+Y`.
pub(crate) fn top_fan(indices: &mut Vec<u32>, ring: u32, apex: u32, segments: u32) {
    for j in 0..segments {
        let jn = (j + 1) % segments;
        indices.extend_from_slice(&[ring + j, apex, ring + jn]);
    }
}

/// Close the bottom of a ring to a single `apex` vertex sitting below it (the south pole, or a flat
/// `-Y` cap centre). `ring` is the ring's first-vertex index. Triangles face outward / `-Y`.
pub(crate) fn bottom_fan(indices: &mut Vec<u32>, apex: u32, ring: u32, segments: u32) {
    for j in 0..segments {
        let jn = (j + 1) % segments;
        indices.extend_from_slice(&[apex, ring + j, ring + jn]);
    }
}

/// A UV sphere of the given `radius` centred at the origin, with `segments` longitude divisions and
/// `rings` latitude stacks (two poles plus `rings - 1` interior rings). Normals are radial (outward).
///
/// `segments` clamps up to 3, `rings` up to 2, so a degenerate tessellation parameter yields the
/// coarsest closed sphere rather than panicking or emitting degenerate triangles. The poles are
/// single apex vertices (fans), not zero-radius rings, so there are no zero-area triangles.
pub(crate) fn uv_sphere(segments: usize, rings: usize, radius: f32) -> MeshCpu {
    let segments = segments.max(3);
    let rings = rings.max(2);
    let seg_u32 = segments as u32;

    let mut vertices = Vec::with_capacity(segments * (rings - 1) + 2);
    let mut indices = Vec::with_capacity(segments * (rings - 1) * 6);

    // South pole, then the interior latitude rings (i = 1..rings), then the north pole.
    let south = vertices.len() as u32;
    vertices.push(Vertex::new(Vec3::new(0.0, -radius, 0.0), Vec3::NEG_Y));

    let mut ring_starts = Vec::with_capacity(rings - 1);
    for i in 1..rings {
        let lat = -FRAC_PI_2 + PI * (i as f32 / rings as f32);
        let (sin_lat, cos_lat) = lat.sin_cos();
        let y = radius * sin_lat;
        let ring_radius = radius * cos_lat;
        ring_starts.push(vertices.len() as u32);
        for j in 0..segments {
            let lon = TAU * (j as f32 / segments as f32);
            let (sin_lon, cos_lon) = lon.sin_cos();
            let position = Vec3::new(ring_radius * cos_lon, y, ring_radius * sin_lon);
            vertices.push(Vertex::new(position, position.normalize()));
        }
    }

    let north = vertices.len() as u32;
    vertices.push(Vertex::new(Vec3::new(0.0, radius, 0.0), Vec3::Y));

    bottom_fan(&mut indices, south, ring_starts[0], seg_u32);
    for pair in ring_starts.windows(2) {
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

    #[test]
    fn uv_sphere_vertices_lie_on_the_radius_surface() {
        let mesh = uv_sphere(16, 12, 0.5);
        for v in &mesh.vertices {
            assert!((v.position.length() - 0.5).abs() < 1e-6, "off-surface vertex {:?}", v.position);
        }
    }

    #[test]
    fn uv_sphere_normals_point_outward_and_are_unit() {
        let mesh = uv_sphere(16, 12, 0.5);
        for v in &mesh.vertices {
            assert!((v.normal.length() - 1.0).abs() < 1e-6, "non-unit normal {:?}", v.normal);
            assert!(v.normal.dot(v.position) > 0.0, "inward normal {:?}", v.normal);
        }
    }

    #[test]
    fn uv_sphere_triangles_all_wind_outward() {
        assert!(faces_outward(&uv_sphere(16, 12, 0.5), Vec3::ZERO));
    }

    #[test]
    fn uv_sphere_has_no_degenerate_triangles() {
        let mesh = uv_sphere(16, 12, 0.5);
        for t in mesh.indices.chunks_exact(3) {
            let a = mesh.vertices[t[0] as usize].position;
            let b = mesh.vertices[t[1] as usize].position;
            let c = mesh.vertices[t[2] as usize].position;
            assert!((b - a).cross(c - a).length() > 1e-9, "degenerate triangle {t:?}");
        }
    }

    #[test]
    fn uv_sphere_clamps_degenerate_tessellation() {
        // Zero segments / rings must not panic; they clamp to the coarsest closed sphere.
        let mesh = uv_sphere(0, 0, 0.5);
        assert!(faces_outward(&mesh, Vec3::ZERO));
        assert!(mesh.triangle_count() > 0);
    }
}
