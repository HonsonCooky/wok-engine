//! Level 1 GPU tests, headless: a wgpu adapter requested without a surface, so they run under
//! plain `cargo test` with no window. Coverage is structural only - shaders validate, the
//! pipeline builds, and a rendered frame lands geometry - because exact pixel comparison is
//! Level 3's screenshot diff with tolerances, later.
//!
//! Split by pass, one module each, all sharing this root's harness in a single test binary:
//! `forward` (shader validation, pipeline build, the sky and geometry smoke, the opacity
//! screen-door), `shadow` (the shadow map smoke), and `lines` (the debug line pass in both depth
//! modes).

mod forward;
mod lines;
mod shadow;

use glam::Vec3;
use wok_light::LightState;
use wok_platform::wgpu;
use wok_render::{Camera, DepthMode, LineSegment, RenderItem, Renderer};
use wok_scene::Aabb;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SIZE: u32 = 128;

// A headless device: no window, no surface, any adapter. Panics with a clear message when the
// environment has no usable GPU; this machine is expected to have one, so a missing adapter is a
// real failure rather than a skip.
fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("no headless wgpu adapter available");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
        .expect("failed to open headless wgpu device")
}

fn assert_no_validation_error(device: &wgpu::Device, what: &str) {
    let error = pollster::block_on(device.pop_error_scope());
    assert!(error.is_none(), "{what} raised a validation error: {error:?}");
}

/// Render one frame into an offscreen texture and read the RGBA8 pixels back, with `lines`
/// overlaid after the meshes (under `depth`'s policy) when any are given. SIZE is chosen so a
/// row (SIZE * 4 bytes) already meets wgpu's 256-byte copy row alignment.
fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    camera: &Camera,
    light: &LightState,
    shadow_region: Aabb,
    items: &[RenderItem],
    lines: &[LineSegment],
    depth: DepthMode,
) -> Vec<u8> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("smoke_target"),
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke_readback"),
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("smoke") });
    renderer.render(device, queue, &mut encoder, &view, camera, light, shadow_region, items);
    renderer.render_lines(device, queue, &mut encoder, &view, lines, depth);
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::Maintain::Wait);
    rx.recv().expect("map_async callback dropped").expect("readback map failed");
    let pixels = slice.get_mapped_range().to_vec();
    readback.unmap();
    pixels
}

/// The pixel of a world point under `camera`, for sampling a rendered frame at known geometry.
fn pixel_of(camera: &Camera, world: Vec3) -> (u32, u32) {
    let ndc = camera.view_proj.project_point3(world);
    let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32) as u32;
    let y = ((-ndc.y * 0.5 + 0.5) * SIZE as f32) as u32;
    (x.min(SIZE - 1), y.min(SIZE - 1))
}

/// Mean luminance (plain RGB average) of the 5x5 pixel block centred on `(x, y)`.
fn block_luminance(pixels: &[u8], x: u32, y: u32) -> f32 {
    let mut sum = 0.0;
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let px = (x as i32 + dx).clamp(0, SIZE as i32 - 1) as u32;
            let py = (y as i32 + dy).clamp(0, SIZE as i32 - 1) as u32;
            let at = ((py * SIZE + px) * 4) as usize;
            sum += (pixels[at] as f32 + pixels[at + 1] as f32 + pixels[at + 2] as f32) / 3.0;
        }
    }
    sum / 25.0
}
