//! The transform gizmo: a world-axis translate handle the mouse drags, and a hold-key rotate / scale
//! fast path - the device-split manipulator over the selected placement (designs/editor-design.md,
//! Input: the mouse is coarse motion, the left hand is the operators).
//!
//! Two ways to transform, one device each, no overlapping widgets. The translate gizmo draws three
//! world-axis lines (X/Y/Z) from the selection's anchor; pressing one and dragging slides the
//! placement along that axis, snapped to the 1m grid (hold Alt for fine). Rotate and scale are a
//! deliberately widget-less hold-key path instead of a ring/box gizmo: hold `R` and move the mouse to
//! yaw about world Y (5deg snap), hold `S` to scale uniformly - less to draw, and a better fit for the
//! left-hand-keyboard / right-hand-mouse split than a third on-screen handle.
//!
//! Every mutation routes through the existing edit seam: [`update`] returns an
//! [`Action::SetInstanceTransform`](crate::action::Action) the frame loop applies through
//! `crate::action::handle`, exactly as the inspector's fields do. [`LoadedScene::set_transform`] already
//! no-ops an unchanged transform, so emitting the full transform every held frame is free; Ctrl+S
//! persists, unchanged.
//!
//! Split of concerns: [`draw`] is the screen-space overlay (called from `view::chrome` beside the
//! inspector, on the editor's floating layer), and [`update`] is the per-frame interaction (called from
//! the frame loop in `crate::main`, where the camera, the render residency, and the raw input live). A
//! press that catches a handle is consumed by the gizmo for its whole press-to-release, so a handle
//! click never also resolves to a pick or a deselect ([`Outcome::consumed_press`]); a press that misses
//! every handle leaves the click to pick as before. The pure geometry (closest point on an axis line to
//! the cursor ray, the world-to-screen projection, the grid snap) is unit tested below.

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

/// Handle length as a fraction of the distance from the camera to the anchor, so the on-screen length
/// is constant and the handles stay grabbable at any zoom (a far selection gets a longer world handle).
const HANDLE_SCREEN_FRAC: f32 = 0.15;
/// Axis line stroke width in points, and the thicker width for the axis being dragged.
const HANDLE_WIDTH: f32 = 2.5;
const HANDLE_WIDTH_ACTIVE: f32 = 4.0;
/// Arrowhead leg length (points) and half-angle (radians) of the two legs off the axis at the tip.
const ARROW_LEN: f32 = 11.0;
const ARROW_HALF_ANGLE: f32 = 0.45;
/// How near (points) the cursor must fall to a projected axis line to catch its handle on a press.
const GRAB_PX: f32 = 9.0;

/// The axis tint colours, mirroring the inspector's `AXIS_*`: fixed regardless of the light/dark theme
/// because they name the world axes, not chrome surfaces (X warm red, Y green, Z blue).
const AXIS_X: Color32 = Color32::from_rgb(0xd8, 0x53, 0x4a);
const AXIS_Y: Color32 = Color32::from_rgb(0x5b, 0xbd, 0x5b);
const AXIS_Z: Color32 = Color32::from_rgb(0x4a, 0x86, 0xd8);

/// Snap defaults (canon: 1m grid, 5deg steps). Alt disables snapping per drag.
const TRANSLATE_SNAP_M: f32 = 1.0;
const ROTATE_SNAP_DEG: f32 = 5.0;
/// Hold-key sensitivities: degrees of yaw and scale exponent per point of raw horizontal mouse motion.
const ROTATE_DEG_PER_PX: f32 = 0.5;
const SCALE_PER_PX: f32 = 0.005;

/// A world axis a translate handle moves along (world-only this bite; no local-vs-world toggle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
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

    /// The translation-vector component index the axis drives (X=0, Y=1, Z=2).
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

