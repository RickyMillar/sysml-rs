//! RSC-3.5a.2-i — `ExchangePlane`: the interned-LinkId classified routing
//! surface that REPLACED the legacy string-keyed `FlowRouter` (deleted in
//! RSC-3.5e.2; this is now the sole message-routing backend).
//!
//! §3 D-3.0.5, §7 (3.5a), §8 (the implementation blueprint).
//!
//! # What this is
//!
//! `FlowRouter` routes on STRING keys (`source_index: HashMap<String, …>`,
//! `delivery_queues: HashMap<String, …>`). `ExchangePlane` routes on the
//! interned [`LinkGraph`](crate::links::LinkGraph): the link graph IS the
//! routing table — a dense `Vec<LinkIR>` indexed by [`LinkId`], with the
//! `source_index` keyed to those dense ids. Delivery / pull queues are
//! `Vec<VecDeque<FlowMessage>>` indexed by an interned dense
//! [`TargetId`](self::TargetId) (the per-target endpoint identity), not a
//! string `HashMap`. [`FlowMessage`] becomes a PROJECTED VIEW
//! ([`project_message`]) rendered from the internal [`MessageRecord`] + the
//! link graph, keeping its exact 6-field serde shape so `snap.messages` stays
//! byte-identical.
//!
//! # Scope of THIS wave (additive, isolated)
//!
//! `ExchangePlane` is built and unit-tested **in isolation**: it is NOT wired
//! into the [`Orchestrator`](crate::orchestrator::Orchestrator), NOT called by
//! any consumer, and changes NO existing behaviour. The only edit outside this
//! file is the `pub mod exchange;` line in `lib.rs`. Correctness bar: it
//! reproduces `FlowRouter`'s observable behaviour method-for-method,
//! byte-for-byte, proven by the parity unit tests below that drive BOTH
//! structures with identical inputs and assert identical outputs.
//!
//! # Interning state
//!
//! - **Routing table** = the [`LinkGraph`]. Each [`add_flow`](ExchangePlane::add_flow)
//!   classifies a [`FlowConnectionIR`] into a [`LinkIR`] and interns it,
//!   yielding a dense [`LinkId`]. `source_index` maps a source routing key →
//!   the `LinkId`s sourced there (the dense analogue of FlowRouter's
//!   `HashMap<String, Vec<usize>>`).
//! - **Delivery / pull queues** are `Vec<VecDeque<…>>` indexed by [`TargetId`]
//!   — the interned per-target-endpoint id. `intern_target` is the only place
//!   a target string is interned; thereafter routing keys on the dense id.
//! - **AcceptorRegistry** (`acceptors`) stays string-keyed. **String-key
//!   fallback, documented per the brief:** accept surfaces are `"owner.port"`
//!   strings (SM `PortMessage` triggers / accepting action nodes); the
//!   LinkGraph carries no payload `SlotId`/`RuntimeId` for an accept surface in
//!   this wave (SM triggers intern in Phase 4 — the SM string-projection
//!   boundary SURVIVES, design-doc §8). There is no slot id to key on yet, so
//!   inventing one would violate fail-hard / "don't invent ids that don't
//!   exist". This is the ONE string-keyed surface that remains, matching the
//!   design-doc's "SM-delivery string-projection boundary that survives 3.5".

use std::collections::{BTreeSet, HashMap, VecDeque};

use sysml_core::Value;

use crate::flows::{
    clear_payload_at_source, FlowError, FlowEvent, FlowEventKind, FlowMessage, PortDirection,
    PortRegistry,
};
use crate::links::{LinkClass, LinkGraph, LinkIR, LinkId};

/// Cap on the per-window human-readable detail entries (counts stay exact).
/// Mirrors `flows::RECENT_DETAIL_CAP` byte-for-byte.
const RECENT_DETAIL_CAP: usize = 16;

/// Dense interned identifier for a target endpoint in an [`ExchangePlane`].
///
/// Replaces FlowRouter's `HashMap<String, VecDeque<FlowMessage>>` string keys
/// with dense `Vec`-indexed delivery / pull queues. `intern_target` is the
/// only place a target string crosses into the dense plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetId(pub u32);

impl TargetId {
    /// The dense array index this id refers to.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The internal routed-message record. [`FlowMessage`] is a PROJECTED VIEW of
/// this (see [`project_message`]) — the record carries exactly the data needed
/// to render the 6-field `FlowMessage` byte-identically with FlowRouter.
///
/// `flow_id` is stored explicitly because a MessageTransfer (occurrence
/// addressing) traverses no declared link element and synthesises its id as
/// `"message:{src}->{tgt}"`; for a declared link the id is the link's element
/// name (carried by the `FlowConnectionIR` and threaded onto the `LinkIR`).
#[derive(Clone, Debug)]
struct MessageRecord {
    /// The declared link this message traversed, when one exists. `None` for
    /// occurrence-addressed MessageTransfers (no declared link element).
    ///
    /// Retained for the [`project_message`] contract: a later wave (3.5c)
    /// renders link-derived fields (`link_class` / `via_interface`) into
    /// `flow_inspect` from this id. The 6-field [`FlowMessage`] view does not
    /// read it yet, hence `#[allow(dead_code)]`.
    #[allow(dead_code)]
    link: Option<LinkId>,
    /// The rendered `flow_id` (link element name, or `"message:{src}->{tgt}"`).
    flow_id: String,
    /// Source endpoint routing key (`"{owner}.{port}"`).
    source: String,
    /// Target endpoint routing key (`"{owner}.{port}"`).
    target: String,
    /// The payload value.
    payload: Value,
    /// Monotonic sequence number (assigned at routing time).
    sequence: u64,
}

/// Project a [`MessageRecord`] into the byte-identical [`FlowMessage`] view
/// (design-doc §8 Q5). Invariants, verified against FlowRouter's construction
/// in `route_pending` / `send`:
/// - `id` = `"msg_{sequence}"`;
/// - `flow_id` = the link element name, or `"message:{src}->{tgt}"` for an
///   occurrence-addressed MessageTransfer;
/// - `source` / `target` = `"{owner}.{port}"` routing keys.
///
/// `_link_graph` is accepted (the blueprint's `project_message(record,
/// link_graph)` signature) so a future wave can render link-derived fields
/// (`link_class` / `via_interface`) into `flow_inspect`; the 6-field
/// `FlowMessage` itself needs only the record, so it is currently unused.
fn project_message(record: &MessageRecord, _link_graph: &LinkGraph) -> FlowMessage {
    FlowMessage {
        id: format!("msg_{}", record.sequence),
        flow_id: record.flow_id.clone(),
        source: record.source.clone(),
        target: record.target.clone(),
        payload: record.payload.clone(),
        sequence: record.sequence,
    }
}

/// A message waiting to be routed (mirror of `flows::PendingMessage`).
#[derive(Debug, Clone)]
struct PendingMessage {
    source: String,
    payload: Value,
    /// The named receiver of an occurrence-addressed MessageTransfer
    /// ([`ExchangePlane::send_message`]). `None` for plain link-keyed sends.
    named_target: Option<String>,
}

/// The classified-link routing surface (RSC-3.5a). Routes on the interned
/// [`LinkGraph`]; keys delivery / pull queues on dense [`TargetId`]s. See the
/// module docs for the FlowRouter→ExchangePlane mapping.
#[derive(Clone)]
pub struct ExchangePlane {
    /// The classified routing table — the LinkGraph IS the routing table
    /// (design-doc §8). Each `add_flow` interns one classified [`LinkIR`].
    links: LinkGraph,

    /// Rendered `flow_id` per [`LinkId`] (dense, parallel to `links`). This is
    /// the `FlowConnectionIR::id` FlowRouter renders into every `FlowMessage`.
    /// [`LinkIR`] (owned by `links.rs`, un-editable this wave) has no string
    /// name field, so it is stored alongside here, keyed by the dense
    /// `LinkId::index()`. A later wave feeding a pre-built `LinkGraph` will
    /// carry the element name on the link directly.
    link_flow_ids: Vec<String>,

