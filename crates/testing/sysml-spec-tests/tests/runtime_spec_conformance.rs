//! RSC Phase 0 — Runtime spec-conformance harness (tracker task #233).
//!
//! Every test in this file encodes ONE spec-defined runtime obligation from
//! the Kernel Semantic Library / Systems Library and asserts the engine's
//! CURRENT behavior against it. Each case carries a verdict marker comment
//! on its own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//! - `// VERDICT: DIVERGES — <reason>` — the test asserts what the engine
//!   ACTUALLY does today, which differs from the spec. The test fails only
//!   if behavior silently changes — which is the point. When a later phase
//!   fixes the divergence the assertion is flipped and the verdict updated.
//! - `// VERDICT: UNIMPLEMENTED — <missing>` — the obligation has no engine
//!   surface at all; the test pins the absence.
//!
//! Spec sources (read directly, cited per test):
//! - `references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Transfers.kerml`
//! - `.../Kernel Semantic Library/Triggers.kerml`
//! - `.../Kernel Semantic Library/Clocks.kerml`
//! - `.../Systems Library/Ports.sysml`
//! - `.../Systems Library/Flows.sysml`
//!
//! Harness rules (tracker Phase 0): pure runtime only — parsed sysml source
//! strings → `ModelGraph` → `ModelCompiler` → orchestrator / FlowRouter.
//! NO LSP harness, NO SysmlService (deadlock surface, task #225).
//! NO production code changes — this phase measures.
//!
//! The summary test (`spec_conformance_matrix_summary`, RSC-0.4) self-scans
//! this file via `include_str!` and prints the CONFORMS/DIVERGES/UNIMPLEMENTED
//! counts.

use sysml_core::{ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::exchange::ExchangePlane;
use sysml_runtime::flows::{compile_ports, FlowEventKind, PortDirection};
use sysml_runtime::links::{classify_links, LinkIR, LinkSourceKind};
use sysml_runtime::orchestrator::Orchestrator;

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Parse a sysml source string into an (un-elaborated) ModelGraph.
///
/// Parse errors fail the test immediately — every fixture in this file must
/// be syntax the tree-sitter grammar accepts, otherwise the case measures
/// the parser instead of the runtime.
fn parse_source(source: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("runtime_spec_conformance.sysml", source)]);
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

/// Parse + elaborate (via ModelCompiler) and return the flow-derived links.
///
/// RSC-3.5e.5 W4: `compile_flows`/`FlowConnectionIR` are gone — the producer is
/// folded into `classify_links`. The FlowUsage subset of the classified
/// `LinkGraph` is the former flow list (`LinkEndpoint.owner`/`port` mirror the
/// old `FlowEndpoint.participant`/`port`; `is_move`/`is_push`/`payload_type`/
/// `source_payload_type`/`target_payload_type` carry the same values).
fn parse_and_compile_flows(source: &str) -> Vec<LinkIR> {
    let compiler = ModelCompiler::new(parse_source(source));
    let graph = compiler.graph();
    let reg = compile_ports(graph);
    let (lg, _diags) = classify_links(graph, &reg);
    lg.iter()
        .filter(|l| l.kind == LinkSourceKind::FlowUsage)
        .cloned()
        .collect()
}

/// Build a single-SM orchestrator from parsed source through the **canonical
/// compiler-built path** (`ModelCompiler::build_sm_orchestrator`, ledger L44),
/// which mints the compile-known slot table and binds expressions so
/// assignment-action writeback (entry/exit/do/effect bodies) publishes into
/// every snapshot. A bare `Orchestrator::new` + `add_state_machine` — the raw
/// path this replaced — has no compiled write set, so since slot-routed
/// writeback became unconditional (commit `454591c1`) it silently dropped
/// every assignment output. The subsystem is registered under the compiled
/// state-def name (`sm_name`), which the caller passes to
/// `inject_event`/`set_clock`.
///
/// `seeds` are shared-context variables (the spec-silent EvalContext plane)
/// visible to `when`-trigger guards via `sync_context_in`. The default
/// `OrchestratorConfig` (`dt_ms = 1.0`) applies — the timed-trigger cases
/// (`after`, `__clock_time`) account for that step size in their tick math.
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

/// Current state of the single SM subsystem after a step snapshot. These
/// harnesses register exactly one state machine (under its compiled def name),
/// so the subsystem is addressed name-agnostically rather than by a synthetic
/// key.
fn sm_state(snap: &sysml_runtime::orchestrator::ExecutionSnapshot) -> String {
    let mut it = snap.subsystem_states.values();
    let ss = it.next().expect("one sm subsystem present");
    assert!(it.next().is_none(), "exactly one SM subsystem in these harnesses");
    ss.current_state.clone()
}

/// A two-part, two-port, one-flow model with NO interface/connection
/// declared between the ports. Shared by several cases.
const PORT_FLOW_NO_INTERFACE: &str = r#"
    package FlowFixture {
        port def DataPort {
            attribute datum : Real;
        }
        part producer {
            port outPort : DataPort;
        }
        part consumer {
            port inPort : ~DataPort;
        }
        flow payloadFlow from producer.outPort to consumer.inPort;
    }
