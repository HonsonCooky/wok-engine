//! Swept vertical cylinder vs the static colliders: the flat-bottomed player's continuous
//! collision, and the earliest-impact dispatch over mixed [`Collider`] sets.
//!
//! ## Method: conservative advancement over solid-to-solid closest pairs
//!
//! The capsule sweeps reduce the moving shape to its segment and advance on segment-to-shape
//! distance. A flat-capped cylinder has no segment-plus-radius reduction (its Minkowski core
//! would be a segment plus a horizontal disc, and disc-dilated distance has no closed form
//! against a box), so [`advance_cylinder`] runs the identical conservative-advancement argument
//! one level up: on the distance between the solid cylinder and the static solid. Both are
//! convex, so the gap along the motion is a convex function of time and stepping by
//! `gap / closing_speed` never overshoots true contact - the same convexity argument as
//! [`crate::sweep`], shape-for-shape.
//!
//! The closest pair per static shape comes from the exact projections [`crate::geom`] already
//! owns, composed by alternating projection ([`crate::geom::alternate_pair`]): project onto the
//! static, back onto the cylinder, repeat. Flat faces, walls, and caps settle in a round or two
//! (the direction is constant), so contact there is exact to the loop's `GAP_EPS`, the same bound
//! every sweep in this crate carries.
//!
//! ## The rim fillet (the documented rounding)
//!
//! The capsule loop is robust because its witness vector (segment to shape) is always at least a
//! radius long: the contact normal is the direction of a healthy-length vector. Advancing the
//! bare solid would end with the surfaces `GAP_EPS` apart and the normal read off a
//! micrometre-long vector - noise. So the cylinder borrows the capsule's structure: the
//! advancement erodes the shape to a core (radius and half-height each reduced by the fillet) and
//! declares contact at core-to-shape distance `fillet + skin`. The swept solid is therefore the
//! core dilated by a ball: a cylinder whose flat faces and walls sit EXACTLY at the true surface
//! (erosion and dilation cancel there) and whose rim carries a fillet of [`RIM_FILLET`] (5cm,
//! clamped to half the radius and half-height of small shapes) - it recedes from the sharp corner
//! by at most `(sqrt(2) - 1) * RIM_FILLET`, about 2cm, at the corner bisector. This is the
//! contract's conservatively-rounded rim, chosen deliberately: a razor-sharp rim is a noise
//! amplifier in float, and a couple of centimetres of edge chamfer is imperceptible at player
//! scale (and kinder underfoot than a knife edge).
//!
//! ## Skin
//!
//! The multi-collider sweep takes `skin` directly (the capsule API hides it inside
//! [`crate::slide`]): a character controller owns its skin, and a flat-capped shape cannot fake
//! one by resizing - growing the radius widens but does not lower, and the slide needs the gap in
//! every direction. The skin adds to the fillet's contact distance, which is exactly the
//! Minkowski inflation of the (filleted) cylinder by a ball of `skin`: the body stops `skin`
//! short of every surface, in every direction. `skin = 0.0` is the bare filleted shape.
//!
//! Determinism (canon contract): fixed iteration orders and caps everywhere (the advancement, the
//! projection loop, the slice-order dispatch), no RNG, no parallelism; everything reads relative
//! positions only, so the queries are position-independent to float precision.

use glam::Vec3;
use wok_scene::Aabb;

use crate::collider::Collider;
use crate::cylinder::Cylinder;
use crate::geom::{alternate_pair, closest_point_on_aabb, closest_point_on_cylinder};
use crate::sweep::{CLOSING_EPS, GAP_EPS, MAX_STEPS, MOTION_EPS_SQ, SweptHit, TOUCHING, face_normal};
use crate::sweep_cyl_obb::sweep_cylinder_obb_inflated;
use crate::sweep_cyl_round::{sweep_cylinder_sphere_inflated, sweep_cylinder_vert_cylinder_inflated};

/// Sweep `cylinder` through `delta` against a single static `aabb`; `None` if it never makes
/// contact. Same [`SweptHit`] semantics as every sweep here: true geometric `toi` (within
/// `GAP_EPS`), outward normal pointing shape-to-cylinder, no hit when the motion is not closing.
pub fn sweep_cylinder_aabb(cylinder: &Cylinder, delta: Vec3, aabb: &Aabb) -> Option<SweptHit> {
    sweep_cylinder_aabb_inflated(cylinder, delta, aabb, 0.0)
}

