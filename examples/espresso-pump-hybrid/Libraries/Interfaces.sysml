package PumpInterfaces {
    doc /* Fluid-power port and part vocabulary for the structural (bond-graph)
     * view of the pump. These are DEFINITIONS only — no top-level connected
     * network is instantiated in the fixture project, so the pump's simulation
     * stays a self-contained ODE + state-machine hybrid (no physics DAE
     * subsystem is co-scheduled with it). The effort/flow port classification
     * that these defs enable is exercised in isolation by the EX-POWER gate,
     * which assembles a supply -> stage -> load network from this vocabulary and
     * checks it classifies into hydraulic PowerBonds.
     *
     * Port defs are named with the "Fluid" stem so the domain heuristic places
     * them in the hydraulic domain; each carries a HydraulicSample (pressure
     * effort + flow), the prerequisite for a PowerBond. */

    private import PumpTypes::*;

    port def FluidOut {
        out item fluid : HydraulicSample;
    }
    port def FluidIn {
        in item fluid : HydraulicSample;
    }

    part def FluidSupply {
        port outlet : FluidOut;
    }
    part def PumpStage {
        port inlet : FluidIn;
        port outlet : FluidOut;
    }
    part def FluidLoad {
        port inlet : FluidIn;
    }
}
