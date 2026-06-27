//! The editor's camera state and the pure matrix and framing math the renderer and picking read.
//!
//! The view-math primitives live here; the mode-switching editor camera that holds them is in
//! [`modes`] (re-exported as [`Camera`] and [`Mode`], with the Orbit-mode `OrbitCamera` private to it).
//! [`LayoutCamera`] is the top-down orthographic camera over a focus point - the Layout home of the
//! keyboard-first camera model
//! (designs/movement-camera-design.md). It builds the orthographic [`view_proj`](LayoutCamera::view_proj)
//! the renderer draws with and the straight-down [`cursor_ray`](LayoutCamera::cursor_ray) that picking
//! and the drag-and-drop cast through, the Look target pans and zooms it ([`pan`](LayoutCamera::pan) /
//! [`zoom`](LayoutCamera::zoom)), and [`fit_to`](LayoutCamera::fit_to) frames a bounds top-down.
//! [`FlyCamera`] is the perspective basis ([`forward`](FlyCamera::forward) / [`right`](FlyCamera::right)
//! / [`up`](FlyCamera::up) / [`view_proj`](FlyCamera::view_proj) / [`cursor_ray`](FlyCamera::cursor_ray));
//! the Orbit camera ([`modes`]) builds on it and [`frame`] fits a bounds into its fov. Everything here is
//! pure - no egui, no input, no window - and unit tested below.
//!
//! Conventions: right-handed, `+Y` up. For `FlyCamera`, `yaw` is radians about `+Y` with `0` facing
//! `-Z`, positive turning right (toward `+X`); `pitch` is radians with positive looking up. `forward`
//! follows the yaw/pitch; `right` is always horizontal (no roll); `up` is their cross, equal to `+Y`
//! when level. For `LayoutCamera`, the view looks straight down (`-Y`) with screen-up mapped to world
//! `-Z` (a map orientation: north is up, east is right).

use glam::{Mat4, Vec2, Vec3};

/// The mode-switching editor camera (the two-mode Layout/Orbit camera and the Orbit-mode state).
mod modes;
pub use modes::{Camera, Mode};

/// Vertical field of view and near plane for the projection. The far plane is per-frame data (the
/// scene's render distance - its streaming extent, per the HLD), so it is a [`view_proj`] parameter.
const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
const NEAR_PLANE: f32 = 0.1;

/// The perspective camera's whole state between frames - the basis the Orbit camera ([`modes`]) derives
/// its eye position and view from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyCamera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

impl FlyCamera {
    /// The unit vector the camera looks along.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    /// The unit vector to the camera's right, always horizontal (roll is never introduced). Still parked:
    /// the Orbit-relative move derives its right grid axis from the snapped forward rather than this, so
    /// only the tests exercise it for now (it lands with the macro tier's canonical vantages).
    #[allow(dead_code)]
    pub fn right(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, 0.0, sin_yaw)
    }

    /// The unit vector for the camera's up across the view plane: perpendicular to forward and right,
    /// so it tilts with pitch (unlike world up). Equals `+Y` when the view is level. The basis
    /// (`right`, `up`, `-forward`) is orthonormal at any orientation. Still parked like
    /// [`right`](Self::right): only the tests exercise it for now.
    #[allow(dead_code)]
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    /// The combined view-projection matrix for a target with the given aspect ratio, with the far
    /// plane supplied per frame (the editor derives it from the fog distance). `perspective_rh`
    /// maps depth to `0..=1`, which is wgpu's clip-space convention.
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        let projection = Mat4::perspective_rh(FOV_Y_RADIANS, aspect, NEAR_PLANE, far);
        let view = Mat4::look_to_rh(self.position, self.forward(), Vec3::Y);
        projection * view
    }

    /// The world-space ray for a cursor click in the viewport: the inverse of [`view_proj`], used by
    /// the 3D picking (3b). `pos_in_rect` is the click in egui points relative to the well's top-left,
    /// and `rect_size` is that same well's size - the rect the 3D rendered into, so the ray matches
    /// what the user clicked (sharp-edges 2: one shared cursor-to-ray source). `far` is the scene's far
    /// plane, the same one the render projects with, so the unprojection inverts the exact matrix.
    ///
    /// The click maps to normalized device coordinates (egui's y runs down, NDC's up, so y flips),
    /// then two clip points - on the near plane (`z = 0`) and the far plane (`z = 1`, the depth range
    /// `perspective_rh` maps to, wgpu's convention) - are unprojected through the inverse
    /// view-projection (`project_point3` is the divide by `w`). The ray runs from the eye along their
    /// difference, normalized so a pick `t` reads as world distance. The ndc is a ratio of point
    /// coordinates, so the result is identical whether measured in points or pixels.
    ///
    /// [`view_proj`]: Self::view_proj
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        let ndc_x = 2.0 * pos_in_rect.x / rect_size.x - 1.0;
        let ndc_y = 1.0 - 2.0 * pos_in_rect.y / rect_size.y;
        let inv = self.view_proj(rect_size.x / rect_size.y, far).inverse();
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_point = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (self.position, (far_point - near).normalize())
    }
}

