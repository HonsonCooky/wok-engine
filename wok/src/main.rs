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
//! well's rect through a god-cam (`crate::camera`). The frame order is load-bearing: reconcile the
//! authored scene, build the chrome, drain the chrome's actions (a viewport click resolves to a pick
//! here), reconcile the render residency (an edit just applied shows this frame), then draw the 3D with
//! the chrome painted over it (`crate::render`).
//!
//! The viewport interaction is the keyboard-first camera and selection model
//! (designs/movement-camera-design.md, the 2026-06-29 revision: one free-fly god camera). It runs in the
//! interaction seam (`crate::interaction`), between the action drain and the draw: the status-bar target
//! toggle aims the directional cluster, which flies the camera (the Look target) or grid-steps the
//! selection (the Move target); a held right-drag looks (the cursor locked for the drag). After the
//! render residency reconciles, the Frame key (`f`) eases the camera onto the selection
//! (`interaction::framing`) - explicit, never automatic. The mouse resolves a left click to a selection
//! and a left drag to a drag-and-drop move. The inspector stays the precise editing surface.
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
mod geom;
mod gui;
mod icons;
mod inspector;
mod interaction;
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
use camera::Camera;
use glam::{Vec2, Vec3};
use gui::Gui;
use interaction::Interaction;
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
    /// The camera the viewport renders through: one free-fly god camera (`crate::camera`). Positioned over
    /// the scene when one loads (`RenderScene::spawn_camera`), then flown by the interaction seam's Look
    /// target and the right-drag look, and eased onto the selection by the Frame verb (`crate::interaction`).
    camera: Camera,
    /// The keyboard-first interaction's cross-frame state (`crate::interaction`): the drag-and-drop grab.
    /// The keyboard verbs are stateless; only the mouse drag remembers its grab between frames.
    interaction: Interaction,
    /// The cursor-lock anchor while a right-drag look is held: the press position the cursor is hidden and
    /// pinned at, restored there on release so it never jumps (`interaction::update_cursor_grab`). `None`
    /// when no look drag is active.
    cursor_grab: Option<PhysicalPosition<f64>>,
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
            interaction: Interaction::new(),
            cursor_grab: None,
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

        // Drain the buffer through the single writer, and the next frame re-renders the new state. A
        // viewport gesture resolves first, where the camera, the render residency, the well rect, and the
        // drag grab a pick / surface-follow need live (not the pure Model): it becomes the Select /
        // Deselect / SetInstanceTransform the gesture implies (`crate::interaction`), each run through the
        // writer like every other action. The handler returns the effects it cannot perform itself:
        // persisting the recent-projects list, and saving the open scene (handle stays filesystem-free).
        for action in actions {
            let resolved = match action {
                Action::ViewportGesture(gesture) => self.interaction.gesture(
                    gesture,
                    self.render_scene.as_ref(),
                    &self.camera,
                    editor_rect,
                    self.loaded_scene.as_ref(),
                ),
                other => vec![other],
            };
            for action in resolved {
                let handled = action::handle(&mut self.model, self.loaded_scene.as_mut(), action);
                if handled.save_recents {
                    recent::save(&self.model.recents);
                }
                if handled.save {
                    if let Some(scene) = self.loaded_scene.as_mut() {
                        // Best-effort write; a failure leaves the scene dirty, so the save dot stays lit
                        // as the signal (surfacing a save error is a later bite, like load errors).
                        let _ = scene.save();
                    }
                }
            }
        }

        // ---- interaction seam (designs/movement-camera-design.md) ----
        // The keyboard-first verbs read wok-platform InputState (not egui), here between the drain and
        // the draw. Focus-gated by egui's keyboard-want, so a focused text field types instead of driving
        // the editor. The Look target flies the camera in place (camera state is frame-loop residency, not
        // the model); the Move target grid-steps the selection and the target toggle both route through the
        // single writer like every other action.
        let typing = gui.ctx.wants_keyboard_input();
        for action in interaction::keyboard(&ctx.input, typing, &self.model, self.loaded_scene.as_ref(), &mut self.camera) {
            action::handle(&mut self.model, self.loaded_scene.as_mut(), action);
        }

        // The mouse looks the camera only over the well: the pointer is in the well rect and under no
        // foreground egui layer (a menu, the floating inspector). The well is egui's background layer under
        // a CentralPanel, so is_pointer_over_area is always true over it and cannot tell the viewport from
        // a panel - gate on the rect plus the layer order (sharp-edges 2). `over_well` omits
        // is_using_pointer so a right-press engages the cursor lock on its press frame (the well's own
        // click-sense sets is_using_pointer then; gating on it would miss the engage).
        let pointer = gui.ctx.pointer_latest_pos();
        let over_well = pointer.is_some_and(|p| editor_rect.contains(p))
            && pointer.and_then(|p| gui.ctx.layer_id_at(p)).is_none_or(|layer| layer.order == egui::Order::Background);

        // The editor well is a transparent egui panel, so the surface clear behind it (when no scene
        // draws) is the well's colour: the active theme's editor background. The surface is sRGB and
        // wgpu reads the clear value as linear; `render::draw` decodes it through Rgba.
        let editor_bg = theme::palette(&gui.ctx).editor_bg;

        // The render residency and the 3D pass need the device. Reconcile the residency to the open scene
        // (a fresh build spawns the god-cam over it; an in-memory edit just applied is re-derived here so
        // it shows this frame), then drive the camera from the mouse and ease any Frame transition, then
        // draw the 3D into the well and paint the chrome over it.
        let Some(gpu) = self.gpu.as_mut() else { return };
        if render_scene::reconcile(&mut self.render_scene, gpu, ctx.platform, self.loaded_scene.as_ref()) {
            if let Some(scene) = self.render_scene.as_ref() {
                self.camera = scene.spawn_camera();
            }
        }

        // Hide and pin the cursor while a right-drag look pressed over the viewport is held, restoring it
        // on release (`interaction::update_cursor_grab`). The look fires exactly while the cursor is
        // locked, reading raw motion (which a synthetic warp never produces, so the per-frame re-warp pins
        // the cursor without feeding the look). lock_active drives from the press frame and keeps driving
        // even as a confined cursor drifts off the well.
        let lock_active = interaction::update_cursor_grab(&mut self.cursor_grab, &ctx.platform.window, &ctx.input, over_well);
        if lock_active {
            self.camera.look(Vec2::new(ctx.input.mouse_motion.0 as f32, ctx.input.mouse_motion.1 as f32));
        }

        // The Frame verb (f) and the per-frame ease advance, after the residency reconciles so the
        // selection's world bounds are current. Framing is explicit: a selection change moves nothing on
        // its own (`interaction::framing`).
        interaction::framing(&ctx.input, typing, &self.model, self.render_scene.as_ref(), &mut self.camera);

        render::draw(ctx.platform, gpu, self.render_scene.as_ref(), self.camera, editor_rect, editor_bg, gui, output);

        // Keep the window title on the open project's name (or just the app name when none).
        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

/// The camera before any scene loads - never rendered (the empty well just clears), but the field
/// needs a value; [`RenderScene::spawn_camera`] overwrites it when a scene opens.
fn default_camera() -> Camera {
    Camera::over(Vec3::ZERO)
}

fn main() {
    wok_platform::run(Editor::new(), Desc { title: "wok", width: 0, height: 0, vsync: true });
}
