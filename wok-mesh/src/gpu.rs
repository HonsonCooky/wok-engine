//! GPU mesh residency: [`MeshGpu`] and the [`MeshCpu`] upload path.
//!
//! This is the half of wok-mesh that touches the GPU, and the reason the crate depends on
//! wok-platform (its pinned wgpu re-export; see the HLD's wok-mesh section). Keeping the upload
//! here rather than in wok-render puts the vertex byte layout next to the [`Vertex`] type it
//! mirrors, so the two cannot drift apart unnoticed; wok-render's pipelines consume
//! [`VERTEX_LAYOUT`] instead of restating offsets.
//!
//! The byte layout is the obvious interleaving of [`Vertex`]: position then normal, three `f32`
//! each, [`VERTEX_STRIDE`] (24) bytes per vertex, with `u32` indices. Upload flattens the vertex
//! list into one `f32` array and writes both buffers once; meshes are static after upload (there
//! is no update path, and none is needed while meshes are generated or loaded whole).
//!
//! Upload is infallible by the same argument as generation (see the crate docs): wgpu buffer
//! creation does not return errors (device loss and validation failures surface through wgpu's
//! own error machinery), so there is no failure mode to put in a `Result`.
//!
//! [`Vertex`]: crate::Vertex

use wok_platform::bytemuck;
use wok_platform::wgpu;
use wok_platform::wgpu::util::DeviceExt;

use crate::mesh::MeshCpu;

/// Bytes per vertex on the GPU: position (3 x f32) plus normal (3 x f32), interleaved.
pub const VERTEX_STRIDE: u64 = 24;

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

/// The vertex buffer layout every [`MeshGpu`] vertex buffer satisfies: shader location 0 is the
/// position, location 1 the outward normal. Pipelines that draw a [`MeshGpu`] take this as their
/// sole vertex input.
pub const VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: VERTEX_STRIDE,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &VERTEX_ATTRIBUTES,
};

/// A mesh resident on the GPU: one vertex buffer in [`VERTEX_LAYOUT`], one `u32` index buffer,
/// and the index count to draw. Fields are public in the [`MeshCpu`] spirit: the only invariant
/// ("the buffers hold what `upload` wrote, `index_count` indices' worth") is upheld by
/// construction, and a renderer needs all three to issue a draw.
#[derive(Debug)]
pub struct MeshGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

impl MeshGpu {
    /// Upload `mesh` to new GPU buffers on `device`. The buffers are exactly sized (vertex count
    /// times [`VERTEX_STRIDE`]; index count times 4) and ready to bind without further writes.
    pub fn upload(device: &wgpu::Device, mesh: &MeshCpu) -> MeshGpu {
        let mut floats: Vec<f32> = Vec::with_capacity(mesh.vertices.len() * 6);
        for v in &mesh.vertices {
            floats.extend_from_slice(&[
                v.position.x, v.position.y, v.position.z,
                v.normal.x, v.normal.y, v.normal.z,
            ]);
        }

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wok_mesh_vertices"),
            contents: bytemuck::cast_slice(&floats),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wok_mesh_indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        MeshGpu {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cube::cube;
    use crate::ellipsoid::ellipsoid;

    // A headless device: no window, no surface, any adapter. Panics with a clear message when the
    // environment has no usable GPU; this machine is expected to have one, so a missing adapter is
    // a real failure rather than a skip.
    fn device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("no headless wgpu adapter available");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("failed to open headless wgpu device")
    }

    #[test]
    fn vertex_layout_mirrors_the_vertex_struct() {
        assert_eq!(VERTEX_LAYOUT.array_stride, VERTEX_STRIDE);
        assert_eq!(VERTEX_LAYOUT.step_mode, wgpu::VertexStepMode::Vertex);
        let [position, normal] = VERTEX_LAYOUT.attributes else {
            panic!("expected exactly two vertex attributes");
        };
        assert_eq!(position.shader_location, 0);
        assert_eq!(position.offset, 0);
        assert_eq!(position.format, wgpu::VertexFormat::Float32x3);
        assert_eq!(normal.shader_location, 1);
        assert_eq!(normal.offset, 12);
        assert_eq!(normal.format, wgpu::VertexFormat::Float32x3);
    }

    #[test]
    fn upload_round_trips_buffer_sizes() {
        let (device, _queue) = device();
        for mesh in [cube(), ellipsoid(8, 4)] {
            let gpu = MeshGpu::upload(&device, &mesh);
            assert_eq!(gpu.vertex_buffer.size(), mesh.vertices.len() as u64 * VERTEX_STRIDE);
            assert_eq!(gpu.index_buffer.size(), mesh.indices.len() as u64 * 4);
            assert_eq!(gpu.index_count as usize, mesh.indices.len());
        }
    }

    #[test]
    fn upload_buffers_carry_draw_usages() {
        let (device, _queue) = device();
        let gpu = MeshGpu::upload(&device, &cube());
        assert!(gpu.vertex_buffer.usage().contains(wgpu::BufferUsages::VERTEX));
        assert!(gpu.index_buffer.usage().contains(wgpu::BufferUsages::INDEX));
    }
}