    /// Source routing key → the `LinkId`s sourced there (dense analogue of
    /// FlowRouter's `source_index: HashMap<String, Vec<usize>>`).
    source_index: HashMap<String, Vec<LinkId>>,

    /// Target-endpoint string → its interned dense [`TargetId`].
    target_index: HashMap<String, TargetId>,
    /// Reverse map: dense id → target string (for projection / queries).
    target_names: Vec<String>,

    /// Pending messages waiting to be routed.
    pending: VecDeque<PendingMessage>,

    /// Per-target delivery queues, dense-indexed by [`TargetId`].
    delivery_queues: Vec<VecDeque<MessageRecord>>,

    /// Per-target pull store (`is_push == false` links), dense-indexed.
    pull_pending: Vec<VecDeque<MessageRecord>>,

    /// Event log.
    events: Vec<FlowEvent>,

    /// Monotonic sequence counter.
    sequence: u64,

    /// Step counter for event tracking.
    step: u64,

    /// Participants whose actions have completed (succession ordering).
    completed_participants: std::collections::HashSet<String>,

    /// Maximum pending message queue size (memory bound).
    max_queue_size: usize,

    /// Cumulative capacity-eviction count.
    dropped_total: u64,
    /// Cumulative capacity-eviction counts keyed by source endpoint.
    dropped_by_source: HashMap<String, u64>,
    /// Capacity evictions since the last drain.
    recent_drops: HashMap<String, u64>,

    /// Cumulative count of unrouted sends (no matching link / acceptor).
    unrouted_total: u64,

    /// Whether capacity overflow is tolerated (lossy mode).
    allow_lossy: bool,

    /// Payload-conformance rejections since the last drain (RSC-3.3b D3).
    recent_conformance_rejections: Vec<String>,
    /// Cumulative payload-conformance rejection count.
    conformance_rejected_total: u64,

    /// Registered accepting surfaces for occurrence-addressed MessageTransfers
    /// (string-keyed — see the module-level fallback note). Topology, not tick
    /// state: survives [`reset`](Self::reset).
    acceptors: BTreeSet<String>,

    /// Unrouted sends since the last drain (capped detail; count exact).
    recent_unrouted: Vec<String>,
    /// Exact unrouted-send count for the current window.
    recent_unrouted_count: u64,

    /// Ambiguous occurrence-addressed sends since the last drain.
    recent_ambiguous: Vec<String>,
    /// Exact ambiguous-send count for the current window.
    recent_ambiguous_count: u64,
}

impl ExchangePlane {
    /// Create a new empty plane with the default max queue size (1000).
    pub fn new() -> Self {
        Self::with_max_queue_size(1000)
    }

