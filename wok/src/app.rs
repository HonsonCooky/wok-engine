//! The editor application: the window state, the camera, and the per-frame loop.
//!
//! The chrome (`crate::menu`, `crate::workspace`, composed by `crate::view`) reads the model and the
//! open project's content summary and emits actions the loop applies through the one handler
//! (`crate::action`, the single writer). The scene the viewport draws is a separate residency
//! (`crate::scene`, `LoadedScene`) reconciled to the open project: opening a project loads (or first
//! generates) its content and uploads its GPU meshes, closing it drops them. The camera is modal
//! (`crate::mode`): free-fly flies the god-cam (`crate::camera`), Object is the resting mode.
//!
//! The frame order is load-bearing: hot reload first (the scene is current before anything reads it),
//! then the UI (its focus queries decide what input the rest of the frame may use), then the UI's
//! actions, then the scene reconcile (an open/close just applied takes effect), then the viewport
//! input (the mode toggle), then the camera advance - last, so it flies on this frame's final mode -
//! and finally the render with the chrome painted over it.

use std::path::{Path, PathBuf};

use glam::Vec3;
use wok_platform::input::InputState;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform};

use crate::action::{self, Action};
use crate::camera::{self, FlyCamera};
use crate::gui::Gui;
use crate::input;
use crate::mode::Mode;
use crate::model::Model;
use crate::project::Project;
use crate::recent;
use crate::render::Gpu;
use crate::scene::LoadedScene;
use crate::theme;
use crate::view;

pub struct EditorApp {
    /// The shell state the action layer writes and the view reads: the open project and the layout.
    model: Model,
    /// egui integration, built once a GPU device exists (`init`).
    pub(crate) gui: Option<Gui>,
    /// The scene-independent GPU residency (renderer + unit primitive meshes + terrain cache), built
    /// in `init`. Terrain is filled when a project's content loads (`crate::render`).
    pub(crate) gpu: Option<Gpu>,
    /// The open project's loaded content - the scene the viewport draws - or none when no project is
    /// open. Reconciled to `model.project` each frame.
    pub(crate) scene: Option<LoadedScene>,
    /// The project root the scene residency last reconciled to. Keyed here (not derived from
    /// `scene`) so a load that failed is not retried every frame: reconcile acts only when the
    /// project's root differs from this.
    loaded_root: Option<PathBuf>,
    /// The god-cam the renderer reads. Spawned over the scene when a project loads; advanced in
    /// free-fly, at rest in Object mode.
    pub(crate) camera: FlyCamera,
    /// The interaction mode (`crate::mode`), toggled in place by the viewport input (backtick).
    mode: Mode,
    pub(crate) size: (u32, u32),
    /// The editor-area rect (egui points) the chrome settled into last frame, captured from
    /// `view::chrome`. The render scopes the 3D viewport to it (`crate::render`), and it is the one
    /// rect cursor-to-ray picking will map against (3b) - both read this single source rather than
    /// recomputing the layout. Updated every frame, so docking or toggling the nav panel and
    /// resizing the window track automatically.
    pub(crate) editor_rect: egui::Rect,
    /// The window title last set, so it is only pushed to the OS when it changes.
    title: String,
}

impl EditorApp {
    /// Build the app. The recent-projects list is seeded from disk (a missing or malformed file reads
    /// as empty, so the editor always starts). An optional startup folder (from the CLI) opens as the
    /// initial project, routed through the one writer so it lands in recents the way a menu open does;
    /// its content loads in `init`, once a GPU device exists.
    pub fn new(initial: Option<PathBuf>) -> EditorApp {
        let mut model = Model::new(Project::None);
        model.recents = recent::load();
        if let Some(root) = initial {
            if action::handle(Action::OpenProject(root), &mut model).save_recents {
                recent::save(&model.recents);
            }
        }
        EditorApp {
            model,
            gui: None,
            gpu: None,
            scene: None,
            loaded_root: None,
            camera: default_camera(),
            mode: Mode::default(),
            size: (0, 0),
            // Overwritten by the first frame's chrome before any render reads it; NOTHING reads as
            // "no usable rect", which the render treats as the full target.
            editor_rect: egui::Rect::NOTHING,
            title: String::new(),
        }
    }