/// Sweep `cylinder` through `delta` against one static [`Collider`], dispatching to the shape's
/// own sweep; `None` if it never makes contact.
pub fn sweep_cylinder_collider(cylinder: &Cylinder, delta: Vec3, collider: &Collider) -> Option<SweptHit> {
    sweep_collider_one(cylinder, delta, collider, 0.0)
}

/// Sweep `cylinder` through `delta` against a mixed set of static colliders with the surfaces
/// inflated by `skin` (see the module docs), returning the earliest impact: the same slice-order,
/// earliest-toi-wins, tie-keeps-the-earlier-index contract as [`crate::sweep_capsule_colliders`].
pub fn sweep_cylinder_colliders(
    cylinder: &Cylinder,
    delta: Vec3,
    colliders: &[Collider],
    skin: f32,
) -> Option<SweptHit> {
    let mut best: Option<SweptHit> = None;
    for collider in colliders {
        if let Some(hit) = sweep_collider_one(cylinder, delta, collider, skin) {
            // Earliest impact wins; an exact tie keeps the earlier-indexed collider.
            match best {
                Some(b) if b.toi <= hit.toi => {}
                _ => best = Some(hit),
            }
        }
    }
    best
}

/// One collider, one sweep: the per-shape dispatch every multi-collider query funnels through.
fn sweep_collider_one(cylinder: &Cylinder, delta: Vec3, collider: &Collider, skin: f32) -> Option<SweptHit> {
    match *collider {
        Collider::Aabb(ref aabb) => sweep_cylinder_aabb_inflated(cylinder, delta, aabb, skin),
        Collider::Sphere { center, radius } => sweep_cylinder_sphere_inflated(cylinder, delta, center, radius, skin),
        Collider::VertCylinder { center, radius, half_height } => {
            sweep_cylinder_vert_cylinder_inflated(cylinder, delta, center, radius, half_height, skin)
        }
        Collider::Obb { center, half_extents, rotation } => {
            sweep_cylinder_obb_inflated(cylinder, delta, center, half_extents, rotation, skin)
        }
    }
}

/// The box pair: alternating projection between the cylinder and the box (both projections are
/// exact and convex, so the fixed point is the closest pair). The deep fallback is the box's own
/// face normal at the cylinder centre; for a centre outside the box (a wall overlap) the
/// least-signed-distance pick degrades to the nearest face's outward normal, still a sane push.
fn sweep_cylinder_aabb_inflated(cylinder: &Cylinder, delta: Vec3, aabb: &Aabb, skin: f32) -> Option<SweptHit> {
    advance_cylinder(
        cylinder,
        delta,
        skin,
        |c| {
            alternate_pair(
                c.center,
                |p| closest_point_on_cylinder(c.center, c.radius, c.half_height, p),
                |p| closest_point_on_aabb(aabb, p),
            )
        },
        |center| face_normal(aabb, center),
    )
}

/// The rim fillet radius, in metres (see the module docs): the erosion-dilation margin that keeps
/// the advancement's witness vector healthy. Clamped per shape so small cylinders never invert.
pub(crate) const RIM_FILLET: f32 = 0.05;

/// The fillet actually applied to `cylinder`: [`RIM_FILLET`], clamped to half the radius and half
/// the half-height (a degenerate disc or axis gets no fillet and accepts noisier normals).
pub(crate) fn fillet_of(cylinder: &Cylinder) -> f32 {
    RIM_FILLET.min(0.5 * cylinder.radius).min(0.5 * cylinder.half_height).max(0.0)
}

