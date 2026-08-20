//! Normalized projections of `ExecutionSnapshot` for UI / streaming consumers.
//!
//! The orchestrator's `ExecutionSnapshot` carries the raw execution state
//! (including internal accounting variables, Quantity/Complex value variants,
//! etc.). `NormalizedSnapshot` is the frontend-friendly projection every
//! client (React, CLI, MCP) shares: scalars are collapsed to `f64`, strings
//! to `String`, subsystems to a minimal `SubsystemView`, and internal
//! `__`-prefixed variables are filtered out.
//!

use std::collections::{HashMap, HashSet};

use sysml_core::Value;

use crate::cases::VerdictKind;
use crate::expressions::is_internal_var;
use crate::orchestrator::{ConstraintEvalResult, ExecutionSnapshot};
use crate::slots::SlotStore;
use crate::SubsystemState;

/// Typed projection of a subsystem's live state.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SubsystemView {
    /// Current state name (for state machines) or node id (for actions).
    pub current_state: String,
    /// Whether the subsystem has completed.
    pub completed: bool,
    /// Human-readable kind label ("stateMachine", "action", "ode", ...).
    pub kind_label: String,
    /// Transitions eligible from `current_state` as `(event_name,
    /// target_state)` — the set the frontend needs to render an
    /// "inject event" rail + transition table, closing GAP-SM-002.
    /// `#[serde(default)]` keeps on-wire compatibility for clients
    /// that pre-date the field (it's the only new field here).
    #[cfg_attr(feature = "serde", serde(default))]
    pub available_transitions: Vec<(String, String)>,
    /// `ElementId` of the subsystem's source element (the StateUsage /
    /// StateDefinition / ODE owner that compiled into this runtime
    /// subsystem). Surfaced so the frontend can build an id-keyed
    /// lookup and stop matching subsystems by short name (which
    /// collides across nested scopes). Sourced from
    /// `SubsystemState.source_element_id` at projection time.
    /// `None` for legacy subsystems with no recorded element id; the
    /// frontend falls back to name-keyed lookup in that case.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub element_id: Option<sysml_core::ElementId>,
}

/// Typed projection of a single constraint evaluation.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ConstraintView {
    pub name: String,
    pub expression: Option<String>,
    /// Four-valued verdict (`pass` / `fail` / `inconclusive` / `error`),
    /// forwarded from [`ConstraintEvalResult::verdict`]. A constraint the
    /// run could not decide is `inconclusive`, never `fail`.
    pub verdict: VerdictKind,
    /// Live numeric values of every identifier referenced by this
    /// constraint at the tick it was evaluated. Lets the UI render
    /// the "why does this constraint currently pass/fail?" surface
    /// (GAP-CONSTR-002). Empty when the constraint has no free
    /// variables, when none are numeric, or for older cached frames.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub operands: HashMap<String, f64>,
    /// `ElementId` of the constraint usage in the model graph.
    /// Surfaced so the frontend can build an id-keyed lookup and
    /// stop matching constraints by short name (which collides
    /// across nested scopes). Sourced from `ConstraintIR.owner_id`
    /// at projection time. `None` when the underlying IR has no
    /// owner id; the frontend falls back to name-keyed lookup.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub element_id: Option<sysml_core::ElementId>,
}

