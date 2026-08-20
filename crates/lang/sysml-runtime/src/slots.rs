//! RSC-2.1/2.2 — `RuntimeId` + typed slot store (ADR-017 D1/D2).
//!
//! Element-relational runtime identity (L0) and dense, single-writer value
//! §3 D-2.0.1 / D-2.0.2 / D-2.0.3 / D-2.0.6.
//!
//! As of RSC-2.2 the slot table is **live storage behind the legacy map**:
//! the compiler mints it during `build_orchestrator` /
//! `build_workspace_orchestrator`, the orchestrator wraps it in a
//! [`SharedSlotStore`] handle attached to its master `EvalContext`, and every
//! `EvalContext::set` on that context write-throughs to the bound slot while
//! keeping the legacy name map coherent (the wire contract still serializes
//! the map). RSC-2.3+ binds expressions to `SlotId`s directly. (The RSC-2.2
//! `enabled` rollback gate was removed in RSC-3.5f.3 — slot routing is now
//! unconditional, the sole execution path.)
//!
//! ## Non-slot runtime state (taxonomy — cull-arc W4)
//!
//! Not every runtime value is — or should be — a slot. A value earns a slot
//! iff it has all three of: (a) a stable compile-time [`RuntimeId`] (decl +
//! usage-path `ElementId`s), (b) a single sanctioned tick-time [`WriterId`],
//! and (c) cross-tick persistence as a model feature/state variable. State
//! that legitimately lives OUTSIDE the slot plane — and why minting a slot for
//! it would be meaningless or wrong — falls in four categories:
//!
//! 1. **Expression-scratch locals.** Lambda iteration variables
//!    ([`EvalContext::child_with`](crate::expressions::EvalContext::child_with)
//!    for `select`/`collect`/`reduce`), calc-def argument bindings, and ODE
//!    RHS-stage scratch values. Bound directly into the `variables` map for the
//!    span of one expression/stage evaluation; no cross-tick identity, no
//!    writer — fails (a) and (c).
//! 2. **Speculative / throwaway evaluation.** Everything written into a
//!    [`scratch_snapshot`](crate::expressions::EvalContext::scratch_snapshot)
//!    (read-only-demoted) context: numeric residual/root-finding probes, Monte
//!    Carlo samples, trade-study alternatives, what-if overlays, compile-time
//!    constant folding. These MUST stay local (the context is demoted precisely
//!    so `set` never writes through); slotting them would defeat the isolation.
//! 3. **Runtime-dynamic keys with no compile-time identity.** Names bound at
//!    tick time whose identity is not compile-enumerable — the local-clock
//!    `__clock_time` key and any residual dynamic keys carried by the name-keyed
//!    `slot_write_fallbacks` class. (Port payloads that ARE compile-static are
//!    pre-minted as `Discrete` slots; only the genuinely dynamic residue stays
//!    name-keyed.) Fails (a): no stable `RuntimeId`.
//! 4. **The legacy `variables` map itself.** `EvalContext.variables` is the
//!    coherence mirror and wire/serialization surface, NOT a parallel identity
//!    plane. Slot-bound values are mirrored INTO it (every spelling, via
//!    `set_slot` + `aliases_of`) so name-first reads and serialization see live
//!    values; the map is downstream of slot identity, never a source of it.
//!
//! So "shouldn't everything be a slot?" has a bounded answer: no — categories
//! 1–3 are ephemeral or identity-less by construction, and category 4 is the
//! projection of the slot plane, not a rival to it.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use smallvec::SmallVec;
use sysml_core::physics::DimensionVector;
use sysml_core::{ElementId, Value};

/// RSC-5 (D-5.0.1) — the compile-time-static measurement reference for a slot.
///
/// A feature's ISQ typing does not change at runtime, so the measurement
/// reference is **metadata**: it lives in [`SlotMeta`], never in the per-tick
/// `Value`. This keeps the dense `Vec<Value>` hot path as bare f64 magnitudes
/// (ADR-017 D2) and reconstitutes `Value::Quantity` only at the boundaries that
/// need it (snapshot rendering, `ConvertQuantity`, cross-unit bindings,
/// diagnostics).
///
/// The slot's magnitude is stored *in `unit`'s storage scale*: SI value =
/// `magnitude * scale + offset` (from `units.rs::UnitEntry`). `unit: None` means
/// an SI-derived dimension with no canonical single-unit name (e.g. `kg·m·s⁻²`).
///
/// Per steward Q1 there is **no `source: ElementId` field** — a `None`-only
/// element anchor with no population path would be a dead field. It is added
/// (with a ledger entry) only when user-declared `MeasurementUnit`s in the model
/// need resolution; SI-table units need no element anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementRef {
    /// ISQ 7-exponent dimension vector (L M T I Θ N J).
    pub dimension: DimensionVector,
    /// Canonical display/storage unit name (`"V"`, `"mA"`); `None` = SI-derived
    /// with no simple name.
    pub unit: Option<Arc<str>>,
    /// Linear scale factor to SI base (`magnitude * scale + offset = SI`).
    pub scale: f64,
    /// Additive offset to SI base (non-zero only for affine units, e.g. °C→K).
    pub offset: f64,
}

/// Element-relational runtime identity. Display names are derived metadata
/// (see [`SlotMeta::canonical_name`] / [`SlotMeta::runtime_name`]).
///
/// An `ElementId` names a *declaration* (the one `bimetalTemp` attribute in
/// the source); a `RuntimeId` names an *instance's copy* of it (ten at
/// runtime when `circuit1..circuit10` each instantiate the declaring type).
/// The declaration id is shared across instances, the usage id alone doesn't
/// say which attribute — the pair is unique.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeId {
    /// The declared feature this value belongs to (AttributeUsage, state
    /// feature, ODE state var's defining element, …).
    pub declaration: ElementId,
    /// Chain of instantiating usage elements, outermost first
    /// (e.g. `[circuit5, thermalModel]`). Empty = top-level.
    pub instance_path: SmallVec<[ElementId; 4]>,
}

impl RuntimeId {
    /// A top-level (non-instance-scoped) runtime id for a declaration.
    pub fn top_level(declaration: ElementId) -> Self {
        RuntimeId {
            declaration,
            instance_path: SmallVec::new(),
        }
    }

    /// An instance-scoped runtime id.
    pub fn scoped(
        declaration: ElementId,
        instance_path: impl IntoIterator<Item = ElementId>,
    ) -> Self {
        RuntimeId {
            declaration,
            instance_path: instance_path.into_iter().collect(),
        }
    }
}

/// Dense index into a [`SlotStore`]. Interned at compile time; per-tick
/// access is array indexing, never name hashing.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(u32);

impl SlotId {
    /// The dense index of this slot.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// How a slot's value evolves over a run (design doc D-2.0.3).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Variability {
    /// Provably never written, not even by override.
    Constant,
    /// Literal-default value written by no executor at tick time;
    /// settable out-of-band (overrides, sweeps).
    Parameter,
    /// Written by discrete executors (SM transition assignments, action
    /// assigns, accept payloads, flow gates).
    Discrete,
    /// Written continuously (ODE/discrete-state state vars, computed/gated
    /// expression targets).
    Continuous,
}

