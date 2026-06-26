//! Viewport input: the frame's raw input snapshot mapped to the mouse-only editor camera, plus the
//! during-drag cursor lock.
//!
//! egui sees every raw window event first (via `App::on_window_event`); [`camera_input`] then
//! consults `pointer_free` - the cursor is over the editor viewport and egui is not using the pointer
//! for its own UI (computed in `crate::main`, the 911a258 gate) - so the same motion never drives a
//! panel and the camera at once. The camera is mouse-only and always live (designs/editor-design.md,
//! Input): hold the right button and move to look, scroll to dolly along the look, hold the middle
//! button and move to pan the view plane. There is no keyboard movement and no camera mode, so the
//! left hand is left wholly to operators and precision (which return with picking and place).
//!
//! [`update_cursor_grab`] hides and locks the cursor for the duration of a look/pan drag so the
//! mouse never leaves the window or distracts mid-drag, restoring it where the drag began on release.
//! The pure mapping ([`camera_input`], [`grab_transition`]) is unit testable with no window; only the
//! window side effects in [`update_cursor_grab`] are not.

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::dpi::PhysicalPosition;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::window::{CursorGrabMode, Window};

use crate::camera::CameraInput;

/// Mouse-look sensitivity, radians per pixel of raw motion. Unchanged from the prior fly camera.
const LOOK_SENSITIVITY: f32 = 0.0035;

/// Dolly distance per scroll notch, in metres. Generous on purpose: the starter scene is one ~128 m
/// chunk, so crossing it should be a few flicks of the wheel rather than a long grind. Fixed for now;
/// scaling it by the distance to what the camera looks at (so close work dollies finer) is the
/// parked refinement, not a second way to translate.
const DOLLY_PER_NOTCH: f32 = 6.0;

/// Pan distance per pixel of raw motion, in metres. Tuned for a near one-to-one grab-the-world feel
/// at a typical working distance (tens of metres); far out it reads slow, which the same
/// view-distance scale as the dolly would fix. Fixed for now.
const PAN_SENSITIVITY: f32 = 0.06;

/// Map the frame's raw input snapshot to the camera's input, all from the mouse: hold the right
/// button and move to look, scroll to dolly along the look, hold the middle button and move to pan
/// the view plane. When the cursor is not free for the viewport (`pointer_free` is false - over the
/// chrome, an open menu, or an in-progress egui drag) nothing drives the camera, so the UI keeps its
/// own pointer input. Raw motion (not the cursor delta) is used so a future cursor lock would not
/// change the feel.
pub fn camera_input(input: &InputState, pointer_free: bool) -> CameraInput {
    if !pointer_free {
        return CameraInput::default();
    }
    let motion = Vec2::new(input.mouse_motion.0 as f32, input.mouse_motion.1 as f32);
    let look_delta = if input.mouse_held(MouseButton::Right) {
        // Rightward motion turns right (+yaw); downward motion pitches down (the view follows the
        // mouse), so screen y is negated against the pitch-up convention.
        Vec2::new(motion.x, -motion.y) * LOOK_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    let pan = if input.mouse_held(MouseButton::Middle) {
        // Scene tracks the drag: dragging right moves the camera left (-right) and dragging down moves
        // it up (+up), so the grabbed point follows the cursor. crate::camera applies pan.x along
        // right and pan.y along up, so the signs live here.
        Vec2::new(-motion.x, motion.y) * PAN_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    CameraInput { look_delta, dolly: input.scroll_delta.1 * DOLLY_PER_NOTCH, pan }
}

/// The look/pan drag state this frame, distilled from the mouse buttons: a fresh press (the engage
/// edge), a button still held mid-drag, or no drag button down. A wheel-only scroll is `Idle` (no
/// button), which is why scroll never engages a lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragInput {
    /// A look or pan button went down this frame - the edge that engages a lock.
    Started,
    /// A look or pan button is held, but not freshly pressed - a drag in progress.
    Held,
    /// No look or pan button is down.
    Idle,
}

/// What the cursor lock should do this frame. A pure decision over the drag state, so the rule -
/// engage on a look/pan press over the viewport, release the moment no drag button is held - is
/// testable without a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrabAction {
    /// A camera drag began over the viewport: hide and lock the cursor, anchoring it here.
    Engage,
    /// The drag ended: ungrab, restore the cursor to the anchor, and show it.
    Release,
    /// A lock is held this frame: re-assert it (hide + pin) against egui's per-frame cursor handling
    /// and the confined cursor's drift. Also the do-nothing case when no drag and no lock.
    Hold,
}

