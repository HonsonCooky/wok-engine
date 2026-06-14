//! The floating details panel: the v1 inspector, moved out of the left panel into a
//! right-anchored window that exists exactly while something is selected.
//!
//! Existence follows selection ([`visible`], pure and tested): no selection, no window - which is
//! also how Esc closes it, since Esc deselects. Values are rebuilt from the authored form every
//! frame and committed through [`Action::Edit`] the moment a field changes, so a drag edits live
//! and the authored form stays the single source of truth, exactly as the v1 inspector did.
//!
//! Rotation is three degree fields over one documented euler order, YXZ: yaw about world Y, then
//! pitch about the yawed X, then roll about the resulting Z (glam's intrinsic `EulerRot::YXZ`).
//! The decomposition is display-only: a commit recomposes the quat from the displayed angles only
//! when a rotation field itself changed ([`committed_rotation`]), so edits to position, scale, or
//! state carry the authored quat bit for bit and can never drift a rotation through repeated
//! decompose-recompose cycles.

use glam::{Quat, Vec3};
use wok_physics::{Collider, basis_is_axis_aligned, classify_collider};
use wok_scene::{Placement, Prefab, Transform};

use crate::model::{EditorModel, Selection};
use crate::outline;
use crate::panels::Action;

/// Whether the details window exists this frame: a selection that still resolves to a placement.
/// Pure, so the panel's visibility rule is testable without a window.
pub fn visible(model: &EditorModel) -> bool {
    model.selection.primary().is_some_and(|sel| model.placement(sel).is_some())
}

/// Build the details window (or nothing, per [`visible`]).
pub fn window(ctx: &egui::Context, model: &EditorModel, actions: &mut Vec<Action>) {
    if !visible(model) {
        return;
    }
    let Some(sel) = model.selection.primary() else { return };
    let Some(placement) = model.placement(sel) else { return };
    egui::Window::new("wok_details")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
        .default_width(250.0)
        .show(ctx, |ui| body(ui, model, sel, placement, actions));
}

fn body(ui: &mut egui::Ui, model: &EditorModel, sel: Selection, placement: &Placement, actions: &mut Vec<Action>) {
    let prefab = model.prefabs.get(&placement.prefab);
    let generated = outline::generated_label(placement);

    // Header: the display name (or the generated label), with a close affordance that simply
    // deselects - the window's existence is the selection.
    ui.horizontal(|ui| {
        ui.strong(placement.name.as_deref().unwrap_or(&generated));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("x").on_hover_text("Deselect (Esc)").clicked() {
                actions.push(Action::Select(None));
            }
        });
    });
    let id_line = if placement.name.is_some() {
        format!("{generated}  (chunk {}_{})", sel.coord.x, sel.coord.z)
    } else {
        format!("chunk {}_{}", sel.coord.x, sel.coord.z)
    };
    ui.label(egui::RichText::new(id_line).small().weak());
    ui.separator();

    let authored = placement.transform;
    let mut pos = authored.translation;
    let (mut yaw_deg, mut pitch_deg, mut roll_deg) = euler_degrees(authored.rotation);
    let mut scale = authored.scale.x;
    let (mut pos_changed, mut rot_changed, mut scale_changed) = (false, false, false);

    egui::Grid::new("wok_details_grid").num_columns(2).show(ui, |ui| {
        ui.label("position");
        ui.horizontal(|ui| {
            pos_changed |= drag(ui, &mut pos.x, 0.05, "x ");
            pos_changed |= drag(ui, &mut pos.y, 0.05, "y ");
            pos_changed |= drag(ui, &mut pos.z, 0.05, "z ");
        });
        ui.end_row();
        ui.label("rotation deg");
        ui.horizontal(|ui| {
            rot_changed |= drag(ui, &mut yaw_deg, 1.0, "y ");
            rot_changed |= drag(ui, &mut pitch_deg, 1.0, "p ");
            rot_changed |= drag(ui, &mut roll_deg, 1.0, "r ");
        });
        ui.end_row();
        ui.label("scale");
        scale_changed = ui
            .add(egui::DragValue::new(&mut scale).speed(0.02).range(0.01..=f32::INFINITY))
            .changed();
        ui.end_row();
    });
    ui.label(egui::RichText::new("position is chunk-local").small().weak());

    let mut state_changed = false;
    let mut new_state = placement.state.clone();
    if let Some(prefab) = prefab {
        if prefab.states.len() > 1 {
            let current = placement.state.clone().unwrap_or_else(|| prefab.default_state.clone());
            let mut chosen = current.clone();
            egui::ComboBox::from_label("state")
                .selected_text(chosen.clone())
                .show_ui(ui, |ui| {
                    for state in &prefab.states {
                        ui.selectable_value(&mut chosen, state.name.clone(), &state.name);
                    }
                });
            if chosen != current {
                state_changed = true;
                // The default state stays implicit (state: None) so the authored file stays
                // minimal, exactly as the slicer resolves it.
                new_state = if chosen == prefab.default_state { None } else { Some(chosen) };
            }
        }
        if has_conservative_rotated_solid(prefab, placement) {
            // Surfaced where the user acts: only for rotations the colliders cannot carry exactly.
            ui.label(
                egui::RichText::new("rotated solids collide as their conservative box")
                    .small()
                    .color(egui::Color32::from_rgb(230, 160, 60)),
            );
        }
    }

    if pos_changed || rot_changed || scale_changed || state_changed {
        let transform = Transform {
            translation: pos,
            rotation: committed_rotation(authored.rotation, (yaw_deg, pitch_deg, roll_deg), rot_changed),
            scale: if scale_changed { Vec3::splat(scale) } else { authored.scale },
        };
        actions.push(Action::Edit { sel, transform, state: new_state });
    }
}

