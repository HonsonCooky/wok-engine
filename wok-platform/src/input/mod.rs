use std::collections::{HashMap, HashSet};
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::keyboard::{Key, NamedKey};

/// Snapshot of all input state for a single frame.
#[derive(Clone, Debug)]
pub struct InputState {
    pub keys_held: HashSet<Key>,
    pub keys_pressed: HashSet<Key>,
    /// Keys the OS auto-repeated this frame (a held key past the initial repeat delay). Kept separate
    /// from `keys_pressed`, which stays a clean one-per-press edge so menus and shortcuts never
    /// double-fire; hold-to-repeat actions read the two together (press-or-repeat). Cleared each frame.
    pub keys_repeating: HashSet<Key>,
    pub keys_released: HashSet<Key>,
    pub mouse_pos: (f64, f64),
    /// Cursor-position-derived delta. Goes to zero when the cursor is locked. Use
    /// `mouse_motion` for raw motion that survives cursor capture.
    pub mouse_delta: (f64, f64),
    /// Raw mouse motion accumulated this frame from `DeviceEvent::MouseMotion`. Works with
    /// locked or hidden cursors - use this for first/third-person mouselook.
    pub mouse_motion: (f64, f64),
    pub mouse_buttons_held: HashSet<MouseButton>,
    pub mouse_buttons_pressed: HashSet<MouseButton>,
    pub mouse_buttons_released: HashSet<MouseButton>,
    pub scroll_delta: (f32, f32),
    pub gamepads: Vec<GamepadState>,
}

impl InputState {
    #[must_use]
    pub fn key_held(&self, key: NamedKey) -> bool {
        self.keys_held.contains(&Key::Named(key))
    }

    #[must_use]
    pub fn key_pressed(&self, key: NamedKey) -> bool {
        self.keys_pressed.contains(&Key::Named(key))
    }

    #[must_use]
    pub fn key_released(&self, key: NamedKey) -> bool {
        self.keys_released.contains(&Key::Named(key))
    }

    /// Was a printable character key pressed this frame? Compares case-
    /// insensitively so callers don't have to handle shift state - hotkeys
    /// like `q` / `w` work whether caps lock is on or shift is held.
    #[must_use]
    pub fn char_pressed(&self, ch: char) -> bool {
        self.keys_pressed.iter().any(|k| match k {
            Key::Character(s) => s.chars().any(|c| c.eq_ignore_ascii_case(&ch)),
            _ => false,
        })
    }

    /// Is a printable character key held this frame? The held counterpart to
    /// [`char_pressed`](Self::char_pressed) - same case-insensitive match, but
    /// over the keys still down rather than the press edge, for hold-to-act
    /// keys (e.g. the editor's hold-`r` / hold-`s` transform fast path).
    #[must_use]
    pub fn char_held(&self, ch: char) -> bool {
        self.keys_held.iter().any(|k| match k {
            Key::Character(s) => s.chars().any(|c| c.eq_ignore_ascii_case(&ch)),
            _ => false,
        })
    }

    /// Was a printable character key OS-auto-repeated this frame (a held key past the initial delay)?
    /// Separate from [`char_pressed`](Self::char_pressed), which stays a clean one-per-press edge: a
    /// hold-to-repeat action fires on `char_pressed(k) || char_repeating(k)`, so a quick tap (released
    /// before the first repeat) is one step and a hold spins at the OS repeat rate after the OS delay.
    /// Same case-insensitive match as [`char_pressed`](Self::char_pressed).
    #[must_use]
    pub fn char_repeating(&self, ch: char) -> bool {
        self.keys_repeating.iter().any(|k| match k {
            Key::Character(s) => s.chars().any(|c| c.eq_ignore_ascii_case(&ch)),
            _ => false,
        })
    }

