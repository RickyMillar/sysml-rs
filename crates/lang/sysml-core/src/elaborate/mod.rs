//! Model elaboration pass for parsed ModelGraphs.
//!
//! The parser produces an **ownership-based** ModelGraph: elements are connected
//! through parent/child relationships. The execution crates (state machines,
//! constraints, actions, flows) expect **property-based** access: explicit
//! properties like `initial`, `entry`, `exit`, and `Relationship::Transition`
//! edges between states.
//!
//! This module bridges the gap with an additive, idempotent elaboration pass
//! that derives implicit relationships and properties from structural parse output.
//!
//! ## Usage
//!
//! ```ignore
//! use sysml_core::elaborate::elaborate;
//!
//! let mut graph = parse("file.sysml");
//! let report = elaborate(&mut graph);
//! // graph now has derived properties and relationships
//! ```
//!
//! ## Design Principles
//!
//! - **Additive**: only adds props/relationships, never removes or overwrites
//! - **Idempotent**: safe to call multiple times with the same result
//! - **In-place**: mutates `&mut ModelGraph` (no cloning)

mod actions;
mod connectors;
mod constraints;
mod dependencies;
mod flows;
mod implicit_generalization;
mod imports;
mod ports;
mod requirements;
mod state_machines;
mod successions;

pub use implicit_generalization::IS_IMPLIED;

use crate::{ElementId, ModelGraph};

/// Resolve an element name to an `ElementId`.
///
/// Shared by all elaboration passes that need to resolve endpoint names
/// to `ElementId`s. Supports both simple names and dotted paths.
///
/// Resolution order:
/// 1. Simple name: search children of `context` (siblings), then global
/// 2. Dotted path (e.g., "sensor.dataOut"): walk the path segment by segment
pub(super) fn resolve_name(
    graph: &ModelGraph,
    context: &Option<ElementId>,
    name: &str,
) -> Option<ElementId> {
    // Handle dotted paths (e.g., "sensor.dataOut")
    if name.contains('.') {
        return resolve_dotted_path(graph, context, name);
    }

    // Qualified names (`Pkg::member`) go through the real resolver — the
    // local/global simple-name fallbacks below never understood `::`, so a
    // qualified satisfy/verify/derivation reference silently minted nothing
    // (design §7.6: printed qualified references must round-trip). No
    // terminal-segment fallback: an unresolvable qualified name stays
    // unresolved.
    if name.contains("::") {
        return graph.resolve_qualified(name);
    }

    // Try local scope first (siblings under the same owner)
    if let Some(owner_id) = context.as_ref() {
        if let Some(found) = graph
            .children_of(owner_id)
            .find(|e| e.name.as_deref() == Some(name))
            .map(|e| e.id.clone())
        {
            return Some(found);
        }
    }
    // Fall back to global name search
    graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some(name))
        .map(|e| e.id.clone())
}

/// Resolve a connector/flow endpoint string (`"participant.port"` or a bare
/// `"participant"`) to a **per-usage** `ElementId` for `LinkEndpoint.element_id`
/// (RSC-3.5a.1 / ledger L19).
///
/// ## Why this is not just `resolve_name`
///
/// Ports are owned by their *definition*, not materialized per usage (see
/// `sysml-runtime` `flows/port.rs` — "the registry uses definition-owner"). So
/// `resolve_name(ctx, "controllerA.currentIn")` and
/// `resolve_name(ctx, "controllerB.currentIn")` both resolve to the **same**
/// definition-level `currentIn` port element when `controllerA`/`controllerB`
/// are two usages of one `ControllerDef`. That shared-def id cannot give the
/// per-instance routing identity the classified exchange plane keys on.
///
/// This helper returns a **per-usage** id by the design-doc §8 L19 rule
/// ("stamp pre-dedup OR disambiguate by owner"):
/// - If the endpoint resolves to a port/feature element that is **usage-owned**
///   (its owner is itself a usage, i.e. per-instance materialization exists),
///   that resolved id is already per-usage — return it.
/// - If the endpoint resolves to a **definition-owned** port (the shared-def
///   case above), the resolved port id is not per-usage. Return the **owner
///   participant usage** id (`controllerA` / `controllerB`) instead — the
///   element that genuinely distinguishes the two endpoints and is reachable
///   from elaboration. (A per-usage *port* element does not exist in the graph;
///   the participant usage is the per-instance discriminator the downstream
///   `RuntimeId.instance_path` is built from.)
///
/// Returns `None` when neither the endpoint nor its participant resolves.
pub(super) fn resolve_endpoint_usage_id(
    graph: &ModelGraph,
    context: &Option<ElementId>,
    endpoint: &str,
) -> Option<ElementId> {
    let resolved = resolve_name(graph, context, endpoint);

    // Is the resolved element usage-owned (per-instance) already? If so it is
    // the genuine per-usage element — prefer it.
    if let Some(id) = &resolved {
        if let Some(elem) = graph.get_element(id) {
            let owner_is_definition = elem
                .owner
                .as_ref()
                .and_then(|o| graph.get_element(o))
                .map(|o| o.kind.is_definition())
                .unwrap_or(false);
            if !owner_is_definition {
                return resolved;
            }
        }
    }

    // Shared-definition (or unresolved-port) case: disambiguate by the owner
    // participant usage — the first path segment (`controllerA` in
    // `controllerA.currentIn`), or the whole string when there is no dot.
    let participant = endpoint.split('.').next().unwrap_or(endpoint);
    resolve_name(graph, context, participant).or(resolved)
}