/// An in-progress gizmo manipulation, held across frames by the frame loop (`Option<Gizmo>` on the
/// editor). At most one is active at a time: a translate-handle drag (mouse) or a hold-key rotate /
/// scale (keyboard). Idle is the `None` outside.
pub enum Gizmo {
    /// A world-axis translate drag begun by pressing a handle; lives until the left button releases.
    Translate(TranslateDrag),
    /// A hold-key rotate or scale, live only while its letter key (`R` / `S`) is down.
    Hold(HoldDrag),
}

/// A translate drag: the axis caught, the fixed world line it slides along (captured at grab so it does
/// not chase the moving placement), the along-axis parameter where the cursor first caught it (so the
/// drag is relative, with no jump on engage), and the placement's whole transform at grab (only its
/// translation changes).
pub struct TranslateDrag {
    id: InstanceId,
    axis: Axis,
    anchor: Vec3,
    grab_param: f32,
    grab_transform: Transform,
}

/// Whether a hold-key manipulation yaws or scales.
#[derive(Clone, Copy)]
enum HoldKind {
    Rotate,
    Scale,
}

/// A hold-key rotate / scale: the placement transform when the hold began (the base the motion folds
/// into) and the accumulated raw horizontal mouse motion since (mapped to degrees or a scale factor at
/// emit time, so snapping never loses sub-step motion).
pub struct HoldDrag {
    id: InstanceId,
    kind: HoldKind,
    base: Transform,
    accum: f32,
}

/// What the gizmo draw needs from the frame loop, computed where the camera and render residency live
/// and threaded through `view::chrome` to the [`draw`] call. The anchor is resolved inside [`draw`] from
/// the same loaded scene the chrome already holds. `Copy` so it can be read out of the egui build
/// closure (typed `FnMut`) by value rather than moved.
#[derive(Clone, Copy)]
pub struct GizmoView {
    /// The viewport camera, for projecting the world-space handles to screen.
    pub camera: FlyCamera,
    /// The scene's far plane (its render distance), so the projection matches the 3D and the picking.
    pub far: f32,
    /// The axis currently being dragged, drawn highlighted; `None` when no translate drag is active.
    pub active: Option<Axis>,
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
    /// The editor-well rect (egui points) the 3D rendered into - the gizmo projects and hit-tests here.
    pub rect: egui::Rect,
    /// The pointer position in window points, or `None` when egui has no pointer this frame.
    pub cursor: Option<Vec2>,
    /// The pointer is over the viewport well under no foreground layer (the camera-lock engage gate):
    /// gates a handle press and the hold-key fast path so neither fires over a panel, menu, or the
    /// inspector.
    pub over_well: bool,
    /// No egui text field holds keyboard focus, so the hold-key letters drive the gizmo rather than
    /// typing into the inspector's Name field.
    pub keyboard_free: bool,
}

/// What [`update`] tells the frame loop: a transform edit to route through `action::handle` this frame,
/// and whether a translate-handle press is being consumed (so the loop drops this frame's
/// `ViewportClick` rather than letting a handle click pick or deselect).
#[derive(Default)]
pub struct Outcome {
    pub action: Option<Action>,
    pub consumed_press: bool,
}

/// The axis a translate drag is manipulating, for the draw highlight; `None` when idle or holding a key.
pub fn active_axis(gizmo: Option<&Gizmo>) -> Option<Axis> {
    match gizmo {
        Some(Gizmo::Translate(drag)) => Some(drag.axis),
        _ => None,
    }
}