    /// Was this named key OS-auto-repeated this frame? The named-key counterpart to
    /// [`char_repeating`](Self::char_repeating).
    #[must_use]
    pub fn key_repeating(&self, key: NamedKey) -> bool {
        self.keys_repeating.contains(&Key::Named(key))
    }

    #[must_use]
    pub fn mouse_held(&self, button: MouseButton) -> bool {
        self.mouse_buttons_held.contains(&button)
    }

    #[must_use]
    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button)
    }

    #[must_use]
    pub fn gamepad(&self, index: usize) -> Option<&GamepadState> {
        self.gamepads.get(index)
    }
}

/// State of a single gamepad.
#[derive(Clone, Debug)]
pub struct GamepadState {
    pub left_stick: (f32, f32),
    pub right_stick: (f32, f32),
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub buttons_held: HashSet<gilrs::Button>,
    pub buttons_pressed: HashSet<gilrs::Button>,
}

/// Accumulates input events between frames and produces snapshots. Exposed for consumers that
/// own their own winit event loop (e.g. tools that open and close windows on demand) and want
/// wok-platform's input semantics without going through `wok_platform::run`.
pub struct InputCollector {
    keys_held: HashSet<Key>,
    keys_pressed: HashSet<Key>,
    keys_repeating: HashSet<Key>,
    keys_released: HashSet<Key>,
    mouse_pos: (f64, f64),
    prev_mouse_pos: (f64, f64),
    mouse_buttons_held: HashSet<MouseButton>,
    mouse_buttons_pressed: HashSet<MouseButton>,
    mouse_buttons_released: HashSet<MouseButton>,
    scroll_delta: (f32, f32),
    mouse_motion: (f64, f64),
    gamepad_buttons_pressed: HashMap<gilrs::GamepadId, HashSet<gilrs::Button>>,
    gamepad_buttons_held: HashMap<gilrs::GamepadId, HashSet<gilrs::Button>>,
    gamepad_axes: HashMap<gilrs::GamepadId, HashMap<gilrs::Axis, f32>>,
    connected_gamepads: Vec<gilrs::GamepadId>,
}

