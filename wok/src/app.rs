//! The editor application: GPU state and the per-frame loop.
//!
//! Composition only, per the HLD's application layer: the authored model (`crate::model`) flows
//! through wok-content's store into runtime arrays, wok-mesh uploads them, wok-render draws
//! exactly the list it is handed, and the chunk-origin composition happens here because the
//! render contract makes it caller policy. egui paints last into the same encoder
//! (`crate::gui`), the UI emits actions the loop applies (`crate::panels`), input routing honors
//! egui's focus (`crate::input`), and watcher changes apply content-compared (`crate::reload`).
//!
//! The frame order is load-bearing: hot reload first (the model is current before anything reads
//! it), then the UI (its focus queries decide what input the rest of the frame may use), then
//! actions, camera, clicks, and finally the render with the UI output painted over it.

use std::collections::BTreeMap;
use std::error::Error;

use glam::{Mat4, Vec3};
use wok_light::LightState;
use wok_mesh::{MeshGpu, primitive_mesh};
use wok_physics::world_aabb;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform, gfx};
use wok_render::{Camera, DepthMode, LineSegment, RenderItem, Renderer};
use wok_scene::{Aabb, ChunkCoord, Primitive, SurfaceTag, VisibleItem, Watcher};

use crate::camera::{self, FlyCamera};
use crate::content::{ContentPaths, LoadedContent};
use crate::gui::Gui;
use crate::input;
use crate::lines;
use crate::model::{CHUNK_SIZE_M, EditorModel, chunk_origin};
use crate::panels::{self, Action, Stats, UiState};
use crate::pick;
use crate::reload;

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

/// World-space bounds of everything the loaded chunks hold - the shadow region the frame call
/// passes (caller policy per the render contract). Falls back to a small box around the origin
/// when nothing is loaded. Recomputed per frame because edits and hot reload can change the store
/// between any two frames; the scan is a few thousand min/max ops per chunk, frame-state-cheap.
fn scene_bounds(store: &wok_content::ChunkStore) -> Aabb {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut grow = |b: Aabb| {
        min = min.min(b.min);
        max = max.max(b.max);
    };
    for (coord, runtime) in store.iter_loaded() {
        let origin = chunk_origin(coord);
        let origin_mat = Mat4::from_translation(origin);
        if let Some(mesh) = runtime.terrain_mesh.as_ref() {
            let b = mesh.bounds();
            grow(Aabb::new(b.min + origin, b.max + origin));
        }
        for item in &runtime.visible {
            if let VisibleItem::Primitive { primitive, transform, .. } = item {
                grow(world_aabb(*primitive, origin_mat * *transform));
            }
        }
        for hitbox in &runtime.hitboxes {
            grow(world_aabb(hitbox.primitive, origin_mat * hitbox.transform));
        }
    }
    if min.x > max.x {
        return Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
    }
    Aabb::new(min, max)
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

/// GPU residency, created in `init` once a device exists: the renderer, the egui integration, one
/// uploaded mesh per unit primitive (shared by every placement), and one terrain mesh per chunk.
struct Gpu {
    renderer: Renderer,
    gui: Gui,
    primitives: Vec<MeshGpu>,
    terrain: BTreeMap<ChunkCoord, MeshGpu>,
}

pub struct EditorApp {
    paths: ContentPaths,
    model: EditorModel,
    light: LightState,
    watcher: Watcher,
    camera: FlyCamera,
    ui: UiState,
    size: (u32, u32),
    gpu: Option<Gpu>,
    /// Exponentially smoothed frame time in milliseconds, for the stats overlay.
    frame_ms: f32,
    /// Render-list length of the previous frame, for the stats overlay.
    draw_items: usize,
    title: String,
}

impl EditorApp {
    /// Build the app from loaded content: the authored model transforms every chunk through the
    /// store (synchronous, no streaming), and the content root is watched for hot reload.
    pub fn new(paths: ContentPaths, loaded: LoadedContent) -> Result<EditorApp, Box<dyn Error>> {
        let watcher = Watcher::new(&paths.root)?;
        let model = EditorModel::new(loaded.scene, loaded.prefabs, loaded.chunks)?;
        let camera = spawn_camera(&model);
        Ok(EditorApp {
            paths,
            model,
            light: loaded.light,
            watcher,
            camera,
            ui: UiState::default(),
            size: (0, 0),
            gpu: None,
            frame_ms: 0.0,
            draw_items: 0,
            title: String::new(),
        })
    }

    /// Fog distance sets render distance (HLD); the far plane sits past full occlusion. Picking
    /// shares it: what you can see, you can click.
    fn far_plane(&self) -> f32 {
        (self.light.fog.end * 1.2).max(50.0)
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::Select(sel) => self.model.selection = sel,
            Action::Edit { sel, transform, state } => {
                if let Err(err) = self.model.edit_placement(sel, transform, state) {
                    eprintln!("wok: edit failed: {err}");
                }
            }
            Action::ArmPlace(prefab) => self.ui.placing = Some(prefab),
            Action::DisarmPlace => self.ui.placing = None,
        }
    }

    /// The selected placement's classified colliders as an x-ray cage.
    fn selection_lines(&self) -> Vec<LineSegment> {
        let mut out = Vec::new();
        if let Some(sel) = self.model.selection
            && let Some(placement) = self.model.placement(sel)
            && let Some(prefab) = self.model.prefabs.get(&placement.prefab)
        {
            for collider in pick::placement_colliders(prefab, placement, chunk_origin(sel.coord)) {
                lines::collider_lines(&collider, lines::SELECTION_COLOR, &mut out);
            }
        }
        out
    }

    fn render(&mut self, ctx: &mut FrameCtx, ui_output: Option<egui::FullOutput>) {
        let far = self.far_plane();
        let cage = self.selection_lines();
        let model = &self.model;
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Gpu { renderer, gui, primitives, terrain } = gpu;

        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        let camera = Camera { view_proj: self.camera.view_proj(aspect, far), eye: self.camera.position };

        let mut items: Vec<RenderItem> = Vec::new();
        for (coord, runtime) in model.store.iter_loaded() {
            let origin = Mat4::from_translation(chunk_origin(coord));
            if let Some(mesh) = terrain.get(&coord) {
                items.push(RenderItem { transform: origin, mesh, color: TERRAIN_COLOR, opacity: 1.0 });
            }
            for item in &runtime.visible {
                match item {
                    VisibleItem::Primitive { primitive, transform, surface } => {
                        items.push(RenderItem {
                            transform: origin * *transform,
                            mesh: &primitives[primitive_index(*primitive)],
                            color: surface_color(surface.as_ref()),
                            opacity: 1.0,
                        });
                    }
                    // Named replacement meshes need the glTF loader (wok-mesh, later); their
                    // placements simply do not draw yet.
                    VisibleItem::Mesh { .. } => {}
                }
            }
        }
        self.draw_items = items.len();

        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };
        renderer.render(
            &ctx.platform.device,
            &ctx.platform.queue,
            &mut frame.encoder,
            &frame.view,
            &camera,
            &self.light,
            scene_bounds(&model.store),
            &items,
        );
        if !cage.is_empty() {
            renderer.render_lines(
                &ctx.platform.device,
                &ctx.platform.queue,
                &mut frame.encoder,
                &frame.view,
                &cage,
                DepthMode::XRay,
            );
        }
        if let Some(output) = ui_output {
            gui.paint(ctx.platform, &mut frame.encoder, &frame.view, output, self.size);
        }
        frame.finish(ctx.platform);
    }

    /// Keep the window title showing the scene name and the unsaved-changes indicator.
    fn refresh_title(&mut self, platform: &Platform) {
        let dirty = if self.model.is_dirty() { " *" } else { "" };
        let title = format!("wok - {}{dirty}", self.model.scene.name);
        if title != self.title {
            platform.window.set_title(&title);
            self.title = title;
        }
    }
}

