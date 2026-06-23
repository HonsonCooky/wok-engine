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
//! Read-only this bite: every field is static text, not a `DragValue` or `TextEdit`. Editing (rename,
//! transform entry) plus the working-copy / dirty / Save machinery is the next bite - shipping editable
//! fields without persistence would silently lose edits.

use glam::{EulerRot, Quat};

use crate::loaded::LoadedScene;
use crate::model::Model;
use crate::theme;

/// The inspector window's content width in points. Fixed (the window does not resize this bite) so the
/// readout reads as a steady panel and the boundary line wraps to a known column rather than stretching
/// the window to its full length.
const WIDTH: f32 = 232.0;

/// The inspector's inset from the editor-area edge in points, applied at the top-right corner it anchors
/// to, so it floats clear of the viewport edge rather than flush against it.
const MARGIN: f32 = 12.0;

/// The axis tint colours (handoff view 1), fixed regardless of the light/dark theme because they name
/// the world axes, not chrome surfaces: X warm red, Y green, Z the blue that doubles as the dark accent.
const AXIS_X: egui::Color32 = egui::Color32::from_rgb(0xd8, 0x53, 0x4a);
const AXIS_Y: egui::Color32 = egui::Color32::from_rgb(0x5b, 0xbd, 0x5b);
const AXIS_Z: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x86, 0xd8);

/// The boundary line that closes the inspector, **verbatim** (editor-design.md / handoff view 1): the
/// editor authors space, physical properties, and identity only - never a gameplay property bag - so
/// gameplay config binds in the game's code by id or name instead.
const BOUNDARY: &str = "Gameplay config binds in code by id or name. The editor authors no property bag.";

/// Show the floating inspector for the current selection, if any. A no-op (no window) unless the
/// selection resolves to a placement in `loaded_scene`, so the window is present exactly when something
/// is selected and still on disk. `editor_rect` is the editor area (the region left after the chrome
/// panels); the window anchors to its top-right corner and is constrained to it, so it floats over the
/// viewport well and cannot be dragged out of the editor area.
pub fn floating(ctx: &egui::Context, model: &Model, loaded_scene: Option<&LoadedScene>, editor_rect: egui::Rect) {
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
                field(ui, "Name", placement.name.as_deref().unwrap_or(""));
                field(ui, "Prefab", placement.prefab.as_str());
                ui.label(egui::RichText::new("Id").color(p.text_dim));
                ui.label(egui::RichText::new(format!("{:#010x}", id.0)).monospace().color(p.text));
                ui.end_row();
            });

            // TRANSFORM: position, rotation (YXZ Euler degrees), and scale, each as an X/Y/Z triplet
            // with the axis-tinted letters.
            section(ui, "TRANSFORM");
            let t = &placement.transform;
            egui::Grid::new("inspector_transform").num_columns(7).spacing([6.0, 4.0]).show(ui, |ui| {
                axis_row(ui, "Pos", [t.translation.x, t.translation.y, t.translation.z]);
                axis_row(ui, "Rot", euler_xyz_degrees(t.rotation));
                axis_row(ui, "Scale", [t.scale.x, t.scale.y, t.scale.z]);
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

/// One transform row in the TRANSFORM grid: the dim row label (Pos / Rot / Scale), then three
/// axis-tinted letter + value pairs across the grid's aligned columns. Ends the grid row.
fn axis_row(ui: &mut egui::Ui, label: &str, values: [f32; 3]) {
    let p = theme::palette(ui.ctx());
    ui.label(egui::RichText::new(label).color(p.text_dim));
    for (i, (letter, color)) in [("X", AXIS_X), ("Y", AXIS_Y), ("Z", AXIS_Z)].iter().enumerate() {
        ui.label(egui::RichText::new(*letter).strong().color(*color));
        ui.label(egui::RichText::new(fmt(values[i])).color(p.text));
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

/// Format a transform component for display: two decimals, with sub-milli magnitudes snapped to `0.00`
/// so float noise from the Euler decomposition and a signed zero both read as a clean `0.00` rather than
/// `-0.00` or `0.0001`.
fn fmt(v: f32) -> String {
    let v = if v.abs() < 1e-3 { 0.0 } else { v };
    format!("{v:.2}")
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
    fn fmt_snaps_signed_zero_and_noise_to_clean_zero() {
        assert_eq!(fmt(-0.0), "0.00");
        assert_eq!(fmt(0.0001), "0.00");
        assert_eq!(fmt(12.0), "12.00");
        assert_eq!(fmt(-3.0), "-3.00");
    }
}
