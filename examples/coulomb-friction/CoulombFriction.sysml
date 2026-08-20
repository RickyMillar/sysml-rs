package CoulombFriction {
    doc /* Sliding block with smoothed Coulomb friction.
     *
     * Physics: F_friction = -mu*N*sign(v), smoothed near v=0 to avoid
     * discontinuity: F = -mu*m*g*(v / sqrt(v^2 + v_thresh^2))
     *
     * ODE system (1D sliding block on flat surface):
     *   dx/dt = v
     *   dv/dt = (F_applied - mu*m*g*(v / sqrt(v^2 + v_thresh^2))) / m
     *
     * Where:
     *   mu = 0.3 (kinetic friction coefficient, steel on steel)
     *   m = 1.0 kg
     *   g = 9.81 m/s^2
     *   v_thresh = 0.01 m/s (smoothing threshold)
     *   F_applied = 5.0 N (constant push)
     *
     * Expected: block accelerates until friction balances applied force.
     * Steady state: v_ss where mu*m*g ~ F_applied -> impossible if mu*m*g > F_applied.
     * With mu=0.3, m=1, g=9.81: F_friction_max = 2.943N < 5N -> block accelerates.
     *
     * Uses spec-standard StateSpaceRepresentation pattern:
     * calc def :> GetDerivative defines dx/dt = f(u, x). */

    private import ISQ::*;
    private import SI::*;
    private import StateSpaceRepresentation::*;

    part def SlidingBlock {
        // Parameters
        attribute mass : MassValue default 1.0;
        attribute frictionCoeff : Real default 0.3;
        attribute gravity : AccelerationValue default 9.81;
        attribute smoothingThreshold : Real default 0.01;
        attribute appliedForce : ForceValue default 5.0;

        // ODE state variables (direction=out marks them as integrated)
        out attribute position : LengthValue default 0.0;
        out attribute velocity : SpeedValue default 1.0;

        // SSR derivatives — spec-standard GetDerivative pattern
        calc def PositionDerivative :> GetDerivative {
            return dxdt = velocity;
        }

        calc def VelocityDerivative :> GetDerivative {
            return dvdt = (appliedForce - frictionCoeff * mass * gravity * velocity / sqrt(velocity * velocity + smoothingThreshold * smoothingThreshold)) / mass;
        }
    }
}
