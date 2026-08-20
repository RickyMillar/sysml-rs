use std::collections::{HashMap, HashSet, VecDeque};

use sysml_core::element_ordering::{primary_span, sort_elements_by_source_order};
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_span::{Diagnostic, DiagnosticTier, Span};

use super::parse_action;
use crate::TransitionActionIR;

/// Diagnose state-machine health issues across all state definitions in a graph.
///
/// This pass is intended for editor diagnostics and preflight checks before
/// interactive simulation.
pub fn state_machine_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let mut defs: Vec<&Element> = graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .collect();
    sort_elements_by_source_order(&mut defs);

    let config_targets = configuration_attribute_targets(graph);

    let mut diagnostics = Vec::new();
    for def in defs {
        diagnostics.extend(analyze_state_machine(graph, def));
        diagnostics.extend(check_configuration_assignments(graph, def, &config_targets));
    }
    diagnostics
}

/// Diagnose health issues for one named state machine.
///
/// Returns `SM007` if no matching state machine exists.
pub fn state_machine_health_diagnostics_for_name(
    graph: &ModelGraph,
    sm_name: &str,
) -> Vec<Diagnostic> {
    let mut defs: Vec<&Element> = graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .collect();
    sort_elements_by_source_order(&mut defs);

    let Some(def) = defs
        .iter()
        .copied()
        .find(|d| d.name.as_deref() == Some(sm_name))
    else {
        let mut diag = Diagnostic::error(format!("state machine '{}' not found", sm_name))
            .with_code("SM007")
            .with_tier(DiagnosticTier::Semantic);
        let available: Vec<String> = defs.iter().filter_map(|d| d.name.clone()).collect();
        if !available.is_empty() {
            diag = diag.with_note(format!(
                "available state machines: {}",
                available.join(", ")
            ));
        }
        return vec![diag];
    };

    let config_targets = configuration_attribute_targets(graph);
    let mut diagnostics = analyze_state_machine(graph, def);
    diagnostics.extend(check_configuration_assignments(graph, def, &config_targets));
    diagnostics
}

