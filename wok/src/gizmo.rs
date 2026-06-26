//! The transform manipulator: a small left-hand grammar over the selected placement. Per the device
//! split (designs/editor-design.md, Input: the mouse is motion, the left hand is the operators), move
//! and scale are an operator key held with the mouse (held-not-modal: the manipulation lives only while
//! the key is down, releasing commits, and the edit is persisted live throughout), while rotate is
//! discrete key taps. The grammar is being reworked for the ZSA Voyager left hand key by key: move is
//! two reachable keys (no axis chords, no tripod), rotate is W / E / R taps, and scale stays the
//! shipped held + mouse op (its rework is a later bite; the inspector is the precise path meanwhile).
//!
//! The grammar ([`update`], driven from the frame loop where the camera, the render residency, and the
//! raw input live):
//! - Move onto the surface (`g`): the placement follows the cursor onto whatever it aims at - the
//!   terrain or a prefab beneath. The cursor ray's surface hit
//!   ([`RenderScene::surface_ray`](crate::render_scene::RenderScene::surface_ray)) sets the pivot that
//!   rests the placement's bottom on that surface (lifted by its pivot-to-bottom offset from
//!   [`RenderScene::instance_aabb`](crate::render_scene::RenderScene::instance_aabb)), and that whole
//!   pivot - translation x, y, and z - snaps to the 1m grid. Sourcing the height from the actual hit
//!   means G follows the aim - terrain under or past a prefab rests on the ground, a prefab top stacks,
//!   with no teleport - and snapping the pivot leaves the base within +/-0.5m of the surface
//!   (penetration is allowed).
//! - Move free (`f`): the placement slides in the horizontal plane at its current height (the cursor
//!   ray meets `y = translation.y`), snapped to the 1m grid. The scroll wheel steps the height by 1m
//!   of world vertical per notch instead of dollying the camera (the frame loop gates the dolly off
//!   while `f` is held - see [`Outcome::consumed_scroll`]); a scroll with the cursor still holds the
//!   XZ, so the placement rises and lowers straight up in world space, not along the view (which a
//!   re-read of the cursor on the new height plane would pull toward the camera).
//! - Rotate (`w` / `e` / `r`): keyboard-only discrete taps, no mouse and no chords. Each press turns the
//!   selection 5deg about a world axis - W pitch (X), E yaw (Y), R roll (Z) - reversed with Shift. One
//!   tap is one committed step (not a hold, no Esc-cancel); the press edge swallows OS auto-repeat, so
//!   holding a key does not spin - large turns are repeated taps or the inspector's Rot fields.
//! - Scale (`s`): held + mouse, the remaining hold op - no axis scales uniformly, `x` / `y` / `z`
//!   scales that one component, from raw horizontal mouse motion.
//!
//! Move snaps to a 1m grid by default (sub-grid precision stays on the inspector); there are no axis
//! chords and no Alt on move (the Voyager cannot reach them; a reachable snap-off toggle is a later
//! bite). A hold ends on op-key-up (the edit is already committed) and Esc cancels it (restores the
//! captured transform, keeps the selection); a rotate tap is immediate, with nothing to cancel. Keys
//! read through `char_held`, the rotate taps through `char_pressed`, so they are rebindable later (table
//! parked). Every change routes through the edit seam: [`update`] returns an
//! [`Action::SetInstanceTransform`](crate::action::Action) the frame loop applies through
//! `crate::action::handle`, exactly as the inspector's fields do; [`LoadedScene::set_transform`] no-ops
//! an unchanged transform, so emitting the full transform every held frame is free.
//!
//! No viewport visual marks the selection this bite (the Instances tree and the inspector show it; a
//! subtle 3D highlight is a later bite). The move shares one cursor-to-ray source with the picking and
//! the render (sharp-edges 2): [`cursor_ray`] maps the cursor against the same well rect with the same
//! far plane, and the surface query runs the same `classify_collider` -> `ray_collider` reduction the
//! pick uses, so a snap lands on what a placement collides as. The pure geometry (ray vs the ground
//! plane, the height step, the rotate step) is unit tested below; the surface query and its terrain
//! march live on the render residency beside the pick.

