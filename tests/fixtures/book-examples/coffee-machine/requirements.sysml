// Smart Coffee Machine: Requirements and Verification
// Demonstrates: requirement def, require, satisfy, verify, use case, subject, stakeholder

package Requirements {
    import Definitions::*;
    import Calculations::*;

    // Stakeholder
    part def Barista;
    part def Customer;

    // Top-level requirement
    requirement def CoffeeMachineRequirements {
        doc /* Top-level requirements for the Smart Coffee Machine. */
        subject machine : CoffeeMachine;
    }

    // Functional requirements
    requirement def BrewTimeReq :> CoffeeMachineRequirements {
        doc /* The machine shall complete a brew cycle in under 60 seconds. */
        subject machine : CoffeeMachine;

        require constraint : BrewTimeConstraint;
    }

    requirement def BrewTempReq :> CoffeeMachineRequirements {
        doc /* Water temperature shall be maintained between 90 and 96 C
             * during extraction.
             */
        subject machine : CoffeeMachine;

        require constraint : BrewTempConstraint;
    }

    requirement def SafetyInterlock :> CoffeeMachineRequirements {
        doc /* The machine shall not dispense coffee unless a cup is detected. */
        subject machine : CoffeeMachine;

        assume constraint {
            machine.cupSensor.cupDetected == true;
        }
    }

    requirement def GrindConsistency :> CoffeeMachineRequirements {
        doc /* Grind particle size shall be within 0.1mm of target. */
        subject machine : CoffeeMachine;

        require constraint : GrindConsistencyConstraint;
    }

    // Satisfaction: link requirements to the design
    part def VerifiedCoffeeMachine :> CoffeeMachine {
        satisfy BrewTimeReq by VerifiedCoffeeMachine;
        satisfy BrewTempReq by VerifiedCoffeeMachine;
        satisfy SafetyInterlock by VerifiedCoffeeMachine;
    }
}

package Verification {
    import Definitions::*;
    import Requirements::*;

    // Verification case for brew time
    verification def BrewTimeTest {
        doc /* Verify that brew cycle completes within 60 seconds. */
        subject machine : CoffeeMachine;

        objective {
            verify BrewTimeReq;
        }

        action measureBrewTime {
            doc /* Start brew cycle and measure elapsed time. */
        }

        action evaluateResult {
            doc /* Compare measured time against 60-second threshold. */
        }

        first measureBrewTime then evaluateResult;
    }

    // Verification case for temperature
    verification def BrewTempTest {
        doc /* Verify water temperature stays in 90-96 C range during extraction. */
        subject machine : CoffeeMachine;

        objective {
            verify BrewTempReq;
        }
    }
}
