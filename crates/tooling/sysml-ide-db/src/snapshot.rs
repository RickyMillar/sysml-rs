//! Typed runtime entry point.
//!
//! `Snapshot` is the typed entry point for compiling runtime IR and starting
//! execution sessions. Per ADR-011, it replaces the `ModelCompiler::from_arc`
//! pattern at every production call site. Today it is a thin facade over
//! `sysml_runtime::compiler::ModelCompiler` — same elaboration cost, same
//! method surface — so the structural migration is decoupled from the
//! tracked-query performance work that lands in subsequent commits
//! (S3.T12 / T13 / T14).
//!
//! Construction takes an `Arc<ModelGraph>` directly so callers can hand in
//! the elaborated workspace graph from `Analysis::elaborate_workspace[_with_library]`
//! or any other source (tests, synthetic graphs).

use std::sync::Arc;

use sysml_core::ModelGraph;
use sysml_runtime::compiler::{CompileError, ModelCompiler, OdeDetection, PreparedSingleOde};
use sysml_runtime::constraints::PrecompiledConstraintSet;
use sysml_runtime::expressions::{EvalContext, RefResolveCache};
use std::sync::Mutex;
use sysml_runtime::flows::PortFlowResources;
use sysml_runtime::orchestrator::Orchestrator;
use sysml_runtime::{actions::ActionGraphIR, StateMachineIR};

/// Typed runtime entry point. See module docs.
///
/// `Snapshot` owns an internal `ModelCompiler` whose elaboration pass runs
/// once at construction time. Calling multiple methods on the same
/// `Snapshot` reuses that elaborated state — never re-elaborates.
pub struct Snapshot {
    compiler: ModelCompiler,
}

impl Snapshot {
    /// Build a snapshot from a workspace graph (already elaborated by salsa,
    /// or pre-elaborated by the caller).
    ///
    /// Today this still triggers `ModelCompiler::from_arc`'s defensive
    /// re-elaborate; S3.T7 lifts that once the sysml-core elaboration
    /// ordering fix lands.
    pub fn new(graph: Arc<ModelGraph>) -> Self {
        Self {
            compiler: ModelCompiler::from_arc(graph),
        }
    }

