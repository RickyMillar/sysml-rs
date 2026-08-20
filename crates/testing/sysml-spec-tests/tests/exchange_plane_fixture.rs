//! RSC-3.5-pre — ExchangePlane corpus fixture intent tests
//! (behavioural-baseline-first).
//!
//! §8 (RSC-3.5 implementation blueprint).
//!
//! The fixture (`examples/exchange-plane-fixture/ExchangePlane.sysml`) is the
//! real-data target the classified ExchangePlane (ADR-017 Phase 3.5) is built
//! against. It carries NAME-MATCHED, reconcilable
//! typed port features on every link class, plus a 2-instance (L19 def/usage)
//! case (two `ControllerDef` usages).
//!
//! These tests are authored behavioural-baseline-first:
//! - LIVE (un-ignored) tests assert behaviour that HEAD already satisfies and
//!   must PASS now — they pin the fixture / classification baseline.
//! - `#[ignore = "RSC-3.5x: ..."]` tests carry a real, compiling body asserting
//!   the INTENDED outcome of a not-yet-landed wave; un-gating each is a
//!   one-line change once that wave ships.
//!
//! Harness rules (mirrors `rsc3_exchange_baseline.rs`): pure runtime only —
//! `.sysml` files → `TreeSitterParser` → `ModelGraph` → `ModelCompiler`. NO LSP
//! harness, NO SysmlService (deadlock surface, task #225). The ExchangePlane
//! type does NOT exist at HEAD, so it is never referenced here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_core::ModelGraph;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::{context_from_graph, ModelCompiler};
use sysml_runtime::flows::compile_ports;
use sysml_runtime::links::{classify_links, LinkClass, LinkSourceKind};
use sysml_runtime::orchestrator::Orchestrator;

// ---------------------------------------------------------------------------
// Harness helpers (mirrored from rsc3_exchange_baseline.rs)
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

fn load_dir(rel: &str) -> ModelGraph {
    let dir = workspace_root().join(rel);
    assert!(dir.exists(), "model dir not found: {}", dir.display());
    let mut files = Vec::new();
    collect_sysml_files(&dir, &mut files);
    assert!(!files.is_empty(), "no .sysml files in {}", dir.display());
    let parser = TreeSitterParser::new();
    let sysml_files: Vec<SysmlFile> = files
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
    parser.parse(&sysml_files).graph
}

/// The fixture model, parsed (un-elaborated). All consumers build a
/// `ModelCompiler` on top (which elaborates).
fn fixture_graph() -> ModelGraph {
    load_dir("examples/exchange-plane-fixture")
}

/// The fixture model, parsed AND name-resolved — the same order the ide-db
/// product pipeline uses (resolve → elaborate). Required for any test that
/// exercises port-feature typing: without `resolve_references`, `FeatureTyping`
/// children keep only `unresolved_type` strings, `find_feature_type` returns
/// `None`, and `compile_ports` produces feature-less port entries (so signal
/// propagation forms zero pairs). `ModelCompiler::{new,from_arc}` elaborate but
/// never resolve — production resolves upstream in ide-db; the runtime unit-test
/// path must resolve here for fidelity (ledger L36 sub-gap A).
fn fixture_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/exchange-plane-fixture");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// Build a fully-compiled workspace orchestrator for the fixture. The
/// orchestrator's build path runs `classify_links` + `compile_signal_propagation`
/// internally and sets the slot store + signal propagation plan.
fn fixture_orchestrator(compiler: &ModelCompiler) -> Orchestrator {
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
            &[],
            Some(10.0),
            Some(60_000.0),
        )
        .expect("fixture workspace orchestrator should compile")
}

// ===========================================================================
// 1. fixture_three_classes_distinct — BASELINE (LIVE, must PASS)
// ===========================================================================

/// The fixture must classify into all three execution classes with zero
/// `Unknown`, proving the name-matched typed-feature ports route deterministically:
///   - PowerBond:      supply.powerOut -> breakerA.powerIn   (@PowerPort, v/i pair)
///   - SignalLink:     sensor.currentOut -> controller.currentIn  (@Signal, item reading)
///   - MessageChannel: controller.tripOut -> breaker.tripIn   (item TripCommand)
///
/// The 2-instance case (two SignalLinks, two MessageChannels) is reflected in
/// the EXACT distribution asserted below. This is the LIVE baseline — it pins
/// the fixture against the shipped 3.1 classifier.
#[test]
fn fixture_three_classes_distinct() {
    let compiler = ModelCompiler::new(fixture_graph());
    let registry = compile_ports(compiler.graph());
    let (link_graph, diags) = classify_links(compiler.graph(), &registry);

    let dist = link_graph.class_distribution();
    eprintln!(
        "exchange-plane-fixture class distribution: power_bond={} signal_link={} \
         message_channel={} unknown={} (total links={})",
        dist.power_bond,
        dist.signal_link,
        dist.message_channel,
        dist.unknown,
        link_graph.len()
    );
    for d in &diags {
        eprintln!("  classify diag: {d}");
    }

    // EXACT distribution the fixture produces (asserted after observing the
    // real counts — see the report). At minimum: >=1 of each class, 0 unknown.
    assert!(dist.power_bond >= 1, "at least one PowerBond");
    assert!(dist.signal_link >= 1, "at least one SignalLink");
    assert!(dist.message_channel >= 1, "at least one MessageChannel");
    assert_eq!(dist.unknown, 0, "no Unknown links (no FL017)");

    // Pinned exact counts.
    assert_eq!(dist.power_bond, EXPECTED_POWER_BOND, "PowerBond count");
    assert_eq!(dist.signal_link, EXPECTED_SIGNAL_LINK, "SignalLink count");
    assert_eq!(
        dist.message_channel, EXPECTED_MESSAGE_CHANNEL,
        "MessageChannel count"
    );
}