impl App for EditorApp {
    fn init(&mut self, platform: &Platform) {
        let config = &platform.surface_config;
        let renderer = Renderer::new(&platform.device, config.format, config.width, config.height);
        self.size = (config.width, config.height);

        let primitives = PRIMITIVES
            .iter()
            .map(|&p| MeshGpu::upload(&platform.device, &primitive_mesh(p)))
            .collect();
        let mut terrain = BTreeMap::new();
        for (coord, runtime) in self.model.store.iter_loaded() {
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
            }
        }
        self.gpu = Some(Gpu { renderer, gui: Gui::new(platform), primitives, terrain });
        self.refresh_title(platform);
    }

    fn on_window_event(&mut self, platform: Option<&Platform>, event: &WindowEvent) {
        if let (Some(platform), Some(gpu)) = (platform, self.gpu.as_mut()) {
            gpu.gui.on_event(&platform.window, event);
        }
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        if ctx.width > 0 && ctx.height > 0 && (ctx.width, ctx.height) != self.size {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.renderer.resize(&ctx.platform.device, ctx.width, ctx.height);
            }
            self.size = (ctx.width, ctx.height);
        }

        let changed = self.watcher.poll();
        if !changed.is_empty()
            && let Some(gpu) = self.gpu.as_mut()
        {
            reload::apply(
                &self.paths,
                &mut self.model,
                &mut self.light,
                &mut gpu.terrain,
                &ctx.platform.device,
                changed,
            );
        }

        if ctx.dt > 0.0 {
            let ms = ctx.dt * 1000.0;
            self.frame_ms = if self.frame_ms <= 0.0 { ms } else { 0.9 * self.frame_ms + 0.1 * ms };
        }

        // Run the UI first: its focus queries decide what input the rest of the frame may use.
        let mut actions = Vec::new();
        let mut ui_output = None;
        let (mut pointer_free, mut keys_free) = (true, true);
        {
            let model = &self.model;
            let ui_state = &self.ui;
            let stats = Stats {
                fps: if self.frame_ms > 0.0 { 1000.0 / self.frame_ms } else { 0.0 },
                frame_ms: self.frame_ms,
                chunk_count: model.chunks.len(),
                placement_count: model.placement_count(),
                draw_items: self.draw_items,
            };
            if let Some(gpu) = self.gpu.as_mut() {
                let output = gpu.gui.run(&ctx.platform.window, |egui_ctx| {
                    panels::ui(egui_ctx, model, ui_state, &stats, &mut actions);
                });
                pointer_free = !gpu.gui.ctx.is_pointer_over_area() && !gpu.gui.ctx.wants_pointer_input();
                keys_free = !gpu.gui.ctx.wants_keyboard_input();
                ui_output = Some(output);
            }
        }
        for action in actions {
            self.apply_action(action);
        }

        self.camera =
            camera::update(&self.camera, &input::camera_input(&ctx.input, pointer_free, keys_free), ctx.dt);
        input::handle(
            &ctx.input,
            pointer_free,
            keys_free,
            &self.camera,
            self.size,
            self.far_plane(),
            &mut self.model,
            &mut self.ui,
            &self.paths,
        );

        self.render(ctx, ui_output);
        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// Spawn over the first loaded chunk, mid-south looking north across it, a little above the
/// terrain there (or above the origin plane when the scene has no terrain).
fn spawn_camera(model: &EditorModel) -> FlyCamera {
    let half = CHUNK_SIZE_M * 0.5;
    let south = CHUNK_SIZE_M * 0.8;
    let (origin, ground) = model
        .store
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
