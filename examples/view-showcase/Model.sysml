// Domain model used by the view showcase.
//
// Carries one element of every kind a `view def` might expose:
// parts with internal structure + ports + connections (for
// Interconnection), states with transitions (for StateTransition),
// actions with succession (for ActionFlow), requirements with
// satisfy / verify (for Requirements / Grid), constraints with
// parameters (for Parametric), and spatial attributes (for Geometry).

package Showcase {

    // ── Parts with structure, ports, and connections ─────────────────

    part def Engine {
        port fuelIn;
        port torqueOut;
        attribute power = 100.0;
        // Spatial — for GeometryView
        attribute x = 50.0;
        attribute y = 100.0;
        attribute width = 200.0;
        attribute height = 150.0;
    }

    part def Gearbox {
        port torqueIn;
        port wheelOut;
        attribute ratio = 4.5;
        attribute x = 280.0;
        attribute y = 100.0;
        attribute width = 150.0;
        attribute height = 100.0;
    }

    part def Wheel {
        attribute radius = 0.3;
    }

    // Message payload carried by the engine→gearbox flow below — gives the
    // Sequence view (CommsView) a real behavioral exchange to render.
    item def Torque;

    part def Vehicle {
        part engine : Engine;
        part gearbox : Gearbox;
        part frontLeft : Wheel;
        part frontRight : Wheel;
        // Internal connections — exercised by Interconnection / IBD
        connection engineToGearbox connect engine.torqueOut to gearbox.torqueIn;
        // Behavioral message — exercised by Sequence (CommsView). Spec syntax:
        // FlowDeclaration (SysML.xtext) — `flow <name> of <payload> from <end> to <end>`.
        flow torqueFlow of Torque from engine.torqueOut to gearbox.torqueIn;
    }

    // ── State machines ────────────────────────────────────────────────

    state def DriveStateMachine {
        state idle;
        state driving;
        state braking;
        transition first idle then driving;
        transition first driving then braking;
        transition first braking then idle;
    }

    // ── Action flows ──────────────────────────────────────────────────

    action def StartSequence {
        action checkSafety;
        action ignite;
        action accelerate;
        succession first checkSafety then ignite;
        succession first ignite then accelerate;
    }

    // ── Requirements ──────────────────────────────────────────────────

    requirement def SafetyReq {
        doc /* The vehicle shall start safely under all operating conditions. */
    }

    requirement def PerformanceReq {
        doc /* The vehicle shall reach 60mph in under 5 seconds. */
    }

    requirement def DurabilityReq {
        doc /* The drivetrain shall withstand 200,000 km of operation. */
    }

    // ── Constraints (for Parametric) ──────────────────────────────────

    constraint def MaxPowerConstraint {
        doc /* engine.power <= 200 */
    }

    constraint def TorqueBalance {
        doc /* engine.torqueOut == gearbox.torqueIn */
    }
}

// ── Verification cases (evidence for the Grid / traceability matrix) ─────
//
// Kept in its OWN top-level package, deliberately: `Showcase::*` is exposed
// recursively by AllPartsView / CatalogView / NothingView, so adding these
// cases inside `Showcase` would change those tiles too. The traceability
// matrix scopes only its ROWS to the exposed subtree — columns are drawn from
// whatever verifies those rows, wherever it lives — so a sibling package
// gives TraceMatrixView a real 2D grid without disturbing any other view.
//
// Spec syntax (SysML-spec-r2025-04):
//  - §8.2.2.24 Verification Cases Textual Notation:
//      VerificationCaseDefinition = OccurrenceDefinitionPrefix
//                                   'verification' 'def'
//                                   DefinitionDeclaration CaseBody
//      RequirementVerificationMember = MemberPrefix 'verify'
//                                      { kind = 'requirement' } …
//  - §8.3.24.2 RequirementVerificationMembership /
//    validateRequirementVerificationMembershipOwningType: the verified
//    RequirementUsage must be owned by an ObjectiveMembership — i.e. the
//    `verify requirement …` member MUST sit inside an `objective { }` block,
//    not directly in the case body.
//  - Systems Library `VerificationCases.sysml` (normative model library):
//    VerificationCase :> Case with `subject subj`, `objective obj`, and
//    `requirementVerifications : RequirementCheck[0..*]`.
package ShowcaseVerification {
    private import Showcase::*;

    // Tests:
    //  - `verification def` + `objective { verify requirement … : Req; }`
    //  - one case → one requirement: the diagonal of the matrix
    verification def SafetyStartTest {
        doc /* Bench start-up test: 500 cold starts under fault injection. */
        subject vehicle : Vehicle;
        objective safetyObjective {
            verify requirement safetyCheck : SafetyReq;
        }
    }

    verification def AccelerationTest {
        doc /* Track test: 0–60mph timing over three runs. */
        subject vehicle : Vehicle;
        objective accelerationObjective {
            verify requirement performanceCheck : PerformanceReq;
        }
    }

    verification def EnduranceTest {
        doc /* Dyno endurance run scaled to 200,000 km of drivetrain duty. */
        subject vehicle : Vehicle;
        objective enduranceObjective {
            verify requirement durabilityCheck : DurabilityReq;
        }
    }

    // Tests:
    //  - ONE case verifying SEVERAL requirements — two `verify requirement`
    //    members under a single objective. Gives the matrix an off-diagonal
    //    column so it is visibly 2D rather than a relabelled list.
    verification def IntegratedRoadTest {
        doc /* Public-road trial covering safe start and acceleration together. */
        subject vehicle : Vehicle;
        objective roadObjective {
            verify requirement roadSafetyCheck : SafetyReq;
            verify requirement roadPerformanceCheck : PerformanceReq;
        }
    }
}