/// Decide the lock transition. Engage when a look/pan button goes down this frame over the viewport
/// (`over_well`: the pointer is in the well rect and under no foreground layer) and no lock is held.
///
/// `over_well` deliberately omits the camera gate's `is_using_pointer` term: on the very press frame
/// egui marks its background click-sense (the well's deselect interaction) as the potential click, so
/// `is_using_pointer` - and thus the full `pointer_free` - is true exactly when the drag begins, and
/// gating engage on it would miss the lock until the press cleared a few pixels later (the bug behind
/// "the cursor still moves"). Keying on a press *over the well* instead engages on frame one and still
/// excludes a panel-resize drag straying onto the viewport: that drag's press landed on the panel
/// edge, not the well, so it never produces a press edge here. A wheel-only scroll is `Idle`. Release
/// as soon as no look/pan button is held (even if the captured cursor has wandered off the well - the
/// button being up ends the lock, not the rect).
fn grab_transition(locked: bool, drag: DragInput, over_well: bool) -> GrabAction {
    match drag {
        DragInput::Started if !locked && over_well => GrabAction::Engage,
        DragInput::Idle if locked => GrabAction::Release,
        _ => GrabAction::Hold,
    }
}

/// Hide and lock the cursor for the duration of a camera drag (right-button look or middle-button
/// pan) that began over the viewport, restoring it on release where the drag started so it never
/// jumps. `over_well` is whether the pointer is in the well rect under no foreground layer (the engage
/// gate; see [`grab_transition`]). `grab` carries the press-position anchor while a lock is active
/// (the caller keeps it across frames). Returns whether a lock is active, so the caller keeps driving
/// the camera from frame one even while the captured cursor would nominally fall outside the well (a
/// confined cursor can drift over a panel; once a drag is held the lock, not the rect, gates the
/// camera).
///
/// The look and pan read raw `DeviceEvent::MouseMotion` (`InputState::mouse_motion`), which survives
/// cursor capture, so the lock changes nothing about the *input*: it is purely cosmetic, keeping the
/// cursor out of the way. [`Locked`] freezes the cursor where supported; where it is not (Windows
/// commonly gives only [`Confined`], which still lets the cursor move inside the window) the lock
/// re-warps the cursor to the anchor every frame to pin it in place. Warping is safe precisely because
/// the look reads raw motion: a synthetic warp produces a `CursorMoved`, never a `MouseMotion`, so it
/// never feeds the look and there is nothing to discard - the warp pins the cursor without touching
/// the camera. The hide is re-asserted each frame too, so egui's own per-frame cursor handling cannot
/// quietly show it again mid-drag.
///
/// [`Locked`]: CursorGrabMode::Locked
/// [`Confined`]: CursorGrabMode::Confined
pub fn update_cursor_grab(
    grab: &mut Option<PhysicalPosition<f64>>,
    window: &Window,
    input: &InputState,
    over_well: bool,
) -> bool {
    let drag = if input.mouse_pressed(MouseButton::Right) || input.mouse_pressed(MouseButton::Middle) {
        DragInput::Started
    } else if input.mouse_held(MouseButton::Right) || input.mouse_held(MouseButton::Middle) {
        DragInput::Held
    } else {
        DragInput::Idle
    };
    match grab_transition(grab.is_some(), drag, over_well) {
        GrabAction::Engage => {
            // Anchor at the press position, hide, and grab. Locked freezes the cursor where supported;
            // Confined is the Windows fallback. Best-effort - a refused grab leaves the cursor merely
            // hidden, and raw motion still drives the camera.
            let anchor = PhysicalPosition::new(input.mouse_pos.0, input.mouse_pos.1);
            window.set_cursor_visible(false);
            let _ = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            *grab = Some(anchor);
        }
        GrabAction::Hold => {
            // While a lock is held, re-assert the hide (egui re-applies the cursor icon on change) and
            // pin the cursor to the anchor: a no-op under a true Lock, but under Confined it undoes the
            // frame's drift so the cursor stays put. Inert when no lock is held.
            if let Some(anchor) = *grab {
                window.set_cursor_visible(false);
                let _ = window.set_cursor_position(anchor);
            }
        }
        GrabAction::Release => {
            if let Some(anchor) = grab.take() {
                // Ungrab first so the warp is not blocked by an active lock, then restore the cursor to
                // the anchor and show it.
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                let _ = window.set_cursor_position(anchor);
                window.set_cursor_visible(true);
            }
        }
    }
    grab.is_some()
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// An input snapshot with the given buttons held, raw motion, and vertical scroll - all the camera
    /// reads. Everything else is empty (the camera is mouse-only).
    fn mouse(buttons: &[MouseButton], motion: (f64, f64), scroll: f32) -> InputState {
        InputState {
            keys_held: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_repeating: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: motion,
            mouse_buttons_held: buttons.iter().copied().collect(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, scroll),
            gamepads: vec![],
        }
    }

    #[test]
    fn right_drag_looks_and_leaves_dolly_and_pan_idle() {
        // Hold the right button and move: rightward motion turns right (+yaw), downward motion pitches
        // down (-pitch). Nothing else fires.
        let look = camera_input(&mouse(&[MouseButton::Right], (10.0, 4.0), 0.0), true);
        assert!(look.look_delta.x > 0.0, "rightward motion turns right: {:?}", look.look_delta);
        assert!(look.look_delta.y < 0.0, "downward motion pitches down: {:?}", look.look_delta);
        assert_eq!(look.dolly, 0.0);
        assert_eq!(look.pan, Vec2::ZERO);
    }

    #[test]
    fn scroll_dollies_along_the_look_and_scales_with_the_notches() {
        // Scroll dollies with no button held; the amount scales linearly with the notch count.
        let one = camera_input(&mouse(&[], (0.0, 0.0), 1.0), true);
        assert_eq!(one.dolly, DOLLY_PER_NOTCH, "one notch dollies one step");
        assert_eq!(one.look_delta, Vec2::ZERO);
        assert_eq!(one.pan, Vec2::ZERO);
        let three = camera_input(&mouse(&[], (0.0, 0.0), 3.0), true);
        assert_eq!(three.dolly, 3.0 * DOLLY_PER_NOTCH, "dolly scales with the notch count");
    }

    #[test]
    fn middle_drag_pans_so_the_scene_tracks_the_drag() {
        // Dragging right (+x motion) moves the camera left (pan.x < 0, applied along +right) and
        // dragging down (+y motion) moves it up (pan.y > 0, applied along +up), so the grabbed point
        // follows the cursor.
        let input = camera_input(&mouse(&[MouseButton::Middle], (10.0, 4.0), 0.0), true);
        assert!(input.pan.x < 0.0, "drag right moves the camera left: {:?}", input.pan);
        assert!(input.pan.y > 0.0, "drag down moves the camera up: {:?}", input.pan);
        assert_eq!(input.look_delta, Vec2::ZERO, "a middle-drag does not look");
        assert_eq!(input.dolly, 0.0);
    }

    #[test]
    fn the_chrome_and_menus_take_all_camera_input_when_the_pointer_is_not_free() {
        // pointer_free is false over the chrome, an open menu, or an in-progress egui drag (the
        // 911a258 gate): no look, no dolly, no pan, even with both buttons held and the wheel turning.
        let busy = mouse(&[MouseButton::Right, MouseButton::Middle], (10.0, 4.0), 2.0);
        assert_eq!(camera_input(&busy, false), CameraInput::default());
    }

    // ---- cursor-lock transitions ----

    #[test]
    fn a_drag_press_over_the_viewport_engages_the_lock() {
        // Not yet locked, a button went down this frame, over the well -> grab. `over_well` omits
        // is_using_pointer, so this fires on the press frame even though egui marks its own background
        // click-sense as the potential click there.
        assert_eq!(grab_transition(false, DragInput::Started, true), GrabAction::Engage);
    }

    #[test]
    fn a_drag_press_off_the_viewport_does_not_engage() {
        // The press did not land over the well (on a panel, a menu, or under a foreground layer): no
        // grab. A panel-resize drag that later strays onto the viewport pressed on the panel edge, so
        // it never produces a press edge over the well and never captures the cursor.
        assert_eq!(grab_transition(false, DragInput::Started, false), GrabAction::Hold);
    }

    #[test]
    fn releasing_the_drag_releases_the_lock() {
        // A lock is held and no drag button remains down -> restore the cursor.
        assert_eq!(grab_transition(true, DragInput::Idle, true), GrabAction::Release);
        // Still released even if the pointer has wandered off the viewport (a confined cursor over a
        // panel): the button being up is what ends the lock, not the rect.
        assert_eq!(grab_transition(true, DragInput::Idle, false), GrabAction::Release);
    }

    #[test]
    fn a_held_drag_holds_the_lock_without_re_engaging() {
        // Mid-drag: still locked, a button held but not freshly pressed -> Hold (re-assert, never a
        // second Engage), whether or not the captured cursor still reads as over the viewport.
        assert_eq!(grab_transition(true, DragInput::Held, true), GrabAction::Hold);
        assert_eq!(grab_transition(true, DragInput::Held, false), GrabAction::Hold);
        // A second button pressed mid-drag must not re-engage over the existing lock.
        assert_eq!(grab_transition(true, DragInput::Started, true), GrabAction::Hold);
    }

    #[test]
    fn a_wheel_only_scroll_never_engages_a_lock() {
        // Scroll-dolly is a wheel event with no button down, so the drag input is Idle even over the
        // viewport: no lock engages for a scroll, and with no lock there is nothing to release.
        assert_eq!(grab_transition(false, DragInput::Idle, true), GrabAction::Hold);
    }
}