use glam::{Quat, Vec2, Vec3};
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::NamedKey;
use wok_scene::{InstanceId, Transform};

use crate::action::Action;
use crate::camera::FlyCamera;
use crate::loaded::LoadedScene;
use crate::render_scene::RenderScene;

/// The held-op keys (read case-insensitively; rebindable later, the keybind table is parked). Move is
/// surface-snap (`g`, like Blender's grab) and free (`f`); scale (`s`) stays the held + mouse op.
const MOVE_SURFACE_KEY: char = 'g';
const MOVE_FREE_KEY: char = 'f';
const SCALE_KEY: char = 's';

/// Discrete rotate taps (keyboard-only, no mouse, no chords): each press turns the selection one step
/// about a world axis - W pitch (X), E yaw (Y), R roll (Z) - reversed with Shift. The step is the 5deg
/// canon; larger turns are repeated taps or the inspector's Rot fields.
const ROTATE_PITCH_KEY: char = 'w';
const ROTATE_YAW_KEY: char = 'e';
const ROTATE_ROLL_KEY: char = 'r';
const ROTATE_STEP_DEG: f32 = 5.0;

/// World-vertical height step per scroll notch for the free move (`f` + wheel), in metres; positive
/// scroll raises. A clean 1m, matching the translate grid; sub-grid height stays on the inspector.
const MOVE_Y_STEP_M: f32 = 1.0;

/// Translate grid (canon: 1m): the default snap for both moves' XZ; sub-grid precision is the
/// inspector's (no Alt fine-modifier - unreachable on the Voyager). Scale has no grid.
const TRANSLATE_SNAP_M: f32 = 1.0;
/// Scale exponent per point of raw horizontal mouse motion (the held + mouse scale op).
const SCALE_PER_PX: f32 = 0.005;

/// A world axis named by the X / Y / Z keys for rotate and scale (move has no axis chords this bite).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// The unit world direction of the axis (rotate's pivot).
    fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    /// The scale component index the axis drives (X=0, Y=1, Z=2).
    fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

/// The operator a hold applies, chosen by the key down. Each lives only while its key is held.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    /// Move onto the surface under the cursor (`g`).
    MoveSurface,
    /// Move free in the horizontal plane, the scroll wheel stepping height (`f`).
    MoveFree,
    /// Scale uniformly or by one axis (`s`). (Rotate is no longer a hold - it is discrete W / E / R
    /// taps, handled outside the hold machinery.)
    Scale,
}

impl Op {
    /// The key that drives this manipulation.
    fn key(self) -> char {
        match self {
            Op::MoveSurface => MOVE_SURFACE_KEY,
            Op::MoveFree => MOVE_FREE_KEY,
            Op::Scale => SCALE_KEY,
        }
    }

    /// Whether this op takes an X / Y / Z axis chord (scale does; the move ops do not).
    fn takes_axis(self) -> bool {
        matches!(self, Op::Scale)
    }
}

/// The current sub-drag's reference. The move ops are absolute - each frame reads the cursor fresh and
/// (for free) the live height - so they carry no reference; scale accumulates raw horizontal mouse
/// motion since the seed.
#[derive(Clone, Copy)]
enum Drag {
    /// A move op (surface or free): nothing to accumulate, the cursor is read fresh each frame.
    Cursor,
    /// Scale: raw horizontal mouse motion accumulated since the seed.
    Motion(f32),
}

