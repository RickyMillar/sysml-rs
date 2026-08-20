//! RSC-2.B0 — Runtime Semantic Core Phase 2 behavioural baselines +
//! writer-collision inventory (tracker `runtime-semantic-core-tracker.md`,
//! §4 row RSC-2.B0).
//!
//! Discipline per `feedback_behavioral_baseline_first`: these tests pin the
//! engine's CURRENT observable behaviour *before* any RSC-2.x code lands.
//! Every later phase (slot store, slot binding, executor cutovers, the
//! RSC-2.5 deletions) must keep these baselines byte-identical — except where
//! a baseline is explicitly flipped alongside the phase that changes it
//! (RS002, see `rsc25_unknown_override_must_fail`).
//!
//! Two pinned models:
//! 1. `crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml` — the mixed
//!    SM + ODE + action model (chosen over `tests/fixtures/book-examples/
//!    coffee-machine`, which has no ODE, and over `examples/valve-gating`,
//!    which has no action defs — this is the only committed example carrying
//!    all three executor kinds plus constraints in one file).
//! 2. `crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml` — the
//!    orchestration stress fixture (2 SMs, action fork/join, RK45 ODE,
//!    flows, constraints).
//!
//! Pinning strategy: deterministic report strings snapshotted with `insta`
//! (the repo's baseline convention). Reports contain NO UUIDs/ElementIds and
//! NO wall-clock data — only model-derived names, simulated time, and
//! computed values. Every list is sorted before rendering, so HashMap
//! iteration order (RandomState, varies per process) cannot leak in.
//! Determinism across runs is verified by running the suite twice.
//!
//! Harness rules (same as `runtime_spec_conformance.rs`): pure runtime only —
//! `TreeSitterParser` → `ModelGraph` → `ModelCompiler::build_workspace_orchestrator`.
//! NO LSP harness, NO SysmlService (deadlock surface, task #225). NO
//! production-code changes — RSC-2.B0 measures.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_core::{ElementKind, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::{build_gated_expressions, context_from_graph, ModelCompiler};
use sysml_runtime::orchestrator::{ExecutionSnapshot, Orchestrator};
use sysml_runtime::statemachine::{action_parser, StateMachineCompiler};
use sysml_runtime::{RegionIR, StateMachineIR, TransitionActionIR};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Workspace root (`crates/testing/sysml-spec-tests` → `../../..`).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

/// Recursively collect `.sysml` files under `dir`, sorted by path so the
/// parse order (and therefore element creation order) is deterministic.
fn collect_sysml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_sysml_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("sysml") {
            out.push(path);
        }
    }
}

/// Parse a set of `.sysml` files into one merged (un-elaborated) ModelGraph.
/// Parse diagnostics are tolerated (several example models reference stdlib
/// types that are unresolved in this pure-runtime pipeline); the graph is
/// what the runtime compiles.
fn load_graph(paths: &[PathBuf]) -> ModelGraph {
    assert!(!paths.is_empty(), "no .sysml files to load");
    let parser = TreeSitterParser::new();
    let files: Vec<SysmlFile> = paths
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("file name")
                .to_owned();
            SysmlFile::new(name, source)
        })
        .collect();
    parser.parse(&files).graph
}

/// Load a single workspace-relative `.sysml` file.
fn load_file(rel: &str) -> ModelGraph {
    let path = workspace_root().join(rel);
    assert!(path.exists(), "model file not found: {}", path.display());
    load_graph(&[path])
}

/// Build the workspace orchestrator the way the service layer does:
/// `ModelCompiler::new` (elaborates) + `context_from_graph` seed +
/// precompiled constraints. dt = 10ms so tick 100 = 1000ms simulated.
fn build_orchestrator(compiler: &ModelCompiler, overrides: &[(String, String)]) -> Orchestrator {
    let base_ctx = context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    compiler
        .build_workspace_orchestrator(
            base_ctx,
            Some(precompiled),
            None,
            None,
            None,
            overrides,
            Some(10.0),
            Some(60_000.0),
        )
        .expect("workspace orchestrator should compile")
}

