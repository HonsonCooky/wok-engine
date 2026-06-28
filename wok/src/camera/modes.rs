//! The mode-switching editor camera: the [`Mode`] the viewport is in, the `OrbitCamera` the Orbit mode
//! adds, and the combined [`Camera`] the frame loop, the interaction layer, and the renderer hold.
//!
//! `super` (`crate::camera`) holds the view-math primitives - the orthographic [`LayoutCamera`], the
//! perspective [`FlyCamera`] basis, and the [`frame`](crate::camera::frame) helper. This module is the
//! layer above them: the everyday two-mode camera of the keyboard-first model
//! (designs/movement-camera-design.md "Camera"). [`Camera`] carries both views, shares one focus point
//! between them so a mode switch keeps you looking at the same place, dispatches the matrices / picking
//! / nav by the current [`Mode`], and keeps the selection auto-framed (the selection rung of the framing
//! ladder).
//!
//! `OrbitCamera` is the Orbit mode's state - a focus, a distance, and a yaw/pitch about it - built into a
//! [`FlyCamera`] at a derived position (`fly`) so the perspective matrices and the cursor ray come
//! straight from the parked basis the brief un-parks here. Pure like the primitives: no egui, no input,
//! no window. It does name two wok-scene data types (`InstanceId`, `Aabb`) for the auto-frame coupling -
//! identity and bounds, not residency - which the frame loop feeds in.
//!
//! Walk (player eye height) is the reserved third mode (the doc): deferred, so it is not a [`Mode`]
//! variant yet - it joins with its own [`view_proj`](Camera::view_proj) arm and a slot in `Mode::cycled`
//! when built. The macro chunk-framing tier (framing chunks, snap-cycling canonical vantages) is the
//! next bite; this module is the object/selection altitude only.

use glam::{Mat4, Vec2, Vec3};
use wok_scene::{Aabb, InstanceId};

use super::{FlyCamera, LayoutCamera, frame};

// ---- orbit camera ----

/// Default orbit distance from the focus, in metres - the spawn-over-a-scene framing: far enough to read
/// an object-placement working view (a vertical span comparable to the Layout default zoom under the
/// perspective fov), near enough to stay inside the scene fog. Camera feel is the brief's to settle.
const DEFAULT_DISTANCE: f32 = 40.0;
/// Default orbit yaw (radians): `0` faces world `-Z` (north), so Orbit looks from the south like a map
/// read from below, the same heading the Layout view's screen-up points to.
const DEFAULT_YAW: f32 = 0.0;
/// Default orbit pitch (radians): a gentle look-down (the camera floats above the focus), inside the
/// framing pitch band so the spawn view reads form and height without being top-down.
const DEFAULT_PITCH: f32 = -0.6;

/// Radians per cluster input the orbit yaw and pitch step by (3 degrees), small enough that a held
/// cluster sweeps smoothly at the OS key-repeat rate. Tunable; camera feel is the parked tweak.
const ORBIT_ANGLE_STEP: f32 = std::f32::consts::PI / 60.0;
/// Pitch clamp, kept off the poles: at exactly `+/-pi/2` the look direction is parallel to world up and
/// the view matrix degenerates, so the orbit cannot tip past this (the doc's "clamp pitch off the
/// poles"). ~85.9 degrees either side of level.
const ORBIT_PITCH_LIMIT: f32 = 1.5;
/// Multiplicative dolly per vertical-pair input (like the Layout zoom), so each step changes the
/// distance by a constant ratio rather than a constant span.
const ORBIT_DOLLY_STEP: f32 = 1.05;
/// Dolly clamp: in to a couple of metres, out to a few chunks - the same sane band the Layout zoom uses,
/// since the macro chunk-framing tier (the next bite) owns wider surveys.
const ORBIT_MIN_DISTANCE: f32 = 2.0;
const ORBIT_MAX_DISTANCE: f32 = 512.0;

/// A perspective camera orbiting a focus point: the Orbit inspect mode (designs/movement-camera-design.md).
/// The state is the focus, the distance out from it, and the yaw/pitch the camera looks at it from; the
/// view position is derived (`fly`), never stored, so the camera always looks straight at the focus. The
/// matrices, the cursor ray, and the basis all come from the [`FlyCamera`] it builds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    /// The world point the camera orbits and looks at - the shared focus (see [`Camera`]).
    pub focus: Vec3,
    /// How far the eye sits from the focus, in metres - the dolly (the Look vertical pair drives it).
    pub distance: f32,
    /// Yaw about the focus, radians, [`FlyCamera`]'s convention (`0` faces `-Z`, positive turns toward
    /// `+X`). The Look cluster's left/right drives it.
    pub yaw: f32,
    /// Pitch about the focus, radians, positive looking up, clamped off the poles
    /// ([`ORBIT_PITCH_LIMIT`]). The Look cluster's forward/back drives it.
    pub pitch: f32,
}

