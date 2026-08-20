package QuantitySnapshot {
    doc /* RSC-5.4 proving fixture — explicit-[unit] ISQ slot in a stepping model.
     *
     * Minimal sliding-block ODE (same proven-stepping structure as
     * coulomb-friction) plus ONE constant attribute carrying an explicit
     * `[unit]` annotation (`refTemp = 273.15 [K]`). The explicit unit makes
     * the slot's MeasurementRef carry `unit: Some("K")` (the `infer_m_ref`
     * path #1, d2f0dc2b) — unlike type-only ISQ attributes whose m_ref unit
     * is `None`.
     *
     * The ODE supplies the stepping; `refTemp` stays constant, so its slot's
     * m_ref unit is stable across ticks. RSC-5.4 surfaces that unit through
     * the snapshot sinks. */

    private import ISQ::*;
    private import SI::*;
    private import StateSpaceRepresentation::*;

    part def Block {
        attribute mass : MassValue default 1.0;
        attribute appliedForce : ForceValue default 5.0;

        // Explicit-[unit] constant ISQ slot — the unit-bearing slot under test.
        attribute refTemp : TemperatureValue default 273.15 [K];

        // ODE state variable (direction=out marks it integrated).
        out attribute velocity : SpeedValue default 1.0;

        calc def VelocityDerivative :> GetDerivative {
            return dvdt = appliedForce / mass;
        }
    }
}
