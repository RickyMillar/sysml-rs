//! RSC-3.B0 — Runtime Semantic Core Phase 3 behavioural baselines +
//! interface-topology inventory + grammar research.
//!
//! §4, row RSC-3.B0.
//!
//! **Purpose (per the design doc):**
//! 1. Routing parity baselines — pin current routing outcomes (delivery sets,
//!    message counts, port_values, flow_drop_warnings) at ticks {1, 10, 100}
//!    for sim-mega-coffee-machine, orchestration-complex.
//!    These are the BEFORE snapshots; every RSC-3.x phase must keep them
//!    byte-identical (or consciously flip them alongside the phase).
//! 2. Interface-topology inventory — corpus scan: every flow whose endpoint
//!    ports are NOT connected by any declared ConnectionUsage/InterfaceUsage/
//!    BindingConnector. This drives the FL018 severity decision (§5 risk /
//!    §6 Q2 resolved: hard error via RS001 playbook).
//! 3. Intent tests — assert target semantics for D1-D6/U1 from §3 D-3.0.4
//!    of the design doc. Authored #[ignore]'d (describing the DESIRED
//!    state); flipped live alongside the phase that implements each.
//!    ALL LIVE as of RSC-3.3c (D1 → 3.3a; D2/D5/D6 → 3.3b; D4/U1 → 3.3c).
//!
//! ---
//!
//! ## Grammar research: isMove / isPush surface syntax
//!
//! **Researched 2026-06-12 against:**
//! - Transfers.kerml:
//!   `references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/
//!    Kernel Libraries/Kernel Semantic Library/Transfers.kerml`
//!   (lines 84, 92 for isMove/isPush with `default true`)
//! - SysML.xtext:
//!   `references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/
//!    src/org/omg/sysml/xtext/SysML.xtext`
//!   (lines 1264-1285 FlowUsage / SuccessionFlowUsage / FlowDeclaration)
//! - KerML.xtext (same installation):
//!   No `isMove` / `isPush` token found anywhere in the Xtext grammar.
//!
//! **Findings:**
//!
//! `isMove` and `isPush` are library-defined **features** of `FlowTransfer`,
//! not surface keywords in the grammar. They are defined in Transfers.kerml:
//!
//! ```kerml
//! // Transfers.kerml line 84:
//! feature isMove: Boolean[1] default true {
//!     ...
//! }
//! // Transfers.kerml line 92:
//! feature isPush: Boolean[1] default true {
//!     ...
//! }
//! ```
//!
//! **Neither `isMove` nor `isPush` appears anywhere in SysML.xtext or KerML.xtext
//! as a keyword, rule, or terminal token.**  The grammar has no special syntax
//! for overriding these defaults on a per-flow-usage basis at the language level.
//!
//! **What FlowDeclaration (SysML.xtext:1278) allows:**
//! ```xtext
//! fragment FlowDeclaration returns SysML::FlowUsage :
//!     UsageDeclaration? ValuePart?
//!     ( 'of'  ownedRelationship += PayloadFeatureMember )?
//!     ( 'from' ownedRelationship += FlowEndMember
//!       'to' ownedRelationship += FlowEndMember )?
//!   | ownedRelationship += FlowEndMember 'to'
//!     ownedRelationship += FlowEndMember
//! ;
//! ```
//! There is no `'isMove'` or `'isPush'` keyword anywhere in this rule.
//!
//! **Decision for RSC-3.3a (lowering):**
//! `isMove` and `isPush` are redefinable features of `FlowTransfer`. A user
//! CAN override them by redefining them on a concrete FlowTransfer subtype or
//! by placing attribute usages inside a flow def/usage body:
//! ```sysml
//! flow f1 from a.out to b.in {
//!     attribute isMove = false;  // redefines via value
//! }
//! ```
//! Our ast_builder should lower these redefinitions by reading the child
//! attribute with name `"isMove"` / `"isPush"` from the FlowUsage body.
//! Absent such a child, BOTH default to **true** (spec-authoritative per
//! Transfers.kerml:84,:92). The current `compile_single_flow` hardcodes
//! `is_move: false, is_push: false` — both are WRONG per the spec.
//!
//! **Implementation path for 3.3a:**
//! - `compile_single_flow` (flows/mod.rs:1138): read child attribute named
//!   `"isMove"` from the element (or its body members); default true if absent.
//!   Same for `"isPush"`. No grammar changes required (no new keyword syntax).
//! - The spec exposes them only as redefinable features — our lowering reads
//!   redefinitions, not invented syntax. No Xtext changes in-scope for RSC-3.
//!
//! ---
//!
//! Harness rules (same as rsc2_behavioural_baseline.rs):
//! - Pure runtime only: `TreeSitterParser` → `ModelGraph` → `ModelCompiler::
//!   build_workspace_orchestrator`.
//! - NO LSP harness, NO SysmlService (deadlock surface, task #225).
//! - NO production-code changes. RSC-3.B0 measures.
//! - Determinism: UUID/ElementId-free report strings, sorted lists, no wall-clock.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::{context_from_graph, ModelCompiler};
use sysml_runtime::orchestrator::{ExecutionSnapshot, Orchestrator};