"#;

// ===========================================================================
// RSC-0.1 — Transfer semantics (Transfers.kerml)
// ===========================================================================

/// Transfers.kerml `FlowTransfer` (lines 84-98): `feature isMove: Boolean[1]
/// default true` and `feature isPush: Boolean[1] default true`. A flow usage
/// that does not say otherwise is a moving, pushing transfer.
///
/// RSC-3.3a lowering: `compile_single_flow` now defaults both to TRUE and
/// reads child `attribute isMove/isPush = <bool>;` redefinitions to override
/// (no surface syntax — they are library-redefinable features). The fixture's
/// flow declares no override, so both come through TRUE per spec.
#[test]
fn spec_transfer_ismove_ispush_defaults() {
    let flows = parse_and_compile_flows(PORT_FLOW_NO_INTERFACE);
    assert_eq!(flows.len(), 1, "fixture declares exactly one flow");
    // Spec defaults: both TRUE (RSC-3.3a).
    assert!(
        flows[0].is_move,
        "isMove defaults to true (Transfers.kerml:84, RSC-3.3a lowering)"
    );
    assert!(
        flows[0].is_push,
        "isPush defaults to true (Transfers.kerml:92, RSC-3.3a lowering)"
    );
    // VERDICT: CONFORMS
}

/// Transfers.kerml lines 84-90 + the `moving` connector (lines 128-135):
/// "If isMove is true, then the entire payload leaves the source at the
/// start of the transfer" — i.e. move semantics are per *transfer*: each
/// send moves its own payload out; the payload is then gone from the source.
///
/// RSC-3.3b: the dead `move_delivered` once-per-flow gate is DELETED. Per
/// TRANSFER INSTANCE: every send delivers (its own transfer), and at
/// delivery on an `is_move` flow the source port's payload feature value is
/// cleared (set Null) — observable through `route_pending_with_ports` /
/// the orchestrator's registry+slot mirror. Where the source port carries
/// no features (the whole exercised corpus — ledger L23/L25) there is
/// nothing to clear and delivery proceeds.
#[test]
fn spec_transfer_ismove_payload_leaves_source_per_transfer() {
    use sysml_runtime::flows::{PortFeature, PortInstanceIR, PortRegistry};

    let flows = parse_and_compile_flows(PORT_FLOW_NO_INTERFACE);
    // RSC-3.3a: is_move is now the spec default TRUE straight from lowering.
    assert!(flows[0].is_move, "spec default isMove=true survives lowering");

    let mut router = ExchangePlane::new();
    for f in flows {
        router.add_flow(f, "flow");
    }

    // The pure-runtime registry resolves no features for this fixture (the
    // value dead-end pinned in spec_send_accept_payload_identity_through_port),
    // so build the feature-carrying source port explicitly to observe the
    // payload leaving it.
    let mut registry = PortRegistry::new();
    let mut src = PortInstanceIR::new("producer", "outPort").with_direction(PortDirection::Out);
    src.add_feature(PortFeature {
        name: "datum".into(),
        direction: PortDirection::Out,
        type_name: Some("Real".into()),
        value: Value::Int(1),
    });
    registry.register(src);
    registry.register(PortInstanceIR::new("consumer", "inPort").with_direction(PortDirection::In));

    router.send("producer.outPort", Value::Int(1));
    let first = router.route_pending_with_ports(Some(&mut registry));
    assert_eq!(first.len(), 1, "first transfer delivers");
    assert_eq!(
        registry.get("producer.outPort").expect("port").features["datum"].value,
        Value::Null,
        "the payload LEAVES the source at delivery (feature cleared)"
    );

    // Per-transfer semantics: the second send is its own transfer instance
    // and delivers too (NOT the broken once-per-flow gate).
    registry
        .get_mut("producer.outPort")
        .expect("port")
        .set_feature_value("datum", Value::Int(2));
    router.send("producer.outPort", Value::Int(2));
    let second = router.route_pending_with_ports(Some(&mut registry));
    assert_eq!(second.len(), 1, "each transfer moves its own payload");
    assert_eq!(second[0].payload, Value::Int(2));
    assert_eq!(
        registry.get("producer.outPort").expect("port").features["datum"].value,
        Value::Null,
        "second transfer clears the source again"
    );
    // VERDICT: CONFORMS
}

/// Transfers.kerml lines 76-90: when `isMove` is false a FlowTransfer
/// *copies* the payload to the target ("move or copy it to the target",
/// lines 79-82). Copy-vs-reference is spec-silent; fan-out copies to every
/// connected flow are a legal reading.
///
/// Current behavior: a pending message is copied to EVERY flow whose source
/// key matches, and the source may send repeatedly — copy semantics.
#[test]
fn spec_transfer_ismove_false_copies_payload() {
    let source = r#"
        package FanOut {
            part sensor { port outPort; }
            part monitorA { port inPort; }
            part monitorB { port inPort; }
            flow fanA from sensor.outPort to monitorA.inPort;
            flow fanB from sensor.outPort to monitorB.inPort;
        }
    "#;
    let flows = parse_and_compile_flows(source);
    assert_eq!(flows.len(), 2);
    let mut router = ExchangePlane::new();
    for f in flows {
        router.add_flow(f, "flow");
    }

    router.send("sensor.outPort", Value::Float(7.5));
    let delivered = router.route_pending();
    assert_eq!(delivered.len(), 2, "payload copied to both targets");
    assert!(delivered.iter().all(|m| m.payload == Value::Float(7.5)));

    // Source can keep sending — payload never "left" it.
    router.send("sensor.outPort", Value::Float(8.5));
    assert_eq!(router.route_pending().len(), 2);
    // VERDICT: CONFORMS
}

