package EspressoTypes {
    doc /* Value types and the item families that ride the cell's three exchange
     * planes. Independently derived from the capability contract (COV rows
     * EX-POWER / EX-SIGNAL / EX-MSG) and first-principles hydraulic + thermal
     * physics — no product source consulted.
     *
     *   - power planes carry an effort/flow conjugate pair (PowerBond):
     *       hydraulic  = pressure (effort) + volume-flow (flow)
     *       thermal    = temperature (effort) + heat-flow (flow)
     *   - measurement planes carry a single directed signal quantity (SignalLink)
     *   - command planes carry a discrete item payload (MessageChannel)
     *
     * Libraries never import concrete station/scenario packages (plan §10.2). */

    private import ISQ::*;
    private import SI::*;
    private import ScalarValues::*;

    // -- Station lifecycle phase (a measurable, enumerable signal) ----------
    enum def StationPhase {
        idle;
        preheat;
        ready;
        extract;
        recover;
        purge;
        cleaning;
    }

    // -- Power-plane payloads (effort/flow conjugate pairs) -----------------
    item def HydraulicSample {
        doc /* Hydraulic power sample: pressure is the effort, volumetric flow
         * is the flow. Both present ⇒ classifies into a power domain ⇒ the
         * prerequisite for a PowerBond. */
        attribute pressure : PressureValue;
        attribute flowRate : VolumeFlowRateValue;
    }

    item def ThermalSample {
        doc /* Thermal power sample: temperature is the effort, heat-flow is
         * the flow. */
        attribute temperature : ThermodynamicTemperatureValue;
        attribute heatFlow : HeatFlowRateValue;
    }

    // -- Signal-plane payloads (single directed measurement quantity) -------
    item def PressureReading { attribute value : PressureValue; }
    item def TemperatureReading { attribute value : ThermodynamicTemperatureValue; }
    item def FlowReading { attribute value : VolumeFlowRateValue; }
    item def PhaseReading { attribute phase : StationPhase; }

    // -- Command-plane payloads (discrete message items) --------------------
    item def BrewCommand { doc /* Recipe start request for one extraction. */ }
    item def StopCommand { doc /* Abort the running recipe, return to Ready. */ }
    item def PurgeCommand { doc /* Flush the group head between shots. */ }
    item def CleanCommand { doc /* Enter the cleaning cycle. */ }
    item def PermitGrant {
        doc /* Supervisor grant of a bounded shared-resource permit. */
        attribute granted : Boolean;
    }
}
