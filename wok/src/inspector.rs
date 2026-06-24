//! The floating inspector: a conditional, read-only readout of the selected placement.
//!
//! The editor's floating layer (editor-design.md: "scoped to the editor area and clipped to it"): an
//! `egui::Window` constrained to the editor area, present **only** when a selection resolves to a
//! placement in the loaded scene. It reads `model.shell.selection()`, looks the placement up through
//! `LoadedScene::placement`, and shows it; an empty or unresolvable selection shows nothing (no window).
//!
//! What it shows, per the placement boundary (editor-design.md): the three things a placement authors -
//! identity (name, prefab ref, instance id) and the transform (position, rotation, scale). It does
//! **not** show a PHYSICAL section: `is_hitbox` / `is_visible` and the surface tag are prefab-level, not
//! per-placement, so they belong to the prefab editor, not here. It closes with the boundary line that
//! states why there is no general property bag.
//!
//! Both identity and transform are editable through the same single-writer edit seam (`crate::action`,
//! `crate::loaded`): an edit dirties the scene and persists on Save. Name is a `TextEdit` committing a
//! rename (`SetInstanceName`) on blur or Enter. Pos / Rot / Scale are `DragValue`s emitting
//! `SetInstanceTransform` on change - the precise authoring path: exact typed or dragged values with no
//! grid snap (the 1m / 5deg snapping is the future viewport-gizmo bite). Rotation edits through a held
//! Euler scratch so adjusting one axis never scrambles the other two (see `rot_row`).

use glam::{EulerRot, Quat, Vec3};

use wok_scene::{InstanceId, Transform};

use crate::action::Action;
use crate::loaded::LoadedScene;
use crate::model::Model;
use crate::theme;

/// The inspector window's content width in points. Fixed (the window does not resize this bite) so the
/// readout reads as a steady panel and the boundary line wraps to a known column rather than stretching
/// the window to its full length.
const WIDTH: f32 = 232.0;

/// The Name `TextEdit`'s width in points: it fills the IDENTITY grid's right column, leaving the dim
/// "Name" label its own column and a little slack to the inspector's edge.
const NAME_FIELD_WIDTH: f32 = 160.0;

/// The inspector's inset from the editor-area edge in points, applied at the top-right corner it anchors
/// to, so it floats clear of the viewport edge rather than flush against it.
const MARGIN: f32 = 12.0;

/// The axis tint colours (handoff view 1), fixed regardless of the light/dark theme because they name
/// the world axes, not chrome surfaces: X warm red, Y green, Z the blue that doubles as the dark accent.
const AXIS_X: egui::Color32 = egui::Color32::from_rgb(0xd8, 0x53, 0x4a);
const AXIS_Y: egui::Color32 = egui::Color32::from_rgb(0x5b, 0xbd, 0x5b);
const AXIS_Z: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x86, 0xd8);

/// `DragValue` drag sensitivity for the linear transform fields (Pos and Scale), in units per point of
/// pointer travel. Fine on purpose: the inspector is the precise path, so a drag nudges by hundredths
/// and a typed value is the way to jump far. No grid snap here (the 1m / 5deg snapping is the future
/// viewport-gizmo bite).
const LINEAR_DRAG_SPEED: f64 = 0.01;

/// `DragValue` drag sensitivity for the rotation fields, in degrees per point of pointer travel. A
/// touch coarser than the linear fields so a drag sweeps a usable angle, still fine enough to settle on
/// a precise value; typed entry covers the rest.
const ANGULAR_DRAG_SPEED: f64 = 0.25;

/// The most decimal places any transform `DragValue` shows. Caps the float noise the Euler
/// decomposition leaves in the rotation fields (and a signed zero) so they read as a clean `0`, while
/// still showing typed precision to three places. Display only - the bound value keeps full `f32`
/// precision, so this never snaps the stored transform.
const MAX_DECIMALS: usize = 3;

/// The boundary line that closes the inspector, **verbatim** (editor-design.md / handoff view 1): the
/// editor authors space, physical properties, and identity only - never a gameplay property bag - so
/// gameplay config binds in the game's code by id or name instead.
const BOUNDARY: &str = "Gameplay config binds in code by id or name. The editor authors no property bag.";