/// Advance the gizmo one frame: continue or end an active manipulation, or engage a new one. Pure over
/// its [`Inputs`] except for the `&mut Option<Gizmo>` it owns - the camera, the model, and the disk are
/// the frame loop's. Returns the [`Outcome`] the loop applies and consults.
pub fn update(gizmo: &mut Option<Gizmo>, f: &Inputs) -> Outcome {
    // A translate drag consumes the press for its whole life, including the release frame: egui reports
    // a no-move press+release as a click, which must not also resolve to a pick.
    let consumed_press = matches!(gizmo, Some(Gizmo::Translate(_)));

    // 1) Continue or end the active manipulation.
    if let Some(active) = gizmo {
        match active {
            Gizmo::Translate(drag) => {
                if f.input.mouse_held(MouseButton::Left) {
                    return Outcome { action: drag_translate(drag, f), consumed_press };
                }
                *gizmo = None;
                return Outcome { action: None, consumed_press };
            }
            Gizmo::Hold(hold) => {
                let key = hold.kind.key();
                if f.input.char_held(key) && f.selection == Some(hold.id) && hold_allowed(f) {
                    return Outcome { action: drive_hold(hold, f), consumed_press };
                }
                *gizmo = None;
                return Outcome::default();
            }
        }
    }

    // 2) Idle: a left press over the well that catches a handle begins a translate drag, consumed from
    // frame one so even a click-without-drag on a handle never picks.
    if f.input.mouse_pressed(MouseButton::Left) && f.over_well {
        if let Some(drag) = engage_translate(f) {
            *gizmo = Some(Gizmo::Translate(drag));
            return Outcome { action: None, consumed_press: true };
        }
    }

    // 3) Idle: hold R / S (gated) begins a rotate / scale, applying this frame's motion at once so a
    // quick flick is not dropped.
    if hold_allowed(f) {
        if let Some(mut hold) = engage_hold(f) {
            let action = drive_hold(&mut hold, f);
            *gizmo = Some(Gizmo::Hold(hold));
            return Outcome { action, consumed_press: false };
        }
    }

    Outcome::default()
}

impl HoldKind {
    /// The letter key that drives this manipulation.
    fn key(self) -> char {
        match self {
            HoldKind::Rotate => 'r',
            HoldKind::Scale => 's',
        }
    }
}

/// The world-space anchor of the selected placement: its chunk origin plus its local translation (the
/// chunk origin is a pure translation, and the gizmo's axes are world axes this bite). `None` when the
/// id resolves to no placement or its chunk is absent.
fn anchor(loaded: &LoadedScene, id: InstanceId) -> Option<Vec3> {
    let placement = loaded.placement(id)?;
    let chunk = loaded.chunks().iter().find(|c| c.placements.iter().any(|p| p.instance_id == id))?;
    Some(chunk_origin(chunk.coord) + placement.transform.translation)
}

/// Catch the nearest axis handle under the cursor on a press, capturing the drag. `None` when nothing is
/// selected, the anchor is off screen, or no handle is within [`GRAB_PX`].
fn engage_translate(f: &Inputs) -> Option<TranslateDrag> {
    let id = f.selection?;
    let anchor = anchor(f.loaded, id)?;
    let cursor = f.cursor?;
    let view_proj = view_proj(f)?;
    let base = world_to_screen(view_proj, f.rect, anchor)?;
    let len = handle_len(f.camera, anchor);

    let mut best: Option<(f32, Axis)> = None;
    for axis in Axis::ALL {
        let Some(tip) = world_to_screen(view_proj, f.rect, anchor + axis.unit() * len) else { continue };
        let dist = point_segment_dist(cursor, base, tip);
        if dist <= GRAB_PX && best.is_none_or(|(best_dist, _)| dist < best_dist) {
            best = Some((dist, axis));
        }
    }
    let (_, axis) = best?;

    let (origin, dir) = cursor_ray(f, cursor);
    let grab_param = closest_param_on_axis(origin, dir, anchor, axis.unit())?;
    let grab_transform = f.loaded.placement(id)?.transform;
    Some(TranslateDrag { id, axis, anchor, grab_param, grab_transform })
}

