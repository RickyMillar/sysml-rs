package OscillatorVerification {
    doc /* Settling verification for the suspension-tuning study.
     *
     * Follows the live-verification idiom from the legacy oscillator fixture
     * (its Verification/IECCompliance.sysml): requirement constraints reference
     * the SIMULATION-PRODUCED model attributes `x` / `v` (TunedOscillator,
     * OscillatorStudy.sysml) as free names, resolved at check time from the
     * running session's final-tick state by `sysml.sessions.verify` — they
     * are NOT attributes of the requirement.
     *
     * The demo contract: run the oscillator for the study horizon
     * (default 3000 ticks at dt = 1 ms = 3 s of model time), then verify.
     * A well-tuned (near-critically-damped) design has come to rest and
     * PASSES; an underdamped design is still ringing and an overdamped one
     * is still creeping home — both FAIL. Squares avoid needing abs():
     * x*x <= 0.0025 is |x| <= 0.05 m (5% of the 1 m release). */

    private import ScalarValues::*;

    // ══════════════════════════════════════════════════════════════
    //  REQUIREMENT DEFINITIONS
    // ══════════════════════════════════════════════════════════════

    requirement def SettlesWithinHorizon {
        doc /* The mass shall be at rest at the target position by the end
         * of the study horizon: within 5 cm of x = 0, moving slower than
         * 5 cm/s. Underdamped designs fail on both (still oscillating);
         * overdamped designs fail on displacement (still creeping). */

        require constraint displacementSettled {
            x * x <= 0.0025
        }
        require constraint velocitySettled {
            v * v <= 0.0025
        }
    }

    requirement def RemainsStable {
        doc /* Sanity envelope: the state never diverges — at the horizon the
         * mass is still inside 2 m of the release point. A numerically
         * unstable or sign-flipped configuration fails loudly here instead
         * of masquerading as "just unsettled". */

        require constraint bounded {
            x * x <= 4.0
        }
    }

    // ══════════════════════════════════════════════════════════════
    //  VERIFICATION CASES — objective-block idiom (spec-canonical)
    // ══════════════════════════════════════════════════════════════

    verification def SettlingCase {
        doc /* Expected: PASS in the c ~ 2*sqrt(k) band, FAIL either side —
         * the diagonal the Analyze heatmap paints. */
        subject oscillator;
        objective settlingObjective {
            verify requirement settlingCheck : SettlesWithinHorizon;
        }
    }

    verification def StabilityCase {
        doc /* Expected: PASS everywhere in the suggested sweep grid. */
        subject oscillator;
        objective stabilityObjective {
            verify requirement stabilityCheck : RemainsStable;
        }
    }
}
