//! Public result types for the service layer.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Serialize, Deserialize};

// Re-export commonly used types from downstream crates so consumers don't
// need to depend on sysml-core / sysml-runtime / sysml-diagram directly.
pub use sysml_core::{
    query::TraceMatrixRow, Element, ElementId, ElementKind, ExposeRef, ModelGraph,
    RelationshipKind, RenderingRef, Value, ViewSummary,
};
pub use sysml_runtime::cases::VerdictKind;
pub use sysml_span::Diagnostic;

/// One point in a session's time-series query result (ADR-011 §6 /
/// S3.T9 Phase B).
///
/// `time_ms` is the orchestrator's monotonic millisecond timestamp at
/// the tick the value was recorded; `value` is the scalar projection
/// of the named variable at that tick (per
/// `snapshot_view::value_to_scalar` — bool → 0/1, int/float/quantity
/// → raw, complex → real part, everything else dropped).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct TimeSeriesPoint {
    pub time_ms: f64,
    pub value: f64,
}

/// Result of `sysml.sessions.timeseries` and
/// `sysml.sessions.timeseries_decimated`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimeSeriesResult {
    /// Variable name as it appears in the session's
    /// `NormalizedSnapshot::scalar_vars`.
    pub var: String,
    /// Points in oldest → newest order. NaN entries are dropped by
    /// the underlying `TimeSeriesBuffer::series` accessor so chart
    /// libraries don't have to filter them out.
    pub points: Vec<TimeSeriesPoint>,
}

/// Result of `sysml.sessions.timeseries_names` — the variable names
/// currently captured in a session's canonical store.
///
/// Names are sorted for deterministic ordering across calls — callers
/// that key off `names[0]` get a stable identity. Sorting is per-call
/// (the underlying buffer's `HashMap` doesn't preserve insertion
/// order) so a new variable appearing at tick N will sort into its
/// alphabetic slot on the next call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimeSeriesNamesResult {
    pub names: Vec<String>,
    /// Total point count currently in the buffer (one per recorded
    /// tick). Useful for sizing UI controls / progress bars before
    /// requesting actual series data.
    pub len: usize,
    /// Configured ring-buffer capacity per series. Once `len ==
    /// capacity` the oldest points start being overwritten.
    pub capacity: usize,
}

/// Result of `sysml.get_source` (S4.T2): a source slice + byte/line span
/// for one element.
///
/// Wraps the [`sysml-ide-db::file_source_query::FileSourceSlice`] tracked
/// query in a wire-friendly shape. `file` is omitted — callers pass the
/// URI on the request side and don't need it echoed back.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSourceResult {
    /// Source text covered by the element's primary span. Length equals
    /// `end - start` in bytes.
    pub text: String,
    /// Byte offset of the slice start in the file (0-indexed).
    pub start: usize,
    /// Byte offset of the slice end in the file (exclusive).
    pub end: usize,
    /// Start line number (1-indexed). Used by Monaco's
    /// `revealLineInCenter` when the FE scrolls a sneak-peek to the
    /// element. `None` only when the parse layer omitted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Start column number (1-indexed). Pairs with `line` for Monaco
    /// cursor positioning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
}

/// Result of `sysml.workspace.capabilities` (S4.T3): model-content
/// feature flags + name lists for the loaded workspace.
///
/// Replaces the FE's hand-written tree walk in
/// `editors/simulation-app/src/store/workspace.ts:178-321`. Field names
/// map 1-to-1 onto the FE's `Capabilities` interface (the FE's existing
/// `Capabilities` is `camelCase`; T6 will swap its in-memory shape to
/// these snake_case fields). All flags default to `false` so a missing
/// workspace yields a non-capable report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct WorkspaceCapabilitiesResult {
    /// `StateDefinition` or `ExhibitStateUsage` present anywhere in the
    /// workspace. Gates the state-machine panel.
    pub has_state_machines: bool,
    /// `ActionDefinition` or `ActionUsage` present. Gates the action
    /// panel.
    pub has_action_flows: bool,
    /// Aggregate of (a) explicit `MetadataUsage("ToolExecution")` on a
    /// state machine and (b) the FE's "multi-file workspace with state
    /// machines ⇒ orchestrator mode" heuristic. Gates ODE editor and
    /// orchestrator wiring.
    pub has_ode_dynamics: bool,
    /// `PortUsage` or `PortDefinition` present. Gates the port-flow
    /// panel.
    pub has_port_flows: bool,
    /// `state_machine_names.len() + (has_action_flows ? 1 : 0) > 1`.
    /// Gates the subsystem-selector UI.
    pub has_multiple_subsystems: bool,
    /// `ConstraintUsage`, `ConstraintDefinition`, or
    /// `AssertConstraintUsage` present.
    pub has_constraints: bool,
    /// `RequirementDefinition` or `RequirementUsage` present.
    pub has_requirements: bool,
    /// `AnalysisCaseDefinition` or `AnalysisCaseUsage` present. Gates
    /// the trade-study view.
    pub has_trade_studies: bool,
    /// Names of every named `StateDefinition` in the workspace. Used by
    /// the SM selector.
    pub state_machine_names: Vec<String>,
    /// Names of every named `ActionDefinition` in the workspace.
    pub action_flow_names: Vec<String>,
    /// Names of every named `AnalysisCaseDefinition` / `AnalysisCaseUsage`.
    pub trade_study_names: Vec<String>,
}

