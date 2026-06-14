//! The floating details panel: the v1 inspector, moved out of the left panel into a right-anchored
//! window that exists exactly while something is selected.
//!
//! Existence follows selection ([`visible`], pure and tested): no selection, no window - which is
//! also how Esc closes it, since Esc deselects. Values are rebuilt from the authored form every
//! frame and committed the moment a field changes, so a drag edits live and the authored form stays
//! the single source of truth.
//!
//! The fields always show the primary's values as the representative. With exactly one selected, a
//! change commits the v1 absolute [`Action::Edit`]. With more than one, it commits the DELTA from
//! the primary's shown value to the whole set ([`commit_actions`]): position as a `MoveSelection`,
//! rotation as a `RotateSelection`, scale as a `ScaleSelection`, the state combo as a
//! `SetStateSelection` - so the primary lands on exactly the shown value and the rest follow by the
//! same amount. The rotation display math (decompose to euler degrees, and the commit rule that
//! keeps other-field edits from drifting the quat) lives in the [`rotation`] submodule.

mod collision;
mod rotation;

use glam::Vec3;
use wok_scene::{Placement, Transform};

use crate::model::{EditorModel, Selection};
use crate::outline;
use crate::panels::Action;
use collision::has_conservative_rotated_solid;
use rotation::{committed_rotation, euler_degrees, rotation_from_degrees};

/// Whether the details window exists this frame: a selection that still resolves to a placement.
/// Pure, so the panel's visibility rule is testable without a window.
pub fn visible(model: &EditorModel) -> bool {
    model.selection.primary().is_some_and(|sel| model.placement(sel).is_some())
}

/// The multi-selection banner: `Some("N selected")` when more than one placement is selected, `None`
/// otherwise. The fields below show the primary's values but edit the whole set (see
/// [`commit_actions`]). Pure, so the count rule is testable without a window.
fn multi_select_header(model: &EditorModel) -> Option<String> {
    let n = model.selection.len();
    (n > 1).then(|| format!("{n} selected"))
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
    // When several are selected, a small banner notes the count; the fields show the primary's
    // values as the representative and edit the whole set.
    if let Some(note) = multi_select_header(model) {
        ui.label(egui::RichText::new(note).small().weak());
    }
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
        let edit = FieldEdit {
            pos,
            pos_changed,
            rot_deg: (yaw_deg, pitch_deg, roll_deg),
            rot_changed,
            scale,
            scale_changed,
            state: new_state,
            state_changed,
        };
        actions.extend(commit_actions(sel, authored, model.selection.len() > 1, &edit));
    }
}

fn drag(ui: &mut egui::Ui, value: &mut f32, speed: f64, prefix: &str) -> bool {
    ui.add(egui::DragValue::new(value).speed(speed).prefix(prefix)).changed()
}

/// One frame's field values read off the inspector widgets, with which fields actually changed. The
/// values are the primary's representative readout; [`commit_actions`] turns them into the right
/// action(s) for the selection size.
struct FieldEdit {
    pos: Vec3,
    pos_changed: bool,
    rot_deg: (f32, f32, f32),
    rot_changed: bool,
    scale: f32,
    scale_changed: bool,
    state: Option<String>,
    state_changed: bool,
}