/// A typed, frontend-friendly projection of [`ExecutionSnapshot`].
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NormalizedSnapshot {
    pub tick: u64,
    pub time_ms: f64,
    pub completed: bool,
    pub subsystems: HashMap<String, SubsystemView>,
    /// Numeric variables (Int / Float / Quantity.value / Complex.re / Bool → 0|1).
    pub scalar_vars: HashMap<String, f64>,
    /// String-valued variables (String / Enum).
    ///
    /// `Value::Ref` entries in the raw context are internal runtime
    /// sentinels that drive lazy feature-chain resolution in the
    /// expression evaluator (see `compiler::context_from_graph_with_options`).
    /// They are not user-facing data and must not land here — otherwise
    /// every un-valued attribute surfaces to the UI as a raw element id.
    pub string_vars: HashMap<String, String>,
    pub constraint_results: Vec<ConstraintView>,
    /// Live port feature scalars keyed by `owner.port` → `feature_name` →
    /// `f64`. Populated when the orchestrator has a `PortRegistry`
    /// configured; an empty map (and omitted from the JSON wire format)
    /// otherwise. Non-scalar features are dropped in the same way as
    /// `scalar_vars` — they can't render in a live Connections panel.
    /// Closes GAP-FLOW-001.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub port_values: HashMap<String, HashMap<String, f64>>,
    /// Instantaneous `dy/dt` values for every ODE state variable at
    /// this tick, keyed by the same name the state value carries in
    /// `scalar_vars` (prefixed when the subsystem is scoped to an
    /// instance). Empty when no ODE subsystems are present or none
    /// have stepped yet. Closes GAP-ODE-002.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub derivatives: HashMap<String, f64>,
    /// RSC-5.4 (D-5.0.7): unit name for projected variables whose slot carries
    /// an explicit unit (`"K"`, `"mA"`), keyed like `scalar_vars`. Sourced from
    /// the snapshot's slot-derived `value_units`; absent for type-only ISQ slots
    /// (no explicit unit) and non-quantity variables. The FE prefers this
    /// runtime unit over the static projection unit.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub unit_vars: HashMap<String, String>,
    /// RSC-5.4 (D-5.0.7): ISQ dimension string (`"M·L·T⁻²"` etc.) for every
    /// projected variable whose slot carries a `MeasurementRef`, keyed like
    /// `scalar_vars`. Present for all ISQ-typed slots (unit-bearing or not).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub dimension_vars: HashMap<String, String>,
}

/// Options for [`normalize_with`].
///
/// In Stage 1 we only expose an external variable-name filter. Later stages
/// will wire an `is_stdlib` flag on `Element`, at which point
/// `ModelCompiler` can produce the stdlib filter set once and hand it to
/// this function instead of each client reconstructing it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NormalizeOptions<'a> {
    /// Variable names to exclude from the output (applied after the
    /// built-in `__`/`t_ms`/`tick` filter).
    pub exclude_vars: Option<&'a HashSet<String>>,
    /// Task #8 (steward ruling 2026-07-07): the slot store, used to gate the
    /// observable projection to **meta spellings only**. When present, a
    /// variable key that resolves to a slot is admitted only if it is that
    /// slot's `canonical_name` or `runtime_name`
    /// ([`SlotStore::is_meta_spelling`]); `add_alias` spellings (e.g. an ODE's
    /// qualified `{ode}.duty` observable key) are resolution-only and are
    /// dropped from `scalar_vars`/`string_vars`. Keys that resolve to no slot
    /// (context vars, lambda temporaries, `config.*`) always pass through.
    ///
    /// `None` = no slot store available (store-less, hand-built snapshots such
    /// as this module's own unit tests): nothing is classifiable, so every key
    /// passes — those snapshots populate `variables` directly and carry no
    /// alias spellings. The one production caller
    /// (`sysml-service` `RuntimeSession::step`) always supplies the session's
    /// live store, so the sim-app / service / MCP timeseries surface is gated.
    pub slots: Option<&'a SlotStore>,
}

/// Extract a scalar `f64` from a [`Value`] when one is defined.
///
/// * `Bool(b)` → `1.0` if true, `0.0` if false (mirrors the TS normalizer).
/// * `Int` / `Float` / `Quantity.value` → as-is.
/// * `Complex { re, .. }` → the real part (time-series can only display a
///   scalar per series).
/// * Everything else → `None`.
pub fn value_to_scalar(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        Value::Quantity { value, .. } => Some(*value),
        Value::Complex { re, .. } => Some(*re),
        _ => None,
    }
}

/// Extract a string rendering from a [`Value`] when one makes sense.
///
/// Returns `Some` for `String` and `Enum`; everything else returns `None`.
///
/// `Value::Ref` is deliberately *not* rendered: the runtime stashes
/// `Value::Ref(self.id)` into the eval context for every un-valued
/// feature so that expression evaluation can lazily resolve feature
/// chains against the graph. Those sentinels carry no user-facing
/// meaning — surfacing them as string ids would show every "no value
/// yet" attribute as a raw UUID in the UI.
pub fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Enum(s) => Some(s.clone()),
        _ => None,
    }
}

