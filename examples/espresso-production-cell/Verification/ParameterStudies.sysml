package ParameterStudies {
    doc /* Parameter-study analysis cases (COV VER-SIM / trade-study, CELL-15).
     * An analysis case computes a derived quantity about the cell design — here
     * the thermal-headroom margin between the boiler equilibrium and its safety
     * limit — and its objective judges the computed result against a
     * requirement. This is the analysis counterpart to the verification cases:
     * it judges the RESULT (§7.23.2) rather than the subject. */

    private import ScalarValues::*;

    requirement def ThermalHeadroomAdequate {
        doc /* The boiler equilibrium shall sit at least 15 C below its safety
         * limit so a transient cannot breach it. */
        subject marginC : Real;
        require constraint { marginC >= 15.0 }
    }

    analysis def <'AN-1'> ThermalHeadroomStudy {
        doc /* Compute the boiler thermal headroom: safety limit (130 C) minus the
         * declared equilibrium (112 C) = 18 C, which satisfies the margin. */
        in attribute limitC : Real = 130.0;
        in attribute equilibriumC : Real = 112.0;
        return attribute marginC : Real = limitC - equilibriumC;
        objective {
            verify ThermalHeadroomAdequate;
        }
    }
}
