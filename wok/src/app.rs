//! The editor application: GPU state, the per-frame loop, input mapping, and hot reload.
//!
//! This is composition only, per the HLD's application layer: authored data flows through
//! wok-content's store into runtime arrays, wok-mesh uploads them, wok-render draws exactly the
//! list it is handed each frame, and the chunk-origin composition (chunk-local transforms lifted
//! into world space) happens here because the render contract makes it caller policy.
//!
//! Hot reload follows the HLD data flow: wok-scene's watcher reports raw changed paths, the
//! editor polls each frame, classifies them (`crate::content::classify`), and re-runs the
//! authored-to-runtime transform for affected chunks. A light-state change swaps per-frame data
//! only and never touches a chunk. Reload failures are printed and skipped rather than crashing:
//! a watcher event can arrive mid-write with a half-written file, and the completing write
//! delivers a second event that retries.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::path::PathBuf;

use glam::{Mat4, Vec2, Vec3};
use wok_content::{ChunkState, ChunkStore};
use wok_light::LightState;
use wok_mesh::{MeshGpu, primitive_mesh};
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::Key;
use wok_platform::{App, FrameCtx, Platform, gfx};
use wok_render::{Camera, RenderItem, Renderer};
use wok_scene::{CHUNK_GRID_DIM, ChunkCoord, Prefab, PrefabRef, Primitive, Scene, SurfaceTag, VisibleItem, Watcher};

use crate::camera::{self, CameraInput, FlyCamera};
use crate::content::{self, ContentPaths, LoadedContent, Reload};

/// Chunk side in metres, derived from the heightmap grid (128 one-metre cells; the 129th sample
/// is the shared edge). wok-scene deliberately does not bake the chunk size into ChunkCoord.
const CHUNK_SIZE_M: f32 = (CHUNK_GRID_DIM - 1) as f32;

/// Mouse-look sensitivity, radians per pixel of raw motion.
const LOOK_SENSITIVITY: f32 = 0.0035;

const TERRAIN_COLOR: Vec3 = Vec3::new(0.40, 0.60, 0.35);

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

/// World-space origin of a chunk: its grid coordinate times the chunk size.
fn chunk_origin(coord: ChunkCoord) -> Vec3 {
    Vec3::new(coord.x as f32 * CHUNK_SIZE_M, 0.0, coord.z as f32 * CHUNK_SIZE_M)
}

/// Flat base color for a placeholder by its surface tag; editor presentation policy, not engine
/// data (the engine only carries the tag).
fn surface_color(surface: Option<&SurfaceTag>) -> Vec3 {
    match surface.map(SurfaceTag::as_str) {
        Some("grass") => Vec3::new(0.40, 0.60, 0.35),
        Some("wood") => Vec3::new(0.60, 0.42, 0.24),
        Some("stone") => Vec3::new(0.55, 0.55, 0.58),
        Some("metal") => Vec3::new(0.80, 0.45, 0.25),
        _ => Vec3::new(0.70, 0.70, 0.70),
    }
}

/// GPU residency, created in `init` once a device exists: the renderer, one uploaded mesh per
/// unit primitive (shared by every placement), and one terrain mesh per loaded chunk.
struct Gpu {
    renderer: Renderer,
    primitives: Vec<MeshGpu>,
    terrain: BTreeMap<ChunkCoord, MeshGpu>,
}

pub struct EditorApp {
    paths: ContentPaths,
    scene: Scene,
    prefabs: HashMap<PrefabRef, Prefab>,
    light: LightState,
    store: ChunkStore,
    watcher: Watcher,
    camera: FlyCamera,
    size: (u32, u32),
    gpu: Option<Gpu>,
}

impl EditorApp {
    /// Build the app from loaded content: transform every chunk through the store (synchronous,
    /// no streaming in v0) and start watching the content root for hot reload.
    pub fn new(paths: ContentPaths, loaded: LoadedContent) -> Result<EditorApp, Box<dyn Error>> {
        let watcher = Watcher::new(&paths.root)?;
        let mut store = ChunkStore::new();
        for (chunk, heightmap) in loaded.chunks {
            store.load(chunk, heightmap, &loaded.prefabs)?;
        }
        let camera = spawn_camera(&store);
        Ok(EditorApp {
            paths,
            scene: loaded.scene,
            prefabs: loaded.prefabs,
            light: loaded.light,
            store,
            watcher,
            camera,
            size: (0, 0),
            gpu: None,
        })
    }

