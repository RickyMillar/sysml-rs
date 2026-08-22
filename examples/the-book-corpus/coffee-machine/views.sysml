// Smart Coffee Machine: Views, Viewpoints, Stakeholders, and Renderings
//
// Canonical worked example for the full SysML v2 view chain:
//   Stakeholder → Viewpoint → View Definition → View Usage → Rendering
//
// Every diagram a tool can produce against this model is a declared
// `view def` or `view <usage>`. There is no ad-hoc "show me this file
// as an interconnection" — the spec models views as authored artifacts.
//
// Division of labour (SysML.xtext ViewDefinitionBodyItem / ViewBodyItem):
//   view def  = the reusable recipe — supertype, `filter`, `render`
//   view      = the concrete selection — `expose`, `satisfy`
// `expose` is legal only in a view USAGE body, never in a `view def`.
// See the-book/src/ch13-metadata-and-views.md.

package Views {
    import Definitions::*;
    import Requirements::*;
    // No wildcard import of StandardViewDefinitions: that package re-exports
    // SysML::*, which would make the bare `kind` filter bindings below
    // ambiguous. The standard view definitions are referenced qualified.

    // ─────────────────────────────────────────────────────────────────
    // Stakeholders — the people / roles whose concerns drive the views.
    // Stakeholders are PartUsage instances of a stakeholder PartDefinition.
    // ─────────────────────────────────────────────────────────────────

    part def SystemArchitect;
    part def SafetyEngineer;
    part def TestEngineer;

    // ─────────────────────────────────────────────────────────────────
    // Viewpoints — the stakeholder concerns, expressed as requirements.
    // (`viewpoint def` is a kind of `requirement def`.)
    // ─────────────────────────────────────────────────────────────────

    viewpoint def StructuralCompletenessViewpoint {
        stakeholder architect : SystemArchitect;
        doc /* All physical components must be visible and their connections
             * traceable. Used at architecture review.
             */
    }

    viewpoint def SafetyComplianceViewpoint {
        stakeholder reviewer : SafetyEngineer;
        doc /* All safety-critical requirements must have visible
             * satisfaction links and verification cases.
             */
    }

    viewpoint def VerificationCoverageViewpoint {
        stakeholder tester : TestEngineer;
        doc /* Every requirement must have at least one verification case
             * and a current verdict.
             */
    }

    // ─────────────────────────────────────────────────────────────────
    // Renderings — reusable visual styles. A view definition binds one
    // with an inline `render rendering <name> : <RenderingDef>;` clause.
    // ─────────────────────────────────────────────────────────────────

    rendering def StandardInterconnection;
    rendering def TraceabilityTable;

    // ─────────────────────────────────────────────────────────────────
    // View Definitions — recipes. Each one specializes a standard view
    // definition from StandardViewDefinitions, binds a rendering, and
    // may declare filters. View usages instantiate them with concrete
    // content (`expose`) and declare which viewpoints they satisfy.
    // ─────────────────────────────────────────────────────────────────

    // 1) Architectural overview — what parts make up the machine and
    //    how they connect. Spec-faithful interconnection diagram.
    view def StructuralOverviewView :> StandardViewDefinitions::InterconnectionView {
        render rendering structuralStyle : StandardInterconnection;
        filter kind == "PartUsage" or kind == "PartDefinition";
    }

    view structuralOverview : StructuralOverviewView {
        satisfy StructuralCompletenessViewpoint;
        expose CoffeeMachine;
    }

    // 2) Safety review — only requirements plus their satisfaction links.
    //    The standard library defines NO RequirementView; the library's
    //    GeneralView doc prescribes this instead: specialize GeneralView
    //    and narrow it with a requirement filter. Two filters compose as AND.
    view def SafetyReviewView :> StandardViewDefinitions::GeneralView {
        render rendering safetyStyle : TraceabilityTable;
        filter kind == "RequirementDefinition" or kind == "RequirementUsage";
        // A predicate over a metadata attribute -- for example
        //   filter @SafetyClassification.level == "SIL-2";
        // is not accepted by the current parser, so this view filters on
        // element kind instead.
    }

    view safetyReview : SafetyReviewView {
        satisfy SafetyComplianceViewpoint;
        expose Requirements::*;
    }

    // 3) Verification dashboard — every requirement and every verification
    //    case, side by side. Same GeneralView-plus-filter recipe, and the
    //    usage demonstrates namespace expose pulling content from two
    //    packages into a single view.
    view def VerificationDashboardView :> StandardViewDefinitions::GeneralView {
        render rendering dashboardStyle : TraceabilityTable;
        filter kind == "RequirementDefinition"
            or kind == "RequirementUsage"
            or kind == "VerificationCaseDefinition"
            or kind == "VerificationCaseUsage";
    }

    view verificationDashboard : VerificationDashboardView {
        satisfy VerificationCoverageViewpoint;
        expose Requirements::*;
        expose Verification::*;
    }

    // 4) Action-flow view — the brew cycle, ordered by control flow.
    //    Different supertype, different shape; no other change.
    view def BrewSequenceView :> StandardViewDefinitions::ActionFlowView {
        render rendering brewStyle : StandardInterconnection;
    }

    view brewSequence : BrewSequenceView {
        expose Actions::BrewCycle;
    }

    // 5) Anonymous scratch view — a one-shot view used while authoring.
    //    Lives in the file like any other view; rename it when you want
    //    to keep it. This is the spec-faithful replacement for
    //    "right-click → show as IBD".
    view scratch : StandardViewDefinitions::InterconnectionView {
        expose CoffeeMachine::brewer;
    }
}
