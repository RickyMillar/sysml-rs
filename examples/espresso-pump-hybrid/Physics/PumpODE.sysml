package PumpDynamics {
    doc /* Reciprocating-pump hybrid dynamics.
     *
     * Continuous states (all `out attribute`, integrated by the SSR pattern):
     *   stroke        centered actuator position; physical position x = stroke + 0.5 in [0,1]
     *   strokeRate    actuator velocity (dx/dt)
     *   chamberPressure     p_c, chamber pressure                    (pressure)
     *   accumulatorPressure p_a, downstream/accumulator pressure     (pressure)
     *   exposure      z, integral of sustained over-pressure         (pressure*time)
     *
     * Actuator: an undamped harmonic oscillator about the mid-stroke, so it
     * self-sustains a phase-normalized cosine position with NO manual seed:
     *   stroke''   = -omega^2 * stroke        =>   x(t) = 0.5*(1 - cos(omega t))
     * released from stroke = -0.5, strokeRate = 0. Energy is conserved, so the
     * unforced actuator neither grows nor decays (bounded-drift invariant).
     *
     * Hydraulics (first principles; see README for the full derivation):
     *   q_src   = pistonArea * max(0, strokeRate)          delivery-stroke inflow
     *   u       = clamp(dP / dpRef, 0, 1)                  normalized valve command
     *   A_eff   = interpolate(opening|closing branch, u)   sampled check-valve area
     *   q_out   = Cq * A_eff * sqrt(max(dP, epsRoot))      regularized orifice outflow
     *   q_leak  = kLeak * chamberPressure                  linear chamber leak
     *   q_rest  = restrictionConductance * sqrt(max(accumulatorPressure, epsRoot))
     *   p_c'    = Kc * (q_src - q_out - q_leak)
     *   p_a'    = Ka * (q_out - q_rest)
     *   z'      = max(0, chamberPressure - pWarning)       exposure accumulation
     *
     * The check valve is forward-only: dP = p_c - p_a is clamped by max(dP, epsRoot)
     * so the orifice never draws fluid backwards, and epsRoot regularizes the
     * square root's otherwise-infinite slope at dP = 0 (bounded slope 1/(2*sqrt(epsRoot))).
     * The branch is chosen by the sign of strokeRate: rising command (advancing
     * stroke, strokeRate >= 0) uses the opening branch, falling command uses the
     * closing branch. strokeRate >= 0 is the explicit initial branch (strokeRate
     * starts at 0). */

    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;
    private import SampledFunctions::*;
    private import PumpCharacteristic::*;

    part def ReciprocatingPump {

        // ── Actuator parameters ──────────────────────────────────────
        attribute omega : Real default 6.2832;      // actuator angular frequency (1 Hz)

        // ── Hydraulic parameters ─────────────────────────────────────
        attribute pistonArea : Real default 0.5;    // displacement gain on the advancing stroke
        attribute dischargeCoeff : Real default 2.0; // Cq, orifice discharge coefficient
        attribute dpRef : Real default 1.0;         // command-saturation pressure difference
        attribute epsRoot : Real default 0.0001;    // sqrt regularization floor
        attribute leakCoeff : Real default 0.05;    // kLeak, linear chamber leak
        attribute chamberGain : Real default 4.0;   // Kc, chamber compliance factor
        attribute accumulatorGain : Real default 2.0; // Ka, accumulator compliance factor

        // Scenario parameter (overridden by Restrictions.sysml scenarios): the
        // downstream discharge conductance. Large = free discharge (nominal);
        // small = restricted, so pressure backs up.
        attribute restrictionConductance : Real default 1.5;

        // ── Safety parameters ────────────────────────────────────────
        // The exposure integral accumulates sustained downstream (accumulator)
        // over-pressure; the accumulator smooths the reciprocating pulsation, so
        // this is the physically-meaningful hazard signal (chamber pressure
        // pulses every stroke and would false-trip).
        attribute pWarning : Real default 0.5;      // accumulator over-pressure threshold
        attribute exposureTrip : Real default 1.0;  // latched-relief exposure threshold

        // ── Continuous states (out attribute => integrated) ──────────
        // State names are chosen so no name is a substring of another (the
        // derivative-to-state matcher keys on the return-variable name
        // containing the full state name), hence `velocity` rather than
        // `strokeRate`.
        out attribute stroke : Real default -0.5;
        out attribute velocity : Real default 0.0;
        out attribute chamberPressure : Real default 0.0;
        out attribute accumulatorPressure : Real default 0.0;
        out attribute exposure : Real default 0.0;

        // ── Continuous dynamics ──────────────────────────────────────
        action def PumpFieldDynamics :> ContinuousStateSpaceDynamics {

            // Algebraic outputs (the shared hydraulic chain).
            calc def CommandOutput :> GetOutput {
                return u = max(0.0, min(1.0, (chamberPressure - accumulatorPressure) / dpRef));
            }
            calc def AreaOutput :> GetOutput {
                return aEff =
                    if velocity >= 0.0 ? interpolateLinear(openingBranch, u)
                    else interpolateLinear(closingBranch, u);
            }
            calc def OrificeOutput :> GetOutput {
                return qOut = dischargeCoeff * aEff
                    * sqrt(max(chamberPressure - accumulatorPressure, epsRoot));
            }

            // Actuator derivatives (undamped harmonic oscillator).
            calc def StrokeDerivative :> GetDerivative {
                return dstrokedt = velocity;
            }
            calc def VelocityDerivative :> GetDerivative {
                return dvelocitydt = (0.0 - omega) * omega * stroke;
            }

            // Pressure derivatives.
            calc def ChamberDerivative :> GetDerivative {
                // Written as `src - (out + leak)` — a single subtraction — so it
                // is correct regardless of how the evaluator associates a
                // chained `a - b - c`.
                return dchamberPressuredt = chamberGain
                    * (pistonArea * max(0.0, velocity)
                        - (qOut + leakCoeff * chamberPressure));
            }
            calc def AccumulatorDerivative :> GetDerivative {
                return daccumulatorPressuredt = accumulatorGain
                    * (qOut - restrictionConductance
                        * sqrt(max(accumulatorPressure, epsRoot)));
            }

            // Exposure integral (downstream/accumulator over-pressure).
            calc def ExposureDerivative :> GetDerivative {
                return dexposuredt = max(0.0, accumulatorPressure - pWarning);
            }
        }
    }
}