/// Render a `Value` without leaking identifiers. `Value::Ref` carries an
/// ElementId (UUID-shaped) — pinned data must not contain it, so refs and
/// other non-scalar values render as their shape only.
fn fmt_value(v: &Value) -> String {
    match v {
        Value::Float(f) => format!("{f:?}"),
        Value::Int(i) => format!("{i}"),
        Value::Bool(b) => format!("{b}"),
        Value::String(s) => format!("{s:?}"),
        Value::Null => "null".to_owned(),
        Value::Ref(_) => "<ref>".to_owned(),
        other => format!("<{}>", value_shape(other)),
    }
}

fn value_shape(v: &Value) -> &'static str {
    match v {
        Value::Float(_) => "float",
        Value::Int(_) => "int",
        Value::Bool(_) => "bool",
        Value::String(_) => "string",
        Value::Null => "null",
        Value::Ref(_) => "ref",
        Value::List(_) => "list",
        _ => "other",
    }
}

/// One captured tick of the parity report.
fn render_tick(out: &mut String, snap: &ExecutionSnapshot, full_keys: bool) {
    use std::fmt::Write;

    writeln!(out, "== tick {} ==", snap.tick).unwrap();
    // The clock representative pair.
    writeln!(out, "clock: tick={} time_ms={:?}", snap.tick, snap.time_ms).unwrap();

    // Every SM / subsystem state, sorted by subsystem name.
    let mut states: Vec<(&String, String)> = snap
        .subsystem_states
        .iter()
        .map(|(name, st)| {
            (
                name,
                format!(
                    "kind={} state={} completed={}",
                    st.kind, st.current_state, st.completed
                ),
            )
        })
        .collect();
    states.sort();
    for (name, line) in states {
        writeln!(out, "subsystem-state: {name} :: {line}").unwrap();
    }

    // Every ODE state variable (the derivatives map keys are exactly the
    // prefixed ODE state-var names) with its live value.
    let mut deriv_keys: Vec<&String> = snap.derivatives.keys().collect();
    deriv_keys.sort();
    for key in &deriv_keys {
        let val = snap
            .variables
            .get(key.as_str())
            .map(fmt_value)
            .unwrap_or_else(|| "<missing>".to_owned());
        writeln!(out, "ode-state: {key} = {val}").unwrap();
    }
    writeln!(
        out,
        "derivatives-keys: [{}]",
        deriv_keys
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
    .unwrap();

    // Constraint results, sorted by (name, verdict).
    //
    // Pins the four-valued `VerdictKind`, not a bool: the old `satisfied=`
    // rendering collapsed "evaluated and violated" together with "could not
    // be evaluated", so a baseline that read `satisfied=false` did not say
    // which of the two it had pinned.
    let mut constraints: Vec<String> = snap
        .constraint_results
        .iter()
        .map(|c| format!("constraint: {} verdict={}", c.name, c.verdict))
        .collect();
    constraints.sort();
    for line in constraints {
        writeln!(out, "{line}").unwrap();
    }

    // Full sorted variables key set (names only — values pinned above for
    // the representative set). Captured at the first and last tick to catch
    // key-set growth (the canonical/runtime dual-write minting new keys).
    //
    // This is the RAW value MIRROR (`snap.variables`), NOT the observable
    // projection. `EvalContext::set_slot` mirrors each value under EVERY
    // spelling bound to the slot — canonical + runtime + any `add_alias`
    // extras (e.g. an ODE's qualified `{ode}.duty` / `EnclosureThermalNetwork.T_*`
    // observable keys). That alias-mirror growth is LEGITIMATE, blessed mirror
    // behaviour (read coherence), so it is expected and correct here — do NOT
    // "shrink" these key counts back by filtering the mirror. The observable
    // projection is a SEPARATE surface, gated to meta spellings only by
    // `SlotStore::is_meta_spelling` in `snapshot_view::normalize` (task #8,
    // steward ruling 2026-07-07) — add_alias spellings never reach it.
    if full_keys {
        let mut keys: Vec<&String> = snap.variables.keys().collect();
        keys.sort();
        writeln!(out, "variables-key-count: {}", keys.len()).unwrap();
        for key in keys {
            writeln!(out, "var-key: {key}").unwrap();
        }
    }
}

/// Step the orchestrator to ticks 1, 10, 100 and render the parity report.
fn parity_report(label: &str, orch: &mut Orchestrator) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    writeln!(out, "RSC-2.B0 parity baseline :: {label}").unwrap();

    let mut names = orch.subsystem_names();
    names.sort();
    writeln!(out, "subsystems ({}):", names.len()).unwrap();
    for n in &names {
        writeln!(out, "  {n}").unwrap();
    }

    let snap1 = orch.step();
    assert_eq!(snap1.tick, 1, "first step must be tick 1");
    render_tick(&mut out, &snap1, true);

    let mut snap = snap1;
    while snap.tick < 10 {
        snap = orch.step();
    }
    render_tick(&mut out, &snap, false);

    while snap.tick < 100 {
        snap = orch.step();
    }
    render_tick(&mut out, &snap, true);

    out
}