fn drag(ui: &mut egui::Ui, value: &mut f32, speed: f64, prefix: &str) -> bool {
    ui.add(egui::DragValue::new(value).speed(speed).prefix(prefix)).changed()
}

/// The angles the rotation fields show, degrees: the quat decomposed in the panel's one euler
/// order, YXZ - yaw about world Y, then pitch about the yawed X, then roll about the resulting Z
/// (glam's intrinsic `EulerRot::YXZ`, so `from_euler(YXZ, ..)` is its exact inverse). Yaw-first
/// because yaw is the rotation placements actually author most; gimbal lock (pitch at +/-90)
/// folds yaw and roll into one axis there, as any euler display must.
fn euler_degrees(rotation: Quat) -> (f32, f32, f32) {
    let (yaw, pitch, roll) = rotation.to_euler(glam::EulerRot::YXZ);
    (yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees())
}

/// The quat the displayed angles describe, recomposed in the same YXZ order [`euler_degrees`]
/// decomposes in.
fn rotation_from_degrees((yaw, pitch, roll): (f32, f32, f32)) -> Quat {
    Quat::from_euler(glam::EulerRot::YXZ, yaw.to_radians(), pitch.to_radians(), roll.to_radians())
}

/// The rotation a commit writes: recomposed from the displayed angles only when a rotation field
/// itself changed this frame, the authored quat untouched otherwise. The untouched path is the
/// load-bearing half - the euler decomposition is lossy at float precision (and degenerate at
/// gimbal lock), so a commit that recomposed on every edit would drift an authored rotation a
/// little further on each position or scale tweak.
fn committed_rotation(authored: Quat, displayed_deg: (f32, f32, f32), rot_changed: bool) -> Quat {
    if rot_changed { rotation_from_degrees(displayed_deg) } else { authored }
}

