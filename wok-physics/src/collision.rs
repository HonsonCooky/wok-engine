//! AABB-vs-AABB collision: overlap, minimum translation, and sequential resolution against statics.
//!
//! [`aabb_contact`] is the core query: it detects penetration and returns the minimum translation
//! vector (MTV), the smallest move that ends it. [`resolve_statics`] applies that, one static box at
//! a time in a caller-defined order, to push a player box clear.
//!
//! Determinism (canon contract): the MTV picks the axis of least penetration with a fixed tie-break
//! (x, then y, then z), and resolution walks the statics sequentially with no parallel reduction, so
//! the result is reproducible bit-for-bit. The math reads only relative positions (interval overlaps
//! and the sign of the centre difference), so it is position-independent: translating every input by
//! the same vector translates the output by that vector and changes nothing else.

use glam::Vec3;
use wok_scene::Aabb;

use crate::bounds::{aabb_center, aabb_translated};

/// The minimum translation that separates two overlapping AABBs: the smallest move that ends the
/// penetration.
///
/// `normal` is the unit axis the first box must travel along to exit the second, and `depth` is how
/// far. Moving the first box by `normal * depth` brings the pair to a just-touching rest. Only ever
/// produced for genuine penetration, so `depth` is strictly positive; a gap or a bare touch is "no
/// contact" and yields `None` from [`aabb_contact`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub normal: Vec3,
    pub depth: f32,
}

/// Whether two AABBs overlap, by strict interval overlap on all three axes. Boxes that merely touch
/// (share a face or edge with zero overlap) are not overlapping, matching [`aabb_contact`] returning
/// `None` for the same configuration. Cheaper than [`aabb_contact`] when only the yes/no is needed.
pub fn aabb_overlap(a: &Aabb, b: &Aabb) -> bool {
    a.min.x < b.max.x
        && a.max.x > b.min.x
        && a.min.y < b.max.y
        && a.max.y > b.min.y
        && a.min.z < b.max.z
        && a.max.z > b.min.z
}

/// The minimum translation that separates `a` from `b`, or `None` if they are not penetrating.
///
/// The MTV separates along the axis of least penetration, the cheapest way out. On a tie the axes
/// are tried in the fixed order x, y, z, and when the two centres coincide on the chosen axis the
/// normal points along `+axis`; both rules keep the result deterministic.
pub fn aabb_contact(a: &Aabb, b: &Aabb) -> Option<Contact> {
    // Overlap of the two intervals on each axis. A non-positive value is a gap (or a bare touch) on
    // that axis, which means no penetration anywhere.
    let overlap = Vec3::new(
        a.max.x.min(b.max.x) - a.min.x.max(b.min.x),
        a.max.y.min(b.max.y) - a.min.y.max(b.min.y),
        a.max.z.min(b.max.z) - a.min.z.max(b.min.z),
    );
    if overlap.x <= 0.0 || overlap.y <= 0.0 || overlap.z <= 0.0 {
        return None;
    }

    // Axis of least penetration; ties resolve x, then y, then z.
    let (axis, depth) = if overlap.x <= overlap.y && overlap.x <= overlap.z {
        (Axis::X, overlap.x)
    } else if overlap.y <= overlap.z {
        (Axis::Y, overlap.y)
    } else {
        (Axis::Z, overlap.z)
    };

    // Point the normal so `a` moves away from `b`. With centres equal on the axis, default to +.
    let ca = aabb_center(a);
    let cb = aabb_center(b);
    let normal = match axis {
        Axis::X => Vec3::new(sign_away(ca.x, cb.x), 0.0, 0.0),
        Axis::Y => Vec3::new(0.0, sign_away(ca.y, cb.y), 0.0),
        Axis::Z => Vec3::new(0.0, 0.0, sign_away(ca.z, cb.z)),
    };
    Some(Contact { normal, depth })
}