// ---------------------------------------------------------------------------
// 1. Parity baselines (live)
// ---------------------------------------------------------------------------

/// Mixed SM + ODE + action model: `sim-mega-coffee-machine.sysml`.
#[test]
fn rsc2_baseline_mega_coffee_machine() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
    ));
    let mut orch = build_orchestrator(&compiler, &[]);
    let report = parity_report("sim-mega-coffee-machine", &mut orch);
    insta::assert_snapshot!("rsc2_mega_coffee_machine", report);
}

/// Orchestration stress fixture: 2 SMs + action fork/join + RK45 ODE + flows.
#[test]
fn rsc2_baseline_orchestration_complex() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ));
    let mut orch = build_orchestrator(&compiler, &[]);
    let report = parity_report("orchestration-complex", &mut orch);
    insta::assert_snapshot!("rsc2_orchestration_complex", report);
}

// ---------------------------------------------------------------------------
// 2. RS002 fail-hard unknown override target (live since RSC-2.5)
// ---------------------------------------------------------------------------
//
// SANCTIONED BASELINE FLIP (the ONLY intentional test change of RSC-2.5,
// 2026-06-11): the `rsc2_override_unknown_name_silently_creates` pin that
// used to live here asserted the pre-2.5 latent-bug behaviour — an override
// naming a variable no executor or seed ever bound silently CREATED it in
// the shared context (`EvalContext::set` was an unconditional insert), so a
// typo'd whatif/inject swept a variable nothing reads. It was written to be
// flipped: RS002 (design doc D-2.0.7 / §8 Q3, user-approved) now fails hard,
// the silent-creation pin is deleted, and the intent test below runs live.
// section + changelog 2026-06-11.

/// RS002 (`RSC-2.5`, design doc D-2.0.7, decision §8 Q3, user-approved
/// 2026-06-11): an override naming a variable that neither the slot alias
/// table nor the existing context binds must FAIL HARD — the entry point
/// returns an error and the unknown name does NOT materialize in the
/// runtime namespace. Formerly `#[ignore]`d intent test; live since the
/// RSC-2.5 deletion pass (which removed the silent-creation pin
/// `rsc2_override_unknown_name_silently_creates` in the same change).
#[test]
fn rsc25_unknown_override_must_fail() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ));
    let mut orch = build_orchestrator(&compiler, &[]);

    let bogus = "rsc2NoSuchVariableXyz";
    assert!(
        orch.context.get(bogus).is_none(),
        "precondition: the bogus name is unbound"
    );

    let err = orch
        .apply_overrides_with_aliases(&[(bogus.to_owned(), "1.25".to_owned())])
        .expect_err("RS002: an unknown override target must fail hard");
    assert!(
        err.to_string().contains("RS002") && err.to_string().contains(bogus),
        "the error names its code and the offending target: {err}"
    );

    assert!(
        orch.context.get(bogus).is_none(),
        "RS002: an unknown override target must fail hard, not silently create '{bogus}'"
    );
}

// ---------------------------------------------------------------------------
// 3. Collision inventory (gates RS001)
// ---------------------------------------------------------------------------

/// Collect every assignment target a `TransitionActionIR` can write.
fn action_ir_targets(action: &TransitionActionIR, out: &mut BTreeSet<String>) {
    match action {
        TransitionActionIR::Structured { assignments, .. } => {
            for a in assignments {
                out.insert(a.variable.clone());
            }
        }
        TransitionActionIR::Simple(s) => {
            // The SM compiler stores entry/do/exit/effect text; normalize it
            // through the same parser the runner uses so `x = 1; y = 2`
            // strings surface their targets.
            if let TransitionActionIR::Structured { assignments, .. } = action_parser::parse_action(s)
            {
                for a in assignments {
                    out.insert(a.variable.clone());
                }
            }
        }
    }
}