// ---- layout camera ----
// The active editor camera (brief 2): a top-down orthographic view of the world plane, the Layout home
// of the camera model (designs/movement-camera-design.md). Pure math like FlyCamera; the interaction
// layer pans and zooms it and the renderer draws whatever view_proj it is handed.

/// Near plane for the orthographic projection. Tiny like the perspective near; the far plane is
/// per-frame (the scene's render distance), a [`LayoutCamera::view_proj`] parameter.
const LAYOUT_NEAR: f32 = 0.1;

/// How far the eye floats above the focus plane, on top of the half-height. The eye doubles as the
/// pick-ray origin, so it must clear the framed content; it is also the fog distance (the renderer fogs
/// by distance from the eye), so it is kept modest - half-height plus this margin - to keep the working
/// (zoomed-in) view inside the scene's fog start rather than washed out. Camera feel is the brief's to
/// settle (the doc leaves it open); this is the basic balance.
const EYE_MARGIN: f32 = 20.0;

/// Orthographic half-height (metres) the camera spawns at - the default zoom over a fresh scene: a few
/// dozen metres of plane, an object-placement working view rather than a chunk survey (the macro tier
/// is brief 3).
const DEFAULT_HALF_HEIGHT: f32 = 24.0;

/// Zoom clamp: in to a couple of metres of plane, out to roughly two chunks. The macro chunk-framing
/// (brief 3) handles wider surveys; this keeps the basic Layout zoom in a sane band.
const MIN_HALF_HEIGHT: f32 = 1.0;
const MAX_HALF_HEIGHT: f32 = 256.0;

/// Multiplicative zoom per vertical-pair input, so each step changes the view by a constant ratio
/// rather than a constant span (a fixed metre step crawls when zoomed out and lurches when zoomed in).
const ZOOM_STEP: f32 = 1.05;

/// Pan distance per cluster input as a fraction of the half-height, so panning covers the same share of
/// the view at any zoom (Look target).
const PAN_FRACTION: f32 = 0.06;

/// Margin on the framed zoom: the bounds' larger horizontal extent is the half-height times this, so the
/// selection sits inside the view with breathing room rather than touching the edges.
const FIT_MARGIN: f32 = 1.3;

/// Smallest half-height framing zooms to, so a tiny placement gets a readable working view instead of the
/// camera diving onto it (an 8m-tall view at this floor) - the orthographic analogue of [`FRAME_MIN_RADIUS`].
const FIT_MIN_HALF_HEIGHT: f32 = 4.0;

/// A top-down orthographic camera over a focus point on the world plane. The default Layout camera: the
/// world reads as a map, the grid is exact, and a grid step is unambiguous. The focus is the pan
/// position (its XZ) on the plane it pans over (its Y, the ground); `half_height` is the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutCamera {
    /// The world point the camera looks straight down at: XZ is the pan position on the plane, Y the
    /// plane height it floats over (the ground; `0` by default).
    pub focus: Vec3,
    /// Orthographic half-height in world metres - the zoom. The view spans `2 * half_height` vertically
    /// and `2 * half_height * aspect` horizontally; smaller is more zoomed in.
    pub half_height: f32,
}

