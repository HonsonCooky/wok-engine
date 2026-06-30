//! The editor's viewport input: the get-around camera drive and click-to-select, read from the egui
//! pointer state and the wok-platform [`InputState`] and turned into camera look / fly / dolly and a
//! selection action.
//!
//! Rebuild bites of the editor interaction (designs/orchestrator-state.md; the direction is in
//! designs/editor-design.md's Input section). The get-around camera: hold the right mouse button over the
//! viewport to fly - mouse-look turns, WASD moves along the look and strafes level, E/Q raise and lower,
//! Shift boosts - and scroll to dolly along the look. Click-to-select: a left press over the well picks
//! the placement under the cursor, and a miss on terrain or sky deselects - routed through the single
//! writer, so it lights the same Instances-tree highlight and floating inspector a tree-select does.
//! Drag-to-move: a left press also arms the picked instance; dragging it past a small threshold rests it
//! on the surface under the cursor (base grounded, snapped to the 1m grid), routed as a transform edit.
//! Rotate / scale: with an instance selected and the camera not flying, A/D yaw it and W/S pitch it in
//! 5-degree steps (Shift+A/D roll it) and Q/E scale it uniformly - the fly cluster reused for a second
//! context, focus-gated so a name being typed never rotates - each routed as the same transform edit.
//!
//! Where it runs: the frame loop's viewport interaction seam (`crate::main`), between the chrome's action
//! drain and the draw. [`camera_input`] reads the egui pointer state (a cloned [`egui::Context`]) and the
//! wok-platform [`InputState`], runs the right-drag cursor lock, and flies the [`Camera`] in place - the
//! camera is frame-loop residency, not the model, so the drive routes through neither the single writer
//! nor an action. [`pick_input`] is the other entry: it casts the cursor ray on a left press and returns
//! the [`Select`](crate::action::Action::Select) / [`Deselect`](crate::action::Action::Deselect) action
//! the seam routes through `action::handle`, since the selection IS model state - and on a hit it arms
//! the drag. [`drag_input`] is the third: while the left button stays down it moves the armed instance
//! along the surface, returning a [`SetInstanceTransform`](crate::action::Action::SetInstanceTransform)
//! the seam routes the same way, and the button lifting clears the arm. [`transform_input`] is the
//! fourth: with a selection and the camera idle it folds the A/D, W/S rotate and Q/E scale keys into one
//! [`SetInstanceTransform`](crate::action::Action::SetInstanceTransform) the seam routes the same way.
//!
//! The right-drag look cursor lock (the proven pattern, designs/sharp-edges.md section 2): while a
//! right-drag begun over the well is held, the cursor is hidden and pinned so the mouse never leaves the
//! window or distracts mid-look, restored where the drag began on release. The look reads raw
//! `DeviceEvent::MouseMotion`, so a synthetic warp never feeds it and the lock changes the feel of
//! nothing. The lock decision ([`grab_transition`]) is pure and unit tested; the window side effects are
//! in [`update_cursor_grab`].

use glam::{Vec2, Vec3};
use wok_platform::FrameCtx;
use wok_platform::input::InputState;
use wok_platform::winit::dpi::PhysicalPosition;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::NamedKey;
use wok_platform::winit::window::{CursorGrabMode, Window};
use wok_scene::{Aabb, InstanceId, Transform};

use crate::action::Action;
use crate::camera::Camera;
use crate::geom;
use crate::loaded::LoadedScene;
use crate::render_scene::RenderScene;

/// The boost key (Shift): a held boost multiplies the fly speed (`Camera::fly`). The cluster, raise/lower,
/// and boost are sane placeholders, not the final binding - the rebindable layout is a later bite.
const BOOST: NamedKey = NamedKey::Shift;

/// The world grid the drag-to-move snaps to, in metres - the snap-assisted-placement default
/// (editor-design Input: grid snap 1m, on by default). The escape toggle is a later bite (W5).
const GRID_STEP: f32 = 1.0;

/// How far (in egui points) the pointer must travel from the left press before the drag-to-move starts,
/// so a plain click stays select-only. Critical: [`RenderScene::surface_ray`](crate::render_scene::RenderScene::surface_ray)
/// excludes the dragged instance, so without this gate a click would drop a floating instance to the
/// ground under it. A few points is enough to tell a click from a drag without feeling sticky.
const DRAG_THRESHOLD_PX: f32 = 4.0;

