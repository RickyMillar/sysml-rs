// 09 — All eight standard view definitions.
//
// Asserts: each of the eight standard view definitions from the library
// package StandardViewDefinitions resolves and produces a distinct
// rendered shape: GeneralView, InterconnectionView, ActionFlowView,
// StateTransitionView, SequenceView, GeometryView, GridView, BrowserView.
// There is NO standard UseCaseView or RequirementView — for those slices
// the library's GeneralView documentation prescribes a filter recipe,
// shown at the end of this file.

package Demo {
    part def Foo;
    part def Bar {
        part children : Foo[*];
    }

    action def DoFoo;
    action def DoBar;

    state def Idle;
    state def Working;

    item def SpatialProbe;

    requirement def MustWork;
}

package Views {
    import Demo::*;
    import StandardViewDefinitions::*;

    // 1. GeneralView — the most general projection: any exposed elements.
    view def GeneralKindView :> GeneralView;
    view generalKind : GeneralKindView { expose Demo::*; }

    // 2. InterconnectionView — features as nodes, connections as edges.
    view def InterconnectionKindView :> InterconnectionView;
    view interconnectionKind : InterconnectionKindView { expose Bar; }

    // 3. ActionFlowView — actions and the flows between them.
    view def ActionFlowKindView :> ActionFlowView;
    view actionFlowKind : ActionFlowKindView { expose DoFoo; expose DoBar; }

    // 4. StateTransitionView — states and transitions.
    view def StateTransitionKindView :> StateTransitionView;
    view stateTransitionKind : StateTransitionKindView {
        expose Idle;
        expose Working;
    }

    // 5. SequenceView — time-ordered event occurrences on lifelines.
    view def SequenceKindView :> SequenceView;
    view sequenceKind : SequenceKindView { expose DoFoo; }

    // 6. GeometryView — spatial items in two or three dimensions.
    view def GeometryKindView :> GeometryView;
    view geometryKind : GeometryKindView { expose SpatialProbe; }

    // 7. GridView — elements and relationships in a rectangular grid.
    view def GridKindView :> GridView;
    view gridKind : GridKindView { expose Demo::*; }

    // 8. BrowserView — containment hierarchy.
    view def BrowserKindView :> BrowserView;
    view browserKind : BrowserKindView { expose Demo::*; }

    // Not on the list: UseCaseView and RequirementView do not exist in
    // the standard library. The recipe is a GeneralView specialization
    // narrowed by a metaclass filter — here, a requirement view.
    view def RequirementRecipeView :> GeneralView {
        filter @SysML::RequirementDefinition or @SysML::RequirementUsage;
    }
    view requirementRecipe : RequirementRecipeView { expose Demo::*; }
}