// Pinned distribution constants — confirmed against the live (RSC-3.1)
// classifier. Each payload flow rides on a DECLARED connector (the corpus
// invariant rsc3_topology_inventory pins at zero orphan flows for this
// fixture). classify_links interns the flow-link and the
// connector-link separately (different element ids, no dedup until L29/RSC-3.5e),
// so each pair counts twice:
//   SignalLink     = 2 flows + 2 connects = 4 (sensorA/B.currentOut -> controllerA/B.currentIn)
//   MessageChannel = 2 flows + 2 connects = 4 (controllerA/B.tripOut -> breakerA/B.tripIn)
//   PowerBond      = 1 flow  + 1 connect  = 2 (supply.powerOut -> breakerA.powerIn)
// 10 links total, 0 unknown.
const EXPECTED_POWER_BOND: usize = 2;
const EXPECTED_SIGNAL_LINK: usize = 4;
const EXPECTED_MESSAGE_CHANNEL: usize = 4;

// ===========================================================================
// 2. fixture_signal_link_propagates_reading_slot_to_slot
// ===========================================================================

/// INTENDED: the SignalLink propagates `sensor.currentOut.reading` to
/// `controller.currentIn.reading` slot-to-slot — the slot-backed signal
/// materializer (RSC-3.5d) replaces the legacy string-copy `propagate_port_values`.
///
/// At HEAD the SignalLink class is live and `compile_signal_propagation` emits
/// pairs, but the slot-backed materializer that closes the port-value stream is
/// not yet wired, so the propagation pairs do not (yet) resolve `reading` slots
/// end-to-end. We assert the intended SignalPropagation pair shape: for each of
/// the two SignalLinks there is a `reading` propagation pair.
#[test]
fn fixture_signal_link_propagates_reading_slot_to_slot() {
    let compiler = ModelCompiler::new(fixture_graph_resolved());
    let orch = fixture_orchestrator(&compiler);
    let plan = orch.signal_propagation();

    let reading_pairs: Vec<_> = plan
        .pairs()
        .iter()
        .filter(|p| p.feature == "reading")
        .collect();

    // Two SignalLinks (controllerA + controllerB), each carrying the `reading`
    // feature, must each produce a slot-to-slot propagation pair.
    assert_eq!(
        reading_pairs.len(),
        2,
        "both SignalLinks propagate the `reading` feature slot-to-slot"
    );
    for p in reading_pairs {
        assert!(
            p.source_port_key.ends_with(".currentOut"),
            "source is a sensor currentOut port: {}",
            p.source_port_key
        );
        assert!(
            p.target_port_key.ends_with(".currentIn"),
            "target is a controller currentIn port: {}",
            p.target_port_key
        );
    }
}

// ===========================================================================
// 3. fixture_message_channel_delivers_typed_trip_command
// ===========================================================================

/// INTENDED: the `tripOut -> tripIn` MessageChannel delivers a `TripCommand`
/// and D3 derives a non-None source/target payload type (the
/// `source_payload_type` / `target_payload_type` fields populated from the
/// item-typed port features). The ExchangePlane delivery + payload slots land
/// in RSC-3.5a/b.
#[test]
fn fixture_message_channel_delivers_typed_trip_command() {
    // D3 payload typing reads the item-typed port features (`cmd : TripCommand`),
    // which only populate on a name-resolved graph (sub-gap A, L36) — the
    // `ModelCompiler::new` path elaborates but never resolves, so use the
    // resolved fixture the product pipeline mirrors.
    let compiler = ModelCompiler::new(fixture_graph_resolved());
    let registry = compile_ports(compiler.graph());
    let (link_graph, _diags) = classify_links(compiler.graph(), &registry);


    // The `tripOut -> tripIn` payload moves on the TRANSFER (the `flow`), not the
    // interface-declaring `connect`: payload typing is a transfer property, so
    // `classify_links` derives it on FlowUsage links and leaves connector links
    // `None` (the flow + connect doubling per pair is L29's distinct-links shape).
    let channels: Vec<_> = link_graph
        .ids_of_class(LinkClass::MessageChannel)
        .iter()
        .filter_map(|&id| link_graph.get(id))
        .filter(|l| {
            l.kind == LinkSourceKind::FlowUsage
                && l.source.port == "tripOut"
                && l.target.port == "tripIn"
        })
        .collect();

    // 2 = the two controllerA/B.tripOut -> breakerA/B.tripIn transfer flows.
    assert_eq!(
        channels.len(),
        2,
        "controller->breaker MessageChannel transfers (one flow each)"
    );
    for ch in channels {
        // D3 payload typing — the item feature `cmd : TripCommand` on both ends
        // must yield a derived source/target payload type of TripCommand.
        assert_eq!(
            ch.source_payload_type.as_deref(),
            Some("TripCommand"),
            "source payload type derived from `out item cmd : TripCommand`"
        );
        assert_eq!(
            ch.target_payload_type.as_deref(),
            Some("TripCommand"),
            "target payload type derived from `in item cmd : TripCommand`"
        );
    }
}

