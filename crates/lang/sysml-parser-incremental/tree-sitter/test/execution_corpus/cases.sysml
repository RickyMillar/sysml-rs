// Tier 4: Use case, verification case, and analysis case patterns
// Tests: use case defs/usages, verification/analysis case defs

package CaseTests {

    // Use case definition
    case def DriveVehicle {
        subject driver : Person;
        objective {
            require constraint {
                driver.destination != null
            }
        }
    }

    // Verification case definition
    verification def BrakeTest {
        subject vehicle : Vehicle;
        objective verifyBraking {
            require constraint {
                vehicle.brakingDistance <= 50
            }
        }
        action performTest {
            action accelerate;
            action applyBrakes;
            action measureDistance;
        }
    }

    // Analysis case definition
    analysis def ThermalAnalysis {
        subject system : ThermalSystem;
        return result : AnalysisResult;
    }

    // Use case with include
    case def SystemOperation {
        subject operator : Person;

        action startup;
        action normalOperation;
        action shutdown;

        succession startup then normalOperation;
        succession normalOperation then shutdown;
    }

    // Stakeholder and concern
    // NB: `stakeholder` is usage-only (SysML.xtext:2098, no StakeholderDefinition);
    // the stakeholder type is a part def, referenced by stakeholder usages below.
    part def SafetyEngineer;
    concern def SafetyConcern {
        stakeholder safetyEng : SafetyEngineer;
    }

    // Viewpoint definition
    viewpoint def SafetyViewpoint {
        stakeholder safetyEng : SafetyEngineer;
        frame concern : SafetyConcern;
    }
}
