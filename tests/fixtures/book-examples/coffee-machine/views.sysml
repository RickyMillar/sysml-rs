// Smart Coffee Machine: Views and Viewpoints
// Demonstrates: view def, viewpoint def, view usage, expose, rendering

package Views {
    import Definitions::*;
    import Requirements::*;

    // Viewpoint: what the structural architect cares about
    viewpoint def StructuralViewpoint {
        doc /* Shows the physical decomposition and connections
             * of the coffee machine.
             */

        frame concern structuralCompleteness {
            doc /* Are all physical components identified and connected? */
        }
    }

    // Viewpoint: what the safety engineer cares about
    viewpoint def SafetyViewpoint {
        doc /* Shows safety-critical requirements and their
             * satisfaction by design elements.
             */

        frame concern safetyCompliance {
            doc /* Do all safety requirements have verified satisfaction? */
        }
    }

    // Viewpoint: what the test engineer cares about
    viewpoint def VerificationViewpoint {
        doc /* Shows requirements and their verification status. */

        frame concern verificationCoverage {
            doc /* Does every requirement have at least one verification case? */
        }
    }

    // View: structural overview of the machine
    view structuralOverview : StructuralViewpoint {
        expose Definitions::CoffeeMachine;
        expose Definitions::Grinder;
        expose Definitions::Brewer;
        expose Definitions::WaterTank;
    }

    // View: requirements traceability
    view requirementsTrace : VerificationViewpoint {
        expose Requirements::BrewTimeReq;
        expose Requirements::BrewTempReq;
        expose Requirements::SafetyInterlock;
        expose Verification::BrewTimeTest;
        expose Verification::BrewTempTest;
    }

    // Rendering definition for a tabular format
    rendering def TableRendering {
        doc /* Render view contents as a table with columns for
             * element name, type, and status.
             */
    }
}