/// Does this placement have a solid shape whose rotation the colliders cannot carry exactly, so it
/// still collides as the conservative axis-aligned box? Cubes rotate honestly (Obb), spheres and
/// vertical cylinders spin onto themselves, and an axis-aligned shape's box is not widened by the
/// reduction - none of those warrant the warning. What remains is the genuinely conservative
/// fall: a rotated shape (tilted round solids, yawed capsules or planes) or a sheared matrix,
/// where the felt surface really is wider than the drawn one. `basis_is_axis_aligned` is
/// wok-physics's own tolerance, so the warning cannot disagree with classification.
fn has_conservative_rotated_solid(prefab: &Prefab, placement: &Placement) -> bool {
    let state_name = placement.state.as_deref().unwrap_or(prefab.default_state.as_str());
    let Some(state) = prefab.states.iter().find(|s| s.name == state_name) else { return false };
    let placement_mat = placement.transform.to_mat4();
    state.shapes.iter().filter(|s| s.is_hitbox).any(|shape| {
        let world = placement_mat * shape.transform.to_mat4();
        matches!(classify_collider(shape.primitive, world), Collider::Aabb(_)) && !basis_is_axis_aligned(&world)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;
    use wok_scene::{ChunkCoord, InstanceId, PrefabRef, PrefabState, Primitive, Shape};

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    // ---- visibility follows selection ----

    #[test]
    fn the_details_panel_exists_exactly_while_a_selection_resolves() {
        let mut model = sample_model();
        assert!(!visible(&model), "no selection, no panel");

        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(3) };
        model.selection.replace(sel);
        assert!(visible(&model), "a live selection shows the panel");

        model.delete(sel).unwrap();
        assert!(!visible(&model), "deleting the selection closes the panel");

        // A selection dangling at a placement that no longer exists (set before validation ran)
        // must not show a panel either.
        model.selection.replace(Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(999) });
        assert!(!visible(&model), "a dangling selection shows nothing");
    }

    // ---- euler round-trip stability ----

    /// A rotation that exercises all three axes, nowhere near gimbal lock.
    fn non_trivial() -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, 0.7, 0.4, 0.2)
    }

    #[test]
    fn edits_of_other_fields_never_touch_a_non_trivial_rotation() {
        // The decomposition trap: a commit that rebuilt the quat from the displayed euler angles
        // even when only position or scale changed would add float error on every edit. The
        // commit rule must instead carry the authored quat bit for bit through any number of
        // other-field edits.
        let authored = non_trivial();
        let mut rotation = authored;
        for _ in 0..100 {
            let shown = euler_degrees(rotation);
            rotation = committed_rotation(rotation, shown, false);
        }
        assert_eq!(rotation, authored, "bitwise: other-field edits must not touch the quat");
    }

    #[test]
    fn other_field_edits_through_the_model_keep_the_rotation_bitwise() {
        // The same property end to end: simulate the panel committing position edits through
        // edit_placement on a placement holding a non-trivial rotation.
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let authored = non_trivial();
        let start = model.placement(sel).unwrap().transform;
        model
            .edit_placement(sel, Transform { rotation: authored, ..start }, None)
            .unwrap();

        for step in 0..50 {
            let current = model.placement(sel).unwrap().transform;
            let shown = euler_degrees(current.rotation);
            let transform = Transform {
                translation: current.translation + Vec3::new(0.1, 0.0, 0.1),
                rotation: committed_rotation(current.rotation, shown, false),
                scale: current.scale,
            };
            model.edit_placement(sel, transform, None).unwrap();
            let after = model.placement(sel).unwrap().transform.rotation;
            assert_eq!(after, authored, "rotation drifted by position edit {step}");
        }
    }

    #[test]
    fn a_rotation_edit_recomposes_the_displayed_angles_faithfully() {
        let q = rotation_from_degrees((40.0, 30.0, 20.0));
        let (yaw, pitch, roll) = euler_degrees(q);
        assert!((yaw - 40.0).abs() < 1e-3, "yaw {yaw}");
        assert!((pitch - 30.0).abs() < 1e-3, "pitch {pitch}");
        assert!((roll - 20.0).abs() < 1e-3, "roll {roll}");
        let recomposed = committed_rotation(q, (yaw, pitch, roll), true);
        assert!(recomposed.dot(q).abs() > 1.0 - 1e-6, "recompose is the decompose's inverse");
    }

    #[test]
    fn repeated_rotation_edits_stay_stable_across_decompose_recompose_cycles() {
        // A multi-frame drag of a rotation field decomposes and recomposes every frame; the pair
        // must be stable enough that a hundred frames of it leave the rotation where it visually
        // started.
        let start = rotation_from_degrees((40.0, 30.0, 20.0));
        let mut q = start;
        for _ in 0..100 {
            q = committed_rotation(q, euler_degrees(q), true);
        }
        assert!(q.dot(start).abs() > 1.0 - 1e-4, "drifted after 100 cycles: dot {}", q.dot(start));
    }

    // ---- the narrowed collision warning ----

    fn solid(primitive: Primitive) -> Prefab {
        Prefab {
            states: vec![PrefabState {
                name: "default".to_string(),
                shapes: vec![Shape {
                    primitive,
                    transform: Transform::IDENTITY,
                    surface: None,
                    is_hitbox: true,
                    is_visible: true,
                }],
                mesh: None,
            }],
            default_state: "default".to_string(),
        }
    }

    fn placed(rotation: Quat, scale: Vec3) -> Placement {
        Placement {
            prefab: PrefabRef::new("p"),
            instance_id: InstanceId(0),
            name: None,
            transform: Transform { translation: Vec3::new(4.0, 1.0, 4.0), rotation, scale },
            state: None,
        }
    }

    #[test]
    fn a_clean_yawed_cube_no_longer_warns() {
        // The Obb carries the yaw exactly: no conservative box, no warning.
        let prefab = solid(Primitive::Cube);
        let yawed = placed(Quat::from_rotation_y(0.6), Vec3::new(2.0, 1.0, 1.5));
        assert!(!has_conservative_rotated_solid(&prefab, &yawed));
    }

    #[test]
    fn axis_aligned_solids_never_warn() {
        // Unrotated shapes collide as their own box (or better); the warning is about rotation
        // the colliders cannot carry, not about placeholder-grade boxes.
        for primitive in [Primitive::Cube, Primitive::Capsule, Primitive::Plane, Primitive::Cylinder] {
            let prefab = solid(primitive);
            let unrotated = placed(Quat::IDENTITY, Vec3::new(2.0, 1.0, 1.5));
            assert!(!has_conservative_rotated_solid(&prefab, &unrotated), "{primitive:?}");
        }
    }

    #[test]
    fn rotated_shapes_that_still_fall_to_the_box_warn() {
        // A yawed capsule and a tilted cylinder have no exact collider: the conservative box
        // genuinely outgrows them, which is exactly what the user should hear.
        let capsule = solid(Primitive::Capsule);
        assert!(has_conservative_rotated_solid(&capsule, &placed(Quat::from_rotation_y(0.6), Vec3::ONE)));
        let cylinder = solid(Primitive::Cylinder);
        assert!(has_conservative_rotated_solid(&cylinder, &placed(Quat::from_rotation_x(0.5), Vec3::ONE)));
    }

    #[test]
    fn a_rotated_round_shape_with_an_exact_collider_does_not_warn() {
        // A yawed upright cylinder classifies exactly (yaw spins it onto itself): no warning.
        let cylinder = solid(Primitive::Cylinder);
        assert!(!has_conservative_rotated_solid(&cylinder, &placed(Quat::from_rotation_y(0.8), Vec3::ONE)));
    }
}
