//! The object-mode orbit camera, and the modal camera advance that picks fly vs orbit.
//!
//! In object mode the camera locks to the selection (designs/editor-design.md, Input): it frames
//! the set when the selection changes, then orbits its centroid - the right-drag turns the boom,
//! the scroll zooms the arm - while the home row stays inert (the object verbs are the next slice).
//! Unlike the free-fly camera (`crate::camera`), this one is constrained by world geometry, so it
//! composes the wok-physics math the game's follow camera uses: [`boom_direction`] / [`boom_point`]
//! place the camera on the boom around the pivot, and [`spring_arm`] shortens the boom so it does
//! not pass through other placements. The terrain-floor clamp (the boom's third constraint) needs
//! the chunk heightmap under the camera, so the caller applies it after [`step`] and re-aims with
//! [`camera::look_at`]; everything in this module is pure and window-free.
//!
//! [`Orbit`] is the desired orbit (the boom's angles plus its length); the pivot is the live
//! selection centroid, passed in each frame so the camera follows the selection if it moves. The
//! arm length is kept here, not derived from the camera each frame, so the spring arm can clamp the
//! displayed boom inward while the orbit still recovers to the user's chosen distance once the
//! obstruction clears. [`advance`] is the modal gate: free-fly flies, object orbits, and object with
//! nothing selected holds the last pose.

use glam::{Vec2, Vec3};
use wok_physics::{Collider, boom_direction, boom_point, spring_arm};
use wok_scene::Aabb;

use crate::camera::{self, CameraInput, FlyCamera};
use crate::mode::Mode;

/// Boom-angle pitch limit (radians), a touch short of the poles: at straight up or down the boom
/// aligns with world up and the look-at basis is singular.
const PITCH_LIMIT: f32 = 1.5;
/// Arm-length zoom: one scroll notch scales the boom by this. Scrolling up (positive) zooms in.
const ZOOM_FACTOR: f32 = 1.3;
/// Arm-length bounds in metres: a close inspect out to a few chunks.
const DIST_MIN: f32 = 1.0;
const DIST_MAX: f32 = 300.0;
/// Spring-arm probe radius: the standoff the camera keeps in front of an obstruction (metres).
const PROBE_RADIUS: f32 = 0.5;

/// The desired orbit around the selection: the boom's two angles and its length. The convention is
/// wok-physics's boom (see [`boom_direction`]): `(0, 0)` puts the camera on the `+Z` side of the
/// pivot, positive yaw swings toward `+X`, positive pitch lifts the camera overhead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    /// The desired boom length; the spring arm may display a shorter one, recovering back to this.
    pub distance: f32,
}

impl Default for Orbit {
    fn default() -> Orbit {
        Orbit { yaw: 0.0, pitch: 0.5, distance: 20.0 }
    }
}

impl Orbit {
    /// The orbit that reproduces a [`FlyCamera`] already aimed at `pivot`: the boom angles are the
    /// camera's look angles negated (looking at the pivot is the `-boom` direction), and the arm is
    /// the current distance. The bridge from a framed or flown pose into the orbit's own state.
    pub fn aiming(camera: &FlyCamera, pivot: Vec3) -> Orbit {
        Orbit {
            yaw: -camera.yaw,
            pitch: -camera.pitch,
            distance: (camera.position - pivot).length().clamp(DIST_MIN, DIST_MAX),
        }
    }

    /// The orbit that frames `bounds` and aims at `pivot`: run the same framing [`camera::frame`]
    /// uses (keep yaw, clamp to a gentle look-down, back off to fit the fov), then read the orbit
    /// off it. The auto-frame triggered by a selection change.
    pub fn framing(camera: &FlyCamera, bounds: Aabb, pivot: Vec3) -> Orbit {
        Orbit::aiming(&camera::frame(camera, bounds.min, bounds.max), pivot)
    }