/// Machine-readable feature-flag response from `sysml.system.capabilities`.
///
/// Frontends gate UI paths on these flags so a new backend feature can be
/// detected without sniffing versions or parsing error strings. Flags are
/// snake_case to match the transport wire format; add new flags at the
/// bottom of the struct to preserve JSON ordering.
///
/// All flags default to `false` so a stale backend is treated as
/// non-capable by default.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceCapabilities {
    /// `sysml.sessions.fork_with_overrides` supports the optional
    /// `at_tick` parameter (R4 — golden-baseline compare replay).
    pub has_fork_at_tick: bool,
    /// Default orchestrator snapshot retention window (in ticks) used by
    /// fork-at-tick. Informational; per-session overrides are not
    /// exposed through this command.
    pub snapshot_retention_ticks: u64,
}

/// Salsa query execution statistics.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SalsaStats {
    /// Number of tracked queries that actually executed.
    pub executions: u64,
    /// Number of cache validations against unchanged inputs.
    pub validations: u64,
    /// `validations / (executions + validations)` — approximate cache hit
    /// rate. Returns `0.0` if both counters are zero.
    pub hit_ratio: f64,
}

/// One row in the discovered-projects list returned by
/// `sysml.workspace.refresh`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRefreshSummary {
    /// Deterministic project ID assigned during refresh (`10 + index`).
    pub id: u32,
    /// Project name from its manifest or directory.
    pub name: String,
    /// Project root directory if it's an on-disk project; `None` for
    /// in-memory or `.kpar` archive projects.
    pub root: Option<String>,
}

/// Result of `sysml.workspace.refresh`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceRefreshResult {
    /// Projects discovered across all workspace roots, sorted by
    /// canonical root path + project name for determinism.
    pub projects: Vec<ProjectRefreshSummary>,
    /// Whether the stdlib bundle was (re-)enabled during this refresh.
    pub stdlib_loaded: bool,
    /// Number of workspace roots that contributed to the refresh.
    pub roots_count: usize,
}

/// Result of resetting salsa query statistics.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SalsaStatsResetResult {
    /// Always `"reset"`.
    pub status: String,
}

/// Result of loading a workspace directory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceLoadResult {
    /// URIs of successfully loaded files.
    pub loaded_uris: Vec<String>,
    /// Number of files that failed to load.
    pub error_count: usize,
    /// Error messages for files that failed to load.
    pub errors: Vec<String>,
}

/// Summary statistics for a model graph.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ElementStats {
    pub total_elements: usize,
    pub total_relationships: usize,
    pub elements_by_kind: HashMap<String, usize>,
    pub relationships_by_kind: HashMap<String, usize>,
}

/// Information about the current project/workspace.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectInfo {
    pub name: Option<String>,
    pub root: std::path::PathBuf,
    pub file_count: usize,
    pub manifest_path: Option<std::path::PathBuf>,
}

/// Static transition descriptor for an SM TreeNode. Lets the
/// frontend render the state-graph SVG without walking
/// `TransitionUsage` children — those are filtered in the
/// `user_facing` model tree view as of R2.1.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SmTransitionDescriptor {
    pub id: ElementId,
    /// Display name of the transition (may be `None` for anonymous
    /// transitions — most are named `state_a_to_state_b`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Source state's short name. Same resolution rules as the
    /// `source` field on a TransitionUsage TreeNode (R2.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Target state's short name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Bound marker for an AttributeUsage TreeNode (R3.3). Encodes a
