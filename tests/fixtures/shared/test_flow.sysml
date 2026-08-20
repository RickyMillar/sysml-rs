// ===========================================================================
// TEST: Flow visualization
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. COMMANDS TO TEST:
//      - Place cursor on "WaterSystem" package, run: sysml.flow.visualize
//      - Should return DOT graph showing flow connections:
//        pump --water--> filter --cleanWater--> tank
//
//   2. OUTLINE:
//      - "WaterSystem" should appear as a package in outline
//      - Parts (pump, filter, tank) should be visible
//      - Flow connections should appear
//
//   3. HOVER:
//      - Hover over "pump" → should show part info
//      - Hover over "flow" keyword → should show syntax doc
//      - Hover over port names → should show port info
//
// ===========================================================================

package WaterSystem {
    private import ScalarValues::Real;


    part def Pump {
        attribute flowRate : Real;
        out port waterOut;
    }

    part def Filter {
        in port waterIn;
        out port cleanWaterOut;
        attribute efficiency : Real;
    }

    part def Tank {
        in port waterIn;
        attribute capacity : Real;
    }

    part pump : Pump {
        attribute flowRate = 50;
    }

    part filter : Filter {
        attribute efficiency = 95;
    }

    part tank : Tank {
        attribute capacity = 1000;
    }

    // Flow connections
    flow waterFlow from pump.waterOut to filter.waterIn;
    flow cleanFlow from filter.cleanWaterOut to tank.waterIn;
}
