//! The floating details panel: the v1 inspector, moved out of the left panel into a
//! right-anchored window that exists exactly while something is selected.
//!
//! Existence follows selection ([`visible`], pure and tested): no selection, no window - which is
//! also how Esc closes it, since Esc deselects. Values are rebuilt from the authored form every
//! frame and committed through [`Action::Edit`] the moment a field changes, so a drag edits live
//! and the authored form stays the single source of truth, exactly as the v1 inspector did.

use glam::{Quat, Vec3};
use wok_physics::{Collider, basis_is_axis_aligned, classify_collider};
use wok_scene::{Placement, Prefab, Transform};

use crate::model::{EditorModel, Selection};
use crate::outline;
use crate::panels::Action;

/// Whether the details window exists this frame: a selection that still resolves to a placement.
/// Pure, so the panel's visibility rule is testable without a window.
pub fn visible(model: &EditorModel) -> bool {
    model.selection.is_some_and(|sel| model.placement(sel).is_some())
}

/// Build the details window (or nothing, per [`visible`]).
pub fn window(ctx: &egui::Context, model: &EditorModel, actions: &mut Vec<Action>) {
    if !visible(model) {
        return;
    }
    let Some(sel) = model.selection else { return };
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
    let mut yaw_deg = yaw_degrees(authored.rotation);
    let mut scale = authored.scale.x;
    let (mut pos_changed, mut yaw_changed, mut scale_changed) = (false, false, false);

    egui::Grid::new("wok_details_grid").num_columns(2).show(ui, |ui| {
        ui.label("position");
        ui.horizontal(|ui| {
            pos_changed |= drag(ui, &mut pos.x, 0.05, "x ");
            pos_changed |= drag(ui, &mut pos.y, 0.05, "y ");
            pos_changed |= drag(ui, &mut pos.z, 0.05, "z ");
        });
        ui.end_row();
        ui.label("yaw deg");
        yaw_changed = ui.add(egui::DragValue::new(&mut yaw_deg).speed(1.0)).changed();
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

    if pos_changed || yaw_changed || scale_changed || state_changed {
        let transform = Transform {
            translation: pos,
            // Only a yaw edit rebuilds the rotation, so an authored non-yaw rotation survives
            // edits to the other fields; same for non-uniform scale.
            rotation: if yaw_changed { Quat::from_rotation_y(yaw_deg.to_radians()) } else { authored.rotation },
            scale: if scale_changed { Vec3::splat(scale) } else { authored.scale },
        };
        actions.push(Action::Edit { sel, transform, state: new_state });
    }
}

fn drag(ui: &mut egui::Ui, value: &mut f32, speed: f64, prefix: &str) -> bool {
    ui.add(egui::DragValue::new(value).speed(speed).prefix(prefix)).changed()
}

/// The yaw the details panel shows: the Y component of the rotation decomposed yaw-first. Exact
/// for pure-yaw rotations (everything this editor writes); for an externally authored tilt it is
/// the yaw about world up, and editing the field replaces the rotation with pure yaw.
fn yaw_degrees(rotation: Quat) -> f32 {
    let (yaw, _, _) = rotation.to_euler(glam::EulerRot::YXZ);
    yaw.to_degrees()
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
        model.selection = Some(sel);
        assert!(visible(&model), "a live selection shows the panel");

        model.delete(sel).unwrap();
        assert!(!visible(&model), "deleting the selection closes the panel");

        // A selection dangling at a placement that no longer exists (set before validation ran)
        // must not show a panel either.
        model.selection = Some(Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(999) });
        assert!(!visible(&model), "a dangling selection shows nothing");
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
