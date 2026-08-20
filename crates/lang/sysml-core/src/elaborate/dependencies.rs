//! Dependency + requirement-derivation elaboration (billet B1).
//!
//! Synthesizes the three requirement-relationship kinds the trace matrix
//! could not answer before:
//!
//! - **Trace** — a plain `Dependency` (SysML's generic trace mechanism;
//!   no endpoint-kind guard, the spec prescribes none). Edge:
//!   source = client, target = supplier.
//! - **Refine** — a `Dependency` annotated with the normative library's
//!   `ModelingMetadata::Refinement` metadata ("the source elements ...
//!   provide a more precise and/or accurate representation than the
//!   target elements", ModelingMetadata.sysml:132-141). The discriminator
//!   is the RESOLVED annotation type being the library element itself —
//!   never a name-string match (a user's own `metadata def Refinement`
//!   must not classify; it degrades to Trace, which stays correct).
//! - **Derive** — a connection typed (directly or transitively) by the
//!   normative `DerivationConnections::Derivation` abstract connection
//!   def. Ends discriminate as original vs derived by explicit subsetting
//!   of the library's `originalRequirements` / `derivedRequirements`
//!   features when authored; otherwise POSITIONALLY — KerML prescribes
//!   that an owned end with no explicit redefinition "implicitly
//!   redefine[s] the association end at the same position, in order, of
//!   the superclassifier", and `Derivation` declares
//!   `originalRequirement[1]` first, `derivedRequirements[1..*]` second.
//!   Edge per derived end: source = derived requirement, target =
//!   original requirement.
//!
//! When the standard library is not merged into the graph, the library
//! anchors don't exist: Refinement annotations are not recognized (the
//! Dependency still surfaces as Trace — degraded but correct) and
//! Derivation-typed connections are not classified. No soft fallback.

use std::collections::{HashSet, VecDeque};

use super::ElaborationReport;
use crate::resolution::{resolved_props, unresolved_props};
use crate::{
    CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Relationship, RelationshipKind,
    Value,
};

/// Library anchor elements the discriminators resolve against.
struct LibraryAnchors {
    /// `ModelingMetadata::Refinement` metadata def.
    refinement: Option<ElementId>,
    /// `DerivationConnections::Derivation` connection def + its base
    /// feature `derivations` (typing by either classifies).
    derivation: HashSet<ElementId>,
    /// `DerivationConnections::originalRequirement(s)` — explicit
    /// original-end subsetting targets.
    original_ends: HashSet<ElementId>,
    /// `DerivationConnections::derivedRequirements` — explicit
    /// derived-end subsetting target.
    derived_ends: HashSet<ElementId>,
}

impl LibraryAnchors {
    fn find(graph: &ModelGraph) -> Self {
        let mut refinement = None;
        let mut derivation = HashSet::new();
        let mut original_ends = HashSet::new();
        let mut derived_ends = HashSet::new();

        for e in graph.elements.values() {
            if !graph.is_library_element(&e.id) {
                continue;
            }
            match (e.name.as_deref(), &e.kind) {
                (Some("Refinement"), ElementKind::MetadataDefinition) => {
                    // Deterministic even in the (never expected) case of
                    // duplicates: smallest id wins.
                    match &refinement {
                        Some(prev) if *prev < e.id => {}
                        _ => refinement = Some(e.id.clone()),
                    }
                }
                (Some("Derivation"), ElementKind::ConnectionDefinition) => {
                    derivation.insert(e.id.clone());
                }
                (Some("derivations"), ElementKind::ConnectionUsage) => {
                    derivation.insert(e.id.clone());
                }
                (Some("originalRequirement"), _) | (Some("originalRequirements"), _) => {
                    if e.kind.is_usage() {
                        original_ends.insert(e.id.clone());
                    }
                }
                (Some("derivedRequirements"), _) => {
                    if e.kind.is_usage() {
                        derived_ends.insert(e.id.clone());
                    }
                }
                _ => {}
            }
        }

        LibraryAnchors {
            refinement,
            derivation,
            original_ends,
            derived_ends,
        }
    }
}

/// Elaborate Trace/Refine edges from Dependency elements and Derive edges
/// from Derivation-typed connections.
pub(super) fn elaborate_dependencies(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let anchors = LibraryAnchors::find(graph);
    synthesize_trace_refine(graph, &anchors, report);
    synthesize_derive(graph, &anchors, report);
}