/// comparison constraint involving the attribute, e.g.
/// `temperature < 80` -> `BoundMarker { y: 80.0, kind: "upper",
/// operator: "<", constraint_name: "thermalSafe" }`.
///
/// Replaces the FE's `boundExtractor.ts` AST walker. The backend is
/// authoritative for which constraints attribute to which attribute,
/// and resolves cross-instance references by ElementId so two
/// circuits sharing a `temperature` short name get separate bounds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BoundMarker {
    pub y: f64,
    /// "upper" | "lower" | "target".
    pub kind: String,
    /// Display name of the owning constraint (`(constraint)` if the
    /// element has no name).
    pub constraint_name: String,
    /// "<" | "<=" | ">" | ">=" | "==".
    pub operator: String,
}

/// User-facing archetype for a TreeNode — coarse grouping used by
/// the simulation UI's tree renderer (R2.4). Lifts the static
/// `KIND_MAP` dictionary that lived in the frontend's
/// `buildModelTree.ts` so the backend is authoritative for
/// classification (the SysML v2 hierarchy is already there — the FE
/// was duplicating it as a dictionary).
///
/// `Calc` and `Ode` are not separated here: ODE detection rides on
/// the existing `is_ode` flag. After R2.4 the FE upgrades
/// `archetype: Calc && is_ode` → `'ode'` (same logic as today,
/// but the base is server-authoritative).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Archetype {
    Part,
    /// PortUsage / PortDefinition (incl. ConjugatedPortDefinition).
    Port,
    Sm,
    /// ActionUsage subtree minus the more-specific archetypes (Sm, Calc,
    /// Case, Constraint already matched). Catches PerformAction / Accept /
    /// Send / Assignment / If / While / For / Loop / Terminate and their
    /// definitions.
    Action,
    /// The case family: CaseUsage / AnalysisCaseUsage /
    /// VerificationCaseUsage / UseCaseUsage / IncludeUseCaseUsage and
    /// their definitions. Per spec these subtype CalculationUsage →
    /// ActionUsage (SysML-vocab.ttl:151-161), so they'd otherwise fall
    /// into `Action` — but they read as their own thing in the UI; a
    /// verification case under an "Actions" header is misleading. Kept
    /// as ONE coarse bucket (not split into Verification / Analysis /
    /// UseCase) until a UX need is shown.
    Case,
    Constraint,
    Calc,
    Attribute,
    /// ConnectionUsage / InterfaceUsage / AllocationUsage / FlowUsage /
    /// SuccessionFlowUsage / BindingConnectorAsUsage and their
    /// Definition counterparts (ConnectionDefinition + Allocation /
    /// InterfaceDefinition, FlowDefinition).
    Connection,
    Section,
    Other,
}

impl Archetype {
    /// Stable sort rank used when ordering sibling TreeNodes (R3.4).
    /// Mirrors the FE's old `KIND_SORT_ORDER` so the simulation tree
    /// surfaces structural content (parts, state machines) before
    /// measurable / derived rows (constraints, calcs, attributes).
    ///
    /// `Section` is FE-injected today (cross-file root section
    /// headers) so it never appears in backend output, but the rank
    /// is included for parity in case that changes.
    pub fn sort_rank(&self) -> u8 {
        match self {
            Archetype::Part => 0,
            Archetype::Port => 1,
            Archetype::Sm => 2,
            Archetype::Action => 3,
            Archetype::Case => 4,
            Archetype::Constraint => 5,
            Archetype::Calc => 6,
            Archetype::Attribute => 7,
            Archetype::Connection => 8,
            Archetype::Section => 9,
            Archetype::Other => 10,
        }
    }
}

