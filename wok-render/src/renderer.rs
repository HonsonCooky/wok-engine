//! The [`Renderer`]: owned pipeline state and the per-frame [`Renderer::render`] entry point.

use glam::{Mat4, Vec3};
use wok_light::LightState;
use wok_mesh::MeshGpu;
use wok_platform::bytemuck;
use wok_platform::wgpu;
use wok_scene::Aabb;

use crate::pipeline;
use crate::shadow;
use crate::uniforms;

/// Per-draw buffer slots allocated up front; the buffer grows (power of two) when a frame's
/// render list exceeds the current capacity, so steady-state frames never reallocate.
const INITIAL_DRAW_CAPACITY: usize = 64;

/// Line segments the line vertex buffer holds before growing, same policy as the draw buffer:
/// rewritten every frame, reallocated (power of two) only when a frame's list outgrows it.
const INITIAL_LINE_CAPACITY: usize = 256;

/// Default shadow map resolution (texels per side), used by [`Renderer::new`]. 2048 over a
/// chunk-scale region (~128m) is roughly 6cm per texel before PCF, which reads clean at the
/// engine's fidelity; pass another size via [`Renderer::with_shadow_map_size`] to trade memory
/// against edge sharpness.
pub const DEFAULT_SHADOW_MAP_SIZE: u32 = 2048;

/// The caller's camera for one frame. The caller supplies final matrices: how view and projection
/// compose, and any chunk-origin rebasing, are its policy. `eye` is the camera's world position,
/// which a combined matrix does not expose; fog distance and rim lighting measure from it.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub view_proj: Mat4,
    pub eye: Vec3,
}

/// One entry in the frame's render list: a world transform, a mesh to draw, a flat base color
/// (linear RGB), and an opacity. The transform is final; wok-render applies no chunk or parent
/// composition.
///
/// `opacity` is 1.0 for an ordinary opaque item. Below 1.0 the mesh pass discards fragments on a
/// 4x4 Bayer screen-door pattern in screen space - alpha cutout used as a fade, not blending (the
/// renderer's cutout-only transparency commitment holds; see the crate docs). The pattern is a
/// pure function of the pixel coordinate, so it is stable frame to frame with no temporal noise.
/// Surviving fragments shade and write depth exactly as an opaque item's do, and the item still
/// casts its full shadow (the depth-only shadow pass ignores opacity) - v1 policy, documented in
/// the crate docs. At exactly 1.0 the discard can never fire and the output is bit-identical to
/// an item without the field.
#[derive(Debug)]
pub struct RenderItem<'m> {
    pub transform: Mat4,
    pub mesh: &'m MeshGpu,
    pub color: Vec3,
    pub opacity: f32,
}

/// One debug line segment for [`Renderer::render_lines`]: world-space endpoints and a flat color
/// (linear RGB). Positions are final, exactly as a [`RenderItem`] transform is.
#[derive(Clone, Copy, Debug)]
pub struct LineSegment {
    pub start: Vec3,
    pub end: Vec3,
    pub color: Vec3,
}

/// Depth policy for one [`Renderer::render_lines`] call. The two modes are one depth-compare
/// function apart (LessEqual vs Always; neither writes depth), which is why this is a parameter
/// and not a second method: the variation is an argument's worth of pipeline state, and a
/// parameter makes every call site state whether its lines are scene-anchored or x-ray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthMode {
    /// Depth-tested against the frame's geometry: hidden lines hide. For world-anchored cues
    /// that should behave like scene elements (markers, reticles).
    Tested,
    /// Drawn regardless of the depth buffer (compare Always, still no depth write): the whole
    /// line lands even behind geometry. For diagnostics describing structure the geometry
    /// occludes - a hitbox cage is useless if it hides behind the surface it describes.
    XRay,
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
    shadow_pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    shadow_view: wgpu::TextureView,
    shadow_group: wgpu::BindGroup,
    shadow_map_size: u32,
    camera_buffer: wgpu::Buffer,
    light_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    draw_layout: wgpu::BindGroupLayout,
    draw_buffer: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    draw_capacity: usize,
    draw_stride: u64,
    // Line state is per [`DepthMode`], indexed by `DepthMode as usize`. Separate buffers are
    // load-bearing, not just tidy: `queue.write_buffer` executes at submit, before any recorded
    // pass runs, so a frame drawing both modes through one encoder would have a shared buffer's
    // second write clobber what the first pass draws.
    line_pipelines: [wgpu::RenderPipeline; 2],
    line_buffers: [wgpu::Buffer; 2],
    line_capacities: [usize; 2],
}