fn analyze_state_machine(graph: &ModelGraph, state_def: &Element) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let sm_name = state_def
        .name
        .clone()
        .unwrap_or_else(|| "<anonymous>".to_owned());

    let mut states: Vec<&Element> = graph
        .children_of(&state_def.id)
        .filter(|e| e.kind == ElementKind::StateUsage)
        .collect();
    sort_elements_by_source_order(&mut states);

    if states.is_empty() {
        diagnostics.push(
            Diagnostic::error(format!("state machine '{}' has no states", sm_name))
                .with_code("SM001")
                .with_tier(DiagnosticTier::Semantic)
                .with_span(primary_span(state_def)),
        );
        return diagnostics;
    }

    let state_ids: HashSet<ElementId> = states.iter().map(|s| s.id.clone()).collect();
    let mut initial_states: Vec<&Element> = states
        .iter()
        .copied()
        .filter(|s| s.get_prop("initial").and_then(|v| v.as_bool()) == Some(true))
        .collect();
    sort_elements_by_source_order(&mut initial_states);

    // Fallback: detect initial state from `entry; then <state>;` pattern.
    // This creates a SuccessionAsUsage whose unresolved target names a child state.
    if initial_states.is_empty() {
        for elem in graph.children_of(&state_def.id) {
            if elem.kind == ElementKind::SuccessionAsUsage {
                if let Some(target_name) = elem
                    .get_prop("target")
                    .and_then(|v| v.as_str())
                    .or_else(|| elem.get_prop("unresolved_target").and_then(|v| v.as_str()))
                {
                    if let Some(target_state) = states
                        .iter()
                        .find(|s| s.name.as_deref() == Some(target_name))
                    {
                        initial_states.push(target_state);
                        break;
                    }
                }
            }
        }
    }

    if initial_states.is_empty() {
        diagnostics.push(
            Diagnostic::warning(format!(
                "state machine '{}' has no explicit initial state",
                sm_name
            ))
            .with_code("SM002")
            .with_tier(DiagnosticTier::Semantic)
            .with_span(name_span_or_primary(state_def))
            .with_note("mark a state as initial or add an entry transition"),
        );
    }

    let initial_state = initial_states
        .first()
        .copied()
        .or_else(|| states.first().copied());
    let initial_name = initial_state
        .and_then(|s| s.name.clone())
        .unwrap_or_else(|| "<unknown>".to_owned());
    let initial_id = initial_state.map(|s| s.id.clone());

    let mut transitions_in_machine = Vec::new();
    for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
        let src_in = state_ids.contains(&rel.source);
        let tgt_in = state_ids.contains(&rel.target);
        if src_in && tgt_in {
            transitions_in_machine.push(rel);
            continue;
        }
        if src_in || tgt_in {
            // Skip composite state internal transitions: if one endpoint is a
            // direct child state and the other is a descendant of that child
            // (i.e. a nested sub-state), this is a valid composite state
            // entry/exit transition, not an error.
            let src_is_descendant = src_in
                && state_ids
                    .iter()
                    .any(|sid| graph.is_descendant_of(&rel.target, sid));
            let tgt_is_descendant = tgt_in
                && state_ids
                    .iter()
                    .any(|sid| graph.is_descendant_of(&rel.source, sid));
            if src_is_descendant || tgt_is_descendant {
                // Valid composite state transition — include in machine transitions
                transitions_in_machine.push(rel);
                continue;
            }
            let source_label = graph
                .get_element(&rel.source)
                .and_then(|e| e.name.clone())
                .unwrap_or_else(|| rel.source.to_string());
            let target_label = graph
                .get_element(&rel.target)
                .and_then(|e| e.name.clone())
                .unwrap_or_else(|| rel.target.to_string());
            let transition_span = transition_span_for_relationship(graph, state_def, rel)
                .unwrap_or_else(|| primary_span(state_def));
            let mut diag = Diagnostic::error(format!(
                "transition source '{}' is not a state inside '{}'",
                source_label, sm_name
            ))
                .with_code("SM006")
                .with_tier(DiagnosticTier::Semantic)
                .with_span(transition_span)
                .with_note(format!(
                    "the transition {} -> {} has an endpoint that resolved to the parent definition or an element outside this state machine",
                    source_label, target_label
                ))
                .with_note("check that `first <state>` and `then <state>` match names of `state` declarations directly inside this `state def`");
            if let Some(source_elem) = graph.get_element(&rel.source) {
                diag = diag.with_related(
                    name_span_or_primary(source_elem),
                    format!("source endpoint resolves to '{}'", source_label),
                );
            }
            if let Some(target_elem) = graph.get_element(&rel.target) {
                diag = diag.with_related(
                    name_span_or_primary(target_elem),
                    format!("target endpoint resolves to '{}'", target_label),
                );
            }
            diagnostics.push(diag);
        }
    }

    if transitions_in_machine.is_empty() {
        let mut diag = Diagnostic::warning(format!(
            "state machine '{}' has no valid transitions",
            sm_name
        ))
        .with_code("SM003")
        .with_tier(DiagnosticTier::Semantic)
        .with_span(name_span_or_primary(state_def))
        .with_note("none of the `first ... then ...` declarations resolved between child states");

        // Distribute context across actual state locations for better editor UX.
        for state in &states {
            let state_name = state.name.as_deref().unwrap_or("<anonymous>");
            diag = diag.with_related(
                name_span_or_primary(state),
                format!("child state '{}' is declared here", state_name),
            );
        }

        // Point to transition declarations that were found inside the state machine.
        let transition_decls = transition_declarations_for_machine(graph, state_def);
        if transition_decls.is_empty() {
            diag = diag.with_note(
                "add at least one `transition ... first <state> ... then <state>;` declaration",
            );
        } else {
            for transition in transition_decls.iter().take(8) {
                let transition_name = transition.name.as_deref().unwrap_or("<anonymous>");
                diag = diag.with_related(
                    primary_span(transition),
                    format!("transition '{}' is declared here", transition_name),
                );
            }
            if transition_decls.len() > 8 {
                diag = diag.with_note(format!(
                    "{} additional transition declarations omitted",
                    transition_decls.len() - 8
                ));
            }
        }

        if diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("SM006"))
        {
            diag = diag.with_note("fix SM006 endpoint diagnostics above first");
        } else {
            diag = diag.with_note(
                "ensure each transition endpoint name matches a `state` name in this machine",
            );
        }
        diagnostics.push(diag);
        return diagnostics;
    }

    let mut adjacency: HashMap<ElementId, Vec<ElementId>> = HashMap::new();
    for rel in &transitions_in_machine {
        adjacency
            .entry(rel.source.clone())
            .or_default()
            .push(rel.target.clone());
    }

    if let Some(initial_id) = initial_id {
        let reachable = compute_reachable(&adjacency, &initial_id);
        for state in &states {
            if state.id == initial_id {
                continue;
            }
            if !reachable.contains(&state.id) {
                let state_name = state.name.clone().unwrap_or_else(|| state.id.to_string());
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "state '{}' in '{}' is unreachable from initial state '{}'",
                        state_name, sm_name, initial_name
                    ))
                    .with_code("SM004")
                    .with_tier(DiagnosticTier::Semantic)
                    .with_span(primary_span(state))
                    .with_note(format!(
                        "add a transition targeting this state, e.g. `transition t first someState then {};`",
                        state_name
                    ))
                    .with_note("if SM006 errors exist, fixing those may resolve this"),
                );
            }
        }
    }

    for state in &states {
        let out_degree = adjacency.get(&state.id).map_or(0, |targets| targets.len());
        let is_final = state.get_prop("final").and_then(|v| v.as_bool()) == Some(true);
        let inferred_final =
            state.get_prop("final_inferred").and_then(|v| v.as_bool()) == Some(true);
        let explicitly_final = is_final && !inferred_final;
        if out_degree == 0 && !explicitly_final {
            let state_name = state.name.clone().unwrap_or_else(|| state.id.to_string());
            // Info, not warning: terminal states (trip latches, lockouts)
            // are legal and common, and the textual notation has no way to
            // mark a state explicitly final — elaboration only infers it
            // (final_inferred), which this check deliberately ignores.
            // Unreachable states are the real defect and stay SM004.
            diagnostics.push(
                Diagnostic::info(format!(
                    "state '{}' in state machine '{}' has no outgoing transitions",
                    state_name, sm_name
                ))
                .with_code("SM005")
                .with_tier(DiagnosticTier::Semantic)
                .with_span(primary_span(state))
                .with_note("if this is not a terminal state, add an outgoing transition"),
            );
        }
    }

    diagnostics
}