/// A node in the model tree (for tree views).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TreeNode {
    /// Tree-position identity — **guaranteed unique within a single
    /// `model_tree` response**. For nodes that appear only once this is
    /// the underlying element id; for nodes reached via typed-definition
    /// inlining (a `PortUsage` typed by a `FlowConnectionDefinition`
    /// whose `power` child gets inlined under each port) a fresh UUID
    /// is minted so React keys, expansion state, and pin state all
    /// disambiguate the two tree positions. The original element id
    /// lives in [`element_id`] for navigation / hover / detail lookups.
    pub id: ElementId,
    pub name: Option<String>,
    pub kind: ElementKind,
    /// User-facing archetype for the simulation UI's tree renderer.
    /// Replaces the FE's static `KIND_MAP` dictionary (R2.4). The FE
    /// upgrades `archetype == Calc && is_ode == true` → its `'ode'`
    /// renderer kind at build time. Always emitted on the wire — every
    /// TreeNode carries a classification, and absent-equals-other lets
    /// buggy callers slip through silently.
    pub archetype: Archetype,
    pub children: Vec<TreeNode>,
    /// Authoritative ODE classification for calc kinds. `true` when the
    /// element is a `CalculationUsage` / `CalculationDefinition` whose
    /// subsetting chain reaches the spec's `GetDerivative` type. Lets
    /// the frontend tree classifier skip the `scalar_vars` heuristic
    /// that used to be the only signal (GAP-ODE-001).
    ///
    /// Serde default + `skip_serializing_if` keeps the wire format
    /// minimal: only calc kinds that are actually ODEs emit the flag,
    /// everything else stays shape-compatible with the previous
    /// JSON.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_ode: bool,
    /// For Usage-kind elements (PartUsage, ItemUsage, PortUsage),
    /// the ElementId of the resolved PartDefinition / ItemDefinition /
    /// PortDefinition they are typed by — or `None` when no
    /// `FeatureTyping` child resolves. Surfaces the same backend
    /// resolution that powers the tree's typed-def inlining, so the
    /// frontend's Usages filter can drop a definition cleanly when
    /// any usage points at it (no more name-string heuristics).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_as: Option<ElementId>,
    /// The original `ElementId` of the model element this node
    /// represents — only emitted when it differs from [`id`], which
    /// happens when the dedupe post-pass reassigned `id` to break a
    /// React-key collision. Consumers that need to open the element's
    /// detail panel, resolve live values, or pin/hover should prefer
    /// `element_id.unwrap_or(id)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    /// For TransitionUsage / SuccessionAsUsage TreeNodes, the source state's
    /// short name (resolved via the element's `source` property; falls back
    /// to `unresolved_source`). Lets the frontend drop the
    /// `parseTransitionName` regex that used to split `state_a_to_state_b`
    /// names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Mirror of [`source`] for the transition's target state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// For AttributeUsage TreeNodes, the SI unit string the backend
    /// inferred for the attribute (e.g. "V", "kg·m/s²"). Sourced from the
    /// element's `unit` property or its typed unit element. Lets the
    /// frontend drop the `metricRegistry` name-keyed unit lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// For AttributeUsage TreeNodes, the canonical ISQ dimension string
    /// (e.g. "L^2·M·T^-3·I^-1"). Sourced from the physics layer's ISQ
    /// inference. None when the element has no resolvable ISQ typing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isq_dimension: Option<String>,
    /// For SM kinds (StateUsage / StateDefinition / ExhibitStateUsage),
    /// the static transition list extracted from the element's
    /// TransitionUsage children at projection time. Lets the frontend
    /// render the SM state-graph without walking children — those are
    /// filtered in `user_facing` view as of R2.1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<SmTransitionDescriptor>,
    /// For AttributeUsage TreeNodes, the inequality / equality bounds the
    /// backend extracted from constraint expressions referencing this
    /// attribute. Replaces the FE's `boundExtractor.ts` walker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bounds: Vec<BoundMarker>,
    /// Hint to the frontend: render this node collapsed on initial load.
    /// Set by the projection layer when an archetype tends to have heavy
    /// fan-out (Port / Connection — typed-def inlining can produce many
    /// signal-attribute children that aren't usually relevant at first
    /// glance). The FE's expand-state store may override this when the
    /// user has explicitly expanded the node.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default_collapsed: bool,
    /// URI of the source file that declares this element, from its first
    /// span. Per-node (not per-response) because the workspace-scoped
    /// tree (`__workspace__`) merges every file into one graph — without
    /// this the FE cannot attribute a node to its file for
    /// click-to-open / source preview. Also stamped on per-file trees,
    /// where it is MORE precise than the request uri: typed-def inlining
    /// surfaces definition children that live in a different file (often
    /// the standard library). `None` for synthetic nodes with no span
    /// (e.g. the depth-cap sentinel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

/// Combined tree + stats for a single loaded URI, used by
/// `sysml.workspace.info` to batch hydration of UI workspace state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceUriInfo {
    /// URI key of the loaded model (matches `sysml.loaded_uris` entries).
    pub uri: String,
    /// Hierarchical root tree for the model (same shape as `sysml.model.tree`).
    pub tree: Vec<TreeNode>,
    /// Element/relationship statistics (same shape as `sysml.stats`).
    pub stats: ElementStats,
}

/// One discovery entry for a workspace root, exposed by
/// `sysml.workspace.info_summary`. Either a successful discovery (with
/// the project list) or an error report. Mirrors the LSP's
/// `handle_workspace_info` shape so that command can delegate.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum WorkspaceDiscoveryEntry {
    /// Successful project discovery for a workspace root.
    Discovered {
        root: String,
        mode: String,
        description: String,
        include_stdlib: bool,
        project_count: usize,
        project_names: Vec<String>,
        project_roots: Vec<String>,
    },
    /// Discovery failed for this root; carries the formatted error.
    Failed { root: String, error: String },
}

