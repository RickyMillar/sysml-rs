// ===========================================================================
// TEST: Hover information
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. HOVER OVER DEFINITIONS:
//      - "Engine" (line 21) → should show:
//        ```
//        part def Engine
//        ```
//        with specialization info if present
//
//      - "Powertrain" (line 26) → should show:
//        ```
//        part def Powertrain
//        ```
//
//   2. HOVER OVER USAGES:
//      - "horsepower" (line 22) → should show:
//        ```
//        attribute horsepower : Real
//        ```
//
//      - "engine" (line 28) → should show:
//        ```
//        part engine : Engine
//        ```
//
//   3. HOVER OVER KEYWORDS:
//      - "part" keyword → should show SysML syntax documentation
//      - "attribute" keyword → should show attribute syntax doc
//      - "constraint" keyword → should show constraint syntax doc
//      - "package" keyword → should show package syntax doc
//
//   4. HOVER OVER QUALIFIED NAMES:
//      - "Drivetrain::engine" references → should show qualified path
//
// ===========================================================================

package HoverTest {
    private import ScalarValues::Integer;
    private import ScalarValues::Real;
    private import ScalarValues::String;


    part def Engine {
        attribute horsepower : Real;
        attribute displacement : Real;
    }

    part def Powertrain {
        part engine : Engine;
        attribute gearCount : Integer;
    }

    part def Drivetrain {
        part powertrain : Powertrain;
        attribute driveType : String;
    }

    part myCar : Drivetrain {
        attribute driveType = "AWD";
    }
}
