package PumpSafety {
    doc /* Simulation-backed safety verification.
     *
     * The requirement constraints reference the SIMULATION-PRODUCED state
     * attributes of the pump (`exposure`, `chamberPressure`) and the pump
     * parameter `exposureTrip` as free names. They are resolved at check time
     * from the running session's final-tick state — they are NOT attributes of
     * the requirement. Run the pump for the study horizon, then verify.
     *
     * The two safety requirements are complementary, which makes the
     * scenario x requirement verdict matrix non-vacuous (diagonal pass,
     * off-diagonal fail):
     *
     *              SafeUnderNominal   ProtectsUnderSevere
     *   nominal        PASS                 FAIL
     *   severe         FAIL                 PASS
     *
     * Analytic bounds (full derivation in README.md):
     *   - Nominal (free discharge): the accumulator settles below pWarning, so
     *     the exposure integral never accumulates and stays at ~0 << exposureTrip.
     *   - Severe (restricted discharge): the accumulator settles above pWarning,
     *     so exposure grows at a strictly positive rate and eventually reaches
     *     exposureTrip; relief cannot latch before exposureTrip/(pa_max - pWarning). */

    private import ScalarValues::*;

    // ══════════════════════════════════════════════════════════════
    //  REQUIREMENT DEFINITIONS
    // ══════════════════════════════════════════════════════════════

    requirement def SafeUnderNominal {
        doc /* Under nominal (free) discharge the accumulated over-pressure
         * exposure shall stay below the relief trip level, i.e. relief is never
         * latched. Nominal PASSES (exposure ~ 0); a restricted run FAILS. */
        require constraint noReliefLatched {
            exposure < exposureTrip
        }
    }

    requirement def ProtectsUnderSevere {
        doc /* Under a severe synthetic restriction the exposure integral shall
         * reach the relief trip level within the horizon, i.e. the latched
         * safety path engages. Severe PASSES; nominal FAILS (exposure ~ 0). */
        require constraint reliefEngaged {
            exposure >= exposureTrip
        }
    }

    requirement def BoundedChamberPressure {
        doc /* Sanity envelope: the chamber pressure stays finite and bounded
         * across the run for every restriction level in the study grid — a
         * numerically unstable configuration fails loudly here instead of
         * masquerading as merely relieved/unrelieved. */
        require constraint bounded {
            chamberPressure * chamberPressure <= 100.0
        }
    }

    // ══════════════════════════════════════════════════════════════
    //  VERIFICATION CASES — objective-block idiom (spec-canonical)
    // ══════════════════════════════════════════════════════════════

    verification def NominalSafetyCase {
        doc /* Expected PASS at nominal restriction, FAIL under severe. */
        subject pump;
        objective nominalObjective {
            verify requirement nominalCheck : SafeUnderNominal;
        }
    }

    verification def SevereProtectionCase {
        doc /* Expected PASS under severe restriction, FAIL at nominal. */
        subject pump;
        objective severeObjective {
            verify requirement severeCheck : ProtectsUnderSevere;
        }
    }

    verification def StabilityCase {
        doc /* Expected PASS for every restriction level in the study grid. */
        subject pump;
        objective stabilityObjective {
            verify requirement stabilityCheck : BoundedChamberPressure;
        }
    }
}
