package PumpScenarios {
    doc /* Synthetic downstream-restriction scenarios.
     *
     * The restriction is a dimensionless discharge conductance applied to the
     * accumulator outflow (`restrictionConductance` on ReciprocatingPump). It is
     * NOT a product rating — it is a synthetic multiplier chosen only to place
     * the accumulator's settling pressure on either side of the relief warning
     * threshold:
     *
     *   nominal   free discharge   -> accumulator settles below pWarning (safe)
     *   moderate  partial restrict -> accumulator settles near  pWarning (marginal)
     *   severe    heavy restrict   -> accumulator settles above pWarning (relieves)
     *
     * A verification harness applies one of these as a `restrictionConductance`
     * override on the pump session; the model itself carries the nominal value
     * as the default so an un-overridden run is the safe baseline. */

    private import ScalarValues::*;

    attribute nominalConductance : Real = 1.5;
    attribute moderateConductance : Real = 0.6;
    attribute severeConductance : Real = 0.3;
}
