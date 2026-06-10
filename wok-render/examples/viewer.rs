//! Visual verification vehicle for wok-render.
//!
//! Opens a wok-platform window and renders a fixture scene built in code: a grid of the five
//! placeholder primitives at increasing scales standing on a terrain mesh generated from a
//! synthetic heightmap, all cel-shaded under a hand-built `LightState` with fog, the gradient
//! sky, and the sun's shadow map (primitives cast onto the terrain and each other; the fixture's
//! bounds are the shadow region). The camera orbits the scene slowly. No input handling; close
//! the window to exit.
//!
//! Run with: cargo run -p wok-render --example viewer

use glam::{Mat4, Vec3};
use wok_light::{CelParams, Fog, LightState, SkyGradient, Sun};
use wok_mesh::{MeshGpu, primitive_mesh, terrain_mesh};
use wok_platform::{App, Desc, FrameCtx, Platform, gfx, run};
use wok_render::{Camera, RenderItem, Renderer};
use wok_scene::{Aabb, CHUNK_GRID_DIM, CHUNK_GRID_LEN, Heightmap, Primitive, SurfaceTag};

// One row per primitive kind, one column per scale, standing on the terrain.
const ROWS: [(Primitive, Vec3); 5] = [
    (Primitive::Cube, Vec3::new(0.85, 0.30, 0.25)),
    (Primitive::Ellipsoid, Vec3::new(0.95, 0.65, 0.20)),
    (Primitive::Cylinder, Vec3::new(0.90, 0.85, 0.30)),
    (Primitive::Capsule, Vec3::new(0.35, 0.75, 0.40)),
    (Primitive::Plane, Vec3::new(0.45, 0.55, 0.90)),
];
const SCALES: [f32; 4] = [2.0, 3.0, 4.0, 5.0];
const TERRAIN_COLOR: Vec3 = Vec3::new(0.40, 0.60, 0.35);

// The grid sits near the chunk center so the orbit camera always has terrain behind it.
const GRID_ORIGIN: (f32, f32) = (40.0, 36.0);
const COLUMN_SPACING: f32 = 12.0;
const ROW_SPACING: f32 = 14.0;

const ORBIT_CENTER_XZ: f32 = 64.0;
const ORBIT_RADIUS: f32 = 38.0;
const ORBIT_HEIGHT: f32 = 16.0;
const ORBIT_SPEED: f32 = 0.15; // radians per second

fn main() {
    run(
        Viewer { renderer: None, fixture: None, size: (0, 0), angle: 0.0 },
        Desc { title: "wok-render viewer", width: 0, height: 0, vsync: true },
    );
}

