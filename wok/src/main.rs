//! wok: the engine's reference editor application.
//!
//! This is the editor shell over the platform frame loop (see designs/editor-design.md "The shell"
//! and designs/design_handoff_editor_surfaces): the egui chrome's five regions - the full-height
//! navigation panel and its bottom icon bar, the tab bar with the app-menu hamburger, the editor well,
//! and the status bar - drawn through one composition root (`view::chrome`). The chrome reads a
//! [`Model`] and emits [`Action`](action::Action)s; this frame loop drains them through the single
//! writer, `action::handle` (see `crate::action`), so the model has exactly one mutation point. The
//! loop also carries out the effects the pure handler cannot: it persists the recent-projects list
//! when it changes (`crate::recent`), saves the open scene's chunks when an edit asks for it (Ctrl+S,
//! via `Handled::save`; `crate::loaded`), and keeps the window title on the open project.
//!
//! The editor well is the 3D Scene viewport. The authored scene (`crate::loaded`) is derived into a
//! render residency (`crate::render_scene`, distinct from the editable model) reconciled to the open
//! scene, and wok-render draws it - terrain, placeholder prefab shapes, the scene's lighting - into the
//! well's rect, flown by a mouse-only god-cam (`crate::camera`, `crate::input`). The frame order is
//! load-bearing: reconcile the authored scene, build the chrome (its focus queries decide what input
//! the camera may use), drain the chrome's actions, reconcile the render residency (an edit just
//! applied shows this frame), advance the camera last (on this frame's focus state), then draw the 3D
//! with the chrome painted over it (`crate::render`).
//!
//! The frame loop is the platform's `gfx::begin_frame -> draw -> Frame::finish` (inside `render::draw`):
//! each frame runs the egui pass (building the chrome), draws the 3D into the well rect (or clears the
//! surface to the editor background when no scene is open), then paints the chrome over it as the final
//! pass. The clear and the snapshot harness's background fill use the same `theme::palette(ctx).editor_bg`,
//! so the transparent editor well reads identically live and in the snapshot. Sizing comes from
//! `Frame::size()` (the acquired surface texture), never a separately tracked window size - see
//! designs/sharp-edges.md section 1.

mod action;
mod camera;
mod gizmo;
mod gui;
mod icons;
mod input;
mod inspector;
mod loaded;
mod menu;
mod model;
mod project;
mod recent;
mod render;
mod render_scene;
mod theme;
mod view;
mod workspace;

use action::Action;
use camera::FlyCamera;
use glam::{Vec2, Vec3};
use gui::Gui;
use loaded::LoadedScene;
use model::Model;
use render::Gpu;
use render_scene::RenderScene;
use std::path::Path;
use wok_platform::winit::dpi::PhysicalPosition;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, Desc, FrameCtx, Platform};

struct Editor {
    /// egui integration, built once a GPU device exists (`init`).
    gui: Option<Gui>,
    /// The scene-independent GPU residency (the renderer + the unit-primitive meshes + the terrain
    /// cache), built once a device exists (`init`). `None` until then.
    gpu: Option<Gpu>,
    /// The editor state the chrome reads and the action layer writes - the single writer's model: the
    /// open project, the recent-projects list, and the shell layout.
    model: Model,
    /// The active scene tab's loaded authored data (or `None` when no scene tab is active). Reconciled
    /// to the model each frame (`crate::loaded`); it is filesystem residency, so it lives here beside
    /// the model rather than inside the egui- and disk-free `Model`. The editable placements.
    loaded_scene: Option<LoadedScene>,
    /// The open scene's runtime residency the viewport draws (`crate::render_scene`), derived from
    /// `loaded_scene` and reconciled to it each frame so edits show. Separate from the editable model;
    /// `None` when no scene is open or it failed to load.
    render_scene: Option<RenderScene>,
    /// The god-cam the viewport renders through. Spawned over the scene when one loads, then advanced
    /// from the mouse each frame (`crate::camera`, `crate::input`).
    camera: FlyCamera,
    /// The cursor-lock anchor while a camera drag is held: the press position the cursor is hidden and
    /// grabbed at, restored there on release so it never jumps (`input::update_cursor_grab`). `None`
    /// when no drag is capturing the cursor.
    cursor_grab: Option<PhysicalPosition<f64>>,
    /// The in-progress transform manipulation (`crate::gizmo`): a held-key grammar (G surface move, F
    /// free move + scroll height, R rotate, S scale) advanced each frame from the mouse and the held
    /// keys. `None` when the gizmo is idle. It rides here beside the camera, since like the camera it is
    /// viewport interaction state the egui- and disk-free `Model` does not hold.
    gizmo: Option<gizmo::Hold>,
    /// The window title last pushed to the OS, so `set_title` fires only when it changes.
    title: String,
}