/// Drag the selection along the captured axis: the closest point on the fixed axis line to the current
/// cursor ray gives the along-axis parameter; its delta from the grab parameter is the translation,
/// snapped to the 1m grid unless Alt is held. `None` (hold position) when the cursor is gone or the ray
/// is near-parallel to the axis (no well-defined closest point).
fn drag_translate(drag: &TranslateDrag, f: &Inputs) -> Option<Action> {
    let cursor = f.cursor?;
    let (origin, dir) = cursor_ray(f, cursor);
    let param = closest_param_on_axis(origin, dir, drag.anchor, drag.axis.unit())?;
    let mut translation = drag.grab_transform.translation + drag.axis.unit() * (param - drag.grab_param);
    if !f.input.key_held(NamedKey::Alt) {
        let i = drag.axis.index();
        translation[i] = snap(translation[i], TRANSLATE_SNAP_M);
    }
    Some(Action::SetInstanceTransform(drag.id, Transform { translation, ..drag.grab_transform }))
}

/// Begin a hold-key rotate (`R`) or scale (`S`) over the current selection, capturing the base
/// transform. `None` when neither key is held or the selection does not resolve.
fn engage_hold(f: &Inputs) -> Option<HoldDrag> {
    let kind = if f.input.char_held('r') {
        HoldKind::Rotate
    } else if f.input.char_held('s') {
        HoldKind::Scale
    } else {
        return None;
    };
    let id = f.selection?;
    let base = f.loaded.placement(id)?.transform;
    Some(HoldDrag { id, kind, base, accum: 0.0 })
}

/// Fold this frame's raw horizontal mouse motion into the hold and emit the resulting transform: a yaw
/// about world Y in 5deg steps (Alt for fine) for rotate, a uniform scale (right enlarges) for scale.
/// The angle / factor is always derived from the base plus the accumulated motion, so the emit is
/// idempotent for a steady cursor (the seam no-ops it).
fn drive_hold(hold: &mut HoldDrag, f: &Inputs) -> Option<Action> {
    hold.accum += f.input.mouse_motion.0 as f32;
    let transform = match hold.kind {
        HoldKind::Rotate => {
            let degrees = hold.accum * ROTATE_DEG_PER_PX;
            let degrees = if f.input.key_held(NamedKey::Alt) { degrees } else { snap(degrees, ROTATE_SNAP_DEG) };
            // World-Y yaw: pre-multiply so the axis is world up regardless of the placement's heading.
            let rotation = Quat::from_rotation_y(degrees.to_radians()) * hold.base.rotation;
            Transform { rotation, ..hold.base }
        }
        HoldKind::Scale => {
            // Exponential so left / right are symmetric and the factor never reaches zero.
            let factor = (hold.accum * SCALE_PER_PX).exp();
            Transform { scale: hold.base.scale * factor, ..hold.base }
        }
    };
    Some(Action::SetInstanceTransform(hold.id, transform))
}

/// Whether a hold-key manipulation may run this frame: something selected, the pointer over the well, no
/// text field focused, and no camera drag in progress (right / middle held).
fn hold_allowed(f: &Inputs) -> bool {
    f.selection.is_some()
        && f.over_well
        && f.keyboard_free
        && !f.input.mouse_held(MouseButton::Right)
        && !f.input.mouse_held(MouseButton::Middle)
}

/// The view-projection for the current well, or `None` for a degenerate (zero-area) rect.
fn view_proj(f: &Inputs) -> Option<Mat4> {
    if f.rect.width() <= 0.0 || f.rect.height() <= 0.0 {
        return None;
    }
    Some(f.camera.view_proj(f.rect.width() / f.rect.height(), f.far))
}

/// The cursor ray for a window-space cursor position, mapped against the well rect (the same source the
/// pick uses - sharp-edges 2).
fn cursor_ray(f: &Inputs, cursor: Vec2) -> (Vec3, Vec3) {
    let size = Vec2::new(f.rect.width(), f.rect.height());
    let pos_in_rect = cursor - Vec2::new(f.rect.min.x, f.rect.min.y);
    f.camera.cursor_ray(pos_in_rect, size, f.far)
}