/// Resolve the relationship-child reference of `child`: prefer the
/// resolution pass's `Value::Ref` prop, fall back to resolving the
/// unresolved name string from `scope` (parse-only graphs).
fn child_ref_target(
    graph: &ModelGraph,
    child: &Element,
    scope: &Option<ElementId>,
    resolved_prop: &str,
    unresolved_prop: &str,
) -> Option<ElementId> {
    if let Some(id) = child.get_prop(resolved_prop).and_then(|v| v.as_ref()) {
        return Some(id.clone());
    }
    let name = child.get_prop(unresolved_prop).and_then(|v| v.as_str())?;
    super::resolve_name(graph, scope, name)
}

/// Does `dep` carry a MetadataUsage child whose type resolves to THE
/// library `ModelingMetadata::Refinement` element?
fn is_refinement_annotated(
    graph: &ModelGraph,
    dep: &Element,
    refinement: &Option<ElementId>,
) -> bool {
    let Some(refinement_id) = refinement else {
        return false;
    };
    graph
        .children_of(&dep.id)
        .filter(|c| c.kind == ElementKind::MetadataUsage)
        .any(|meta| {
            // The parser mints a real FeatureTyping child on the metadata
            // usage (G16); resolution stamps `type`. Fall back to resolving
            // the raw annotation name for parse-only graphs.
            graph
                .children_of(&meta.id)
                .filter(|c| c.kind == ElementKind::FeatureTyping)
                .filter_map(|ft| {
                    child_ref_target(
                        graph,
                        ft,
                        &meta.owner,
                        resolved_props::TYPE,
                        unresolved_props::TYPE,
                    )
                })
                .chain(
                    meta.get_prop("unresolvedTypeName")
                        .and_then(|v| v.as_str())
                        .and_then(|n| super::resolve_name(graph, &dep.owner, n)),
                )
                .any(|id| &id == refinement_id)
        })
}

/// The full client or supplier name list of a Dependency. The parser's
/// lossless lowering contract is the list props (`unresolved_clients` /
/// `unresolved_suppliers`); the singular props carry only the first
/// endpoint (kept for older lowering vintages and hand-built graphs)
/// and are read only when the list prop is absent or empty.
fn dependency_endpoint_names(dep: &Element, list_prop: &str, single_prop: &str) -> Vec<String> {
    if let Some(items) = dep.get_prop(list_prop).and_then(|v| v.as_list()) {
        let names: Vec<String> = items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        if !names.is_empty() {
            return names;
        }
    }
    dep.get_prop(single_prop)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .into_iter()
        .collect()
}

/// Synthesize Trace (plain) / Refine (Refinement-annotated) edges from
/// Dependency elements. A Dependency relates ALL of its clients to ALL
/// of its suppliers ("one or more client Elements require one or more
/// supplier Elements", SysML-vocab.ttl; grammar: `client+ 'to'
/// supplier+`, SysML.xtext:55-61) — so the binary-edge projection is
/// the full client×supplier cross-product. Unresolvable endpoint names
/// skip that pair only (per-item resolve-or-skip, matching every
/// sibling elaboration pass; the loud variant is the E009
/// structural-check fast-follow).
fn synthesize_trace_refine(
    graph: &mut ModelGraph,
    anchors: &LibraryAnchors,
    report: &mut ElaborationReport,
) {
    let dep_ids: Vec<ElementId> = graph.element_ids_by_kind(&ElementKind::Dependency).to_vec();

    let mut to_mint: Vec<(RelationshipKind, ElementId, ElementId)> = Vec::new();
    for id in &dep_ids {
        let Some(dep) = graph.get_element(id) else {
            continue;
        };
        let client_names =
            dependency_endpoint_names(dep, "unresolved_clients", "unresolved_client");
        let supplier_names =
            dependency_endpoint_names(dep, "unresolved_suppliers", "unresolved_supplier");
        if client_names.is_empty() || supplier_names.is_empty() {
            continue;
        }
        let kind = if is_refinement_annotated(graph, dep, &anchors.refinement) {
            RelationshipKind::Refine
        } else {
            RelationshipKind::Trace
        };
        for client_name in &client_names {
            let Some(client_id) = super::resolve_name(graph, &dep.owner, client_name) else {
                continue;
            };
            for supplier_name in &supplier_names {
                let Some(supplier_id) = super::resolve_name(graph, &dep.owner, supplier_name)
                else {
                    continue;
                };
                to_mint.push((kind.clone(), client_id.clone(), supplier_id));
            }
        }
    }

    for (kind, source, target) in to_mint {
        mint_edge(graph, kind, source, target, report);
    }
}