/// An in-progress held transform, held across frames by the frame loop (`Option<Hold>` on the editor).
/// Lives only while its op key is down; releasing it (or moving the selection off it) commits, Esc
/// cancels. The idle state is the `None` outside.
pub struct Hold {
    id: InstanceId,
    op: Op,
    /// The transform when the op key went down - the cancel target Esc restores; never changes during
    /// the hold.
    captured: Transform,
    /// The current axis constraint for scale (`None` = uniform). Always `None` for the move ops. A
    /// change re-seeds the sub-drag.
    axis: Option<Axis>,
    /// The base the sub-drag's motion folds into (scale), re-seeded to the live transform on an axis
    /// change so the constraint switch picks up where the last left off. The move ops read the live
    /// placement directly, so they ignore it.
    base: Transform,
    /// The sub-drag's reference.
    drag: Drag,
}

/// The read-only per-frame inputs [`update`] consults, gathered in the frame loop. `loaded`, `scene`,
/// and a `Some` selection are guaranteed meaningful by the caller (the gizmo is inert without a loaded
/// scene and its render residency).
pub struct Inputs<'a> {
    pub input: &'a InputState,
    pub camera: &'a FlyCamera,
    pub loaded: &'a LoadedScene,
    /// The open scene's render residency: the surface-snap move casts against its colliders and terrain
    /// ([`surface_ray`](RenderScene::surface_ray)), and its far plane is the cursor ray's range (one
    /// cursor-to-ray source with the pick and the render, sharp-edges 2).
    pub scene: &'a RenderScene,
    pub selection: Option<InstanceId>,
    /// The editor-well rect (egui points) the 3D rendered into - the gizmo casts its rays against this.
    pub rect: egui::Rect,
    /// The pointer position in window points, or `None` when egui has no pointer this frame.
    pub cursor: Option<Vec2>,
    /// The pointer is over the viewport well under no foreground layer (the camera-lock engage gate):
    /// gates the held fast path so it never fires over a panel, menu, or the inspector.
    pub over_well: bool,
    /// No egui text field holds keyboard focus, so the op letters drive the gizmo rather than typing
    /// into the inspector's Name field.
    pub keyboard_free: bool,
}

/// What [`update`] tells the frame loop: a transform edit to route through `action::handle` this frame,
/// whether the gizmo consumed this frame's Esc to cancel a hold (so the loop drops the chrome's
/// deselect - a cancel unwinds the transform but keeps the selection), and whether it consumed the
/// scroll wheel for a free move's height (so the loop gates the camera dolly off this frame).
#[derive(Default)]
pub struct Outcome {
    pub action: Option<Action>,
    pub consumed_esc: bool,
    pub consumed_scroll: bool,
}

/// Advance the gizmo one frame: cancel, end, or advance an active hold, or engage a new one. Pure over
/// its [`Inputs`] except for the `&mut Option<Hold>` it owns - the camera, the model, and the disk are
/// the frame loop's. Returns the [`Outcome`] the loop applies and consults.
pub fn update(gizmo: &mut Option<Hold>, f: &Inputs) -> Outcome {
    // 1) An active hold: cancel, end, or advance it.
    if let Some(hold) = gizmo {
        // Esc cancels: restore the captured transform and end, telling the loop to drop the chrome's
        // deselect raised by the same Esc (a cancel unwinds the transform but keeps the selection).
        if f.input.key_pressed(NamedKey::Escape) {
            let (id, restore) = (hold.id, hold.captured);
            *gizmo = None;
            return Outcome {
                action: Some(Action::SetInstanceTransform(id, restore)),
                consumed_esc: true,
                consumed_scroll: false,
            };
        }
        // End when the op key lifts, the selection moves off the held instance, or the hold is no
        // longer allowed (the pointer left the well, a field took focus, a camera drag began). The edit
        // is already persisted live, so ending is just dropping the state.
        if !f.input.char_held(hold.op.key()) || f.selection != Some(hold.id) || !hold_allowed(f) {
            *gizmo = None;
            return Outcome::default();
        }
        // Re-seed when the axis constraint changes (scale), fold this frame's motion in, then emit. A
        // free move consumes the scroll, so the loop gates the camera dolly off.
        reseed_if_axis_changed(hold, f);
        accumulate(hold, f);
        let consumed_scroll = hold.op == Op::MoveFree;
        return Outcome { action: emit(hold, f), consumed_esc: false, consumed_scroll };
    }

    // 2) Idle (no active hold): a discrete rotate tap (W / E / R) is a one-shot edit checked first, then
    // an op key held over the well engages a hold, applying this frame's motion at once so a quick flick
    // on the engage frame is not dropped. Both share the gate (selection, over the well, no focused
    // field, no camera drag); taps fire only from idle, so they never interrupt a move or scale hold.
    if hold_allowed(f) {
        if let Some(action) = rotate_tap(f) {
            return Outcome { action: Some(action), ..Outcome::default() };
        }
        if let Some(mut hold) = engage(f) {
            accumulate(&mut hold, f);
            let action = emit(&hold, f);
            let consumed_scroll = hold.op == Op::MoveFree;
            *gizmo = Some(hold);
            return Outcome { action, consumed_esc: false, consumed_scroll };
        }
    }
    Outcome::default()
}