/// The keyboard rotation step in degrees - the rotation-snap default (editor-design Input: rotation snap
/// 5 degrees, on by default), applied per rotate-key (A/D yaw, W/S pitch, Shift+A/D roll) tap or repeat
/// about a world axis. The toggle to free it (unsnapped rotation) is a later bite (W5). Transform feel is tunable.
const ROTATE_STEP_DEG: f32 = 5.0;

/// The uniform scale factor per D (up) / S (down) tap or repeat: D multiplies the scale by this, S by its
/// inverse, so a tap each way returns near the original. Transform feel is tunable.
const SCALE_STEP: f32 = 1.1;

/// The per-component floor a scale-down clamps to, so a shrink never reaches zero or flips negative (a
/// degenerate, un-pickable instance). Transform feel is tunable.
const SCALE_MIN: f32 = 0.05;

/// An armed viewport drag-to-move: the instance a left press selected and the press position (egui
/// points). Threaded across frames by the seam (`crate::main`) beside the camera grab anchor: the press
/// arms it ([`pick_input`]), a later held frame past [`DRAG_THRESHOLD_PX`] moves it ([`drag_input`]),
/// and the button lifting clears it. The press anchor is what tells a genuine drag from a plain click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportDrag {
    /// The picked instance the drag moves.
    pub id: InstanceId,
    /// The left-press position (egui points), the threshold anchor.
    pub press: Vec2,
}

/// Drive the get-around camera from one frame of input, the single entry the viewport seam calls
/// (`crate::main`). Reads the egui pointer state (`egui_ctx`, a cloned [`egui::Context`] handle so the
/// seam can mutate the camera and the grab in the same statement) to tell whether the pointer is over the
/// well, runs the right-drag cursor lock, and - while the look is locked - turns and flies the camera;
/// a scroll over the well dollies it. The camera is frame-loop residency, so nothing here routes through
/// the single writer.
///
/// The over-the-well gate ([`pointer_over_well`]) is shared with the click-select seam ([`pick_input`]).
/// The lock engage reads it bare, not `is_using_pointer`: on a right press the well's own background
/// click-sense marks `is_using_pointer`, so gating the engage on it would miss the press (the cursor
/// would not hide until the click cleared a few pixels in). The dolly does add `is_using_pointer`, so a
/// panel-resize drag or a widget interaction straying onto the well with the wheel turning does not also
/// dolly (sharp-edges 2, the viewport-input gate).
pub fn camera_input(
    egui_ctx: &egui::Context,
    editor_rect: egui::Rect,
    ctx: &FrameCtx,
    camera: &mut Camera,
    grab: &mut Option<PhysicalPosition<f64>>,
) {
    let input = &ctx.input;
    let over_well = pointer_over_well(egui_ctx, editor_rect);

    // Hide and pin the cursor while a right-drag look pressed over the well is held, restoring it on
    // release. The look fires exactly while the cursor is locked, reading raw motion (which a synthetic
    // warp never produces, so the per-frame re-warp pins the cursor without feeding the look). lock_active
    // drives from the press frame and keeps driving even as a confined cursor drifts off the well.
    let lock_active = update_cursor_grab(grab, &ctx.platform.window, input, over_well);
    if lock_active {
        camera.look(Vec2::new(input.mouse_motion.0 as f32, input.mouse_motion.1 as f32));
        camera.fly(
            axis(input, 'w', 's'),
            axis(input, 'd', 'a'),
            axis(input, 'e', 'q'),
            input.key_held(BOOST),
            ctx.dt,
        );
    }

    // Scroll dollies along the look whenever the pointer is over the well (no right button needed). Gated
    // off egui's pointer use so a panel-resize drag or a widget interaction straying over the well does
    // not also dolly.
    if over_well && !egui_ctx.is_using_pointer() {
        camera.dolly(input.scroll_delta.1);
    }
}

