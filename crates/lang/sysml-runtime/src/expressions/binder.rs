//! RSC-2.3 — compile-time expression slot binding (design doc
//! §3 D-2.0.4).
//!
//! [`bind_slots`] is a pass over a compiled [`ExprIR`] that rewrites
//! name-resolved references into slot-resolved ones **after**
//! `compile_expression` and **before** any closure captures the IR (the seam
//! identified in `ode_builder.rs::OdeSpec::build_rhs`):
//!
//! - `FeatureRef(name)` whose name a [`SlotBinder`] resolves becomes
//!   [`ExprIR::SlotRef`] (original spelling kept — the evaluator stays
//!   context-name-first for byte-identical baseline parity; the slot is the
//!   fallback that replaces the eval-time graph-wide name scan at RSC-2.5).
//! - `FeatureChain` binds fully (whole dotted chain names a slot) to
//!   `SlotRef`, else its **longest static head** to
//!   [`ExprIR::SlotChainHead`], else stays untouched.
//! - `FunctionCall` names are untouched (function/calc registry, not
//!   variables). Lambda bindings (`select`/`collect`/… binding variables)
//!   shadow and are never bound.
//!
//! Names that resolve to neither a slot nor a binder-declared local are
//! collected in [`BindReport::unresolved`]; the compiler filters those
//! against the model graph and emits the `RS003 unresolved runtime name`
//! warning (a hard error from RSC-2.5).

use std::collections::{HashMap, HashSet};

use super::ir::ExprIR;
use crate::slots::{SlotId, SlotStore};

/// Name-resolution scope for [`bind_slots`].
///
/// Two scopes exist (design doc D-2.0.4):
/// - **Orchestrator scope** ([`SlotBinder::global`]): constraints and
///   computed/gated expressions evaluate against the master context, so
///   canonical and runtime spellings resolve through the store's alias
///   table directly.
/// - **Subsystem scope** ([`SlotBinder::for_subsystem`]): expressions inside
///   a prefixed subsystem instance (e.g. circuit5's ODE RHS) use the
///   instance's *local* names. The binder maps every store name of the form
///   `{var_prefix}.{local}` to its slot under the bare `local` spelling, so
///   `temperature` inside circuit5's subsystem binds to *circuit5's* slot.
///   Local aliases win over global spellings.
pub struct SlotBinder<'a> {
    store: &'a SlotStore,
    /// Subsystem-local aliases (`local name → slot`), empty in global scope.
    local: HashMap<String, SlotId>,
    /// Whether this binder was built for a prefixed subsystem instance
    /// (`for_subsystem(_, Some(prefix))`). Distinguishes "resolved through
    /// the instance's own namespace" from "fell through to a global
    /// spelling" — see [`resolve_subsystem_local`](Self::resolve_subsystem_local).
    has_prefix: bool,
    /// Names known to be bound into the evaluation context at tick time
    /// without being slots (ODE `t`, signal targets, injected config
    /// values, …). Never bound; suppressed from `unresolved` reporting.
    locally_known: HashSet<String>,
}

impl<'a> SlotBinder<'a> {
    /// Orchestrator-scope binder: resolves names through the store's alias
    /// table only (canonical + runtime spellings).
    pub fn global(store: &'a SlotStore) -> Self {
        SlotBinder {
            store,
            local: HashMap::new(),
            has_prefix: false,
            locally_known: HashSet::new(),
        }
    }