/// Begin a hold over the current selection, capturing the base transform as the cancel target and
/// seeding the sub-drag from the held op key. `None` when no op key is held or the selection does not
/// resolve to a placement; a held op key just retries next frame.
fn engage(f: &Inputs) -> Option<Hold> {
    let op = current_op(f)?;
    let id = f.selection?;
    let base = f.loaded.placement(id)?.transform;
    let axis = axis_for(op, f);
    Some(Hold { id, op, captured: base, axis, base, drag: seed(op) })
}

/// Re-anchor the sub-drag when the held axis keys differ from last frame's constraint (scale only - the
/// move ops never take an axis, so their `None` never changes): a new base from the live transform (so
/// the switch continues from where the last constraint left off) and a fresh motion accumulator (so the
/// toggle does not jump).
fn reseed_if_axis_changed(hold: &mut Hold, f: &Inputs) {
    let axis = axis_for(hold.op, f);
    if axis == hold.axis {
        return;
    }
    hold.base = f.loaded.placement(hold.id).map_or(hold.base, |p| p.transform);
    hold.axis = axis;
    hold.drag = seed(hold.op);
}

/// The sub-drag reference for an op at engage (and re-anchor): a fresh cursor reference for a move, a
/// zeroed motion accumulator for scale.
fn seed(op: Op) -> Drag {
    match op {
        Op::MoveSurface | Op::MoveFree => Drag::Cursor,
        Op::Scale => Drag::Motion(0.0),
    }
}

/// Fold this frame's raw horizontal mouse motion into the scale accumulator. A move drag reads the
/// cursor ray fresh each frame, so it has nothing to accumulate.
fn accumulate(hold: &mut Hold, f: &Inputs) {
    if let Drag::Motion(accum) = &mut hold.drag {
        *accum += f.input.mouse_motion.0 as f32;
    }
}

/// The edit a hold emits this frame, or `None` (hold position, the seam no-ops) when the target cannot
/// be resolved (the cursor is gone, or the ray meets nothing).
fn emit(hold: &Hold, f: &Inputs) -> Option<Action> {
    Some(Action::SetInstanceTransform(hold.id, target(hold, f)?))
}

/// Resolve the held op + input into the placement's new transform. `None` when the move's cursor ray is
/// gone or meets nothing, so the caller holds position.
fn target(hold: &Hold, f: &Inputs) -> Option<Transform> {
    match hold.op {
        Op::MoveSurface => target_surface(hold, f),
        Op::MoveFree => target_free(hold, f),
        Op::Scale => target_scale(hold),
    }
}