/// Per-workspace loaded-state snapshot for `workspace_info_summary`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfoLoaded {
    pub user_projects: usize,
    pub total_projects_including_stdlib: usize,
    pub tracked_files: usize,
}

/// Workspace-level lifecycle summary used by the LSP `sysml.workspace.info`
/// and `sysml.project.info` handlers. Aggregates discovery state, loaded
/// host counts, and transport-supplied telemetry counters into one record.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfoSummary {
    /// Workspace roots passed in by the caller (LSP `workspace_roots`).
    pub workspace_roots: Vec<String>,
    /// Per-root discovery report (success or error).
    pub discovery: Vec<WorkspaceDiscoveryEntry>,
    /// Loaded-host counts: user projects, total (incl. stdlib), tracked files.
    pub loaded: WorkspaceInfoLoaded,
    /// Telemetry counters keyed by name. The LSP populates this from its
    /// global counter map; other transports may pass an empty map.
    pub telemetry_counters: std::collections::BTreeMap<String, u64>,
}

/// Result of evaluating a single constraint occurrence.
///
/// A constraint declared on a definition with N usages produces N
/// `ConstraintResult`s — one per occurrence — distinguished by `instance_*`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConstraintResult {
    /// Constraint name or description.
    pub name: String,
    /// The constraint expression (e.g. "temperature < 300").
    pub expression: Option<String>,
    /// The verdict for this occurrence: Pass, Fail, Inconclusive, or Error.
    ///
    /// Inconclusive (e.g. a value-less / unresolved reference) is distinct from
    /// Fail: the constraint could not be determined, not determined-false. See
    /// `VerdictKind` (SysML v2 VerificationCases.sysml).
    pub verdict: VerdictKind,
    /// The actual computed value (if available).
    pub actual: Option<Value>,
    /// The expected value (if available).
    pub expected: Option<Value>,
    /// Human-readable message explaining the result.
    pub message: Option<String>,
    /// ElementId of the usage occurrence this verdict was evaluated against
    /// (per-instance evaluation). `None` for a constraint with no instantiable
    /// owner (package-level). Omitted from JSON when `None` (backward-compat).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instance_element_id: Option<ElementId>,
    /// Name/path of the usage occurrence. `None` for package-level constraints.
    /// Omitted from JSON when `None`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instance_path: Option<String>,
}

/// One node of an expression AST as projected from the ModelGraph.
///
/// The parser emits expression elements (`OperatorExpression`,
/// `FeatureReferenceExpression`, `InvocationExpression`, literals, …) as
/// child elements; this struct serializes that subtree in a form the
/// frontend can render directly (e.g. as KaTeX) without needing to walk
/// `Element` / `Relationship` shapes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExpressionAstNode {
    /// `ElementKind` debug name (e.g. `"OperatorExpression"`).
    pub kind: String,
    /// `element.name` if set (e.g. function name, variable name, qualified name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// All non-default props (operator, value, isBodyParameter, …).
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub props: HashMap<String, Value>,
    /// Recursively projected child expression elements, in `argIndex` order.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<ExpressionAstNode>,
}

/// One result of `sysml.expression.ast`: the full subtree for a single
/// constraint / attribute / assignment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExpressionAstResult {
    /// Owning element id (e.g. the constraint or attribute).
    pub element_id: ElementId,
    /// Owning element name (if any).
    pub element_name: Option<String>,
    /// Owning element kind (e.g. `"ConstraintUsage"`, `"AttributeUsage"`).
    pub element_kind: String,
    /// Original source text of the expression (from `unresolved_value`),
    /// useful for editor display alongside the rendered AST.
    pub source: Option<String>,
    /// Projected expression tree. `None` if the element has no
    /// structured expression children (e.g. a literal-only attribute).
    pub ast: Option<ExpressionAstNode>,
}

/// Rollup over the per-requirement verdicts of a verification run.
///
/// Pre-computed by the backend from the runtime's `VerdictRollup` so
/// every client (CLI, REST, MCP, report generators) reads one
/// authoritative shape instead of re-aggregating verdict strings.
/// Stays service-owned because the same projection is useful across
/// case-level (this struct) and eventual workspace-level responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct VerifySummary {
    pub pass: usize,
    pub fail: usize,
    pub inconclusive: usize,
    pub error: usize,
    /// Worst-wins verdict over the rollup. Uses the same 4-valued
    /// union as `VerifyResult.verdict` / `VerifyRequirementResult.verdict`
    /// ("Pass", "Fail", "Inconclusive", "Error"). `"Pass"` on an empty
    /// rollup.
    pub overall: String,
}

