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
//! emitting `SetInstanceTransform` on change - the precise authoring path. The TRANSFORM section is laid
//! out for a steady panel: Position is three even-spaced editable X/Y/Z cells; Scale is a single value
//! when uniform (the common case, editing all three together) and an even X/Y/Z triplet otherwise; and
//! every number is fixed at two decimals in a monospace cell, so the panel never resizes as values
//! change. Rotation is a READ-ONLY axis-angle readout (`<angle>deg about (x, y, z)`): the W / E / R
//! rotate taps (`crate::gizmo`) spin the placement's quaternion, which axis-angle shows readably (the
//! Euler readout it replaces was lossy and ambiguous, the source of the messy numbers) - a single-axis
//! spin reads a clean angle, a compound one reads honestly. Rotation is authored via the taps, not here.
//! Rotation and Scale each carry a small reset button at the row's right edge (to identity / one) - the
//! one way back once a relative spin has compounded; Position has none (it is moved or typed).

use glam::{Quat, Vec3};

use wok_scene::{InstanceId, Transform};

use crate::action::Action;
use crate::icons;
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

/// The axis letters and their tints, in X/Y/Z order - the per-component cells of the Pos and Scale rows.
const AXES: [(&str, egui::Color32); 3] = [("X", AXIS_X), ("Y", AXIS_Y), ("Z", AXIS_Z)];

/// `DragValue` drag sensitivity for the linear transform fields (Pos and Scale), in units per point of
/// pointer travel. Fine on purpose: the inspector is the precise path, so a drag nudges by hundredths
/// and a typed value is the way to jump far. No grid snap here (the 1m / 5deg snapping is the gizmo's).
const LINEAR_DRAG_SPEED: f64 = 0.01;

/// Decimal places for every TRANSFORM number, fixed (not a max), so the panel width never changes as
/// values change. Display only - the stored transform keeps full `f32` precision.
const DECIMALS: usize = 2;

/// The leading-label width (points) of a TRANSFORM row, so the value cells line up across Pos / Rot /
/// Scale whatever the label's length.
const ROW_LABEL_WIDTH: f32 = 38.0;

/// The width (points) of the reset-button cell at a TRANSFORM row's right edge. Reserved on every row
/// (empty on Position, which has no reset) so the value cells line up whether or not the row resets.
const RESET_CELL: f32 = 16.0;

/// Half the smallest 2dp step: a value within this of zero rounds to `0.00`, so [`fmt2`] folds it to a
/// clean unsigned zero (no stray `-0.00`) and the axis-angle readout treats it as identity.
const ZERO_FOLD: f32 = 0.005;

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

            // TRANSFORM: Position (even editable X/Y/Z cells) and Scale (one value when uniform, else the
            // X/Y/Z triplet) commit through the edit seam the Name field uses; Rotation is a read-only
            // axis-angle readout (authored via the W/E/R gizmo taps). Rotation and Scale carry a reset
            // button at the right edge (identity / one); Position has none (it is moved or typed, never
            // reset to the world origin). Each row reserves the same label and reset cells, so the value
            // columns line up and the panel does not resize as values change.
            section(ui, "TRANSFORM");
            let t = placement.transform;
            pos_row(ui, id, t, actions);
            rot_row(ui, id, t, actions);
            scale_row(ui, id, t, actions);

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

/// One TRANSFORM row's leading label (Pos / Rot / Scale): a dim cell of fixed width, so the value cells
/// line up across the rows whatever the label's length.
fn row_label(ui: &mut egui::Ui, label: &str) {
    let dim = theme::palette(ui.ctx()).text_dim;
    let size = egui::vec2(ROW_LABEL_WIDTH, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(label).color(dim));
    });
}

/// Lay out a vector's three components as even-width editable cells across the row's remaining width -
/// each an axis-tinted letter and a monospace fixed-2dp `DragValue` hugging it - and return the edited
/// vector with whether any cell changed. Even cells (and fixed decimals) keep the panel width steady as
/// the values change. Shared by the Pos row and the non-uniform Scale row.
fn axis_cells(ui: &mut egui::Ui, value: Vec3) -> (Vec3, bool) {
    let gap = ui.spacing().item_spacing.x;
    let size = egui::vec2((ui.available_width() - 2.0 * gap) / 3.0, ui.spacing().interact_size.y);
    let mut v = value;
    let mut changed = false;
    for (i, (letter, color)) in AXES.iter().enumerate() {
        ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(*letter).strong().color(*color));
            ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
            let dv = egui::DragValue::new(&mut v[i]).speed(LINEAR_DRAG_SPEED).fixed_decimals(DECIMALS);
            changed |= ui.add(dv).changed();
        });
    }
    (v, changed)
}

