//! The editor's keyboard-first spatial interaction: the directional cluster, the vertical pair, and the
//! target toggle, read from wok-platform [`InputState`] and turned into camera nav or selection edits.
//!
//! This is the rebuilt interaction layer (designs/movement-camera-design.md), the clean-slate
//! replacement for the demolished mouse-only camera and held-key gizmo. The whole scheme is one
//! directional cluster (4-way) plus a vertical pair (raise / lower), time-shared by the persistent
//! [`Target`](crate::model::Target) toggle: in `Look` the cluster drives the [`Camera`] (pan + zoom in
//! Layout, orbit + dolly in Orbit), in `Move` they grid-step the selection (the screen cardinals in
//! Layout, the world axis nearest the camera in Orbit). A dedicated key cycles the camera mode
//! (Layout <-> Orbit), separate from the target toggle. The verbs read wok-platform input (not egui), so
//! the same grammar maps onto a controller later (gilrs is in wok-platform); the chrome's egui shortcuts
//! (Ctrl+S, Esc) stay separate.
//!
//! The mouse is for the big jump: [`Interaction::gesture`] resolves the viewport pointer gestures the
//! well raises (`crate::workspace::editor_area`) - a click selects the instance under the cursor, and a
//! drag grabs it and drops it on the surface under the cursor (drag-and-drop, snapped to the grid). The
//! drag grab is the one piece of cross-frame state, held on [`Interaction`]; the keyboard verbs are
//! stateless.
//!
//! Where it runs: the frame loop's interaction seam (`crate::main`), after the chrome's actions drain
//! and before the draw - the spot the old interaction plugged into. [`keyboard`] reads the cluster each
//! frame and either mutates the camera (Look) or returns the selection's grid-step (Move); the mouse
//! [`Interaction::gesture`] resolves a well gesture against the camera, the render residency, and the
//! grab. Both return actions the frame loop routes through the single writer, and both are gated so a
//! focused text field (keyboard) or a higher egui layer (mouse) takes the input instead.
//!
//! Pure where it can be: the cluster-to-cardinal mapping ([`cluster_step`]) and the grid step
//! ([`grid_step`]) are pure and unit tested; the camera math lives in [`crate::camera`]. `keyboard` and
//! `gesture` read an [`InputState`] snapshot or a [`Gesture`], so they are testable by building one.
//!
//! Keybinding is PARKED: the keycaps below are temporary, sane placeholders so the verbs are testable
//! now, NOT the final layout. The binding settles as a rebindable table fitted to the ZSA Voyager
//! (movement-camera-design.md "Keybinding"); the prior scheme failed on left-hand modifier reach, which
//! that binding addresses, so do not read these letters as the design.

use glam::{Vec2, Vec3};
use wok_platform::input::InputState;
use wok_platform::winit::keyboard::NamedKey;
use wok_scene::{InstanceId, Transform};

use crate::action::{Action, Gesture};
use crate::camera::Camera;
use crate::geom;
use crate::loaded::LoadedScene;
use crate::model::{Model, Target};
use crate::render_scene::RenderScene;

// ---- placeholder keybinds (PARKED - see the module doc) ----

/// The directional cluster (4-way), camera-relative. In Layout (top-down) the screen directions map to
/// fixed world cardinals (see [`cluster_step`]).
const CLUSTER_FORWARD: char = 'w'; // screen up    -> world -Z (north)
const CLUSTER_BACK: char = 's'; //    screen down  -> world +Z (south)
const CLUSTER_LEFT: char = 'a'; //    screen left  -> world -X (west)
const CLUSTER_RIGHT: char = 'd'; //   screen right -> world +X (east)

/// The vertical pair: world +Y / -Y in Move, zoom out / in in Look (Layout) or dolly out / in (Orbit).
const RAISE: char = 'e';
const LOWER: char = 'q';

/// The target toggle (Move <-> Look) - a thumb-tap placeholder.
const TOGGLE: NamedKey = NamedKey::Space;

/// The camera-mode cycle (Layout <-> Orbit) - a one-shot tap, separate from the target toggle.
const MODE_CYCLE: char = 'c';

/// The 1m world grid the keyboard move and the drag both snap to (the surviving grid-snap;
/// movement-camera-design.md "Move"). World placement is grid-locked - fine work is the inspector.
const GRID_STEP: f32 = 1.0;

