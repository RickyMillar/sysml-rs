// Tier 4: Constraint and requirement execution patterns
// Tests: assert/assume, constraint evaluation, expressions

package ConstraintTests {
    private import ScalarValues::Real;


    // Basic constraint definition
    constraint def PositiveWeight {
        attribute weight : Real;
        weight > 0
    }

    // Assert constraint usage
    part vehicle {
        attribute mass : Real;
        assert constraint massCheck {
            mass > 0 and mass < 10000
        }
    }

    // Constraint with assume/require
    requirement def SafeSpeed {
        subject car : Vehicle;
        assume constraint {
            car.speed >= 0
        }
        require constraint {
            car.speed <= car.maxSpeed
        }
    }

    // Complex expression in constraint
    constraint def ThermalBudget {
        attribute totalPower : Real;
        attribute maxTemp : Real;
        attribute coolingCapacity : Real;

        totalPower * 0.85 <= coolingCapacity * (maxTemp - 25.0)
    }

    // Invariant constraint
    part reactor {
        attribute pressure : Real;
        attribute maxPressure : Real;
        inv { pressure <= maxPressure }
    }

    // Requirement with textual representation
    requirement def DocumentedReq {
        subject sys : System;
        require constraint {
            language "English"
            /* The system shall operate within safe parameters */
        }
    }
}