// ===========================================================================
// 4. fixture_power_bond_feeds_connection_graph
// ===========================================================================

/// The ConnectionGraph (physics/connection.rs) must see the PowerBond the
/// LinkGraph classified. The PowerBond pair (supply.powerOut -> breakerA.powerIn)
/// is declared as a flow + connect — classify_links interns both (flow-link +
/// connector-link, same endpoints), so there are TWO PowerBond links over the
/// one pair. RSC-3.5f.2 drives `from_link_graph` from the LinkGraph PowerBond
/// subset directly (the L30 completion), deduping the doubly-declared bond to a
/// single ConnectionGraph edge. This also means CONNECTOR-ONLY PowerBonds (a
/// `connect` with no paired `flow`) are now ingested — the legacy flow-driven
/// path silently dropped them; the GS corpus exercises that case.
#[test]
fn fixture_power_bond_feeds_connection_graph() {
    use sysml_runtime::physics::connection::ConnectionGraph;
    use sysml_core::physics::domain::PhysicsDomainRegistry;

    let compiler = ModelCompiler::new(fixture_graph());
    let graph = compiler.graph();
    let registry = compile_ports(graph);
    let (link_graph, _diags) = classify_links(graph, &registry);

    // Two PowerBond links for the one pair (the flow and the connect), both
    // supply.powerOut -> breakerA.powerIn.
    let power_bonds: Vec<_> = link_graph
        .ids_of_class(LinkClass::PowerBond)
        .iter()
        .filter_map(|&id| link_graph.get(id))
        .collect();
    assert_eq!(power_bonds.len(), 2, "two PowerBond links (the flow + the connect)");
    let bond = power_bonds[0];
    assert_eq!(bond.source.owner, "supply");
    assert_eq!(bond.source.port, "powerOut");
    assert_eq!(bond.target.owner, "breakerA");
    assert_eq!(bond.target.port, "powerIn");
    // Both links share the same endpoint pair.
    assert!(
        power_bonds.iter().all(|b| b.source.owner == "supply"
            && b.source.port == "powerOut"
            && b.target.owner == "breakerA"
            && b.target.port == "powerIn"),
        "both PowerBond links are supply.powerOut -> breakerA.powerIn"
    );

    let phys = PhysicsDomainRegistry::from_workspace_graph(graph);
    let (cg, _diags) =
        ConnectionGraph::from_link_graph(&link_graph, &registry, graph, &phys);

    // The PowerBond endpoint pair must show up as a node pair in the
    // ConnectionGraph (the bond feeds the DAE / physics topology).
    let src_path = format!("{}.{}", bond.source.owner, bond.source.port);
    let tgt_path = format!("{}.{}", bond.target.owner, bond.target.port);
    assert!(
        cg.find_node_by_path(&src_path).is_some(),
        "PowerBond source {src_path} is a ConnectionGraph node; nodes={:?}",
        cg.nodes.iter().map(|n| &n.qualified_path).collect::<Vec<_>>()
    );
    assert!(
        cg.find_node_by_path(&tgt_path).is_some(),
        "PowerBond target {tgt_path} is a ConnectionGraph node"
    );
    assert!(
        !cg.edges.is_empty(),
        "the PowerBond contributes at least one ConnectionGraph edge"
    );
}

// ===========================================================================
// 5b. fixture_link_endpoints_resolve_per_usage — RSC-3.5a.1 / L19 (LIVE)
// ===========================================================================

