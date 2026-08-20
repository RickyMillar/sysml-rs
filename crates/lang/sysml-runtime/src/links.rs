//! RSC-3.1 — the classified link graph (design doc
//! §3 D-3.0.1).
//!
//! One element-keyed, compiled representation of every exchange edge in the
//! model — flows, connections, interface connections, bindings — classified
//! into one of three execution classes (PowerBond / SignalLink /
//! MessageChannel) plus an `Unknown` fallback. The pass is **additive**: in
//! RSC-3.1 nothing consumes the graph beyond `flow_inspect`'s additive
//! `link_class` / `via_interface` fields. Subsequent waves (3.2 SignalLink,
//! 3.3 MessageChannel, 3.4 PowerBond) re-point execution at it. RSC-3.5e.5 W4
//! deleted the legacy `FlowConnectionIR` compatibility view: `classify_links`
//! loop 1 now walks the flow elements and builds the `LinkIR` directly, so the
//! classified `LinkGraph` is the single link representation.
//!
//! Classification follows the ADR D3 precedence **declared > inferred >
//! unknown**:
//! 1. **Declared** — `@Signal`/`@SignalPort`/`@PowerPort` metadata on either
//!    endpoint's port definition (the Phase-1 [`classify_port_definition`]
//!    machinery, which already folds the metadata override into
//!    [`ClassificationConfidence::Declared`]). Conflicting declared ends
//!    (one declared-signal, one declared-power) fail loud (diagnostic) and
//!    fall through to `Unknown`.
//! 2. **Inferred** — both endpoints power-classified in the same/compatible
//!    domain → `PowerBond`; either endpoint signal-classified → `SignalLink`;
//!    item/event-typed or unclassified endpoints → `MessageChannel`.
//! 3. **Unknown** — neither declared nor inferable → `LinkClass::Unknown`,
//!    emitting **FL017** (warning) and routing as a `MessageChannel` (the
//!    conservative default, stated in the message).

use std::collections::{HashMap, HashSet};

use smallvec::SmallVec;
use sysml_core::physics::classify::{classify_port_definition, ClassificationConfidence};
use sysml_core::physics::domain::PhysicsDomainRegistry;
use sysml_core::resolution::scoping::chaining::find_feature_type;
use sysml_core::{ElementId, ElementKind, ModelGraph};
use sysml_span::Diagnostic;

use crate::flows::port::PortRegistry;
use crate::slots::{SlotId, SlotStore};

// ---------------------------------------------------------------------------
// Interning
// ---------------------------------------------------------------------------

/// Dense interned identifier for a [`LinkIR`] in a [`LinkGraph`].
///
/// Mirrors the [`SlotId`](crate::slots::SlotId) pattern — integer keys at tick
/// time, dense arrays, strings only at serialization. The exchange plane
/// (RSC-3.5) keys its message queues and routing tables on `LinkId`
/// exclusively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LinkId(pub u32);

impl LinkId {
    /// The dense array index this id refers to.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// What kind of model element this link was compiled from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkSourceKind {
    /// A `FlowUsage` / `SuccessionFlowUsage` (the `compile_flows` source).
    FlowUsage,
    /// A `ConnectionUsage` / `ConnectorAsUsage`.
    ConnectionUsage,
    /// An `InterfaceUsage` (interface connection).
    InterfaceUsage,
    /// A `BindingConnector` (equality constraint).
    Binding,
}

/// The execution class of a link (the three-class plane, D-3.0.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkClass {
    /// Conjugate effort/flow power exchange — fed to the DAE machinery
    /// (RSC-3.4). Both endpoints power-classified in a compatible domain.
    PowerBond,
    /// Directed signal propagation (RSC-3.2). At least one endpoint is
    /// signal-classified (incomplete conjugate pair / `@Signal`).
    SignalLink,
    /// Spec `Transfer` message channel (RSC-3.3). Item/event-typed or
    /// unclassified endpoints. Also the conservative routing default for
    /// `Unknown`-class links.
    MessageChannel,
    /// Class could not be resolved — FL017. Routes as a `MessageChannel`.
    Unknown,
}

impl LinkClass {
    /// Stable lowercase wire spelling for `flow_inspect` / snapshots.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkClass::PowerBond => "power_bond",
            LinkClass::SignalLink => "signal_link",
            LinkClass::MessageChannel => "message_channel",
            LinkClass::Unknown => "unknown",
        }
    }

    /// Whether a link of this class is routed as a discrete message — i.e.
    /// participates in message/sequence semantics. PowerBond links are
    /// continuous effort/flow physics fed to the DAE machinery, not messages,
    /// so they are excluded. Signal/MessageChannel/Unknown all route as
    /// messages (Unknown routes conservatively as a message channel).
    pub fn routes_as_message(self) -> bool {
        !matches!(self, LinkClass::PowerBond)
    }
}

/// One end of a link. `element_id` is the resolved port/feature element when
/// elaboration recovered it; `owner`/`port` are the (participant, port) name
/// strings that survive `compile_flows`' string flattening.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkEndpoint {
    /// Resolved port/feature element id, when recoverable.
    pub element_id: Option<ElementId>,
    /// Owning participant (part) name.
    pub owner: String,
    /// Port / feature name.
    pub port: String,
    /// The [`PortRegistry`] key this endpoint resolves to on the registry's
    /// **definition-owner** basis (e.g. `"SensorDef.currentOut"`), reconstructed
    /// from the participant usage's resolved type in [`resolve_endpoint`].
    ///
    /// The registry is keyed per-definition (ports are definition-owned), while
    /// this endpoint's [`key`](Self::key) is the per-instance spelling
    /// (`"okSensor.currentOut"`) that slots are minted under. This field bridges
    /// the two bases so feature-name discovery (signal propagation, slot
    /// minting) can hit the registry without the ambiguous
    /// [`PortRegistry::find_by_port_name`] name-only fallback. `None` when the
    /// participant type is unresolved (elaboration incomplete) — callers then
    /// fail rather than guess. See ledger L36 (def-vs-instance registry basis).
    pub resolved_registry_key: Option<String>,
}

impl LinkEndpoint {
    /// `owner.port` short key (the `FlowEndpoint::key` spelling) — the
    /// **per-instance** routing/slot key.
    pub fn key(&self) -> String {
        format!("{}.{}", self.owner, self.port)
    }

    /// The key to read this endpoint's port from the [`PortRegistry`]: the
    /// resolved definition-owner key when available, else the per-instance
    /// [`key`](Self::key) (which matches the registry for usage-owned ports).
    pub fn registry_key(&self) -> String {
        self.resolved_registry_key
            .clone()
            .unwrap_or_else(|| self.key())
    }
}

/// The feature names of an endpoint's port, read from the **definition-keyed**
/// [`PortRegistry`] via the endpoint's [`registry_key`](LinkEndpoint::registry_key)
/// (ports are definition-owned; ledger L36). Empty when the port is unknown to
/// the registry or carries no features.
///
/// This is the single home for endpoint→feature-name discovery: signal
/// propagation, signal-feature slot minting, and move-clear all route through
/// it so the def-vs-instance bridge is expressed exactly once. The
/// one site that previously derived this keying by hand — the orchestrator's
/// move-clear — was reading the registry with the per-*instance* key and so
/// always missed (the bug this consolidation closes).
pub(crate) fn endpoint_feature_names(
    registry: &PortRegistry,
    endpoint: &LinkEndpoint,
) -> Vec<String> {
    registry
        .get(&endpoint.registry_key())
        .map(|p| p.features.keys().cloned().collect())
        .unwrap_or_default()
}

/// One classified exchange edge — the only link representation (collapse-hard,
/// D-3.0.1). For flow-derived links, `classify_links` loop 1 extracts the
/// transfer fields directly from the flow element (RSC-3.5e.5 W4 folded the
/// former `compile_flows` producer in); the spec `isMove`/`isPush` defaults
/// (Transfers.kerml — both `default true`) are applied there.
#[derive(Clone, Debug)]
pub struct LinkIR {
    /// The connector / flow element this link was compiled from.
    pub element_id: ElementId,
    /// Which model element kind produced this link.
    pub kind: LinkSourceKind,
    /// Source endpoint.
    pub source: LinkEndpoint,
    /// Target endpoint.
    pub target: LinkEndpoint,
    /// Resolved execution class.
    pub class: LinkClass,
    /// Confidence of the class assignment (surfaced from the weakest
    /// endpoint, or `Declared` when metadata decided / `Unknown` when neither
    /// endpoint classified).
    pub class_confidence: ClassificationConfidence,
    /// Succession-ordered flow (`SuccessionFlowUsage`).
    pub is_succession: bool,
    /// Spec `FlowTransfer.isMove` (default true; overridden by a child
    /// `attribute isMove = <bool>;` on the flow element). `false` for connectors.
    pub is_move: bool,
    /// Spec `FlowTransfer.isPush` (default true; overridden by a child
    /// `attribute isPush = <bool>;` on the flow element). `false` for connectors.
    pub is_push: bool,
    /// Payload type — the route-time check field, from the flow element's
    /// explicit `payloadType` prop only (RSC-3.3a keeps routing inert).
    pub payload_type: Option<String>,
    /// Derived source-output payload typing (`Transfers.kerml` `sourceOutput`),
    /// from the source port's feature typing (RSC-3.3a D3). `None` when not
    /// recoverable.
    pub source_payload_type: Option<String>,
    /// Derived target-input payload typing (`Transfers.kerml` `targetInput`),
    /// from the target port's feature typing (RSC-3.3a D3). `None` when not
    /// recoverable.
    pub target_payload_type: Option<String>,
    /// The declared interface element satisfying the topology, when known
    /// (D-3.0.4 D5 records the satisfying connector here).
    pub via_interface: Option<ElementId>,
}

impl LinkIR {
    /// Human-facing label for this link, reproducing the legacy
    /// `FlowConnectionIR::id` (the originating element's name, or its id
    /// string when unnamed). Used by the FL010/FL014/FL016 port diagnostics
    /// (RSC-3.5e.5 W2 — the diagnostics consume the `LinkGraph` directly and
    /// recover the flow's display id from the graph rather than carrying a
    /// `FlowConnectionIR`). Byte-identical to `compile_flow`'s `id`:
    /// `element.name.unwrap_or_else(|| element.id.to_string())` — even under a
    /// name collision the label is the name, and unnamed flows resolve to the
    /// same element id.
    pub fn display_label(&self, graph: &ModelGraph) -> String {
        graph
            .get_element(&self.element_id)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| self.element_id.to_string())
    }

    /// Build a conservative `MessageChannel` flow-derived link between two raw
    /// `"owner.port"` endpoints, with a fresh element id and the spec default
    /// transfer flags (`is_move`/`is_push = true`, `is_succession = false`).
    ///
    /// This is the incremental builder for tests + callers that feed an
    /// [`ExchangePlane`] one link at a time via
    /// [`ExchangePlane::add_flow`](crate::exchange::ExchangePlane::add_flow) — the
    /// per-link dual of building a whole classified [`LinkGraph`] and calling
    /// [`ExchangePlane::ingest_classified`](crate::exchange::ExchangePlane::ingest_classified).
    /// Tweak the public fields afterwards for succession / typed-payload cases.
    /// (Production routing is class-agnostic, so the `MessageChannel` default is
    /// what the former `FlowConnectionIR`→`link_ir_from_flow` path interned.)
    pub fn message_channel(
        src_owner: impl Into<String>,
        src_port: impl Into<String>,
        tgt_owner: impl Into<String>,
        tgt_port: impl Into<String>,
    ) -> LinkIR {
        LinkIR {
            element_id: ElementId::new_v4(),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: src_owner.into(),
                port: src_port.into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: tgt_owner.into(),
                port: tgt_port.into(),
                resolved_registry_key: None,
            },
            class: LinkClass::MessageChannel,
            class_confidence: ClassificationConfidence::Unknown,
            is_succession: false,
            is_move: true,
            is_push: true,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        }
    }
}

