package BouncingBall {
    doc /* Hybrid automaton bouncing ball.
     *
     * Physics:
     *   Flight mode:  dy/dt = v,  dv/dt = -g
     *   Bounce event: when y <= 0, v resets to -restitution * v
     *
     * Uses spec-standard StateSpaceRepresentation composite pattern:
     *   action def :> ContinuousStateSpaceDynamics bundles
     *   getDerivative, getOutput, and zeroCrossingEvents.
     *
     * Initial conditions: y = 10.0 m, v = 0.0 m/s */

    private import ISQ::*;
    private import SI::*;
    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def BouncingBallPart {
        doc /* Hybrid bouncing ball with continuous flight ODE.
         * Two state variables integrated by the SSR pattern. */

        // Parameters
        attribute g : Real default 9.81;
        attribute restitution : Real default 0.8;

        // ODE state variables (direction=out marks them as integrated)
        out attribute y : Real default 10.0;
        out attribute v : Real default 0.0;

        // Formal composite SSR dynamics action
        action def FlightDynamics :> ContinuousStateSpaceDynamics {
            doc /* Continuous flight dynamics: free-fall under gravity. */

            // Derivatives — spec-standard GetDerivative pattern
            calc def HeightDerivative :> GetDerivative {
                return dydt = v;
            }

            calc def VelocityDerivative :> GetDerivative {
                return dvdt = 0 - g;
            }

            // Algebraic output: kinetic energy (demonstrates GetOutput)
            calc def KineticEnergyOutput :> GetOutput {
                return kinetic_energy = 0.5 * v * v;
            }

            // Zero-crossing event: ground collision
            // Fires when height y crosses zero from above (y <= 0)
            event occurrence groundCollision : ZeroCrossingEventDef;
        }
    }
}