impl LayoutCamera {
    /// A camera looking down at `focus` at the default zoom - the spawn-over-a-scene and pre-scene
    /// default vantage.
    pub fn over(focus: Vec3) -> LayoutCamera {
        LayoutCamera { focus, half_height: DEFAULT_HALF_HEIGHT }
    }

    /// The eye's world height: the focus plane plus the half-height plus the margin, so the eye clears
    /// the framed content (it is the pick-ray origin) while staying as low as the zoom allows (it is the
    /// fog distance). See [`EYE_MARGIN`].
    fn eye_height(&self) -> f32 {
        self.focus.y + self.half_height + EYE_MARGIN
    }

    /// The eye position the renderer reads: directly over the focus, looking straight down.
    pub fn eye(&self) -> Vec3 {
        Vec3::new(self.focus.x, self.eye_height(), self.focus.z)
    }

    /// The top-down orthographic view-projection for the given aspect ratio, far plane supplied per
    /// frame (the scene's render distance, like [`FlyCamera::view_proj`]). Looks straight down (`-Y`)
    /// with screen-up mapped to world `-Z` (north up, east right - a map orientation). `orthographic_rh`
    /// maps depth to `0..=1`, wgpu's clip-space convention (the same range `perspective_rh` uses).
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        let half_w = self.half_height * aspect;
        let projection =
            Mat4::orthographic_rh(-half_w, half_w, -self.half_height, self.half_height, LAYOUT_NEAR, far);
        let view = Mat4::look_to_rh(self.eye(), Vec3::NEG_Y, Vec3::NEG_Z);
        projection * view
    }

    /// The world-space ray for a cursor click: straight down (`-Y`) through the cursor's world point on
    /// the plane. An orthographic view has no convergence, so the ray's world XZ is fixed by the cursor
    /// alone (the eye height only sets the origin, kept above the content). `pos_in_rect` is the click in
    /// egui points relative to the well's top-left, `rect_size` that same well's size - the rect the 3D
    /// rendered into (sharp-edges 2: one shared cursor-to-ray source). The picking and the drag-and-drop
    /// both cast through this.
    ///
    /// The click maps to normalized device coordinates (egui's y runs down, NDC's up, so y flips), which
    /// scale by the orthographic half-extents into a world offset from the focus: `+x` is world `+X`
    /// (east), screen-up (`+ndc_y`) is world `-Z` (north). The ndc is a ratio of point coordinates, so
    /// the result is identical whether measured in points or pixels.
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2) -> (Vec3, Vec3) {
        let ndc_x = 2.0 * pos_in_rect.x / rect_size.x - 1.0;
        let ndc_y = 1.0 - 2.0 * pos_in_rect.y / rect_size.y;
        let aspect = rect_size.x / rect_size.y;
        let world_x = self.focus.x + ndc_x * self.half_height * aspect;
        let world_z = self.focus.z - ndc_y * self.half_height;
        (Vec3::new(world_x, self.eye_height(), world_z), Vec3::NEG_Y)
    }

    /// Pan the focus across the world plane by `(dx, dz)` cluster cells (`+x` east, `+z` south), each a
    /// fixed fraction of the zoom so a step covers the same share of the view at any zoom. The Look
    /// target's cluster drives this.
    pub fn pan(&mut self, dx: i32, dz: i32) {
        let step = PAN_FRACTION * self.half_height;
        self.focus.x += dx as f32 * step;
        self.focus.z += dz as f32 * step;
    }

    /// Zoom the orthographic scale by `steps` (positive zooms out, negative in), multiplicatively and
    /// clamped to the sane band. The Look target's vertical pair drives this.
    pub fn zoom(&mut self, steps: i32) {
        let factor = ZOOM_STEP.powi(steps);
        self.half_height = (self.half_height * factor).clamp(MIN_HALF_HEIGHT, MAX_HALF_HEIGHT);
    }

    /// Frame an axis-aligned bounds top-down (the selection rung of the framing ladder,
    /// designs/movement-camera-design.md): centre the focus on it and size the zoom so the bounds' larger
    /// horizontal extent fits the view with margin, floored so a tiny placement still reads and clamped to
    /// the zoom band. Half-height is a half-extent, so fitting the larger of the X/Z extents covers both
    /// axes for any landscape aspect; the wide axis sits a touch looser than the tall one, which the
    /// margin absorbs. The combined [`Camera`] calls this for the Layout half of an auto-frame.
    pub fn fit_to(&mut self, min: Vec3, max: Vec3) {
        self.focus = (min + max) * 0.5;
        let extent = (max - min).max(Vec3::ZERO);
        let half = extent.x.max(extent.z) * 0.5 * FIT_MARGIN;
        self.half_height = half.max(FIT_MIN_HALF_HEIGHT).clamp(MIN_HALF_HEIGHT, MAX_HALF_HEIGHT);
    }
}

