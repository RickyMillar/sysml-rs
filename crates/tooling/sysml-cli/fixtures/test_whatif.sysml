// ===========================================================================
// TEST: What-If analysis + Parameter sweep
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. CONSTRAINT DIAGNOSTICS:
//      - "speedLimit" (line 46) → WARNING: speed(85) < 100, so PASS
//      - "overweightCheck" (line 47) → WARNING: mass(3500) > 3000 is FALSE → FAIL
//      - "positiveSpeed" (line 48) → PASS: speed(85) > 0
//
//   2. WHAT-IF COMMAND:
//      - Place cursor on "speed" (line 43), run: sysml.whatif
//      - When prompted, enter override value: 150
//      - Should show comparison:
//        BASELINE: speedLimit=PASS, overweightCheck=FAIL, positiveSpeed=PASS
//        OVERRIDE: speedLimit=FAIL (flipped!), overweightCheck=FAIL, positiveSpeed=PASS
//      - The "speedLimit" constraint flips from PASS to FAIL
//
//      - Place cursor on "mass" (line 44), run: sysml.whatif
//      - Enter override value: 2000
//      - Should show "overweightCheck" flips from FAIL to PASS
//
//   3. PARAMETER SWEEP COMMAND:
//      - Place cursor on "speed" (line 43), run: sysml.whatif.sweep
//      - Parameters: start=0, end=200, steps=11
//      - Should show a table:
//        value=0   → speedLimit=PASS, positiveSpeed=FAIL
//        value=20  → speedLimit=PASS, positiveSpeed=PASS
//        value=100 → speedLimit=FAIL, positiveSpeed=PASS  ← threshold!
//        value=200 → speedLimit=FAIL, positiveSpeed=PASS
//
//   4. CODE LENSES:
//      - Each constraint should show PASS/FAIL lens above it
//
// ===========================================================================

package WhatIfTest {
    private import ScalarValues::Real;


    part def Truck {
        attribute speed : Real;
        attribute mass : Real;
        attribute fuelLevel : Real;
    }

    part myTruck : Truck {
        attribute speed = 85;
        attribute mass = 3500;
        attribute fuelLevel = 40;

        assert constraint speedLimit { speed < 100 }
        constraint overweightCheck { mass < 3000 }
        assert constraint positiveSpeed { speed > 0 }
        constraint fuelWarning { fuelLevel > 20 }
    }
}
