// Visualization Coverage Test: Compartment Routing (Phase 3)
// Open in GeneralView (default). Collapsed nodes should show children
// organized into typed compartments.
//
// WHAT TO CHECK:
// 1. Vehicle (collapsed): children sorted into compartments:
//    - "attributes" compartment: speed, weight
//    - "parts" compartment: engine, wheel
//    - "ports" compartment: shown on boundary (not in compartment)
//    - "directed features" compartment: dataIn (has direction)
//    - "constraints" compartment: maxSpeed
//    - "states" compartment: driving, parked
//    - "actions" compartment: accelerate
// 2. Gear enum: shows literals (park, reverse, neutral, drive)
// 3. Expand Vehicle: children become nested SNode boxes
// 4. VariantVehicle: variant children should appear in "variants" compartment

package CompartmentRouting {

    part def Vehicle {
        // Attributes compartment
        attribute speed : Real;
        attribute weight : Real;

        // Parts compartment
        part engine : Engine;
        part wheel : Wheel;

        // Ports (rendered on boundary)
        port fuelIn : FuelPort;

        // Directed feature (has direction property)
        in attribute dataIn : Real;

        // Constraint compartment
        constraint maxSpeed {
            doc /* speed <= 300 */
        }

        // State children
        state parked;
        state driving;

        // Action children
        action accelerate;

        // Documentation
        doc /* A generic vehicle definition */
    }

    // Enumeration with literals
    enum def Gear {
        park;
        reverse;
        neutral;
        drive;
    }

    // Variation point with variants
    variation part def VariantVehicle {
        variant part sedan;
        variant part suv;
        variant part truck;
    }

    // Occurrence with individual
    occurrence def TestEvent {
        individual occurrence singleTest;
    }

    // Supporting types
    part def Engine;
    part def Wheel;
    port def FuelPort;
    attribute def Real;
}
