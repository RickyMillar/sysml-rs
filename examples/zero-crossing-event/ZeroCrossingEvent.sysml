package ZeroCrossingEvent {
    doc /* Zero-crossing event detection demo.
     *
     * A simple temperature system where a heater raises temperature
     * linearly from 20 degC. A ZeroCrossingEventDef fires when the
     * temperature crosses 100 degC (boiling point threshold).
     *
     * This exercises the spec-typed ZeroCrossingEventDef auto-wire
     * path end-to-end through ContinuousStateSpaceDynamics.
     *
     * Physics:
     *   dT/dt = heaterRate  (constant heating)
     *   Event: when T - 100 crosses zero from below (Rising) */

    private import ISQ::*;
    private import SI::*;
    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def HeatedVessel {
        doc /* Vessel with constant-rate heater and boiling-point event. */

        // Parameters
        attribute heaterRate : Real default 5.0;
        attribute boilingPoint : Real default 100.0;

        // ODE state variable
        out attribute temperature : Real default 20.0;

        // Formal composite SSR dynamics action
        action def HeatingDynamics :> ContinuousStateSpaceDynamics {
            doc /* Continuous heating dynamics: constant rate heater. */

            // Derivative: dT/dt = heaterRate
            calc def TemperatureDerivative :> GetDerivative {
                return dTdt = heaterRate;
            }

            // Algebraic output: overshoot above boiling point
            calc def OvershootOutput :> GetOutput {
                return overshoot = temperature - boilingPoint;
            }

            // Zero-crossing event: boiling point reached
            // g(t, y, ctx) = temperature - boilingPoint
            // Fires on rising edge (temperature crosses from below)
            event occurrence boilingReached : ZeroCrossingEventDef;
        }
    }
}
