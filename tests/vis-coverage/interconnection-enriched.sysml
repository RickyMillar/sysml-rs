// Visualization Coverage Test: Interconnection View (Phase 5)
// Switch to "Interconnection" view to test these features.
//
// WHAT TO CHECK:
// 1. Parts render as rectangles with ports on boundaries
// 2. Flow edges (filled arrow) between parts
// 3. Connection edges between parts
// 4. Ports show on part boundaries

package InterconnectionEnriched {
    port def DataPort {
        out item data;
    }
    port def FuelPort;

    part def Engine {
        port fuelIn : FuelPort;
        port dataOut : DataPort;
    }

    part def FuelTank {
        port fuelOut : FuelPort;
    }

    part def ECU {
        // 4B: sensorIn uses conjugated DataPort (out→in reversal)
        port sensorIn : ~DataPort;
        port dataOut : DataPort;
    }

    part def Display {
        port dataIn : DataPort;
    }

    item def Fuel;

    // System container with usages and connections
    part def VehicleSystem {
        part engine : Engine;
        part tank : FuelTank;
        part ecu : ECU;
        part display : Display;

        // 4A: Reference part (should render with dashed border)
        ref part externalSensor : ECU;

        // Flow: fuel from tank to engine
        flow of Fuel from tank.fuelOut to engine.fuelIn;

        // Connection: ECU data to display
        connection ecuToDisplay connect ecu.dataOut to display.dataIn;

        // Binding: ECU data output to display input
        binding of ecu.dataOut = display.dataIn;
    }
}