/// Rolling hills well inside the +/-32m height range: two low-frequency waves summed, so slopes
/// show the cel bands and the horizon shows fog over terrain.
fn synthetic_heightmap() -> Heightmap {
    let mut heights = Vec::with_capacity(CHUNK_GRID_LEN);
    for z in 0..CHUNK_GRID_DIM {
        for x in 0..CHUNK_GRID_DIM {
            let (xf, zf) = (x as f32, z as f32);
            let h = 3.0 * (xf * 0.07).sin() * (zf * 0.05).cos() + 1.5 * ((xf + zf) * 0.045).sin();
            heights.push(Heightmap::meters_to_raw(h));
        }
    }
    Heightmap::new(heights, vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
        .expect("synthetic heightmap grids are the right length by construction")
}

fn fixture_light() -> LightState {
    LightState {
        sun: Sun {
            direction: Vec3::new(-0.4, -1.0, -0.3),
            color: Vec3::new(1.0, 0.95, 0.85),
        },
        ambient: Vec3::new(0.12, 0.12, 0.16),
        // The horizon matches the fog color (HLD: fog color drives the sky's horizon), so distant
        // terrain dissolves into the sky instead of meeting it at a seam.
        fog: Fog { color: Vec3::new(0.65, 0.70, 0.80), start: 40.0, end: 180.0 },
        sky: SkyGradient {
            horizon: Vec3::new(0.65, 0.70, 0.80),
            zenith: Vec3::new(0.25, 0.45, 0.85),
        },
        cel: CelParams { band_count: 4, transition_softness: 0.08, rim_intensity: 0.35 },
    }
}

/// The uploaded meshes, the static placements that reference them by index, and the fixture's
/// world bounds (the shadow region the frame call passes). Built once in `init`; the per-frame
/// render list borrows from `meshes`.
struct Fixture {
    meshes: Vec<MeshGpu>,
    placements: Vec<(usize, Mat4, Vec3)>,
    orbit_center: Vec3,
    bounds: Aabb,
}

fn build_fixture(platform: &Platform) -> Fixture {
    let terrain = synthetic_heightmap();
    let terrain_cpu = terrain_mesh(&terrain);
    // The shadow region starts as the terrain's bounds and grows over each placement: every
    // placement here is an axis-aligned unit primitive, so its box is the center +/- half the
    // scale on each axis (conservative for the plane's flat y, which is fine for a fit).
    let mut bounds = terrain_cpu.bounds();
    let mut meshes = Vec::new();
    let mut placements = Vec::new();

    for (row, (primitive, color)) in ROWS.iter().enumerate() {
        meshes.push(MeshGpu::upload(&platform.device, &primitive_mesh(*primitive)));
        for (col, scale) in SCALES.iter().enumerate() {
            let x = GRID_ORIGIN.0 + col as f32 * COLUMN_SPACING;
            let z = GRID_ORIGIN.1 + row as f32 * ROW_SPACING;
            // Volumetric primitives sit on the ground (their unit shapes span +/-0.5, so half the
            // scale is below center); the flat plane floats above it to stay visible.
            let y = match primitive {
                Primitive::Plane => terrain.height_at(x, z) + 1.5,
                _ => terrain.height_at(x, z) + 0.5 * scale + 0.05,
            };
            let center = Vec3::new(x, y, z);
            let transform = Mat4::from_scale_rotation_translation(
                Vec3::splat(*scale),
                glam::Quat::IDENTITY,
                center,
            );
            placements.push((row, transform, *color));
            bounds.min = bounds.min.min(center - Vec3::splat(0.5 * scale));
            bounds.max = bounds.max.max(center + Vec3::splat(0.5 * scale));
        }
    }

    let terrain_index = meshes.len();
    meshes.push(MeshGpu::upload(&platform.device, &terrain_cpu));
    placements.push((terrain_index, Mat4::IDENTITY, TERRAIN_COLOR));

    let center_height = terrain.height_at(ORBIT_CENTER_XZ, ORBIT_CENTER_XZ);
    Fixture {
        meshes,
        placements,
        orbit_center: Vec3::new(ORBIT_CENTER_XZ, center_height + 2.0, ORBIT_CENTER_XZ),
        bounds,
    }
}

struct Viewer {
    renderer: Option<Renderer>,
    fixture: Option<Fixture>,
    size: (u32, u32),
    angle: f32,
}

impl App for Viewer {
    fn init(&mut self, platform: &Platform) {
        let config = &platform.surface_config;
        self.renderer = Some(Renderer::new(
            &platform.device,
            config.format,
            config.width,
            config.height,
        ));
        self.size = (config.width, config.height);
        self.fixture = Some(build_fixture(platform));
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        self.angle += ORBIT_SPEED * ctx.dt;
        let (Some(renderer), Some(fixture)) = (self.renderer.as_mut(), self.fixture.as_ref())
        else {
            return;
        };

        if ctx.width > 0 && ctx.height > 0 && (ctx.width, ctx.height) != self.size {
            renderer.resize(&ctx.platform.device, ctx.width, ctx.height);
            self.size = (ctx.width, ctx.height);
        }

        let eye = fixture.orbit_center
            + Vec3::new(
                ORBIT_RADIUS * self.angle.cos(),
                ORBIT_HEIGHT,
                ORBIT_RADIUS * self.angle.sin(),
            );
        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        let projection = Mat4::perspective_rh(60f32.to_radians(), aspect, 0.1, 400.0);
        let view = Mat4::look_at_rh(eye, fixture.orbit_center, Vec3::Y);
        let camera = Camera { view_proj: projection * view, eye };

        let items: Vec<RenderItem> = fixture
            .placements
            .iter()
            .map(|&(mesh, transform, color)| RenderItem {
                transform,
                mesh: &fixture.meshes[mesh],
                color,
            })
            .collect();

        let Some(mut frame) = gfx::begin_frame(ctx.platform) else {
            return;
        };
        renderer.render(
            &ctx.platform.device,
            &ctx.platform.queue,
            &mut frame.encoder,
            &frame.view,
            &camera,
            &fixture_light(),
            fixture.bounds,
            &items,
        );
        frame.finish(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}