impl VerifySummary {
    /// Build a summary from a `sysml_runtime::VerdictRollup`.
    pub fn from_rollup(rollup: sysml_runtime::aggregates::VerdictRollup) -> Self {
        Self {
            pass: rollup.pass,
            fail: rollup.fail,
            inconclusive: rollup.inconclusive,
            error: rollup.error,
            overall: format!("{}", rollup.overall()),
        }
    }
}

/// Result of running a verification case.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    /// Overall verdict: "Pass", "Fail", "Inconclusive", or "Error".
    pub verdict: String,
    /// HOW the verdict was computed — `"static"` (constraints evaluated
    /// against current/default values: `sysml.verify`), `"trajectory"`
    /// (against a live session's run state: `sysml.sessions.verify`), or
    /// `"external"` (produced outside the tool, ingested via
    /// `sysml.verify.record_external` — B10). One vocabulary home:
    /// `sysml_store::EvaluationMode` (this string slot is its wire form).
    /// BINDING label (§2.1a ruling (d), 2026-07-17): the modes answer
    /// DIFFERENT questions and a UI must never let one read as another;
    /// render it always, not only on disagreement. `#[serde(default)]`
    /// (empty string) only for wire compat with pre-ruling clients.
    #[serde(default)]
    pub evaluation_mode: String,
    /// Pre-computed per-verdict-kind rollup + overall verdict. Clients
    /// should read this instead of re-aggregating the `requirements`
    /// list.
    pub summary: VerifySummary,
    /// Individual requirement results.
    pub requirements: Vec<VerifyRequirementResult>,
    /// Diagnostics from the verification run.
    pub diagnostics: Vec<String>,
}

/// Result of a single requirement within a verification case.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyRequirementResult {
    /// Requirement identifier.
    pub requirement_id: String,
    /// Requirement text/body when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement_text: Option<String>,
    /// Verdict for this requirement.
    pub verdict: String,
    /// Computed actual value or left-hand side for simple comparisons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<serde_json::Value>,
    /// Expected value or right-hand side for simple comparisons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<serde_json::Value>,
    /// Numeric margin (actual - expected) when both sides are numeric.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    /// Per-constraint structured details.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<serde_json::Value>,
    /// Source element id of the requirement, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// Source element id of the underlying requirement definition, when
    /// distinct from `element_id`. Mirrors `serialize_requirement_result`'s
    /// `requirement_element_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement_element_id: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// Result of running an analysis case.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    /// Case name.
    pub case_name: String,
    /// External solver/tool name (if any).
    pub tool_name: Option<String>,
    /// Input parameters declared in the analysis case.
    pub input_parameters: Vec<AnalysisParameter>,
    /// Solved output values keyed by parameter name.
    pub outputs: HashMap<String, String>,
    /// Whether the solver converged.
    pub converged: bool,
    /// Number of solver iterations (if applicable).
    pub iterations: Option<usize>,
    /// Objective verdict — present ONLY when the case declares a `verify`'d
    /// objective (§7.23.2: the objective's subject binds to the analysis
    /// result). The solver mechanism (`tool_name`/`converged`/`iterations`
    /// above) already discloses that the value was numerically produced, so the
    /// verdict is `evaluation_mode: "static"` — Trajectory is reserved for a
    /// live/archived RuntimeSession, which a one-shot solve does not have
    /// (verification-analysis-model-study.md §3.2, steward ruling). Reuses the
    /// ONE verdict wire shape [`VerifyResult`] (same as `sysml.verify` /
    /// `sysml.sessions.verify`) — no parallel verdict struct. ABSENT (not
    /// null-faked) when the case has no verified objective.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_verdict: Option<VerifyResult>,
}

/// An input/output parameter of an analysis case.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter type (e.g. "Real", "Int").
    pub param_type: String,
    /// Default value (if provided).
    pub default_value: Option<String>,
    /// Direction: "in", "out", or "inout".
    pub direction: String,
}

/// Result of constraint network solving (propagation, rollup, DOF, sweep).
#[derive(Debug, Clone, Serialize)]
pub struct SolveResult {
    /// Values solved by binding propagation.
    pub solved: HashMap<String, String>,
    /// Number of propagation iterations.
    pub iterations: usize,
    /// Variables that remain unsolved.
    pub unsolved: Vec<String>,
    /// Degrees of freedom analysis.
    pub dof: DofInfo,
    /// Rollup results (if a rollup property was requested).
    pub rollups: HashMap<String, String>,
    /// Sensitivity sweep results (if a sweep was requested).
    pub sensitivity: Option<SensitivityInfo>,
}

