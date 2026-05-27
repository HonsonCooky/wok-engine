//! Primitive generation tests for plan section 7.2. CPU-side only; the GPU upload test
//! belongs in storage/mesh.rs tests under the integration tag (step 4).

use pantry::math::{Vec2, Vec3};
use wok_scene::ShapePrimitive;
use wok_content::{ContentConfig, MeshCpu, primitives};

const PRIMITIVES: &[ShapePrimitive] = &[
    ShapePrimitive::Cube {
        half_extents: Vec3::new(0.5, 0.5, 0.5),
    },
    ShapePrimitive::Ellipsoid {
        radii: Vec3::new(1.0, 0.5, 0.75),
    },
    ShapePrimitive::Cylinder {
        radius: 0.6,
        half_height: 1.2,
    },
    ShapePrimitive::Capsule {
        radius: 0.4,
        half_height: 0.9,
    },
    ShapePrimitive::Plane {
        half_extents: Vec2::new(10.0, 5.0),
    },
];

// §7.2 #1: Each primitive generates a MeshCpu with non-empty vertices and triangle-count > 0.
#[test]
fn t01_each_primitive_nonempty() {
    let cfg = ContentConfig::default();
    for primitive in PRIMITIVES {
        let mesh = primitives::generate(primitive, &cfg);
        assert!(
            !mesh.vertices.is_empty(),
            "primitive {primitive:?} produced 0 vertices"
        );
        assert!(
            mesh.triangle_count() > 0,
            "primitive {primitive:?} produced 0 triangles"
        );
    }
}

// §7.2 #2: Determinism: same ShapePrimitive → byte-identical MeshCpu.
#[test]
fn t02_determinism() {
    let cfg = ContentConfig::default();
    for primitive in PRIMITIVES {
        let a = primitives::generate(primitive, &cfg);
        let b = primitives::generate(primitive, &cfg);
        assert_eq!(a, b, "primitive {primitive:?} not deterministic");
    }
}

// §7.2 #3: Vertex normals point outward.
//
// For each non-degenerate vertex (i.e. position not at the center), the dot product of the
// vertex position with the vertex normal should be >= 0 (outward-facing). Degenerate
// vertices live at the geometric center and have a normalize_or_zero() output that is the
// zero vector; we ignore those.
#[test]
fn t03_normals_outward() {
    let cfg = ContentConfig::default();
    for primitive in PRIMITIVES {
        let mesh = primitives::generate(primitive, &cfg);
        // Center is implied by the primitive's symmetry: every primitive in the set above is
        // centered at the origin (cube, ellipsoid, cylinder, capsule, plane).
        let center = Vec3::ZERO;
        let mut tested = 0usize;
        for v in &mesh.vertices {
            let pos = Vec3::from_array(v.position);
            let normal = Vec3::from_array(v.normal);
            let to_vertex = pos - center;
            if to_vertex.length_squared() < 1e-12 {
                // Pole vertices on the axis: cylinder centers, plane center (the plane has no
                // such vertex actually). Skip these; their position vector has no direction.
                continue;
            }
            let dot = to_vertex.normalize_or_zero().dot(normal);
            assert!(
                dot >= -1e-3,
                "primitive {primitive:?} vertex at {pos:?} has inward normal {normal:?}, dot {dot}"
            );
            tested += 1;
        }
        assert!(
            tested > 0,
            "primitive {primitive:?} had no testable vertices"
        );
    }
}

// §7.2 #4: Bounding AABB encloses all vertices.
#[test]
fn t04_aabb_encloses_vertices() {
    let cfg = ContentConfig::default();
    for primitive in PRIMITIVES {
        let mesh = primitives::generate(primitive, &cfg);
        let aabb = mesh.bounding_aabb;
        let eps = 1e-5;
        for v in &mesh.vertices {
            let p = Vec3::from_array(v.position);
            assert!(
                p.x >= aabb.min.x - eps
                    && p.y >= aabb.min.y - eps
                    && p.z >= aabb.min.z - eps,
                "primitive {primitive:?} vertex {p:?} below aabb.min {:?}",
                aabb.min
            );
            assert!(
                p.x <= aabb.max.x + eps
                    && p.y <= aabb.max.y + eps
                    && p.z <= aabb.max.z + eps,
                "primitive {primitive:?} vertex {p:?} above aabb.max {:?}",
                aabb.max
            );
        }
    }
}

// §7.2 #5: Tessellation parameters from ContentConfig honored.
//
// Ellipsoid with subdivisions = N produces 2 poles + (N - 1) latitude rings each of
// (2N + 1) vertices, for a total of `(N - 1) * (2N + 1) + 2`. Verify the formula for the
// default subdivisions and one explicit override.
#[test]
fn t05_tessellation_count() {
    let primitive = ShapePrimitive::Ellipsoid {
        radii: Vec3::new(1.0, 1.0, 1.0),
    };

    let cfg = ContentConfig::default(); // subdivisions = 16
    let m = primitives::generate(&primitive, &cfg);
    let n = cfg.ellipsoid_subdivisions;
    let expected = ((n - 1) * (2 * n + 1) + 2) as usize;
    assert_eq!(
        m.vertices.len(),
        expected,
        "default ellipsoid vertex count off"
    );

    let cfg_high = ContentConfig {
        ellipsoid_subdivisions: 32,
        ..ContentConfig::default()
    };
    let m_high = primitives::generate(&primitive, &cfg_high);
    let n_high = cfg_high.ellipsoid_subdivisions;
    let expected_high = ((n_high - 1) * (2 * n_high + 1) + 2) as usize;
    assert_eq!(
        m_high.vertices.len(),
        expected_high,
        "subdivisions=32 ellipsoid vertex count off"
    );

    // Same primitive at different config produces different mesh (sanity-check that the
    // config really threads through).
    let cfg_lo = ContentConfig {
        ellipsoid_subdivisions: 8,
        ..ContentConfig::default()
    };
    let m_lo = primitives::generate(&primitive, &cfg_lo);
    assert_ne!(
        m_lo.vertices.len(),
        m.vertices.len(),
        "config tessellation change had no effect"
    );

    // Cylinder segments knob also threads through.
    let cyl = ShapePrimitive::Cylinder {
        radius: 0.5,
        half_height: 1.0,
    };
    let m_cyl_24 = primitives::generate(&cyl, &ContentConfig::default());
    let m_cyl_8 = primitives::generate(
        &cyl,
        &ContentConfig {
            cylinder_segments: 8,
            ..ContentConfig::default()
        },
    );
    assert!(
        m_cyl_8.vertices.len() < m_cyl_24.vertices.len(),
        "segments=8 cylinder should have fewer vertices than segments=24"
    );

    // Suppress the unused-method-on-MeshCpu warning by referencing triangle_count.
    let _ = MeshCpu::triangle_count;
}
