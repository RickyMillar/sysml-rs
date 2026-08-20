// ===========================================================================
// DEMO: Phase 10 — Port-Aware Flow Routing
// ===========================================================================
//
// COMMANDS TO TEST:
//   sysml flow tests/fixtures/shared/test_port_types.sysml
//   sysml flow tests/fixtures/shared/test_port_types.sysml --json
//   sysml flow tests/fixtures/shared/test_port_types.sysml --inject '{"temperature": 92.5, "flowRate": 1.2}'
//
// EXPECTED:
//   - 4 ports detected: heater.steamOut, heater.waterIn, brewer.waterIn, brewer.coffeeOut
//   - Directions: out/in correctly assigned
//   - Port definitions: WaterPort, SteamPort, CoffeePort
//   - Conjugated port: brewer.waterIn is ~WaterPort (reversed direction)
//
// ===========================================================================

package PortTypeDemo {

    // --- Port Definitions ---

    port def WaterPort {
        attribute temperature : ScalarValues::Real;
        attribute flowRate : ScalarValues::Real;
        out attribute pressure : ScalarValues::Real;
    }

    port def SteamPort {
        out attribute temperature : ScalarValues::Real;
        out attribute pressure : ScalarValues::Real;
    }

    port def CoffeePort {
        out attribute volume : ScalarValues::Real;
    }

    // --- Parts with Ports ---

    part def Heater {
        in port waterIn : WaterPort;
        out port steamOut : SteamPort;
    }

    part def Brewer {
        in port waterIn : ~WaterPort;    // conjugated — reverses direction
        out port coffeeOut : CoffeePort;
    }

    // --- Part Instances ---

    part heater : Heater;
    part brewer : Brewer;

    // --- Flow Connections ---

    flow waterToBrewerFlow from heater.steamOut to brewer.waterIn;
}
