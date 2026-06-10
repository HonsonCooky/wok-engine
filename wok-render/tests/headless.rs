//! Level 1 GPU tests, headless: a wgpu adapter requested without a surface, so they run under
//! plain `cargo test` with no window. Coverage is structural only - shaders validate, the
//! pipeline builds, and a rendered frame lands geometry - because exact pixel comparison is
//! Level 3's screenshot diff with tolerances, later.

use glam::{Mat4, Vec3};
use wok_light::LightState;
use wok_mesh::MeshGpu;
use wok_platform::wgpu;
use wok_render::{Camera, RenderItem, Renderer};

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

#[test]
fn shader_modules_compile_and_validate() {
    let (device, _queue) = device();
    let common = include_str!("../src/shaders/common.wgsl");
    let passes = [
        ("mesh", include_str!("../src/shaders/mesh.wgsl")),
        ("sky", include_str!("../src/shaders/sky.wgsl")),
    ];
    for (name, body) in passes {
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(format!("{common}\n{body}").into()),
        });
        assert_no_validation_error(&device, name);
    }
}

#[test]
fn renderer_builds_without_validation_errors() {
    let (device, _queue) = device();
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _renderer = Renderer::new(&device, FORMAT, SIZE, SIZE);
    assert_no_validation_error(&device, "Renderer::new");
}

/// Render one frame into an offscreen texture and read the RGBA8 pixels back. SIZE is chosen so
/// a row (SIZE * 4 bytes) already meets wgpu's 256-byte copy row alignment.
fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    camera: &Camera,
    light: &LightState,
    items: &[RenderItem],
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
    renderer.render(device, queue, &mut encoder, &view, camera, light, items);
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

#[test]
fn render_smoke_sky_gradient_and_geometry_land() {
    let (device, queue) = device();
    let mut renderer = Renderer::new(&device, FORMAT, SIZE, SIZE);

    // Camera 6m back on +Z looking at the origin; the default LightState has distinct horizon and
    // zenith colors and its fog starts at 50m, so the cube at 6m is essentially unfogged.
    let eye = Vec3::new(0.0, 0.0, 6.0);
    let projection = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 400.0);
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
    let camera = Camera { view_proj: projection * view, eye };
    let light = LightState::default();

    let sky_only = render_frame(&device, &queue, &mut renderer, &camera, &light, &[]);

    // Structural property 1: the sky pass produced a vertical gradient, not a uniform clear.
    let first = &sky_only[0..4];
    assert!(
        sky_only.chunks_exact(4).any(|pixel| pixel != first),
        "sky-only frame is uniform; the gradient pass did not land"
    );

    // Structural property 2: adding geometry changes a meaningful number of pixels.
    let cube = MeshGpu::upload(&device, &wok_mesh::cube());
    let items = [RenderItem {
        transform: Mat4::from_scale(Vec3::splat(2.0)),
        mesh: &cube,
        color: Vec3::new(1.0, 0.1, 0.1),
    }];
    let with_cube = render_frame(&device, &queue, &mut renderer, &camera, &light, &items);
    let differing = sky_only
        .chunks_exact(4)
        .zip(with_cube.chunks_exact(4))
        .filter(|(sky, cube)| sky != cube)
        .count();
    // A 2m cube 6m away under a 60 degree fov covers roughly a tenth of the frame; require a
    // generous fraction of that so the assertion is structural, not pixel-exact.
    assert!(
        differing > 500,
        "geometry changed only {differing} pixels; expected the cube to cover far more"
    );
}
