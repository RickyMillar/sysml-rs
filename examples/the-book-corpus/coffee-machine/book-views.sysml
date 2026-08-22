// Smart Coffee Machine: Declared views backing the book's embedded diagrams.
//
// Every diagram iframe in the book renders one of the views below (plus
// Views::brewSequence, which already backs the brew-cycle flow embed).
// One view per embed; the fixture-baking step consumes tools/embed-views.map
// to pair each view with its fixture stem.
//
// House rules (see views.sysml and src/ch13-metadata-and-views.md):
//   view def  = the reusable recipe — supertype, `filter`, `render`
//   view      = the concrete selection — `expose`
// Plain views are typed directly by a standard view definition; only the
// requirements trace needs a local `view def` to carry its metaclass filter.
// Standard view definitions are referenced qualified, matching views.sysml.

package BookViews {

    // ── Chapter 1 / 3 / 11: everything in the definitions model ──
    view definitionsOverview : StandardViewDefinitions::GeneralView {
        expose Definitions::*;
    }

    // ── Chapter 4: the package containment tree ──
    view packageTree : StandardViewDefinitions::BrowserView {
        expose CoffeeMachineDomain::*;
    }

    // ── Chapter 5: ports and interface definitions ──
    view portsOverview : StandardViewDefinitions::GeneralView {
        expose Ports::*;
    }

    // ── Chapter 6: the connected machine as an interconnection diagram ──
    view connectionOverview : StandardViewDefinitions::InterconnectionView {
        expose Connections::ConnectedCoffeeMachine;
    }

    // ── Chapter 7: the actions model as a general projection ──
    view actionsOverview : StandardViewDefinitions::GeneralView {
        expose Actions::*;
    }

    // ── Chapter 11: every action definition, rendered as action flows ──
    view actionFlows : StandardViewDefinitions::ActionFlowView {
        expose Actions::*;
    }

    // ── Chapters 8 / 11: the composite lifecycle state machine ──
    view lifecycleStates : StandardViewDefinitions::StateTransitionView {
        expose States::CoffeeMachineLifecycle;
    }

    // ── Chapter 9: calculations and the constrained machine ──
    view calculationsOverview : StandardViewDefinitions::GeneralView {
        expose Calculations::*;
    }

    // ── Chapters 10 / 11: requirements traceability ──
    // The standard library defines NO RequirementView; per GeneralView's
    // own library doc, specialize GeneralView and narrow it with a
    // requirement-metaclass filter. Verification cases and satisfy links
    // are part of the trace, so the filter admits them too.
    view def RequirementsTraceView :> StandardViewDefinitions::GeneralView {
        filter @SysML::RequirementDefinition
            or @SysML::RequirementUsage
            or @SysML::SatisfyRequirementUsage
            or @SysML::VerificationCaseDefinition
            or @SysML::VerificationCaseUsage;
    }

    view requirementsTrace : RequirementsTraceView {
        expose Requirements::*;
        expose Verification::*;
    }

    // ── Chapter 10: the same requirement set as a grid ──
    view requirementsGrid : StandardViewDefinitions::GridView {
        expose Requirements::*;
        expose Verification::*;
    }

    // ── Chapter 12: specialization hierarchy ──
    view specializationOverview : StandardViewDefinitions::GeneralView {
        expose TypingAndSpecialization::*;
    }

    // ── Chapter 13: the views model itself, as a containment tree ──
    view viewsTree : StandardViewDefinitions::BrowserView {
        expose Views::*;
    }

    // ── Chapter 13: metadata definitions and the annotated machine ──
    view metadataOverview : StandardViewDefinitions::GeneralView {
        expose CoffeeMachineMetadata::*;
    }
}