/// The load-bearing L19 proof: `LinkEndpoint.element_id` must carry
/// **per-USAGE** identity, not the shared definition-level port.
///
/// The fixture has `controllerA` and `controllerB` — two usages of ONE
/// `ControllerDef`. The two SignalLinks `sensorA.currentOut -> controllerA.currentIn`
/// and `sensorB.currentOut -> controllerB.currentIn` resolve their target port
/// by name. Because ports are definition-owned (not materialized per usage),
/// the legacy name-scan / `resolve_name` would return the SAME def-level
/// `currentIn` port id for both — def-level identity, wrong for per-instance
/// routing.
///
/// After RSC-3.5a.1 the endpoint id is the stamped per-usage id (the owner
/// participant usage: `controllerA` vs `controllerB`), so the two endpoints
/// must be DISTINCT, and each must resolve to / be the correct controller
/// usage element. If the id were still def-level (shared), the distinct-id
/// assertion would FAIL — that is the signal Layer B needs the per-usage id.
#[test]
fn fixture_link_endpoints_resolve_per_usage() {
    use sysml_core::ElementKind;

    let compiler = ModelCompiler::new(fixture_graph());
    let graph = compiler.graph();
    let registry = compile_ports(graph);
    let (link_graph, _diags) = classify_links(graph, &registry);

    // The two sensor->controller SignalLinks (one per controller usage).
    let signal_targets: Vec<_> = link_graph
        .ids_of_class(LinkClass::SignalLink)
        .iter()
        .filter_map(|&id| link_graph.get(id))
        .filter(|l| l.source.port == "currentOut" && l.target.port == "currentIn")
        .collect();

    // Find the link targeting each controller usage by owner name.
    let link_to = |owner: &str| {
        signal_targets
            .iter()
            .find(|l| l.target.owner == owner)
            .unwrap_or_else(|| panic!("a SignalLink targets {owner}.currentIn"))
    };
    let a = link_to("controllerA");
    let b = link_to("controllerB");

    let a_id = a
        .target
        .element_id
        .clone()
        .expect("controllerA.currentIn endpoint has a stamped element_id (L19)");
    let b_id = b
        .target
        .element_id
        .clone()
        .expect("controllerB.currentIn endpoint has a stamped element_id (L19)");

    // (1) Per-usage: the two target endpoints are DISTINCT element ids.
    assert_ne!(
        a_id, b_id,
        "controllerA.currentIn and controllerB.currentIn must resolve to DISTINCT \
         element ids — per-usage identity, not the shared ControllerDef port \
         (a={a_id} b={b_id})"
    );

    // (2) Each id resolves to / is the correct controller USAGE (a PartUsage
    // named controllerA / controllerB), NOT the shared ControllerDef.
    let resolve_usage_name = |id: &sysml_core::ElementId| -> Option<String> {
        let elem = graph.get_element(id)?;
        // The stamped id is the owner participant usage (per-usage discriminator)
        // when the port itself is definition-owned (the shared-def case). It must
        // be a PartUsage, never a PartDefinition.
        assert_ne!(
            elem.kind,
            ElementKind::PartDefinition,
            "endpoint id must not be the shared ControllerDef (def-level identity)"
        );
        elem.name.clone()
    };
    assert_eq!(
        resolve_usage_name(&a_id).as_deref(),
        Some("controllerA"),
        "controllerA.currentIn endpoint id is the controllerA usage"
    );
    assert_eq!(
        resolve_usage_name(&b_id).as_deref(),
        Some("controllerB"),
        "controllerB.currentIn endpoint id is the controllerB usage"
    );
}

// ===========================================================================
// 4b. Port-message delivery (RSC-3.5d step 10b `wire_message_router`)
// ===========================================================================

/// The `examples/port-message-delivery` model, parsed AND name-resolved
/// (sub-gap A, L36 — `ModelCompiler` elaborates but never resolves, so port
/// features only materialize on a resolved graph).
fn delivery_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/port-message-delivery");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// LIVE (must PASS): the production build path now populates the ExchangePlane
/// router from the classified link graph (step 10b `wire_message_router`).
/// Before that step the router was an empty `ExchangePlane::new()` on a compiled
/// model — every `ingest_classified`/`set_exchange_plane` caller was test-only.
///
/// This pins the FIRST of the three gaps that gate end-to-end SM port delivery
/// (see `fixture_port_message_trips_receiver_sm` for the other two): the routing
/// TABLE is present and classified on a compiled, resolved model.
#[test]
fn fixture_compiled_router_is_wired_from_link_graph() {
    let compiler = ModelCompiler::new(delivery_graph_resolved());
    let orch = fixture_orchestrator(&compiler);

    // The MessageChannel relay.tripOut -> breaker.tripIn is present and
    // classified on the COMPILED orchestrator's link graph (the router adopts a
    // clone of exactly this as its routing table).
    let mc: Vec<_> = orch
        .link_graph()
        .iter()
        .filter(|l| {
            l.class == LinkClass::MessageChannel
                && l.source.key() == "relay.tripOut"
                && l.target.key() == "breaker.tripIn"
        })
        .collect();
    assert!(
        !mc.is_empty(),
        "compiled link graph must carry the relay.tripOut -> breaker.tripIn \
         MessageChannel that step 10b ingests into the router; links = {:?}",
        orch.link_graph()
            .iter()
            .map(|l| (l.source.key(), l.target.key(), l.class))
            .collect::<Vec<_>>(),
    );
    // The flow-derived link defaults to is_move=true (Transfers.kerml, L17).
    assert!(
        mc.iter().any(|l| l.kind == LinkSourceKind::FlowUsage && l.is_move),
        "the FlowUsage MessageChannel defaults to is_move=true",
    );
}