/// The single sanctioned tick-time writer of a slot (design doc D-2.0.3).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum WriterId {
    /// Index into `Orchestrator::subsystems`.
    Executor(u16),
    /// Orchestrator-internal: clock, computed/gated expressions,
    /// introspection vars, succession bookkeeping.
    Orchestrator,
    /// No tick-time writer. Parameter/Constant slots.
    External,
}

impl From<crate::orchestrator::SubsystemIndex> for WriterId {
    fn from(index: crate::orchestrator::SubsystemIndex) -> Self {
        WriterId::Executor(index.0)
    }
}

impl WriterId {
    /// The [`SubsystemIndex`](crate::orchestrator::SubsystemIndex) this writer
    /// names, or `None` for the non-executor writers (RSC-4.2 ruling 1).
    pub fn as_subsystem_index(&self) -> Option<crate::orchestrator::SubsystemIndex> {
        match self {
            WriterId::Executor(i) => Some(crate::orchestrator::SubsystemIndex(*i)),
            WriterId::Orchestrator | WriterId::External => None,
        }
    }
}

/// Per-slot metadata: identity, classification, and the precomputed display
/// names. Nothing at tick time builds or parses name strings.
#[derive(Debug, Clone)]
pub struct SlotMeta {
    /// Element-relational identity of this slot.
    pub runtime_id: RuntimeId,
    /// Value-evolution class.
    pub variability: Variability,
    /// The single sanctioned tick-time writer.
    pub writer: WriterId,
    /// Canonical tree-path name, e.g. `"Panel.circuit5.thermalModel.bimetalTemp"`.
    pub canonical_name: Arc<str>,
    /// Legacy runtime (`var_prefix`) name, e.g. `"circuit5.bimetalTemp"`.
    pub runtime_name: Arc<str>,
    /// Orchestrator bookkeeping slot (`t_ms`, `tick`, `__clock_time`,
    /// `__active_substates`, …). The contract's `__*`-prefix filtering keys
    /// off this flag instead of string sniffing.
    pub bookkeeping: bool,
    /// RSC-5 (D-5.0.2) — the slot's measurement reference, or `None` for a
    /// dimensionless/untyped slot (today's default behaviour, byte-identical).
    /// `Some` means the slot's value is a magnitude in `m_ref`'s storage unit.
    /// Minted from the attribute's ISQ type / `[unit]` annotation at slot mint;
    /// pure metadata (never in the per-tick value vector).
    pub m_ref: Option<MeasurementRef>,
}

impl SlotMeta {
    /// Construct a non-bookkeeping slot meta.
    pub fn new(
        runtime_id: RuntimeId,
        variability: Variability,
        writer: WriterId,
        canonical_name: impl Into<Arc<str>>,
        runtime_name: impl Into<Arc<str>>,
    ) -> Self {
        SlotMeta {
            runtime_id,
            variability,
            writer,
            canonical_name: canonical_name.into(),
            runtime_name: runtime_name.into(),
            bookkeeping: false,
            m_ref: None,
        }
    }

    /// Mark this slot as orchestrator bookkeeping.
    pub fn bookkeeping(mut self) -> Self {
        self.bookkeeping = true;
        self
    }

    /// Attach a [`MeasurementRef`] (RSC-5 D-5.0.2). Builder form for slot mint.
    pub fn with_m_ref(mut self, m_ref: Option<MeasurementRef>) -> Self {
        self.m_ref = m_ref;
        self
    }
}

/// Error returned by [`SlotStore::try_set`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlotWriteError {
    /// The slot is classified [`Variability::Constant`] and may not be
    /// written, not even by override.
    #[error("slot '{name}' is Constant and cannot be written")]
    ConstantSlot {
        /// The rejected slot.
        slot: SlotId,
        /// Canonical name of the rejected slot.
        name: Arc<str>,
    },
    /// The `SlotId` does not name a slot in this store.
    #[error("invalid slot id {0:?}")]
    InvalidSlot(SlotId),
}

/// Shared handle to a live [`SlotStore`] (RSC-2.2, design doc D-2.0.6).
///
/// The orchestrator owns one per session and attaches a clone to its master
/// `EvalContext` at build time; `EvalContext::clone` (scoped contexts,
/// snapshots) shares the handle so routed writes always land in the session's
/// one store. `Orchestrator::clone`/`fork` deep-copies the store into a fresh
/// handle so forked sessions share no mutable state.
pub type SharedSlotStore = Arc<RwLock<SlotStore>>;

/// Typed, dense, single-writer value storage (design doc D-2.0.2).
///
/// `values` / `meta` are parallel vectors indexed by [`SlotId`]; `by_name`
/// carries **every legal spelling** (canonical + runtime forms) so one slot
/// has several names pointing at it — replacing the runtime dual-write.
#[derive(Debug, Clone)]
pub struct SlotStore {
    /// Dense value storage, indexed by `SlotId`.
    values: Vec<Value>,
    /// Parallel metadata.
    meta: Vec<SlotMeta>,
    /// Slots touched since the last [`clear_dirty`](Self::clear_dirty), for
    /// per-tick materialization of the legacy name map (RSC-2.2+).
    dirty: Vec<SlotId>,
    /// Parallel dedupe flags for the dirty list.
    dirty_flags: Vec<bool>,
    /// Compile/override-time lookup only — never consulted at tick time.
    by_runtime_id: HashMap<RuntimeId, SlotId>,
    /// Alias table carrying BOTH name forms (canonical + runtime).
    /// First binding wins on cross-slot name collisions.
    by_name: HashMap<Arc<str>, SlotId>,
    /// Task #8 (steward-ruled): index-aligned REVERSE of `by_name` — every
    /// legal spelling bound to each slot (canonical + runtime + any
    /// `add_alias` extras), in registration order. Parallel to `values`/`meta`.
    /// Populated O(1) at the two alias chokepoints (`register_names`,
    /// `add_alias`); read by [`aliases_of`](Self::aliases_of) so
    /// `EvalContext::set_slot` can mirror the value into the legacy map under
    /// EVERY spelling — completing the "every legal spelling" contract stated
    /// on `by_name` above (the prior two-field mirror was an incomplete
    /// implementation of it, silently dropping `add_alias` spellings such as
    /// an ODE's qualified `{ode}.duty` observable key). Index, don't scan.
    aliases: Vec<SmallVec<[Arc<str>; 2]>>,
    /// Distinct tick-time writers claimed per slot (beyond the first).
    /// Input for the RS001 `multiple runtime writers` diagnostic (RSC-2.2).
    writer_claims: HashMap<SlotId, Vec<WriterId>>,
}

impl Default for SlotStore {
    fn default() -> Self {
        SlotStore {
            values: Vec::new(),
            meta: Vec::new(),
            dirty: Vec::new(),
            dirty_flags: Vec::new(),
            by_runtime_id: HashMap::new(),
            by_name: HashMap::new(),
            aliases: Vec::new(),
            writer_claims: HashMap::new(),
        }
    }
}