/// The compiled, element-keyed, class-partitioned link graph.
///
/// Dense `Vec<LinkIR>` indexed by [`LinkId`], with a back-index from the
/// originating element id and a class-partitioned index for the per-class
/// execution passes (3.2/3.3/3.4) to claim their subset without re-scanning.
#[derive(Clone, Debug, Default)]
pub struct LinkGraph {
    links: Vec<LinkIR>,
    by_element_id: HashMap<ElementId, LinkId>,
    by_class: HashMap<LinkClassKey, Vec<LinkId>>,
}

/// Hashable key for the class-partition index (`LinkClass` is `Copy` but we
/// keep the index keyed by a small enum copy).
type LinkClassKey = u8;

fn class_key(class: LinkClass) -> LinkClassKey {
    match class {
        LinkClass::PowerBond => 0,
        LinkClass::SignalLink => 1,
        LinkClass::MessageChannel => 2,
        LinkClass::Unknown => 3,
    }
}

impl LinkGraph {
    /// Empty graph.
    pub fn new() -> Self {
        LinkGraph::default()
    }

    /// Intern a link, returning its dense [`LinkId`]. Maintains the
    /// element-id back-index and the class partition.
    pub fn intern(&mut self, link: LinkIR) -> LinkId {
        let id = LinkId(self.links.len() as u32);
        self.by_element_id.insert(link.element_id.clone(), id);
        self.by_class
            .entry(class_key(link.class))
            .or_default()
            .push(id);
        self.links.push(link);
        id
    }

    /// Number of links.
    pub fn len(&self) -> usize {
        self.links.len()
    }

    /// Whether the graph holds no links.
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Borrow a link by its dense id.
    pub fn get(&self, id: LinkId) -> Option<&LinkIR> {
        self.links.get(id.index())
    }

    /// Look up the link compiled from a given model element.
    pub fn by_element(&self, element_id: &ElementId) -> Option<&LinkIR> {
        self.by_element_id
            .get(element_id)
            .and_then(|id| self.get(*id))
    }

    /// Iterate all links in interning order.
    pub fn iter(&self) -> impl Iterator<Item = &LinkIR> {
        self.links.iter()
    }

    /// The interned ids of every link of a given class (the per-class
    /// execution passes consume these subsets).
    pub fn ids_of_class(&self, class: LinkClass) -> &[LinkId] {
        self.by_class
            .get(&class_key(class))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Distribution of links by class (for diagnostics / reporting).
    pub fn class_distribution(&self) -> ClassDistribution {
        let mut d = ClassDistribution::default();
        for link in &self.links {
            match link.class {
                LinkClass::PowerBond => d.power_bond += 1,
                LinkClass::SignalLink => d.signal_link += 1,
                LinkClass::MessageChannel => d.message_channel += 1,
                LinkClass::Unknown => d.unknown += 1,
            }
        }
        d
    }
}

/// Count of links per class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassDistribution {
    /// PowerBond links.
    pub power_bond: usize,
    /// SignalLink links.
    pub signal_link: usize,
    /// MessageChannel links.
    pub message_channel: usize,
    /// Unknown links (each emitted FL017).
    pub unknown: usize,
}

// ---------------------------------------------------------------------------
// The classify_links pass
// ---------------------------------------------------------------------------

/// Endpoint classification facts recovered for one link end.
struct EndpointClass {
    is_signal: bool,
    /// Has a complete effort/flow conjugate pair (power port).
    is_power: bool,
    /// RSC-3.3c D4: the endpoint's port definition carries item/event-typed
    /// payload features (e.g. `out item cmd : TripCommand;`) — a MESSAGE
    /// endpoint. Links with a message endpoint are claimed as
    /// [`LinkClass::MessageChannel`]: spec `Transfer`s whose payloads are
    /// items carried by send/accept performances (Transfers.kerml). This
    /// implements the D-3.0.1 rule "item/event-typed endpoints →
    /// MessageChannel" — previously such endpoints fell through to
    /// `Unknown` + FL017 (the two multi-circuit-fixture firmware command
    /// links).
    ///
    /// NOTE: this is a runtime link-classification signal derived from the
    /// endpoint's *port payload*, and is DISTINCT from the parser's
    /// `FlowUsage.isMessage` property (set in `sysml-parser-incremental`'s
    /// ast_builder when the `message` kind keyword is used — SysML §7.16: a
    /// message is a flow usage). One classifies an endpoint's port; the other
    /// records which kind keyword declared a flow usage. They are not the same
    /// bit and neither is derived from the other.
    is_message: bool,
    domain: Option<&'static str>,
    confidence: ClassificationConfidence,
    /// True when the role was explicitly declared (`@Signal`/`@PowerPort`).
    declared: bool,
    /// True when the endpoint resolved to *some* classification at all.
    classified: bool,
}

/// Build the classified [`LinkGraph`] from the elaborated model and the
/// classified [`PortRegistry`].
///
/// RSC-3.5e.5 W3: this pass now *folds in the flow producer*. Loop 1 walks the
/// flow elements (`FlowUsage` / `SuccessionFlowUsage` / KerML `Flow`) directly
/// — calling [`crate::flows::compile_single_flow`] per element and keying the
/// resulting [`LinkIR`] on the element's own id — instead of consuming a
/// pre-built `Vec<FlowConnectionIR>`. Loop 2 walks the declared connectors
/// (unchanged). Flow elements are walked in the same `elements_by_kind` order
/// the former `compile_flows` used, so the FlowUsage subset of the returned
/// graph is interning-order-identical to the old flow-list path.
///
/// Returns the graph plus the classification diagnostics (FL017 warnings,
/// conflicting-declaration warnings). The pass is additive and infallible —
/// links that cannot be classified become `LinkClass::Unknown` rather than
/// erroring.
///
/// RSC-3.3b (D-3.0.4 D5): `via_interface` is populated from the declared
/// connector (ConnectionUsage / InterfaceUsage / BindingConnector / …) whose
/// endpoint pair matches the link's endpoints (either orientation). Per the
/// 3.3b spec ruling (see [`transfer_contract_diagnostics`]), a plain
/// `ConnectionUsage` between ports satisfies the Ports.sysml interfacing
/// predicate — `Interface` is defined extensionally as "the most general
/// class of links between Ports on Parts" (Interfaces.sysml:34-43), so any
/// declared port-to-port connector establishes `interfacingPorts` membership.
pub fn classify_links(graph: &ModelGraph, registry: &PortRegistry) -> (LinkGraph, Vec<Diagnostic>) {
    let mut link_graph = LinkGraph::new();
    let mut diagnostics = Vec::new();

    // The physics registry is shared across endpoint classifications — built
    // once from the workspace graph (mirrors flows/health.rs).
    let phys = PhysicsDomainRegistry::from_workspace_graph(graph);
    // Declared connector endpoint pairs (D5 — the topology predicate basis).
    let connector_pairs = declared_connector_pairs(graph);
    // Memoize per-port-def classification (a model reuses port defs heavily).
    let mut class_cache: HashMap<String, EndpointClass> = HashMap::new();

    // RSC-3.5e.5 W3/W4: walk the flow elements directly and build the LinkIR
    // inline — this absorbs the source/target/payload extraction the former
    // `compile_flows` / `compile_single_flow` producer did (both deleted in W4).
    // A malformed flow (missing source/target) is skipped per-element, matching
    // loop 2's connector `continue`.
    for kind in FLOW_ELEM_KINDS {
        for element in graph.elements_by_kind(kind) {
            let (Some(src_raw), Some(tgt_raw)) = (
                element.get_prop("source").and_then(|v| v.as_str()),
                element.get_prop("target").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let source_ep = crate::flows::parse_endpoint(src_raw);
            let target_ep = crate::flows::parse_endpoint(tgt_raw);
            // RSC-3.3a lowering (Transfers.kerml): isMove/isPush default TRUE,
            // overridden by a child `attribute isMove/isPush = <bool>;`.
            let is_succession = *kind == ElementKind::SuccessionFlowUsage;
            let is_move = crate::flows::read_bool_child(element, graph, "isMove").unwrap_or(true);
            let is_push = crate::flows::read_bool_child(element, graph, "isPush").unwrap_or(true);
            let payload_type = element
                .get_prop("payloadType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            // RSC-3.3a D3: derived source-output / target-input feature typings.
            let source_payload_type =
                crate::flows::derive_endpoint_payload_type(registry, &source_ep, true);
            let target_payload_type =
                crate::flows::derive_endpoint_payload_type(registry, &target_ep, false);
            // The LinkIR element id is the flow element's own id; the FL017
            // diagnostic label is the element name, else its id (the former
            // `FlowConnectionIR::id`).
            let element_id = element.id.clone();
            let flow_id = element
                .name
                .clone()
                .unwrap_or_else(|| element.id.to_string());

            // RSC-3.5a.1 (L19): per-usage endpoint ids come from the stamped
            // `source_id`/`target_id` on the origin flow element, not a name-scan.
            let src_stamped = stamped_endpoint_id(graph, &element_id, "source_id");
            let tgt_stamped = stamped_endpoint_id(graph, &element_id, "target_id");
            let source = resolve_endpoint(
                graph,
                registry,
                &source_ep.participant,
                &source_ep.port,
                src_stamped,
            );
            let target = resolve_endpoint(
                graph,
                registry,
                &target_ep.participant,
                &target_ep.port,
                tgt_stamped,
            );

            let src_class = endpoint_class(graph, registry, &phys, &mut class_cache, &source);
            let tgt_class = endpoint_class(graph, registry, &phys, &mut class_cache, &target);

            let (class, confidence) =
                classify_pair(&src_class, &tgt_class, &source_ep.key(), &mut diagnostics);

            if class == LinkClass::Unknown {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "link '{}' ({} -> {}): link class unresolved; routing as message channel",
                        flow_id,
                        source_ep.key(),
                        target_ep.key()
                    ))
                    .with_code("FL017")
                    .with_note(
                        "neither endpoint declares @Signal/@PowerPort metadata nor classifies \
                         into a known physics domain; classify the port definitions (ISQ types \
                         or @Signal/@PowerPort) to route this link deterministically",
                    ),
                );
            }

            // D5 — record the declared connector satisfying the
            // interface-topology predicate, when one exists (orientation-
            // agnostic pair match on the raw endpoint spellings).
            let raw_src = raw_endpoint_key(&source_ep.participant, &source_ep.port);
            let raw_tgt = raw_endpoint_key(&target_ep.participant, &target_ep.port);
            let via_interface = connector_pairs
                .iter()
                .find(|(_, s, t)| {
                    (s == &raw_src && t == &raw_tgt) || (s == &raw_tgt && t == &raw_src)
                })
                .map(|(id, _, _)| id.clone());

            let link = LinkIR {
                element_id,
                // loop 1 only produces flow-derived links.
                kind: LinkSourceKind::FlowUsage,
                source: LinkEndpoint {
                    element_id: source.element_id,
                    owner: source_ep.participant.clone(),
                    port: source_ep.port.clone(),
                    resolved_registry_key: source.registry_key,
                },
                target: LinkEndpoint {
                    element_id: target.element_id,
                    owner: target_ep.participant.clone(),
                    port: target_ep.port.clone(),
                    resolved_registry_key: target.registry_key,
                },
                class,
                class_confidence: confidence,
                is_succession,
                is_move,
                is_push,
                payload_type,
                source_payload_type,
                target_payload_type,
                via_interface,
            };
            link_graph.intern(link);
        }
    }

    // RSC-3.4 / L18 fix — feed Connection/Interface/BindingConnector elements
    // into the link graph alongside the flow-derived links from loop 1.
    // Connector elements that have `source`/`target` props (set by
    // `elaborate/connectors.rs`) but no corresponding flow would otherwise be
    // silently missing. This pass ingests them so the PowerBond subset
    // (consumed by `ConnectionGraph`) is complete — the 14 electrical circuits
    // in the multi-circuit fixture each have a `connect a.port to b.port;` whose
    // element now appears in the graph.
    for kind in CONNECTOR_ELEM_KINDS {
        for elem in graph.elements_by_kind(kind) {
            // Defensive de-dup: loop 1 keys flow links on the flow element's
            // own id and loop 2 keys connector links on the connector
            // element's id, so the two sets never collide on a real model
            // (distinct elements ⇒ distinct ids). The guard stays as a cheap
            // belt-and-suspenders against any future kind overlap.
            if link_graph.by_element(&elem.id).is_some() {
                continue;
            }
            let (Some(src_raw), Some(tgt_raw)) = (
                elem.get_prop("source").and_then(|v| v.as_str()),
                elem.get_prop("target").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let (src_owner, src_port) = split_endpoint(src_raw);
            let (tgt_owner, tgt_port) = split_endpoint(tgt_raw);

            // RSC-3.5a.1 (L19): stamped per-usage ids on the connector element.
            let src_stamped = stamped_endpoint_id(graph, &elem.id, "source_id");
            let tgt_stamped = stamped_endpoint_id(graph, &elem.id, "target_id");
            let source = resolve_endpoint(graph, registry, src_owner, src_port, src_stamped);
            let target = resolve_endpoint(graph, registry, tgt_owner, tgt_port, tgt_stamped);

            let src_class = endpoint_class(graph, registry, &phys, &mut class_cache, &source);
            let tgt_class = endpoint_class(graph, registry, &phys, &mut class_cache, &target);

            let link_label_str = format!("{src_raw} -> {tgt_raw}");
            let (class, confidence) =
                classify_pair(&src_class, &tgt_class, &link_label_str, &mut diagnostics);

            if class == LinkClass::Unknown {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "link '{link_label_str}': link class unresolved; routing as message channel"
                    ))
                    .with_code("FL017")
                    .with_note(
                        "neither endpoint declares @Signal/@PowerPort metadata nor classifies                          into a known physics domain; classify the port definitions (ISQ types                          or @Signal/@PowerPort) to route this link deterministically",
                    ),
                );
            }

            // D5 via_interface lookup for connector-sourced links (same
            // orientation-agnostic pair match as the flow path above).
            let via_interface = connector_pairs
                .iter()
                .find(|(_, s, t)| {
                    (s.as_str() == src_raw && t.as_str() == tgt_raw)
                        || (s.as_str() == tgt_raw && t.as_str() == src_raw)
                })
                .map(|(id, _, _)| id.clone());

            let link = LinkIR {
                element_id: elem.id.clone(),
                kind: source_kind_for_connector(kind.clone()),
                source: LinkEndpoint {
                    element_id: source.element_id,
                    owner: src_owner.to_owned(),
                    port: src_port.to_owned(),
                    resolved_registry_key: source.registry_key,
                },
                target: LinkEndpoint {
                    element_id: target.element_id,
                    owner: tgt_owner.to_owned(),
                    port: tgt_port.to_owned(),
                    resolved_registry_key: target.registry_key,
                },
                class,
                class_confidence: confidence,
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
                source_payload_type: None,
                target_payload_type: None,
                via_interface,
            };
            link_graph.intern(link);
        }
    }

    (link_graph, diagnostics)
}