impl Renderer {
    /// Build the pipeline state for a `width` x `height` target of `surface_format`, with the
    /// default [`DEFAULT_SHADOW_MAP_SIZE`] shadow map. For on-screen rendering pass the surface
    /// configuration's format and size; for render-to-texture pass the texture's.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Renderer {
        Renderer::with_shadow_map_size(device, surface_format, width, height, DEFAULT_SHADOW_MAP_SIZE)
    }

    /// [`Renderer::new`] with an explicit shadow map resolution (texels per side).
    pub fn with_shadow_map_size(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        shadow_map_size: u32,
    ) -> Renderer {
        let frame_layout = pipeline::frame_layout(device);
        let draw_layout = pipeline::draw_layout(device);
        let shadow_layout = pipeline::shadow_layout(device);
        let mesh_pipeline =
            pipeline::mesh_pipeline(device, surface_format, &frame_layout, &draw_layout, &shadow_layout);
        let sky_pipeline = pipeline::sky_pipeline(device, surface_format, &frame_layout);
        let shadow_pipeline = pipeline::shadow_pipeline(device, &frame_layout, &draw_layout);
        // Indexed by `DepthMode as usize`: Tested then XRay.
        let line_pipelines = [
            pipeline::line_pipeline(
                device, surface_format, &frame_layout,
                "wok_render_line_pipeline", wgpu::CompareFunction::LessEqual,
            ),
            pipeline::line_pipeline(
                device, surface_format, &frame_layout,
                "wok_render_line_xray_pipeline", wgpu::CompareFunction::Always,
            ),
        ];
        let depth_view = pipeline::depth_texture(device, width, height);

        let shadow_view = pipeline::shadow_texture(device, shadow_map_size);
        let shadow_sampler = pipeline::shadow_sampler(device);
        let shadow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wok_render_shadow_group"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

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
            shadow_pipeline,
            depth_view,
            shadow_view,
            shadow_group,
            shadow_map_size,
            camera_buffer,
            light_buffer,
            frame_group,
            draw_layout,
            draw_buffer,
            draw_group,
            draw_capacity: INITIAL_DRAW_CAPACITY,
            draw_stride,
            line_pipelines,
            line_buffers: [
                line_buffer(device, INITIAL_LINE_CAPACITY),
                line_buffer(device, INITIAL_LINE_CAPACITY),
            ],
            line_capacities: [INITIAL_LINE_CAPACITY; 2],
        }
    }

    /// Recreate the depth buffer for a resized target. Call whenever the target's size changes;
    /// rendering into a target whose size disagrees with the depth buffer is a wgpu validation
    /// error.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.depth_view = pipeline::depth_texture(device, width, height);
    }

    /// Draw one frame into `target`: the sun's shadow map first (every item in `items` rendered
    /// depth-only from the sun's view), then the gradient sky, then every item again, cel-shaded
    /// under `light`'s sun - shadowed by the map - and fogged by its fog. The caller owns the
    /// encoder and submission, so a frame can compose other passes around this one.
    ///
    /// `shadow_region` is the world-space box shadows must cover, caller policy: typically the
    /// AABB of the caller's loaded content (terrain plus placements). The sun's orthographic
    /// projection is fitted to it each frame - tight region, sharp shadows - and surface outside
    /// it renders unshadowed. Everything in `items` casts and receives, terrain included; the sky
    /// does neither; there are no per-object toggles (HLD: one shadow map per frame).
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
        shadow_region: Aabb,
        items: &[RenderItem],
    ) {
        if items.len() > self.draw_capacity {
            self.draw_capacity = items.len().next_power_of_two();
            let (buffer, group) =
                draw_resources(device, &self.draw_layout, self.draw_stride, self.draw_capacity);
            self.draw_buffer = buffer;
            self.draw_group = group;
        }

        let sun_view_proj =
            shadow::sun_view_proj(uniforms::sun_direction(light), shadow_region, self.shadow_map_size);
        let camera_floats = uniforms::camera_floats(camera, sun_view_proj);
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&camera_floats));
        let light_floats = uniforms::light_floats(light);
        queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&light_floats));

        if !items.is_empty() {
            let stride = self.draw_stride as usize;
            let block = uniforms::DRAW_UNIFORM_SIZE as usize;
            let mut draws = vec![0u8; items.len() * stride];
            for (i, item) in items.iter().enumerate() {
                let floats = uniforms::draw_floats(item.transform, item.color, item.opacity);
                draws[i * stride..i * stride + block].copy_from_slice(bytemuck::cast_slice(&floats));
            }
            queue.write_buffer(&self.draw_buffer, 0, &draws);
        }

        // The shadow pass: depth-only, into the shadow map, scoped so its pass ends before the
        // forward pass binds the map as a texture.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wok_render_shadow_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.frame_group, &[]);
            for (i, item) in items.iter().enumerate() {
                pass.set_bind_group(1, &self.draw_group, &[(i as u64 * self.draw_stride) as u32]);
                pass.set_vertex_buffer(0, item.mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(item.mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..item.mesh.index_count, 0, 0..1);
            }
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
        pass.set_bind_group(2, &self.shadow_group, &[]);
        for (i, item) in items.iter().enumerate() {
            pass.set_bind_group(1, &self.draw_group, &[(i as u64 * self.draw_stride) as u32]);
            pass.set_vertex_buffer(0, item.mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(item.mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..item.mesh.index_count, 0, 0..1);
        }
    }

    /// Draw debug `lines` over the frame [`Renderer::render`] just produced, into the same
    /// `target` through the same `encoder`. Call it after `render` in the same frame: the pass
    /// loads the forward pass's color and depth instead of clearing, so `depth` decides what the
    /// frame's geometry does to the lines ([`DepthMode::Tested`] hides hidden lines,
    /// [`DepthMode::XRay`] draws through), and the camera uniform `render` uploaded is the one
    /// the lines project through. Lines are unlit, unfogged, and outside the shadow pass
    /// entirely: they neither cast nor receive. Each mode owns a vertex buffer, reused across
    /// frames and rewritten per call (growing like the draw buffer), so a frame may submit one
    /// call of each mode through one encoder; a second call of the same mode in one frame would
    /// overwrite the first's vertices before either pass runs.
    pub fn render_lines(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        lines: &[LineSegment],
        depth: DepthMode,
    ) {
        if lines.is_empty() {
            return;
        }
        let slot = depth as usize;
        if lines.len() > self.line_capacities[slot] {
            self.line_capacities[slot] = lines.len().next_power_of_two();
            self.line_buffers[slot] = line_buffer(device, self.line_capacities[slot]);
        }

        let mut floats: Vec<f32> = Vec::with_capacity(lines.len() * 12);
        for line in lines {
            floats.extend_from_slice(&[
                line.start.x, line.start.y, line.start.z, line.color.x, line.color.y, line.color.z,
                line.end.x, line.end.y, line.end.z, line.color.x, line.color.y, line.color.z,
            ]);
        }
        queue.write_buffer(&self.line_buffers[slot], 0, bytemuck::cast_slice(&floats));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wok_render_line_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.line_pipelines[slot]);
        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_vertex_buffer(0, self.line_buffers[slot].slice(..));
        pass.draw(0..lines.len() as u32 * 2, 0..1);
    }
}

/// The line vertex buffer for `capacity` segments: two [`pipeline::LINE_VERTEX_STRIDE`]-byte
/// vertices per segment, rewritten each frame.
fn line_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wok_render_lines"),
        size: capacity as u64 * 2 * pipeline::LINE_VERTEX_STRIDE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
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
