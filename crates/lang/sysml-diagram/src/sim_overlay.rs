//! Per-tick **simulation** overlay-delta (Bucket 1.8).
//!
//! A renderer-agnostic, `ElementId`-joined description of how a diagram should be
//! decorated for **one simulation tick**: which model elements are active
//! (highlight), what live scalar reading to badge on an element, and which
//! time-series **channels** exist for plotting. It is *not* pixels and *not*
//! animation timing — the renderer owns presentation (glow, marching-ants,
//! sparkline). The overlay only says *what is true this tick*.
//!
//! ## Why this is NOT on the salsa `ViewModel`
//!
//! `tokens` / `text_map` / `interactions` are each a **pure function of the
//! graph**, so each rides a graph-keyed salsa sidecar. The simulation overlay is
//! **session state** — a function of a live `RuntimeSession`'s latest
//! [`ExecutionSnapshot`] + [`TimeSeriesBuffer`] — and it changes every tick. It
//! is not salsa-cacheable on the graph. So it is built and delivered at the
//! **service layer** as a *separate artifact* via `sysml.diagram.sim_overlay`;
//! there is deliberately **no `overlays` field on `ViewModel`** (it would be
//! permanently `None` on the salsa path — steward ruling 2026-06-25, option A).
//!
//! ## Not to be confused with [`crate::ir::overlays`]
//!
//! `crate::ir::overlays` (the `DiagramOverlay` trait, parametric/requirement
//! projection) is a **structural build-time** overlay — a different concept. This
//! module is the **per-tick simulation** overlay. They never mix.
//!
//! ## The name → `ElementId` join (steward-reviewed)
//!
//! The snapshot is keyed by runtime **names**; the scene by **`ElementId`**.
//! Identity-first, honestly-`Option` where it isn't available:
//! - **Subsystem highlight** joins cleanly via
//!   [`SubsystemState::source_element_id`] — the authoritative id the SM/ODE/action
//!   compiled from.
//! - **Active-substate highlight** resolves `current_state` (a name) against the
//!   *scene's own* id↔name data (scene SM-state nodes are keyed by the state's
//!   `ElementId` and carry the name), scoped under the subsystem node. If a state
//!   name can't be joined, the subsystem-block highlight stands and a `debug!`
//!   diagnostic is emitted — never a silent drop.
//! - **Value badges / channel links** join by unique scene-name match; ambiguous
//!   names are skipped (a name shared by two elements has no single owner).
//!
//! All keys and ids in this artifact derive from an `ElementId` — never a
//! reconstructed name string.

use std::collections::HashMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_core::{ElementId, ModelGraph, Value};
use sysml_runtime::orchestrator::ExecutionSnapshot;
use sysml_runtime::timeseries::TimeSeriesBuffer;

use crate::ir::types::{DiagramChild, DiagramIR, DiagramNode};

/// The per-tick simulation overlay for a diagram scene.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SimOverlay {
    /// Simulation tick this overlay reflects.
    pub tick: u64,
    /// Simulation time in milliseconds.
    pub time_ms: f64,
    /// Per-element visual-state deltas. **Sparse** — only decorated elements.
    ///
    /// Key = [`ElementId::to_string`] (the same string used as the scene node id,
    /// so the renderer joins directly to `DiagramNode::element_id`). **Never a
    /// name string** — every key derives from an `ElementId`.
    pub elements: HashMap<String, ElementOverlay>,
    /// Time-series channels available for plotting this session (the directory
    /// the renderer drives a chart / inline sparkline from).
    pub channels: Vec<OverlayChannel>,
}

/// A single element's visual-state delta for one tick. All fields are `Option`
/// — absent means "no change to report on this facet this tick".
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ElementOverlay {
    /// Highlight state (active SM substate / active subsystem / completed).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub activity: Option<Activity>,
    /// Live scalar reading to badge on this element, if it has one this tick.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub value: Option<OverlayValue>,
}