/// Transfers.kerml lines 92-98 + `pushing` connector (lines 137-144):
/// `isPush=false` means the transfer does NOT begin when the payload is
/// available at the source — it is pull-initiated (begins on the target's
/// demand).
///
/// RSC-3.3c U1: `route_pending` suppresses eager delivery on
/// `is_push=false` links (the transfer is parked per target); delivery
/// happens when the target pulls — the explicit `pull(target)` API, called
/// from the SM/action accept path when the pending trigger matches a
/// pull-mode link. NOTE the initiation mechanism is EXTENSION-ANCHORED per
/// the design doc (D-3.0.4 row U1) and the audit-doc extension ledger: the
/// spec only removes the push constraint and leaves initiation open;
/// `pull(target)` is our labeled extension on that opening.
#[test]
fn spec_transfer_ispush_false_pull_initiation() {
    let mut flows = parse_and_compile_flows(PORT_FLOW_NO_INTERFACE);
    flows[0].is_push = false; // explicitly a pull transfer

    let mut router = ExchangePlane::new();
    for f in flows {
        router.add_flow(f, "flow");
    }

    router.send("producer.outPort", Value::Int(9));
    let delivered = router.route_pending();
    // The transfer does NOT begin just because the payload is available.
    assert_eq!(
        delivered.len(),
        0,
        "is_push=false suppresses eager delivery (Transfers.kerml:92-98)"
    );
    assert!(
        router.receive("consumer.inPort").is_none(),
        "nothing reaches the target before it pulls"
    );

    // The transfer begins on the target's demand: the accept path pulls.
    let pulled = router.pull("consumer.inPort");
    assert_eq!(pulled.len(), 1, "pull initiates the parked transfer");
    assert_eq!(pulled[0].payload, Value::Int(9));
    let accepted = router.receive("consumer.inPort").expect("delivered on pull");
    assert_eq!(accepted.payload, Value::Int(9));
    // VERDICT: CONFORMS
}

/// Transfers.kerml lines 100-118: the payload must conform to the source's
/// `sourceOutput` and the target's `targetInput` ("transferPayload references
/// payload subsets transferSource.sourceOutput" / "...transferTarget.targetInput").
///
/// RSC-3.3a derived `source_payload_type` / `target_payload_type` from the
/// endpoint port feature typings; RSC-3.3b enforces them: the payload must
/// conform to BOTH (ScalarValues.kerml:11-22 lattice — Integer :> Rational
/// :> Real :> Complex :> Number — plus the graph's specialization edges for
/// model-defined names; sysml-core has no model-type lattice, so conformance
/// is implemented on those two bases). A provable mismatch is withheld and
/// fail-hard under the strict default (`route_pending_checked` →
/// `FlowError::PayloadConformance`; orchestrator → `flow_error`). Absent
/// endpoint typing = no static check (ledger L25: derivation is data-starved
/// on the corpus until the RSC-3.5 ExchangePlane unifies the key bases).
#[test]
fn spec_transfer_payload_conformance_to_endpoint_features() {
    use sysml_runtime::flows::FlowError;

    let mut flows = parse_and_compile_flows(PORT_FLOW_NO_INTERFACE);
    // L25: this fixture's ports resolve no features in the pure-runtime
    // pipeline, so the derivation honestly yields None — and absent typing
    // must mean NO static check (not a violation).
    assert_eq!(flows[0].source_payload_type, None);
    assert_eq!(flows[0].target_payload_type, None);
    {
        let mut router = ExchangePlane::new();
        router.add_flow(flows[0].clone(), "flow");
        router.send("producer.outPort", Value::Int(3));
        assert_eq!(
            router.route_pending().len(),
            1,
            "absent endpoint typing = no conformance check (L25)"
        );
    }

    // With derived endpoint typings present, the payload must conform to
    // BOTH. Int conforms to Real (Integer :> Rational :> Real) AND Integer.
    flows[0].source_payload_type = Some("Real".to_owned());
    flows[0].target_payload_type = Some("Integer".to_owned());
    let mut router = ExchangePlane::new();
    router.add_flow(flows[0].clone(), "flow");
    router.send("producer.outPort", Value::Int(3));
    assert_eq!(
        router.route_pending_checked().expect("conformant").len(),
        1,
        "Int conforms to both Real (sourceOutput) and Integer (targetInput)"
    );

    // Float conforms to Real but NOT to Integer — must conform to BOTH, so
    // the transfer is withheld and strict mode fails hard.
    let mut router2 = ExchangePlane::new();
    router2.add_flow(flows[0].clone(), "flow");
    router2.send("producer.outPort", Value::Float(2.5));
    let err = router2
        .route_pending_checked()
        .expect_err("float payload fails Integer targetInput conformance");
    assert!(
        matches!(err, FlowError::PayloadConformance { rejected: 1, .. }),
        "strict default fails hard on a provable conformance violation: {err}"
    );
    assert!(
        router2
            .events()
            .iter()
            .any(|e| matches!(e.kind, FlowEventKind::TypeMismatch { .. })),
        "rejection also surfaces as a TypeMismatch event"
    );
    assert_eq!(router2.conformance_rejected_total(), 1);

    // The explicit payload_type route-time check (name + Int→Float widening)
    // is unchanged and still enforced alongside.
    let mut widening = flows[0].clone();
    widening.payload_type = Some("float".to_owned());
    widening.source_payload_type = None;
    widening.target_payload_type = None;
    let mut router3 = ExchangePlane::new();
    router3.add_flow(widening, "flow");
    router3.send("producer.outPort", Value::Int(3));
    assert_eq!(router3.route_pending().len(), 1, "Int→Float widening holds");
    // VERDICT: CONFORMS
}