impl SlotStore {
    /// Create an empty store.
    pub fn new() -> Self {
        SlotStore::default()
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when no slots have been interned.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Intern a slot. Idempotent on [`RuntimeId`]: re-interning an existing
    /// identity returns the existing [`SlotId`] (the original value and
    /// classification are kept), registers any new name aliases, and records
    /// the additional writer claim when it differs — surfaced later via
    /// [`multi_writer_conflicts`](Self::multi_writer_conflicts).
    pub fn intern(&mut self, meta: SlotMeta, initial: Value) -> SlotId {
        if let Some(&id) = self.by_runtime_id.get(&meta.runtime_id) {
            self.record_writer_claim(id, meta.writer);
            self.register_names(id, &meta);
            return id;
        }
        let id = SlotId(self.values.len() as u32);
        self.by_runtime_id.insert(meta.runtime_id.clone(), id);
        // Allocate this slot's reverse alias list BEFORE registering names, so
        // `register_names` (via `bind_alias`) can record into `aliases[id]`.
        // Kept index-aligned with `values`/`meta` (all pushed below).
        self.aliases.push(SmallVec::new());
        self.register_names(id, &meta);
        self.record_writer_claim(id, meta.writer);
        self.values.push(initial);
        self.meta.push(meta);
        self.dirty_flags.push(false);
        id
    }

    /// Register an extra name alias for an existing slot (e.g. a bare name
    /// where unambiguous). First binding wins on collisions.
    pub fn add_alias(&mut self, name: impl Into<Arc<str>>, id: SlotId) {
        self.bind_alias(name.into(), id);
    }

    fn register_names(&mut self, id: SlotId, meta: &SlotMeta) {
        self.bind_alias(Arc::clone(&meta.canonical_name), id);
        self.bind_alias(Arc::clone(&meta.runtime_name), id);
    }

    /// Bind `name` to `id` in `by_name` (first-binding-wins on cross-slot
    /// collisions) AND, when it actually bound to THIS slot, record it in the
    /// index-aligned reverse [`aliases`](Self::aliases) list (deduped, so
    /// re-interning the same identity doesn't double-record). The single
    /// chokepoint keeping `by_name` and `aliases` coherent.
    fn bind_alias(&mut self, name: Arc<str>, id: SlotId) {
        let bound = *self.by_name.entry(Arc::clone(&name)).or_insert(id);
        if bound == id {
            if let Some(list) = self.aliases.get_mut(id.index()) {
                if !list.iter().any(|n| n.as_ref() == name.as_ref()) {
                    list.push(name);
                }
            }
        }
    }

    /// Task #8: every legal spelling bound to `id` (canonical + runtime + any
    /// `add_alias` extras), in registration order. Empty slice for an unknown
    /// id. This is the reverse of `by_name`, kept for `EvalContext::set_slot`'s
    /// map mirror — so a write reaches every spelling a reader might resolve,
    /// not just the two meta-name fields.
    pub fn aliases_of(&self, id: SlotId) -> &[Arc<str>] {
        self.aliases.get(id.index()).map_or(&[], |v| v.as_slice())
    }

    /// Task #8 (steward ruling 2026-07-07): does `name` match one of the slot's
    /// TWO meta spellings (`canonical_name` or `runtime_name`)?
    ///
    /// This — NOT [`aliases_of`](Self::aliases_of) — is the **observable
    /// projection** predicate: a value surfaces under key `k` for a slot IFF
    /// `k` is one of its meta spellings. Every extra spelling registered via
    /// [`add_alias`](Self::add_alias) (e.g. an ODE's qualified `{ode}.duty`
    /// observable key, minted only so the sim-app's ownerPath-driven
    /// `slot_by_name` lookups resolve) is **resolution-only** and must never
    /// appear as an observable.
    ///
    /// Note the deliberate split from the value MIRROR: `EvalContext::set_slot`
    /// still writes the live value into the legacy map under *every* spelling
    /// (via `aliases_of`) for read coherence — an alias-keyed `SlotRef` read
    /// must see the live value. The mirror carrying a spelling and the
    /// observable projection surfacing it are two different questions;
    /// `is_meta_spelling` answers the latter. Unknown ids return `false`.
    pub fn is_meta_spelling(&self, id: SlotId, name: &str) -> bool {
        self.meta.get(id.index()).is_some_and(|m| {
            m.canonical_name.as_ref() == name || m.runtime_name.as_ref() == name
        })
    }

    fn record_writer_claim(&mut self, id: SlotId, writer: WriterId) {
        let claims = self.writer_claims.entry(id).or_default();
        if !claims.contains(&writer) {
            claims.push(writer);
        }
    }

    /// Record an additional tick-time writer claim on an existing slot
    /// (RSC-2.2). Used by the compiler when a later minting pass discovers
    /// that a second executor targets a name that already resolved to a
    /// slot (e.g. an SM assignment writing an ODE state variable) — the
    /// claim feeds [`multi_writer_conflicts`](Self::multi_writer_conflicts)
    /// and therefore the RS001 hard error.
    pub fn claim_writer(&mut self, id: SlotId, writer: WriterId) {
        self.record_writer_claim(id, writer);
    }

    /// Claim a tick-time writer on an existing slot, PROMOTING slot ownership
    /// away from a non-tick-time placeholder when appropriate (RSC-2.2 /
    /// mint-ordering fix).
    ///
    /// Mint passes run in a fixed order (bookkeeping → ODE → SM assignment
    /// targets → bare bindings → …). A defaulted attribute that is ALSO an
    /// executor write-target (e.g. an SM entry action assigning `V_applied`,
    /// which also carries a literal `default` and is read by an ODE RHS so it
    /// is collected as an ODE *parameter*) is minted FIRST as
    /// [`WriterId::External`] / [`Variability::Parameter`]. When the later
    /// executor pass then targets that same name, a plain
    /// [`claim_writer`](Self::claim_writer) only records a secondary claim: the
    /// slot's *owner* (`meta.writer`) stays `External`, so
    /// [`WriteRoute::resolve`] refuses the executor's route (owner mismatch)
    /// and the write silently drops to the name-keyed fallback.
    ///
    /// `External` is by definition NOT a tick-time writer (it never appears in
    /// [`multi_writer_conflicts`](Self::multi_writer_conflicts)) — a literal
    /// default is an *initial value*, not evidence of external ownership. So
    /// when the current owner is `External` and a real tick-time `writer`
    /// claims the slot, that writer is the true owner: reassign `meta.writer`
    /// to it and reclassify `variability` to `new_variability` (the class of
    /// values the executor produces at tick time — `Discrete` for an SM
    /// assignment, `Continuous` for an ODE/state var). If the current owner is
    /// already a tick-time writer, ownership is untouched and this behaves
    /// exactly like `claim_writer` (a genuine two-executor clash still surfaces
    /// via RS001).
    pub fn claim_writer_promoting(
        &mut self,
        id: SlotId,
        writer: WriterId,
        new_variability: Variability,
    ) {
        self.record_writer_claim(id, writer);
        if let Some(meta) = self.meta.get_mut(id.index()) {
            if meta.writer == WriterId::External && writer != WriterId::External {
                meta.writer = writer;
                meta.variability = new_variability;
            }
        }
    }

    /// Attach (or clear) a slot's [`MeasurementRef`] (RSC-5.1, D-5.0.3). Called
    /// at compile-time slot mint only; mRef is static metadata, never mutated at
    /// tick time. No-op for an invalid `SlotId`.
    pub fn set_m_ref(&mut self, id: SlotId, m_ref: Option<MeasurementRef>) {
        if let Some(meta) = self.meta.get_mut(id.index()) {
            meta.m_ref = m_ref;
        }
    }

    /// Route a by-name write into the slot table (RSC-2.2 write-through).
    /// Returns `true` when the name is bound and the slot is writable
    /// (non-`Constant`); `false` otherwise. The caller (the legacy map)
    /// remains responsible for its own representation.
    pub fn set_by_name(&mut self, name: &str, value: &Value) -> bool {
        let Some(id) = self.by_name.get(name).copied() else {
            return false;
        };
        let writable = self
            .meta
            .get(id.index())
            .is_some_and(|m| m.variability != Variability::Constant);
        if !writable {
            return false;
        }
        // Cull-arc W2: slot-write journal (see `try_set`). The by-name path is
        // the orchestrator's Null-clear; `new` + this callsite identify the
        // clear, `meta.writer` names the owning executor. Feature-gated on
        // `tracing`; compiled out of the default build.
        #[cfg(feature = "tracing")]
        let (canonical, runtime, writer) = self
            .meta
            .get(id.index())
            .map(|m| {
                (
                    Arc::clone(&m.canonical_name),
                    Arc::clone(&m.runtime_name),
                    m.writer,
                )
            })
            .expect("writable check above implies the slot's meta is present");
        if let Some(v) = self.values.get_mut(id.index()) {
            #[cfg(feature = "tracing")]
            let prev = v.clone();
            *v = value.clone();
            self.mark_dirty(id);
            #[cfg(feature = "tracing")]
            tracing::debug!(
                target: "sysml_runtime::slot_write",
                slot = id.index(),
                canonical = %canonical,
                runtime = %runtime,
                writer = ?writer,
                old = ?prev,
                new = ?value,
            );
            true
        } else {
            false
        }
    }

    /// One-time coherence sync at handle-attach time (RSC-2.2): copy the
    /// legacy map's current value into every slot whose runtime or
    /// canonical name is bound in `map`, without touching the dirty list.
    /// Runtime-name binding wins over canonical when both exist (they are
    /// written equal by the compiler; the precedence only makes the sync
    /// deterministic).
    pub fn seed_from_map(&mut self, map: &HashMap<String, Value>) {
        for (i, meta) in self.meta.iter().enumerate() {
            let value = map
                .get(meta.runtime_name.as_ref())
                .or_else(|| map.get(meta.canonical_name.as_ref()));
            if let (Some(value), Some(slot)) = (value, self.values.get_mut(i)) {
                *slot = value.clone();
            }
        }
    }

    /// Read a slot's value.
    pub fn get(&self, id: SlotId) -> Option<&Value> {
        self.values.get(id.index())
    }

    /// Slot metadata.
    pub fn meta(&self, id: SlotId) -> Option<&SlotMeta> {
        self.meta.get(id.index())
    }

    /// Write a slot's value and mark it dirty. Unknown ids are ignored
    /// (debug-asserted); use [`try_set`](Self::try_set) for checked writes
    /// (Constant rejection, invalid-id errors).
    pub fn set(&mut self, id: SlotId, value: Value) {
        debug_assert!(
            self.meta
                .get(id.index())
                .is_some_and(|m| m.variability != Variability::Constant),
            "slot {id:?} is Constant or unknown — use try_set for checked writes"
        );
        if let Some(v) = self.values.get_mut(id.index()) {
            *v = value;
            self.mark_dirty(id);
        }
    }

    /// Checked write: rejects writes to [`Variability::Constant`] slots and
    /// unknown ids. On success the slot is marked dirty.
    pub fn try_set(&mut self, id: SlotId, value: Value) -> Result<(), SlotWriteError> {
        let meta = self
            .meta
            .get(id.index())
            .ok_or(SlotWriteError::InvalidSlot(id))?;
        if meta.variability == Variability::Constant {
            return Err(SlotWriteError::ConstantSlot {
                slot: id,
                name: Arc::clone(&meta.canonical_name),
            });
        }
        // Cull-arc W2: slot-write journal. Snapshot the slot's identity and its
        // *sanctioned* writer (`meta.writer`, D-2.0.3 — the registered owner,
        // never a fabricated default; WriteRoute::resolve enforces actual ==
        // sanctioned upstream) before the write releases the `meta` borrow.
        // Feature-gated on `tracing`: the default build compiles all of this
        // out, so behaviour is byte-identical and no extra clone is taken.
        #[cfg(feature = "tracing")]
        let (canonical, runtime, writer) = (
            Arc::clone(&meta.canonical_name),
            Arc::clone(&meta.runtime_name),
            meta.writer,
        );
        if let Some(v) = self.values.get_mut(id.index()) {
            #[cfg(feature = "tracing")]
            let prev = v.clone();
            *v = value;
            self.mark_dirty(id);
            #[cfg(feature = "tracing")]
            tracing::debug!(
                target: "sysml_runtime::slot_write",
                slot = id.index(),
                canonical = %canonical,
                runtime = %runtime,
                writer = ?writer,
                old = ?prev,
                new = ?self.values[id.index()],
            );
            Ok(())
        } else {
            Err(SlotWriteError::InvalidSlot(id))
        }
    }

    fn mark_dirty(&mut self, id: SlotId) {
        if let Some(flag) = self.dirty_flags.get_mut(id.index()) {
            if !*flag {
                *flag = true;
                self.dirty.push(id);
            }
        }
    }

    /// Slots written since the last [`clear_dirty`](Self::clear_dirty).
    pub fn dirty(&self) -> &[SlotId] {
        &self.dirty
    }

    /// Reset the dirty list (start of a new tick).
    pub fn clear_dirty(&mut self) {
        for id in self.dirty.drain(..) {
            if let Some(flag) = self.dirty_flags.get_mut(id.index()) {
                *flag = false;
            }
        }
    }

    /// Resolve a name (canonical or runtime form) to its slot.
    pub fn slot_by_name(&self, name: &str) -> Option<SlotId> {
        self.by_name.get(name).copied()
    }

    /// Resolve a [`RuntimeId`] to its slot. Compile/override-time only.
    pub fn slot_by_runtime_id(&self, id: &RuntimeId) -> Option<SlotId> {
        self.by_runtime_id.get(id).copied()
    }

    /// Iterate all slots as `(id, meta, value)`.
    pub fn iter(&self) -> impl Iterator<Item = (SlotId, &SlotMeta, &Value)> {
        self.meta
            .iter()
            .zip(self.values.iter())
            .enumerate()
            .map(|(i, (m, v))| (SlotId(i as u32), m, v))
    }

    /// Iterate the alias table as `(name, slot)`.
    pub fn names(&self) -> impl Iterator<Item = (&str, SlotId)> {
        self.by_name.iter().map(|(n, &id)| (n.as_ref(), id))
    }

    /// Slots claimed by more than one distinct tick-time writer
    /// ([`WriterId::External`] is not a tick-time writer and never
    /// conflicts). This is the input list for the RS001
    /// `multiple runtime writers` hard error (RSC-2.2) — emission is one
    /// line on top of this.
    pub fn multi_writer_conflicts(&self) -> Vec<(SlotId, Arc<str>, Vec<WriterId>)> {
        let mut out: Vec<(SlotId, Arc<str>, Vec<WriterId>)> = self
            .writer_claims
            .iter()
            .filter_map(|(&id, claims)| {
                let tick_writers: Vec<WriterId> = claims
                    .iter()
                    .copied()
                    .filter(|w| *w != WriterId::External)
                    .collect();
                if tick_writers.len() < 2 {
                    return None;
                }
                let name = self
                    .meta
                    .get(id.index())
                    .map(|m| Arc::clone(&m.canonical_name))?;
                Some((id, name, tick_writers))
            })
            .collect();
        out.sort_by_key(|(id, _, _)| *id);
        out
    }
}

// ---------------------------------------------------------------------------
// RSC-2.4 — precomputed executor write routes (shared by the migrated
// executor kinds: ODE since RSC-2.4a, SM since RSC-2.4b)
// ---------------------------------------------------------------------------

/// One writeback target of a migrated executor (design doc D-2.0.5).
///
/// Built once at compile time ([`WriteRoute::resolve`]); per tick the
/// executor writes through [`EvalContext::set_slot`]
/// (crate::expressions::EvalContext::set_slot) — which mirrors BOTH name
/// spellings into the legacy map — instead of the orchestrator's
/// `dot_prefix`/`canonical_dot` string-formatting loop. The key strings are
/// retained for the name-keyed fallback so the legacy map always ends the
/// tick with exactly the keys the string loop produced.
#[derive(Debug, Clone)]
pub(crate) struct WriteRoute {
    /// The slot to write, when the slot's name spellings match the legacy
    /// keys exactly (byte-identical map parity) and this executor is the
    /// slot's single sanctioned writer. `None` → name-keyed writes only.
    slot: Option<SlotId>,
    /// Legacy runtime key (`{prefix}.{var}`, or bare for unprefixed).
    runtime_key: String,
    /// Legacy canonical alias key, when the subsystem carries a canonical
    /// prefix distinct from its var prefix.
    canonical_key: Option<String>,
}

impl WriteRoute {
    /// Resolve one writeback target against the compile-minted slot table.
    ///
    /// `key` gets a slot route only when ALL of:
    /// - the legacy runtime key (`{prefix}.{key}` / bare) resolves to a slot,
    /// - the slot's `runtime_name`/`canonical_name` spellings are
    ///   byte-identical to the keys the legacy string loop would write (so
    ///   the legacy map ends the tick with exactly the same key set), and
    /// - the slot's minted writer is `expected_writer` — the single-writer
    ///   promise (design doc D-2.0.3) live for the migrated kinds. A slot
    ///   owned by a DIFFERENT executor trips the debug assertion;
    ///   orchestrator placeholder writers (mint-time name-lookup misses)
    ///   quietly keep the name-keyed fallback.
    pub(crate) fn resolve(
        store: &SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        expected_writer: WriterId,
        key: &str,
    ) -> WriteRoute {
        Self::resolve_inner(
            store,
            var_prefix,
            canonical_prefix,
            expected_writer,
            key,
            true,
        )
    }