impl OrbitCamera {
    /// An orbit camera looking at `focus` from the default distance, yaw, and gentle downward pitch - the
    /// spawn-over-a-scene vantage and the Orbit half of a freshly built [`Camera`].
    pub fn over(focus: Vec3) -> OrbitCamera {
        OrbitCamera { focus, distance: DEFAULT_DISTANCE, yaw: DEFAULT_YAW, pitch: DEFAULT_PITCH }
    }

    /// Build the equivalent [`FlyCamera`]: the same yaw/pitch, positioned back along the look direction by
    /// `distance` so the eye looks straight at the focus (`position = focus - forward * distance`). The
    /// perspective matrices and the cursor ray delegate through this, reusing the parked basis.
    fn fly(&self) -> FlyCamera {
        let oriented = FlyCamera { position: Vec3::ZERO, yaw: self.yaw, pitch: self.pitch };
        FlyCamera { position: self.focus - oriented.forward() * self.distance, ..oriented }
    }

    /// The perspective view-projection (far supplied per frame, like [`LayoutCamera::view_proj`]).
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        self.fly().view_proj(aspect, far)
    }

    /// The eye position the renderer reads (the derived orbit position).
    pub fn eye(&self) -> Vec3 {
        self.fly().position
    }

    /// The world-space ray for a cursor click, through the perspective unprojection ([`FlyCamera::cursor_ray`]).
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        self.fly().cursor_ray(pos_in_rect, rect_size, far)
    }

    /// The camera's forward projected onto the ground plane, as `(x, z)`. Its sign and dominant axis (not
    /// its length) are what the Orbit-relative move reads to pick the nearest grid axis, so it is left
    /// unnormalized. Never zero in practice - the pitch clamp keeps the look off vertical.
    pub fn ground_forward(&self) -> Vec2 {
        let forward = self.fly().forward();
        Vec2::new(forward.x, forward.z)
    }

    /// Orbit about the focus by `(dx, dz)` cluster cells: left/right step the yaw, forward/back the
    /// pitch (forward tips the camera up and over toward top-down, the common orbit feel), pitch clamped
    /// off the poles. The Look target's cluster drives this in Orbit.
    pub fn orbit(&mut self, dx: i32, dz: i32) {
        self.yaw += dx as f32 * ORBIT_ANGLE_STEP;
        self.pitch = (self.pitch + dz as f32 * ORBIT_ANGLE_STEP).clamp(-ORBIT_PITCH_LIMIT, ORBIT_PITCH_LIMIT);
    }

    /// Dolly the eye in or out by `steps` (positive backs off, negative closes in), multiplicatively and
    /// clamped to the sane band. The Look target's vertical pair drives this in Orbit.
    pub fn dolly(&mut self, steps: i32) {
        self.distance = (self.distance * ORBIT_DOLLY_STEP.powi(steps)).clamp(ORBIT_MIN_DISTANCE, ORBIT_MAX_DISTANCE);
    }

    /// Frame an axis-aligned bounds: look at its centre from a distance that fits its enclosing sphere in
    /// the fov, keeping the current yaw (the user's sense of direction) and easing the pitch into the
    /// gentle downward band. Reuses [`frame`](crate::camera::frame) - the parked framing math the brief
    /// un-parks for Orbit - reading the fit distance and clamped pitch back off the [`FlyCamera`] it returns.
    pub fn fit_to(&mut self, min: Vec3, max: Vec3) {
        let framed = frame(&self.fly(), min, max);
        self.focus = (min + max) * 0.5;
        self.distance = (self.focus - framed.position).length().clamp(ORBIT_MIN_DISTANCE, ORBIT_MAX_DISTANCE);
        self.pitch = framed.pitch;
        // yaw is kept (frame keeps it), so framing does not spin the view out from under the author.
    }
}

// ---- mode ----

