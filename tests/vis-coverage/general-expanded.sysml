// Visualization Coverage Test: General View - Nesting + Specialization + Typing
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Default General view shows all elements
//
// WHAT TO CHECK:
// - Vehicle (rect): has attribute compartment (mass, color) + port compartment (fuelPort)
// - ElectricVehicle :> Vehicle: specialization edge (triangle arrowhead)
// - car : Vehicle: typing edge (open arrowhead) from car to Vehicle
// - ev : ElectricVehicle: typing edge from ev to ElectricVehicle
// - FuelPort (port def): distinct port shape
// - Nested attributes visible in collapsed compartments

package GeneralExpanded {
    port def FuelPort;

    part def Vehicle {
        attribute mass : Real;
        attribute color : String;
        port fuelPort : FuelPort;
    }

    part def ElectricVehicle :> Vehicle {
        attribute batteryCapacity : Real;
    }

    part car : Vehicle;
    part ev : ElectricVehicle;

    attribute def Real;
    attribute def String;
}