/// Degrees of freedom summary.
#[derive(Debug, Clone, Serialize)]
pub struct DofInfo {
    pub equations: usize,
    pub variables: usize,
    pub known_count: usize,
    pub free_count: usize,
    pub dof: i64,
    pub status: String,
}

/// Sensitivity sweep summary.
#[derive(Debug, Clone, Serialize)]
pub struct SensitivityInfo {
    pub parameter: String,
    pub steps: usize,
    pub effects: Vec<SensitivityEffect>,
}

/// How a single constraint responds to a parameter sweep.
#[derive(Debug, Clone, Serialize)]
pub struct SensitivityEffect {
    pub constraint_name: String,
    pub flip_value: Option<f64>,
    pub flip_direction: Option<String>,
}

/// Result of a sequence trace simulation.
#[derive(Debug, Clone, Serialize)]
pub struct TraceResult {
    /// Lifelines (participants) in the trace.
    pub lifelines: Vec<TraceLifeline>,
    /// Messages exchanged between lifelines.
    pub messages: Vec<TraceMessage>,
}

/// A lifeline in a sequence trace.
#[derive(Debug, Clone, Serialize)]
pub struct TraceLifeline {
    pub index: usize,
    pub name: String,
    pub kind: String,
}

/// A message in a sequence trace.
#[derive(Debug, Clone, Serialize)]
pub struct TraceMessage {
    pub sequence: u64,
    pub from: String,
    pub to: String,
    pub label: String,
    pub timestamp_ms: f64,
    pub payload: Option<String>,
}

/// Result of port flow inspection.
#[derive(Debug, Clone, Serialize)]
pub struct FlowResult {
    /// Registered ports.
    pub ports: Vec<FlowPortInfo>,
    /// Compiled flows.
    pub flows: Vec<FlowConnectionInfo>,
    /// Delivery results (if injection was performed).
    pub delivery: Vec<FlowDeliveryInfo>,
    /// Port health diagnostics (FL001-FL015).
    pub diagnostics: Vec<FlowDiagnostic>,
}

/// A diagnostic produced by the flow health checker (FL001-FL015).
#[derive(Debug, Clone, Serialize)]
pub struct FlowDiagnostic {
    /// Diagnostic code (e.g. "FL010", "FL014").
    pub code: String,
    /// Severity: "error", "warning", or "info".
    pub severity: String,
    /// Human-readable message.
    pub message: String,
    /// Associated port or flow name (if applicable).
    pub port: Option<String>,
}

/// Information about a registered port.
#[derive(Debug, Clone, Serialize)]
pub struct FlowPortInfo {
    pub key: String,
    pub owner: String,
    pub name: String,
    pub definition: Option<String>,
    pub direction: String,
    pub conjugated: bool,
}

/// Information about a compiled flow connection.
#[derive(Debug, Clone, Serialize)]
pub struct FlowConnectionInfo {
    pub id: String,
    pub source: String,
    pub target: String,
    pub succession: bool,
    pub payload_type: Option<String>,
    /// RSC-3.1 (D-3.0.1): the classified link class — one of `power_bond`,
    /// `signal_link`, `message_channel`, `unknown`. Additive field; `None`
    /// when the link graph could not classify this flow (skipped from the
    /// wire so older fixtures stay byte-identical).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_class: Option<String>,
    /// RSC-3.1: the declared interface element satisfying the topology
    /// (`via_interface`), when known. `None` in 3.1 (records in 3.3 D5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_interface: Option<String>,
}

/// Information about a delivered flow message.
#[derive(Debug, Clone, Serialize)]
pub struct FlowDeliveryInfo {
    pub flow_id: String,
    pub source: String,
    pub target: String,
    pub sequence: u64,
}

// -- Session archive (R4.1) --------------------------------------------------
//
// Archive types are re-exported from `sysml-store` so the frontend has one
// canonical wire format. Service-command *result wrappers* wrap those types
// in a field-named object so we can extend the response envelope later (e.g.
// with pagination cursors) without breaking the stable JSON shape.

pub use sysml_store::{
    ArchiveFilter, ArchivedEvidence, ArchivedSession, ArchivedSessionSummary, ArchivedVerdict,
    EvaluationMode, ExternalEvidence, GoldenMetadata, SessionOrigin,
};

