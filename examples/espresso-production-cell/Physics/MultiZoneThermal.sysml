package ThermalSourcePhysics {
    doc /* Shared multi-zone thermal source (the cell boiler feeding every
     * station's supply loop) — the second shared-plant ODE. Lumped energy
     * balance (COV RT-ODE-CORE / VAL-CONSTR):
     *
     *   C_source * dT_source/dt = P_source
     *                           - h_load * (T_source - T_load_ref)   heat drawn by stations
     *                           - Q_source_loss                      standing loss
     *
     * The distributed station coupling sum(h_supply[i]*(T_source - T[i])) is
     * represented here by a single lumped load term h_load*(T_source-T_load_ref)
     * so the source stays a self-contained subsystem; the per-zone coupling is
     * accounted for by the CELL-PHYS energy-residual gate over the exposed
     * station temperatures. Equilibrium ~112 C (a pressurised boiler), bounded,
     * time constant C_source/h_load ≈ 5. See README "Synthetic coefficients". */

    private import ISQ::*;
    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def ThermalSourceDynamics {
        attribute C_source : Real default 5.0;        // boiler heat capacity
        // `netHeat = P_source - Q_source_loss` is the net element power after
        // the standing loss; `h_load` the aggregate station load; `T_load_ref`
        // the ambient/return reference. Lumped so the derivative stays canonical.
        attribute netHeat : Real default 90.0;        // P_source (100) - Q_source_loss (10)
        attribute h_load : Real default 1.0;          // aggregate station load
        attribute T_load_ref : Real default 22.0;     // ambient/return reference

        // integrated state: source temperature (starts near equilibrium ~112 C)
        out attribute tsource : Real default 90.0;

        // dT_source/dt = ( netHeat - h_load*(T_source - T_load_ref) ) / C_source
        // Equilibrium T_source = T_load_ref + netHeat/h_load = 112 C.
        calc def SourceDeriv :> GetDerivative {
            return dtsourcedt = (netHeat - h_load * (tsource - T_load_ref)) / C_source;
        }
    }
}