/// Write-set of one compiled state machine: every Structured-assignment
/// target across transitions and state entry/do/exit actions, recursively
/// through sub-machines and parallel regions.
fn sm_write_set(ir: &StateMachineIR) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_sm_targets(ir, &mut out);
    out
}

fn collect_sm_targets(ir: &StateMachineIR, out: &mut BTreeSet<String>) {
    for t in &ir.transitions {
        if let Some(a) = &t.action {
            action_ir_targets(a, out);
        }
    }
    for s in &ir.states {
        collect_state_targets(s, out);
    }
    for r in &ir.regions {
        collect_region_targets(r, out);
    }
}

fn collect_region_targets(r: &RegionIR, out: &mut BTreeSet<String>) {
    for t in &r.transitions {
        if let Some(a) = &t.action {
            action_ir_targets(a, out);
        }
    }
    for s in &r.states {
        collect_state_targets(s, out);
    }
}

fn collect_state_targets(s: &sysml_runtime::StateIR, out: &mut BTreeSet<String>) {
    for a in [&s.entry_action, &s.do_action, &s.exit_action].into_iter().flatten() {
        action_ir_targets(a, out);
    }
    if let Some(sub) = &s.sub_machine {
        collect_sm_targets(sub, out);
    }
}

/// Write-set of one compiled action graph: Assign targets, Accept payload
/// bindings, Perform output bindings — recursively through inline
/// sub-action graphs.
fn action_graph_write_set(ir: &sysml_runtime::actions::ActionGraphIR) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_action_graph_targets(ir, &mut out);
    out
}

fn collect_action_graph_targets(
    ir: &sysml_runtime::actions::ActionGraphIR,
    out: &mut BTreeSet<String>,
) {
    use sysml_runtime::actions::ActionNodeIR;
    for node in &ir.nodes {
        match node {
            ActionNodeIR::Assign { target, .. } => {
                out.insert(target.clone());
            }
            ActionNodeIR::Accept { payload_binding, .. } => {
                if !payload_binding.is_empty() {
                    out.insert(payload_binding.clone());
                }
            }
            ActionNodeIR::Perform {
                output_binding,
                sub_action,
                ..
            } => {
                if let Some(b) = output_binding {
                    if !b.is_empty() {
                        out.insert(b.clone());
                    }
                }
                if let Some(sub) = sub_action {
                    collect_action_graph_targets(sub, out);
                }
            }
            _ => {}
        }
    }
}

/// Per-writer write-sets for one compiled model, at the IR level the slot
/// compiler (RSC-2.1) will see when it mints slot claims:
///
/// - `ode:{name}` — `OdeDetection.state_vars` (the executor's integration
///   writeback) plus its `signal_exprs` keys (GetOutput algebraic outputs the
///   same executor writes each sync_out).
/// - `sm:{name}` — every Structured-assignment target in the compiled
///   `StateMachineIR` (transition effects + state entry/do/exit).
/// - `action:{name}` — Assign / Accept-payload / Perform-output targets of
///   every named ActionDefinition's compiled graph.
/// - `computed` — `build_gated_expressions` targets (the orchestrator's own
///   computed/gated expression evaluator — `WriterId::Orchestrator` in the
///   RSC-2.1 taxonomy).
///
/// Instance multiplication clones SM/ODE writers under disjoint
/// `{instance}.` prefixes, so a bare-name overlap between two writer KINDS
/// here is exactly the per-instance slot collision RS001 will report.
/// Instance-prefixed computed targets (`<instance>.tripped`, …) are kept
/// as-is; they only collide if an executor writes the same prefixed key.
fn model_write_sets(compiler: &ModelCompiler) -> Vec<(String, BTreeSet<String>)> {
    let graph = compiler.graph();
    let mut writers: Vec<(String, BTreeSet<String>)> = Vec::new();

    // ODE executors (one per detection; instance expansion clones these
    // per-prefix with identical bare-name write-sets).
    for ode in compiler.detect_all_odes() {
        let label = format!("ode:{}", ode.name.as_deref().unwrap_or("<anonymous>"));
        let mut set: BTreeSet<String> = ode.state_vars.iter().cloned().collect();
        set.extend(ode.signal_exprs.keys().cloned());
        writers.push((label, set));
    }

    // SM executors.
    for (name, result) in StateMachineCompiler::compile_all(graph) {
        if let Ok(ir) = result {
            writers.push((format!("sm:{name}"), sm_write_set(&ir)));
        }
    }

    // Action executors (not added by build_workspace_orchestrator today, but
    // action.start/run executes these graphs against the same shared
    // context, so their write claims gate RS001 just the same).
    let mut action_names: Vec<String> = graph
        .elements_by_kind(&ElementKind::ActionDefinition)
        .filter_map(|e| e.name.clone())
        .collect();
    action_names.sort();
    action_names.dedup();
    for name in action_names {
        if let Ok(ir) = compiler.compile_action(&name) {
            let set = action_graph_write_set(&ir);
            if !set.is_empty() {
                writers.push((format!("action:{name}"), set));
            }
        }
    }

    // Orchestrator-owned computed/gated expressions.
    let computed: BTreeSet<String> = build_gated_expressions(graph)
        .into_iter()
        .map(|spec| spec.target)
        .collect();
    if !computed.is_empty() {
        writers.push(("computed".to_owned(), computed));
    }

    writers
}