impl Default for InputCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl InputCollector {
    pub fn new() -> Self {
        Self {
            keys_held: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_repeating: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            prev_mouse_pos: (0.0, 0.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 0.0),
            mouse_motion: (0.0, 0.0),
            gamepad_buttons_pressed: HashMap::new(),
            gamepad_buttons_held: HashMap::new(),
            gamepad_axes: HashMap::new(),
            connected_gamepads: Vec::new(),
        }
    }

    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                self.key_input(event.logical_key.clone(), event.state, event.repeat);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x, position.y);
            }
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    self.mouse_buttons_held.insert(*button);
                    self.mouse_buttons_pressed.insert(*button);
                }
                ElementState::Released => {
                    self.mouse_buttons_held.remove(button);
                    self.mouse_buttons_released.insert(*button);
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                    #[allow(clippy::cast_possible_truncation)]
                    winit::event::MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                self.scroll_delta.0 += dx;
                self.scroll_delta.1 += dy;
            }
            _ => {}
        }
    }

    /// One keyboard transition, split from the winit event so the edge/held logic is unit-testable
    /// (winit's `KeyEvent` carries a private platform field and cannot be built in a test). `repeat` is
    /// winit's OS-auto-repeat flag. A first press (`repeat` false) edges `keys_pressed` and marks the
    /// key held; an OS repeat (`repeat` true) edges only `keys_repeating`, leaving `keys_pressed` a
    /// clean one-per-press edge (so menus and shortcuts never double-fire) while hold-to-repeat actions
    /// can read the repeat. A release drops the held entry and edges `keys_released`. The held-set
    /// guard also swallows any stray non-repeat press of an already-held key.
    fn key_input(&mut self, key: Key, state: ElementState, repeat: bool) {
        match state {
            ElementState::Pressed if repeat => {
                self.keys_repeating.insert(key);
            }
            ElementState::Pressed => {
                if self.keys_held.insert(key.clone()) {
                    self.keys_pressed.insert(key);
                }
            }
            ElementState::Released => {
                self.keys_held.remove(&key);
                self.keys_released.insert(key);
            }
        }
    }

    /// Forward `DeviceEvent`s here. We only care about `MouseMotion` for now (raw mouse
    /// motion that survives cursor lock - needed for first/third-person mouselook).
    pub fn handle_device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.mouse_motion.0 += delta.0;
            self.mouse_motion.1 += delta.1;
        }
    }

    /// Drain gilrs events and track all gamepad state from the event stream directly.
    /// Avoids gilrs's cached state and reverse-mapping lookups, which can silently fail
    /// for axes on some controller/driver combinations.
    pub fn poll_gamepads(&mut self, gilrs: &mut gilrs::Gilrs) {
        while let Some(event) = gilrs.next_event() {
            let id = event.id;
            match event.event {
                gilrs::EventType::ButtonPressed(btn, _) => {
                    self.gamepad_buttons_pressed
                        .entry(id)
                        .or_default()
                        .insert(btn);
                    self.gamepad_buttons_held.entry(id).or_default().insert(btn);
                }
                gilrs::EventType::ButtonReleased(btn, _) => {
                    self.gamepad_buttons_held
                        .entry(id)
                        .or_default()
                        .remove(&btn);
                }
                gilrs::EventType::AxisChanged(axis, value, _) => {
                    // gilrs reports stick Y+ = physically up. Flip to match the prevailing
                    // game-input convention (XInput, SDL CONTROLLER_AXIS_LEFTY, screen Y) where
                    // stick Y+ = down/forward. Consumers can then wire the value straight into
                    // screen or world axes without per-call sign flips.
                    let value = match axis {
                        gilrs::Axis::LeftStickY | gilrs::Axis::RightStickY => -value,
                        _ => value,
                    };
                    self.gamepad_axes.entry(id).or_default().insert(axis, value);
                }
                gilrs::EventType::Disconnected => {
                    self.gamepad_buttons_pressed.remove(&id);
                    self.gamepad_buttons_held.remove(&id);
                    self.gamepad_axes.remove(&id);
                }
                _ => {}
            }
        }
        // Track which gamepads are currently connected. Filter out devices that gilrs surfaces
        // as gamepads but don't actually have a left stick (e.g. keyboards with media-key axes
        // that show up via evdev EV_ABS), and skip entries gilrs retains after disconnect.
        self.connected_gamepads.clear();
        for (id, gp) in gilrs.gamepads() {
            if gp.is_connected() && gp.axis_code(gilrs::Axis::LeftStickX).is_some() {
                self.connected_gamepads.push(id);
            }
        }
    }

    pub fn snapshot(&mut self) -> InputState {
        let mouse_delta = (
            self.mouse_pos.0 - self.prev_mouse_pos.0,
            self.mouse_pos.1 - self.prev_mouse_pos.1,
        );
        self.prev_mouse_pos = self.mouse_pos;

        let gamepads = self
            .connected_gamepads
            .iter()
            .map(|&id| {
                use gilrs::Axis;
                let axes = self.gamepad_axes.get(&id);
                let axis = |a| axes.and_then(|m| m.get(&a)).copied().unwrap_or(0.0);
                let pressed = self.gamepad_buttons_pressed.remove(&id).unwrap_or_default();
                let held = self
                    .gamepad_buttons_held
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                GamepadState {
                    left_stick: (axis(Axis::LeftStickX), axis(Axis::LeftStickY)),
                    right_stick: (axis(Axis::RightStickX), axis(Axis::RightStickY)),
                    left_trigger: axis(Axis::LeftZ),
                    right_trigger: axis(Axis::RightZ),
                    buttons_held: held,
                    buttons_pressed: pressed,
                }
            })
            .collect();
        self.gamepad_buttons_pressed.clear();

        let state = InputState {
            keys_held: self.keys_held.clone(),
            keys_pressed: self.keys_pressed.clone(),
            keys_repeating: self.keys_repeating.clone(),
            keys_released: self.keys_released.clone(),
            mouse_pos: self.mouse_pos,
            mouse_delta,
            mouse_motion: self.mouse_motion,
            mouse_buttons_held: self.mouse_buttons_held.clone(),
            mouse_buttons_pressed: self.mouse_buttons_pressed.clone(),
            mouse_buttons_released: self.mouse_buttons_released.clone(),
            scroll_delta: self.scroll_delta,
            gamepads,
        };

        // Clear per-frame state
        self.keys_pressed.clear();
        self.keys_repeating.clear();
        self.keys_released.clear();
        self.mouse_buttons_pressed.clear();
        self.mouse_buttons_released.clear();
        self.scroll_delta = (0.0, 0.0);
        self.mouse_motion = (0.0, 0.0);

        state
    }
}

