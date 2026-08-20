// Visualization Coverage Test: Grid / Traceability Matrix
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Switch to "Grid" view
//
// WHAT TO CHECK:
// - Requirements on one axis: speedReq, weightReq, safetyReq
// - Parts on other axis: engine, chassis, brakes
// - satisfy edges shown as filled cells in matrix
// - engine → speedReq: filled cell
// - chassis → weightReq: filled cell
// - brakes → safetyReq: filled cell

package GridMatrix {
    requirement def SpeedReq {
        doc /* Max speed requirement */
    }

    requirement def WeightReq {
        doc /* Max weight requirement */
    }

    requirement def SafetyReq {
        doc /* Safety requirement */
    }

    requirement speedReq : SpeedReq;
    requirement weightReq : WeightReq;
    requirement safetyReq : SafetyReq;

    part def Engine {
        satisfy speedReq;
    }

    part def Chassis {
        satisfy weightReq;
    }

    part def Brakes {
        satisfy safetyReq;
    }

    part engine : Engine;
    part chassis : Chassis;
    part brakes : Brakes;
}
