//! Level 1 GPU tests, headless: a wgpu adapter requested without a surface, so they run under
//! plain `cargo test` with no window. Coverage is structural only - shaders validate, the
//! pipeline builds, and a rendered frame lands geometry - because exact pixel comparison is
//! Level 3's screenshot diff with tolerances, later.

use glam::{Mat4, Vec3};
use wok_light::{CelParams, Fog, LightState, SkyGradient, Sun};
use wok_mesh::MeshGpu;
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

#[test]
fn shader_modules_compile_and_validate() {
    let (device, _queue) = device();
    let common = include_str!("../src/shaders/common.wgsl");
    let passes = [
        ("mesh", include_str!("../src/shaders/mesh.wgsl")),
        ("sky", include_str!("../src/shaders/sky.wgsl")),
        ("shadow", include_str!("../src/shaders/shadow.wgsl")),
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
    let region = Aabb::new(Vec3::splat(-3.0), Vec3::splat(3.0));

    let sky_only = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &[], DepthMode::Tested,
    );

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
    let with_cube = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &items, &[], DepthMode::Tested,
    );
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

#[test]
fn shadow_smoke_a_floating_cube_darkens_the_plane_behind_it() {
    // Structural, not pixel-exact (Level 3 owns exact pixels later): a cube floats above a large
    // plane under a low sun travelling toward +x, so the cube's shadow falls on the plane's +x
    // side. The plane region there must read darker than the mirror point on the -x side, which
    // the same sun lights unoccluded. Rim is zeroed so the comparison sees only the sun term, and
    // fog starts far beyond the scene.
    let (device, queue) = device();
    let mut renderer = Renderer::new(&device, FORMAT, SIZE, SIZE);

    let light = LightState {
        sun: Sun { direction: Vec3::new(1.0, -1.0, 0.0), color: Vec3::ONE },
        ambient: Vec3::splat(0.1),
        fog: Fog { color: Vec3::splat(0.7), start: 500.0, end: 1000.0 },
        sky: SkyGradient { horizon: Vec3::splat(0.7), zenith: Vec3::new(0.3, 0.5, 0.9) },
        cel: CelParams { band_count: 4, transition_softness: 0.05, rim_intensity: 0.0 },
    };

    // Straight down from 20m; up is +Z because the view direction is -Y.
    let eye = Vec3::new(0.0, 20.0, 0.0);
    let projection = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Z);
    let camera = Camera { view_proj: projection * view, eye };

    let plane = MeshGpu::upload(&device, &wok_mesh::plane());
    let cube = MeshGpu::upload(&device, &wok_mesh::cube());
    let items = [
        RenderItem {
            transform: Mat4::from_scale(Vec3::new(24.0, 1.0, 24.0)),
            mesh: &plane,
            color: Vec3::splat(0.8),
        },
        RenderItem {
            transform: Mat4::from_scale_rotation_translation(
                Vec3::splat(2.0),
                glam::Quat::IDENTITY,
                Vec3::new(0.0, 3.0, 0.0),
            ),
            mesh: &cube,
            color: Vec3::splat(0.8),
        },
    ];
    let region = Aabb::new(Vec3::new(-12.0, -0.5, -12.0), Vec3::new(12.0, 4.5, 12.0));

    let pixels = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &items, &[], DepthMode::Tested,
    );

    // The cube (2m wide, bottom at 2m, top at 4m) under a 45 degree sun shadows the plane across
    // x in roughly [1, 5]; (3.5, 0, 0) sits inside that band and clear of the cube's screen
    // footprint from a straight-down camera. (-3.5, 0, 0) is its unoccluded mirror.
    let (sx, sy) = pixel_of(&camera, Vec3::new(3.5, 0.0, 0.0));
    let (lx, ly) = pixel_of(&camera, Vec3::new(-3.5, 0.0, 0.0));
    let shadowed = block_luminance(&pixels, sx, sy);
    let lit = block_luminance(&pixels, lx, ly);
    assert!(
        lit > 60.0,
        "the unoccluded plane reads at luminance {lit}; the sun pass did not light it"
    );
    assert!(
        shadowed < lit * 0.75,
        "shadowed side {shadowed} vs lit side {lit}: the cube cast no measurable shadow"
    );
}