    /// RSC-2.4d: like [`resolve`](Self::resolve), but a name-matched slot
    /// owned by a DIFFERENT executor quietly keeps the name-keyed fallback
    /// instead of tripping the single-writer debug assertion. Used by the
    /// physics executor, whose write universe (port-feature keys,
    /// solve-time-derived names) is Phase 3 exchange identity the slot
    /// table deliberately never minted — a byte-collision with a Phase 2
    /// slot there is the *documented pre-existing* collision class
    /// (design doc §2.4: port-feature key vs prefixed state key), owned by
    /// Phase 3, not a new violation introduced at prepare time.
    pub(crate) fn resolve_quiet(
        store: &SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        expected_writer: WriterId,
        key: &str,
    ) -> WriteRoute {
        Self::resolve_inner(
            store,
            var_prefix,
            canonical_prefix,
            expected_writer,
            key,
            false,
        )
    }

    fn resolve_inner(
        store: &SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        expected_writer: WriterId,
        key: &str,
        assert_on_foreign_writer: bool,
    ) -> WriteRoute {
        let runtime_key = match var_prefix {
            Some(p) => format!("{p}.{key}"),
            None => key.to_owned(),
        };
        // The legacy loops only emitted canonical aliases for prefixed
        // subsystems whose canonical prefix differs from the var prefix.
        let canonical_key = match (var_prefix, canonical_prefix) {
            (Some(p), Some(c)) if c != p => Some(format!("{c}.{key}")),
            _ => None,
        };
        let slot = store.slot_by_name(&runtime_key).filter(|id| {
            let Some(meta) = store.meta(*id) else {
                return false;
            };
            let names_match = meta.runtime_name.as_ref() == runtime_key
                && match &canonical_key {
                    Some(c) => meta.canonical_name.as_ref() == c.as_str(),
                    None => meta.canonical_name == meta.runtime_name,
                };
            let writer_ok = meta.writer == expected_writer;
            if names_match && !writer_ok && assert_on_foreign_writer {
                // Single-writer promise (RSC-2.4a): a writeback target owned
                // by another EXECUTOR is a live variability-guard violation.
                // Mint-time placeholder writers (Orchestrator / External
                // from a subsystem-name lookup miss) are not — they just
                // keep the legacy name-keyed path.
                debug_assert!(
                    !matches!(meta.writer, WriterId::Executor(_)),
                    "write-set target '{}' resolves to slot '{}' owned by \
                     {:?}, expected {:?} — two executors writing one slot",
                    runtime_key,
                    meta.canonical_name,
                    meta.writer,
                    expected_writer,
                );
            }
            names_match && writer_ok
        });
        WriteRoute {
            slot,
            runtime_key,
            canonical_key,
        }
    }

