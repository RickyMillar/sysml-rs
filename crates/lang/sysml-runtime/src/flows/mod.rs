//! # sysml-run-flows
//!
//! Data flow routing and message transfer for the SysML v2 runtime.
//!
//! This crate implements the message-passing infrastructure that connects
//! action send/accept nodes. The [`ExchangePlane`](crate::exchange::ExchangePlane)
//! routes messages between participants based on compiled flow connections
//! (it superseded the legacy string-keyed `FlowRouter`, deleted in RSC-3.5e.2).
//!
//! ## SysML v2 Flow Types
//!
//! ```text
//! FlowConnectionUsage (base)
//! ├── ItemFlow               — general item transfer
//! ├── SuccessionFlowUsage    — temporally ordered flow
//! └── MessageUsage           — typed message transfer
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ActionRunner (send node)
//!        |
//!        v
//!    ExchangePlane ─routes─>  ActionRunner (accept node)
//!        |
//!        v
//!   EventTracker (occurrence logging)
//! ```
//!
//! ## Spec References
//!
//! - `SysML.xtext:1264-1278` — Flow grammar
//! - `library.systems/Flows.sysml` — MessageAction, Message
//! - `library.kernel/Transfers.kerml` — Transfer semantics
//! - `SysML-vocab.ttl` — FlowUsage, SuccessionFlowUsage, ItemFlow

#![allow(clippy::indexing_slicing)]
mod health;
pub mod port;

pub use health::{
    flow_health_diagnostics, port_health_diagnostics, port_health_diagnostics_from_graph,
};
pub use port::{PathError, PortDirection, PortFeature, PortInstanceIR, PortRegistry, ResolvedPath};

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

/// Bundled output of the port+flow compile pass.
///
/// Per ADR-011 §3, `compile_ports` (RT-17) is a pure graph derivative that
/// feeds the compiler's orchestrator-assembly path (the registry to
/// `Orchestrator::set_port_registry`). Wrapping it in a bundle lets the
/// salsa-cached upstream (`sysml_ide_db::port_flow_runtime`) land it in a
/// single `Arc` clone alongside its memoization key.
///
/// RSC-3.5e.5 W3 retired the bundled `connections: Vec<FlowConnectionIR>`
/// field: `classify_links` now walks the flow elements directly (folding the
/// former `compile_flows` producer into classification), so nothing downstream
/// needs a pre-compiled flow list. The bundle survives only as the registry's
/// cache wrapper; `build_port_flow_resources` is the pure-graph constructor.
#[derive(Debug, Clone)]
pub struct PortFlowResources {
    pub registry: PortRegistry,
}

impl PortFlowResources {
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

/// Build [`PortFlowResources`] from a [`ModelGraph`] — pure graph derivative
/// (just `compile_ports`, RT-17; the flow list is no longer pre-compiled —
/// `classify_links` walks the flow elements itself, RSC-3.5e.5 W3).
pub fn build_port_flow_resources(graph: &ModelGraph) -> PortFlowResources {
    let registry = compile_ports(graph);
    PortFlowResources { registry }
}

use sysml_core::resolution::scoping::chaining::find_feature_type;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};

// ---------------------------------------------------------------------------
// Flow IR
// ---------------------------------------------------------------------------
//
// RSC-3.5e.5 W4: `struct FlowConnectionIR` + its builders and the
// `compile_flows` / `compile_flows_with_registry` / `compile_single_flow`
// producer were deleted. The classified `LinkGraph` (built by
// `crate::links::classify_links`, which walks the flow elements itself via the
// `pub(crate)` helpers below) is now the single flow representation. Only
// `FlowEndpoint` survives here, as the parse target shared by `parse_endpoint`
// and `derive_endpoint_payload_type` (and `crate::sequence`).

/// An endpoint of a flow connection.
#[derive(Debug, Clone)]
pub struct FlowEndpoint {
    /// The participant (part) name.
    pub participant: String,
    /// The port or feature name.
    pub port: String,
}

impl FlowEndpoint {
    /// Create a new endpoint.
    pub fn new(participant: impl Into<String>, port: impl Into<String>) -> Self {
        Self {
            participant: participant.into(),
            port: port.into(),
        }
    }

    /// A qualified key for routing: `participant.port`.
    pub fn key(&self) -> String {
        format!("{}.{}", self.participant, self.port)
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// A message in transit through the flow network.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FlowMessage {
    /// Unique message ID.
    pub id: String,
    /// The flow connection this message traverses.
    pub flow_id: String,
    /// Source endpoint key.
    pub source: String,
    /// Target endpoint key.
    pub target: String,
    /// The payload value.
    pub payload: Value,
    /// Monotonic sequence number for ordering.
    pub sequence: u64,
}

// ---------------------------------------------------------------------------
// Event tracking
// ---------------------------------------------------------------------------

/// A recorded flow event for tracing and analysis.
#[derive(Debug, Clone)]
pub struct FlowEvent {
    /// The flow that produced this event.
    pub flow_id: String,
    /// What happened.
    pub kind: FlowEventKind,
    /// Monotonic step number.
    pub step: u64,
}

/// Kinds of flow events.
#[derive(Debug, Clone)]
pub enum FlowEventKind {
    /// A message was sent.
    MessageSent {
        source: String,
        target: String,
        payload: Value,
    },
    /// A message was delivered.
    MessageDelivered { target: String, payload: Value },
    /// A message could not be routed (no matching flow).
    MessageDropped { source: String, payload: Value },
    /// A message was blocked by succession ordering (source not yet complete).
    SuccessionBlocked {
        flow_id: String,
        source: String,
        payload: Value,
    },
    /// A message was rejected due to payload type mismatch.
    TypeMismatch {
        flow_id: String,
        expected: String,
        actual: String,
        payload: Value,
    },
    /// RSC-3.3c U1: a message on an `is_push == false` link was parked in
    /// the pull store instead of being delivered eagerly. Delivery happens
    /// when the target pulls ([`ExchangePlane::pull`](crate::exchange::ExchangePlane::pull)).
    PullParked {
        flow_id: String,
        target: String,
        payload: Value,
    },
    /// RSC-3.3c D4: an occurrence-addressed MessageTransfer named a target
    /// that resolved to MORE than one accepting surface. The message was
    /// withheld (never pick-first silently); strict mode fails hard via
    /// [`ExchangePlane::route_pending_checked`](crate::exchange::ExchangePlane::route_pending_checked).
    MessageAmbiguous {
        target: String,
        candidates: Vec<String>,
        payload: Value,
    },
}

// ---------------------------------------------------------------------------
// Flow errors
// ---------------------------------------------------------------------------

/// Errors surfaced by the fail-hard flow routing APIs (RSC-1.5 / G14).
#[derive(Debug, Clone, thiserror::Error)]
pub enum FlowError {
    /// One or more pending messages were evicted because the pending queue
    /// was at capacity. Per the fail-hard principle, silent message loss is
    /// forbidden: in strict mode (the default) this error is returned by
    /// [`ExchangePlane::route_pending_checked`](crate::exchange::ExchangePlane::route_pending_checked) instead of routing.
    #[error("flow message loss: dropped {dropped} message(s) (pending queue at capacity {capacity}): {detail}")]
    MessageLoss {
        /// Number of messages evicted since the loss window was last drained.
        dropped: u64,
        /// The configured pending-queue capacity.
        capacity: usize,
        /// Per-source breakdown, e.g. `"'sensor.out' x3"`.
        detail: String,
    },