// ---------------------------------------------------------------------------
// Harness helpers (mirrored from rsc2_behavioural_baseline.rs)
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

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

fn load_file(rel: &str) -> ModelGraph {
    let path = workspace_root().join(rel);
    assert!(path.exists(), "model file not found: {}", path.display());
    load_graph(&[path])
}

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

// ---------------------------------------------------------------------------
// RSC-3.B0 routing parity report
//
// Extends the RSC-2.B0 parity format with three exchange-plane fields that
// RSC-3.x will change:
//   - per-tick message counts (total messages routed this tick)
//   - sorted (flow_id, source, target) triples (deterministic message topology)
//   - flow_drop_warnings (expect empty on all well-formed corpus models)
//   - port_values key structure (owner.port → sorted feature names)
//
// Note on routing-backend unrouted/dropped counters:
//   The `Orchestrator` exposes its routing backend (the `ExchangePlane` since
//   RSC-3.5e.2, which replaced `FlowRouter`) only through the
//   `set_exchange_plane` mutator and the per-tick `flow_drop_warnings` on
//   snapshots. The cumulative `unrouted_total` and `dropped_total` counters are
//   private to the plane field (they become first-class snapshot fields in a
//   later wave). For RSC-3.B0 we pin what IS accessible: the per-tick
//   `flow_drop_warnings` vec.
// ---------------------------------------------------------------------------

fn render_exchange_tick(out: &mut String, snap: &ExecutionSnapshot, label: &str) {
    use std::fmt::Write;

    writeln!(out, "== {label} tick {} ==", snap.tick).unwrap();

    // Message count this tick.
    writeln!(out, "message-count: {}", snap.messages.len()).unwrap();

    // Sorted (flow_id, source, target) triples — deterministic topology pin.
    let mut triples: Vec<String> = snap
        .messages
        .iter()
        .map(|m| format!("({}, {}, {})", m.flow_id, m.source, m.target))
        .collect();
    triples.sort();
    triples.dedup();
    for t in &triples {
        writeln!(out, "flow-triple: {t}").unwrap();
    }

    // Port values key structure: owner.port → sorted feature names (no values;
    // values vary numerically and would make the baseline brittle).
    let mut port_keys: Vec<String> = snap.port_values.keys().cloned().collect();
    port_keys.sort();
    for pk in &port_keys {
        let features = snap.port_values.get(pk).unwrap();
        let mut fnames: Vec<&String> = features.keys().collect();
        fnames.sort();
        writeln!(out, "port-key: {pk} -> [{}]", fnames.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")).unwrap();
    }

    // flow_drop_warnings: expect empty on healthy models.
    writeln!(out, "flow-drop-warnings: {}", snap.flow_drop_warnings.len()).unwrap();
    for w in &snap.flow_drop_warnings {
        writeln!(out, "  drop: {w}").unwrap();
    }
}

/// Step to ticks 1, 10, 100 and render the RSC-3 routing parity report.
fn exchange_parity_report(label: &str, orch: &mut Orchestrator) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    writeln!(out, "RSC-3.B0 exchange parity baseline :: {label}").unwrap();

    let mut names = orch.subsystem_names();
    names.sort();
    writeln!(out, "subsystems ({}):", names.len()).unwrap();
    for n in &names {
        writeln!(out, "  {n}").unwrap();
    }

    let snap1 = orch.step();
    assert_eq!(snap1.tick, 1, "first step must be tick 1");
    render_exchange_tick(&mut out, &snap1, label);

    let mut snap = snap1;
    while snap.tick < 10 {
        snap = orch.step();
    }
    render_exchange_tick(&mut out, &snap, label);

    while snap.tick < 100 {
        snap = orch.step();
    }
    render_exchange_tick(&mut out, &snap, label);

    out
}

// ---------------------------------------------------------------------------
// 1. Routing parity baselines (live, insta snapshots)
// ---------------------------------------------------------------------------

#[test]
fn rsc3_baseline_mega_coffee_machine() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
    ));
    let mut orch = build_orchestrator(&compiler, &[]);
    let report = exchange_parity_report("sim-mega-coffee-machine", &mut orch);
    insta::assert_snapshot!("rsc3_mega_coffee_machine", report);
}

#[test]
fn rsc3_baseline_orchestration_complex() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ));
    let mut orch = build_orchestrator(&compiler, &[]);
    let report = exchange_parity_report("orchestration-complex", &mut orch);
    insta::assert_snapshot!("rsc3_orchestration_complex", report);
}