/// Build the classified [`LinkGraph`] directly from a [`ModelGraph`] — the
/// one-shot convenience composing [`crate::flows::build_port_flow_resources`]
/// (`compile_ports`) with [`classify_links`] (which walks the flow elements
/// itself). Used by the diagram + service sequence-rendering paths, which only
/// have a `ModelGraph`.
pub fn classify_links_from_graph(graph: &ModelGraph) -> (LinkGraph, Vec<Diagnostic>) {
    let res = crate::flows::build_port_flow_resources(graph);
    classify_links(graph, &res.registry)
}

// ---------------------------------------------------------------------------
// RSC-3.3b — spec Transfer enforcement (design doc D-3.0.4 rows D3/D5/D6)
// ---------------------------------------------------------------------------

/// Connector element kinds whose declaration establishes the Ports.sysml
/// `interfacingPorts` relation between their endpoints.
///
/// **Spec ruling (RSC-3.3b research item):** plain `ConnectionUsage`
/// satisfies the predicate, not only `InterfaceUsage`. Sources:
/// - Interfaces.sysml:34-43 — `interface def Interface :> Connection` with
///   "Interface is the most general class of links between Ports on Parts
///   within some containing structure": the class is defined extensionally
///   over port-to-port links, so a binary connection whose participants are
///   ports falls within `Interface`'s extension.
/// - The OMG spec's own normative example (Annex A
///   `SimpleVehicleModel.sysml:699-713`) connects ports with bare
///   `connect a.port to b.port;` declarations and runs transfers over them.
/// - The pilot implementation's examples use both forms
///   (`IssueMetadataExample.sysml:28` plain connect; Flashlight/Vehicle use
///   `interface … connect`), and its `ConnectionUsageAdapter` (implied
///   supertype `connections`/`binaryConnections`) attaches no
///   interface-only restriction.
const CONNECTOR_ELEM_KINDS: &[ElementKind] = &[
    ElementKind::ConnectionUsage,
    ElementKind::InterfaceUsage,
    ElementKind::BindingConnector,
    ElementKind::ConnectorAsUsage,
    ElementKind::BindingConnectorAsUsage,
];

/// Flow-element kinds walked by [`classify_links`] loop 1 (RSC-3.5e.5 W3).
/// Same set and order as the former `compile_flows` producer, so the FlowUsage
/// subset of the link graph interns in identical order.
const FLOW_ELEM_KINDS: &[ElementKind] = &[
    ElementKind::FlowUsage,
    ElementKind::SuccessionFlowUsage,
    ElementKind::Flow,
];

