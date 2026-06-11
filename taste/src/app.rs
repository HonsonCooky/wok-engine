//! The taste application: GPU residency, the frame loop, and the fixed-step bridge.
//!
//! Composition only, per the HLD's application layer. Each rendered frame: map raw input to intent,
//! bank the frame time and run however many fixed simulation steps it covers (`crate::clock`), then
//! advance the follow camera at the frame rate and draw. The simulation advances only inside the
//! fixed steps; the draw samples it continuously: the player is drawn at - and the camera targets -
//! the position interpolated between the previous and current sim states by the accumulator's
//! progress (`FixedClock::alpha`, applied by `sim::lerp_position`), so a frame landing mid-step
//! shows mid-step motion instead of snapping to whichever step boundary the frame caught, the
//! jitter when frame and step rates beat against each other. Camera and player read the same
//! interpolated position, so they share one timeline.
//!
//! Drawing is the editor's render path: the same renderer, the same primitive mesh cache, the same
//! chunk-origin composition of terrain and placements, plus one item the editor does not have - the
//! player, a true capsule mesh generated at the collider's exact dimensions (`wok_mesh::capsule_mesh`
//! paired with `Capsule::upright`) in a color nothing else uses. Prefabs crossing the eye-to-anchor
//! segment draw partially faded (`crate::fade`, taste-only: neither the editor nor the viewer has a
//! player to occlude). F1 cycles the hitbox overlay (`crate::debug`) through its modes - off,
//! depth-tested faces, x-ray drawn shapes, x-ray everything - drawn through the renderer's debug
//! line pass after the meshes, each mode with its own depth policy.

use std::collections::BTreeMap;
use std::error::Error;

use glam::{Mat4, Quat, Vec3};
use wok_content::ChunkStore;
use wok_light::LightState;
use wok_mesh::{MeshGpu, capsule_mesh, primitive_mesh};
use wok_platform::winit::keyboard::NamedKey;
use wok_platform::winit::window::CursorGrabMode;
use wok_platform::{App, FrameCtx, Platform, gfx};
use wok_render::{Camera, DepthMode, RenderItem, Renderer};
use wok_scene::{Aabb, ChunkCoord, Primitive, SurfaceTag, VisibleItem};

use crate::clock::FixedClock;
use crate::constants::{
    DEBUG_GROUND_MARKER, MAX_STEPS_PER_FRAME, PLAYER_COLOR, PLAYER_RADIUS,
    PLAYER_SEGMENT, SHOW_RETICLE, SIM_DT,
};
use crate::content::LoadedContent;
use crate::debug::{self, OverlayMode};
use crate::fade::{OcclusionFade, segment_hits_aabb};
use crate::follow::{self, FollowCamera};
use crate::intent::{Intent, map_input};
use crate::jump::JumpLatch;
use crate::sim::{self, Player, StepInput};
use crate::world::{World, chunk_origin};

const TERRAIN_COLOR: Vec3 = Vec3::new(0.40, 0.60, 0.35);

/// Ground-truth marker presentation (see `DEBUG_GROUND_MARKER`): side length of the quad, how far
/// it floats above the sampled height (just enough to not z-fight the terrain it should lie on;
/// small against any gap worth diagnosing), and a magenta nothing else in the scene uses.
const MARKER_SIZE: f32 = 0.6;
const MARKER_LIFT: f32 = 0.01;
const MARKER_COLOR: Vec3 = Vec3::new(1.0, 0.0, 1.0);

/// Vertical headroom added to the shadow region's top: the player must keep casting at the jump
/// apex (JUMP_APEX_HEIGHT, the tuning parameter itself) plus half the capsule's height above the
/// tracked position, even when standing on the region's highest point. Game knowledge, so it lives
/// here rather than in the renderer's fit; the relationship is pinned by a test below.
const SHADOW_HEADROOM_M: f32 = 3.0;

/// Draw order of the primitive mesh cache; `primitive_index` must match.
const PRIMITIVES: [Primitive; 5] =
    [Primitive::Cube, Primitive::Ellipsoid, Primitive::Cylinder, Primitive::Capsule, Primitive::Plane];

fn primitive_index(primitive: Primitive) -> usize {
    match primitive {
        Primitive::Cube => 0,
        Primitive::Ellipsoid => 1,
        Primitive::Cylinder => 2,
        Primitive::Capsule => 3,
        Primitive::Plane => 4,
    }
}

/// Flat base color for a placeholder by its surface tag; presentation policy each application owns
/// (the engine only carries the tag). Same palette as the editor, so authored content reads the
/// same in both.
fn surface_color(surface: Option<&SurfaceTag>) -> Vec3 {
    match surface.map(SurfaceTag::as_str) {
        Some("grass") => Vec3::new(0.40, 0.60, 0.35),
        Some("wood") => Vec3::new(0.60, 0.42, 0.24),
        Some("stone") => Vec3::new(0.55, 0.55, 0.58),
        Some("metal") => Vec3::new(0.80, 0.45, 0.25),
        _ => Vec3::new(0.70, 0.70, 0.70),
    }
}

