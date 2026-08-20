//! # sysml-run-expressions
//!
//! Expression evaluation engine for SysML v2 runtime.
//!
//! This crate provides a typed expression AST ([`ExprIR`]) and a recursive
//! evaluator ([`ExpressionEvaluator`]) capable of evaluating KerML expressions
//! at runtime. It is the **keystone** of the execution runtime — guards,
//! constraints, action conditions, and calculations all depend on it.
//!
//! ## Architecture
//!
//! ```text
//! .sysml source -> parser -> ExprIR (typed AST)
//!                                |
//!                    ExpressionEvaluator::eval(expr, context)
//!                                |
//!                            Value result
//! ```
//!
//! ## Expression Types (from KerML spec)
//!
//! The expression hierarchy follows KerMLExpressions.xtext precedence:
//!
//! | Precedence | Operators | Category |
//! |------------|-----------|----------|
//! | 1 (lowest) | `if ? else` | Conditional |
//! | 2 | `??` | Null coalescing |
//! | 3 | `implies` | Implication |
//! | 4 | `\|`, `or` | Disjunction |
//! | 5 | `xor` | Exclusive or |
//! | 6 | `&`, `and` | Conjunction |
//! | 7 | `==`, `!=`, `===`, `!==` | Equality |
//! | 8 | `<`, `>`, `<=`, `>=` | Relational |
//! | 9-10 | `as`, `meta`, `@`, `@@` | Classification |
//! | 11 | `..` | Range |
//! | 12 | `+`, `-` | Additive |
//! | 13 | `*`, `/`, `%` | Multiplicative |
//! | 14 | `**`, `^` | Exponentiation |
//! | 15 | `-` (unary), `+`, `~`, `not` | Unary |
//! | 16 (highest) | `#` (index), `->`, `.` | Postfix |
//!
//! ## Standard Library Functions
//!
//! From KerML standard library packages:
//! - `BaseFunctions`: `==`, `!=`, `===`, `!==`, `ToString`, `size`
//! - `ScalarFunctions`: `+`, `-`, `*`, `/`, `%`, `**`, `<`, `>`, `<=`, `>=`, `max`, `min`
//! - `BooleanFunctions`: `not`, `xor`, `|`, `&`, `implies`
//! - `SequenceFunctions`: `size`, `isEmpty`, `notEmpty`, `includes`, `excludes`,
//!   `head`, `tail`, `last`, `union`, `intersection`
//! - `CollectionFunctions`: `size`, `isEmpty`, `includes`, `excludes`
//! - `ControlFunctions`: `if`, `??`, `select`, `collect`, `reject`, `forAll`, `exists`

use std::collections::HashMap;
use std::sync::Arc;
use sysml_core::{ModelGraph, Value};

/// Minimal subsystem state shape used by expression temporal helpers.
///
/// The execution runtime has a richer, serde-enabled equivalent at the crate
/// root ([`crate::SubsystemState`]); this lighter copy lives with the
/// expression evaluator so temporal stdlib functions can query historical
/// snapshots without depending on execution-orchestration state. Runtime
/// converts between the two at the orchestration boundary.
#[derive(Debug, Clone, Default)]
pub struct SubsystemState {
    pub name: String,
    pub kind: &'static str,
    pub current_state: String,
    pub completed: bool,
    pub available_transitions: Vec<(String, String)>,
    pub outputs: Vec<String>,
    pub sends: Vec<String>,
    pub active_modes: Vec<String>,
    pub variables: HashMap<String, Value>,
    pub deferred_event_count: usize,
    pub source_element_id: Option<sysml_core::ElementId>,
}

/// Captured variable state at one orchestrator tick for temporal expression
/// helpers (the light counterpart of [`crate::TickSnapshot`]).
#[derive(Debug, Clone, Default)]
pub struct TickSnapshot {
    pub tick: u64,
    pub time_ms: f64,
    pub variables: HashMap<String, Value>,
    pub subsystem_states: HashMap<String, SubsystemState>,
}

// Module declarations
mod binder;
mod compiler;
mod evaluator;
mod ir;
mod stdlib;
pub mod units;

// Re-export public API
pub use binder::{bind_slots, BindReport, SlotBinder};
pub use compiler::{compile_expression, compile_expression_ast, compile_simple_expression};
pub use evaluator::{
    resolve_ref_value, resolve_ref_value_cached, ExpressionEvaluator, RefResolveCache,
};
pub use ir::{BinOp, ExprIR, UnaryOp};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during expression evaluation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EvaluationError {
    /// A referenced variable was not found in the evaluation context.
    #[error("undefined variable: `{0}`")]
    UndefinedVariable(String),

    /// A type mismatch occurred during evaluation.
    #[error("type error: {0}")]
    TypeError(String),

    /// Division by zero.
    #[error("division by zero")]
    DivisionByZero,

    /// Index out of bounds for a collection operation.
    #[error("index out of bounds: {index} (size: {size})")]
    IndexOutOfBounds { index: usize, size: usize },

    /// An unsupported operator was encountered.
    #[error("unsupported operator: `{0}`")]
    UnsupportedOperator(String),

    /// An unsupported function was called.
    #[error("unknown function: `{0}`")]
    UnknownFunction(String),

    /// Arity mismatch on a function call.
    #[error("function `{name}` expects {expected} arguments, got {got}")]
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },

    /// A general runtime error.
    #[error("{0}")]
    Runtime(String),

    /// Integer arithmetic overflow.
    #[error("integer overflow")]
    Overflow,

    /// Recursion depth limit exceeded.
    #[error("recursion limit exceeded (max depth: {max_depth})")]
    RecursionLimit { max_depth: usize },

    /// A function is recognized but requires infrastructure not yet available.
    #[error("function `{name}` is not yet implemented: {reason}")]
    NotYetImplemented { name: String, reason: String },
}

/// Convenience alias for evaluation results.
pub type EvalResult = Result<Value, EvaluationError>;