/// Resolve a player box against a set of static boxes, returning the corrected box.
///
/// The statics are resolved sequentially in slice order (the determinism contract's "defined
/// order"): each penetrating static pushes the player out along its MTV, and the next static sees
/// the already-moved box. The corrected position is the centre of the returned box ([`aabb_center`]);
/// the per-step correction the game uses to clamp velocity is `returned.min - player.min` (resolution
/// only translates, so any corner difference is the same vector).
///
/// Order dependence in a tight corner is inherent to one-at-a-time resolution and is the price of a
/// deterministic, defined-order pass; it is the intended behaviour for this step.
pub fn resolve_statics(player: Aabb, statics: &[Aabb]) -> Aabb {
    let mut p = player;
    for s in statics {
        if let Some(c) = aabb_contact(&p, s) {
            p = aabb_translated(&p, c.normal * c.depth);
        }
    }
    p
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
    Z,
}

/// `-1` when `a` sits on the low side of `b` (so `a` exits toward `-axis`), `+1` otherwise. Equal
/// centres default to `+1`.
fn sign_away(a: f32, b: f32) -> f32 {
    if a < b { -1.0 } else { 1.0 }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn box_at(center: Vec3, half: Vec3) -> Aabb {
        Aabb::from_center_extents(center, half)
    }

    // ---- overlap / no overlap ----

    #[test]
    fn separated_boxes_do_not_overlap() {
        let a = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let b = box_at(Vec3::new(3.0, 0.0, 0.0), Vec3::splat(1.0));
        assert!(!aabb_overlap(&a, &b));
        assert!(aabb_contact(&a, &b).is_none());
    }

    #[test]
    fn touching_faces_are_not_overlap() {
        // a.max.x == b.min.x == 1.0: a bare touch, zero penetration.
        let a = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let b = box_at(Vec3::new(2.0, 0.0, 0.0), Vec3::splat(1.0));
        assert!(!aabb_overlap(&a, &b));
        assert!(aabb_contact(&a, &b).is_none());
    }

    // ---- MTV axis and depth ----

    #[test]
    fn mtv_picks_the_least_penetrating_axis() {
        // a is the unit-ish box at the origin; b overlaps it shallowly in y, deeply in x and z.
        let a = box_at(Vec3::ZERO, Vec3::splat(1.0)); // [-1,1]^3
        let b = box_at(Vec3::new(1.5, 1.8, 0.0), Vec3::splat(1.0)); // min (0.5,0.8,-1)
        // overlaps: x = 0.5, y = 0.2, z = 2.0 -> least is y.
        let c = aabb_contact(&a, &b).expect("boxes overlap");
        assert_eq!(c.normal, Vec3::new(0.0, -1.0, 0.0), "a is below b, exits -y");
        assert!((c.depth - 0.2).abs() < 1e-6, "depth = {}", c.depth);
    }

    #[test]
    fn mtv_normal_flips_with_relative_position() {
        let a = box_at(Vec3::ZERO, Vec3::splat(1.0));
        // b to the left of a, shallow x overlap.
        let b = box_at(Vec3::new(-1.5, 0.0, 0.0), Vec3::splat(1.0));
        let c = aabb_contact(&a, &b).expect("boxes overlap");
        assert_eq!(c.normal, Vec3::new(1.0, 0.0, 0.0), "a is right of b, exits +x");
        assert!((c.depth - 0.5).abs() < 1e-6, "depth = {}", c.depth);
    }

    #[test]
    fn mtv_tie_breaks_x_then_y_then_z() {
        // Equal overlap on every axis: the tie-break must choose x.
        let a = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let b = box_at(Vec3::splat(1.5), Vec3::splat(1.0)); // overlap 0.5 on x, y, z
        let c = aabb_contact(&a, &b).expect("boxes overlap");
        assert_eq!(c.normal, Vec3::new(-1.0, 0.0, 0.0));
        assert!((c.depth - 0.5).abs() < 1e-6);
    }

    // ---- resolution against several statics ----

    #[test]
    fn player_resolves_to_a_known_non_penetrating_position() {
        // Player sunk into a floor and clipping a wall. Floor first, then wall (defined order).
        let player = box_at(Vec3::new(0.3, 0.3, 0.0), Vec3::splat(0.5));
        let floor = box_at(Vec3::new(0.0, -0.5, 0.0), Vec3::new(5.0, 0.5, 5.0));
        let wall = box_at(Vec3::new(1.0, 0.5, 0.0), Vec3::new(0.5, 2.0, 5.0));

        let resolved = resolve_statics(player, &[floor, wall]);

        // Floor lifts the player to y = 0.5 (sitting on the floor top), wall pushes it back to x = 0.
        let c = aabb_center(&resolved);
        let eps = 1e-6;
        assert!((c.x - 0.0).abs() < eps, "x = {}", c.x);
        assert!((c.y - 0.5).abs() < eps, "y = {}", c.y);
        assert!((c.z - 0.0).abs() < eps, "z = {}", c.z);

        // And it no longer penetrates either static (touching is allowed).
        assert!(aabb_contact(&resolved, &floor).is_none());
        assert!(aabb_contact(&resolved, &wall).is_none());
    }

    #[test]
    fn empty_statics_leave_the_player_untouched() {
        let player = box_at(Vec3::new(1.0, 2.0, 3.0), Vec3::splat(0.5));
        assert_eq!(resolve_statics(player, &[]), player);
    }

    // ---- determinism and position independence ----

    #[test]
    fn resolution_is_deterministic() {
        let player = box_at(Vec3::new(0.3, 0.3, 0.1), Vec3::splat(0.5));
        let statics = [
            box_at(Vec3::new(0.0, -0.5, 0.0), Vec3::new(5.0, 0.5, 5.0)),
            box_at(Vec3::new(1.0, 0.5, 0.0), Vec3::new(0.5, 2.0, 5.0)),
        ];
        assert_eq!(
            resolve_statics(player, &statics),
            resolve_statics(player, &statics)
        );
    }

    #[test]
    fn contact_is_position_independent() {
        let a = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let b = box_at(Vec3::new(1.5, 1.8, 0.0), Vec3::splat(1.0));
        let here = aabb_contact(&a, &b).unwrap();

        // Shift both boxes to a far chunk: same penetration, same answer. The qualitative result
        // (the contact axis and direction) is exact; the depth, being a difference of now-large
        // coordinates, matches only to float precision, which is all position-independence can mean
        // once the query runs in chunk-local space rather than at the world origin.
        let offset = Vec3::new(1000.0, -500.0, 750.0);
        let a2 = aabb_translated(&a, offset);
        let b2 = aabb_translated(&b, offset);
        let there = aabb_contact(&a2, &b2).unwrap();

        assert_eq!(here.normal, there.normal);
        assert!((here.depth - there.depth).abs() < 1e-3, "{} vs {}", here.depth, there.depth);
    }

    #[test]
    fn resolution_is_position_independent() {
        let player = box_at(Vec3::new(0.3, 0.3, 0.0), Vec3::splat(0.5));
        let floor = box_at(Vec3::new(0.0, -0.5, 0.0), Vec3::new(5.0, 0.5, 5.0));
        let wall = box_at(Vec3::new(1.0, 0.5, 0.0), Vec3::new(0.5, 2.0, 5.0));
        let offset = Vec3::new(512.0, 128.0, -64.0);

        let here = resolve_statics(player, &[floor, wall]);
        let there = resolve_statics(
            aabb_translated(&player, offset),
            &[aabb_translated(&floor, offset), aabb_translated(&wall, offset)],
        );

        // Same correction, shifted by the offset (to float precision; see the contact test note).
        let drift = (aabb_center(&there) - (aabb_center(&here) + offset)).length();
        assert!(drift < 1e-3, "resolution drifted by {drift} under a world offset");
    }
}
