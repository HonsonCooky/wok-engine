//! The debug line pass: pipeline build and the smoke for both depth modes.

use glam::{Mat4, Vec3};
use wok_light::LightState;
use wok_mesh::MeshGpu;
use wok_platform::wgpu;
use wok_render::{Camera, DepthMode, LineSegment, RenderItem, Renderer};
use wok_scene::Aabb;

use crate::{FORMAT, SIZE, assert_no_validation_error, device, render_frame};

#[test]
fn line_pipeline_builds_without_validation_errors() {
    // The line shader validates and the whole line path - pipeline, buffer, pass recording, draw -
    // records and submits a frame without a validation error. Structural; pixels are the smoke
    // test below.
    let (device, queue) = device();
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let common = include_str!("../../src/shaders/common.wgsl");
    let line = include_str!("../../src/shaders/line.wgsl");
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
        opacity: 1.0,
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