// ---------------------------------------------------------------------------
// 2. Interface-topology inventory (live, pinned-list)
//
// For every compiling corpus model: enumerate every flow (FlowUsage /
// SuccessionFlowUsage / Flow) whose endpoint ports are NOT covered by any
// declared ConnectionUsage, InterfaceUsage, or BindingConnector relationship
// in the elaborated graph. These flows violate Ports.sysml:37 (the
// `outgoingTransfersFromSelf :> interfacingPorts.incomingTransfersToSelf`
// topology constraint), and will become FL018 diagnostics in RSC-3.3b.
//
// Algorithm:
// 1. Parse + elaborate each model (ModelCompiler::new elaborates).
// 2. Collect all Connection / InterfaceConnection / Binding relationships
//    from the elaborated graph (synthesized by elaborate/connectors.rs from
//    ConnectionUsage, InterfaceUsage, BindingConnector elements).
// 3. Collect their endpoint strings (source / target properties on the
//    originating connector elements).
// 4. For each FlowUsage/SuccessionFlowUsage/Flow element, check if BOTH
//    endpoint ports (derived from the flow's "source"/"target" props) appear
//    in the declared-connector endpoint set. A port NOT appearing in any
//    such pair is an undeclared-topology flow.
//
// The EXPECTED_OVERLAPS constant is pinned below. Every new entry requires a
// conscious update; none may be added without a comment. Shrink over time as
// models are corrected or connections are added (the RS001 playbook).
// ---------------------------------------------------------------------------

/// Connector element kinds that indicate a declared topology.
const CONNECTOR_ELEM_KINDS: &[ElementKind] = &[
    ElementKind::ConnectionUsage,
    ElementKind::InterfaceUsage,
    ElementKind::BindingConnector,
    ElementKind::ConnectorAsUsage,
    ElementKind::BindingConnectorAsUsage,
];

/// The pinned topology-violation inventory (RSC-3.B0 deliverable).
///
/// Format: `"{model} :: flow:{flow_id} :: src={source} tgt={target} (src_decl=… tgt_decl=…)"`
///
/// Every entry here is a flow whose endpoints have no declared connector
/// relationship in the elaborated graph. This list drives the FL018 severity
/// decision. Measured 2026-06-12. Shrink consciously; never grow silently.
///
/// Total: 27 violations across 6 models.
///
/// NOTE: The current elaboration pipeline (pure-runtime, no ide-db resolution)
/// may produce fewer elaborated relationships than the full salsa pipeline.
/// Flows appearing here may be correctly connected in the full IDE pipeline but
/// appear unconnected in the pure-parse pass used by this harness. The list
/// is conservative (over-counts violations). Anonymous flows are identified by
/// their deterministic ElementId (content-addressed, stable across runs).
/// See "Topology inventory caveat" in the debt-ledger entry.
const EXPECTED_TOPOLOGY_VIOLATIONS: &[&str] = &[
    // RSC-3.3b (2026-06-12): the corpus is fully pre-cleared — ZERO violations.
    // FL018 is now a HARD compile error (links::transfer_contract_diagnostics,
    // enforced in ModelCompiler::build_workspace_orchestrator), per the §6 Q2
    // RS001-playbook decision.
    //
    // How the 27→7→0 burn-down closed (the largest contributor, a retired
    // 20-violation multi-file workspace, is no longer in the corpus):
    // - sim-mega-coffee-machine (1): `connect tank.waterOut to brewer.waterIn;`
    //   added next to flow waterFlow (RSC-3.3b).
    // - orchestration-complex (2): connects for waterSupply + pressureFeedback
    //   added (RSC-3.3b).
    // - valve-gating (2): connects for the two pipe flows added (RSC-3.3b).
    // - physics-diagnostics-demo (2): connects added alongside the deliberate
    //   PH001/PH004 demo flows — the demos remain (domain mismatch + direction
    //   conflict are orthogonal to topology declaration) (RSC-3.3b).
    //
    // Syntax: SysML v2 connection_usage, `connect <source> to <target>;`
    // Spec ruling: a plain ConnectionUsage between ports satisfies the
    // Ports.sysml interfacing predicate (Interfaces.sysml:34-43 defines
    // Interface extensionally as "the most general class of links between
    // Ports on Parts"; the OMG spec's normative Annex A model connects ports
    // with bare `connect`).
    // Spec ref: Ports.sysml outgoingTransfersFromSelf :> interfacingPorts.incomingTransfersToSelf
];

/// Collect declared connector endpoint string pairs from an elaborated graph.
///
/// Walks all ConnectionUsage, InterfaceUsage, BindingConnector, etc. elements
/// and collects their "source" / "target" string properties (set by
/// elaborate/connectors.rs's `extract_connector_endpoints` pass).
///
/// Returns a set of "participant.port"-style strings that appear as either
/// the source or target of a declared connector element. Any flow whose
/// endpoint strings are both in this set has a declared topology.
fn declared_connector_endpoints(graph: &ModelGraph) -> BTreeSet<String> {
    let mut endpoints = BTreeSet::new();
    for kind in CONNECTOR_ELEM_KINDS {
        for elem in graph.elements_by_kind(kind) {
            if let Some(src) = elem.get_prop("source").and_then(|v| v.as_str()) {
                endpoints.insert(src.to_owned());
            }
            if let Some(tgt) = elem.get_prop("target").and_then(|v| v.as_str()) {
                endpoints.insert(tgt.to_owned());
            }
        }
    }
    endpoints
}