    /// One or more payloads provably failed to conform to a flow's derived
    /// endpoint typing (`source_payload_type` / `target_payload_type`,
    /// Transfers.kerml:100-118). The offending messages were withheld; in
    /// strict mode (the default) this error is returned by
    /// [`ExchangePlane::route_pending_checked`](crate::exchange::ExchangePlane::route_pending_checked) after the routing pass
    /// (RSC-3.3b D3).
    #[error("flow payload conformance violation: rejected {rejected} message(s): {detail}")]
    PayloadConformance {
        /// Number of messages rejected since the window was last drained.
        rejected: u64,
        /// Per-flow breakdown of the rejections.
        detail: String,
    },

    /// One or more sends could not be routed at all: no declared flow
    /// matched the source key AND occurrence addressing found no accepting
    /// surface (RSC-3.3c D4 — the RSC-1.5 unrouted accounting joins the
    /// fail-hard surface of [`ExchangePlane::route_pending_checked`](crate::exchange::ExchangePlane::route_pending_checked); per
    /// ledger L15 the counters reach snapshots at RSC-3.5, not here).
    #[error("unrouted flow message(s): {count} send(s) had no declared flow and no accepting surface: {detail}")]
    Unrouted {
        /// Number of unrouted sends since the window was last drained.
        count: u64,
        /// Per-send breakdown (capped detail window).
        detail: String,
    },

