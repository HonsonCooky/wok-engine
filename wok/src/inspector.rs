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
//! Identity (Name) and the linear transform (Pos, Scale) edit through the same single-writer edit seam
//! (`crate::action`, `crate::loaded`): an edit dirties the scene and persists on Save. Name is a
//! `TextEdit` committing a rename (`SetInstanceName`) on blur or Enter; Pos and Scale are `DragValue`s
//! emitting `SetInstanceTransform` on change - the precise authoring path, exact typed or dragged values
//! with no grid snap. Rotation is a READ-ONLY orientation readout (its YXZ Euler degrees): the W / E / R
//! rotate taps (`crate::gizmo`) spin the placement's quaternion, which has no clean per-axis Euler once
//! it is compound, so the row reports orientation rather than editing it - single-axis spins read clean,
//! compound shows the true value as honest feedback. Rotation is authored via the taps, not here.

use glam::{Quat, Vec3};

use wok_scene::{InstanceId, Transform};

use crate::action::Action;
use crate::euler::euler_xyz_degrees;
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

/// The most decimal places the transform readouts show: the Pos / Scale `DragValue`s, and the read-only
/// Rot row's degrees (via [`fmt_deg`]). Caps the float noise the Euler decomposition leaves in the Rot
/// readout so it reads clean, while still showing typed precision to three places on Pos / Scale.
/// Display only - the stored transform keeps full `f32` precision.
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

            // TRANSFORM: position and scale as editable X/Y/Z DragValue triplets (an edit folds the
            // changed field into the whole transform and emits SetInstanceTransform through the same seam
            // the Name field uses), and rotation as a read-only X/Y/Z orientation readout (YXZ Euler
            // degrees) - rotation is authored via the W/E/R gizmo taps, so the inspector only reports it.
            section(ui, "TRANSFORM");
            let t = placement.transform;
            egui::Grid::new("inspector_transform").num_columns(7).spacing([6.0, 4.0]).show(ui, |ui| {
                vec3_row(ui, "Pos", id, t.translation, |v| Transform { translation: v, ..t }, actions);
                rot_row(ui, t.rotation);
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

/// The rotation row in the TRANSFORM grid: the same axis-tinted X/Y/Z triplet, but a READ-ONLY readout
/// of the placement's orientation rather than an editable field. Rotation is authored by the gizmo's
/// W / E / R taps (`crate::gizmo`), which spin the stored quaternion; a compound quaternion has no clean
/// per-axis Euler, so the inspector reports the orientation instead of editing it. The display is the
/// quaternion decomposed to `[X, Y, Z]` YXZ Euler degrees ([`euler_xyz_degrees`]), each tidied
/// ([`tidy_zero`] folds the decomposition's sub-milli noise and signed zeros to a clean `0`) and shown
/// to [`MAX_DECIMALS`] with trailing zeros trimmed ([`fmt_deg`]) - so a single-axis spin reads a clean
/// multiple of 5 and a compound one reads its true (non-round) value. Ends the grid row.
fn rot_row(ui: &mut egui::Ui, rotation: Quat) {
    let p = theme::palette(ui.ctx());
    let deg = euler_xyz_degrees(rotation).map(tidy_zero);
    ui.label(egui::RichText::new("Rot").color(p.text_dim));
    for (i, (letter, color)) in [("X", AXIS_X), ("Y", AXIS_Y), ("Z", AXIS_Z)].iter().enumerate() {
        ui.label(egui::RichText::new(*letter).strong().color(*color));
        ui.label(egui::RichText::new(fmt_deg(deg[i])).color(p.text));
    }
    ui.end_row();
}

/// Tidy a rotation degree for display: a magnitude below a thousandth (the float noise a `Quat -> Euler`
/// decomposition leaves, and a signed zero) reads as a clean positive `0.0`. Applied only to the
/// canonical decomposition that seeds the Rot fields, never to a value mid-edit, so it cleans the
/// readout without ever snapping what the user is dragging or typing. (The linear Pos / Scale fields
/// store their values cleanly and need no such tidy.)
fn tidy_zero(v: f32) -> f32 {
    if v.abs() < 1e-3 { 0.0 } else { v }
}

/// Format a rotation degree for the read-only Rot readout: up to [`MAX_DECIMALS`] places with trailing
/// zeros (and a bare decimal point) trimmed, so a single-axis spin reads `45` rather than `45.000`
/// while a compound one keeps its true `13.752`. Pairs with [`tidy_zero`], which has already folded
/// near-zero noise to a clean `0`. An integer's own zeros sit left of the point, so the trim leaves them.
fn fmt_deg(v: f32) -> String {
    let s = format!("{:.*}", MAX_DECIMALS, v);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
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
    fn tidy_zero_reads_noise_and_signed_zero_as_clean_zero() {
        // The Rot seed runs through this so the decomposition's float noise and a signed zero read as a
        // clean positive 0.0, while real angles pass through. Bit-compared on the zeros so the
        // positive-zero is exact (a plain `== 0.0` would also accept `-0.0`).
        assert_eq!(tidy_zero(-0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(tidy_zero(0.0004).to_bits(), 0.0_f32.to_bits());
        approx([tidy_zero(45.0), tidy_zero(-3.0), tidy_zero(0.0)], [45.0, -3.0, 0.0]);
    }

    #[test]
    fn fmt_deg_trims_to_a_clean_readout() {
        // The read-only Rot readout: a clean spin drops its trailing zeros, a compound one keeps its
        // value to MAX_DECIMALS, an integer keeps its own zeros, and a tidied zero reads "0".
        assert_eq!(fmt_deg(45.0), "45");
        assert_eq!(fmt_deg(-3.0), "-3");
        assert_eq!(fmt_deg(13.752), "13.752");
        assert_eq!(fmt_deg(13.75), "13.75");
        assert_eq!(fmt_deg(100.0), "100");
        assert_eq!(fmt_deg(0.0), "0");
    }
}