impl ElementOverlay {
    fn is_empty(&self) -> bool {
        self.activity.is_none() && self.value.is_none()
    }
}

/// Activation state of an element this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Activity {
    /// Currently executing / the active substate.
    Active,
    /// The subsystem has completed.
    Completed,
}

/// A live scalar reading to badge on an element (value + optional unit).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OverlayValue {
    pub value: f64,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub unit: Option<String>,
}

/// One plottable time-series channel.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OverlayChannel {
    /// Channel key — the time-series variable name, identical to the `var`
    /// argument of `sysml.sessions.timeseries{,_decimated}`. This is a
    /// runtime-internal **name**, not an identity (series are named, not
    /// element-keyed) — hence [`Self::element_id`] is `Option`.
    pub channel: String,
    /// The element this channel decorates, when it joins cleanly to exactly one
    /// scene node by name. `None` when unjoinable or ambiguous — the renderer
    /// still plots it in a panel, just unattached. Always an `ElementId`, never
    /// a name string.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub element_id: Option<ElementId>,
    /// Current reading at this tick (saves a separate timeseries fetch for a
    /// "latest value" badge). `None` when the variable has no scalar this tick.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub latest: Option<f64>,
    /// Display unit, when the value carries one.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub unit: Option<String>,
}

/// Build the per-tick [`SimOverlay`] for `scene` from a session's latest
/// `snapshot` and time-series `buffer`.
///
/// Pure and deterministic. The builder lives here (not the service layer)
/// because the join is against the **scene**, and `sysml-diagram` already
/// depends on `sysml-runtime` for the snapshot/buffer types (principle #6,
/// lowest reasonable crate).
///
/// `graph` is the source model the scene was generated from. It is consulted
/// only to resolve a collapsed attribute **compartment row**'s `ElementId` to
/// its simple name (F16-1): those rows are `DiagramChild::Text` projections
/// with no `name` in the scene, so the name→value join needs the model to
/// recover the join key — identity-first (keyed by `ElementId`), never parsed
/// from the row's display text.
pub fn build_sim_overlay(
    scene: &DiagramIR,
    snapshot: &ExecutionSnapshot,
    buffer: &TimeSeriesBuffer,
    graph: &ModelGraph,
) -> SimOverlay {
    let mut elements: HashMap<String, ElementOverlay> = HashMap::new();

    // Scene name → element-id index for unique-match joins (value badges,
    // channel links). A name owned by >1 element has no single owner → excluded.
    let name_index = build_unique_name_index(scene, graph);

    // ── Activity: subsystem-level (clean ElementId) + active substate ──────────
    for state in snapshot.subsystem_states.values() {
        let Some(subsystem_id) = state.source_element_id.as_ref() else {
            // Legacy/test-only subsystems with no source element id — nothing to
            // join to a scene node. Skip (no silent fabricated identity).
            continue;
        };
        let subsystem_key = subsystem_id.to_string();

        if state.completed {
            elements
                .entry(subsystem_key.clone())
                .or_default()
                .activity = Some(Activity::Completed);
            // A completed subsystem has no "active substate" to highlight.
            continue;
        }

        // The subsystem block itself is active.
        elements
            .entry(subsystem_key.clone())
            .or_default()
            .activity
            .get_or_insert(Activity::Active);

        // Resolve the active substate against the subsystem's scene subtree.
        if !state.current_state.is_empty() {
            match find_node(&scene.nodes, &subsystem_key) {
                Some(subsystem_node) => {
                    match find_descendant_by_name(subsystem_node, &state.current_state) {
                        Some(substate_id) => {
                            elements
                                .entry(substate_id.to_owned())
                                .or_default()
                                .activity
                                .get_or_insert(Activity::Active);
                        }
                        None => {
                            // Snapshot names a state the scene subtree doesn't
                            // carry — scene/snapshot disagree on the state graph.
                            // Subsystem-block highlight stands; surface, never drop.
                            tracing::debug!(
                                subsystem = %subsystem_key,
                                state = %state.current_state,
                                "sim_overlay: active substate not found under subsystem node; \
                                 falling back to subsystem-block highlight"
                            );
                        }
                    }
                }
                None => {
                    tracing::debug!(
                        subsystem = %subsystem_key,
                        "sim_overlay: subsystem source element not present in scene"
                    );
                }
            }
        }
    }

    // ── Value badges: unique scene-name match against this tick's scalars ──────
    for (name, id) in &name_index {
        if let Some((value, unit)) = scalar_for(snapshot, name) {
            elements
                .entry(id.clone())
                .or_default()
                .value
                .get_or_insert(OverlayValue { value, unit });
        }
    }

    // Drop any entries that ended up empty (defensive — shouldn't happen).
    elements.retain(|_, e| !e.is_empty());

    // ── Channels: the time-series directory, joined where unambiguous ─────────
    let mut names: Vec<&str> = buffer.series_names();
    names.sort_unstable();
    let channels = names
        .into_iter()
        .map(|name| {
            let (latest, unit) = match scalar_for(snapshot, name) {
                Some((v, u)) => (Some(v), u),
                None => (None, None),
            };
            let element_id = name_index
                .get(name)
                .map(|id| ElementId::from_string(id.as_str()));
            OverlayChannel {
                channel: name.to_owned(),
                element_id,
                latest,
                unit,
            }
        })
        .collect();

    SimOverlay {
        tick: snapshot.tick,
        time_ms: snapshot.time_ms,
        elements,
        channels,
    }
}