/// Read one frame of keyboard input and drive the editor: the toggle flips the cluster target and the
/// mode key cycles the camera mode, then the cluster and vertical pair either drive the camera (Look) or
/// step the selection (Move). Returns the actions for the frame loop to route through the single writer
/// (the toggle, and - in Move - the selection's transform edits); the camera mutates in place, since it
/// is frame-loop residency, not model state (so the mode cycle and the Look nav route through neither the
/// writer nor an action).
///
/// Look drives the [`Camera`] by mode: in Layout the cluster pans and the vertical pair zooms, in Orbit
/// the cluster orbits (yaw/pitch) and the vertical pair dollies. Move grid-steps the selection one cell
/// per input: the screen cardinals ARE the world cardinals (exact under the top-down Layout view; the
/// camera-relative Orbit mapping is a later commit), the vertical pair stepping world Y. All Move steps
/// snap to the grid (grid-locked, no fine nudge) and route through the single writer, so the edit dirties
/// the scene and Ctrl+S persists it.
///
/// Focus-gated: `typing` is true when a text field holds keyboard focus, and a held Ctrl is a chrome
/// chord (Ctrl+S and friends), so in either case the spatial verbs stay inert and the keys reach the
/// chrome instead. Hold-to-repeat rides the OS key-repeat (wok-platform's `keys_repeating`): a tap is one
/// step, a hold repeats at the OS rate. The toggle and the mode cycle are the press edge only, so a held
/// thumb does not flip-flop.
pub fn keyboard(
    input: &InputState,
    typing: bool,
    model: &Model,
    loaded: Option<&LoadedScene>,
    camera: &mut Camera,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if typing || input.key_held(NamedKey::Control) || input.key_held(NamedKey::Super) {
        return actions;
    }
    if input.key_pressed(TOGGLE) {
        actions.push(Action::ToggleTarget);
    }
    // The camera-mode cycle is a one-shot tap (press edge), so a held key does not spin through modes.
    // Camera residency, not model state, so it mutates in place rather than routing through the writer.
    if input.char_pressed(MODE_CYCLE) {
        camera.cycle_mode();
    }
    // The press edge OR an OS auto-repeat, so a tap steps once and a hold repeats.
    let on = |c: char| input.char_pressed(c) || input.char_repeating(c);
    let (dx, dz) = cluster_step(on(CLUSTER_FORWARD), on(CLUSTER_BACK), on(CLUSTER_LEFT), on(CLUSTER_RIGHT));
    let dy = on(RAISE) as i32 - on(LOWER) as i32;
    match model.shell.target() {
        Target::Look => {
            // The cluster and vertical pair drive the camera by mode (pan/zoom in Layout, orbit/dolly in
            // Orbit), in place - camera-only, so nothing routes through the writer, and the manual nav
            // decouples the auto-follow (survey mode; `crate::camera`).
            if dx != 0 || dz != 0 {
                camera.look_cluster(dx, dz);
            }
            if dy != 0 {
                camera.look_vertical(dy);
            }
        }
        Target::Move => {
            // The cluster grid-steps the selection one cell per input (camera-relative: in the top-down
            // Layout the screen cardinals ARE the world cardinals, exact - the view looks straight down;
            // the Orbit-relative mapping is a later commit); the vertical pair steps its world Y,
            // camera-independent. All snapped to the grid (grid-locked, no fine nudge) and routed through
            // the single writer, so the edit dirties the scene and Ctrl+S persists it.
            if dx != 0 || dz != 0 || dy != 0 {
                if let (Some(id), Some(loaded)) = (model.shell.selection(), loaded) {
                    if let Some(placement) = loaded.placement(id) {
                        let translation = grid_step(placement.transform.translation, dx, dy, dz, GRID_STEP);
                        actions.push(Action::SetInstanceTransform(id, Transform { translation, ..placement.transform }));
                    }
                }
            }
        }
    }
    actions
}

/// The interaction's cross-frame state: the drag-and-drop grab. The keyboard verbs are stateless
/// ([`keyboard`]); only the mouse drag must remember, between frames, which instance it grabbed.
#[derive(Default)]
pub struct Interaction {
    /// The instance a press grabbed and the drag is moving, or `None` when not dragging.
    grabbed: Option<InstanceId>,
}

impl Interaction {
    pub fn new() -> Interaction {
        Interaction::default()
    }

