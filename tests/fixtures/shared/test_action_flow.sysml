// ===========================================================================
// DEMO: Phase 13 — Activity Token Flow with Port Routing
// ===========================================================================
//
// COMMANDS TO TEST:
//   sysml run BrewCoffee tests/fixtures/shared/test_action_flow.sysml --json
//   sysml flow tests/fixtures/shared/test_action_flow.sysml
//
// SCENARIO:
//   A coffee brew process with parallel water heating and bean grinding,
//   joined before the pour-over step. Demonstrates fork/join with port
//   routing and succession ordering.
//
//   HeatWater ──→ (port: hotWaterOut) ─┐
//                                       ├──→ PourOver ──→ Serve
//   GrindBeans ──→ (port: groundsOut) ─┘
//
// EXPECTED:
//   - Fork splits into HeatWater and GrindBeans (parallel)
//   - Both complete → Join synchronizes
//   - PourOver receives both inputs via ports
//   - Succession: HeatWater must complete before PourOver starts
//   - Sequence trace shows correct message ordering
//
// ===========================================================================

package ActionFlowDemo {

    port def WaterPort {
        out attribute temperature : ScalarValues::Real;
    }

    port def GroundsPort {
        out attribute grindSize : ScalarValues::Real;
    }

    part def WaterHeater {
        out port hotWaterOut : WaterPort;
    }

    part def Grinder {
        out port groundsOut : GroundsPort;
    }

    part def Brewer {
        in port waterIn : ~WaterPort;
        in port groundsIn : ~GroundsPort;
    }

    // --- Action Definitions ---

    action def HeatWater {
        out item hotWater;
    }

    action def GrindBeans {
        out item grounds;
    }

    action def PourOver {
        in item water;
        in item grounds;
        out item coffee;
    }

    action def Serve {
        in item coffee;
    }

    // --- Brew Process (fork/join + succession) ---

    action def BrewCoffee {
        action heatWater : HeatWater;
        action grindBeans : GrindBeans;
        action pourOver : PourOver;
        action serve : Serve;

        // Fork: heat and grind happen in parallel
        fork forkStart;
        first forkStart then heatWater;
        first forkStart then grindBeans;

        // Join: both must complete before pour-over
        join joinBeforePour;
        first heatWater then joinBeforePour;
        first grindBeans then joinBeforePour;

        // Succession: pour after join, serve after pour
        first joinBeforePour then pourOver;
        first pourOver then serve;
    }

    // --- Part Instances ---

    part heater : WaterHeater;
    part grinder : Grinder;
    part brewer : Brewer;

    // --- Flow Connections ---

    flow waterFlow from heater.hotWaterOut to brewer.waterIn;
    flow groundsFlow from grinder.groundsOut to brewer.groundsIn;
}
