//! The transform manipulator: one held grammar over the selected placement, plus a static axis tripod
//! marking it. Per the device split (designs/editor-design.md, Input: the mouse is motion, the left
//! hand is the operators), a transform is an OP key held with an optional world-AXIS key and the mouse
//! - Blender's G / R / S + X / Y / Z, but held-not-modal: the manipulation lives only while the op key
//! is down, and releasing it commits (the edit is already persisted live). The grammar ([`update`],
//! driven from the frame loop where the camera and the raw input live):
//! - Move (`g`): no axis slides on the ground plane (XZ at the placement's current Y) under the cursor
//!   ray; `x` / `y` / `z` constrains to that world axis. 1m grid snap.
//! - Rotate (`r`): no axis or `y` yaws about world Y, `x` pitches, `z` rolls, from raw horizontal mouse
//!   motion. 5deg snap.
//! - Scale (`s`): no axis scales uniformly, `x` / `y` / `z` scales that one component, from raw motion.
//!
//! Alt held disables snapping (fine). An axis key pressed or released mid-hold re-anchors the drag at
//! the live transform, so a toggle never jumps. Esc cancels (restores the captured transform, keeps the
//! selection); op-key-up commits. Keys read through `char_held`, so they are rebindable later (table
//! parked). Every change routes through the existing edit seam: [`update`] returns an
//! [`Action::SetInstanceTransform`](crate::action::Action) the frame loop applies through
//! `crate::action::handle`, exactly as the inspector's fields do; [`LoadedScene::set_transform`] no-ops
//! an unchanged transform, so emitting the full transform every held frame is free.
//!
//! The only viewport visual is [`draw`]'s tripod: three short world-axis lines (X red, Y green, Z blue)
//! from the anchor, screen-constant length, non-interactive - there are no draggable handles, the
//! keyboard names the axis. It paints on a Background-order layer clipped to the well, under the
//! floating inspector and the menus; the static snapshot tests pass no [`GizmoView`], so it never enters
//! their PNGs. The pure geometry (ray vs axis, ray vs ground plane, world-to-screen, the grid snap) is
//! unit tested below, and one cursor-to-ray source serves the draw, the pick, and the drag
//! (sharp-edges 2).

use egui::{Color32, Stroke};
use glam::{Mat4, Quat, Vec2, Vec3};
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::NamedKey;
use wok_scene::{InstanceId, Transform};

use crate::action::Action;
use crate::camera::FlyCamera;
use crate::loaded::LoadedScene;
use crate::render_scene::chunk_origin;

/// The op keys (read case-insensitively through `char_held`; rebindable later, the keybind table is
/// parked). Move is `g` like Blender's grab, leaving the inspector's fields the precise path.
const MOVE_KEY: char = 'g';
const ROTATE_KEY: char = 'r';
const SCALE_KEY: char = 's';

/// Tripod line length as a fraction of the distance from the camera to the anchor, so the on-screen
/// length is constant at any zoom (a far selection gets a longer world line).
const AXIS_SCREEN_FRAC: f32 = 0.15;
/// Tripod stroke width in points, and the arrowhead leg length (points) and half-angle (radians).
const AXIS_WIDTH: f32 = 2.5;
const ARROW_LEN: f32 = 11.0;
const ARROW_HALF_ANGLE: f32 = 0.45;

/// The axis tint colours, mirroring the inspector's `AXIS_*`: fixed regardless of the light/dark theme
/// because they name the world axes, not chrome surfaces (X warm red, Y green, Z blue).
const AXIS_X: Color32 = Color32::from_rgb(0xd8, 0x53, 0x4a);
const AXIS_Y: Color32 = Color32::from_rgb(0x5b, 0xbd, 0x5b);
const AXIS_Z: Color32 = Color32::from_rgb(0x4a, 0x86, 0xd8);

/// Snap defaults (canon: 1m grid, 5deg steps). Alt disables snapping per hold; scale has no grid, so
/// only move and rotate snap.
const TRANSLATE_SNAP_M: f32 = 1.0;
const ROTATE_SNAP_DEG: f32 = 5.0;
/// Hold sensitivities: degrees of rotation and scale exponent per point of raw horizontal mouse motion.
const ROTATE_DEG_PER_PX: f32 = 0.5;
const SCALE_PER_PX: f32 = 0.005;