/// Which camera the editor is in (designs/movement-camera-design.md "Camera"). Cycled by a dedicated
/// camera-mode key, separate from the [`Target`](crate::model::Target) toggle that aims the cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Top-down orthographic - the precise placement view, where Move maps the screen cardinals to the
    /// world axes exactly (the world reads as a map). Cycle to it for grid-true arranging.
    Layout,
    /// Perspective, orbiting the focus - the default home, for reading form, silhouette, and height.
    #[default]
    Orbit,
    // Walk (perspective at player eye height) is the reserved third mode (the doc): deferred, so it is
    // not a variant yet. It joins here with its own view_proj arm and a slot in `cycled` when built.
}

impl Mode {
    /// The next mode the camera-mode key steps to. Layout and Orbit cycle between each other; Walk joins
    /// the rotation when it is built.
    fn cycled(self) -> Mode {
        match self {
            Mode::Layout => Mode::Orbit,
            Mode::Orbit => Mode::Layout,
        }
    }

    /// The mode's status-bar label.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Layout => "Layout",
            Mode::Orbit => "Orbit",
        }
    }
}

// ---- the editor camera ----

/// The editor's camera: the current [`Mode`] and both the [`LayoutCamera`] and [`OrbitCamera`] states,
/// kept on one shared focus so cycling the mode keeps you looking at the same place. The frame loop holds
/// one of these (frame-loop residency, not model state, like the rest of the camera); the interaction
/// layer aims it (the Look target's cluster), the renderer reads its matrices, and it keeps the selection
/// framed.
///
/// Auto-frame coupling: selecting an instance frames it and couples the follow, so a later move keeps it
/// framed; a manual Look-nav decouples the follow (survey mode) and the recenter key re-frames and
/// re-couples ([`track_selection`](Self::track_selection)). The doc's rule: manual nav is for surveying,
/// not chasing the work, and recenter is explicit, never an automatic jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    mode: Mode,
    layout: LayoutCamera,
    orbit: OrbitCamera,
    /// Whether the focus tracks the selection each frame (coupled), or the author has decoupled it with a
    /// manual Look-nav to survey. Re-coupled by a new selection or the recenter key.
    following: bool,
    /// The selection the camera last framed, so a *new* selection (not a re-pick of the same one)
    /// auto-frames and re-couples. `None` when nothing was framed.
    framed: Option<InstanceId>,
}

impl Camera {
    /// A camera over `focus` in the Orbit home (both modes looking at it), not yet following a selection
    /// - the spawn-over-a-scene and pre-scene default. [`RenderScene::spawn_camera`](crate::render_scene::RenderScene::spawn_camera)
    /// builds this over a freshly loaded scene.
    pub fn over(focus: Vec3) -> Camera {
        Camera {
            mode: Mode::Orbit,
            layout: LayoutCamera::over(focus),
            orbit: OrbitCamera::over(focus),
            following: false,
            framed: None,
        }
    }

    /// The current camera mode (the status bar shows it; the interaction layer reads it to map the move).
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Cycle to the next camera mode, carrying the shared focus across so the switch keeps you looking at
    /// the same place. Driven by the dedicated camera-mode key.
    pub fn cycle_mode(&mut self) {
        let focus = self.focus();
        self.mode = self.mode.cycled();
        self.set_focus(focus);
    }

    /// The active mode's focus point.
    fn focus(&self) -> Vec3 {
        match self.mode {
            Mode::Layout => self.layout.focus,
            Mode::Orbit => self.orbit.focus,
        }
    }

    /// Put both modes' focus on `focus` - the shared-focus update behind cycling, framing, and the
    /// per-frame follow, so a later mode switch stays on target.
    fn set_focus(&mut self, focus: Vec3) {
        self.layout.focus = focus;
        self.orbit.focus = focus;
    }

    /// The Look target's cluster, by mode: in Layout it pans the focus across the plane, in Orbit it
    /// orbits yaw and pitch about the focus. Manual nav, so it decouples the auto-follow (survey mode).
    pub fn look_cluster(&mut self, dx: i32, dz: i32) {
        match self.mode {
            Mode::Layout => self.layout.pan(dx, dz),
            Mode::Orbit => self.orbit.orbit(dx, dz),
        }
        self.following = false;
    }

    /// The Look target's vertical pair, by mode: in Layout it zooms the orthographic scale, in Orbit it
    /// dollies the distance. Manual nav, so it decouples the auto-follow.
    pub fn look_vertical(&mut self, steps: i32) {
        match self.mode {
            Mode::Layout => self.layout.zoom(steps),
            Mode::Orbit => self.orbit.dolly(steps),
        }
        self.following = false;
    }