#[test]
fn line_pipeline_builds_without_validation_errors() {
    // The line shader validates and the whole line path - pipeline, buffer, pass recording, draw -
    // records and submits a frame without a validation error. Structural; pixels are the smoke
    // test below.
    let (device, queue) = device();
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let common = include_str!("../src/shaders/common.wgsl");
    let line = include_str!("../src/shaders/line.wgsl");
    let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("line"),
        source: wgpu::ShaderSource::Wgsl(format!("{common}\n{line}").into()),
    });
    assert_no_validation_error(&device, "line shader");

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut renderer = Renderer::new(&device, FORMAT, SIZE, SIZE);
    let eye = Vec3::new(0.0, 0.0, 6.0);
    let camera = Camera {
        view_proj: Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 400.0)
            * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y),
        eye,
    };
    let lines = [LineSegment { start: Vec3::NEG_X, end: Vec3::X, color: Vec3::ONE }];
    let region = Aabb::new(Vec3::splat(-3.0), Vec3::splat(3.0));
    let _frame = render_frame(
        &device, &queue, &mut renderer, &camera, &LightState::default(), region, &[], &lines,
        DepthMode::Tested,
    );
    assert_no_validation_error(&device, "render_lines");

    // The x-ray variant records and submits cleanly too: same shader, the other depth compare.
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _frame = render_frame(
        &device, &queue, &mut renderer, &camera, &LightState::default(), region, &[], &lines,
        DepthMode::XRay,
    );
    assert_no_validation_error(&device, "render_lines x-ray");
}

#[test]
fn line_smoke_a_line_lands_its_pixels_in_its_own_color() {
    // A pure red horizontal line across the view, over the sky alone (nothing occludes it). The
    // frame must differ from the line-less frame along a line's worth of pixels, and those pixels
    // must be the line's color verbatim: unlit and unfogged means nothing modulates it.
    let (device, queue) = device();
    let mut renderer = Renderer::new(&device, FORMAT, SIZE, SIZE);

    let eye = Vec3::new(0.0, 0.0, 6.0);
    let camera = Camera {
        view_proj: Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 400.0)
            * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y),
        eye,
    };
    let light = LightState::default();
    let region = Aabb::new(Vec3::splat(-3.0), Vec3::splat(3.0));

    let without = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &[], DepthMode::Tested,
    );
    let lines = [LineSegment {
        start: Vec3::new(-3.0, 0.0, 0.0),
        end: Vec3::new(3.0, 0.0, 0.0),
        color: Vec3::new(1.0, 0.0, 0.0),
    }];
    let with = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &lines, DepthMode::Tested,
    );

    let line_pixels = without
        .chunks_exact(4)
        .zip(with.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .map(|(_, b)| b)
        .collect::<Vec<_>>();
    // A 6m line spanning the 60 degree frustum at 6m crosses the full SIZE-wide frame; require a
    // generous fraction so the assertion is structural rather than rasterization-exact.
    assert!(
        line_pixels.len() > (SIZE / 2) as usize,
        "only {} pixels changed; the line did not land",
        line_pixels.len()
    );
    for px in line_pixels {
        assert_eq!(&px[0..3], &[255, 0, 0], "line pixel is not the authored color: {px:?}");
    }
}

#[test]
fn xray_smoke_a_line_behind_a_mesh_still_lands_its_pixels() {
    // The mode pair's whole point, exercised from both sides: a red line entirely inside a cube's
    // screen footprint and behind it in depth must vanish depth-tested (the control: occlusion
    // works) and land its pixels verbatim x-ray (compare Always reads no depth).
    let (device, queue) = device();
    let mut renderer = Renderer::new(&device, FORMAT, SIZE, SIZE);

    let eye = Vec3::new(0.0, 0.0, 6.0);
    let camera = Camera {
        view_proj: Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 400.0)
            * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y),
        eye,
    };
    let light = LightState::default();
    let region = Aabb::new(Vec3::splat(-3.0), Vec3::splat(3.0));

    // The 2m cube spans [-1, 1]^3; the line sits at z = -2 (behind its back face) and spans only
    // half its width, so from the camera on +Z the cube's footprint covers the line completely.
    let cube = MeshGpu::upload(&device, &wok_mesh::cube());
    let items = [RenderItem {
        transform: Mat4::from_scale(Vec3::splat(2.0)),
        mesh: &cube,
        color: Vec3::splat(0.8),
    }];
    let lines = [LineSegment {
        start: Vec3::new(-0.5, 0.0, -2.0),
        end: Vec3::new(0.5, 0.0, -2.0),
        color: Vec3::new(1.0, 0.0, 0.0),
    }];

    let cube_only = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &items, &[], DepthMode::Tested,
    );
    let tested = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &items, &lines, DepthMode::Tested,
    );
    assert_eq!(
        cube_only, tested,
        "a fully occluded depth-tested line changed pixels; the occlusion control is broken"
    );

    let xray = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &items, &lines, DepthMode::XRay,
    );
    let line_pixels = cube_only
        .chunks_exact(4)
        .zip(xray.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .map(|(_, b)| b)
        .collect::<Vec<_>>();
    // The 1m line is 8m from the eye; under the 60 degree fov that projects to roughly a tenth of
    // the SIZE-wide frame. Require a generous fraction so the assertion stays structural.
    assert!(
        line_pixels.len() > (SIZE / 16) as usize,
        "only {} pixels changed; the x-ray line did not land through the cube",
        line_pixels.len()
    );
    for px in line_pixels {
        assert_eq!(&px[0..3], &[255, 0, 0], "x-ray line pixel is not the authored color: {px:?}");
    }
}