    /// Attach a source directory for relative-path metadata (e.g.
    /// `@DataSource { file = "data/bh.csv" }`).
    pub fn with_source_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.compiler = self.compiler.with_source_dir(dir);
        self
    }

    /// Thread a salsa-cached physics executor onto the inner compiler (RSC-6.4).
    ///
    /// Workspace callers pass the `Arc<PhysicsExecutor>` from
    /// `SysmlService::workspace_physics_executor` (salsa-memoized via
    /// `workspace_physics_executor_best`); `build_workspace_orchestrator` then
    /// clones it instead of rebuilding the executor from the graph. Pure
    /// pass-through — not threaded for no-physics models (the query returns
    /// `None`, so the caller simply doesn't call this).
    pub fn with_cached_physics_executor(
        mut self,
        executor: Arc<sysml_runtime::physics::PhysicsExecutor>,
    ) -> Self {
        self.compiler = self.compiler.with_cached_physics_executor(executor);
        self
    }

    /// The elaborated graph backing this snapshot.
    pub fn graph(&self) -> &Arc<ModelGraph> {
        self.compiler.graph()
    }

    /// Compile a named state machine.
    pub fn compile_state_machine(&self, name: &str) -> Result<StateMachineIR, CompileError> {
        self.compiler.compile_state_machine(name)
    }

    /// Compile a named action.
    pub fn compile_action(&self, name: &str) -> Result<ActionGraphIR, CompileError> {
        self.compiler.compile_action(name)
    }

    /// Detect ODE metadata in the elaborated graph, if any.
    pub fn detect_ode(&self) -> Option<OdeDetection> {
        self.compiler.detect_ode()
    }

    /// Build a single-state-machine orchestrator with no continuous
    /// dynamics — the mint/bind entry point `sysml.simulate.start` needs for
    /// an SM-only model (ledger L44). See
    /// `ModelCompiler::build_sm_orchestrator` for the full rationale.
    pub fn build_sm_orchestrator(
        &self,
        sm_name: &str,
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        self.compiler.build_sm_orchestrator(sm_name, dt_ms, max_time_ms)
    }

    /// Build a single-state-machine orchestrator with optional dt/max-time.
    pub fn build_orchestrator(
        &self,
        sm_name: &str,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        self.compiler
            .build_orchestrator(sm_name, overrides, dt_ms, max_time_ms)
    }

    /// Build a workspace orchestrator (multi-subsystem — SMs, ODE, discrete).
    ///
    /// Takes the pre-built `EvalContext` seed, (optional) pre-built
    /// `PrecompiledConstraintSet`, and (optional) pre-built
    /// `PortFlowResources` as parameters. Callers choose how to
    /// materialise them:
    ///
    /// - Workspace-graph callers should use
    ///   `SysmlService::eval_context_with_overrides` +
    ///   `SysmlService::workspace_precompiled_constraints` +
    ///   `SysmlService::workspace_port_flow_resources` (all salsa-cached).
    /// - Callers with a non-workspace graph (fresh reparse, synthetic
    ///   test fixture) should call `eval_context_seed::context_from_graph`,
    ///   `sysml_runtime::constraints::extract_and_precompile`, and
    ///   `sysml_runtime::flows::build_port_flow_resources` directly — or
    ///   pass `None` and let the compiler do a fresh in-place build.
    ///
    /// Pass `None` for `precompiled_constraints` to skip continuous
    /// constraint monitoring. Pass `None` for `port_flow` to let the
    /// compiler walk the graph in-place (preserves prior behaviour for
    /// callers that haven't been migrated). Pass `None` for
    /// `gated_expressions` to let the compiler detect computed and
    /// instance-scoped expressions in-place. Pass `None` for
    /// `ref_resolve_cache` to let the orchestrator install its own
    /// fresh empty cache (per-session lifetime); pass `Some` (typically
    /// the salsa-cached Arc from
    /// `SysmlService::workspace_ref_resolve_cache`) to share a
    /// snapshot-scoped cache across orchestrators on the same
    /// elaborated-graph revision (ADR-011 §6, S3.T14). Snapshot is a
    /// pure pass-through — these resources are not Snapshot's contract
    /// per ADR-011 §1.
    #[allow(clippy::too_many_arguments)]
    pub fn build_workspace_orchestrator(
        &self,
        base_ctx: EvalContext,
        precompiled_constraints: Option<Arc<PrecompiledConstraintSet>>,
        port_flow: Option<Arc<PortFlowResources>>,
        gated_expressions: Option<Arc<Vec<sysml_runtime::compiler::GatedExprSpec>>>,
        ref_resolve_cache: Option<Arc<Mutex<RefResolveCache>>>,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        self.compiler.build_workspace_orchestrator(
            base_ctx,
            precompiled_constraints,
            port_flow,
            gated_expressions,
            ref_resolve_cache,
            overrides,
            dt_ms,
            max_time_ms,
        )
    }

    // `build_orchestrator_explicit` was removed: its only caller was the
    // deleted `simulate.continuous.start` command, whose explicit
    // ODE-derivative-string input bypassed the model
    // (execution-entry-unification-plan.md P5). Model-driven ODE construction
    // goes through `build_orchestrator` / `build_workspace_orchestrator`.

    /// Build an orchestrator and run it to completion.
    pub fn run_simulation(
        &self,
        sm_name: &str,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<
        (
            Orchestrator,
            Option<sysml_runtime::orchestrator::ExecutionSnapshot>,
        ),
        CompileError,
    > {
        self.compiler
            .run_simulation(sm_name, overrides, dt_ms, max_time_ms)
    }

    /// Prepare the graph-invariant pieces of a single-SM ODE orchestrator once,
    /// for reuse across many [`run_simulation_prepared`](Self::run_simulation_prepared)
    /// calls that differ only in parameter `overrides` (e.g. an `ode_sweep`).
    /// Pure pass-through to [`ModelCompiler::prepare_single_ode`] (RSC-6.2).
    pub fn prepare_single_ode(&self, sm_name: &str) -> Result<PreparedSingleOde, CompileError> {
        self.compiler.prepare_single_ode(sm_name)
    }

    /// Assemble a single-SM ODE orchestrator from pre-compiled pieces plus the
    /// per-variant `overrides` and run it to completion. The sweep-friendly
    /// counterpart of [`run_simulation`](Self::run_simulation): the graph walk
    /// done by [`prepare_single_ode`](Self::prepare_single_ode) is paid once,
    /// not per variant.
    pub fn run_simulation_prepared(
        &self,
        prepared: &PreparedSingleOde,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<
        (
            Orchestrator,
            Option<sysml_runtime::orchestrator::ExecutionSnapshot>,
        ),
        CompileError,
    > {
        let mut orch =
            self.compiler
                .build_orchestrator_from_prepared(prepared, overrides, dt_ms, max_time_ms)?;
        let snap = orch.run_to_completion();
        Ok((orch, snap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::ModelGraph;

    #[test]
    fn snapshot_constructs_from_empty_graph() {
        let snap = Snapshot::new(Arc::new(ModelGraph::default()));
        assert!(snap.graph().elements.is_empty());
    }

    #[test]
    fn snapshot_with_source_dir_threads_through() {
        let snap = Snapshot::new(Arc::new(ModelGraph::default()))
            .with_source_dir("/tmp/sysml-test");
        assert!(snap.graph().elements.is_empty());
    }
}
