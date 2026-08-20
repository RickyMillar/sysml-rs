package ProductQualityRequirements {
    doc /* Product-quality requirements (COV VER-SIM / VER-REQROWS, CELL-15):
     * the brew temperature and line pressure a shot is pulled at must sit in
     * their quality bands. Bounds are synthetic, analytically justified against
     * the declared physics equilibria (station ~89 C, manifold ~14.75 bar), not
     * calibrated against any product. Verified against the simulation in
     * Verification/ScenarioVerification. */

    private import ScalarValues::*;

    requirement def <'REQ-Q-01'> BrewTemperatureBand {
        doc /* Group-head temperature at extraction shall be within the espresso
         * brew band 88–96 C. */
        subject temp : Real;
        require constraint { temp >= 88.0 and temp <= 96.0 }
    }

    requirement def <'REQ-Q-02'> BrewPressureBand {
        doc /* Manifold pressure at extraction shall be within 9–16 bar. */
        subject pressure : Real;
        require constraint { pressure >= 9.0 and pressure <= 16.0 }
    }
}