/// Surface-snap move (`g`): rest the placement's BOTTOM on the surface the cursor aims at, then snap the
/// resulting pivot to the 1m grid - consistent with X/Z, so translation x, y, and z are all grid-whole.
/// The cursor ray's surface hit (terrain or a prefab beneath, never the moving instance) gives the XZ
/// and the bottom-rest height, lifted to the pivot by the item's pivot-to-bottom offset ([`rest_y`] over
/// the instance's live AABB) so a centre-pivoted prefab is not half-buried; snapping that pivot leaves
/// the base within +/-0.5m of the surface. Sourcing the height from the actual hit - not the highest
/// surface in the column - means G follows the aim: point at terrain under or past a prefab and it rests
/// on the ground; point at a prefab top and it stacks; no teleport onto a prefab the XZ slides under.
/// (Penetration is allowed - nothing clamps.) `None` (hold position) when the cursor is gone or the ray
/// meets only sky; a shape-less instance snaps the hit height directly.
fn target_surface(hold: &Hold, f: &Inputs) -> Option<Transform> {
    let base = f.loaded.placement(hold.id)?.transform;
    let (origin, dir) = cursor_ray(f, f.cursor?);
    let hit = f.scene.surface_ray(origin, dir, hold.id)?;
    let x = snap(hit.x, TRANSLATE_SNAP_M);
    let z = snap(hit.z, TRANSLATE_SNAP_M);
    // The pivot that rests the bottom on the surface hit, snapped like X/Z so translation is grid-whole
    // (the base then sits within +/-0.5m of the surface; penetration is fine).
    let pivot = match f.scene.instance_aabb(hold.id) {
        Some(aabb) => rest_y(hit.y, base.translation.y, aabb.min.y),
        None => hit.y,
    };
    let y = snap(pivot, TRANSLATE_SNAP_M);
    Some(Transform { translation: Vec3::new(x, y, z), ..base })
}

/// Free move (`f`): slide in the horizontal plane at the live height (cursor ray vs the plane, XZ
/// snapped to the 1m grid), the scroll wheel stepping the height by 1m of world vertical per notch.
/// The two are decoupled: XZ is re-read from the cursor only when the cursor actually moved, so a
/// scroll-only frame holds XZ and the placement rises and lowers straight up in world space - rather
/// than re-reading the cursor on the new height plane, which on an angled camera would pull XZ toward
/// the eye and read as moving closer. `None` (hold position) when a cursor move's ray runs parallel to
/// the plane or the cursor is gone.
fn target_free(hold: &Hold, f: &Inputs) -> Option<Transform> {
    let base = f.loaded.placement(hold.id)?.transform;
    let y = stepped_y(base.translation.y, f.input.scroll_delta.1);
    let (mut x, mut z) = (base.translation.x, base.translation.z);
    if cursor_moved(f) {
        let (origin, dir) = cursor_ray(f, f.cursor?);
        let hit = ray_vs_ground_plane(origin, dir, base.translation.y)?;
        x = snap(hit.x, TRANSLATE_SNAP_M);
        z = snap(hit.z, TRANSLATE_SNAP_M);
    }
    Some(Transform { translation: Vec3::new(x, y, z), ..base })
}

/// Scale (`s`): unchanged - exponential in raw horizontal motion (so left / right are symmetric and the
/// factor never reaches zero), uniform or one component by axis. Scale has no grid, so unlike rotate it
/// reads no Alt and needs no per-frame input beyond its accumulated motion.
fn target_scale(hold: &Hold) -> Option<Transform> {
    let Drag::Motion(accum) = hold.drag else { return None };
    let factor = (accum * SCALE_PER_PX).exp();
    let scale = match hold.axis {
        None => hold.base.scale * factor,
        Some(ax) => {
            let mut s = hold.base.scale;
            s[ax.index()] *= factor;
            s
        }
    };
    Some(Transform { scale, ..hold.base })
}

/// The held op key this frame, if any (G surface move, F free move, S scale; first match wins when
/// several are down). Rotate is not here - it is a discrete tap, not a held op.
fn current_op(f: &Inputs) -> Option<Op> {
    if f.input.char_held(MOVE_SURFACE_KEY) {
        Some(Op::MoveSurface)
    } else if f.input.char_held(MOVE_FREE_KEY) {
        Some(Op::MoveFree)
    } else if f.input.char_held(SCALE_KEY) {
        Some(Op::Scale)
    } else {
        None
    }
}