    /// Write `value` into the shared context **by `SlotId`** (the set_slot
    /// dual-spelling mirror keeps the legacy map coherent). Every write-set
    /// this method serves are the **claimed** writes — ODE / ODE45 / BDF state
    /// vectors, ODE signal outputs (RSC-3.0 category 2), state-machine
    /// assignment targets, hybrid continuous state.
    ///
    /// **The unrouted branch is a static-invariant backstop, not the
    /// enforcement mechanism.** Whether every such write mints a claimed slot is
    /// decided entirely at compile time: the compiler's **RS005** gate
    /// ([`ModelCompiler::rs005_diagnostics`](crate::compiler::ModelCompiler),
    /// fed by [`Orchestrator::unrouted_slot_writes`]
    /// (crate::orchestrator::Orchestrator::unrouted_slot_writes)) hard-fails the
    /// build if any strict-`apply` write-set entry lacks a slot. So by the time
    /// a tick calls `apply`, an unrouted route is *statically impossible* — the
    /// `debug_assert` below is an `unreachable!`-shaped backstop (defence in
    /// depth), exactly like the tail of an exhaustive match. **It does NOT make
    /// a release-mode miss observable**: the `tracing::warn!` is feature-gated
    /// and services build without it, so a hypothetical unrouted write in
    /// release is a silent skip — which is *why* enforcement lives at compile,
    /// where the gap is caught once rather than dropped every tick. (Before A2,
    /// this branch was miscast as the hard-error; RS005 is now the hard error.)
    ///
    /// The former general name-keyed fallback (`set(runtime_key)` /
    /// `set(canonical_key)` tail) was deleted with the string-identity cull.
    /// Two write categories legitimately still name-key and use the explicit
    /// [`apply_name_keyed`](Self::apply_name_keyed) path instead of this one:
    /// physics port/flow writes and state-machine port payloads (both blocked
    /// on the L34 port-flow-activation wave — ledger L31); RS005 does not flag
    /// those.
    pub(crate) fn apply(&self, shared: &mut crate::expressions::EvalContext, value: Value) {
        let Some(id) = self.slot else {
            // Statically impossible after the RS005 compile gate — a backstop,
            // not the enforcement path. If this ever fires, a strict-`apply`
            // write escaped RS005 (a mint gap the gate failed to catch), not a
            // recoverable case. Release has no tracing feature, so the skip is
            // silent by default — see the doc above for why that is acceptable
            // given compile-time enforcement.
            debug_assert!(
                false,
                "WriteRoute::apply on an UNROUTED route for '{}' — this is \
                 statically impossible after the RS005 gate (every strict-apply \
                 write mints a claimed slot). If it fires, RS005 missed a mint \
                 gap; it must not be papered over with a name-keyed write. \
                 Physics + SM-payload name-keying goes through apply_name_keyed.",
                self.runtime_key,
            );
            #[cfg(feature = "tracing")]
            tracing::warn!(
                key = %self.runtime_key,
                "WriteRoute::apply reached an unrouted route (RS005 backstop tripped)"
            );
            return;
        };
        if !shared.set_slot(id, value) {
            // The slot exists but the store refused the write (store detached
            // mid-run). Surface it — a dropped state write is a correctness
            // bug, not a recoverable soft path.
            debug_assert!(
                false,
                "WriteRoute::apply: set_slot refused for routed key '{}' \
                 (slot store detached mid-run?)",
                self.runtime_key,
            );
            #[cfg(feature = "tracing")]
            tracing::warn!(
                key = %self.runtime_key,
                "WriteRoute::apply: routed set_slot refused (store detached?)"
            );
        }
    }

