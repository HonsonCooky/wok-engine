//! Construction of the renderer's wgpu objects: bind group layouts, the two render pipelines,
//! and the depth texture.
//!
//! Bind group structure, shared by both pipelines so group 0 binds once per frame:
//! - group 0: the frame uniforms (camera at binding 0, light at binding 1).
//! - group 1 (mesh pipeline only): the per-draw block, one uniform binding with a dynamic offset
//!   so every item in the render list shares a single buffer and bind group.
//!
//! Shader modules are built by concatenating the shared bindings file with the pass-specific
//! file, so the `Camera` and `Light` declarations exist in exactly one source file.

use wok_platform::wgpu;

use crate::uniforms::{CAMERA_UNIFORM_SIZE, DRAW_UNIFORM_SIZE, LIGHT_UNIFORM_SIZE};

/// The depth buffer format. 32-bit float depth, no stencil: nothing in the forward pipeline
/// stencils, and a single fixed format keeps the one-target rule simple.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const COMMON_WGSL: &str = include_str!("shaders/common.wgsl");
const MESH_WGSL: &str = include_str!("shaders/mesh.wgsl");
const SKY_WGSL: &str = include_str!("shaders/sky.wgsl");

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
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wok_render_mesh_shader"),
        source: wgpu::ShaderSource::Wgsl(format!("{COMMON_WGSL}\n{MESH_WGSL}").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("wok_render_mesh_pipeline_layout"),
        bind_group_layouts: &[frame_layout, draw_layout],
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
