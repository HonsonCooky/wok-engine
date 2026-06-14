//! The editor application: the camera, the window state, and the per-frame loop that ties the
//! modules together.
//!
//! Composition only, per the HLD's application layer: the authored model (`crate::model`) flows
//! through wok-content's store into runtime arrays, which `crate::render` draws each frame. egui
//! paints last (`crate::gui`); the UI and the viewport routing (`crate::panels`, `crate::input`)
//! emit actions the loop applies at one point (`crate::actions`, the model's single writer); watcher
//! changes apply content-compared (`crate::reload`). The camera is modal (`crate::mode`): free-fly
//! flies (`crate::camera`), object mode locks to the selection and orbits it (`crate::orbit`).
//!
//! The frame order is load-bearing: hot reload first (the model is current before anything reads
//! it), then the UI (its focus queries decide what input the rest of the frame may use), then the
//! UI's actions, then the viewport input and its actions, then the camera advance - last, so it
//! sees this frame's final selection and mode and a click-to-select frames the same frame - and
//! finally the render with the UI output painted over it.

use std::error::Error;

use glam::Vec3;
use wok_light::LightState;
use wok_platform::input::InputState;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform};
use wok_scene::Watcher;

use crate::camera::FlyCamera;
use crate::content::{ContentPaths, LoadedContent};
use crate::input;
use crate::mode::Mode;
use crate::model::{CHUNK_SIZE_M, EditorModel, chunk_origin};
use crate::orbit::{self, Orbit};
use crate::panels::{self, Stats, UiState};
use crate::reload;
use crate::render::Gpu;
use crate::selection::SelectionSet;

pub struct EditorApp {
    pub(crate) paths: ContentPaths,
    pub(crate) model: EditorModel,
    pub(crate) light: LightState,
    watcher: Watcher,
    pub(crate) camera: FlyCamera,
    /// The desired object-mode orbit (boom angles + arm length). The source of truth for the camera
    /// in object mode; stale and ignored in free-fly (`crate::orbit`).
    pub(crate) orbit: Orbit,
    /// The selection the object-mode camera is locked onto; empty means not locked. The camera
    /// frames only when it locks on from empty - so switching between objects keeps the zoom and
    /// only re-centres - and the lock is cleared in free-fly, so entering object mode re-locks.
    framed_selection: SelectionSet,
    pub(crate) ui: UiState,
    pub(crate) size: (u32, u32),
    pub(crate) gpu: Option<Gpu>,
    /// Frame-time window for the stats overlay: seconds and frames accumulated since the last
    /// refresh. The displayed numbers update once a second (the window's average), because
    /// per-frame fps and ms churn faster than they can be read.
    stat_accum_s: f32,
    stat_accum_frames: u32,
    /// The displayed averages, refreshed once per second from the window above.
    fps: f32,
    frame_ms: f32,
    /// Render-list length of the previous frame, for the stats overlay.
    pub(crate) draw_items: usize,
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
            orbit: Orbit::default(),
            framed_selection: SelectionSet::new(),
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
    pub(crate) fn far_plane(&self) -> f32 {
        (self.light.fog.end * 1.2).max(50.0)
    }

    /// Advance the camera one frame, modal on the interaction mode. Free-fly flies; object mode
    /// locks to the selection - it frames the set when it first locks on (from an empty selection),
    /// then orbits the centroid (right-drag turns, scroll zooms) at the set distance, holding the
    /// last pose when nothing is selected. Runs after input is applied, so it sees this frame's
    /// final selection and mode.
    fn advance_camera(&mut self, input: &InputState, pointer_free: bool, keys_free: bool, dt: f32) {
        let nav = input::camera_input(input, pointer_free, keys_free);
        let pivot = self.model.selection_pivot();

        // Frame only when the object-mode camera first locks on (from an empty selection); switching
        // between objects keeps the orbit's distance and angles, re-centring on the new pivot on its
        // own. In free-fly the lock is left cleared, so entering object mode frames the selection.
        if self.ui.mode == Mode::Object {
            if self.framed_selection.is_empty()
                && let (Some(bounds), Some(pivot)) = (self.model.selection_bounds(), pivot)
            {
                self.orbit = Orbit::framing(&self.camera, bounds, pivot);
            }
            self.framed_selection = self.model.selection.clone();
        }

        self.camera = orbit::advance(self.ui.mode, &self.camera, &mut self.orbit, &nav, dt, pivot);
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
        self.size = (config.width, config.height);
        self.gpu = Some(Gpu::new(platform, &self.model));
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
            // fps and frame-ms are the once-per-second window averages; the camera speed and the
            // counts stay live. The speed is read before this frame's camera update (the UI must
            // run first for its focus queries), so a scroll lands in the readout next frame -
            // one frame behind the hand, immediate to the eye.
            let stats = Stats {
                fps: self.fps,
                frame_ms: self.frame_ms,
                cam_speed: self.camera.speed,
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

        // Viewport input next (hotkeys, clicks, drags, the mode toggle), reading this frame's camera
        // - the one that drew the image being clicked on - and emitted as actions applied right
        // away, so a selection made this frame is visible to the camera step below.
        let mut input_actions = Vec::new();
        input::handle(
            &ctx.input,
            pointer_free,
            keys_free,
            &self.camera,
            self.size,
            self.far_plane(),
            &self.model,
            &mut self.ui,
            &mut input_actions,
        );
        for action in input_actions {
            self.apply_action(action);
        }

        // Advance the camera last, so it sees the final selection and mode for this frame.
        self.advance_camera(&ctx.input, pointer_free, keys_free, ctx.dt);

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