/// One topology violation line: `"{model} :: flow:{flow_id} :: src={s} tgt={t}"`.
fn topology_violation_lines(model: &str, graph: &ModelGraph) -> Vec<String> {
    let declared = declared_connector_endpoints(graph);
    let mut lines = Vec::new();

    let flow_kinds = [
        ElementKind::FlowUsage,
        ElementKind::SuccessionFlowUsage,
        ElementKind::Flow,
    ];
    for kind in &flow_kinds {
        for elem in graph.elements_by_kind(kind) {
            let src = elem.get_prop("source").and_then(|v| v.as_str());
            let tgt = elem.get_prop("target").and_then(|v| v.as_str());
            // Only examine flows that have both endpoints resolved.
            let (Some(src), Some(tgt)) = (src, tgt) else {
                continue;
            };
            // A flow is "undeclared" if at least one of its endpoint strings
            // does NOT appear in the declared-connector endpoint set.
            let src_declared = declared.contains(src);
            let tgt_declared = declared.contains(tgt);
            if !src_declared || !tgt_declared {
                let flow_id_owned = elem.name.clone().unwrap_or_else(|| elem.id.to_string());
                let src_owned = src.to_owned();
                let tgt_owned = tgt.to_owned();
                lines.push(format!(
                    "{model} :: flow:{flow_id_owned} :: src={src_owned} tgt={tgt_owned} (src_decl={src_declared} tgt_decl={tgt_declared})"
                ));
            }
        }
    }
    lines.sort();
    lines
}

#[test]
fn rsc3_topology_inventory() {
    let examples_dir = workspace_root().join("examples");
    let mut model_dirs: Vec<PathBuf> = std::fs::read_dir(&examples_dir)
        .expect("examples/ dir readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    model_dirs.sort();

    let mut violations: Vec<String> = Vec::new();

    // Scan all example subdirs that parse to non-empty graphs.
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
        // ModelCompiler::new runs elaboration (which synthesizes connector
        // relationships), so the graph it wraps is the elaborated graph.
        let compiler = ModelCompiler::new(load_graph(&files));
        violations.extend(topology_violation_lines(&name, compiler.graph()));
    }

    // Also scan the two non-examples/ models pinned in the parity baselines.
    for rel in [
        "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ] {
        let compiler = ModelCompiler::new(load_file(rel));
        violations.extend(topology_violation_lines(rel, compiler.graph()));
    }

    violations.sort();
    violations.dedup();

    // === REPORT: print the full inventory regardless of pass/fail ===
    eprintln!("== RSC-3.B0 topology inventory (FL018 candidates) ==");
    eprintln!("total topology violations: {}", violations.len());
    for v in &violations {
        eprintln!("  {v}");
    }
    eprintln!(
        "==\nDecision (§6 Q2): FL018 will be a hard error (RS001 playbook).\n\
         Fix the corpus before landing RSC-3.3b; inventory must be zero at that point."
    );

    // Pin the exact count. First run will fail here and print the full list
    // above so you can populate EXPECTED_TOPOLOGY_VIOLATIONS.
    assert_eq!(
        violations.len(),
        EXPECTED_TOPOLOGY_VIOLATIONS.len(),
        "topology violation count changed — re-run consciously and update \
         EXPECTED_TOPOLOGY_VIOLATIONS (RSC-3.B0 deliverable). Full list printed above."
    );

    // Exact list pin.
    let pinned: Vec<String> = EXPECTED_TOPOLOGY_VIOLATIONS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(
        violations, pinned,
        "topology violation list changed — update EXPECTED_TOPOLOGY_VIOLATIONS"
    );
}

// ---------------------------------------------------------------------------
// 3. Intent tests (RSC-3.x target semantics)
//
// These assert the DESIRED state per §3 D-3.0.4. Authored #[ignore]'d; each
// #[ignore] was removed alongside the phase that implemented the behavior.
// ALL LIVE since RSC-3.3c (D1 → 3.3a; D2/D5/D6 → 3.3b; D4/U1 → 3.3c).
// ---------------------------------------------------------------------------

/// D1 — isMove default true (spec-authoritative).
/// Current: `compile_single_flow` hardcodes `is_move: false`.
/// Target: absent an explicit child attribute `isMove = false`, every flow
/// compiles with `is_move: true` (Transfers.kerml:84, default true).
#[test]
fn rsc3_intent_is_move_default_true() {
    use sysml_runtime::flows::compile_ports;
    use sysml_runtime::links::{classify_links, LinkSourceKind};

    let graph = {
        let source = r#"
            part def Source { port out : ~SomePort; }
            part def Sink   { port in  :  SomePort; }
            part src : Source;
            part snk : Sink;
            flow from src.out to snk.in;
        "#;
        let parser = TreeSitterParser::new();
        parser.parse(&[sysml_parser_trait::SysmlFile::new("test.sysml", source.to_owned())]).graph
    };

    let registry = compile_ports(&graph);
    let (lg, _diags) = classify_links(&graph, &registry);
    let flows: Vec<_> = lg
        .iter()
        .filter(|l| l.kind == LinkSourceKind::FlowUsage)
        .collect();
    assert!(!flows.is_empty(), "fixture must compile at least one flow");
    // RSC-3.3a: every flow without an explicit `isMove = false` child
    // attribute compiles with is_move = true.
    assert!(
        flows.iter().all(|f| f.is_move),
        "D1: all flows without explicit isMove=false must default to is_move=true \
         (Transfers.kerml:84 default true)"
    );
}

