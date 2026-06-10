//! Fitting the sun's orthographic shadow projection to the caller's world-space region.
//!
//! The region is caller policy (the AABB of whatever the caller's loaded content occupies); this
//! module owns the mechanics: look along the sun, bound the region in light space, and produce the
//! view-projection the shadow pass renders with and the mesh pass samples by. Two details carry
//! the stability of the whole shadow map:
//!
//! - The ortho window snaps to whole shadow-map texels (see `snap_axis`). Without the snap, any
//!   sub-texel movement of the region re-rasterizes every caster against a shifted texel grid and
//!   the shadow edges crawl; with it, the window only ever moves in whole texels, so a static sun
//!   over a moving region keeps every shadow edge still.
//! - The near plane is padded toward the sun (`NEAR_PAD_M`), so a caster slightly above the
//!   region (a jumping player, a placement poking past authored bounds) still lands in the map
//!   instead of being clipped and silently losing its shadow.
//!
//! Pure math: deterministic, testable without a GPU.

use glam::{Mat4, Vec3};
use wok_scene::Aabb;

/// Extra metres of depth range toward the sun, past the region's nearest corner. Costs only depth
/// precision (negligible against a 32-bit depth map over chunk-scale spans); buys shadows from
/// casters slightly above the region.
const NEAR_PAD_M: f32 = 8.0;

/// Extra metres of depth range past the region's farthest corner, so a receiver exactly on the
/// boundary plane does not sit at depth 1.0 where clipping and the compare get twitchy.
const FAR_PAD_M: f32 = 1.0;

/// Floor for the light-space window extent, so a degenerate region (a single flat plane seen
/// edge-on, an empty scene's fallback box) still yields an invertible projection.
const MIN_EXTENT_M: f32 = 1.0;

/// The sun's view-projection: an orthographic frustum fitted to `region` looking along `sun_dir`
/// (the travel direction of sunlight, normalized by the caller). `shadow_map_size` is the map's
/// resolution in texels; the texel snap is computed against it.
pub(crate) fn sun_view_proj(sun_dir: Vec3, region: Aabb, shadow_map_size: u32) -> Mat4 {
    // The basis comes from the sun direction alone, never from the region, so the view does not
    // twist as the region moves. The up fallback handles a sun pointing straight down (the
    // default LightState), where Y would be degenerate.
    let up = if sun_dir.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_to_rh(Vec3::ZERO, sun_dir, up);

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for corner in corners(region) {
        let p = view.transform_point3(corner);
        min = min.min(p);
        max = max.max(p);
    }

    let res = shadow_map_size.max(2) as f32;
    let (left, right) = snap_axis(min.x, max.x, res);
    let (bottom, top) = snap_axis(min.y, max.y, res);

    // Right-handed view space puts the scene at negative z; orthographic_rh takes near and far as
    // positive distances along -z, so the padded z bounds negate.
    let near = -(max.z + NEAR_PAD_M);
    let far = -(min.z - FAR_PAD_M);
    Mat4::orthographic_rh(left, right, bottom, top, near, far) * view
}

/// One light-space axis of the ortho window. The texel size is fixed by the region's extent
/// (`extent / (res - 1)`), the window is exactly `res` texels (one texel wider than the region, so
/// snapping down can never cut the region's far edge off), and the window origin snaps down to a
/// whole texel. Because the snap step equals the rendered texel size exactly, the window only
/// moves in whole texels as the region translates - the anti-shimmer property.
fn snap_axis(min: f32, max: f32, res: f32) -> (f32, f32) {
    let extent = (max - min).max(MIN_EXTENT_M);
    let texel = extent / (res - 1.0);
    let lo = (min / texel).floor() * texel;
    (lo, lo + texel * res)
}

/// The eight corners of an AABB.
fn corners(b: Aabb) -> [Vec3; 8] {
    [
        Vec3::new(b.min.x, b.min.y, b.min.z),
        Vec3::new(b.max.x, b.min.y, b.min.z),
        Vec3::new(b.min.x, b.max.y, b.min.z),
        Vec3::new(b.max.x, b.max.y, b.min.z),
        Vec3::new(b.min.x, b.min.y, b.max.z),
        Vec3::new(b.max.x, b.min.y, b.max.z),
        Vec3::new(b.min.x, b.max.y, b.max.z),
        Vec3::new(b.max.x, b.max.y, b.max.z),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const RES: u32 = 2048;

    fn region() -> Aabb {
        Aabb::new(Vec3::new(-3.0, 0.0, 10.0), Vec3::new(40.0, 9.0, 60.0))
    }

    fn sun() -> Vec3 {
        Vec3::new(-0.4, -1.0, -0.3).normalize()
    }

    #[test]
    fn the_whole_region_projects_inside_the_map() {
        let m = sun_view_proj(sun(), region(), RES);
        for corner in corners(region()) {
            let ndc = m.project_point3(corner);
            assert!(ndc.x.abs() <= 1.0, "corner {corner} ndc.x {} outside the map", ndc.x);
            assert!(ndc.y.abs() <= 1.0, "corner {corner} ndc.y {} outside the map", ndc.y);
            assert!(ndc.z > 0.0 && ndc.z < 1.0, "corner {corner} depth {} outside (0, 1)", ndc.z);
        }
    }

    #[test]
    fn a_vertical_sun_is_not_degenerate() {
        // The default LightState's sun points straight down, where the usual Y-up view basis has
        // no cross product; the fallback basis must keep the fit finite and covering.
        let m = sun_view_proj(Vec3::NEG_Y, region(), RES);
        assert!(m.to_cols_array().iter().all(|v| v.is_finite()));
        for corner in corners(region()) {
            let ndc = m.project_point3(corner);
            assert!(ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && ndc.z > 0.0 && ndc.z < 1.0);
        }
    }

    #[test]
    fn sub_texel_region_movement_keeps_the_texel_grid_fixed() {
        // The anti-shimmer property, asserted directly: nudge the region by a fraction of a texel
        // and a fixed world point must land at the same sub-texel position in the map (the window
        // moved by zero or whole texels), so its shadow edge does not crawl.
        let p = Vec3::new(11.3, 2.4, 27.9);
        let nudge = Vec3::new(0.011, 0.0, 0.007);
        let moved = Aabb::new(region().min + nudge, region().max + nudge);
        let res = RES as f32;
        let texel_coords = |m: Mat4| (m.project_point3(p).truncate() * 0.5 + 0.5) * res;
        let a = texel_coords(sun_view_proj(sun(), region(), RES));
        let b = texel_coords(sun_view_proj(sun(), moved, RES));
        let frac = |v: f32| v - v.round();
        assert!(
            (frac(a.x - b.x)).abs() < 1e-2 && (frac(a.y - b.y)).abs() < 1e-2,
            "texel coords {a} vs {b} differ by a non-whole number of texels"
        );
    }

    #[test]
    fn nearer_the_sun_means_smaller_depth() {
        // Depth ordering is what the shadow compare relies on: a caster between the sun and a
        // receiver must store a smaller depth than the receiver would.
        let m = sun_view_proj(sun(), region(), RES);
        let receiver = Vec3::new(20.0, 0.0, 30.0);
        let caster = receiver - sun() * 5.0; // 5m toward the sun
        assert!(m.project_point3(caster).z < m.project_point3(receiver).z);
    }

    #[test]
    fn fitting_is_deterministic() {
        let a = sun_view_proj(sun(), region(), RES);
        let b = sun_view_proj(sun(), region(), RES);
        assert_eq!(a.to_cols_array(), b.to_cols_array());
    }
}
