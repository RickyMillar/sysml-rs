package AmbientProfiles {
    doc /* External ambient-temperature profile, backed by the deterministically
     * generated data/generated_ambient.csv. Same @DataSource / source-dir
     * contract as DemandProfiles (COV SVC-SESSION). A low-frequency diurnal
     * sinusoid (22 ± 3 °C); an observable input, not wired into the ODEs. */

    private import ScalarValues::*;
    private import SampledFunctions::*;
    private import DemandProfiles::DataSource;

    part def AmbientProfile {
        attribute ambient : SampledFunction {
            @DataSource {
                file = "data/generated_ambient.csv";
                domainColumn = 0;
                rangeColumn = 1;
                hasHeader = true;
            }
        }
    }

    part ambientProfile : AmbientProfile;
}
