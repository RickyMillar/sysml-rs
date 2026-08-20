//! # Hybrid Automaton Executors
//!
//! Mode-switched ODE dynamics driven by a state machine, split (RSC-4.3,
//! ledger L41) into two `Executor` subsystems: [`ContinuousDynamicsExecutor`]
//! (the ODE half — mode-dependent derivatives, entry resets) and
//! [`ModeSelectionExecutor`] (the SM half — mode selection). They are
//! wired by the orchestrator like any other executor pair and communicate
//! through the shared context the orchestrator threads between executors
//! each tick, never through a private Rust field reference between them.
//!
//! ## Spec Alignment
//!
//! SysML v2 `StateSpaceRepresentation.sysml` defines
//! `ContinuousStateSpaceDynamics` with mode-dependent derivatives and
//! `ZeroCrossingEventDef` for hybrid transitions. These executors
//! implement the runtime semantics.
//!
//! ## History
//!
//! This crate originally had a single fused `HybridExecutor` combining both
//! roles (SM embedded as a private field, ODE step handled inline within its
//! own `tick()`). It had zero production callers — no `ModelCompiler` entry
//! point ever constructed one — and its ODE half evaluated raw `OdeRhs`
//! closures with no retained `OdeSpec`, so `read_slots()` was empty by
//! construction (ledger L41): a hybrid whose derivative read a peer's slot
//! produced no scheduler edge, and RSC-4.2 ruling 6 forbade papering over
//! that with a partially-harvested read-set. The split below is the fix —
//! deleted in favor of the two-executor design, not demoted (CLAUDE.md #5).
//! One real behavior difference from the fused type: it applied a mode
//! transition's entry reset synchronously within its own `tick()`, visible
//! to that same tick's ODE step. Split into two ordinarily-phased
//! subsystems (`ContinuousDynamics` before `StateMachine`), a reset now
//! lands on the ODE half one tick later — the same lag every other
//! SM-effect-drives-ODE-parameter coupling in this crate has outside
//! RSC-4.3 Wave-1's mid-tick re-step (reserved for the located `accept when`
//! crossing path; building a second copy of it for this zero-caller type
//! would have been a disproportionate-budget spend).

use std::collections::HashMap;
use std::sync::Arc;

use sysml_core::Value;

use crate::expressions::EvalContext;
use crate::ode::OdeRhs;
use crate::orchestrator::{ExecutionPhase, Executor, TickContext, TickOutput};
use crate::statemachine::StateMachineRunner;

/// Configuration for a single mode in a hybrid automaton.
#[derive(Clone)]
pub struct HybridMode {
    /// ODE right-hand side for this mode.
    pub rhs: OdeRhs,
    /// Optional state resets applied on entry to this mode.
    /// Maps state variable index → new value.
    pub entry_resets: HashMap<usize, ResetAction>,
}

/// What happens to a state variable on mode entry.
#[derive(Debug, Clone)]
pub enum ResetAction {
    /// Set to a fixed value.
    Set(f64),
    /// Multiply current value by a factor (e.g., coefficient of restitution).
    Scale(f64),
    /// Negate the current value.
    Negate,
}

