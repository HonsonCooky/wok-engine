//! Construction of the renderer's wgpu objects: bind group layouts, the three render pipelines,
//! the depth texture, and the shadow map.
//!
//! Bind group structure, shared across pipelines so group 0 binds once per frame:
//! - group 0: the frame uniforms (camera at binding 0, light at binding 1).
//! - group 1 (mesh and shadow pipelines): the per-draw block, one uniform binding with a dynamic
//!   offset so every item in the render list shares a single buffer and bind group.
//! - group 2 (mesh pipeline only): the shadow map and its comparison sampler. The shadow pass
//!   cannot bind these (it renders into the map), which is why they sit in their own group.
//!
//! Shader modules are built by concatenating the shared bindings file with the pass-specific
//! file, so the `Camera`, `Light`, and `Draw` declarations exist in exactly one source file.

use wok_platform::wgpu;

use crate::uniforms::{CAMERA_UNIFORM_SIZE, DRAW_UNIFORM_SIZE, LIGHT_UNIFORM_SIZE};

/// The depth buffer format. 32-bit float depth, no stencil: nothing in the forward pipeline
/// stencils, and a single fixed format keeps the one-target rule simple. The shadow map shares
/// the format so depth behavior is one decision.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Depth bias on the shadow pipeline: the counter to shadow acne (a surface shadowing itself
/// because its rasterized depth and its sampled depth quantize differently). `constant` is in
/// units of the smallest representable depth difference at each fragment; `slope_scale`
/// multiplies the polygon's depth gradient, which is what keeps sun-grazing slopes acne-free
/// where a constant alone would need to be huge. These values are tuned against the default 2048
/// map fitted to chunk-scale regions (tens to a couple hundred metres); if scenes change scale
/// and acne (too little bias) or peter-panning - shadows detaching from their casters (too much)
/// appears, this is the knob.
const SHADOW_BIAS: wgpu::DepthBiasState =
    wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 };

const COMMON_WGSL: &str = include_str!("shaders/common.wgsl");
const MESH_WGSL: &str = include_str!("shaders/mesh.wgsl");
const SKY_WGSL: &str = include_str!("shaders/sky.wgsl");
const SHADOW_WGSL: &str = include_str!("shaders/shadow.wgsl");
const LINE_WGSL: &str = include_str!("shaders/line.wgsl");

/// Bytes per debug line vertex: world position (3 x f32) plus color (3 x f32), interleaved. Local
/// to wok-render: line vertices are built per frame from `LineSegment`s, never from a `MeshCpu`,
/// so wok-mesh's layout does not apply.
pub(crate) const LINE_VERTEX_STRIDE: u64 = 24;

const LINE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

const LINE_VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: LINE_VERTEX_STRIDE,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &LINE_VERTEX_ATTRIBUTES,
};

/// Group 0: per-frame camera and light uniforms.
pub(crate) fn frame_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("wok_render_frame_layout"),
        entries: &[
            uniform_entry(0, CAMERA_UNIFORM_SIZE, false),
            uniform_entry(1, LIGHT_UNIFORM_SIZE, false),
        ],
    })
}

/// Group 1: the per-draw block, bound at a dynamic offset per item.
pub(crate) fn draw_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("wok_render_draw_layout"),
        entries: &[uniform_entry(0, DRAW_UNIFORM_SIZE, true)],
    })
}

/// Group 2: the shadow map and its comparison sampler, read by the mesh pass's fragment stage.
pub(crate) fn shadow_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("wok_render_shadow_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    })
}

fn uniform_entry(binding: u32, size: u64, dynamic_offset: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic_offset,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

/// The forward mesh pipeline: wok-mesh's vertex layout in, cel-shaded and fogged color out.
/// Back faces cull against the crate-wide counter-clockwise winding wok-mesh guarantees.
pub(crate) fn mesh_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    frame_layout: &wgpu::BindGroupLayout,
    draw_layout: &wgpu::BindGroupLayout,
    shadow_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wok_render_mesh_shader"),
        source: wgpu::ShaderSource::Wgsl(format!("{COMMON_WGSL}\n{MESH_WGSL}").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wok_render_mesh_pipeline_layout"),
        bind_group_layouts: &[frame_layout, draw_layout, shadow_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("wok_render_mesh_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_mesh"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wok_mesh::VERTEX_LAYOUT],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_mesh"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// The sky pipeline: one fullscreen triangle, no vertex buffer, drawn before geometry. It never
/// tests or writes depth (the pass clears depth to 1.0 and geometry draws over the sky), but it
/// must still declare the pass's depth format to be compatible with the shared render pass.
pub(crate) fn sky_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    frame_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wok_render_sky_shader"),
        source: wgpu::ShaderSource::Wgsl(format!("{COMMON_WGSL}\n{SKY_WGSL}").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wok_render_sky_pipeline_layout"),
        bind_group_layouts: &[frame_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("wok_render_sky_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_sky"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_sky"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// The shadow depth pass: every mesh item rendered from the sun's view into the shadow map,
/// depth only - no fragment stage, no color targets. Back faces cull exactly as the forward pass
/// does: front-face culling would fight acne more cheaply, but single-sided geometry (the Plane
/// primitive, terrain) must cast too ("everything casts", no per-object toggles), so the depth
/// bias ([`SHADOW_BIAS`]) does that work instead.
pub(crate) fn shadow_pipeline(
    device: &wgpu::Device,
    frame_layout: &wgpu::BindGroupLayout,
    draw_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wok_render_shadow_shader"),
        source: wgpu::ShaderSource::Wgsl(format!("{COMMON_WGSL}\n{SHADOW_WGSL}").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wok_render_shadow_pipeline_layout"),
        bind_group_layouts: &[frame_layout, draw_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("wok_render_shadow_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wok_mesh::VERTEX_LAYOUT],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: SHADOW_BIAS,
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview: None,
        cache: None,
    })
}

/// The debug line pipeline: LineList topology, unlit vertex color out, drawn after the meshes in
/// the frame's forward pass output. Depth-tested against the depth the mesh pass wrote (hidden
/// geometry hides its lines too) but compared LessEqual and not written: a line traced exactly on
/// a surface - an AABB edge on a box face - must not lose the tie to the face that defines it,
/// and lines have no later pass to occlude. No culling: a line has no winding.
pub(crate) fn line_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    frame_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wok_render_line_shader"),
        source: wgpu::ShaderSource::Wgsl(format!("{COMMON_WGSL}\n{LINE_WGSL}").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wok_render_line_pipeline_layout"),
        bind_group_layouts: &[frame_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("wok_render_line_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_line"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[LINE_VERTEX_LAYOUT],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_line"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Create the square shadow map of `size` texels per side: a depth texture the shadow pass
/// renders into and the mesh pass samples. Created once at renderer construction; unlike the
/// depth buffer it never follows the window size.
pub(crate) fn shadow_texture(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wok_render_shadow_map"),
        size: wgpu::Extent3d { width: size.max(1), height: size.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The shadow map's comparison sampler. Linear filtering on a comparison sampler is hardware PCF:
/// each tap blends the pass/fail results of its 2x2 footprint, which the shader's 3x3 tap grid
/// spreads further. Clamped to edge; the shader's own bounds guard decides what happens outside
/// the map (lit), the clamp just keeps edge taps well-defined.
pub(crate) fn shadow_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("wok_render_shadow_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    })
}

/// Create the depth buffer for a `width` x `height` target. Dimensions clamp to 1 so a minimized
/// window cannot request a zero-sized texture.
pub(crate) fn depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wok_render_depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
