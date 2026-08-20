package CellViews {
    doc /* Declared views over the production cell (COV UI-DIAGRAM / UI-VIEWS,
     * CELL-16): geometry, interconnection, state, action, requirement, and
     * traceability viewpoints. Each specializes a standard view definition the
     * spec-canonical way (`:>`) and exposes the relevant subtree. */

    private import ProductionCellStructure::*;
    private import CellLayout::*;
    private import BrewStationStructure::*;
    private import RecipeExecutionBehaviour::*;
    private import ProductQualityRequirements::*;
    private import ScenarioVerification::*;

    // 1. Geometry / layout view — the bench placement.
    view def CellGeometryView {
        expose BenchLayout;
    }

    // 2. Interconnection view — plants, stations, and their exchange planes.
    view def CellInterconnectionView :> Interconnection {
        expose ProductionCell;
    }

    // 3. State-transition view — a station's lifecycle SM.
    view def StationStateView :> StateTransition {
        expose BrewStation;
    }

    // 4. Action view — the fork/join extraction recipe.
    view def RecipeActionView :> Action {
        expose RecipeExecution;
    }

    // 5. Requirement view — the product-quality requirements.
    view def QualityRequirementView {
        expose ProductQualityRequirements;
    }

    // 6. Traceability view — verification cases to the requirements they verify.
    view def TraceabilityView {
        expose ScenarioVerification;
    }

    // A concrete view USAGE (not just a def) of the traceability view, exposing
    // the verification cases and the product-quality requirements they verify.
    // This is the ViewUsage the verdict-overlay / canvas-join consumers bind to
    // (a declared view instance, distinct from the view *definitions* above).
    view verificationTraceability : TraceabilityView {
        expose ScenarioVerification::*;
        expose ProductQualityRequirements::*;
    }
}