/// The action(s) a field edit commits. One selected: the absolute single [`Action::Edit`], unchanged
/// from v1 (rotation recomposes only when a rotation field changed, so other-field edits carry the
/// authored quat bit for bit). More than one: the per-field DELTA from the primary's shown value,
/// one action per changed field - the primary lands on exactly the shown new value, and the rest
/// move by the same vector, turn by the same delta rotation, scale by the same factor, or take the
/// same state. Scale is a factor (`new / shown`) for the set rather than the single path's uniform
/// splat, so each member keeps its own proportions.
fn commit_actions(sel: Selection, authored: Transform, multi: bool, edit: &FieldEdit) -> Vec<Action> {
    if !multi {
        let transform = Transform {
            translation: edit.pos,
            rotation: committed_rotation(authored.rotation, edit.rot_deg, edit.rot_changed),
            scale: if edit.scale_changed { Vec3::splat(edit.scale) } else { authored.scale },
        };
        return vec![Action::Edit { sel, transform, state: edit.state.clone() }];
    }
    let mut actions = Vec::new();
    if edit.pos_changed {
        actions.push(Action::MoveSelection { delta: edit.pos - authored.translation });
    }
    if edit.rot_changed {
        let delta = rotation_from_degrees(edit.rot_deg) * authored.rotation.inverse();
        actions.push(Action::RotateSelection { delta });
    }
    if edit.scale_changed {
        actions.push(Action::ScaleSelection { factor: edit.scale / authored.scale.x });
    }
    if edit.state_changed {
        actions.push(Action::SetStateSelection { state: edit.state.clone() });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;
    use wok_scene::{ChunkCoord, InstanceId};

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

    #[test]
    fn the_inspector_notes_a_multi_selection_count() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        // One selected: the fields stand alone, no banner.
        model.selection.replace(Selection { coord, id: InstanceId(0) });
        assert_eq!(multi_select_header(&model), None, "a single selection shows no count");

        // A second selected: the banner reports the count.
        model.selection.toggle(Selection { coord, id: InstanceId(2) });
        assert_eq!(multi_select_header(&model).as_deref(), Some("2 selected"));
    }

    // ---- commit_actions: single absolute edit vs multi per-field delta ----

    fn unchanged(authored: Transform) -> FieldEdit {
        FieldEdit {
            pos: authored.translation,
            pos_changed: false,
            rot_deg: euler_degrees(authored.rotation),
            rot_changed: false,
            scale: authored.scale.x,
            scale_changed: false,
            state: None,
            state_changed: false,
        }
    }

    #[test]
    fn a_single_selection_commits_one_absolute_edit() {
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let authored = Transform { translation: Vec3::new(1.0, 2.0, 3.0), ..Transform::IDENTITY };
        let edit = FieldEdit { pos: Vec3::new(1.5, 2.0, 3.0), pos_changed: true, ..unchanged(authored) };

        match commit_actions(sel, authored, false, &edit).as_slice() {
            [Action::Edit { sel: s, transform, state }] => {
                assert_eq!(*s, sel);
                assert_eq!(transform.translation, Vec3::new(1.5, 2.0, 3.0), "absolute new position");
                assert_eq!(*state, None);
            }
            other => panic!("expected one absolute Edit, got {other:?}"),
        }
    }

    #[test]
    fn a_multi_selection_position_edit_commits_a_move_delta() {
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let authored = Transform { translation: Vec3::new(1.0, 2.0, 3.0), ..Transform::IDENTITY };
        let edit = FieldEdit { pos: Vec3::new(1.5, 2.0, 3.0), pos_changed: true, ..unchanged(authored) };

        match commit_actions(sel, authored, true, &edit).as_slice() {
            [Action::MoveSelection { delta }] => {
                assert!((*delta - Vec3::new(0.5, 0.0, 0.0)).length() < 1e-6, "delta = new - shown: {delta:?}");
            }
            other => panic!("expected one MoveSelection, got {other:?}"),
        }
    }

    #[test]
    fn a_multi_selection_rotation_and_scale_edits_commit_deltas() {
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let authored = Transform { scale: Vec3::splat(2.0), ..Transform::IDENTITY };

        // Rotation: the delta carries the primary from its shown orientation to the displayed one.
        let new_deg = (30.0, 0.0, 0.0);
        let rot_edit = FieldEdit { rot_deg: new_deg, rot_changed: true, ..unchanged(authored) };
        match commit_actions(sel, authored, true, &rot_edit).as_slice() {
            [Action::RotateSelection { delta }] => {
                let landed = *delta * authored.rotation;
                assert!(landed.dot(rotation_from_degrees(new_deg)).abs() > 1.0 - 1e-6, "delta lands the primary");
            }
            other => panic!("expected one RotateSelection, got {other:?}"),
        }

        // Scale: factor = new / shown, not the single path's uniform splat.
        let scale_edit = FieldEdit { scale: 3.0, scale_changed: true, ..unchanged(authored) };
        match commit_actions(sel, authored, true, &scale_edit).as_slice() {
            [Action::ScaleSelection { factor }] => assert!((*factor - 1.5).abs() < 1e-6, "3 / 2 = 1.5"),
            other => panic!("expected one ScaleSelection, got {other:?}"),
        }
    }

    #[test]
    fn a_multi_selection_state_edit_commits_a_set_state() {
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let authored = Transform::IDENTITY;
        let edit = FieldEdit { state: Some("open".to_string()), state_changed: true, ..unchanged(authored) };

        match commit_actions(sel, authored, true, &edit).as_slice() {
            [Action::SetStateSelection { state }] => assert_eq!(state.as_deref(), Some("open")),
            other => panic!("expected one SetStateSelection, got {other:?}"),
        }
    }
}