/// Show the floating inspector for the current selection, if any. A no-op (no window) unless the
/// selection resolves to a placement in `loaded_scene`, so the window is present exactly when something
/// is selected and still on disk. `editor_rect` is the editor area (the region left after the chrome
/// panels); the window anchors to its top-right corner and is constrained to it, so it floats over the
/// viewport well and cannot be dragged out of the editor area.
pub fn floating(
    ctx: &egui::Context,
    model: &Model,
    loaded_scene: Option<&LoadedScene>,
    editor_rect: egui::Rect,
    actions: &mut Vec<Action>,
) {
    let Some(id) = model.shell.selection() else { return };
    let Some(loaded) = loaded_scene else { return };
    let Some(placement) = loaded.placement(id) else { return };
    let p = theme::palette(ctx);

    egui::Window::new("Inspector")
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(editor_rect.right_top() + egui::vec2(-MARGIN, MARGIN))
        .constrain_to(editor_rect)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);

            // IDENTITY: the name (author-given or blank), the prefab reference, and the stable instance
            // id (mono, as a 0x handle - the same id gameplay binds against).
            section(ui, "IDENTITY");
            egui::Grid::new("inspector_identity").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                // Name is editable: the dim label in the left column, a TextEdit in the right that
                // commits a rename on blur or Enter. Prefab and Id stay read-only.
                ui.label(egui::RichText::new("Name").color(p.text_dim));
                name_edit(ui, id, placement.name.as_deref().unwrap_or(""), actions);
                ui.end_row();
                field(ui, "Prefab", placement.prefab.as_str());
                ui.label(egui::RichText::new("Id").color(p.text_dim));
                ui.label(egui::RichText::new(format!("{:#010x}", id.0)).monospace().color(p.text));
                ui.end_row();
            });

            // TRANSFORM: position, rotation (YXZ Euler degrees), and scale, each as an X/Y/Z triplet of
            // editable DragValues with the axis-tinted letters. An edit folds the changed field into the
            // whole transform and emits SetInstanceTransform, routed through the same edit seam the Name
            // field uses.
            section(ui, "TRANSFORM");
            let t = placement.transform;
            egui::Grid::new("inspector_transform").num_columns(7).spacing([6.0, 4.0]).show(ui, |ui| {
                vec3_row(ui, "Pos", id, t.translation, |v| Transform { translation: v, ..t }, actions);
                rot_row(ui, id, t, actions);
                vec3_row(ui, "Scale", id, t.scale, |v| Transform { scale: v, ..t }, actions);
            });

            ui.add_space(8.0);
            ui.label(egui::RichText::new(BOUNDARY).small().italics().color(p.text_dim));
        });
}

/// A section header: a little space, then the title in dim, small, strong caps - the handoff's section
/// label (`text_dim`). Used for IDENTITY and TRANSFORM.
fn section(ui: &mut egui::Ui, title: &str) {
    let dim = theme::palette(ui.ctx()).text_dim;
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).small().strong().color(dim));
    ui.add_space(2.0);
}

/// One label/value identity row in the IDENTITY grid: the dim field label, then the value in primary
/// text. Ends the grid row.
fn field(ui: &mut egui::Ui, label: &str, value: &str) {
    let p = theme::palette(ui.ctx());
    ui.label(egui::RichText::new(label).color(p.text_dim));
    ui.label(egui::RichText::new(value).color(p.text));
    ui.end_row();
}

/// The editable Name field: a singleline `TextEdit` in the IDENTITY grid's right column that commits a
/// rename on blur or Enter ([`Action::SetInstanceName`]). The in-progress text is held in egui temp
/// memory keyed by the instance id, so typing is not reset to the stored name each frame and switching
/// selection never carries a stale buffer over; on commit the action is emitted and the scratch cleared,
/// so the next frame reseeds from the (now updated) placement name. An untouched field commits its
/// unchanged value, which the handler treats as a no-op, so focusing and blurring never dirties.
fn name_edit(ui: &mut egui::Ui, id: InstanceId, current: &str, actions: &mut Vec<Action>) {
    let scratch = egui::Id::new(("inspector_name_edit", id.0));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(scratch)).unwrap_or_else(|| current.to_owned());
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(NAME_FIELD_WIDTH));
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(scratch, text.clone()));
    }
    if response.lost_focus() {
        actions.push(Action::SetInstanceName(id, text.clone()));
        ui.data_mut(|d| d.remove::<String>(scratch));
    }
}