/// Is `start` typed — directly or through a specialization/subsetting/
/// typing chain — by the library `Derivation` connection def (or its base
/// feature `derivations`)?
fn is_derivation_typed(graph: &ModelGraph, start: &Element, anchors: &LibraryAnchors) -> bool {
    if anchors.derivation.is_empty() {
        return false;
    }
    let mut visited: HashSet<ElementId> = HashSet::new();
    let mut queue: VecDeque<ElementId> = VecDeque::new();
    queue.push_back(start.id.clone());

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if anchors.derivation.contains(&id) {
            return true;
        }
        let Some(elem) = graph.get_element(&id) else {
            continue;
        };
        let hops: Vec<(&str, &str)> = vec![
            (resolved_props::TYPE, unresolved_props::TYPE),
            (resolved_props::GENERAL, unresolved_props::GENERAL),
            (
                resolved_props::SUPERCLASSIFIER,
                unresolved_props::SUPERCLASSIFIER,
            ),
            (
                resolved_props::SUBSETTED_FEATURE,
                unresolved_props::SUBSETTED_FEATURE,
            ),
        ];
        for child in graph.children_of(&id) {
            if !matches!(
                child.kind,
                ElementKind::FeatureTyping
                    | ElementKind::Specialization
                    | ElementKind::Subclassification
                    | ElementKind::Subsetting
            ) {
                continue;
            }
            for (resolved, unresolved) in &hops {
                if let Some(next) =
                    child_ref_target(graph, child, &elem.owner, resolved, unresolved)
                {
                    queue.push_back(next);
                }
            }
        }
    }
    false
}

/// End-role classification for a Derivation connection end.
enum EndRole {
    Original,
    Derived,
    Unmarked,
}

/// Explicit role from end subsetting/redefinition of the library's
/// `originalRequirement(s)` / `derivedRequirements` features. Written
/// against the spec rule even though the `::>`-on-ends authoring syntax
/// is a known parser gap (B1b) — when the grammar lands, this branch
/// activates without a matcher rewrite.
fn explicit_end_role(graph: &ModelGraph, end: &Element, anchors: &LibraryAnchors) -> EndRole {
    let hops: Vec<(&str, &str)> = vec![
        (
            resolved_props::SUBSETTED_FEATURE,
            unresolved_props::SUBSETTED_FEATURE,
        ),
        (
            resolved_props::REDEFINED_FEATURE,
            unresolved_props::REDEFINED_FEATURE,
        ),
    ];
    for child in graph.children_of(&end.id) {
        if !matches!(
            child.kind,
            ElementKind::Subsetting | ElementKind::Redefinition
        ) {
            continue;
        }
        for (resolved, unresolved) in &hops {
            if let Some(target) = child_ref_target(graph, child, &end.owner, resolved, unresolved)
            {
                if anchors.original_ends.contains(&target) {
                    return EndRole::Original;
                }
                if anchors.derived_ends.contains(&target) {
                    return EndRole::Derived;
                }
            }
        }
    }
    EndRole::Unmarked
}

/// The requirement an end references (its ReferenceSubsetting target).
fn end_requirement(graph: &ModelGraph, end: &Element) -> Option<ElementId> {
    graph
        .children_of(&end.id)
        .filter(|c| c.kind == ElementKind::ReferenceSubsetting)
        .find_map(|rs| {
            child_ref_target(
                graph,
                rs,
                &end.owner,
                resolved_props::REFERENCED_FEATURE,
                unresolved_props::REFERENCED_FEATURE,
            )
        })
}

