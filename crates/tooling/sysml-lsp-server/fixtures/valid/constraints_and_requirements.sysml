/* Fixture: constraints_and_requirements.sysml
Expected: valid SysML constraint and requirement constructs.
Tests: ConstraintDef/Usage, RequirementDef/Usage, assume/require, satisfy, concern */

package ConstraintsAndRequirements {
    attribute def Mass;
    attribute def Speed;

    constraint def MaxSpeed {
        attribute maxAllowed : Speed;
        require constraint speedLimit;
    }

    constraint def WeightLimit {
        attribute maxMass : Mass;
    }

    requirement def SafetyRequirement {
        doc /* The system shall be safe. */

        subject part thePart;

        assume constraint safeAssumption {
            doc /* Environment is stable. */
        }

        require constraint safetyRule {
            doc /* Must not exceed limits. */
        }
    }

    requirement def PerformanceRequirement {
        attribute targetSpeed : Speed;
    }

    concern def ReliabilityConcern {
        doc /* System must be reliable. */
    }

    part def Vehicle {
        attribute mass : Mass;
        attribute topSpeed : Speed;

        constraint massLimit : WeightLimit;
        requirement safety : SafetyRequirement;

        satisfy requirement safety by Vehicle;
    }
}