    /// Reconcile the scene residency to the open project. The project is the source of truth: when
    /// its root differs from the one last loaded, this opens that project's content (generating the
    /// sample first when the folder is empty), spawns the god-cam over it, uploads its terrain, and
    /// opens the Scene tab - or drops the residency when the project closed. A failed open clears the
    /// scene and is not retried (the root is recorded either way). Needs a device, so it runs from
    /// `init` and the frame loop, never the pure action handler.
    fn reconcile_scene(&mut self, platform: &Platform) {
        let want = self.model.project.root().map(Path::to_path_buf);
        if want == self.loaded_root {
            return;
        }
        self.loaded_root.clone_from(&want);

        match want.map(LoadedScene::open) {
            Some(Ok(scene)) => {
                self.camera = scene.spawn_camera();
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.set_terrain(platform, &scene.store);
                }
                self.scene = Some(scene);
                // Auto-open the Scene tab so the loaded scene renders at once (open-or-focus).
                action::handle(Action::OpenScene, &mut self.model);
            }
            Some(Err(err)) => {
                eprintln!("wok: failed to open project: {err}");
                self.drop_scene();
            }
            None => self.drop_scene(),
        }
    }

    /// Drop the loaded scene and its GPU terrain - the project closed or failed to open.
    fn drop_scene(&mut self) {
        self.scene = None;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.clear_terrain();
        }
    }

    /// Advance the camera one frame, modal on the interaction mode. Free-fly flies (WASD pans, Q/E
    /// changes altitude, right-drag looks); Object is the resting mode, so the camera holds its pose.
    /// Runs after input is applied, so it flies on this frame's final mode.
    fn advance_camera(&mut self, input: &InputState, pointer_free: bool, keys_free: bool, dt: f32) {
        if self.mode == Mode::FreeFly {
            let nav = input::camera_input(input, pointer_free, keys_free);
            self.camera = camera::update(&self.camera, &nav, dt);
        }
    }

    /// Keep the window title showing the open project's name, or just the app name when none.
    fn refresh_title(&mut self, platform: &Platform) {
        let title = match self.model.project.display_name() {
            Some(name) => format!("wok - {name}"),
            None => "wok".to_string(),
        };
        if title != self.title {
            platform.window.set_title(&title);
            self.title = title;
        }
    }
}

impl App for EditorApp {
    fn init(&mut self, platform: &Platform) {
        // The OS provides the title bar, window drag, resize, and the min/max/close buttons; the
        // editor draws only its client area.
        let gui = Gui::new(platform);
        theme::apply(&gui.ctx);
        self.gui = Some(gui);
        self.gpu = Some(Gpu::new(platform));
        let config = &platform.surface_config;
        self.size = (config.width, config.height);
        // Load a startup project (from the CLI) now that a device exists.
        self.reconcile_scene(platform);
        self.refresh_title(platform);
    }

    fn on_window_event(&mut self, platform: Option<&Platform>, event: &WindowEvent) {
        if let (Some(platform), Some(gui)) = (platform, self.gui.as_mut()) {
            gui.on_event(&platform.window, event);
        }
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        // Keep the renderer's depth buffer sized to the window.
        if ctx.width > 0 && ctx.height > 0 && (ctx.width, ctx.height) != self.size {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.renderer.resize(&ctx.platform.device, ctx.width, ctx.height);
            }
            self.size = (ctx.width, ctx.height);
        }

        // Hot reload first, so the scene is current before anything reads it. A chunk change rebuilds
        // the terrain GPU meshes; prefab, scene, and light changes update the runtime arrays and
        // light state the render reads each frame.
        if let (Some(scene), Some(gpu)) = (self.scene.as_mut(), self.gpu.as_mut()) {
            if scene.poll_reload() {
                gpu.set_terrain(ctx.platform, &scene.store);
            }
        }

        // The UI: the regions read the model and the content summary and emit actions; egui's focus
        // claims decide what input the rest of the frame may use.
        let mut actions: Vec<Action> = Vec::new();
        let mut ui_output = None;
        let (mut pointer_free, mut keys_free) = (true, true);
        let mut editor_rect = egui::Rect::NOTHING;
        {
            let model = &self.model;
            let mode = self.mode;
            let content = self.scene.as_ref().map(LoadedScene::content_view);
            if let Some(gui) = self.gui.as_mut() {
                ui_output = Some(gui.run(&ctx.platform.window, |egui_ctx| {
                    editor_rect = view::chrome(egui_ctx, model, content, mode, &mut actions);
                }));
                pointer_free = !gui.ctx.is_pointer_over_area() && !gui.ctx.wants_pointer_input();
                keys_free = !gui.ctx.wants_keyboard_input();
            }
        }
        // The editor-area rect the chrome just settled into; the render confines the 3D to it.
        self.editor_rect = editor_rect;

        // Apply the actions through the one handler - the single writer for the model. The effects
        // the pure state cannot perform are carried out here: quit closes the window, a recents
        // change is flushed to disk.
        for action in actions {
            let handled = action::handle(action, &mut self.model);
            if handled.quit {
                ctx.should_close = true;
            }
            if handled.save_recents {
                recent::save(&self.model.recents);
            }
        }

        // Reconcile the scene to the open project: an OpenProject/CloseProject just applied takes
        // effect here (it needs the device).
        self.reconcile_scene(ctx.platform);

        // Viewport input: the backtick mode toggle (picking and the home-row verbs return later),
        // focus-gated so a text field types it. Then advance the camera last, on the final mode.
        self.mode = input::mode_toggle(&ctx.input, keys_free, self.mode);
        self.advance_camera(&ctx.input, pointer_free, keys_free, ctx.dt);

        // Render the scene (or the empty viewport) with the chrome painted over it.
        self.render(ctx, ui_output);
        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// The camera before any scene loads - never rendered (the empty viewport just clears), but the
/// field needs a value; `LoadedScene::spawn_camera` overwrites it when a project opens.
fn default_camera() -> FlyCamera {
    FlyCamera { position: Vec3::new(64.0, 30.0, 128.0), yaw: 0.0, pitch: -0.2, speed: 16.0 }
}