// The collector's per-frame edge/held transitions are the pure logic of this crate, tested here.
// Keyboard goes through `key_input` directly (winit's `KeyEvent` cannot be built outside winit);
// mouse, scroll, and raw motion drive the real event handlers with winit's plain-data events. The
// gamepad sets follow the same pressed/held shape, but gilrs's `GamepadId` is mintable only by a
// live gilrs session, so those transitions are exercised through the applications instead.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::{DeviceId, MouseScrollDelta, TouchPhase};

    fn key(ch: &str) -> Key {
        Key::Character(ch.into())
    }

    #[test]
    fn a_press_edges_pressed_once_then_the_key_is_only_held() {
        let mut c = InputCollector::new();
        c.key_input(key("w"), ElementState::Pressed, false);

        let frame = c.snapshot();
        assert!(frame.keys_pressed.contains(&key("w")), "the press frame edges pressed");
        assert!(frame.keys_held.contains(&key("w")), "and the key is held the same frame");

        let next = c.snapshot();
        assert!(!next.keys_pressed.contains(&key("w")), "the edge lasts exactly one frame");
        assert!(next.keys_held.contains(&key("w")), "held persists until release");
    }

    #[test]
    fn os_auto_repeat_populates_repeating_not_pressed() {
        let mut c = InputCollector::new();
        c.key_input(key("w"), ElementState::Pressed, false);
        let _ = c.snapshot();

        // Holding a key makes the OS deliver repeat events (repeat = true): they surface as
        // keys_repeating for hold-to-repeat, but never re-edge keys_pressed, so menus and shortcuts do
        // not double-fire. The key stays held throughout.
        c.key_input(key("w"), ElementState::Pressed, true);
        let frame = c.snapshot();
        assert!(frame.keys_pressed.is_empty(), "a repeat is not a new press");
        assert!(!frame.char_pressed('w'), "and char_pressed stays a clean one-per-press edge");
        assert!(frame.keys_repeating.contains(&key("w")), "the repeat edges keys_repeating");
        assert!(frame.char_repeating('w'), "and char_repeating, case-insensitively");
        assert!(frame.keys_held.contains(&key("w")), "the key is still held");

        // The repeating edge lasts exactly one frame, like pressed.
        assert!(!c.snapshot().char_repeating('w'), "the repeat edge clears next frame");
    }

    #[test]
    fn char_pressed_edges_once_and_char_held_persists_case_insensitively() {
        // The letter-key accessors the editor's hold-to-act keys read: the press edges char_pressed
        // for exactly one frame, char_held tracks the key while it is down, and both match the typed
        // letter regardless of case (shift / caps lock).
        let mut c = InputCollector::new();
        c.key_input(key("r"), ElementState::Pressed, false);

        let frame = c.snapshot();
        assert!(frame.char_pressed('r'), "the press frame edges char_pressed");
        assert!(frame.char_pressed('R'), "the match is case-insensitive");
        assert!(frame.char_held('r'), "and the key is held the same frame");

        let next = c.snapshot();
        assert!(!next.char_pressed('r'), "the pressed edge lasts exactly one frame");
        assert!(next.char_held('R'), "held persists until release, case-insensitively");

        c.key_input(key("r"), ElementState::Released, false);
        assert!(!c.snapshot().char_held('r'), "held clears on release");
    }

    #[test]
    fn a_release_edges_released_for_one_frame_and_clears_held() {
        let mut c = InputCollector::new();
        c.key_input(Key::Named(NamedKey::Space), ElementState::Pressed, false);
        let _ = c.snapshot();

        c.key_input(Key::Named(NamedKey::Space), ElementState::Released, false);
        let frame = c.snapshot();
        assert!(frame.key_released(NamedKey::Space), "the release frame edges released");
        assert!(!frame.key_held(NamedKey::Space), "held clears on release");

        let next = c.snapshot();
        assert!(!next.key_released(NamedKey::Space), "the released edge lasts exactly one frame");
    }

    #[test]
    fn a_mouse_click_edges_pressed_and_released_one_frame_each() {
        let mut c = InputCollector::new();
        let press = WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        c.handle_window_event(&press);

        let frame = c.snapshot();
        assert!(frame.mouse_pressed(MouseButton::Left));
        assert!(frame.mouse_held(MouseButton::Left));
        let held = c.snapshot();
        assert!(!held.mouse_pressed(MouseButton::Left), "the pressed edge lasts one frame");
        assert!(held.mouse_held(MouseButton::Left));

        let release = WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Released,
            button: MouseButton::Left,
        };
        c.handle_window_event(&release);
        let frame = c.snapshot();
        assert!(frame.mouse_buttons_released.contains(&MouseButton::Left));
        assert!(!frame.mouse_held(MouseButton::Left), "held clears on release");
        assert!(c.snapshot().mouse_buttons_released.is_empty(), "the released edge lasts one frame");
    }

    #[test]
    fn scroll_accumulates_within_a_frame_and_clears_at_snapshot() {
        let mut c = InputCollector::new();
        let wheel = |x: f32, y: f32| WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(x, y),
            phase: TouchPhase::Moved,
        };
        c.handle_window_event(&wheel(0.0, 1.0));
        c.handle_window_event(&wheel(0.5, 2.0));

        assert_eq!(c.snapshot().scroll_delta, (0.5, 3.0), "ticks within a frame accumulate");
        assert_eq!(c.snapshot().scroll_delta, (0.0, 0.0), "and the accumulator clears per frame");
    }

    #[test]
    fn raw_mouse_motion_accumulates_and_clears_at_snapshot() {
        let mut c = InputCollector::new();
        c.handle_device_event(&DeviceEvent::MouseMotion { delta: (3.0, -1.0) });
        c.handle_device_event(&DeviceEvent::MouseMotion { delta: (2.0, 4.0) });

        assert_eq!(c.snapshot().mouse_motion, (5.0, 3.0), "raw motion accumulates within the frame");
        assert_eq!(c.snapshot().mouse_motion, (0.0, 0.0), "and clears per frame");
    }

    #[test]
    fn cursor_delta_is_the_per_frame_position_difference() {
        let mut c = InputCollector::new();
        let moved = |x: f64, y: f64| WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        };
        c.handle_window_event(&moved(10.0, 20.0));
        assert_eq!(c.snapshot().mouse_delta, (10.0, 20.0));

        c.handle_window_event(&moved(15.0, 18.0));
        let frame = c.snapshot();
        assert_eq!(frame.mouse_pos, (15.0, 18.0));
        assert_eq!(frame.mouse_delta, (5.0, -2.0), "the delta is against the previous frame's position");
        assert_eq!(c.snapshot().mouse_delta, (0.0, 0.0), "a still cursor reads zero delta");
    }
}