/// Collect `(connector element id, source endpoint, target endpoint)` for
/// every declared connector in the elaborated graph (endpoint strings are
/// the `"participant.port"` spellings set by `elaborate/connectors.rs`).
fn declared_connector_pairs(graph: &ModelGraph) -> Vec<(ElementId, String, String)> {
    let mut pairs = Vec::new();
    for kind in CONNECTOR_ELEM_KINDS {
        for elem in graph.elements_by_kind(kind) {
            let (Some(src), Some(tgt)) = (
                elem.get_prop("source").and_then(|v| v.as_str()),
                elem.get_prop("target").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            pairs.push((elem.id.clone(), src.to_owned(), tgt.to_owned()));
        }
    }
    pairs
}

/// The raw endpoint spelling as it appears on flow/connector `source`/
/// `target` props: `"participant.port"`, or just `"participant"` when the
/// endpoint has no port segment (e.g. a context-level port like
/// `flow from source_out to …`).
fn raw_endpoint_key(participant: &str, port: &str) -> String {
    if port.is_empty() {
        participant.to_owned()
    } else {
        format!("{participant}.{port}")
    }
}

/// True when a link endpoint demonstrably refers to a Port: its routing key
/// resolves in the [`PortRegistry`] (exact `owner.port` key) or its resolved
/// element is a `PortUsage`.
///
/// Endpoints that resolve to neither are NOT statically checked (no FL018):
/// per L23/L25 the corpus' flow endpoints use part-USAGE spellings while the
/// registry keys use part-DEFINITION spellings, so absent endpoint resolution
/// means "no static claim possible", not "violation". Coverage widens when
/// the RSC-3.5 ExchangePlane unifies the string bases onto interned ids.
fn endpoint_is_port(graph: &ModelGraph, registry: &PortRegistry, ep: &LinkEndpoint) -> bool {
    if registry.get(&ep.key()).is_some() {
        return true;
    }
    ep.element_id
        .as_ref()
        .and_then(|id| graph.get_element(id))
        .is_some_and(|e| e.kind == ElementKind::PortUsage)
}

/// RSC-3.3b — compile-time spec Transfer contract checks over the classified
/// link graph (design doc D-3.0.4 rows D3/D5/D6, all **hard errors** per the
/// §6 Q2 RS001-playbook decision; the corpus was pre-cleared in e2ca2546 +
/// the 3.3b model fixes).
///
/// Scope: links of the routed classes (`MessageChannel`, `SignalLink`, and
/// `Unknown` — which routes as a MessageChannel). `PowerBond` links are the
/// acausal DAE plane (RSC-3.4) and are exempt from transfer-contract checks.
///
/// - **FL018 (D5, Ports.sysml:30-44)** — both endpoints are ports but no
///   declared connector (see [`CONNECTOR_ELEM_KINDS`]) connects them: the
///   transfer targets a port that is not an `interfacingPort`.
/// - **FL019 (D6, Transfers.kerml:100-118 + Ports.sysml conjugation)** — the
///   link's source port has effective direction `in`, or its target port has
///   effective direction `out` (directions in the registry are
///   post-conjugation: `elaborate/ports.rs` folds `~P` into
///   `effectiveDirection`). Checked only on exact registry-key resolution —
///   the name-only fallback is ambiguous across defs and makes no claim.
/// - **FL020 (D3, Transfers.kerml:100-118)** — the explicit `payload_type`
///   provably fails to conform to a derived `source_payload_type` /
///   `target_payload_type`. "Provably" = both names are in the
///   ScalarValues.kerml lattice and no conformance path exists; non-scalar
///   name pairs are checked via the graph's specialization edges and make no
///   claim when no edge path is found (open-world: the type may be defined
///   elsewhere).
pub fn transfer_contract_diagnostics(
    graph: &ModelGraph,
    registry: &PortRegistry,
    link_graph: &LinkGraph,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    for link in link_graph.iter() {
        if !matches!(
            link.class,
            LinkClass::MessageChannel | LinkClass::SignalLink | LinkClass::Unknown
        ) {
            continue;
        }
        let src_key = link.source.key();
        let tgt_key = link.target.key();

        // D5 — FL018 interface topology.
        if link.via_interface.is_none()
            && endpoint_is_port(graph, registry, &link.source)
            && endpoint_is_port(graph, registry, &link.target)
        {
            diags.push(
                Diagnostic::error(format!(
                    "FL018: flow '{}' transfers between ports '{src_key}' and '{tgt_key}' \
                     that are not connected by any declared interface or connection",
                    link_label(link),
                ))
                .with_code("FL018")
                .with_note(
                    "Ports.sysml: the target of each outgoingTransfersFromSelf of a Port must \
                     be an interfacingPort (a Port connected to it by an Interface). Declare \
                     the topology, e.g. `connect <source> to <target>;` (a plain connection \
                     between ports satisfies the predicate — Interfaces.sysml defines \
                     Interface as the most general class of links between Ports).",
                ),
            );
        }

        // D6 — FL019 direction checks (post-conjugation effective directions).
        if let Some(port) = registry.get(&src_key) {
            if port.direction == crate::flows::PortDirection::In {
                diags.push(
                    Diagnostic::error(format!(
                        "FL019: flow '{}' picks up its payload at '{src_key}', whose effective \
                         direction is 'in' — a transfer must pick up at a source output",
                        link_label(link),
                    ))
                    .with_code("FL019")
                    .with_note(
                        "Transfers.kerml: the transfer payload subsets \
                         transferSource.sourceOutput; an in-direction port has no outputs \
                         (directions are post-conjugation).",
                    ),
                );
            }
        }
        if let Some(port) = registry.get(&tgt_key) {
            if port.direction == crate::flows::PortDirection::Out {
                diags.push(
                    Diagnostic::error(format!(
                        "FL019: flow '{}' delivers its payload into '{tgt_key}', whose \
                         effective direction is 'out' — a transfer must drop off at a target \
                         input",
                        link_label(link),
                    ))
                    .with_code("FL019")
                    .with_note(
                        "Transfers.kerml: the transfer payload subsets \
                         transferTarget.targetInput; an out-direction port has no inputs \
                         (directions are post-conjugation).",
                    ),
                );
            }
        }

        // D3 — FL020 static payload-subset conformance, when statically known.
        if let Some(payload) = &link.payload_type {
            for (role, declared) in [
                ("sourceOutput", link.source_payload_type.as_deref()),
                ("targetInput", link.target_payload_type.as_deref()),
            ] {
                let Some(declared) = declared else { continue };
                if crate::flows::type_names_provably_nonconformant(graph, payload, declared) {
                    diags.push(
                        Diagnostic::error(format!(
                            "FL020: flow '{}' declares payload type '{payload}' which does \
                             not conform to its {role} type '{declared}'",
                            link_label(link),
                        ))
                        .with_code("FL020")
                        .with_note(
                            "Transfers.kerml: transferPayload subsets both \
                             transferSource.sourceOutput and transferTarget.targetInput; \
                             the payload type must conform to BOTH endpoint typings.",
                        ),
                    );
                }
            }
        }
    }

    diags
}

/// Human label for a link in diagnostics: prefer the source element's name,
/// fall back to the endpoint pair.
fn link_label(link: &LinkIR) -> String {
    format!("{} -> {}", link.source.key(), link.target.key())
}

/// Apply the declared > inferred > unknown precedence to a pair of endpoint
/// classifications, returning the resolved class + confidence.
fn classify_pair(
    src: &EndpointClass,
    tgt: &EndpointClass,
    link_label: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> (LinkClass, ClassificationConfidence) {
    // (1) Declared precedence. A declared end forces the class. Conflicting
    // declared ends (one declared-signal, one declared-power) fail loud.
    let src_declared_signal = src.declared && src.is_signal;
    let src_declared_power = src.declared && src.is_power && !src.is_signal;
    let tgt_declared_signal = tgt.declared && tgt.is_signal;
    let tgt_declared_power = tgt.declared && tgt.is_power && !tgt.is_signal;

    let any_declared_signal = src_declared_signal || tgt_declared_signal;
    let any_declared_power = src_declared_power || tgt_declared_power;

    if any_declared_signal && any_declared_power {
        diagnostics.push(
            Diagnostic::warning(format!(
                "link '{link_label}': endpoints declare conflicting roles (one @Signal, one \
                 @PowerPort); class is left unresolved"
            ))
            .with_code("FL017")
            .with_note(
                "resolve the contradictory @Signal/@PowerPort declarations on the two ports",
            ),
        );
        return (LinkClass::Unknown, ClassificationConfidence::Unknown);
    }
    if any_declared_signal {
        return (LinkClass::SignalLink, ClassificationConfidence::Declared);
    }
    if any_declared_power {
        return (LinkClass::PowerBond, ClassificationConfidence::Declared);
    }

    // (2) Inferred precedence.
    //  - either endpoint signal-classified -> SignalLink
    //  - both power-classified, compatible domain -> PowerBond
    //  - any endpoint classified (item/quantity) but not the above -> MessageChannel
    let confidence = weakest_confidence(src.confidence.clone(), tgt.confidence.clone());

    if src.is_signal || tgt.is_signal {
        return (LinkClass::SignalLink, confidence);
    }
    if src.is_power && tgt.is_power && domains_compatible(src.domain, tgt.domain) {
        return (LinkClass::PowerBond, confidence);
    }
    if src.is_message || tgt.is_message {
        // RSC-3.3c D4: an item/event-typed endpoint is a message surface —
        // the link carries spec Transfers (payload items moved by
        // send/accept performances, Transfers.kerml). Claimed as a
        // MessageChannel instead of falling through to Unknown + FL017
        // (the D-3.0.1 "item/event-typed endpoints → MessageChannel" rule;
        // on the multi-circuit fixture this claims the two firmware command links,
        // firmware.tripOut→breaker.tripIn and firmware.relayOut→relay.commandIn).
        return (LinkClass::MessageChannel, confidence);
    }
    if src.classified || tgt.classified {
        // Classified but not power/signal — a partial pair across
        // mismatched domains, etc. -> message channel.
        return (LinkClass::MessageChannel, confidence);
    }

    // (3) Neither endpoint classified at all.
    (LinkClass::Unknown, ClassificationConfidence::Unknown)
}

/// Two power domains are compatible if both resolved and equal, or if either
/// is unknown (conservative — a bare power port with no domain still bonds).
fn domains_compatible(a: Option<&'static str>, b: Option<&'static str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// Confidence lattice: `Unknown` < `NameHeuristic` < `ISQTyped` < `Declared`.
/// The pair takes the *weakest* (least trustworthy) of the two ends.
fn weakest_confidence(
    a: ClassificationConfidence,
    b: ClassificationConfidence,
) -> ClassificationConfidence {
    fn rank(c: &ClassificationConfidence) -> u8 {
        match c {
            ClassificationConfidence::Unknown => 0,
            ClassificationConfidence::NameHeuristic => 1,
            ClassificationConfidence::ISQTyped => 2,
            ClassificationConfidence::Declared => 3,
        }
    }
    if rank(&a) <= rank(&b) {
        a
    } else {
        b
    }
}

/// Resolved endpoint (best-effort) element id + backing port-definition name.
struct ResolvedEndpoint {
    element_id: Option<ElementId>,
    /// The PortDefinition name backing this endpoint, if recoverable.
    port_def: Option<String>,
    /// The [`PortRegistry`] key on the registry's definition-owner basis
    /// (`"{owning_part_def}.{port}"`), reconstructed from the participant
    /// usage's resolved type. `None` when the type is unresolved. Carried onto
    /// [`LinkEndpoint::resolved_registry_key`].
    registry_key: Option<String>,
}

/// Resolve an endpoint's port-definition name and element id.
///
/// RSC-3.5a.1 (ledger L19): `element_id` is read from the **stamped per-usage
/// id** that `elaborate/{connectors,flows}.rs` writes onto the origin
/// flow/connector element (`source_id` / `target_id`, a `Value::Ref`). That id
/// carries per-USAGE identity (e.g. `controllerA` vs `controllerB` when two
/// usages share a `ControllerDef`) — the def-level name-scan
/// (`find_port_element_id`, deleted here) could not, because ports are
/// definition-owned and the scan found a usage's children, missing the
/// definition-owned port entirely (it returned `None` for the common case).
///
/// The stamped id is threaded in by the callers via [`StampedEndpoint`] (read
/// from the origin element once per link); this function only resolves the
/// `port_def` name from the registry / graph.
fn resolve_endpoint(
    graph: &ModelGraph,
    registry: &PortRegistry,
    owner: &str,
    port: &str,
    stamped: StampedEndpoint,
) -> ResolvedEndpoint {
    let key = format!("{owner}.{port}");
    let port_def = if let Some(inst) = registry.get(&key) {
        inst.definition
            .clone()
            .or_else(|| crate::physics::connection::find_port_definition_for_name(port, graph))
    } else {
        // Registry miss — fall back to a port-name lookup.
        registry
            .find_by_port_name(port)
            .and_then(|p| p.definition.clone())
            .or_else(|| crate::physics::connection::find_port_definition_for_name(port, graph))
    };
    let registry_key = resolve_registry_key(graph, registry, owner, port, stamped.0.as_ref());
    ResolvedEndpoint {
        element_id: stamped.0,
        port_def,
        registry_key,
    }
}

/// Reconstruct the [`PortRegistry`] key for an endpoint on the registry's
/// **definition-owner** basis.
///
/// The registry is keyed `"{owning_part_def}.{port}"` because ports are
/// definition-owned ([`flows::compile_port_from_elaborated`] keys via
/// `find_owner_name`). A link endpoint, however, names a per-instance
/// participant (`"okSensor.currentOut"`). This bridges the two so the
/// signal-propagation / slot-minting feature lookup hits the registry —
/// *without* the ambiguous [`PortRegistry::find_by_port_name`] (steward ruling:
/// feature-set identity is load-bearing there, so a first-match guess is a
/// silent wrong-answer bug; fail hard instead).
///
/// Resolution order:
/// 1. Direct instance-key hit — usage-owned ports register under `"{usage}.{port}"`.
/// 2. Reconstruct `"{participant_type}.{port}"` from the stamped participant
///    usage's resolved type ([`find_feature_type`]). Returned only when it
///    actually hits the registry.
///
/// Returns `None` when the participant type is unresolved — the dormant lookup
/// then misses and the feature is skipped (no slots minted), which is correct:
/// without resolution the registry entry has no features anyway.
fn resolve_registry_key(
    graph: &ModelGraph,
    registry: &PortRegistry,
    owner: &str,
    port: &str,
    stamped: Option<&ElementId>,
) -> Option<String> {
    // (1) Usage-owned port: the registry already holds the instance key.
    let instance_key = format!("{owner}.{port}");
    if registry.contains(&instance_key) {
        return Some(instance_key);
    }
    // (2) Definition-owned port: the stamped id is the participant usage
    //     (`resolve_endpoint_usage_id` returns the participant when the port is
    //     definition-owned). Resolve its type → "{part_def}.{port}".
    let usage_id = stamped?;
    let type_id = find_feature_type(graph, usage_id)?;
    let type_name = graph.get_element(&type_id)?.name.as_deref()?;
    let def_key = format!("{type_name}.{port}");
    registry.contains(&def_key).then_some(def_key)
}

/// The stamped per-usage endpoint id (RSC-3.5a.1 / L19), read from the origin
/// element's `source_id` / `target_id` prop. A newtype so the
/// [`resolve_endpoint`] signature documents that the id flows in from
/// elaboration, not a graph name-scan.
#[derive(Clone, Debug, Default)]
struct StampedEndpoint(Option<ElementId>);

/// Read the stamped `source_id` / `target_id` (`Value::Ref`) the elaboration
/// pass writes onto a flow/connector origin element.
fn stamped_endpoint_id(graph: &ModelGraph, origin: &ElementId, prop: &str) -> StampedEndpoint {
    StampedEndpoint(
        graph
            .get_element(origin)
            .and_then(|e| e.get_prop(prop))
            .and_then(|v| match v {
                sysml_core::Value::Ref(id) => Some(id.clone()),
                _ => None,
            }),
    )
}

/// Classify one resolved endpoint (memoized on its port-def name).
fn endpoint_class(
    graph: &ModelGraph,
    _registry: &PortRegistry,
    phys: &PhysicsDomainRegistry,
    cache: &mut HashMap<String, EndpointClass>,
    ep: &ResolvedEndpoint,
) -> EndpointClass {
    let def = match &ep.port_def {
        Some(d) => d.clone(),
        None => {
            // No port definition recoverable — unclassifiable endpoint.
            return EndpointClass {
                is_signal: false,
                is_power: false,
                is_message: false,
                domain: None,
                confidence: ClassificationConfidence::Unknown,
                declared: false,
                classified: false,
            };
        }
    };
    if let Some(cached) = cache.get(&def) {
        return EndpointClass {
            is_signal: cached.is_signal,
            is_power: cached.is_power,
            is_message: cached.is_message,
            domain: cached.domain,
            confidence: cached.confidence.clone(),
            declared: cached.declared,
            classified: cached.classified,
        };
    }
    let pc = classify_port_definition(&def, graph, phys);
    // RSC-3.3c D4: item/event-typed payload features make a MESSAGE
    // endpoint (only consulted when the definition classifies as neither
    // signal nor power — declared/inferred roles keep precedence).
    let is_message = !pc.is_signal && pc.domain.is_none() && port_def_has_item_payload(graph, &def);
    let classified =
        pc.domain.is_some() || pc.confidence != ClassificationConfidence::Unknown || is_message;
    let is_power = pc.domain.is_some() && !pc.is_signal;
    let declared = pc.confidence == ClassificationConfidence::Declared;
    let ec = EndpointClass {
        is_signal: pc.is_signal,
        is_power,
        is_message,
        domain: pc.domain,
        confidence: pc.confidence,
        declared,
        classified,
    };
    cache.insert(
        def,
        EndpointClass {
            is_signal: ec.is_signal,
            is_power: ec.is_power,
            is_message: ec.is_message,
            domain: ec.domain,
            confidence: ec.confidence.clone(),
            declared: ec.declared,
            classified: ec.classified,
        },
    );
    ec
}

/// RSC-3.3c D4 — does the port definition named `def_name` carry an
/// item/event-typed payload feature (`out item cmd : TripCommand;`)?
///
/// Such ports are message surfaces: their payloads are items moved by spec
/// `Transfer`s (Transfers.kerml), not signal samples or power conjugates.
/// Walks the definition's children for `ItemUsage` / occurrence-usage
/// features (the same child shapes `compile_port_from_elaborated` accepts
/// as port features).
fn port_def_has_item_payload(graph: &ModelGraph, def_name: &str) -> bool {
    let Some(def_elem) = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some(def_name) && e.kind == ElementKind::PortDefinition)
    else {
        return false;
    };
    graph.children_of(&def_elem.id).any(|child| {
        matches!(
            child.kind,
            ElementKind::ItemUsage
                | ElementKind::OccurrenceUsage
                | ElementKind::EventOccurrenceUsage
        )
    })
}

/// RSC-3.4 / L18 — split a raw `"participant.port"` endpoint string into its
/// owner and port components. Endpoints without a dot (a bare participant) are
/// returned as `(raw, "")`.
fn split_endpoint(raw: &str) -> (&str, &str) {
    match raw.find('.') {
        Some(pos) => (&raw[..pos], &raw[pos + 1..]),
        None => (raw, ""),
    }
}

/// RSC-3.4 / L18 — map a connector [`ElementKind`] to a [`LinkSourceKind`].
fn source_kind_for_connector(kind: ElementKind) -> LinkSourceKind {
    match kind {
        ElementKind::ConnectionUsage | ElementKind::ConnectorAsUsage => {
            LinkSourceKind::ConnectionUsage
        }
        ElementKind::InterfaceUsage => LinkSourceKind::InterfaceUsage,
        ElementKind::BindingConnector | ElementKind::BindingConnectorAsUsage => {
            LinkSourceKind::Binding
        }
        // Fallback — should not happen given the CONNECTOR_ELEM_KINDS list.
        _ => LinkSourceKind::ConnectionUsage,
    }
}

// ---------------------------------------------------------------------------
// RSC-3.2 — SignalLink directed propagation (design doc D-3.0.3)
// ---------------------------------------------------------------------------

/// One compiled signal-propagation pair: a single matched feature on one
/// [`LinkClass::SignalLink`], routed `source slot → target slot` per tick.
///
/// The slot spellings are the today's port-value/context keys
/// (`"{owner}.{port}.{feature}"`), so the slot write-through (`set_slot`,
/// dual-spelling mirror) keeps the legacy variables map byte-identical with
/// the string-copy path it replaces. The `*_port_key` / `feature` strings are
/// retained so the per-tick pass can also keep the [`PortRegistry`] feature
/// values updated (the `port_values` snapshot reads the registry, not the
/// context — both must stay in lockstep for baseline parity).
#[derive(Clone, Debug)]
pub struct SignalPropPair {
    /// The signal link this pair belongs to.
    pub link: LinkId,
    /// Source feature slot (`"{src_owner}.{src_port}.{feature}"`).
    pub source_slot: SlotId,
    /// Target feature slot (`"{tgt_owner}.{tgt_port}.{feature}"`).
    pub target_slot: SlotId,
    /// Matched feature name (feature-name match, as the string path did).
    pub feature: String,
    /// Source port routing key (`"{owner}.{port}"`) — registry read key.
    pub source_port_key: String,
    /// Target port routing key (`"{owner}.{port}"`) — registry write key.
    pub target_port_key: String,
    /// RSC-5.3 (D-5.0.4): precomputed boundary unit conversion. `Some` only
    /// when the source and target slots have resolved measurement references of
    /// EQUAL dimension but a different `(scale, offset)` — the transfer then
    /// converts the magnitude (the source slot stores its value in the source
    /// unit, the target expects its own). Computed once at compile from the
    /// slot metas so the per-tick pass does no metadata lookup. `None` for
    /// identical units, untyped slots, or cross-dimension endpoints (the last
    /// is UQ002's hard error, RSC-5.2 — not converted here).
    pub convert: Option<BoundaryConversion>,
}

/// RSC-5.3 (D-5.0.4) — a same-dimension/different-scale unit conversion between
/// two slot boundaries, captured as the source and target SI affine parameters
/// (`magnitude * scale + offset = SI`). Applied to a value as it crosses a
/// binding/flow endpoint via the one [`units::convert_magnitude`] arithmetic
/// home.
///
/// [`units::convert_magnitude`]: crate::expressions::units::convert_magnitude
#[derive(Clone, Debug)]
pub struct BoundaryConversion {
    src_scale: f64,
    src_offset: f64,
    tgt_scale: f64,
    tgt_offset: f64,
    /// The target unit name, adopted by a converted [`Value::Quantity`] so it
    /// carries its destination unit. `None` for an SI-derived target with no
    /// simple name. [`Value`]: sysml_core::Value
    tgt_unit: Option<std::sync::Arc<str>>,
}

impl BoundaryConversion {
    /// Build the conversion between two slots, or `None` when no conversion is
    /// warranted: a slot lacks a measurement reference, the dimensions differ
    /// (UQ002 territory — left untouched so the diagnostic owns it), or the
    /// units are identical (`(scale, offset)` equal → byte-identical no-op).
    fn between(store: &SlotStore, source: SlotId, target: SlotId) -> Option<Self> {
        let src = store.meta(source)?.m_ref.as_ref()?;
        let tgt = store.meta(target)?.m_ref.as_ref()?;
        if src.dimension != tgt.dimension {
            return None;
        }
        if src.scale == tgt.scale && src.offset == tgt.offset {
            return None;
        }
        Some(BoundaryConversion {
            src_scale: src.scale,
            src_offset: src.offset,
            tgt_scale: tgt.scale,
            tgt_offset: tgt.offset,
            tgt_unit: tgt.unit.clone(),
        })
    }

    /// Apply the conversion to a value crossing the boundary. Numeric
    /// magnitudes are converted (a bare `Int`/`Float` becomes a `Float`); a
    /// `Quantity` keeps its dimension and adopts the target unit; non-numeric
    /// values pass through unchanged (an ISQ slot never carries them).
    pub fn apply(&self, value: sysml_core::Value) -> sysml_core::Value {
        use sysml_core::Value;
        let convert = |m: f64| {
            crate::expressions::units::convert_magnitude(
                m,
                self.src_scale,
                self.src_offset,
                self.tgt_scale,
                self.tgt_offset,
            )
        };
        match value {
            Value::Float(f) => Value::Float(convert(f)),
            Value::Int(n) => Value::Float(convert(n as f64)),
            Value::Quantity {
                value, dimension, ..
            } => Value::Quantity {
                value: convert(value),
                dimension,
                unit: self.tgt_unit.as_ref().map(|u| u.to_string()),
            },
            other => other,
        }
    }
}

/// A slot-dependency edge in the **Phase-4 within-phase scheduler's** input
/// shape (design doc D-3.0.3): the set of writer slots that must be settled
/// before the set of reader slots is read.
///
/// RSC-3.2 emits one edge per signal-propagation pair (`[source] → [target]`);
/// the interim per-tick pass is a minimal linear walk over [`pairs`] in
/// dependency order. When RSC-4.1's scheduler lands it consumes these edges
/// directly and the interim ordering is deleted — no rework, the edges ARE
/// the deliverable.
pub type SlotDependencyEdge = (SmallVec<[SlotId; 2]>, SmallVec<[SlotId; 2]>);

/// Compiled SignalLink propagation plan (RSC-3.2, design doc D-3.0.3).
///
/// Holds the directed `(source slot, target slot)` pairs in dependency order
/// (a linear pass over them does per-tick propagation now), the Phase-4
/// scheduler input edges, and the set of links propagated via this plan (so
/// `propagate_port_values` phase 2 can skip them in the legacy string copy).
#[derive(Clone, Debug, Default)]
pub struct SignalPropagation {
    /// Propagation pairs in execution (dependency) order.
    pairs: Vec<SignalPropPair>,
    /// Slot-dependency edges — the Phase-4 scheduler consumes this shape.
    dependency_edges: Vec<SlotDependencyEdge>,
    /// Links routed through this plan (their string-copy phase-2 is skipped).
    signal_links: Vec<LinkId>,
    /// Element ids of the signal links, for the phase-2 skip set (the
    /// `flow_connections` loop keys on `FlowConnectionIR`, which carries the
    /// originating element id via `classify_links`).
    signal_link_element_ids: Vec<ElementId>,
    /// True when the signal-link graph had a cycle (the interim pass then
    /// falls back to interning order = current same-tick-stale behaviour).
    has_cycle: bool,
}

impl SignalPropagation {
    /// The compiled propagation pairs, in per-tick execution order.
    pub fn pairs(&self) -> &[SignalPropPair] {
        &self.pairs
    }

    /// The slot-dependency edges in the **RSC-4.1 scheduler's** input shape —
    /// `(writer slots, reader slots)`, a projection of [`order_pairs`]' already-
    /// ordered output (one edge per pair).
    ///
    /// `crate::scheduler::assemble_edges` consumes this as one of its two edge
    /// sources, projecting each slot edge onto subsystem nodes via `WriterId`.
    /// NOTE (2026-07-03 correction): this does **not** retire the pair pass.
    /// [`Self::pairs`] + `order_pairs` sequence signal propagation at slot-pair
    /// granularity (a source slot settled before it is consumed within a signal
    /// chain); the scheduler orders the strictly coarser *subsystem* object and
    /// cannot express same-subsystem pair order, so `order_pairs` is the
    /// primitive the scheduler composes on top of, not a redundant engine. See
    /// `rsc-4.0-scheduler.md` §D-4.1.2 Seam-1 (closed as a category error).
    pub fn dependency_edges(&self) -> &[SlotDependencyEdge] {
        &self.dependency_edges
    }

    /// Element ids of the links routed through this plan. `propagate_port_values`
    /// phase 2 skips flows whose element id is in this set (the slot path owns
    /// them); non-signal links keep the string copy until RSC-3.3/3.5.
    pub fn signal_link_element_ids(&self) -> &[ElementId] {
        &self.signal_link_element_ids
    }

    /// Interned ids of the signal links in this plan.
    pub fn signal_links(&self) -> &[LinkId] {
        &self.signal_links
    }

    /// Whether the signal-link graph contained a cycle (propagation then runs
    /// in interning order — current same-tick-stale behaviour, the cycle named
    /// in a compile diagnostic).
    pub fn has_cycle(&self) -> bool {
        self.has_cycle
    }

    /// Number of compiled pairs.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Whether the plan is empty (no signal propagation compiled).
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// Compile the SignalLink propagation plan (RSC-3.2, design doc D-3.0.3).
///
/// For every [`LinkClass::SignalLink`] in `link_graph`, match source/target
/// port features by name (the criterion the string-copy path used) and, for
/// each matched feature whose `"{owner}.{port}.{feature}"` slots both resolve
/// in `store`, emit a `(source slot, target slot)` pair. Feature mismatches
/// are surfaced through the existing FL010/FL011 machinery
/// (`flows::health`) — this pass invents no new diagnostics; it silently
/// skips an unmatched/unminted feature.
///
/// Pairs are ordered by a linear topological pass over the slot-dependency
/// graph (one source→target edge per pair). A cycle leaves the involved pairs
/// in interning order (current same-tick-stale behaviour) and emits a compile
/// diagnostic (RS010) naming the cycle.
///
/// COMPLEMENTARY-PATHS INVARIANT (RSC-3.5d, steward-ruled — ledger L26): this
/// is the continuous per-tick delivery path for `SignalLink`s. The discrete
/// [`ExchangePlane`] router (`ingest_classified`, wired by the compiler's
/// `wire_message_router`) ALSO holds these `SignalLink`s in its routing table,
/// but its delivery gate stays inert for them unless an SM explicitly
/// `send()`s to a signal source key — so the two paths never double-deliver.
/// See `ModelCompiler::wire_generic_flow_bridge` for the full invariant.
pub fn compile_signal_propagation(
    link_graph: &LinkGraph,
    store: &SlotStore,
    registry: &PortRegistry,
) -> (SignalPropagation, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut raw_pairs: Vec<SignalPropPair> = Vec::new();
    let mut signal_links: Vec<LinkId> = Vec::new();
    let mut signal_link_element_ids: Vec<ElementId> = Vec::new();
    // A signal *path* is identified by its (source slot, target slot, feature) —
    // the value movement, not the model element that declares it. A `flow` and a
    // `connect` between the same endpoints are two distinct links (L29, WONTFIX —
    // pinned by `flow_and_connector_same_endpoints_are_distinct_links`), but they
    // designate ONE propagation: copying the same source slot to the same target
    // slot twice per tick is redundant work (perf is in the DNA). Collapse here,
    // matching the PowerBond plane which already dedups multiply-declared bonds by
    // endpoint pair (L30, `ConnectionGraph::from_link_graph`).
    let mut seen_paths: HashSet<(SlotId, SlotId, String)> = HashSet::new();

    for &link_id in link_graph.ids_of_class(LinkClass::SignalLink) {
        let Some(link) = link_graph.get(link_id) else {
            continue;
        };
        signal_links.push(link_id);
        signal_link_element_ids.push(link.element_id.clone());

        // Slot names are minted on the per-instance basis (`mint_signal_feature_slots`
        // uses the same `{owner}.{port}.{feature}` spelling), so the slot key is the
        // endpoint's instance key. The registry, however, is keyed per-definition
        // (ports are definition-owned) — read it via `registry_key()`, the resolved
        // `{part_def}.{port}` bridge (ledger L36). Keeping the two distinct is what
        // makes propagation form pairs on parsed models.
        let src_key = link.source.key();
        let tgt_key = link.target.key();
        let src_reg_key = link.source.registry_key();
        let tgt_reg_key = link.target.registry_key();

        // Feature-name match (today's criterion). When the registry knows the
        // source port, match its feature set against the target slot universe;
        // otherwise fall back to the target port's features (the value still
        // routes if both slots exist). Feature discovery routes through the
        // shared `endpoint_feature_names` bridge (def-keyed read, ledger L36).
        // Feature mismatches are FL010/FL011 territory (flows::health) — not
        // re-diagnosed here.
        let src_features = endpoint_feature_names(registry, &link.source);
        let candidate_features: Vec<String> = if src_features.is_empty() {
            endpoint_feature_names(registry, &link.target)
        } else {
            src_features
        };

        for feature in candidate_features {
            let src_slot_name = format!("{src_key}.{feature}");
            let tgt_slot_name = format!("{tgt_key}.{feature}");
            let (Some(source_slot), Some(target_slot)) = (
                store.slot_by_name(&src_slot_name),
                store.slot_by_name(&tgt_slot_name),
            ) else {
                // A feature whose slots were not minted (e.g. target lacks the
                // feature — an FL011 case) routes nothing; skip silently.
                continue;
            };
            // Collapse duplicate signal paths (e.g. the same endpoints declared by
            // both a `flow` and a `connect` — L29). The first declaring link wins;
            // the propagation is identical regardless of which link carries it.
            if !seen_paths.insert((source_slot, target_slot, feature.clone())) {
                continue;
            }
            // UQ002 (RSC-5.2): a SignalLink crossing two slots of incompatible
            // (non-zero, non-equal) quantity dimensions is a HARD ERROR — there is
            // no meaningful boundary conversion across dimensions. This is the one
            // home for the cross-dim error at the signal seam (RSC-5.3 steward
            // ruling #2: same-dim/diff-scale is `BoundaryConversion`'s job, cross-dim
            // is UQ002's). The check is EXPLICIT, not a `between() == None` hook —
            // `between` returns `None` for cross-dim AND for an absent mRef AND for
            // an identical unit, so only an explicit dimension compare distinguishes
            // the error. Predicate matches UQ001/UQ003 (quantity_health.rs).
            if let (Some(s), Some(t)) = (
                store.meta(source_slot).and_then(|m| m.m_ref.as_ref()),
                store.meta(target_slot).and_then(|m| m.m_ref.as_ref()),
            ) {
                if s.dimension != t.dimension && !s.dimension.is_zero() && !t.dimension.is_zero() {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "signal link connects incompatible quantity dimensions: \
                             {src_slot_name} [{}] and {tgt_slot_name} [{}]",
                            s.dimension, t.dimension,
                        ))
                        .with_code("UQ002"),
                    );
                }
            }
            raw_pairs.push(SignalPropPair {
                link: link_id,
                source_slot,
                target_slot,
                feature,
                // The per-tick orchestrator reads/writes the def-keyed registry
                // by these keys (`registry.get(&pair.source_port_key)` /
                // `get_mut(&pair.target_port_key)`), so they carry the registry
                // (definition-owner) key, NOT the per-instance slot key — using
                // the instance key here would miss the registry at tick time.
                source_port_key: src_reg_key.clone(),
                target_port_key: tgt_reg_key.clone(),
                // RSC-5.3 (D-5.0.4): precompute the boundary unit conversion
                // from the (already-minted) slot mRefs — the per-tick pass then
                // applies it with no metadata lookup.
                convert: BoundaryConversion::between(store, source_slot, target_slot),
            });
        }
    }

    // Order the pairs so a producing pair (writes slot S) runs before a
    // consuming pair (reads slot S) — directed propagation in link-dependency
    // order (D-3.0.3). One source→target slot edge per pair.
    let (ordered, has_cycle, cycle_names) = order_pairs(&raw_pairs, store);

    if has_cycle {
        diagnostics.push(
            Diagnostic::warning(format!(
                "signal-link propagation has a cycle ({}); propagating in interning order \
                 (same-tick-stale values within the cycle)",
                cycle_names.join(" -> ")
            ))
            .with_code("RS010")
            .with_note(
                "a feedback loop of signal links cannot be resolved in one pass; values inside \
                 the cycle use the previous tick's value. Break the cycle or model the feedback \
                 with an explicit state (RSC-4 scheduler will formalise multi-rate ordering).",
            ),
        );
    }

    // Slot-dependency edges in the Phase-4 scheduler's input shape: one
    // `[source] -> [target]` edge per pair (D-3.0.3). Phase 4 will fuse these
    // into a within-phase order; we emit them verbatim.
    let dependency_edges: Vec<SlotDependencyEdge> = ordered
        .iter()
        .map(|p| {
            let writers: SmallVec<[SlotId; 2]> = SmallVec::from_slice(&[p.source_slot]);
            let readers: SmallVec<[SlotId; 2]> = SmallVec::from_slice(&[p.target_slot]);
            (writers, readers)
        })
        .collect();

    let plan = SignalPropagation {
        pairs: ordered,
        dependency_edges,
        signal_links,
        signal_link_element_ids,
        has_cycle,
    };
    (plan, diagnostics)
}