/// D1 — isPush default true (spec-authoritative).
/// Current: `compile_single_flow` hardcodes `is_push: false`.
/// Target: `is_push: true` by default (Transfers.kerml:92, default true).
#[test]
fn rsc3_intent_is_push_default_true() {
    use sysml_runtime::flows::compile_ports;
    use sysml_runtime::links::{classify_links, LinkSourceKind};

    let graph = {
        let source = r#"
            part def A { port out : ~P; }
            part def B { port in  :  P; }
            part a : A;
            part b : B;
            flow from a.out to b.in;
        "#;
        let parser = TreeSitterParser::new();
        parser.parse(&[sysml_parser_trait::SysmlFile::new("test.sysml", source.to_owned())]).graph
    };

    let registry = compile_ports(&graph);
    let (lg, _diags) = classify_links(&graph, &registry);
    let flows: Vec<_> = lg
        .iter()
        .filter(|l| l.kind == LinkSourceKind::FlowUsage)
        .collect();
    assert!(!flows.is_empty(), "fixture must compile at least one flow");
    assert!(
        flows.iter().all(|f| f.is_push),
        "D1: all flows without explicit isPush=false must default to is_push=true \
         (Transfers.kerml:92 default true)"
    );
}

/// D2 — isMove semantics: payload leaves source port after delivery.
/// LIVE since RSC-3.3b: the dead `move_delivered` once-per-flow gate is
/// deleted; per-TRANSFER move semantics clear the source port's payload
/// feature value (set Null) at delivery, and every subsequent send delivers
/// its own transfer instance.
#[test]
fn rsc3_intent_is_move_clears_source_on_delivery() {
    use sysml_core::Value;
    use sysml_runtime::exchange::ExchangePlane;
    use sysml_runtime::flows::{PortDirection, PortFeature, PortInstanceIR, PortRegistry};
    use sysml_runtime::links::LinkIR;

    // Registry: source port carries a payload feature holding a value.
    let mut registry = PortRegistry::new();
    let mut src = PortInstanceIR::new("producer", "outPort").with_direction(PortDirection::Out);
    src.add_feature(PortFeature {
        name: "datum".into(),
        direction: PortDirection::Out,
        type_name: Some("Real".into()),
        value: Value::Float(42.0),
    });
    registry.register(src);
    let mut tgt = PortInstanceIR::new("consumer", "inPort").with_direction(PortDirection::In);
    tgt.add_feature(PortFeature {
        name: "datum".into(),
        direction: PortDirection::In,
        type_name: Some("Real".into()),
        value: Value::Null,
    });
    registry.register(tgt);

    let mut router = ExchangePlane::new();
    // is_move/is_push = true are the message_channel (spec) defaults.
    router.add_flow(
        LinkIR::message_channel("producer", "outPort", "consumer", "inPort"),
        "moveFlow",
    );

    // First transfer: delivers, payload bound at target, source CLEARED.
    router.send("producer.outPort", Value::Float(42.0));
    let first = router.route_pending_with_ports(Some(&mut registry));
    assert_eq!(first.len(), 1, "first transfer delivers");
    assert_eq!(
        registry.get("consumer.inPort").unwrap().features["datum"].value,
        Value::Float(42.0),
        "payload bound at the target"
    );
    assert_eq!(
        registry.get("producer.outPort").unwrap().features["datum"].value,
        Value::Null,
        "D2: the payload LEAVES the source — feature cleared at delivery"
    );

    // Second transfer: per-TRANSFER instance — delivers again (the old
    // once-per-flow gate must not exist) and clears again.
    registry
        .get_mut("producer.outPort")
        .unwrap()
        .set_feature_value("datum", Value::Float(7.0));
    router.send("producer.outPort", Value::Float(7.0));
    let second = router.route_pending_with_ports(Some(&mut registry));
    assert_eq!(
        second.len(),
        1,
        "D2: each send is its own transfer instance — second send delivers"
    );
    assert_eq!(
        registry.get("producer.outPort").unwrap().features["datum"].value,
        Value::Null,
        "second transfer clears the source again"
    );
}