// === VR001 — variability lint (RSC-1.4 / audit gap G10) ===
//
// Attributes declared with a literal default on a part def/usage are
// configuration values (Modelica `parameter` variability): they are fixed for
// the duration of a run. A state-machine action or action-body assignment
// writing one is almost certainly a modelling bug — runtime state belongs on
// a dedicated (non-defaulted) attribute.
//
// Tier-1 heuristic, deliberately conservative (zero false-positive budget):
// a bare assignment target only fires when EVERY value-feature of that name
// in the graph is a part-owned defaulted attribute. SM-local variables,
// action-local variables, in/out parameters, ODE states (ToolVariable
// metadata) and calc-written features all suppress the name entirely.

/// True when `element` is a configuration-like attribute: an `AttributeUsage`
/// with a literal default value, owned directly by a part def/usage, that is
/// not a parameter and not written by ODE/metadata machinery.
fn is_configuration_attribute(graph: &ModelGraph, element: &Element) -> bool {
    if element.kind != ElementKind::AttributeUsage {
        return false;
    }
    // (a) literal default value. The parser stores `attribute x default 5` /
    // `attribute x = 5` as a typed `value` prop plus `isDefault = true`
    // (see sysml-parser-trait element_builder.rs); non-literal defaults get
    // an expression subtree instead of a `value` prop and never qualify.
    if element.get_prop("isDefault").and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    if !matches!(
        element.get_prop("value"),
        Some(Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_))
    ) {
        return false;
    }
    // Not an in/out/inout parameter.
    if element.get_prop("direction").is_some() {
        return false;
    }
    // Explicitly mutable / computed features are not configuration.
    if element.get_prop("isVariable").and_then(|v| v.as_bool()) == Some(true)
        || element.get_prop("isDerived").and_then(|v| v.as_bool()) == Some(true)
    {
        return false;
    }
    // (b) owned directly by a part def/usage — NOT a local variable of a
    // state machine, action, or calc.
    let Some(owner) = element.owner.as_ref().and_then(|id| graph.get_element(id)) else {
        return false;
    };
    if !matches!(
        owner.kind,
        ElementKind::PartDefinition | ElementKind::PartUsage
    ) {
        return false;
    }
    // (c) not ODE/metadata-driven (e.g. a `@ToolVariable` state variable).
    if graph
        .children_of(&element.id)
        .any(|c| c.kind == ElementKind::MetadataUsage)
    {
        return false;
    }
    true
}

/// Collect assignment-target names that resolve *unambiguously* to
/// configuration-like attributes (see [`is_configuration_attribute`]).
///
/// A name is excluded entirely when any same-named value feature exists that
/// is not itself configuration-like (local variables, parameters, …) or when
/// the name is owned/produced by a calculation — assignments to those names
/// are normal runtime behaviour and must stay silent.
pub(crate) fn configuration_attribute_targets(graph: &ModelGraph) -> HashMap<String, ElementId> {
    // Names owned or produced by calculations (calc results and calc
    // parameters) are runtime-written by design — never configuration.
    let mut calc_written: HashSet<&str> = HashSet::new();
    for calc in graph
        .elements_by_kind(&ElementKind::CalculationDefinition)
        .chain(graph.elements_by_kind(&ElementKind::CalculationUsage))
    {
        if let Some(name) = calc.name.as_deref() {
            calc_written.insert(name);
        }
        for child in graph.children_of(&calc.id) {
            if let Some(name) = child.name.as_deref() {
                calc_written.insert(name);
            }
        }
    }

    let mut candidates: HashMap<String, Vec<&Element>> = HashMap::new();
    let mut vetoed: HashSet<&str> = HashSet::new();

    for element in graph.elements.values() {
        let Some(name) = element.name.as_deref() else {
            continue;
        };
        if calc_written.contains(name) {
            vetoed.insert(name);
            continue;
        }
        if is_configuration_attribute(graph, element) {
            candidates.entry(name.to_owned()).or_default().push(element);
        } else if matches!(
            element.kind,
            ElementKind::AttributeUsage
                | ElementKind::ReferenceUsage
                | ElementKind::ItemUsage
                | ElementKind::Feature
        ) {
            // A same-named non-configuration value feature anywhere in the
            // graph makes the bare assignment name ambiguous — never fire.
            vetoed.insert(name);
        }
    }

    candidates
        .into_iter()
        .filter(|(name, _)| !vetoed.contains(name.as_str()))
        .filter_map(|(name, mut attrs)| {
            sort_elements_by_source_order(&mut attrs);
            attrs.first().map(|attr| (name, attr.id.clone()))
        })
        .collect()
}

/// Build the VR001 diagnostic for one assignment site, or `None` when the
/// target does not resolve to a configuration-like attribute.
pub(crate) fn configuration_assignment_diagnostic(
    graph: &ModelGraph,
    config_targets: &HashMap<String, ElementId>,
    target_name: &str,
    span: Span,
    context: &str,
) -> Option<Diagnostic> {
    // Dotted feature chains are out of tier-1 scope — bare names only.
    if target_name.contains('.') {
        return None;
    }
    let attr_id = config_targets.get(target_name)?;
    let mut diag = Diagnostic::warning(format!(
        "assignment to configuration attribute '{}' at runtime — attributes with defaults on parts are configuration; runtime state belongs on a dedicated attribute",
        target_name
    ))
    .with_code("VR001")
    .with_tier(DiagnosticTier::Semantic)
    .with_span(span)
    .with_note(format!("the {} writes '{}'", context, target_name))
    .with_note(
        "model mutable runtime state as a separate attribute (without a default) and keep the defaulted attribute as an immutable configuration value",
    );
    if let Some(attr) = graph.get_element(attr_id) {
        diag = diag.with_related(
            name_span_or_primary(attr),
            format!(
                "configuration attribute '{}' is declared here with a default",
                target_name
            ),
        );
    }
    Some(diag)
}