/// A world axis named by the X / Y / Z keys (world-only this bite; no local-vs-world toggle).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    /// The unit world direction of the axis.
    fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    /// The translation / scale component index the axis drives (X=0, Y=1, Z=2).
    fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    /// The axis tint.
    fn color(self) -> Color32 {
        match self {
            Axis::X => AXIS_X,
            Axis::Y => AXIS_Y,
            Axis::Z => AXIS_Z,
        }
    }
}

/// The operator a hold applies, chosen by the op key down. Each lives only while its key is held.
#[derive(Clone, Copy)]
enum Op {
    Move,
    Rotate,
    Scale,
}

impl Op {
    /// The op key that drives this manipulation.
    fn key(self) -> char {
        match self {
            Op::Move => MOVE_KEY,
            Op::Rotate => ROTATE_KEY,
            Op::Scale => SCALE_KEY,
        }
    }
}

/// The current sub-drag's motion reference, seeded at op-key-down and re-seeded on an axis change. The
/// variant follows the op + axis: a ground slide, an axis slide, or the raw-motion accumulation rotate
/// and scale share. Relative in every case, so the engage frame emits the base unchanged (no jump).
#[derive(Clone, Copy)]
enum Drag {
    /// Move, no axis: the world point on the ground plane the cursor ray grabbed at seed.
    Ground(Vec3),
    /// Move, +axis: the parameter along the constrained axis line the cursor ray grabbed at seed.
    Axis(f32),
    /// Rotate or scale: raw horizontal mouse motion accumulated since the seed.
    Motion(f32),
}

/// An in-progress held transform (G / R / S), held across frames by the frame loop (`Option<Hold>` on
/// the editor). Lives only while its op key is down; releasing it (or moving the selection off it)
/// commits, Esc cancels. The idle state is the `None` outside.
pub struct Hold {
    id: InstanceId,
    op: Op,
    /// The transform when the op key went down - the cancel target Esc restores; never changes during
    /// the hold.
    captured: Transform,
    /// The current axis constraint (`None` = the op's default: ground for move, world Y for rotate,
    /// uniform for scale). A change re-seeds the sub-drag.
    axis: Option<Axis>,
    /// The base the sub-drag's motion folds into, re-seeded to the live transform on an axis change so
    /// the constraint switch picks up where the last one left off.
    base: Transform,
    /// The sub-drag's motion reference.
    drag: Drag,
}

/// What the tripod draw needs from the frame loop, computed where the camera and render residency live
/// and threaded through `view::chrome` to the [`draw`] call. The anchor is resolved inside [`draw`] from
/// the same loaded scene the chrome already holds. `Copy` so it can be read out of the egui build
/// closure (typed `FnMut`) by value rather than moved.
#[derive(Clone, Copy)]
pub struct GizmoView {
    /// The viewport camera, for projecting the world-space tripod to screen.
    pub camera: FlyCamera,
    /// The scene's far plane (its render distance), so the projection matches the 3D and the picking.
    pub far: f32,
}

/// The read-only per-frame inputs [`update`] consults, gathered in the frame loop. `loaded` and a
/// `Some` selection are guaranteed by the caller (the gizmo is inert without a loaded scene).
pub struct Inputs<'a> {
    pub input: &'a InputState,
    pub camera: &'a FlyCamera,
    pub loaded: &'a LoadedScene,
    pub selection: Option<InstanceId>,
    /// The scene far plane, matching the picking and the render so one cursor-to-ray source serves all.
    pub far: f32,
    /// The editor-well rect (egui points) the 3D rendered into - the gizmo casts its rays against this.
    pub rect: egui::Rect,
    /// The pointer position in window points, or `None` when egui has no pointer this frame.
    pub cursor: Option<Vec2>,
    /// The pointer is over the viewport well under no foreground layer (the camera-lock engage gate):
    /// gates the held fast path so it never fires over a panel, menu, or the inspector.
    pub over_well: bool,
    /// No egui text field holds keyboard focus, so the op / axis letters drive the gizmo rather than
    /// typing into the inspector's Name field.
    pub keyboard_free: bool,
}

