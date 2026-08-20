// Visualization Coverage Test: Embedded Sub-Diagrams (Phase 2/3)
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Click the expand button [+] on "Vehicle"
//
// WHAT TO CHECK:
// - Collapsed Vehicle: shows children as text lines in compartments
// - Expanded Vehicle: behavioral children embed as sub-diagrams:
//   - VehicleState shows "stv" badge + state transition diagram inside
//   - StartEngine shows "afv" badge + action flow diagram inside
// - Port "fuelIn" appears on Vehicle's boundary (small square)

package StateActionEmbedded {
    port def FuelPort;

    part def Vehicle {
        state def VehicleState {
            state parked;
            state driving;
            state reversing;

            transition parked_to_driving
                first parked then driving;
            transition driving_to_parked
                first driving then parked;
        }

        action def StartEngine {
            action turnKey;
            action checkFuel;
            action ignite;

            succession turnKey then checkFuel;
            succession checkFuel then ignite;
        }

        port fuelIn : FuelPort;
    }
}