/// Extract assignment-target variable names from an action string
/// (entry/do/exit props, transition `action`/`effect` props).
fn assignment_targets_in_action_text(text: &str) -> Vec<String> {
    match parse_action(text) {
        TransitionActionIR::Structured { assignments, .. } => {
            assignments.into_iter().map(|a| a.variable).collect()
        }
        TransitionActionIR::Simple(_) => Vec::new(),
    }
}

/// Assignment target of a structured `AssignmentActionUsage` element.
fn assignment_target_of(assign: &Element) -> Option<String> {
    assign
        .get_prop("targetFeature")
        .and_then(|v| v.as_str())
        .or_else(|| assign.get_prop("target").and_then(|v| v.as_str()))
        .or(assign.name.as_deref())
        .map(str::to_owned)
}

/// VR001: warn when a state-machine action (entry/do/exit or transition
/// effect) assigns to a configuration-like attribute.
fn check_configuration_assignments(
    graph: &ModelGraph,
    state_def: &Element,
    config_targets: &HashMap<String, ElementId>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config_targets.is_empty() {
        return diagnostics;
    }
    let sm_name = state_def
        .name
        .clone()
        .unwrap_or_else(|| "<anonymous>".to_owned());

    let mut states: Vec<&Element> = graph
        .children_of(&state_def.id)
        .filter(|e| e.kind == ElementKind::StateUsage)
        .collect();
    sort_elements_by_source_order(&mut states);
    let state_ids: HashSet<ElementId> = states.iter().map(|s| s.id.clone()).collect();

    // Assignment sites: (target name, span, human-readable context).
    let mut sites: Vec<(String, Span, String)> = Vec::new();

    for state in &states {
        let state_name = state.name.as_deref().unwrap_or("<anonymous>");
        // Legacy string-prop subactions (`entry`, `do_action`, `exit`).
        for (prop, label) in [("entry", "entry"), ("do_action", "do"), ("exit", "exit")] {
            if let Some(text) = state.get_prop(prop).and_then(|v| v.as_str()) {
                for var in assignment_targets_in_action_text(text) {
                    sites.push((
                        var,
                        primary_span(state),
                        format!(
                            "{} action of state '{}' in state machine '{}'",
                            label, state_name, sm_name
                        ),
                    ));
                }
            }
        }
        // Structured subactions: ActionUsage tagged with stateSubactionKind,
        // containing AssignmentActionUsage children.
        for sub in graph.children_of(&state.id).filter(|e| {
            e.kind == ElementKind::ActionUsage && e.get_prop("stateSubactionKind").is_some()
        }) {
            let label = sub
                .get_prop("stateSubactionKind")
                .and_then(|v| v.as_str())
                .unwrap_or("entry");
            for assign in graph
                .children_of(&sub.id)
                .filter(|e| e.kind == ElementKind::AssignmentActionUsage)
            {
                if let Some(var) = assignment_target_of(assign) {
                    sites.push((
                        var,
                        primary_span(assign),
                        format!(
                            "{} action of state '{}' in state machine '{}'",
                            label, state_name, sm_name
                        ),
                    ));
                }
            }
        }
    }

    // Transition effects — declared elements first (best spans), then
    // elaborated Transition relationships (`action` / `effect` props). The
    // relationship span resolves back to the declaration, so the (name, span)
    // de-dup below collapses mirrored props into one report.
    for transition in transition_declarations_for_machine(graph, state_def) {
        if let Some(text) = graph.transition_feature_text(&transition.id, "effect") {
            let t_name = transition.name.as_deref().unwrap_or("<anonymous>");
            for var in assignment_targets_in_action_text(&text) {
                sites.push((
                    var,
                    primary_span(transition),
                    format!(
                        "effect of transition '{}' in state machine '{}'",
                        t_name, sm_name
                    ),
                ));
            }
        }
    }
    for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
        if !state_ids.contains(&rel.source) && !state_ids.contains(&rel.target) {
            continue;
        }
        for prop in ["action", "effect"] {
            if let Some(text) = rel.props.get(prop).and_then(|v| v.as_str()) {
                let span = transition_span_for_relationship(graph, state_def, rel)
                    .unwrap_or_else(|| primary_span(state_def));
                for var in assignment_targets_in_action_text(text) {
                    sites.push((
                        var,
                        span.clone(),
                        format!("transition effect in state machine '{}'", sm_name),
                    ));
                }
            }
        }
    }

    let mut seen: HashSet<(String, String, usize, usize)> = HashSet::new();
    for (var, span, context) in sites {
        if !seen.insert((var.clone(), span.file.clone(), span.start, span.end)) {
            continue;
        }
        if let Some(diag) =
            configuration_assignment_diagnostic(graph, config_targets, &var, span, &context)
        {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

/// Return the element's name span if available, otherwise the primary (full) span.
fn name_span_or_primary(element: &Element) -> Span {
    element
        .name_span
        .clone()
        .unwrap_or_else(|| primary_span(element))
}

/// Best-effort span for the TransitionUsage/SuccessionAsUsage that produced `rel`.
///
/// Elaborated Transition relationships do not keep an explicit back-pointer to the
/// source element, so we match by resolved refs first and by endpoint names second.
fn transition_span_for_relationship(
    graph: &ModelGraph,
    state_def: &Element,
    rel: &sysml_core::Relationship,
) -> Option<Span> {
    // Fast path: elaboration records the originating TransitionUsage/SuccessionAsUsage.
    if let Some(origin_id) = rel.props.get("origin_transition").and_then(|v| v.as_ref()) {
        if let Some(origin) = graph.get_element(origin_id) {
            return Some(primary_span(origin));
        }
    }

    let source_name = graph.get_element(&rel.source).and_then(|e| e.name.clone());
    let target_name = graph.get_element(&rel.target).and_then(|e| e.name.clone());

    let mut candidates: Vec<&Element> = graph
        .elements
        .values()
        .filter(|elem| {
            matches!(
                elem.kind,
                ElementKind::TransitionUsage | ElementKind::SuccessionAsUsage
            ) && graph.is_descendant_of(&elem.id, &state_def.id)
        })
        .collect();
    sort_elements_by_source_order(&mut candidates);

    for candidate in candidates {
        let source_ref_matches = candidate
            .get_prop("source")
            .and_then(|v| v.as_ref())
            .map(|id| id == &rel.source)
            .unwrap_or(false);
        let target_ref_matches = candidate
            .get_prop("target")
            .and_then(|v| v.as_ref())
            .map(|id| id == &rel.target)
            .unwrap_or(false);
        if source_ref_matches && target_ref_matches {
            return Some(primary_span(candidate));
        }

        let source_prop = candidate
            .get_prop("source")
            .and_then(|v| v.as_str())
            .or_else(|| {
                candidate
                    .get_prop("unresolved_source")
                    .and_then(|v| v.as_str())
            });
        let target_prop = candidate
            .get_prop("target")
            .and_then(|v| v.as_str())
            .or_else(|| {
                candidate
                    .get_prop("unresolved_target")
                    .and_then(|v| v.as_str())
            });
        let source_name_matches = source_prop
            .zip(source_name.as_deref())
            .map(|(left, right)| left == right)
            .unwrap_or(false);
        let target_name_matches = target_prop
            .zip(target_name.as_deref())
            .map(|(left, right)| left == right)
            .unwrap_or(false);
        if source_name_matches && target_name_matches {
            return Some(primary_span(candidate));
        }
    }

    None
}

/// Collect transition declaration elements inside a specific state machine.
fn transition_declarations_for_machine<'a>(
    graph: &'a ModelGraph,
    state_def: &'a Element,
) -> Vec<&'a Element> {
    let mut transitions: Vec<&Element> = graph
        .children_of(&state_def.id)
        .filter(|elem| {
            matches!(
                elem.kind,
                ElementKind::TransitionUsage | ElementKind::SuccessionAsUsage
            )
        })
        .collect();
    sort_elements_by_source_order(&mut transitions);
    transitions
}

