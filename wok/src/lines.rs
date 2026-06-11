//! Line-cage geometry for collider shapes: the selection highlight's x-ray cage.
//!
//! Pure geometry building, no GPU: collider in, `LineSegment`s out, drawn by wok-render's debug
//! line pass with `DepthMode::XRay` so the cage reads even behind the surface it describes. Each
//! collider draws in its true classified shape (box edges, sphere great circles, cylinder rings
//! and verticals) - drawing a sphere as its box would misstate what the pick and the simulation
//! actually test. The stroke functions mirror the taste app's F1 overlay; applications rebuild
//! presentation like this per-app by design (the engine carries no overlay policy).

use std::f32::consts::TAU;

use glam::{Quat, Vec3};
use wok_physics::Collider;
use wok_render::LineSegment;
use wok_scene::Aabb;

/// Selection cage color: saturated orange, distinct from the F1-style diagnostic palette (pure
/// green hitboxes, pure yellow player capsule) so a selected cage never reads as a diagnostic.
pub const SELECTION_COLOR: Vec3 = Vec3::new(1.0, 0.55, 0.05);

/// Line segments per ring: enough that a metre-scale circle reads round, few enough that the
/// cage stays obviously an overlay.
const RING_SEGMENTS: usize = 16;

/// Verticals on a cylinder cage, evenly spaced around the wall.
const CAGE_VERTICALS: usize = 4;

/// Append one collider's cage in its true shape.
pub fn collider_lines(collider: &Collider, color: Vec3, out: &mut Vec<LineSegment>) {
    match *collider {
        Collider::Aabb(ref aabb) => aabb_lines(aabb, color, out),
        Collider::Sphere { center, radius } => sphere_lines(center, radius, color, out),
        Collider::VertCylinder { center, radius, half_height } => {
            cylinder_lines(center, radius, half_height, color, out);
        }
        Collider::Obb { center, half_extents, rotation } => {
            obb_lines(center, half_extents, rotation, color, out);
        }
    }
}

/// The 12 edges of an AABB.
fn aabb_lines(aabb: &Aabb, color: Vec3, out: &mut Vec<LineSegment>) {
    let (lo, hi) = (aabb.min, aabb.max);
    // Each corner as a bit pattern (x, y, z from lo or hi); an edge joins corners differing in
    // exactly one bit, taken once by only walking toward the hi side.
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                out.push(LineSegment { start: corner(i), end: corner(i | bit), color });
            }
        }
    }
}

/// The 12 edges of an oriented box: the AABB stroke in the box's own frame, each corner rotated
/// out by the collider's rotation, so the cage turns with a yawed crate instead of redrawing the
/// conservative margin the Obb collider removed.
fn obb_lines(center: Vec3, half_extents: Vec3, rotation: Quat, color: Vec3, out: &mut Vec<LineSegment>) {
    let corner = |i: usize| {
        let local = Vec3::new(
            if i & 1 == 0 { -half_extents.x } else { half_extents.x },
            if i & 2 == 0 { -half_extents.y } else { half_extents.y },
            if i & 4 == 0 { -half_extents.z } else { half_extents.z },
        );
        center + rotation * local
    };
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                out.push(LineSegment { start: corner(i), end: corner(i | bit), color });
            }
        }
    }
}

/// A circle of `RING_SEGMENTS` segments in the plane spanned by the orthonormal `u`, `v` around
/// `center`: the one stroke every round cage is drawn with.
fn circle_lines(center: Vec3, u: Vec3, v: Vec3, radius: f32, color: Vec3, out: &mut Vec<LineSegment>) {
    let at = |j: usize| {
        let angle = TAU * (j as f32 / RING_SEGMENTS as f32);
        center + (u * angle.cos() + v * angle.sin()) * radius
    };
    for j in 0..RING_SEGMENTS {
        out.push(LineSegment { start: at(j), end: at(j + 1), color });
    }
}