/// Extract a scalar `(value, unit)` for `name` from this tick. Prefers the
/// resolved-ref concrete (the orchestrator-resolved value of a `Value::Ref`
/// binding) over the raw variable binding.
fn scalar_for(snapshot: &ExecutionSnapshot, name: &str) -> Option<(f64, Option<String>)> {
    snapshot
        .resolved_refs
        .get(name)
        .and_then(scalar_of)
        .or_else(|| snapshot.variables.get(name).and_then(scalar_of))
}

/// Reduce a [`Value`] to a display scalar, if it is numeric.
fn scalar_of(value: &Value) -> Option<(f64, Option<String>)> {
    match value {
        Value::Float(f) => Some((*f, None)),
        Value::Int(i) => Some((*i as f64, None)),
        Value::Quantity { value, unit, .. } => Some((*value, unit.clone())),
        _ => None,
    }
}

/// Build a `name → element_id` index over every node in the scene, keeping only
/// names owned by **exactly one** element. Used for value-badge and channel
/// joins where an ambiguous name has no single owner.
fn build_unique_name_index(scene: &DiagramIR, graph: &ModelGraph) -> HashMap<String, String> {
    let mut counts: HashMap<String, (String, u32)> = HashMap::new();
    for node in &scene.nodes {
        index_node_names(node, graph, &mut counts);
    }
    counts
        .into_iter()
        .filter_map(|(name, (id, n))| (n == 1).then_some((name, id)))
        .collect()
}

fn index_node_names(
    node: &DiagramNode,
    graph: &ModelGraph,
    counts: &mut HashMap<String, (String, u32)>,
) {
    if !node.name.is_empty() {
        counts
            .entry(node.name.clone())
            .and_modify(|(_, n)| *n += 1)
            .or_insert((node.element_id.clone(), 1));
    }
    for child in &node.children {
        index_child_names(child, graph, counts);
    }
}