    /// Explicit name-keyed writeback: write `value` by the precomputed legacy
    /// keys — `runtime_key` plus the optional `canonical_key` alias — using the
    /// slot route when one happens to exist.
    ///
    /// This is the surviving name-keyed write path, deliberately restricted to
    /// the write categories the slot table does not claim by construction:
    ///
    /// - **Physics port/flow writes** (ledger L31): the slot table mints no
    ///   physics port/flow identity until the L34 port-flow-activation wave, so
    ///   every physics write target is name-keyed.
    /// - **State-machine port payloads**: `{port}.payload` values bound from a
    ///   runtime delivery in `tick`, not compile-claimed state. A pre-minted
    ///   payload slot routes by `SlotId`; an unminted one name-keys.
    ///
    /// (ODE **signal outputs** — `GetOutput` derived quantities like
    /// `kinetic_energy`/`i_drive` — used to name-key here too, but RSC-3.0
    /// category 2 now mints them claimed slots so they route by `SlotId`
    /// through [`apply`](Self::apply) like state vectors.)
    ///
    /// State vectors, ODE signal outputs and SM assignment targets do NOT use
    /// this path — they are claimed slots, route by `SlotId` through
    /// [`apply`](Self::apply), and any unrouted such write is a mint bug that
    /// hard-errors there. Keeping this
    /// name-keying explicit (rather than a silent fallback inside `apply`) lets
    /// the routed machinery hard-error on every OTHER unrouted write.
    pub(crate) fn apply_name_keyed(
        &self,
        shared: &mut crate::expressions::EvalContext,
        value: Value,
    ) {
        if let Some(id) = self.slot {
            if shared.set_slot(id, value.clone()) {
                return;
            }
        }
        shared.set(self.runtime_key.clone(), value.clone());
        if let Some(canonical) = &self.canonical_key {
            shared.set(canonical.clone(), value);
        }
    }

    /// Whether this route writes by `SlotId` (vs the name-keyed fallback).
    pub(crate) fn is_routed(&self) -> bool {
        self.slot.is_some()
    }