    /// Create a new plane with a custom max queue size.
    pub fn with_max_queue_size(max_queue_size: usize) -> Self {
        Self {
            links: LinkGraph::new(),
            link_flow_ids: Vec::new(),
            source_index: HashMap::new(),
            target_index: HashMap::new(),
            target_names: Vec::new(),
            pending: VecDeque::new(),
            delivery_queues: Vec::new(),
            pull_pending: Vec::new(),
            events: Vec::new(),
            sequence: 0,
            step: 0,
            completed_participants: std::collections::HashSet::new(),
            max_queue_size,
            dropped_total: 0,
            dropped_by_source: HashMap::new(),
            recent_drops: HashMap::new(),
            unrouted_total: 0,
            allow_lossy: false,
            recent_conformance_rejections: Vec::new(),
            conformance_rejected_total: 0,
            acceptors: BTreeSet::new(),
            recent_unrouted: Vec::new(),
            recent_unrouted_count: 0,
            recent_ambiguous: Vec::new(),
            recent_ambiguous_count: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Interning
    // -----------------------------------------------------------------------

    /// Intern a target-endpoint string into its dense [`TargetId`], growing the
    /// delivery / pull queue arrays in lockstep so they are always indexable by
    /// the returned id.
    fn intern_target(&mut self, key: &str) -> TargetId {
        if let Some(id) = self.target_index.get(key) {
            return *id;
        }
        let id = TargetId(self.target_names.len() as u32);
        self.target_index.insert(key.to_owned(), id);
        self.target_names.push(key.to_owned());
        self.delivery_queues.push(VecDeque::new());
        self.pull_pending.push(VecDeque::new());
        id
    }

    /// The dense [`TargetId`] for a target key, if one was ever interned.
    fn target_id(&self, key: &str) -> Option<TargetId> {
        self.target_index.get(key).copied()
    }

    /// Intern a single pre-built [`LinkIR`] into the routing [`LinkGraph`] under
    /// the supplied `flow_id` label; indexes its source key → `LinkId`.
    ///
    /// This is the incremental dual of [`ingest_classified`](Self::ingest_classified)
    /// (which adopts a whole classified [`LinkGraph`] + parallel labels at once).
    /// Production routing is class-agnostic, so callers building a plane by hand
    /// typically pass [`LinkIR::message_channel`](crate::links::LinkIR::message_channel)
    /// (the conservative default the retired `FlowConnectionIR`→`link_ir_from_flow`
    /// path used to synthesize). RSC-3.5e.5 W4 retyped this from `FlowConnectionIR`
    /// to `LinkIR` when that struct was deleted.
    pub fn add_flow(&mut self, link: LinkIR, flow_id: impl Into<String>) {
        let source_key = link.source.key();
        let id = self.links.intern(link);
        debug_assert_eq!(
            id.index(),
            self.link_flow_ids.len(),
            "link_flow_ids must stay dense-parallel to the LinkGraph"
        );
        self.link_flow_ids.push(flow_id.into());
        self.source_index.entry(source_key).or_default().push(id);
    }

    /// Populate the plane from a PRE-CLASSIFIED [`LinkGraph`] (RSC-3.5c.1) —
    /// the real-class output of [`classify_links`](crate::links::classify_links),
    /// as opposed to the class-agnostic [`add_flow`](Self::add_flow) path which
    /// interns every flow as the conservative [`LinkClass::MessageChannel`]
    /// default.
    ///
    /// The plane adopts `link_graph` AS its routing table (the LinkGraph IS the
    /// routing table, design-doc §8), builds `source_index` from each
    /// [`LinkIR`]'s source endpoint exactly as `add_flow` does from
    /// `flow.source.key()`, and stores the parallel `flow_ids` label vector
    /// (LinkIR carries no string name; design-doc §8 Q5: `flow_id` =
    /// element-name where present, else `"message:{src}->{tgt}"`).
    ///
    /// `flow_ids[i]` MUST be the label for [`LinkId(i)`] — i.e. the vector is
    /// dense-parallel to the graph's interning order. [`classify_links`] is
    /// order-preserving relative to its input `flows: &[FlowConnectionIR]`
    /// (it interns one `LinkIR` per flow in iteration order before appending
    /// connector-only links), so `flow_ids` is built by walking
    /// [`LinkGraph::iter`] in order and pairing each link with its caller-
    /// supplied label (a declared element name, or the §8 Q5 synthesized
    /// `"message:{src}->{tgt}"` fallback). This API takes the already-derived
    /// labels so the plane makes no assumption about how they were threaded.
    ///
    /// Panics (debug) if `flow_ids.len() != link_graph.len()`: the label vector
    /// must be dense-parallel to the graph, mirroring `add_flow`'s
    /// `link_flow_ids`-parallel invariant.
    pub fn ingest_classified(&mut self, link_graph: LinkGraph, flow_ids: Vec<String>) {
        debug_assert_eq!(
            flow_ids.len(),
            link_graph.len(),
            "flow_ids must be dense-parallel to the classified LinkGraph (one label per LinkId)"
        );
        // Build the source index from the LinkIR endpoints — the dense analogue
        // of add_flow's `source_index.entry(flow.source.key())`.
        self.source_index.clear();
        for (idx, link) in link_graph.iter().enumerate() {
            let id = LinkId(idx as u32);
            self.source_index
                .entry(link.source.key())
                .or_default()
                .push(id);
        }
        self.links = link_graph;
        self.link_flow_ids = flow_ids;
    }

    /// The rendered `flow_id` for an interned link.
    fn flow_id_of(&self, id: LinkId) -> &str {
        self.link_flow_ids
            .get(id.index())
            .map(String::as_str)
            .expect("interned LinkId has a parallel flow_id")
    }

    /// Borrow the classified routing table.
    pub fn link_graph(&self) -> &LinkGraph {
        &self.links
    }

    /// Look up a registered link by its flow id (the `FlowConnectionIR::id`).
    pub fn flow_by_id(&self, flow_id: &str) -> Option<&LinkIR> {
        self.link_flow_ids
            .iter()
            .position(|id| id == flow_id)
            .and_then(|idx| self.links.get(LinkId(idx as u32)))
    }

    // -----------------------------------------------------------------------
    // Succession bookkeeping
    // -----------------------------------------------------------------------

    /// Mark a participant as having completed its action.
    pub fn mark_completed(&mut self, participant: &str) {
        self.completed_participants.insert(participant.to_owned());
    }

    /// Check whether a participant has completed its action.
    pub fn is_completed(&self, participant: &str) -> bool {
        self.completed_participants.contains(participant)
    }

    // -----------------------------------------------------------------------
    // Sending
    // -----------------------------------------------------------------------

    /// Queue a message for routing from a source endpoint.
    pub fn send(&mut self, source_key: &str, payload: Value) {
        self.enqueue_pending(PendingMessage {
            source: source_key.to_owned(),
            payload,
            named_target: None,
        });
    }

    /// Queue an occurrence-addressed MessageTransfer (RSC-3.3c D4).
    pub fn send_message(&mut self, source_key: &str, target: &str, payload: Value) {
        self.enqueue_pending(PendingMessage {
            source: source_key.to_owned(),
            payload,
            named_target: Some(target.to_owned()),
        });
    }

    /// Register an accepting surface for occurrence-addressed MessageTransfers
    /// (RSC-3.3c D4). String-keyed — see the module-level fallback note.
    pub fn register_acceptor(&mut self, key: impl Into<String>) {
        self.acceptors.insert(key.into());
    }

    /// The registered accepting surfaces, in sorted order.
    pub fn acceptors(&self) -> impl Iterator<Item = &str> {
        self.acceptors.iter().map(String::as_str)
    }

    /// Shared enqueue path — capacity eviction is identical for both transfer
    /// kinds (mirror of `FlowRouter::enqueue_pending` byte-for-byte).
    fn enqueue_pending(&mut self, msg: PendingMessage) {
        if self.pending.len() >= self.max_queue_size {
            if let Some(dropped) = self.pending.pop_front() {
                self.step += 1;
                self.dropped_total += 1;
                *self
                    .dropped_by_source
                    .entry(dropped.source.clone())
                    .or_insert(0) += 1;
                *self.recent_drops.entry(dropped.source.clone()).or_insert(0) += 1;
                self.events.push(FlowEvent {
                    flow_id: String::new(),
                    kind: FlowEventKind::MessageDropped {
                        source: dropped.source,
                        payload: dropped.payload,
                    },
                    step: self.step,
                });
            }
        }
        self.pending.push_back(msg);
    }

    // -----------------------------------------------------------------------
    // Routing
    // -----------------------------------------------------------------------

    /// Route all pending messages to their targets (mirror of
    /// `FlowRouter::route_pending`). Returns the delivered messages as
    /// projected [`FlowMessage`] views.
    pub fn route_pending(&mut self) -> Vec<FlowMessage> {
        self.step += 1;
        let mut delivered: Vec<MessageRecord> = Vec::new();
        let mut deferred = VecDeque::new();

        while let Some(pending) = self.pending.pop_front() {
            if let Some(link_ids) = self.source_index.get(&pending.source).cloned() {
                let mut any_deferred = false;

                for link_id in link_ids {
                    // Borrow the link facts we need, then drop the borrow so we
                    // can mutate self below.
                    let flow_id = self.flow_id_of(link_id).to_owned();
                    let link = self.links.get(link_id).expect("interned LinkId is valid");
                    // Class-aware delivery gate (RSC-3.5c.1, design-doc D-3.0.2):
                    // PowerBond links are continuous power feeding the DAE
                    // machinery, NOT discrete messages — they produce NO message
                    // delivery here. MessageChannel / SignalLink / Unknown keep
                    // delivering exactly as before. (SignalLink's slot-
                    // propagation migration is RSC-3.5d, untouched here.) This
                    // is a strict no-op for `add_flow`-populated planes, which
                    // intern every link as MessageChannel — no PowerBond exists,
                    // so the gate never fires and rsc3 parity stays byte-
                    // identical.
                    if link.class == LinkClass::PowerBond {
                        continue;
                    }
                    let is_succession = link.is_succession;
                    let src_participant = link.source.owner.clone();
                    let target_key = link.target.key();
                    let payload_type = link.payload_type.clone();
                    let source_payload_type = link.source_payload_type.clone();
                    let target_payload_type = link.target_payload_type.clone();
                    let is_push = link.is_push;

                    // S2a: Succession ordering — block if source not yet complete.
                    if is_succession && !self.completed_participants.contains(&src_participant) {
                        self.events.push(FlowEvent {
                            flow_id: flow_id.clone(),
                            kind: FlowEventKind::SuccessionBlocked {
                                flow_id: flow_id.clone(),
                                source: pending.source.clone(),
                                payload: pending.payload.clone(),
                            },
                            step: self.step,
                        });
                        any_deferred = true;
                        continue;
                    }

                    // S2b: Payload type checking (explicit payload_type prop).
                    if let Some(ref expected_type) = payload_type {
                        let actual_type = pending.payload.type_name();
                        if !is_type_compatible(actual_type, expected_type) {
                            self.events.push(FlowEvent {
                                flow_id: flow_id.clone(),
                                kind: FlowEventKind::TypeMismatch {
                                    flow_id: flow_id.clone(),
                                    expected: expected_type.clone(),
                                    actual: actual_type.to_owned(),
                                    payload: pending.payload.clone(),
                                },
                                step: self.step,
                            });
                            continue;
                        }
                    }

                    // S2c (RSC-3.3b D3): payload-subset conformance against the
                    // DERIVED endpoint typings.
                    let mut conformance_violation: Option<(&'static str, String)> = None;
                    for (role, declared) in [
                        ("sourceOutput", source_payload_type.as_deref()),
                        ("targetInput", target_payload_type.as_deref()),
                    ] {
                        let Some(declared) = declared else { continue };
                        if value_kind_provably_nonconformant(pending.payload.type_name(), declared)
                        {
                            conformance_violation = Some((role, declared.to_owned()));
                            break;
                        }
                    }
                    if let Some((role, declared)) = conformance_violation {
                        let detail = format!(
                            "flow '{}': payload of type '{}' does not conform to the {} type \
                             '{}'",
                            flow_id,
                            pending.payload.type_name(),
                            role,
                            declared,
                        );
                        self.events.push(FlowEvent {
                            flow_id: flow_id.clone(),
                            kind: FlowEventKind::TypeMismatch {
                                flow_id: flow_id.clone(),
                                expected: declared,
                                actual: pending.payload.type_name().to_owned(),
                                payload: pending.payload.clone(),
                            },
                            step: self.step,
                        });
                        self.conformance_rejected_total += 1;
                        self.recent_conformance_rejections.push(detail);
                        continue;
                    }

                    // U1 (RSC-3.3c): a pull link (`is_push == false`) parks the
                    // transfer per target; delivery happens on `pull`.
                    if !is_push {
                        self.sequence += 1;
                        let record = MessageRecord {
                            link: Some(link_id),
                            flow_id: flow_id.clone(),
                            source: pending.source.clone(),
                            target: target_key.clone(),
                            payload: pending.payload.clone(),
                            sequence: self.sequence,
                        };
                        self.events.push(FlowEvent {
                            flow_id: flow_id.clone(),
                            kind: FlowEventKind::PullParked {
                                flow_id: flow_id.clone(),
                                target: target_key.clone(),
                                payload: record.payload.clone(),
                            },
                            step: self.step,
                        });
                        let tid = self.intern_target(&target_key);
                        self.pull_pending[tid.index()].push_back(record);
                        continue;
                    }

                    // Eager push delivery.
                    self.sequence += 1;
                    let record = MessageRecord {
                        link: Some(link_id),
                        flow_id: flow_id.clone(),
                        source: pending.source.clone(),
                        target: target_key.clone(),
                        payload: pending.payload.clone(),
                        sequence: self.sequence,
                    };
                    self.events.push(FlowEvent {
                        flow_id: flow_id.clone(),
                        kind: FlowEventKind::MessageDelivered {
                            target: target_key.clone(),
                            payload: record.payload.clone(),
                        },
                        step: self.step,
                    });
                    let tid = self.intern_target(&target_key);
                    self.delivery_queues[tid.index()].push_back(record.clone());
                    delivered.push(record);
                }

                if any_deferred {
                    deferred.push_back(pending);
                }
            } else if let Some(target) = pending.named_target.as_deref() {
                // RSC-3.3c D4: occurrence/participant addressing.
                let candidates = self.resolve_acceptors(target);
                match candidates.len() {
                    1 => {
                        let acceptor = candidates.into_iter().next().expect("len checked");
                        self.sequence += 1;
                        let record = MessageRecord {
                            link: None,
                            flow_id: format!("message:{}->{}", pending.source, acceptor),
                            source: pending.source.clone(),
                            target: acceptor.clone(),
                            payload: pending.payload.clone(),
                            sequence: self.sequence,
                        };
                        self.events.push(FlowEvent {
                            flow_id: record.flow_id.clone(),
                            kind: FlowEventKind::MessageDelivered {
                                target: acceptor.clone(),
                                payload: record.payload.clone(),
                            },
                            step: self.step,
                        });
                        let tid = self.intern_target(&acceptor);
                        self.delivery_queues[tid.index()].push_back(record.clone());
                        delivered.push(record);
                    }
                    0 => {
                        self.record_unrouted(&pending, Some(target.to_owned()));
                    }
                    _ => {
                        self.recent_ambiguous_count += 1;
                        if self.recent_ambiguous.len() < RECENT_DETAIL_CAP {
                            self.recent_ambiguous.push(format!(
                                "send from '{}' to '{}' matches {} accepting surfaces: [{}]",
                                pending.source,
                                target,
                                candidates.len(),
                                candidates.join(", "),
                            ));
                        }
                        self.events.push(FlowEvent {
                            flow_id: String::new(),
                            kind: FlowEventKind::MessageAmbiguous {
                                target: target.to_owned(),
                                candidates,
                                payload: pending.payload.clone(),
                            },
                            step: self.step,
                        });
                    }
                }
            } else {
                self.record_unrouted(&pending, None);
            }
        }

        self.pending.extend(deferred);

        let graph = &self.links;
        delivered
            .iter()
            .map(|r| project_message(r, graph))
            .collect()
    }

    /// Resolve a named MessageTransfer target to candidate accepting surfaces
    /// (mirror of `FlowRouter::resolve_acceptors`).
    fn resolve_acceptors(&self, target: &str) -> Vec<String> {
        if self.acceptors.contains(target) {
            return vec![target.to_owned()];
        }
        let prefix = format!("{target}.");
        self.acceptors
            .iter()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect()
    }

    /// Record an unrouted send (mirror of `FlowRouter::record_unrouted`).
    fn record_unrouted(&mut self, pending: &PendingMessage, named_target: Option<String>) {
        self.unrouted_total += 1;
        self.recent_unrouted_count += 1;
        if self.recent_unrouted.len() < RECENT_DETAIL_CAP {
            self.recent_unrouted.push(match &named_target {
                Some(target) => format!(
                    "send from '{}' to '{}': no declared flow and no accepting surface",
                    pending.source, target
                ),
                None => format!(
                    "send from '{}': no declared flow matches the source",
                    pending.source
                ),
            });
        }
        self.events.push(FlowEvent {
            flow_id: String::new(),
            kind: FlowEventKind::MessageDropped {
                source: pending.source.clone(),
                payload: pending.payload.clone(),
            },
            step: self.step,
        });
    }

    /// Pull-initiate delivery for a target (RSC-3.3c U1, mirror of
    /// `FlowRouter::pull`).
    pub fn pull(&mut self, target_key: &str) -> Vec<FlowMessage> {
        let Some(tid) = self.target_id(target_key) else {
            return Vec::new();
        };
        let drained: Vec<MessageRecord> =
            std::mem::take(&mut self.pull_pending[tid.index()]).into();
        if drained.is_empty() {
            return Vec::new();
        }
        self.step += 1;
        for record in &drained {
            self.events.push(FlowEvent {
                flow_id: record.flow_id.clone(),
                kind: FlowEventKind::MessageDelivered {
                    target: record.target.clone(),
                    payload: record.payload.clone(),
                },
                step: self.step,
            });
            self.delivery_queues[tid.index()].push_back(record.clone());
        }
        let graph = &self.links;
        drained.iter().map(|r| project_message(r, graph)).collect()
    }

    /// Whether any transfer is parked awaiting a pull for `target_key`.
    pub fn has_pull_pending(&self, target_key: &str) -> bool {
        self.target_id(target_key)
            .is_some_and(|tid| !self.pull_pending[tid.index()].is_empty())
    }

    /// Route pending messages with port-aware value binding (mirror of
    /// `FlowRouter::route_pending_with_ports`).
    pub fn route_pending_with_ports(
        &mut self,
        registry: Option<&mut PortRegistry>,
    ) -> Vec<FlowMessage> {
        let delivered = self.route_pending();

        if let Some(reg) = registry {
            for msg in &delivered {
                debug_assert!(
                    reg.get(&msg.target)
                        .is_none_or(|p| p.direction != PortDirection::Out),
                    "flow '{}' delivers into out-direction port '{}' (FL019 should have \
                     rejected this at compile)",
                    msg.flow_id,
                    msg.target,
                );
                debug_assert!(
                    reg.get(&msg.source)
                        .is_none_or(|p| p.direction != PortDirection::In),
                    "flow '{}' picks up at in-direction port '{}' (FL019 should have \
                     rejected this at compile)",
                    msg.flow_id,
                    msg.source,
                );

                bind_payload_to_port(reg, &msg.target, &msg.payload);

                if self.flow_by_id(&msg.flow_id).is_some_and(|l| l.is_move) {
                    clear_payload_at_source(reg, &msg.source, &msg.payload);
                }
            }
        }

        delivered
    }

    /// Fail-hard variant of [`route_pending`](Self::route_pending) (mirror of
    /// `FlowRouter::route_pending_checked`).
    pub fn route_pending_checked(&mut self) -> Result<Vec<FlowMessage>, FlowError> {
        if !self.allow_lossy && !self.recent_drops.is_empty() {
            let drops = std::mem::take(&mut self.recent_drops);
            let dropped: u64 = drops.values().sum();
            let mut sources: Vec<String> = drops
                .into_iter()
                .map(|(source, count)| format!("'{source}' x{count}"))
                .collect();
            sources.sort();
            return Err(FlowError::MessageLoss {
                dropped,
                capacity: self.max_queue_size,
                detail: sources.join(", "),
            });
        }
        let delivered = self.route_pending();
        if !self.allow_lossy && !self.recent_conformance_rejections.is_empty() {
            let rejections = std::mem::take(&mut self.recent_conformance_rejections);
            return Err(FlowError::PayloadConformance {
                rejected: rejections.len() as u64,
                detail: rejections.join("; "),
            });
        }
        if !self.allow_lossy && self.recent_ambiguous_count > 0 {
            let count = std::mem::take(&mut self.recent_ambiguous_count);
            let detail = std::mem::take(&mut self.recent_ambiguous).join("; ");
            return Err(FlowError::AmbiguousMessageTarget { count, detail });
        }
        if !self.allow_lossy && self.recent_unrouted_count > 0 {
            let count = std::mem::take(&mut self.recent_unrouted_count);
            let detail = std::mem::take(&mut self.recent_unrouted).join("; ");
            return Err(FlowError::Unrouted { count, detail });
        }
        Ok(delivered)
    }

    // -----------------------------------------------------------------------
    // Drop / unrouted / conformance accounting (RSC-1.5)
    // -----------------------------------------------------------------------

    /// Total messages evicted from the pending queue at capacity since
    /// construction (or the last [`reset`](Self::reset)).
    pub fn dropped_message_total(&self) -> u64 {
        self.dropped_total
    }

    /// Cumulative capacity-eviction counts keyed by source endpoint.
    pub fn dropped_message_counts(&self) -> &HashMap<String, u64> {
        &self.dropped_by_source
    }

    /// Total messages dropped because no link matched their source key.
    pub fn unrouted_message_total(&self) -> u64 {
        self.unrouted_total
    }

    /// Cumulative payload-conformance rejection count (RSC-3.3b D3).
    pub fn conformance_rejected_total(&self) -> u64 {
        self.conformance_rejected_total
    }

    /// Drain the payload-conformance rejections recorded since the last drain.
    pub fn take_recent_conformance_rejections(&mut self) -> Vec<String> {
        std::mem::take(&mut self.recent_conformance_rejections)
    }

    /// Drain the capacity evictions recorded since the last drain.
    pub fn take_recent_drops(&mut self) -> HashMap<String, u64> {
        std::mem::take(&mut self.recent_drops)
    }

    /// The configured pending-queue capacity.
    pub fn max_queue_size(&self) -> usize {
        self.max_queue_size
    }

    /// Whether lossy mode is enabled.
    pub fn allow_lossy(&self) -> bool {
        self.allow_lossy
    }

    /// Opt in to (or back out of) lossy flows.
    pub fn set_allow_lossy(&mut self, allow_lossy: bool) {
        self.allow_lossy = allow_lossy;
    }

    // -----------------------------------------------------------------------
    // Receiving / inspection
    // -----------------------------------------------------------------------

    /// Consume the next message from a target's delivery queue.
    pub fn receive(&mut self, target_key: &str) -> Option<FlowMessage> {
        let tid = self.target_id(target_key)?;
        let record = self.delivery_queues[tid.index()].pop_front()?;
        Some(project_message(&record, &self.links))
    }

    /// Peek at the next message without consuming it.
    pub fn peek(&self, target_key: &str) -> Option<FlowMessage> {
        let tid = self.target_id(target_key)?;
        self.delivery_queues[tid.index()]
            .front()
            .map(|r| project_message(r, &self.links))
    }

    /// Check if a target has pending messages.
    pub fn has_messages(&self, target_key: &str) -> bool {
        self.target_id(target_key)
            .is_some_and(|tid| !self.delivery_queues[tid.index()].is_empty())
    }

    /// Get all flow events (for tracing/debugging).
    pub fn events(&self) -> &[FlowEvent] {
        &self.events
    }

    /// Clear the event log.
    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    /// Reset the plane (clear all queues, events, and drop counters).
    ///
    /// Topology survives: the routing [`LinkGraph`] AND registered accepting
    /// surfaces are compile-time structure, not tick state — matching
    /// `FlowRouter::reset` exactly. The dense target queues are cleared in
    /// place (not deallocated) so interned [`TargetId`]s stay valid; only the
    /// queued records are discarded.
    pub fn reset(&mut self) {
        self.pending.clear();
        for q in &mut self.delivery_queues {
            q.clear();
        }
        for q in &mut self.pull_pending {
            q.clear();
        }
        self.events.clear();
        self.completed_participants.clear();
        self.sequence = 0;
        self.step = 0;
        self.dropped_total = 0;
        self.dropped_by_source.clear();
        self.recent_drops.clear();
        self.unrouted_total = 0;
        self.recent_conformance_rejections.clear();
        self.conformance_rejected_total = 0;
        self.recent_unrouted.clear();
        self.recent_unrouted_count = 0;
        self.recent_ambiguous.clear();
        self.recent_ambiguous_count = 0;
    }
}

impl Default for ExchangePlane {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Route-time conformance / binding helpers
//
// These mirror the PRIVATE `flows::{is_type_compatible,
// value_kind_provably_nonconformant, bind_payload_to_port}` byte-for-byte.
// They are reimplemented here (not re-exported) because this wave is ADDITIVE
// and must NOT edit `flows/mod.rs` to widen their visibility. `flows::
// clear_payload_at_source` is already `pub(crate)` and is reused directly. A
// later wave (3.5e) that deletes FlowRouter will collapse these to one home.
// ---------------------------------------------------------------------------

/// Mirror of `flows::is_type_compatible` (the explicit `payload_type`
/// route-check: exact match + Int→Real/float widening).
fn is_type_compatible(actual: &str, expected: &str) -> bool {
    if actual == expected {
        return true;
    }
    if actual == "int" && (expected == "float" || expected == "Real") {
        return true;
    }
    false
}

/// Mirror of `flows::value_kind_provably_nonconformant` (RSC-3.3b D3, the
/// ScalarValues.kerml lattice). Only PROVABLE scalar mismatches return true;
/// unknown declared names and structured value kinds make no claim.
fn value_kind_provably_nonconformant(value_kind: &str, declared: &str) -> bool {
    fn local(name: &str) -> &str {
        name.rsplit("::").next().unwrap_or(name)
    }
    fn is_known_scalar(name: &str) -> bool {
        matches!(
            local(name),
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
    fn conforms(value_kind: &str, declared: &str) -> bool {
        let declared = local(declared);
        let chain: &[&str] = match value_kind {
            "int" => &[
                "int",
                "float",
                "Integer",
                "Rational",
                "Real",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "float" => &[
                "float",
                "Real",
                "Complex",
                "Number",
                "NumericalValue",
                "ScalarValue",
                "DataValue",
            ],
            "bool" => &["bool", "Boolean", "ScalarValue", "DataValue"],
            "string" => &["string", "String", "ScalarValue", "DataValue"],
            _ => return false,
        };
        chain.contains(&declared)
    }
    if !is_known_scalar(declared) {
        return false;
    }
    if !matches!(value_kind, "int" | "float" | "bool" | "string") {
        return false;
    }
    !conforms(value_kind, declared)
}

/// Mirror of `flows::bind_payload_to_port`: bind a delivered payload to the
/// destination port's features (`Value::Map` field-by-name, simple value →
/// first feature, `Null`/missing port → no-op).
fn bind_payload_to_port(registry: &mut PortRegistry, target_key: &str, payload: &Value) {
    let Some(port) = registry.get_mut(target_key) else {
        return;
    };
    match payload {
        Value::Map(fields) => {
            for (name, value) in fields {
                port.set_feature_value(name, value.clone());
            }
        }
        Value::Null => {}
        value => {
            if let Some(first_name) = port.features.keys().next().cloned() {
                port.set_feature_value(&first_name, value.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parity unit tests — every test drives a FlowRouter AND an ExchangePlane with
// the SAME inputs and asserts identical observable output (design-doc §8: the
// plane reproduces FlowRouter method-for-method, byte-for-byte).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::{PortFeature, PortInstanceIR};

    /// A `MessageChannel` flow-derived [`LinkIR`] with the exchange tests'
    /// historical transfer defaults (`is_move=false`, `is_push=true`) — the
    /// values the retired `FlowConnectionIR` `flow()` helper used. (RSC-3.5e.5
    /// W4: the struct is gone; routing is class-agnostic, so `MessageChannel`
    /// is what the former `link_ir_from_flow` interned.) Tweak the public fields
    /// afterwards for succession / typed-payload cases.
    fn msg(src: (&str, &str), tgt: (&str, &str)) -> LinkIR {
        let mut l = LinkIR::message_channel(src.0, src.1, tgt.0, tgt.1);
        l.is_move = false;
        l
    }

    /// Build an [`ExchangePlane`] seeded from `(flow_id, source, target)` triples.
    ///
    /// RSC-3.5e.2: these were router-vs-plane parity tests (`both` returned a
    /// `(FlowRouter, ExchangePlane)` pair driven with identical inputs). With
    /// FlowRouter deleted, the plane is now the sole backend; each test asserts
    /// the plane's concrete observable behaviour directly (the same byte values
    /// the parity comparison pinned).
    fn plane(flows: &[(&str, (&str, &str), (&str, &str))]) -> ExchangePlane {
        let mut p = ExchangePlane::new();
        for &(id, src, tgt) in flows {
            p.add_flow(msg(src, tgt), id);
        }
        p
    }

    #[test]
    fn parity_basic_routing() {
        let mut p = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        p.send("a.out", Value::Int(42));
        let pd = p.route_pending();
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].target, "b.in");
        assert_eq!(pd[0].payload, Value::Int(42));
        assert_eq!(pd[0].id, "msg_1");
        assert_eq!(pd[0].flow_id, "f1");
    }

    #[test]
    fn fork_clone_yields_independent_routing_plane() {
        // RSC-3.5g fork-contract amendment: `Orchestrator::fork` deep-clones the
        // ExchangePlane (the sole routing backend; session-backend-contract.md
        // fork clause). A clone must share NO mutable routing state with the
        // original — sending + routing on one leaves the other's delivery
        // queues and sequence counter untouched, in BOTH directions.
        let mut parent = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        parent.send("a.out", Value::Int(1));
        parent.route_pending();

        // Fork point: the clone carries the parent's already-delivered message.
        let mut child = parent.clone();
        assert!(
            child.has_messages("b.in"),
            "clone inherits the message queued at fork time"
        );

        // Drive the parent divergently — a second message routed post-fork.
        parent.send("a.out", Value::Int(2));
        parent.route_pending();

        // Child is frozen at the fork: it sees ONLY the pre-fork message and
        // draining it does not touch the parent's queue.
        assert_eq!(child.receive("b.in").unwrap().payload, Value::Int(1));
        assert!(
            child.receive("b.in").is_none(),
            "child must NOT observe the parent's post-fork message"
        );

        // Parent still holds BOTH of its messages — the child's receive drained
        // a separate queue, proving the deep copy.
        assert_eq!(parent.receive("b.in").unwrap().payload, Value::Int(1));
        assert_eq!(parent.receive("b.in").unwrap().payload, Value::Int(2));
        assert!(parent.receive("b.in").is_none());
    }

    #[test]
    fn parity_multicast_routing() {
        let mut p = plane(&[
            ("f1", ("sensor", "data"), ("controller", "input")),
            ("f2", ("sensor", "data"), ("logger", "input")),
        ]);
        p.send("sensor.data", Value::Float(25.0));
        assert_eq!(p.route_pending().len(), 2);
    }

    #[test]
    fn parity_receive_consumes_message() {
        let mut p = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        for v in [1, 2] {
            p.send("a.out", Value::Int(v));
        }
        p.route_pending();
        for expected in [Some(1), Some(2), None] {
            let pm = p.receive("b.in");
            match expected {
                Some(v) => assert_eq!(pm.unwrap().payload, Value::Int(v)),
                None => assert!(pm.is_none()),
            }
        }
    }

    #[test]
    fn parity_unroutable_message_dropped() {
        let mut p = plane(&[]);
        p.send("unknown.port", Value::Bool(true));
        assert!(p.route_pending().is_empty());
        assert_eq!(p.events().len(), 1);
        assert!(matches!(
            &p.events()[0].kind,
            FlowEventKind::MessageDropped { .. }
        ));
        assert_eq!(p.unrouted_message_total(), 1);
    }

    #[test]
    fn parity_reset_clears_state() {
        let mut p = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        p.send("a.out", Value::Int(1));
        p.route_pending();
        assert!(p.has_messages("b.in"));
        p.reset();
        assert!(!p.has_messages("b.in"));
        assert!(p.events().is_empty());
    }

    #[test]
    fn parity_succession_ordering() {
        let mut succ = msg(("action_a", "out"), ("action_b", "in"));
        succ.is_succession = true;
        let mut p = ExchangePlane::new();
        p.add_flow(succ, "sf1");
        p.send("action_a.out", Value::Int(10));
        // Blocked until completion.
        assert!(p.route_pending().is_empty());
        assert!(!p.has_messages("action_b.in"));
        // Complete the source; now it delivers.
        p.mark_completed("action_a");
        assert!(p.is_completed("action_a"));
        let pd = p.route_pending();
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].target, "action_b.in");
        assert_eq!(pd[0].payload, Value::Int(10));
    }

    #[test]
    fn parity_type_mismatch_rejected() {
        let mut typed = msg(("a", "out"), ("b", "in"));
        typed.payload_type = Some("string".into());
        let mut p = ExchangePlane::new();
        p.add_flow(typed, "f1");
        p.send("a.out", Value::Int(42));
        assert!(p.route_pending().is_empty());
        assert!(p
            .events()
            .iter()
            .any(|e| matches!(&e.kind, FlowEventKind::TypeMismatch { .. })));
    }

    #[test]
    fn parity_compatible_type_accepted() {
        let mut typed = msg(("a", "out"), ("b", "in"));
        typed.payload_type = Some("float".into());
        let mut p = ExchangePlane::new();
        p.add_flow(typed, "f1");
        p.send("a.out", Value::Int(7));
        let pd = p.route_pending();
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].payload, Value::Int(7));
    }

    #[test]
    fn parity_untyped_flow_accepts_all() {
        let mut p = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        for v in [
            Value::Int(1),
            Value::Float(2.0),
            Value::String("hello".into()),
            Value::Bool(true),
        ] {
            p.send("a.out", v);
        }
        assert_eq!(p.route_pending().len(), 4);
    }

    #[test]
    fn parity_bounded_queue_eviction_and_counters() {
        let mut p = ExchangePlane::with_max_queue_size(3);
        p.add_flow(msg(("a", "out"), ("b", "in")), "f1");
        for v in 1..=4 {
            p.send("a.out", Value::Int(v));
        }
        let pd = p.route_pending();
        assert_eq!(pd.len(), 3);
        assert_eq!(pd[0].payload, Value::Int(2)); // oldest evicted
        assert_eq!(p.dropped_message_total(), 1);
        assert_eq!(p.dropped_message_counts().get("a.out"), Some(&1));
    }

    #[test]
    fn parity_strict_mode_capacity_error() {
        let mut p = ExchangePlane::with_max_queue_size(1);
        assert!(!p.allow_lossy()); // strict (fail-hard) is the default
        p.add_flow(msg(("a", "out"), ("b", "in")), "f1");
        p.send("a.out", Value::Int(1));
        p.send("a.out", Value::Int(2));
        let pe = p.route_pending_checked().expect_err("strict fails");
        let msg = pe.to_string();
        assert!(msg.contains("dropped 1 message"), "got: {msg}");
        assert!(msg.contains("capacity 1"), "got: {msg}");
        assert!(msg.contains("'a.out'"), "got: {msg}");
        // Window drained → next pass clean.
        let pd = p.route_pending_checked().expect("clean");
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].payload, Value::Int(2));
    }

    #[test]
    fn parity_lossy_mode_warn_only() {
        let mut p = ExchangePlane::with_max_queue_size(1);
        p.set_allow_lossy(true);
        p.add_flow(msg(("a", "out"), ("b", "in")), "f1");
        p.send("a.out", Value::Int(1));
        p.send("a.out", Value::Int(2));
        let pd = p.route_pending_checked().expect("lossy");
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].payload, Value::Int(2));
        assert_eq!(p.dropped_message_total(), 1);
    }

    #[test]
    fn parity_unrouted_vs_capacity_separation() {
        let mut p = plane(&[]);
        p.send("nowhere.out", Value::Int(1));
        let pe = p.route_pending_checked().expect_err("unrouted");
        assert!(matches!(pe, FlowError::Unrouted { count: 1, .. }));
        assert_eq!(p.unrouted_message_total(), 1);
        assert_eq!(p.dropped_message_total(), 0);
    }

    #[test]
    fn parity_message_transfer_named_acceptor() {
        let mut p = plane(&[]);
        p.register_acceptor("consumer.statusIn");
        p.send_message("producer.statusOut", "consumer.statusIn", Value::Int(7));
        let pd = p.route_pending();
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].source, "producer.statusOut");
        assert_eq!(pd[0].target, "consumer.statusIn");
        assert_eq!(pd[0].payload, Value::Int(7));
        assert!(pd[0].flow_id.starts_with("message:"));
        assert_eq!(
            p.receive("consumer.statusIn").unwrap().payload,
            Value::Int(7)
        );
        assert_eq!(p.unrouted_message_total(), 0);
    }

