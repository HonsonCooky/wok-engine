//! Fixed-step kinematic integration: advance a position and velocity one step under acceleration.
//!
//! [`integrate`] is one step. The game owns the fixed-timestep loop and the body's state and calls
//! this each step with a fixed `dt`; passing a fixed `dt` (never a wall-clock delta) is the day-one
//! decision behind deterministic scripted-input replay.

use glam::Vec3;

/// Position and velocity of a point mass: the state a fixed-step integration carries forward. The
/// game stores this (or its own equivalent) on each actor; wok-physics never holds it between calls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub position: Vec3,
    pub velocity: Vec3,
}

/// Advance `motion` by one fixed step of `dt` seconds under constant `acceleration`.
///
/// The update is `position += velocity*dt + 0.5*acceleration*dt*dt` using the velocity from the
/// start of the step, then `velocity += acceleration*dt`. For a constant acceleration this
/// reproduces the closed-form kinematics `x = x0 + v0*t + 0.5*a*t*t` and `v = v0 + a*t` exactly in
/// real arithmetic (it is not a drifting first-order Euler step), which is the dominant case for a
/// body falling under gravity. Deterministic: identical `motion`, `acceleration` and `dt` give an
/// identical result, with no wall-clock and no stored state.
pub fn integrate(motion: Motion, acceleration: Vec3, dt: f32) -> Motion {
    Motion {
        position: motion.position + motion.velocity * dt + 0.5 * acceleration * dt * dt,
        velocity: motion.velocity + acceleration * dt,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn run(mut motion: Motion, acceleration: Vec3, dt: f32, steps: u32) -> Motion {
        for _ in 0..steps {
            motion = integrate(motion, acceleration, dt);
        }
        motion
    }

    #[test]
    fn single_step_matches_the_scheme_formula() {
        // Guards the exact update the scheme promises (and so the closed-form property that rests
        // on it). Same expression as the implementation, so this is a bit-exact regression check.
        let dt = 1.0 / 60.0;
        let a = Vec3::new(0.0, -9.8, 0.0);
        let m0 = Motion { position: Vec3::new(0.0, 100.0, 0.0), velocity: Vec3::new(1.0, 2.0, 3.0) };
        let m1 = integrate(m0, a, dt);
        assert_eq!(m1.velocity, m0.velocity + a * dt);
        assert_eq!(m1.position, m0.position + m0.velocity * dt + 0.5 * a * dt * dt);
    }

    #[test]
    fn zero_dt_is_a_no_op() {
        let m0 = Motion { position: Vec3::new(5.0, 6.0, 7.0), velocity: Vec3::new(-1.0, 0.0, 2.0) };
        assert_eq!(integrate(m0, Vec3::new(0.0, -9.8, 0.0), 0.0), m0);
    }

    #[test]
    fn position_matches_closed_form_under_constant_acceleration() {
        let dt = 1.0 / 120.0;
        let steps = 240; // t = 2.0s
        let m0 = Motion { position: Vec3::new(2.0, 100.0, -5.0), velocity: Vec3::new(1.0, 0.0, -3.0) };
        let a = Vec3::new(0.0, -9.8, 0.5);

        let m = run(m0, a, dt, steps);

        let t = steps as f32 * dt;
        let expected_pos = m0.position + m0.velocity * t + 0.5 * a * t * t;
        let expected_vel = m0.velocity + a * t;
        assert!((m.position - expected_pos).length() < 1e-2, "pos {:?} vs {:?}", m.position, expected_pos);
        assert!((m.velocity - expected_vel).length() < 1e-2, "vel {:?} vs {:?}", m.velocity, expected_vel);
    }

    #[test]
    fn acceleration_applies_per_component() {
        // A general acceleration must integrate independently on each axis.
        let dt = 0.01;
        let steps = 100; // t = 1.0s
        let m0 = Motion { position: Vec3::ZERO, velocity: Vec3::ZERO };
        let a = Vec3::new(3.0, -9.8, 2.0);
        let m = run(m0, a, dt, steps);
        let t = steps as f32 * dt;
        let expected = 0.5 * a * t * t;
        assert!((m.position - expected).length() < 1e-3, "pos {:?} vs {:?}", m.position, expected);
    }

    #[test]
    fn identical_sequences_give_identical_results() {
        let dt = 1.0 / 60.0;
        let m0 = Motion { position: Vec3::new(0.0, 50.0, 0.0), velocity: Vec3::new(2.0, 0.0, -1.0) };
        let a = Vec3::new(0.1, -9.8, 0.0);
        assert_eq!(run(m0, a, dt, 500), run(m0, a, dt, 500));
    }
}
