//! The shadow pass: the single-map smoke through the real forward composition.

use glam::{Mat4, Vec3};
use wok_light::{CelParams, Fog, LightState, SkyGradient, Sun};
use wok_mesh::MeshGpu;
use wok_render::{Camera, DepthMode, RenderItem, Renderer};
use wok_scene::Aabb;

use crate::{FORMAT, SIZE, block_luminance, device, pixel_of, render_frame};

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
        fog: Fog { enabled: true, color: Vec3::splat(0.7), start: 500.0, end: 1000.0 },
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
            opacity: 1.0,
        },
        RenderItem {
            transform: Mat4::from_scale_rotation_translation(
                Vec3::splat(2.0),
                glam::Quat::IDENTITY,
                Vec3::new(0.0, 3.0, 0.0),
            ),
            mesh: &cube,
            color: Vec3::splat(0.8),
            opacity: 1.0,
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
