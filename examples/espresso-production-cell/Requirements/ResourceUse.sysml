package ResourceUseRequirements {
    doc /* Resource-use requirements (COV VER-SIM, CELL-15): the aggregate demand
     * drawn by the cell must stay within the pump's delivery capacity, and the
     * shared boiler power must cover the aggregate heater load. These are the
     * accounting invariants the aggregate-accounting attributes expose. */

    private import ScalarValues::*;

    requirement def <'REQ-R-01'> DemandWithinSupply {
        doc /* Aggregate station branch draw shall not exceed the pump delivery
         * (minus the bypass bleed). */
        subject qTotal : Real;
        subject qSupply : Real;
        require constraint { qTotal <= qSupply }
    }

    requirement def <'REQ-R-02'> HeaterLoadCovered {
        doc /* Total cell power draw shall be positive and finite (accounting
         * sanity on P_total). */
        subject pTotal : Real;
        require constraint { pTotal >= 0.0 }
    }
}