/// GPU residency, created in `init` once a device exists: the renderer, one uploaded mesh per unit
/// primitive (shared by every placement), the player's capsule mesh (generated at the collider's
/// dimensions, so the draw transform never scales it), and one terrain mesh per loaded chunk.
struct Gpu {
    renderer: Renderer,
    primitives: Vec<MeshGpu>,
    player: MeshGpu,
    terrain: BTreeMap<ChunkCoord, MeshGpu>,
}

pub struct TasteApp {
    scene_name: String,
    light: LightState,
    store: ChunkStore,
    /// The shadow region the frame call passes: the loaded content's bounds plus jump headroom,
    /// computed once because taste loads everything up front and never reloads.
    shadow_region: Aabb,
    world: World,
    player: Player,
    /// The sim state one fixed step behind `player`: the other end of the draw interpolation.
    player_prev: Player,
    /// Pending jump press, latched across frames until a fixed step consumes it (`crate::jump`).
    jump: JumpLatch,
    camera: FollowCamera,
    /// Per-prefab-item occlusion fade state (`crate::fade`), advanced each rendered frame.
    fade: OcclusionFade,
    clock: FixedClock,
    size: (u32, u32),
    /// The hitbox overlay's mode (`crate::debug`), starting off and cycled by F1.
    overlay: OverlayMode,
    gpu: Option<Gpu>,
}

impl TasteApp {
    /// Build the app from loaded content: transform every chunk through the store (synchronous; the
    /// demo loads everything), reduce the world, and spawn the player and camera over it.
    pub fn new(loaded: LoadedContent) -> Result<TasteApp, Box<dyn Error>> {
        let mut store = ChunkStore::new();
        for (chunk, heightmap) in loaded.chunks {
            store.load(chunk, heightmap, &loaded.prefabs)?;
        }
        let mut shadow_region = World::scene_bounds(&store);
        shadow_region.max.y += SHADOW_HEADROOM_M;
        let world = World::from_store(&store);
        let player = sim::spawn(&world);
        let camera = FollowCamera::spawn(camera_target(player.motion.position));
        Ok(TasteApp {
            scene_name: loaded.scene.name,
            light: loaded.light,
            store,
            shadow_region,
            world,
            player,
            player_prev: player,
            jump: JumpLatch::new(),
            camera,
            fade: OcclusionFade::new(),
            clock: FixedClock::new(SIM_DT, MAX_STEPS_PER_FRAME),
            size: (0, 0),
            overlay: OverlayMode::default(),
            gpu: None,
        })
    }

    /// Run this frame's fixed steps. The camera yaw is resolved into a move direction once per
    /// frame (the camera turns at frame rate, between steps it is constant). The jump edge goes
    /// through the latch (`crate::jump`): it survives a frame that runs zero steps, fires on the
    /// first step that can jump - `Player::can_jump`: grounded, inside the coyote window, or with
    /// an air jump in hand - inside the buffer window, and is consumed there, so a multi-step
    /// catch-up frame still cannot bounce twice on one press, and a zero-step frame cannot eat the
    /// double jump.
    fn simulate(&mut self, intent: &Intent, steps: u32) {
        if intent.jump {
            self.jump.press();
        }
        let move_dir = sim::move_direction(self.camera.yaw, intent.move_forward, intent.move_right);
        for _ in 0..steps {
            let input = StepInput { move_dir, jump: self.jump.consume(self.player.can_jump()) };
            self.player_prev = self.player;
            self.player = sim::step(self.player, input, &self.world);
        }
    }

    /// Draw the frame with the player at `view_pos`, the interpolated position the camera also
    /// targets.
    fn render(&mut self, ctx: &mut FrameCtx, view_pos: Vec3) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Gpu { renderer, primitives, player, terrain } = gpu;

        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        // Fog distance sets render distance (HLD); the far plane sits past full occlusion.
        let far = (self.light.fog.end * 1.2).max(50.0);
        let camera = Camera {
            view_proj: self.camera.view_proj(aspect, far),
            eye: self.camera.position,
        };

        // The occlusion fade: a drawn prefab whose world AABB crosses the eye-to-anchor segment
        // fades partially out instead of clamping the camera (`crate::fade`). Terrain never fades
        // (opacity pinned 1.0 below); the player never fades; index association is the stable
        // draw order of this loop. Frame dt, like the camera: presentation, not simulation.
        let (eye, anchor) = (self.camera.position, self.camera.anchor);
        let mut prefab_index = 0usize;