/// Resolve a dotted path like "sensor.dataOut" to the final segment's ElementId.
///
/// Walks from the context scope: first finds "sensor" among siblings,
/// then finds "dataOut" among sensor's children. If a segment is a typed
/// usage (e.g., `part sensor : Sensor`), also searches the definition's children
/// for the next segment (since ports are owned by the definition, not the usage).
#[allow(clippy::indexing_slicing)] // segments[0] and segments[1..] safe: checked !is_empty() above
fn resolve_dotted_path(
    graph: &ModelGraph,
    context: &Option<ElementId>,
    path: &str,
) -> Option<ElementId> {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return None;
    }

    // Find the first segment in the context scope
    let mut current_id = resolve_name(graph, context, segments[0])?;

    // Walk remaining segments as children (or definition children)
    for &segment in &segments[1..] {
        current_id = find_member_or_typed_member(graph, &current_id, segment)?;
    }

    Some(current_id)
}

/// Find a named member of an element, including members inherited from typed definitions.
///
/// If the element is a typed usage (e.g., `part sensor : Sensor`), and the member
/// isn't found among direct children, searches the definition's children.
fn find_member_or_typed_member(
    graph: &ModelGraph,
    element_id: &ElementId,
    name: &str,
) -> Option<ElementId> {
    // Try direct children first
    if let Some(found) = graph
        .children_of(element_id)
        .find(|e| e.name.as_deref() == Some(name))
    {
        return Some(found.id.clone());
    }

    // If this element has a type, resolve it and search there.
    // Check the element's own `unresolved_type` prop first, then look for
    // FeatureTyping children which carry the type reference.
    let element = graph.get_element(element_id)?;
    let type_name = element
        .get_prop("unresolved_type")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            graph.children_of(element_id).find_map(|c| {
                if c.kind == crate::ElementKind::FeatureTyping
                    || c.kind.is_subtype_of(crate::ElementKind::FeatureTyping)
                {
                    c.get_prop("unresolved_type")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                } else {
                    None
                }
            })
        })?;

    // Find the definition element by name (global search)
    let def_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some(type_name.as_str()))
        .map(|e| e.id.clone())?;

    // Search definition's children
    graph
        .children_of(&def_id)
        .find(|e| e.name.as_deref() == Some(name))
        .map(|e| e.id.clone())
}

/// Report of what the elaboration pass added.
#[derive(Debug, Clone, Default)]
pub struct ElaborationReport {
    /// Number of states tagged as initial.
    pub initial_states_tagged: usize,
    /// Number of states tagged as final.
    pub final_states_tagged: usize,
    /// Number of Transition relationships created.
    pub transitions_created: usize,
    /// Number of entry/do/exit actions tagged on states.
    pub state_actions_tagged: usize,
    /// Number of constraint/expr properties derived.
    pub constraints_derived: usize,
    /// Number of succession transitions created.
    pub successions_created: usize,
    /// Number of implicit `stateSequencing` successions derived between a state's
    /// exclusive substates (States.sysml:71-77).
    pub state_sequencing_created: usize,
    /// Number of flow source/target properties set.
    pub flows_derived: usize,
    /// Number of import properties normalized.
    pub imports_elaborated: usize,
    /// Number of action properties derived.
    pub actions_elaborated: usize,
    /// Number of requirement properties derived.
    pub requirements_elaborated: usize,
    /// Number of Trace/Refine/Derive edges + derivation tags minted (B1).
    pub dependencies_elaborated: usize,
    /// Number of connector properties derived.
    pub connectors_elaborated: usize,
    /// Number of port properties elaborated.
    pub ports_elaborated: usize,
    /// Number of implicit base specialization edges minted (IG-1).
    pub implicit_generalizations_minted: usize,
}

impl ElaborationReport {
    /// Total number of modifications made.
    pub fn total(&self) -> usize {
        self.initial_states_tagged
            + self.final_states_tagged
            + self.transitions_created
            + self.state_actions_tagged
            + self.constraints_derived
            + self.successions_created
            + self.state_sequencing_created
            + self.flows_derived
            + self.imports_elaborated
            + self.actions_elaborated
            + self.requirements_elaborated
            + self.dependencies_elaborated
            + self.connectors_elaborated
            + self.ports_elaborated
            + self.implicit_generalizations_minted
    }

