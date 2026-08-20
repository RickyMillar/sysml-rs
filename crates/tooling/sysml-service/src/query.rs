//! Query operations — delegates to `sysml_core::query`.
//!
//! These methods provide service-context-aware wrappers around the core query
//! functions, operating on the service's internal model graph(s).

use std::collections::{HashMap, HashSet};

use sysml_core::query;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};

use crate::bounds::extract_bounds_by_attribute;
use crate::types::{Archetype, BoundMarker, ElementStats, SmTransitionDescriptor, TreeNode};

/// Generate a trace matrix between element kinds via a relationship kind.
pub fn trace_matrix(
    graph: &ModelGraph,
    source_kind: &ElementKind,
    rel_kind: &RelationshipKind,
    target_kind: &ElementKind,
) -> Vec<query::TraceMatrixRow> {
    query::trace_matrix(graph, source_kind, rel_kind, target_kind)
}

/// Find unverified requirements (excludes stdlib/library requirements).
pub fn unverified(graph: &ModelGraph) -> Vec<&Element> {
    query::requirements_unverified(graph)
        .filter(|e| !graph.is_library_element(&e.id))
        .collect()
}

/// Compute element/relationship statistics.
pub fn stats(graph: &ModelGraph) -> ElementStats {
    ElementStats {
        total_elements: graph.elements.len(),
        total_relationships: graph.relationships.len(),
        elements_by_kind: query::count_elements_by_kind(graph),
        relationships_by_kind: query::count_relationships_by_kind(graph),
    }
}

/// Walk the ownership chain upward.
pub fn ancestors<'a>(graph: &'a ModelGraph, id: &'a ElementId) -> Vec<&'a Element> {
    query::ancestors(graph, id)
}

/// Which subset of elements the tree projection should expose.
///
/// `UserFacing` (the default for the simulation UI) drops the structural
/// noise kinds the FE used to filter via `PRUNE_KINDS` — membership edges,
/// type-binding relationships, expression AST sub-nodes, ports, flows,
/// connections, transitions, plain metadata. `Full` keeps everything for
/// AI-agent / refactoring callers that need the raw graph shape.
///
/// R2.1 of the backend-first cleansing audit: moving the `PRUNE_KINDS`
/// filter server-side so the frontend doesn't have to enumerate spec
/// internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeView {
    /// Drop spec-mandated wrappers + currently-hidden domain kinds. Mirror
    /// of the FE's `PRUNE_KINDS` set (see `editors/simulation-app/src/
    /// features/sessions/tree/buildModelTree.ts`).
    UserFacing,
    /// Keep every element kind. Used by AI-agent inspection and by
    /// callers that need the unfiltered graph.
    Full,
}

impl TreeView {
    /// Map a transport-supplied string (`"user_facing"` / `"full"`) to a
    /// `TreeView`. Unknown / `None` → `UserFacing` (safe default — the
    /// simulation UI's expectation today).
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("full") => Self::Full,
            // Anything else (typo, omitted, `"user_facing"`) falls
            // back to the simulation-UI default.
            _ => Self::UserFacing,
        }
    }
}

// Classification of `ElementKind` into `Archetype` + user-facing noise is
// codegen'd from `crates/lang/codegen/src/archetype_rules.toml`. The build
// validates that every `ElementKind` variant resolves to either an archetype
// anchor, a noise predicate / explicit-hide entry, or `[other].explicit_other`
// — so spec extensions that introduce new kinds force a conscious routing
// decision rather than silently drifting to `Archetype::Other`.
//
// The two emitted functions are:
//   - `archetype_for(&ElementKind) -> Archetype`       (flat match)
//   - `is_user_facing_noise_for(&ElementKind) -> bool` (matches!)
include!(concat!(env!("OUT_DIR"), "/element_kind_classification.generated.rs"));

/// Build a tree of root elements and their children (for tree views).
///
/// When `max_depth` is `Some(n)`, recursion stops at depth `n` and any
/// remaining children are summarised as a single truncated node showing
/// how many children were hidden.
pub fn model_tree(graph: &ModelGraph, max_depth: Option<usize>) -> Vec<TreeNode> {
    model_tree_with_resolver(graph, graph, max_depth, TreeView::UserFacing)
}

/// Like `model_tree`, but uses a separate `resolver` graph to look up
/// typed-definition children for each usage.
///
/// SysML v2 separates **definition** (`PartDefinition`, `ItemDefinition`,
/// `PortDefinition`) from **usage** (`PartUsage`, `ItemUsage`,
/// `PortUsage`). A usage only owns the attributes/sub-parts it
/// *overrides* — everything else it inherits from its type via a
/// `FeatureTyping` relationship. A naive owner-only tree walk (what
/// the earlier implementation did) therefore shows a shallow usage
/// with nothing under it, even when the type has a rich subsystem
/// structure.
///
/// This variant follows each usage's `FeatureTyping` child to locate
/// the typed definition in the `resolver` graph, inlines the
/// definition's children under the usage (deduped by name — the
/// usage's own children win on collision), and recurses so nested
/// usages resolve through the same mechanism.
///
/// The `resolver` graph can be the same as `graph` for single-file
/// views, or the workspace-wide merged graph so cross-file typing
/// resolves (e.g. a usage in `Panel.sysml` whose type lives
/// in `CircuitPath.sysml`).
pub fn model_tree_with_resolver(
    graph: &ModelGraph,
    resolver: &ModelGraph,
    max_depth: Option<usize>,
    view: TreeView,
) -> Vec<TreeNode> {
    // Filter out internal structural elements that aren't meaningful domain roots.
    // Memberships and FeatureTyping are spec-mandated wrappers — they have no
    // owner but aren't user-visible concepts. Use the generated is_relationship()
    // predicate as the primary filter (covers all *Membership + FeatureTyping),
    // but exclude connector/interface usages which are also relationships but
    // ARE meaningful domain elements.
    let roots: Vec<_> = graph
        .roots()
        .filter(|e| !e.kind.is_relationship() || e.kind.is_usage() || e.kind.is_definition())
        .collect();
    // R3.3: extract every constraint-bound on every AttributeUsage in
    // a single bulk pass so per-node lookup during the recursive walk
    // is O(1). Replaces the FE's `boundExtractor.ts` walker.
    let bounds_map = extract_bounds_by_attribute(graph);
    let mut visited: HashSet<ElementId> = HashSet::new();
    let mut tree: Vec<TreeNode> = roots
        .iter()
        .filter_map(|e| {
            build_tree_node(
                graph,
                resolver,
                e,
                max_depth,
                0,
                &mut visited,
                view,
                &bounds_map,
            )
        })
        .collect();
    // R3.2 + R3.4 at the root level: same (name, kind) dedupe + stable
    // archetype sort `build_tree_node` applies per node. Per-file roots
    // also need to be authoritative for the FE so it never re-sorts.
    {
        let mut seen: HashSet<(Option<String>, ElementKind)> = HashSet::new();
        tree.retain(|c| seen.insert((c.name.clone(), c.kind.clone())));
    }
    tree.sort_by_key(|c| c.archetype.sort_rank());
    // Typed-def inlining deliberately surfaces the same element under
    // every usage of its definition — that's the right semantics, but
    // React keys (and expansion / pin state, which are keyed by id)
    // require `TreeNode.id` to be unique within the response. Fix-up
    // post-pass assigns fresh ids to second+ occurrences and stashes
    // the original in `element_id`.
    dedupe_tree_node_ids(&mut tree);
    tree
}

