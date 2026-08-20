package ManifoldPhysics {
    doc /* Shared compliant hydraulic manifold — the single shared-plant ODE.
     * Independently-derived lumped compliance law (COV RT-ODE-CORE / EX-POWER):
     *
     *   dp_m/dt = Km * ( q_pump - q_draw(p_m) - q_bypass )
     *
     * where the aggregate station draw is folded into a lumped linear
     * conductance
     *
     *   q_draw(p_m) = k_draw * ( p_m - p_return )
     *
     * so the manifold is self-contained (derivative reads only p_m + its own
     * coefficients) AND self-regulating: pressure climbs until the pump inflow
     * balances the draw, giving a bounded equilibrium (~14.75 bar with the
     * defaults) rather than an unbounded integrator. A linear lumped draw is
     * used here (a first-order stable law) so the explicit integrator converges
     * monotonically without overshoot; the turbulent signed-root law lives on
     * the *per-station* branch flow (BrewStation.qBranch), which is where §10.2
     * asks for signed_root. The per-station branch sum is accounted separately
     * (ProductionCell.qTotal + the CELL-PHYS residual gate).
     *
     * Km sets the pressure time constant; k_draw / q_bypass set the
     * equilibrium. See README "Synthetic coefficients". */

    private import ISQ::*;
    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def ManifoldDynamics {
        // Coefficients. `netIn = q_pump - q_bypass` is the net supply into the
        // manifold; `k_draw` is the aggregate branch conductance; `tau` the
        // compliance time constant. Written as a single lumped supply so the
        // derivative stays in the canonical `(A - B*(X - C))/D` shape.
        attribute netIn : Real default 5.5;       // q_pump (6.0) - q_bypass (0.5)
        attribute k_draw : Real default 0.4;      // aggregate branch conductance
        attribute p_return : Real default 1.0;    // return-line pressure
        attribute tau : Real default 2.0;         // compliance time constant

        // integrated state: manifold pressure (starts near equilibrium)
        out attribute p_m : Real default 12.0;

        // dp_m/dt = ( netIn - k_draw*(p_m - p_return) ) / tau
        // Equilibrium p_m = p_return + netIn/k_draw = 14.75 bar.
        calc def PressureDeriv :> GetDerivative {
            return dp_mdt = (netIn - k_draw * (p_m - p_return)) / tau;
        }
    }
}