    /// Whether any modifications were made.
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

impl std::fmt::Display for ElaborationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Elaboration: {} initial, {} final, {} transitions, {} actions, {} constraints, \
             {} successions, {} flows, {} imports, {} action-props, {} requirements, \
             {} connectors, {} ports, {} implicit-generals ({} total)",
            self.initial_states_tagged,
            self.final_states_tagged,
            self.transitions_created,
            self.state_actions_tagged,
            self.constraints_derived,
            self.successions_created,
            self.flows_derived,
            self.imports_elaborated,
            self.actions_elaborated,
            self.requirements_elaborated,
            self.connectors_elaborated,
            self.ports_elaborated,
            self.implicit_generalizations_minted,
            self.total(),
        )
    }
}

/// Elaborate a parsed ModelGraph by deriving implicit structure.
///
/// This is the main entry point. It runs all sub-elaboration passes:
/// 1. State machines: tag initial/final states, create transitions, tag actions
/// 2. Constraints: derive `constraint`/`expr` properties from `unresolved_value`
/// 3. Successions: create transitions from succession elements in actions
/// 4. Flows: extract source/target from flow endpoint children
/// 5. Imports: normalize imported namespace, recursive flag, isNamespace
/// 6. Actions: tag condition/collection/target/receiver on control-flow actions
/// 7. Requirements: tag subject/objective from membership children
/// 8. Connectors: extract source/target endpoints, tag allocation roles
///
/// Returns a report of what was added.
pub fn elaborate(graph: &mut ModelGraph) -> ElaborationReport {
    elaborate_with_library(graph, None, None)
}

/// Elaborate a parsed ModelGraph, resolving implicit-generalization base types
/// against the supplied standard-library graph (kept as a linked / fallback
/// graph, NOT merged).
///
/// This is the library-aware entry point. The implicit-generalization pass
/// (IG-1) needs the stdlib to resolve qualified base names like
/// `Connections::Connection`. When `library` is `None` the IG pass resolves
/// only against the user graph (most bases live in the stdlib, so it is a
/// near-no-op — the correct behaviour: unresolved bases are silently skipped).
///
/// `lib_inheritance_index`, if provided, is a long-lived pre-built inheritance
/// index for `library`. IG-1 will reuse it via the new
/// `ResolutionContext::*_with_lib_inheritance_index` ctors instead of having
/// every per-candidate `ResolutionContext::new(lib)` rebuild the closure over
/// the ~77k stdlib elements. Pass `None` for the legacy lazy-rebuild path
/// (still correct, just slow on workspace shapes). May 29 perf baseline: this
/// `collect_specializations` frame was 39.4 % exclusive on workspace elaborate.
///
/// All other passes are library-independent and run identically.
pub fn elaborate_with_library(
    graph: &mut ModelGraph,
    library: Option<&ModelGraph>,
    lib_inheritance_index: Option<&std::sync::Arc<crate::resolution::InheritanceIndex>>,
) -> ElaborationReport {
    let mut report = ElaborationReport::default();

    state_machines::elaborate_state_machines(graph, &mut report);
    constraints::elaborate_constraints(graph, &mut report);
    successions::elaborate_successions(graph, &mut report);
    flows::elaborate_flows(graph, &mut report);
    imports::elaborate_imports(graph, &mut report);
    actions::elaborate_actions(graph, &mut report);
    requirements::elaborate_requirements(graph, &mut report);
    dependencies::elaborate_dependencies(graph, &mut report);
    connectors::elaborate_connectors(graph, &mut report);
    ports::elaborate_ports(graph, &mut report);
    // IG-1: implicit base specializations. Runs last — it only needs element
    // kinds + the resolved stdlib, not the structure derived above.
    implicit_generalization::elaborate_implicit_generalization(
        graph,
        library,
        lib_inheritance_index,
        &mut report,
    );

    // Final act: mark the graph elaborated. Must run AFTER every pass above —
    // each pass adds derived structure via `add_element`/`add_relationship`,
    // which clears the marker. `ModelCompiler::from_arc` trusts this flag to
    // skip a redundant re-elaborate of an already-elaborated graph (RSC-6.1).
    graph.mark_elaborated();

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elaborate_empty_graph() {
        let mut graph = ModelGraph::new();
        let report = elaborate(&mut graph);
        assert!(report.is_empty());
        assert_eq!(report.total(), 0);
    }

    #[test]
    fn report_display() {
        let report = ElaborationReport {
            initial_states_tagged: 1,
            transitions_created: 3,
            imports_elaborated: 2,
            ..Default::default()
        };
        let s = format!("{}", report);
        assert!(s.contains("1 initial"));
        assert!(s.contains("3 transitions"));
        assert!(s.contains("2 imports"));
        assert!(s.contains("6 total"));
    }
}