/// Overlap lines for one model: `"{model} :: {var} :: writerA + writerB"`,
/// one line per variable claimed by ≥2 distinct writers, sorted.
fn collision_lines(model: &str, compiler: &ModelCompiler) -> Vec<String> {
    let writers = model_write_sets(compiler);

    // Duplicate writer labels would hide same-label collisions — surface
    // them as their own finding instead of silently merging.
    let mut label_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (label, _) in &writers {
        *label_counts.entry(label.as_str()).or_default() += 1;
    }

    let mut claims: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (label, set) in &writers {
        for var in set {
            claims.entry(var.as_str()).or_default().insert(label.as_str());
        }
    }

    let mut lines = Vec::new();
    for (label, count) in label_counts {
        if count > 1 {
            lines.push(format!("{model} :: <duplicate writer label> :: {label} ×{count}"));
        }
    }
    for (var, labels) in claims {
        if labels.len() >= 2 {
            let joined = labels.into_iter().collect::<Vec<_>>().join(" + ");
            lines.push(format!("{model} :: {var} :: {joined}"));
        }
    }
    lines.sort();
    lines
}

/// The corpus whose write-sets gate RS001: every workspace under `examples/`
/// that compiles to an orchestrator, plus the two diagram example models the
/// parity baselines pin. Pinned so adding a model (or a model gaining a new
/// executor) forces a conscious inventory re-run before RS001 can land.
/// Measured 2026-06-11: `damped-oscillator`, `physics-diagnostics-demo` and
/// `radiation-cooling` do NOT currently build a workspace orchestrator in
/// this pipeline (no compilable SM/ODE/discrete subsystem found);
/// `view-showcase` does (it carries a state def).
const EXPECTED_COMPILING_MODELS: &[&str] = &[
    // RSC port-flow Wave B-inc-2: an ACTION receiver advances past `accept …
    // via <port>`. Compiles to an orchestrator via its sender SM `RelayLogic`
    // (plus two registered action subsystems recvWired/recvUnwired);
    // cross-executor overlaps=0 (actions write token-local only — no shared
    // var claims; verified at add time).
    "action-port-message-delivery",
    "bouncing-ball",
    "coulomb-friction",
    // The two non-`examples/` models pinned by the parity baselines (moved
    // here into their string-sorted position — `compiling.sort()` below
    // sorts the live result, so the pin must match: "crates/..." sorts
    // between "coulomb-friction" and "dc-motor").
    "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
    // `damped-oscillator` is deliberately NOT here. Its usage-form dynamics
    // action IS detected now, but the model declares 2 state variables (x, v)
    // against a single scalar `getNextState`, which the normative library says
    // returns the whole StateSpace vector. That is under-determined, and the
    // detector's old one-expression-for-N-states broadcast made it look like it
    // was not — a zeta sweep returned five identical numbers. The broadcast is
    // deleted and the model is now refused with a diagnostic naming the
    // mismatch, so it does not compile to an orchestrator. Re-add it here only
    // if the fixture is corrected to one return per state variable.
    "dc-motor",
    "digital-filter",
    // D8/D9 clean-room replacement fixtures (plan §10.1/§10.2). Both compile to
    // an orchestrator; cross-executor overlaps=0 (station lifecycle SM + shared
    // plants; the two DispatchLogic state defs were renamed distinct to avoid an
    // RS001 collision — verified at add time).
    "espresso-production-cell",
    "espresso-pump-hybrid",
    // RSC-3.5-pre: ExchangePlane corpus fixture (SM + 3 link classes). Compiles
    // to an orchestrator via its TripLogic state def; cross-executor overlaps=0
    // (introduces no RS001 collision — verified at add time).
    "exchange-plane-fixture",
    // The legacy multi-circuit model was removed from the collision-inventory
    // pin by the retired internal PURGE-01 lockstep packet (the model dir was deleted
    // from the public tree; its behavioural coverage lives on espresso-production-cell).
    // Analyze-workbench demo model (added 1b7cc820): damped spring-mass
    // sweep study. Compiles to an orchestrator via its SpringDynamics ODE
    // (PositionDerivative/VelocityDerivative); pure-Real states, no SMs,
    // no shared writers — cross-executor overlaps=0 (introduces no RS001
    // collision; verified at re-pin time, inventory run 2026-07-27).
    "oscillator-tuning-study",
    // RSC-3.5d: port-message delivery behavioural baseline (relay -> breaker SM
    // via `accept cmd via tripIn`). Compiles to an orchestrator via its Logic
    // state def; cross-executor overlaps=0 (single isolated SM — no RS001
    // collision). Proves the router is wired from the link graph (step 10b).
    "port-message-delivery",
    // RSC-3.5d / L26 GAP 2: multi-instance port-message delivery (two
    // breaker1/breaker2 : BreakerDef usages → multiplied breaker1.Logic /
    // breaker2.Logic). Compiles to an orchestrator via the multiplied Logic
    // state defs; cross-executor overlaps=0 (isolated per-instance SMs — no
    // RS001 collision; verified at add time). Proves owner-key fan-out isolates
    // sibling instances.
    "port-message-delivery-multi",
    // A-scalar payload-carrying messages: two sender/receiver pairs prove the
    // SM-send wire carries a real payload Value consumed in a guard — `true`
    // trips the receiver, `false` is delivered but blocked. Four distinct
    // state-def names → four isolated SMs; cross-executor overlaps=0.
    "port-message-payload",
    // A-structured payload: `send new TripCommand(tripValue = <bool>)` → a
    // Value::Map the receiver reads via `if cmd.tripValue`. Positive trips,
    // negative twin stays closed. Four distinct state-def names; overlaps=0.
    "port-message-payload-structured",
    // L26 SEND half: a compiled SENDER SM (`RelayLogic`) drives `send TripCommand
    // via tripOut`, routing through the wired router to trip the receiver SM —
    // no `send_to_router` injection. Two distinct state-def names (RelayLogic /
    // Logic) → two isolated SMs; cross-executor overlaps=0 (no RS001 collision).
    "port-message-send",
    // RSC-5.2 UQ002: cross-dimension signal-link proving fixture (a trivial
    // MonitorLogic state def makes it build). Compiles to an orchestrator;
    // cross-executor overlaps=0 (single isolated SM — no RS001 collision).
    "quantity-signal-mismatch",
    // RSC-5.4: explicit-`[unit]` snapshot proving fixture (a minimal sliding
    // block ODE + one unit-bearing constant ISQ slot). Compiles to an
    // orchestrator via its VelocityDerivative ODE; cross-executor overlaps=0
    // (single isolated executor — introduces no RS001 collision).
    "quantity-snapshot-demo",
    // J4 sweep repair (2026-08-19). Its Stefan-Boltzmann RHS spells
    // exponentiation `^`, which the string-expression compiler did not accept
    // (only `**`), so `RadiatingBody`'s solver failed to build and the model
    // died at slot-mint with a mint-gap error. Now registers one RK45 ODE
    // subsystem (`RadiatingBody`, state `temperature`); cross-executor
    // overlaps=0 — single isolated executor, no RS001 collision. Verified at
    // re-pin time, inventory run 2026-08-19.
    "radiation-cooling",
    // The legacy oscillator model was removed from the collision-inventory pin by the retired internal
    // PURGE-01 lockstep packet (model dir deleted; coverage on espresso-pump-hybrid).
    "three-phase-ac",
    "valve-gating",
    "view-showcase",
    "zero-crossing-event",
];

