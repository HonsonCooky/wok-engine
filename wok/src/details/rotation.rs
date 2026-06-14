//! The inspector's rotation display math: decompose the authored quat into editable euler angles
//! and recompose it, with the commit rule that keeps other-field edits from drifting it.
//!
//! Rotation is shown as three degree fields over one documented euler order, YXZ: yaw about world
//! Y, then pitch about the yawed X, then roll about the resulting Z (glam's intrinsic
//! `EulerRot::YXZ`, so `from_euler(YXZ, ..)` is its exact inverse). The decomposition is
//! display-only: a commit recomposes the quat from the displayed angles only when a rotation field
//! itself changed ([`committed_rotation`]), so edits to position, scale, or state carry the
//! authored quat bit for bit and can never drift a rotation through repeated decompose-recompose
//! cycles. Yaw-first because yaw is the rotation placements actually author most; gimbal lock
//! (pitch at +/-90) folds yaw and roll into one axis there, as any euler display must.

use glam::Quat;

/// The angles the rotation fields show, degrees: the quat decomposed in the panel's one euler
/// order, YXZ.
pub(super) fn euler_degrees(rotation: Quat) -> (f32, f32, f32) {
    let (yaw, pitch, roll) = rotation.to_euler(glam::EulerRot::YXZ);
    (yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees())
}

/// The quat the displayed angles describe, recomposed in the same YXZ order [`euler_degrees`]
/// decomposes in.
pub(super) fn rotation_from_degrees((yaw, pitch, roll): (f32, f32, f32)) -> Quat {
    Quat::from_euler(glam::EulerRot::YXZ, yaw.to_radians(), pitch.to_radians(), roll.to_radians())
}

/// The rotation a commit writes: recomposed from the displayed angles only when a rotation field
/// itself changed this frame, the authored quat untouched otherwise. The untouched path is the
/// load-bearing half - the euler decomposition is lossy at float precision (and degenerate at
/// gimbal lock), so a commit that recomposed on every edit would drift an authored rotation a
/// little further on each position or scale tweak.
pub(super) fn committed_rotation(authored: Quat, displayed_deg: (f32, f32, f32), rot_changed: bool) -> Quat {
    if rot_changed { rotation_from_degrees(displayed_deg) } else { authored }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EditorModel, Selection};
    use crate::sample;
    use glam::Vec3;
    use wok_scene::{ChunkCoord, InstanceId, Transform};

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    /// A rotation that exercises all three axes, nowhere near gimbal lock.
    fn non_trivial() -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, 0.7, 0.4, 0.2)
    }

    #[test]
    fn edits_of_other_fields_never_touch_a_non_trivial_rotation() {
        // The decomposition trap: a commit that rebuilt the quat from the displayed euler angles
        // even when only position or scale changed would add float error on every edit. The commit
        // rule must instead carry the authored quat bit for bit through any number of other-field
        // edits.
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
}
