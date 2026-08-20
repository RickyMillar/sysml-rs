// Smart Coffee Machine: Typing, Specialization, and Redefinition
// Demonstrates: specialization (:>), redefinition (:>>), subsetting, typing

package TypingAndSpecialization {
    import Definitions::*;

    // Specialization: EspressoMachine is a more specific CoffeeMachine
    part def EspressoMachine :> CoffeeMachine {
        doc /* A high-pressure espresso machine specializing CoffeeMachine.
             * Redefines brewer with a specialized PressureBrewer.
             */

        // Redefinition: replace the general Brewer with a specialized one
        part brewer : PressureBrewer :>> CoffeeMachine::brewer;
    }

    // Specialized brewer with higher pressure capability
    part def PressureBrewer :> Brewer {
        // Subset the base pressure with a constrained range
        attribute brewPressure :>> Brewer::brewPressure;
        attribute maxPressure : Real;  // bar, typically 15 for espresso
    }

    // Another specialization: commercial-grade machine
    part def CommercialMachine :> CoffeeMachine {
        doc /* A commercial machine with dual grinders and larger tank. */

        // Subsetting: add a second grinder alongside the inherited one
        part fineGrinder :> grinder;
        part coarseGrinder : Grinder;

        // Redefine water tank with larger capacity
        part waterTank : LargeWaterTank :>> CoffeeMachine::waterTank;
    }

    part def LargeWaterTank :> WaterTank {
        // Inherits maxCapacity, currentLevel from WaterTank
    }

    // Typing: a specific machine instance typed by EspressoMachine
    part myEspresso : EspressoMachine {
        attribute serialNumber :>> CoffeeMachine::serialNumber = "ESP-2026-001";
    }
}
