//! The editor's egui surfaces: the left side panel (scene tree, inspector, prefab library) and
//! the stats overlay.
//!
//! Panels read the model and emit [`Action`]s; they never mutate editor state directly, so the
//! frame loop stays the single writer and the UI cannot race an edit against a reload. Widget
//! behavior here is verified by running the editor (egui rendering is not unit-testable by
//! design); everything the panels display comes from `crate::model`'s tested tree and lookup
//! functions.

use glam::{Quat, Vec3};
use wok_physics::{Collider, basis_is_axis_aligned, classify_collider};
use wok_scene::{Placement, Prefab, PrefabRef, Transform};

use crate::model::{EditorModel, Selection};

/// UI-only state that persists across frames but is never saved.
#[derive(Default)]
pub struct UiState {
    /// Place mode: the prefab the next viewport click places.
    pub placing: Option<PrefabRef>,
}

/// What the user asked for this frame, applied by the frame loop after the UI runs.
pub enum Action {
    Select(Option<Selection>),
    Edit { sel: Selection, transform: Transform, state: Option<String> },
    ArmPlace(PrefabRef),
    DisarmPlace,
}

/// The frame numbers the stats overlay shows.
pub struct Stats {
    pub fps: f32,
    pub frame_ms: f32,
    pub chunk_count: usize,
    pub placement_count: usize,
    pub draw_items: usize,
}

/// Build the whole UI for one frame.
pub fn ui(ctx: &egui::Context, model: &EditorModel, ui_state: &UiState, stats: &Stats, actions: &mut Vec<Action>) {
    egui::SidePanel::left("wok_side_panel")
        .resizable(true)
        .default_width(320.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                scene_tree(ui, model, actions);
                ui.separator();
                inspector(ui, model, actions);
                ui.separator();
                library(ui, model, ui_state, actions);
            });
        });
    stats_overlay(ctx, stats);
}

/// The scene's structure: chunks, and under each its placements by instance label.
fn scene_tree(ui: &mut egui::Ui, model: &EditorModel, actions: &mut Vec<Action>) {
    ui.heading("Scene");
    for node in model.tree() {
        let header = format!("chunk {}_{}", node.coord.x, node.coord.z);
        egui::CollapsingHeader::new(header).default_open(true).show(ui, |ui| {
            for row in &node.rows {
                let sel = Selection { coord: node.coord, id: row.id };
                let selected = model.selection == Some(sel);
                let text = format!("{}  ({}, {})", row.label, row.prefab, row.state);
                if ui.selectable_label(selected, text).clicked() {
                    actions.push(Action::Select(Some(sel)));
                }
            }
            if node.rows.is_empty() {
                ui.weak("no placements");
            }
        });
    }
}

/// Numeric fields for the selected placement. Values are rebuilt from the authored form every
/// frame and committed through an [`Action::Edit`] the moment a field changes, so a drag edits
/// live and the authored form stays the single source of truth.
fn inspector(ui: &mut egui::Ui, model: &EditorModel, actions: &mut Vec<Action>) {
    ui.heading("Inspector");
    let Some(sel) = model.selection else {
        ui.weak("nothing selected");
        return;
    };
    let Some(placement) = model.placement(sel) else { return };
    let prefab = model.prefabs.get(&placement.prefab);

    ui.label(format!("prefab: {}", placement.prefab.as_str()));
    ui.label(format!("instance: {}", placement.instance_id.0));

    let authored = placement.transform;
    let mut pos = authored.translation;
    let mut yaw_deg = yaw_degrees(authored.rotation);
    let mut scale = authored.scale.x;
    let (mut pos_changed, mut yaw_changed, mut scale_changed) = (false, false, false);

    egui::Grid::new("wok_inspector_grid").num_columns(2).show(ui, |ui| {
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
    ui.label(
        egui::RichText::new(format!("position is chunk-local (chunk {}_{})", sel.coord.x, sel.coord.z))
            .small()
            .weak(),
    );

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

/// The prefab library: every prefab on disk, read-only; clicking arms place mode.
fn library(ui: &mut egui::Ui, model: &EditorModel, ui_state: &UiState, actions: &mut Vec<Action>) {
    ui.heading("Prefabs");
    let mut names: Vec<&str> = model.prefabs.keys().map(PrefabRef::as_str).collect();
    names.sort_unstable();
    for name in names {
        let armed = ui_state.placing.as_ref().is_some_and(|p| p.as_str() == name);
        if ui.selectable_label(armed, name).clicked() {
            actions.push(if armed {
                Action::DisarmPlace
            } else {
                Action::ArmPlace(PrefabRef::new(name))
            });
        }
    }
    if let Some(placing) = &ui_state.placing {
        ui.label(
            egui::RichText::new(format!("click terrain to place {} (Esc cancels)", placing.as_str()))
                .small(),
        );
    }
}

/// The corner stats overlay.
fn stats_overlay(ctx: &egui::Context, stats: &Stats) {
    egui::Window::new("wok_stats")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .title_bar(false)
        .resizable(false)
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(format!("{:.0} fps  {:.2} ms", stats.fps, stats.frame_ms));
            ui.label("sim: none (editor)");
            ui.label(format!("chunks: {}", stats.chunk_count));
            ui.label(format!("placements: {}", stats.placement_count));
            ui.label(format!("draw items: {}", stats.draw_items));
        });
}

fn drag(ui: &mut egui::Ui, value: &mut f32, speed: f64, prefix: &str) -> bool {
    ui.add(egui::DragValue::new(value).speed(speed).prefix(prefix)).changed()
}

/// The yaw the inspector shows: the Y component of the rotation decomposed yaw-first. Exact for
/// pure-yaw rotations (everything this editor writes); for an externally authored tilt it is the
/// yaw about world up, and editing the field replaces the rotation with pure yaw.
fn yaw_degrees(rotation: Quat) -> f32 {
    let (yaw, _, _) = rotation.to_euler(glam::EulerRot::YXZ);
    yaw.to_degrees()
}

/// Does this placement have a solid shape whose rotation the colliders cannot carry exactly, so it
/// still collides as the conservative axis-aligned box? Cubes now rotate honestly (Obb), spheres
/// and vertical cylinders spin onto themselves, and an axis-aligned shape's box is not widened by
/// the reduction - none of those warrant the warning. What remains is the genuinely conservative
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
    use wok_scene::{InstanceId, PrefabState, Primitive, Shape};

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