/// The viewport's click-to-select, the seam's second entry (`crate::main`): on a left-button press over
/// the well - and not while a right-drag look holds the camera - cast the cursor ray at the open scene's
/// placements and return the selection action it implies. [`Select`](Action::Select) the nearest
/// placement hit, or [`Deselect`](Action::Deselect) on empty ground or sky. `None` means the click is not
/// a select this frame (no scene open, mid-look, no press, or off the well), so the seam routes nothing.
/// A hit also ARMS the drag-to-move (`drag`), anchored at this press, so a later held frame past the
/// threshold moves the instance ([`drag_input`]); a miss clears any armed drag.
///
/// Shares the camera drive's over-the-well gate ([`pointer_over_well`]) and is likewise blind to
/// `is_using_pointer`: on the press frame the well's own background click-sense marks the pointer as
/// used, so gating on it would drop the very press that selects (sharp-edges 2, the same press-frame
/// reason the lock engage omits it). The press edge and the click position come from egui's pointer
/// (`primary_pressed` + `interact_pos`), in points consistent with `editor_rect`; the ray uses the
/// scene's own `far_plane`, so the unprojection inverts the exact matrix the renderer drew with.
pub fn pick_input(
    egui_ctx: &egui::Context,
    editor_rect: egui::Rect,
    camera: &Camera,
    render_scene: Option<&RenderScene>,
    lock_active: bool,
    drag: &mut Option<ViewportDrag>,
) -> Option<Action> {
    // With no scene open there is nothing to pick; a held right-drag look is a fly, not a select, so a
    // left-click mid-look does not select either (the lock state the camera drive set this frame).
    let render_scene = render_scene?;
    if lock_active {
        return None;
    }
    // The left-press edge and the click position, both from egui's pointer in the same points editor_rect
    // is measured in. Fire only on the press, over the well; with no interact position there is nothing
    // to cast.
    let (pressed, pos) = egui_ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos()));
    if !pressed || !pointer_over_well(egui_ctx, editor_rect) {
        return None;
    }
    let pos = pos?;
    // Build the cursor ray against the same well rect the 3D drew into, and pick the nearest placement
    // under it. The decision (hit -> select, miss -> deselect) is the pure `pick_action`.
    let pos_in_rect = Vec2::new(pos.x - editor_rect.min.x, pos.y - editor_rect.min.y);
    let rect_size = Vec2::new(editor_rect.width(), editor_rect.height());
    let (origin, dir) = camera.cursor_ray(pos_in_rect, rect_size, render_scene.far_plane());
    let action = pick_action(render_scene.pick(origin, dir));
    // Arm the drag-to-move from this same press: a hit becomes the drag target anchored here, so a held
    // frame past the threshold moves it ([`drag_input`]); a miss disarms. The move itself never runs on
    // the press frame - the threshold has not been crossed yet.
    *drag = match &action {
        Action::Select(id) => Some(ViewportDrag { id: *id, press: Vec2::new(pos.x, pos.y) }),
        _ => None,
    };
    Some(action)
}