    /// The camera forward projected to the ground plane (Orbit's; meaningless in Layout, where the move
    /// uses the exact screen cardinals instead). The Orbit-relative move reads this to pick the grid axis
    /// nearest the way the camera faces.
    pub fn ground_forward(&self) -> Vec2 {
        self.orbit.ground_forward()
    }

    /// The view-projection for the active mode (far supplied per frame - the scene's render distance).
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        match self.mode {
            Mode::Layout => self.layout.view_proj(aspect, far),
            Mode::Orbit => self.orbit.view_proj(aspect, far),
        }
    }

    /// The eye position the renderer reads (fog distances from it), by mode.
    pub fn eye(&self) -> Vec3 {
        match self.mode {
            Mode::Layout => self.layout.eye(),
            Mode::Orbit => self.orbit.eye(),
        }
    }

    /// The world-space cursor ray for picking and drag-and-drop, by mode. Layout's straight-down ortho
    /// ray ignores `far`; Orbit's perspective unprojection needs it (the same far the view projects with).
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        match self.mode {
            Mode::Layout => self.layout.cursor_ray(pos_in_rect, rect_size),
            Mode::Orbit => self.orbit.cursor_ray(pos_in_rect, rect_size, far),
        }
    }

    /// Frame `bounds` in the active mode and couple the follow: Layout centres the focus and fits the
    /// zoom, Orbit looks at the centre from a fit distance. The shared focus is set on both modes after,
    /// so a later switch stays framed.
    fn frame_to(&mut self, bounds: Aabb) {
        match self.mode {
            Mode::Layout => self.layout.fit_to(bounds.min, bounds.max),
            Mode::Orbit => self.orbit.fit_to(bounds.min, bounds.max),
        }
        let focus = self.focus();
        self.set_focus(focus);
        self.following = true;
    }

    /// Keep the camera framed on the selection (the selection rung of the framing ladder). Called each
    /// frame after the render residency reconciles, so `bounds` is the selection's current world AABB
    /// (`None` when nothing is selected or it has no resolvable bounds); `recenter` is the recenter key.
    ///
    /// A new selection (different from the last framed) or a recenter (re)frames it and re-couples the
    /// follow; while coupled the focus tracks the selection's centre, so a keyboard move or a
    /// drag-and-drop keeps it framed. A decoupled follow (a manual Look-nav surveyed away) stays put until
    /// the next new selection or recenter - never an automatic jump.
    pub fn track_selection(&mut self, selection: Option<InstanceId>, bounds: Option<Aabb>, recenter: bool) {
        let changed = selection != self.framed;
        self.framed = selection;
        let Some(bounds) = bounds else { return };
        if changed || recenter {
            self.frame_to(bounds);
        } else if self.following {
            self.set_focus((bounds.min + bounds.max) * 0.5);
        }
    }

    /// Track the selection id during a manual mouse drag without moving the camera. A drag positions the
    /// selection against the current view (its cursor ray reads the focus), so following it would feed the
    /// moved focus back into that mapping and run away - the view must hold still through the drag. Syncing
    /// the framed id keeps the drop from reading as a new selection; the follow coupling is left as-is, so
    /// the drop frame (no longer dragging) re-centers a coupled selection once - it stays framed - while a
    /// decoupled one is left where the survey put it.
    pub fn hold_selection(&mut self, selection: Option<InstanceId>) {
        self.framed = selection;
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn bounds(center: Vec3, half: f32) -> Aabb {
        Aabb::new(center - Vec3::splat(half), center + Vec3::splat(half))
    }

    // ---- orbit camera ----

    #[test]
    fn the_orbit_position_sits_at_distance_and_looks_at_the_focus() {
        // The orbit position is built from focus + distance + yaw + pitch: it sits `distance` out from the
        // focus and its forward points straight back at it - the property the perspective view relies on.
        let cam = OrbitCamera { focus: Vec3::new(4.0, 1.0, -2.0), distance: 12.0, yaw: 0.7, pitch: -0.4 };
        let fly = cam.fly();
        assert!(((cam.focus - fly.position).length() - 12.0).abs() < EPS, "the eye is `distance` from the focus");
        let to_focus = (cam.focus - fly.position).normalize();
        assert!((to_focus - fly.forward()).length() < EPS, "forward points at the focus: {:?}", fly.forward());
        assert_eq!(cam.eye(), fly.position, "eye is the derived orbit position");
    }

    #[test]
    fn orbit_steps_yaw_and_pitch_and_clamps_pitch_off_the_poles() {
        let mut cam = OrbitCamera::over(Vec3::ZERO);
        let yaw0 = cam.yaw;
        cam.orbit(1, 0); // right steps yaw
        assert!((cam.yaw - (yaw0 + ORBIT_ANGLE_STEP)).abs() < EPS, "right steps yaw by one angle step");
        let pitch0 = cam.pitch;
        cam.orbit(0, -1); // forward tips toward top-down (pitch decreases)
        assert!(cam.pitch < pitch0, "forward tips the pitch down (camera up and over)");
        // Drive the pitch hard into both poles: it clamps off them and never reaches +/-pi/2.
        for _ in 0..1000 {
            cam.orbit(0, 1);
        }
        assert!((cam.pitch - ORBIT_PITCH_LIMIT).abs() < EPS, "pitch clamps off the up pole: {}", cam.pitch);
        for _ in 0..1000 {
            cam.orbit(0, -1);
        }
        assert!((cam.pitch + ORBIT_PITCH_LIMIT).abs() < EPS, "pitch clamps off the down pole: {}", cam.pitch);
        assert!(cam.pitch.abs() < std::f32::consts::FRAC_PI_2, "never reaches the pole");
    }

    #[test]
    fn dolly_scales_the_distance_clamped_and_reverses() {
        let mut cam = OrbitCamera::over(Vec3::ZERO);
        let before = cam.distance;
        cam.dolly(1);
        assert!(cam.distance > before, "a positive step backs off");
        cam.dolly(-1);
        assert!((cam.distance - before).abs() < 1e-2, "the inverse step returns to the start");
        for _ in 0..1000 {
            cam.dolly(-1);
        }
        assert!(cam.distance >= ORBIT_MIN_DISTANCE - EPS, "dolly in clamps to the band");
        for _ in 0..1000 {
            cam.dolly(1);
        }
        assert!(cam.distance <= ORBIT_MAX_DISTANCE + EPS, "dolly out clamps to the band");
    }

    // ---- per-mode fit ----

    #[test]
    fn orbit_fit_looks_at_the_centre_from_a_fit_distance() {
        // Orbit framing sets focus + a fit distance: the eye sits far enough out for the bounds' sphere to
        // fit the fov, looking at the centre, keeping the yaw and easing the pitch into the downward band.
        let mut cam = OrbitCamera { focus: Vec3::ZERO, distance: 5.0, yaw: 1.2, pitch: 0.8 };
        let b = bounds(Vec3::new(10.0, 2.0, -4.0), 3.0);
        cam.fit_to(b.min, b.max);
        assert!((cam.focus - Vec3::new(10.0, 2.0, -4.0)).length() < EPS, "focus on the bounds centre");
        assert_eq!(cam.yaw, 1.2, "framing keeps the yaw");
        assert!(cam.pitch < 0.0, "pitch eased into the gentle downward band: {}", cam.pitch);
        let fly = cam.fly();
        assert!((cam.focus - fly.position).length() > 3.0, "backs off past the bounds radius");
        assert!(((cam.focus - fly.position).normalize() - fly.forward()).length() < EPS, "still looks at the centre");
    }

    #[test]
    fn layout_fit_centres_and_sizes_the_zoom_to_the_bounds() {
        // Layout framing sets focus.xz + a fit half_height: the focus centres on the bounds and the zoom
        // grows with the larger horizontal extent, floored so a tiny placement still reads.
        let mut cam = LayoutCamera::over(Vec3::ZERO);
        let wide = bounds(Vec3::new(6.0, 0.0, 6.0), 20.0); // a 40m box
        cam.fit_to(wide.min, wide.max);
        assert!((cam.focus.x - 6.0).abs() < EPS && (cam.focus.z - 6.0).abs() < EPS, "focus centres on the bounds");
        assert!(cam.half_height > 20.0, "the zoom grows to fit the 40m box: {}", cam.half_height);
        // A tiny placement floors to a readable view rather than diving onto it.
        let tiny = bounds(Vec3::ZERO, 0.25);
        cam.fit_to(tiny.min, tiny.max);
        assert!(cam.half_height >= 4.0, "a tiny bounds floors to a readable zoom: {}", cam.half_height);
    }

    // ---- mode and shared focus ----

    #[test]
    fn cycling_the_mode_keeps_the_shared_focus() {
        let mut cam = Camera::over(Vec3::new(7.0, 0.0, -5.0));
        assert_eq!(cam.mode(), Mode::Orbit, "the default home is Orbit");
        // Cycle to Layout and pan its focus, then cycle back: Orbit must look at the same place the
        // Layout view ended on - the shared focus carries across the switch.
        cam.cycle_mode();
        assert_eq!(cam.mode(), Mode::Layout);
        cam.look_cluster(1, 0);
        let layout_focus = cam.layout.focus;
        cam.cycle_mode();
        assert_eq!(cam.mode(), Mode::Orbit, "cycles back to Orbit");
        assert_eq!(cam.orbit.focus, layout_focus, "the switch carries the shared focus across");
    }

    // ---- auto-frame coupling ----

    #[test]
    fn selecting_frames_and_couples_then_the_follow_tracks_a_move() {
        let mut cam = Camera::over(Vec3::ZERO);
        let first = bounds(Vec3::new(10.0, 0.0, 10.0), 1.0);
        // A new selection frames it and couples the follow.
        cam.track_selection(Some(InstanceId(1)), Some(first), false);
        assert!(cam.following, "a new selection couples the follow");
        assert!((cam.focus() - Vec3::new(10.0, 0.0, 10.0)).length() < EPS, "framed on the selection");
        // The same selection at a new position (a move): the focus tracks it without re-framing.
        let moved = bounds(Vec3::new(13.0, 0.0, 10.0), 1.0);
        cam.track_selection(Some(InstanceId(1)), Some(moved), false);
        assert!((cam.focus() - Vec3::new(13.0, 0.0, 10.0)).length() < EPS, "the follow keeps it framed as it moves");
    }

    #[test]
    fn a_manual_nav_decouples_and_recenter_re_couples() {
        let mut cam = Camera::over(Vec3::ZERO);
        let at = bounds(Vec3::new(10.0, 0.0, 10.0), 1.0);
        cam.track_selection(Some(InstanceId(1)), Some(at), false);
        // Survey away with a Look-nav: the follow decouples.
        cam.look_cluster(1, 0);
        assert!(!cam.following, "a manual Look-nav decouples the follow (survey)");
        // The selection now moves, but a decoupled camera does not chase it.
        let moved = bounds(Vec3::new(20.0, 0.0, 10.0), 1.0);
        let surveyed = cam.focus();
        cam.track_selection(Some(InstanceId(1)), Some(moved), false);
        assert_eq!(cam.focus(), surveyed, "surveying does not chase the work");
        // Recenter re-frames the current selection and re-couples.
        cam.track_selection(Some(InstanceId(1)), Some(moved), true);
        assert!(cam.following && (cam.focus() - Vec3::new(20.0, 0.0, 10.0)).length() < EPS, "recenter snaps back and re-couples");
    }

    #[test]
    fn a_new_selection_reframes_even_while_decoupled() {
        // Cycling to a different instance re-frames and re-couples, even after surveying away - the doc's
        // "moving or cycling the selection re-frames it".
        let mut cam = Camera::over(Vec3::ZERO);
        cam.track_selection(Some(InstanceId(1)), Some(bounds(Vec3::new(5.0, 0.0, 0.0), 1.0)), false);
        cam.look_cluster(0, 1); // decouple
        cam.track_selection(Some(InstanceId(2)), Some(bounds(Vec3::new(-8.0, 0.0, 3.0), 1.0)), false);
        assert!(cam.following, "a different selection re-couples");
        assert!((cam.focus() - Vec3::new(-8.0, 0.0, 3.0)).length() < EPS, "and frames the new instance");
    }

    #[test]
    fn a_drag_holds_the_camera_still_then_the_drop_re_centers_a_coupled_selection() {
        // A mouse drag positions the selection against the current view, so the camera must hold still
        // through it (a follow would feed the moved focus back into the cursor ray and run away).
        // hold_selection tracks the id without moving; the coupled follow survives, so the drop frame's
        // track_selection re-centers once and the selection stays framed.
        let mut cam = Camera::over(Vec3::ZERO);
        cam.track_selection(Some(InstanceId(1)), Some(bounds(Vec3::new(5.0, 0.0, 0.0), 1.0)), false);
        let framed = cam.focus();
        cam.hold_selection(Some(InstanceId(1)));
        assert_eq!(cam.focus(), framed, "the camera holds still through the drag");
        let dropped = bounds(Vec3::new(30.0, 0.0, 0.0), 1.0);
        cam.track_selection(Some(InstanceId(1)), Some(dropped), false);
        assert!((cam.focus() - Vec3::new(30.0, 0.0, 0.0)).length() < EPS, "the drop re-centers - it stays framed");
    }
}