/// LIVE end-to-end (driven through the COMPILED orchestrator, NOT a hand-wired
/// `set_exchange_plane`): a message injected at `relay.tripOut` routes through
/// the wired router to `breaker.tripIn` and fires the receiver SM's
/// `accept cmd via tripIn` PortMessage trigger, tripping it `closed -> tripped`.
///
/// This was the three-gap end-to-end intent. All three now land:
///   - GAP 1 (router wiring, step 10b `wire_message_router`) — DONE.
///   - GAP 2 (L23 sub-gap B, SM-subsystem ↔ instance-key naming) — the receiver
///     SM compiles under its `state def` name "Logic", but the compiler now
///     records the owner-instance map `breaker → ["Logic"]`
///     (`register_owner_subsystem`), so `convert_deliveries_to_port_events`
///     reconciles the flow target `breaker.tripIn` to `port_events["Logic"]`.
///     Subsystem names stay byte-identical (no rename) — the SM still reads
///     under `subsystem.name == "Logic"`, which is why the assertion below
///     queries `subsystem_states["Logic"]`, not `"breaker"`.
///   - GAP 3 (StepFirst feeds delivered → port_events) — the production build
///     runs `TickStrategy::StepFirst`; `convert_deliveries_to_port_events` is
///     now called post-route in BOTH strategies, with the `port_events.clear()`
///     repositioned so the delivery survives one tick to the SM's next read
///     (1-tick latency, spec-silent on tick granularity). Hence the message
///     injected before step 2 trips the SM on step 3.
///
/// See ledger L26.
#[test]
fn fixture_port_message_trips_receiver_sm() {
    use sysml_core::Value;
    let compiler = ModelCompiler::new(delivery_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);

    // Settle into `closed` (entry; then closed).
    let _ = orch.step();
    // External stimulus at the source port (legitimate — this is NOT hand-
    // wiring the routing table; the router was wired by the compiler).
    orch.send_to_router("relay.tripOut", Value::Bool(true));
    // Step 2: StepFirst routes + converts the delivery into port_events.
    let _ = orch.step();
    // Step 3: the SM reads the (1-tick-latent) port event and trips.
    let snap = orch.step();

    // GAP 2: the non-multiplied receiver SM is registered under its `state def`
    // name "Logic" (subsystem names are NOT renamed — snapshots byte-identical);
    // the owner map routes `breaker.tripIn` deliveries to it.
    let breaker = snap
        .subsystem_states
        .get("Logic")
        .expect("receiver SM registered under its `state def` name (GAP 2: no rename)");
    assert_eq!(
        breaker.current_state, "tripped",
        "the routed `accept cmd via tripIn` message must trip the receiver SM",
    );
}

/// GAP 2 non-mutation guard: the owner-instance map must NOT rename subsystems.
/// The receiver SM stays registered under its `state def` name "Logic"; the
/// part-usage key "breaker" the flow targets must NEVER become a subsystem name
/// (a rename would move every `subsystem_states` snapshot baseline). This is the
/// targeted regression for "guards accidental rename" — the corpus-wide
/// byte-identical proof is rsc2_behavioural_baseline + service_command_baseline.
#[test]
fn gap2_owner_map_does_not_rename_subsystems() {
    let compiler = ModelCompiler::new(delivery_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);
    let snap = orch.step();

    assert!(
        snap.subsystem_states.contains_key("Logic"),
        "receiver SM keyed by its `state def` name; keys = {:?}",
        snap.subsystem_states.keys().collect::<Vec<_>>(),
    );
    assert!(
        !snap.subsystem_states.contains_key("breaker"),
        "the part-usage key `breaker` must NOT be a subsystem name (no rename); \
         keys = {:?}",
        snap.subsystem_states.keys().collect::<Vec<_>>(),
    );
}

/// The `examples/port-message-delivery-multi` model, parsed AND name-resolved.
fn multi_delivery_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/port-message-delivery-multi");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// GAP 2 fan-out under multiplication (the owner-key + clear-ordering trap):
/// two `breaker1 / breaker2 : BreakerDef` usages multiply into `breaker1.Logic`
/// / `breaker2.Logic`. A message routed to `breaker1.tripIn` must trip ONLY
/// `breaker1.Logic`; `breaker2.Logic` stays `closed` (no sibling leak through
/// the shared `port_events` map across the repositioned `clear()`).
#[test]
fn gap2_multi_instance_routing_isolates_siblings() {
    use sysml_core::Value;
    let compiler = ModelCompiler::new(multi_delivery_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);

    // Settle both SMs into `closed`.
    let _ = orch.step();
    // Stimulate ONLY breaker1's source port.
    orch.send_to_router("relay1.tripOut", Value::Bool(true));
    let _ = orch.step(); // route + convert → port_events["breaker1.Logic"]
    let snap = orch.step(); // breaker1.Logic reads + trips

    let b1 = snap
        .subsystem_states
        .get("breaker1.Logic")
        .expect("breaker1.Logic subsystem present (multiplied)");
    let b2 = snap
        .subsystem_states
        .get("breaker2.Logic")
        .expect("breaker2.Logic subsystem present (multiplied)");
    assert_eq!(
        b1.current_state, "tripped",
        "the message routed to breaker1.tripIn trips breaker1.Logic",
    );
    assert_eq!(
        b2.current_state, "closed",
        "breaker2.Logic must NOT trip — the delivery is owner-isolated, no \
         sibling leak across the shared port_events map / clear() reposition",
    );
}