    /// Resolve one viewport pointer [`Gesture`] (egui-gated to the well) into the selection and transform
    /// edits it implies, updating the drag grab. The frame loop owns the camera, the render residency,
    /// the well rect, and this grab - which the pure handler cannot - so it resolves the gesture here and
    /// routes the returned actions through the single writer.
    ///
    /// - [`Click`](Gesture::Click): select the instance under the cursor, or deselect over empty space.
    /// - [`GrabStart`](Gesture::GrabStart): begin a drag - grab and select the instance under the cursor.
    ///   A drag begun on empty space grabs nothing and leaves the selection.
    /// - [`GrabMove`](Gesture::GrabMove): the grabbed instance follows the cursor's surface point, snapped
    ///   to the grid in XZ and rested on the surface in Y. No surface under the cursor (empty sky): hold,
    ///   rather than fling it.
    /// - [`GrabEnd`](Gesture::GrabEnd): drop (each move already committed through the seam; clear the grab).
    pub fn gesture(
        &mut self,
        gesture: Gesture,
        render_scene: Option<&RenderScene>,
        camera: &Camera,
        editor_rect: egui::Rect,
        loaded: Option<&LoadedScene>,
    ) -> Vec<Action> {
        let Some(scene) = render_scene else {
            // No scene to pick or rest against: a click clears any selection, a grab is impossible.
            self.grabbed = None;
            return match gesture {
                Gesture::Click(_) => vec![Action::Deselect],
                _ => vec![],
            };
        };
        let size = Vec2::new(editor_rect.width(), editor_rect.height());
        if size.x <= 0.0 || size.y <= 0.0 {
            return vec![];
        }
        match gesture {
            Gesture::Click(pos) => match pick(scene, camera, pos, editor_rect, size) {
                Some(id) => vec![Action::Select(id)],
                None => vec![Action::Deselect],
            },
            Gesture::GrabStart(pos) => match pick(scene, camera, pos, editor_rect, size) {
                Some(id) => {
                    self.grabbed = Some(id);
                    vec![Action::Select(id)]
                }
                None => {
                    self.grabbed = None;
                    vec![]
                }
            },
            Gesture::GrabMove(pos) => self.drag_to(scene, camera, pos, editor_rect, size, loaded),
            Gesture::GrabEnd => {
                self.grabbed = None;
                vec![]
            }
        }
    }

    /// The grabbed instance follows the cursor's surface point: cast the ortho ray, rest on whatever lies
    /// under the cursor (ground, terrain, or another prefab - the grabbed instance excluded so it never
    /// snaps to itself), snap XZ to the grid, and rest the item's bottom on the surface with the pivot
    /// snapped to the grid (the same drop the keyboard move lands on). Returns the transform edit, or
    /// nothing when there is no grab, no loaded placement, or no surface under the cursor.
    fn drag_to(
        &self,
        scene: &RenderScene,
        camera: &Camera,
        pos: Vec2,
        editor_rect: egui::Rect,
        size: Vec2,
        loaded: Option<&LoadedScene>,
    ) -> Vec<Action> {
        let Some(id) = self.grabbed else { return vec![] };
        let Some(placement) = loaded.and_then(|l| l.placement(id)) else { return vec![] };
        let pos_in_rect = pos - Vec2::new(editor_rect.min.x, editor_rect.min.y);
        let (origin, dir) = camera.cursor_ray(pos_in_rect, size, scene.far_plane());
        let Some(hit) = scene.surface_ray(origin, dir, id) else { return vec![] };
        let Some(aabb) = scene.instance_aabb(id) else { return vec![] };
        let t = placement.transform;
        // The world hit is written into the chunk-local translation - exact for the single-chunk scenes
        // the editor authors today; the world-to-local re-home is the deferred multi-chunk bite.
        let pivot_y = geom::snap(geom::rest_y(hit.y, t.translation.y, aabb.min.y), GRID_STEP);
        let translation = Vec3::new(geom::snap(hit.x, GRID_STEP), pivot_y, geom::snap(hit.z, GRID_STEP));
        vec![Action::SetInstanceTransform(id, Transform { translation, ..t })]
    }
}