/// A sphere collider as three orthogonal great circles: the equator plus two meridians, enough
/// that the cage reads round from any camera angle.
fn sphere_lines(center: Vec3, radius: f32, color: Vec3, out: &mut Vec<LineSegment>) {
    circle_lines(center, Vec3::X, Vec3::Z, radius, color, out);
    circle_lines(center, Vec3::X, Vec3::Y, radius, color, out);
    circle_lines(center, Vec3::Z, Vec3::Y, radius, color, out);
}

/// A vertical-cylinder collider as a ring at each cap plus verticals spanning the wall.
fn cylinder_lines(center: Vec3, radius: f32, half_height: f32, color: Vec3, out: &mut Vec<LineSegment>) {
    for y in [-half_height, half_height] {
        circle_lines(center + Vec3::Y * y, Vec3::X, Vec3::Z, radius, color, out);
    }
    for j in 0..CAGE_VERTICALS {
        let angle = TAU * (j as f32 / CAGE_VERTICALS as f32);
        let on_wall = Vec3::new(radius * angle.cos(), 0.0, radius * angle.sin());
        out.push(LineSegment {
            start: center + on_wall - Vec3::Y * half_height,
            end: center + on_wall + Vec3::Y * half_height,
            color,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_collider_shape_draws_its_expected_segment_count() {
        let mut out = Vec::new();
        collider_lines(&Collider::Aabb(Aabb::new(Vec3::ZERO, Vec3::ONE)), SELECTION_COLOR, &mut out);
        assert_eq!(out.len(), 12);

        out.clear();
        collider_lines(&Collider::Sphere { center: Vec3::ZERO, radius: 1.0 }, SELECTION_COLOR, &mut out);
        assert_eq!(out.len(), 3 * RING_SEGMENTS);

        out.clear();
        let cyl = Collider::VertCylinder { center: Vec3::ZERO, radius: 1.0, half_height: 2.0 };
        collider_lines(&cyl, SELECTION_COLOR, &mut out);
        assert_eq!(out.len(), 2 * RING_SEGMENTS + CAGE_VERTICALS);

        out.clear();
        let obb = Collider::Obb { center: Vec3::ZERO, half_extents: Vec3::ONE, rotation: Quat::from_rotation_y(0.5) };
        collider_lines(&obb, SELECTION_COLOR, &mut out);
        assert_eq!(out.len(), 12);
    }

    #[test]
    fn an_obb_cage_rotates_with_its_collider() {
        // Every cage endpoint must be a rotated corner: distance sqrt(3) from the centre (the
        // box's own corner reach), and mapping it back by the inverse rotation lands on a corner
        // of the local box. An axis-aligned cage would fail the map-back check.
        let rotation = Quat::from_rotation_y(0.5);
        let center = Vec3::new(3.0, 1.0, -2.0);
        let mut out = Vec::new();
        collider_lines(
            &Collider::Obb { center, half_extents: Vec3::ONE, rotation },
            SELECTION_COLOR,
            &mut out,
        );
        for seg in &out {
            for p in [seg.start, seg.end] {
                let local = rotation.conjugate() * (p - center);
                assert!(
                    (local.x.abs() - 1.0).abs() < 1e-5
                        && (local.y.abs() - 1.0).abs() < 1e-5
                        && (local.z.abs() - 1.0).abs() < 1e-5,
                    "cage endpoint {p:?} is not a rotated corner (local {local:?})"
                );
            }
        }
    }

    #[test]
    fn cage_segments_lie_on_their_collider_surface() {
        let mut out = Vec::new();
        let center = Vec3::new(3.0, 1.0, -2.0);
        collider_lines(&Collider::Sphere { center, radius: 2.0 }, SELECTION_COLOR, &mut out);
        for seg in &out {
            assert!(((seg.start - center).length() - 2.0).abs() < 1e-4);
        }
    }
}
