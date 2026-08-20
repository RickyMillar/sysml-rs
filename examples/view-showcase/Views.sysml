// View showcase — exercises every spec view kind and every ViewUsage
// feature recognised by the backend.
//
// View kind is declared the spec-canonical way: each `view def`
// specializes a standard view definition with `:>` (e.g.
// `:> Interconnection`). There is NO name-suffix inference — the view's
// name is free-form; the `:>` supertype picks the diagram shape. See
// heuristic").
//
// Each view below is annotated with WHAT IT TESTS so a reader can map a
// specific feature to a specific declaration.

package ShowcaseViews {
    private import Showcase::*;

    // A rendering reference used by the WithRendering view below.
    rendering def StandardRendering;

    // ── 1. Default General view, single Expose ───────────────────────
    // Tests:
    //  - basic ViewDefinition discovery
    //  - single Expose with concrete element name
    //  - default ViewType (no `:>` supertype → falls back to General)
    view def OverviewView {
        expose Vehicle;
    }

    // ── 2. Interconnection view ──────────────────────────────────────
    // Tests:
    //  - `:> Interconnection` supertype → ViewType::Interconnection
    //  - canvas centred on the exposed subject — siblings hidden
    view def PowertrainView :> Interconnection {
        expose Vehicle;
    }

    // ── 3. State transition view ─────────────────────────────────────
    // Tests:
    //  - `:> StateTransition` supertype
    //  - Expose pointing at a StateDefinition
    view def DriveModesView :> StateTransition {
        expose DriveStateMachine;
    }

    // ── 4. Action flow view ──────────────────────────────────────────
    // Tests:
    //  - `:> ActionFlow` supertype
    view def StartFlowView :> ActionFlow {
        expose StartSequence;
    }

    // ── 5. Requirement view ──────────────────────────────────────────
    // Tests:
    //  - `:> Requirement` resolves to a GeneralView via the stdlib
    //    supertype chain (Requirement :> RequirementView :> GeneralView).
    //    Requirement notation renders in the General path, gated on
    //    element kind. The requirement filter is pending grammar support
    //    (see requirements-parametric-retirement.md §9.2).
    //  - multiple Expose clauses (every requirement listed)
    view def AllReqsView :> Requirement {
        expose SafetyReq;
        expose PerformanceReq;
        expose DurabilityReq;
    }

    // ── 6. Sequence view ─────────────────────────────────────────────
    // Tests:
    //  - `:> Sequence` supertype
    view def CommsView :> Sequence {
        expose Vehicle;
    }

    // ── 7. Browser view ──────────────────────────────────────────────
    // Tests:
    //  - `:> Browser` supertype
    //  - recursive Expose (`Foo::*`) — exposes the whole Showcase namespace
    view def CatalogView :> Browser {
        expose Showcase::*;
    }

    // ── 8. Grid view (traceability matrix) ───────────────────────────
    // Tests:
    //  - `:> Grid` supertype
    //  - a REAL 2D matrix: requirement rows × verification-case columns,
    //    marked from the `verify requirement` members in
    //    `ShowcaseVerification` (see Model.sysml for the spec citations).
    //  - matrix scoping: the Grid generator scopes ROWS to the first
    //    Expose's subtree, so a single package-recursive Expose is what
    //    puts all three requirements on the rows. The previous
    //    `expose SafetyReq; expose Vehicle;` pair scoped rows to a single
    //    requirement (and Vehicle contributed nothing — it owns no
    //    requirements). Columns are NOT scope-restricted: the cases live
    //    in the sibling `ShowcaseVerification` package and still appear.
    view def TraceMatrixView :> Grid {
        expose Showcase::*;
    }

    // ── 9. Geometry view ─────────────────────────────────────────────
    // Tests:
    //  - `:> Geometry` supertype
    //  - exposing an element whose attrs include x/y/width/height
    view def LayoutView :> Geometry {
        expose Vehicle;
    }

    // ── 10. Constraint ("parametric") view ───────────────────────────
    // Tests:
    //  - SysML v2 has no parametric diagram kind. Constraint–parameter
    //    binding is ordinary Interconnection notation (binding connectors
    //    + constraint blocks), so a "parametric" view is just an
    //    Interconnection view exposing the constraint(s). The retired
    //    `Parametric` own-name escape hatch is gone; declare the base kind
    //    explicitly with `:> Interconnection`. See
    //    requirements-parametric-retirement.md §1.4 / §3.2.
    view def ConstraintView :> Interconnection {
        expose MaxPowerConstraint;
        expose TorqueBalance;
    }

    // ── 11. Multiple Expose, mixed kinds ─────────────────────────────
    // Tests:
    //  - several Expose clauses of different element kinds in one view
    //  - composer picks the FIRST resolvable id as the diagram subject
    //  - other Expose clauses are recorded in `ViewSummary.exposed`
    //  - no `:>` supertype → General
    view def MixedExposeView {
        expose Engine;
        expose Gearbox;
        expose DriveStateMachine;
    }

    // ── 12. Filter — literal true (passes everything) ───────────────
    // Tests:
    //  - ElementFilterMembership discovery and round-trip
    //  - viewCondition expression evaluation: `true` literal returns
    //    Bool(true), so every element exposed passes the filter
    view def AllPartsView {
        expose Showcase::*;
        filter true;
    }

    // ── 13. Filter — literal false (excludes everything) ─────────────
    // Tests:
    //  - viewCondition expression evaluation: `false` literal returns
    //    Bool(false), so the diagram should render empty
    //  - safe-default contract: a non-Boolean / failing expression
    //    returns true, so this MUST be a literal false to actually
    //    exclude
    view def NothingView {
        expose Showcase::*;
        filter false;
    }

    // ── 14. View typed-as a user view def (transitive supertype) ─────
    // Tests:
    //  - `:>` specialization chained through a user view def: TypedAsView
    //    :> PowertrainView :> Interconnection, so the kind walk resolves
    //    TRANSITIVELY to Interconnection.
    view def TypedAsView :> PowertrainView {
        expose Vehicle;
    }

    // ── 15. With render member (rendering reference) ─────────────────
    // Tests:
    //  - render :>> RenderingDef syntax
    //  - ViewRenderingMembership child appears in ViewSummary.renderings
    //  - no `:>` supertype → General
    view def RenderedView {
        expose Vehicle;
        render :>> StandardRendering;
    }

    // ── 16. Anonymous view usage ─────────────────────────────────────
    // Tests:
    //  - `view <name> { ... }` (ViewUsage, not ViewDefinition)
    //  - composer + discovery handle ViewUsage as well as ViewDefinition
    view scratch {
        expose Vehicle;
    }

    // ── 17. View with no Expose at all ──────────────────────────────
    // Tests:
    //  - empty-Expose view doesn't crash; renders with default top-level
    //    selection (no canvas-root override), standard library EXCLUDED
    //    (tracker 3.10 — a no-expose view must not dump the stdlib)
    // SPEC-SILENT: a `view def` with no Expose has no `exposedElement`
    //   derivation (that OCL is on ViewUsage, SysML-spec-r2025-04 ~37385).
    //   Rendering the user model's top-level here is an implementation
    //   choice, not a spec-prescribed contract.
    view def EmptyExposeView;
}
