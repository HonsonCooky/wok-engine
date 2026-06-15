//! Input routing: what the camera, hotkeys, the object-mode keyboard, and viewport clicks get to
//! see after egui has claimed its share.
//!
//! egui sees every raw window event first (via `App::on_window_event`); these functions then
//! consult the two focus flags the frame loop reads from egui - `pointer_free` (the cursor is
//! not over a panel and no widget is being dragged) and `keys_free` (no field has keyboard
//! focus) - so the same physical input never acts in the UI and the viewport at once. The fly
//! camera keeps right-mouse-hold to look, which leaves the cursor free for the UI by
//! construction. The mouse is selection-only: the left button picks, places, and box-selects with
//! a marquee (`marquee`); the selection is moved by the keyboard, not by dragging it. In object
//! mode the home row nudges the selection (`object`). Every authored-model change is emitted as an
//! [`Action`](crate::panels::Action) for the frame loop to apply, never written here, so the loop
//! stays the model's single writer; only presentation state (place mode, the context menu, the
//! marquee) is touched in place.
//!
//! The module is split at the camera-vs-viewport seam: [`camera_input`] maps the raw snapshot to
//! camera movement and look, [`handle`] routes hotkeys, the mode toggle, clicks, and the marquee,
//! delegating the object-mode key verbs to `object` and the per-frame marquee lifecycle to
//! `marquee`. The public entry points are re-exported here so callers address the `input` module,
//! not its parts.

mod camera;
mod marquee;
mod object;
mod viewport;

pub use camera::camera_input;
pub use marquee::Marquee;
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
    use wok_platform::winit::event::MouseButton;
    use wok_platform::winit::keyboard::{Key, NamedKey};
    use wok_scene::ChunkCoord;

    use crate::camera::FlyCamera;
    use crate::model::{EditorModel, Selection, chunk_origin};
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

    /// The sample's first pillar and a camera aimed at it from above-south, so a centre-cursor ray
    /// meets it before any terrain. Shared by the viewport pick tests (plain and Ctrl click).
    pub(crate) fn aim_at_pillar(model: &EditorModel) -> (Selection, FlyCamera) {
        let coord = ChunkCoord::new(0, 0);
        let pillar = model.chunks[&coord]
            .placements
            .iter()
            .find(|p| p.prefab.as_str() == "pillar")
            .expect("a pillar in the sample");
        let sel = Selection { coord, id: pillar.instance_id };
        let target = chunk_origin(coord) + pillar.transform.translation;
        (sel, looking_from_at(target + Vec3::new(0.0, 30.0, 30.0), target))
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

    /// Run a full left click at `input`'s cursor: a press frame (which arms a marquee) then a
    /// release frame with the cursor unmoved (under the slop, so the click resolves), returning the
    /// actions the release emits. Any modifiers and the cursor on `input` carry into both frames.
    /// The click resolves on release now that a press first arms a marquee.
    pub(crate) fn clicked(
        input: &InputState,
        camera: &FlyCamera,
        model: &EditorModel,
        ui: &mut UiState,
    ) -> Vec<Action> {
        let mut press = input.clone();
        press.mouse_buttons_pressed.insert(MouseButton::Left);
        press.mouse_buttons_held.insert(MouseButton::Left);
        let _ = emitted(&press, camera, model, ui);

        let mut release = input.clone();
        release.mouse_buttons_released.insert(MouseButton::Left);
        emitted(&release, camera, model, ui)
    }

    /// Run a left marquee drag from `from` to `to` (physical pixels), optionally with Ctrl held the
    /// whole gesture: a press at `from`, a held frame at `to`, then a release at `to`. The corners
    /// must be at least the slop apart so the box goes active. Returns the actions the release
    /// emits. Every left press arms a marquee now (the mouse is selection-only), so `from` may land
    /// anywhere, a selected member included.
    pub(crate) fn marquee_dragged(
        camera: &FlyCamera,
        model: &EditorModel,
        ui: &mut UiState,
        from: (f64, f64),
        to: (f64, f64),
        ctrl: bool,
    ) -> Vec<Action> {
        let frame = |pos: (f64, f64), pressed: bool, held: bool, released: bool| {
            let mut input = blank_input();
            input.mouse_pos = pos;
            if pressed {
                input.mouse_buttons_pressed.insert(MouseButton::Left);
            }
            if held {
                input.mouse_buttons_held.insert(MouseButton::Left);
            }
            if released {
                input.mouse_buttons_released.insert(MouseButton::Left);
            }
            if ctrl {
                input.keys_held.insert(Key::Named(NamedKey::Control));
            }
            input
        };
        let _ = emitted(&frame(from, true, true, false), camera, model, ui);
        let _ = emitted(&frame(to, false, true, false), camera, model, ui);
        emitted(&frame(to, false, false, true), camera, model, ui)
    }

    /// Screen centre for the viewport `emitted` uses; a centred cursor rays along forward.
    pub(crate) const CENTER_CURSOR: (f64, f64) = (400.0, 300.0);
}