    fn apply_reloads(&mut self, platform: &Platform, changed: Vec<PathBuf>) {
        let mut scene_changed = false;
        let mut prefabs_changed = false;
        let mut light_changed = false;
        let mut chunks: BTreeSet<ChunkCoord> = BTreeSet::new();
        for path in changed {
            match content::classify(&self.paths.root, &path) {
                Some(Reload::Scene) => scene_changed = true,
                Some(Reload::Prefabs) => prefabs_changed = true,
                Some(Reload::Chunk(coord)) => {
                    chunks.insert(coord);
                }
                Some(Reload::Light(name)) if name == self.scene.default_lighting.as_str() => {
                    light_changed = true;
                }
                _ => {}
            }
        }

        if scene_changed {
            match wok_scene::load_scene(self.paths.scene()) {
                Ok(scene) => {
                    if scene.default_lighting != self.scene.default_lighting {
                        light_changed = true;
                    }
                    platform.window.set_title(&format!("wok - {}", scene.name));
                    self.scene = scene;
                    println!("wok: reloaded scene manifest");
                }
                Err(err) => eprintln!("wok: scene reload failed, keeping previous: {err}"),
            }
        }

        if prefabs_changed {
            match content::load_prefab_library(&self.paths) {
                Ok(prefabs) => {
                    self.prefabs = prefabs;
                    // The runtime arrays do not retain which prefabs a chunk placed, so a prefab
                    // change re-transforms every loaded chunk.
                    chunks.extend(self.store.iter_loaded().map(|(coord, _)| coord));
                    println!("wok: reloaded prefab library");
                }
                Err(err) => eprintln!("wok: prefab reload failed, keeping previous: {err}"),
            }
        }

        for coord in chunks {
            self.reload_chunk(platform, coord);
        }

        if light_changed {
            let name = self.scene.default_lighting.as_str();
            match wok_light::load_light_state(self.paths.light(name)) {
                Ok((_, light)) => {
                    self.light = light;
                    println!("wok: reloaded light state {name:?}");
                }
                Err(err) => eprintln!("wok: light reload failed, keeping previous: {err}"),
            }
        }
    }

    /// Release and re-transform one chunk from disk, replacing its terrain mesh on the GPU. A
    /// missing chunk file means the chunk was deleted; it stays released.
    fn reload_chunk(&mut self, platform: &Platform, coord: ChunkCoord) {
        if self.store.state(coord) == ChunkState::Loaded {
            let _ = self.store.release(coord);
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.terrain.remove(&coord);
        }
        if !self.paths.chunk(coord).exists() {
            println!("wok: chunk {}_{} removed", coord.x, coord.z);
            return;
        }
        let loaded = content::load_chunk_with_heightmap(&self.paths, coord)
            .and_then(|(chunk, hm)| Ok(self.store.load(chunk, hm, &self.prefabs)?));
        match loaded {
            Ok(runtime) => {
                if let Some(mesh) = runtime.terrain_mesh.as_ref()
                    && let Some(gpu) = self.gpu.as_mut()
                {
                    gpu.terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
                }
                println!("wok: reloaded chunk {}_{}", coord.x, coord.z);
            }
            Err(err) => eprintln!("wok: chunk {}_{} reload failed, now unloaded: {err}", coord.x, coord.z),
        }
    }

    fn render(&mut self, ctx: &mut FrameCtx) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Gpu { renderer, primitives, terrain } = gpu;

        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        // Fog distance sets render distance (HLD); the far plane sits past full occlusion.
        let far = (self.light.fog.end * 1.2).max(50.0);
        let camera = Camera { view_proj: self.camera.view_proj(aspect, far), eye: self.camera.position };

        let mut items: Vec<RenderItem> = Vec::new();
        for (coord, runtime) in self.store.iter_loaded() {
            let origin = Mat4::from_translation(chunk_origin(coord));
            if let Some(mesh) = terrain.get(&coord) {
                items.push(RenderItem { transform: origin, mesh, color: TERRAIN_COLOR });
            }
            for item in &runtime.visible {
                match item {
                    VisibleItem::Primitive { primitive, transform, surface } => {
                        items.push(RenderItem {
                            transform: origin * *transform,
                            mesh: &primitives[primitive_index(*primitive)],
                            color: surface_color(surface.as_ref()),
                        });
                    }
                    // Named replacement meshes need the glTF loader (wok-mesh, later); their
                    // placements simply do not draw in v0.
                    VisibleItem::Mesh { .. } => {}
                }
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
            &items,
        );
        frame.finish(ctx.platform);
    }
}

