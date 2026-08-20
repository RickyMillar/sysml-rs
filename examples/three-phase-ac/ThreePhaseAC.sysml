package ThreePhaseAC {
    doc /* Three-phase AC power source with balanced sinusoidal voltages.
     *
     * Physics: V_a = V_peak * sin(omega*t)
     *          V_b = V_peak * sin(omega*t - 2pi/3)
     *          V_c = V_peak * sin(omega*t + 2pi/3)
     *
     * With a balanced resistive load (R per phase):
     *   I_a = V_a / R, I_b = V_b / R, I_c = V_c / R
     *   I_neutral = I_a + I_b + I_c = 0 (balanced load)
     *
     * Total instantaneous power: P = V_a*I_a + V_b*I_b + V_c*I_c
     *   = V_peak^2/R * (sin^2(wt) + sin^2(wt-2pi/3) + sin^2(wt+2pi/3))
     *   = 3*V_peak^2/(2*R) = constant (no ripple for balanced load)
     *
     * Parameters:
     *   V_peak = 325.27V (230V RMS * sqrt(2), AU/EU standard)
     *   f = 50Hz -> omega = 2pi*50 = 314.159 rad/s
     *   R = 10 Ohm per phase
     *
     * This model uses signal expressions (sin function) rather than
     * SampledFunction lookup tables, demonstrating continuous waveform
     * generation in the ODE engine.
     *
     * Uses spec-standard StateSpaceRepresentation pattern:
     * - calc def :> GetDerivative defines dx/dt = f(u, x)
     * - calc def :> GetOutput computes complex phasors from time-domain state */

    private import ISQ::*;
    private import SI::*;
    private import StateSpaceRepresentation::*;

    part def ThreePhaseSource {
        doc /* Balanced 3-phase voltage source + resistive load.
         *
         * Demonstrates complex phasor representation alongside
         * time-domain simulation. Phasors are computed via GetOutput
         * using rect(magnitude, phase) at each time step. */

        // Parameters
        attribute vPeak : ElectricPotentialValue default 325.27;
        attribute omega : AngularVelocityValue default 314.159;
        attribute resistance : ResistanceValue default 10.0;

        // Phase offsets (radians)
        attribute phaseB_offset : Real default -2.0943951;
        attribute phaseC_offset : Real default 2.0943951;

        // Phase voltages (computed from t via signal expressions)
        attribute vA : ElectricPotentialValue default 0.0;
        attribute vB : ElectricPotentialValue default 0.0;
        attribute vC : ElectricPotentialValue default 0.0;

        // Phase currents (Ohm's law)
        attribute iA : ElectricCurrentValue default 0.0;
        attribute iB : ElectricCurrentValue default 0.0;
        attribute iC : ElectricCurrentValue default 0.0;

        // Neutral current (should be ~0 for balanced load)
        attribute iNeutral : ElectricCurrentValue default 0.0;

        // Total instantaneous power
        attribute totalPower : PowerValue default 0.0;

        // ODE state variable (direction=out marks it as integrated)
        out attribute energy : EnergyValue default 0.0;

        // SSR derivative — spec-standard GetDerivative pattern
        calc def EnergyDerivative :> GetDerivative {
            return dEdt = totalPower;
        }

        // ── Complex phasor outputs (spec-standard GetOutput) ─────────
        // Voltage phasors: polar(V_peak, omega*t + phase_offset)
        // These demonstrate complex number support in the simulation
        // engine, computed algebraically from ODE state each step.
        calc def PhasorA :> GetOutput {
            return vPhasorA = polar(vPeak, omega * t);
        }
        calc def PhasorB :> GetOutput {
            return vPhasorB = polar(vPeak, omega * t + phaseB_offset);
        }
        calc def PhasorC :> GetOutput {
            return vPhasorC = polar(vPeak, omega * t + phaseC_offset);
        }
    }
}
