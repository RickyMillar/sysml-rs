package SafetyRequirements {
    doc /* Safety requirements (COV VER-SIM, CELL-15): the shared plant must stay
     * inside safe operating envelopes at all times — the boiler must not exceed
     * its pressure-vessel temperature limit and the manifold must not exceed its
     * pressure limit. These are the safety envelopes the ProductionCell
     * conservation constraints also assert; verified against the run. */

    private import ScalarValues::*;

    requirement def <'REQ-S-01'> BoilerTemperatureSafe {
        doc /* Boiler (thermal source) temperature shall never exceed 130 C. */
        subject sourceTemp : Real;
        require constraint { sourceTemp <= 130.0 }
    }

    requirement def <'REQ-S-02'> ManifoldPressureSafe {
        doc /* Manifold pressure shall never exceed 20 bar. */
        subject manifoldPressure : Real;
        require constraint { manifoldPressure <= 20.0 }
    }
}
