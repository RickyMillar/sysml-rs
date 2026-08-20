// Sprint 11 conformance fixture: requirement/case membership dispatch
// Tests that subject, objective, assume, require, frame, verify, satisfy
// all dispatch to the correct ElementKind.

package RequirementCaseConformance {

    // Basic requirement with constraint members
    requirement def SafetyReq {
        subject vehicle;

        assume constraint safeSpeedAssumption {
            true
        }

        require constraint mustBrake {
            true
        }
    }

    // Concern with framed concern membership (per spec: `frame concern`, not `frame constraint`)
    requirement def EnvironmentalReq {
        frame concern emissions {
            true
        }
    }

    // Verification case with objective and verify
    verification def CheckSafety {
        objective safetyObjective {
            verify SafetyReq;
        }
    }

    // Satisfy requirement usage
    satisfy requirement :>> SafetyReq;

    // Verify requirement usage (via satisfy_requirement grammar rule)
    verify requirement :>> SafetyReq;
}