/// The pinned cross-executor overlap list (RSC-2.B0 deliverable). Every
/// entry here is a variable that two different runtime writers claim today
/// — last-write-wins with no diagnostic (design doc §2.4). RS001 lands as a
/// hard error only once this list is EMPTY (models fixed or claims
/// reclassified); shrink it consciously, never grow it.
const EXPECTED_OVERLAPS: &[&str] = &[];

#[test]
fn rsc2_collision_inventory() {
    let examples_dir = workspace_root().join("examples");
    let mut model_dirs: Vec<PathBuf> = std::fs::read_dir(&examples_dir)
        .expect("examples/ dir readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    model_dirs.sort();

    let mut compiling: Vec<String> = Vec::new();
    let mut overlaps: Vec<String> = Vec::new();

    for dir in &model_dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("dir name")
            .to_owned();
        let mut files = Vec::new();
        collect_sysml_files(dir, &mut files);
        if files.is_empty() {
            continue;
        }
        // Only workspaces that actually produce an orchestrator are in
        // scope — RS001 fires at orchestrator build time. Build the way
        // production consumers do: source dir attached (resolves
        // @DataSource CSVs → `__sf_*` sampled-function keys) + the ide-db
        // production seeder. RSC-2.5's RS003 hard error correctly rejects
        // unseeded builds of models that need them (the legacy oscillator model
        // referenced its B-H waveform from an ODE RHS).
        let compiler = ModelCompiler::new(load_graph(&files)).with_source_dir(dir);
        let base_ctx =
            sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
        let built = compiler.build_workspace_orchestrator(
            base_ctx,
            None,
            None,
            None,
            None,
            &[],
            Some(10.0),
            Some(1_000.0),
        );
        if built.is_err() {
            continue;
        }
        compiling.push(name.clone());
        overlaps.extend(collision_lines(&name, &compiler));
    }

    // The two non-`examples/` models pinned by the parity baselines.
    for rel in [
        "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ] {
        let file_dir = workspace_root()
            .join(rel)
            .parent()
            .expect("model file has a parent dir")
            .to_path_buf();
        let compiler = ModelCompiler::new(load_file(rel)).with_source_dir(&file_dir);
        let base_ctx =
            sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
        let built = compiler.build_workspace_orchestrator(
            base_ctx,
            None,
            None,
            None,
            None,
            &[],
            Some(10.0),
            Some(1_000.0),
        );
        assert!(built.is_ok(), "{rel} must compile to an orchestrator");
        compiling.push(rel.to_owned());
        overlaps.extend(collision_lines(rel, &compiler));
    }

    compiling.sort();
    overlaps.sort();

    eprintln!("== RSC-2.B0 collision inventory ==");
    eprintln!("compiling models ({}):", compiling.len());
    for m in &compiling {
        eprintln!("  {m}");
    }
    eprintln!("cross-executor overlaps ({}):", overlaps.len());
    for o in &overlaps {
        eprintln!("  {o}");
    }

    assert_eq!(
        compiling,
        EXPECTED_COMPILING_MODELS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>(),
        "the set of orchestrator-compiling example models changed — re-run the \
         collision inventory consciously and update the pin"
    );
    assert_eq!(
        overlaps,
        EXPECTED_OVERLAPS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>(),
        "cross-executor write-set overlaps changed — RS001 (hard error) is gated \
         on this list; fix the model or reclassify the writer before updating the pin"
    );
}