/// Conservative advancement of one cylinder against one convex static shape, the shape supplied
/// as its closest-pair query (`closest`, the advancing ERODED core in, `(on_core, on_shape)` out)
/// and a fallback outward normal for the overlapping case (`deep_normal`, given the cylinder
/// centre). The core is the cylinder eroded by [`fillet_of`]; contact is declared at
/// core-to-shape distance `fillet + skin`, which sweeps the filleted solid of the module docs -
/// structurally the capsule advancement with the fillet playing the radius, convexity argument
/// and all.
pub(crate) fn advance_cylinder(
    cylinder: &Cylinder,
    delta: Vec3,
    skin: f32,
    closest: impl Fn(&Cylinder) -> (Vec3, Vec3),
    deep_normal: impl Fn(Vec3) -> Vec3,
) -> Option<SweptHit> {
    if delta.length_squared() <= MOTION_EPS_SQ {
        return None;
    }
    let fillet = fillet_of(cylinder);
    let contact_dist = fillet + skin;
    let mut core = Cylinder::new(cylinder.center, cylinder.radius - fillet, cylinder.half_height - fillet);
    let mut toi = 0.0_f32;

    for _ in 0..MAX_STEPS {
        let (on_core, on_shape) = closest(&core);
        let to_shape = on_shape - on_core;
        let dist = to_shape.length();

        if dist <= TOUCHING {
            // The core itself touches or enters the shape (a deep overlap): the closest direction
            // is degenerate, so use the shape's deep normal for a sane push-out direction.
            return Some(SweptHit { toi, normal: deep_normal(core.center), point: on_shape });
        }

        let toward = to_shape / dist;
        let closing = delta.dot(toward);
        if closing <= CLOSING_EPS {
            // Moving away from or parallel to the shape: the gap never closes, so no impact.
            return None;
        }

        let gap = dist - contact_dist;
        if gap <= GAP_EPS {
            // The filleted (and skin-inflated) surface has met the shape: outward normal points
            // shape -> cylinder, read off a vector at least a fillet long.
            return Some(SweptHit { toi, normal: -toward, point: on_shape });
        }

        // Advance to where the current tangent predicts contact. Convexity guarantees this does
        // not overshoot the true impact (the capsule sweep's argument, on the solid pair).
        let step = gap / closing;
        toi += step;
        if toi > 1.0 {
            return None;
        }
        core = core.translated(delta * step);
    }

    // Did not converge within the cap: a grazing approach the controller can treat as a miss.
    None
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // The standing player cylinder these tests share with the other cylinder sweeps: feet at
    // `feet`, 1.5m tall, 0.5m radius (the flat-bottomed sibling of the capsule suites' player).
    fn player(feet: Vec3) -> Cylinder {
        Cylinder::new(feet + Vec3::new(0.0, 0.75, 0.0), 0.5, 0.75)
    }

    fn wall() -> Aabb {
        Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0))
    }

    #[test]
    fn cylinder_moving_toward_a_wall_stops_at_the_analytic_toi() {
        // Wall face at x = 3, radius 0.5: contact when the centre reaches 2.5, toi 0.5 of +5.
        // A flat face: the advancement converges in two aims, exact to GAP_EPS.
        let hit = sweep_cylinder_aabb(&player(Vec3::ZERO), Vec3::new(5.0, 0.0, 0.0), &wall()).expect("should hit");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
        assert!((hit.point.x - 3.0).abs() < 1e-3, "contact on the face: {:?}", hit.point);
    }

    #[test]
    fn the_flat_bottom_lands_on_a_floor_at_the_analytic_toi() {
        // Floor top at y = 0, base starting at y = 2 falling -4: the flat bottom meets the floor
        // when the base reaches 0, toi 0.5, normal +Y - no curvature, no radius drop.
        let floor = Aabb::new(Vec3::new(-5.0, -1.0, -5.0), Vec3::new(5.0, 0.0, 5.0));
        let hit = sweep_cylinder_aabb(&player(Vec3::new(0.0, 2.0, 0.0)), Vec3::new(0.0, -4.0, 0.0), &floor)
            .expect("should land");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::Y).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn the_flat_bottom_lands_on_a_partial_overlap_at_the_full_face_toi() {
        // The overhang pin at sweep level: the box's edge at x = 0.3 cuts under a disc whose axis
        // (x = 0.5) is already past it - only the rim's inner part is over the box. The flat
        // bottom still contacts at exactly the full-face toi with a flat +Y normal: any overlap
        // bears. A rounded bottom would contact later and tilted, which is the shed this shape
        // retires.
        let ledge = Aabb::new(Vec3::new(-5.0, -1.0, -5.0), Vec3::new(0.3, 0.0, 5.0));
        let hit = sweep_cylinder_aabb(&player(Vec3::new(0.5, 2.0, 0.0)), Vec3::new(0.0, -4.0, 0.0), &ledge)
            .expect("the overlapped rim should land");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::Y).length() < 1e-4, "normal = {:?}", hit.normal);
        assert!(hit.point.x <= 0.3 + 1e-3, "contact on the box top, not past its edge: {:?}", hit.point);
    }

    #[test]
    fn out_of_reach_behind_parallel_or_zero_motion_misses() {
        let c = player(Vec3::ZERO);
        let w = wall();
        assert!(sweep_cylinder_aabb(&c, Vec3::new(1.0, 0.0, 0.0), &w).is_none(), "too short");
        assert!(sweep_cylinder_aabb(&c, Vec3::new(-5.0, 0.0, 0.0), &w).is_none(), "moving away");
        assert!(sweep_cylinder_aabb(&c, Vec3::ZERO, &w).is_none(), "zero motion");
        // A graze: passing beside the wall with the lateral gap a hair open never closes it.
        let beside = player(Vec3::new(0.0, 0.0, 1.51));
        assert!(sweep_cylinder_aabb(&beside, Vec3::new(10.0, 0.0, 0.0), &w).is_none(), "a graze must miss");
    }

    #[test]
    fn a_cylinder_starting_inside_a_box_resolves_at_toi_zero() {
        let c = player(Vec3::new(0.0, -0.75, 0.0));
        let box_around = Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0));
        let hit = sweep_cylinder_aabb(&c, Vec3::new(5.0, 0.0, 0.0), &box_around).expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!(hit.normal.is_finite(), "normal must be finite: {:?}", hit.normal);
        assert!((hit.normal.length() - 1.0).abs() < 1e-5, "normal must be unit: {:?}", hit.normal);
    }

    #[test]
    fn a_skin_separated_flush_wall_blocks_inward_and_frees_parallel_motion() {
        // The slide's resting configuration: the cylinder holds a skin of separation. Moving
        // along the wall the gap never closes (no impact, the slide keeps moving); moving inward
        // the skin-inflated surface is already met, an immediate stop.
        let skin = 1e-3;
        let c = player(Vec3::new(2.5 - skin, 0.0, 0.0));
        let statics = [Collider::from(wall())];
        assert!(
            sweep_cylinder_colliders(&c, Vec3::new(0.0, 0.0, 5.0), &statics, skin).is_none(),
            "flush parallel motion must slide freely"
        );
        let hit = sweep_cylinder_colliders(&c, Vec3::new(5.0, 0.0, 0.0), &statics, skin).expect("inward is a hit");
        assert!(hit.toi < 1e-4, "flush inward stops immediately: toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn the_earliest_impact_wins_across_mixed_collider_shapes() {
        // The dispatch contract: earliest toi over the slice, in slice order. A sphere in front
        // of a box must win whichever the slice lists first.
        let c = player(Vec3::ZERO);
        let near_sphere = Collider::Sphere { center: Vec3::new(3.0, 0.75, 0.0), radius: 1.0 }; // toi 0.3
        let far_box = Collider::Aabb(Aabb::new(Vec3::new(6.0, 0.0, -1.0), Vec3::new(7.0, 3.0, 1.0)));
        let hit = sweep_cylinder_colliders(&c, Vec3::new(5.0, 0.0, 0.0), &[far_box, near_sphere], 0.0)
            .expect("hits the sphere");
        assert!((hit.toi - 0.3).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn the_aabb_sweep_is_deterministic_and_position_independent() {
        let d = Vec3::new(5.0, -0.3, 0.2);
        let here = sweep_cylinder_aabb(&player(Vec3::ZERO), d, &wall());
        assert_eq!(here, sweep_cylinder_aabb(&player(Vec3::ZERO), d, &wall()), "bitwise reproduction");

        // Chunk-local position-independence: offset cylinder and box together; the qualitative
        // contact must match to float precision.
        let offset = Vec3::new(128.0, 0.0, -256.0);
        let shifted = Aabb::new(wall().min + offset, wall().max + offset);
        let there = sweep_cylinder_aabb(&player(offset), d, &shifted);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        assert!((a.normal - b.normal).length() < 1e-4, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }
}
