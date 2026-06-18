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

/// A sub-rectangle of the render target, in physical pixels (origin at the top-left): where on the
/// target the frame draws. Set it with [`Renderer::set_viewport`]; the default is the whole target,
/// so a caller that never sets one renders full-frame exactly as before.
///
/// This is "where on the target", not a second configuration (HLD: one target). It scopes only the
/// wgpu viewport and scissor of the colour passes; the camera, light, sky, fog, shadow region, and
/// the offscreen resources (depth buffer, shadow map) are unchanged and stay sized to the full
/// target. A caller confining the view to a sub-rect supplies a camera whose projection already
/// matches the sub-rect's aspect, so geometry sits centred and undistorted within it; this type
/// does not touch the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewportRect {
    /// The clamped integer pixel rect `(x, y, width, height)` this viewport occupies on a
    /// `target_w` x `target_h` target. The wgpu viewport and the scissor both use it, so the two
    /// never disagree by a sub-pixel. The origin floors and the far edges clamp to the target, so a
    /// rect nudged past an edge by rounding (or a stale rect after a resize) trims to fit instead of
    /// raising a wgpu out-of-bounds validation error; an origin at or past an edge yields a
    /// zero-size rect, which the caller draws as nothing.
    fn scissor(self, target_w: u32, target_h: u32) -> (u32, u32, u32, u32) {
        let x = (self.x.max(0.0) as u32).min(target_w);
        let y = (self.y.max(0.0) as u32).min(target_h);
        let right = ((self.x + self.width).max(0.0) as u32).min(target_w);
        let bottom = ((self.y + self.height).max(0.0) as u32).min(target_h);
        (x, y, right.saturating_sub(x), bottom.saturating_sub(y))
    }
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
    // The current target size (width, height), tracked from `new` and `resize` so the viewport
    // clamps to it; the colour-pass scissor must stay inside the attachment.
    target_size: (u32, u32),
    // The sub-rect the colour passes draw into (physical pixels), or `None` for the whole target.
    // Renderer state set per frame by the caller, like the target size `resize` tracks; see
    // `set_viewport` and `ViewportRect`.
    viewport: Option<ViewportRect>,
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
            target_size: (width, height),
            viewport: None,
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

    /// Size the depth buffer to a `width` x `height` target. Rendering into a target whose size
    /// disagrees with the depth buffer is a wgpu validation error, so the caller drives this from
    /// the size of the surface texture it just acquired (`Frame::size`), every frame. The call is
    /// idempotent - it recreates the depth texture only when the size actually changed - so feeding
    /// it the acquired size each frame costs nothing in steady state and keeps depth and colour in
    /// lockstep across a resize.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.target_size == (width, height) {
            return;
        }
        self.depth_view = pipeline::depth_texture(device, width, height);
        self.target_size = (width, height);
    }

    /// Confine subsequent frames to a sub-rect of the target, or pass `None` for the whole target.
    /// Per-frame render input the caller refreshes as its layout changes (a docked panel, a window
    /// resize); it is renderer state, like the target size [`Renderer::resize`] tracks, so a caller
    /// that never calls this renders full-frame and is bit-identical to before. The rect scopes the
    /// viewport and scissor of the colour passes ([`Renderer::render`] and
    /// [`Renderer::render_lines`]); everything else, including the offscreen depth and shadow
    /// resources, is unchanged. See [`ViewportRect`].
    pub fn set_viewport(&mut self, viewport: Option<ViewportRect>) {
        self.viewport = viewport;
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
    ///
    /// The frame draws into the whole target unless a [`ViewportRect`] is set
    /// ([`Renderer::set_viewport`]), which scopes the colour pass to a sub-rect of it.
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

        // Confine the draw to the viewport sub-rect (default: the whole target). The load-op clear
        // above is not scissored - it clears the whole attachment - so outside the sub-rect the
        // target keeps whatever it held; only the sky and geometry are scoped to the rect.
        apply_viewport(&mut pass, self.viewport, self.target_size);
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
        // The same sub-rect the frame's geometry drew into: lines project through the same camera,
        // so an unconfined line pass would scatter overlay lines across the whole target while the
        // geometry sits in the viewport. Default (no viewport set) is the whole target, as before.
        apply_viewport(&mut pass, self.viewport, self.target_size);
        pass.set_pipeline(&self.line_pipelines[slot]);
        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_vertex_buffer(0, self.line_buffers[slot].slice(..));
        pass.draw(0..lines.len() as u32 * 2, 0..1);
    }
}

/// Scope `pass` to `viewport` (a sub-rect of a `target`-sized attachment) by setting the wgpu
/// viewport and scissor together. `None` leaves the pass at its default - the whole target - so the
/// no-viewport path issues no extra commands and is bit-identical to before. A rect whose clamped
/// size is empty (its origin sits at or past an edge) sets a zero scissor, so the pass draws
/// nothing rather than falling back to the full target.
fn apply_viewport(pass: &mut wgpu::RenderPass, viewport: Option<ViewportRect>, target: (u32, u32)) {
    let Some(vp) = viewport else { return };
    let (x, y, w, h) = vp.scissor(target.0, target.1);
    if w == 0 || h == 0 {
        pass.set_scissor_rect(0, 0, 0, 0);
        return;
    }
    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
    pass.set_scissor_rect(x, y, w, h);
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

#[cfg(test)]
mod tests {
    use super::ViewportRect;

    #[test]
    fn scissor_passes_an_in_bounds_rect_through_unchanged() {
        let vp = ViewportRect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
        assert_eq!(vp.scissor(800, 600), (10, 20, 100, 50));
    }

    #[test]
    fn scissor_trims_a_rect_that_spills_past_the_target_edges() {
        // A rect 100px past the right (x 700 + w 200 vs width 800) and 60px past the bottom (y 560 +
        // h 100 vs height 600) keeps its origin and shrinks to the target, never reaching past it.
        let vp = ViewportRect { x: 700.0, y: 560.0, width: 200.0, height: 100.0 };
        assert_eq!(vp.scissor(800, 600), (700, 560, 100, 40));
    }

    #[test]
    fn scissor_origin_at_or_past_an_edge_is_empty() {
        // Origin past the right edge clamps to the edge with zero width - the caller draws nothing
        // rather than spilling across the whole target.
        let vp = ViewportRect { x: 900.0, y: 0.0, width: 50.0, height: 50.0 };
        assert_eq!(vp.scissor(800, 600), (800, 0, 0, 50));
    }

    #[test]
    fn scissor_floors_each_edge_to_whole_pixels() {
        // egui points times pixels_per_point land on fractional pixels; each edge floors
        // independently (origin 10.9 -> 10, right 111.6 -> 111), so the box is whole-pixel and the
        // viewport and scissor agree. Flooring the edges, not the size, keeps adjacent sub-rects
        // touching without a seam.
        let vp = ViewportRect { x: 10.9, y: 20.4, width: 100.7, height: 50.5 };
        assert_eq!(vp.scissor(800, 600), (10, 20, 101, 50));
    }
}