    /// Turn the boom by a look delta (the right-drag), clamping pitch short of the poles.
    pub fn turn(&mut self, look: Vec2) {
        self.yaw += look.x;
        self.pitch = (self.pitch + look.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Zoom the arm by scroll notches; positive (scroll up) shortens it, clamped to the arm bounds.
    pub fn zoom(&mut self, steps: f32) {
        self.distance = (self.distance * ZOOM_FACTOR.powf(-steps)).clamp(DIST_MIN, DIST_MAX);
    }
}

/// Place the camera for one orbit step: the boom around `pivot`, shortened by the spring arm so it
/// clears `statics`, aimed back at the pivot. `speed` carries the free-fly speed through unchanged
/// (object mode does not use it, but toggling back to free-fly should find it where it was left).
/// The terrain-floor clamp is the caller's, after this.
pub fn step(orbit: Orbit, pivot: Vec3, statics: &[Collider], speed: f32) -> FlyCamera {
    let dir = boom_direction(orbit.yaw, orbit.pitch);
    let arm = spring_arm(pivot, dir, orbit.distance, PROBE_RADIUS, statics);
    let position = boom_point(pivot, dir, arm);
    let (yaw, pitch) = camera::look_at(position, pivot);
    FlyCamera { position, yaw, pitch, speed }
}

/// Advance the camera one frame under the interaction `mode`. Free-fly flies from `nav`
/// ([`camera::update`]); object mode orbits `pivot` - the right-drag look and the scroll act, the
/// home row's movement axes are inert - and holds the last pose when `pivot` is `None` (nothing
/// selected). The caller resolves `pivot` (the selection centroid) and `statics` (the other
/// placements' colliders); this sequences the rest.
pub fn advance(
    mode: Mode,
    camera: &FlyCamera,
    orbit: &mut Orbit,
    nav: &CameraInput,
    dt: f32,
    pivot: Option<Vec3>,
    statics: &[Collider],
) -> FlyCamera {
    match mode {
        Mode::FreeFly => camera::update(camera, nav, dt),
        Mode::Object => {
            let Some(pivot) = pivot else { return *camera };
            orbit.turn(nav.look_delta);
            orbit.zoom(nav.speed_steps);
            step(*orbit, pivot, statics, camera.speed)
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn free_fly_flies_on_the_home_row_but_object_mode_does_not() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, speed: 10.0 };
        let forward = CameraInput { move_forward: 1.0, ..Default::default() };

        // Free-fly: the home row moves the camera.
        let mut o = Orbit::default();
        let flew = advance(Mode::FreeFly, &cam, &mut o, &forward, 1.0, None, &[]);
        assert_ne!(flew.position, cam.position, "free-fly flies on the home row");

        // Object, nothing selected: the camera holds its pose, the home row inert.
        let mut o = Orbit::default();
        let held = advance(Mode::Object, &cam, &mut o, &forward, 1.0, None, &[]);
        assert_eq!(held.position, cam.position, "object mode holds the pose with no selection");

        // Object, with a selection: the home row still never translates the camera - only the orbit
        // input (look, scroll) does. Two navs differing only in the home row give the same camera.
        let pivot = Vec3::new(0.0, 2.0, -20.0);
        let mut a = Orbit { yaw: 0.0, pitch: 0.2, distance: 18.0 };
        let mut b = a;
        let with_row = advance(Mode::Object, &cam, &mut a, &forward, 1.0, Some(pivot), &[]);
        let without_row =
            advance(Mode::Object, &cam, &mut b, &CameraInput::default(), 1.0, Some(pivot), &[]);
        assert_eq!(with_row.position, without_row.position, "the home row never flies in object mode");
    }

    #[test]
    fn framing_then_a_step_frames_the_selection() {
        // Object mode's auto-frame: build the orbit from a framing of the bounds, step it, and the
        // camera ends up looking straight at the centroid from far enough to take in the whole box.
        let start = FlyCamera { position: Vec3::new(100.0, 80.0, 100.0), yaw: 0.6, pitch: -0.2, speed: 16.0 };
        let bounds = Aabb::new(Vec3::new(10.0, 0.0, 10.0), Vec3::new(16.0, 6.0, 14.0));
        let pivot = (bounds.min + bounds.max) * 0.5;

        let cam = step(Orbit::framing(&start, bounds, pivot), pivot, &[], 16.0);
        let to_pivot = (pivot - cam.position).normalize();
        assert!((to_pivot - cam.forward()).length() < EPS, "forward {:?} vs {to_pivot:?}", cam.forward());
        let radius = (bounds.max - bounds.min).length() * 0.5;
        assert!((cam.position - pivot).length() >= radius, "the framed camera should clear the bounds");
    }

    #[test]
    fn the_orbit_springs_in_front_of_an_obstruction() {
        // A wall on the +Z side of the pivot, where the boom wants the camera: the spring arm pulls
        // the camera in front of it instead of letting the boom pass through.
        let pivot = Vec3::ZERO;
        let wall = Collider::from(Aabb::new(Vec3::new(-5.0, -5.0, 5.0), Vec3::new(5.0, 5.0, 6.0)));
        let cam = step(Orbit { yaw: 0.0, pitch: 0.0, distance: 10.0 }, pivot, &[wall], 16.0);
        assert!(cam.position.z < 5.0, "camera should sit in front of the wall, z = {}", cam.position.z);
        assert!((cam.position - pivot).length() < 10.0, "the arm was shortened from the full distance");
    }

    #[test]
    fn zoom_shortens_on_scroll_up_and_clamps() {
        let mut o = Orbit { yaw: 0.0, pitch: 0.0, distance: 20.0 };
        o.zoom(1.0);
        assert!(o.distance < 20.0, "scroll up zooms in: {}", o.distance);
        o.zoom(-1.0);
        assert!((o.distance - 20.0).abs() < 1e-3, "scrolling back out returns near the start: {}", o.distance);
        for _ in 0..100 {
            o.zoom(1.0);
        }
        assert!((o.distance - DIST_MIN).abs() < 1e-3, "the arm clamps to the minimum");
    }

    #[test]
    fn turn_clamps_pitch_short_of_the_poles_and_yaw_is_free() {
        let mut o = Orbit::default();
        o.turn(Vec2::new(0.0, 100.0));
        assert!((o.pitch - PITCH_LIMIT).abs() < 1e-6, "pitch clamps up");
        o.turn(Vec2::new(0.0, -200.0));
        assert!((o.pitch + PITCH_LIMIT).abs() < 1e-6, "pitch clamps down");
        let before = o.yaw;
        o.turn(Vec2::new(0.5, 0.0));
        assert!((o.yaw - (before + 0.5)).abs() < 1e-6, "yaw is unbounded");
    }
}