/// Projection-level bookkeeping filter for the observable surface.
///
/// [`is_internal_var`] (the eval-context / mirror predicate) catches the *bare*
/// internal tokens (`__clock_time`, `t_ms`, `tick`) but not the SM-qualified gate
/// variable `{sm}.__flow_gate`, whose `__` token sits in the LEAF segment — so
/// that name leaks into `scalar_vars` → `sysml.sessions.timeseries_names`
/// (task #11). The observable projection narrows one step further: it also drops a
/// name whose leaf segment is a `__`-internal token.
///
/// This narrows the PROJECTION only. The value mirror (`snapshot.variables`) and
/// the shared `is_internal_var` predicate are untouched, so raw-mirror / debug
/// reads and `SlotStore::slot_by_name` still resolve `{sm}.__flow_gate`. When
/// bookkeeping slots become typed (`SlotMeta::bookkeeping`, the eval-identity cull
/// arc), that classification subsumes both this narrowing and `is_internal_var`'s
/// leaf gap — this is the designated fold-in point.
fn is_projection_hidden(name: &str) -> bool {
    is_internal_var(name)
        || name
            .rsplit('.')
            .next()
            .is_some_and(|leaf| leaf.starts_with("__"))
}

/// Normalize an [`ExecutionSnapshot`] with default options.
pub fn normalize(snapshot: &ExecutionSnapshot) -> NormalizedSnapshot {
    normalize_with(snapshot, NormalizeOptions::default())
}