    #[test]
    fn parity_message_transfer_participant_addressing() {
        let mut p = plane(&[]);
        p.register_acceptor("breaker.tripIn");
        p.send_message("firmware.tripOut", "breaker", Value::Bool(true));
        let pd = p.route_pending();
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].target, "breaker.tripIn");
    }

    #[test]
    fn parity_message_transfer_ambiguous_fails_loud() {
        let mut p = plane(&[]);
        for a in ["breaker.tripIn", "breaker.resetIn"] {
            p.register_acceptor(a);
        }
        p.send_message("firmware.tripOut", "breaker", Value::Bool(true));
        let pe = p.route_pending_checked().expect_err("ambiguous");
        match &pe {
            FlowError::AmbiguousMessageTarget { count, detail } => {
                assert_eq!(*count, 1);
                assert!(
                    detail.contains("breaker.resetIn, breaker.tripIn"),
                    "got: {detail}"
                );
            }
            other => panic!("expected AmbiguousMessageTarget, got: {other}"),
        }
        assert!(p.receive("breaker.tripIn").is_none());
        assert!(p.receive("breaker.resetIn").is_none());
        assert!(p
            .events()
            .iter()
            .any(|e| matches!(e.kind, FlowEventKind::MessageAmbiguous { .. })));
    }

    #[test]
    fn parity_message_transfer_zero_acceptors_strict_loss() {
        let mut p = plane(&[]);
        p.send_message("producer.statusOut", "nobody", Value::Int(1));
        let pe = p.route_pending_checked().expect_err("unrouted");
        assert!(matches!(pe, FlowError::Unrouted { count: 1, .. }));
        assert_eq!(p.unrouted_message_total(), 1);
    }

    #[test]
    fn parity_declared_flow_wins_over_occurrence_addressing() {
        let mut declared = msg(("sensor", "out"), ("monitor", "in"));
        declared.is_move = true;
        let mut p = ExchangePlane::new();
        p.add_flow(declared, "f1");
        p.register_acceptor("elsewhere.in");
        p.send_message("sensor.out", "elsewhere.in", Value::Int(3));
        let pd = p.route_pending();
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].target, "monitor.in", "declared flow wins");
        assert_eq!(pd[0].flow_id, "f1");
        assert!(p.receive("elsewhere.in").is_none());
    }

    #[test]
    fn parity_pull_suppress_until_pull() {
        let mut pull_flow = msg(("tank", "out"), ("brewer", "in"));
        pull_flow.is_push = false;
        pull_flow.is_move = true;
        let mut p = ExchangePlane::new();
        p.add_flow(pull_flow, "pf");
        p.send("tank.out", Value::Float(1.5));
        // No eager delivery.
        assert!(p.route_pending().is_empty());
        assert!(p.receive("brewer.in").is_none());
        assert!(p.has_pull_pending("brewer.in"));
        assert!(p
            .events()
            .iter()
            .any(|e| matches!(e.kind, FlowEventKind::PullParked { .. })));
        // Pull round-trip.
        let pp = p.pull("brewer.in");
        assert_eq!(pp.len(), 1);
        assert_eq!(pp[0].payload, Value::Float(1.5));
        assert_eq!(pp[0].flow_id, "pf");
        assert_eq!(p.receive("brewer.in").unwrap().payload, Value::Float(1.5));
        // Drained.
        assert!(p.pull("brewer.in").is_empty());
        assert!(!p.has_pull_pending("brewer.in"));
    }

    fn water_port(owner: &str, name: &str, dir: PortDirection) -> PortInstanceIR {
        let mut port = PortInstanceIR::new(owner, name)
            .with_definition("WaterPort")
            .with_direction(dir);
        port.add_feature(PortFeature {
            name: "flowRate".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        port.add_feature(PortFeature {
            name: "temperature".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        port
    }

    #[test]
    fn parity_route_with_ports_map_payload() {
        let mut p = plane(&[("waterFlow", ("tank", "waterOut"), ("brewer", "waterIn"))]);
        let mut reg = PortRegistry::new();
        reg.register(water_port("tank", "waterOut", PortDirection::Out));
        reg.register(water_port("brewer", "waterIn", PortDirection::In));

        let mut fields = std::collections::BTreeMap::new();
        fields.insert("flowRate".to_owned(), Value::Float(1.5));
        fields.insert("temperature".to_owned(), Value::Float(92.0));
        p.send("tank.waterOut", Value::Map(fields));
        assert_eq!(p.route_pending_with_ports(Some(&mut reg)).len(), 1);
        assert_eq!(
            reg.get("brewer.waterIn")
                .unwrap()
                .get_feature_value("flowRate"),
            Some(&Value::Float(1.5)),
        );
        assert_eq!(
            reg.get("brewer.waterIn")
                .unwrap()
                .get_feature_value("temperature"),
            Some(&Value::Float(92.0)),
        );
    }

    #[test]
    fn parity_route_with_ports_simple_payload() {
        let mut p = plane(&[("tempFlow", ("sensor", "tempOut"), ("display", "tempIn"))]);
        let mut reg = PortRegistry::new();
        let mut sport = PortInstanceIR::new("sensor", "tempOut").with_direction(PortDirection::Out);
        sport.add_feature(PortFeature {
            name: "value".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(sport);
        let mut dport = PortInstanceIR::new("display", "tempIn").with_direction(PortDirection::In);
        dport.add_feature(PortFeature {
            name: "value".into(),
            direction: PortDirection::In,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(dport);

        p.send("sensor.tempOut", Value::Float(98.6));
        p.route_pending_with_ports(Some(&mut reg));
        assert_eq!(
            reg.get("display.tempIn")
                .unwrap()
                .get_feature_value("value"),
            Some(&Value::Float(98.6)),
        );
    }

    #[test]
    fn parity_route_with_ports_none_registry_noop() {
        let mut p = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        p.send("a.out", Value::Float(42.0));
        let pd = p.route_pending_with_ports(None);
        assert_eq!(pd.len(), 1);
        assert_eq!(pd[0].target, "b.in");
    }

    /// D2 (rsc3_exchange_baseline twin): is_move clears the source feature on
    /// delivery, and every send is its own transfer instance.
    #[test]
    fn parity_is_move_clears_source_on_delivery() {
        let mut move_flow = msg(("producer", "outPort"), ("consumer", "inPort"));
        move_flow.is_move = true;
        let mut p = ExchangePlane::new();
        p.add_flow(move_flow, "moveFlow");

        let build_reg = || {
            let mut reg = PortRegistry::new();
            let mut src =
                PortInstanceIR::new("producer", "outPort").with_direction(PortDirection::Out);
            src.add_feature(PortFeature {
                name: "datum".into(),
                direction: PortDirection::Out,
                type_name: Some("Real".into()),
                value: Value::Float(42.0),
            });
            reg.register(src);
            let mut tgt =
                PortInstanceIR::new("consumer", "inPort").with_direction(PortDirection::In);
            tgt.add_feature(PortFeature {
                name: "datum".into(),
                direction: PortDirection::In,
                type_name: Some("Real".into()),
                value: Value::Null,
            });
            reg.register(tgt);
            reg
        };
        let mut reg = build_reg();

        p.send("producer.outPort", Value::Float(42.0));
        p.route_pending_with_ports(Some(&mut reg));
        assert_eq!(
            reg.get("consumer.inPort").unwrap().features["datum"].value,
            Value::Float(42.0),
        );
        assert_eq!(
            reg.get("producer.outPort").unwrap().features["datum"].value,
            Value::Null,
            "payload leaves the source"
        );

        // Second transfer instance delivers again and clears again.
        reg.get_mut("producer.outPort")
            .unwrap()
            .set_feature_value("datum", Value::Float(7.0));
        p.send("producer.outPort", Value::Float(7.0));
        let pd = p.route_pending_with_ports(Some(&mut reg));
        assert_eq!(pd.len(), 1);
        assert_eq!(
            reg.get("producer.outPort").unwrap().features["datum"].value,
            Value::Null,
        );
    }

    #[test]
    fn parity_peek_and_clear_events() {
        let mut p = plane(&[("f1", ("a", "out"), ("b", "in"))]);
        p.send("a.out", Value::Int(5));
        p.route_pending();
        assert_eq!(p.peek("b.in").map(|m| m.payload), Some(Value::Int(5)));
        // peek does not consume.
        assert!(p.has_messages("b.in"));
        p.clear_events();
        assert!(p.events().is_empty());
    }

    #[test]
    fn parity_flow_by_id_and_conformance_counters() {
        let mut typed = msg(("a", "out"), ("b", "in"));
        typed.source_payload_type = Some("Integer".into());
        let mut p = ExchangePlane::new();
        p.add_flow(typed, "cf");
        // float payload provably non-conformant to Integer source typing.
        p.send("a.out", Value::Float(1.0));
        assert!(p.route_pending().is_empty());
        assert_eq!(p.conformance_rejected_total(), 1);
        assert!(!p.take_recent_conformance_rejections().is_empty());
        assert!(p.flow_by_id("cf").is_some());
        assert!(p.flow_by_id("nope").is_none());
    }

    #[test]
    fn parity_max_queue_size_defaults() {
        assert_eq!(ExchangePlane::new().max_queue_size(), 1000);
        assert_eq!(
            ExchangePlane::with_max_queue_size(500).max_queue_size(),
            500
        );
    }

    // -----------------------------------------------------------------------
    // RSC-3.5c.1 — classified-LinkGraph ingestion + class-aware delivery gate
    // -----------------------------------------------------------------------

    use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
    use sysml_core::physics::classify::ClassificationConfidence;
    use sysml_core::ElementId;

    /// Build a classified `LinkIR` with an explicit class (the synthetic
    /// equivalent of a `classify_links` output, endpoints name-keyed).
    fn classified_link(
        seed: &str,
        src: (&str, &str),
        tgt: (&str, &str),
        class: LinkClass,
    ) -> LinkIR {
        LinkIR {
            element_id: ElementId::from_string(format!("test-link:{seed}")),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: src.0.to_owned(),
                port: src.1.to_owned(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: tgt.0.to_owned(),
                port: tgt.1.to_owned(),
                resolved_registry_key: None,
            },
            class,
            class_confidence: ClassificationConfidence::Unknown,
            is_succession: false,
            is_move: false,
            is_push: true,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        }
    }

    /// A classified plane holding one link of each routed class plus a
    /// PowerBond, populated through `ingest_classified`.
    fn classified_plane() -> ExchangePlane {
        let mut graph = LinkGraph::new();
        // Interning order == flow_ids order (classify_links is order-preserving).
        graph.intern(classified_link(
            "power",
            ("supply", "powerOut"),
            ("breaker", "powerIn"),
            LinkClass::PowerBond,
        ));
        graph.intern(classified_link(
            "msg",
            ("controller", "tripOut"),
            ("breaker", "tripIn"),
            LinkClass::MessageChannel,
        ));
        graph.intern(classified_link(
            "signal",
            ("sensor", "currentOut"),
            ("controller", "currentIn"),
            LinkClass::SignalLink,
        ));
        let flow_ids = vec![
            "powerBond".to_owned(),
            "tripCmd".to_owned(),
            "reading".to_owned(),
        ];
        let mut plane = ExchangePlane::new();
        plane.ingest_classified(graph, flow_ids);
        plane
    }

    /// The gate: a PowerBond source produces ZERO deliveries (it feeds the DAE
    /// machinery, not the message plane — D-3.0.2). The MessageChannel and
    /// SignalLink sources deliver exactly as before.
    #[test]
    fn classified_power_bond_excluded_message_and_signal_deliver() {
        // PowerBond source — no message delivery.
        let mut plane = classified_plane();
        plane.send("supply.powerOut", Value::Float(230.0));
        let delivered = plane.route_pending();
        assert!(
            delivered.is_empty(),
            "PowerBond source produces no message delivery (fed to the DAE plane)"
        );
        assert!(
            !plane.has_messages("breaker.powerIn"),
            "no message queued at a PowerBond target"
        );

        // MessageChannel source — delivers normally.
        let mut plane = classified_plane();
        plane.send("controller.tripOut", Value::Bool(true));
        let delivered = plane.route_pending();
        assert_eq!(delivered.len(), 1, "MessageChannel delivers");
        assert_eq!(delivered[0].target, "breaker.tripIn");
        assert_eq!(delivered[0].flow_id, "tripCmd");
        assert_eq!(delivered[0].payload, Value::Bool(true));

        // SignalLink source — still delivers through the message plane in this
        // wave (its slot-propagation migration is RSC-3.5d, not here).
        let mut plane = classified_plane();
        plane.send("sensor.currentOut", Value::Float(12.5));
        let delivered = plane.route_pending();
        assert_eq!(
            delivered.len(),
            1,
            "SignalLink delivers (unchanged in 3.5c)"
        );
        assert_eq!(delivered[0].target, "controller.currentIn");
        assert_eq!(delivered[0].flow_id, "reading");
        assert_eq!(delivered[0].payload, Value::Float(12.5));
    }

    /// The HARD parity requirement: an `add_flow`-populated plane interns every
    /// link as MessageChannel, so NO PowerBond exists and the gate is a strict
    /// no-op — every send delivers, exactly as before the gate existed. (This
    /// is the byte-identical-rsc3 safety argument expressed as a unit test.)
    #[test]
    fn add_flow_plane_gate_is_noop_all_deliver() {
        let mut plane = ExchangePlane::new();
        // Same endpoint as the PowerBond above, but interned via add_flow → it
        // is a MessageChannel, so the gate does NOT fire.
        plane.add_flow(msg(("supply", "powerOut"), ("breaker", "powerIn")), "pb");
        plane.add_flow(msg(("controller", "tripOut"), ("breaker", "tripIn")), "mc");
        plane.send("supply.powerOut", Value::Float(230.0));
        plane.send("controller.tripOut", Value::Bool(true));
        let delivered = plane.route_pending();
        assert_eq!(
            delivered.len(),
            2,
            "add_flow planes have no PowerBond — gate is a no-op, both deliver"
        );
        assert!(plane.has_messages("breaker.powerIn"));
        assert!(plane.has_messages("breaker.tripIn"));
    }

    /// `ingest_classified` builds `source_index` / `link_flow_ids` such that the
    /// labels project byte-correctly (the flow_id is the caller-supplied label,
    /// paired by interning order) and `flow_by_id` resolves them.
    #[test]
    fn ingest_classified_pairs_labels_by_interning_order() {
        let plane = classified_plane();
        // Labels resolve to the correct interned link (dense-parallel pairing).
        assert_eq!(
            plane.flow_by_id("powerBond").unwrap().class,
            LinkClass::PowerBond
        );
        assert_eq!(
            plane.flow_by_id("tripCmd").unwrap().class,
            LinkClass::MessageChannel
        );
        assert_eq!(
            plane.flow_by_id("reading").unwrap().class,
            LinkClass::SignalLink
        );
        assert_eq!(plane.link_graph().len(), 3);
    }
}
