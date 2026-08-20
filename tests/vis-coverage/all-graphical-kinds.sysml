// Visualization Coverage Test: All GraphicalKind Types
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. View should default to "General" view
//
// WHAT TO CHECK:
// - Package tab shape (AllGraphicalKinds wrapper)
// - Part/Item/Connection: plain rectangles (node:block)
// - Action/Calculation: rounded rectangles (node:action)
// - State: rounded rectangle (node:state)
// - Constraint: rounded rectangle (node:constraint)
// - Requirement/Concern/Verification: plain rectangles (node:requirement)
// - UseCase/AnalysisCase: ellipses (node:usecase)
// - Interface/Attribute/Enumeration: distinct rectangles
// - Comment: note shape with folded corner
// - Specialize edge (Car → Vehicle): triangle arrowhead

package AllGraphicalKinds {
    // === Structural (rect) ===
    part def Vehicle;
    item def Fuel;
    connection def DataLink;

    // === Behavioral (rounded rect) ===
    action def Accelerate {
        action step1;
    }
    state def Moving {
        state idle;
    }
    constraint def SpeedLimit;
    calc def FuelConsumption;

    // === Requirements family ===
    requirement def Performance {
        doc /* The system shall meet performance targets */
    }
    concern def Safety;

    // === Cases ===
    use case def DriveVehicle;
    analysis def FuelAnalysis;

    // === Type-specific ===
    interface def USB;
    attribute def Color;
    enum def Gear {
        park; reverse; neutral; drive;
    }
    allocation def SWAllocation;
    occurrence def Crash;
    flow def FuelFlow;
    view def SystemOverview;
    viewpoint def EngineerPOV;
    rendering def StandardLayout;
    metadata def ReviewStatus;

    // === Special shapes ===
    comment about Vehicle /* This is a vehicle model */

    // === Relationships (edge types) ===
    part def Car :> Vehicle;
    part engine : Vehicle;
}