// ---- framing ----
// [`frame`] fits a bounds into the perspective fov - the framing-ladder primitive the [`OrbitCamera`]
// uses to frame the selection (modes). [`look_at`] stays parked under `#[allow(dead_code)]`: it lands
// with the macro tier's canonical vantages (the next bite), where a fixed yaw/pitch aims at a chunk.

/// The `(yaw, pitch)` that aim a camera at `position` toward `target`, in [`FlyCamera`]'s
/// convention (yaw about `+Y`, `0` facing `-Z`; pitch positive looking up). The inverse of
/// [`FlyCamera::forward`]: feeding these back through `forward` points at the target. Degenerate
/// (`position == target`) yields a level forward rather than a NaN, and the `asin` domain is
/// guarded against float drift past `+/-1`. Parked for the macro tier's canonical vantages (the orbit
/// nav and framing it aims at the focus by construction, so it needs no aim solve yet).
#[allow(dead_code)]
pub fn look_at(position: Vec3, target: Vec3) -> (f32, f32) {
    let dir = (target - position).normalize_or_zero();
    (dir.x.atan2(-dir.z), dir.y.clamp(-1.0, 1.0).asin())
}

/// Margin factor on the framing distance: the framed bounds' enclosing sphere fills roughly 70%
/// of the vertical field of view instead of touching its edges.
const FRAME_MARGIN: f32 = 1.4;

/// Smallest radius framing treats as real, so a tiny or flat placement still gets a readable
/// view instead of the camera diving onto it.
const FRAME_MIN_RADIUS: f32 = 1.0;

/// The framing pitch band, radians: a gentle look down at the subject. Framing keeps the user's
/// yaw (their sense of direction survives the jump) but a level or upward pitch would frame the
/// subject against the sky edge-on, so pitch is clamped into this band.
const FRAME_PITCH_MIN: f32 = -0.9;
const FRAME_PITCH_MAX: f32 = -0.15;

