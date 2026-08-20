// ===========================================================================
// FIXTURE: Health diagnostics showcase (AX / FL / VC)
// ===========================================================================
//
// Open this file in the editor to exercise action, flow, and verification
// health diagnostics. Every code below should produce at least one
// diagnostic when the LSP pipeline runs.
//
// Expected diagnostics:
//
//   ACTIONS (AX):
//     EmptyAction         -> AX001  (no action steps)
//     ImplicitSequential  -> AX002  (no explicit control flow)
//     BadDecision         -> AX003  (decision node, no outgoing branches)
//     ForkNoJoin          -> AX004  (fork without matching join)
//     IslandStep          -> AX005  (unreachable action step)
//     CallMissing         -> AX006  (perform references unknown action)
//
//   FLOWS (FL):
//     noSourceFlow        -> FL001  (missing source endpoint)
//     noTargetFlow        -> FL002  (missing target endpoint)
//     selfLoop            -> FL003  (source == target)
//     multicast pair      -> FL004  (multiple flows from same source)
//     badTypeFlow         -> FL005  (unknown payload type)
//     orderedDelivery     -> FL006  (succession flow — informational)
//
//   VERIFICATION (VC):
//     EmptyVerification   -> VC001  (no requirements)
//     BadConstraint       -> VC002  (invalid constraint expression)
//     NoConstraintReq     -> VC003  (requirement with no constraint)
//     AllBadConstraints   -> VC004  (all constraints failed to compile)
//     DanglingRef         -> VC005  (references unknown requirement)
//
// ===========================================================================

package HealthDiagnosticShowcase {

    // ===================================================================
    // ACTIONS
    // ===================================================================

    // AX001: Action definition with no steps at all
    action def EmptyAction;

    // AX002: Steps exist but no explicit succession / transition edges
    action def ImplicitSequential {
        action step1;
        action step2;
        action step3;
    }

    // AX003: Decision node with zero outgoing branches
    action def BadDecision {
        action start;
        decide node checkResult;

        // succession from start to checkResult, but nothing leaves the decision
        succession first start then checkResult;
    }

    // AX004: Fork node without a matching join
    action def ForkNoJoin {
        action begin;
        fork node split;
        action branchA;
        action branchB;

        succession first begin then split;
        succession first split then branchA;
        succession first split then branchB;
        // no join node — parallel branches never reconverge
    }

    // AX005: Unreachable step ("island" disconnected from the flow)
    action def IslandStep {
        action alpha;
        action beta;
        action island;  // no incoming edge

        succession first alpha then beta;
    }

    // AX006: perform references an action that doesn't exist
    action def CallMissing {
        perform doSomethingUndefined;
    }

    // ===================================================================
    // FLOWS
    // ===================================================================

    part def Sensor {
        out port reading;
    }
    part def Controller {
        in port input;
    }
    part def Logger {
        in port data;
    }

    part sensor : Sensor;
    part controller : Controller;
    part logger : Logger;

    // FL001: Flow with no source
    flow noSourceFlow to controller.input;

    // FL002: Flow with no target
    flow noTargetFlow from sensor.reading;

    // FL003: Self-loop (source and target are the same port)
    flow selfLoop from sensor.reading to sensor.reading;

    // FL004: Multiple flows from the same source (multicast warning)
    flow fanout1 from sensor.reading to controller.input;
    flow fanout2 from sensor.reading to logger.data;

    // FL005: Payload type that doesn't exist in the model
    flow badTypeFlow : NonExistentPayload from sensor.reading to controller.input;

    // FL006: Succession flow (informational — delivery blocked until source completes)
    succession flow orderedDelivery from sensor.reading to controller.input;

    // ===================================================================
    // VERIFICATION CASES
    // ===================================================================

    // VC001: Verification case with no requirements
    verification def EmptyVerification;

    // VC002 + VC004: Requirement with invalid constraint expression
    requirement def BadConstraint {
        doc /* speed <<< 100 is not valid operator syntax */
    }

    // VC003: Requirement with no constraint property at all
    requirement def NoConstraintReq {
        doc /* This requirement has text but no constraint */
    }

    // VC005: Verification case that references a requirement by name
    //        but the requirement doesn't exist in the model
    verification def DanglingRef {
        // references "SafetyReq" which is not defined anywhere
    }
}