/// The `examples/port-message-send` model, parsed AND name-resolved.
fn send_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/port-message-send");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// LIVE end-to-end SEND half (L26 finish line) — the dual of
/// `fixture_port_message_trips_receiver_sm`. There is NO `send_to_router`
/// injection: a fully-compiled SENDER SM (`RelayLogic`) drives the message.
///
/// The relay settles into `idle`, takes its guard-only transition into
/// `sending`, and that state's entry action `send TripCommand via tripOut`
/// lowers (SM compiler) to the canonical `send via tripOut` string. The
/// orchestrator decodes it, resolves the sender's owner-instance key `relay`
/// (inverse of the GAP 2 `owner_to_subsystems` map) and enqueues an addressed
/// MessageTransfer at `relay.tripOut`. The wired router (step 10b) routes it
/// along the declared flow to `breaker.tripIn`, and GAP 2+3 delivery trips the
/// receiver SM `closed -> tripped`. This closes the send→route→deliver→trigger
/// loop entirely inside the production orchestrator. See ledger L26.
#[test]
fn fixture_port_message_send_drives_receiver_sm() {
    let compiler = ModelCompiler::new(send_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);

    // Drive the production orchestrator. The relay reaches `sending` (firing its
    // entry send), the message routes, and the breaker trips — all without any
    // external `send_to_router`. Bounded loop tolerates the exact send/route/
    // deliver latency (guard-only transition + 1-tick StepFirst delivery).
    let mut last = orch.step();
    for _ in 0..7 {
        if last
            .subsystem_states
            .get("Logic")
            .is_some_and(|s| s.current_state == "tripped")
        {
            break;
        }
        last = orch.step();
    }

    // The sender SM actually advanced to `sending` (proves the send path ran,
    // not an accidental trip).
    let relay = last
        .subsystem_states
        .get("RelayLogic")
        .expect("sender SM registered under its `state def` name `RelayLogic`");
    assert_eq!(
        relay.current_state, "sending",
        "the sender SM must reach `sending` and fire its entry `send ... via tripOut`",
    );

    // The receiver SM tripped — driven by the compiled SENDER, no injection.
    let breaker = last
        .subsystem_states
        .get("Logic")
        .expect("receiver SM registered under its `state def` name `Logic` (GAP 2)");
    assert_eq!(
        breaker.current_state, "tripped",
        "the compiled sender's `send TripCommand via tripOut` must route through \
         the wired router and trip the receiver SM — with no send_to_router",
    );
}

/// The `examples/action-port-message-delivery` model, parsed AND name-resolved.
fn action_delivery_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/action-port-message-delivery");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// Wave B-inc-2 behavioural gate — an ACTION receiver advances past `accept …
/// via <port>` on the PRODUCTION orchestrator (the action-graph dual of
/// `fixture_port_message_send_drives_receiver_sm`).
///
/// A fully-compiled SENDER SM (`RelayLogic`) drives `send TripCommand via
/// tripOut`; the wired router (step 10b) routes it along the declared flow to
/// `receiverWired.inPort`; GAP 2+3 surfaces it as `port_events["recvWired"]`;
/// and the receiver ACTION's `accept cmd via inPort` node consumes it from the
/// unified `TickContext::port_payloads` channel (Wave B-inc-2 `port_inbox`),
/// advancing the token past Accept to completion. NO `deliver_message`, NO
/// `send_to_router` — the whole send→route→deliver→accept arc runs inside
/// `build_workspace_orchestrator` + `step()`.
///
/// `recvUnwired` is the load-bearing negative twin: the same `accept cmd via
/// inPort` action, but no flow feeds its port — so its token MUST stay blocked
/// at Accept. This proves the action advances only on a genuine delivery.
///
/// Gate scope: the action ADVANCES PAST its Accept node; CONSUMING the payload
/// value in a guard/effect is the receiver-consumes-payload follow-up.
#[test]
fn fixture_port_message_drives_receiver_action() {
    let compiler = ModelCompiler::new(action_delivery_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);

    // Drive the production orchestrator. Bounded loop tolerates the sender's
    // guard-only transition + 1-tick StepFirst delivery + the action's
    // initial→accept→final token steps.
    let mut last = orch.step();
    for _ in 0..12 {
        if last
            .subsystem_states
            .get("recvWired")
            .is_some_and(|s| s.completed)
        {
            break;
        }
        last = orch.step();
    }

    // The sender SM actually fired its send (proves the send path ran).
    let relay = last
        .subsystem_states
        .get("RelayLogic")
        .expect("sender SM registered under its `state def` name `RelayLogic`");
    assert_eq!(
        relay.current_state, "sending",
        "the sender SM must reach `sending` and fire its entry `send ... via tripOut`",
    );

    // The wired receiver ACTION registered (Wave B-inc-1 usage-based
    // registration) and advanced past `accept cmd via inPort` to completion,
    // driven entirely by the compiled sender through the production orchestrator.
    let wired = last
        .subsystem_states
        .get("recvWired")
        .expect("wired receiver action registered as a subsystem (Wave B-inc-1)");
    assert_eq!(wired.kind, "action", "recvWired is an action subsystem");
    assert!(
        wired.completed,
        "the routed message must drive recvWired past `accept cmd via inPort`; \
         current_state = {:?}",
        wired.current_state,
    );

    // Negative twin: no flow feeds receiverUnwired.inPort → the token stays
    // blocked at its Accept node (the action never completes).
    let unwired = last
        .subsystem_states
        .get("recvUnwired")
        .expect("unwired receiver action registered");
    assert!(
        !unwired.completed,
        "recvUnwired has no incoming flow → its token must stay blocked at Accept",
    );
}