/// D5 — Topology violation: flow whose endpoints lack a declared interface
/// connection must produce FL018 (compile-time HARD ERROR via the RS001
/// playbook per §6 Q2 decision).
/// LIVE since RSC-3.3b: `links::transfer_contract_diagnostics` emits the
/// error; `ModelCompiler::build_workspace_orchestrator` fails the compile on
/// it. Declaring the connect clears it and records `via_interface`.
#[test]
fn rsc3_intent_undeclared_topology_is_fl018() {
    use sysml_runtime::flows::compile_ports;
    use sysml_runtime::links::{classify_links, transfer_contract_diagnostics};

    let source_undeclared = r#"
        package Fl018Fixture {
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
    let graph = {
        let parser = TreeSitterParser::new();
        let compiler = sysml_runtime::compiler::ModelCompiler::new(
            parser
                .parse(&[sysml_parser_trait::SysmlFile::new(
                    "fl018.sysml",
                    source_undeclared.to_owned(),
                )])
                .graph,
        );
        compiler.graph().clone()
    };
    let registry = compile_ports(&graph);
    let (link_graph, _diags) = classify_links(&graph, &registry);
    let contract = transfer_contract_diagnostics(&graph, &registry, &link_graph);
    assert!(
        contract.iter().any(|d| d.code.as_deref() == Some("FL018")
            && d.severity == sysml_span::Severity::Error),
        "D5: an undeclared port-to-port flow must produce the FL018 hard error, got: {contract:?}"
    );

    // Declaring the connection clears FL018 and records via_interface.
    let source_declared = source_undeclared.replace(
        "flow payloadFlow from producer.outPort to consumer.inPort;",
        "flow payloadFlow from producer.outPort to consumer.inPort;\n            \
         connect producer.outPort to consumer.inPort;",
    );
    let graph2 = {
        let parser = TreeSitterParser::new();
        let compiler = sysml_runtime::compiler::ModelCompiler::new(
            parser
                .parse(&[sysml_parser_trait::SysmlFile::new(
                    "fl018_ok.sysml",
                    source_declared,
                )])
                .graph,
        );
        compiler.graph().clone()
    };
    let registry2 = compile_ports(&graph2);
    let (link_graph2, _d2) = classify_links(&graph2, &registry2);
    let contract2 = transfer_contract_diagnostics(&graph2, &registry2, &link_graph2);
    assert!(
        contract2.iter().all(|d| d.code.as_deref() != Some("FL018")),
        "declared connect must satisfy the topology predicate, got: {contract2:?}"
    );
    assert!(
        link_graph2.iter().next().unwrap().via_interface.is_some(),
        "D5: via_interface records the satisfying connector"
    );
}

/// D6 — Direction violation: routing to an `out`-direction target must fail.
/// LIVE since RSC-3.3b: `links::transfer_contract_diagnostics` emits the
/// FL019 hard error for a flow that delivers into an out-direction port or
/// picks up at an in-direction port (post-conjugation effective directions);
/// a route-time debug_assert in `route_pending_with_ports` is the belt.
#[test]
fn rsc3_intent_direction_violation_out_target_rejected() {
    use sysml_runtime::flows::compile_ports;
    use sysml_runtime::links::{classify_links, transfer_contract_diagnostics};

    // The flow runs BACKWARDS: from the consumer's in-port into the
    // producer's out-port.
    let source = r#"
        package Fl019Fixture {
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
    let graph = {
        let parser = TreeSitterParser::new();
        let compiler = sysml_runtime::compiler::ModelCompiler::new(
            parser
                .parse(&[sysml_parser_trait::SysmlFile::new(
                    "fl019.sysml",
                    source.to_owned(),
                )])
                .graph,
        );
        compiler.graph().clone()
    };
    let registry = compile_ports(&graph);
    let (link_graph, _diags) = classify_links(&graph, &registry);
    let contract = transfer_contract_diagnostics(&graph, &registry, &link_graph);

    let fl019: Vec<_> = contract
        .iter()
        .filter(|d| d.code.as_deref() == Some("FL019"))
        .collect();
    assert!(
        !fl019.is_empty(),
        "D6: a backwards flow (in-direction source / out-direction target) must \
         produce FL019 hard errors, got: {contract:?}"
    );
    assert!(
        fl019
            .iter()
            .all(|d| d.severity == sysml_span::Severity::Error),
        "FL019 is a hard error"
    );
}

/// D4 — MessageTransfer: sends with no declared flow endpoint must NOT be
/// silently dropped; they must become fail-hard FlowError (RSC-1.5 path).
/// LIVE since RSC-3.3c: unrouted sends (no declared flow AND no accepting
/// surface) join `route_pending_checked`'s fail-hard surface as
/// `FlowError::Unrouted`; occurrence-addressed sends with a resolvable
/// named receiver deliver instead (see the D4 conformance case in
/// runtime_spec_conformance.rs).
#[test]
fn rsc3_intent_unrouted_send_is_fail_hard() {
    use sysml_runtime::exchange::ExchangePlane;
    use sysml_runtime::flows::FlowError;
    use sysml_core::Value;

    let mut router = ExchangePlane::new();
    // No flows registered — any send is unrouted.
    router.send("no_such_source.out", Value::Int(1));

    let result = router.route_pending_checked();
    assert!(
        matches!(result, Err(FlowError::Unrouted { count: 1, .. })),
        "D4: an unrouted send in strict mode must fail hard (FlowError::Unrouted), \
         not be silently dropped; got: {result:?}"
    );
}

/// U1 — Pull semantics: is_push=false suppresses eager delivery; delivery
/// requires an explicit `pull(target_key)` call.
/// LIVE since RSC-3.3c (labeled extension per the design doc D-3.0.4 row
/// U1 — the spec removes the push constraint and leaves initiation open):
/// `route_pending` parks transfers on `is_push=false` links per target;
/// `pull(target_key)` delivers them on the target's demand. The accept
/// paths call it: the orchestrator pull-polls armed SM `PortMessage` ports
/// each tick, and `router_receive_for_action` pulls before consuming.
#[test]
fn rsc3_intent_is_push_false_requires_pull() {
    use sysml_runtime::exchange::ExchangePlane;
    use sysml_runtime::links::LinkIR;
    use sysml_core::Value;

    let mut router = ExchangePlane::new();
    let mut flow = LinkIR::message_channel("tank", "out", "brewer", "in");
    flow.is_push = false; // pull transfer (is_move stays the spec default true)
    router.add_flow(flow, "pullFlow");

    router.send("tank.out", Value::Float(2.5));
    // No eager delivery: route_pending alone delivers nothing.
    assert!(
        router.route_pending().is_empty(),
        "U1: is_push=false suppresses eager delivery"
    );
    assert!(
        router.receive("brewer.in").is_none(),
        "nothing reaches the target before it pulls"
    );

    // The accept pulls; the payload arrives.
    let pulled = router.pull("brewer.in");
    assert_eq!(pulled.len(), 1, "pull(target_key) initiates the transfer");
    assert_eq!(pulled[0].payload, Value::Float(2.5));
    assert_eq!(
        router.receive("brewer.in").expect("queued by pull").payload,
        Value::Float(2.5)
    );
}

// ---------------------------------------------------------------------------
// 4. ExchangePlane determinism — two plane-backed Orchestrators of the same
//    model produce byte-identical exchange reports.
//
// RSC-3.5e.2: these were RoutingPlane parity gates (router-backed vs
// plane-backed). FlowRouter is deleted; the ExchangePlane is now the sole
// routing backend, so the "two backends agree" assertion collapses to "the
// one backend is deterministic". These are CODE assertions, not insta
// snapshots — they need no `.snap` file.
//
// Two layers of proof:
//   (a) The three real corpus baseline models, stepped to ticks 1/10/100 —
//       the same models the rsc3_baseline_* snapshots pin. These prove the
//       plane is deterministic on the production compile path.
//   (b) A focused full-tick-loop port-message scenario driving real sends +
//       an SM transition — this exercises the routing machinery end-to-end
//       (the corpus models route zero messages through the exchange surface
//       in this pure-runtime harness, so (a) alone does not cover delivery —
//       see the note in `render_exchange_tick`).
//
// Field-level comparison uses the same `exchange_parity_report` /
// `render_exchange_tick` projection the baselines use (message count, sorted
// flow-triples, port-value key structure, flow_drop_warnings), PLUS a direct
// subsystem-state comparison so behavioural divergence (not just message
// topology) is caught.
// ---------------------------------------------------------------------------

use sysml_runtime::exchange::ExchangePlane;

/// Build two ExchangePlane-backed orchestrators for the same model, step both
/// to 1/10/100, and assert the exchange reports are byte-identical.
///
/// RSC-3.5e.2: FlowRouter was deleted, so this is no longer a router-vs-plane
/// comparison — both backends ARE the `ExchangePlane`. It now pins the plane's
/// determinism on the production compile path (same model + same overrides →
/// byte-identical exchange report), which is the residual value of this gate.
fn assert_corpus_parity(label: &str, build: impl Fn() -> Orchestrator) {
    let mut orch_a = build();
    // `set_exchange_plane(ExchangePlane::new())` re-registers the subsystem
    // acceptors on a fresh empty plane — the exact state `Orchestrator::new`
    // installs by default, exercised explicitly here.
    orch_a.set_exchange_plane(ExchangePlane::new());
    let mut orch_b = build();

    let report_a = exchange_parity_report(label, &mut orch_a);
    let report_b = exchange_parity_report(label, &mut orch_b);

    assert_eq!(
        report_a, report_b,
        "ExchangePlane determinism FAILED for {label}: two builds of the same \
         model diverged in their exchange reports at ticks 1/10/100"
    );
}

#[test]
fn rsc3_routing_plane_parity_mega_coffee_machine() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/sim-mega-coffee-machine.sysml",
    ));
    assert_corpus_parity("sim-mega-coffee-machine", || {
        build_orchestrator(&compiler, &[])
    });
}