impl App for EditorApp {
    fn init(&mut self, platform: &Platform) {
        platform.window.set_title(&format!("wok - {}", self.scene.name));
        let config = &platform.surface_config;
        let renderer = Renderer::new(&platform.device, config.format, config.width, config.height);
        self.size = (config.width, config.height);

        let primitives = PRIMITIVES
            .iter()
            .map(|&p| MeshGpu::upload(&platform.device, &primitive_mesh(p)))
            .collect();
        let mut terrain = BTreeMap::new();
        for (coord, runtime) in self.store.iter_loaded() {
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
            }
        }
        self.gpu = Some(Gpu { renderer, primitives, terrain });
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        if ctx.width > 0 && ctx.height > 0 && (ctx.width, ctx.height) != self.size {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.renderer.resize(&ctx.platform.device, ctx.width, ctx.height);
            }
            self.size = (ctx.width, ctx.height);
        }

        let changed = self.watcher.poll();
        if !changed.is_empty() {
            self.apply_reloads(ctx.platform, changed);
        }

        self.camera = camera::update(&self.camera, &camera_input(&ctx.input), ctx.dt);
        self.render(ctx);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// Spawn over the first loaded chunk, mid-south looking north across it, a little above the
/// terrain there (or above the origin plane when the scene has no terrain).
fn spawn_camera(store: &ChunkStore) -> FlyCamera {
    let half = CHUNK_SIZE_M * 0.5;
    let south = CHUNK_SIZE_M * 0.8;
    let (origin, ground) = store
        .iter_loaded()
        .next()
        .map_or((Vec3::ZERO, 0.0), |(coord, runtime)| {
            let ground = runtime.heightmap.as_ref().map_or(0.0, |h| h.height_at(half, south));
            (chunk_origin(coord), ground)
        });
    FlyCamera {
        position: origin + Vec3::new(half, ground + 12.0, south),
        yaw: 0.0,
        pitch: -0.15,
        speed: 16.0,
    }
}

/// Map the frame's raw input snapshot to the camera's input: WASD moves, Q/E sink and rise,
/// holding the right mouse button turns raw mouse motion into look, scroll adjusts speed.
fn camera_input(input: &InputState) -> CameraInput {
    let axis = |pos: char, neg: char| f32::from(char_held(input, pos)) - f32::from(char_held(input, neg));
    let look_delta = if input.mouse_held(MouseButton::Right) {
        Vec2::new(input.mouse_motion.0 as f32, -input.mouse_motion.1 as f32) * LOOK_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    CameraInput {
        move_forward: axis('w', 's'),
        move_right: axis('d', 'a'),
        move_up: axis('e', 'q'),
        look_delta,
        speed_steps: input.scroll_delta.1,
    }
}

/// Is a printable character key held, compared case-insensitively so shift state does not stick
/// a movement key (the held-key analogue of `InputState::char_pressed`).
fn char_held(input: &InputState, ch: char) -> bool {
    input.keys_held.iter().any(|k| match k {
        Key::Character(s) => s.chars().any(|c| c.eq_ignore_ascii_case(&ch)),
        _ => false,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn input_with(keys: &[&str]) -> InputState {
        InputState {
            keys_held: keys.iter().map(|s| Key::Character((*s).into())).collect(),
            keys_pressed: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: (10.0, 4.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 2.0),
            gamepads: vec![],
        }
    }

    #[test]
    fn wasd_and_qe_map_to_movement_axes() {
        let input = input_with(&["w", "d", "q"]);
        let mapped = camera_input(&input);
        assert_eq!(mapped.move_forward, 1.0);
        assert_eq!(mapped.move_right, 1.0);
        assert_eq!(mapped.move_up, -1.0);
        assert_eq!(mapped.speed_steps, 2.0);
    }

    #[test]
    fn opposed_keys_cancel_and_shifted_keys_still_count() {
        let input = input_with(&["W", "s"]);
        assert_eq!(camera_input(&input).move_forward, 0.0);
    }

    #[test]
    fn mouse_motion_is_look_only_while_right_button_is_held() {
        let mut input = input_with(&[]);
        assert_eq!(camera_input(&input).look_delta, Vec2::ZERO);

        input.mouse_buttons_held.insert(MouseButton::Right);
        let look = camera_input(&input).look_delta;
        assert!(look.x > 0.0, "rightward motion should turn right: {look:?}");
        assert!(look.y < 0.0, "downward motion should pitch down: {look:?}");
    }

    #[test]
    fn chunk_origin_scales_by_the_chunk_size() {
        assert_eq!(chunk_origin(ChunkCoord::new(0, 0)), Vec3::ZERO);
        assert_eq!(chunk_origin(ChunkCoord::new(2, -1)), Vec3::new(256.0, 0.0, -128.0));
    }
}