/// The world-space handle length: a fraction of the camera-to-anchor distance, so the projected length
/// is roughly constant on screen (floored so a camera sitting on the anchor still yields a finite line).
fn handle_len(camera: &FlyCamera, anchor: Vec3) -> f32 {
    (HANDLE_SCREEN_FRAC * (anchor - camera.position).length()).max(0.01)
}

/// Draw the world-axis translate gizmo over the viewport when a placement is selected: three coloured
/// axis lines from the anchor, each tipped with an arrowhead, clipped to the editor area. It paints on
/// a Background-order layer: above the 3D well (which wok-render draws before the whole egui pass, so
/// every egui layer sits over it) yet below the floating inspector (a Middle-order window) and the menus
/// (Foreground), and the clip keeps it off the surrounding panels. A no-op unless the selection resolves
/// to a placement (the inspector's gate) and the anchor projects in front of the camera. Handle length
/// is screen-constant, so the handles stay the same size - and grabbable - at any zoom.
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
    let len = handle_len(&view.camera, anchor);

    let painter = ctx
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("translate_gizmo")))
        .with_clip_rect(rect);
    for axis in Axis::ALL {
        let Some(tip) = world_to_screen(view_proj, rect, anchor + axis.unit() * len) else { continue };
        let width = if view.active == Some(axis) { HANDLE_WIDTH_ACTIVE } else { HANDLE_WIDTH };
        let stroke = Stroke::new(width, axis.color());
        painter.line_segment([pos(base), pos(tip)], stroke);
        draw_arrowhead(&painter, base, tip, stroke);
    }
}

/// Paint a two-legged arrowhead at `tip`, opening back toward `base`, in the handle's stroke. Screen
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
/// two-skew-lines solution.
fn closest_param_on_axis(origin: Vec3, dir: Vec3, point: Vec3, unit: Vec3) -> Option<f32> {
    let r = point - origin;
    let b = unit.dot(dir);
    let denom = 1.0 - b * b;
    if denom < 1e-5 {
        return None;
    }
    Some((b * dir.dot(r) - unit.dot(r)) / denom)
}

/// Project a world point to a screen position in egui points within `rect`, or `None` when it is at or
/// behind the camera plane (`w <= 0`). The NDC-to-screen mapping is the inverse of
/// [`FlyCamera::cursor_ray`](crate::camera::FlyCamera::cursor_ray)'s (egui y runs down, NDC up), so a
/// projected handle and a cursor ray cast back through it agree (sharp-edges 2: one cursor-to-ray
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

/// The 2D distance from `p` to the segment `[a, b]` (egui points) - the cursor-to-handle hit test.
fn point_segment_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
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
        // point on X is at parameter 3.
        let s = closest_param_on_axis(Vec3::new(3.0, 5.0, 0.0), Vec3::NEG_Y, Vec3::ZERO, Vec3::X);
        assert!((s.unwrap() - 3.0).abs() < EPS, "got {s:?}");
    }

    #[test]
    fn closest_param_is_none_when_the_ray_is_parallel_to_the_axis() {
        // A ray parallel to X has no single closest point - the helper bails so the drag holds position.
        assert_eq!(closest_param_on_axis(Vec3::new(0.0, 1.0, 0.0), Vec3::X, Vec3::ZERO, Vec3::X), None);
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
        // The draw projects a handle to a pixel; the drag casts a ray back through that pixel. They
        // share one mapping (sharp-edges 2), so a ray through the projected point aims back at it.
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

    #[test]
    fn point_segment_dist_measures_to_the_nearest_point_on_the_segment() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(10.0, 0.0);
        assert!((point_segment_dist(Vec2::new(5.0, 3.0), a, b) - 3.0).abs() < EPS, "perpendicular drop");
        assert!((point_segment_dist(Vec2::new(-4.0, 0.0), a, b) - 4.0).abs() < EPS, "clamped to the near end");
        assert!((point_segment_dist(Vec2::new(13.0, 4.0), a, b) - 5.0).abs() < EPS, "clamped to the far end");
    }
}