/// The ODE half of a split hybrid automaton (RSC-4.3, ledger L41).
///
/// Mode-dependent derivatives are still `OdeRhs` closures (unchanged
/// `HybridMode` shape), but each mode may additionally carry a retained
/// [`OdeSpec`](crate::ode_builder::OdeSpec) purely so [`read_slots`] can
/// report the derivative/signal expression's real slot reads — mirroring
/// the `spec: Option<OdeSpec>` pattern `Rk4Solver`/`Rk45Solver` already use
/// (RSC-2.3). A mode with no attached spec contributes nothing to the
/// read-set (honestly empty, never a partial harvest — RSC-4.2 ruling 6).
pub struct ContinuousDynamicsExecutor {
    /// Context key read each tick to learn the active mode, written by the
    /// paired [`ModeSelectionExecutor`] via `sync_context_out_slots`.
    mode_signal: String,
    /// Mode name -> derivative set + entry resets.
    modes: HashMap<String, HybridMode>,
    /// RSC-4.3 (L41): retained `OdeSpec` per mode, read-set-only (see module
    /// doc above). Keyed the same as `modes`; a mode may be present in one
    /// map and absent from the other.
    mode_specs: HashMap<String, crate::ode_builder::OdeSpec>,
    /// Fallback RHS when the current mode has no registered derivatives.
    default_rhs: OdeRhs,
    /// Continuous state variable names.
    state_vars: Vec<String>,
    /// Current continuous state values.
    state: Vec<f64>,
    /// Previous mode (for detecting transitions / entry resets).
    prev_mode: String,
    /// Internal eval context for ODE evaluation.
    eval_ctx: EvalContext,
    /// Precomputed slot route per continuous state variable, parallel to
    /// [`state_vars`](Self::state_vars). `None` for hand-built executors
    /// (tests) with no compile-minted slot table.
    state_routes: Option<Vec<crate::slots::WriteRoute>>,
}

impl ContinuousDynamicsExecutor {
    /// Create a new continuous-dynamics half.
    ///
    /// - `mode_signal` — context key this executor reads to learn the
    ///   active mode (must match the paired `ModeSelectionExecutor`'s key).
    /// - `initial_mode` — mode assumed before the first context sync.
    /// - `state_vars` — names of the continuous state variables.
    /// - `initial_state` — initial values for `state_vars` (same order).
    /// - `default_rhs` — fallback derivative RHS for a mode with none registered.
    pub fn new(
        mode_signal: impl Into<String>,
        initial_mode: impl Into<String>,
        state_vars: Vec<String>,
        initial_state: Vec<f64>,
        default_rhs: OdeRhs,
    ) -> Self {
        Self {
            mode_signal: mode_signal.into(),
            modes: HashMap::new(),
            mode_specs: HashMap::new(),
            default_rhs,
            state_vars,
            state: initial_state,
            prev_mode: initial_mode.into(),
            eval_ctx: EvalContext::new(),
            state_routes: None,
        }
    }

    /// Register a mode with its derivative RHS and optional entry resets.
    pub fn add_mode(&mut self, mode_name: impl Into<String>, mode: HybridMode) {
        self.modes.insert(mode_name.into(), mode);
    }

    /// RSC-4.3 (L41): attach a retained `OdeSpec` to a mode so
    /// [`read_slots`] can report its real reads. Read-set-only — does not
    /// change which `OdeRhs` closure `tick()` evaluates for this mode.
    pub fn with_mode_spec(
        mut self,
        mode_name: impl Into<String>,
        spec: crate::ode_builder::OdeSpec,
    ) -> Self {
        self.mode_specs.insert(mode_name.into(), spec);
        self
    }

    fn apply_resets(&mut self, mode_name: &str) {
        if let Some(mode) = self.modes.get(mode_name) {
            for (&idx, action) in &mode.entry_resets {
                if idx < self.state.len() {
                    match action {
                        ResetAction::Set(v) => self.state[idx] = *v,
                        ResetAction::Scale(f) => self.state[idx] *= f,
                        ResetAction::Negate => self.state[idx] = -self.state[idx],
                    }
                }
            }
        }
    }
}

impl Executor for ContinuousDynamicsExecutor {
    fn phase(&self) -> ExecutionPhase {
        ExecutionPhase::ContinuousDynamics
    }