/// The drag-to-move, the seam's third entry (`crate::main`): while the left button stays down and an
/// armed instance ([`pick_input`] set `drag` on the press) has been dragged past [`DRAG_THRESHOLD_PX`]
/// over the well, rest that instance on the surface under the cursor and return the
/// [`SetInstanceTransform`](Action::SetInstanceTransform) the seam routes through `action::handle`. The
/// button lifting (or never being down) clears the arm and moves nothing - the release-clear, so the
/// next press re-arms cleanly. `None` means no move this frame: no scene, mid-look (flying), no armed
/// drag, a click that has not crossed the threshold, the pointer off the well, a stale id, or the cursor
/// over empty sky (off any surface) - all hold the instance where it is rather than fling it.
///
/// The threshold is what keeps a plain click select-only: [`surface_ray`](RenderScene::surface_ray)
/// excludes the dragged instance, so a press that does not move would otherwise drop a floating instance
/// straight to the ground beneath it. The lifted math (1677821) composes
/// [`surface_ray`](RenderScene::surface_ray) -> [`instance_aabb`](RenderScene::instance_aabb) ->
/// [`grounded_drop`] with the snap-assisted-placement defaults baked on (grid + grounded); the escape
/// toggles are a later bite (W5). The world hit is written into the chunk-local translation - exact for
/// the single-chunk scenes the editor authors today; the world-to-local re-home is a deferred bite.
pub fn drag_input(
    egui_ctx: &egui::Context,
    editor_rect: egui::Rect,
    camera: &Camera,
    render_scene: Option<&RenderScene>,
    loaded: Option<&LoadedScene>,
    drag: &mut Option<ViewportDrag>,
    lock_active: bool,
) -> Option<Action> {
    // The button lifting (or never down) ends the drag: clear the arm and move nothing, so the next
    // press re-arms cleanly. This runs before every other gate, so a release always disarms.
    if !egui_ctx.input(|i| i.pointer.primary_down()) {
        *drag = None;
        return None;
    }
    // A held right-drag look is flying, not moving (can't move while flying). The button is still down,
    // so the arm holds - only the release above clears it.
    if lock_active {
        return None;
    }
    let render_scene = render_scene?;
    let armed = (*drag)?;
    // Only a genuine drag moves: the pointer must be over the well and have travelled past the threshold
    // from the press anchor, so a plain click (press + release in place) stays select-only.
    let pos = egui_ctx.pointer_latest_pos()?;
    let pos = Vec2::new(pos.x, pos.y);
    if !pointer_over_well(egui_ctx, editor_rect) || (pos - armed.press).length() < DRAG_THRESHOLD_PX {
        return None;
    }
    // The dragged instance's current transform (the move keeps its rotation and scale); a stale id or no
    // loaded scene means nothing to move.
    let current = loaded?.placement(armed.id)?.transform;
    // Cast the cursor ray at the surface under it, excluding the dragged instance so it never rests on
    // itself, and rest its base there snapped to the grid. Off any surface (empty sky) holds - no move.
    let pos_in_rect = Vec2::new(pos.x - editor_rect.min.x, pos.y - editor_rect.min.y);
    let rect_size = Vec2::new(editor_rect.width(), editor_rect.height());
    let (origin, dir) = camera.cursor_ray(pos_in_rect, rect_size, render_scene.far_plane());
    let hit = render_scene.surface_ray(origin, dir, armed.id)?;
    let bounds = render_scene.instance_aabb(armed.id)?;
    Some(Action::SetInstanceTransform(armed.id, grounded_drop(hit, current, bounds)))
}

/// The drag-to-move's grounded grid snap: rest the instance's base on the surface point `hit` under the
/// cursor, snapping X/Z and the resting pivot to the 1m grid, keeping `current`'s rotation and scale.
/// The lifted drag_to math (1677821) with the snap-assisted-placement defaults on (grid + grounded). Pure
/// - hit + current transform + world bounds in, snapped grounded transform out - so it is unit tested
/// without a scene; the grid snap and the rest-the-bottom pivot are `geom`'s own tested primitives.
fn grounded_drop(hit: Vec3, current: Transform, bounds: Aabb) -> Transform {
    let pivot_y = geom::snap(geom::rest_y(hit.y, current.translation.y, bounds.min.y), GRID_STEP);
    let translation = Vec3::new(geom::snap(hit.x, GRID_STEP), pivot_y, geom::snap(hit.z, GRID_STEP));
    Transform { translation, ..current }
}