        let mut items: Vec<RenderItem> = Vec::new();
        for (coord, runtime) in self.store.iter_loaded() {
            let origin = Mat4::from_translation(chunk_origin(coord));
            if let Some(mesh) = terrain.get(&coord) {
                items.push(RenderItem { transform: origin, mesh, color: TERRAIN_COLOR, opacity: 1.0 });
            }
            for item in &runtime.visible {
                match item {
                    VisibleItem::Primitive { primitive, transform, surface } => {
                        let world_transform = origin * *transform;
                        let bounds = wok_physics::world_aabb(*primitive, world_transform);
                        let occluded = segment_hits_aabb(eye, anchor, &bounds);
                        let opacity = self.fade.advance(prefab_index, occluded, ctx.dt);
                        prefab_index += 1;
                        items.push(RenderItem {
                            transform: world_transform,
                            mesh: &primitives[primitive_index(*primitive)],
                            color: surface_color(surface.as_ref()),
                            opacity,
                        });
                    }
                    // Named replacement meshes need the glTF loader (wok-mesh, later); their
                    // placements simply do not draw yet, the same as the editor.
                    VisibleItem::Mesh { .. } => {}
                }
            }
        }

        items.push(RenderItem {
            transform: player_transform(view_pos),
            mesh: player,
            color: PLAYER_COLOR,
            opacity: 1.0,
        });

        // Ground-truth marker (floating diagnosis): a bright quad at the sampled terrain height
        // under the player, composed origin * chunk-local exactly as the terrain mesh is, so in
        // play it shows whether the sampler and the drawn surface agree where the player stands.
        if DEBUG_GROUND_MARKER {
            if let Some(t) = self.world.terrain_under(view_pos.x, view_pos.z) {
                let local = view_pos - t.origin;
                let ground = t.heightmap.height_at(local.x, local.z);
                let quad = Mat4::from_scale_rotation_translation(
                    Vec3::new(MARKER_SIZE, 1.0, MARKER_SIZE),
                    Quat::IDENTITY,
                    Vec3::new(local.x, ground + MARKER_LIFT, local.z),
                );
                items.push(RenderItem {
                    transform: Mat4::from_translation(t.origin) * quad,
                    mesh: &primitives[primitive_index(Primitive::Plane)],
                    color: MARKER_COLOR,
                    opacity: 1.0,
                });
            }
        }

        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };
        renderer.render(
            &ctx.platform.device,
            &ctx.platform.queue,
            &mut frame.encoder,
            &frame.view,
            &camera,
            &self.light,
            self.shadow_region,
            &items,
        );
        // Two line-pass submissions, depth policy per source. The hitbox overlay's policy is the
        // mode's own (`OverlayMode::depth`): Faces compares cage edges against drawn surfaces, so
        // it tests; the x-ray modes read through the very geometry their cages describe. The
        // look-ahead reticle stays depth-tested: it is a world-anchored framing cue, and reading
        // through hills would lie about where it sits.
        if self.overlay != OverlayMode::Off {
            renderer.render_lines(
                &ctx.platform.device,
                &ctx.platform.queue,
                &mut frame.encoder,
                &frame.view,
                &debug::overlay_lines(self.overlay, &self.world, view_pos),
                self.overlay.depth(),
            );
        }
        if SHOW_RETICLE {
            let mut reticle = Vec::new();
            debug::reticle_lines(self.camera.look_target(), &mut reticle);
            renderer.render_lines(
                &ctx.platform.device,
                &ctx.platform.queue,
                &mut frame.encoder,
                &frame.view,
                &reticle,
                DepthMode::Tested,
            );
        }
        frame.finish(ctx.platform);
    }
}

impl App for TasteApp {
    fn init(&mut self, platform: &Platform) {
        platform.window.set_title(&format!("taste - {}", self.scene_name));
        // The game owns the pointer: mouse look is always live (no held-button gate), so the OS
        // cursor is captured and hidden for the run. Locked pins it in place; platforms without
        // Locked (Windows among them) confine it to the window instead, and with the cursor
        // hidden and look reading raw motion the two are indistinguishable in play. If neither
        // works the game still runs, just with a visible pointer - worth a line on stdout.
        let grabbed = platform
            .window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| platform.window.set_cursor_grab(CursorGrabMode::Confined));
        if grabbed.is_err() {
            println!("taste: cursor capture unavailable; the pointer stays visible");
        }
        platform.window.set_cursor_visible(false);
        let config = &platform.surface_config;
        // Diagnostic: which present mode the platform picked (vsync was requested; AutoVsync and
        // Fifo honour it). Jitter hunting starts with knowing whether frames are paced at all.
        println!("taste: present mode {:?}", config.present_mode);
        // The overlay's controls are invisible in play until used; the one line documents them.
        println!("taste: F1 cycles the hitbox overlay: off -> faces -> visible -> all");
        let renderer = Renderer::new(&platform.device, config.format, config.width, config.height);
        self.size = (config.width, config.height);