/// Lay out one TRANSFORM row: the fixed-width dim label, the value area (a fixed-width region the
/// `value` closure fills, so every row's values line up), and a fixed reset cell at the right edge. The
/// `value` closure returns the edit it made (if any); `reset` is `Some((tooltip, action))` for a row
/// with a reset button, `None` for Position (its reset cell stays empty so the columns still align).
/// At most one of the value edit and the reset is pushed per frame.
fn transform_row(
    ui: &mut egui::Ui,
    label: &str,
    value: impl FnOnce(&mut egui::Ui) -> Option<Action>,
    reset: Option<(&str, Action)>,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        row_label(ui, label);
        let h = ui.spacing().interact_size.y;
        let value_w = (ui.available_width() - RESET_CELL - ui.spacing().item_spacing.x).max(0.0);
        let edit = ui
            .allocate_ui_with_layout(egui::vec2(value_w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // Reserve the full value width even when the content is narrow (a uniform Scale is just
                // one number), so the reset cell - and the value cells - line up across every row.
                ui.set_min_width(value_w);
                value(ui)
            })
            .inner;
        if let Some(action) = edit {
            actions.push(action);
        }
        match reset {
            Some((tooltip, action)) if reset_button(ui, tooltip) => actions.push(action),
            // No reset (Position), or the button was not clicked: keep the cell's width reserved so the
            // value columns line up across every row.
            Some(_) => {}
            None => {
                ui.allocate_exact_size(egui::vec2(RESET_CELL, h), egui::Sense::hover());
            }
        }
    });
}

/// A small reset glyph button at a row's right edge: a dim restore icon that brightens on hover, with
/// `tooltip`, in a fixed [`RESET_CELL`] cell. Returns whether it was clicked. Resetting through the same
/// [`Action::SetInstanceTransform`] seam means an already-default value is a clean no-op (the loaded
/// scene no-ops an unchanged transform), so a second click never dirties.
fn reset_button(ui: &mut egui::Ui, tooltip: &str) -> bool {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(RESET_CELL, ui.spacing().interact_size.y), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tooltip);
    let color = if response.hovered() { p.text } else { p.text_dim };
    icons::paint(ui.painter(), rect, icons::RESET, color);
    response.clicked()
}

/// The Position row: the three translation components as even editable cells, committed through the edit
/// seam on any change. No reset - position is moved or typed, not reset to the world origin.
fn pos_row(ui: &mut egui::Ui, id: InstanceId, t: Transform, actions: &mut Vec<Action>) {
    transform_row(
        ui,
        "Pos",
        |ui| {
            let (v, changed) = axis_cells(ui, t.translation);
            changed.then_some(Action::SetInstanceTransform(id, Transform { translation: v, ..t }))
        },
        None,
        actions,
    );
}

/// The Rotation row: a READ-ONLY axis-angle readout (authored by the gizmo's W / E / R taps), with a
/// reset button to clear it to identity. Axis-angle, not Euler, because Euler is lossy and ambiguous
/// (the source of the old messy numbers); a single-axis spin reads a clean angle about a unit axis, a
/// compound one reads honestly. Reset is the only way back to no-rotation once a relative spin has
/// compounded.
fn rot_row(ui: &mut egui::Ui, id: InstanceId, t: Transform, actions: &mut Vec<Action>) {
    transform_row(
        ui,
        "Rot",
        |ui| {
            rot_readout(ui, t.rotation);
            None
        },
        Some(("Reset rotation", Action::SetInstanceTransform(id, reset_rotation(t)))),
        actions,
    );
}

/// The Scale row: a single editable value when the scale is uniform (the common case - editing it sets
/// all three components together), else the even X/Y/Z editable cells, with a reset button to one.
fn scale_row(ui: &mut egui::Ui, id: InstanceId, t: Transform, actions: &mut Vec<Action>) {
    transform_row(
        ui,
        "Scale",
        |ui| scale_value(ui, id, t),
        Some(("Reset scale", Action::SetInstanceTransform(id, reset_scale(t)))),
        actions,
    );
}

/// The Scale row's value area: a single monospace 2dp `DragValue` when the scale is uniform (editing it
/// sets all three components via `splat`), else the even X/Y/Z editable cells. Returns the edit it made.
fn scale_value(ui: &mut egui::Ui, id: InstanceId, t: Transform) -> Option<Action> {
    if is_uniform(t.scale) {
        let mut s = t.scale.x;
        let changed = ui
            .scope(|ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                let dv = egui::DragValue::new(&mut s).speed(LINEAR_DRAG_SPEED).fixed_decimals(DECIMALS);
                ui.add(dv).changed()
            })
            .inner;
        changed.then_some(Action::SetInstanceTransform(id, Transform { scale: Vec3::splat(s), ..t }))
    } else {
        let (v, changed) = axis_cells(ui, t.scale);
        changed.then_some(Action::SetInstanceTransform(id, Transform { scale: v, ..t }))
    }
}

/// Paint the two-line axis-angle readout (the angle, then the axis tuple) into the Rotation row's value
/// area. Dim, monospace; the break before the tuple keeps it whole and the panel width steady.
fn rot_readout(ui: &mut egui::Ui, rotation: Quat) {
    let dim = theme::palette(ui.ctx()).text_dim;
    let text = axis_angle_text(rotation);
    ui.vertical(|ui| {
        let (angle, axis) = text.split_once(" (").unwrap_or((text.as_str(), ""));
        ui.label(egui::RichText::new(angle).monospace().color(dim));
        if !axis.is_empty() {
            ui.label(egui::RichText::new(format!("({axis}")).monospace().color(dim));
        }
    });
}