/// The keyboard rotate / scale, the seam's fourth entry (`crate::main`): with an instance selected and
/// the fly cluster otherwise idle (no right-drag look holding the camera, no left-drag move), the A/D,
/// W/S, and Q/E keys transform the selection in place and return the
/// [`SetInstanceTransform`](Action::SetInstanceTransform) the seam routes through `action::handle`. A/D
/// yaw it about world Y (A left, D right), W/S pitch it about world X, and Shift+A / Shift+D roll it about
/// world Z (Shift is the roll modifier, re-routing A/D from yaw to roll); each spin is a [`ROTATE_STEP_DEG`]
/// step. Q/E scale it uniformly down/up by [`SCALE_STEP`] (Q/E = down/up, as in the fly cluster's
/// raise/lower), floored at [`SCALE_MIN`] so a shrink never reaches zero. A tap is one step; a hold repeats
/// at the OS rate (`char_pressed || char_repeating`). `None` means no transform this frame: no selection,
/// flying, mid-drag, a focused text field (a name being typed, not a rotate), a held Ctrl (so Ctrl+S stays
/// Save), a stale id, or simply no transform key down.
///
/// Context-gated, reusing the fly cluster (editor-design Input, "modes reuse one small key set across
/// contexts"): the same WASD + Q/E that fly the camera while the right button looks instead rotate / scale
/// when it does not. The scheme is directional and controller-shaped (canon: controller-mappable) - A/D and
/// W/S are bidirectional on their own, so Shift is freed to be the roll modifier rather than a reverse
/// sign, and the mnemonic G/R/T keys stay open for the W5 snap toggles. Focus-gated on egui's
/// `wants_keyboard_input`, so a focused inspector field types into the field rather than transforming;
/// Shift stays the camera boost while flying (a different context). The 5-degree step is the rotation-snap
/// default baked on (editor-design Input); the toggle to free it is a later bite (W5). Multiple keys in one
/// frame fold deterministically (rotations in a fixed key order, then scale) into a single transform, so
/// the edit is one action however many keys are down. The gimbal-free spin is [`geom::rotate_step`]; the
/// floored multiply is [`geom::scale_uniform`].
pub fn transform_input(
    egui_ctx: &egui::Context,
    input: &InputState,
    selection: Option<InstanceId>,
    loaded: Option<&LoadedScene>,
    lock_active: bool,
    drag: Option<ViewportDrag>,
) -> Option<Action> {
    // A held right-drag look is flying (the fly cluster drives the camera), and a held left-drag is
    // moving - neither is a rotate / scale. With nothing selected there is nothing to transform.
    if lock_active || drag.is_some() {
        return None;
    }
    let id = selection?;
    // Focus gate (editor-design Input): a focused text field (the inspector's Name) types, it does not
    // transform. Ctrl is reserved for the command combos (Ctrl+S Save), so a held Ctrl is never a
    // transform; Shift is allowed below - it is the roll modifier on A/D.
    if egui_ctx.wants_keyboard_input() || input.key_held(NamedKey::Control) {
        return None;
    }
    // The selected instance's current transform - the same source the drag-to-move reads it from (a
    // stale id, or no loaded scene, means nothing to transform).
    let current = loaded?.placement(id)?.transform;

    // Fold every transform key down this frame into one new transform, starting from the current: a tap
    // fires the press edge, a hold the OS repeat. Rotations apply in a fixed key order, then scale, so a
    // frame with several keys down is one deterministic edit.
    let pressed = |key: char| input.char_pressed(key) || input.char_repeating(key);
    // Shift is the roll modifier: held, it re-routes A/D from yaw (world Y) to roll (world Z). W/S pitch
    // (world X) and Q/E scale regardless of Shift. A is the left / counter-clockwise sense, D the right.
    let ad_axis = if input.key_held(NamedKey::Shift) { Vec3::Z } else { Vec3::Y };
    let mut t = current;
    let mut changed = false;
    for (down, axis, degrees) in [
        (pressed('a'), ad_axis, ROTATE_STEP_DEG),
        (pressed('d'), ad_axis, -ROTATE_STEP_DEG),
        (pressed('w'), Vec3::X, -ROTATE_STEP_DEG),
        (pressed('s'), Vec3::X, ROTATE_STEP_DEG),
    ] {
        if down {
            t = geom::rotate_step(t, axis, degrees);
            changed = true;
        }
    }
    // Scale uniform: Q down (x 1/SCALE_STEP), E up (x SCALE_STEP) - Q/E = down/up, the same sense as the
    // fly cluster's raise/lower, so the keys read alike in both contexts.
    for (down, factor) in [(pressed('q'), 1.0 / SCALE_STEP), (pressed('e'), SCALE_STEP)] {
        if down {
            t = geom::scale_uniform(t, factor, SCALE_MIN);
            changed = true;
        }
    }
    // No transform key down changes nothing, so route nothing (an equal transform would no-op in the
    // loaded scene anyway). One key or several, the fold is a single transform edit.
    changed.then_some(Action::SetInstanceTransform(id, t))
}

/// The pointer is over the viewport well: inside the editor rect and under no foreground egui layer (a
/// menu, the floating inspector). The well is egui's background layer under a `CentralPanel`, so the rect
/// plus the layer order is what tells the viewport from a panel (designs/sharp-edges.md section 2). Reads
/// the latest pointer position, not the hover position: on a press frame egui treats the pointer as down,
/// so hover_pos can drop to None, but the latest pos still lands the gate. Shared bare (no
/// `is_using_pointer`) by both press-frame seams - the camera lock engage and the click-select - which is
/// what keeps a press from being dropped by the well's own click-sense; the dolly adds the term itself.
fn pointer_over_well(egui_ctx: &egui::Context, editor_rect: egui::Rect) -> bool {
    let pointer = egui_ctx.pointer_latest_pos();
    pointer.is_some_and(|p| editor_rect.contains(p))
        && pointer.and_then(|p| egui_ctx.layer_id_at(p)).is_none_or(|layer| layer.order == egui::Order::Background)
}