fn index_child_names(
    child: &DiagramChild,
    graph: &ModelGraph,
    counts: &mut HashMap<String, (String, u32)>,
) {
    match child {
        DiagramChild::Node(n) => index_node_names(n, graph, counts),
        DiagramChild::Compartment { children, .. } => {
            for c in children {
                index_child_names(c, graph, counts);
            }
        }
        DiagramChild::Island { subtree, .. } => {
            for n in &subtree.nodes {
                index_node_names(n, graph, counts);
            }
        }
        // A collapsed attribute compartment row (F16-1): a `Text` projection
        // carrying the attribute's real `ElementId` but no `name` in the scene.
        // Recover the simple name from the model (identity-first — keyed by the
        // row's `ElementId`, name from the graph, never parsed from `text`) so
        // its live scalar can badge the row. Synthetic rows (constraint/doc/
        // reqId ids) don't resolve in the graph and are skipped.
        DiagramChild::Text { element_id, .. } => {
            if let Some(name) = graph
                .get_element(&ElementId::from_string(element_id))
                .and_then(|e| e.name.as_deref())
                .filter(|n| !n.is_empty())
            {
                counts
                    .entry(name.to_owned())
                    .and_modify(|(_, n)| *n += 1)
                    .or_insert((element_id.clone(), 1));
            }
        }
        // `Edge` is not a node — no name owner.
        DiagramChild::Edge(_) => {}
    }
}

/// Find the node with `element_id == id` anywhere in `nodes` (depth-first).
fn find_node<'a>(nodes: &'a [DiagramNode], id: &str) -> Option<&'a DiagramNode> {
    for node in nodes {
        if node.element_id == id {
            return Some(node);
        }
        if let Some(found) = node.children.iter().find_map(|c| find_child_node(c, id)) {
            return Some(found);
        }
    }
    None
}

fn find_child_node<'a>(child: &'a DiagramChild, id: &str) -> Option<&'a DiagramNode> {
    match child {
        DiagramChild::Node(n) => {
            if n.element_id == id {
                Some(n)
            } else {
                n.children.iter().find_map(|c| find_child_node(c, id))
            }
        }
        DiagramChild::Compartment { children, .. } => {
            children.iter().find_map(|c| find_child_node(c, id))
        }
        DiagramChild::Island { subtree, .. } => find_node(&subtree.nodes, id),
        DiagramChild::Text { .. } | DiagramChild::Edge(_) => None,
    }
}

/// Find the `element_id` of a descendant of `root` whose `name` matches
/// `target_name`. Scoped to `root`'s subtree so identically-named states in
/// other subsystems don't collide.
fn find_descendant_by_name<'a>(root: &'a DiagramNode, target_name: &str) -> Option<&'a str> {
    for child in &root.children {
        if let Some(id) = child_node_by_name(child, target_name) {
            return Some(id);
        }
    }
    None
}