/// RSC-3.6 Phase 0 census + corpus-wide gate. Builds every orchestrator-producing
/// corpus model the way the service does and records every prefixed (instance-
/// multiplied) executor that is NOT scoped-view-bypass-eligible
/// (`Orchestrator::scoped_view_fallbacks`). Those executors are served only by the
/// legacy `build_scoped_context` variable-map clone; it can be deleted (RSC-3.5f.3)
/// only once this is EMPTY corpus-wide. This is the whole-corpus gate that
/// 3.5f.1's per-model pins lacked — see `rsc-3.6-slot-coverage-completion.md`.
///
/// Census finding (2026-06-16, EMPIRICAL — corrected a wrong survey claim that the
/// corpus was clean): of 13 orchestrator-producing models, exactly ONE real model
/// has bypass-ineligible prefixed executors — `exchange-plane-fixture`'s
/// `controllerA.TripLogic` / `controllerB.TripLogic` (one `state def TripLogic`
/// instantiated 2×). Its transition guard `currentIn.reading.value > threshold`
/// is not fully slot-bound (the signal-port-feature chain + the `threshold`
/// attribute do not resolve to instance-local SlotRefs), so `scoped_bypass_eligible`
/// (statemachine/mod.rs:1820) rejects it. Every other model has 0 prefixed
/// executors.
///
/// So the real-corpus #253 worklist was ONE SM shape (an unbound-guard binding), not
/// a pile. The structured-action-assignment shape (`tripCount = tripCount + 1`) and
/// the no-slots hand-built shape live only in test fixtures (`two_unit_sm_graph`,
/// the orchestrator.rs raw-API tests).
///
/// **2026-06-16 — Phase 2 step (1) DONE (RSC-3.6).** The compiler now mints a
/// per-instance slot per read-leaf of a prefixed SM's guard/trigger expressions
/// (`SmGuardRead` → `mint_slot_store` step 6c), so the `TripLogic` guard binds to
/// instance-local `SlotRef`s (`controllerA.threshold` = `Float(5.0)`,
/// `controllerA.currentIn.reading.value` = `Null`) and both executors are now
/// scoped-view-bypass-eligible. Byte-identical: `Null > 5.0` errors → guard
/// event-name fallback → false, exactly the legacy chain-resolution verdict (the SM
/// never trips on either path; rsc3_exchange_baseline + exchange_plane_fixture +
/// runtime_spec_conformance all green). The whole REAL corpus is now bypass-clean
/// (census == 0). Pin is EMPTY — it must STAY empty (a new entry = a real model
/// introduced an ineligible shape; fix its slot-coverage, do not grow the pin).
/// (The test-only `two_unit_sm` structured-action + raw-API shapes — Phase 2 steps
/// 2/3 — are not in this corpus census but still block the actual `build_scoped_context`
/// deletion in 3.5f.3.)
const EXPECTED_BYPASS_FALLBACKS: &[&str] = &[];

