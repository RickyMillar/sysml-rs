package DemandProfiles {
    doc /* External aggregate-demand profile, backed by the deterministically
     * generated data/generated_demand.csv (see scripts/generate_profiles.py).
     * Declared as an @DataSource SampledFunction so loading the workspace
     * resolves the CSV against the project source directory — exercising the
     * source-dir threading the session layer must provide (COV SVC-SESSION).
     *
     * The profile is an *observable driving input* (aggregate service demand in
     * l/min over a 120 s window); it is intentionally NOT wired into the
     * continuous ODEs, which stay self-contained. The supervisor and the
     * time-series/streaming gates read it. */

    private import ScalarValues::*;
    private import SampledFunctions::*;

    // Local metadata def so the @DataSource application resolves without a
    // stdlib dependency; the runtime CSV loader reads file/columns/header.
    metadata def DataSource {
        attribute file : String;
        attribute domainColumn : Integer;
        attribute rangeColumn : Integer;
        attribute hasHeader : Boolean;
    }

    part def DemandProfile {
        attribute demand : SampledFunction {
            @DataSource {
                file = "data/generated_demand.csv";
                domainColumn = 0;
                rangeColumn = 1;
                hasHeader = true;
            }
        }
    }

    part demandProfile : DemandProfile;
}
