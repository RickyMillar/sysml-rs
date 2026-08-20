// Visualization Coverage Test: Actor Rendering (Phase 2)
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Click expand button on DriveVehicle to see children
//
// WHAT TO CHECK:
// - DriveVehicle renders as ellipse (use case shape)
// - When expanded, "driver" and "passenger" render as STICK FIGURES
//   (not rectangles) — this tests the Actor context-aware rendering
// - "vehicle" (subject) renders as a RECTANGLE (Part shape)
// - SafetyReq renders as requirement rectangle
// - When expanded, "inspector" renders as STICK FIGURE

package UseCaseActors {
    use case def DriveVehicle {
        actor driver;
        actor passenger;
        subject vehicle;
    }

    requirement def SafetyReq {
        actor inspector;
    }
}