fn build_tree_node(
    graph: &ModelGraph,
    resolver: &ModelGraph,
    element: &Element,
    max_depth: Option<usize>,
    current_depth: usize,
    visited_definitions: &mut HashSet<ElementId>,
    view: TreeView,
    bounds_map: &HashMap<ElementId, Vec<BoundMarker>>,
) -> Option<TreeNode> {
    // R2.1 of the backend-first cleansing audit: drop noise kinds at
    // the source in user-facing mode so the FE doesn't have to
    // re-enumerate the same set. Returning `None` is correct here —
    // the recursive caller `filter_map`s, and the promotion-of-children
    // semantics the FE used to do via `PRUNE_KINDS` is replaced by the
    // graph already not connecting noise wrappers to their content's
    // owner (children sit directly under the meaningful parent).
    if view == TreeView::UserFacing && is_user_facing_noise_for(&element.kind) {
        return None;
    }

    let mut children: Vec<TreeNode> = if max_depth.is_some_and(|d| current_depth >= d) {
        let child_count = graph.children_of(&element.id).count();
        if child_count > 0 {
            vec![TreeNode {
                // canonical-key: synthetic-runtime-state — sentinel "... (N children)"
                // depth-cap node; not a real model element, never persists.
                id: ElementId::new_v4(),
                name: Some(format!("... ({child_count} children)")),
                kind: ElementKind::Comment, // lightweight sentinel kind
                archetype: Archetype::Other,
                children: vec![],
                is_ode: false,
                typed_as: None,
                element_id: None,
                source: None,
                target: None,
                unit: None,
                isq_dimension: None,
                transitions: Vec::new(),
                bounds: Vec::new(),
                default_collapsed: false,
                source_uri: None,
            }]
        } else {
            vec![]
        }
    } else {
        graph
            .children_of(&element.id)
            .filter_map(|child| {
                build_tree_node(
                    graph,
                    resolver,
                    child,
                    max_depth,
                    current_depth + 1,
                    visited_definitions,
                    view,
                    bounds_map,
                )
            })
            .collect()
    };

    // Inline typed-definition children for usages so a `circuit1 : CircuitPath`
    // PartUsage surfaces all of CircuitPath's sub-parts / ports / attrs under
    // it. Matches SysML v2 semantics — the usage *has* everything the
    // definition declares, even though the tree's ownership relation only
    // connects it to its own overrides.
    //
    // Also captures the resolved definition id into `typed_as` so the
    // frontend Usages filter can drop a definition whose id is the
    // type of any usage (no name-string heuristics).
    //
    // Skipped when the depth limit has already bitten (the truncation sentinel
    // would duplicate confusingly) and cycle-guarded through
    // `visited_definitions` so a definition that transitively references
    // itself doesn't infinitely recurse.
    let depth_budget_ok = !max_depth.is_some_and(|d| current_depth >= d);
    // Hard cap on the typed-def inlining chain. Backstop guardrail: with
    // connection-shaped usages now going through the inliner, a sane
    // model shouldn't approach this depth, but a maliciously self-
    // referencing definition graph could still slip past `visited_defs`
    // if alternative pipelines mint distinct ElementIds along the chain.
    // 16 is well past any realistic nesting (a deepest-known stdlib
    // pattern hits ~5).
    const MAX_INLINE_DEPTH: usize = 16;
    let inline_depth_ok = visited_definitions.len() < MAX_INLINE_DEPTH;
    let mut typed_as: Option<ElementId> = None;
    if is_resolvable_usage_kind(&element.kind) {
        if let Some(definition) = resolve_type_definition(resolver, element) {
            typed_as = Some(definition.id.clone());
            if depth_budget_ok
                && inline_depth_ok
                && visited_definitions.insert(definition.id.clone())
            {
                let own_names: HashSet<String> = children
                    .iter()
                    .filter_map(|c| c.name.clone())
                    .collect();
                let inherited: Vec<TreeNode> = resolver
                    .children_of(&definition.id)
                    .filter(|c| !is_usage_only_decoration(&c.kind))
                    .filter(|c| {
                        c.name
                            .as_ref()
                            .map(|n| !own_names.contains(n))
                            .unwrap_or(true)
                    })
                    .filter_map(|c| {
                        build_tree_node(
                            resolver,
                            resolver,
                            c,
                            max_depth,
                            current_depth + 1,
                            visited_definitions,
                            view,
                            bounds_map,
                        )
                    })
                    .collect();
                visited_definitions.remove(&definition.id);
                children.extend(inherited);
            }
        }
    }

    // R3.2: dedupe siblings by (name, kind). Typed-def inlining can
    // surface the same `power` payload through multiple FlowUsage
    // chains — first occurrence wins, later duplicates share identity
    // from a reader's point of view so hiding them reduces noise.
    {
        let mut seen: HashSet<(Option<String>, ElementKind)> = HashSet::new();
        children.retain(|c| seen.insert((c.name.clone(), c.kind.clone())));
    }

    // R3.4: stable sort by archetype rank. `Vec::sort_by_key` is
    // stable, so two siblings with the same archetype keep their
    // authoring order. Replaces the FE's `KIND_SORT_ORDER` /
    // `kindRank` pair — backend is now authoritative for sibling
    // ordering, the FE never re-sorts within a file.
    children.sort_by_key(|c| c.archetype.sort_rank());

    // Authoritative ODE detection: a CalculationUsage / CalculationDefinition
    // whose subsetting chain reaches `GetDerivative` is an ODE RHS per the
    // spec's StateSpaceRepresentation pattern. Only compute for calc kinds
    // so plain attributes etc. keep the default `false`.
    let is_ode = matches!(
        element.kind,
        ElementKind::CalculationUsage | ElementKind::CalculationDefinition
    ) && sysml_runtime::compiler::specializes_name(graph, &element.id, "GetDerivative");

    // For AttributeUsage kinds, project unit + ISQ dimension so the
    // frontend can drop the `metricRegistry` name-keyed unit lookup
    // (R3.1 of the backend-cleansing audit). Computed via
    // `attribute_physics_info` which walks the element's `unit` prop
    // and FeatureTyping chain to the physics layer's ISQ tables.
    let (unit, isq_dimension) = if element.kind == ElementKind::AttributeUsage {
        attribute_physics_info(graph, resolver, element)
    } else {
        (None, None)
    };

    // For transition kinds, project the source/target state short names so
    // the frontend can drop its `parseTransitionName` regex. Reuses the
    // shared resolver so SM TreeNodes can populate `transitions` with the
    // same logic (R2.1 fixup).
    let (source, target) = if matches!(
        element.kind,
        ElementKind::TransitionUsage | ElementKind::SuccessionAsUsage
    ) {
        resolve_transition_endpoints(graph, element)
    } else {
        (None, None)
    };

    // For SM kinds, project the static transition list extracted from
    // each TransitionUsage child. The same source/target resolver is
    // applied per child, so the FE state-graph SVG can render even in
    // `user_facing` mode where TransitionUsage children themselves are
    // filtered out (R2.1 fixup). Populated in BOTH views — redundant
    // in `Full` (each child also has its own TreeNode with source /
    // target) but harmless and keeps the API consistent.
    let transitions: Vec<SmTransitionDescriptor> = if matches!(
        element.kind,
        ElementKind::StateUsage
            | ElementKind::StateDefinition
            | ElementKind::ExhibitStateUsage
    ) {
        graph
            .children_of(&element.id)
            .filter(|c| c.kind == ElementKind::TransitionUsage)
            .map(|c| {
                let (src, tgt) = resolve_transition_endpoints(graph, c);
                SmTransitionDescriptor {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    source: src,
                    target: tgt,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let archetype = archetype_for(&element.kind);

    // Suggest a collapsed initial render for archetypes whose typed-def
    // inlining tends to produce heavy fan-out (Port + Connection — a
    // single FlowConnectionDefinition inlined under every PortUsage can
    // surface 6+ signal-attribute children). The FE consumes this hint
    // when seeding initial expand state on first arrival; explicit user
    // expansion overrides on subsequent renders. Leaf nodes get `false`
    // — there's nothing to collapse.
    let default_collapsed = matches!(archetype, Archetype::Port | Archetype::Connection)
        && !children.is_empty();

    // For AttributeUsage TreeNodes, look up the pre-computed bounds
    // for this element's id (R3.3). Empty for everything else, and
    // serialised away by `skip_serializing_if = "Vec::is_empty"`.
    let bounds = if element.kind == ElementKind::AttributeUsage {
        bounds_map.get(&element.id).cloned().unwrap_or_default()
    } else {
        Vec::new()
    };

    // Per-node file attribution from the element's first span. Needed
    // by workspace-scoped trees (one merged graph — no per-response
    // uri to fall back on) and more precise than the request uri on
    // per-file trees (typed-def inlining surfaces children declared in
    // other files, often the stdlib).
    let source_uri = element.spans.first().map(|s| s.file.clone());

    Some(TreeNode {
        id: element.id.clone(),
        name: element.name.clone(),
        kind: element.kind.clone(),
        archetype,
        children,
        is_ode,
        typed_as,
        element_id: None,
        source,
        target,
        unit,
        isq_dimension,
        transitions,
        bounds,
        default_collapsed,
        source_uri,
    })
}

/// Resolve a TransitionUsage / SuccessionAsUsage's `source` / `target`
/// short names. The element's `source` / `target` props are either:
///   - `Value::String(name)` — pre-resolution, the parser-emitted name
///   - `Value::Ref(id)`      — post-resolution, points at the sibling
/// …with `unresolved_source` / `unresolved_target` as a final fallback
/// when neither resolved nor named (rare).
///
/// Extracted for reuse: called once per TransitionUsage TreeNode (in
/// `Full` view) and once per TransitionUsage child of an SM (when
/// projecting the SM's `transitions` field).
fn resolve_transition_endpoints(
    graph: &ModelGraph,
    element: &Element,
) -> (Option<String>, Option<String>) {
    let resolve = |prop: &str, unresolved_prop: &str| -> Option<String> {
        match element.get_prop(prop) {
            Some(Value::String(name)) => Some(name.clone()),
            Some(Value::Ref(id)) => graph
                .get_element(id)
                .and_then(|e| e.name.clone())
                .or_else(|| {
                    element
                        .get_prop(unresolved_prop)
                        .and_then(|v| v.as_str())
                        .map(String::from)
                }),
            _ => element
                .get_prop(unresolved_prop)
                .and_then(|v| v.as_str())
                .map(String::from),
        }
    };
    (
        resolve("source", "unresolved_source"),
        resolve("target", "unresolved_target"),
    )
}

/// Compute `(unit, isq_dimension)` for an AttributeUsage element.
///
/// R3.1 of the backend-first cleansing audit: projects the same
/// information the frontend used to look up via the name-keyed
/// `metricRegistry` directly onto each attribute's TreeNode.
///
/// Strategy:
///   1. **Unit string** — first check the element's own `unit`
///      property (a directly authored unit name). If absent, walk
///      the element's `FeatureTyping` chain to find a typed unit /
///      ISQ-typed element, then read its `name` as the unit symbol.
///   2. **ISQ dimension** — walk the FeatureTyping chain. If the
///      typed element's name appears in `ISQ_TYPES`, format the
///      cached `DimensionVector` via `Display`. Otherwise call the
///      physics-layer `extract_dimension_from_unit_element` walker
///      on the typed element id; format the resulting vector if
///      non-zero. `None` when neither path resolves (the attribute
///      has no ISQ typing the workspace can see).
///
/// Both pieces share the same FeatureTyping resolution so a single
/// graph traversal feeds both fields.
fn attribute_physics_info(
    graph: &ModelGraph,
    resolver: &ModelGraph,
    element: &Element,
) -> (Option<String>, Option<String>) {
    use sysml_core::physics::dimension::extract_dimension_from_unit_element;
    use sysml_core::physics::isq_types::lookup_isq_type;

    // Unit from the element's own `unit` prop (authored form: `attribute x : Real [V];`).
    let mut unit: Option<String> = element
        .get_prop("unit")
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        });

    // Resolve typed feature via FeatureTyping. Use the resolver
    // graph (cross-file workspace view when available) so types
    // declared in another file still resolve.
    let typed_id =
        sysml_core::resolution::scoping::chaining::find_feature_type(resolver, &element.id)
            .or_else(|| {
                // Fallback: pre-resolution graphs only carry
                // `unresolved_type` on the FeatureTyping child.
                for child in graph.children_of(&element.id) {
                    if child.kind != ElementKind::FeatureTyping {
                        continue;
                    }
                    if let Some(Value::String(type_name)) = child.get_prop("unresolved_type") {
                        let ids = resolver.lookup_by_name(type_name);
                        if let Some(first) = ids.first() {
                            return Some(first.clone());
                        }
                    }
                }
                None
            });

    // ISQ dimension: prefer the cached ISQ_TYPES table (covers stdlib
    // ScalarQuantityValue types without requiring the unit element to
    // be present), then fall back to walking the typed element's
    // QuantityPowerFactor children via the physics-layer extractor.
    let mut isq_dimension: Option<String> = None;

    if let Some(type_id) = typed_id.as_ref() {
        if let Some(typed_elem) = resolver.get_element(type_id) {
            // If unit prop wasn't authored, fall back to the typed
            // element's name (e.g. `attribute v : VoltageValue;` →
            // unit `"VoltageValue"` in the absence of a concrete
            // unit string). Keeps the field meaningful even when the
            // model author didn't pick a specific unit.
            if unit.is_none() {
                unit = typed_elem.name.clone();
            }
            if let Some(name) = typed_elem.name.as_deref() {
                if let Some(entry) = lookup_isq_type(name) {
                    isq_dimension = Some(entry.1.to_string());
                }
            }
            if isq_dimension.is_none() {
                let dim = extract_dimension_from_unit_element(type_id, resolver);
                if !dim.is_zero() {
                    isq_dimension = Some(dim.to_string());
                }
            }
        }
    }

    (unit, isq_dimension)
}

/// Walk the tree and ensure every node's `id` is unique within this
/// response. Duplicates arise legitimately from typed-definition
/// inlining: a single `FlowConnectionDefinition` with a `power : PowerFlow`
/// child shows that child under every `PortUsage` typed by it, so 20
/// PortUsages → 20 tree positions carrying the same underlying
/// `power` ElementId. React keys + expansion state + pin state all
/// collide, so we mint fresh ids for every occurrence after the
/// first, moving the original into `element_id` for callers that
/// still need to hit the real element (detail panel, hover, live
/// value lookup). Depth-first so the first occurrence — typically
/// the owner's direct child — keeps the canonical id.
pub(crate) fn dedupe_tree_node_ids(nodes: &mut [TreeNode]) {
    fn walk(n: &mut TreeNode, seen: &mut HashSet<ElementId>) {
        if !seen.insert(n.id.clone()) {
            // canonical-key: synthetic-runtime-state — id-collision rebrand for tree
            // traversal; preserves the original id under `element_id` so the model
            // mapping is recoverable.
            let original = std::mem::replace(&mut n.id, ElementId::new_v4());
            if n.element_id.is_none() {
                n.element_id = Some(original);
            }
        }
        for c in &mut n.children {
            walk(c, seen);
        }
    }
    let mut seen = HashSet::new();
    for n in nodes {
        walk(n, &mut seen);
    }
}

/// Usage kinds whose tree node should inherit structural children from
/// the typed definition. Kept intentionally narrow — attribute /
/// calculation / state usages have their own shape and don't benefit
/// from inlining; port usages inherit port-definition features which
/// surface attrs users care about (e.g. `flowRate` on a `WaterPort`);
/// connection-shaped usages (ConnectionUsage / InterfaceUsage /
/// AllocationUsage / FlowUsage / SuccessionFlowUsage /
/// BindingConnectorAsUsage) inherit signal-attribute children from their
/// connection-shaped definitions (the canonical case is a
/// `connection : WireType` whose `WireType : ConnectionDefinition` carries
/// `voltage` / `current` attributes worth surfacing under each usage).
///
/// Family roots that participate in typed-def inlining are caught via
/// `is_subtype_of` so every usage descendant resolves per spec without
/// listing them explicitly. The `matches!` arm covers both the family
/// roots themselves (e.g. `ItemUsage` is_subtype_of itself returns false)
/// and a few legacy callers that hit the kind directly.
fn is_resolvable_usage_kind(kind: &ElementKind) -> bool {
    if kind.is_subtype_of(ElementKind::ItemUsage)
        || kind.is_subtype_of(ElementKind::ConnectorAsUsage)
    {
        return true;
    }
    matches!(
        kind,
        ElementKind::ItemUsage
            | ElementKind::PartUsage
            | ElementKind::PortUsage
            | ElementKind::ConnectorAsUsage
            | ElementKind::ConnectionUsage
            | ElementKind::InterfaceUsage
            | ElementKind::AllocationUsage
            | ElementKind::FlowUsage
            | ElementKind::SuccessionFlowUsage
            | ElementKind::BindingConnectorAsUsage
    )
}

/// Drop the definition's own "decoration" kinds when inlining so a
/// usage doesn't duplicate every `FeatureTyping`, `Specialization`,
/// `Documentation`, etc. that the definition carries — only the
/// semantically interesting children belong under the usage.
fn is_usage_only_decoration(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::FeatureTyping
            | ElementKind::Subclassification
            | ElementKind::Specialization
            | ElementKind::Subsetting
            | ElementKind::Redefinition
            | ElementKind::Conjugation
            | ElementKind::ReferenceSubsetting
    )
}

/// Resolve a usage element to its typed definition (if one exists in
/// the resolver graph). Uses two strategies in order:
///
///   1. Walk the usage's own children for a `FeatureTyping` element,
///      read its `unresolved_type` prop (the parser emits this as the
///      definition's name), and find the first matching definition.
///   2. Fall back to outgoing `FeatureTyping` / `Specialize`
///      relationships, which post-resolution point at the target.
/// Definition kinds a usage may resolve to. Kept as `&'static`
/// references so `find_by_name` (which takes `Option<&ElementKind>`)
/// can consume them without tying the returned element's lifetime to
/// a temporary local.
///
/// Connection-shaped definitions (`ConnectionDefinition`,
/// `InterfaceDefinition`, `AllocationDefinition`, `FlowDefinition`)
/// resolve usages like `connection : WireType` so the typed-def
/// inliner can surface their attribute children under each usage.
const DEFINITION_KINDS: &[ElementKind] = &[
    ElementKind::PartDefinition,
    ElementKind::ItemDefinition,
    ElementKind::PortDefinition,
    ElementKind::ConnectionDefinition,
    ElementKind::InterfaceDefinition,
    ElementKind::AllocationDefinition,
    ElementKind::FlowDefinition,
];

/// Test whether `kind` is a definition kind we'll inline children from.
/// Catches direct entries in `DEFINITION_KINDS` AND subtypes (so
/// `ConjugatedPortDefinition`, which subtypes `PortDefinition`, flows
/// through automatically without enumerating it).
fn is_inlineable_definition_kind(kind: &ElementKind) -> bool {
    if DEFINITION_KINDS.contains(kind) {
        return true;
    }
    DEFINITION_KINDS
        .iter()
        .any(|k| kind.is_subtype_of(k.clone()))
}

fn resolve_type_definition<'a>(
    resolver: &'a ModelGraph,
    usage: &Element,
) -> Option<&'a Element> {
    // Primary: sysml-core's O(1) reverse index. Post-resolution
    // `find_feature_type` reads the FeatureTyping element's
    // `type` prop (a resolved ElementId ref) via the
    // `typed_feature_to_typings` index — no property-walking
    // or name-matching needed.
    if let Some(type_id) =
        sysml_core::resolution::scoping::chaining::find_feature_type(resolver, &usage.id)
    {
        if let Some(def) = resolver.get_element(&type_id) {
            if is_inlineable_definition_kind(&def.kind) {
                return Some(def);
            }
        }
    }
    // Fallback: pre-resolution models (or test graphs that skip
    // Pass 2) only carry `unresolved_type` on the FeatureTyping
    // child. Walk that as a last resort so the typed-def inlining
    // still works before the resolver has run.
    for child in resolver.children_of(&usage.id) {
        if child.kind != ElementKind::FeatureTyping {
            continue;
        }
        if let Some(Value::String(type_name)) = child.get_prop("unresolved_type") {
            for kind in DEFINITION_KINDS {
                if let Some(def) = query::find_by_name(resolver, Some(kind), type_name).next() {
                    return Some(def);
                }
            }
        }
    }
    // Last-ditch: follow outgoing Specialize relationships for
    // graphs elaborated via alternative pipelines.
    for rel in resolver.outgoing(&usage.id) {
        if rel.kind != RelationshipKind::Specialize {
            continue;
        }
        if let Some(target) = resolver.get_element(&rel.target) {
            if is_inlineable_definition_kind(&target.kind) {
                return Some(target);
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, Relationship};

    fn test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("SafetyReq")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        let satisfy = Relationship::new(RelationshipKind::Satisfy, part_id, req_id);
        graph.add_relationship(satisfy);

        graph
    }

    #[test]
    fn test_find_and_stats() {
        let graph = test_graph();
        let found = query::find_by_name(&graph, None, "Engine").count();
        assert_eq!(found, 1);

        let s = stats(&graph);
        assert_eq!(s.total_elements, 3);
        assert_eq!(s.total_relationships, 1);
    }

    #[test]
    fn test_model_tree() {
        let graph = test_graph();
        let tree = model_tree(&graph, None);
        assert_eq!(tree.len(), 1); // one root (Pkg)
        assert_eq!(tree[0].children.len(), 2); // Engine + SafetyReq
    }

    #[test]
    fn test_model_tree_max_depth() {
        let graph = test_graph();

        // depth=0 means roots show but children are truncated
        let tree = model_tree(&graph, Some(0));
        assert_eq!(tree.len(), 1);
        // The root's children should be a single truncated summary
        assert_eq!(tree[0].children.len(), 1);
        assert!(tree[0].children[0]
            .name
            .as_deref()
            .unwrap()
            .contains("2 children"));

        // depth=1 means roots + their direct children are shown fully
        let tree = model_tree(&graph, Some(1));
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 2); // Engine + SafetyReq (leaf nodes, no truncation)
    }

    #[test]
    fn test_unverified() {
        let graph = test_graph();
        let uv = unverified(&graph);
        assert_eq!(uv.len(), 1);
        assert_eq!(uv[0].name.as_deref(), Some("SafetyReq"));
    }

    #[test]
    fn test_model_tree_inlines_typed_definition_children() {
        // A `breaker : DualPoleBreaker` PartUsage should surface every
        // child `DualPoleBreaker` declares (phaseIn, phaseOut, ...)
        // under it in the tree view, modelling the SysML v2 "usage
        // inherits type features" rule.
        //
        // Uses TreeView::Full because PortUsage is filtered in
        // user-facing mode (R2.1) — the inlining logic still does the
        // right thing, we just need ports visible to assert on.
        let mut graph = ModelGraph::new();

        // DualPoleBreaker definition with two port children.
        let breaker_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("DualPoleBreaker");
        let breaker_def_id = graph.add_element(breaker_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("phaseIn")
                .with_owner(breaker_def_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("phaseOut")
                .with_owner(breaker_def_id.clone()),
        );

        // A Circuit PartUsage owning `breaker : DualPoleBreaker`.
        let circuit = Element::new_with_kind(ElementKind::PartUsage).with_name("circuit1");
        let circuit_id = graph.add_element(circuit);
        let breaker_usage = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("breaker")
            .with_owner(circuit_id.clone());
        let breaker_id = graph.add_element(breaker_usage);

        // FeatureTyping child — the parser emits this with an
        // `unresolved_type` prop naming the definition.
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(breaker_id.clone())
                .with_prop("unresolved_type", Value::String("DualPoleBreaker".into())),
        );

        let tree = model_tree_with_resolver(&graph, &graph, None, TreeView::Full);
        // Find the `circuit1` node, then its `breaker` child, then
        // assert phaseIn and phaseOut got inlined.
        let circuit_node = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("circuit1"))
            .expect("circuit1 must appear at root");
        let breaker_node = circuit_node
            .children
            .iter()
            .find(|n| n.name.as_deref() == Some("breaker"))
            .expect("breaker must appear under circuit1");
        let names: Vec<_> = breaker_node
            .children
            .iter()
            .filter_map(|c| c.name.as_deref())
            .collect();
        assert!(
            names.contains(&"phaseIn"),
            "phaseIn missing from inlined children: {:?}",
            names
        );
        assert!(
            names.contains(&"phaseOut"),
            "phaseOut missing from inlined children: {:?}",
            names
        );
    }

    #[test]
    fn test_model_tree_user_facing_drops_noise_kinds() {
        // Inverted-policy filter (commit 1 of model-tree rework): only the
        // truly non-semantic kinds are dropped (membership wrappers,
        // type-binding edges, expression AST, chrome / metadata,
        // transitions). Ports + connections + flows now SURVIVE because
        // they have their own Port / Connection archetypes.
        let mut graph = ModelGraph::new();

        let pkg_id = graph.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Pkg"),
        );

        // A real domain part — never noise.
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("realPart")
                .with_owner(pkg_id.clone()),
        );

        // PortUsage now SURVIVES user_facing (assigned Archetype::Port).
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("port1")
                .with_owner(pkg_id.clone()),
        );

        // True noise kinds that should still disappear in user_facing.
        graph.add_element(
            Element::new_with_kind(ElementKind::Comment).with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::OperatorExpression).with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping).with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::TransitionUsage)
                .with_name("t1")
                .with_owner(pkg_id.clone()),
        );

        // user_facing: PartUsage and PortUsage both survive under the
        // package; Comment / OperatorExpression / FeatureTyping /
        // TransitionUsage are filtered.
        let user_tree = model_tree_with_resolver(&graph, &graph, None, TreeView::UserFacing);
        let pkg = user_tree
            .iter()
            .find(|n| n.name.as_deref() == Some("Pkg"))
            .expect("Pkg root");
        let user_kinds: Vec<_> = pkg.children.iter().map(|c| c.kind.clone()).collect();
        assert!(
            user_kinds.iter().all(|k| matches!(
                k,
                ElementKind::PartUsage | ElementKind::PortUsage
            )),
            "user_facing kept unexpected noise: {:?}",
            user_kinds
        );
        assert!(
            user_kinds.contains(&ElementKind::PortUsage),
            "user_facing should now keep PortUsage (Port archetype): {:?}",
            user_kinds
        );
        assert!(
            !user_kinds.contains(&ElementKind::Comment),
            "user_facing should drop Comment: {:?}",
            user_kinds
        );
        assert!(
            !user_kinds.contains(&ElementKind::OperatorExpression),
            "user_facing should drop OperatorExpression: {:?}",
            user_kinds
        );
        assert!(
            !user_kinds.contains(&ElementKind::FeatureTyping),
            "user_facing should drop FeatureTyping: {:?}",
            user_kinds
        );
        assert!(
            !user_kinds.contains(&ElementKind::TransitionUsage),
            "user_facing should drop TransitionUsage: {:?}",
            user_kinds
        );

        // R2.4: the surviving PartUsage should classify as Archetype::Part,
        // the surviving PortUsage as Archetype::Port, and the package
        // itself as Archetype::Other.
        assert_eq!(pkg.archetype, Archetype::Other, "Pkg → Other");
        let real_part = pkg
            .children
            .iter()
            .find(|c| c.name.as_deref() == Some("realPart"))
            .expect("realPart");
        assert_eq!(real_part.archetype, Archetype::Part, "PartUsage → Part");
        let port1 = pkg
            .children
            .iter()
            .find(|c| c.name.as_deref() == Some("port1"))
            .expect("port1 should now appear in user_facing");
        assert_eq!(port1.archetype, Archetype::Port, "PortUsage → Port");

        // full: every child is present.
        let full_tree = model_tree_with_resolver(&graph, &graph, None, TreeView::Full);
        let full_pkg = full_tree
            .iter()
            .find(|n| n.name.as_deref() == Some("Pkg"))
            .expect("Pkg root");
        let full_kinds: Vec<_> = full_pkg.children.iter().map(|c| c.kind.clone()).collect();
        assert!(
            full_kinds.contains(&ElementKind::Comment),
            "full mode missing Comment: {:?}",
            full_kinds
        );
        assert!(
            full_kinds.contains(&ElementKind::PortUsage),
            "full mode missing PortUsage: {:?}",
            full_kinds
        );
        assert!(
            full_kinds.contains(&ElementKind::OperatorExpression),
            "full mode missing OperatorExpression: {:?}",
            full_kinds
        );
        assert!(
            full_kinds.contains(&ElementKind::TransitionUsage),
            "full mode missing TransitionUsage: {:?}",
            full_kinds
        );
    }

    #[test]
    fn test_sm_transitions_populated_in_user_facing_view() {
        // R2.1 fixup: SM TreeNodes carry a static `transitions` Vec
        // built by walking TransitionUsage children even when those
        // children are filtered out of the tree in `user_facing` view
        // (otherwise the FE's static state-graph SVG breaks).
        let mut graph = ModelGraph::new();

        // States `armed` and `tripped`, plus a transition between them.
        let sm_id = graph.add_element(
            Element::new_with_kind(ElementKind::StateDefinition)
                .with_name("BreakerStates"),
        );
        let armed_id = graph.add_element(
            Element::new_with_kind(ElementKind::StateUsage)
                .with_name("armed")
                .with_owner(sm_id.clone()),
        );
        let tripped_id = graph.add_element(
            Element::new_with_kind(ElementKind::StateUsage)
                .with_name("tripped")
                .with_owner(sm_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::TransitionUsage)
                .with_name("armed_to_tripped")
                .with_owner(sm_id.clone())
                .with_prop("source", Value::Ref(armed_id.clone()))
                .with_prop("target", Value::Ref(tripped_id.clone())),
        );

        // user_facing: TransitionUsage child rows are filtered out, but
        // the SM still carries the transition list.
        let user_tree =
            model_tree_with_resolver(&graph, &graph, None, TreeView::UserFacing);
        let sm = user_tree
            .iter()
            .find(|n| n.name.as_deref() == Some("BreakerStates"))
            .expect("SM root");
        // Confirm the TransitionUsage child is gone in user_facing.
        assert!(
            sm.children
                .iter()
                .all(|c| c.kind != ElementKind::TransitionUsage),
            "user_facing left TransitionUsage child: {:?}",
            sm.children.iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
        );
        // …but the static transition list is populated.
        assert_eq!(sm.transitions.len(), 1, "expected 1 transition");
        let t = &sm.transitions[0];
        assert_eq!(t.name.as_deref(), Some("armed_to_tripped"));
        assert_eq!(t.source.as_deref(), Some("armed"));
        assert_eq!(t.target.as_deref(), Some("tripped"));

        // full view: same transitions list (kept consistent across views).
        let full_tree = model_tree_with_resolver(&graph, &graph, None, TreeView::Full);
        let full_sm = full_tree
            .iter()
            .find(|n| n.name.as_deref() == Some("BreakerStates"))
            .expect("SM root (full)");
        assert_eq!(
            full_sm.transitions.len(),
            1,
            "full view missing transition list"
        );
    }

    #[test]
    fn test_model_tree_sets_typed_as_on_usages() {
        // Round-trips the typed-definition id onto the usage's
        // TreeNode so the frontend Usages filter can drop a
        // definition iff any usage's `typed_as` points at it.
        let mut graph = ModelGraph::new();

        let breaker_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("DualPoleBreaker");
        let def_id = graph.add_element(breaker_def);

        let circuit = Element::new_with_kind(ElementKind::PartUsage).with_name("circuit");
        let circuit_id = graph.add_element(circuit);
        let breaker = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("breaker")
            .with_owner(circuit_id);
        let breaker_id = graph.add_element(breaker);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(breaker_id.clone())
                .with_prop("unresolved_type", Value::String("DualPoleBreaker".into())),
        );

        let tree = model_tree(&graph, None);
        let circuit_node = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("circuit"))
            .expect("circuit");
        let breaker_node = circuit_node
            .children
            .iter()
            .find(|n| n.name.as_deref() == Some("breaker"))
            .expect("breaker");
        assert_eq!(breaker_node.typed_as.as_ref(), Some(&def_id));

        // Definitions themselves (and any non-usage kinds) carry
        // `typed_as = None`.
        let def_node = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("DualPoleBreaker"));
        if let Some(d) = def_node {
            assert!(d.typed_as.is_none());
        }
    }

    #[test]
    fn test_model_tree_usage_overrides_win_over_definition() {
        // When the usage owns its own child with the same name as a
        // definition child, the usage wins — this is the SysML v2
        // redefinition / override semantics: you can't see both a
        // definition `ratedCurrent` and a usage `ratedCurrent` under
        // the same usage node.
        let mut graph = ModelGraph::new();

        let def = Element::new_with_kind(ElementKind::PartDefinition).with_name("Breaker");
        let def_id = graph.add_element(def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("ratedCurrent")
                .with_owner(def_id.clone())
                .with_prop("unresolved_value", Value::String("def-default".into())),
        );

        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("breaker");
        let usage_id = graph.add_element(usage);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("ratedCurrent")
                .with_owner(usage_id.clone())
                .with_prop("unresolved_value", Value::String("usage-override".into())),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(usage_id.clone())
                .with_prop("unresolved_type", Value::String("Breaker".into())),
        );

        let tree = model_tree(&graph, None);
        let breaker = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("breaker"))
            .expect("breaker");
        let rated: Vec<_> = breaker
            .children
            .iter()
            .filter(|c| c.name.as_deref() == Some("ratedCurrent"))
            .collect();
        assert_eq!(rated.len(), 1, "exactly one ratedCurrent wins");
    }

    #[test]
    fn test_classify_archetype_covers_subtype_families() {
        // R2.4: backend projection lifts the FE's KIND_MAP. Verify the
        // hierarchy-based families catch their notable subtypes via
        // `is_subtype_of`, that the explicit anchors still classify
        // correctly (since `is_subtype_of` is strict), and that Calc
        // is exact-match (CaseUsage / RequirementUsage do NOT slip in).

        // Part family.
        assert_eq!(archetype_for(&ElementKind::PartUsage), Archetype::Part);
        assert_eq!(
            archetype_for(&ElementKind::PartDefinition),
            Archetype::Part
        );
        assert_eq!(archetype_for(&ElementKind::ItemUsage), Archetype::Part);
        assert_eq!(
            archetype_for(&ElementKind::ItemDefinition),
            Archetype::Part
        );

        // Attribute family.
        assert_eq!(
            archetype_for(&ElementKind::AttributeUsage),
            Archetype::Attribute
        );
        assert_eq!(
            archetype_for(&ElementKind::AttributeDefinition),
            Archetype::Attribute
        );
        // EnumerationUsage subtypes AttributeUsage per spec — auto-caught.
        assert_eq!(
            archetype_for(&ElementKind::EnumerationUsage),
            Archetype::Attribute,
            "EnumerationUsage should classify as Attribute (subtype of AttributeUsage)"
        );

        // SM family.
        assert_eq!(archetype_for(&ElementKind::StateUsage), Archetype::Sm);
        assert_eq!(
            archetype_for(&ElementKind::StateDefinition),
            Archetype::Sm
        );
        assert_eq!(
            archetype_for(&ElementKind::ExhibitStateUsage),
            Archetype::Sm,
            "ExhibitStateUsage should classify as Sm (subtype of StateUsage)"
        );

        // Constraint family — RequirementUsage / AssertConstraintUsage
        // / SatisfyRequirementUsage / ConcernUsage all subtype
        // ConstraintUsage per spec.
        assert_eq!(
            archetype_for(&ElementKind::ConstraintUsage),
            Archetype::Constraint
        );
        assert_eq!(
            archetype_for(&ElementKind::ConstraintDefinition),
            Archetype::Constraint
        );
        assert_eq!(
            archetype_for(&ElementKind::RequirementUsage),
            Archetype::Constraint,
            "RequirementUsage should upgrade from Other → Constraint via is_subtype_of"
        );
        assert_eq!(
            archetype_for(&ElementKind::AssertConstraintUsage),
            Archetype::Constraint
        );
        assert_eq!(
            archetype_for(&ElementKind::SatisfyRequirementUsage),
            Archetype::Constraint
        );
        assert_eq!(
            archetype_for(&ElementKind::ConcernUsage),
            Archetype::Constraint
        );

        // Calc — exact match only. CaseUsage subtypes CalculationUsage
        // per spec but must NOT classify as Calc (it's case-shaped, not
        // calc-shaped, in our UI).
        assert_eq!(
            archetype_for(&ElementKind::CalculationUsage),
            Archetype::Calc
        );
        assert_eq!(
            archetype_for(&ElementKind::CalculationDefinition),
            Archetype::Calc
        );
        // Case family — subtypes CalculationUsage → ActionUsage per spec,
        // but gets its own `Case` bucket (matched before `Action`; `Calc`
        // is exact-match so it never catches them).
        assert_eq!(archetype_for(&ElementKind::CaseUsage), Archetype::Case);
        assert_eq!(
            archetype_for(&ElementKind::CaseDefinition),
            Archetype::Case
        );
        assert_eq!(
            archetype_for(&ElementKind::AnalysisCaseUsage),
            Archetype::Case
        );
        assert_eq!(
            archetype_for(&ElementKind::AnalysisCaseDefinition),
            Archetype::Case
        );
        assert_eq!(
            archetype_for(&ElementKind::VerificationCaseUsage),
            Archetype::Case
        );
        assert_eq!(
            archetype_for(&ElementKind::VerificationCaseDefinition),
            Archetype::Case
        );
        assert_eq!(
            archetype_for(&ElementKind::UseCaseUsage),
            Archetype::Case
        );
        // Plain actions stay Action — the Case rule must not widen.
        assert_eq!(
            archetype_for(&ElementKind::ActionUsage),
            Archetype::Action
        );
        assert_eq!(
            archetype_for(&ElementKind::PerformActionUsage),
            Archetype::Action
        );

        // Port family — PortUsage + PortDefinition + ConjugatedPortDefinition
        // (subtypes PortDefinition per spec).
        assert_eq!(
            archetype_for(&ElementKind::PortUsage),
            Archetype::Port
        );
        assert_eq!(
            archetype_for(&ElementKind::PortDefinition),
            Archetype::Port
        );
        assert_eq!(
            archetype_for(&ElementKind::ConjugatedPortDefinition),
            Archetype::Port,
            "ConjugatedPortDefinition should classify as Port (subtype of PortDefinition)"
        );

        // Connection family — ConnectorAsUsage subtypes catch every
        // connection-shaped usage; ConnectionDefinition + subtypes catch
        // the definition side; FlowDefinition stands alone.
        assert_eq!(
            archetype_for(&ElementKind::ConnectionUsage),
            Archetype::Connection
        );
        assert_eq!(
            archetype_for(&ElementKind::FlowUsage),
            Archetype::Connection
        );
        assert_eq!(
            archetype_for(&ElementKind::InterfaceUsage),
            Archetype::Connection
        );
        assert_eq!(
            archetype_for(&ElementKind::AllocationUsage),
            Archetype::Connection
        );
        assert_eq!(
            archetype_for(&ElementKind::ConnectionDefinition),
            Archetype::Connection
        );
        assert_eq!(
            archetype_for(&ElementKind::InterfaceDefinition),
            Archetype::Connection,
            "InterfaceDefinition should classify as Connection (subtype of ConnectionDefinition)"
        );
        assert_eq!(
            archetype_for(&ElementKind::AllocationDefinition),
            Archetype::Connection,
            "AllocationDefinition should classify as Connection (subtype of ConnectionDefinition)"
        );
        assert_eq!(
            archetype_for(&ElementKind::BindingConnectorAsUsage),
            Archetype::Connection,
            "BindingConnectorAsUsage should classify as Connection (subtype of ConnectorAsUsage)"
        );
        assert_eq!(
            archetype_for(&ElementKind::FlowDefinition),
            Archetype::Connection
        );
        assert_eq!(
            archetype_for(&ElementKind::SuccessionFlowUsage),
            Archetype::Connection,
            "SuccessionFlowUsage should classify as Connection (subtype of ConnectorAsUsage)"
        );

        // Other — anything not in the families. Comment is a stable
        // sentinel of "definitely Other".
        assert_eq!(
            archetype_for(&ElementKind::Comment),
            Archetype::Other
        );
        assert_eq!(
            archetype_for(&ElementKind::Package),
            Archetype::Other
        );
    }

    #[test]
    fn test_model_tree_emits_archetype_field() {
        // Smoke test: a couple of mixed kinds in a real model_tree
        // call all surface their archetype on the projected TreeNode.
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("part1")
                .with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("attr1")
                .with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage)
                .with_name("c1")
                .with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("req1")
                .with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::CalculationUsage)
                .with_name("calc1")
                .with_owner(pkg_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::StateDefinition)
                .with_name("sm1")
                .with_owner(pkg_id.clone()),
        );

        let tree = model_tree(&graph, None);
        let pkg = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("Pkg"))
            .expect("Pkg root");

        let by_name = |n: &str| {
            pkg.children
                .iter()
                .find(|c| c.name.as_deref() == Some(n))
                .unwrap_or_else(|| panic!("missing {n}"))
                .archetype
        };

        assert_eq!(by_name("part1"), Archetype::Part);
        assert_eq!(by_name("attr1"), Archetype::Attribute);
        assert_eq!(by_name("c1"), Archetype::Constraint);
        assert_eq!(
            by_name("req1"),
            Archetype::Constraint,
            "RequirementUsage upgrades Other → Constraint"
        );
        assert_eq!(by_name("calc1"), Archetype::Calc);
        assert_eq!(by_name("sm1"), Archetype::Sm);
    }

    #[test]
    fn test_model_tree_is_ode_flag() {
        // GAP-ODE-001: tree projection must stamp `is_ode: true` on
        // CalculationUsage / CalculationDefinition elements whose
        // subsetting chain reaches `GetDerivative`. Plain calcs stay
        // `is_ode: false`, and non-calc kinds always default to false.
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // ODE RHS: calc def :> GetDerivative (simulated via unresolvedTypeName)
        let ode_calc = Element::new_with_kind(ElementKind::CalculationDefinition)
            .with_name("dvdt")
            .with_owner(pkg_id.clone())
            .with_prop("unresolvedTypeName", Value::String("GetDerivative".into()));
        graph.add_element(ode_calc);

        // Plain calc — no GetDerivative subsetting.
        let plain_calc = Element::new_with_kind(ElementKind::CalculationDefinition)
            .with_name("plainCalc")
            .with_owner(pkg_id.clone());
        graph.add_element(plain_calc);

        // Non-calc sibling should always be false (guard-rail).
        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("temperature")
            .with_owner(pkg_id.clone());
        graph.add_element(attr);

        let tree = model_tree(&graph, None);
        assert_eq!(tree.len(), 1);
        let pkg_children = &tree[0].children;
        let ode = pkg_children
            .iter()
            .find(|c| c.name.as_deref() == Some("dvdt"))
            .expect("ode calc must be present");
        let plain = pkg_children
            .iter()
            .find(|c| c.name.as_deref() == Some("plainCalc"))
            .expect("plain calc must be present");
        let attr = pkg_children
            .iter()
            .find(|c| c.name.as_deref() == Some("temperature"))
            .expect("attribute must be present");

        assert!(ode.is_ode, "GetDerivative specializer must be flagged is_ode");
        assert!(!plain.is_ode, "plain calc must NOT be flagged is_ode");
        assert!(!attr.is_ode, "non-calc kinds must always be false");
    }

    #[test]
    fn test_model_tree_dedupes_typed_def_inlined_children() {
        // R3.2: typed-def inlining can surface the same element name
        // multiple times under one usage (the canonical case is a
        // FlowConnectionDefinition's `power` payload appearing under
        // every PortUsage typed by it). First occurrence wins; later
        // (name, kind) duplicates are dropped server-side so the FE
        // doesn't have to.
        //
        // Fixture: a part with three sibling AttributeUsages all named
        // `power` — exercises the same dedupe code path without
        // depending on the typed-def inliner being present.
        let mut graph = ModelGraph::new();
        let parent = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("breaker");
        let parent_id = graph.add_element(parent);
        for _ in 0..3 {
            graph.add_element(
                Element::new_with_kind(ElementKind::AttributeUsage)
                    .with_name("power")
                    .with_owner(parent_id.clone()),
            );
        }
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("cmd")
                .with_owner(parent_id.clone()),
        );

        let tree = model_tree(&graph, None);
        let breaker = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("breaker"))
            .expect("breaker root");
        let names: Vec<_> = breaker
            .children
            .iter()
            .filter_map(|c| c.name.as_deref())
            .collect();
        // Exactly one `power` survives — the first wins.
        assert_eq!(
            names.iter().filter(|n| **n == "power").count(),
            1,
            "expected exactly one `power` after dedupe, got {:?}",
            names
        );
        assert!(names.contains(&"cmd"), "cmd survives dedupe: {:?}", names);
    }

    #[test]
    fn test_model_tree_sorts_children_by_archetype() {
        // R3.4: backend is authoritative for sibling sort order. Mixed
        // children in deliberately scrambled authoring order should
        // come back ordered by Archetype::sort_rank (Part → Sm →
        // Constraint → Calc → Attribute → Other).
        let mut graph = ModelGraph::new();
        let root = Element::new_with_kind(ElementKind::PartUsage).with_name("Root");
        let root_id = graph.add_element(root);

        // Authoring order intentionally NOT in sort order.
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("attr1")
                .with_owner(root_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("subPart")
                .with_owner(root_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage)
                .with_name("c1")
                .with_owner(root_id.clone()),
        );

        let tree = model_tree(&graph, None);
        let root_node = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("Root"))
            .expect("Root must appear");
        let archetypes: Vec<_> =
            root_node.children.iter().map(|c| c.archetype).collect();
        assert_eq!(
            archetypes,
            vec![Archetype::Part, Archetype::Constraint, Archetype::Attribute],
            "children must be sorted by archetype rank, got {:?}",
            archetypes
        );
    }

    #[test]
    fn test_model_tree_default_collapsed_for_port_with_children() {
        // Commit 2 of the model-tree rework: Port + Connection nodes
        // that have children get `default_collapsed = true` so the FE
        // doesn't bury structural nodes when it inlines heavy
        // signal-attribute fan-out from the typed definition. Leaves
        // stay `false` (nothing to collapse).
        let mut graph = ModelGraph::new();

        // PortDefinition with one attribute child — every PortUsage
        // typed by it inlines the attribute, picking up children.
        let port_def = Element::new_with_kind(ElementKind::PortDefinition)
            .with_name("VoltagePort");
        let port_def_id = graph.add_element(port_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("voltage")
                .with_owner(port_def_id.clone()),
        );

        // PortUsage typed by VoltagePort — gets the inlined `voltage` child.
        let port_usage = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("p1");
        let port_usage_id = graph.add_element(port_usage);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(port_usage_id.clone())
                .with_prop("unresolved_type", Value::String("VoltagePort".into())),
        );

        // Standalone PortUsage with no typing → no children → not
        // default-collapsed.
        let leaf_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("leaf");
        graph.add_element(leaf_port);

        let tree = model_tree_with_resolver(&graph, &graph, None, TreeView::Full);
        let p1 = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("p1"))
            .expect("p1 root");
        let leaf = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("leaf"))
            .expect("leaf root");

        assert_eq!(p1.archetype, Archetype::Port);
        assert!(
            !p1.children.is_empty(),
            "p1 should have inlined children: {:?}",
            p1.children
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            p1.default_collapsed,
            "Port with children should be default_collapsed=true"
        );
        assert!(
            leaf.children.is_empty(),
            "leaf port shouldn't have children"
        );
        assert!(
            !leaf.default_collapsed,
            "leaf Port should be default_collapsed=false"
        );
    }

    #[test]
    fn test_model_tree_inlines_typed_definition_children_for_connection_usage() {
        // Commit 2: a `connection : WireType` ConnectionUsage typed
        // by a ConnectionDefinition `WireType` should inherit
        // `WireType`'s attribute children (voltage / current).
        let mut graph = ModelGraph::new();

        let wire_def = Element::new_with_kind(ElementKind::ConnectionDefinition)
            .with_name("WireType");
        let wire_def_id = graph.add_element(wire_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("voltage")
                .with_owner(wire_def_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("current")
                .with_owner(wire_def_id.clone()),
        );

        // Hosting part with a connection usage.
        let circuit = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("circuit");
        let circuit_id = graph.add_element(circuit);
        let conn = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("link")
            .with_owner(circuit_id.clone());
        let conn_id = graph.add_element(conn);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(conn_id.clone())
                .with_prop("unresolved_type", Value::String("WireType".into())),
        );

        let tree = model_tree_with_resolver(&graph, &graph, None, TreeView::Full);
        let circuit_node = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("circuit"))
            .expect("circuit root");
        let conn_node = circuit_node
            .children
            .iter()
            .find(|n| n.name.as_deref() == Some("link"))
            .expect("link must be under circuit");
        let names: Vec<_> = conn_node
            .children
            .iter()
            .filter_map(|c| c.name.as_deref())
            .collect();
        assert!(
            names.contains(&"voltage"),
            "voltage missing from inlined children: {:?}",
            names
        );
        assert!(
            names.contains(&"current"),
            "current missing from inlined children: {:?}",
            names
        );
        assert_eq!(conn_node.archetype, Archetype::Connection);
        assert!(
            conn_node.default_collapsed,
            "Connection with inlined children should be default_collapsed=true"
        );
    }

    #[test]
    fn test_model_tree_inlines_for_flow_and_interface_usages() {
        // Commit 2: FlowUsage / InterfaceUsage / AllocationUsage are
        // also resolvable usage kinds — their typed definitions get
        // inlined. Smoke test: each kind, when typed, surfaces a
        // single attribute child.
        let mut graph = ModelGraph::new();

        let make_def = |graph: &mut ModelGraph, name: &str, kind: ElementKind| -> ElementId {
            let def = Element::new_with_kind(kind).with_name(name);
            let def_id = graph.add_element(def);
            graph.add_element(
                Element::new_with_kind(ElementKind::AttributeUsage)
                    .with_name("payload")
                    .with_owner(def_id.clone()),
            );
            def_id
        };

        let _flow_def_id =
            make_def(&mut graph, "FlowType", ElementKind::FlowDefinition);
        let _iface_def_id =
            make_def(&mut graph, "IfaceType", ElementKind::InterfaceDefinition);
        let _alloc_def_id =
            make_def(&mut graph, "AllocType", ElementKind::AllocationDefinition);

        let make_usage =
            |graph: &mut ModelGraph, name: &str, kind: ElementKind, type_name: &str| {
                let usage = Element::new_with_kind(kind).with_name(name);
                let usage_id = graph.add_element(usage);
                graph.add_element(
                    Element::new_with_kind(ElementKind::FeatureTyping)
                        .with_owner(usage_id.clone())
                        .with_prop(
                            "unresolved_type",
                            Value::String(type_name.into()),
                        ),
                );
            };

        make_usage(&mut graph, "f", ElementKind::FlowUsage, "FlowType");
        make_usage(&mut graph, "i", ElementKind::InterfaceUsage, "IfaceType");
        make_usage(&mut graph, "a", ElementKind::AllocationUsage, "AllocType");

        let tree = model_tree_with_resolver(&graph, &graph, None, TreeView::Full);
        for usage_name in &["f", "i", "a"] {
            let n = tree
                .iter()
                .find(|n| n.name.as_deref() == Some(*usage_name))
                .unwrap_or_else(|| panic!("usage {usage_name} missing"));
            assert_eq!(n.archetype, Archetype::Connection);
            let names: Vec<_> = n
                .children
                .iter()
                .filter_map(|c| c.name.as_deref())
                .collect();
            assert!(
                names.contains(&"payload"),
                "{} missing inlined `payload`: {:?}",
                usage_name,
                names
            );
            assert!(
                n.default_collapsed,
                "{} should be default_collapsed=true",
                usage_name
            );
        }
    }

    #[test]
    fn test_model_tree_typed_def_inlining_handles_self_cycle() {
        // Commit 2 cycle guard test: a definition that transitively
        // references itself (`WireType` contains a usage typed by
        // `WireType`) must not blow the stack. The visited set in
        // `build_tree_node` blocks reentry within one chain; this
        // test exercises that path through the new connection
        // resolution.
        let mut graph = ModelGraph::new();

        let wire_def = Element::new_with_kind(ElementKind::ConnectionDefinition)
            .with_name("WireType");
        let wire_def_id = graph.add_element(wire_def);
        // `WireType` owns a `nested : WireType` ConnectionUsage —
        // direct self-reference.
        let nested = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("nested")
            .with_owner(wire_def_id.clone());
        let nested_id = graph.add_element(nested);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(nested_id.clone())
                .with_prop("unresolved_type", Value::String("WireType".into())),
        );

        // Top-level `link : WireType` — the recursion must terminate
        // when the inliner walks `WireType` → `nested : WireType` →
        // `WireType` (already visited).
        let link = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("link");
        let link_id = graph.add_element(link);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(link_id.clone())
                .with_prop("unresolved_type", Value::String("WireType".into())),
        );

        // If the cycle guard or the depth backstop is broken, this
        // call recurses unboundedly and panics on stack overflow.
        // Use UserFacing view so the FeatureTyping decoration child is
        // filtered out and the only thing under `nested` would be a
        // re-inlined cycle.
        let tree =
            model_tree_with_resolver(&graph, &graph, None, TreeView::UserFacing);
        let link_node = tree
            .iter()
            .find(|n| n.name.as_deref() == Some("link"))
            .expect("link root must survive");
        // First inlining level surfaces `nested`; the cycle guard then
        // blocks re-inlining `WireType` under `nested`, so `nested`
        // has no children (its FeatureTyping decoration is filtered
        // out by user-facing view, and the cycle guard prevents the
        // typed-def inliner from re-walking WireType's children).
        let nested_child = link_node
            .children
            .iter()
            .find(|c| c.name.as_deref() == Some("nested"))
            .expect("nested ConnectionUsage must surface once via inlining");
        let inlined_names: Vec<_> = nested_child
            .children
            .iter()
            .filter_map(|c| c.name.clone())
            .collect();
        assert!(
            !inlined_names.contains(&"nested".to_string()),
            "cycle guard should prevent infinite re-inlining of `nested`: got {:?}",
            inlined_names
        );
    }
}
