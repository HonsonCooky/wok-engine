//! Minimal GPU upload smoke test for the storage layer (plan section 11 step 4).
//!
//! The full GPU test surface (worker dedup, terrain upload count, etc.) sits in the
//! workspace integration test crate in later phases. Here we verify only that the upload
//! helper produces a `MeshGpu` whose `index_count` matches the source `MeshCpu`.

mod common;

use pantry::math::Vec3;
use wok_scene::ShapePrimitive;
use wok_content::{ContentConfig, primitives, storage};

#[test]
fn upload_primitive_produces_index_count() {
    let (device, queue) = common::init_gpu();
    let cfg = ContentConfig::default();
    let cpu = primitives::generate(
        &ShapePrimitive::Cube {
            half_extents: Vec3::new(0.5, 0.5, 0.5),
        },
        &cfg,
    );
    let cpu_index_count = cpu.indices.len() as u32;
    let gpu = storage::upload(&device, &queue, &cpu, "cube-upload").expect("upload");
    assert_eq!(gpu.index_count, cpu_index_count);
    assert_eq!(gpu.bounding_aabb, cpu.bounding_aabb);
}