/// Map a cursor-ray pick to the selection action it implies: a hit selects that instance, a miss (terrain
/// or empty space) deselects. The whole click-to-select decision as a pure value, so it is unit testable
/// without a window, a camera, or a scene.
fn pick_action(pick: Option<InstanceId>) -> Action {
    match pick {
        Some(id) => Action::Select(id),
        None => Action::Deselect,
    }
}

/// One directional axis from a key pair: `+1.0` while `positive` is held, `-1.0` while `negative` is,
/// `0.0` with neither or both (they cancel). The held state, not the press edge, so a held key flies
/// continuously while down.
fn axis(input: &InputState, positive: char, negative: char) -> f32 {
    (input.char_held(positive) as i32 - input.char_held(negative) as i32) as f32
}

// ---- the right-drag look cursor lock ----
// A held right-drag looks the camera; while it is held the cursor is hidden and pinned so the mouse never
// leaves the window or distracts mid-look, restored where the drag began on release. The proven pattern
// (designs/sharp-edges.md section 2, the camera-drag cursor-lock trap): engage on a right press over the
// well rect (NOT is_using_pointer - egui's well click-sense sets it on the press frame, which would miss
// the engage), and under Windows' Confined fallback re-warp to the anchor and re-hide each frame. The
// look reads raw DeviceEvent::MouseMotion, which a synthetic warp never produces, so the warp pins the
// cursor without feeding the look. The window side effects are in `update_cursor_grab`; the decision
// (`grab_transition`) is pure and unit tested.

/// The look-drag state this frame: a fresh right press (the engage edge), the right button still held
/// mid-look, or no look button down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragInput {
    /// The right button went down this frame - the edge that engages the lock.
    Started,
    /// The right button is held, but not freshly pressed - a look in progress.
    Held,
    /// The right button is up.
    Idle,
}

/// What the cursor lock should do this frame - a pure decision over the drag state, so the rule (engage
/// on a right press over the well, release the moment the button is up) is testable without a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrabAction {
    /// A right-drag began over the viewport: hide and lock the cursor, anchoring it here.
    Engage,
    /// The drag ended: ungrab, restore the cursor to the anchor, and show it.
    Release,
    /// A lock is held: re-assert it (hide + pin) against egui's per-frame cursor handling and the
    /// confined cursor's drift. Also the do-nothing case when no drag and no lock.
    Hold,
}

/// Decide the lock transition: engage when the right button goes down this frame over the well
/// (`over_well`: the pointer is in the well rect under no foreground layer) and no lock is held; release
/// the moment the right button is up; otherwise hold. `over_well` deliberately omits the camera gate's
/// `is_using_pointer` term: on the press frame egui marks the well's own deselect click-sense as the
/// potential click, so gating engage on it would miss the press (the cursor would not hide until the
/// click cleared a few pixels in). A press lands over the well only for a genuine viewport drag, never a
/// panel-resize drag (which presses the panel edge), so keying on a press over the well is the right
/// engage signal.
fn grab_transition(locked: bool, drag: DragInput, over_well: bool) -> GrabAction {
    match drag {
        DragInput::Started if !locked && over_well => GrabAction::Engage,
        DragInput::Idle if locked => GrabAction::Release,
        _ => GrabAction::Hold,
    }
}

