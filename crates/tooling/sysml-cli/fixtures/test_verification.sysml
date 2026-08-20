// ===========================================================================
// TEST: Verification cases + Code lenses
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. CODE LENSES (above verification cases):
//      - "SafetyCheck" (line 45) → should show:
//        "[Run Verification]" button
//        Verdict lens: "PASS (2/2)" or similar
//
//      - "SpeedViolation" (line 52) → should show:
//        "[Run Verification]" button
//        Verdict lens: "FAIL" (speed 250 violates < 200)
//
//   2. CONSTRAINT LENSES (above constraint lines):
//      - speedLimit → "PASS" or "FAIL" depending on the owning part
//      - positiveWeight → "PASS"
//
//   3. COMMANDS TO TEST:
//      - Place cursor on "SafetyCheck", run: sysml.verify
//        Should report pass/fail verdict in output panel
//
//      - Run: sysml.workspace.verify (no cursor needed)
//        Should find ALL verification cases in the workspace
//        and report aggregate results
//
// ===========================================================================

package VerificationTest {
    private import ScalarValues::Real;


    part def Vehicle {
        attribute speed : Real;
        attribute mass : Real;
    }

    // Safe vehicle — all constraints should pass
    part safeVehicle : Vehicle {
        attribute speed = 120;
        attribute mass = 1500;

        assert constraint speedLimit { speed < 200 }
        assert constraint positiveWeight { mass > 0 }

        verification def SafetyCheck {
            // This verification should PASS
            // because speed(120) < 200 and mass(1500) > 0
        }
    }

    // Unsafe vehicle — speed constraint should fail
    part unsafeVehicle : Vehicle {
        attribute speed = 250;
        attribute mass = 800;

        assert constraint speedLimit { speed < 200 }
        assert constraint positiveWeight { mass > 0 }

        verification def SpeedViolation {
            // This verification should FAIL
            // because speed(250) >= 200
        }
    }
}