/// What [`update`] tells the frame loop: a transform edit to route through `action::handle` this frame,
/// and whether the gizmo consumed this frame's Esc to cancel a hold (so the loop drops the chrome's
/// deselect - a cancel unwinds the transform but keeps the selection).
#[derive(Default)]
pub struct Outcome {
    pub action: Option<Action>,
    pub consumed_esc: bool,
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
            return Outcome { action: Some(Action::SetInstanceTransform(id, restore)), consumed_esc: true };
        }
        // End when the op key lifts, the selection moves off the held instance, or the hold is no
        // longer allowed (the pointer left the well, a field took focus, a camera drag began). The edit
        // is already persisted live, so ending is just dropping the state.
        if !f.input.char_held(hold.op.key()) || f.selection != Some(hold.id) || !hold_allowed(f) {
            *gizmo = None;
            return Outcome::default();
        }
        // Re-seed when the axis constraint changes, fold this frame's motion in (rotate / scale; move
        // reads the cursor fresh), then emit.
        reseed_if_axis_changed(hold, f);
        accumulate(hold, f);
        return Outcome { action: emit(hold, f), consumed_esc: false };
    }

    // 2) Idle: an op key held over the well (gated) engages a hold, applying this frame's motion at once
    // so a quick flick on the engage frame is not dropped.
    if hold_allowed(f) {
        if let Some(mut hold) = engage(f) {
            accumulate(&mut hold, f);
            let action = emit(&hold, f);
            *gizmo = Some(hold);
            return Outcome { action, consumed_esc: false };
        }
    }
    Outcome::default()
}

/// Begin a hold over the current selection, capturing the base transform as the cancel target and
/// seeding the sub-drag from the held op + axis keys. `None` when no op key is held, the selection does
/// not resolve, or a move seed is degenerate (the cursor ray meets neither the ground plane nor the
/// axis); a held op key just retries next frame.
fn engage(f: &Inputs) -> Option<Hold> {
    let op = current_op(f)?;
    let id = f.selection?;
    let base = f.loaded.placement(id)?.transform;
    let axis = current_axis(f);
    let drag = seed(op, axis, &base, f)?;
    Some(Hold { id, op, captured: base, axis, base, drag })
}

/// Re-anchor the sub-drag when the held axis keys differ from last frame's constraint: a new base from
/// the live transform (so the switch continues from where the last constraint left off) and a fresh
/// cursor / motion reference (so the toggle does not jump). A degenerate seed leaves the sub-drag as it
/// was and retries next frame, keeping `axis` and `drag` consistent.
fn reseed_if_axis_changed(hold: &mut Hold, f: &Inputs) {
    let axis = current_axis(f);
    if axis == hold.axis {
        return;
    }
    let base = f.loaded.placement(hold.id).map_or(hold.base, |p| p.transform);
    if let Some(drag) = seed(hold.op, axis, &base, f) {
        hold.axis = axis;
        hold.base = base;
        hold.drag = drag;
    }
}

/// The sub-drag reference for an op + axis at the current cursor: a ground point, an axis parameter, or
/// a zeroed motion accumulator. `None` for a move whose cursor ray is gone or near-parallel to the
/// target (no well-defined grab), so the caller retries rather than seeding a jump.
fn seed(op: Op, axis: Option<Axis>, base: &Transform, f: &Inputs) -> Option<Drag> {
    match op {
        Op::Rotate | Op::Scale => Some(Drag::Motion(0.0)),
        Op::Move => {
            let (origin, dir) = cursor_ray(f, f.cursor?);
            match axis {
                None => ray_vs_ground_plane(origin, dir, base.translation.y).map(Drag::Ground),
                Some(ax) => closest_param_on_axis(origin, dir, base.translation, ax.unit()).map(Drag::Axis),
            }
        }
    }
}

/// Fold this frame's raw horizontal mouse motion into a rotate / scale accumulator. A move drag reads
/// the cursor ray fresh each frame, so it has nothing to accumulate.
fn accumulate(hold: &mut Hold, f: &Inputs) {
    if let Drag::Motion(accum) = &mut hold.drag {
        *accum += f.input.mouse_motion.0 as f32;
    }
}

/// The edit a hold emits this frame, or `None` (hold position, the seam no-ops) when the cursor is gone
/// or its ray is degenerate.
fn emit(hold: &Hold, f: &Inputs) -> Option<Action> {
    Some(Action::SetInstanceTransform(hold.id, target(hold, f)?))
}