/// Is `name` an internal runtime-bookkeeping variable rather than a
/// user-facing model variable?
///
/// Bookkeeping names are the orchestrator's own scratch keys — the clock
/// pair (`t_ms`, `tick`) and every `__`-prefixed internal token
/// (`__clock_time`, `__active_substates`, `__recent_transitions`,
/// `{sm}.__flow_gate`, …). They drift by construction and must be filtered
/// out of snapshot views, diff reports, time-series collection and
/// occurrence feature capture.
///
/// This is the single home for the predicate (CLAUDE.md #4): the eval
/// context, snapshot view, orchestrator and service layer all route their
/// `starts_with("__")` checks here. When bookkeeping slots become typed
/// (`SlotMeta::bookkeeping`, RSC-2.0 §D-2.0.2) this string predicate is the
/// designated replacement seam.
#[inline]
pub fn is_internal_var(name: &str) -> bool {
    name.starts_with("__") || name == "t_ms" || name == "tick"
}

// ---------------------------------------------------------------------------
// Evaluation context
// ---------------------------------------------------------------------------

/// Runtime context for expression evaluation.
///
/// Holds variable bindings that expressions can reference. This is shared
/// across runners (state machines, actions, constraints).
///
/// `variables` is `Arc<HashMap>` so that `EvalContext::clone()` — used by
/// fork, child-scope creation, Monte Carlo batch start, and the scoped
/// context builder in `Orchestrator::step` — is a pointer-bump copy-on-
/// write. Mutations route through `Arc::make_mut`, which clones the inner
/// `HashMap` exactly once when a second reader exists and is free
/// otherwise.
// Cull-arc W3: `Clone` is deliberately NOT derived. A bare `.clone()` hid the
// choice between a LIVE alias (shares the slot write handle → writes reach
// production) and a speculative SNAPSHOT (writes must stay local). Copy sites
// must now name that choice via [`alias_live`](EvalContext::alias_live) or
// [`scratch_snapshot`](EvalContext::scratch_snapshot).
#[derive(Debug, Default)]
pub struct EvalContext {
    /// Variable bindings: name -> value. See the struct docstring for the
    /// Arc-CoW invariants; use the accessors (`set`, `get`) or route
    /// direct mutations through `Arc::make_mut`.
    pub variables: Arc<HashMap<String, Value>>,
    /// Optional execution trace for temporal query functions.
    /// When set, stdlib temporal functions (was_in_state, held_for, etc.)
    /// can query historical snapshots.
    pub trace: Option<Arc<Vec<TickSnapshot>>>,
    /// Optional model graph for lazy feature chain resolution.
    /// When set, `Value::Ref(ElementId)` values in feature chains are resolved
    /// by looking up named features on the referenced element's type.
    pub graph: Option<Arc<ModelGraph>>,
    /// Optional occurrence registry for spec occurrence lifecycle functions
    /// (`create`, `destroy`, `isDuring`, `addNew`, `addNewAt`).
    pub occurrence_registry:
        Option<Arc<std::sync::Mutex<sysml_core::occurrence::OccurrenceRegistry>>>,
    /// Optional spatial frame registry for coordinate frame functions
    /// (`PositionOf`, `transform`, `CoordinateFrame*`, etc.).
    pub frame_registry: Option<Arc<sysml_core::spatial::FrameRegistry>>,
    /// Optional calculation registry for user-defined `calc def` evaluation.
    /// When set, `FunctionCall` expressions that aren't in stdlib will fall
    /// back to looking up the name in this registry.
    pub calculations: Option<Arc<crate::calculations::CalculationRegistry>>,
    /// Optional live slot-store handle (RSC-2.2, design doc D-2.0.6).
    ///
    /// Attached by the orchestrator at build time
    /// (`Orchestrator::set_slot_store`) to its master context only. When
    /// present and the store is enabled, [`set`](Self::set) write-throughs
    /// to the slot bound for the name (the legacy `variables` map stays
    /// coherent — it is written on every path that writes a slot, so
    /// [`get`](Self::get) reads it directly without taking the lock).
    ///
    /// Deliberately NOT propagated by [`merge_from`](Self::merge_from):
    /// executor-internal contexts merge the shared context in before each
    /// tick, and carrying the live handle there would route executor-local
    /// bare-name writes (e.g. `temperature` inside `circuit5`'s scoped
    /// view) into globally-named slots — cross-instance pollution. Scoped
    /// contexts built by the orchestrator are constructed fresh
    /// (`EvalContext::new`) and never carry the handle either. `clone()`
    /// shares the handle (Arc bump), which is what snapshot/scoped-view
    /// clones of the master context want.
    pub slots: Option<crate::slots::SharedSlotStore>,
    /// RSC-2.3: **read-only** slot-store handle, propagated where the live
    /// write handle ([`slots`](Self::slots)) deliberately is not — the thin
    /// per-instance slot-read view (`build_slot_read_context`) and
    /// executor-internal contexts assembled via [`merge_from`](Self::merge_from)
    /// (ODE RHS template contexts included).
    ///
    /// The RSC-2.2 pollution concern was *bare-NAME writes* routing to
    /// globally-named slots from instance-scoped views; by-`SlotId` READS
    /// are globally unique and safe everywhere. [`set`](Self::set) never
    /// routes through this handle (name-write routing stays master-only);
    /// only [`get_slot`](Self::get_slot) / [`slot_id`](Self::slot_id)
    /// consult it, as the fallback for [`ExprIR::SlotRef`] evaluation in
    /// contexts that lack the master handle.
    pub slot_reader: Option<crate::slots::SharedSlotStore>,
    /// OPT #1 (runtime-hotpath-perf-plan): slot-indexed fast lane for
    /// float-valued slots, written by the ODE RHS scratch context and read
    /// **first** by [`ExprIR::SlotRef`] evaluation (before the `variables`
    /// map and the slot store).
    ///
    /// Indexed by [`SlotId::index`](crate::slots::SlotId::index); `None` means
    /// "no fast-lane value for this slot — fall through to the normal path".
    ///
    /// Populated ONLY in the per-orchestrator-step RHS scratch context built
    /// inside `ode_builder::OdeSpec::build_rhs`, and only for state-var slots
    /// the binder proved (it rewrote every reference to a `SlotRef` for the
    /// same slot, so no surviving `FeatureRef` reads the name map for them).
    /// Every other context leaves this empty, so `SlotRef` evaluation there is
    /// byte-identical to the pre-OPT-#1 name-first path. Deliberately NOT
    /// propagated by [`merge_from`](Self::merge_from) — the fast lane is
    /// scratch-local stage state, exactly like the live [`slots`](Self::slots)
    /// write handle, and leaking it into other contexts would surface a
    /// half-finished RK stage's state.
    pub fast_slots: Vec<Option<f64>>,
}