/// Topologically order propagation pairs so every pair that writes a slot runs
/// before any pair that reads it. Returns the ordered pairs, whether a cycle
/// was found, and (when so) the canonical names of the slots on the cycle.
///
/// On a cycle the pairs are returned in interning order (the current
/// same-tick-stale behaviour the design doc preserves), so propagation never
/// hangs — it just uses last-tick values inside the loop.
fn order_pairs(
    pairs: &[SignalPropPair],
    store: &SlotStore,
) -> (Vec<SignalPropPair>, bool, Vec<String>) {
    let n = pairs.len();
    if n == 0 {
        return (Vec::new(), false, Vec::new());
    }
    // Edge i -> j when pair i's target slot is pair j's source slot
    // (i must run before j). Kahn's algorithm over pair indices.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];
    for (i, pi) in pairs.iter().enumerate() {
        for (j, pj) in pairs.iter().enumerate() {
            if i != j && pi.target_slot == pj.source_slot {
                adj[i].push(j);
                indegree[j] += 1;
            }
        }
    }
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    while let Some(i) = queue.pop_front() {
        order.push(i);
        for &j in &adj[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                queue.push_back(j);
            }
        }
    }
    if order.len() == n {
        let ordered = order.into_iter().map(|i| pairs[i].clone()).collect();
        return (ordered, false, Vec::new());
    }

    // Cycle: the pairs still on a positive indegree are part of (or downstream
    // of) the cycle. Name the slots for the diagnostic; fall back to interning
    // order so the interim pass still runs.
    let cycle_names: Vec<String> = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| indegree[*i] > 0)
        .filter_map(|(_, p)| {
            store
                .meta(p.target_slot)
                .map(|m| m.canonical_name.as_ref().to_owned())
        })
        .collect();
    (pairs.to_vec(), true, cycle_names)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementId, ElementKind, Value};

    /// Add a real `FlowUsage` element (with `source`/`target` props and explicit
    /// `isMove`/`isPush = false` children) so `classify_links`' producer-fold
    /// loop 1 (RSC-3.5e.5 W3) picks it up. Replaces the old synthetic
    /// `FlowConnectionIR` the tests built before the fold — which likewise
    /// hardcoded is_move/is_push = false, so the explicit children keep the
    /// carried values byte-identical.
    fn add_flow(graph: &mut ModelGraph, id: &str, so: &str, sp: &str, to: &str, tp: &str) {
        let fid = ElementId::new_v4();
        graph.add_element(
            Element::new(fid.clone(), ElementKind::FlowUsage)
                .with_name(id)
                .with_prop("source", Value::String(format!("{so}.{sp}")))
                .with_prop("target", Value::String(format!("{to}.{tp}"))),
        );
        for (n, v) in [("isMove", false), ("isPush", false)] {
            graph.add_element(
                Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                    .with_owner(fid.clone())
                    .with_name(n)
                    .with_prop("value", Value::Bool(v)),
            );
        }
    }

    /// Attach an anonymous MetadataUsage + FeatureTyping child (mirrors the
    /// parser's `@Type` lowering — see classify.rs test helper).
    fn attach_metadata(graph: &mut ModelGraph, owner: &ElementId, type_ref: &str) {
        let meta_id = ElementId::new_v4();
        graph.add_element(
            Element::new(meta_id.clone(), ElementKind::MetadataUsage).with_owner(owner.clone()),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::FeatureTyping)
                .with_owner(meta_id.clone())
                .with_prop("typedFeature", Value::Ref(meta_id))
                .with_prop("unresolved_type", Value::String(type_ref.into())),
        );
    }

    /// Build a power-shaped port def (complete voltage+current pair) named
    /// `def_name`, plus a part `owner` with a port `port_name : def_name`.
    fn power_port(
        graph: &mut ModelGraph,
        owner: &str,
        port_name: &str,
        def_name: &str,
    ) -> ElementId {
        let port_def = ElementId::new_v4();
        graph.add_element(
            Element::new(port_def.clone(), ElementKind::PortDefinition).with_name(def_name),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_def.clone())
                .with_name("voltage")
                .with_prop("typeName", Value::String("Real".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_def.clone())
                .with_name("current")
                .with_prop("typeName", Value::String("Real".into())),
        );
        let owner_id = part(graph, owner);
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PortUsage)
                .with_owner(owner_id)
                .with_name(port_name)
                // `portDefinition` is the prop elaboration sets; compile_ports
                // and find_port_definition_for_name both read it.
                .with_prop("portDefinition", Value::String(def_name.into())),
        );
        port_def
    }

    /// Build a signal-shaped (flow-only measurement) port def + a port on
    /// `owner`.
    fn signal_port(graph: &mut ModelGraph, owner: &str, port_name: &str, def_name: &str) {
        let port_def = ElementId::new_v4();
        graph.add_element(
            Element::new(port_def.clone(), ElementKind::PortDefinition).with_name(def_name),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_def.clone())
                .with_name("measuredCurrent")
                .with_prop("typeName", Value::String("Real".into())),
        );
        let owner_id = part(graph, owner);
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PortUsage)
                .with_owner(owner_id)
                .with_name(port_name)
                .with_prop("portDefinition", Value::String(def_name.into())),
        );
    }

    fn part(graph: &mut ModelGraph, name: &str) -> ElementId {
        if let Some(e) = graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
        {
            return e.id.clone();
        }
        let id = ElementId::new_v4();
        graph.add_element(Element::new(id.clone(), ElementKind::PartUsage).with_name(name));
        id
    }

    fn registry_from(graph: &ModelGraph) -> PortRegistry {
        crate::flows::compile_ports(graph)
    }

    #[test]
    fn both_power_endpoints_infer_power_bond() {
        let mut graph = ModelGraph::new();
        power_port(&mut graph, "src", "p", "PowerPortA");
        power_port(&mut graph, "dst", "q", "PowerPortB");
        let reg = registry_from(&graph);
        add_flow(&mut graph, "f1", "src", "p", "dst", "q");
        let (lg, diags) = classify_links(&graph, &reg);
        assert_eq!(lg.len(), 1);
        assert_eq!(lg.iter().next().unwrap().class, LinkClass::PowerBond);
        assert!(diags.iter().all(|d| !format!("{d}").contains("FL017")));
    }

    #[test]
    fn signal_endpoint_infers_signal_link() {
        let mut graph = ModelGraph::new();
        signal_port(&mut graph, "sensor", "out", "CurrentSense");
        power_port(&mut graph, "bus", "in", "BusPort");
        let reg = registry_from(&graph);
        add_flow(&mut graph, "f1", "sensor", "out", "bus", "in");
        let (lg, _diags) = classify_links(&graph, &reg);
        assert_eq!(lg.iter().next().unwrap().class, LinkClass::SignalLink);
    }

    #[test]
    fn declared_signal_beats_inferred_power() {
        let mut graph = ModelGraph::new();
        // A power-shaped def, but declared @Signal -> SignalLink wins.
        let def = power_port(&mut graph, "src", "p", "DeclaredSig");
        attach_metadata(&mut graph, &def, "Signal");
        power_port(&mut graph, "dst", "q", "PowerPortB");
        let reg = registry_from(&graph);
        add_flow(&mut graph, "f1", "src", "p", "dst", "q");
        let (lg, _diags) = classify_links(&graph, &reg);
        let link = lg.iter().next().unwrap();
        assert_eq!(link.class, LinkClass::SignalLink);
        assert_eq!(link.class_confidence, ClassificationConfidence::Declared);
    }

    #[test]
    fn conflicting_declarations_yield_unknown_and_fl017() {
        let mut graph = ModelGraph::new();
        let sdef = power_port(&mut graph, "src", "p", "DeclSig");
        attach_metadata(&mut graph, &sdef, "Signal");
        let pdef = power_port(&mut graph, "dst", "q", "DeclPow");
        attach_metadata(&mut graph, &pdef, "PowerPort");
        add_flow(&mut graph, "f1", "src", "p", "dst", "q");
        let reg = registry_from(&graph);
        let (lg, diags) = classify_links(&graph, &reg);
        assert_eq!(lg.iter().next().unwrap().class, LinkClass::Unknown);
        assert!(diags.iter().any(|d| format!("{d}").contains("FL017")));
    }

    #[test]
    fn unclassifiable_endpoints_yield_unknown_and_fl017() {
        // No port definitions at all — both endpoints unclassifiable.
        let mut graph = ModelGraph::new();
        part(&mut graph, "a");
        part(&mut graph, "b");
        let reg = registry_from(&graph);
        add_flow(&mut graph, "f1", "a", "x", "b", "y");
        let (lg, diags) = classify_links(&graph, &reg);
        assert_eq!(lg.iter().next().unwrap().class, LinkClass::Unknown);
        assert_eq!(lg.class_distribution().unknown, 1);
        assert!(diags.iter().any(|d| format!("{d}").contains("FL017")));
    }

    #[test]
    fn link_id_interning_is_dense_and_indexed() {
        let mut lg = LinkGraph::new();
        let e1 = ElementId::new_v4();
        let e2 = ElementId::new_v4();
        let mk = |eid: ElementId, class: LinkClass| LinkIR {
            element_id: eid,
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: "a".into(),
                port: "p".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: "b".into(),
                port: "q".into(),
                resolved_registry_key: None,
            },
            class,
            class_confidence: ClassificationConfidence::Unknown,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        };
        let id1 = lg.intern(mk(e1.clone(), LinkClass::PowerBond));
        let id2 = lg.intern(mk(e2.clone(), LinkClass::SignalLink));
        assert_eq!(id1, LinkId(0));
        assert_eq!(id2, LinkId(1));
        assert_eq!(lg.by_element(&e1).unwrap().class, LinkClass::PowerBond);
        assert_eq!(lg.by_element(&e2).unwrap().class, LinkClass::SignalLink);
        assert_eq!(lg.ids_of_class(LinkClass::PowerBond), &[id1]);
        assert_eq!(lg.ids_of_class(LinkClass::SignalLink), &[id2]);
    }

    #[test]
    fn is_move_push_carried_from_flow_unchanged() {
        let mut graph = ModelGraph::new();
        power_port(&mut graph, "src", "p", "PowerPortA");
        power_port(&mut graph, "dst", "q", "PowerPortB");
        let reg = registry_from(&graph);
        // The flow element declares `isMove = false` / `isPush = false`
        // (see `add_flow`); the pass must carry those verbatim into the LinkIR.
        add_flow(&mut graph, "f1", "src", "p", "dst", "q");
        let (lg, _d) = classify_links(&graph, &reg);
        let link = lg.iter().next().unwrap();
        assert!(!link.is_move);
        assert!(!link.is_push);
    }

    // -----------------------------------------------------------------------
    // RSC-3.3c D4 — item/event-typed endpoints claimed as MessageChannel
    // -----------------------------------------------------------------------

    /// Build a command-shaped port def (item-typed payload feature, like a
    /// multi-circuit fixture's `TripCommandPort { out item cmd : TripCommand; }`)
    /// plus a part `owner` with a port `port_name : def_name`.
    fn item_command_port(graph: &mut ModelGraph, owner: &str, port_name: &str, def_name: &str) {
        let port_def = ElementId::new_v4();
        graph.add_element(
            Element::new(port_def.clone(), ElementKind::PortDefinition).with_name(def_name),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::ItemUsage)
                .with_owner(port_def.clone())
                .with_name("cmd")
                .with_prop("typeName", Value::String("TripCommand".into())),
        );
        let owner_id = part(graph, owner);
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PortUsage)
                .with_owner(owner_id)
                .with_name(port_name)
                .with_prop("portDefinition", Value::String(def_name.into())),
        );
    }

    #[test]
    fn item_typed_endpoints_claimed_as_message_channel() {
        // RSC-3.3c D4: item-typed command ports are message surfaces — the
        // link is a spec Transfer carrier (MessageChannel), not Unknown.
        // This is the multi-circuit-fixture firmware command-link shape
        // (firmware.tripOut -> breaker.tripIn), Unknown + FL017 before 3.3c.
        let mut graph = ModelGraph::new();
        item_command_port(&mut graph, "firmware", "tripOut", "TripCommandPort");
        item_command_port(&mut graph, "breaker", "tripIn", "TripCommandInPort");
        let reg = registry_from(&graph);
        add_flow(&mut graph, "f1", "firmware", "tripOut", "breaker", "tripIn");
        let (lg, diags) = classify_links(&graph, &reg);
        let link = lg.iter().next().unwrap();
        assert_eq!(
            link.class,
            LinkClass::MessageChannel,
            "item-typed endpoints are claimed as MessageChannel (D-3.0.1 row 2)"
        );
        assert!(
            diags.iter().all(|d| !format!("{d}").contains("FL017")),
            "a claimed link fires no FL017: {diags:?}"
        );
    }

    #[test]
    fn attribute_only_unclassifiable_endpoints_stay_unknown() {
        // The claim is item/event-typed ONLY: a def with no features at all
        // still falls through to Unknown + FL017.
        let mut graph = ModelGraph::new();
        let port_def = ElementId::new_v4();
        graph
            .add_element(Element::new(port_def, ElementKind::PortDefinition).with_name("BarePort"));
        let owner_id = part(&mut graph, "a");
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PortUsage)
                .with_owner(owner_id)
                .with_name("p")
                .with_prop("portDefinition", Value::String("BarePort".into())),
        );
        let owner_b = part(&mut graph, "b");
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PortUsage)
                .with_owner(owner_b)
                .with_name("q")
                .with_prop("portDefinition", Value::String("BarePort".into())),
        );
        let reg = registry_from(&graph);
        add_flow(&mut graph, "f1", "a", "p", "b", "q");
        let (lg, diags) = classify_links(&graph, &reg);
        assert_eq!(lg.iter().next().unwrap().class, LinkClass::Unknown);
        assert!(diags.iter().any(|d| format!("{d}").contains("FL017")));
    }

    // -----------------------------------------------------------------------
    // RSC-3.2 — SignalLink directed propagation (compile_signal_propagation)
    // -----------------------------------------------------------------------

    use crate::flows::port::{PortFeature, PortInstanceIR};
    use crate::flows::PortDirection;
    use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
    use std::collections::BTreeSet;

    /// Build a signal `LinkGraph` directly (bypassing classification) for the
    /// propagation tests.
    fn signal_link_graph(edges: &[(&str, &str, &str, &str)]) -> LinkGraph {
        let mut lg = LinkGraph::new();
        for (so, sp, to, tp) in edges {
            lg.intern(LinkIR {
                element_id: ElementId::from_string(format!("link:{so}.{sp}->{to}.{tp}")),
                kind: LinkSourceKind::FlowUsage,
                source: LinkEndpoint {
                    element_id: None,
                    owner: (*so).into(),
                    port: (*sp).into(),
                    resolved_registry_key: None,
                },
                target: LinkEndpoint {
                    element_id: None,
                    owner: (*to).into(),
                    port: (*tp).into(),
                    resolved_registry_key: None,
                },
                class: LinkClass::SignalLink,
                class_confidence: ClassificationConfidence::Declared,
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
                source_payload_type: None,
                target_payload_type: None,
                via_interface: None,
            });
        }
        lg
    }

    /// A registry with one feature `feat` per `(owner, port)` in `ports`.
    fn registry_with_features(ports: &[(&str, &str)], feat: &str) -> PortRegistry {
        let mut reg = PortRegistry::new();
        for (owner, port) in ports {
            let mut inst = PortInstanceIR::new(*owner, *port).with_direction(PortDirection::InOut);
            inst.add_feature(PortFeature {
                name: feat.to_owned(),
                direction: PortDirection::InOut,
                type_name: Some("Real".into()),
                value: Value::Float(0.0),
            });
            reg.register(inst);
        }
        reg
    }

    /// A slot store with a Continuous/Orchestrator feature slot per
    /// `"{owner}.{port}.{feat}"`.
    fn store_with_feature_slots(ports: &[(&str, &str)], feat: &str) -> SlotStore {
        let mut store = SlotStore::new();
        for (owner, port) in ports {
            let name = format!("{owner}.{port}.{feat}");
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(ElementId::from_string(format!("decl:{name}"))),
                    Variability::Continuous,
                    WriterId::Orchestrator,
                    name.as_str(),
                    name.as_str(),
                ),
                Value::Float(0.0),
            );
        }
        store
    }

    #[test]
    fn signal_propagation_compiles_one_pair_per_matched_feature() {
        let lg = signal_link_graph(&[("sensor", "out", "fw", "in")]);
        let ports = [("sensor", "out"), ("fw", "in")];
        let reg = registry_with_features(&ports, "v");
        let store = store_with_feature_slots(&ports, "v");

        let (plan, diags) = compile_signal_propagation(&lg, &store, &reg);
        assert_eq!(plan.len(), 1, "one matched feature -> one pair");
        assert!(diags.is_empty(), "no cycle, no diagnostics");
        let pair = &plan.pairs()[0];
        assert_eq!(pair.feature, "v");
        assert_eq!(
            pair.source_slot,
            store.slot_by_name("sensor.out.v").unwrap()
        );
        assert_eq!(pair.target_slot, store.slot_by_name("fw.in.v").unwrap());
        // Element-id skip set carries the one signal link.
        assert_eq!(plan.signal_link_element_ids().len(), 1);
    }

    /// RSC-5.3 (D-5.0.4): the boundary conversion on a signal pair is `Some`
    /// only when the source and target slots have measurement references of the
    /// SAME dimension but a DIFFERENT scale. Untyped slots, identical units, and
    /// cross-dimension endpoints all yield `None` (byte-identical pass-through;
    /// cross-dimension is UQ002's hard error, not converted here).
    #[test]
    fn boundary_conversion_only_for_same_dim_different_scale() {
        use crate::slots::MeasurementRef;
        let lg = signal_link_graph(&[("sensor", "out", "fw", "in")]);
        let ports = [("sensor", "out"), ("fw", "in")];
        let reg = registry_with_features(&ports, "v");

        let mref = |unit: &str| {
            let e = crate::expressions::units::lookup_unit(unit).expect("unit");
            MeasurementRef {
                dimension: e.dimension,
                unit: Some(std::sync::Arc::from(unit)),
                scale: e.scale,
                offset: e.offset,
            }
        };
        let build = |src_unit: Option<&str>, tgt_unit: Option<&str>| {
            let mut store = store_with_feature_slots(&ports, "v");
            if let Some(u) = src_unit {
                let id = store.slot_by_name("sensor.out.v").unwrap();
                store.set_m_ref(id, Some(mref(u)));
            }
            if let Some(u) = tgt_unit {
                let id = store.slot_by_name("fw.in.v").unwrap();
                store.set_m_ref(id, Some(mref(u)));
            }
            let (plan, _diags) = compile_signal_propagation(&lg, &store, &reg);
            plan.pairs()[0].convert.clone()
        };

        // Untyped both ends → None (today's corpus default, byte-identical).
        assert!(build(None, None).is_none(), "untyped slots: no conversion");
        // Same dimension, different scale (A -> mA) → Some.
        assert!(
            build(Some("A"), Some("mA")).is_some(),
            "A -> mA is a same-dim/different-scale conversion"
        );
        // Identical unit on both ends → None (no-op, byte-identical).
        assert!(
            build(Some("A"), Some("A")).is_none(),
            "identical units: no conversion"
        );
        // Cross-dimension (A current -> m length) → None (UQ002 owns this).
        assert!(
            build(Some("A"), Some("m")).is_none(),
            "cross-dimension endpoints are not converted (UQ002 territory)"
        );
        // One end untyped → None (the untyped/warning tier, not a conversion).
        assert!(
            build(Some("A"), None).is_none(),
            "one untyped endpoint: no conversion"
        );

        // The Some conversion actually maps 2 A -> 2000 mA.
        let conv = build(Some("A"), Some("mA")).unwrap();
        match conv.apply(Value::Float(2.0)) {
            Value::Float(v) => assert!((v - 2000.0).abs() < 1e-9, "2 A = 2000 mA, got {v}"),
            other => panic!("expected a converted Float, got {other:?}"),
        }
    }

    #[test]
    fn signal_propagation_fan_out_one_source_two_targets() {
        // One source feeds two targets (fan-out, N=2).
        let lg = signal_link_graph(&[
            ("sensor", "out", "fwA", "in"),
            ("sensor", "out", "fwB", "in"),
        ]);
        let ports = [("sensor", "out"), ("fwA", "in"), ("fwB", "in")];
        let reg = registry_with_features(&ports, "v");
        let store = store_with_feature_slots(&ports, "v");

        let (plan, diags) = compile_signal_propagation(&lg, &store, &reg);
        assert!(diags.is_empty());
        assert_eq!(plan.len(), 2, "fan-out yields two pairs from one source");
        let src = store.slot_by_name("sensor.out.v").unwrap();
        assert!(
            plan.pairs().iter().all(|p| p.source_slot == src),
            "both pairs share the one source slot"
        );
        let targets: BTreeSet<_> = plan.pairs().iter().map(|p| p.target_slot).collect();
        assert_eq!(targets.len(), 2, "two distinct target slots");
    }

    #[test]
    fn signal_propagation_dependency_edge_shape() {
        // a.out -> b.in, then b.out -> c.in: a chained dependency. The
        // dependency edges are the Phase-4 scheduler input shape:
        // ([source slot], [target slot]) per pair, in dependency order.
        let lg = signal_link_graph(&[("a", "out", "b", "in"), ("b", "out", "c", "in")]);
        // b has both an `in` and an `out` port; chain via feature `v`.
        let ports = [("a", "out"), ("b", "in"), ("b", "out"), ("c", "in")];
        let reg = registry_with_features(&ports, "v");
        // Make b.in and b.out the SAME slot so the chain is a true dependency
        // (a.out.v -> b.v -> c.in.v): alias both port-feature names to one slot.
        let mut store = SlotStore::new();
        let a = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string("decl:a.out.v")),
                Variability::Continuous,
                WriterId::Orchestrator,
                "a.out.v",
                "a.out.v",
            ),
            Value::Float(0.0),
        );
        let b = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string("decl:b.v")),
                Variability::Continuous,
                WriterId::Orchestrator,
                "b.in.v",
                "b.in.v",
            ),
            Value::Float(0.0),
        );
        store.add_alias("b.out.v", b);
        let c = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string("decl:c.in.v")),
                Variability::Continuous,
                WriterId::Orchestrator,
                "c.in.v",
                "c.in.v",
            ),
            Value::Float(0.0),
        );

        let (plan, diags) = compile_signal_propagation(&lg, &store, &reg);
        assert!(diags.is_empty(), "chain is acyclic");
        let edges = plan.dependency_edges();
        assert_eq!(edges.len(), 2, "one edge per pair");
        // Each edge is ([writer slot], [reader slot]).
        for (writers, readers) in edges {
            assert_eq!(writers.len(), 1);
            assert_eq!(readers.len(), 1);
        }
        // Dependency order: the pair writing `b` (a.out->b.in) must come
        // before the pair reading `b` (b.out->c.in).
        let order: Vec<(SlotId, SlotId)> = plan
            .pairs()
            .iter()
            .map(|p| (p.source_slot, p.target_slot))
            .collect();
        let pos_writes_b = order.iter().position(|(_, t)| *t == b).unwrap();
        let pos_reads_b = order.iter().position(|(s, _)| *s == b).unwrap();
        assert!(
            pos_writes_b < pos_reads_b,
            "producer of slot b must be ordered before its consumer: {order:?} (a={a:?} b={b:?} c={c:?})"
        );
    }

    #[test]
    fn signal_propagation_cycle_emits_diagnostic_and_falls_back() {
        // a.out -> b.in and b.out -> a.in, with b.in/b.out and a.in/a.out
        // aliased to one slot each, forming a 2-cycle.
        let lg = signal_link_graph(&[("a", "out", "b", "in"), ("b", "out", "a", "in")]);
        let ports = [("a", "out"), ("a", "in"), ("b", "in"), ("b", "out")];
        let reg = registry_with_features(&ports, "v");
        let mut store = SlotStore::new();
        let a = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string("decl:a.v")),
                Variability::Continuous,
                WriterId::Orchestrator,
                "a.out.v",
                "a.out.v",
            ),
            Value::Float(0.0),
        );
        store.add_alias("a.in.v", a);
        let b = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string("decl:b.v")),
                Variability::Continuous,
                WriterId::Orchestrator,
                "b.in.v",
                "b.in.v",
            ),
            Value::Float(0.0),
        );
        store.add_alias("b.out.v", b);

        let (plan, diags) = compile_signal_propagation(&lg, &store, &reg);
        assert!(plan.has_cycle(), "a<->b is a cycle");
        assert!(
            diags.iter().any(|d| format!("{d}").contains("RS010")),
            "cycle must emit the RS010 diagnostic, got: {diags:?}"
        );
        // Fallback: both pairs are still present (interning order), so the
        // interim per-tick pass still runs (same-tick-stale).
        assert_eq!(plan.len(), 2, "cyclic pairs are kept in interning order");
        let _ = (a, b);
    }

    // -----------------------------------------------------------------------
    // RSC-3.4 / L18 — connector elements ingested into the link graph
    // -----------------------------------------------------------------------

    /// Build a `ConnectionUsage` element with `source` / `target` props,
    /// mimicking what `elaborate/connectors.rs` sets when it processes a
    /// `connect src.p to dst.q;` declaration.
    fn add_connection_elem(
        graph: &mut ModelGraph,
        src_owner: &str,
        src_port: &str,
        dst_owner: &str,
        dst_port: &str,
    ) -> ElementId {
        let conn_id = ElementId::new_v4();
        graph.add_element(
            Element::new(conn_id.clone(), ElementKind::ConnectionUsage)
                .with_prop("source", Value::String(format!("{src_owner}.{src_port}")))
                .with_prop("target", Value::String(format!("{dst_owner}.{dst_port}"))),
        );
        conn_id
    }

    #[test]
    fn l18_connection_usage_ingested_alongside_flows() {
        // A `ConnectionUsage` element with `source`/`target` props and no
        // corresponding FlowConnectionIR must appear in the link graph after
        // the L18 fix (RSC-3.4 Phase A). Before the fix, only FlowUsage
        // entries (from compile_flows) were ingested; connector elements were
        // silent missing entries.
        let mut graph = ModelGraph::new();
        // Power-shaped ports on both ends so the link classifies as PowerBond.
        power_port(&mut graph, "src", "p", "PowerPortA");
        power_port(&mut graph, "dst", "q", "PowerPortB");
        let conn_id = add_connection_elem(&mut graph, "src", "p", "dst", "q");

        let reg = registry_from(&graph);
        // No flow element — purely connector-element driven.
        let (lg, diags) = classify_links(&graph, &reg);
        assert_eq!(
            lg.len(),
            1,
            "connector-only graph: one link expected from the L18 fix"
        );
        let link = lg
            .by_element(&conn_id)
            .expect("link indexed by connector element id");
        assert_eq!(
            link.class,
            LinkClass::PowerBond,
            "both power endpoints → PowerBond"
        );
        assert_eq!(
            link.kind,
            LinkSourceKind::ConnectionUsage,
            "source kind must be ConnectionUsage"
        );
        assert!(
            diags.iter().all(|d| !format!("{d}").contains("FL017")),
            "classified link fires no FL017: {diags:?}"
        );
    }

    #[test]
    fn flow_and_connector_same_endpoints_are_distinct_links() {
        // RSC-3.5e.5 W3: a `flow` and a `connect` between the SAME endpoints
        // are two distinct model elements, so loop 1 (flow-derived) and loop 2
        // (connector-derived) intern two distinct links — each keyed by its own
        // element id, with its own `kind`. Pre-W3 a synthetic FlowConnectionIR
        // whose `id` was forced to equal the connector's id triggered the
        // loop-2 dedup guard; that collision is no longer constructible because
        // the flow link is now keyed on the flow element's own (distinct) id.
        let mut graph = ModelGraph::new();
        power_port(&mut graph, "src", "p", "PowerPortA");
        power_port(&mut graph, "dst", "q", "PowerPortB");
        let conn_id = add_connection_elem(&mut graph, "src", "p", "dst", "q");
        add_flow(&mut graph, "f1", "src", "p", "dst", "q");

        let reg = registry_from(&graph);
        let (lg, _diags) = classify_links(&graph, &reg);
        assert_eq!(
            lg.len(),
            2,
            "distinct flow + connector elements yield two distinct links"
        );
        // The connector link is indexed by the connector element id…
        assert_eq!(
            lg.by_element(&conn_id).expect("connector link").kind,
            LinkSourceKind::ConnectionUsage
        );
        // …and exactly one FlowUsage-derived link is present.
        assert_eq!(
            lg.iter()
                .filter(|l| l.kind == LinkSourceKind::FlowUsage)
                .count(),
            1
        );
    }
}