/// The `examples/port-message-payload` model, parsed AND name-resolved.
fn payload_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/port-message-payload");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// A-scalar payload-carrying-messages gate — the Value-carrying upgrade of the
/// SM-send wire. A compiled sender SM `send <bool> via <port>` carries a REAL
/// payload Value (not the `send via` trace string) that the receiver CONSUMES
/// in its `accept cmd via <port> if cmd` guard. NO `send_to_router`.
///
/// Two independent pairs run in one production orchestrator:
///   - senderTrue  sends `true`  → receiverTrue (`GuardedTrue`)  guard passes → `tripped`
///   - senderFalse sends `false` → receiverFalse (`GuardedFalse`) guard fails  → stays `closed`
///
/// The negative twin is load-bearing: BOTH messages are delivered (the
/// PortMessage trigger is name-matched), but only the `true` value gates the
/// transition open — proving the payload Value is genuinely consumed in the
/// guard, not merely delivered. Spec: the accepted transfer's payload binds the
/// accept parameter (Transfers.kerml:254-266). See ledger L26 / the steward
/// ruling that split Wave A into scalar (this) + structured (`new T(...)`).
#[test]
fn fixture_port_message_payload_drives_guard() {
    let compiler = ModelCompiler::new(payload_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);

    // Drive the production orchestrator until the positive receiver trips
    // (bounded loop tolerates the guard-only send + 1-tick StepFirst delivery).
    let mut last = orch.step();
    for _ in 0..8 {
        if last
            .subsystem_states
            .get("GuardedTrue")
            .is_some_and(|s| s.current_state == "tripped")
        {
            break;
        }
        last = orch.step();
    }

    // POSITIVE: the `true` payload passed the `if cmd` guard → tripped.
    let pass = last
        .subsystem_states
        .get("GuardedTrue")
        .expect("receiver SM `GuardedTrue` registered under its state-def name");
    assert_eq!(
        pass.current_state, "tripped",
        "a `true` payload delivered via the Value channel must pass the \
         `accept cmd via in1 if cmd` guard and trip the receiver",
    );

    // NEGATIVE twin: the `false` payload was delivered (same wire) but the
    // `if cmd` guard read `Bool(false)` → no fire → stays `closed`. This is the
    // proof the payload is CONSUMED, not just routed.
    let block = last
        .subsystem_states
        .get("GuardedFalse")
        .expect("receiver SM `GuardedFalse` registered under its state-def name");
    assert_eq!(
        block.current_state, "closed",
        "a `false` payload must be delivered yet BLOCKED by the `if cmd` guard — \
         proving the receiver genuinely consumes the payload Value, not just the \
         message arrival",
    );
}

