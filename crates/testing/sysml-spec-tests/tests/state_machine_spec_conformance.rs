//! SMC — State-machine behavioral spec-conformance harness.
//!
//! Sibling to `runtime_spec_conformance.rs` (RSC, which gates the
//! trigger/guard/clock subset) and `constraint_spec_conformance.rs` (CSC).
//! This file gates the **BEHAVIORAL** state-machine obligations catalogued in
//! `spec-obligations/state-machines.md` — transition selection, guard gating,
//! firing order (exit→effect→entry), entry-on-enter, exit-on-leave, and
//! do-action-while-resident. The STRUCTURAL and SPEC-SILENT rows of that
//! matrix are recorded there and are NOT gated here; rows already gated
//! elsewhere (guard-boolean / trigger-is-accept / guard-during-source via
//! RSC-0.2) get a focused, cheap pin here where it is observable.
//!
//! Each test encodes ONE spec-defined obligation and asserts the engine's
//! CURRENT behavior against it, carrying a verdict marker on its own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation (test
//!   runs and passes).
//! - `// VERDICT: DIVERGES — <spec vs actual>` — a CONFIRMED gap. The test
//!   asserts the SPEC-CORRECT expectation and is `#[ignore]`d pending the fix,
//!   so plain `cargo test` shows it ignored/pending; deleting the `#[ignore]`
//!   when fixed turns it green. `cargo test -- --ignored` runs it and it FAILS
//!   against current behavior (the proof the gap is real).
//! - `// VERDICT: UNIMPLEMENTED — <why>` — the obligation has no observable
//!   engine surface; `#[ignore]`d, asserting the spec-correct expectation.
//!
//! Each test names the obligation it gates with an `// OBL:` line whose id
//! matches `spec-obligations/state-machines.md` (the authority for citations).
//!
//! Spec sources (cited per obligation in the tracker):
//! - SysML §7.18 "States" (`SysML-spec-r2025-04_REF.html`):
//!   - §7.18.1 entry/do/exit lifecycle.
//!   - §7.18.3 transition firing order
//!     ("…source do interrupted… source exit… effect… target entry…").
//! - KerML `Kernel Semantic Library/StatePerformances.kerml`,
//!   `TransitionPerformances.kerml`.
//!
//! Harness rules (match RSC): pure runtime only — parsed sysml source →
//! `ModelGraph` → `ModelCompiler::build_sm_orchestrator` (the canonical
//! single-SM-no-ODE compile path that mints the slot table and binds
//! expressions, ledger L44). NO LSP, NO SysmlService, NO production code
//! changes — this file measures.
//!
//! OBSERVABILITY NOTE: state/transition actions execute observably only as
//! ASSIGNMENT actions (text containing `=`); bare-name actions are formatted
//! but have no context effect. Each behavioral case here writes a context
//! variable from its action and reads it back through the snapshot.
//!
//! KEY EMPIRICAL FINDING (the SM behavioral gap this gate pins): the runtime
//! compiles and executes the assignment body of ENTRY actions, but for EXIT
//! actions, DO actions, and TRANSITION EFFECTS the compiler collapses the
//! action to `Simple("")` (no assignments lowered) — see
//! `StateMachineCompiler::compile_state` taking the string-prop branch for
//! `exit`/`do_action` instead of walking the action children the way the
//! `entry` branch does, and `compile_transition_usage` reading only the
//! `effect` string prop. The state-change *phases* still run (the SM moves
//! through the transition and emits an `"exit: "` trace), but the action's
//! *effect* is dropped. These rows are gated DIVERGES: each test asserts the
//! SPEC-CORRECT expectation and is `#[ignore]`d pending the fix, so a fix-wave
//! that lowers the bodies turns them green by deleting the `#[ignore]`.
//!
//! The summary test (`smc_matrix_summary`) self-scans this file via
//! `include_str!` and prints the CONFORMS / DIVERGES / UNIMPLEMENTED counts.

