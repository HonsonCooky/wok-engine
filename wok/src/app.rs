//! The editor application: the window state, the camera, and the per-frame loop.
//!
//! The chrome (`crate::menu`, `crate::workspace`, composed by `crate::view`) reads the model and the
//! open project's content summary and emits actions the loop applies through the one handler
//! (`crate::action`, the single writer). The scene the viewport draws is a separate residency
//! (`crate::scene`, `LoadedScene`) reconciled to the open project: opening a project loads (or first
//! generates) its content and uploads its GPU meshes, closing it drops them. The camera
//! (`crate::camera`) is mouse-only and always live: right-drag looks, scroll dollies, middle-drag pans
//! (`crate::input`), with no mode to enter.
//!
//! The frame order is load-bearing: hot reload first (the scene is current before anything reads it),
//! then the UI (its focus queries decide what input the rest of the frame may use), then the UI's
//! actions, then the scene reconcile (an open/close just applied takes effect), then the camera
//! advance - last, so it navigates on this frame's focus state - and finally the render with the
//! chrome painted over it.

use std::path::{Path, PathBuf};

use glam::Vec3;
use wok_platform::input::InputState;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform};

use crate::action::{self, Action};
use crate::camera::{self, FlyCamera};
use crate::content::ContentPaths;
use crate::gui::Gui;
use crate::input;
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
    /// The project root the scene residency last reconciled to. Recorded only on a successful open
    /// (and cleared to `None` on failure or close), so reconcile acts whenever the project's root
    /// differs from this - and picking a folder that failed to open retries rather than sticking.
    loaded_root: Option<PathBuf>,
    /// The last open failure's message, surfaced in the status bar until the next successful open.
    /// `None` when the last open succeeded or no open has failed. App-side, not `Model` state: it
    /// arises from the device-side reconcile, not a pure action.
    open_error: Option<String>,
    /// The camera the renderer reads. Spawned over the scene when a project loads, then advanced from
    /// the mouse each frame (`crate::camera`, `crate::input`).
    pub(crate) camera: FlyCamera,
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
    /// as empty, so the editor always starts). The startup project is an explicit CLI folder if given,
    /// otherwise the most-recent recent that is still a loadable wok project (so a fresh install or a
    /// cleared list starts empty); either is routed through the one writer so it lands in recents the
    /// way a menu open does, and its content loads in `init`, once a GPU device exists.
    pub fn new(initial: Option<PathBuf>) -> EditorApp {
        let mut model = Model::new(Project::None);
        model.recents = recent::load();
        // Reopen-last selects only a folder that still holds a scene.json, so it never generates: a
        // missing or emptied recent is skipped, falling back to no project. Creating a project stays
        // an explicit act (an empty folder opened from the menu), never a launch side effect.
        let startup = initial.or_else(|| pick_startup(&model.recents, is_wok_project));
        if let Some(root) = startup {
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
            open_error: None,
            camera: default_camera(),
            size: (0, 0),
            // Overwritten by the first frame's chrome before any render reads it; NOTHING reads as
            // "no usable rect", which the render treats as the full target.
            editor_rect: egui::Rect::NOTHING,
            title: String::new(),
        }
    }

    /// Reconcile the scene residency to the open project. The project is the source of truth: when
    /// its root differs from the one last loaded, this opens that project's content (generating the
    /// starter scene into an empty folder; a non-empty non-wok folder is an error), spawns the god-cam
    /// over it, uploads its terrain, and opens the Scene tab - or drops the residency when the project
    /// closed. On a failed open it surfaces the error in the status bar and falls back to no project,
    /// so the editor stays usable and picking the same (or another) folder retries: `loaded_root` is
    /// recorded only on success, never poisoned by a failure. Needs a device, so it runs from `init`
    /// and the frame loop, never the pure action handler.
    fn reconcile_scene(&mut self, platform: &Platform) {
        let want = self.model.project.root().map(Path::to_path_buf);
        if want == self.loaded_root {
            return;
        }

        match want.as_ref().map(|root| LoadedScene::open(root.clone())) {
            Some(Ok(scene)) => {
                self.loaded_root = want;
                self.open_error = None;
                self.camera = scene.spawn_camera();
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.set_terrain(platform, &scene.store);
                }
                self.scene = Some(scene);
                // Auto-open the Scene tab so the loaded scene renders at once (open-or-focus).
                action::handle(Action::OpenScene, &mut self.model);
            }
            Some(Err(err)) => {
                // Surface the failure and fall back to no project. Closing it (through the one writer)
                // leaves the project and `loaded_root` agreed at None, so this does not retry every
                // frame, while re-picking the folder sets the project again and does retry.
                self.open_error = Some(format!("{err}"));
                self.drop_scene();
                action::handle(Action::CloseProject, &mut self.model);
                self.loaded_root = None;
            }
            None => {
                self.loaded_root = None;
                self.open_error = None;
                self.drop_scene();
            }
        }
    }

    /// Drop the loaded scene and its GPU terrain - the project closed or failed to open.
    fn drop_scene(&mut self) {
        self.scene = None;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.clear_terrain();
        }
    }

    /// Advance the camera one frame from the mouse: right-drag looks, scroll dollies along the look,
    /// middle-drag pans the view plane (`crate::camera`, `crate::input`). Always live - there is no
    /// camera mode - but inert unless the cursor is free for the viewport (`pointer_free`), so the
    /// chrome and an open menu keep their own pointer input. Runs after the UI, so it sees this frame's
    /// focus state.
    fn advance_camera(&mut self, input: &InputState, pointer_free: bool) {
        let nav = input::camera_input(input, pointer_free);
        self.camera = camera::update(&self.camera, &nav);
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
        let mut pointer_free = true;
        let mut editor_rect = egui::Rect::NOTHING;
        {
            let model = &self.model;
            let content = self.scene.as_ref().map(LoadedScene::content_view);
            let open_error = self.open_error.as_deref();
            if let Some(gui) = self.gui.as_mut() {
                ui_output = Some(gui.run(&ctx.platform.window, |egui_ctx| {
                    editor_rect = view::chrome(egui_ctx, model, content, open_error, &mut actions);
                }));
                // Look, scroll, and pan drive the camera only when the cursor is over the editor-area
                // viewport and egui is not using the pointer for its own UI. The viewport is egui's
                // background layer, and a CentralPanel marks the whole central region as used, so
                // is_pointer_over_area() - and thus wants_pointer_input() on a hover with no button
                // down - is always true over it; neither can tell our viewport from a panel. So gate
                // on the rect itself, exclude any foreground area sitting over it (an open menu, and
                // later the floating inspector, which are not the background layer), and exclude an
                // in-progress egui widget drag (a panel-resize sweep that strays onto the viewport).
                let pointer = gui.ctx.pointer_latest_pos();
                let over_viewport = pointer.is_some_and(|p| editor_rect.contains(p));
                let over_foreground = pointer
                    .and_then(|p| gui.ctx.layer_id_at(p))
                    .is_some_and(|layer| layer.order != egui::Order::Background);
                pointer_free = over_viewport && !over_foreground && !gui.ctx.is_using_pointer();
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

        // Advance the camera from the mouse last, so it sees this frame's focus state. Picking and the
        // home-row verbs (which will read keyboard focus) return with later surfaces.
        self.advance_camera(&ctx.input, pointer_free);

        // Render the scene (or the empty viewport) with the chrome painted over it.
        self.render(ctx, ui_output);
        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// The camera before any scene loads - never rendered (the empty viewport just clears), but the
/// field needs a value; `LoadedScene::spawn_camera` overwrites it when a project opens.
fn default_camera() -> FlyCamera {
    FlyCamera { position: Vec3::new(64.0, 30.0, 128.0), yaw: 0.0, pitch: -0.2 }
}

/// The most-recent recent project that satisfies `is_project`, most-recent first, or `None`. Pure
/// over the predicate so the most-recent-first selection is testable without a filesystem; the live
/// caller passes the on-disk [`is_wok_project`] check.
fn pick_startup(recents: &recent::Recents, is_project: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    recents.paths().iter().find(|root| is_project(root)).cloned()
}

/// Whether `root` is a loadable wok project: it holds a `scene.json`. Reopen-last selects only these,
/// so a recent whose folder was deleted or emptied is skipped rather than regenerated on launch.
fn is_wok_project(root: &Path) -> bool {
    ContentPaths::new(root.to_path_buf()).scene().is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_startup_takes_the_most_recent_matching_project() {
        // Most-recent first: c, b, a. Only a and b qualify as projects, so reopen-last picks the more
        // recent of those (b), skipping the non-project c at the front.
        let recents = recent::Recents::from_paths(["c", "b", "a"].iter().map(PathBuf::from));
        let picked = pick_startup(&recents, |p| p == Path::new("a") || p == Path::new("b"));
        assert_eq!(picked, Some(PathBuf::from("b")));
    }

    #[test]
    fn pick_startup_is_none_when_nothing_qualifies() {
        // A list of recents none of which is still a project (all deleted or emptied) starts empty.
        let recents = recent::Recents::from_paths(["a", "b"].iter().map(PathBuf::from));
        assert_eq!(pick_startup(&recents, |_| false), None);
        // An empty recents list is also none, with no panic on the empty iterator.
        assert_eq!(pick_startup(&recent::Recents::default(), |_| true), None);
    }
}
