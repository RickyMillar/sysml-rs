//! Unified model compilation pipeline.
//!
//! `ModelCompiler` is the single entry point for transforming a `ModelGraph`
//! into runtime IR. It always elaborates, always builds context the same way,
//! and provides `build_orchestrator()` — the key method that replaces the
//! ~80-line ODE build pipeline previously duplicated across multiple service
//! commands.
//!
//! Lives in `sysml-runtime` (not `sysml-service`) because all compilation and
//! execution logic belongs in runtime. The service layer is a thin dispatch
//! layer that calls into this.

use std::collections::HashSet;
use std::sync::Arc;

use sysml_core::elaborate;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_span::Diagnostic;

use crate::constraints::{
    extract_constraints_filtered, precompile_constraint_set, PrecompiledConstraintSet,
};
use crate::statemachine::StateMachineCompiler;
use crate::StateMachineIR;

mod context;
mod expressions;
mod ode_detection;
mod orchestrator_build;
mod slots;
mod instances;
mod wiring;
mod solver_build;

pub use context::*;
pub use expressions::*;
pub use ode_detection::*;
pub use orchestrator_build::*;
pub(crate) use slots::*;
pub use instances::*;

/// Check whether an element specializes a type with the given name.
///
/// KerML models every specialization edge as a `Specialization` (8.3.3.1):
/// `Subclassification` relates two Classifiers, `Subsetting` two Features, and
/// `FeatureTyping` a Feature to its type. All three answer the same question,
/// and the parser emits a DIFFERENT one depending on whether the declaration is
/// a DEFINITION or a USAGE:
///
/// ```text
/// action def SpringDynamics :> ContinuousStateSpaceDynamics   → Subclassification
/// action     dynamics       :> StateSpaceDynamics             → Subsetting
/// calc   :>> getNextState   :  GetNextState                   → FeatureTyping
/// ```
///
/// Only the definition spellings were checked, so a usage-form dynamics action
/// specialized nothing as far as every SSR/ODE detector was concerned. On
/// `examples/damped-oscillator` — the only fixture written in the usage form —
/// that meant zero detected subsystems and a workspace that compiled to
/// "no state machines, ODE, discrete, or action subsystems found".
///
/// Checks, in order:
/// 1. `unresolvedTypeName` property on the element itself (pre-resolution).
/// 2. Owned `Subclassification` children with an `unresolved_superclassifier`
///    prop (parser emits these for a DEFINITION's `:>`).
/// 3. Owned `Subsetting`-family / `FeatureTyping` children — the USAGE
///    spellings of the same two edges.
/// 4. Outgoing `Specialize` relationships (post-resolution).
///
/// `Redefinition` (`:>>`) is deliberately NOT read here. It is a kind of
/// Subsetting in KerML, but its `unresolved_redefinedFeature` names the
/// INHERITED FEATURE being redefined, not a type — `calc :>> getNextState`
/// redefines a feature called `getNextState`, and it is the `: GetNextState`
/// typing on the same declaration that answers this question.
///
/// Exposed so tree projection layers (`sysml-service::query`) can stamp
/// authoritative classification flags like `is_ode` on `TreeNode` without
/// duplicating the subsetting-chain logic.
pub fn specializes_name(graph: &ModelGraph, element_id: &ElementId, target_name: &str) -> bool {
    // Strategy 1: Check unresolvedTypeName property (quick, pre-resolution)
    if let Some(element) = graph.get_element(element_id) {
        if let Some(Value::String(s)) = element.get_prop("unresolvedTypeName") {
            if s == target_name || s.ends_with(&format!("::{}", target_name)) {
                return true;
            }
        }
    }

    // Strategy 2: Walk owned Subclassification elements (parser creates these for :>)
    for child in graph.children_of(element_id) {
        if child.kind == ElementKind::Subclassification {
            if let Some(Value::String(s)) = child.get_prop("unresolved_superclassifier") {
                if s == target_name || s.ends_with(&format!("::{}", target_name)) {
                    return true;
                }
            }
        }
    }

    // Strategy 3: the USAGE spellings of the same two edges. A usage's `:>`
    // is an owned `Subsetting` (or its reference/cross refinements) and its
    // `: Type` an owned `FeatureTyping`; both carry the unresolved name under
    // their own prop key. Matching is identical to strategy 2 — exact, or a
    // qualified name whose terminal segment matches.
    for child in graph.children_of(element_id) {
        let unresolved = match child.kind {
            ElementKind::Subsetting
            | ElementKind::ReferenceSubsetting
            | ElementKind::CrossSubsetting => child.get_prop("unresolved_subsettedFeature"),
            ElementKind::FeatureTyping => child.get_prop("unresolved_type"),
            _ => continue,
        };
        if let Some(Value::String(s)) = unresolved {
            if s == target_name || s.ends_with(&format!("::{}", target_name)) {
                return true;
            }
        }
    }

    // Strategy 4: Walk outgoing Specialization relationships (post-resolution)
    for rel in graph.outgoing(element_id) {
        if rel.kind != RelationshipKind::Specialize {
            continue;
        }
        if let Some(supertype) = graph.get_element(&rel.target) {
            if supertype.name.as_deref() == Some(target_name) {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Gated-expression extraction (S3.T12 cached_gated_expressions, ADR-011 §3
// RT-16).
//
// Combines direct `= expr` attribute bindings (`detect_computed_expressions`,
// see `ModelCompiler`) with instance-multiplied attribute bindings (this
// module's `extract_instance_scoped_pairs`). Both produce `(target_var, expr_str)`
// pairs that the orchestrator-build path parses to `ExprIR` and feeds into
// `Orchestrator::add_computed_expression`.
//
// The cached upstream
// (`sysml_ide_db::gated_expressions::workspace_gated_expressions_with_library`
// and siblings) calls `build_gated_expressions` once per elaborated-graph
// revision and stores the parsed `Vec<(String, ExprIR)>` so subsequent
// `build_workspace_orchestrator` calls skip both walks plus the per-expr
// parse.
//
// The instance-scoped walk here is intentionally a *strict superset* of the
// in-place path at `build_workspace_orchestrator`: that path filters
// instances by reachable SMs / ODEs via `expand_part_instances`, which in
// turn calls `detect_all_odes_unified` (heavy walk over the elaborated
// graph). The cached walk skips that filter and emits prefixed expressions
// for every multiplied PartUsage of an attribute-bearing PartDefinition.
// Extra entries are inert — `add_computed_expression` is a HashMap-like
// `insert` keyed on `target_variable`, and unreferenced targets simply
// never fire.
// ---------------------------------------------------------------------------

/// Compilation error — wraps diagnostics from the various compilers.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct CompileError {
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl CompileError {
    pub fn from_diagnostics(diags: Vec<Diagnostic>) -> Self {
        Self {
            message: diags
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; "),
            diagnostics: diags,
        }
    }

    pub fn from_message(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            diagnostics: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ODE detection result
// ---------------------------------------------------------------------------

/// Unified model compilation pipeline.
///
/// Wraps a `ModelGraph`, eagerly elaborates it, and exposes compilation
/// helpers that eliminate duplicated build pipelines.
pub struct ModelCompiler {
    /// Elaborated graph (always elaborated in constructor).
    ///
    /// Metadata-driven detectors (e.g. `detect_solver_selections_from_metadata`)
    /// walk `MetadataUsage` owner chains against this same graph. The
    /// pre-elaboration snapshot field that used to live here was retired in
    /// S3.T7 once RW-2 (now `sysml-runtime/tests/elaborate_metadata_invariant.rs`) proved
    /// that none of the nine `sysml_core::elaborate::elaborate` passes
    /// disrupt the metadata owner-chain walk.
    graph: Arc<ModelGraph>,
    /// Optional source directory for resolving relative file paths
    /// (e.g., `@DataSource { file = "data/bh.csv" }`).
    source_dir: Option<std::path::PathBuf>,
    /// RSC-6.4: optional pre-built physics executor, supplied by the
    /// salsa-aware caller (`workspace_physics_executor` query →
    /// `Snapshot`/service). When `Some`, `build_workspace_orchestrator` clones
    /// it (`clone_concrete`) instead of reconstructing it from the graph — the
    /// physics executor is a complete graph-derived compiled subsystem, so it
    /// belongs on salsa like the other compile seeds. When `None` (every
    /// test/bench/raw caller, and any no-physics model), the orchestrator
    /// builds it inline exactly as before — byte-identical.
    cached_physics_executor: Option<Arc<crate::physics::executor::PhysicsExecutor>>,
}

impl ModelCompiler {
    /// Create a new compiler from an owned `ModelGraph`.
    pub fn new(graph: ModelGraph) -> Self {
        let mut elaborated = graph;
        elaborate::elaborate(&mut elaborated);
        Self {
            graph: Arc::new(elaborated),
            source_dir: None,
            cached_physics_executor: None,
        }
    }

    /// Set the source directory for resolving relative file paths in metadata
    /// (e.g., `@DataSource { file = "data/bh.csv" }`).
    pub fn with_source_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.source_dir = Some(dir.into());
        self
    }

    /// RSC-6.4: supply a pre-built, salsa-memoized physics executor.
    ///
    /// The salsa-aware caller (service / `Snapshot`) computes
    /// `workspace_physics_executor` once and threads the resulting
    /// `Arc<PhysicsExecutor>` here; `build_workspace_orchestrator` then clones
    /// it per build instead of reconstructing it from the graph. Pass only a
    /// POSITIVE executor (the query returns `None` for no-physics models, in
    /// which case this stays unset and the orchestrator builds inline — which
    /// is also where the no-physics fast-fail lives). Byte-identical: a cloned
    /// executor is field-equal to a freshly built one.
    pub fn with_cached_physics_executor(
        mut self,
        executor: Arc<crate::physics::executor::PhysicsExecutor>,
    ) -> Self {
        self.cached_physics_executor = Some(executor);
        self
    }

    /// Create a compiler from a pre-loaded `Arc<ModelGraph>`.
    ///
    /// If the graph is already elaborated (the salsa workspace graph, or any
    /// caller that pre-elaborated it — see [`ModelGraph::is_elaborated`]), it is
    /// trusted as-is: no clone, no re-elaborate. This is the production fast
    /// path (RSC-6.1, ADR-011 S3.T7); every session start / batch child / sweep
    /// iteration used to pay a redundant full re-elaboration here.
    ///
    /// A raw (un-elaborated) graph — passed by tests, benches, and synthetic
    /// callers — is cloned and elaborated, exactly as before. Elaboration is
    /// whole-graph idempotent (gated by sysml-core's `elaborate_idempotency`
    /// test), so trusting the marker is byte-identical to always re-elaborating.
    pub fn from_arc(graph: Arc<ModelGraph>) -> Self {
        if graph.is_elaborated() {
            return Self {
                graph,
                source_dir: None,
                cached_physics_executor: None,
            };
        }
        let mut elaborated = (*graph).clone();
        elaborate::elaborate(&mut elaborated);
        debug_assert!(
            elaborated.is_elaborated(),
            "elaborate() must set the is_elaborated marker; the from_arc skip \
             path depends on this invariant holding"
        );
        Self {
            graph: Arc::new(elaborated),
            source_dir: None,
            cached_physics_executor: None,
        }
    }

    /// Returns the elaborated graph.
    pub fn graph(&self) -> &Arc<ModelGraph> {
        &self.graph
    }

    // -- State machine compilation ------------------------------------------

    /// Compile a named state machine from the elaborated graph.
    pub fn compile_state_machine(&self, name: &str) -> Result<StateMachineIR, CompileError> {
        StateMachineCompiler::compile_named(&self.graph, name)
            .map_err(CompileError::from_diagnostics)
    }

    // -- Action compilation --------------------------------------------------

    /// Compile a named action from the elaborated graph.
    pub fn compile_action(
        &self,
        name: &str,
    ) -> Result<crate::actions::ActionGraphIR, CompileError> {
        crate::actions::compile_action(name, &self.graph).map_err(CompileError::from_diagnostics)
    }

    // -- ODE detection ------------------------------------------------------

    /// Thin wrapper over the free `specializes_name` function, bound to
    /// this compiler's graph. See the free function for the detection
    /// strategies and rationale.
    fn specializes_name(&self, element_id: &ElementId, target_name: &str) -> bool {
        specializes_name(&self.graph, element_id, target_name)
    }

    // -- Constraint compilation ---------------------------------------------

    /// Extract and pre-compile all constraints from the elaborated graph.
    pub fn compile_constraints(&self) -> PrecompiledConstraintSet {
        let set = extract_constraints_filtered(&self.graph, |_| true);
        precompile_constraint_set(&set)
    }

    // -- Orchestrator build (THE key dedup target) --------------------------

    /// Given the `PartDefinition` the instance is typed by (e.g.
    /// `CircuitPath`) and the ODE's owning element (e.g. the
    /// `ThermalProtectionModel` PartDefinition that declares the
    /// `calc def :> GetDerivative`), return the chain of `PartUsage`
    /// names inside the instance's type hierarchy that leads to the
    /// ODE. Example: `CircuitPath` contains `part thermalModel :
    /// ThermalProtectionModel;` → returns `["thermalModel"]`. Deeper nesting
    /// returns multiple segments.
    ///
    /// When the ODE owner IS the instance type (attribute lives
    /// directly on the instance) returns `[]`. When no usage chain
    /// resolves, returns `[]` — canonical prefix then falls back to
    /// `{container}.{instance}`, which is still better than the bare
    /// `{instance}.{var}` runtime key for matching the tree's
    /// `ownerPath`.
    ///
    /// Uses `sysml_core::find_feature_type` (O(1) typed-def index,
    /// the same resolver the frontend tree uses) so the search
    /// traverses exactly the usage → definition chain the UI sees.
    /// RSC-2.1: this is the former `in_type_path`, now carrying the usage
    /// ElementId of every chain segment alongside its name — the sub-part
    /// portion of a `RuntimeId::instance_path`. Same BFS, same result set.
    fn in_type_path_with_ids(
        &self,
        instance_type_id: &ElementId,
        ode_owner_id: &ElementId,
    ) -> Vec<(String, ElementId)> {
        if instance_type_id == ode_owner_id {
            return Vec::new();
        }
        let mut queue: std::collections::VecDeque<(ElementId, Vec<(String, ElementId)>)> =
            std::collections::VecDeque::new();
        queue.push_back((instance_type_id.clone(), Vec::new()));
        let mut visited: HashSet<ElementId> = HashSet::new();
        while let Some((current_type, path)) = queue.pop_front() {
            if !visited.insert(current_type.clone()) {
                continue;
            }
            for child in self.graph.children_of(&current_type) {
                if !matches!(child.kind, ElementKind::PartUsage | ElementKind::ItemUsage) {
                    continue;
                }
                let Some(child_name) = child.name.as_ref() else {
                    continue;
                };
                let Some(child_type_id) =
                    sysml_core::resolution::scoping::chaining::find_feature_type(
                        &self.graph,
                        &child.id,
                    )
                else {
                    continue;
                };
                let mut next_path = path.clone();
                next_path.push((child_name.clone(), child.id.clone()));
                if child_type_id == *ode_owner_id {
                    return next_path;
                }
                queue.push_back((child_type_id, next_path));
            }
        }
        Vec::new()
    }

    /// BFS the containment subtree under `owner` for the first
    /// non-expression-AST element named `name` (direct children are
    /// visited before deeper levels).
    fn find_feature_decl(&self, owner: &ElementId, name: &str) -> Option<ElementId> {
        let mut queue: std::collections::VecDeque<ElementId> = std::collections::VecDeque::new();
        queue.push_back(owner.clone());
        let mut visited: HashSet<ElementId> = HashSet::new();
        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            for child in self.graph.children_of(&current) {
                if is_expression_ast_kind(&child.kind) {
                    continue;
                }
                if child.name.as_deref() == Some(name) {
                    return Some(child.id.clone());
                }
                queue.push_back(child.id.clone());
            }
        }
        None
    }

    /// The unique `AttributeUsage` named `name`, or `None` when absent or
    /// ambiguous (ambiguous names stay out of the slot table rather than
    /// risking a wrong declaration id).
    fn find_unique_named_attribute(&self, name: &str) -> Option<ElementId> {
        let mut candidates =
            self.graph.elements.values().filter(|e| {
                e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some(name)
            });
        let first = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        Some(first.id.clone())
    }

    /// Resolve a part usage's declared type name: `unresolvedTypeName`
    /// prop → `unresolved_type` prop → owned `FeatureTyping` child's
    /// `unresolved_type` (the same three-step lookup legacy instance
    /// discovery used).
    fn resolve_usage_type_name(
        graph: &ModelGraph,
        element: &sysml_core::Element,
    ) -> Option<String> {
        element
            .get_prop("unresolvedTypeName")
            .or_else(|| element.get_prop("unresolved_type"))
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .or_else(|| {
                graph
                    .children_of(&element.id)
                    .find(|ft| ft.kind == ElementKind::FeatureTyping)
                    .and_then(|ft| {
                        ft.get_prop("unresolved_type")
                            .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    })
            })
    }

    /// GAP 2 (L23 sub-gap B): derive the owner-instance key for a NON-multiplied
    /// state machine compiled from the `state def` named `sm_name` — the
    /// part-usage instance whose type owns that `state def`. That instance is
    /// the receiver Occurrence a transfer to its port addresses
    /// (`TransitionPerformances.kerml:43-46`, `Transfers.kerml:254-265`), while
    /// the subsystem is named after the `state def`. Returns `None` for a
    /// package-level `state def` with no owning part usage — the caller falls
    /// back to the SM name itself (documented spec-silent identity). Multiplied
    /// SMs do NOT use this: their owner key is the instance prefix.
    fn part_usage_owner_of_state_def(&self, sm_name: &str) -> Option<String> {
        // The StateDefinition element this SM compiled from. A `state def` is
        // always exhibited (`exhibit state x : Def`, itself a
        // PerformActionUsage), so a part usage to reconcile to always exists —
        // keying SM registration on the definition is a safe shortcut. Actions
        // have no `exhibit` equivalent (a bare `action def` is pure vocabulary),
        // so they anchor on the root usage instead and call the shared walk
        // below directly. See `part_usage_owner_of_behavior_def`.
        let state_def = self.graph.elements.values().find(|e| {
            matches!(e.kind, ElementKind::StateDefinition) && e.name.as_deref() == Some(sm_name)
        })?;
        self.part_usage_owner_of_behavior_def(state_def)
    }

    /// Reconcile a root-behavior element (a `state def`, or a root action
    /// usage) to the part-usage instance that performs it. Walks `owner` up to
    /// the nearest enclosing `PartDefinition`, then returns the name of the
    /// `PartUsage` typed by that definition — the receiver Occurrence a
    /// transfer to its port addresses (`TransitionPerformances.kerml:43-46`,
    /// `Transfers.kerml:254-265`). The subsystem is named after the behavior;
    /// this gives the orchestrator the instance key a routed `accept … via
    /// <port>` message is addressed to. Returns `None` for a package-level
    /// behavior with no owning part usage — callers fall back to the behavior
    /// name itself (documented spec-silent identity). Multiplied instances do
    /// NOT use this: their owner key is the instance prefix.
    fn part_usage_owner_of_behavior_def(&self, behavior: &Element) -> Option<String> {
        let mut cur = behavior.owner.clone();
        let owning_def_name = loop {
            let id = cur?;
            let elem = self.graph.get_element(&id)?;
            if matches!(elem.kind, ElementKind::PartDefinition) {
                break elem.name.clone()?;
            }
            cur = elem.owner.clone();
        };
        // A PartUsage typed by that definition. Non-multiplied ⇒ at most one
        // such usage per container; the first match is the receiver instance.
        let usage = self.graph.elements.values().find(|e| {
            matches!(e.kind, ElementKind::PartUsage)
                && Self::resolve_usage_type_name(&self.graph, e).as_deref()
                    == Some(owning_def_name.as_str())
        })?;
        usage.name.clone()
    }

    /// True when `action` is a ROOT action performance — a usage owned
    /// directly by a `PartDefinition` (the part's behaviour) or by a package
    /// namespace (a top-level action) — and NOT a subperformance. Walking
    /// owners outward, the first enclosing element decides: a `PartDefinition`
    /// qualifies; any other Definition/Usage (another action = subperformance,
    /// a `state def`/state = SM-owned accept, a verification/calc/constraint
    /// case = that case's behaviour) disqualifies; running out of owners means
    /// package/namespace-level and qualifies. Transparent namespace containers
    /// (`Package`/`Namespace`) are walked through. Spec basis: only a Usage is
    /// a root Performance (Performances.kerml:63/190, Actions.sysml:180); a
    /// subperformance is `owner`ed by another performance.
    fn is_root_part_or_package_action(&self, action: &Element) -> bool {
        let mut cur = action.owner.clone();
        loop {
            let Some(id) = cur else {
                return true; // no enclosing performance container → package root
            };
            let Some(elem) = self.graph.get_element(&id) else {
                return true;
            };
            if matches!(elem.kind, ElementKind::PartDefinition) {
                return true; // owned by a part → the part's root action
            }
            if elem.kind.is_definition() || elem.kind.is_usage() {
                return false; // owned by another feature (action/state/case/…)
            }
            cur = elem.owner.clone();
        }
    }

    /// True if `action` is a state-space DYNAMICS action — it specializes or
    /// subsets any SSR base type (`StateSpaceDynamics` and its Continuous /
    /// Discrete refinements). Such actions are continuous/discrete-dynamics
    /// subsystems (the ODE/discrete lane), never plain action graphs.
    ///
    /// This used to carry its own owned-`Subsetting` walk because
    /// `specializes_name` saw only the definition spellings. That walk now
    /// lives in `specializes_name` itself, where every SSR detector reads it —
    /// having the exclusion side know about usage-form `:>` while the
    /// DETECTION side did not is precisely what left `damped-oscillator`
    /// excluded from the action lane and invisible to the SSR lane, i.e. with
    /// no subsystems at all.
    fn is_dynamics_action(&self, action: &Element) -> bool {
        const SSR_BASES: [&str; 3] = [
            "StateSpaceDynamics",
            "ContinuousStateSpaceDynamics",
            "DiscreteStateSpaceDynamics",
        ];
        SSR_BASES
            .iter()
            .any(|base| self.specializes_name(&action.id, base))
    }

}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
