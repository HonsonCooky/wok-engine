//! The forward pass: shader validation, pipeline build, the sky and geometry smoke, and the
//! opacity screen-door contract.

use glam::{Mat4, Vec3};
use wok_light::LightState;
use wok_mesh::MeshGpu;
use wok_platform::wgpu;
use wok_render::{Camera, DepthMode, RenderItem, Renderer, ViewportRect};
use wok_scene::Aabb;

use crate::{FORMAT, SIZE, assert_no_validation_error, device, render_frame};

#[test]
fn shader_modules_compile_and_validate() {
    let (device, _queue) = device();
    let common = include_str!("../../src/shaders/common.wgsl");
    let passes = [
        ("mesh", include_str!("../../src/shaders/mesh.wgsl")),
        ("sky", include_str!("../../src/shaders/sky.wgsl")),
        ("shadow", include_str!("../../src/shaders/shadow.wgsl")),
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
        opacity: 1.0,
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

#[test]
fn opacity_screen_doors_about_half_the_pixels_and_full_opacity_is_inert() {
    // The per-item opacity contract, structurally: at 0.5 the 4x4 Bayer screen-door keeps half of
    // every fully covered tile, so the faded cube lands roughly half the pixels the opaque one
    // does; and because the fade is cutout (not blending), every kept pixel shades exactly as the
    // opaque cube's and every dropped pixel shows exactly what was behind (the sky). At 1.0 the
    // discard can never fire (the largest threshold is 15.5/16), so a full-opacity frame
    // reproduces bitwise - the inertness that keeps existing callers' output unchanged.
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
    let cube = MeshGpu::upload(&device, &wok_mesh::cube());
    let cube_at = |opacity: f32| {
        [RenderItem {
            transform: Mat4::from_scale(Vec3::splat(2.0)),
            mesh: &cube,
            color: Vec3::new(1.0, 0.1, 0.1),
            opacity,
        }]
    };

    let sky = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &[], DepthMode::Tested,
    );
    let opaque = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &cube_at(1.0), &[], DepthMode::Tested,
    );
    let opaque_again = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &cube_at(1.0), &[], DepthMode::Tested,
    );
    assert_eq!(opaque, opaque_again, "a full-opacity frame must reproduce bitwise");

    let half = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &cube_at(0.5), &[], DepthMode::Tested,
    );

    let mut footprint = 0usize;
    let mut kept = 0usize;
    for ((s, o), h) in sky.chunks_exact(4).zip(opaque.chunks_exact(4)).zip(half.chunks_exact(4)) {
        // Cutout semantics per pixel: the half frame is the opaque frame's pixel (survived) or
        // the sky's pixel (discarded), never a blend of the two.
        assert!(h == o || h == s, "a faded pixel is neither the cube's nor the sky's: {h:?}");
        if o != s {
            footprint += 1;
            if h != s {
                kept += 1;
            }
        }
    }
    assert!(footprint > 500, "the opaque cube covered only {footprint} pixels");
    let ratio = kept as f32 / footprint as f32;
    assert!(
        (0.4..=0.6).contains(&ratio),
        "opacity 0.5 kept {kept} of {footprint} cube pixels ({ratio}); expected about half"
    );
}

#[test]
fn viewport_confines_the_frame_to_its_sub_rect_and_none_restores_full_target() {
    // The viewport-rect contract, structurally: an explicit rect carries through (the sky and
    // geometry land only inside it, the rest of the target keeps the clear), and clearing it back to
    // None restores the full-target frame bitwise (the inertness taste relies on).
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

    // Full target: the sky pass covers every pixel, the left half included.
    let full = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &[], DepthMode::Tested,
    );

    // Confine to the right half. The forward pass still clears the whole attachment to black (the
    // load-op clear is not scissored), then the viewport and scissor keep the sky and geometry inside
    // the right half, so the left half stays at the black clear.
    let half = SIZE / 2;
    renderer.set_viewport(Some(ViewportRect { x: half as f32, y: 0.0, width: half as f32, height: SIZE as f32 }));
    let confined = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &[], DepthMode::Tested,
    );

    // Clearing it back to the whole target reproduces the first frame bitwise.
    renderer.set_viewport(None);
    let full_again = render_frame(
        &device, &queue, &mut renderer, &camera, &light, region, &[], &[], DepthMode::Tested,
    );
    assert_eq!(full, full_again, "clearing the viewport must restore the full-target frame bitwise");

    let is_black = |frame: &[u8], x: u32, y: u32| {
        let at = ((y * SIZE + x) * 4) as usize;
        frame[at] == 0 && frame[at + 1] == 0 && frame[at + 2] == 0
    };
    // The confined frame left its left half untouched (the black clear) where the full frame painted
    // the same pixels with sky - so the viewport, not the scene, moved them.
    for y in (0..SIZE).step_by(7) {
        for x in (0..half).step_by(7) {
            assert!(is_black(&confined, x, y), "the viewport leaked a draw into the left half at ({x}, {y})");
        }
    }
    assert!(!is_black(&full, half / 2, SIZE / 2), "the full frame should paint the left half (sky)");
    assert!(!is_black(&confined, half + half / 2, SIZE / 2), "the viewport drew nothing inside its rect");
}
