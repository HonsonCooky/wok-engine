//! CPU-side packing of the GPU uniform blocks.
//!
//! Each function lowers an engine-side type into the flat `f32` array whose byte layout matches
//! the corresponding WGSL struct (see `src/shaders/common.wgsl` and `mesh.wgsl`). Everything is
//! vec4-aligned by construction: WGSL's uniform address space aligns vec3 fields to 16 bytes, so
//! the arrays pack scalars into the fourth lane of a related vector instead of carrying padding.
//!
//! Sanitizing happens here, once per frame, rather than per fragment in the shader: the sun
//! direction is normalized (wok-light documents it may arrive unnormalized), the band count is
//! floored at 2 so the band divisor stays positive (the HLD sets no upper clamp), the
//! transition softness is kept strictly positive so the band smoothstep's edges never coincide,
//! and the fog end is kept strictly past its start so the fog divisor is never zero. The
//! per-scene fog on/off rides in the sky zenith's spare `w` lane (1.0 on, 0.0 off); the mesh pass
//! tests it before blending, so a fog-off scene skips fog entirely.

use glam::{Mat3, Mat4, Vec3};
use wok_light::LightState;

use crate::renderer::Camera;

// Byte sizes of the corresponding WGSL uniform structs.
pub(crate) const CAMERA_UNIFORM_SIZE: u64 = 208; // three mat4 plus one vec4
pub(crate) const LIGHT_UNIFORM_SIZE: u64 = 96; // six vec4
pub(crate) const DRAW_UNIFORM_SIZE: u64 = 144; // two mat4 plus one vec4

// Floors for the shader's two divisions; small enough to read as "hard edge" and "no fog band"
// while keeping the math finite.
const MIN_SOFTNESS: f32 = 1.0e-3;
const MIN_FOG_SPAN: f32 = 1.0e-3;

/// Pack `camera` into the WGSL `Camera` block: view-projection, its inverse (for the sky pass's
/// unprojection), the sun's shadow view-projection (computed per frame by `crate::shadow`), and
/// the eye position. A non-invertible view-projection is the caller's bug; the inverse is not
/// checked here.
pub(crate) fn camera_floats(camera: &Camera, sun_view_proj: Mat4) -> [f32; 52] {
    let mut out = [0.0; 52];
    out[0..16].copy_from_slice(&camera.view_proj.to_cols_array());
    out[16..32].copy_from_slice(&camera.view_proj.inverse().to_cols_array());
    out[32..48].copy_from_slice(&sun_view_proj.to_cols_array());
    out[48..51].copy_from_slice(&camera.eye.to_array());
    out
}

/// The sanitized sun travel direction: normalized, falling back to straight down for a zero
/// vector (wok-light documents the direction may arrive unnormalized). One function shared by the
/// light packing and the shadow fit, so the map is rendered along exactly the axis the lambert
/// term reads; if the two drifted apart, every shadow would sit offset from its caster.
pub(crate) fn sun_direction(light: &LightState) -> Vec3 {
    light.sun.direction.try_normalize().unwrap_or(Vec3::NEG_Y)
}

/// Pack `light` into the WGSL `Light` block, sanitizing as documented on the module.
pub(crate) fn light_floats(light: &LightState) -> [f32; 24] {
    let sun_dir = sun_direction(light);
    let bands = light.cel.band_count.max(2) as f32;
    let softness = light.cel.transition_softness.clamp(MIN_SOFTNESS, 1.0);
    let fog_start = light.fog.start;
    let fog_end = light.fog.end.max(fog_start + MIN_FOG_SPAN);
    let fog_on = if light.fog.enabled { 1.0 } else { 0.0 };

    let mut out = [0.0; 24];
    pack(&mut out, 0, sun_dir, bands);
    pack(&mut out, 4, light.sun.color, softness);
    pack(&mut out, 8, light.ambient, light.cel.rim_intensity);
    pack(&mut out, 12, light.fog.color, fog_start);
    pack(&mut out, 16, light.sky.horizon, fog_end);
    pack(&mut out, 20, light.sky.zenith, fog_on);
    out
}

/// Pack one render item's per-draw block: the model matrix, its normal matrix, the flat base
/// color, and the opacity riding the color's fourth lane (the screen-door threshold the mesh
/// shader compares against; 1.0 is opaque and can never discard). The normal matrix is the
/// inverse-transpose of the model's linear part, so normals stay perpendicular to surfaces under
/// non-uniform scale; computed once per draw here rather than per vertex in the shader. A
/// singular model matrix is the caller's bug, as with the camera.
pub(crate) fn draw_floats(transform: Mat4, color: Vec3, opacity: f32) -> [f32; 36] {
    let normal = Mat4::from_mat3(Mat3::from_mat4(transform).inverse().transpose());
    let mut out = [0.0; 36];
    out[0..16].copy_from_slice(&transform.to_cols_array());
    out[16..32].copy_from_slice(&normal.to_cols_array());
    out[32..35].copy_from_slice(&color.to_array());
    out[35] = opacity;
    out
}