impl Editor {
    fn new() -> Editor {
        // Seed the recent-projects list from disk (a missing or malformed file reads as empty), then
        // reopen the most-recent recent whose folder is still present, so a relaunch returns to where
        // it left off. A deleted or moved recent is skipped (the predicate is just "the folder still
        // exists" - there is no project gate), and an empty list (a fresh install or a cleared list)
        // starts with no project. Reopen-last is routed through the single writer the same way a menu
        // open is, and the reordered recents are persisted.
        let mut model = Model { recents: recent::load(), ..Model::default() };
        if let Some(root) = project::pick_startup(&model.recents, Path::is_dir) {
            // No scene is loaded yet at startup, so the edit channel gets `None`.
            if action::handle(&mut model, None, Action::OpenProject(root)).save_recents {
                recent::save(&model.recents);
            }
        }
        Editor {
            gui: None,
            gpu: None,
            model,
            loaded_scene: None,
            render_scene: None,
            camera: default_camera(),
            cursor_grab: None,
            gizmo: None,
            title: String::new(),
        }
    }

    /// Keep the window title showing the open project's name (`wok - {name}`), or just `wok` when none
    /// is open. Pushed to the OS only when it changes, so it is cheap to call every frame.
    fn refresh_title(&mut self, platform: &Platform) {
        let title = match self.model.project.as_ref() {
            Some(project) => format!("wok - {}", project.name()),
            None => "wok".to_string(),
        };
        if title != self.title {
            platform.window.set_title(&title);
            self.title = title;
        }
    }
}

/// Resolve a viewport click into a selection action. The click is mapped against the SAME well rect
/// the 3D rendered into (sharp-edges 2 - one shared cursor-to-ray source), the cursor ray is cast, and
/// the nearest instance under it is picked. No open scene, a degenerate well, or a ray that meets only
/// terrain or empty space all deselect. The model mutation itself still goes through the single writer
/// (`Select` / `Deselect` via `action::handle`); this only turns the click into the right one, where
/// the camera and render residency a pick needs live (not the pure Model). A free function over those
/// two fields so it borrows them disjointly from the mutable `gui` the frame loop still holds.
fn resolve_viewport_pick(
    render_scene: Option<&RenderScene>,
    camera: &FlyCamera,
    pos: Vec2,
    editor_rect: egui::Rect,
) -> Action {
    let Some(scene) = render_scene else { return Action::Deselect };
    let size = Vec2::new(editor_rect.width(), editor_rect.height());
    if size.x <= 0.0 || size.y <= 0.0 {
        return Action::Deselect;
    }
    let pos_in_rect = pos - Vec2::new(editor_rect.min.x, editor_rect.min.y);
    let (origin, dir) = camera.cursor_ray(pos_in_rect, size, scene.far_plane());
    match scene.pick(origin, dir) {
        Some(id) => Action::Select(id),
        None => Action::Deselect,
    }
}

impl App for Editor {
    fn init(&mut self, platform: &Platform) {
        // The OS owns the title bar, window drag, resize, and the min/max/close buttons; the editor
        // draws only its client area. Build egui and apply the editor theme (which follows the OS
        // light/dark from here on), then the GPU residency now that a device exists.
        let gui = Gui::new(platform);
        theme::apply(&gui.ctx);
        self.gui = Some(gui);
        self.gpu = Some(Gpu::new(platform));
    }