/// Hide and pin the cursor for the duration of a right-drag look that began over the viewport, restoring
/// it on release where the drag started so it never jumps. `grab` carries the press-position anchor while
/// a lock is active (the caller keeps it across frames). Returns whether a lock is active, which the
/// caller uses to gate the look (the look fires exactly while the cursor is locked) and to keep driving
/// from frame one even as a confined cursor drifts off the well.
///
/// Best-effort: a refused grab leaves the cursor merely hidden, and the raw-motion look still works.
/// Windows commonly grants only [`Confined`](CursorGrabMode::Confined) (not
/// [`Locked`](CursorGrabMode::Locked)), which still lets the cursor move inside the window, so a held lock
/// re-warps to the anchor and re-asserts the hide each frame (egui re-applies the cursor icon on change).
fn update_cursor_grab(
    grab: &mut Option<PhysicalPosition<f64>>,
    window: &Window,
    input: &InputState,
    over_well: bool,
) -> bool {
    let drag = if input.mouse_pressed(MouseButton::Right) {
        DragInput::Started
    } else if input.mouse_held(MouseButton::Right) {
        DragInput::Held
    } else {
        DragInput::Idle
    };
    match grab_transition(grab.is_some(), drag, over_well) {
        GrabAction::Engage => {
            // Anchor at the press position, hide, and grab. Locked freezes the cursor where supported;
            // Confined is the Windows fallback. Best-effort - a refused grab leaves it merely hidden.
            let anchor = PhysicalPosition::new(input.mouse_pos.0, input.mouse_pos.1);
            window.set_cursor_visible(false);
            let _ = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            *grab = Some(anchor);
        }
        GrabAction::Hold => {
            // While a lock is held, re-assert the hide and pin the cursor to the anchor: a no-op under a
            // true Lock, but under Confined it undoes the frame's drift. Inert when no lock is held.
            if let Some(anchor) = *grab {
                window.set_cursor_visible(false);
                let _ = window.set_cursor_position(anchor);
            }
        }
        GrabAction::Release => {
            if let Some(anchor) = grab.take() {
                // Ungrab first so the warp is not blocked by an active lock, then restore and show.
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                let _ = window.set_cursor_position(anchor);
                window.set_cursor_visible(true);
            }
        }
    }
    grab.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_transition_engages_on_a_press_over_the_well_and_releases_when_the_button_lifts() {
        // The pure lock decision. Engage on a fresh right press over the well with no lock held; a press
        // off the well (a panel-resize drag) does not. Once locked, a held button re-asserts (Hold), and
        // the button lifting releases. A wheel-only frame (Idle, no lock) does nothing.
        assert_eq!(grab_transition(false, DragInput::Started, true), GrabAction::Engage, "press over the well engages");
        assert_eq!(grab_transition(false, DragInput::Started, false), GrabAction::Hold, "press off the well does not");
        assert_eq!(grab_transition(true, DragInput::Started, true), GrabAction::Hold, "already locked - no re-engage");
        assert_eq!(grab_transition(true, DragInput::Held, true), GrabAction::Hold, "a held look re-asserts the lock");
        assert_eq!(grab_transition(true, DragInput::Idle, true), GrabAction::Release, "the button lifting releases");
        assert_eq!(grab_transition(false, DragInput::Idle, true), GrabAction::Hold, "no drag, no lock - inert");
    }

    #[test]
    fn pick_action_selects_a_hit_and_deselects_a_miss() {
        // The pure click-to-select decision: a placement under the cursor selects it, empty ground or sky
        // (a None pick) clears the selection. The ray math and the pick itself are tested in camera.rs and
        // render_scene.rs; this pins only the mapping the viewport seam routes through the single writer.
        assert_eq!(pick_action(Some(InstanceId(7))), Action::Select(InstanceId(7)), "a hit selects that instance");
        assert_eq!(pick_action(None), Action::Deselect, "a miss on terrain or sky deselects");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn grounded_drop_rests_the_base_on_the_grid_and_keeps_rotation_and_scale() {
        // The lifted drag-to-move compose: rest a unit cube's base (world AABB min.y -0.5 at its current
        // height 0) on a surface hit at (2.4, 3.4, -1.6) and snap to the 1m grid. X 2.4 -> 2, Z -1.6 ->
        // -2, and the resting pivot (flush 3.4 + 0.5 = 3.9) snaps to a grid-whole 4; rotation and scale
        // pass through untouched. The grid snap and the rest-the-bottom pivot are geom's tested primitives;
        // this pins only that the move composes them as the inspector-equivalent edit the seam routes.
        let current = Transform {
            translation: Vec3::new(7.3, 0.0, -2.8),
            rotation: glam::Quat::from_rotation_y(1.0),
            scale: Vec3::splat(2.0),
        };
        let bounds = Aabb::new(Vec3::new(-0.5, -0.5, -0.5), Vec3::new(0.5, 0.5, 0.5));
        let dropped = grounded_drop(Vec3::new(2.4, 3.4, -1.6), current, bounds);
        assert_eq!(dropped.translation, Vec3::new(2.0, 4.0, -2.0), "base grounded, X/Z and pivot grid-snapped");
        assert_eq!(dropped.rotation, current.rotation, "rotation is kept");
        assert_eq!(dropped.scale, current.scale, "scale is kept");
    }
}