/// Resolve the held op + axis + accumulated input into the placement's new transform, folded onto the
/// sub-drag base. Snapped (1m / 5deg) unless Alt is held. `None` when a move's cursor ray is gone or
/// near-parallel to its target, so the caller holds position.
fn target(hold: &Hold, f: &Inputs) -> Option<Transform> {
    let base = hold.base;
    let snap_on = !f.input.key_held(NamedKey::Alt);
    match hold.op {
        Op::Move => {
            let (origin, dir) = cursor_ray(f, f.cursor?);
            let translation = match hold.drag {
                // Ground slide: the cursor's ground-plane point moves the placement by the same delta
                // it has travelled since the grab, height held. Snap X and Z to the 1m grid.
                Drag::Ground(grab) => {
                    let hit = ray_vs_ground_plane(origin, dir, base.translation.y)?;
                    let mut t = base.translation + (hit - grab);
                    t.y = base.translation.y;
                    if snap_on {
                        t.x = snap(t.x, TRANSLATE_SNAP_M);
                        t.z = snap(t.z, TRANSLATE_SNAP_M);
                    }
                    t
                }
                // Axis slide: the closest point on the fixed axis line to the cursor ray gives the
                // along-axis parameter; its delta from the grab is the move. Snap the moved component.
                Drag::Axis(grab_param) => {
                    let ax = hold.axis?;
                    let param = closest_param_on_axis(origin, dir, base.translation, ax.unit())?;
                    let mut t = base.translation + ax.unit() * (param - grab_param);
                    if snap_on {
                        let i = ax.index();
                        t[i] = snap(t[i], TRANSLATE_SNAP_M);
                    }
                    t
                }
                Drag::Motion(_) => return None, // a move never seeds a motion accumulator
            };
            Some(Transform { translation, ..base })
        }
        Op::Rotate => {
            let Drag::Motion(accum) = hold.drag else { return None };
            let mut degrees = accum * ROTATE_DEG_PER_PX;
            if snap_on {
                degrees = snap(degrees, ROTATE_SNAP_DEG);
            }
            // Pre-multiply so the rotation is about the chosen world axis (default Y) regardless of the
            // placement's heading.
            let ax = hold.axis.unwrap_or(Axis::Y);
            let rotation = Quat::from_axis_angle(ax.unit(), degrees.to_radians()) * base.rotation;
            Some(Transform { rotation, ..base })
        }
        Op::Scale => {
            let Drag::Motion(accum) = hold.drag else { return None };
            // Exponential so left / right are symmetric and the factor never reaches zero.
            let factor = (accum * SCALE_PER_PX).exp();
            let scale = match hold.axis {
                None => base.scale * factor,
                Some(ax) => {
                    let mut s = base.scale;
                    s[ax.index()] *= factor;
                    s
                }
            };
            Some(Transform { scale, ..base })
        }
    }
}

/// The op key held this frame, if any (G move, R rotate, S scale; first match wins when several are
/// down).
fn current_op(f: &Inputs) -> Option<Op> {
    if f.input.char_held(MOVE_KEY) {
        Some(Op::Move)
    } else if f.input.char_held(ROTATE_KEY) {
        Some(Op::Rotate)
    } else if f.input.char_held(SCALE_KEY) {
        Some(Op::Scale)
    } else {
        None
    }
}