/// The `examples/port-message-payload-structured` model, parsed + resolved.
fn structured_payload_graph_resolved() -> ModelGraph {
    let mut graph = load_dir("examples/port-message-payload-structured");
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// A-structured payload gate — a compiled sender SM sends a CONSTRUCTED payload
/// `new TripCommand(tripValue = <bool>)`, and the receiver reads the named field
/// in a guard `if cmd.tripValue`. This exercises the full structured path:
/// `new T(field=value)` → ConstructorExpression → `ConstructorCall{named_args}`
/// → `Value::Map{tripValue→Bool}` → bound to `cmd` → member access in the guard.
///
///   - senderTrue  sends `tripValue = true`  → receiverTrue guard opens → `tripped`
///   - senderFalse sends `tripValue = false` → receiverFalse guard blocks → `closed`
///
/// The negative twin proves the FIELD value gates the transition (both messages
/// are delivered, both carry a Map; only the `true` field opens the guard) — the
/// structured analogue of the A-scalar consumption proof. NO `send_to_router`.
#[test]
fn fixture_structured_payload_field_drives_guard() {
    let compiler = ModelCompiler::new(structured_payload_graph_resolved());
    let mut orch = fixture_orchestrator(&compiler);

    let mut last = orch.step();
    for _ in 0..8 {
        if last
            .subsystem_states
            .get("StructGuardedTrue")
            .is_some_and(|s| s.current_state == "tripped")
        {
            break;
        }
        last = orch.step();
    }

    // POSITIVE: `new TripCommand(tripValue = true)` → Map{tripValue→true};
    // the guard `if cmd.tripValue` reads the named field and opens.
    let pass = last
        .subsystem_states
        .get("StructGuardedTrue")
        .expect("receiver SM `StructGuardedTrue` registered under its state-def name");
    assert_eq!(
        pass.current_state, "tripped",
        "a constructed payload `new TripCommand(tripValue = true)` must evaluate \
         to a Value::Map whose `tripValue` field passes `if cmd.tripValue`",
    );

    // NEGATIVE twin: `tripValue = false` is delivered and carries a Map, but the
    // member-access guard reads `false` → no fire → stays `closed`. Proves the
    // named FIELD value is what gates the transition.
    let block = last
        .subsystem_states
        .get("StructGuardedFalse")
        .expect("receiver SM `StructGuardedFalse` registered under its state-def name");
    assert_eq!(
        block.current_state, "closed",
        "a `tripValue = false` payload must be delivered yet BLOCKED by \
         `if cmd.tripValue` — proving the receiver reads the named field, not \
         just the message arrival",
    );
}

// ===========================================================================
// 5. fixture_ismove_clears_source_payload_on_delivery
// ===========================================================================

/// INTENDED: on an `is_move` MessageChannel delivery, the source payload slot
/// is cleared (the move semantics of Transfers.kerml applied to a TYPED
/// payload). The blueprint (risk c) flags that L26 SM-sends are bare event
/// names today and the controller's `tripOut` port carries NO runtime `cmd`
/// feature (the L23/L25 "ports are a value dead-end" gap) — so there is no
/// typed payload slot for the move to clear. This passes once L26 SM-send
/// payload extraction + RSC-3.5b payload slots land.
///
/// This test deliberately routes through the COMPILED port registry
/// (`compile_ports`), NOT a hand-built one — so it genuinely depends on the
/// controller port materializing its `cmd : TripCommand` feature, which is the
/// thing L26/3.5b deliver. At HEAD the compiled `tripOut` port has no `cmd`
/// feature, so the move-clear has nothing to act on.
#[test]
#[ignore = "Wave C-inc-2 DEFERRED (verify-before-build, Jun 19 2026): isMove \
            move-clear cannot be exercised end-to-end on the compiled \
            orchestrator today. apply_move_semantics consumes `delivered` from \
            router.route_pending(); the only internal router.send callers are \
            SM-send / action-send / addressed-message outputs — i.e. \
            MessageTransfers (the `send via` lane). A bare `flow` (FlowTransfer) \
            moves via continuous propagation (propagate_port_values / \
            compile_signal_propagation) and produces NO router delivery, so it \
            never reaches move-clear. The steward (a0bf7a94) ruled move-clear is \
            FlowTransfer-ONLY and MessageTransfers must NOT move-clear — so the \
            one category that reaches the router is the one that should be inert. \
            (This test is itself on a MessageChannel via route_pending_with_ports \
            — the category error the steward flagged.) The bridge/keying half of \
            Wave C shipped in C-inc-1 (52e40364): the def-keyed registry is no \
            longer mutated and the instance/def keying miss is fixed. Un-ignoring \
            requires a future steward+director-gated wave that FIRST gives bare \
            flows a discrete-transfer routing path (or re-targets move-clear onto \
            the continuous-copy path). See ledger L26 / plan project-rsc-portflow-\
            completeness-plan.md WAVE C STATUS."]
fn fixture_ismove_clears_source_payload_on_delivery() {
    use sysml_core::Value;
    use sysml_runtime::exchange::ExchangePlane;

    let compiler = ModelCompiler::new(fixture_graph());
    let registry = compile_ports(compiler.graph());
    let (link_graph, _diags) = classify_links(compiler.graph(), &registry);

    // The controllerA->breakerA MessageChannel is an is_move transfer (spec
    // default true, RSC-3.3a / L17 preserved).
    let channel = link_graph
        .ids_of_class(LinkClass::MessageChannel)
        .iter()
        .filter_map(|&id| link_graph.get(id))
        .find(|l| l.source.owner == "controllerA" && l.source.port == "tripOut")
        .expect("controllerA tripOut MessageChannel present");
    assert!(channel.is_move, "MessageChannel defaults to is_move=true");

    // INTENDED (L26/3.5b): the compiled controller `tripOut` port materializes a
    // `cmd : TripCommand` feature that the SM send populates. At HEAD this
    // feature does NOT exist (the value dead-end), so this assertion fails until
    // payload slots are wired — the gate the #[ignore] names.
    let mut out_registry = registry.clone();
    let cmd_present = out_registry
        .get("controllerA.tripOut")
        .map(|p| p.features.contains_key("cmd"))
        .unwrap_or(false);
    assert!(
        cmd_present,
        "controllerA.tripOut must materialize its `cmd : TripCommand` payload \
         feature (L26/3.5b); at HEAD the port is a value dead-end"
    );
    // Seed the typed payload as the SM send would, then route the move transfer.
    out_registry
        .get_mut("controllerA.tripOut")
        .expect("port")
        .set_feature_value("cmd", Value::Bool(true));

    let mut router = ExchangePlane::new();
    let (lg, _diags) = classify_links(compiler.graph(), &registry);
    for link in lg.iter().filter(|l| l.kind == LinkSourceKind::FlowUsage) {
        let id = link.display_label(compiler.graph());
        router.add_flow(link.clone(), id);
    }
    router.send("controllerA.tripOut", Value::Bool(true));
    let delivered = router.route_pending_with_ports(Some(&mut out_registry));
    assert_eq!(delivered.len(), 1, "the trip command is delivered");
    assert_eq!(
        out_registry.get("controllerA.tripOut").expect("port").features["cmd"].value,
        Value::Null,
        "the move transfer clears the source payload slot on delivery"
    );
}
