//! Input routing: what the camera, hotkeys, and viewport clicks get to see after egui has
//! claimed its share.
//!
//! egui sees every raw window event first (via `App::on_window_event`); these functions then
//! consult the two focus flags the frame loop reads from egui - `pointer_free` (the cursor is
//! not over a panel and no widget is being dragged) and `keys_free` (no field has keyboard
//! focus) - so the same physical input never acts in the UI and the viewport at once. The fly
//! camera keeps right-mouse-hold to look, which leaves the cursor free for the UI by
//! construction. The left button picks, places, and (on the already-selected placement) drags to
//! move, with `crate::drag` owning the drag math. Every authored-model change is emitted as an
//! [`Action`](crate::panels::Action) for the frame loop to apply, never written here, so the loop
//! stays the model's single writer; only presentation state (place mode, the context menu, the
//! drag) is touched in place.
//!
//! The module is split at the camera-vs-viewport seam: [`camera_input`] maps the raw snapshot to
//! camera movement and look, [`handle`] routes hotkeys, clicks, and the placement drag. Both are
//! re-exported here so callers address the `input` module, not its parts.

mod camera;
mod viewport;

pub use camera::camera_input;
pub use viewport::handle;

/// Motion (pixels) under which a press-and-release still reads as a click rather than a drag:
/// enough for a twitchy hand, far under any deliberate drag. Shared by the right button (camera
/// look vs context click) and the left button (placement drag vs pick).
const CLICK_SLOP_PX: f32 = 4.0;

/// Shared fixtures for the input module's viewport tests. They live here (not in `viewport.rs`)
/// so that file stays under the file-size target with its full test set; `camera.rs` keeps its
/// own `input_with`, which only its nav tests use. The next brief's viewport tests reuse these.
#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashSet;

    use glam::Vec3;
    use wok_platform::input::InputState;

    use crate::camera::FlyCamera;
    use crate::model::EditorModel;
    use crate::panels::{Action, UiState};
    use crate::sample;

    use super::handle;

    pub(crate) fn blank_input() -> InputState {
        InputState {
            keys_held: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: (0.0, 0.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 0.0),
            gamepads: vec![],
        }
    }

    pub(crate) fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    /// A camera at `eye` looking straight at `target`, so a screen-centre cursor rays exactly along
    /// that line (the centre-ray-is-forward property `crate::pick` pins). Keep `target` off the
    /// camera's vertical line: looking along world up is the look-matrix singularity.
    pub(crate) fn looking_from_at(eye: Vec3, target: Vec3) -> FlyCamera {
        let d = (target - eye).normalize();
        FlyCamera { position: eye, yaw: d.x.atan2(-d.z), pitch: d.y.asin(), speed: 16.0 }
    }

    /// A camera for the keyboard-only cases, where no ray is cast.
    pub(crate) fn any_camera() -> FlyCamera {
        FlyCamera { position: Vec3::new(64.0, 40.0, 100.0), yaw: 0.0, pitch: -0.6, speed: 16.0 }
    }

    /// Run `handle` with the pointer and keys free (egui claims nothing) over an 800x600 viewport,
    /// returning the actions it emitted. A centred `mouse_pos` then rays along the camera forward.
    pub(crate) fn emitted(
        input: &InputState,
        camera: &FlyCamera,
        model: &EditorModel,
        ui: &mut UiState,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        handle(input, true, true, camera, (800, 600), 500.0, model, ui, &mut actions);
        actions
    }

    /// Screen centre for the viewport `emitted` uses; a centred cursor rays along forward.
    pub(crate) const CENTER_CURSOR: (f64, f64) = (400.0, 300.0);
}