impl EvalContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clone this context as a LIVE alias (cull-arc W3). Every field is copied,
    /// including the live [`slots`](Self::slots) write handle — a
    /// [`SharedSlotStore`](crate::slots::SharedSlotStore) Arc refcount bump, so
    /// the copy shares the SAME [`SlotStore`](crate::slots::SlotStore) and its
    /// `set`/[`set_slot`](Self::set_slot) still write through to production
    /// state. This is exactly the semantics the derived `Clone` used to
    /// provide; it replaced the derive so every copy site must now say, by name,
    /// whether it wants a live alias or a [`scratch_snapshot`](Self::scratch_snapshot).
    ///
    /// Use for genuine live copies that MUST keep writing through: executor
    /// forks (`Executor::clone_boxed`), [`Orchestrator`](crate::orchestrator::Orchestrator)
    /// clone/fork, and composite state-machine sub-runners that execute and
    /// whose assignment effects reach the shared store.
    pub fn alias_live(&self) -> Self {
        Self {
            variables: self.variables.clone(),
            trace: self.trace.clone(),
            graph: self.graph.clone(),
            occurrence_registry: self.occurrence_registry.clone(),
            frame_registry: self.frame_registry.clone(),
            calculations: self.calculations.clone(),
            slots: self.slots.clone(),
            slot_reader: self.slot_reader.clone(),
            fast_slots: self.fast_slots.clone(),
        }
    }

    /// Clone this context for SPECULATIVE / throwaway evaluation whose writes
    /// must NEVER reach production state (cull-arc W3). Copies every field like
    /// [`alias_live`](Self::alias_live), then [`demote_to_read_only`](Self::demote_to_read_only):
    /// reads still see the shared slot store, but `set`/`set_slot` stay LOCAL to
    /// the snapshot. When the source carries no live write handle (the common
    /// case for compile-time, RHS-template, and `merge_from`-scoped contexts)
    /// the demotion is a no-op, so the snapshot is byte-identical to the old
    /// derived clone.
    ///
    /// Use for numeric solver residual/root-finding probes, Monte Carlo samples,
    /// trade-study alternatives, calc-def argument binding, and compile-time
    /// evaluation.
    pub fn scratch_snapshot(&self) -> Self {
        let mut snap = self.alias_live();
        snap.demote_to_read_only();
        snap
    }

    /// Bind a variable.
    ///
    /// RSC-2.2 (design doc D-2.0.6): when a live slot-store handle is
    /// attached and the store is enabled, the write also routes through the
    /// slot bound for `name` (write-through). The legacy map is written
    /// unconditionally so both representations stay coherent — the map is
    /// still the wire/serialization surface during the dual-key window.
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        let name = name.into();
        // W1 (eval-identity cull, Defect-A root fix): when a MASTER write store
        // is attached and `name` is bound there, delegate to `set_slot` so the
        // write inherits the FULL alias mirror — every spelling bound to the
        // slot (canonical + runtime + `add_alias` extras), not just this one.
        // The prior path (`set_by_name` + single-key insert) mirrored only the
        // spelling passed here, leaving sibling alias spellings stale in the
        // legacy map and shadowing the live slot on a name-first read — the
        // write-leak family (L44 / task-8).
        //
        // The gate is the MASTER handle (`self.slots`), NOT `slot_id()`: a
        // read-only / demoted context (`slot_reader` only — zero-crossing
        // bisection clones + `demote_to_read_only` precisely so writes stay
        // LOCAL and never touch production) has no writable slot, so `set_slot`
        // would no-op and drop the value. Such contexts, and genuinely unbound
        // names (lambda params, RHS scratch locals), keep the bare map insert.
        let bound = self.slots.as_ref().and_then(|s| {
            s.read().unwrap_or_else(|p| p.into_inner()).slot_by_name(&name)
        });
        match bound {
            Some(id) => {
                // W4: the bool is intentionally dropped here (not asserted).
                // `bound` is Some only when the master handle is attached and the
                // name resolves to a slot, so `set_slot` returns false only when
                // that slot is `Constant` — a legitimate write-refusal (Constants
                // are never tick-writable), NOT a lost write. Every writable bound
                // slot succeeds. Asserting would panic on a valid Constant refusal.
                let _ = self.set_slot(id, value);
            }
            None => {
                Arc::make_mut(&mut self.variables).insert(name, value);
            }
        }
    }

    /// Look up a variable.
    ///
    /// Served from the legacy map. The map is coherent with the slot store
    /// by construction: every write path that touches a slot
    /// ([`set`](Self::set) write-through, [`set_slot`](Self::set_slot)
    /// mirroring) also writes the map, so no lock is taken on the hot read
    /// path. Use [`get_slot`](Self::get_slot) for by-`SlotId` reads.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }

    /// Resolve a name (canonical or runtime spelling) to its slot, when a
    /// slot store is attached (RSC-2.2). Consults the master write handle
    /// first, then the RSC-2.3 read-only handle.
    pub fn slot_id(&self, name: &str) -> Option<crate::slots::SlotId> {
        let slots = self.slots.as_ref().or(self.slot_reader.as_ref())?;
        let store = slots.read().unwrap_or_else(|p| p.into_inner());
        store.slot_by_name(name)
    }

    /// Read a slot's current value by `SlotId` (RSC-2.2). `None` when no
    /// store is attached or the id is unknown. Consults the master write
    /// handle first, then the RSC-2.3 read-only handle ([`slot_reader`]
    /// (Self::slot_reader)) so scoped/executor views can serve
    /// [`ExprIR::SlotRef`] reads.
    pub fn get_slot(&self, id: crate::slots::SlotId) -> Option<Value> {
        let slots = self.slots.as_ref().or(self.slot_reader.as_ref())?;
        let store = slots.read().unwrap_or_else(|p| p.into_inner());
        store.get(id).cloned()
    }

    /// Read a port-feature (or any named) slot's value by name, when a slot
    /// store (or read view) is attached and the name is bound. Returns `None`
    /// so callers fall back to the legacy string key when no slot backs the
    /// name. (Pre-RSC-3.5f.3 this also gated on the `enabled` rollback flag,
    /// now removed — slot routing is unconditional.)
    pub fn slot_value_if_enabled(&self, name: &str) -> Option<Value> {
        let slots = self.slots.as_ref().or(self.slot_reader.as_ref())?;
        let store = slots.read().unwrap_or_else(|p| p.into_inner());
        let id = store.slot_by_name(name)?;
        store.get(id).cloned()
    }

    /// Downgrade a live write handle to read-only (RSC-2.3's [`slot_reader`]
    /// (Self::slot_reader)), in place. A no-op when [`slots`](Self::slots) is
    /// already `None`; an existing `slot_reader` always wins (never lose read
    /// access by overwriting it with a demoted handle).
    ///
    /// `EvalContext::clone()` only bumps the `slots` Arc's refcount — a
    /// cloned context silently aliases the SAME live [`SlotStore`]
    /// (crate::slots::SlotStore) as its source, so `EvalContext::set`/
    /// [`set_slot`](Self::set_slot) on the clone still write through to
    /// production state. Callers that hand out a cloned context for
    /// speculative or repeated evaluation that must never mutate production
    /// state (e.g. zero-crossing bisection root-finding, which clones the
    /// live master context and calls `set()` repeatedly while probing
    /// interpolated states) must call this immediately after cloning, before
    /// any write.
    pub fn demote_to_read_only(&mut self) {
        if self.slot_reader.is_none() {
            self.slot_reader = self.slots.take();
        } else {
            self.slots = None;
        }
    }

    /// Write a slot by `SlotId` and mirror the value into the legacy map
    /// under both of the slot's name forms, keeping the two representations
    /// coherent (RSC-2.2, design doc D-2.0.6). Returns `false` when no
    /// store is attached, or the id is unknown / `Constant`.
    ///
    /// Cull-arc W4: `#[must_use]` — a dropped `false` is a silently-lost slot
    /// write, exactly the write-leak class this arc kills. Callers must handle
    /// it: assert success where the write must land (`let ok = …; debug_assert!(ok, …)`
    /// — never wrap the call in `debug_assert!`, whose condition is compiled out
    /// in release), or `let _ =` with a comment only where a dropped write is
    /// genuinely intended (e.g. a store-not-attached test path).
    #[must_use = "a dropped set_slot() result is a silently-lost slot write (cull-arc W4)"]
    pub fn set_slot(&mut self, id: crate::slots::SlotId, value: Value) -> bool {
        let Some(slots) = &self.slots else {
            return false;
        };
        // Task #8: mirror the value into the legacy map under EVERY spelling
        // bound to this slot (canonical + runtime + any `add_alias` extras,
        // e.g. an ODE's qualified `{ode}.duty` observable key), not just the
        // two meta-name fields — completes the store's "every legal spelling"
        // contract (`SlotStore::aliases_of`). The prior two-field mirror
        // silently dropped alias-only spellings, so a name-first `SlotRef`
        // read of such a spelling shadowed the live slot with a stale seed.
        let names: Vec<Arc<str>> = {
            let mut store = slots.write().unwrap_or_else(|p| p.into_inner());
            if store.try_set(id, value.clone()).is_err() {
                return false;
            }
            store.aliases_of(id).to_vec()
        };
        let vars = Arc::make_mut(&mut self.variables);
        for name in names {
            vars.insert(name.as_ref().to_owned(), value.clone());
        }
        true
    }

    /// OPT #1 (runtime-hotpath-perf-plan): write a float into the slot-fast
    /// lane (see [`fast_slots`](Self::fast_slots)). Does NOT touch the
    /// `variables` map or the slot store — no `String` allocation, no hash,
    /// no lock. The matching [`ExprIR::SlotRef`] read picks it up by index.
    /// Grows the lane on demand so callers need not pre-size it.
    #[inline]
    pub fn set_slot_fast(&mut self, id: crate::slots::SlotId, value: f64) {
        let idx = id.index();
        if idx >= self.fast_slots.len() {
            self.fast_slots.resize(idx + 1, None);
        }
        self.fast_slots[idx] = Some(value);
    }

    /// OPT #1: read a float from the slot-fast lane, if present. `None` when
    /// the lane is empty for this slot (every context except an ODE RHS
    /// scratch), so the [`ExprIR::SlotRef`] evaluator falls through to its
    /// normal name-first / slot-store path.
    #[inline]
    pub fn fast_slot(&self, id: crate::slots::SlotId) -> Option<f64> {
        self.fast_slots.get(id.index()).copied().flatten()
    }

    /// Create a child context (for lambda bindings in select/collect/etc.).
    pub fn child_with(&self, name: impl Into<String>, value: Value) -> Self {
        // Cull-arc W3: alias_live preserves the pre-cull `.clone()` semantics
        // exactly. The lambda variable is inserted straight into `variables`
        // (below), never through `set`, so no write reaches the slot plane
        // regardless; alias_live is the zero-risk, behaviour-identical choice.
        let mut child = self.alias_live();
        Arc::make_mut(&mut child.variables).insert(name.into(), value);
        child
    }

    /// Merge all bindings from `other` into this context.
    /// Existing bindings are overwritten on conflict.
    ///
    /// Also propagates the graph reference and spatial frame registry from
    /// `other` to `self` (only if `self`
    /// doesn't already have one). This is what lets a cached `EvalContext`
    /// (built once by `sysml_ide_db::eval_context_seed::context_from_graph`)
    /// carry all its derived registries through to a freshly-constructed
    /// `Orchestrator` context — the registries are graph-derived, so all
    /// contexts built from the same elaborated graph should share the same
    /// `Arc` allocations.
    pub fn merge_from(&mut self, other: &EvalContext) {
        let self_vars = Arc::make_mut(&mut self.variables);
        for (k, v) in other.variables.iter() {
            // Don't overwrite resolved values (Float, Int, Bool, Map) with Ref —
            // Refs are unresolved element placeholders from context_from_graph and
            // should never replace concrete numeric/config values.
            if matches!(v, Value::Ref(_)) {
                if let Some(existing) = self_vars.get(k) {
                    if matches!(
                        existing,
                        Value::Float(_) | Value::Int(_) | Value::Bool(_) | Value::Map(_)
                    ) {
                        continue;
                    }
                }
            }
            self_vars.insert(k.clone(), v.clone());
        }
        // Propagate graph reference if source has one and we don't
        if self.graph.is_none() {
            if let Some(ref g) = other.graph {
                self.graph = Some(Arc::clone(g));
            }
        }
        // Propagate spatial frame registry (cached upstream)
        if self.frame_registry.is_none() {
            if let Some(ref f) = other.frame_registry {
                self.frame_registry = Some(Arc::clone(f));
            }
        }
        // Propagate calculation registry (cached upstream)
        if self.calculations.is_none() {
            if let Some(ref c) = other.calculations {
                self.calculations = Some(Arc::clone(c));
            }
        }
        // RSC-2.3: propagate slot READ access (never the write handle —
        // name-write routing stays master-context-only, see the
        // `slot_reader` field docs). This is what carries by-`SlotId` read
        // capability into executor-internal contexts and the ODE RHS
        // template contexts built fresh inside `ode_builder` closures.
        if self.slots.is_none() && self.slot_reader.is_none() {
            self.slot_reader = other
                .slot_reader
                .as_ref()
                .or(other.slots.as_ref())
                .map(Arc::clone);
        }
    }

    /// Return bindings that differ between `self` and `other`.
    /// Includes keys present in `other` but not `self`, and keys with different values.
    pub fn diff(&self, other: &EvalContext) -> HashMap<String, Value> {
        let mut changed = HashMap::new();
        for (k, v) in other.variables.iter() {
            match self.variables.get(k) {
                Some(existing) if existing == v => {} // same value, skip
                _ => {
                    changed.insert(k.clone(), v.clone());
                }
            }
        }
        changed
    }
}