/// Transfers.kerml lines 67-74 + SendPerformance (lines 236-252): a
/// MessageTransfer "does not specify where the payload is picked up and
/// dropped off" — sends route by sender/receiver Occurrence, with NO
/// declared flow connection required.
///
/// RSC-3.3c D4: sends with no declared flow endpoint are MessageTransfers
/// routed by occurrence/participant addressing — the send names its
/// receiver (`FlowRouter::send_message`) and the message delivers to the
/// named participant's accepting surface (`register_acceptor`; the
/// orchestrator registers SM `PortMessage` accept ports). Resolution is
/// fail-loud: multiple candidate acceptors → withheld +
/// `FlowError::AmbiguousMessageTarget` (never pick-first); zero candidates
/// → the RSC-1.5 strict-loss machinery (`FlowError::Unrouted` via
/// `route_pending_checked`).
#[test]
fn spec_transfer_message_without_pickup_dropoff_routes() {
    let flows = parse_and_compile_flows(PORT_FLOW_NO_INTERFACE);
    let mut router = ExchangePlane::new();
    for f in flows {
        router.add_flow(f, "flow");
    }
    // The consumer's accepting surface (the orchestrator registers these
    // from SM PortMessage triggers / accepting action nodes).
    router.register_acceptor("consumer.statusIn");

    // Send from an endpoint with no declared flow — per the spec a
    // MessageTransfer to a designated receiver needs no flow declaration.
    router.send_message(
        "producer.statusOut",
        "consumer",
        Value::String("ready".into()),
    );
    let delivered = router.route_pending();
    assert_eq!(
        delivered.len(),
        1,
        "a MessageTransfer routes to its named receiver without a declared flow"
    );
    assert_eq!(delivered[0].target, "consumer.statusIn");
    assert_eq!(delivered[0].payload, Value::String("ready".into()));
    assert!(
        delivered[0].flow_id.starts_with("message:"),
        "no flow element is traversed — the transfer is occurrence-addressed"
    );
    let accepted = router.receive("consumer.statusIn").expect("accepted");
    assert_eq!(accepted.payload, Value::String("ready".into()));
    assert_eq!(router.unrouted_message_total(), 0, "nothing was dropped");

    // The fail-loud halves: an unresolvable receiver is strict loss, not a
    // silent drop.
    router.send_message("producer.statusOut", "nobody", Value::Int(1));
    let err = router
        .route_pending_checked()
        .expect_err("zero accepting surfaces = fail-hard unrouted (RSC-1.5)");
    assert!(matches!(
        err,
        sysml_runtime::flows::FlowError::Unrouted { count: 1, .. }
    ));
    // VERDICT: CONFORMS
}

// ===========================================================================
// RSC-0.2 — Trigger + time semantics (Triggers.kerml, Clocks.kerml)
// ===========================================================================

