package ScenarioVerification {
    doc /* Verification cases binding simulation scenarios to requirements (COV
     * VER-SIM, CELL-15). Each case names the requirement(s) it discharges; the
     * VerificationRunner evaluates them against the simulation-produced values
     * for a run of the production cell (driven by the Stage-D service gate). The
     * verdict matrix is non-vacuous: a nominal run passes the quality/safety
     * bands, and an over-limit synthetic scenario fails the safety band. */

    private import ProductQualityRequirements::*;
    private import ThroughputRequirements::*;
    private import SafetyRequirements::*;
    private import ResourceUseRequirements::*;

    part def TestBench;

    verification def <'VC-Q'> BrewQualityCase {
        doc /* Verify the extraction quality bands (temperature + pressure) hold
         * at steady state. */
        @VerificationMethod { kind = VerificationMethodKind::analyze; }
        subject bench : TestBench;
        objective {
            verify BrewTemperatureBand;
            verify BrewPressureBand;
        }
    }

    verification def <'VC-S'> SafetyEnvelopeCase {
        doc /* Verify the boiler and manifold stay inside their safety envelopes
         * across the run. */
        @VerificationMethod { kind = VerificationMethodKind::analyze; }
        subject bench : TestBench;
        objective {
            verify BoilerTemperatureSafe;
            verify ManifoldPressureSafe;
        }
    }

    verification def <'VC-T'> ThroughputCase {
        doc /* Verify warm-up and recovery timing budgets. */
        @VerificationMethod { kind = VerificationMethodKind::test; }
        subject bench : TestBench;
        objective {
            verify WarmupWithinBudget;
            verify RecoveryBounded;
        }
    }

    verification def <'VC-R'> ResourceCase {
        doc /* Verify the aggregate-accounting resource invariants. */
        @VerificationMethod { kind = VerificationMethodKind::analyze; }
        subject bench : TestBench;
        objective {
            verify DemandWithinSupply;
            verify HeaterLoadCovered;
        }
    }
}