use sysml_core::{elaborate::elaborate, ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::Orchestrator;

// ---------------------------------------------------------------------------
// Harness helpers (copied from runtime_spec_conformance.rs — not pub there)
// ---------------------------------------------------------------------------

/// Parse a sysml source string into an (un-elaborated) ModelGraph.
///
/// Parse errors fail the test immediately — every fixture here must be syntax
/// the tree-sitter grammar accepts, otherwise the case measures the parser
/// instead of the runtime.
fn parse_source(source: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("state_machine_spec_conformance.sysml", source)]);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "fixture source must parse cleanly, got: {errors:?}"
    );
    result.graph
}

/// Build a single-SM orchestrator from parsed source through the **canonical
/// compiler-built path** (`ModelCompiler::build_sm_orchestrator`, ledger L44).
///
/// This mints the compile-known slot table and binds expressions, so an
/// assignment-action body (entry/exit/do/effect) routes its write through a
/// real slot and survives into every snapshot. A bare `Orchestrator::new` +
/// `add_state_machine` — the raw path this replaced — has no compiled write
/// set, so since slot-routed writeback became unconditional (commit
/// `454591c1`) it silently dropped every assignment output and counters stayed
/// `0.0`. The subsystem is registered under the compiled state-def name
/// (`sm_name`), which the caller passes to `inject_event`/`set_clock`.
fn sm_orchestrator(source: &str, sm_name: &str, seeds: &[(&str, f64)]) -> Orchestrator {
    let compiler = ModelCompiler::new(parse_source(source));
    let mut orch = compiler
        .build_sm_orchestrator(sm_name, None, None)
        .expect("state machine should compile");
    for (k, v) in seeds {
        orch.context.set((*k).to_owned(), Value::Float(*v));
    }
    orch
}

/// The sole SM subsystem's state in a step snapshot. These harnesses register
/// exactly one state machine (under its compiled def name), so the subsystem
/// is addressed name-agnostically rather than by a synthetic key.
fn sole_subsystem(
    snap: &sysml_runtime::orchestrator::ExecutionSnapshot,
) -> &sysml_runtime::SubsystemState {
    let mut it = snap.subsystem_states.values();
    let ss = it.next().expect("one sm subsystem present");
    assert!(it.next().is_none(), "exactly one SM subsystem in these harnesses");
    ss
}

/// Current state of the single SM subsystem after a step snapshot.
fn sm_state(snap: &sysml_runtime::orchestrator::ExecutionSnapshot) -> String {
    sole_subsystem(snap).current_state.clone()
}

/// Read a numeric context variable from a snapshot (None if unset/non-numeric).
fn var(snap: &sysml_runtime::orchestrator::ExecutionSnapshot, name: &str) -> Option<f64> {
    match snap.variables.get(name) {
        Some(Value::Float(f)) => Some(*f),
        Some(Value::Int(i)) => Some(*i as f64),
        _ => None,
    }
}

// ===========================================================================
// SMC-1 — transition-selection (event → target)
// An event-triggered transition fires and moves the machine to the named
// target state when its triggering event is accepted.
// Spec: §7.18.3 transitions; TransitionPerformances.kerml (an accepted trigger
// drives the transition to its target). Related to the GATED-elsewhere
// `trigger-is-accept-action` row; pinned here for the SM-state surface.
// ===========================================================================