// ---------------------------------------------------------------------------
// Public convenience functions
// ---------------------------------------------------------------------------

/// Compile a guard expression string and return the set of variable names it references.
/// Returns an empty set if compilation fails.
pub fn analyze_guard_dependencies(guard: &str) -> std::collections::HashSet<String> {
    match compile_simple_expression(guard) {
        Ok(expr) => expr.free_variables(),
        Err(_) => std::collections::HashSet::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Result<Value, Vec<sysml_span::Diagnostic>> {
        let ir = compile_simple_expression(expr)?;
        let ctx = EvalContext::new();
        let evaluator = ExpressionEvaluator::new();
        evaluator
            .eval(&ir, &ctx)
            .map_err(|e| vec![sysml_span::Diagnostic::error(e.to_string())])
    }

    fn eval_with_ctx(expr: &str, ctx: &EvalContext) -> Result<Value, Vec<sysml_span::Diagnostic>> {
        let ir = compile_simple_expression(expr)?;
        let evaluator = ExpressionEvaluator::new();
        evaluator
            .eval(&ir, ctx)
            .map_err(|e| vec![sysml_span::Diagnostic::error(e.to_string())])
    }

    // -- Literal tests ----------------------------------------------------------

    #[test]
    fn literal_bool_true() {
        assert_eq!(eval("true").unwrap(), Value::Bool(true));
    }

    #[test]
    fn literal_bool_false() {
        assert_eq!(eval("false").unwrap(), Value::Bool(false));
    }

    #[test]
    fn literal_int() {
        assert_eq!(eval("42").unwrap(), Value::Int(42));
    }

    #[test]
    fn literal_negative_int() {
        assert_eq!(eval("-42").unwrap(), Value::Int(-42));
    }

    #[test]
    fn literal_float() {
        assert_eq!(eval("3.14").unwrap(), Value::Float(3.14));
    }

    #[test]
    fn literal_string() {
        assert_eq!(
            eval(r#""hello""#).unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn literal_null() {
        assert_eq!(eval("null").unwrap(), Value::Null);
    }

    // -- Binary operators -------------------------------------------------------

    #[test]
    fn add_integers() {
        assert_eq!(eval("5 + 3").unwrap(), Value::Int(8));
    }

    #[test]
    fn subtract_integers() {
        assert_eq!(eval("10 - 4").unwrap(), Value::Int(6));
    }

    #[test]
    fn multiply_integers() {
        assert_eq!(eval("6 * 7").unwrap(), Value::Int(42));
    }

    #[test]
    fn divide_integers() {
        assert_eq!(eval("20 / 4").unwrap(), Value::Int(5));
    }

    #[test]
    fn remainder() {
        assert_eq!(eval("17 % 5").unwrap(), Value::Int(2));
    }

    #[test]
    fn power() {
        assert_eq!(eval("2 ** 8").unwrap(), Value::Float(256.0));
    }

    #[test]
    fn less_than() {
        assert_eq!(eval("3 < 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("7 < 2").unwrap(), Value::Bool(false));
    }

    #[test]
    fn greater_than() {
        assert_eq!(eval("10 > 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("2 > 9").unwrap(), Value::Bool(false));
    }

    #[test]
    fn less_equal() {
        assert_eq!(eval("3 <= 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("5 <= 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("7 <= 2").unwrap(), Value::Bool(false));
    }

    #[test]
    fn greater_equal() {
        assert_eq!(eval("10 >= 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("5 >= 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("2 >= 9").unwrap(), Value::Bool(false));
    }

    #[test]
    fn equal() {
        assert_eq!(eval("5 == 5").unwrap(), Value::Bool(true));
        assert_eq!(eval("5 == 3").unwrap(), Value::Bool(false));
    }

    #[test]
    fn not_equal() {
        assert_eq!(eval("5 != 3").unwrap(), Value::Bool(true));
        assert_eq!(eval("5 != 5").unwrap(), Value::Bool(false));
    }

    #[test]
    fn logical_and() {
        assert_eq!(eval("true and true").unwrap(), Value::Bool(true));
        assert_eq!(eval("true and false").unwrap(), Value::Bool(false));
        assert_eq!(eval("false and true").unwrap(), Value::Bool(false));
        assert_eq!(eval("false and false").unwrap(), Value::Bool(false));
    }

    #[test]
    fn logical_or() {
        assert_eq!(eval("true or true").unwrap(), Value::Bool(true));
        assert_eq!(eval("true or false").unwrap(), Value::Bool(true));
        assert_eq!(eval("false or true").unwrap(), Value::Bool(true));
        assert_eq!(eval("false or false").unwrap(), Value::Bool(false));
    }

    #[test]
    fn logical_xor() {
        assert_eq!(eval("true xor true").unwrap(), Value::Bool(false));
        assert_eq!(eval("true xor false").unwrap(), Value::Bool(true));
        assert_eq!(eval("false xor true").unwrap(), Value::Bool(true));
        assert_eq!(eval("false xor false").unwrap(), Value::Bool(false));
    }

    #[test]
    fn logical_implies() {
        assert_eq!(eval("false implies false").unwrap(), Value::Bool(true));
        assert_eq!(eval("false implies true").unwrap(), Value::Bool(true));
        assert_eq!(eval("true implies false").unwrap(), Value::Bool(false));
        assert_eq!(eval("true implies true").unwrap(), Value::Bool(true));
    }

    // -- Unary operators --------------------------------------------------------

    #[test]
    fn unary_not() {
        assert_eq!(eval("not true").unwrap(), Value::Bool(false));
        assert_eq!(eval("not false").unwrap(), Value::Bool(true));
    }

    #[test]
    fn unary_negate() {
        assert_eq!(eval("-10").unwrap(), Value::Int(-10));
        assert_eq!(eval("-(-5)").unwrap(), Value::Int(5));
    }

    // -- Conditional ------------------------------------------------------------

    #[test]
    fn conditional_true() {
        assert_eq!(eval("if true ? 10 else 20").unwrap(), Value::Int(10));
    }

    #[test]
    fn conditional_false() {
        assert_eq!(eval("if false ? 10 else 20").unwrap(), Value::Int(20));
    }

    // -- Null coalescing --------------------------------------------------------

    #[test]
    fn null_coalescing_null() {
        assert_eq!(eval("null ?? 42").unwrap(), Value::Int(42));
    }

    #[test]
    fn null_coalescing_non_null() {
        assert_eq!(eval("10 ?? 42").unwrap(), Value::Int(10));
    }

    // -- Variables --------------------------------------------------------------

    #[test]
    fn variable_lookup() {
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(100));
        assert_eq!(eval_with_ctx("x", &ctx).unwrap(), Value::Int(100));
    }

    #[test]
    fn variable_in_expression() {
        let mut ctx = EvalContext::new();
        ctx.set("speed", Value::Float(85.0));
        assert_eq!(
            eval_with_ctx("speed < 100", &ctx).unwrap(),
            Value::Bool(true)
        );
    }

    // -- Function calls ---------------------------------------------------------

    #[test]
    fn function_size() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        assert_eq!(eval_with_ctx("size(items)", &ctx).unwrap(), Value::Int(3));
    }

    #[test]
    fn function_abs() {
        assert_eq!(eval("abs(-42)").unwrap(), Value::Int(42));
        assert_eq!(eval("abs(42)").unwrap(), Value::Int(42));
    }

    #[test]
    fn function_max() {
        assert_eq!(eval("max(10, 20)").unwrap(), Value::Float(20.0));
    }

    #[test]
    fn function_min() {
        assert_eq!(eval("min(10, 20)").unwrap(), Value::Float(10.0));
    }

    // -- Range ------------------------------------------------------------------

    #[test]
    fn range_expression() {
        let result = eval("1..5").unwrap();
        assert_eq!(
            result,
            Value::List(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
            ])
        );
    }

    // -- Parentheses ------------------------------------------------------------

    #[test]
    fn parentheses_precedence() {
        assert_eq!(eval("(2 + 3) * 4").unwrap(), Value::Int(20));
        assert_eq!(eval("2 + 3 * 4").unwrap(), Value::Int(14));
    }

    // -- free_variables tests ---------------------------------------------------

    #[test]
    fn free_variables_literals() {
        let ir = compile_simple_expression("42").unwrap();
        assert!(ir.free_variables().is_empty());
    }

    #[test]
    fn free_variables_single_ref() {
        let ir = compile_simple_expression("speed").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 1);
        assert!(vars.contains("speed"));
    }

    #[test]
    fn free_variables_feature_chain() {
        let ir = compile_simple_expression("vehicle.speed").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 1);
        assert!(vars.contains("vehicle"));
    }

    #[test]
    fn free_variables_binary_op() {
        let ir = compile_simple_expression("speed + altitude").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("speed"));
        assert!(vars.contains("altitude"));
    }

    #[test]
    fn free_variables_unary_op() {
        let ir = compile_simple_expression("not isDone").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 1);
        assert!(vars.contains("isDone"));
    }

    #[test]
    fn free_variables_conditional() {
        let ir = compile_simple_expression("if flag ? x else y").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 3);
        assert!(vars.contains("flag"));
        assert!(vars.contains("x"));
        assert!(vars.contains("y"));
    }

    #[test]
    fn free_variables_select_with_binding() {
        let ir = compile_simple_expression("items->select{|x| x > threshold}").unwrap();
        let vars = ir.free_variables();
        // 'items' and 'threshold' are free; 'x' is bound
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("items"));
        assert!(vars.contains("threshold"));
        assert!(!vars.contains("x"));
    }

    #[test]
    fn free_variables_collect_with_binding() {
        let ir = compile_simple_expression("items->collect{|it| it + offset}").unwrap();
        let vars = ir.free_variables();
        // 'items' and 'offset' are free; 'it' is bound
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("items"));
        assert!(vars.contains("offset"));
        assert!(!vars.contains("it"));
    }

    #[test]
    fn free_variables_nested_bindings() {
        let ir =
            compile_simple_expression("outer->select{|x| inner->collect{|y| x + y + z}}").unwrap();
        let vars = ir.free_variables();
        // 'outer', 'inner', and 'z' are free; 'x' and 'y' are bound
        assert_eq!(vars.len(), 3);
        assert!(vars.contains("outer"));
        assert!(vars.contains("inner"));
        assert!(vars.contains("z"));
        assert!(!vars.contains("x"));
        assert!(!vars.contains("y"));
    }

    #[test]
    fn free_variables_function_call() {
        let ir = compile_simple_expression("sqrt(x + y)").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("x"));
        assert!(vars.contains("y"));
    }

    #[test]
    fn free_variables_sequence() {
        let ir = ExprIR::Sequence(vec![
            ExprIR::FeatureRef("a".into()),
            ExprIR::FeatureRef("b".into()),
            ExprIR::FeatureRef("c".into()),
        ]);
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 3);
        assert!(vars.contains("a"));
        assert!(vars.contains("b"));
        assert!(vars.contains("c"));
    }

    #[test]
    fn free_variables_range() {
        let ir = compile_simple_expression("start..end").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("start"));
        assert!(vars.contains("end"));
    }

    #[test]
    fn free_variables_index() {
        let ir = compile_simple_expression("arr#(idx)").unwrap();
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("arr"));
        assert!(vars.contains("idx"));
    }

    // -- Overflow tests (F004) -----------------------------------------------

    #[test]
    fn test_overflow_add() {
        let ir = compile_simple_expression(&format!("{} + 1", i64::MAX)).unwrap();
        let ctx = EvalContext::new();
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&ir, &ctx);
        assert!(
            matches!(result, Err(EvaluationError::Overflow)),
            "i64::MAX + 1 should overflow, got {:?}",
            result
        );
    }

    #[test]
    fn test_overflow_mul() {
        let ir = compile_simple_expression(&format!("{} * 2", i64::MAX)).unwrap();
        let ctx = EvalContext::new();
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&ir, &ctx);
        assert!(
            matches!(result, Err(EvaluationError::Overflow)),
            "i64::MAX * 2 should overflow, got {:?}",
            result
        );
    }

    #[test]
    fn test_overflow_sub() {
        // Build the IR directly since i64::MIN as a string gets parsed as float
        let ir = ExprIR::BinaryOp {
            op: ir::BinOp::Subtract,
            left: Box::new(ExprIR::LiteralInt(i64::MIN)),
            right: Box::new(ExprIR::LiteralInt(1)),
        };
        let ctx = EvalContext::new();
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&ir, &ctx);
        assert!(
            matches!(result, Err(EvaluationError::Overflow)),
            "i64::MIN - 1 should overflow, got {:?}",
            result
        );
    }

    #[test]
    fn test_overflow_neg() {
        // i64::MIN negated overflows because -i64::MIN > i64::MAX
        let ir = ExprIR::UnaryOp {
            op: ir::UnaryOp::Negate,
            operand: Box::new(ExprIR::LiteralInt(i64::MIN)),
        };
        let ctx = EvalContext::new();
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&ir, &ctx);
        assert!(
            matches!(result, Err(EvaluationError::Overflow)),
            "-i64::MIN should overflow, got {:?}",
            result
        );
    }

    // -- Recursion limit tests (F005) ----------------------------------------

    #[test]
    fn test_eval_recursion_limit() {
        // Build a deeply nested expression: ((((1 + 1) + 1) + 1) ... )
        // 200 levels deep — exceeds the 128 limit
        let mut expr = ExprIR::LiteralInt(1);
        for _ in 0..200 {
            expr = ExprIR::BinaryOp {
                op: ir::BinOp::Add,
                left: Box::new(expr),
                right: Box::new(ExprIR::LiteralInt(1)),
            };
        }
        let ctx = EvalContext::new();
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&expr, &ctx);
        assert!(
            matches!(result, Err(EvaluationError::RecursionLimit { .. })),
            "deeply nested expression should hit recursion limit, got {:?}",
            result
        );
    }

    // -- Sequence construction regression tests (Item 6) -------------------------

    #[test]
    fn sequence_construction_evaluates() {
        let ir = ExprIR::Sequence(vec![
            ExprIR::LiteralInt(1),
            ExprIR::LiteralInt(2),
            ExprIR::LiteralInt(3),
        ]);
        let result = ExpressionEvaluator::new()
            .eval(&ir, &EvalContext::new())
            .unwrap();
        assert_eq!(
            result,
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    }

    #[test]
    fn sequence_empty() {
        let ir = ExprIR::Sequence(vec![]);
        let result = ExpressionEvaluator::new()
            .eval(&ir, &EvalContext::new())
            .unwrap();
        assert_eq!(result, Value::List(vec![]));
    }

    // -- Collection operation regression tests -----------------------------------

    #[test]
    fn select_filters_positive() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![
                Value::Int(-1),
                Value::Int(0),
                Value::Int(3),
                Value::Int(-5),
                Value::Int(7),
            ]),
        );
        let result = eval_with_ctx("items->select{|x| x > 0}", &ctx).unwrap();
        assert_eq!(result, Value::List(vec![Value::Int(3), Value::Int(7)]));
    }

    #[test]
    fn collect_doubles_values() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        let result = eval_with_ctx("items->collect{|x| x * 2}", &ctx).unwrap();
        assert_eq!(
            result,
            Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
        );
    }

    #[test]
    fn reject_removes_negative() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![
                Value::Int(-1),
                Value::Int(2),
                Value::Int(-3),
                Value::Int(4),
            ]),
        );
        let result = eval_with_ctx("items->reject{|x| x < 0}", &ctx).unwrap();
        assert_eq!(result, Value::List(vec![Value::Int(2), Value::Int(4)]));
    }

    #[test]
    fn select_on_empty_list() {
        let mut ctx = EvalContext::new();
        ctx.set("items", Value::List(vec![]));
        let result = eval_with_ctx("items->select{|x| x > 0}", &ctx).unwrap();
        assert_eq!(result, Value::List(vec![]));
    }

    #[test]
    fn forall_all_positive() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        let result = eval_with_ctx("items->forAll{|x| x > 0}", &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn exists_finds_negative() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(-2), Value::Int(3)]),
        );
        let result = eval_with_ctx("items->exists{|x| x < 0}", &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn exists_none_match() {
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );
        let result = eval_with_ctx("items->exists{|x| x < 0}", &ctx).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn index_on_empty_list_is_error() {
        let ir = ExprIR::Index {
            sequence: Box::new(ExprIR::Sequence(vec![])),
            index: Box::new(ExprIR::LiteralInt(1)),
        };
        let result = ExpressionEvaluator::new().eval(&ir, &EvalContext::new());
        assert!(matches!(
            result,
            Err(EvaluationError::IndexOutOfBounds { .. })
        ));
    }

    // -- Tier 2: Classification operator regression tests -------------------------

    #[test]
    fn hastype_compiles_and_evaluates() {
        let ir = compile_simple_expression("x hastype Integer").unwrap();
        assert!(matches!(
            ir,
            ExprIR::BinaryOp {
                op: ir::BinOp::HasType,
                ..
            }
        ));
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(1));
        ctx.set("Integer", Value::String("Integer".to_string()));
        let result = ExpressionEvaluator::new().eval(&ir, &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));

        // Non-matching type
        ctx.set("Real", Value::String("Real".to_string()));
        let ir2 = compile_simple_expression("x hastype Real").unwrap();
        let result2 = ExpressionEvaluator::new().eval(&ir2, &ctx).unwrap();
        // Int matches Real (numeric hierarchy)
        assert_eq!(result2, Value::Bool(true));
    }

    #[test]
    fn istype_compiles_and_evaluates() {
        let ir = compile_simple_expression("x istype Integer").unwrap();
        assert!(matches!(
            ir,
            ExprIR::BinaryOp {
                op: ir::BinOp::IsType,
                ..
            }
        ));
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(42));
        ctx.set("Integer", Value::String("Integer".to_string()));
        let result = ExpressionEvaluator::new().eval(&ir, &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));

        // Non-matching type
        ctx.set("String", Value::String("String".to_string()));
        let ir2 = compile_simple_expression("x istype String").unwrap();
        let result2 = ExpressionEvaluator::new().eval(&ir2, &ctx).unwrap();
        assert_eq!(result2, Value::Bool(false));
    }

    #[test]
    fn as_cast_compiles_and_evaluates() {
        let ir = compile_simple_expression("x as Integer").unwrap();
        assert!(matches!(
            ir,
            ExprIR::BinaryOp {
                op: ir::BinOp::As,
                ..
            }
        ));
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(42));
        ctx.set("Integer", Value::String("Integer".to_string()));
        let result = ExpressionEvaluator::new().eval(&ir, &ctx).unwrap();
        assert_eq!(result, Value::Int(42)); // passes through

        // Non-matching type → Null
        ctx.set("Boolean", Value::String("Boolean".to_string()));
        let ir2 = compile_simple_expression("x as Boolean").unwrap();
        let result2 = ExpressionEvaluator::new().eval(&ir2, &ctx).unwrap();
        assert_eq!(result2, Value::Null);
    }

    #[test]
    fn meta_access_single_classifies_against_self_metaclass() {
        // Single `@` is the KerML classification operator (filter shorthand
        // `@T` ≡ `self @ T`): it evaluates to a Bool testing whether the
        // bound `self` element's abstract-syntax metaclass conforms to T.
        // (Previously this returned UnsupportedOperator — a stub that made
        // authored `@`-filters silently pass everything.)
        use std::sync::Arc;
        use sysml_core::{ElementFactory, ElementKind, ModelGraph, Value as CoreValue};

        let ir = compile_simple_expression("@SysML::PartUsage").unwrap();
        assert!(matches!(
            ir,
            ExprIR::MetaAccess {
                is_double: false,
                ..
            }
        ));

        let mut graph = ModelGraph::new();
        let part = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        let part_id = part.id.clone();
        graph.add_element(part);

        let mut ctx = EvalContext::new();
        ctx.graph = Some(Arc::new(graph));
        ctx.set("self", CoreValue::Ref(part_id));

        let result = ExpressionEvaluator::new().eval(&ir, &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));

        // A non-conforming metaclass evaluates to false.
        let ir2 = compile_simple_expression("@SysML::ActionUsage").unwrap();
        let result2 = ExpressionEvaluator::new().eval(&ir2, &ctx).unwrap();
        assert_eq!(result2, Value::Bool(false));
    }

    #[test]
    fn meta_access_double_compiles_and_returns_unsupported() {
        let ir = compile_simple_expression("@@myElement").unwrap();
        assert!(matches!(
            ir,
            ExprIR::MetaAccess {
                is_double: true,
                ..
            }
        ));
        let mut ctx = EvalContext::new();
        ctx.set("myElement", Value::Null);
        let result = ExpressionEvaluator::new().eval(&ir, &ctx);
        assert!(matches!(
            result,
            Err(EvaluationError::UnsupportedOperator(_))
        ));
    }

    #[test]
    fn constructor_call_evaluates_to_field_map() {
        // `Pair(a = 1, b = 2)` evaluates to a Value::Map keyed by field name,
        // so a receiver can read `payload.a` via member access.
        let ir = ExprIR::ConstructorCall {
            type_name: "Pair".to_string(),
            named_args: vec![
                ("a".to_string(), Box::new(ExprIR::LiteralInt(1))),
                ("b".to_string(), Box::new(ExprIR::LiteralInt(2))),
            ],
        };
        let result = ExpressionEvaluator::new().eval(&ir, &EvalContext::new());
        match result {
            Ok(Value::Map(fields)) => {
                assert_eq!(fields.get("a"), Some(&Value::Int(1)));
                assert_eq!(fields.get("b"), Some(&Value::Int(2)));
                assert_eq!(fields.len(), 2);
            }
            other => panic!("expected Value::Map, got {other:?}"),
        }
    }

    #[test]
    fn constructor_free_variables() {
        let ir = ExprIR::ConstructorCall {
            type_name: "Pair".to_string(),
            named_args: vec![
                ("a".to_string(), Box::new(ExprIR::FeatureRef("x".into()))),
                ("b".to_string(), Box::new(ExprIR::FeatureRef("y".into()))),
            ],
        };
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("x"));
        assert!(vars.contains("y"));
    }

    #[test]
    fn meta_access_free_variables() {
        let ir = ExprIR::MetaAccess {
            operand: Box::new(ExprIR::FeatureRef("element".into())),
            is_double: false,
        };
        let vars = ir.free_variables();
        assert_eq!(vars.len(), 1);
        assert!(vars.contains("element"));
    }
}
