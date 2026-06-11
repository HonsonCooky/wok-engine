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
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform, gfx};
use wok_render::{Camera, DepthMode, LineSegment, RenderItem, Renderer};
use wok_scene::{ChunkCoord, Primitive, SurfaceTag, VisibleItem, Watcher};

use crate::camera::{self, FlyCamera};
use crate::content::{ContentPaths, LoadedContent};
use crate::gui::Gui;
use crate::input;
use crate::lines;
use crate::model::{CHUNK_SIZE_M, EditorModel, chunk_origin, scene_bounds};
use crate::panels::{self, Action, Stats, UiState};
use crate::pick;
use crate::reload;
use crate::theme;

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
    /// Frame-time window for the stats overlay: seconds and frames accumulated since the last
    /// refresh. The displayed numbers update once a second (the window's average), because
    /// per-frame fps and ms churn faster than they can be read.
    stat_accum_s: f32,
    stat_accum_frames: u32,
    /// The displayed averages, refreshed once per second from the window above.
    fps: f32,
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
            stat_accum_s: 0.0,
            stat_accum_frames: 0,
            fps: 0.0,
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
            Action::Duplicate(sel) => match self.model.duplicate(sel) {
                // The copy is selected by the model; bring its tree row into view.
                Ok(Some(_)) => self.ui.scroll_to_selection = true,
                Ok(None) => {}
                Err(err) => eprintln!("wok: duplicate failed: {err}"),
            },
            Action::Rename { sel, name } => {
                self.model.rename(sel, &name);
            }
            Action::Delete(sel) => {
                if let Err(err) = self.model.delete(sel) {
                    eprintln!("wok: delete failed: {err}");
                }
            }
            Action::Frame(sel) => {
                if let Some(bounds) = self.model.world_bounds(sel) {
                    self.camera = camera::frame(&self.camera, bounds.min, bounds.max);
                }
            }
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
        let gui = Gui::new(platform);
        theme::apply(&gui.ctx);
        self.gpu = Some(Gpu { renderer, gui, primitives, terrain });
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
            self.stat_accum_s += ctx.dt;
            self.stat_accum_frames += 1;
            if self.stat_accum_s >= 1.0 {
                self.frame_ms = 1000.0 * self.stat_accum_s / self.stat_accum_frames as f32;
                self.fps = self.stat_accum_frames as f32 / self.stat_accum_s;
                self.stat_accum_s = 0.0;
                self.stat_accum_frames = 0;
            }
        }

        // Run the UI first: its focus queries decide what input the rest of the frame may use.
        let mut actions = Vec::new();
        let mut ui_output = None;
        let (mut pointer_free, mut keys_free) = (true, true);
        {
            let model = &self.model;
            let ui_state = &mut self.ui;
            // fps and frame-ms are the once-per-second window averages; the counts stay live.
            let stats = Stats {
                fps: self.fps,
                frame_ms: self.frame_ms,
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
