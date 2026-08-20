package PumpCharacteristic {
    doc /* Check-valve effective-area characteristic, as two CSV-backed sampled
     * functions of the normalized valve command `u` in [0,1].
     *
     *   openingBranch  effective area while the command is rising  (advancing
     *                  stroke) — a smoothstep from A_min to A_max.
     *   closingBranch  effective area while the command is falling (retracting
     *                  stroke) — the opening branch minus a bounded hysteresis
     *                  lag that vanishes at both endpoints, so the closing area
     *                  is never larger than the opening area for the same
     *                  command.
     *
     * Both tables are emitted by scripts/generate_characteristics.py; their
     * exact bytes and equations are pinned in fixture-provenance.toml. The
     * runtime loads each CSV via the @DataSource annotation and exposes it to
     * expressions as a SampledFunction the model interpolates with
     * `interpolateLinear`. */

    private import ScalarValues::*;
    private import SampledFunctions::*;
    private import PumpTooling::*;

    // Opening (advancing) branch: monotone smoothstep A_min -> A_max.
    attribute openingBranch : SampledFunction {
        @DataSource {
            file = "data/generated_pump_opening.csv";
            domainColumn = 0;
            rangeColumn = 1;
            hasHeader = true;
        }
    }

    // Closing (retracting) branch: trails the opening branch by a bounded,
    // endpoint-vanishing hysteresis separation.
    attribute closingBranch : SampledFunction {
        @DataSource {
            file = "data/generated_pump_closing.csv";
            domainColumn = 0;
            rangeColumn = 1;
            hasHeader = true;
        }
    }
}