fn child_node_by_name<'a>(child: &'a DiagramChild, target_name: &str) -> Option<&'a str> {
    match child {
        DiagramChild::Node(n) => {
            if n.name == target_name {
                return Some(&n.element_id);
            }
            for c in &n.children {
                if let Some(id) = child_node_by_name(c, target_name) {
                    return Some(id);
                }
            }
            None
        }
        DiagramChild::Compartment { children, .. } => {
            children.iter().find_map(|c| child_node_by_name(c, target_name))
        }
        DiagramChild::Island { subtree, .. } => {
            for n in &subtree.nodes {
                if n.name == target_name {
                    return Some(&n.element_id);
                }
                for c in &n.children {
                    if let Some(id) = child_node_by_name(c, target_name) {
                        return Some(id);
                    }
                }
            }
            None
        }
        DiagramChild::Text { .. } | DiagramChild::Edge(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sysml_runtime::SubsystemState;

    use super::*;
    use crate::ViewType;
    use crate::visual_kind::VisualKind;

    /// Canonical scene-id string for a logical id — the same round-trip every
    /// real scene node makes (`element.id.to_string()`), so it equals
    /// `source_element_id.to_string()` in the join (UUID feature on by default).
    fn eid(id: &str) -> String {
        ElementId::from_string(id).to_string()
    }

    fn node(id: &str, name: &str) -> DiagramNode {
        DiagramNode::new(eid(id), VisualKind::State, name)
    }

    fn subsystem_state(
        name: &str,
        source: &str,
        current_state: &str,
        completed: bool,
    ) -> SubsystemState {
        SubsystemState {
            name: name.to_owned(),
            kind: "stateMachine",
            current_state: current_state.to_owned(),
            completed,
            available_transitions: Vec::new(),
            outputs: Vec::new(),
            sends: Vec::new(),
            incoming_transition_trigger: None,
            deferred_event_count: 0,
            source_element_id: Some(ElementId::from_string(source)),
        }
    }

    fn snapshot(
        tick: u64,
        states: Vec<SubsystemState>,
        variables: HashMap<String, Value>,
    ) -> ExecutionSnapshot {
        let subsystem_states = states.into_iter().map(|s| (s.name.clone(), s)).collect();
        ExecutionSnapshot {
            tick,
            time_ms: tick as f64,
            subsystem_states,
            variables: Arc::new(variables),
            messages: Vec::new(),
            constraint_results: Vec::new(),
            assertion_checkpoints: Vec::new(),
            guard_diagnoses: Vec::new(),
            causation_links: Vec::new(),
            completed: false,
            port_values: HashMap::new(),
            derivatives: HashMap::new(),
            resolved_refs: HashMap::new(),
            flow_drop_warnings: Vec::new(),
            value_units: Arc::new(HashMap::new()),
            step_size_health: Vec::new(),
        }
    }

    /// SM block + the active substate both highlight by their real ElementIds;
    /// a value badge attaches to the uniquely-named attribute node.
    #[test]
    fn highlights_subsystem_active_substate_and_value() {
        let mut sm = node("sm-1", "SM");
        sm.children.push(DiagramChild::Node(node("st-idle", "Idle")));
        sm.children.push(DiagramChild::Node(node("st-run", "Run")));
        let scene = DiagramIR {
            view_type: ViewType::StateTransition,
            nodes: vec![sm, node("attr-1", "temp")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };

        let snap = snapshot(
            7,
            vec![subsystem_state("SM", "sm-1", "Run", false)],
            HashMap::from([("temp".to_owned(), Value::Float(92.3))]),
        );

        let overlay = build_sim_overlay(&scene, &snap, &TimeSeriesBuffer::with_capacity(8), &ModelGraph::new());

        assert_eq!(overlay.tick, 7);
        assert_eq!(overlay.elements[&eid("sm-1")].activity, Some(Activity::Active));
        assert_eq!(overlay.elements[&eid("st-run")].activity, Some(Activity::Active));
        assert!(!overlay.elements.contains_key(&eid("st-idle")));
        assert_eq!(
            overlay.elements[&eid("attr-1")].value,
            Some(OverlayValue { value: 92.3, unit: None })
        );
    }

    /// A completed subsystem reports `Completed` and no substate highlight.
    #[test]
    fn completed_subsystem_has_no_substate_highlight() {
        let mut sm = node("sm-1", "SM");
        sm.children.push(DiagramChild::Node(node("st-run", "Run")));
        let scene = DiagramIR {
            view_type: ViewType::StateTransition,
            nodes: vec![sm],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(3, vec![subsystem_state("SM", "sm-1", "Run", true)], HashMap::new());

        let overlay = build_sim_overlay(&scene, &snap, &TimeSeriesBuffer::with_capacity(8), &ModelGraph::new());
        assert_eq!(overlay.elements[&eid("sm-1")].activity, Some(Activity::Completed));
        assert!(!overlay.elements.contains_key(&eid("st-run")));
    }

    /// Channels list every series; the element link is populated only for a
    /// unique scene-name match, `None` otherwise.
    #[test]
    fn channels_join_uniquely_else_none() {
        let scene = DiagramIR {
            view_type: ViewType::General,
            nodes: vec![node("attr-1", "temp")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(
            1,
            Vec::new(),
            HashMap::from([("temp".to_owned(), Value::Float(50.0))]),
        );
        let mut buf = TimeSeriesBuffer::with_capacity(8);
        buf.append(
            1.0,
            &HashMap::from([("temp".to_owned(), 50.0), ("unmapped".to_owned(), 1.0)]),
        );

        let overlay = build_sim_overlay(&scene, &snap, &buf, &ModelGraph::new());
        let temp = overlay.channels.iter().find(|c| c.channel == "temp").unwrap();
        assert_eq!(temp.element_id, Some(ElementId::from_string("attr-1")));
        assert_eq!(temp.latest, Some(50.0));
        let unmapped = overlay.channels.iter().find(|c| c.channel == "unmapped").unwrap();
        assert_eq!(unmapped.element_id, None);
    }

    /// A `current_state` the scene subtree doesn't carry degrades to
    /// subsystem-block highlight (and logs) — no panic, no fabricated id.
    #[test]
    fn unmatched_substate_degrades_to_subsystem_block() {
        let scene = DiagramIR {
            view_type: ViewType::StateTransition,
            nodes: vec![node("sm-1", "SM")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(2, vec![subsystem_state("SM", "sm-1", "Ghost", false)], HashMap::new());
        let overlay = build_sim_overlay(&scene, &snap, &TimeSeriesBuffer::with_capacity(8), &ModelGraph::new());
        assert_eq!(overlay.elements[&eid("sm-1")].activity, Some(Activity::Active));
        assert_eq!(overlay.elements.len(), 1);
    }

    /// F16-1: a live scalar badges a collapsed attribute **compartment row**
    /// (`DiagramChild::Text`), joined by the row's `ElementId` → the model's
    /// simple name — the design's "name + value + unit" compartment row. The
    /// attribute is only present in the scene as a collapsed `Text` (no `Node`),
    /// so the join can't work off `node.name`; it needs the graph.
    #[test]
    fn value_badges_collapsed_attribute_compartment_row() {
        use sysml_core::{Element, ElementKind};

        // A part node whose `voltage` attribute is collapsed to a Text row that
        // carries the attribute's real ElementId (dual-projection invariant).
        let mut part = node("part-1", "Circuit");
        part.children.push(DiagramChild::Text {
            compartment: crate::visual_kind::CompartmentKind::Attributes,
            text: "voltage : Real".to_owned(),
            element_id: eid("attr-v"),
            source: crate::ir::types::CompartmentItemSource::Owned,
        });
        let scene = DiagramIR {
            view_type: ViewType::General,
            nodes: vec![part],
            edges: Vec::new(),
            buttons: Vec::new(),
        };

        // The model carries the attribute element, keyed by the same id, named
        // `voltage` — the join key the scene's Text row lacks.
        let mut graph = ModelGraph::new();
        graph.add_element(
            Element::new(ElementId::from_string(&eid("attr-v")), ElementKind::AttributeUsage)
                .with_name("voltage"),
        );

        let snap = snapshot(
            4,
            Vec::new(),
            HashMap::from([("voltage".to_owned(), Value::Float(3.3))]),
        );

        let overlay = build_sim_overlay(&scene, &snap, &TimeSeriesBuffer::with_capacity(8), &graph);
        assert_eq!(
            overlay.elements[&eid("attr-v")].value,
            Some(OverlayValue { value: 3.3, unit: None })
        );
    }

    /// F16-1 guard: without the model, the collapsed row cannot be joined (no
    /// name in the scene) — an empty graph yields no badge, never a wrong one.
    #[test]
    fn collapsed_row_without_model_yields_no_badge() {
        let mut part = node("part-1", "Circuit");
        part.children.push(DiagramChild::Text {
            compartment: crate::visual_kind::CompartmentKind::Attributes,
            text: "voltage : Real".to_owned(),
            element_id: eid("attr-v"),
            source: crate::ir::types::CompartmentItemSource::Owned,
        });
        let scene = DiagramIR {
            view_type: ViewType::General,
            nodes: vec![part],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(
            4,
            Vec::new(),
            HashMap::from([("voltage".to_owned(), Value::Float(3.3))]),
        );
        let overlay = build_sim_overlay(&scene, &snap, &TimeSeriesBuffer::with_capacity(8), &ModelGraph::new());
        assert!(!overlay.elements.contains_key(&eid("attr-v")));
    }
}
