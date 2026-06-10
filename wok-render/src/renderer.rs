//! The [`Renderer`]: owned pipeline state and the per-frame [`Renderer::render`] entry point.

use glam::{Mat4, Vec3};
use wok_light::LightState;
use wok_mesh::MeshGpu;
use wok_platform::bytemuck;
use wok_platform::wgpu;

use crate::pipeline;
use crate::uniforms;

/// Per-draw buffer slots allocated up front; the buffer grows (power of two) when a frame's
/// render list exceeds the current capacity, so steady-state frames never reallocate.
const INITIAL_DRAW_CAPACITY: usize = 64;

/// The caller's camera for one frame. The caller supplies final matrices: how view and projection
/// compose, and any chunk-origin rebasing, are its policy. `eye` is the camera's world position,
/// which a combined matrix does not expose; fog distance and rim lighting measure from it.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub view_proj: Mat4,
    pub eye: Vec3,
}

/// One entry in the frame's render list: a world transform, a mesh to draw, and a flat base
/// color (linear RGB). The transform is final; wok-render applies no chunk or parent composition.
#[derive(Debug)]
pub struct RenderItem<'m> {
    pub transform: Mat4,
    pub mesh: &'m MeshGpu,
    pub color: Vec3,
}

/// The forward renderer: depth buffer, frame uniforms, per-draw storage, and the sky and mesh
/// pipelines. One per render target size; create with [`Renderer::new`] against the target's
/// texture format and keep [`Renderer::resize`] in step with the target.
///
/// There is no error enum: per `designs/project-canon.md` an error type earns its place only when
/// a genuine failure mode needs distinguishing, and nothing here has one. The shaders are
/// compiled into the binary and validated by the crate's tests, and wgpu's create and write calls
/// do not return errors (device loss surfaces through wgpu's own machinery).
pub struct Renderer {
    mesh_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    camera_buffer: wgpu::Buffer,
    light_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    draw_layout: wgpu::BindGroupLayout,
    draw_buffer: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    draw_capacity: usize,
    draw_stride: u64,
}

impl Renderer {
    /// Build the pipeline state for a `width` x `height` target of `surface_format`. For on-screen
    /// rendering pass the surface configuration's format and size; for render-to-texture pass the
    /// texture's.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Renderer {
        let frame_layout = pipeline::frame_layout(device);
        let draw_layout = pipeline::draw_layout(device);
        let mesh_pipeline = pipeline::mesh_pipeline(device, surface_format, &frame_layout, &draw_layout);
        let sky_pipeline = pipeline::sky_pipeline(device, surface_format, &frame_layout);
        let depth_view = pipeline::depth_texture(device, width, height);

        let camera_buffer = uniform_buffer(device, "wok_render_camera", uniforms::CAMERA_UNIFORM_SIZE);
        let light_buffer = uniform_buffer(device, "wok_render_light", uniforms::LIGHT_UNIFORM_SIZE);
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wok_render_frame_group"),
            layout: &frame_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: light_buffer.as_entire_binding() },
            ],
        });

        // Dynamic offsets must be multiples of the device's uniform alignment, so the per-draw
        // stride is the block size rounded up to it.
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let draw_stride = uniforms::DRAW_UNIFORM_SIZE.next_multiple_of(alignment);
        let (draw_buffer, draw_group) =
            draw_resources(device, &draw_layout, draw_stride, INITIAL_DRAW_CAPACITY);

        Renderer {
            mesh_pipeline,
            sky_pipeline,
            depth_view,
            camera_buffer,
            light_buffer,
            frame_group,
            draw_layout,
            draw_buffer,
            draw_group,
            draw_capacity: INITIAL_DRAW_CAPACITY,
            draw_stride,
        }
    }

    /// Recreate the depth buffer for a resized target. Call whenever the target's size changes;
    /// rendering into a target whose size disagrees with the depth buffer is a wgpu validation
    /// error.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.depth_view = pipeline::depth_texture(device, width, height);
    }

    /// Draw one frame into `target`: the gradient sky first, then every item in `items`,
    /// cel-shaded under `light`'s sun and fogged by its fog. The caller owns the encoder and
    /// submission, so a frame can compose other passes around this one.
    ///
    /// `items` is the whole contract: wok-render reads no stores and no pools, and draws exactly
    /// what it is handed, in order, with no culling.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        camera: &Camera,
        light: &LightState,
        items: &[RenderItem],
    ) {
        if items.len() > self.draw_capacity {
            self.draw_capacity = items.len().next_power_of_two();
            let (buffer, group) =
                draw_resources(device, &self.draw_layout, self.draw_stride, self.draw_capacity);
            self.draw_buffer = buffer;
            self.draw_group = group;
        }

        let camera_floats = uniforms::camera_floats(camera);
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&camera_floats));
        let light_floats = uniforms::light_floats(light);
        queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&light_floats));

        if !items.is_empty() {
            let stride = self.draw_stride as usize;
            let block = uniforms::DRAW_UNIFORM_SIZE as usize;
            let mut draws = vec![0u8; items.len() * stride];
            for (i, item) in items.iter().enumerate() {
                let floats = uniforms::draw_floats(item.transform, item.color);
                draws[i * stride..i * stride + block].copy_from_slice(bytemuck::cast_slice(&floats));
            }
            queue.write_buffer(&self.draw_buffer, 0, &draws);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wok_render_forward_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The sky covers every pixel, so the clear color never survives; black makes
                    // a missing sky pass obvious.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_pipeline(&self.sky_pipeline);
        pass.draw(0..3, 0..1);

        pass.set_pipeline(&self.mesh_pipeline);
        for (i, item) in items.iter().enumerate() {
            pass.set_bind_group(1, &self.draw_group, &[(i as u64 * self.draw_stride) as u32]);
            pass.set_vertex_buffer(0, item.mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(item.mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..item.mesh.index_count, 0, 0..1);
        }
    }
}

fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The per-draw buffer and its bind group. The binding window is one block
/// ([`uniforms::DRAW_UNIFORM_SIZE`]) wide; the dynamic offset slides it through the buffer, one
/// stride per item.
fn draw_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wok_render_draws"),
        size: stride * capacity as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("wok_render_draw_group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(uniforms::DRAW_UNIFORM_SIZE),
            }),
        }],
    });
    (buffer, group)
}