    fn kind_label(&self) -> &'static str {
        "continuousDynamicsHybrid"
    }

    fn tick(&mut self, ctx: &TickContext<'_>) -> TickOutput {
        // Learn the active mode from the shared context (published by the
        // paired ModeSelectionExecutor last tick); fall back to the
        // previous mode when unset (first tick, or a hand-built executor
        // with no paired SM at all).
        let current_mode = match ctx.context.get(&self.mode_signal) {
            Some(Value::String(s)) => s.clone(),
            _ => self.prev_mode.clone(),
        };

        if current_mode != self.prev_mode {
            self.apply_resets(&current_mode);
            self.prev_mode = current_mode.clone();
        }

        let rhs = self
            .modes
            .get(&current_mode)
            .map(|m| Arc::clone(&m.rhs))
            .unwrap_or_else(|| Arc::clone(&self.default_rhs));

        let dt = ctx.dt;
        let t = ctx.t;

        self.eval_ctx.merge_from(ctx.context);
        for (i, name) in self.state_vars.iter().enumerate() {
            self.eval_ctx.set(name.clone(), Value::Float(self.state[i]));
        }

        // RK4 step (clone state to avoid borrow issues).
        let y0 = self.state.clone();
        let n = y0.len();
        let k1 = rhs(t, &y0, &self.eval_ctx);
        let y_mid: Vec<f64> = (0..n).map(|i| y0[i] + 0.5 * dt * k1[i]).collect();
        let k2 = rhs(t + 0.5 * dt, &y_mid, &self.eval_ctx);
        let y_mid2: Vec<f64> = (0..n).map(|i| y0[i] + 0.5 * dt * k2[i]).collect();
        let k3 = rhs(t + 0.5 * dt, &y_mid2, &self.eval_ctx);
        let y_end: Vec<f64> = (0..n).map(|i| y0[i] + dt * k3[i]).collect();
        let k4 = rhs(t + dt, &y_end, &self.eval_ctx);

        for i in 0..n {
            self.state[i] = y0[i] + dt / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
        }

        let trace_outputs: Vec<String> = self
            .state_vars
            .iter()
            .zip(self.state.iter())
            .map(|(name, val)| format!("{name}={val:.6}"))
            .collect();

        TickOutput::solver(current_mode, trace_outputs)
    }

    fn reset_executor(&mut self) {
        // Note: state is not reset — caller should set initial_state.
    }

    fn is_completed(&self) -> bool {
        false
    }

    fn clone_boxed(&self) -> Box<dyn Executor> {
        Box::new(ContinuousDynamicsExecutor {
            mode_signal: self.mode_signal.clone(),
            modes: self.modes.clone(),
            mode_specs: self.mode_specs.clone(),
            default_rhs: Arc::clone(&self.default_rhs),
            state_vars: self.state_vars.clone(),
            state: self.state.clone(),
            prev_mode: self.prev_mode.clone(),
            eval_ctx: self.eval_ctx.alias_live(),
            state_routes: self.state_routes.clone(),
        })
    }

    fn sync_context_in(&mut self, shared: &EvalContext) {
        self.eval_ctx.merge_from(shared);
    }

    /// Slot-routed continuous-state writeback (the ODE half of the old
    /// fused `sync_context_out_slots` — the SM half now lives on
    /// [`ModeSelectionExecutor`], routed independently). Returns `true`
    /// only when every state var has a prepared route.
    fn sync_context_out_slots(
        &self,
        shared: &mut EvalContext,
        _mode: crate::ode::SignalEvalMode,
    ) -> bool {
        let Some(routes) = &self.state_routes else {
            return false;
        };
        for (route, &value) in routes.iter().zip(self.state.iter()) {
            route.apply(shared, Value::Float(value));
        }
        true
    }

    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        self.state_routes = Some(
            self.state_vars
                .iter()
                .map(|name| {
                    crate::slots::WriteRoute::resolve(
                        store,
                        var_prefix,
                        canonical_prefix,
                        writer,
                        name,
                    )
                })
                .collect(),
        );
    }

    fn unrouted_slot_writes(&self) -> Vec<String> {
        self.state_routes
            .as_ref()
            .map(|routes| {
                routes
                    .iter()
                    .filter(|r| !r.is_routed())
                    .map(|r| r.runtime_key().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn get_state_snapshot(&self) -> Option<Vec<f64>> {
        Some(self.state.clone())
    }

    /// RSC-4.3 (L41): read-set = union of every registered mode's retained
    /// `OdeSpec` reads. Real (compiler-resolved slot reads), not a partial
    /// harvest from the opaque `OdeRhs` closures — closes L41 for this
    /// (still zero-caller) type.
    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        let mut v: Vec<crate::slots::SlotId> = self
            .mode_specs
            .values()
            .flat_map(|spec| spec.slot_reads())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn current_state_name(&self) -> &str {
        &self.prev_mode
    }
}

/// The state-machine half of a split hybrid automaton (RSC-4.3, ledger
/// L41). A thin `Executor` delegate over [`StateMachineRunner`] that
/// additionally publishes its current state name into the shared context
/// under `mode_signal` each tick, so a paired [`ContinuousDynamicsExecutor`]
/// can read it — the slot-store-mediated connection the fused
/// `HybridExecutor` used to make through a private Rust field instead.
pub struct ModeSelectionExecutor {
    sm: StateMachineRunner,
    mode_signal: String,
}

impl ModeSelectionExecutor {
    pub fn new(sm: StateMachineRunner, mode_signal: impl Into<String>) -> Self {
        Self {
            sm,
            mode_signal: mode_signal.into(),
        }
    }
}

impl Executor for ModeSelectionExecutor {
    fn phase(&self) -> ExecutionPhase {
        ExecutionPhase::StateMachine
    }

    fn kind_label(&self) -> &'static str {
        "modeSelectionHybrid"
    }

    fn tick(&mut self, ctx: &TickContext<'_>) -> TickOutput {
        self.sm.sync_context_in(ctx.context);
        Executor::tick(&mut self.sm, ctx)
    }

    fn reset_executor(&mut self) {
        Executor::reset_executor(&mut self.sm)
    }

    fn is_completed(&self) -> bool {
        Executor::is_completed(&self.sm)
    }

    fn clone_boxed(&self) -> Box<dyn Executor> {
        Box::new(ModeSelectionExecutor {
            sm: self.sm.clone(),
            mode_signal: self.mode_signal.clone(),
        })
    }

    fn rewrite_when_to_located(&mut self, index: usize, event_name: &str) -> bool {
        Executor::rewrite_when_to_located(&mut self.sm, index, event_name)
    }

    fn sync_context_in(&mut self, shared: &EvalContext) {
        Executor::sync_context_in(&mut self.sm, shared)
    }

    /// Delegates to the inner SM's own slot-routed writeback (its compiled
    /// assignment/payload write-set — unchanged), then ALWAYS additionally
    /// publishes the current state name under `mode_signal` — this publish
    /// does not depend on slot-mint status, it is plain context passthrough,
    /// same as a hand-built executor's legacy writeback used to be.
    fn sync_context_out_slots(
        &self,
        shared: &mut EvalContext,
        mode: crate::ode::SignalEvalMode,
    ) -> bool {
        let routed = Executor::sync_context_out_slots(&self.sm, shared, mode);
        shared.set(
            self.mode_signal.clone(),
            Value::String(Executor::current_state_name(&self.sm).to_owned()),
        );
        routed
    }

    fn slot_write_fallbacks(&self) -> Vec<String> {
        Executor::slot_write_fallbacks(&self.sm)
    }

    fn unrouted_slot_writes(&self) -> Vec<String> {
        Executor::unrouted_slot_writes(&self.sm)
    }

    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        Executor::prepare_slot_writeback(&mut self.sm, store, var_prefix, canonical_prefix, writer)
    }

    fn scoped_view_bypass(&self) -> bool {
        Executor::scoped_view_bypass(&self.sm)
    }

    fn bind_expression_slots(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        Executor::bind_expression_slots(&mut self.sm, store, var_prefix)
    }

    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        Executor::read_slots(&self.sm)
    }

    fn current_state_name(&self) -> &str {
        Executor::current_state_name(&self.sm)
    }

    fn diagnose_guards(&self, event: Option<&str>) -> Vec<crate::statemachine::GuardDiagnosis> {
        Executor::diagnose_guards(&self.sm, event)
    }

    fn eval_context(&self) -> Option<&EvalContext> {
        Executor::eval_context(&self.sm)
    }

    fn transitions(&self) -> Option<&[crate::TransitionIR]> {
        Executor::transitions(&self.sm)
    }

    fn deferred_event_count(&self) -> usize {
        Executor::deferred_event_count(&self.sm)
    }

    fn accept_ports(&self) -> Vec<String> {
        Executor::accept_ports(&self.sm)
    }

    fn armed_accept_ports(&self) -> Vec<String> {
        Executor::armed_accept_ports(&self.sm)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{StateIR, StateMachineIR, TransitionIR};

    /// Sanity test: a single-mode `ContinuousDynamicsExecutor` with no paired
    /// SM (mode signal never set — falls back to `initial_mode`) steps the
    /// ODE correctly without an orchestrator.
    #[test]
    fn continuous_dynamics_split_step_basic() {
        let rhs: OdeRhs = Arc::new(|_t, _y, _ctx| vec![1.0]);
        let mut cd = ContinuousDynamicsExecutor::new(
            "TestSM.mode",
            "running",
            vec!["y".to_string()],
            vec![0.0],
            rhs.clone(),
        );
        cd.add_mode(
            "running",
            HybridMode {
                rhs,
                entry_resets: HashMap::new(),
            },
        );

        let ctx = EvalContext::new();
        let tick_ctx = TickContext {
            t: 0.0,
            dt: 0.1,
            tick: 0,
            context: &ctx,
            event: None,
            port_payloads: &[],
            local_clock_time: None,
        };

        let output = cd.tick(&tick_ctx);
        assert!(
            (cd.state[0] - 0.1).abs() < 0.001,
            "y should be ~0.1 after one step, got {}",
            cd.state[0]
        );
        assert_eq!(output.current_state, "running");
    }

    /// RSC-4.3 (L41): a mode with an attached `OdeSpec` reports real slot
    /// reads — the gap the fused `HybridExecutor::read_slots` used to leave
    /// unfilled (empty harvest, no retained `OdeSpec`) is closed by
    /// `ContinuousDynamicsExecutor::read_slots`, which the L41 split landed.
    #[test]
    fn continuous_dynamics_read_slots_from_retained_spec() {
        use crate::expressions::{compile_simple_expression, SlotBinder};
        use crate::ode_builder::OdeSpec;
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

        let mut store = SlotStore::new();
        let peer_slot = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(sysml_core::ElementId::from_string("decl:peer".to_string())),
                Variability::Continuous,
                WriterId::Executor(7),
                "peer",
                "peer",
            ),
            Value::Float(2.0),
        );

        let mut expr = compile_simple_expression("peer").unwrap();
        let binder = SlotBinder::for_subsystem(&store, None);
        let mut report = crate::expressions::BindReport::default();
        crate::expressions::bind_slots(&mut expr, &binder, &mut report);
        assert!(report.unresolved.is_empty(), "peer must bind to a slot");

        let mut spec = OdeSpec::new().with_state_var("y", 0.0, expr);
        spec.bind_slots(&store, None);

        let rhs: OdeRhs = Arc::new(|_t, y, _ctx| vec![y[0]]);
        let cd = ContinuousDynamicsExecutor::new(
            "TestSM.mode",
            "running",
            vec!["y".to_string()],
            vec![0.0],
            rhs.clone(),
        )
        .with_mode_spec("running", spec);

        let reads = cd.read_slots();
        assert_eq!(
            reads,
            vec![peer_slot],
            "read_slots must report the retained OdeSpec's real reads, not an empty harvest"
        );
    }

    /// Bouncing-ball hybrid automaton: `ModeSelectionExecutor` (SM half) +
    /// `ContinuousDynamicsExecutor` (ODE half) as two orchestrator
    /// subsystems connected only through the shared context — no private
    /// Rust reference between them. See the module-level "History" note
    /// above for why a mode-transition's entry reset lands one tick later
    /// than the original fused executor's synchronous reset would have
    /// (an extra `orch.step()` after event injection is needed to observe
    /// it below); the assertions here are loose physical bounds, not
    /// pinned numbers, so the lag is not a correctness concern.
    #[test]
    fn bouncing_ball_split_executors() {
        let ir = StateMachineIR::new("BallSM", "flight")
            .with_state(StateIR::new("flight"))
            .with_state(StateIR::new("bounce"))
            .with_transition(TransitionIR::new("flight", "bounce").with_event("ground_hit"))
            .with_transition(TransitionIR::new("bounce", "flight").with_event("launched"));

        let sm = StateMachineRunner::new(ir);
        let mode_selector = ModeSelectionExecutor::new(sm, "BallSM.mode");

        let flight_rhs: OdeRhs = Arc::new(|_t, y, _ctx| vec![y[1], -9.81]);
        let bounce_rhs: OdeRhs = Arc::new(|_t, y, _ctx| vec![y[1], -9.81]);

        let mut cd = ContinuousDynamicsExecutor::new(
            "BallSM.mode",
            "flight",
            vec!["y".to_string(), "v".to_string()],
            vec![10.0, 0.0],
            flight_rhs.clone(),
        );
        cd.add_mode(
            "flight",
            HybridMode {
                rhs: flight_rhs,
                entry_resets: HashMap::new(),
            },
        );
        cd.add_mode(
            "bounce",
            HybridMode {
                rhs: bounce_rhs,
                entry_resets: {
                    let mut resets = HashMap::new();
                    resets.insert(0, ResetAction::Set(0.0));
                    resets.insert(1, ResetAction::Scale(-0.8));
                    resets
                },
            },
        );

        let mut orch =
            crate::orchestrator::Orchestrator::new(crate::orchestrator::OrchestratorConfig {
                dt_ms: 1.0,
                max_ticks: 2000,
                max_time_ms: 2000.0,
                ..Default::default()
            });

        // SM half first (index 0), ODE half second (index 1) — mirrors the
        // ContinuousDynamics-before-StateMachine ExecutionPhase order.
        orch.add_executor("ball_sm", Box::new(mode_selector));
        orch.add_executor("ball_ode", Box::new(cd));
        mint_split_ball_state_slots(&mut orch);

        let mut last_snap = None;
        for _ in 0..1400 {
            last_snap = Some(orch.step());
        }

        let snap = last_snap.unwrap();
        let y = snap
            .variables
            .get("y")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0);
        let v = snap
            .variables
            .get("v")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0);
        assert!(y < 1.0, "y should be near ground at t=1.4s, got {y}");
        assert!(v < -10.0, "v should be negative (falling), got {v}");

        // One-tick lag (see module doc "History"): the SM processes
        // `ground_hit` and publishes mode="bounce" during THIS step() call's
        // StateMachine phase, which runs AFTER ContinuousDynamics already
        // integrated this tick under the old "flight" mode — so the reset
        // is only visible to the ODE half's read on the NEXT step() call.
        orch.inject_event("ball_sm", "ground_hit");
        let _pre_reset = orch.step();
        let snap2 = orch.step();
        let y2 = snap2
            .variables
            .get("y")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0);
        let v2 = snap2
            .variables
            .get("v")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0);
        assert!(y2.abs() < 0.5, "y should be near 0 after bounce, got {y2}");
        assert!(
            v2 > 0.0,
            "v should be positive (bouncing up) after restitution, got {v2}"
        );

        orch.inject_event("ball_sm", "launched");
        let mut last_snap3 = None;
        for _ in 0..100 {
            last_snap3 = Some(orch.step());
        }
        let snap3 = last_snap3.unwrap();
        let y3 = snap3
            .variables
            .get("y")
            .and_then(|v| v.as_float())
            .unwrap_or(0.0);
        assert!(y3 > 0.0, "ball should be rising after bounce, got {y3}");
    }

    /// Mint the split ball's continuous state slots (`y`, `v`) owned by the
    /// `ContinuousDynamicsExecutor` at subsystem index 1 (SM half registered
    /// first at index 0) — the split analogue of `mint_ball_state_slots`.
    fn mint_split_ball_state_slots(orch: &mut crate::orchestrator::Orchestrator) {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

        let mut store = SlotStore::new();
        for var in ["y", "v"] {
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(sysml_core::ElementId::from_string(format!(
                        "decl:{var}"
                    ))),
                    Variability::Continuous,
                    WriterId::Executor(1),
                    var,
                    var,
                ),
                Value::Float(0.0),
            );
        }
        orch.set_slot_store(store);
        orch.bind_expression_slots(None);
    }
}