/// Synthesize Derive edges from Derivation-typed connections.
fn synthesize_derive(
    graph: &mut ModelGraph,
    anchors: &LibraryAnchors,
    report: &mut ElaborationReport,
) {
    let mut conn_ids: Vec<ElementId> = Vec::new();
    conn_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConnectionUsage));
    conn_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConnectionDefinition));

    let mut tagged: Vec<ElementId> = Vec::new();
    let mut to_mint: Vec<(ElementId, ElementId)> = Vec::new(); // (derived, original)

    for id in &conn_ids {
        let Some(conn) = graph.get_element(id) else {
            continue;
        };
        if graph.is_library_element(id) {
            continue;
        }
        if !is_derivation_typed(graph, conn, anchors) {
            continue;
        }
        tagged.push(id.clone());

        // Ends in source order (the KerML positional rule keys on
        // declaration order; spans give it to us).
        let mut ends: Vec<&Element> = graph
            .children_of(id)
            .filter(|c| c.get_prop("isEnd").is_some())
            .collect();
        ends.sort_by_key(|e| e.spans.first().map(|s| s.start).unwrap_or(usize::MAX));

        // The DV001 arity check (runtime health) flags tagged connections
        // with fewer than 2 ends — never partially elaborate them here.
        if ends.len() < 2 {
            continue;
        }

        // Explicit roles first, positional fallback second.
        let mut original: Option<&Element> = None;
        let mut deriveds: Vec<&Element> = Vec::new();
        let mut unmarked: Vec<&Element> = Vec::new();
        for end in &ends {
            match explicit_end_role(graph, end, anchors) {
                EndRole::Original => original = Some(end),
                EndRole::Derived => deriveds.push(end),
                EndRole::Unmarked => unmarked.push(end),
            }
        }
        if original.is_none() && deriveds.is_empty() {
            // Fully positional: first end = originalRequirement[1].
            original = Some(unmarked[0]);
            deriveds = unmarked[1..].to_vec();
        } else {
            // Partially explicit: unmarked ends join the derived side
            // (Derivation has exactly one original).
            if original.is_none() {
                original = unmarked.first().copied();
                deriveds.extend(unmarked.iter().skip(1).copied());
            } else {
                deriveds.extend(unmarked);
            }
        }

        let Some(original) = original else { continue };
        let Some(original_req) = end_requirement(graph, original) else {
            continue;
        };
        for derived in deriveds {
            let Some(derived_req) = end_requirement(graph, derived) else {
                continue;
            };
            to_mint.push((derived_req, original_req.clone()));
        }
    }

    // Tag classified connections so the runtime health pass (DV001) can
    // check arity without re-deriving the library typing walk. Additive +
    // idempotent (never overwrites).
    for id in tagged {
        if let Some(elem) = graph.get_element_mut(&id) {
            if elem.get_prop("isDerivationConnection").is_none() {
                elem.set_prop("isDerivationConnection", Value::Bool(true));
                report.dependencies_elaborated += 1;
            }
        }
    }

    for (derived_req, original_req) in to_mint {
        mint_edge(
            graph,
            RelationshipKind::Derive,
            derived_req,
            original_req,
            report,
        );
    }
}