#[test]
fn smc1_event_drives_transition_to_target() {
    // OBL: transition-selection
    // VERDICT: CONFORMS
    let source = r#"
        package SelectSM {
            state def SelSM {
                state Idle;
                state Running;
                transition go first Idle accept go then Running;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "SelSM", &[]);
    // No event yet → stays in the initial state.
    let snap0 = orch.step();
    assert_eq!(sm_state(&snap0), "Idle", "no trigger ⇒ no transition");
    // Inject the matching event → moves to the named target.
    orch.inject_event("SelSM", "go");
    let snap1 = orch.step();
    assert_eq!(
        sm_state(&snap1),
        "Running",
        "the accepted event selects the transition and moves to its target"
    );
}

// ===========================================================================
// SMC-2 — no-transition-on-unmatched-event
// An event that matches no outgoing transition of the current state leaves the
// machine in place (no spurious firing).
// Spec: §7.18.3 — a transition fires only when its trigger is accepted.
// ===========================================================================

#[test]
fn smc2_unmatched_event_does_not_fire() {
    // OBL: no-transition-on-unmatched-event
    // VERDICT: CONFORMS
    let source = r#"
        package NoMatchSM {
            state def NmSM {
                state Idle;
                state Running;
                transition go first Idle accept go then Running;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "NmSM", &[]);
    orch.inject_event("NmSM", "nonexistent");
    let snap = orch.step();
    assert_eq!(
        sm_state(&snap),
        "Idle",
        "an event matching no outgoing transition leaves the state unchanged"
    );
}

// ===========================================================================
// SMC-3 — guard-gates-transition
// A transition with a Boolean guard fires only when the guard is true; a false
// guard blocks it. (The `transition-guard-boolean` row is GATED-elsewhere via
// RSC `spec_trigger_when_*`; this is a focused state-surface pin.)
// Spec: §7.18.3 / §8.3.18.8 — guard is Boolean[1..1], must be true to occur.
// NOTE: a `when <expr>` accept lowers to a guard-only transition (RSC G3),
// which is the clean way to exercise pure guard gating from parsed source.
// ===========================================================================

#[test]
fn smc3_guard_true_fires_false_blocks() {
    // OBL: transition-guard-boolean
    // VERDICT: CONFORMS
    let source = r#"
        package GuardSM {
            state def GdSM {
                state Idle;
                state Running;
                transition first Idle accept when level > 5 then Running;
            }
        }
    "#;
    // Guard false (level=1): transition blocked, machine rests in Idle.
    let mut blocked = sm_orchestrator(source, "GdSM", &[("level", 1.0)]);
    let snap_blocked = blocked.step();
    assert_eq!(
        sm_state(&snap_blocked),
        "Idle",
        "a false guard blocks the transition"
    );

    // Guard true (level=10): the same transition fires.
    let mut open = sm_orchestrator(source, "GdSM", &[("level", 10.0)]);
    let snap_open = open.step();
    assert_eq!(
        sm_state(&snap_open),
        "Running",
        "a true Boolean guard admits the transition"
    );
}

// ===========================================================================
// SMC-4 — transition-firing-order-exit-effect-entry  [GAP-SM-1, highest value]
// Firing sequence on a transition: source exit action → transition effect →
// target entry action (the source do-action is interrupted first).
// Spec: §7.18.3 "If the source state has a do action that is still being
// performed, that is interrupted… source exit… effect… target entry…".
//
// ACTUAL: the firing order cannot be observed because two of its three steps
// have NO effect — the runtime lowers EXIT actions and TRANSITION EFFECTS to
// `Simple("")` (assignment body dropped; see module-doc finding). Only the
// target ENTRY action executes its body. So a `seq`-counter probe records only
// the entry write; exit and effect contribute nothing. This pins that actual
// behavior: of {exit, effect, entry}, only entry is observable.
// ===========================================================================

#[test]
fn smc4_firing_order_exit_effect_entry() {
    // OBL: transition-firing-order-exit-effect-entry
    // VERDICT: CONFORMS — GAP-SM-EFFECT closed: the grammar now attaches an inline
    // `do action {…}` effect to the transition (effect_action accepts the `action`
    // keyword form), and the effect body executes between exit and entry (SysML §7.18.3).
    // NOTE: the effect `do action {…}` is written BEFORE `then <target>`, per the spec
    // grammar (xtext TransitionUsage 1849-1857: source trigger guard effect 'then' target).
    let source = r#"
        package OrderSM {
            state def OrdSM {
                state A {
                    exit action { aExit = seq; seq = seq + 1; }
                }
                state B {
                    entry action { bEntry = seq; seq = seq + 1; }
                }
                transition first A accept go do action { eff = seq; seq = seq + 1; } then B;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "OrdSM", &[("seq", 0.0)]);
    orch.inject_event("OrdSM", "go");
    let snap = orch.step();
    assert_eq!(sm_state(&snap), "B", "the transition fires into the target");

    // SPEC §7.18.3: all three action bodies run, in order exit → effect → entry,
    // so `seq` advances three times and the captured counters are strictly
    // ordered aExit(0) < eff(1) < bEntry(2).
    assert_eq!(
        var(&snap, "seq"),
        Some(3.0),
        "SPEC: all three action bodies (exit, effect, entry) run — seq advances 3×"
    );
    let a = var(&snap, "aExit").expect("SPEC: source exit body runs");
    let e = var(&snap, "eff").expect("SPEC: transition effect body runs");
    let b = var(&snap, "bEntry").expect("SPEC: target entry body runs");
    assert!(
        a < e && e < b,
        "SPEC §7.18.3 firing order exit→effect→entry: expected aExit({a}) < eff({e}) < bEntry({b})"
    );
}

// ===========================================================================
// SMC-5 — entry-on-enter
// A state's entry action runs (and applies its body) when the state is entered
// via a transition.
// Spec: §7.18.1 "An entry action starts when the state is activated";
// StatePerformances.kerml:43-45.
// ===========================================================================

#[test]
fn smc5_entry_action_runs_on_state_entry() {
    // OBL: entry-do-exit-ordering
    // VERDICT: CONFORMS
    let source = r#"
        package EntrySM {
            state def EnSM {
                state A;
                state B {
                    entry action { bEntered = 1; }
                }
                transition first A accept go then B;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "EnSM", &[("bEntered", 0.0)]);
    // Before entering B, the entry action has not run.
    let snap0 = orch.step();
    assert_eq!(var(&snap0, "bEntered"), Some(0.0), "entry not yet run while in A");
    // Enter B → entry action runs and applies its body.
    orch.inject_event("EnSM", "go");
    let snap1 = orch.step();
    assert_eq!(sm_state(&snap1), "B");
    assert_eq!(
        var(&snap1, "bEntered"),
        Some(1.0),
        "entry action body runs exactly when the state is entered"
    );
}

// ===========================================================================
// SMC-6 — exit-on-leave
// A state's exit action runs (and applies its body) when the state is left via
// a transition.
// Spec: §7.18.1 / §7.18.3 — "on exit, the do action is interrupted and the
// exit action runs to completion".
//
// ACTUAL: the exit PHASE runs (the SM emits an `"exit: "` trace and leaves the
// state), but the exit action's assignment body is dropped — the compiler
// lowers it to `Simple("")`. The body's write never lands.
// ===========================================================================

#[test]
fn smc6_exit_action_runs_on_leave() {
    // OBL: entry-do-exit-ordering
    // VERDICT: CONFORMS — GAP-SM-EXEC (exit) closed: compile_state now walks the tagged exit child to a Structured body (SysML §7.18.1/§7.18.3).
    let source = r#"
        package ExitSM {
            state def ExSM {
                state A {
                    exit action { aLeft = 1; }
                }
                state B;
                transition first A accept go then B;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "ExSM", &[("aLeft", 0.0)]);
    // Resident in A, not leaving yet.
    let snap0 = orch.step();
    assert_eq!(var(&snap0, "aLeft"), Some(0.0), "exit not run while resident in A");
    // Leave A → the SM moves to B and the exit body applies.
    orch.inject_event("ExSM", "go");
    let snap1 = orch.step();
    assert_eq!(sm_state(&snap1), "B", "the SM leaves A for B");
    assert_eq!(
        var(&snap1, "aLeft"),
        Some(1.0),
        "SPEC §7.18.1: exit action body runs on leave — aLeft must be 1"
    );
}

// ===========================================================================
// SMC-7 — do-action-while-resident
// A state's do action runs (and applies its body) on each step while the state
// is the active (resident) state.
// Spec: §7.18.1 "a do action … runs while the state is active";
// StatePerformances.kerml do.startShot.
//
// ACTUAL: the do action's assignment body is dropped (lowered to `Simple("")`),
// so an incrementing do-action body never advances its counter even while the
// state is continuously resident.
// ===========================================================================

#[test]
fn smc7_do_action_runs_while_resident() {
    // OBL: entry-do-exit-ordering
    // VERDICT: CONFORMS — GAP-SM-EXEC (do) closed: compile_state now walks the tagged do child to a Structured body (SysML §7.18.1).
    let source = r#"
        package DoSM {
            state def DoActSM {
                state A {
                    do action { doRuns = doRuns + 1; }
                }
                state B;
                transition first A accept go then B;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "DoActSM", &[("doRuns", 0.0)]);
    let _s1 = orch.step();
    let s2 = orch.step();
    let runs = var(&s2, "doRuns").expect("doRuns present");
    assert!(
        runs >= 2.0,
        "SPEC §7.18.1: do action body runs each resident step — doRuns must be ≥2 after 2 steps, got {runs}"
    );
}

// ===========================================================================
// SMC-8 — initial-state-via-entry-succession
// On construction the machine rests in its initial (first-declared) substate
// before any event; the initial substate is the target of the entry succession.
// Spec: §7.18.2; StatePerformances.kerml:43 ("entry then <initial>").
// ===========================================================================

#[test]
fn smc8_initial_state_is_first_declared_before_any_event() {
    // OBL: initial-state-via-entry-succession
    // VERDICT: CONFORMS
    let source = r#"
        package InitSM {
            state def InSM {
                state First;
                state Second;
                transition first First accept go then Second;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "InSM", &[]);
    let snap = orch.step();
    assert_eq!(
        sm_state(&snap),
        "First",
        "the machine rests in the first-declared substate before any trigger"
    );
}

// ===========================================================================
// SMC-9 — at-most-one-transition-fires-per-trigger
// When two outgoing transitions of the current state both match an event, at
// most one fires per trigger — the machine lands in exactly one target state.
// Spec: StatePerformances.kerml:56,77-83 (accepted[0..1] + dispatch ordering).
// The tracker marks the standalone "one fires" SHALL as SPEC-SILENT, but the
// observable single-target landing is cheap to pin as a determinism guard.
// (Observed via the landed state only — the exit-counter approach is unusable
// here because exit action bodies are dropped, see SMC-6.)
// ===========================================================================

#[test]
fn smc9_at_most_one_transition_fires_per_trigger() {
    // OBL: at-most-one-transition-fires-per-trigger
    // VERDICT: CONFORMS
    let source = r#"
        package OneFireSM {
            state def OfSM {
                state A;
                state B;
                state C;
                transition tb first A accept go then B;
                transition tc first A accept go then C;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "OfSM", &[]);
    orch.inject_event("OfSM", "go");
    let snap = orch.step();
    let landed = sm_state(&snap);
    assert!(
        landed == "B" || landed == "C",
        "exactly one target is reached, got {landed}"
    );
    assert_ne!(
        landed, "A",
        "the trigger fires exactly one transition out of A — the machine does not stay put \
         (a single occurrence lands in one target)"
    );
}

// ===========================================================================
// SMC-10 — incoming-transition-trigger-recorded
// On entry into a state, the transfer/event that triggered the entry must be
// recorded on the new state's performance (so the triggering event is
// observably associated with the entered state).
// Spec: `StatePerformances.kerml:48-53` — `incomingTransitionTrigger` is bound
// to the triggering transfer when the state is entered.
//
// ACTUAL: there is NO runtime surface that records the triggering event on the
// entered state. The runner emits exit/action/entry *traces* into
// `SubsystemState.outputs` (e.g. `"entry: …"`), and `available_transitions`
// lists *outgoing* transitions, but neither `SubsystemState` nor `StateIR`
// carries the *incoming* trigger that caused the current state — there is no
// `incomingTransitionTrigger`-equivalent field, no "entered via <event>"
// record, and no context variable written with the triggering event. So the
// obligation has no observable engine surface: UNIMPLEMENTED.
// ===========================================================================

#[test]
fn smc10_incoming_transition_trigger_recorded() {
    // OBL: incoming-transition-trigger-recorded
    // VERDICT: CONFORMS — SubsystemState.incoming_transition_trigger records the
    // triggering event on entry (StatePerformances.kerml:48 incomingTransitionTrigger).
    let source = r#"
        package IncomingSM {
            state def IncSM {
                state Idle;
                state Running;
                transition go first Idle accept go then Running;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "IncSM", &[]);
    orch.inject_event("IncSM", "go");
    let snap = orch.step();
    assert_eq!(sm_state(&snap), "Running", "the trigger drives entry into Running");

    // SPEC StatePerformances.kerml:48: the entered state performance records the
    // transfer that triggered its entry. The triggering event was "go".
    let ss = sole_subsystem(&snap);
    assert_eq!(
        ss.incoming_transition_trigger.as_deref(),
        Some("go"),
        "SPEC: the triggering transfer 'go' must be recorded on the entered state \
         (incomingTransitionTrigger)"
    );
}

// ===========================================================================
// SMC-11 — state-sequencing-count-invariant
// Topology invariant on the implicit sequencing graph between a state's
// mutually-exclusive substates: with N exclusive substates there are exactly
// N-1 sequencing links (the substates can be strictly ordered in time).
// Spec: `States.sysml:71-77` —
//   `succession stateSequencing first [0..1] exclusiveStates then [0..1] exclusiveStates;`
//   `assert constraint { notEmpty(exclusiveStates) implies size(stateSequencing) == size(exclusiveStates) - 1 }`
//
// FRAMING (core-steward, 2026-06-22): `stateSequencing` is a LIBRARY-DERIVED
// feature of the abstract `StateAction`/`StateDefinition` element — its honest
// home is the ELABORATION layer (the `ModelGraph`), NOT the execution
// `StateMachineIR`. Materialising the invariant means elaboration generating the
// N-1 implicit `succession` relationships linking a state's `exclusiveStates`
// children, evaluatable as ordinary succession elements; adding a
// `stateSequencing` vec to the execution IR would duplicate that graph structure
// and put abstract-syntax library semantics in an execution IR (CLAUDE #4/#6).
//
// CURRENT BEHAVIOUR IS CORRECT, NOT A FALSE GREEN: neither `exclusiveStates` nor
// `stateSequencing` is populated at elaboration time, so the constraint evaluator
// honestly reports `Error: undefined variable exclusiveStates` (see the
// service-baseline `sysml.evaluate.constraints` snapshots) rather than a vacuous
// pass. Whether to elaborate these library-derived successions at all is
// SPEC-SILENT (design-undecided) — so this stays a documented gap, not a fix.
// The gate below probes the (wrong-layer) execution IR purely to pin that no IR
// surface exists; the obligation's real home is elaboration.
// ===========================================================================

#[test]
fn smc11_state_sequencing_count_invariant() {
    // OBL: state-sequencing-count-invariant
    // VERDICT: CONFORMS — elaboration derives the N-1 `stateSequencing` successions
    // between a state's exclusive substates (States.sysml:71-77).
    let source = r#"
        package SeqSM {
            state def SqSM {
                state A;
                state B;
                state C;
            }
        }
    "#;
    let mut graph = parse_source(source);
    let _ = elaborate(&mut graph);

    // exclusiveStates = the StateUsage children of the (non-parallel) state def.
    let sqsm = graph
        .element_ids_by_kind(&ElementKind::StateDefinition)
        .iter()
        .find(|id| graph.get_element(id).and_then(|e| e.name.as_deref()) == Some("SqSM"))
        .cloned()
        .expect("SqSM present");
    let exclusive_states = graph
        .children_of(&sqsm)
        .filter(|e| e.kind == ElementKind::StateUsage)
        .count();
    assert_eq!(exclusive_states, 3, "three exclusive substates declared");

    // stateSequencing = the elaboration-derived tagged successions among them.
    let state_sequencing_links = graph
        .relationships_by_kind(&RelationshipKind::Succession)
        .filter(|r| {
            r.props
                .get("stateSequencing")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();

    // SPEC States.sysml:71-77: notEmpty(exclusiveStates) ⇒
    //   size(stateSequencing) == size(exclusiveStates) - 1.
    assert_eq!(
        state_sequencing_links,
        exclusive_states - 1,
        "SPEC States.sysml:71-77: size(stateSequencing) must equal size(exclusiveStates) - 1"
    );
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn smc_matrix_summary() {
    let src = include_str!("state_machine_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();

    let conforms = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let diverges = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: DIVERGES"))
        .count();
    let unimpl = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    let total = verdicts.len();

    println!("== SMC state-machine behavioral spec-conformance matrix (2026-06) ==");
    println!("CONFORMS:      {conforms}");
    println!("DIVERGES:      {diverges}");
    println!("UNIMPLEMENTED: {unimpl}");
    println!("TOTAL CASES:   {total}");
    println!(
        "conformance ratio: {conforms}/{total} ({:.0}%)",
        100.0 * conforms as f64 / total.max(1) as f64
    );
    for line in &verdicts {
        println!("  {line}");
    }

    assert_eq!(
        conforms + diverges + unimpl,
        total,
        "every verdict marker must be CONFORMS, DIVERGES or UNIMPLEMENTED"
    );
    assert!(
        total >= 7,
        "expected ≥7 verdict-marked behavioral SM gates, got {total}"
    );
    // Pinned matrix — update alongside the tracker when a fix-wave flips a
    // verdict. GAP-SM-EXEC + GAP-SM-EFFECT fix-wave (2026-06-21) CLOSED all three
    // firing-execution DIVERGES (SMC-4/6/7 → CONFORMS): compile_state walks the
    // tagged exit/do children to a Structured body like entry; the grammar's
    // effect_action accepts `do action {…}` so the transition effect attaches and
    // executes; and compile_action_from_children sorts statement children by
    // source span (children_of iterates an unordered set), so exit→effect→entry
    // run in order with correct assignment sequencing. SMC-10
    // (incomingTransitionTrigger recorded on entry) was CLOSED 2026-06-22
    // (SubsystemState.incoming_transition_trigger). SMC-11 (stateSequencing count
    // invariant) was CLOSED 2026-06-22: elaboration (`derive_state_sequencing`)
    // materialises the N-1 `stateSequencing` successions between a state's
    // exclusive substates (States.sysml:71-77). All SM behavioral obligations now
    // CONFORM.
    assert_eq!(conforms, 11, "CONFORMS count changed — update tracker matrix");
    assert_eq!(diverges, 0, "DIVERGES count changed — update tracker matrix");
    assert_eq!(unimpl, 0, "UNIMPLEMENTED count changed — update tracker matrix");
}

// ===========================================================================
// Harness-construction guard (WP1) — pins the fix that keeps these gates real.
// ===========================================================================

/// The six firing-execution gates were red because the harness built
/// orchestrators via a raw `Orchestrator::new` + `add_state_machine`, which has
/// no compiled write set — since slot-routed writeback became unconditional
/// (commit `454591c1`) that path silently dropped every assignment-action
/// output and counters stayed `0.0`. This self-scan pins the repair: the
/// harness must build through the canonical `ModelCompiler::build_sm_orchestrator`
/// (which mints the slot table + binds expressions), never the raw path.
///
/// The raw-path needles are assembled at runtime from parts so this guard does
/// not match its own source text via `include_str!`.
#[test]
fn harness_builds_through_compiler_minted_orchestrator() {
    let src = include_str!("state_machine_spec_conformance.rs");
    assert!(
        src.contains("build_sm_orchestrator("),
        "harness must construct orchestrators through ModelCompiler::build_sm_orchestrator"
    );
    let raw_ctor = format!("{}::{}(", "Orchestrator", "new");
    assert!(
        !src.contains(&raw_ctor),
        "no raw Orchestrator::new — slot-routed writeback needs the compiler-minted slot table (WP1)"
    );
    let raw_reg = format!(".{}(", "add_state_machine");
    assert!(
        !src.contains(&raw_reg),
        "no raw add_state_machine — use ModelCompiler::build_sm_orchestrator (WP1)"
    );
}