/// Normalize an [`ExecutionSnapshot`], honouring [`NormalizeOptions`].
pub fn normalize_with(
    snapshot: &ExecutionSnapshot,
    opts: NormalizeOptions<'_>,
) -> NormalizedSnapshot {
    let mut scalar_vars = HashMap::with_capacity(snapshot.variables.len());
    let mut string_vars = HashMap::new();

    for (name, value) in snapshot.variables.iter() {
        if is_projection_hidden(name) {
            continue;
        }
        if let Some(exclude) = opts.exclude_vars {
            if exclude.contains(name.as_str()) {
                continue;
            }
        }
        // Task #8 (steward ruling 2026-07-07): observable projection = meta
        // spellings only. When a key resolves to a slot, admit it iff it is
        // that slot's canonical or runtime name; `add_alias` spellings (e.g.
        // an ODE's qualified `{ode}.duty`, minted only so ownerPath-driven
        // `slot_by_name` lookups resolve) are resolution-only and never
        // surface as observables. Keys that resolve to no slot (context vars,
        // lambda temporaries, `config.*`) pass through unfiltered. The value
        // MIRROR in `snapshot.variables` still carries every spelling for read
        // coherence — this gate is the separate observable-projection surface.
        if let Some(slots) = opts.slots {
            if let Some(id) = slots.slot_by_name(name) {
                if !slots.is_meta_spelling(id, name) {
                    continue;
                }
            }
        }
        // Prefer the orchestrator-resolved value when the raw binding is
        // a lazy `Value::Ref` sentinel (see `ExecutionSnapshot.resolved_refs`
        // and `compiler::context_from_graph_with_options`). The raw Ref
        // itself projects to nothing — it's internal state — so without
        // this overlay every expression-resolvable attribute surfaces as
        // "—" in the UI, breaking sparkline buffering, tree value cells,
        // and the detail panel's current-value cell.
        let effective = match value {
            Value::Ref(_) => snapshot.resolved_refs.get(name).unwrap_or(value),
            _ => value,
        };
        if let Some(f) = value_to_scalar(effective) {
            scalar_vars.insert(name.clone(), f);
        } else if let Some(s) = value_to_string(effective) {
            string_vars.insert(name.clone(), s);
        }
        // Non-scalar, non-string values (List / Map / Null) are dropped —
        // they aren't renderable as a live tick value and clients that
        // need them can fetch the raw snapshot.
    }

    let subsystems = snapshot
        .subsystem_states
        .iter()
        .map(|(name, state)| (name.clone(), subsystem_view(state)))
        .collect();

    let constraint_results = snapshot
        .constraint_results
        .iter()
        .map(constraint_view)
        .collect();

    // Drop ports whose features carry no scalar values (reject silently
    // — the UI can't render a port with nothing to plot). An empty
    // outer map serializes as an omitted field via `skip_serializing_if`.
    let port_values = snapshot
        .port_values
        .iter()
        .filter_map(|(key, features)| {
            let scalars: HashMap<String, f64> = features
                .iter()
                .filter_map(|(name, value)| value_to_scalar(value).map(|f| (name.clone(), f)))
                .collect();
            if scalars.is_empty() {
                None
            } else {
                Some((key.clone(), scalars))
            }
        })
        .collect();

    // Derivatives are already `f64` on the runtime side — we just
    // clone-forward. Apply the same exclude filter so a client that
    // wants to hide a specific state var's chart also hides its dy/dt.
    let derivatives: HashMap<String, f64> = snapshot
        .derivatives
        .iter()
        .filter(|(name, _)| {
            opts.exclude_vars
                .map(|ex| !ex.contains(name.as_str()))
                .unwrap_or(true)
        })
        .map(|(k, v)| (k.clone(), *v))
        .collect();

    // RSC-5.4 (D-5.0.7): surface each projected variable's measurement metadata.
    // Join the snapshot's slot-sourced `value_units` (keyed by both name
    // spellings) to the names we actually projected into `scalar_vars`, so
    // excluded/internal vars never leak a unit. Dimension surfaces for every
    // ISQ slot; unit only when the slot's m_ref carries one (explicit `[unit]`).
    let mut unit_vars: HashMap<String, String> = HashMap::new();
    let mut dimension_vars: HashMap<String, String> = HashMap::new();
    if !snapshot.value_units.is_empty() {
        for name in scalar_vars.keys() {
            if let Some(vm) = snapshot.value_units.get(name) {
                dimension_vars.insert(name.clone(), vm.dimension.to_string());
                if let Some(u) = &vm.unit {
                    unit_vars.insert(name.clone(), u.clone());
                }
            }
        }
    }

    NormalizedSnapshot {
        tick: snapshot.tick,
        time_ms: snapshot.time_ms,
        completed: snapshot.completed,
        subsystems,
        scalar_vars,
        string_vars,
        constraint_results,
        port_values,
        derivatives,
        unit_vars,
        dimension_vars,
    }
}

fn subsystem_view(state: &SubsystemState) -> SubsystemView {
    SubsystemView {
        current_state: state.current_state.clone(),
        completed: state.completed,
        kind_label: state.kind.to_string(),
        available_transitions: state.available_transitions.clone(),
        element_id: state.source_element_id.clone(),
    }
}