    /// Subsystem-scope binder for an instance with the given `var_prefix`
    /// (`None` behaves like [`global`](Self::global) — a top-level
    /// subsystem's local names ARE the store's bare spellings).
    ///
    /// Alias construction is deterministic: candidate names are sorted
    /// before first-wins insertion so a (pathological) collision between
    /// two store spellings stripping to the same local name cannot flap
    /// across builds.
    pub fn for_subsystem(store: &'a SlotStore, var_prefix: Option<&str>) -> Self {
        let mut local = HashMap::new();
        if let Some(prefix) = var_prefix {
            let dot = format!("{prefix}.");
            let mut candidates: Vec<(&str, SlotId)> = store
                .names()
                .filter_map(|(name, id)| name.strip_prefix(dot.as_str()).map(|rest| (rest, id)))
                .collect();
            candidates.sort_by_key(|(rest, _)| *rest);
            for (rest, id) in candidates {
                local.entry(rest.to_owned()).or_insert(id);
            }
        }
        SlotBinder {
            store,
            local,
            has_prefix: var_prefix.is_some(),
            locally_known: HashSet::new(),
        }
    }

    /// Declare names that the executor binds into its evaluation context at
    /// tick time without them being slots (suppresses RS003 noise).
    pub fn with_locals(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.locally_known.extend(names);
        self
    }

    /// RSC-3.1 (design doc D-3.0.6-B): overlay owner-scoped local aliases on
    /// top of the global binder, mirroring the eval-time owner overlay the
    /// service layer applies in `check_constraints`. Each `(name, slot)` pair
    /// is the constraint owner's (or an ancestor's) attribute feature resolved
    /// to its minted slot. Owner-local aliases win over the global store
    /// spelling (the [`resolve`](Self::resolve) `local`-first ordering), so a
    /// bare attribute name inside a constraint binds to *its owner's* slot.
    ///
    /// Insertion order matters: pass nearest-owner names first; an already
    /// present alias is not overwritten, so the owner shadows its ancestors.
    pub fn with_owner_aliases(
        mut self,
        aliases: impl IntoIterator<Item = (String, SlotId)>,
    ) -> Self {
        for (name, slot) in aliases {
            self.local.entry(name).or_insert(slot);
        }
        self
    }

    /// Resolve a name (or a dotted join of chain segments) to a slot.
    /// Subsystem-local aliases win over the store's global spellings.
    pub fn resolve(&self, name: &str) -> Option<SlotId> {
        self.local
            .get(name)
            .copied()
            .or_else(|| self.store.slot_by_name(name))
    }

    /// RSC-2.4a: resolve a name strictly within the subsystem's OWN
    /// namespace. For a prefixed binder this consults only the
    /// prefix-stripped alias map (the instance's own slots) — a fall-through
    /// to a same-named *global* spelling is deliberately NOT a match,
    /// because the global slot may carry a different (top-level default)
    /// value than the instance's. For an unprefixed binder the subsystem's
    /// local names ARE the store's bare spellings, so this is a plain store
    /// lookup. Used by `OdeSpec::bind_slots` to decide which template
    /// parameters the slot table can serve live.
    pub fn resolve_subsystem_local(&self, name: &str) -> Option<SlotId> {
        if self.has_prefix {
            self.local.get(name).copied()
        } else {
            self.store.slot_by_name(name)
        }
    }

    /// Whether `name` was declared via [`with_locals`](Self::with_locals).
    pub fn is_locally_known(&self, name: &str) -> bool {
        self.locally_known.contains(name)
    }
}

/// Outcome counters of one or more [`bind_slots`] passes.
#[derive(Debug, Clone, Default)]
pub struct BindReport {
    /// `FeatureRef`s rewritten to `SlotRef`.
    pub bound_refs: usize,
    /// `FeatureChain`s fully bound to `SlotRef` (whole chain named a slot).
    pub bound_chains: usize,
    /// `FeatureChain`s partially bound to `SlotChainHead` (longest head).
    pub bound_chain_heads: usize,
    /// `FeatureRef`s / `FeatureChain`s left untouched.
    pub unbound: usize,
    /// Names that resolved to neither a slot nor a binder-declared local —
    /// RS003 candidates (the compiler still filters against graph features).
    pub unresolved: Vec<String>,
}

impl BindReport {
    /// Fold another report into this one.
    pub fn merge(&mut self, other: BindReport) {
        self.bound_refs += other.bound_refs;
        self.bound_chains += other.bound_chains;
        self.bound_chain_heads += other.bound_chain_heads;
        self.unbound += other.unbound;
        self.unresolved.extend(other.unresolved);
    }