/// The world-axis constraint held this frame, or `None` for the op's default (first match wins when
/// several are down).
fn current_axis(f: &Inputs) -> Option<Axis> {
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

/// The world-space anchor of the selected placement: its chunk origin plus its local translation (the
/// chunk origin is a pure translation, and the tripod's axes are world axes this bite). `None` when the
/// id resolves to no placement or its chunk is absent.
fn anchor(loaded: &LoadedScene, id: InstanceId) -> Option<Vec3> {
    let placement = loaded.placement(id)?;
    let chunk = loaded.chunks().iter().find(|c| c.placements.iter().any(|p| p.instance_id == id))?;
    Some(chunk_origin(chunk.coord) + placement.transform.translation)
}

/// The cursor ray for a window-space cursor position, mapped against the well rect (the same source the
/// pick uses - sharp-edges 2).
fn cursor_ray(f: &Inputs, cursor: Vec2) -> (Vec3, Vec3) {
    let size = Vec2::new(f.rect.width(), f.rect.height());
    let pos_in_rect = cursor - Vec2::new(f.rect.min.x, f.rect.min.y);
    f.camera.cursor_ray(pos_in_rect, size, f.far)
}

/// The world-space tripod length: a fraction of the camera-to-anchor distance, so the projected length
/// is roughly constant on screen (floored so a camera sitting on the anchor still yields a finite line).
fn axis_len(camera: &FlyCamera, anchor: Vec3) -> f32 {
    (AXIS_SCREEN_FRAC * (anchor - camera.position).length()).max(0.01)
}

/// Draw the static axis tripod over the viewport when a placement is selected: three coloured world-axis
/// lines from the anchor, each tipped with an arrowhead pointing the positive direction, on a
/// Background-order layer clipped to the well (above the 3D, under the inspector and menus - see the
/// module doc). Non-interactive: it marks the selection and names the axes the keyboard constrains to.
/// A no-op unless the selection resolves to a placement and the anchor projects in front of the camera;
/// the length is screen-constant, so the tripod stays the same size at any zoom.
pub fn draw(
    ctx: &egui::Context,
    loaded_scene: Option<&LoadedScene>,
    selection: Option<InstanceId>,
    rect: egui::Rect,
    view: &GizmoView,
) {
    let Some(id) = selection else { return };
    let Some(loaded) = loaded_scene else { return };
    let Some(anchor) = anchor(loaded, id) else { return };
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let view_proj = view.camera.view_proj(rect.width() / rect.height(), view.far);
    let Some(base) = world_to_screen(view_proj, rect, anchor) else { return };
    let len = axis_len(&view.camera, anchor);

    let painter = ctx
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("axis_tripod")))
        .with_clip_rect(rect);
    for axis in Axis::ALL {
        let Some(tip) = world_to_screen(view_proj, rect, anchor + axis.unit() * len) else { continue };
        let stroke = Stroke::new(AXIS_WIDTH, axis.color());
        painter.line_segment([pos(base), pos(tip)], stroke);
        draw_arrowhead(&painter, base, tip, stroke);
    }
}

/// Paint a two-legged arrowhead at `tip`, opening back toward `base`, in the line's stroke. Screen
/// space, so the head is a constant size at any zoom.
fn draw_arrowhead(painter: &egui::Painter, base: Vec2, tip: Vec2, stroke: Stroke) {
    let back = (base - tip).normalize_or_zero();
    if back == Vec2::ZERO {
        return;
    }
    let (s, c) = ARROW_HALF_ANGLE.sin_cos();
    let leg_a = Vec2::new(back.x * c - back.y * s, back.x * s + back.y * c) * ARROW_LEN;
    let leg_b = Vec2::new(back.x * c + back.y * s, -back.x * s + back.y * c) * ARROW_LEN;
    painter.line_segment([pos(tip), pos(tip + leg_a)], stroke);
    painter.line_segment([pos(tip), pos(tip + leg_b)], stroke);
}

/// A glam point as an egui position.
fn pos(v: Vec2) -> egui::Pos2 {
    egui::pos2(v.x, v.y)
}

// ---- pure geometry (unit tested) ----

/// Snap `v` to the nearest multiple of `step`; a non-positive `step` (Alt = no snap) passes through.
fn snap(v: f32, step: f32) -> f32 {
    if step <= 0.0 { v } else { (v / step).round() * step }
}

/// The parameter `s` along the axis line `point + s*unit` of the point closest to the ray
/// `origin + t*dir` (`unit` and `dir` both unit length). `None` when the ray is near-parallel to the
/// axis (no well-conditioned closest point), so the caller holds rather than jumping. The standard
/// two-skew-lines solution, reused by the axis-constrained move.
fn closest_param_on_axis(origin: Vec3, dir: Vec3, point: Vec3, unit: Vec3) -> Option<f32> {
    let r = point - origin;
    let b = unit.dot(dir);
    let denom = 1.0 - b * b;
    if denom < 1e-5 {
        return None;
    }
    Some((b * dir.dot(r) - unit.dot(r)) / denom)
}

/// The world point where the ray `origin + t*dir` meets the horizontal plane `y = height`, or `None`
/// when the ray runs parallel to the plane (no crossing) or the crossing is behind the eye (`t <= 0`).
/// The ground-plane move casts the cursor ray at the selection's current Y.
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

