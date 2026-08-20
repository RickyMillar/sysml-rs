// Visualization Coverage Test: Requirements with Traceability Relationships
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Switch to "Requirements" view
//
// WHAT TO CHECK:
// - Requirements render as rectangles with doc text
// - satisfy edge (dashed, open arrow): system → perfReq
// - satisfy edge: system → safetyReq
// - DerivedReq shown as standalone requirement node
// - safetyReq and perfReq show doc text in compartment

package RequirementsTraceability {
    requirement def PerformanceReq {
        doc /* System shall meet performance targets */
    }

    requirement def SafetyReq {
        doc /* System shall be safe */
    }

    requirement def DerivedReq {
        doc /* Derived from performance: 0-100 km/h in 8 seconds */
    }

    requirement perfReq : PerformanceReq;
    requirement safetyReq : SafetyReq;
    requirement derivedReq : DerivedReq;

    part def SystemPart {
        satisfy perfReq;
        satisfy safetyReq;
    }
}