/// A discrete rotate tap this frame, if any: one [`ROTATE_STEP_DEG`] step about the world axis named by
/// a W / E / R press (pitch / yaw / roll), reversed with Shift. Keyboard-only and one-shot - no hold,
/// no cancel; the press edge (`char_pressed`, OS auto-repeat swallowed) makes one tap exactly one step,
/// so holding a key does not spin. `None` when no rotate key edged this frame or the selection or its
/// placement does not resolve. The caller has already gated it (`hold_allowed`).
fn rotate_tap(f: &Inputs) -> Option<Action> {
    let axis = tapped_axis(f)?;
    let id = f.selection?;
    let base = f.loaded.placement(id)?.transform;
    let degrees = if f.input.key_held(NamedKey::Shift) { -ROTATE_STEP_DEG } else { ROTATE_STEP_DEG };
    Some(Action::SetInstanceTransform(id, rotate_step(base, axis, degrees)))
}

/// The world axis a rotate tap names this frame (W pitch X, E yaw Y, R roll Z; first match wins when
/// several edge the same frame), or `None` when no rotate key was pressed.
fn tapped_axis(f: &Inputs) -> Option<Axis> {
    if f.input.char_pressed(ROTATE_PITCH_KEY) {
        Some(Axis::X)
    } else if f.input.char_pressed(ROTATE_YAW_KEY) {
        Some(Axis::Y)
    } else if f.input.char_pressed(ROTATE_ROLL_KEY) {
        Some(Axis::Z)
    } else {
        None
    }
}

/// The world-axis constraint for an op this frame: the X / Y / Z key for scale (first match wins when
/// several are down), always `None` for the move ops (no axis chords on move).
fn axis_for(op: Op, f: &Inputs) -> Option<Axis> {
    if !op.takes_axis() {
        return None;
    }
    if f.input.char_held('x') {
        Some(Axis::X)
    } else if f.input.char_held('y') {
        Some(Axis::Y)
    } else if f.input.char_held('z') {
        Some(Axis::Z)
    } else {
        None
    }
}

/// Whether a hold may run this frame: something selected, the pointer over the well, no text field
/// focused, and no camera drag in progress (right / middle held).
fn hold_allowed(f: &Inputs) -> bool {
    f.selection.is_some()
        && f.over_well
        && f.keyboard_free
        && !f.input.mouse_held(MouseButton::Right)
        && !f.input.mouse_held(MouseButton::Middle)
}

/// Did the mouse physically move this frame? The free move re-reads the cursor's XZ only when it did,
/// so a scroll-only frame (hand on the wheel, mouse still) holds XZ and steps the height purely
/// vertically. Raw motion is the right signal: it is non-zero exactly when the device moved, and the
/// free move runs with no cursor lock, so it tracks the visible cursor.
fn cursor_moved(f: &Inputs) -> bool {
    f.input.mouse_motion.0 != 0.0 || f.input.mouse_motion.1 != 0.0
}

/// The cursor ray for a window-space cursor position, mapped against the well rect at the scene far
/// plane (the same source the pick and the render use - sharp-edges 2).
fn cursor_ray(f: &Inputs, cursor: Vec2) -> (Vec3, Vec3) {
    let size = Vec2::new(f.rect.width(), f.rect.height());
    let pos_in_rect = cursor - Vec2::new(f.rect.min.x, f.rect.min.y);
    f.camera.cursor_ray(pos_in_rect, size, f.scene.far_plane())
}

// ---- pure geometry (unit tested) ----

/// Snap `v` to the nearest multiple of `step`; a non-positive `step` (Alt = no snap) passes through.
fn snap(v: f32, step: f32) -> f32 {
    if step <= 0.0 { v } else { (v / step).round() * step }
}

/// The free move's new height after `notches` of scroll: a fixed step per notch, positive scroll
/// raising. Pure so the step is unit tested; the frame loop gates the camera dolly off while this
/// drives the height.
fn stepped_y(base_y: f32, notches: f32) -> f32 {
    base_y + notches * MOVE_Y_STEP_M
}