/// One linear transform row (Pos or Scale) in the TRANSFORM grid: the dim row label, then three
/// axis-tinted letter + `DragValue` pairs over the components of `value`. On any field's change it folds
/// the edited vector back into the whole transform through `rebuild` and emits a single
/// [`Action::SetInstanceTransform`]; an untouched row emits nothing. The values bind to the stored
/// component each frame (no scratch needed - a linear component has one display), so a live drag tracks
/// the placement as the frame loop applies it. Ends the grid row.
fn vec3_row(
    ui: &mut egui::Ui,
    label: &str,
    id: InstanceId,
    value: Vec3,
    rebuild: impl Fn(Vec3) -> Transform,
    actions: &mut Vec<Action>,
) {
    let p = theme::palette(ui.ctx());
    ui.label(egui::RichText::new(label).color(p.text_dim));
    let mut v = value;
    let mut changed = false;
    for (i, (letter, color)) in [("X", AXIS_X), ("Y", AXIS_Y), ("Z", AXIS_Z)].iter().enumerate() {
        ui.label(egui::RichText::new(*letter).strong().color(*color));
        let dv = egui::DragValue::new(&mut v[i]).speed(LINEAR_DRAG_SPEED).max_decimals(MAX_DECIMALS);
        changed |= ui.add(dv).changed();
    }
    if changed {
        actions.push(Action::SetInstanceTransform(id, rebuild(v)));
    }
    ui.end_row();
}

/// The rotation row in the TRANSFORM grid: the same axis-tinted X/Y/Z triplet, but editing a quaternion
/// through Euler degrees. The display is `Quat -> YXZ Euler degrees` ([`euler_xyz_degrees`]); an edit
/// goes back `Euler -> Quat` ([`quat_from_euler_xyz_degrees`]). A quaternion has many Euler
/// decompositions, and recomposing then re-decomposing only agrees within the principal range, so to
/// stop an edit from scrambling the untouched axes the three degree values live in an egui-temp scratch
/// keyed by the instance id - like the Name field. While a Rot field is being edited the scratch is both
/// the display and the edit source: each change updates it and emits [`Action::SetInstanceTransform`]
/// with the scratch recomposed to a quaternion. When no Rot field is active the scratch is dropped, so
/// the next frame the display reverts to the canonical decomposition of the stored quaternion (on blur,
/// the values read clean). Ends the grid row.
fn rot_row(ui: &mut egui::Ui, id: InstanceId, t: Transform, actions: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());
    let scratch = egui::Id::new(("inspector_rot_edit", id.0));
    // Display/edit source: the scratch while mid-edit, else the stored quaternion decomposed. The
    // canonical decomposition is tidied (its sub-milli noise and signed zeros read as a clean 0.0); the
    // scratch is left raw, so a tidy never touches a value the user is dragging or typing.
    let mut deg = ui
        .data_mut(|d| d.get_temp::<[f32; 3]>(scratch))
        .unwrap_or_else(|| euler_xyz_degrees(t.rotation).map(tidy_zero));

    ui.label(egui::RichText::new("Rot").color(p.text_dim));
    let mut changed = false;
    let mut active = false;
    for (i, (letter, color)) in [("X", AXIS_X), ("Y", AXIS_Y), ("Z", AXIS_Z)].iter().enumerate() {
        ui.label(egui::RichText::new(*letter).strong().color(*color));
        let dv = egui::DragValue::new(&mut deg[i]).speed(ANGULAR_DRAG_SPEED).max_decimals(MAX_DECIMALS);
        let response = ui.add(dv);
        changed |= response.changed();
        active |= response.has_focus() || response.dragged();
    }
    if changed {
        // Hold the edited degrees so next frame's display does not re-derive (and possibly scramble)
        // them, and emit the recomposed rotation folded into the whole transform.
        ui.data_mut(|d| d.insert_temp(scratch, deg));
        let rotation = quat_from_euler_xyz_degrees(deg);
        actions.push(Action::SetInstanceTransform(id, Transform { rotation, ..t }));
    } else if !active {
        // No Rot field is being edited: drop the scratch so the display reverts to the canonical
        // decomposition of the stored quaternion next frame.
        ui.data_mut(|d| d.remove::<[f32; 3]>(scratch));
    }
    ui.end_row();
}

/// The selected placement's rotation as `[X, Y, Z]` Euler degrees, decomposed in the editor's `YXZ`
/// order (the convention lifted from the prior editor's inspector): `to_euler(YXZ)` yields
/// `(yaw, pitch, roll)` = rotation about (Y, X, Z), which this reorders to the X/Y/Z the inspector shows
/// and converts to degrees. Pure, so the axis mapping is unit-tested directly.
fn euler_xyz_degrees(rotation: Quat) -> [f32; 3] {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    [pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()]
}