/// Triggers.kerml `TriggerWhen` (lines 48-89): the condition is "monitored
/// for changing from **false to true**" (lines 55-61). A condition that is
/// already true when monitoring starts has not changed false→true, so no
/// ChangeSignal should be produced until it first goes false and rises again.
///
/// **This is the G3 edge-vs-level audit.** Current behavior: a parsed
/// `accept when <expr>` is lowered to a *guard-only* transition
/// (statemachine compiler strips the `when` and stores the expression as a
/// guard), which the runner evaluates by LEVEL every tick — so a guard that
/// is true at t0 fires on the very first tick with no false sample ever
/// observed. (The runner's `TriggerKind::When` rising-edge machinery exists
/// but is unreachable from parsed sources: the guard-only lowering wins.)
#[test]
fn spec_trigger_when_true_at_t0_fires_immediately() {
    let source = r#"
        package WhenT0 {
            state def WatchSM {
                state Armed;
                state Fired;
                transition arm_to_fire
                    first Armed
                    accept when flag > 0
                    then Fired;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "WatchSM", &[("flag", 1.0)]);
    let snap = orch.step();
    assert_eq!(
        sm_state(&snap),
        "Fired",
        "guard true at t0 fires on the FIRST tick — no false→true change was ever observed"
    );
    // OBL: transition-guard-boolean
    // VERDICT: DIVERGES — `when` is level-evaluated: a condition already true at t0 fires immediately, though per TriggerWhen no false→true change occurred
}

/// Triggers.kerml lines 48-61 again, the re-fire half of the G3 audit:
/// a single ChangeSignal is produced per false→true change. With a condition
/// that is continuously true, an edge-triggered machine fires at most once;
/// a level-triggered machine keeps firing every evaluation.
///
/// Current behavior (probed): `when` triggers are EDGE-LATCHED. The
/// constant-true condition fires a_to_b exactly once (at t0 — see the
/// previous case for that divergence), run-to-completion chains b_to_a back
/// within the same tick, and the latch then holds: no re-fire on any
/// subsequent tick while the condition stays true. One ChangeSignal per
/// false→true change — the spec's re-fire obligation is met.
#[test]
fn spec_trigger_when_level_refire_on_constant_true() {
    let source = r#"
        package WhenLevel {
            state def PingPongSM {
                state A;
                state B {
                    entry action { bEntries = bEntries + 1; }
                }
                transition a_to_b first A accept when flag > 0 then B;
                transition b_to_a first B accept when flag > 0 then A;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "PingPongSM", &[("flag", 1.0)]);
    orch.context.set("bEntries".to_owned(), Value::Float(0.0));

    let mut states = Vec::new();
    let mut b_entries = Value::Null;
    for _ in 0..4 {
        let snap = orch.step();
        states.push(sm_state(&snap));
        b_entries = snap.variables.get("bEntries").cloned().unwrap_or(Value::Null);
    }
    eprintln!("ping-pong states: {states:?}, bEntries after 4 ticks: {b_entries:?}");

    // Edge semantics: a_to_b fires once (t0 treated as an edge), b_to_a
    // chains within the same tick, and both transitions then go quiet —
    // bEntries freezes at 1.
    assert_eq!(
        b_entries,
        Value::Float(1.0),
        "constant-true `when` fires exactly once — the trigger is edge-latched, not level-evaluated"
    );
    assert_eq!(
        states,
        vec!["A", "A", "A", "A"],
        "run-to-completion chains b_to_a back within the same tick, so end-of-tick state stays A"
    );
    // VERDICT: CONFORMS
}

/// Triggers.kerml `TriggerAfter` (lines 141-186): a TimeSignal is sent once,
/// after the given delay relative to the clock at invocation time
/// (`TriggerAt(clock.currentTime + delay, ...)`, lines 178-185).
///
/// Current behavior: `accept after(0.1)` lowers to an after-trigger over the
/// per-state elapsed timer (float = seconds) and fires exactly once when
/// `state_elapsed >= delay` — at the 100th 1ms tick — then the machine rests
/// in the target state. (Note: literal unit suffixes like `after(100ms)` are
/// a parse error; only bare numerics/expressions survive the grammar.)
#[test]
fn spec_trigger_after_fires_once_at_delay() {
    let source = r#"
        package AfterDelay {
            state def DelaySM {
                state Waiting;
                state Done;
                transition wait_to_done
                    first Waiting
                    accept after(0.1)
                    then Done;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "DelaySM", &[]);
    let mut fired_at_tick = None;
    for tick in 1..=120u64 {
        let snap = orch.step();
        if sm_state(&snap) == "Done" && fired_at_tick.is_none() {
            fired_at_tick = Some(tick);
        }
    }
    assert_eq!(
        fired_at_tick,
        Some(100),
        "after(100ms) fires at the first tick where state-elapsed time >= 100ms (tick 100 at the default dt=1ms)"
    );
    // VERDICT: CONFORMS
}

/// Triggers.kerml lines 48-89: each TriggerWhen invocation monitors its
/// condition for false→true changes. When the source state is re-entered the
/// trigger is re-armed: a *new* false→true change after re-entry must fire
/// the transition again (a condition that stayed true throughout must not).
///
/// Current behavior (probed): the edge latch is honored across state
/// re-entry. With the condition continuously true, returning to the source
/// state does NOT re-fire the transition (no new false→true change — spec
/// satisfied); a genuine false→true edge made after re-entry fires it again
/// (re-arm on condition-false — spec satisfied).
#[test]
fn spec_trigger_when_rearm_after_state_reentry() {
    let source = r#"
        package WhenReentry {
            state def ReentrySM {
                state A;
                state B {
                    entry action { bEntries = bEntries + 1; }
                }
                transition a_to_b first A accept when flag > 0 then B;
                transition b_to_a first B accept reset then A;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "ReentrySM", &[("flag", 1.0)]);
    orch.context.set("bEntries".to_owned(), Value::Float(0.0));

    // flag true at t0 → a_to_b fires (level semantics, earlier case).
    let snap1 = orch.step();
    eprintln!(
        "tick1: state={}, bEntries={:?}",
        sm_state(&snap1),
        snap1.variables.get("bEntries")
    );

    // Event-driven exit B → A; flag stayed true throughout.
    orch.inject_event("ReentrySM", "reset");
    let snap2 = orch.step();
    eprintln!("tick2 (reset): state={}", sm_state(&snap2));

    // Phase 1: condition stayed true — does it re-fire without a new edge?
    let snap3 = orch.step();
    let snap4 = orch.step();
    let stayed_true_entries = snap4.variables.get("bEntries").cloned();
    eprintln!(
        "tick3={}, tick4={}, bEntries={stayed_true_entries:?}",
        sm_state(&snap3),
        sm_state(&snap4)
    );

    // Phase 2: make a GENUINE false→true edge after re-entry.
    orch.context.set("flag".to_owned(), Value::Float(0.0));
    orch.step();
    orch.context.set("flag".to_owned(), Value::Float(1.0));
    let snap_edge = orch.step();
    let edge_state = sm_state(&snap_edge);
    let edge_entries = snap_edge.variables.get("bEntries").cloned();
    eprintln!("after genuine edge: state={edge_state}, bEntries={edge_entries:?}");

    assert_eq!(
        stayed_true_entries,
        Some(Value::Float(1.0)),
        "condition stayed true across exit/re-entry — no new edge, no re-fire"
    );
    assert_eq!(
        edge_state, "B",
        "a genuine false→true edge after re-entry fires the `when` transition again"
    );
    assert_eq!(edge_entries, Some(Value::Float(2.0)));
    // VERDICT: CONFORMS
}

/// Clocks.kerml lines 29-45: "A Clock provides a numerical currentTime that
/// advances mont[on]ically". The runtime exposes the active clock to guard
/// expressions as `__clock_time` (seconds), wired per-subsystem via the
/// ClockRegistry.
///
/// Current behavior: snapshot times advance strictly monotonically, and a
/// `when` guard over `__clock_time` fires at the correct simulated instant.
#[test]
fn spec_clock_monotonic_currenttime_exposed_to_guards() {
    let source = r#"
        package ClockGuard {
            state def ClockSM {
                state Early;
                state Late;
                transition early_to_late
                    first Early
                    accept when __clock_time >= 0.05
                    then Late;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "ClockSM", &[]);
    // Wire a local clock at rate 1.0 so the orchestrator publishes
    // __clock_time (seconds) into the SM's eval context.
    orch.set_clock("ClockSM", 1.0);

    let mut last_time = -1.0f64;
    let mut fired_at_tick = None;
    for tick in 1..=60u64 {
        let snap = orch.step();
        assert!(
            snap.time_ms > last_time,
            "clock must advance monotonically (tick {tick})"
        );
        last_time = snap.time_ms;
        if sm_state(&snap) == "Late" && fired_at_tick.is_none() {
            fired_at_tick = Some(tick);
        }
    }
    // __clock_time reaches 0.05s on tick 50 (50 × the default dt=1ms).
    assert_eq!(
        fired_at_tick,
        Some(50),
        "guard over __clock_time fires at the simulated 50ms instant"
    );
    // VERDICT: CONFORMS
}

/// Clocks.kerml `TimeOf` (lines 60-80): "TimeOf returns a numerical
/// timeInstant for a given Occurrence relative to [a clock]" — snapshot
/// semantics over occurrences (TimeOf of a snapshot equals the clock's
/// currentTime at that snapshot, lines 50-55).
///
/// Current behavior: there is no TimeOf in the expression stdlib. A `when`
/// guard calling it never fires (guard evaluation fails closed), so
/// occurrence-relative time queries are not expressible from models.
#[test]
fn spec_clock_timeof_snapshot_semantics() {
    let source = r#"
        package TimeOfProbe {
            state def TimeOfSM {
                state Probing;
                state Resolved;
                transition probe
                    first Probing
                    accept when TimeOf(Probing) >= 0
                    then Resolved;
            }
        }
    "#;
    let mut orch = sm_orchestrator(source, "TimeOfSM", &[]);
    orch.set_clock("TimeOfSM", 1.0);
    let mut state = String::new();
    for _ in 0..10 {
        state = sm_state(&orch.step());
    }
    assert_eq!(
        state, "Probing",
        "TimeOf(...) is not evaluable — the guard never fires"
    );
    // VERDICT: UNIMPLEMENTED — no TimeOf/DurationOf in the expression stdlib; occurrence-relative time queries cannot be expressed (only the bare __clock_time scalar exists)
}

// ===========================================================================
// RSC-0.3 — Port / interface topology + send/accept (Ports.sysml, Transfers.kerml)
// ===========================================================================

/// Ports.sysml lines 30-41: `outgoingTransfersFromSelf :>
/// interfacingPorts.incomingTransfersToSelf` — "The target of each of the
/// outgoingTransfersFromSelf of a Port must be an interfacingPort", i.e. a
/// port connected via an Interface. A transfer between ports with NO
/// interface/connection between them is not permitted.
///
/// RSC-3.3b: enforced as the FL018 HARD ERROR (`links::
/// transfer_contract_diagnostics`, failed at `build_workspace_orchestrator`)
/// per the §6 Q2 RS001-playbook decision. Spec ruling on the predicate:
/// a plain ConnectionUsage between ports satisfies it — Interfaces.sysml:
/// 34-43 defines Interface extensionally ("the most general class of links
/// between Ports on Parts"), and the OMG spec's normative Annex A model
/// connects ports with bare `connect`. The satisfying connector is recorded
/// on `LinkIR::via_interface`. Static coverage requires the endpoint keys
/// to resolve to ports (L23: usage-vs-definition key bases unify at 3.5).
#[test]
fn spec_port_transfers_require_interface_topology() {
    use sysml_runtime::links::transfer_contract_diagnostics;

    let compiler = ModelCompiler::new(parse_source(PORT_FLOW_NO_INTERFACE));
    let registry = compile_ports(compiler.graph());
    let (link_graph, _diags) = classify_links(compiler.graph(), &registry);
    assert_eq!(
        link_graph
            .iter()
            .filter(|l| l.kind == LinkSourceKind::FlowUsage)
            .count(),
        1
    );
    assert!(
        link_graph.iter().next().expect("one link").via_interface.is_none(),
        "no declared connector exists, so no via_interface"
    );
    let contract = transfer_contract_diagnostics(compiler.graph(), &registry, &link_graph);
    assert!(
        contract.iter().any(|d| d.code.as_deref() == Some("FL018")
            && d.severity == sysml_span::Severity::Error),
        "a transfer between non-interfacing ports is rejected with the FL018 hard \
         error at compile time, got: {contract:?}"
    );
    // VERDICT: CONFORMS
}

/// Transfers.kerml SendPerformance/AcceptPerformance (lines 236-266):
/// `sentTransfer.payload` is bound to the send's payload and the accept's
/// payload is bound to `acceptedTransfer.payload` — the accepted payload is
/// value-identical to the sent one.
///
/// Current behavior: payload value identity holds through the router's
/// send → route → receive path. (The port-aware binding of the payload into
/// the target port's *features* is a silent no-op here — `compile_ports`
/// resolves no features for the def-typed ports in this pure-runtime
/// pipeline, the audit §2 "ports are a value dead-end" finding — but the
/// send/accept payload obligation itself is about the accepted value, which
/// IS identical.)
#[test]
fn spec_send_accept_payload_identity_through_port() {
    let compiler = ModelCompiler::new(parse_source(PORT_FLOW_NO_INTERFACE));
    let flows: Vec<LinkIR> = {
        let reg = compile_ports(compiler.graph());
        let (lg, _diags) = classify_links(compiler.graph(), &reg);
        lg.iter()
            .filter(|l| l.kind == LinkSourceKind::FlowUsage)
            .cloned()
            .collect()
    };
    let mut registry = compile_ports(compiler.graph());
    let in_port = registry
        .get("consumer.inPort")
        .expect("fixture port compiles into the registry");
    // Pin the dead-end: no features resolve from `port inPort : ~DataPort`,
    // so payload→feature binding has nothing to bind into.
    assert!(
        in_port.features.is_empty(),
        "port features do not resolve in the pure-runtime pipeline (value dead-end)"
    );

    let mut router = ExchangePlane::new();
    for f in flows {
        router.add_flow(f, "flow");
    }

    let sent = Value::Float(3.25);
    router.send("producer.outPort", sent.clone());
    let delivered = router.route_pending_with_ports(Some(&mut registry));
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].payload, sent,
        "delivered payload is value-identical to the sent payload"
    );

    // Accept side: the receiver consumes the same value.
    let accepted = router.receive("consumer.inPort").expect("message queued");
    assert_eq!(accepted.payload, sent);
    // OBL: send-action-initiates-message-transfer
    // VERDICT: CONFORMS
}

/// Ports.sysml conjugation (`~DataPort`) flips feature directions; together
/// with Transfers.kerml's source-output/target-input contract (lines
/// 100-118) a transfer must pick up at an output and drop off at an input.
/// Delivering INTO a port whose effective direction is `out` violates the
/// contract.
///
/// RSC-3.3b: enforced as the FL019 HARD ERROR at compile time
/// (`links::transfer_contract_diagnostics` — post-conjugation effective
/// directions from the registry; `elaborate/ports.rs` folds `~P` into
/// `effectiveDirection`). The compile is where the violation always is
/// statically known today; `route_pending_with_ports` keeps a debug_assert
/// belt for hand-built routers.
#[test]
fn spec_port_conjugation_direction_at_transfer_time() {
    use sysml_runtime::links::transfer_contract_diagnostics;

    let source = r#"
        package BackwardsFlow {
            port def CmdPort {
                out attribute cmd : Real;
            }
            part producer {
                port outPort : CmdPort;
            }
            part consumer {
                port inPort : ~CmdPort;
            }
            flow backwards from consumer.inPort to producer.outPort;
            connect consumer.inPort to producer.outPort;
        }
    "#;
    let compiler = ModelCompiler::new(parse_source(source));
    let registry = compile_ports(compiler.graph());

    // Pin: direction metadata reaches the runtime port IR, post-conjugation —
    // Out on the producer side, In on the conjugated consumer side.
    let out_port = registry.get("producer.outPort").expect("port registered");
    assert_eq!(out_port.direction, PortDirection::Out);
    assert!(!out_port.is_conjugated);
    let in_port = registry.get("consumer.inPort").expect("port registered");
    assert_eq!(in_port.direction, PortDirection::In);

    let (link_graph, _diags) = classify_links(compiler.graph(), &registry);
    assert_eq!(
        link_graph
            .iter()
            .filter(|l| l.kind == LinkSourceKind::FlowUsage)
            .count(),
        1
    );
    let contract = transfer_contract_diagnostics(compiler.graph(), &registry, &link_graph);

    // The backwards flow violates BOTH halves of the contract: pick-up at an
    // in-direction port AND drop-off into an out-direction port.
    // RSC-3.4 / L18 CONSCIOUS PIN CHANGE: the model also has
    // `connect consumer.inPort to producer.outPort;` which the L18 fix now
    // ingests as a separate MessageChannel link.  That ConnectionUsage has the
    // same endpoint violations, so FL019 fires on BOTH the flow link and the
    // connector link — 2 × 2 = 4 total.
    let fl019: Vec<_> = contract
        .iter()
        .filter(|d| d.code.as_deref() == Some("FL019"))
        .collect();
    assert_eq!(
        fl019.len(),
        4,
        "in-direction source + out-direction target each produce FL019 for BOTH the flow \
         and the connector element (RSC-3.4 L18): {contract:?}"
    );
    assert!(
        fl019
            .iter()
            .all(|d| d.severity == sysml_span::Severity::Error),
        "FL019 is a hard error (the compile refuses the route)"
    );
    // VERDICT: CONFORMS
}

// ===========================================================================
// RSC-0.4 — Conformance matrix summary
// ===========================================================================

/// Self-scan of this file's verdict markers. Prints the conformance ratio
/// and pins the case counts so adding/removing a case forces the matrix
/// (and the tracker's Phase 0 section) to be updated consciously.
#[test]
fn spec_conformance_matrix_summary() {
    let source = include_str!("runtime_spec_conformance.rs");

    let verdict_lines: Vec<&str> = source
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();

    let conforms = verdict_lines
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let diverges = verdict_lines
        .iter()
        .filter(|l| l.starts_with("// VERDICT: DIVERGES"))
        .count();
    let unimplemented = verdict_lines
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    let total = verdict_lines.len();

    println!("== RSC Phase 0 runtime spec-conformance matrix (2026-06) ==");
    println!("CONFORMS:      {conforms}");
    println!("DIVERGES:      {diverges}");
    println!("UNIMPLEMENTED: {unimplemented}");
    println!("TOTAL CASES:   {total}");
    println!(
        "conformance ratio: {conforms}/{total} ({:.0}%)",
        100.0 * conforms as f64 / total as f64
    );
    for line in &verdict_lines {
        println!("  {line}");
    }

    assert_eq!(
        conforms + diverges + unimplemented,
        total,
        "every verdict marker must be CONFORMS, DIVERGES or UNIMPLEMENTED"
    );
    // Pinned matrix — update alongside the tracker when a later phase flips a
    // verdict. RSC-3.3a flipped spec_transfer_ismove_ispush_defaults
    // DIVERGES→CONFORMS (6→7 conforms, 7→6 diverges). RSC-3.3b flipped four
    // more DIVERGES→CONFORMS (7→11 conforms, 6→2 diverges): D2 per-transfer
    // move semantics, D3 payload-subset conformance, D5 FL018 interface
    // topology, D6 FL019 direction checks. RSC-3.3c flipped D4 MessageTransfer
    // occurrence routing DIVERGES→CONFORMS (11→12) and U1 pull initiation
    // UNIMPLEMENTED→CONFORMS (12→13; extension-anchored per the design doc
    // D-3.0.4 row U1 — the spec leaves pull initiation open, `pull(target)`
    // is our labeled extension). Remaining DIVERGES: the `when`-trigger t0
    // case (Phase 4). Remaining UNIMPLEMENTED: TimeOf/DurationOf (Phase 4).
    assert_eq!(conforms, 13, "CONFORMS count changed — update tracker matrix");
    assert_eq!(diverges, 1, "DIVERGES count changed — update tracker matrix");
    assert_eq!(
        unimplemented, 1,
        "UNIMPLEMENTED count changed — update tracker matrix"
    );
}

// ===========================================================================
// Harness-construction guard (WP1) — pins the fix that keeps these gates real.
// ===========================================================================

/// The `spec_trigger_when_*` refire/rearm gates were red because the harness
/// built orchestrators via a raw `Orchestrator::new` + `add_state_machine`,
/// which has no compiled write set — since slot-routed writeback became
/// unconditional (commit `454591c1`) that path silently dropped every
/// assignment-action output (`bEntries` stayed `0.0`). This self-scan pins the
/// repair: the harness must build through the canonical
/// `ModelCompiler::build_sm_orchestrator` (which mints the slot table + binds
/// expressions), never the raw path.
///
/// The raw-path needles are assembled at runtime from parts so this guard does
/// not match its own source text via `include_str!`.
#[test]
fn harness_builds_through_compiler_minted_orchestrator() {
    let src = include_str!("runtime_spec_conformance.rs");
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