/// The translation Y that rests an item's BOTTOM at world height `floor`: `floor` plus the item's
/// pivot-to-bottom offset (`base_y - aabb_min_y`, the height of the origin above the item's lowest point
/// at the live rotation and scale). A centre-pivoted prefab would otherwise sink half-in; this lifts it
/// so the lowest point sits at `floor`. The offset cancels `base_y`, so it is invariant under where the
/// item currently sits and depends only on the item's shape. G feeds `floor` the raw cursor surface-hit
/// height and snaps the resulting pivot to the 1m grid (so translation.y is grid-whole, like X/Z).
fn rest_y(floor: f32, base_y: f32, aabb_min_y: f32) -> f32 {
    floor + (base_y - aabb_min_y)
}

/// Turn `base` by `degrees` about world `axis`, pre-multiplied so the rotation is about the world axis
/// regardless of the placement's current heading (the W / E / R taps, +/-[`ROTATE_STEP_DEG`]). The
/// non-rotation fields pass through. Pure so the step is unit tested.
fn rotate_step(base: Transform, axis: Axis, degrees: f32) -> Transform {
    let rotation = Quat::from_axis_angle(axis.unit(), degrees.to_radians()) * base.rotation;
    Transform { rotation, ..base }
}

/// The world point where the ray `origin + t*dir` meets the horizontal plane `y = height`, or `None`
/// when the ray runs parallel to the plane (no crossing) or the crossing is behind the eye (`t <= 0`).
/// The free move casts the cursor ray at the selection's height.
fn ray_vs_ground_plane(origin: Vec3, dir: Vec3, height: f32) -> Option<Vec3> {
    if dir.y.abs() < 1e-5 {
        return None;
    }
    let t = (height - origin.y) / dir.y;
    if t <= 0.0 {
        return None;
    }
    Some(origin + dir * t)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn snap_rounds_to_the_nearest_multiple_and_a_nonpositive_step_passes_through() {
        // The 1m translate grid (G / F XZ), plus the no-snap pass-through. snap is generic over the step.
        assert_eq!(TRANSLATE_SNAP_M, 1.0, "the move grid is a clean 1m");
        assert_eq!(snap(0.4, TRANSLATE_SNAP_M), 0.0);
        assert_eq!(snap(0.6, TRANSLATE_SNAP_M), 1.0);
        assert_eq!(snap(-1.4, TRANSLATE_SNAP_M), -1.0);
        assert_eq!(snap(7.0, 5.0), 5.0, "rounds to the nearest multiple of any step");
        assert_eq!(snap(3.3, 0.0), 3.3, "a non-positive step is a pass-through");
    }

    #[test]
    fn rotate_step_turns_5deg_about_the_world_axis_premultiplied() {
        // W / E / R taps turn 5deg about world X / Y / Z; +5 unshifted, -5 with Shift; pre-multiplied so
        // the turn is about the world axis regardless of the placement's heading.
        assert_eq!(ROTATE_STEP_DEG, 5.0, "a tap is the 5deg canon step");
        let v = Vec3::new(0.3, 0.5, 0.8);
        // E: +5deg about world Y; Shift+W: -5deg about world X.
        let yaw = rotate_step(Transform::IDENTITY, Axis::Y, ROTATE_STEP_DEG).rotation;
        assert!((yaw * v - Quat::from_rotation_y(ROTATE_STEP_DEG.to_radians()) * v).length() < EPS, "E yaws +5 about Y");
        let pitch = rotate_step(Transform::IDENTITY, Axis::X, -ROTATE_STEP_DEG).rotation;
        assert!((pitch * v - Quat::from_rotation_x(-ROTATE_STEP_DEG.to_radians()) * v).length() < EPS, "Shift+W pitches -5");
        // Pre-multiplied: from a turned base, the world-axis turn composes on the world (left) side.
        let base = Transform { rotation: Quat::from_rotation_y(1.0), ..Transform::IDENTITY };
        let rolled = rotate_step(base, Axis::Z, ROTATE_STEP_DEG).rotation;
        let world = Quat::from_rotation_z(ROTATE_STEP_DEG.to_radians()) * base.rotation;
        assert!((rolled * v - world * v).length() < EPS, "roll pre-multiplies about world Z");
        // The non-rotation fields pass through.
        let scaled = Transform { scale: Vec3::splat(2.0), ..Transform::IDENTITY };
        assert_eq!(rotate_step(scaled, Axis::Y, ROTATE_STEP_DEG).scale, Vec3::splat(2.0), "scale untouched");
    }

    #[test]
    fn stepped_y_steps_one_metre_of_world_vertical_per_notch_and_scroll_up_raises() {
        // F + scroll: exactly 1m of world height per notch, positive (scroll up) raising, independent
        // of the camera. The dolly is gated off in the same condition so the wheel drives only height.
        assert_eq!(MOVE_Y_STEP_M, 1.0, "world-vertical steps are a clean 1m");
        assert_eq!(stepped_y(2.0, 0.0), 2.0, "no scroll holds the height");
        assert_eq!(stepped_y(2.0, 1.0), 3.0, "one notch up raises 1m");
        assert_eq!(stepped_y(2.0, -2.0), 0.0, "two notches down lowers 2m");
    }

    #[test]
    fn rest_y_lifts_the_bottom_to_the_floor_and_g_snaps_the_pivot_whole() {
        // rest_y places an item's bottom at `floor`: a centred 2m box (AABB min.y = base - 1) rests its
        // centre at 1.0 on flat ground (floor 0) and at 2.0 on a 1m floor. The pivot-to-bottom offset
        // cancels the current height, so the result is invariant under where the item sits now.
        assert_eq!(rest_y(0.0, 0.0, -1.0), 1.0, "on flat ground the 2m box's centre lifts to 1.0");
        assert_eq!(rest_y(1.0, 0.0, -1.0), 2.0, "a 1m floor rests it at 2.0");
        assert_eq!(rest_y(0.0, 5.0, 4.0), 1.0, "the same 1m offset, measured from a different height");
        // G snaps the resulting PIVOT to the grid (like X/Z), so translation.y is grid-whole even when
        // the half-height is non-integer. A unit cube (AABB min.y = base - 0.5) aimed at a surface
        // y = 3.4: the flush pivot 3.9 snaps to a whole 4.0 (the base then sits within +/-0.5m of the
        // surface). Before, the base was snapped and the offset added, leaving translation.y at X.5.
        let pivot = snap(rest_y(3.4, 0.0, -0.5), TRANSLATE_SNAP_M);
        assert_eq!(pivot, 4.0, "the snapped pivot is whole");
        assert_eq!(pivot.fract(), 0.0, "translation.y is grid-whole, not X.5");
    }

    #[test]
    fn ray_vs_ground_plane_hits_at_the_plane_height_under_the_cursor() {
        // A ray dropping straight down from (3, 5, 2) meets the plane y = 1 at (3, 1, 2) - the free
        // move places the selection under the cursor at its height.
        let hit = ray_vs_ground_plane(Vec3::new(3.0, 5.0, 2.0), Vec3::NEG_Y, 1.0).unwrap();
        assert!((hit - Vec3::new(3.0, 1.0, 2.0)).length() < EPS, "got {hit:?}");
    }

    #[test]
    fn ray_vs_ground_plane_is_none_when_parallel_or_behind() {
        // Parallel to the plane: no crossing. Pointing up away from a plane below the eye: the crossing
        // is behind, so None - the move holds rather than flinging the selection.
        assert_eq!(ray_vs_ground_plane(Vec3::new(0.0, 5.0, 0.0), Vec3::X, 1.0), None);
        assert_eq!(ray_vs_ground_plane(Vec3::new(0.0, 5.0, 0.0), Vec3::Y, 1.0), None);
    }
}
