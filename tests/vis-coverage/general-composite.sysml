// Visualization Coverage Test: Multi-View Composite GeneralView
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Click the expand button [+] on "Vehicle"
//
// WHAT TO CHECK:
// - Expanded Vehicle should show ALL of the following simultaneously:
//   - State-transition sub-diagram (stv) with VehicleState containing parked, driving
//   - Action-flow sub-diagram (afv) with StartEngine containing turnKey, checkFuel, ignite
//   - Port "fuelIn" on Vehicle boundary
//   - Structural BDD nodes for "engine" and "maxSpeed"
//   - Combined view-badge "stv+afv"
// - StartSequence (action def) should show ONLY action-flow sub-diagram
// - TrafficLight (state def) should show ONLY state-transition sub-diagram

package GeneralComposite {
    port def FuelPort;
    port def DataPort;
    attribute def Real;
    part def Engine;

    part def Vehicle {
        // Ports
        port fuelIn : FuelPort;
        port dataOut : DataPort;

        // State machine (→ embedded state-transition diagram)
        state def VehicleState {
            state parked;
            state driving;
            state reversing;
            transition first parked then driving;
            transition first driving then reversing;
        }

        // Action flow (→ embedded action-flow diagram)
        action def StartEngine {
            action turnKey;
            action checkFuel;
            action ignite;
            succession first turnKey then checkFuel;
            succession first checkFuel then ignite;
        }

        // Structural children (→ BDD nodes alongside sub-diagrams)
        part engine : Engine;
        attribute maxSpeed : Real;
    }

    // Action def with its own flow (single sub-diagram, not composite)
    action def StartSequence {
        action turnKey;
        action checkFuel;
        action ignite;
        succession first turnKey then checkFuel;
        succession first checkFuel then ignite;
    }

    // State def with its own states (single sub-diagram)
    state def TrafficLight {
        state red;
        state green;
        state yellow;
        transition first red then green;
        transition first green then yellow;
        transition first yellow then red;
    }

    part vehicle : Vehicle;
}