#[test]
fn rsc3_routing_plane_parity_orchestration_complex() {
    let compiler = ModelCompiler::new(load_file(
        "crates/lang/sysml-runtime/tests/fixtures/orchestration-complex.sysml",
    ));
    assert_corpus_parity("orchestration-complex", || {
        build_orchestrator(&compiler, &[])
    });
}

/// Full-tick-loop routing determinism: a port-message-triggered SM transition
/// driven by a real `send`. This exercises the delivery machinery the corpus
/// models do not reach in this harness.
///
/// RSC-3.5e.2: was a router-vs-plane parity test; FlowRouter is deleted, so it
/// now drives two ExchangePlane-backed orchestrators with the identical SM +
/// flow + RouteFirst config and the same injected sends, asserting their
/// `ExecutionSnapshot`s agree (the plane routes the overcurrent message and
/// trips the breaker deterministically).
#[test]
fn rsc3_routing_plane_parity_full_loop_port_message() {
    use sysml_core::Value;
    use sysml_runtime::links::LinkIR;
    use sysml_runtime::orchestrator::{OrchestratorConfig, TickStrategy};
    use sysml_runtime::statemachine::{StateMachineRunner, TriggerKind};
    use sysml_runtime::{StateIR, StateMachineIR, TransitionIR};

    fn make_runner() -> StateMachineRunner {
        let ir = StateMachineIR {
            name: "Breaker".into(),
            states: vec![StateIR::new("closed"), StateIR::new("tripped")],
            transitions: vec![TransitionIR {
                from: "closed".into(),
                to: "tripped".into(),
                event: Some("overcurrent".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
            accept_param: None,
            }],
            initial: "closed".into(),
            regions: vec![],
        };
        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(
            0,
            TriggerKind::PortMessage {
                port_name: "overcurrent".into(),
                payload_type: None,
            param_name: None,
            },
        );
        runner
    }

    fn make_flow() -> LinkIR {
        let mut l = LinkIR::message_channel("sensor", "currentOut", "breaker", "overcurrent");
        l.is_move = false;
        l
    }

    fn config() -> OrchestratorConfig {
        OrchestratorConfig {
            dt_ms: 10.0,
            tick_strategy: TickStrategy::RouteFirst,
            ..Default::default()
        }
    }

    // Drive an orchestrator: step, inject, step; collect the (state, completed)
    // projection at each of the two steps.
    fn drive(orch: &mut Orchestrator) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        let snap1 = orch.step();
        let s1 = snap1.subsystem_states.get("breaker").unwrap();
        out.push((s1.current_state.clone(), s1.completed));
        // Inject overcurrent into the flow source.
        orch.send_to_router("sensor.currentOut", Value::Float(150.0));
        let snap2 = orch.step();
        let s2 = snap2.subsystem_states.get("breaker").unwrap();
        out.push((s2.current_state.clone(), s2.completed));
        out
    }

    // Build A: the flow interned into an ExchangePlane via set_exchange_plane.
    let mut orch_a = Orchestrator::new(config());
    orch_a.add_state_machine("breaker", make_runner());
    orch_a.set_exchange_plane({
        let mut p = ExchangePlane::new();
        p.add_flow(make_flow(), "currentFlow");
        p
    });
    let states_a = drive(&mut orch_a);

    // Build B: the identical flow into a second ExchangePlane.
    let mut orch_b = Orchestrator::new(config());
    orch_b.add_state_machine("breaker", make_runner());
    orch_b.set_exchange_plane({
        let mut p = ExchangePlane::new();
        p.add_flow(make_flow(), "currentFlow");
        p
    });
    let plane_states = drive(&mut orch_b);

    assert_eq!(
        states_a, plane_states,
        "full-loop routing determinism FAILED: the port-message SM transition \
         must fire identically across two ExchangePlane-backed builds"
    );
    // Sanity: the scenario actually exercises delivery (transition fires).
    assert_eq!(
        plane_states[0].0, "closed",
        "tick 1: breaker starts closed"
    );
    assert_eq!(
        plane_states[1].0, "tripped",
        "tick 2: the routed overcurrent message must trip the breaker"
    );
}