    fn on_window_event(&mut self, platform: Option<&Platform>, event: &WindowEvent) {
        if let (Some(platform), Some(gui)) = (platform, self.gui.as_mut()) {
            gui.on_event(&platform.window, event);
        }
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        let Some(gui) = self.gui.as_mut() else { return };
        // Reconcile the loaded scene to the active tab before building the chrome, so the Instances
        // view lists the active scene's placements this frame (reload-on-tab-change; disk hot reload is
        // a later bite). This is filesystem I/O, so it lives here beside the model, not inside it. When
        // the active scene changes under it, drop the selection through the single writer: an instance
        // id is per-scene, so a selection made in one scene must not carry onto the next.
        if loaded::reconcile(&mut self.loaded_scene, &self.model) {
            action::handle(&mut self.model, self.loaded_scene.as_mut(), Action::Deselect);
        }

        // Build the chrome for this frame, reading the model and the loaded scene. The immutable borrows
        // are scoped to this block so they release before the mutable drain below. The regions emit
        // actions into a buffer rather than mutating state inside their egui closures, and return the
        // editor-well rect the 3D viewport scopes to.
        let mut actions = Vec::new();
        let mut editor_rect = egui::Rect::NOTHING;
        let output = {
            let model = &self.model;
            let loaded_scene = self.loaded_scene.as_ref();
            gui.run(&ctx.platform.window, |egui_ctx| {
                let (acts, rect) = view::chrome(egui_ctx, model, loaded_scene);
                actions.extend(acts);
                editor_rect = rect;
            })
        };

        // The mouse drives the camera only when the cursor is free for the viewport: over the well rect,
        // not under a foreground area (an open menu, the floating inspector), and not in an egui pointer
        // gesture (a panel-resize drag straying onto the well). The well is egui's background layer under
        // a CentralPanel, so is_pointer_over_area is always true over it and cannot tell the viewport
        // from a panel - gate on the rect plus the layer order plus is_using_pointer (sharp-edges 2).
        let pointer = gui.ctx.pointer_latest_pos();
        let over_viewport = pointer.is_some_and(|p| editor_rect.contains(p));
        let over_foreground =
            pointer.and_then(|p| gui.ctx.layer_id_at(p)).is_some_and(|layer| layer.order != egui::Order::Background);
        // The well rect under no foreground layer - the cursor-lock engage gate. It omits is_using_pointer
        // on purpose: on a drag's press frame egui marks the well's own deselect click-sense as the
        // potential click, so is_using_pointer (and pointer_free) is true exactly then, and gating the lock
        // on it would miss the press (the cursor would not hide until the click cleared a few pixels in). A
        // press lands over the well only for a genuine viewport drag, never a panel-resize drag (which
        // presses on the panel edge), so this is the right engage signal.
        let over_well = over_viewport && !over_foreground;
        let pointer_free = over_well && !gui.ctx.is_using_pointer();

        // Drive the transform gizmo before the action drain, where the camera, the render residency, and
        // the raw input live (not the pure Model). It advances or ends a held-key manipulation (G / F
        // move, R / S rotate / scale) and emits the resulting transform edit through the same single
        // writer the inspector uses. It reports when it consumed this frame's Esc to cancel a hold (so
        // the chrome's deselect is dropped below - a cancel keeps the selection) and when a free move
        // claimed the scroll (so the camera dolly is gated off below). It casts against the residency's
        // colliders and terrain at its far plane (one cursor-to-ray source, sharp-edges 2), so it is
        // inert until a render residency exists; any edit shows this frame via the reconcile below.
        let gizmo_out = match (self.loaded_scene.as_ref(), self.render_scene.as_ref()) {
            (Some(loaded), Some(scene)) => {
                let inputs = gizmo::Inputs {
                    input: &ctx.input,
                    camera: &self.camera,
                    loaded,
                    scene,
                    selection: self.model.shell.selection(),
                    rect: editor_rect,
                    cursor: pointer.map(|p| Vec2::new(p.x, p.y)),
                    over_well,
                    keyboard_free: !gui.ctx.wants_keyboard_input(),
                };
                gizmo::update(&mut self.gizmo, &inputs)
            }
            _ => gizmo::Outcome::default(),
        };
        if let Some(action) = gizmo_out.action {
            action::handle(&mut self.model, self.loaded_scene.as_mut(), action);
        }

        // Drain the buffer through the single writer: click -> Action -> handle, and the next frame
        // re-renders the new state. The handler returns the effects it cannot perform itself: persisting
        // the recent-projects list, and saving the open scene (handle stays filesystem-free).
        for action in actions {
            // The gizmo cancelled a hold on this frame's Esc (restoring the transform); a cancel keeps
            // the selection, so swallow the chrome's deselect that the same Esc raised. A left click no
            // longer routes through the gizmo at all - there are no handles to intercept it - so it
            // always falls through to the pick below.
            if gizmo_out.consumed_esc && matches!(action, Action::Deselect) {
                continue;
            }
            // A viewport click resolves to a pick here, where the camera and render residency live (not
            // in the pure Model): it becomes a Select of the nearest instance or a Deselect, then runs
            // through the single writer like every other action.
            let action = match action {
                Action::ViewportClick(pos) => {
                    resolve_viewport_pick(self.render_scene.as_ref(), &self.camera, pos, editor_rect)
                }
                other => other,
            };
            let handled = action::handle(&mut self.model, self.loaded_scene.as_mut(), action);
            if handled.save_recents {
                recent::save(&self.model.recents);
            }
            if handled.save {
                if let Some(scene) = self.loaded_scene.as_mut() {
                    // Best-effort write; a failure leaves the scene dirty, so the save dot stays lit as
                    // the signal (surfacing a save error is a later bite, like load errors).
                    let _ = scene.save();
                }
            }
        }

        // The editor well is a transparent egui panel, so the surface clear behind it (when no scene
        // draws) is the well's colour: the active theme's editor background. The surface is sRGB and
        // wgpu reads the clear value as linear; `render::draw` decodes it through Rgba.
        let editor_bg = theme::palette(&gui.ctx).editor_bg;

        // The render residency and the 3D pass need the device. Reconcile the residency to the open
        // scene (a fresh build spawns the god-cam over it; an in-memory edit just applied is re-derived
        // here so it shows this frame), advance the camera from the mouse last so it sees this frame's
        // focus state, then draw the 3D into the well and paint the chrome over it.
        let Some(gpu) = self.gpu.as_mut() else { return };
        if render_scene::reconcile(&mut self.render_scene, gpu, ctx.platform, self.loaded_scene.as_ref()) {
            if let Some(scene) = self.render_scene.as_ref() {
                self.camera = scene.spawn_camera();
            }
        }
        // Hide and lock the cursor while a look/pan drag pressed over the viewport is held, restoring it
        // on release (`input::update_cursor_grab`). Engage gates on `over_well` (the press frame, where
        // is_using_pointer is set by the well's own click-sense, so pointer_free would miss it). While a
        // lock is active the camera stays driven even if the captured cursor would nominally leave the
        // well, so a confined cursor (the Windows fallback) drifting over a panel does not cut the drag -
        // and it drives from frame one, covering pointer_free's press-frame dead spot.
        let lock_active = input::update_cursor_grab(&mut self.cursor_grab, &ctx.platform.window, &ctx.input, over_well);
        // The free move (`f` held) steps the instance's height with the wheel, so gate the camera's
        // scroll-dolly off this frame when the gizmo claimed the scroll; everything else (look, pan,
        // and dolly when `f` is not held) is untouched. The gizmo ran above, so the flag is this frame's.
        let mut camera_input = input::camera_input(&ctx.input, pointer_free || lock_active);
        if gizmo_out.consumed_scroll {
            camera_input.dolly = 0.0;
        }
        self.camera = camera::update(&self.camera, &camera_input);
        render::draw(ctx.platform, gpu, self.render_scene.as_ref(), self.camera, editor_rect, editor_bg, gui, output);

        // Keep the window title on the open project's name (or just the app name when none).
        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// The camera before any scene loads - never rendered (the empty well just clears), but the field
/// needs a value; [`RenderScene::spawn_camera`] overwrites it when a scene opens.
fn default_camera() -> FlyCamera {
    FlyCamera { position: Vec3::new(64.0, 30.0, 128.0), yaw: 0.0, pitch: -0.2 }
}

fn main() {
    wok_platform::run(Editor::new(), Desc { title: "wok", width: 0, height: 0, vsync: true });
}