        let primitives = PRIMITIVES
            .iter()
            .map(|&p| MeshGpu::upload(&platform.device, &primitive_mesh(p)))
            .collect();
        let player =
            MeshGpu::upload(&platform.device, &capsule_mesh(PLAYER_RADIUS, PLAYER_SEGMENT));
        let mut terrain = BTreeMap::new();
        for (coord, runtime) in self.store.iter_loaded() {
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
            }
        }
        self.gpu = Some(Gpu { renderer, primitives, player, terrain });
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        if ctx.width > 0 && ctx.height > 0 && (ctx.width, ctx.height) != self.size {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.renderer.resize(&ctx.platform.device, ctx.width, ctx.height);
            }
            self.size = (ctx.width, ctx.height);
        }

        // The overlay cycle reads the raw input directly: it is a diagnostic, not part of what
        // the player meant, so it stays out of the Intent the simulation consumes.
        if ctx.input.key_pressed(NamedKey::F1) {
            self.overlay = self.overlay.next();
        }

        let intent = map_input(&ctx.input, ctx.dt);
        // Quit is the platform's clean shutdown (cleanup runs, the loop exits); nothing this
        // frame would show is worth simulating or drawing on the way out.
        if intent.quit {
            ctx.should_close = true;
            return;
        }
        let steps = self.clock.advance(ctx.dt);
        self.simulate(&intent, steps);

        // Draw and camera both read the interpolated position: one timeline.
        let view_pos = sim::lerp_position(&self.player_prev, &self.player, self.clock.alpha());
        self.camera = follow::update(&self.camera, camera_target(view_pos), intent.look_delta, &self.world, ctx.dt);
        self.render(ctx, view_pos);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// The point the camera orbits and frames: a little above the capsule centre.
fn camera_target(player_pos: Vec3) -> Vec3 {
    player_pos + Vec3::new(0.0, crate::constants::CAMERA_TARGET_LIFT, 0.0)
}

/// The player's draw transform: a pure translation to the capsule centre. The mesh is generated at
/// the collider's exact dimensions (`capsule_mesh(PLAYER_RADIUS, PLAYER_SEGMENT)`, origin-centred
/// like `Capsule::upright` about its centre), so no scale belongs here - scaling would be the one
/// way the drawn body and the collider could disagree again.
fn player_transform(position: Vec3) -> Mat4 {
    Mat4::from_translation(position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{JUMP_APEX_HEIGHT, PLAYER_HEIGHT};
    use wok_physics::Capsule;

    #[test]
    // Asserting on constants is the point: the test pins a relationship between tuning values so
    // a retune that breaks it fails loudly (the same stance as the constants module's tests).
    #[allow(clippy::assertions_on_constants)]
    fn the_shadow_headroom_clears_the_jump_apex() {
        // The headroom must cover the apex plus the capsule's upper half from the region's highest
        // point, or the shadow pops off mid-jump; pinned so a jump retune cannot out-jump the fit.
        assert!(
            SHADOW_HEADROOM_M >= JUMP_APEX_HEIGHT + PLAYER_HEIGHT * 0.5,
            "headroom {SHADOW_HEADROOM_M} cannot cover the apex {JUMP_APEX_HEIGHT} plus the capsule's upper half"
        );
    }

    #[test]
    fn the_drawn_capsule_is_the_collider() {
        // The visual half of the at-rest contract, now exact in shape and not just in bounds: the
        // mesh is generated at the collider's dimensions and the transform only translates, so the
        // drawn extremes must be the collider's base, tip, and radius. The bound is float roundoff
        // between two derivations of the same numbers, not a tolerance for visual slack.
        let position = Vec3::new(3.0, 7.25, -2.0);
        let capsule = Capsule::upright(position, PLAYER_HEIGHT, PLAYER_RADIUS);
        let bounds = wok_mesh::capsule_mesh(PLAYER_RADIUS, PLAYER_SEGMENT).bounds();
        let to_world = player_transform(position);

        let bottom = to_world.transform_point3(Vec3::new(0.0, bounds.min.y, 0.0));
        assert!(
            (bottom.y - capsule.base().y).abs() < 1e-6,
            "mesh bottom {} vs capsule base {}",
            bottom.y,
            capsule.base().y
        );
        let top = to_world.transform_point3(Vec3::new(0.0, bounds.max.y, 0.0));
        assert!((top.y - (capsule.base().y + PLAYER_HEIGHT)).abs() < 1e-6);
        // And the width matches the capsule's: the wall spans the radius each way.
        let side = to_world.transform_point3(Vec3::new(bounds.max.x, 0.0, 0.0));
        assert!((side.x - (position.x + PLAYER_RADIUS)).abs() < 1e-6);
    }
}