/// Project a world point to a screen position in egui points within `rect`, or `None` when it is at or
/// behind the camera plane (`w <= 0`). The NDC-to-screen mapping is the inverse of
/// [`FlyCamera::cursor_ray`](crate::camera::FlyCamera::cursor_ray)'s (egui y runs down, NDC up), so a
/// projected tripod and a cursor ray cast back through it agree (sharp-edges 2: one cursor-to-ray
/// source).
fn world_to_screen(view_proj: Mat4, rect: egui::Rect, world: Vec3) -> Option<Vec2> {
    let clip = view_proj * world.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let x = rect.min.x + (ndc.x * 0.5 + 0.5) * rect.width();
    let y = rect.min.y + (0.5 - ndc.y * 0.5) * rect.height();
    Some(Vec2::new(x, y))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn snap_rounds_to_the_nearest_multiple_and_a_nonpositive_step_passes_through() {
        // The 1m translate grid and the 5deg rotate steps, plus the Alt = no-snap pass-through.
        assert_eq!(snap(0.4, TRANSLATE_SNAP_M), 0.0);
        assert_eq!(snap(0.6, TRANSLATE_SNAP_M), 1.0);
        assert_eq!(snap(-1.4, TRANSLATE_SNAP_M), -1.0);
        assert_eq!(snap(7.0, ROTATE_SNAP_DEG), 5.0);
        assert_eq!(snap(8.0, ROTATE_SNAP_DEG), 10.0);
        assert_eq!(snap(3.3, 0.0), 3.3, "a non-positive step is a pass-through (Alt disables snap)");
    }

    #[test]
    fn closest_param_is_the_along_axis_position_of_the_ray_crossing() {
        // The X axis through the origin, and a ray dropping straight down through x = 3: the closest
        // point on X is at parameter 3. This drives the axis-constrained move.
        let s = closest_param_on_axis(Vec3::new(3.0, 5.0, 0.0), Vec3::NEG_Y, Vec3::ZERO, Vec3::X);
        assert!((s.unwrap() - 3.0).abs() < EPS, "got {s:?}");
    }

    #[test]
    fn closest_param_is_none_when_the_ray_is_parallel_to_the_axis() {
        // A ray parallel to X has no single closest point - the helper bails so the move holds position.
        assert_eq!(closest_param_on_axis(Vec3::new(0.0, 1.0, 0.0), Vec3::X, Vec3::ZERO, Vec3::X), None);
    }

    #[test]
    fn ray_vs_ground_plane_hits_at_the_plane_height_under_the_cursor() {
        // A ray dropping straight down from (3, 5, 2) meets the plane y = 1 at (3, 1, 2) - the
        // ground-plane move places the selection under the cursor.
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

    #[test]
    fn world_to_screen_centers_a_point_on_the_camera_axis() {
        // A point straight ahead lands at the centre of the rect (offset origin and the y-flip included).
        let cam = FlyCamera { position: Vec3::new(2.0, 3.0, 4.0), yaw: 0.5, pitch: -0.2 };
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(rect.width() / rect.height(), 400.0);
        let p = world_to_screen(vp, rect, cam.position + cam.forward() * 30.0).unwrap();
        assert!((p.x - rect.center().x).abs() < 0.5, "horizontally centred: {p:?}");
        assert!((p.y - rect.center().y).abs() < 0.5, "vertically centred: {p:?}");
    }

    #[test]
    fn world_to_screen_is_none_behind_the_camera() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0 };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
        let vp = cam.view_proj(640.0 / 480.0, 200.0);
        // The camera faces -Z, so a point at +Z is behind it.
        assert_eq!(world_to_screen(vp, rect, Vec3::new(0.0, 0.0, 10.0)), None);
    }

    #[test]
    fn world_to_screen_inverts_the_cursor_ray() {
        // The draw projects the tripod to pixels; the move casts a ray back through the cursor pixel.
        // They share one mapping (sharp-edges 2), so a ray through a projected point aims back at it.
        let cam = FlyCamera { position: Vec3::new(1.0, 2.0, -3.0), yaw: 0.4, pitch: -0.3 };
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 12.0), egui::vec2(1024.0, 768.0));
        let far = 500.0;
        let vp = cam.view_proj(rect.width() / rect.height(), far);
        let world = cam.position + cam.forward() * 40.0 + cam.right() * 6.0 + cam.up() * 4.0;
        let screen = world_to_screen(vp, rect, world).unwrap();
        let pos_in_rect = screen - Vec2::new(rect.min.x, rect.min.y);
        let (origin, dir) = cam.cursor_ray(pos_in_rect, Vec2::new(rect.width(), rect.height()), far);
        assert!((dir - (world - origin).normalize()).length() < 1e-3, "the ray aims back at the point");
    }
}
