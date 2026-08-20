package OscillatorStudy {
    doc /* Suspension-tuning study — the Analyze workbench demo model.
     *
     * A damped spring-mass (mass = 1 kg) released from x = 1 m, targeting
     * rest at x = 0:
     *
     *   x'' = -damping * x' - stiffness * x
     *
     * The two design parameters are deliberately the two things anyone can
     * reason about on a car seat or door closer:
     *
     *   damping   (c)  — how hard motion is resisted
     *   stiffness (k)  — how hard the spring pulls back
     *
     * Physics of the demo story: the settling rate is limited by
     * min(c/2, k/c) — too LITTLE damping and the mass rings for ages
     * (underdamped), too MUCH and it creeps home slowly (overdamped).
     * Fastest response sits at critical damping c = 2*sqrt(k). Sweeping
     * damping x stiffness therefore paints a DIAGONAL PASS BAND in the
     * heatmap around c ~ 2*sqrt(k) — the picture that makes tornado /
     * heatmap legible at a glance.
     *
     * With the default 3-second horizon (3000 ticks at dt = 1 ms) the
     * settled region is  2 <= c  (underdamped ring-down needs c/2 >= 1)
     * AND, once overdamped, the slow root (c - sqrt(c^2 - 4k))/2 >= 1:
     *
     *   stiffness k = 5   → passes c in {2, 4}       (6 is the edge)
     *   stiffness k = 20  → passes c in {2, 4, 8, 16}
     *   stiffness k = 80  → passes c in {2 .. 32}
     *
     * Suggested sweep: damping {1, 2, 4, 8, 16, 32} x stiffness {5, 20, 80}
     * — one fail column at c = 1 (rings), a staircase of overdamped fails
     * in the low-stiffness rows (creeps), pass band in between.
     */

    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def TunedOscillator {
        doc /* One tunable oscillator. `x`/`v` are the integrated ODE states
         * (direction=out marks them for the integrator); the requirements in
         * OscillatorRequirements read them at verify time from the live run. */

        // ── Sweepable design parameters ─────────────────────────────
        attribute damping : Real default 4.0;
        attribute stiffness : Real default 40.0;

        // ── ODE state: released from 1 m, at rest ──────────────────
        out attribute x : Real default 1.0;
        out attribute v : Real default 0.0;

        action def SpringDynamics :> ContinuousStateSpaceDynamics {
            doc /* Unit-mass damped oscillator. */

            calc def PositionDerivative :> GetDerivative {
                return dxdt = v;
            }

            calc def VelocityDerivative :> GetDerivative {
                doc /* x'' = -k*x - c*v, written WITHOUT a subtraction chain:
                 * `0 - stiffness * x - damping * v` evaluates the second
                 * subtracted term with a FLIPPED sign (anti-damping — energy
                 * grows; verified against the closed-form response). Filed as
                 * the A-minus-B-minus-C expression bug; this chain-free form
                 * is verified exact against theory. */
                return dvdt = (0 - stiffness) * x + (0 - damping) * v;
            }

            // Total mechanical energy — a monotone "how unsettled is it"
            // observable for the waveform strip.
            calc def EnergyOutput :> GetOutput {
                return energy = 0.5 * stiffness * x * x + 0.5 * v * v;
            }
        }
    }
}