    /// One or more occurrence-addressed MessageTransfers named a target
    /// that resolved to MULTIPLE accepting surfaces (RSC-3.3c D4). The
    /// router never picks first silently: the messages were withheld and
    /// strict mode fails hard.
    #[error("ambiguous message target(s): {count} send(s) named a target with multiple accepting surfaces: {detail}")]
    AmbiguousMessageTarget {
        /// Number of ambiguous sends since the window was last drained.
        count: u64,
        /// Per-send breakdown (capped detail window).
        detail: String,
    },
}

// ---------------------------------------------------------------------------
// RSC-3.3b D3 — payload-subset conformance (Transfers.kerml:100-118)
// ---------------------------------------------------------------------------
//
// What machinery exists for type conformance (researched 3.3b): sysml-core's
// `is_subtype_of` lattice is the ELEMENT-KIND lattice (codegen from the
// vocab TTLs), not a model-type lattice; there is no core "type name A
// conforms to type name B" primitive. Conformance here is therefore
// implemented two ways, per the design-doc instruction:
// 1. The **ScalarValues.kerml lattice** (spec-pinned, ScalarValues.kerml:11-22:
//    Positive :> Natural :> Integer :> Rational :> Real :> Complex :> Number
//    :> NumericalValue :> ScalarValue; Boolean/String :> ScalarValue), which
//    lets runtime `Value` kinds (int/float/bool/string) be checked against
//    declared scalar type names without a graph.
// 2. The **graph's specialization edges** (`Subclassification` children with
//    `unresolved_superclassifier`, and resolved `Specialize` relationships)
//    for model-defined type names, walked transitively at compile time.
//
// Both checks are PROVABLE-MISMATCH-ONLY: a name pair we cannot place in
// either lattice makes no claim (open world — the type may be defined in an
// unloaded library). Absent endpoint typing is no check at all (ledger L25).

/// Strip a qualified-name prefix: `"ScalarValues::Real"` → `"Real"`.
fn local_type_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// The declared scalar type names the runtime can reason about statically
/// (ScalarValues.kerml + the runtime `Value::type_name` spellings).
fn is_known_scalar_type(name: &str) -> bool {
    matches!(
        local_type_name(name),
        "int"
            | "float"
            | "bool"
            | "string"
            | "Boolean"
            | "String"
            | "Positive"
            | "Natural"
            | "Integer"
            | "Rational"
            | "Real"
            | "Complex"
            | "Number"
            | "NumericalValue"
            | "ScalarValue"
            | "DataValue"
    )
}

// `value_kind_conforms_to_scalar` / `value_kind_provably_nonconformant` were the
// FlowRouter route-time payload-conformance helpers; they moved with the routing
// backend to `exchange.rs` (RSC-3.5e.2). The compile-time NAME conformance
// (`type_name_conformance` etc., used by links.rs) stays here.

/// Compile-time half of the D3 check: tri-state conformance of type NAME
/// `actual` to type name `declared`.
///
/// - `Some(true)` — provably conformant: equal local names, a conforming
///   ScalarValues.kerml lattice pair, or a transitive specialization path in
///   the graph (`Espresso :> Beverage`).
/// - `Some(false)` — provably non-conformant: both names are known scalar
///   types and the lattice has no conformance path (e.g. `float` → `Integer`).
/// - `None` — no claim: at least one name is neither a known scalar nor
///   reachable via the graph's specialization edges (open world — the type
///   may be defined in an unloaded library).
pub(crate) fn type_name_conformance(
    graph: &ModelGraph,
    actual: &str,
    declared: &str,
) -> Option<bool> {
    let actual_local = local_type_name(actual);
    let declared_local = local_type_name(declared);
    if actual_local == declared_local {
        return Some(true);
    }
    if is_known_scalar_type(actual_local) && is_known_scalar_type(declared_local) {
        return Some(scalar_name_conforms(actual_local, declared_local));
    }
    // Non-scalar names: a specialization path proves conformance; its
    // absence proves nothing.
    if specializes_transitively(graph, actual_local, declared_local, 0) {
        return Some(true);
    }
    None
}

/// `true` exactly when [`type_name_conformance`] returns a provable
/// mismatch (`Some(false)`).
pub(crate) fn type_names_provably_nonconformant(
    graph: &ModelGraph,
    actual: &str,
    declared: &str,
) -> bool {
    type_name_conformance(graph, actual, declared) == Some(false)
}

/// Scalar-name-to-scalar-name conformance per ScalarValues.kerml:11-22.
fn scalar_name_conforms(actual: &str, declared: &str) -> bool {
    /// Supertype chain (inclusive) for each scalar name, normalised to the
    /// library spellings (`int`/`float`/`bool`/`string` are the runtime
    /// `Value::type_name` aliases of Integer/Real/Boolean/String).
    fn chain(name: &str) -> Option<&'static [&'static str]> {
        Some(match name {
            "Positive" => &[
                "Positive",
                "Natural",
                "Integer",
                "int",
                "Rational",
                "Real",
                "float",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "Natural" => &[
                "Natural",
                "Integer",
                "int",
                "Rational",
                "Real",
                "float",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "Integer" | "int" => &[
                "Integer",
                "int",
                "Rational",
                "Real",
                "float",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "Rational" => &[
                "Rational",
                "Real",
                "float",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "Real" | "float" => &[
                "Real",
                "float",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "Complex" => &[
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "Number" => &["Number", "NumericalValue", "ScalarValue", "DataValue"],
            "NumericalValue" => &["NumericalValue", "ScalarValue", "DataValue"],
            "Boolean" | "bool" => &["Boolean", "bool", "ScalarValue", "DataValue"],
            "String" | "string" => &["String", "string", "ScalarValue", "DataValue"],
            "ScalarValue" => &["ScalarValue", "DataValue"],
            "DataValue" => &["DataValue"],
            _ => return None,
        })
    }
    chain(actual).is_some_and(|c| c.contains(&declared))
}

/// Transitive specialization walk over the graph: does a type element named
/// `name` (anywhere in the graph) specialize — directly or transitively — a
/// type named `target`? Reads both the parser's `Subclassification` children
/// (`unresolved_superclassifier`) and resolved `Specialize` relationships.
fn specializes_transitively(graph: &ModelGraph, name: &str, target: &str, depth: u8) -> bool {
    if depth > 16 {
        return false;
    }
    let Some(elem) = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some(name) && (e.kind.is_definition() || e.kind.is_usage()))
    else {
        return false;
    };
    // Direct supertype names from both edge encodings.
    let mut supers: Vec<String> = Vec::new();
    for child in graph.children_of(&elem.id) {
        if child.kind == ElementKind::Subclassification {
            if let Some(Value::String(s)) = child.get_prop("unresolved_superclassifier") {
                supers.push(local_type_name(s).to_owned());
            }
        }
    }
    for rel in graph.outgoing(&elem.id) {
        if rel.kind == sysml_core::RelationshipKind::Specialize {
            if let Some(sup) = graph.get_element(&rel.target) {
                if let Some(n) = &sup.name {
                    supers.push(n.clone());
                }
            }
        }
    }
    supers.iter().any(|s| {
        s == target
            || scalar_name_conforms(s, target)
            || specializes_transitively(graph, s, target, depth + 1)
    })
}

// ---------------------------------------------------------------------------
// Port value binding
// ---------------------------------------------------------------------------

/// RSC-3.3b D2 — the move-semantics mirror of payload binding (the bind itself
/// lives on `ExchangePlane::bind_payload_to_port` since RSC-3.5e.2):
/// clear the SOURCE port's payload feature value(s) after a delivered
/// `is_move` transfer ("the payload leaves the source", Transfers.kerml
/// 84-90 + the `moving` connector 128-135).
///
/// Matching rule mirrors the bind:
/// - `Value::Map`: each named field clears the matching source feature
/// - simple values: the first feature is cleared
/// - `Value::Null` or unknown port: no-op
///
/// Where the source port carries no features there is nothing to clear and
/// delivery proceeds — this is the WHOLE exercised corpus today (every
/// corpus port compiles with zero features, ledger L23/L25), so move
/// clearing is observable only on feature-carrying ports.
///
/// The subset of a port's feature names that a delivered move-transfer clears,
/// per the move-semantics matching rule (Transfers.kerml 84-90, "the payload
/// leaves the source"): a `Value::Map` clears each named field the source
/// actually carries; a simple value clears the single (first) feature;
/// `Value::Null` clears nothing.
///
/// Pure — the storage-specific clear is the caller's job. This is the ONE home
/// for the match rule (CLAUDE.md #4/#5): both the registry-as-store path
/// ([`clear_payload_at_source`], used by `ExchangePlane::route_pending_with_ports`)
/// and the orchestrator's SlotStore path (`apply_move_semantics`) select the
/// cleared features through it. The two differ only in *where* the value lives
/// (registry feature vs. minted slot), not in *which* features move.
pub(crate) fn moved_feature_names(available: &[String], payload: &Value) -> Vec<String> {
    match payload {
        Value::Map(fields) => available
            .iter()
            .filter(|name| fields.contains_key(*name))
            .cloned()
            .collect(),
        Value::Null => Vec::new(),
        _ => available.first().cloned().into_iter().collect(),
    }
}

/// Registry-as-value-store move-clear: clear the SOURCE port's moved payload
/// feature value(s) directly in the [`PortRegistry`]. Used by
/// [`ExchangePlane::route_pending_with_ports`](crate::exchange::ExchangePlane::route_pending_with_ports),
/// which uses the (instance-keyed) registry as its value store — the clear is
/// symmetric with the `bind_payload_to_port` write on the target side. The
/// service flow-inspection path and the `route_pending_with_ports` conformance
/// gates depend on this in-registry clear.
///
/// NOTE: the production orchestrator does NOT use this path — its registry is
/// definition-keyed (shared across sibling instances; ledger L36) and must
/// stay immutable, so it clears the per-instance SlotStore instead (see
/// `Orchestrator::apply_move_semantics`). Both select features via the shared
/// [`moved_feature_names`] rule.
///
/// Returns the names of the features cleared.
pub(crate) fn clear_payload_at_source(
    registry: &mut PortRegistry,
    source_key: &str,
    payload: &Value,
) -> Vec<String> {
    let available: Vec<String> = match registry.get(source_key) {
        Some(port) => port.features.keys().cloned().collect(),
        None => return Vec::new(),
    };
    let cleared = moved_feature_names(&available, payload);
    if let Some(port) = registry.get_mut(source_key) {
        for name in &cleared {
            port.set_feature_value(name, Value::Null);
        }
    }
    cleared
}

// ---------------------------------------------------------------------------
// S2c — Action integration adapters
// ---------------------------------------------------------------------------

/// Descriptor for a message originating from an action send node.
///
/// This provides a crate-local abstraction for action-produced messages,
/// allowing interop with the routing backend without depending on the
/// `sysml-run-actions` crate.
#[derive(Debug, Clone)]
pub struct ActionSendDescriptor {
    /// The source endpoint key (e.g. `"sender.output"`).
    pub source_key: String,
    /// The payload value to send.
    pub payload: Value,
}

/// Descriptor for a message delivered to an action accept node.
///
/// Wraps a [`FlowMessage`] with the target endpoint key for convenient
/// consumption by action runners.
#[derive(Debug, Clone)]
pub struct ActionReceiveDescriptor {
    /// The target endpoint key that was read from.
    pub target_key: String,
    /// The delivered message payload.
    pub payload: Value,
    /// The originating flow connection id.
    pub flow_id: String,
    /// The source endpoint key.
    pub source_key: String,
}

// `action_send_to_router` / `router_receive_for_action` (the `&mut FlowRouter`
// action adapters) were deleted with FlowRouter in RSC-3.5e.2. The
// ExchangePlane routing backend is driven directly by the orchestrator;
// `ActionSendDescriptor` / `ActionReceiveDescriptor` remain as the action
// message-shape types.

// ---------------------------------------------------------------------------
// S2d — Flow compiler from ModelGraph
// ---------------------------------------------------------------------------

/// Compile port instances from a [`ModelGraph`] into a [`PortRegistry`].
///
/// Reads elaborated port properties (`portDefinition`, `effectiveDirection`,
/// `isConjugated`) set by `sysml_core::elaborate::ports::elaborate_ports()`.
/// Falls back to direct resolution if properties aren't elaborated yet.
///
/// This bridges sysml-core's type model into the runtime execution engine.
pub fn compile_ports(graph: &ModelGraph) -> PortRegistry {
    let mut registry = PortRegistry::new();

    for port_elem in graph.elements_by_kind(&ElementKind::PortUsage) {
        if let Some(port_ir) = compile_port_from_elaborated(port_elem, graph) {
            registry.register(port_ir);
        }
    }

    registry
}

/// Compile a PortInstanceIR from elaborated PortUsage properties.
///
/// The elaboration pass (sysml-core) resolves and tags:
/// - `portDefinition` — PortDefinition name
/// - `isConjugated` — conjugation state
/// - `effectiveDirection` — direction accounting for conjugation
///
/// This function reads those properties to build the runtime IR.
fn compile_port_from_elaborated(port_elem: &Element, graph: &ModelGraph) -> Option<PortInstanceIR> {
    let port_name = port_elem.name.as_deref()?;
    let owner_name = find_owner_name(port_elem, graph)?;

    let mut port_ir = PortInstanceIR::new(owner_name, port_name);

    // Read effectiveDirection (set by elaborate/ports.rs)
    // Falls back to "direction" property if not elaborated
    let dir_str = port_elem
        .get_prop("effectiveDirection")
        .or_else(|| port_elem.get_prop("direction"))
        .and_then(|v| v.as_str())
        .unwrap_or("undirected");
    port_ir.direction = match dir_str {
        "in" => PortDirection::In,
        "out" => PortDirection::Out,
        "inout" => PortDirection::InOut,
        _ => PortDirection::Undirected,
    };

    // Read portDefinition (set by elaborate/ports.rs)
    if let Some(def_name) = port_elem
        .get_prop("portDefinition")
        .and_then(|v| v.as_str())
    {
        port_ir.definition = Some(def_name.to_owned());
    }

    // Read isConjugated (set by elaborate/ports.rs)
    if let Some(true) = port_elem.get_prop("isConjugated").and_then(|v| v.as_bool()) {
        port_ir.is_conjugated = true;
    }

    // Extract features from children (PortDefinition + PortUsage)
    // Features aren't stored as elaborated properties — extract directly
    if let Some(def_id) = resolve_port_def_for_features(port_elem, graph) {
        for child in graph.children_of(&def_id) {
            if let Some(feature) = extract_port_feature(child, graph) {
                port_ir.add_feature(feature);
            }
        }
    }
    for child in graph.children_of(&port_elem.id) {
        if let Some(feature) = extract_port_feature(child, graph) {
            port_ir.add_feature(feature);
        }
    }

    // Extract multiplicity
    if let Some(mult) = port_elem.get_prop("multiplicity").and_then(|v| v.as_int()) {
        if mult > 0 {
            port_ir.multiplicity = Some(mult as usize);
        }
    }

    Some(port_ir)
}

/// Find the name of the part that owns this port by walking up the ownership chain.
fn find_owner_name(elem: &Element, graph: &ModelGraph) -> Option<String> {
    let mut current = elem.owner.clone();
    while let Some(owner_id) = current {
        if let Some(owner) = graph.get_element(&owner_id) {
            match owner.kind {
                ElementKind::PartUsage
                | ElementKind::PartDefinition
                | ElementKind::ItemUsage
                | ElementKind::ItemDefinition
                | ElementKind::ConnectionUsage
                | ElementKind::InterfaceUsage => {
                    return owner.name.clone();
                }
                ElementKind::OwningMembership | ElementKind::FeatureMembership => {
                    current = owner.owner.clone();
                }
                _ if owner.kind.is_usage() || owner.kind.is_definition() => {
                    return owner.name.clone();
                }
                _ => {
                    current = owner.owner.clone();
                }
            }
        } else {
            break;
        }
    }
    None
}

/// Resolve PortDefinition ID for feature extraction.
///
/// Uses the resolution module's find_feature_type (O(1) reverse index).
fn resolve_port_def_for_features(port_elem: &Element, graph: &ModelGraph) -> Option<ElementId> {
    find_feature_type(graph, &port_elem.id)
}

/// Extract a PortFeature from a child element of a port definition/usage.
///
/// `graph` is needed to resolve the feature's declared type into
/// `type_name` (RSC-3.3a D3): a parser-produced graph records the typing as a
/// `FeatureTyping` child / `unresolvedTypeName` prop, never the `typeName`
/// prop the original code read — so without this the field was dead corpus-wide
/// (the L23/L25 "value dead-end"). Resolution goes through the one home
/// `compiler::resolve_attribute_type_name` that `infer_m_ref` already uses.
fn extract_port_feature(child: &Element, graph: &ModelGraph) -> Option<PortFeature> {
    match child.kind {
        ElementKind::AttributeUsage
        | ElementKind::ReferenceUsage
        | ElementKind::ItemUsage
        | ElementKind::PortUsage => {}
        _ => return None,
    }

    let name = child.name.as_deref()?;

    let direction = match child
        .get_prop("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("undirected")
    {
        "in" => PortDirection::In,
        "out" => PortDirection::Out,
        "inout" => PortDirection::InOut,
        _ => PortDirection::Undirected,
    };

    let value = child
        .get_prop("default")
        .or_else(|| child.get_prop("value"))
        .cloned()
        .unwrap_or(Value::Null);

    // Hand-built test graphs stamp the `typeName` prop directly; parser-produced
    // graphs never do — they record the typing as a `FeatureTyping` child /
    // `unresolvedTypeName` prop, so fall through to the real resolver. This is a
    // priority ordering between two legitimate sources, NOT a soft fallback.
    let type_name = child
        .get_prop("typeName")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| crate::compiler::resolve_attribute_type_name(graph, child));

    Some(PortFeature {
        name: name.to_owned(),
        direction,
        type_name,
        value,
    })
}

// ---------------------------------------------------------------------------
// Flow extraction helpers (RSC-3.5e.5 W4)
//
// `compile_flows` / `compile_flows_with_registry` / `compile_single_flow` were
// deleted: `crate::links::classify_links` now walks the flow elements itself
// and builds the `LinkIR` inline, calling the three `pub(crate)` helpers below
// (`parse_endpoint`, `read_bool_child`, `derive_endpoint_payload_type`) for the
// per-endpoint extraction the producer used to do.
// ---------------------------------------------------------------------------

/// Read a boolean override from a child `AttributeUsage` named `name`
/// (e.g. `attribute isMove = false;` in a flow body — see the RSC-3.B0 grammar
/// research). Returns `None` when no such child exists (caller applies the
/// spec default).
///
/// The probe (RSC-3.3a) confirmed the parser lowers `attribute isMove = false;`
/// to a child `AttributeUsage{ name: "isMove", value: Bool(false) }` with a
/// `LiteralBoolean` grandchild also carrying `value: Bool(false)`. The tree-
/// sitter parser writes the typed `value` prop (it does NOT write the legacy
/// `unresolved_value` string for booleans — Phase 6D), so the boolean is read
/// from the typed `value` prop on the attribute, falling back to a literal
/// child's typed `value`.
pub(crate) fn read_bool_child(element: &Element, graph: &ModelGraph, name: &str) -> Option<bool> {
    let child = graph
        .children_of(&element.id)
        .find(|c| c.kind == ElementKind::AttributeUsage && c.name.as_deref() == Some(name))?;

    // 1. Typed boolean `value` prop directly on the AttributeUsage.
    if let Some(Value::Bool(b)) = child.get_prop("value") {
        return Some(*b);
    }
    // 2. A literal (e.g. LiteralBoolean) child carrying a typed boolean `value`.
    graph
        .children_of(&child.id)
        .find_map(|g| match g.get_prop("value") {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        })
}

/// Derive the payload type for one flow endpoint from the port registry's
/// feature typing (RSC-3.3a D3). `is_source` selects the OUT-facing feature
/// (source output) vs the IN-facing feature (target input); falls back to the
/// port's sole feature typing when no directional match exists.
pub(crate) fn derive_endpoint_payload_type(
    registry: &PortRegistry,
    endpoint: &FlowEndpoint,
    is_source: bool,
) -> Option<String> {
    let key = endpoint.key();
    let port = registry
        .get(&key)
        .or_else(|| registry.find_by_port_name(&endpoint.port))?;

    // Effective per-feature direction is post-conjugation: a conjugated port
    // reverses each feature's declared direction.
    let conjugate = port.is_conjugated;
    let want = if is_source {
        PortDirection::Out
    } else {
        PortDirection::In
    };

    // Prefer a feature whose effective direction matches what we want
    // (source output / target input), then InOut, then any typed feature.
    let mut directional: Option<&str> = None;
    let mut inout: Option<&str> = None;
    let mut any: Option<&str> = None;
    for feat in port.features.values() {
        let eff = if conjugate {
            feat.direction.conjugate()
        } else {
            feat.direction
        };
        let ty = feat.type_name.as_deref();
        if ty.is_none() {
            continue;
        }
        if eff == want && directional.is_none() {
            directional = ty;
        } else if eff == PortDirection::InOut && inout.is_none() {
            inout = ty;
        } else if any.is_none() {
            any = ty;
        }
    }
    directional.or(inout).or(any).map(str::to_owned)
}

/// Parse an endpoint string of the form `"participant.port"` into a
/// [`FlowEndpoint`].
///
/// If no dot separator is found, the entire string is used as the
/// participant with an empty port.
pub(crate) fn parse_endpoint(s: &str) -> FlowEndpoint {
    if let Some((participant, port)) = s.rsplit_once('.') {
        FlowEndpoint::new(participant, port)
    } else {
        FlowEndpoint::new(s, "")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::links::{classify_links, LinkIR, LinkSourceKind};

    /// RSC-3.5e.5 W4: `compile_flows` is gone — the producer is folded into
    /// `classify_links`. These tests assert the same per-flow extraction on the
    /// FlowUsage subset of the classified `LinkGraph` (interning order == the
    /// former `compile_flows` order). `LinkEndpoint.owner`/`port` mirror the old
    /// `FlowEndpoint.participant`/`port`; the flow id is `LinkIR::display_label`.
    fn flow_links(graph: &ModelGraph) -> Vec<LinkIR> {
        let reg = compile_ports(graph);
        let (lg, _diags) = classify_links(graph, &reg);
        lg.iter()
            .filter(|l| l.kind == LinkSourceKind::FlowUsage)
            .cloned()
            .collect()
    }

    // RSC-3.5e.2: the FlowRouter core-routing / succession / type-check unit
    // tests + the S2c action-adapter tests were deleted with FlowRouter.
    // Covered 1:1 by the ExchangePlane parity tests in `exchange.rs`:
    //   basic_routing                       → parity_basic_routing
    //   multicast_routing                   → parity_multicast_routing
    //   receive_consumes_message            → parity_receive_consumes_message
    //   unroutable_message_dropped          → parity_unroutable_message_dropped
    //   reset_clears_state                  → parity_reset_clears_state
    //   succession_blocks_until_source_complete / succession_unblocked_after_completion
    //                                       → parity_succession_ordering
    //   non_succession_delivers_immediately → parity_basic_routing (non-succession default)
    //   type_mismatch_rejected              → parity_type_mismatch_rejected
    //   compatible_type_accepted            → parity_compatible_type_accepted
    //   untyped_flow_accepts_all            → parity_untyped_flow_accepts_all
    //   action_message_routed_through_flow / flow_delivers_to_action_inbox /
    //   send_route_accept_end_to_end (adapters deleted)
    //                                       → parity_basic_routing + parity_receive_consumes_message

    // -----------------------------------------------------------------------
    // S2d — Compiler from ModelGraph
    // -----------------------------------------------------------------------

    #[test]
    fn compile_flow_from_graph() {
        let mut graph = ModelGraph::new();

        // Create a FlowUsage element with source and target props
        let flow_elem = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("dataFlow")
            .with_prop("source", "sensor.reading")
            .with_prop("target", "controller.input");
        graph.add_element(flow_elem);

        let compiled = flow_links(&graph);
        assert_eq!(compiled.len(), 1);

        let flow = &compiled[0];
        assert_eq!(flow.display_label(&graph), "dataFlow");
        assert_eq!(flow.source.owner, "sensor");
        assert_eq!(flow.source.port, "reading");
        assert_eq!(flow.target.owner, "controller");
        assert_eq!(flow.target.port, "input");
        assert!(!flow.is_succession);
        assert!(flow.payload_type.is_none());
    }

    #[test]
    fn compile_item_flow() {
        let mut graph = ModelGraph::new();

        // KerML Flow (item flow) with a typed payload
        let flow_elem = Element::new_with_kind(ElementKind::Flow)
            .with_name("tempTransfer")
            .with_prop("source", "thermometer.temp")
            .with_prop("target", "display.value")
            .with_prop("payloadType", "float");
        graph.add_element(flow_elem);

        let compiled = flow_links(&graph);
        assert_eq!(compiled.len(), 1);

        let flow = &compiled[0];
        assert_eq!(flow.display_label(&graph), "tempTransfer");
        assert!(!flow.is_succession);
        assert_eq!(flow.payload_type.as_deref(), Some("float"));
    }

    #[test]
    fn compile_succession_flow() {
        let mut graph = ModelGraph::new();

        // SuccessionFlowUsage — should set is_succession = true
        let flow_elem = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
            .with_name("orderedFlow")
            .with_prop("source", "step1.out")
            .with_prop("target", "step2.in");
        graph.add_element(flow_elem);

        let compiled = flow_links(&graph);
        assert_eq!(compiled.len(), 1);

        let flow = &compiled[0];
        assert_eq!(flow.display_label(&graph), "orderedFlow");
        assert!(flow.is_succession);
        assert_eq!(flow.source.owner, "step1");
        assert_eq!(flow.source.port, "out");
        assert_eq!(flow.target.owner, "step2");
        assert_eq!(flow.target.port, "in");
    }

    // -----------------------------------------------------------------------
    // RSC-3.3a — isMove/isPush lowering (defaults true + child overrides) + D3
    // payload-typing derivation. Spec: Transfers.kerml FlowTransfer
    // (isMove:84, isPush:92 — both `default true`).
    // -----------------------------------------------------------------------

    /// A flow with no body compiles with both spec defaults TRUE.
    #[test]
    fn lowering_ismove_ispush_default_true() {
        let mut graph = ModelGraph::new();
        graph.add_element(
            Element::new_with_kind(ElementKind::FlowUsage)
                .with_name("f")
                .with_prop("source", "a.out")
                .with_prop("target", "b.in"),
        );
        let compiled = flow_links(&graph);
        assert_eq!(compiled.len(), 1);
        assert!(
            compiled[0].is_move,
            "isMove defaults to true (Transfers.kerml:84)"
        );
        assert!(
            compiled[0].is_push,
            "isPush defaults to true (Transfers.kerml:92)"
        );
    }

    /// A child `attribute isMove = false;` / `isPush = false;` (typed Bool
    /// `value` prop — what the parser produces, per the RSC-3.3a probe)
    /// overrides BOTH defaults.
    #[test]
    fn lowering_child_attribute_overrides_both_flags_false() {
        let mut graph = ModelGraph::new();
        let flow_id = ElementId::new_v4();
        graph.add_element(
            Element::new(flow_id.clone(), ElementKind::FlowUsage)
                .with_name("f")
                .with_prop("source", "a.out")
                .with_prop("target", "b.in"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(flow_id.clone())
                .with_name("isMove")
                .with_prop("value", Value::Bool(false)),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(flow_id)
                .with_name("isPush")
                .with_prop("value", Value::Bool(false)),
        );
        let compiled = flow_links(&graph);
        assert_eq!(compiled.len(), 1);
        assert!(!compiled[0].is_move, "explicit isMove=false honored");
        assert!(!compiled[0].is_push, "explicit isPush=false honored");
    }

    /// An explicit `isMove = true` child is also honored (and equals the
    /// default — covers the typed-bool read path independent of the default).
    #[test]
    fn lowering_child_attribute_override_true_honored() {
        let mut graph = ModelGraph::new();
        let flow_id = ElementId::new_v4();
        graph.add_element(
            Element::new(flow_id.clone(), ElementKind::FlowUsage)
                .with_name("f")
                .with_prop("source", "a.out")
                .with_prop("target", "b.in"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(flow_id)
                .with_name("isMove")
                .with_prop("value", Value::Bool(true)),
        );
        let compiled = flow_links(&graph);
        assert!(compiled[0].is_move);
    }

    /// The literal-child fallback: an `isPush` AttributeUsage with no typed
    /// `value` prop but a `LiteralBoolean` child carrying `value: Bool(false)`
    /// is still read as an override (the parser also produces this grandchild).
    #[test]
    fn lowering_child_attribute_literal_child_override() {
        let mut graph = ModelGraph::new();
        let flow_id = ElementId::new_v4();
        graph.add_element(
            Element::new(flow_id.clone(), ElementKind::FlowUsage)
                .with_name("f")
                .with_prop("source", "a.out")
                .with_prop("target", "b.in"),
        );
        let attr_id = ElementId::new_v4();
        graph.add_element(
            Element::new(attr_id.clone(), ElementKind::AttributeUsage)
                .with_owner(flow_id)
                .with_name("isPush"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::LiteralBoolean)
                .with_owner(attr_id)
                .with_prop("value", Value::Bool(false)),
        );
        let compiled = flow_links(&graph);
        assert!(
            !compiled[0].is_push,
            "literal-child Bool(false) override honored"
        );
        assert!(compiled[0].is_move, "absent isMove still defaults true");
    }

    /// Lowering reads the override from the real parser output for
    /// `attribute isMove = false;` in a flow body (end-to-end probe pin).
    #[test]
    fn lowering_parser_flow_body_attribute_override() {
        use sysml_parser_incremental::TreeSitterParser;
        use sysml_parser_trait::{Parser, SysmlFile};
        let source = r#"
            package P {
                part s; part k;
                flow from s.out to k.inp {
                    attribute isMove = false;
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("lowering.sysml", source.to_owned())]);
        let errs: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == sysml_span::Severity::Error)
            .collect();
        assert!(errs.is_empty(), "fixture must parse: {errs:?}");
        let compiled = flow_links(&result.graph);
        assert_eq!(compiled.len(), 1);
        assert!(
            !compiled[0].is_move,
            "body `attribute isMove = false;` lowers to is_move=false"
        );
        assert!(compiled[0].is_push, "isPush absent -> default true");
    }

    /// D3 lowering half: payload types are derived from the source-output and
    /// target-input port feature typings via the registry.
    #[test]
    fn lowering_derives_payload_types_from_port_typing() {
        use crate::flows::port::{PortFeature, PortInstanceIR};
        // Build a registry directly with directional, typed features.
        let mut reg = PortRegistry::new();
        let mut src = PortInstanceIR::new("producer", "outPort");
        src.add_feature(PortFeature {
            name: "datum".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(src);
        let mut tgt = PortInstanceIR::new("consumer", "inPort");
        tgt.add_feature(PortFeature {
            name: "datum".into(),
            direction: PortDirection::In,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(tgt);

        let endpoint_src = FlowEndpoint::new("producer", "outPort");
        let endpoint_tgt = FlowEndpoint::new("consumer", "inPort");
        assert_eq!(
            derive_endpoint_payload_type(&reg, &endpoint_src, true).as_deref(),
            Some("Real"),
            "source-output feature typing derived"
        );
        assert_eq!(
            derive_endpoint_payload_type(&reg, &endpoint_tgt, false).as_deref(),
            Some("Real"),
            "target-input feature typing derived"
        );
    }

    /// `payload_type` (the route-check field) stays sourced from the explicit
    /// `payloadType` prop only — derivation populates the new fields, NOT the
    /// route field (so 3.3a routing is inert).
    #[test]
    fn lowering_route_payload_type_stays_explicit_only() {
        let mut graph = ModelGraph::new();
        graph.add_element(
            Element::new_with_kind(ElementKind::FlowUsage)
                .with_name("f")
                .with_prop("source", "a.out")
                .with_prop("target", "b.in"),
        );
        let compiled = flow_links(&graph);
        // No explicit payloadType prop and no port registry typing -> route
        // field stays None (the existing widening check is untouched).
        assert_eq!(compiled[0].payload_type, None);
    }

    #[test]
    fn flow_missing_source_is_skipped() {
        // RSC-3.5e.5 W4: the former `compile_flows` returned `Err` on a flow
        // missing `source`; the folded `classify_links` skips it per-element
        // (matching the connector loop), so no link is produced for it.
        let mut graph = ModelGraph::new();

        let flow_elem = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("badFlow")
            .with_prop("target", "b.in");
        graph.add_element(flow_elem);

        assert!(flow_links(&graph).is_empty());
    }

    #[test]
    fn compile_multiple_flows() {
        let mut graph = ModelGraph::new();

        let f1 = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("flow1")
            .with_prop("source", "a.out")
            .with_prop("target", "b.in");
        graph.add_element(f1);

        let f2 = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
            .with_name("flow2")
            .with_prop("source", "b.out")
            .with_prop("target", "c.in");
        graph.add_element(f2);

        let f3 = Element::new_with_kind(ElementKind::Flow)
            .with_name("flow3")
            .with_prop("source", "x.data")
            .with_prop("target", "y.data")
            .with_prop("payloadType", "int");
        graph.add_element(f3);

        let compiled = flow_links(&graph);
        assert_eq!(compiled.len(), 3);

        // Verify succession flag
        let succession_count = compiled.iter().filter(|f| f.is_succession).count();
        assert_eq!(succession_count, 1);

        // Verify typed flow
        let typed_count = compiled.iter().filter(|f| f.payload_type.is_some()).count();
        assert_eq!(typed_count, 1);
    }

    // RSC-3.5e.2: the FlowRouter bounded-queue / strict-vs-lossy / drop-counter
    // unit tests + the M3 action-adapter integration test were deleted with
    // FlowRouter. Covered 1:1 by the ExchangePlane parity tests in `exchange.rs`:
    //   bounded_queue_drops_oldest_when_full      → parity_bounded_queue_eviction_and_counters
    //   capacity_eviction_increments_drop_counters → parity_bounded_queue_eviction_and_counters
    //   strict_mode_capacity_overflow_is_an_error  → parity_strict_mode_capacity_error
    //   lossy_mode_capacity_overflow_warns_only    → parity_lossy_mode_warn_only
    //   unrouted_drops_counted_separately_from_capacity_drops → parity_unrouted_vs_capacity_separation
    //   router_reset_clears_drop_counters → parity_reset_clears_state + parity_bounded_queue_eviction_and_counters
    //   with_max_queue_size_sets_custom_limit / default_router_has_1000_limit → parity_max_queue_size_defaults
    //   action_sends_message_through_flow (adapter deleted) → parity_basic_routing + parity_receive_consumes_message

    // RSC-3.5e.2: the FlowRouter port-aware value-binding tests
    // (route_with_ports_*) were deleted with FlowRouter — covered 1:1 by the
    // ExchangePlane parity tests in `exchange.rs`:
    //   route_with_ports_binds_map_payload    → parity_route_with_ports_map_payload
    //   route_with_ports_binds_simple_payload → parity_route_with_ports_simple_payload
    //   route_with_ports_none_registry_is_noop → parity_route_with_ports_none_registry_noop
    // (the `make_water_port` helper went with them; exchange.rs has its own `water_port`.)

    // -----------------------------------------------------------------------
    // parse_endpoint — 3-segment path split correctness
    // -----------------------------------------------------------------------

    #[test]
    fn parse_endpoint_two_segment() {
        let ep = parse_endpoint("busbar.phaseIn");
        assert_eq!(ep.participant, "busbar");
        assert_eq!(ep.port, "phaseIn");
    }

    #[test]
    fn parse_endpoint_three_segment() {
        let ep = parse_endpoint("circuit1.breaker.phaseIn");
        assert_eq!(ep.participant, "circuit1.breaker");
        assert_eq!(ep.port, "phaseIn");
    }

    #[test]
    fn parse_endpoint_one_segment() {
        let ep = parse_endpoint("standalone");
        assert_eq!(ep.participant, "standalone");
        assert_eq!(ep.port, "");
    }

    // RSC-3.5e.2: the FlowRouter MessageTransfer-routing (D4) and pull-init
    // (U1) unit tests were deleted with FlowRouter. Their behaviour is covered
    // 1:1 by the ExchangePlane parity tests in `exchange.rs`:
    //   message_transfer_routes_to_named_acceptor → parity_message_transfer_named_acceptor
    //   message_transfer_participant_addressing_resolves_single_surface
    //                                            → parity_message_transfer_participant_addressing
    //   message_transfer_ambiguous_target_fails_loud → parity_message_transfer_ambiguous_fails_loud
    //   message_transfer_zero_acceptors_is_strict_loss
    //                                            → parity_message_transfer_zero_acceptors_strict_loss
    //   declared_flow_wins_over_occurrence_addressing
    //                                            → parity_declared_flow_wins_over_occurrence_addressing
    //   pull_link_suppresses_eager_delivery_until_pull → parity_pull_suppress_until_pull
    //   pull_round_trip_through_action_accept_adapter → parity_pull_suppress_until_pull (adapter deleted)
    //   pull_store_cleared_by_reset_acceptors_survive
    //                                            → parity_pull_suppress_until_pull + parity_reset_clears_state
}