/// Move the camera to a sensible view of an axis-aligned bounds (the frame-to-subject jump):
/// keep the current yaw, clamp pitch into the gentle downward band, and back off along the
/// resulting forward until the bounds' enclosing sphere fits the vertical fov with margin. Pure:
/// camera and bounds in, camera out. The Orbit camera's `fit_to` ([`modes`]) reads the fit distance and
/// clamped pitch back off the result.
pub fn frame(camera: &FlyCamera, min: Vec3, max: Vec3) -> FlyCamera {
    let center = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.5).max(FRAME_MIN_RADIUS);
    let aimed = FlyCamera {
        pitch: camera.pitch.clamp(FRAME_PITCH_MIN, FRAME_PITCH_MAX),
        ..*camera
    };
    // A sphere of `radius` subtends the full vertical fov at distance radius / sin(fov / 2);
    // the margin backs off further so the subject sits inside the frame, not against it.
    let distance = radius * FRAME_MARGIN / (FOV_Y_RADIANS * 0.5).sin();
    FlyCamera { position: center - aimed.forward() * distance, ..aimed }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn camera() -> FlyCamera {
        FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0 }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

    // ---- orientation ----

    #[test]
    fn yaw_zero_faces_negative_z_with_a_level_basis() {
        let cam = camera();
        assert!(close(cam.forward(), Vec3::NEG_Z));
        assert!(close(cam.right(), Vec3::X));
        assert!(close(cam.up(), Vec3::Y));
    }

    #[test]
    fn the_basis_stays_orthonormal_when_pitched_and_yawed() {
        // up is unit length and perpendicular to forward and right at any orientation - the orthonormal
        // basis the view matrix and the rebuilt camera (brief 2) are built on.
        let cam = FlyCamera { yaw: 1.1, pitch: -0.7, ..camera() };
        assert!((cam.forward().length() - 1.0).abs() < EPS);
        assert!((cam.right().length() - 1.0).abs() < EPS);
        assert!((cam.up().length() - 1.0).abs() < EPS);
        assert!(cam.forward().dot(cam.right()).abs() < EPS);
        assert!(cam.forward().dot(cam.up()).abs() < EPS);
        assert!(cam.right().dot(cam.up()).abs() < EPS);
    }

    // ---- framing ----

    #[test]
    fn framing_looks_straight_at_the_bounds_center() {
        let cam = FlyCamera { position: Vec3::new(50.0, 3.0, -20.0), yaw: 1.2, pitch: 0.4 };
        let (min, max) = (Vec3::new(10.0, 2.0, 10.0), Vec3::new(12.0, 4.0, 13.0));
        let framed = frame(&cam, min, max);
        let center = (min + max) * 0.5;
        let to_center = (center - framed.position).normalize();
        assert!(close(to_center, framed.forward()), "forward {:?} vs {to_center:?}", framed.forward());
    }

    #[test]
    fn framing_backs_off_far_enough_for_the_bounds_to_fit_the_fov() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.3, pitch: -0.4 };
        let (min, max) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 4.0, 6.0));
        let framed = frame(&cam, min, max);
        let center = (min + max) * 0.5;
        let radius = (max - min).length() * 0.5;
        let fits_at = radius / (FOV_Y_RADIANS * 0.5).sin();
        assert!((center - framed.position).length() >= fits_at, "the sphere must fit the vertical fov");
    }

    #[test]
    fn framing_keeps_yaw_and_clamps_pitch_into_the_downward_band() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 2.1, pitch: 0.8 };
        let framed = frame(&cam, Vec3::ZERO, Vec3::ONE);
        assert_eq!(framed.yaw, 2.1, "the user's sense of direction survives the jump");
        assert_eq!(framed.pitch, FRAME_PITCH_MAX, "an upward pitch clamps to the gentle look-down");
        let steep = FlyCamera { pitch: -1.4, ..cam };
        assert_eq!(frame(&steep, Vec3::ZERO, Vec3::ONE).pitch, FRAME_PITCH_MIN);
    }

    #[test]
    fn framing_degenerate_bounds_still_gives_a_readable_distance() {
        // A point-sized bounds (a shapeless placement) frames from at least the minimum radius'
        // distance, never on top of the point.
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: -0.3 };
        let at = Vec3::new(5.0, 2.0, 5.0);
        let framed = frame(&cam, at, at);
        let distance = (at - framed.position).length();
        assert!(distance >= FRAME_MIN_RADIUS, "distance {distance} too close for a readable view");
        assert!(framed.position.is_finite());
    }

    // ---- matrices ----

    #[test]
    fn view_proj_centers_what_the_camera_looks_at() {
        let cam = FlyCamera { position: Vec3::new(3.0, 4.0, 5.0), yaw: 0.7, pitch: -0.2 };
        let target = cam.position + cam.forward() * 50.0;
        let clip = cam.view_proj(16.0 / 9.0, 400.0).project_point3(target);
        assert!(clip.x.abs() < EPS && clip.y.abs() < EPS, "clip was {clip:?}");
        assert!(clip.z > 0.0 && clip.z < 1.0, "depth should be inside wgpu's 0..1 range: {}", clip.z);
    }

    // ---- cursor_ray ----

    #[test]
    fn cursor_ray_through_the_center_points_along_forward() {
        // A click at the centre of the well unprojects to the camera's central axis, so the ray runs
        // from the eye straight along forward.
        let cam = FlyCamera { position: Vec3::new(3.0, 4.0, 5.0), yaw: 0.7, pitch: -0.2 };
        let size = Vec2::new(800.0, 600.0);
        let (origin, dir) = cam.cursor_ray(size * 0.5, size, 400.0);
        assert!(close(origin, cam.position), "the ray starts at the eye");
        assert!(close(dir, cam.forward()), "a centred click looks along forward: {dir:?}");
    }

    #[test]
    fn cursor_ray_inverts_the_projection_for_an_off_center_pixel() {
        // A world point in front projects to some pixel; the ray cast back through that pixel must aim
        // straight at it. This pins the ndc mapping, including egui's y-down -> NDC y-up flip.
        let cam = FlyCamera { position: Vec3::new(1.0, 2.0, -3.0), yaw: 0.4, pitch: -0.3 };
        let size = Vec2::new(1024.0, 768.0);
        let far = 500.0;
        let target = cam.position + cam.forward() * 40.0 + cam.right() * 6.0 + cam.up() * 4.0;
        let clip = cam.view_proj(size.x / size.y, far).project_point3(target);
        // Invert cursor_ray's ndc mapping to recover the egui-point pixel (y-down) the target lands on.
        let pixel = Vec2::new((clip.x + 1.0) * 0.5 * size.x, (1.0 - clip.y) * 0.5 * size.y);
        let (origin, dir) = cam.cursor_ray(pixel, size, far);
        assert!(close(dir, (target - origin).normalize()), "the ray aims at the target: {dir:?}");
    }

    // ---- look_at ----

    #[test]
    fn look_at_inverts_forward() {
        // The angles look_at returns, fed back through forward, point straight at the target: it is
        // the exact inverse framing relies on.
        let cam = FlyCamera { position: Vec3::new(2.0, 5.0, -3.0), yaw: 1.1, pitch: -0.4 };
        let target = cam.position + cam.forward() * 12.0;
        let (yaw, pitch) = look_at(cam.position, target);
        let aimed = FlyCamera { yaw, pitch, ..cam };
        assert!(close(aimed.forward(), cam.forward()), "forward {:?} vs {:?}", aimed.forward(), cam.forward());
    }

    #[test]
    fn look_at_is_graceful_when_position_equals_target() {
        let (yaw, pitch) = look_at(Vec3::splat(4.0), Vec3::splat(4.0));
        assert!(yaw.is_finite() && pitch.is_finite(), "degenerate aim must not be NaN: {yaw}, {pitch}");
    }

    // ---- layout camera ----

    #[test]
    fn layout_view_proj_centers_the_focus_and_projects_in_parallel() {
        // The focus projects to the screen centre at a depth inside wgpu's 0..1 range, and - the
        // orthographic signature - a point at the same XZ but a different height projects to the same
        // screen XY (no perspective convergence), so the top-down grid reads true to scale.
        let cam = LayoutCamera { focus: Vec3::new(10.0, 2.0, -5.0), half_height: 16.0 };
        let vp = cam.view_proj(16.0 / 9.0, 500.0);
        let clip = vp.project_point3(cam.focus);
        assert!(clip.x.abs() < EPS && clip.y.abs() < EPS, "the focus centres: {clip:?}");
        assert!(clip.z > 0.0 && clip.z < 1.0, "depth sits inside wgpu's 0..1 range: {}", clip.z);
        let lower = cam.focus + Vec3::new(0.0, -3.0, 0.0);
        let clip_lower = vp.project_point3(lower);
        assert!(
            (clip.x - clip_lower.x).abs() < EPS && (clip.y - clip_lower.y).abs() < EPS,
            "a height change must not shift the screen XY under an orthographic projection: {clip_lower:?}"
        );
    }

    #[test]
    fn layout_view_proj_maps_screen_up_to_north_and_right_to_east() {
        // The map orientation: world +X (east) by the half-width lands at the right edge, and world -Z
        // (north) by the half-height lands at the top edge - so screen-up is north, screen-right is east.
        let cam = LayoutCamera { focus: Vec3::ZERO, half_height: 10.0 };
        let aspect = 2.0;
        let vp = cam.view_proj(aspect, 500.0);
        let east = vp.project_point3(Vec3::new(cam.half_height * aspect, 0.0, 0.0));
        assert!((east.x - 1.0).abs() < 1e-4 && east.y.abs() < 1e-4, "east hits the right edge: {east:?}");
        let north = vp.project_point3(Vec3::new(0.0, 0.0, -cam.half_height));
        assert!((north.y - 1.0).abs() < 1e-4 && north.x.abs() < 1e-4, "north hits the top edge: {north:?}");
    }

    #[test]
    fn layout_cursor_ray_through_the_center_points_straight_down_at_the_focus() {
        let cam = LayoutCamera { focus: Vec3::new(4.0, 1.0, 7.0), half_height: 12.0 };
        let size = Vec2::new(800.0, 600.0);
        let (origin, dir) = cam.cursor_ray(size * 0.5, size);
        assert!(close(dir, Vec3::NEG_Y), "a top-down view looks straight down: {dir:?}");
        assert!(
            (origin.x - cam.focus.x).abs() < EPS && (origin.z - cam.focus.z).abs() < EPS,
            "a centred click sits over the focus: {origin:?}"
        );
    }

    #[test]
    fn layout_cursor_ray_inverts_the_projection_for_an_off_center_pixel() {
        // Project a world point to its pixel, then cast the cursor ray back through that pixel: the
        // straight-down ray must pass through the point's XZ (its height is irrelevant under ortho). This
        // pins the ndc mapping, including egui's y-down -> NDC y-up flip and the aspect scaling.
        let cam = LayoutCamera { focus: Vec3::new(3.0, 2.0, -4.0), half_height: 20.0 };
        let size = Vec2::new(1024.0, 768.0);
        let far = 600.0;
        let target = cam.focus + Vec3::new(7.0, 0.0, -5.0);
        let clip = cam.view_proj(size.x / size.y, far).project_point3(target);
        let pixel = Vec2::new((clip.x + 1.0) * 0.5 * size.x, (1.0 - clip.y) * 0.5 * size.y);
        let (origin, dir) = cam.cursor_ray(pixel, size);
        assert!(close(dir, Vec3::NEG_Y));
        assert!(
            (origin.x - target.x).abs() < 1e-3 && (origin.z - target.z).abs() < 1e-3,
            "the ray casts back through the point: {origin:?} vs {target:?}"
        );
    }

    #[test]
    fn layout_pan_moves_the_focus_and_zoom_scales_clamped() {
        let mut cam = LayoutCamera::over(Vec3::new(1.0, 0.0, 2.0));
        let before = cam.half_height;
        cam.pan(1, -1); // east, and north (forward, -Z)
        assert!((cam.focus.x - (1.0 + PAN_FRACTION * before)).abs() < EPS, "pans east by a zoom fraction");
        assert!((cam.focus.z - (2.0 - PAN_FRACTION * before)).abs() < EPS, "pans north (-Z) by a zoom fraction");
        cam.zoom(1);
        assert!(cam.half_height > before, "a positive step zooms out");
        cam.zoom(-1);
        assert!((cam.half_height - before).abs() < 1e-3, "the inverse step returns to the start");
        for _ in 0..1000 {
            cam.zoom(1);
        }
        assert!(cam.half_height <= MAX_HALF_HEIGHT + EPS, "zoom out clamps to the band");
        for _ in 0..1000 {
            cam.zoom(-1);
        }
        assert!(cam.half_height >= MIN_HALF_HEIGHT - EPS, "zoom in clamps to the band");
    }
}
