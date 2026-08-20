// ===========================================================================
// TEST: Constraint violation diagnostics + Hover + Code Lenses
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. CONSTRAINT DIAGNOSTICS (squiggly underlines):
//      - "speedLimit" on line 29 should show WARNING (C001):
//        speed is 105 which violates `speed < 100`
//      - "fuelCheck" on line 30 should show INFO (C002):
//        fuel is 5 which violates `fuel > 10`
//      - "massCheck" on line 31 should show NO diagnostic:
//        mass is 2500 which satisfies `mass > 0`
//
//   2. HOVER INFO:
//      - Hover over "speed" (line 27) → should show type + value
//      - Hover over "Vehicle" (line 17) → should show part definition info
//      - Hover over "constraint" keyword → should show syntax doc
//      - Hover over "speedLimit" (line 29) → should show constraint info
//
//   3. CODE LENSES (above constraint lines):
//      - speedLimit → should show "FAIL" lens
//      - fuelCheck  → should show "FAIL" lens
//      - massCheck  → should show "PASS" lens
//
//   4. EVALUATE COMMAND:
//      - Place cursor on "speed" (line 27), run: sysml.evaluate
//      - Should show "105" in the output panel
//
// ===========================================================================

package SimpleVehicle {
private import ScalarValues::Real;


part def Vehicle {
    attribute mass : Real;
    attribute speed : Real;
    attribute fuel : Real;
}

// Values and constraints MUST be siblings (same scope) for
// live evaluation to work. The constraint monitor evaluates
// constraints using sibling attribute values as context.

    part vehicle : Vehicle {
    attribute mass = 2500;
    attribute speed = 105;
    attribute fuel = 5;

// This SHOULD show a WARNING diagnostic — speed(105) >= 100
    assert constraint speedLimit { speed < 100 }

// This SHOULD show an INFO diagnostic — fuel(5) <= 10
    constraint fuelCheck { fuel > 40 }

// This should show NO diagnostic — mass(2500) > 0
    assert constraint massCheck { mass > 0 }

}

}