    /// Total references that bound to a slot (refs + full chains + heads).
    pub fn total_bound(&self) -> usize {
        self.bound_refs + self.bound_chains + self.bound_chain_heads
    }
}

/// Rewrite name references in `expr` to slot references per `binder`
/// (RSC-2.3, design doc D-2.0.4). Idempotent: already-bound `SlotRef` /
/// `SlotChainHead` nodes are left alone.
pub fn bind_slots(expr: &mut ExprIR, binder: &SlotBinder<'_>, report: &mut BindReport) {
    let mut lambda_bound: HashSet<String> = HashSet::new();
    bind_inner(expr, binder, &mut lambda_bound, report);
}

fn bind_inner(
    expr: &mut ExprIR,
    binder: &SlotBinder<'_>,
    lambda_bound: &mut HashSet<String>,
    report: &mut BindReport,
) {
    match expr {
        ExprIR::LiteralInt(_)
        | ExprIR::LiteralReal(_)
        | ExprIR::LiteralBool(_)
        | ExprIR::LiteralString(_)
        | ExprIR::LiteralQuantity { .. }
        | ExprIR::LiteralNull
        | ExprIR::SlotRef { .. }
        | ExprIR::SlotChainHead { .. } => {}

        ExprIR::FeatureRef(name) => {
            if lambda_bound.contains(name.as_str()) {
                return;
            }
            if let Some(slot) = binder.resolve(name) {
                report.bound_refs += 1;
                *expr = ExprIR::SlotRef {
                    slot,
                    name: std::mem::take(name),
                };
            } else {
                report.unbound += 1;
                if !binder.is_locally_known(name) {
                    report.unresolved.push(name.clone());
                }
            }
        }

        ExprIR::FeatureChain(names) => {
            if names.is_empty() || lambda_bound.contains(names[0].as_str()) {
                return;
            }
            // Full bind: the whole dotted chain names a slot.
            let full = names.join(".");
            if let Some(slot) = binder.resolve(&full) {
                report.bound_chains += 1;
                *expr = ExprIR::SlotRef { slot, name: full };
                return;
            }
            // Longest static head (len-1 .. 1 segments).
            for k in (1..names.len()).rev() {
                let head = names[..k].join(".");
                if let Some(slot) = binder.resolve(&head) {
                    report.bound_chain_heads += 1;
                    *expr = ExprIR::SlotChainHead {
                        slot,
                        names: std::mem::take(names),
                        bound: k,
                    };
                    return;
                }
            }
            report.unbound += 1;
            if let Some(first) = names.first() {
                if !binder.is_locally_known(first) {
                    report.unresolved.push(first.clone());
                }
            }
        }

        ExprIR::BinaryOp { left, right, .. } => {
            bind_inner(left, binder, lambda_bound, report);
            bind_inner(right, binder, lambda_bound, report);
        }
        ExprIR::UnaryOp { operand, .. } => bind_inner(operand, binder, lambda_bound, report),
        ExprIR::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            bind_inner(condition, binder, lambda_bound, report);
            bind_inner(then_expr, binder, lambda_bound, report);
            bind_inner(else_expr, binder, lambda_bound, report);
        }
        ExprIR::NullCoalescing { expr, default } => {
            bind_inner(expr, binder, lambda_bound, report);
            bind_inner(default, binder, lambda_bound, report);
        }
        ExprIR::Select {
            source,
            binding,
            predicate,
        }
        | ExprIR::Reject {
            source,
            binding,
            predicate,
        }
        | ExprIR::ForAll {
            source,
            binding,
            predicate,
        }
        | ExprIR::Exists {
            source,
            binding,
            predicate,
        } => {
            bind_inner(source, binder, lambda_bound, report);
            let newly = lambda_bound.insert(binding.clone());
            bind_inner(predicate, binder, lambda_bound, report);
            if newly {
                lambda_bound.remove(binding.as_str());
            }
        }
        ExprIR::Collect {
            source,
            binding,
            transform,
        } => {
            bind_inner(source, binder, lambda_bound, report);
            let newly = lambda_bound.insert(binding.clone());
            bind_inner(transform, binder, lambda_bound, report);
            if newly {
                lambda_bound.remove(binding.as_str());
            }
        }
        ExprIR::Index { sequence, index } => {
            bind_inner(sequence, binder, lambda_bound, report);
            bind_inner(index, binder, lambda_bound, report);
        }
        // Function/calc names are registry lookups, not variables (D-2.0.4).
        ExprIR::FunctionCall { args, .. } => {
            for arg in args {
                bind_inner(arg, binder, lambda_bound, report);
            }
        }
        ExprIR::Sequence(items) => {
            for item in items {
                bind_inner(item, binder, lambda_bound, report);
            }
        }
        ExprIR::Range { lower, upper } => {
            bind_inner(lower, binder, lambda_bound, report);
            bind_inner(upper, binder, lambda_bound, report);
        }
        ExprIR::MetaAccess { operand, .. } => bind_inner(operand, binder, lambda_bound, report),
        ExprIR::ConstructorCall { named_args, .. } => {
            for (_, arg) in named_args {
                bind_inner(arg, binder, lambda_bound, report);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::slots::{RuntimeId, SlotMeta, Variability, WriterId};
    use sysml_core::{ElementId, Value};

    fn meta(decl: &str, path: &[&str], canonical: &str, runtime: &str) -> SlotMeta {
        SlotMeta::new(
            RuntimeId::scoped(
                ElementId::from_string(format!("decl:{decl}")),
                path.iter()
                    .map(|p| ElementId::from_string(format!("usage:{p}"))),
            ),
            Variability::Continuous,
            WriterId::Orchestrator,
            canonical,
            runtime,
        )
    }

    /// Store mimicking two multi-circuit fixture instances + a bare parameter.
    fn demo_store() -> SlotStore {
        let mut store = SlotStore::new();
        store.intern(
            meta(
                "bimetalTemp",
                &["circuit1"],
                "Panel.circuit1.thermalModel.bimetalTemp",
                "circuit1.bimetalTemp",
            ),
            Value::Float(298.15),
        );
        store.intern(
            meta(
                "bimetalTemp",
                &["circuit2"],
                "Panel.circuit2.thermalModel.bimetalTemp",
                "circuit2.bimetalTemp",
            ),
            Value::Float(298.15),
        );
        store.intern(
            meta("ratedCurrent", &[], "ratedCurrent", "ratedCurrent"),
            Value::Float(16.0),
        );
        store
    }

    fn parse(src: &str) -> ExprIR {
        super::super::compile_simple_expression(src).unwrap()
    }

    #[test]
    fn local_name_binds_to_the_instances_own_slot() {
        let store = demo_store();
        let c1 = store.slot_by_name("circuit1.bimetalTemp").unwrap();
        let c2 = store.slot_by_name("circuit2.bimetalTemp").unwrap();

        // Same local spelling, different instance scopes → different slots.
        for (prefix, expected) in [("circuit1", c1), ("circuit2", c2)] {
            let binder = SlotBinder::for_subsystem(&store, Some(prefix));
            let mut expr = ExprIR::FeatureRef("bimetalTemp".to_owned());
            let mut report = BindReport::default();
            bind_slots(&mut expr, &binder, &mut report);
            assert_eq!(report.bound_refs, 1);
            assert!(report.unresolved.is_empty());
            match expr {
                ExprIR::SlotRef { slot, ref name } => {
                    assert_eq!(slot, expected, "{prefix} local must bind its own slot");
                    assert_eq!(name, "bimetalTemp", "original spelling kept");
                }
                other => panic!("expected SlotRef, got {other:?}"),
            }
        }
    }

    #[test]
    fn canonical_chain_binds_fully() {
        let store = demo_store();
        let expected = store
            .slot_by_name("Panel.circuit1.thermalModel.bimetalTemp")
            .unwrap();
        let binder = SlotBinder::global(&store);
        let mut expr = parse("Panel.circuit1.thermalModel.bimetalTemp");
        assert!(matches!(expr, ExprIR::FeatureChain(_)));
        let mut report = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report);
        assert_eq!(report.bound_chains, 1);
        match expr {
            ExprIR::SlotRef { slot, ref name } => {
                assert_eq!(slot, expected);
                assert_eq!(name, "Panel.circuit1.thermalModel.bimetalTemp");
            }
            other => panic!("expected fully-bound SlotRef, got {other:?}"),
        }
    }

    #[test]
    fn longest_static_head_binds_as_chain_head() {
        let store = demo_store();
        let head_slot = store.slot_by_name("circuit1.bimetalTemp").unwrap();
        let binder = SlotBinder::global(&store);
        // "circuit1.bimetalTemp" binds; ".unit" stays a navigated tail.
        let mut expr = parse("circuit1.bimetalTemp.unit");
        let mut report = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report);
        assert_eq!(report.bound_chain_heads, 1);
        match expr {
            ExprIR::SlotChainHead {
                slot,
                ref names,
                bound,
            } => {
                assert_eq!(slot, head_slot);
                assert_eq!(bound, 2, "two leading segments bound");
                assert_eq!(names, &["circuit1", "bimetalTemp", "unit"]);
            }
            other => panic!("expected SlotChainHead, got {other:?}"),
        }
    }

    #[test]
    fn unbindable_names_stay_untouched_and_are_reported() {
        let store = demo_store();
        let binder = SlotBinder::global(&store);
        let mut expr = parse("mysteryName + ratedCurrent");
        let mut report = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report);
        assert_eq!(report.bound_refs, 1, "ratedCurrent binds");
        assert_eq!(report.unbound, 1);
        assert_eq!(report.unresolved, vec!["mysteryName".to_owned()]);
        // The unresolved ref is structurally untouched.
        match expr {
            ExprIR::BinaryOp { ref left, .. } => {
                assert_eq!(**left, ExprIR::FeatureRef("mysteryName".to_owned()));
            }
            other => panic!("expected BinaryOp, got {other:?}"),
        }
        // Unbindable chains stay FeatureChain.
        let mut chain = parse("nowhere.at.all");
        let mut report = BindReport::default();
        bind_slots(&mut chain, &binder, &mut report);
        assert!(matches!(chain, ExprIR::FeatureChain(_)));
        assert_eq!(report.unresolved, vec!["nowhere".to_owned()]);
    }

    #[test]
    fn locals_suppress_unresolved_and_lambda_bindings_shadow() {
        let store = demo_store();
        let binder =
            SlotBinder::for_subsystem(&store, Some("circuit1")).with_locals(["t".to_owned()]);
        let mut expr = parse("t + bimetalTemp");
        let mut report = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report);
        assert_eq!(report.bound_refs, 1, "bimetalTemp binds locally");
        assert_eq!(report.unbound, 1, "t is unbound");
        assert!(
            report.unresolved.is_empty(),
            "t is locally known — no RS003 candidate"
        );

        // Lambda binding shadows: |x| inside the body never binds even if a
        // store name collides.
        let mut store2 = demo_store();
        let rated = store2.slot_by_name("ratedCurrent").unwrap();
        store2.add_alias("x", rated);
        let binder2 = SlotBinder::global(&store2);
        let mut lambda = parse("items->select{|x| x > ratedCurrent}");
        let mut report2 = BindReport::default();
        bind_slots(&mut lambda, &binder2, &mut report2);
        match lambda {
            ExprIR::Select { ref predicate, .. } => match **predicate {
                ExprIR::BinaryOp { ref left, .. } => {
                    assert_eq!(
                        **left,
                        ExprIR::FeatureRef("x".to_owned()),
                        "lambda-bound x stays a FeatureRef"
                    );
                }
                ref other => panic!("expected BinaryOp predicate, got {other:?}"),
            },
            ref other => panic!("expected Select, got {other:?}"),
        }
        assert!(
            !report2.unresolved.contains(&"x".to_owned()),
            "lambda-bound names are not RS003 candidates"
        );
    }

    #[test]
    fn binding_is_idempotent() {
        let store = demo_store();
        let binder = SlotBinder::global(&store);
        let mut expr = parse("ratedCurrent + circuit1.bimetalTemp.unit");
        let mut report = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report);
        let once = expr.clone();
        let mut report2 = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report2);
        assert_eq!(expr, once, "second pass is a no-op");
        assert_eq!(report2.total_bound(), 0);
    }

    /// RSC-3.1 (D-3.0.6-B): owner-scoped aliases let a constraint reference a
    /// bare attribute name that resolves to its owner's (instance-prefixed)
    /// slot. The bare name is NOT a global store spelling, so without the
    /// owner overlay it would be an RS003 candidate; with it, it binds.
    #[test]
    fn owner_aliases_bind_bare_owner_attribute() {
        let store = demo_store();
        // `bimetalTemp` is only stored as `circuit1.bimetalTemp` /
        // `circuit2.bimetalTemp` — a bare reference is unresolved globally.
        let global = SlotBinder::global(&store);
        let mut unbound_expr = parse("bimetalTemp > 150");
        let mut r0 = BindReport::default();
        bind_slots(&mut unbound_expr, &global, &mut r0);
        assert_eq!(r0.bound_refs, 0, "bare bimetalTemp does not bind globally");
        assert!(r0.unresolved.contains(&"bimetalTemp".to_owned()));

        // With the owner overlay mapping bare `bimetalTemp` -> circuit1's slot.
        let c1 = store.slot_by_name("circuit1.bimetalTemp").unwrap();
        let owner_binder =
            SlotBinder::global(&store).with_owner_aliases([("bimetalTemp".to_owned(), c1)]);
        let mut bound_expr = parse("bimetalTemp > 150");
        let mut r1 = BindReport::default();
        bind_slots(&mut bound_expr, &owner_binder, &mut r1);
        assert_eq!(r1.bound_refs, 1, "owner-scoped bare name binds to its slot");
        assert!(r1.unresolved.is_empty());
        assert_eq!(owner_binder.resolve("bimetalTemp"), Some(c1));
    }

    /// RSC-3.1: owner aliases win over a same-named global spelling, and a
    /// genuinely-missing name still reports unresolved. First-inserted alias
    /// wins (owner shadows ancestor when both supply the same name).
    #[test]
    fn owner_aliases_precedence_and_missing() {
        let store = demo_store();
        let c1 = store.slot_by_name("circuit1.bimetalTemp").unwrap();
        let c2 = store.slot_by_name("circuit2.bimetalTemp").unwrap();
        let rated = store.slot_by_name("ratedCurrent").unwrap();

        // Owner supplies `ratedCurrent` -> c1 (nearer), shadowing both the
        // ancestor's c2 entry and the global `ratedCurrent` slot.
        let binder = SlotBinder::global(&store).with_owner_aliases([
            ("ratedCurrent".to_owned(), c1), // owner (first wins)
            ("ratedCurrent".to_owned(), c2), // ancestor (ignored)
        ]);
        assert_eq!(
            binder.resolve("ratedCurrent"),
            Some(c1),
            "owner alias wins over ancestor alias and global spelling"
        );
        assert_ne!(binder.resolve("ratedCurrent"), Some(rated));

        // A name in no scope is still unresolved.
        let mut expr = parse("missingAttr + 1");
        let mut report = BindReport::default();
        bind_slots(&mut expr, &binder, &mut report);
        assert!(report.unresolved.contains(&"missingAttr".to_owned()));
    }
}