/// Response of `sysml.verify.record_external` (B10 external ingestion —
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordExternalResult {
    /// Server-minted id of the synthetic archive entry
    /// (`SessionOrigin::External`) — the key for `sessions.archive.get`.
    pub session_id: String,
    /// Number of verdicts recorded.
    pub recorded: usize,
    /// The digest the client declared the results were produced against.
    pub declared_digest: String,
    /// The workspace digest at ingestion time (provenance capture).
    pub current_digest: String,
    /// `declared_digest == current_digest` — immediate staleness feedback.
    /// `false` is recorded honestly, never rejected: results produced
    /// against an older model are legitimate evidence; the mismatch IS
    /// the signal.
    pub matches_current_model: bool,
}

/// Response of `sysml.sessions.archive.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveListResult {
    /// Archived-session summaries matching the filter, newest-first.
    pub entries: Vec<ArchivedSessionSummary>,
}

/// Response of `sysml.sessions.archive.get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveGetResult {
    /// Full archive entry, or `null` if not found.
    pub entry: Option<ArchivedSession>,
}

/// Response of `sysml.sessions.archive.mark_golden`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMarkGoldenResult {
    /// True on success. Errors surface as a top-level error on the RPC
    /// envelope rather than `ok = false`, so readers can rely on the
    /// happy path returning `true`.
    pub ok: bool,
}

/// Response of `sysml.sessions.archive.unmark_golden`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveUnmarkGoldenResult {
    /// True on success.
    pub ok: bool,
}

// -- Batch sessions (R5.0) ---------------------------------------------------

pub use crate::batch::{
    BatchCreateResult, BatchResultsResult, BatchSliceResult, BatchStatusResult,
};

// -- Sensitivity workflow (R7.4) ---------------------------------------------

pub use crate::sensitivity::{SensitivityMethod, SensitivityResult};

/// Response of `sysml.sensitivity.analyze`.
///
/// The `method` field echoes the request so the frontend can branch on
/// which index fields to render (μ/σ for Morris, S_i/S_Ti for Sobol)
/// without threading the original request through separately. Per-
/// parameter [`SensitivityResult`] entries come back in the same order
/// as `parameters_of_interest` on the request (which mirrors the
/// parameter order the batch's children were generated with).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitivityAnalyzeResult {
    /// Echo of the requested method so the frontend knows which fields
    /// on each `SensitivityResult` are populated.
    pub method: SensitivityMethod,
    /// One entry per parameter, in the same order as
    /// `parameters_of_interest` on the request.
    pub parameters: Vec<SensitivityResult>,
}

// -- Causation trace (R7.1) --------------------------------------------------

/// Response shape for `sysml.causation.trace`. The `root` event is included
/// as a standalone field (for the UI header) and also appears as the first
/// entry in `chain`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CausationTraceResult {
    /// The resolved root event. `None` when nothing matched the request.
    pub root: Option<sysml_runtime::CausationEvent>,
    /// Backward BFS walk from `root` along `caused_by` edges. Root is at
    /// index 0, upstream causes follow in BFS order. Empty when the root
    /// could not be located.
    pub chain: Vec<sysml_runtime::CausationEvent>,
    /// Maximum depth actually used (echoes the request; defaults applied
    /// server-side if absent). Stored as `u32` to avoid `u8` in the wire
    /// format.
    pub max_depth_used: u32,
}

// ---------------------------------------------------------------------------
// Bucket B / B4 — `sysml.workspace.model_tree`
// ---------------------------------------------------------------------------
//
// Multi-URI peer of `sysml.model.tree`. Returns one entry per loaded user
// URI, with the tree projection already converted to line/character ranges
// (LSP `Position` semantics — 0-indexed, UTF-16 code units). The LSP
// `handle_model_tree` flattens this to the existing flat-array wire shape;
// new callers (MCP/CLI/REST) consume the grouped shape directly.

/// 0-indexed line + UTF-16 character offset (LSP `Position` semantics).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TreeNodePosition {
    pub line: u32,
    pub character: u32,
}

/// Start/end position pair for a node's source span.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TreeNodeRange {
    pub start: TreeNodePosition,
    pub end: TreeNodePosition,
}

/// Per-node projection used by `sysml.workspace.model_tree`. The byte→line/col
/// conversion happens once at the service boundary so transports don't
/// re-walk the file content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TreeNodeWithRange {
    pub id: String,
    pub name: String,
    /// `ElementKind` debug name (e.g. `"PartDefinition"`).
    pub kind: String,
    pub uri: String,
    pub range: TreeNodeRange,
    pub children: Vec<TreeNodeWithRange>,
}

/// One per-URI entry in the workspace model tree.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceModelTreeFile {
    pub uri: String,
    pub nodes: Vec<TreeNodeWithRange>,
}