// ===========================================================================
// L34 — port-message-triggered transitions compile from a PARSED model
// ===========================================================================

/// A `transition ... accept <name> via <port> then <target>` in a parsed model
/// must compile to a `TriggerKind::PortMessage` keyed on the receiver port — so
/// the SM exposes that port as an accept surface (`accept_ports`) and the
/// orchestrator registers it on the ExchangePlane. This was DORMANT (L34): the
/// tree-sitter grammar parses the `via_port` field, but the ast_builder lowering
/// dropped it and the runtime SM compiler never derived a port trigger from a
/// parsed model (only hand-built `set_transition_trigger` reached `PortMessage`,
/// as in `rsc3_routing_plane_parity_full_loop_port_message`). The delivery path
/// was already proven by that hand-built test; this closes the compile-time half.
#[test]
fn rsc3_accept_via_port_compiles_to_port_message_trigger() {
    use sysml_runtime::statemachine::StateMachineRunner;

    let source = r#"
        package PortTriggerFixture {
            item def TripCommand { attribute v : Boolean; }
            port def CmdIn { in item cmd : TripCommand; }
            part def Breaker {
                port tripIn : CmdIn;
                state def Logic {
                    entry; then closed;
                    state closed;
                    transition first closed
                        accept cmd via tripIn
                        then tripped;
                    state tripped;
                }
            }
        }
    "#;
    let parser = TreeSitterParser::new();
    let graph = parser
        .parse(&[SysmlFile::new("PortTrigger.sysml", source.to_owned())])
        .graph;

    let runner = StateMachineRunner::from_graph_named(&graph, "Logic")
        .expect("Logic state machine should compile from the parsed model");

    // The `accept cmd via tripIn` trigger must surface `tripIn` as an accept
    // port — i.e. the transition compiled to TriggerKind::PortMessage{tripIn},
    // not a bare Event (which would never register the acceptor / route).
    assert!(
        runner.accept_ports().contains(&"tripIn".to_owned()),
        "parsed `accept ... via tripIn` must compile to a PortMessage trigger on \
         `tripIn`; accept_ports() = {:?}",
        runner.accept_ports(),
    );
}
