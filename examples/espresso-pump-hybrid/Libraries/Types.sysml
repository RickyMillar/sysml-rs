package PumpTypes {
    doc /* Shared payload types for the pump's fluid-power topology.
     *
     * A hydraulic sample carries a complete effort/flow conjugate pair —
     * pressure (effort) and volumetric flow rate (flow). A port carrying it is
     * a power port: the presence of BOTH an effort and a flow feature is what
     * lets the physics classifier infer a hydraulic PowerBond (as opposed to a
     * single-directed signal). */

    private import ISQ::*;

    item def HydraulicSample {
        attribute pressure : PressureValue;        // effort
        attribute flowRate : VolumeFlowRateValue;  // flow
    }
}
