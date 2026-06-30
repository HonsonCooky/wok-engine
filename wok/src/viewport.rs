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
//! Moving a selected instance is a later bite that plugs its own input in here, beside these.
//!
//! Where it runs: the frame loop's viewport interaction seam (`crate::main`), between the chrome's action
//! drain and the draw. [`camera_input`] reads the egui pointer state (a cloned [`egui::Context`]) and the
//! wok-platform [`InputState`], runs the right-drag cursor lock, and flies the [`Camera`] in place - the
//! camera is frame-loop residency, not the model, so the drive routes through neither the single writer
//! nor an action. [`pick_input`] is the other entry: it casts the cursor ray on a left press and returns
//! the [`Select`](crate::action::Action::Select) / [`Deselect`](crate::action::Action::Deselect) action
//! the seam routes through `action::handle`, since the selection IS model state.
//!
//! The right-drag look cursor lock (the proven pattern, designs/sharp-edges.md section 2): while a
//! right-drag begun over the well is held, the cursor is hidden and pinned so the mouse never leaves the
//! window or distracts mid-look, restored where the drag began on release. The look reads raw
//! `DeviceEvent::MouseMotion`, so a synthetic warp never feeds it and the lock changes the feel of
//! nothing. The lock decision ([`grab_transition`]) is pure and unit tested; the window side effects are
//! in [`update_cursor_grab`].

use glam::Vec2;
use wok_platform::FrameCtx;
use wok_platform::input::InputState;
use wok_platform::winit::dpi::PhysicalPosition;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::NamedKey;
use wok_platform::winit::window::{CursorGrabMode, Window};
use wok_scene::InstanceId;

use crate::action::Action;
use crate::camera::Camera;
use crate::render_scene::RenderScene;

/// The boost key (Shift): a held boost multiplies the fly speed (`Camera::fly`). The cluster, raise/lower,
/// and boost are sane placeholders, not the final binding - the rebindable layout is a later bite.
const BOOST: NamedKey = NamedKey::Shift;

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
    Some(pick_action(render_scene.pick(origin, dir)))
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
}