    /// The legacy runtime key this route writes (fallback spelling).
    pub(crate) fn runtime_key(&self) -> &str {
        &self.runtime_key
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn rid(decl: &str, path: &[&str]) -> RuntimeId {
        RuntimeId::scoped(
            ElementId::from_string(format!("decl:{decl}")),
            path.iter()
                .map(|p| ElementId::from_string(format!("usage:{p}"))),
        )
    }

    fn meta(
        decl: &str,
        path: &[&str],
        variability: Variability,
        writer: WriterId,
        canonical: &str,
        runtime: &str,
    ) -> SlotMeta {
        SlotMeta::new(rid(decl, path), variability, writer, canonical, runtime)
    }

    #[test]
    fn aliases_of_carries_every_spelling_beyond_the_two_meta_names() {
        // Task #8: the reverse alias table must carry canonical + runtime PLUS
        // every `add_alias` extra (N > 2), so `set_slot`'s mirror reaches an
        // alias-only spelling like an ODE's qualified `{ode}.duty` observable.
        let mut store = SlotStore::new();
        let id = store.intern(
            meta(
                "duty",
                &[],
                Variability::Continuous,
                WriterId::Orchestrator,
                "duty",
                "duty",
            ),
            Value::Float(0.0),
        );
        // Two extra alias-only spellings (the qualified observable + another).
        store.add_alias("OscillatorModel.duty", id);
        store.add_alias("physics.duty", id);
        let names: Vec<String> = store.aliases_of(id).iter().map(|n| n.to_string()).collect();
        // canonical==runtime=="duty" dedups to one; +2 add_alias = 3 spellings.
        assert!(names.contains(&"duty".to_owned()), "bare spelling: {names:?}");
        assert!(
            names.contains(&"OscillatorModel.duty".to_owned()),
            "qualified observable spelling must be recorded: {names:?}"
        );
        assert!(names.contains(&"physics.duty".to_owned()), "second alias: {names:?}");
        assert!(names.len() >= 3, "N>2 spellings expected, got {names:?}");
        // A colliding add_alias onto a DIFFERENT slot must NOT be recorded here
        // (first-binding-wins; the reverse table stays coherent with by_name).
        let other = store.intern(
            meta(
                "other",
                &[],
                Variability::Continuous,
                WriterId::Orchestrator,
                "other",
                "other",
            ),
            Value::Float(0.0),
        );
        store.add_alias("duty", other); // collides with `id`'s bare spelling
        assert!(
            !store.aliases_of(other).iter().any(|n| n.as_ref() == "duty"),
            "colliding spelling must not be mis-recorded on the losing slot"
        );
    }

    #[test]
    fn is_meta_spelling_admits_only_the_two_meta_names_not_add_alias_extras() {
        // Task #8 (steward ruling 2026-07-07): the observable-projection
        // predicate admits ONLY canonical/runtime; add_alias extras are
        // resolution-only. Two cases the filter must get right.
        let mut store = SlotStore::new();

        // (i) A bare slot (canonical == runtime == "duty") with the qualified
        // `{ode}.duty` observable added via add_alias, like the legacy oscillator fixture's path.
        let duty = store.intern(
            meta(
                "duty",
                &[],
                Variability::Continuous,
                WriterId::Orchestrator,
                "duty",
                "duty",
            ),
            Value::Float(0.0),
        );
        store.add_alias("OscillatorModel.duty", duty);
        assert!(
            store.is_meta_spelling(duty, "duty"),
            "the collapsed canonical==runtime meta spelling is an observable"
        );
        assert!(
            !store.is_meta_spelling(duty, "OscillatorModel.duty"),
            "the add_alias qualified spelling is resolution-only, NOT an observable"
        );
        // It stays resolvable, though — resolution-only, not gone.
        assert_eq!(store.slot_by_name("OscillatorModel.duty"), Some(duty));

        // (ii) A PREFIXED multi-instance slot with canonical != runtime (the
        // multi-circuit fixture's `circuit1.faultIntegral` shape). BOTH meta
        // spellings are observables; a spurious add_alias is not.
        let fi = store.intern(
            meta(
                "faultIntegral",
                &[],
                Variability::Continuous,
                WriterId::Orchestrator,
                "Panel.circuit1.faultIntegral", // canonical (tree path)
                "circuit1.faultIntegral",             // runtime (var_prefix)
            ),
            Value::Float(0.0),
        );
        store.add_alias("some.other.alias.faultIntegral", fi);
        assert!(
            store.is_meta_spelling(fi, "Panel.circuit1.faultIntegral"),
            "canonical of a prefixed slot is an observable"
        );
        assert!(
            store.is_meta_spelling(fi, "circuit1.faultIntegral"),
            "runtime of a prefixed slot is an observable (canonical != runtime)"
        );
        assert!(
            !store.is_meta_spelling(fi, "some.other.alias.faultIntegral"),
            "an add_alias extra on a prefixed slot is NOT an observable"
        );

        // Unknown id is never a meta spelling.
        assert!(!store.is_meta_spelling(SlotId(9999), "duty"));
    }

    #[test]
    fn interning_is_idempotent_on_runtime_id() {
        let mut store = SlotStore::new();
        let a = store.intern(
            meta(
                "bimetalTemp",
                &["circuit1"],
                Variability::Continuous,
                WriterId::Orchestrator,
                "Panel.circuit1.thermalModel.bimetalTemp",
                "circuit1.bimetalTemp",
            ),
            Value::Float(298.15),
        );
        let b = store.intern(
            meta(
                "bimetalTemp",
                &["circuit1"],
                Variability::Continuous,
                WriterId::Orchestrator,
                "Panel.circuit1.thermalModel.bimetalTemp",
                "circuit1.bimetalTemp",
            ),
            Value::Float(999.0),
        );
        assert_eq!(a, b, "same RuntimeId must intern to the same slot");
        assert_eq!(store.len(), 1);
        // Original value is kept on re-intern.
        assert_eq!(store.get(a), Some(&Value::Float(298.15)));
    }

    #[test]
    fn distinct_instance_paths_are_distinct_slots() {
        let mut store = SlotStore::new();
        let a = store.intern(
            meta(
                "bimetalTemp",
                &["circuit1"],
                Variability::Continuous,
                WriterId::Orchestrator,
                "Panel.circuit1.thermalModel.bimetalTemp",
                "circuit1.bimetalTemp",
            ),
            Value::Float(298.15),
        );
        let b = store.intern(
            meta(
                "bimetalTemp",
                &["circuit2"],
                Variability::Continuous,
                WriterId::Orchestrator,
                "Panel.circuit2.thermalModel.bimetalTemp",
                "circuit2.bimetalTemp",
            ),
            Value::Float(298.15),
        );
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn dual_name_lookup_hits_the_same_slot() {
        let mut store = SlotStore::new();
        let id = store.intern(
            meta(
                "bimetalTemp",
                &["circuit1"],
                Variability::Continuous,
                WriterId::Orchestrator,
                "Panel.circuit1.thermalModel.bimetalTemp",
                "circuit1.bimetalTemp",
            ),
            Value::Float(298.15),
        );
        assert_eq!(
            store.slot_by_name("Panel.circuit1.thermalModel.bimetalTemp"),
            Some(id)
        );
        assert_eq!(store.slot_by_name("circuit1.bimetalTemp"), Some(id));
        assert_eq!(store.slot_by_name("circuit2.bimetalTemp"), None);
        // RuntimeId lookup agrees with both name forms.
        assert_eq!(
            store.slot_by_runtime_id(&rid("bimetalTemp", &["circuit1"])),
            Some(id)
        );
    }

    #[test]
    fn set_marks_dirty_once_and_clear_resets() {
        let mut store = SlotStore::new();
        let id = store.intern(
            meta(
                "x",
                &[],
                Variability::Discrete,
                WriterId::Orchestrator,
                "x",
                "x",
            ),
            Value::Int(0),
        );
        assert!(store.dirty().is_empty());
        store.set(id, Value::Int(1));
        store.set(id, Value::Int(2));
        assert_eq!(store.dirty(), &[id], "dirty list dedupes repeat writes");
        assert_eq!(store.get(id), Some(&Value::Int(2)));
        store.clear_dirty();
        assert!(store.dirty().is_empty());
        store.set(id, Value::Int(3));
        assert_eq!(store.dirty(), &[id], "dirty tracking re-arms after clear");
    }

    #[test]
    fn constant_write_is_rejected_by_try_set() {
        let mut store = SlotStore::new();
        let id = store.intern(
            meta(
                "pi",
                &[],
                Variability::Constant,
                WriterId::External,
                "pi",
                "pi",
            ),
            Value::Float(3.14159),
        );
        let err = store.try_set(id, Value::Float(3.0)).unwrap_err();
        assert!(matches!(err, SlotWriteError::ConstantSlot { slot, .. } if slot == id));
        // Value unchanged, nothing marked dirty.
        assert_eq!(store.get(id), Some(&Value::Float(3.14159)));
        assert!(store.dirty().is_empty());
        // Non-constant slots accept checked writes.
        let p = store.intern(
            meta(
                "ratedCurrent",
                &[],
                Variability::Parameter,
                WriterId::External,
                "ratedCurrent",
                "ratedCurrent",
            ),
            Value::Float(16.0),
        );
        assert!(store.try_set(p, Value::Float(20.0)).is_ok());
        assert_eq!(store.get(p), Some(&Value::Float(20.0)));
        assert_eq!(store.dirty(), &[p]);
    }

    #[test]
    fn try_set_rejects_unknown_slot() {
        let mut store = SlotStore::new();
        let bogus = SlotId(42);
        assert_eq!(
            store.try_set(bogus, Value::Int(1)),
            Err(SlotWriteError::InvalidSlot(bogus))
        );
    }

    #[test]
    fn multi_writer_conflicts_lists_distinct_tick_time_writers() {
        let mut store = SlotStore::new();
        // Two executors claim the same slot — conflict.
        let id = store.intern(
            meta(
                "tripped",
                &["circuit1"],
                Variability::Discrete,
                WriterId::Executor(0),
                "circuit1.tripped",
                "circuit1.tripped",
            ),
            Value::Bool(false),
        );
        store.intern(
            meta(
                "tripped",
                &["circuit1"],
                Variability::Continuous,
                WriterId::Executor(3),
                "circuit1.tripped",
                "circuit1.tripped",
            ),
            Value::Bool(false),
        );
        // External re-claim never conflicts.
        let p = store.intern(
            meta(
                "ratedCurrent",
                &[],
                Variability::Parameter,
                WriterId::External,
                "ratedCurrent",
                "ratedCurrent",
            ),
            Value::Float(16.0),
        );
        store.intern(
            meta(
                "ratedCurrent",
                &[],
                Variability::Parameter,
                WriterId::External,
                "ratedCurrent",
                "ratedCurrent",
            ),
            Value::Float(16.0),
        );
        let conflicts = store.multi_writer_conflicts();
        assert_eq!(conflicts.len(), 1);
        let (cid, name, writers) = &conflicts[0];
        assert_eq!(*cid, id);
        assert_eq!(name.as_ref(), "circuit1.tripped");
        assert_eq!(
            writers.as_slice(),
            &[WriterId::Executor(0), WriterId::Executor(3)]
        );
        let _ = p;
    }

    #[test]
    fn set_by_name_routes_to_both_spellings() {
        let mut store = SlotStore::new();
        let id = store.intern(
            meta(
                "x",
                &[],
                Variability::Discrete,
                WriterId::Orchestrator,
                "C.x",
                "x",
            ),
            Value::Int(0),
        );
        assert!(store.set_by_name("x", &Value::Int(5)));
        assert_eq!(store.get(id), Some(&Value::Int(5)));
        assert_eq!(store.dirty(), &[id]);
        assert!(!store.set_by_name("unbound", &Value::Int(9)));
        // Both name forms route to the same slot.
        assert!(store.set_by_name("C.x", &Value::Int(8)));
        assert_eq!(store.get(id), Some(&Value::Int(8)));
        // Constant slots refuse by-name routing.
        let pi = store.intern(
            meta(
                "pi",
                &[],
                Variability::Constant,
                WriterId::External,
                "pi",
                "pi",
            ),
            Value::Float(3.14),
        );
        assert!(!store.set_by_name("pi", &Value::Float(3.0)));
        assert_eq!(store.get(pi), Some(&Value::Float(3.14)));
    }

    /// Cull-arc W2: a `tracing` subscriber can reconstruct the slot-write
    /// journal — target, slot identity, sanctioned writer, and the value
    /// transition — from a `try_set`. Only compiled/run under the `tracing`
    /// feature (the journal is a no-op otherwise). Uses a minimal in-crate
    /// `Subscriber` so no `tracing-subscriber` dependency is pulled in.
    #[cfg(feature = "tracing")]
    #[test]
    fn slot_write_journal_event_is_capturable() {
        use std::collections::HashMap;
        use std::sync::{Arc as StdArc, Mutex};
        use tracing::field::{Field, Visit};

        #[derive(Default)]
        struct Captured {
            target: String,
            fields: HashMap<String, String>,
        }
        struct Grab<'a>(&'a mut Captured);
        impl Visit for Grab<'_> {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0
                    .fields
                    .insert(field.name().to_owned(), format!("{value:?}"));
            }
            fn record_u64(&mut self, field: &Field, value: u64) {
                self.0
                    .fields
                    .insert(field.name().to_owned(), value.to_string());
            }
            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.fields.insert(field.name().to_owned(), value.to_owned());
            }
        }
        struct Capturing(StdArc<Mutex<Vec<Captured>>>);
        impl tracing::Subscriber for Capturing {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                let mut cap = Captured {
                    target: event.metadata().target().to_owned(),
                    ..Default::default()
                };
                event.record(&mut Grab(&mut cap));
                self.0.lock().unwrap().push(cap);
            }
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
        }

        let log = StdArc::new(Mutex::new(Vec::new()));
        let id = tracing::subscriber::with_default(Capturing(log.clone()), || {
            let mut store = SlotStore::new();
            let id = store.intern(
                meta(
                    "tripped",
                    &["circuit1"],
                    Variability::Discrete,
                    WriterId::Executor(2),
                    "Panel.circuit1.tripped",
                    "circuit1.tripped",
                ),
                Value::Bool(false),
            );
            store.try_set(id, Value::Bool(true)).unwrap();
            id
        });

        let events = log.lock().unwrap();
        let ev = events
            .iter()
            .find(|c| c.target == "sysml_runtime::slot_write")
            .expect("try_set emitted a slot_write journal event");
        assert_eq!(
            ev.fields.get("slot").map(String::as_str),
            Some(id.index().to_string().as_str())
        );
        assert_eq!(ev.fields.get("writer").map(String::as_str), Some("Executor(2)"));
        assert_eq!(
            ev.fields.get("canonical").map(String::as_str),
            Some("Panel.circuit1.tripped")
        );
        // Value transition — assert on substring so a custom `Value` Debug
        // formatting (e.g. bare `false`/`true`) still satisfies the journal.
        assert!(ev.fields.get("old").is_some_and(|s| s.contains("false")));
        assert!(ev.fields.get("new").is_some_and(|s| s.contains("true")));
    }

    #[test]
    fn seed_from_map_prefers_runtime_name_and_stays_clean() {
        let mut store = SlotStore::new();
        let id = store.intern(
            meta(
                "x",
                &["i1"],
                Variability::Parameter,
                WriterId::External,
                "C.i1.x",
                "i1.x",
            ),
            Value::Int(0),
        );
        let unseeded = store.intern(
            meta(
                "y",
                &["i1"],
                Variability::Parameter,
                WriterId::External,
                "C.i1.y",
                "i1.y",
            ),
            Value::Int(42),
        );
        let mut map = HashMap::new();
        map.insert("C.i1.x".to_owned(), Value::Int(1));
        map.insert("i1.x".to_owned(), Value::Int(2));
        store.seed_from_map(&map);
        assert_eq!(
            store.get(id),
            Some(&Value::Int(2)),
            "runtime-name binding wins over canonical"
        );
        assert_eq!(
            store.get(unseeded),
            Some(&Value::Int(42)),
            "absent names keep the minted value"
        );
        assert!(store.dirty().is_empty(), "seeding is not a tick-time write");
    }

    #[test]
    fn alias_registration_is_first_wins() {
        let mut store = SlotStore::new();
        let a = store.intern(
            meta(
                "x",
                &["i1"],
                Variability::Parameter,
                WriterId::External,
                "C.i1.x",
                "i1.x",
            ),
            Value::Int(1),
        );
        let b = store.intern(
            meta(
                "y",
                &["i1"],
                Variability::Parameter,
                WriterId::External,
                "C.i1.x",
                "i1.y",
            ),
            Value::Int(2),
        );
        // "C.i1.x" was claimed by `a` first; `b` keeps its other names.
        assert_eq!(store.slot_by_name("C.i1.x"), Some(a));
        assert_eq!(store.slot_by_name("i1.y"), Some(b));
        store.add_alias("x", a);
        store.add_alias("x", b); // first wins
        assert_eq!(store.slot_by_name("x"), Some(a));
    }
}