/// The inverse of [`euler_xyz_degrees`]: rebuild a quaternion from `[X, Y, Z]` Euler degrees in the
/// editor's `YXZ` order. The display order is `[pitch, yaw, roll]` (rotation about X, Y, Z), so this
/// feeds them back as `from_euler(YXZ, yaw, pitch, roll)` in radians - exactly undoing the
/// `to_euler(YXZ)` the display does. Pure, so the round trip is unit-tested directly.
fn quat_from_euler_xyz_degrees(xyz: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::YXZ, xyz[1].to_radians(), xyz[0].to_radians(), xyz[2].to_radians())
}

/// Tidy a rotation degree for display: a magnitude below a thousandth (the float noise a `Quat -> Euler`
/// decomposition leaves, and a signed zero) reads as a clean positive `0.0`. Applied only to the
/// canonical decomposition that seeds the Rot fields, never to a value mid-edit, so it cleans the
/// readout without ever snapping what the user is dragging or typing. (The linear Pos / Scale fields
/// store their values cleanly and need no such tidy.)
fn tidy_zero(v: f32) -> f32 {
    if v.abs() < 1e-3 { 0.0 } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: [f32; 3], expected: [f32; 3]) {
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!((a - e).abs() < 1e-3, "got {actual:?}, expected {expected:?}");
        }
    }

    #[test]
    fn euler_xyz_degrees_maps_each_axis_to_its_field() {
        // A pure rotation about one axis lands wholly in that axis's field and nowhere else, which pins
        // the YXZ -> X/Y/Z reordering: X reads pitch, Y reads yaw, Z reads roll.
        approx(euler_xyz_degrees(Quat::from_rotation_x(30.0_f32.to_radians())), [30.0, 0.0, 0.0]);
        approx(euler_xyz_degrees(Quat::from_rotation_y(45.0_f32.to_radians())), [0.0, 45.0, 0.0]);
        approx(euler_xyz_degrees(Quat::from_rotation_z(60.0_f32.to_radians())), [0.0, 0.0, 60.0]);
    }

    #[test]
    fn euler_xyz_degrees_of_identity_is_zero() {
        approx(euler_xyz_degrees(Quat::IDENTITY), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn quat_from_euler_degrees_inverts_the_display_decomposition() {
        // The Rot field edits by recomposing the held degrees into a quaternion; that must be the exact
        // inverse of the Quat -> Euler display, or even a no-op edit would drift the rotation. Holds for
        // each single axis and a combined rotation, all within the principal YXZ range (pitch in
        // (-90, 90)).
        for e in [[0.0, 0.0, 0.0], [30.0, 0.0, 0.0], [0.0, 45.0, 0.0], [0.0, 0.0, 60.0], [10.0, 20.0, 30.0]] {
            approx(euler_xyz_degrees(quat_from_euler_xyz_degrees(e)), e);
        }
    }

    #[test]
    fn editing_one_euler_axis_leaves_the_others_intact() {
        // Why the Rot fields edit through a held [X, Y, Z] scratch rather than the stored quaternion:
        // changing one axis, recomposing to a quaternion, then re-decomposing returns the other two axes
        // unscrambled. Each axis in turn is set to a fresh value over a non-trivial starting rotation;
        // the round trip pins exactly the edited triple, so the scratch keeps an edit local to its axis.
        let start = [10.0, 20.0, 30.0];
        for axis in 0..3 {
            let mut edited = start;
            edited[axis] = 55.0;
            approx(euler_xyz_degrees(quat_from_euler_xyz_degrees(edited)), edited);
        }
    }

    #[test]
    fn tidy_zero_reads_noise_and_signed_zero_as_clean_zero() {
        // The Rot seed runs through this so the decomposition's float noise and a signed zero read as a
        // clean positive 0.0, while real angles pass through. Bit-compared on the zeros so the
        // positive-zero is exact (a plain `== 0.0` would also accept `-0.0`).
        assert_eq!(tidy_zero(-0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(tidy_zero(0.0004).to_bits(), 0.0_f32.to_bits());
        approx([tidy_zero(45.0), tidy_zero(-3.0), tidy_zero(0.0)], [45.0, -3.0, 0.0]);
    }
}