fn compute_reachable(
    adjacency: &HashMap<ElementId, Vec<ElementId>>,
    start: &ElementId,
) -> HashSet<ElementId> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(start.clone());
    queue.push_back(start.clone());

    while let Some(current) = queue.pop_front() {
        if let Some(next_states) = adjacency.get(&current) {
            for next in next_states {
                if visited.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
    }

    visited
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, Relationship, Value};

    #[test]
    fn reports_unrunnable_machine_without_transitions() {
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Toggle")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let sm_id = graph.add_element(sm);

        let off = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Off")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(off);

        let on = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("On")
            .with_owner(sm_id.clone())
            .with_span(Span::new("file:///test.sysml", 4, 5));
        graph.add_element(on);

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("SM003")));
    }

    #[test]
    fn reports_dead_end_when_final_is_inferred() {
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Door")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let sm_id = graph.add_element(sm);

        let closed = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Closed")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let closed_id = graph.add_element(closed);

        let jammed = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Jammed")
            .with_owner(sm_id.clone())
            .with_prop("final", true)
            .with_prop("final_inferred", true)
            .with_span(Span::new("file:///test.sysml", 4, 5));
        graph.add_element(jammed);

        // Keep the machine runnable so SM003 does not short-circuit.
        let loop_rel =
            Relationship::new(RelationshipKind::Transition, closed_id.clone(), closed_id);
        graph.add_relationship(loop_rel);

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("SM005") && d.message.contains("Jammed")));
    }

    #[test]
    fn does_not_report_dead_end_for_explicit_final() {
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Door")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let sm_id = graph.add_element(sm);

        let closed = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Closed")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let closed_id = graph.add_element(closed);

        let done = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Done")
            .with_owner(sm_id.clone())
            .with_prop("final", true)
            .with_span(Span::new("file:///test.sysml", 4, 5));
        let done_id = graph.add_element(done);

        let to_done = Relationship::new(RelationshipKind::Transition, closed_id, done_id);
        graph.add_relationship(to_done);

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(!diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("SM005") && d.message.contains("Done")));
    }

    #[test]
    fn detects_initial_state_from_succession_target() {
        // Pattern: `entry; then idle;` creates a SuccessionAsUsage with target="idle"
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("CoffeeMachine")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let sm_id = graph.add_element(sm);

        let idle = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone())
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let idle_id = graph.add_element(idle);

        let brewing = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("brewing")
            .with_owner(sm_id.clone())
            .with_span(Span::new("file:///test.sysml", 4, 5));
        let brewing_id = graph.add_element(brewing);

        // SuccessionAsUsage representing `entry; then idle;`
        let succ = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(sm_id.clone())
            .with_prop("target", "idle");
        graph.add_element(succ);

        // Transition idle -> brewing
        let t = Relationship::new(
            RelationshipKind::Transition,
            idle_id.clone(),
            brewing_id.clone(),
        );
        graph.add_relationship(t);
        // Transition brewing -> idle (loop)
        let t2 = Relationship::new(RelationshipKind::Transition, brewing_id, idle_id);
        graph.add_relationship(t2);

        let diagnostics = state_machine_health_diagnostics(&graph);
        // Should NOT have SM002 (no initial state) because succession targets idle
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("SM002")),
            "should detect initial state from succession target, got: {:?}",
            diagnostics
        );
        // Should NOT have SM004 (unreachable) since idle is initial
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("SM004")),
            "no states should be unreachable, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn sm_diagnostics_carry_semantic_tier() {
        // SM001 is the simplest reproducible scenario: a StateDefinition with no
        // child StateUsage elements yields exactly one SM001 emission. We assert
        // the emission carries DiagnosticTier::Semantic per P-RA2 Slice 2.
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Empty")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        graph.add_element(sm);

        let diagnostics = state_machine_health_diagnostics(&graph);
        let sm001 = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("SM001"))
            .expect("expected SM001 emission for stateless state machine");
        assert_eq!(
            sm001.tier,
            DiagnosticTier::Semantic,
            "SM* diagnostics must be tagged Semantic"
        );
    }

    #[test]
    fn sm006_prefers_origin_transition_span_when_available() {
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Machine")
            .with_span(Span::new("file:///test.sysml", 0, 100));
        let sm_id = graph.add_element(sm);

        let a = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("A")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///test.sysml", 10, 20));
        let a_id = graph.add_element(a);

        let b = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("B")
            .with_owner(sm_id.clone())
            .with_span(Span::new("file:///test.sysml", 21, 30));
        graph.add_element(b);

        // External state (outside machine) to trigger SM006.
        let external = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("External")
            .with_span(Span::new("file:///test.sysml", 200, 210));
        let external_id = graph.add_element(external);

        // Origin transition usage with a distinct span.
        let transition_usage = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_name("bad_t")
            .with_owner(sm_id)
            .with_span(Span::new("file:///test.sysml", 50, 70));
        let transition_usage_id = graph.add_element(transition_usage);

        let bad_rel = Relationship::new(RelationshipKind::Transition, a_id, external_id)
            .with_prop("origin_transition", Value::Ref(transition_usage_id));
        graph.add_relationship(bad_rel);

        let diagnostics = state_machine_health_diagnostics(&graph);
        let sm006 = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("SM006"))
            .expect("expected SM006 diagnostic");
        let span = sm006.span.as_ref().expect("SM006 should include a span");
        assert_eq!(
            (span.start, span.end),
            (50, 70),
            "SM006 should point to the origin transition usage span"
        );
    }

    // === VR001 — variability lint ===

    /// Part def with a configuration attribute: literal default, no direction.
    fn add_part_with_config_attr(
        graph: &mut ModelGraph,
        part_name: &str,
        attr_name: &str,
        value: Value,
    ) -> ElementId {
        let part = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name(part_name)
            .with_span(Span::new("file:///test.sysml", 100, 110));
        let part_id = graph.add_element(part);
        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name(attr_name)
            .with_owner(part_id.clone())
            .with_prop("isDefault", true)
            .with_prop("value", value)
            .with_span(Span::new("file:///test.sysml", 111, 120));
        graph.add_element(attr)
    }

    /// Minimal runnable two-state machine; returns (sm_id, first_state_id, second_state_id).
    fn add_two_state_machine(
        graph: &mut ModelGraph,
        name: &str,
    ) -> (ElementId, ElementId, ElementId) {
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name(name)
            .with_span(Span::new("file:///test.sysml", 0, 99));
        let sm_id = graph.add_element(sm);
        let a = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("a")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///test.sysml", 10, 20));
        let a_id = graph.add_element(a);
        let b = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("b")
            .with_owner(sm_id.clone())
            .with_span(Span::new("file:///test.sysml", 21, 30));
        let b_id = graph.add_element(b);
        graph.add_relationship(Relationship::new(
            RelationshipKind::Transition,
            a_id.clone(),
            b_id.clone(),
        ));
        (sm_id, a_id, b_id)
    }

    fn vr001s(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
        diagnostics
            .iter()
            .filter(|d| d.code.as_deref() == Some("VR001"))
            .collect()
    }

    #[test]
    fn vr001_fires_on_transition_effect_writing_part_config_attribute() {
        let mut graph = ModelGraph::new();
        add_part_with_config_attr(&mut graph, "EWMAFilter", "alpha", Value::Float(0.2));
        let (sm_id, a_id, b_id) = add_two_state_machine(&mut graph, "Machine");
        let _ = sm_id;
        graph.add_relationship(
            Relationship::new(RelationshipKind::Transition, b_id, a_id)
                .with_prop("action", "alpha = 0.5"),
        );

        let diagnostics = state_machine_health_diagnostics(&graph);
        let hits = vr001s(&diagnostics);
        assert_eq!(hits.len(), 1, "expected one VR001, got: {:?}", diagnostics);
        assert!(hits[0].message.contains("'alpha'"));
        assert_eq!(hits[0].severity, sysml_span::Severity::Warning);
        assert_eq!(hits[0].tier, DiagnosticTier::Semantic);
    }

    #[test]
    fn vr001_fires_on_entry_string_prop_assignment() {
        let mut graph = ModelGraph::new();
        add_part_with_config_attr(&mut graph, "Breaker", "ratedCurrent", Value::Float(32.0));
        let (_, a_id, _) = add_two_state_machine(&mut graph, "Machine");
        if let Some(state) = graph.elements.get_mut(&a_id) {
            state.set_prop("entry", "ratedCurrent = 63");
        }

        let diagnostics = state_machine_health_diagnostics(&graph);
        let hits = vr001s(&diagnostics);
        assert_eq!(hits.len(), 1, "expected one VR001, got: {:?}", diagnostics);
        assert!(hits[0].message.contains("'ratedCurrent'"));
    }

    #[test]
    fn vr001_fires_on_structured_subaction_assignment() {
        let mut graph = ModelGraph::new();
        add_part_with_config_attr(&mut graph, "Sensor", "threshold", Value::Float(365.15));
        let (_, a_id, _) = add_two_state_machine(&mut graph, "Machine");

        let entry = Element::new_with_kind(ElementKind::ActionUsage)
            .with_owner(a_id)
            .with_prop("stateSubactionKind", "entry")
            .with_span(Span::new("file:///test.sysml", 12, 18));
        let entry_id = graph.add_element(entry);
        let assign = Element::new_with_kind(ElementKind::AssignmentActionUsage)
            .with_owner(entry_id)
            .with_prop("targetFeature", "threshold")
            .with_prop("value", Value::Float(1.0))
            .with_span(Span::new("file:///test.sysml", 13, 17));
        graph.add_element(assign);

        let diagnostics = state_machine_health_diagnostics(&graph);
        let hits = vr001s(&diagnostics);
        assert_eq!(hits.len(), 1, "expected one VR001, got: {:?}", diagnostics);
        let span = hits[0].span.as_ref().expect("VR001 should carry a span");
        assert_eq!(
            (span.start, span.end),
            (13, 17),
            "VR001 should point at the assignment element"
        );
    }

    /// Multi-circuit-fixture / coffee-machine pattern: entry actions writing
    /// SM-local variables (counters, flags declared on the state def) are
    /// normal and must stay silent.
    #[test]
    fn vr001_silent_for_sm_local_variable_assignment() {
        let mut graph = ModelGraph::new();
        let (sm_id, a_id, _) = add_two_state_machine(&mut graph, "Machine");
        // SM-local variable with an initializer — same prop shape the parser
        // emits for `attribute t = 0;` inside a state def.
        let local = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("t")
            .with_owner(sm_id)
            .with_prop("isDefault", true)
            .with_prop("value", Value::Float(0.0));
        graph.add_element(local);
        if let Some(state) = graph.elements.get_mut(&a_id) {
            state.set_prop("entry", "t += 10");
        }

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(
            vr001s(&diagnostics).is_empty(),
            "SM-local variable assignment must not fire VR001, got: {:?}",
            diagnostics
        );
    }

    /// ThermalShutdown-style `in attribute threshold ... default 365.15` on a
    /// state def is a parameter, not part-owned configuration — silent, even
    /// when assigned (and even though it carries a default).
    #[test]
    fn vr001_silent_for_defaulted_in_parameter() {
        let mut graph = ModelGraph::new();
        let (sm_id, a_id, _) = add_two_state_machine(&mut graph, "ThermalShutdownStates");
        let param = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("shutdownThreshold")
            .with_owner(sm_id)
            .with_prop("direction", "in")
            .with_prop("isDefault", true)
            .with_prop("value", Value::Float(365.15));
        graph.add_element(param);
        if let Some(state) = graph.elements.get_mut(&a_id) {
            state.set_prop("entry", "shutdownThreshold = 1");
        }

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(
            vr001s(&diagnostics).is_empty(),
            "defaulted in-parameter must not fire VR001, got: {:?}",
            diagnostics
        );
    }

    /// A same-named local variable anywhere shadows the configuration
    /// attribute — ambiguous, so never fire.
    #[test]
    fn vr001_silent_when_name_is_shadowed_by_local_feature() {
        let mut graph = ModelGraph::new();
        add_part_with_config_attr(&mut graph, "Pump", "count", Value::Int(4));
        let (sm_id, a_id, _) = add_two_state_machine(&mut graph, "Machine");
        let local = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("count")
            .with_owner(sm_id);
        graph.add_element(local);
        if let Some(state) = graph.elements.get_mut(&a_id) {
            state.set_prop("entry", "count = 0");
        }

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(
            vr001s(&diagnostics).is_empty(),
            "shadowed name must not fire VR001, got: {:?}",
            diagnostics
        );
    }

    /// ODE state variables (ToolVariable metadata child) are runtime state by
    /// design — silent.
    #[test]
    fn vr001_silent_for_ode_state_attribute() {
        let mut graph = ModelGraph::new();
        let attr_id = add_part_with_config_attr(&mut graph, "Boiler", "temp", Value::Float(298.15));
        let meta = Element::new_with_kind(ElementKind::MetadataUsage).with_owner(attr_id);
        graph.add_element(meta);
        let (_, a_id, _) = add_two_state_machine(&mut graph, "Machine");
        if let Some(state) = graph.elements.get_mut(&a_id) {
            state.set_prop("entry", "temp = 300");
        }

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(
            vr001s(&diagnostics).is_empty(),
            "ODE/metadata-backed attribute must not fire VR001, got: {:?}",
            diagnostics
        );
    }

    /// Names produced by calculations are runtime-written — silent.
    #[test]
    fn vr001_silent_for_calc_result_name() {
        let mut graph = ModelGraph::new();
        add_part_with_config_attr(&mut graph, "Panel", "power", Value::Float(0.0));
        let calc = Element::new_with_kind(ElementKind::CalculationUsage).with_name("power");
        graph.add_element(calc);
        let (_, a_id, _) = add_two_state_machine(&mut graph, "Machine");
        if let Some(state) = graph.elements.get_mut(&a_id) {
            state.set_prop("entry", "power = 5");
        }

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(
            vr001s(&diagnostics).is_empty(),
            "calc-written name must not fire VR001, got: {:?}",
            diagnostics
        );
    }

    /// Multi-circuit-fixture representative: ProtectionRace (in-Booleans, guard
    /// transitions, no assignments) + ThermalShutdown (defaulted in-params on
    /// the state def, EWMAFilter part with defaulted attributes, no
    /// assignments) must produce zero VR001.
    #[test]
    fn vr001_silent_on_multi_circuit_patterns() {
        let mut graph = ModelGraph::new();

        // ProtectionRaceStates: in Booleans, four states, guard transitions.
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("ProtectionRaceStates")
            .with_span(Span::new("file:///race.sysml", 0, 99));
        let sm_id = graph.add_element(sm);
        for flag in ["thermalProtectionTripped", "magneticProtectionTripped", "protectionTripped"] {
            let attr = Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name(flag)
                .with_owner(sm_id.clone())
                .with_prop("direction", "in");
            graph.add_element(attr);
        }
        let armed = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("armed")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///race.sysml", 10, 20));
        let armed_id = graph.add_element(armed);
        let trip = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("breakerMagneticTrip")
            .with_owner(sm_id)
            .with_span(Span::new("file:///race.sysml", 21, 30));
        let trip_id = graph.add_element(trip);
        graph.add_relationship(
            Relationship::new(RelationshipKind::Transition, armed_id, trip_id)
                .with_prop("guard", "magneticProtectionTripped"),
        );

        // ThermalShutdownBehaviour: defaulted in-params + EWMAFilter part.
        let sm2 = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("ThermalShutdownStates")
            .with_span(Span::new("file:///thermal.sysml", 0, 99));
        let sm2_id = graph.add_element(sm2);
        for (name, default) in [("shutdownThreshold", 365.15), ("resetThreshold", 350.15)] {
            let attr = Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name(name)
                .with_owner(sm2_id.clone())
                .with_prop("direction", "in")
                .with_prop("isDefault", true)
                .with_prop("value", Value::Float(default));
            graph.add_element(attr);
        }
        let normal = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("normal")
            .with_owner(sm2_id.clone())
            .with_prop("initial", true)
            .with_span(Span::new("file:///thermal.sysml", 10, 20));
        let normal_id = graph.add_element(normal);
        let overtemp = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("overtemp")
            .with_owner(sm2_id)
            .with_span(Span::new("file:///thermal.sysml", 21, 30));
        let overtemp_id = graph.add_element(overtemp);
        graph.add_relationship(
            Relationship::new(RelationshipKind::Transition, normal_id, overtemp_id)
                .with_prop("guard", "filteredTemperature >= shutdownThreshold"),
        );
        add_part_with_config_attr(&mut graph, "EWMAFilter", "alpha", Value::Float(0.2));
        add_part_with_config_attr(
            &mut graph,
            "EWMAFilter2",
            "filteredValue",
            Value::Float(298.15),
        );

        let diagnostics = state_machine_health_diagnostics(&graph);
        assert!(
            vr001s(&diagnostics).is_empty(),
            "multi-circuit-fixture patterns must not fire VR001, got: {:?}",
            diagnostics
        );
    }

    /// Mirrored `effect` text on the declaration and the elaborated
    /// relationship must report once, not twice.
    #[test]
    fn vr001_deduplicates_declaration_and_relationship_effects() {
        let mut graph = ModelGraph::new();
        add_part_with_config_attr(&mut graph, "Relay", "coilVoltage", Value::Float(12.0));
        let (sm_id, a_id, b_id) = add_two_state_machine(&mut graph, "Machine");

        let transition_usage = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_name("a_to_b")
            .with_owner(sm_id)
            .with_span(Span::new("file:///test.sysml", 40, 60));
        let transition_usage_id = graph.add_element(transition_usage);
        graph.add_transition_feature(
            &transition_usage_id,
            "effect",
            Element::new_with_kind(ElementKind::ActionUsage).with_prop("text", "coilVoltage = 24"),
        );
        graph.add_relationship(
            Relationship::new(RelationshipKind::Transition, a_id, b_id)
                .with_prop("effect", "coilVoltage = 24")
                .with_prop("origin_transition", Value::Ref(transition_usage_id)),
        );

        let diagnostics = state_machine_health_diagnostics(&graph);
        let hits = vr001s(&diagnostics);
        assert_eq!(
            hits.len(),
            1,
            "mirrored effect must report exactly once, got: {:?}",
            diagnostics
        );
    }
}
