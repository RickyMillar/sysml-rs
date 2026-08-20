// Visualization Coverage Test: All Edge Types (Phase 1)
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Default General view should show edges between top-level elements
//
// WHAT TO CHECK:
// - NO CRASHES (previously 16 edge types would crash)
// - Specialize (Car→Vehicle): solid line, TRIANGLE arrowhead
// - Redefine (Truck→Vehicle): solid line, triangle arrowhead
// - Typing (engine:Vehicle): solid line, open arrowhead
// - Satisfy (SportsCar→MaxSpeed): DASHED line, open arrowhead
// - Dependency: DASHED line, open arrowhead

package AllEdgeTypes {
    // Specialization family (triangle markers)
    part def Vehicle;
    part def Car :> Vehicle;             // edge:specialize
    part def Truck :>> Vehicle;          // edge:redefine

    // Typing
    part engine : Vehicle;               // edge:typing

    // Requirement relationships
    requirement def MaxSpeed;

    part def SportsCar {
        satisfy MaxSpeed;                // edge:satisfy
    }

    // Dependency
    dependency from Car to Vehicle;      // edge:dependency
}

// Top-level relationships visible in general view without expansion
part def Sedan :> AllEdgeTypes::Car;     // edge:specialize (top-level)
requirement def SafetyReq;
part def SafeCar {
    satisfy SafetyReq;                   // edge:satisfy (top-level)
}
