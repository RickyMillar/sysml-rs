package HydraulicConstraints {
    doc /* Static well-formedness constraints on the pump parameters and states.
     * These are checkable with `sysml check` and document the physical envelope
     * the model is designed to stay inside. They reference the pump attributes
     * as free names, resolved from the model/session context. */

    private import ScalarValues::*;

    constraint def PositiveConductance {
        doc /* The discharge conductance must be strictly positive; a zero or
         * negative value would make the accumulator outflow ill-defined. */
        restrictionConductance > 0.0
    }

    constraint def NonNegativeThresholds {
        doc /* The warning and trip thresholds are non-negative pressures /
         * exposures. */
        pWarning >= 0.0 and exposureTrip > 0.0
    }

    constraint def RegularizedRoot {
        doc /* The square-root regularization floor is strictly positive, so the
         * orifice and restriction flows have bounded slope at zero pressure
         * difference (1 / (2*sqrt(epsRoot))). */
        epsRoot > 0.0
    }
}