fn pack(out: &mut [f32], at: usize, v: Vec3, w: f32) {
    out[at..at + 4].copy_from_slice(&[v.x, v.y, v.z, w]);
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_light::{CelParams, Fog, SkyGradient, Sun};

    fn light() -> LightState {
        LightState {
            sun: Sun { direction: Vec3::new(0.0, -2.0, 0.0), color: Vec3::new(1.0, 0.9, 0.8) },
            ambient: Vec3::new(0.1, 0.2, 0.3),
            fog: Fog { enabled: true, color: Vec3::new(0.5, 0.6, 0.7), start: 10.0, end: 100.0 },
            sky: SkyGradient { horizon: Vec3::new(0.7, 0.7, 0.7), zenith: Vec3::new(0.2, 0.4, 0.9) },
            cel: CelParams { band_count: 4, transition_softness: 0.1, rim_intensity: 0.5 },
        }
    }

    #[test]
    fn float_counts_match_the_declared_byte_sizes() {
        let camera = Camera { view_proj: Mat4::IDENTITY, eye: Vec3::ZERO };
        assert_eq!(camera_floats(&camera, Mat4::IDENTITY).len() as u64 * 4, CAMERA_UNIFORM_SIZE);
        assert_eq!(light_floats(&light()).len() as u64 * 4, LIGHT_UNIFORM_SIZE);
        assert_eq!(draw_floats(Mat4::IDENTITY, Vec3::ZERO, 1.0).len() as u64 * 4, DRAW_UNIFORM_SIZE);
    }

    #[test]
    fn camera_packs_view_proj_inverse_sun_then_eye() {
        let view_proj = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let sun_view_proj = Mat4::from_translation(Vec3::new(7.0, 8.0, 9.0));
        let camera = Camera { view_proj, eye: Vec3::new(4.0, 5.0, 6.0) };
        let floats = camera_floats(&camera, sun_view_proj);
        assert_eq!(&floats[0..16], &view_proj.to_cols_array());
        assert_eq!(&floats[16..32], &view_proj.inverse().to_cols_array());
        assert_eq!(&floats[32..48], &sun_view_proj.to_cols_array());
        assert_eq!(&floats[48..52], &[4.0, 5.0, 6.0, 0.0]);
    }

    #[test]
    fn light_normalizes_the_sun_direction() {
        let floats = light_floats(&light());
        // Input direction (0, -2, 0) normalizes to (0, -1, 0); band count rides in the w lane.
        assert_eq!(&floats[0..4], &[0.0, -1.0, 0.0, 4.0]);
    }

    #[test]
    fn light_sanitizes_degenerate_inputs() {
        let mut state = light();
        state.sun.direction = Vec3::ZERO;
        state.cel.band_count = 0;
        state.cel.transition_softness = 0.0;
        state.fog.end = state.fog.start; // zero span
        let floats = light_floats(&state);
        assert_eq!(&floats[0..3], &[0.0, -1.0, 0.0]); // zero direction falls back to straight down
        assert_eq!(floats[3], 2.0); // band count floored at 2
        assert_eq!(floats[7], MIN_SOFTNESS); // softness floored above zero
        assert_eq!(floats[19], state.fog.start + MIN_FOG_SPAN); // fog end pushed past start
    }

    #[test]
    fn light_packs_the_fog_enabled_flag_in_the_zenith_w_lane() {
        // The mesh shader reads index 23 (the zenith vec4's w) to decide whether to blend fog.
        let mut state = light();
        assert_eq!(light_floats(&state)[23], 1.0); // enabled
        state.fog.enabled = false;
        assert_eq!(light_floats(&state)[23], 0.0); // off
    }

    #[test]
    fn draw_normal_matrix_is_the_inverse_transpose() {
        // Non-uniform scale (2, 1, 1): the normal matrix must scale x by 1/2, not 2, so a normal
        // on a stretched surface tilts the correct way.
        let floats = draw_floats(Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0)), Vec3::ONE, 1.0);
        assert_eq!(floats[16], 0.5); // normal matrix column 0, row 0
        assert_eq!(floats[21], 1.0); // column 1, row 1
        assert_eq!(floats[26], 1.0); // column 2, row 2
    }

    #[test]
    fn draw_packs_opacity_in_the_color_w_lane() {
        let floats = draw_floats(Mat4::IDENTITY, Vec3::new(0.1, 0.2, 0.3), 0.35);
        assert_eq!(&floats[32..36], &[0.1, 0.2, 0.3, 0.35]);
    }

    #[test]
    fn packing_is_deterministic() {
        assert_eq!(light_floats(&light()), light_floats(&light()));
        let m = Mat4::from_rotation_y(0.37);
        assert_eq!(draw_floats(m, Vec3::ONE, 0.5), draw_floats(m, Vec3::ONE, 0.5));
    }
}