#[test]
fn rsc36_bypass_eligibility_census() {
    let mut models: Vec<(String, ModelGraph, PathBuf)> = Vec::new();

    let examples_dir = workspace_root().join("examples");
    let mut model_dirs: Vec<PathBuf> = std::fs::read_dir(&examples_dir)
        .expect("examples/ dir readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    model_dirs.sort();
    for dir in &model_dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("dir name")
            .to_owned();
        let mut files = Vec::new();
        collect_sysml_files(dir, &mut files);
        if files.is_empty() {
            continue;
        }
        models.push((name, load_graph(&files), dir.clone()));
    }
    for rel in [
        "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ] {
        let file_dir = workspace_root()
            .join(rel)
            .parent()
            .expect("model file has a parent dir")
            .to_path_buf();
        models.push((rel.to_owned(), load_file(rel), file_dir));
    }

    let mut census: Vec<String> = Vec::new();
    let mut compiling = 0usize;
    eprintln!("== RSC-3.6 bypass-eligibility census ==");
    for (name, graph, dir) in &models {
        let compiler = ModelCompiler::new(graph.clone()).with_source_dir(dir);
        let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
        let built = compiler.build_workspace_orchestrator(
            base_ctx,
            None,
            None,
            None,
            None,
            &[],
            Some(10.0),
            Some(1_000.0),
        );
        let Ok(orch) = built else {
            continue;
        };
        compiling += 1;
        let prefixed = orch
            .subsystems()
            .iter()
            .filter(|s| s.var_prefix.is_some())
            .count();
        let fallbacks = orch.scoped_view_fallbacks();
        eprintln!(
            "  {name}: {prefixed} prefixed executor(s), {} bypass-ineligible",
            fallbacks.len()
        );
        for (exec, phase) in &fallbacks {
            census.push(format!("{name} :: {exec} ({phase:?})"));
        }
    }
    census.sort();
    eprintln!(
        "compiling models: {compiling}; corpus-wide scoped-view fallbacks: {}",
        census.len()
    );
    for c in &census {
        eprintln!("  FALLBACK {c}");
    }

    assert_eq!(
        census,
        EXPECTED_BYPASS_FALLBACKS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>(),
        "RSC-3.6: corpus-wide bypass-ineligible prefixed-executor set changed. This \
         pin gates the 3.5f.3 deletion of build_scoped_context — it must SHRINK to \
         empty as #253 makes each shape slot-servable, and must NEVER GROW (a new \
         entry = a real model introduced the ineligible shape). Fix the model's \
         slot-coverage (route assignment-action writes/reads or unbound guard reads \
         through instance slots, or drain ODE RHS fallbacks) before updating the \
         pin — see rsc-3.6-slot-coverage-completion.md."
    );
}
