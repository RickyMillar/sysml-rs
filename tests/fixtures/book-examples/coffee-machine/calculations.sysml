// Smart Coffee Machine: Calculations and Constraints
// Demonstrates: calc def, calc usage, constraint def, assert constraint

package Calculations {
    import Definitions::*;
    import ScalarValues::*;
    import NumericalFunctions::*;

    // Calculation: compute extraction time from temperature and pressure
    calc def ExtractionTime {
        doc /* Calculate optimal extraction time based on brew parameters. */
        in brewTemp : Real;       // degrees Celsius
        in brewPressure : Real;   // bar

        // Higher temp and pressure reduce extraction time (seconds)
        return time : Real = 30.0 - (brewTemp - 90.0) * 0.5 - (brewPressure - 9.0) * 1.0;
    }

    // Calculation: water volume needed for a given drink
    calc def WaterVolume {
        in drinkType : DrinkType;
        return volume : Real;     // milliliters
    }

    // Constraint: brew temperature must be in safe range
    constraint def BrewTempConstraint {
        doc /* Water temperature must be between 90 and 96 degrees Celsius. */
        in temp : Real;
        temp >= 90.0 and temp <= 96.0
    }

    // Constraint: brew cycle must complete within time limit
    constraint def BrewTimeConstraint {
        doc /* Total brew cycle must take less than 60 seconds. */
        in totalTime : Real;
        totalTime < 60.0
    }

    // Constraint: grind size must be consistent
    constraint def GrindConsistencyConstraint {
        in actualSize : Real;
        in targetSize : Real;
        abs(actualSize - targetSize) <= 0.1
    }

    // Assert constraints in context of the machine
    part def ConstrainedCoffeeMachine :> CoffeeMachine {
        assert constraint brewTempOk : BrewTempConstraint {
            in temp = brewer.waterTemp;
        }

        assert constraint brewTimeFast : BrewTimeConstraint {
            in totalTime = 45.0;  // design target
        }

        assert constraint grindOk : GrindConsistencyConstraint {
            in actualSize = grinder.grindSize;
            in targetSize = 0.5;
        }
    }
}