/// Pick the instance under the cursor via the camera's straight-down ortho ray, or `None` over empty
/// space or terrain. The well rect maps the window-space click into the rect the ray casts through
/// (sharp-edges 2: one shared cursor-to-ray source).
fn pick(scene: &RenderScene, camera: &Camera, pos: Vec2, editor_rect: egui::Rect, size: Vec2) -> Option<InstanceId> {
    let pos_in_rect = pos - Vec2::new(editor_rect.min.x, editor_rect.min.y);
    let (origin, dir) = camera.cursor_ray(pos_in_rect, size, scene.far_plane());
    scene.pick(origin, dir)
}

/// The world-cardinal grid step the directional cluster maps to in Layout (top-down): screen up is world
/// `-Z` (north), down `+Z`, left `-X`, right `+X` - exact, because the view looks straight down
/// (movement-camera-design.md "Move"). Returns `(dx, dz)` in grid cells (one cell per pressed direction;
/// opposite presses cancel). In Look this is the pan direction; in Move (next bite) the selection step.
///
/// Four bools, one per cluster direction: they are independent (forward + left is a held diagonal), so a
/// two-variant enum cannot stand in for them - the directions are the natural signature here.
#[allow(clippy::fn_params_excessive_bools)]
fn cluster_step(forward: bool, back: bool, left: bool, right: bool) -> (i32, i32) {
    let dx = right as i32 - left as i32;
    let dz = back as i32 - forward as i32;
    (dx, dz)
}