fn constraint_view(c: &ConstraintEvalResult) -> ConstraintView {
    ConstraintView {
        name: c.name.clone(),
        expression: c.expression.clone(),
        verdict: c.verdict,
        operands: c.operands.clone(),
        element_id: c.element_id.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{ElementId, Value};

    use crate::physics::DimensionVector;

    fn make_snapshot() -> ExecutionSnapshot {
        let mut variables = HashMap::new();
        variables.insert("T_busbar".into(), Value::Float(320.5));
        variables.insert("ticks_elapsed".into(), Value::Int(42));
        variables.insert("energized".into(), Value::Bool(true));
        variables.insert("mode".into(), Value::Enum("normal".into()));
        variables.insert(
            "current".into(),
            Value::Quantity {
                value: 12.3,
                dimension: DimensionVector::default(),
                unit: Some("A".into()),
            },
        );
        variables.insert("impedance".into(), Value::Complex { re: 0.5, im: -0.1 });
        variables.insert("source_id".into(), Value::Ref(ElementId::new_v4()));
        variables.insert("notes".into(), Value::String("stable".into()));
        variables.insert("__internal_counter".into(), Value::Int(99));
        variables.insert("t_ms".into(), Value::Float(1000.0));
        variables.insert("tick".into(), Value::Int(10));
        variables.insert("__clock_time".into(), Value::Float(1.0));
        variables.insert("history_list".into(), Value::List(vec![Value::Int(1)]));
        variables.insert("null_var".into(), Value::Null);

        let mut subsystem_states = HashMap::new();
        subsystem_states.insert(
            "breaker_1".into(),
            SubsystemState {
                name: "breaker_1".into(),
                kind: "stateMachine",
                current_state: "closed".into(),
                completed: false,
                available_transitions: vec![
                    ("trip".into(), "open".into()),
                    ("manual_open".into(), "open".into()),
                ],
                outputs: vec![],
                sends: vec![],
                incoming_transition_trigger: None,
                deferred_event_count: 0,
                source_element_id: None,
            },
        );

        let mut port_values: HashMap<String, HashMap<String, Value>> = HashMap::new();
        let mut tank_out = HashMap::new();
        tank_out.insert("flowRate".into(), Value::Float(3.14));
        tank_out.insert("pressure".into(), Value::Float(101.3));
        // Non-scalar feature gets dropped by the projection.
        tank_out.insert("tag".into(), Value::String("hot".into()));
        port_values.insert("tank.waterOut".into(), tank_out);
        // A port with NO scalar features should be fully dropped from
        // the projection (nothing meaningful to render).
        let mut empty_port = HashMap::new();
        empty_port.insert("label".into(), Value::String("status".into()));
        port_values.insert("pump.status".into(), empty_port);

        let mut derivatives = HashMap::new();
        derivatives.insert("T_busbar".into(), 0.125);
        derivatives.insert("charge".into(), -0.02);

        ExecutionSnapshot {
            tick: 5,
            time_ms: 500.0,
            subsystem_states,
            variables: std::sync::Arc::new(variables),
            messages: vec![],
            constraint_results: vec![ConstraintEvalResult {
                name: "within_limits".into(),
                verdict: VerdictKind::Pass,
                expression: Some("T_busbar < 400".into()),
                operands: {
                    let mut m = HashMap::new();
                    m.insert("T_busbar".into(), 320.5);
                    m
                },
                element_id: None,
            }],
            assertion_checkpoints: vec![],
            guard_diagnoses: vec![],
            causation_links: vec![],
            completed: false,
            port_values,
            derivatives,
            resolved_refs: HashMap::new(),
            flow_drop_warnings: Vec::new(),
            value_units: Default::default(),
            step_size_health: Vec::new(),
        }
    }

    #[test]
    fn qualified_bookkeeping_gate_var_hidden_from_projection_but_kept_in_mirror() {
        // Task #11: `{sm}.__flow_gate` is orchestrator bookkeeping. It must NOT
        // surface in the observable projection (`scalar_vars`, which feeds
        // `sysml.sessions.timeseries_names`), but the value MIRROR must still
        // carry it for debug / `slot_by_name` reads (projection narrows, mirror
        // untouched). `is_internal_var` alone missed it — the `__` is in the leaf
        // segment of the qualified name.
        let base = make_snapshot();
        let mut vars = (*base.variables).clone();
        vars.insert("TripLatch.__flow_gate".into(), Value::Float(1.0));
        let mut snap = base;
        snap.variables = std::sync::Arc::new(vars);

        let n = normalize(&snap);

        // A real observable from make_snapshot still projects.
        assert!(
            n.scalar_vars.contains_key("T_busbar"),
            "a genuine observable must still reach the projection"
        );
        // The qualified gate var is hidden from the observable projection…
        assert!(
            !n.scalar_vars.contains_key("TripLatch.__flow_gate"),
            "qualified bookkeeping `{{sm}}.__flow_gate` must be filtered out of scalar_vars"
        );
        // …as are the bare internals (unchanged behavior).
        assert!(!n.scalar_vars.contains_key("__clock_time"));
        // But the value mirror retains the gate var for debug / slot reads.
        assert!(
            snap.variables.contains_key("TripLatch.__flow_gate"),
            "the value mirror must still carry the gate var — only the projection narrows"
        );
    }

    #[test]
    fn normalize_filters_add_alias_spellings_to_meta_only() {
        // Task #8 (steward ruling 2026-07-07): with a slot store, the observable
        // projection admits ONLY meta spellings; add_alias spellings are
        // dropped; keys that resolve to no slot pass through.
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

        let mut store = SlotStore::new();
        let duty = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::new_v4()),
                Variability::Continuous,
                WriterId::Orchestrator,
                "duty", // canonical
                "duty", // runtime
            ),
            Value::Float(0.0),
        );
        // The qualified `{ode}.duty` observable is add_alias — resolution-only.
        store.add_alias("ProtectionCorePhysicsModel.duty", duty);

        // The MIRROR carries every spelling (set_slot writes all of
        // aliases_of), so the raw snapshot variables map has BOTH the meta and
        // the alias key — exactly the state the filter must narrow.
        let mut variables = HashMap::new();
        variables.insert("duty".to_string(), Value::Float(-0.26));
        variables.insert("ProtectionCorePhysicsModel.duty".to_string(), Value::Float(-0.26));
        // A key that resolves to NO slot (config/context/lambda) passes through.
        variables.insert("config.threshold".to_string(), Value::Float(30.0));

        let snap = ExecutionSnapshot {
            tick: 1,
            time_ms: 100.0,
            subsystem_states: HashMap::new(),
            variables: std::sync::Arc::new(variables),
            messages: vec![],
            constraint_results: vec![],
            assertion_checkpoints: vec![],
            guard_diagnoses: vec![],
            causation_links: vec![],
            completed: false,
            port_values: HashMap::new(),
            derivatives: HashMap::new(),
            resolved_refs: HashMap::new(),
            flow_drop_warnings: Vec::new(),
            value_units: Default::default(),
            step_size_health: Vec::new(),
        };

        // Without a store: no classification, every key passes (back-compat for
        // store-less callers that build `variables` directly).
        let unfiltered = normalize(&snap);
        assert!(unfiltered.scalar_vars.contains_key("duty"));
        assert!(unfiltered.scalar_vars.contains_key("ProtectionCorePhysicsModel.duty"));
        assert!(unfiltered.scalar_vars.contains_key("config.threshold"));

        // With the store: meta-only observable projection.
        let filtered = normalize_with(
            &snap,
            NormalizeOptions {
                slots: Some(&store),
                ..Default::default()
            },
        );
        assert!(
            filtered.scalar_vars.contains_key("duty"),
            "the meta spelling (canonical==runtime) must be projected"
        );
        assert!(
            !filtered.scalar_vars.contains_key("ProtectionCorePhysicsModel.duty"),
            "the add_alias qualified spelling must NOT surface as an observable"
        );
        assert!(
            filtered.scalar_vars.contains_key("config.threshold"),
            "a key that resolves to no slot must pass through unfiltered"
        );
        // The dropped spelling is still resolvable in the store (resolution-only).
        assert_eq!(store.slot_by_name("ProtectionCorePhysicsModel.duty"), Some(duty));
    }

    #[test]
    fn value_to_scalar_covers_numeric_variants() {
        assert_eq!(value_to_scalar(&Value::Bool(true)), Some(1.0));
        assert_eq!(value_to_scalar(&Value::Bool(false)), Some(0.0));
        assert_eq!(value_to_scalar(&Value::Int(7)), Some(7.0));
        assert_eq!(value_to_scalar(&Value::Float(3.14)), Some(3.14));
        assert_eq!(
            value_to_scalar(&Value::Quantity {
                value: 42.0,
                dimension: DimensionVector::default(),
                unit: None,
            }),
            Some(42.0),
        );
        assert_eq!(
            value_to_scalar(&Value::Complex { re: 1.5, im: 2.0 }),
            Some(1.5),
        );
    }

    #[test]
    fn value_to_scalar_rejects_non_numeric() {
        assert!(value_to_scalar(&Value::String("hi".into())).is_none());
        assert!(value_to_scalar(&Value::Enum("e".into())).is_none());
        assert!(value_to_scalar(&Value::Null).is_none());
        assert!(value_to_scalar(&Value::List(vec![])).is_none());
    }

    #[test]
    fn value_to_string_covers_textual_variants() {
        assert_eq!(
            value_to_string(&Value::String("hi".into())),
            Some("hi".into()),
        );
        assert_eq!(
            value_to_string(&Value::Enum("Active".into())),
            Some("Active".into()),
        );
        // Value::Ref is an internal lazy-resolution sentinel and must
        // not project to the user-facing string_vars map.
        assert!(value_to_string(&Value::Ref(ElementId::new_v4())).is_none());
        assert!(value_to_string(&Value::Int(5)).is_none());
    }

    #[test]
    fn normalize_classifies_variables() {
        let snap = make_snapshot();
        let n = normalize(&snap);

        assert_eq!(n.tick, 5);
        assert_eq!(n.time_ms, 500.0);
        assert!(!n.completed);

        assert_eq!(n.scalar_vars.get("T_busbar"), Some(&320.5));
        assert_eq!(n.scalar_vars.get("ticks_elapsed"), Some(&42.0));
        assert_eq!(n.scalar_vars.get("energized"), Some(&1.0));
        assert_eq!(n.scalar_vars.get("current"), Some(&12.3));
        assert_eq!(n.scalar_vars.get("impedance"), Some(&0.5));

        assert_eq!(
            n.string_vars.get("mode").map(|s| s.as_str()),
            Some("normal")
        );
        assert_eq!(
            n.string_vars.get("notes").map(|s| s.as_str()),
            Some("stable"),
        );
        // Value::Ref sentinels must not surface to string_vars — they are
        // internal runtime state, not user-facing values.
        assert!(!n.string_vars.contains_key("source_id"));
        assert!(!n.scalar_vars.contains_key("source_id"));

        assert!(!n.scalar_vars.contains_key("__internal_counter"));
        assert!(!n.scalar_vars.contains_key("__clock_time"));
        assert!(!n.scalar_vars.contains_key("t_ms"));
        assert!(!n.scalar_vars.contains_key("tick"));

        assert!(!n.scalar_vars.contains_key("history_list"));
        assert!(!n.string_vars.contains_key("history_list"));
        assert!(!n.scalar_vars.contains_key("null_var"));
    }

    #[test]
    fn normalize_copies_subsystem_and_constraints() {
        let n = normalize(&make_snapshot());
        let sub = n.subsystems.get("breaker_1").expect("subsystem present");
        assert_eq!(sub.current_state, "closed");
        assert_eq!(sub.kind_label, "stateMachine");
        assert!(!sub.completed);

        // GAP-SM-002: available_transitions forwards from the runtime
        // SubsystemState so the frontend can render an inject rail.
        assert_eq!(
            sub.available_transitions,
            vec![
                ("trip".into(), "open".into()),
                ("manual_open".into(), "open".into()),
            ],
        );

        assert_eq!(n.constraint_results.len(), 1);
        assert_eq!(n.constraint_results[0].name, "within_limits");
        assert_eq!(
            n.constraint_results[0].expression.as_deref(),
            Some("T_busbar < 400"),
        );
        assert_eq!(n.constraint_results[0].verdict, VerdictKind::Pass);
        // GAP-CONSTR-002: operand scalars forward through the projection.
        assert_eq!(
            n.constraint_results[0].operands.get("T_busbar"),
            Some(&320.5),
        );
    }

    #[test]
    fn normalize_forwards_element_ids_when_present() {
        // R2.3 (backend-first cleansing): the projection forwards
        // `source_element_id` from `SubsystemState` and `element_id`
        // from `ConstraintEvalResult` so the frontend can do an
        // id-keyed lookup instead of matching by short name (which
        // collides across nested scopes).
        let mut snap = make_snapshot();
        let sm_id = ElementId::new_v4();
        let constraint_id = ElementId::new_v4();
        if let Some(state) = snap.subsystem_states.get_mut("breaker_1") {
            state.source_element_id = Some(sm_id.clone());
        }
        snap.constraint_results[0].element_id = Some(constraint_id.clone());

        let n = normalize(&snap);
        assert_eq!(
            n.subsystems
                .get("breaker_1")
                .and_then(|s| s.element_id.clone()),
            Some(sm_id),
        );
        assert_eq!(n.constraint_results[0].element_id, Some(constraint_id));
    }

    #[test]
    fn normalize_forwards_ode_derivatives() {
        // GAP-ODE-002: NormalizedSnapshot must surface dy/dt for every
        // ODE state variable at this tick, keyed by the same name the
        // state value uses in scalar_vars.
        let n = normalize(&make_snapshot());
        assert_eq!(n.derivatives.get("T_busbar"), Some(&0.125));
        assert_eq!(n.derivatives.get("charge"), Some(&-0.02));
    }

    #[test]
    fn normalize_with_exclude_applies_to_derivatives() {
        let snap = make_snapshot();
        let mut exclude = HashSet::new();
        exclude.insert("T_busbar".into());
        let n = normalize_with(
            &snap,
            NormalizeOptions {
                exclude_vars: Some(&exclude),
                ..Default::default()
            },
        );
        assert!(!n.derivatives.contains_key("T_busbar"));
        assert!(n.derivatives.contains_key("charge"));
    }

    #[test]
    fn normalize_projects_port_values_scalars_only() {
        // GAP-FLOW-001: NormalizedSnapshot must project live port
        // feature values as f64 keyed by "owner.port" → feature_name.
        // Non-scalar features drop silently; ports with no scalars at
        // all disappear from the outer map.
        let n = normalize(&make_snapshot());
        let tank_out = n
            .port_values
            .get("tank.waterOut")
            .expect("tank.waterOut must project");
        assert_eq!(tank_out.get("flowRate"), Some(&3.14));
        assert_eq!(tank_out.get("pressure"), Some(&101.3));
        // String feature was dropped.
        assert!(!tank_out.contains_key("tag"));
        // Port with only string features disappears.
        assert!(!n.port_values.contains_key("pump.status"));
    }

    #[test]
    fn normalize_overlays_resolved_refs_onto_raw_ref_bindings() {
        // A `Value::Ref` in `variables` is an internal sentinel that
        // the runtime uses for lazy feature resolution. When the
        // orchestrator manages to resolve it at capture time, the
        // resolved value goes into `resolved_refs` and MUST surface
        // in the projection. Otherwise UI consumers (tree, sparklines,
        // detail panel) lose every expression-resolvable attribute.
        let mut snap = make_snapshot();
        // `source_id` is bound to `Value::Ref` in make_snapshot — pair
        // it with a resolved value and verify it projects.
        let mut resolved = HashMap::new();
        resolved.insert("source_id".into(), Value::Float(42.5));
        // Also seed a resolved string for an "enumish" attribute.
        let mut vars = (*snap.variables).clone();
        vars.insert("mode_ref".into(), Value::Ref(ElementId::new_v4()));
        snap.variables = std::sync::Arc::new(vars);
        resolved.insert("mode_ref".into(), Value::Enum("Armed".into()));
        // And a Ref with no resolution — must still drop to nothing.
        let mut vars = (*snap.variables).clone();
        vars.insert("dangling".into(), Value::Ref(ElementId::new_v4()));
        snap.variables = std::sync::Arc::new(vars);
        snap.resolved_refs = resolved;

        let n = normalize(&snap);
        assert_eq!(n.scalar_vars.get("source_id"), Some(&42.5));
        assert_eq!(
            n.string_vars.get("mode_ref").map(|s| s.as_str()),
            Some("Armed"),
        );
        assert!(!n.scalar_vars.contains_key("dangling"));
        assert!(!n.string_vars.contains_key("dangling"));
    }

    #[test]
    fn normalize_with_exclude_set_drops_named_vars() {
        let snap = make_snapshot();
        let mut exclude = HashSet::new();
        exclude.insert("T_busbar".into());
        exclude.insert("mode".into());
        let n = normalize_with(
            &snap,
            NormalizeOptions {
                exclude_vars: Some(&exclude),
                ..Default::default()
            },
        );
        assert!(!n.scalar_vars.contains_key("T_busbar"));
        assert!(!n.string_vars.contains_key("mode"));
        assert!(n.scalar_vars.contains_key("current"));
    }
}
