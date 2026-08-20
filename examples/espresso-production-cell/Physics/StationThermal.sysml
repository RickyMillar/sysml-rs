package StationThermalPhysics {
    doc /* Per-station group-head thermal mass — the nested ODE bearer that
     * makes each brew station an independent runtime subsystem (one integrated
     * temperature state per instance). Lumped first-principles energy balance
     * (COV RT-ODE-CORE / VAL-CONSTR / RT-INSTANCE):
     *
     *   C * dT/dt = P_heater
     *             + h_supply  * (T_supply  - T)     heat in from the boiler loop
     *             - h_ambient * (T - T_ambient)     loss to the room
     *             - Q_product                       enthalpy carried off in the shot
     *
     * Self-contained: the derivative reads only this part's own state (T) and
     * its coefficient attributes, so the runtime multiplies it cleanly per
     * station and per-instance state stays independent. Coefficients are
     * synthetic, chosen so the equilibrium sits in a plausible brew band
     * (~89 C) and the time constant (C/(h_supply+h_ambient) ≈ 2.9) settles
     * inside a short CI run. See README "Synthetic coefficients". */

    private import ISQ::*;
    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def StationThermal {
        // -- coefficients (overridable per station instance) ----------------
        attribute C : Real default 2.0;            // lumped heat capacity
        attribute P_heater : Real default 10.0;    // heater power
        attribute h_supply : Real default 0.5;     // boiler-loop coupling
        attribute T_supply : Real default 96.0;    // boiler-loop temperature
        attribute h_ambient : Real default 0.2;    // ambient loss coefficient
        attribute T_ambient : Real default 22.0;   // room temperature
        // Combined loss conductance (h_supply + h_ambient); lets the derivative
        // stay an addition chain with a single trailing subtraction.
        attribute lossSum : Real default 0.7;

        // -- integrated state: group-head temperature -----------------------
        out attribute temp : Real default 22.0;

        // dT/dt = ( P_heater + h_supply*T_supply + h_ambient*T_ambient - lossSum*T ) / C
        // (algebraically identical to the effort/flow form; equilibrium ~89 C).
        calc def TempDeriv :> GetDerivative {
            return dtempdt =
                (P_heater + h_supply * T_supply + h_ambient * T_ambient - lossSum * temp) / C;
        }
    }
}