/// The transform that resets rotation to identity (no rotation), keeping position and scale. The
/// Rotation row's reset emits this; pure, so the reset value is unit tested.
fn reset_rotation(t: Transform) -> Transform {
    Transform { rotation: Quat::IDENTITY, ..t }
}

/// The transform that resets scale to one (uniform 1), keeping position and rotation. The Scale row's
/// reset emits this; pure, so the reset value is unit tested.
fn reset_scale(t: Transform) -> Transform {
    Transform { scale: Vec3::ONE, ..t }
}

/// Whether all three components are equal - the uniform-scale case the Scale row shows as one value.
/// Exact equality is intended: a uniform scale is authored via one value (set as `splat`), so its
/// components are bit-equal; a scale that merely sits close is genuinely non-uniform and shows the
/// triplet, never silently collapsed to one editable value that would rewrite the other two.
#[allow(clippy::float_cmp)]
fn is_uniform(v: Vec3) -> bool {
    v.x == v.y && v.y == v.z
}

/// Format a transform number at fixed [`DECIMALS`] places, folding a value that rounds to zero to a
/// clean unsigned `0` (so a near-zero never reads `-0.00`). Display only - the stored f32 keeps full
/// precision. Used by the read-only axis-angle readout; the editable `DragValue`s fix their own decimals.
fn fmt2(v: f32) -> String {
    let v = if v.abs() < ZERO_FOLD { 0.0 } else { v };
    format!("{:.*}", DECIMALS, v)
}

/// The read-only rotation readout: the placement's quaternion as `<angle>deg about (x, y, z)` at fixed
/// 2dp, the axis being the unit rotation axis. A near-zero angle (identity, or `to_axis_angle`'s
/// degenerate fallback) reads `0.00deg about (0.00, 0.00, 0.00)` rather than an arbitrary axis. Pure, so
/// the format is unit tested.
fn axis_angle_text(rotation: Quat) -> String {
    let (axis, angle) = rotation.to_axis_angle();
    let deg = angle.to_degrees();
    let axis = if deg.abs() < ZERO_FOLD { Vec3::ZERO } else { axis };
    format!("{}deg about ({}, {}, {})", fmt2(deg), fmt2(axis.x), fmt2(axis.y), fmt2(axis.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt2_formats_at_two_decimals_and_folds_negative_zero() {
        // Every TRANSFORM number is fixed at 2dp; a value that rounds to zero reads a clean unsigned
        // 0.00 (never a stray -0.00), and other values round to 2 places.
        assert_eq!(fmt2(1.5), "1.50");
        assert_eq!(fmt2(-3.0), "-3.00");
        assert_eq!(fmt2(12.0), "12.00");
        assert_eq!(fmt2(0.0), "0.00");
        assert_eq!(fmt2(-0.001), "0.00", "a near-zero folds, so it never reads -0.00");
        assert_eq!(fmt2(0.126), "0.13", "rounds to 2dp");
    }

    #[test]
    fn axis_angle_text_reads_the_quaternion_readably() {
        // The read-only rotation readout: a single-axis spin reads a clean angle about a unit axis, and
        // identity reads a zero angle about a zero axis (not to_axis_angle's arbitrary fallback).
        assert_eq!(axis_angle_text(Quat::from_rotation_y(45.0_f32.to_radians())), "45.00deg about (0.00, 1.00, 0.00)");
        assert_eq!(axis_angle_text(Quat::from_rotation_x(90.0_f32.to_radians())), "90.00deg about (1.00, 0.00, 0.00)");
        assert_eq!(axis_angle_text(Quat::IDENTITY), "0.00deg about (0.00, 0.00, 0.00)");
    }

    #[test]
    fn is_uniform_detects_equal_scale_components() {
        // The Scale row shows one value when uniform, the X/Y/Z triplet otherwise.
        assert!(is_uniform(Vec3::splat(1.5)));
        assert!(is_uniform(Vec3::ONE));
        assert!(!is_uniform(Vec3::new(1.0, 2.0, 1.0)));
        assert!(!is_uniform(Vec3::new(1.0, 1.0, 1.5)));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn reset_clears_rotation_to_identity_and_scale_to_one_keeping_the_rest() {
        // The per-row resets carry the exact default and leave the other components, so each reset
        // touches only its own axis of the transform.
        let t = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(1.0),
            scale: Vec3::splat(2.0),
        };
        let r = reset_rotation(t);
        assert_eq!(r.rotation, Quat::IDENTITY, "rotation reset clears to identity");
        assert_eq!((r.translation, r.scale), (t.translation, t.scale), "and leaves position and scale");
        let s = reset_scale(t);
        assert_eq!(s.scale, Vec3::ONE, "scale reset clears to one");
        assert_eq!((s.translation, s.rotation), (t.translation, t.rotation), "and leaves position and rotation");
    }
}