/// Idempotently mint a relationship edge with a reparse-stable id.
fn mint_edge(
    graph: &mut ModelGraph,
    kind: RelationshipKind,
    source: ElementId,
    target: ElementId,
    report: &mut ElaborationReport,
) {
    let already_exists = graph
        .relationships_by_kind(&kind)
        .any(|r| r.source == source && r.target == target);
    if already_exists {
        return;
    }
    let src_key = CanonicalKey::root(&source.to_string());
    let tgt_key = CanonicalKey::root(&target.to_string());
    let edge_key = CanonicalKey::for_relationship(&src_key, kind.as_str(), &tgt_key, 0);
    let rel = Relationship::new_with_key(kind, source, target, &edge_key);
    graph.add_relationship(rel);
    report.dependencies_elaborated += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;

    fn library_package(graph: &mut ModelGraph) -> ElementId {
        let pkg = Element::new_with_kind(ElementKind::LibraryPackage).with_name("TestLib");
        let pkg_id = graph.add_element(pkg);
        graph.register_library_package(pkg_id.clone());
        pkg_id
    }

    /// Minimal stand-ins for the normative library elements the
    /// discriminators anchor on.
    fn add_modeling_metadata(graph: &mut ModelGraph, lib: &ElementId) -> ElementId {
        let refinement = Element::new_with_kind(ElementKind::MetadataDefinition)
            .with_name("Refinement")
            .with_owner(lib.clone());
        graph.add_element(refinement)
    }

    fn add_derivation_connections(graph: &mut ModelGraph, lib: &ElementId) -> ElementId {
        let derivation = Element::new_with_kind(ElementKind::ConnectionDefinition)
            .with_name("Derivation")
            .with_owner(lib.clone());
        graph.add_element(derivation)
    }

    fn add_dependency(
        graph: &mut ModelGraph,
        owner: &ElementId,
        client: &str,
        supplier: &str,
    ) -> ElementId {
        let dep = Element::new_with_kind(ElementKind::Dependency)
            .with_owner(owner.clone())
            .with_prop("unresolved_client", client)
            .with_prop("unresolved_supplier", supplier);
        graph.add_element(dep)
    }

    #[test]
    fn plain_dependency_synthesizes_trace() {
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let a = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("a")
                .with_owner(pkg.clone()),
        );
        let b = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("b")
                .with_owner(pkg.clone()),
        );
        add_dependency(&mut graph, &pkg, "a", "b");

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Trace)
            .collect();
        assert_eq!(rels.len(), 1, "plain Dependency must mint one Trace edge");
        assert_eq!(rels[0].source, a, "source = client");
        assert_eq!(rels[0].target, b, "target = supplier");
    }

    #[test]
    fn refinement_annotated_dependency_synthesizes_refine() {
        let mut graph = ModelGraph::new();
        let lib = library_package(&mut graph);
        let refinement_def = add_modeling_metadata(&mut graph, &lib);

        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let a = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("a")
                .with_owner(pkg.clone()),
        );
        let b = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("b")
                .with_owner(pkg.clone()),
        );
        let dep = add_dependency(&mut graph, &pkg, "a", "b");
        let meta = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(dep.clone())
                .with_prop("unresolvedTypeName", "Refinement")
                .with_prop("annotationType", "Refinement"),
        );
        // The parser also mints a FeatureTyping child (G16); model it.
        let ft = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(meta.clone())
            .with_prop("unresolved_type", "Refinement");
        graph.add_element(ft);

        elaborate(&mut graph);

        let refines: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Refine)
            .collect();
        assert_eq!(refines.len(), 1, "annotated Dependency must mint Refine");
        assert_eq!(refines[0].source, a);
        assert_eq!(refines[0].target, b);
        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Trace)
                .count(),
            0,
            "a Refine-classified Dependency must not also mint Trace"
        );
        let _ = refinement_def;
    }

    #[test]
    fn user_defined_refinement_shadow_degrades_to_trace() {
        // A user's own `metadata def Refinement` (NOT a library element)
        // must not classify — name-string matching is exactly the
        // re-stringified-identity trap. Without the library anchor the
        // Dependency stays a Trace.
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        graph.add_element(
            Element::new_with_kind(ElementKind::MetadataDefinition)
                .with_name("Refinement")
                .with_owner(pkg.clone()),
        );
        let a = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("a")
                .with_owner(pkg.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("b")
                .with_owner(pkg.clone()),
        );
        let dep = add_dependency(&mut graph, &pkg, "a", "b");
        graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(dep)
                .with_prop("unresolvedTypeName", "Refinement"),
        );

        elaborate(&mut graph);

        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Refine)
                .count(),
            0,
            "user-defined Refinement must not classify as library Refine"
        );
        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Trace)
                .count(),
            1,
            "the Dependency still surfaces as Trace"
        );
        let _ = a;
    }

    #[test]
    fn derivation_typed_connection_synthesizes_derive_positionally() {
        let mut graph = ModelGraph::new();
        let lib = library_package(&mut graph);
        add_derivation_connections(&mut graph, &lib);

        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let orig = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("origReq")
                .with_owner(pkg.clone()),
        );
        let der1 = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("derReq1")
                .with_owner(pkg.clone()),
        );
        let der2 = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("derReq2")
                .with_owner(pkg.clone()),
        );

        let conn = graph.add_element(
            Element::new_with_kind(ElementKind::ConnectionUsage)
                .with_name("d")
                .with_owner(pkg.clone()),
        );
        let ft = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(conn.clone())
            .with_prop("unresolved_type", "Derivation");
        graph.add_element(ft);

        // Three ends in span order: original first (KerML positional rule).
        for (i, (end_name, req_name)) in [("e1", "origReq"), ("e2", "derReq1"), ("e3", "derReq2")]
            .iter()
            .enumerate()
        {
            let end = graph.add_element(
                Element::new_with_kind(ElementKind::ReferenceUsage)
                    .with_name(*end_name)
                    .with_owner(conn.clone())
                    .with_prop("isEnd", true)
                    .with_span(sysml_span::Span::new("file:///t.sysml", i * 10, i * 10 + 5)),
            );
            graph.add_element(
                Element::new_with_kind(ElementKind::ReferenceSubsetting)
                    .with_owner(end)
                    .with_prop("unresolved_referencedFeature", *req_name),
            );
        }

        elaborate(&mut graph);

        let derives: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Derive)
            .collect();
        assert_eq!(derives.len(), 2, "one Derive per derived end");
        for rel in &derives {
            assert_eq!(rel.target, orig, "target = original requirement");
        }
        let sources: HashSet<_> = derives.iter().map(|r| r.source.clone()).collect();
        assert!(sources.contains(&der1) && sources.contains(&der2));

        // The connection is tagged for the DV001 arity check.
        let conn_elem = graph.get_element(&conn).unwrap();
        assert_eq!(
            conn_elem
                .get_prop("isDerivationConnection")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn transitively_typed_connection_classifies() {
        // `connection def MyDeriv :> Derivation` + `connection x : MyDeriv`.
        let mut graph = ModelGraph::new();
        let lib = library_package(&mut graph);
        add_derivation_connections(&mut graph, &lib);

        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let my_deriv = graph.add_element(
            Element::new_with_kind(ElementKind::ConnectionDefinition)
                .with_name("MyDeriv")
                .with_owner(pkg.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::Subclassification)
                .with_owner(my_deriv.clone())
                .with_prop("unresolved_superclassifier", "Derivation"),
        );
        let conn = graph.add_element(
            Element::new_with_kind(ElementKind::ConnectionUsage)
                .with_name("x")
                .with_owner(pkg.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(conn.clone())
                .with_prop("unresolved_type", "MyDeriv"),
        );

        elaborate(&mut graph);

        assert_eq!(
            graph
                .get_element(&conn)
                .unwrap()
                .get_prop("isDerivationConnection")
                .and_then(|v| v.as_bool()),
            Some(true),
            "typing through a user subclass must classify"
        );
    }

    #[test]
    fn under_arity_derivation_mints_no_partial_edges() {
        let mut graph = ModelGraph::new();
        let lib = library_package(&mut graph);
        add_derivation_connections(&mut graph, &lib);

        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let orig = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("origReq")
                .with_owner(pkg.clone()),
        );
        let conn = graph.add_element(
            Element::new_with_kind(ElementKind::ConnectionUsage)
                .with_name("d")
                .with_owner(pkg.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(conn.clone())
                .with_prop("unresolved_type", "Derivation"),
        );
        let end = graph.add_element(
            Element::new_with_kind(ElementKind::ReferenceUsage)
                .with_owner(conn.clone())
                .with_prop("isEnd", true),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::ReferenceSubsetting)
                .with_owner(end)
                .with_prop("unresolved_referencedFeature", "origReq"),
        );

        elaborate(&mut graph);

        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Derive)
                .count(),
            0,
            "under-arity Derivation must not partially elaborate"
        );
        // ... but it IS tagged, so the DV001 health check can flag it.
        assert_eq!(
            graph
                .get_element(&conn)
                .unwrap()
                .get_prop("isDerivationConnection")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        let _ = orig;
    }

    /// Build a Dependency carrying the parser's full lowering shape:
    /// singular props = first endpoint (compat), list props = complete
    /// endpoint lists (the lossless contract).
    fn add_dependency_with_lists(
        graph: &mut ModelGraph,
        owner: &ElementId,
        clients: &[&str],
        suppliers: &[&str],
    ) -> ElementId {
        let mut dep = Element::new_with_kind(ElementKind::Dependency).with_owner(owner.clone());
        if let Some(first) = clients.first() {
            dep.set_prop("unresolved_client", Value::String((*first).to_owned()));
        }
        if let Some(first) = suppliers.first() {
            dep.set_prop("unresolved_supplier", Value::String((*first).to_owned()));
        }
        dep.set_prop(
            "unresolved_clients",
            Value::List(clients.iter().map(|c| Value::String((*c).to_owned())).collect()),
        );
        dep.set_prop(
            "unresolved_suppliers",
            Value::List(
                suppliers
                    .iter()
                    .map(|s| Value::String((*s).to_owned()))
                    .collect(),
            ),
        );
        graph.add_element(dep)
    }

    #[test]
    fn multi_endpoint_dependency_mints_client_x_supplier_cross_product() {
        // `dependency a1, a2 to b1, b2;` — a Dependency relates ALL
        // clients to ALL suppliers, so the binary projection is 4 Trace
        // edges. The parser sets BOTH the singular (first endpoint) and
        // list props; the list must win and nothing may double-mint.
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let mut ids = Vec::new();
        for name in ["a1", "a2", "b1", "b2"] {
            ids.push(graph.add_element(
                Element::new_with_kind(ElementKind::RequirementUsage)
                    .with_name(name)
                    .with_owner(pkg.clone()),
            ));
        }
        add_dependency_with_lists(&mut graph, &pkg, &["a1", "a2"], &["b1", "b2"]);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Trace)
            .collect();
        assert_eq!(rels.len(), 4, "2 clients × 2 suppliers = 4 Trace edges");
        let pairs: HashSet<(ElementId, ElementId)> = rels
            .iter()
            .map(|r| (r.source.clone(), r.target.clone()))
            .collect();
        for client in &ids[0..2] {
            for supplier in &ids[2..4] {
                assert!(
                    pairs.contains(&(client.clone(), supplier.clone())),
                    "missing client×supplier pair"
                );
            }
        }
    }

    #[test]
    fn multi_endpoint_refinement_dependency_mints_refine_for_every_pair() {
        // The @Refinement annotation classifies the WHOLE Dependency —
        // every client×supplier pair mints Refine, none mint Trace.
        let mut graph = ModelGraph::new();
        let lib = library_package(&mut graph);
        add_modeling_metadata(&mut graph, &lib);

        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        for name in ["a1", "a2", "b1"] {
            graph.add_element(
                Element::new_with_kind(ElementKind::RequirementUsage)
                    .with_name(name)
                    .with_owner(pkg.clone()),
            );
        }
        let dep = add_dependency_with_lists(&mut graph, &pkg, &["a1", "a2"], &["b1"]);
        graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(dep)
                .with_prop("unresolvedTypeName", "Refinement"),
        );

        elaborate(&mut graph);

        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Refine)
                .count(),
            2,
            "2 clients × 1 supplier = 2 Refine edges"
        );
        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Trace)
                .count(),
            0,
            "a Refine-classified Dependency must not also mint Trace"
        );
    }

    #[test]
    fn unresolvable_endpoint_skips_only_its_pairs() {
        // One bogus client: its pairs are skipped, the resolvable
        // client's pairs still mint (per-item resolve-or-skip).
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let a1 = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("a1")
                .with_owner(pkg.clone()),
        );
        let b1 = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("b1")
                .with_owner(pkg.clone()),
        );
        add_dependency_with_lists(&mut graph, &pkg, &["a1", "nonexistent"], &["b1"]);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Trace)
            .collect();
        assert_eq!(rels.len(), 1, "only the resolvable pair mints");
        assert_eq!(rels[0].source, a1);
        assert_eq!(rels[0].target, b1);
    }

    #[test]
    fn multi_endpoint_idempotent() {
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        for name in ["a1", "a2", "b1", "b2"] {
            graph.add_element(
                Element::new_with_kind(ElementKind::RequirementUsage)
                    .with_name(name)
                    .with_owner(pkg.clone()),
            );
        }
        add_dependency_with_lists(&mut graph, &pkg, &["a1", "a2"], &["b1", "b2"]);

        elaborate(&mut graph);
        let count_1 = graph.relationships_by_kind(&RelationshipKind::Trace).count();
        elaborate(&mut graph);
        let count_2 = graph.relationships_by_kind(&RelationshipKind::Trace).count();
        assert_eq!(count_1, 4);
        assert_eq!(count_1, count_2, "must not duplicate cross-product edges");
    }

    #[test]
    fn idempotent() {
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("a")
                .with_owner(pkg.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("b")
                .with_owner(pkg.clone()),
        );
        add_dependency(&mut graph, &pkg, "a", "b");

        elaborate(&mut graph);
        let count_1 = graph.relationships_by_kind(&RelationshipKind::Trace).count();
        elaborate(&mut graph);
        let count_2 = graph.relationships_by_kind(&RelationshipKind::Trace).count();
        assert_eq!(count_1, 1);
        assert_eq!(count_1, count_2, "must not duplicate Trace edges");
    }
}