/// Step a translation by whole grid cells and snap to the grid, so a keyboard move always lands on it
/// (movement-camera-design.md: grid-locked, no fine nudge). Snap-then-step: the position snaps to its
/// nearest cell, then moves exactly `(dx, dy, dz)` cells of `step` - so each input is one clean cell from
/// the grid even if the item arrived off-grid (placed via the inspector). Pure, so the step is unit
/// tested.
fn grid_step(translation: Vec3, dx: i32, dy: i32, dz: i32, step: f32) -> Vec3 {
    Vec3::new(
        geom::snap(translation.x, step) + dx as f32 * step,
        geom::snap(translation.y, step) + dy as f32 * step,
        geom::snap(translation.z, step) + dz as f32 * step,
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};
    use std::collections::HashSet;
    use wok_platform::winit::keyboard::Key;

    /// Build an [`InputState`] snapshot with the given character keys pressed (and so held). Enough to
    /// drive [`keyboard`]; the mouse, scroll, and gamepad fields stay empty.
    fn input_with_chars(pressed: &[char]) -> InputState {
        let keys_pressed: HashSet<Key> = pressed.iter().map(|c| Key::Character(c.to_string().into())).collect();
        InputState {
            keys_held: keys_pressed.clone(),
            keys_pressed,
            keys_repeating: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: (0.0, 0.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 0.0),
            gamepads: Vec::new(),
        }
    }

    #[test]
    fn cluster_step_maps_screen_directions_to_world_cardinals() {
        // The map orientation, exact under the straight-down view: forward is north (-Z), right is east.
        assert_eq!(cluster_step(true, false, false, false), (0, -1), "forward steps world -Z (north)");
        assert_eq!(cluster_step(false, true, false, false), (0, 1), "back steps +Z (south)");
        assert_eq!(cluster_step(false, false, true, false), (-1, 0), "left steps -X (west)");
        assert_eq!(cluster_step(false, false, false, true), (1, 0), "right steps +X (east)");
        assert_eq!(cluster_step(true, true, true, true), (0, 0), "opposite presses cancel");
        assert_eq!(cluster_step(true, false, false, true), (1, -1), "forward + right is a diagonal NE step");
    }

    #[test]
    fn keyboard_look_drives_the_layout_camera_with_no_model_action() {
        // Look in the default Layout mode pans and zooms, observed through the public eye: the precise pan
        // and zoom math is camera.rs's; here we pin that the Look target routes the cluster to the camera
        // and emits no model action. (Move-target asserts and the Orbit path are the other tests.)
        let mut model = Model::default();
        model.shell.toggle_target(); // -> Look
        let mut cam = Camera::over(Vec3::new(5.0, 0.0, 5.0));
        let before = cam.eye();
        let actions = keyboard(&input_with_chars(&['w']), false, &model, None, &mut cam);
        assert!(actions.is_empty(), "Look is camera-only - no model edit to route");
        assert!(cam.eye().z < before.z, "forward pans the focus north (-Z): {:?}", cam.eye());
        // The vertical pair zooms: raise floats the eye higher (a wider top-down view).
        let panned = cam.eye();
        let _ = keyboard(&input_with_chars(&['e']), false, &model, None, &mut cam);
        assert!(cam.eye().y > panned.y, "raise zooms out in Look");
    }

    #[test]
    fn keyboard_cycles_the_camera_mode_on_the_press_edge() {
        // The mode key flips Layout <-> Orbit on a tap (camera residency, so no action routes), and the
        // status bar reads the result through `mode()`.
        use crate::camera::Mode;
        let model = Model::default();
        let mut cam = Camera::over(Vec3::ZERO);
        assert_eq!(cam.mode(), Mode::Layout, "the default home");
        let actions = keyboard(&input_with_chars(&['c']), false, &model, None, &mut cam);
        assert!(actions.is_empty(), "the mode cycle is camera-only - no model action");
        assert_eq!(cam.mode(), Mode::Orbit, "the mode key cycles to Orbit");
    }

    #[test]
    fn keyboard_toggle_routes_through_the_writer_and_a_focused_field_swallows_the_keys() {
        let model = Model::default();
        let mut cam = Camera::over(Vec3::ZERO);
        // The toggle key emits ToggleTarget (the frame loop applies it through the single writer).
        let actions = keyboard(&input_with_chars(&[]), false, &model, None, &mut cam);
        assert!(actions.is_empty(), "no keys, no actions");
        let toggled = {
            let mut input = input_with_chars(&[]);
            input.keys_pressed.insert(Key::Named(NamedKey::Space));
            input.keys_held.insert(Key::Named(NamedKey::Space));
            keyboard(&input, false, &model, None, &mut cam)
        };
        assert_eq!(toggled, vec![Action::ToggleTarget], "the toggle key emits ToggleTarget");
        // Focus-gated: with a text field focused, every verb key is inert and the camera does not move.
        let before = cam.eye();
        let gated = keyboard(&input_with_chars(&['w']), true, &model, None, &mut cam);
        assert!(gated.is_empty() && cam.eye() == before, "a focused field swallows the verb keys");
    }

    #[test]
    fn grid_step_moves_one_cell_and_snaps_to_the_grid() {
        // One cell per input on the 1m grid, snap-then-step: an on-grid item moves exactly one cell each
        // pressed axis (right +X, forward -Z here), the vertical pair steps world Y, and an off-grid item
        // (placed via the inspector) snaps to its nearest cell, then steps one.
        assert_eq!(grid_step(Vec3::new(4.0, 0.0, -3.0), 1, 0, -1, 1.0), Vec3::new(5.0, 0.0, -4.0), "on-grid: one cell each");
        assert_eq!(grid_step(Vec3::new(2.0, 1.0, 2.0), 0, 1, 0, 1.0), Vec3::new(2.0, 2.0, 2.0), "raise steps +Y one cell");
        let stepped = grid_step(Vec3::new(3.4, 0.0, -2.1), 1, 0, -1, 1.0);
        assert_eq!(stepped, Vec3::new(4.0, 0.0, -3.0), "off-grid snaps to a cell then steps one");
        assert_eq!(stepped.x.fract(), 0.0, "the result lands on the grid");
    }

    #[test]
    fn keyboard_move_is_inert_without_a_selection() {
        // Move is the resting target; the cluster needs a selected instance to step. With none it emits
        // nothing (and never panics reaching for a placement).
        let model = Model::default();
        let mut cam = Camera::over(Vec3::ZERO);
        assert!(keyboard(&input_with_chars(&['w']), false, &model, None, &mut cam).is_empty(), "no selection, no move");
    }

    #[test]
    fn gesture_without_a_scene_only_deselects_on_a_click() {
        // With no render residency there is nothing to pick or rest against: a click clears the
        // selection, a drag grabs nothing, and the grab stays empty.
        let mut interaction = Interaction::new();
        let cam = Camera::over(Vec3::ZERO);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        assert_eq!(interaction.gesture(Gesture::Click(Vec2::ZERO), None, &cam, rect, None), vec![Action::Deselect]);
        assert!(interaction.gesture(Gesture::GrabStart(Vec2::ZERO), None, &cam, rect, None).is_empty(), "no scene, no grab");
        assert!(interaction.grabbed.is_none(), "and nothing is grabbed");
    }
}
