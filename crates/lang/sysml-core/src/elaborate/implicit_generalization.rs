//! Implicit generalization (IG-1: core machinery + `base`-kind rules).
//!
//! Every typed element in SysML v2/KerML implicitly specializes a standard-
//! library base type *without* the model writing an explicit `:>` — e.g. a
//! `ConnectionUsage` implicitly subsets `Connections::connections`, a
//! `PartDefinition` implicitly subclassifies `Parts::Part`, every `Classifier`
//! subclassifies `Base::Anything`, every `Feature` subsets `Base::things`.
//!
//! Because the base types themselves specialize deeper bases (Connection →
//! Link, etc.), the implicit edge transitively provides inherited features like
//! `participant`/`source`/`target`/`annotatedElement`. Those edges feed the
//! existing `InheritanceIndex` (resolution/context.rs) so inherited-member
//! lookups pick them up with no resolver change.
//!
//! This is a faithful port of the pilot implementation's
//! `ImplicitGeneralizationMap` (the `*_base` rows) + `TypeAdapter`
//! (`computeImplicitGeneralTypes` / `addImplicitGeneralType` guards /
//! `removeUnnecessaryImplicitGeneralTypes` suppression).
//!
//! ## Scope (IG-1)
//!
//! - Only the default `kind = "base"` row for every element class.
//! - Mints **Subclassification** (`superclassifier`) for Classifiers/Definitions
//!   and **Subsetting** (`subsettedFeature`) for Features/Usages.
//! - All other rows (`binary`, `subpart`, `subaction`, `entry`/`do`/`exit`
//!   redefinitions, the secondary stacked specializations, etc.) are deferred to
//!   IG-2/IG-3.
//!
//! ## Idempotence
//!
//! Minted relationship elements use a `CanonicalKey` derived from
//! `(specific, kind, general)`, so re-running the pass produces the same
//! `ElementId` and the duplicate guard makes it a no-op.

use super::ElaborationReport;
use crate::resolution::resolved_props;
use crate::{CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Value};

/// Marker property set on every implicitly-minted specialization element so it
/// is distinguishable from author-written specializations. Mirrors the pilot's
/// `isImplied` flag on `insertImplicitSpecializations`.
pub const IS_IMPLIED: &str = "isImplied";

/// The default (`kind = "base"`) implicit-generalization rule for an element
/// kind: the qualified name of the standard-library base type it specializes.
///
/// Transcribed from the pilot's `ImplicitGeneralizationMap` `*_base` rows
/// (`org.omg.sysml/.../util/ImplicitGeneralizationMap.java`). Only the `base`
/// kind is implemented in IG-1.
///
/// Returns `None` for kinds with no `base` row (e.g. plain `Multiplicity`,
/// `Succession`, and `BindingConnector` only have non-`base` rows, deferred to
/// IG-2).
fn base_rule(kind: &ElementKind) -> Option<&'static str> {
    use ElementKind as K;
    let qname = match kind {
        // ----- KerML -----
        K::Association => "Links::Link",
        K::AssociationStructure => "Objects::LinkObject",
        K::Behavior => "Performances::Performance",
        K::BooleanExpression => "Performances::booleanEvaluations",
        K::Class => "Occurrences::Occurrence",
        K::Classifier => "Base::Anything",
        K::Connector => "Links::links",
        K::ConstructorExpression => "Performances::constructorEvaluations",
        K::DataType => "Base::DataValue",
        K::Expression => "Performances::evaluations",
        K::Feature => "Base::things",
        K::Function => "Performances::Evaluation",
        K::Invariant => "Performances::trueEvaluations",
        K::Flow => "Transfers::transfers",
        K::LiteralBoolean => "Performances::literalBooleanEvaluations",
        K::LiteralExpression => "Performances::literalEvaluations",
        K::LiteralInfinity => "Performances::literalIntegerEvaluations",
        K::LiteralInteger => "Performances::literalIntegerEvaluations",
        K::LiteralRational => "Performances::literalRationalEvaluations",
        K::LiteralString => "Performances::literalStringEvaluations",
        K::Metaclass => "Metaobjects::Metaobject",
        K::MetadataFeature => "Metaobjects::metaobjects",
        K::MetadataAccessExpression => "Performances::metadataAccessEvaluations",
        K::Multiplicity => "Base::naturals",
        K::NullExpression => "Performances::nullEvaluations",
        K::Predicate => "Performances::BooleanEvaluation",
        K::Step => "Performances::performances",
        K::Structure => "Objects::Object",
        K::SuccessionFlow => "Transfers::flowTransfersBefore",
        K::Type => "Base::Anything",

        // ----- SysML -----
        K::AcceptActionUsage => "Actions::acceptActions",
        K::ActionDefinition => "Actions::Action",
        K::ActionUsage => "Actions::actions",
        K::AllocationDefinition => "Allocations::Allocation",
        K::AllocationUsage => "Allocations::allocations",
        K::AnalysisCaseDefinition => "AnalysisCases::AnalysisCase",
        K::AnalysisCaseUsage => "AnalysisCases::analysisCases",
        K::AssertConstraintUsage => "Constraints::assertedConstraintChecks",
        K::AssignmentActionUsage => "Actions::assignmentActions",
        K::AttributeDefinition => "Base::DataValue",
        K::AttributeUsage => "Base::dataValues",
        K::BindingConnectorAsUsage => "Links::selfLinks",
        K::CalculationDefinition => "Calculations::Calculation",
        K::CalculationUsage => "Calculations::calculations",
        K::CaseDefinition => "Cases::Case",
        K::CaseUsage => "Cases::cases",
        K::ConcernDefinition => "Requirements::ConcernCheck",
        K::ConcernUsage => "Requirements::concernChecks",
        K::ConnectionDefinition => "Connections::Connection",
        K::ConnectionUsage => "Connections::connections",
        K::ConstraintDefinition => "Constraints::ConstraintCheck",
        K::ConstraintUsage => "Constraints::constraintChecks",
        K::FlowDefinition => "Flows::MessageAction",
        K::FlowUsage => "Flows::flows",
        K::ForLoopActionUsage => "Actions::forLoopActions",
        K::IfActionUsage => "Actions::ifThenActions",
        K::InterfaceDefinition => "Interfaces::Interface",
        K::InterfaceUsage => "Interfaces::interfaces",
        K::ItemDefinition => "Items::Item",
        K::ItemUsage => "Items::items",
        K::MetadataDefinition => "Metadata::MetadataItem",
        K::MetadataUsage => "Metadata::metadataItems",
        K::OccurrenceDefinition => "Occurrences::Occurrence",
        K::OccurrenceUsage => "Occurrences::occurrences",
        K::PartDefinition => "Parts::Part",
        K::PartUsage => "Parts::parts",
        K::PortDefinition => "Ports::Port",
        K::PortUsage => "Ports::ports",
        K::RenderingDefinition => "Views::Rendering",
        K::RenderingUsage => "Views::renderings",
        K::RequirementDefinition => "Requirements::RequirementCheck",
        K::RequirementUsage => "Requirements::requirementChecks",
        K::SatisfyRequirementUsage => "Requirements::satisfiedRequirementChecks",
        K::SendActionUsage => "Actions::sendActions",
        K::StateDefinition => "States::StateAction",
        K::StateUsage => "States::stateActions",
        K::SuccessionAsUsage => "Occurrences::happensBeforeLinks",
        K::SuccessionFlowUsage => "Flows::successionFlows",
        K::TerminateActionUsage => "Actions::terminateActions",
        K::TransitionUsage => "Actions::transitionActions",
        K::UseCaseDefinition => "UseCases::UseCase",
        K::UseCaseUsage => "UseCases::useCases",
        K::VerificationCaseDefinition => "VerificationCases::VerificationCase",
        K::VerificationCaseUsage => "VerificationCases::verificationCases",
        K::ViewDefinition => "Views::View",
        K::ViewUsage => "Views::views",
        K::ViewpointDefinition => "Views::ViewpointCheck",
        K::ViewpointUsage => "Views::viewpointChecks",
        K::WhileLoopActionUsage => "Actions::whileLoopActions",

        _ => return None,
    };
    Some(qname)
}

/// A planned implicit specialization edge, collected before mutating the graph.
struct PlannedEdge {
    /// The user element gaining the implicit base specialization.
    specific: ElementId,
    /// The resolved standard-library base type.
    general: ElementId,
    /// The relationship element kind (`Subclassification` or `Subsetting`).
    rel_kind: ElementKind,
    /// The property the resolved general id is written under
    /// (`superclassifier` or `subsettedFeature`).
    resolved_prop: &'static str,
}

/// Elaborate implicit generalizations (IG-1: `base`-kind rules only).
///
/// `library` is the standard-library graph kept as a linked / fallback graph
/// (NOT merged). Base qualified names (`Connections::Connection`, …) resolve
/// against it. When `None` (e.g. the no-library `elaborate()` entry point) the
/// pass resolves only against the user graph itself — most bases live in the
/// stdlib so it is largely a no-op without the library, which is the correct
/// behaviour (silently skip unresolved bases).
pub(super) fn elaborate_implicit_generalization(
    graph: &mut ModelGraph,
    library: Option<&ModelGraph>,
    lib_inheritance_index: Option<&std::sync::Arc<crate::resolution::InheritanceIndex>>,
    report: &mut ElaborationReport,
) {
    // ---- 1. Collect candidate (element, base-rule) pairs. ----
    // Snapshot ids first so we can resolve (immutable borrow) then mutate.
    let candidates: Vec<(ElementId, ElementKind, &'static str)> = graph
        .elements
        .values()
        .filter_map(|e| {
            let qname = base_rule(&e.kind)?;
            // Only real Types get implicit generals.
            if !(e.kind.is_classifier() || e.kind.is_feature()) {
                return None;
            }
            // Skip conjugated types (pilot: `!getTarget().isConjugated()`).
            if is_conjugated(graph, &e.id, &e.kind) {
                return None;
            }
            Some((e.id.clone(), e.kind.clone(), qname))
        })
        .collect();

    // ---- 2. Resolve each base qname → ElementId, build the plan with guards. ----
    let mut planned: Vec<PlannedEdge> = Vec::new();
    for (specific, kind, qname) in candidates {
        // Resolve the qualified base name against the library (fallback) graph,
        // falling back to the user graph alone when no library is supplied.
        let general = resolve_base(graph, library, lib_inheritance_index, qname);
        let Some(general) = general else {
            // Guard: base unresolved → silently skip (pilot: null general dropped).
            continue;
        };

        // Guard: self-reference (pilot: `general != getTarget()`).
        if general == specific {
            continue;
        }

        // Pick the relationship kind: Subclassification for classifiers,
        // Subsetting for features (pilot `getSpecializationEClass`).
        let (rel_kind, resolved_prop) = if kind.is_classifier() {
            (ElementKind::Subclassification, resolved_props::SUPERCLASSIFIER)
        } else {
            (ElementKind::Subsetting, resolved_props::SUBSETTED_FEATURE)
        };

        // Suppression (pilot `removeUnnecessaryImplicitGeneralTypes`,
        // simplified for IG-1): skip the implicit base `G` if an EXPLICIT
        // specialization already transitively specializes `G`.
        if already_specializes(graph, library, &specific, &general) {
            continue;
        }

        planned.push(PlannedEdge {
            specific,
            general,
            rel_kind,
            resolved_prop,
        });
    }

    // ---- 3. Mint the edges (idempotent: dedup on canonical key + IS_IMPLIED). ----
    for edge in planned {
        if implicit_edge_exists(graph, &edge) {
            continue;
        }
        mint_implicit_edge(graph, &edge);
        report.implicit_generalizations_minted += 1;
    }
}

/// Resolve a qualified base name (`Connections::Connection`) to an `ElementId`.
///
/// Uses the dual-graph (`new_with_fallback`) resolver so library base types are
/// reachable without merging the library into the user graph. Implicit-
/// generalization base names are spec-legal global library references, so the
/// import-gate does NOT apply (the gate only blocks the *bare-name member
/// sweep*, never qualified library names).
fn resolve_base(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    lib_inheritance_index: Option<&std::sync::Arc<crate::resolution::InheritanceIndex>>,
    qname: &str,
) -> Option<ElementId> {
    use crate::resolution::ResolutionContext;
    // Bases live in the standard library, which is self-contained: resolve the
    // qualified name *within* the library graph directly. This is the common
    // (and fast) path and avoids cross-graph scope-table quirks of the
    // dual-graph resolver for subsequent path segments.
    //
    // When a prebuilt library inheritance index is provided, reuse it via the
    // `*_with_lib_inheritance_index` ctors so each candidate-resolution context
    // skips the O(|library|) `collect_specializations` rebuild. The library-
    // only ctor stores the `Arc` directly (refcount bump). May 29 perf baseline:
    // the rebuild was 39.4 % exclusive on workspace elaborate runs.
    if let Some(lib) = library {
        let id = match lib_inheritance_index {
            Some(idx) => ResolutionContext::new_with_lib_inheritance_index(lib, idx.clone())
                .resolve_qualified_name_global(qname),
            None => ResolutionContext::new(lib).resolve_qualified_name_global(qname),
        };
        if let Some(id) = id {
            return Some(id);
        }
    }
    // Fall back to dual-graph resolution (covers user-graph-local bases and the
    // no-library case). The dual-graph ctor still pays one user-overlay clone
    // of the lib map per IG-1 call — accepted because this is the less-common
    // path and the user overlay must not be cached across files.
    let mut ctx = match (library, lib_inheritance_index) {
        (Some(lib), Some(idx)) => {
            ResolutionContext::new_with_fallback_and_lib_inheritance_index(graph, lib, &**idx)
        }
        (Some(lib), None) => ResolutionContext::new_with_fallback(graph, lib),
        (None, _) => ResolutionContext::new(graph),
    };
    ctx.resolve_qualified_name_global(qname)
}

/// Whether `specific` already (transitively, via EXPLICIT specializations)
/// specializes `general`. Used for the IG-1 suppression rule.
///
/// Walks the explicit (non-implied) specialization edges of `specific` and
/// asks whether any general transitively reaches `general` through the combined
/// user+library inheritance index. Mirrors the pilot's
/// `specializesExcludingTarget` (target pre-visited to tolerate cycles).
fn already_specializes(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    specific: &ElementId,
    general: &ElementId,
) -> bool {
    // Collect explicit direct supertypes of `specific` from its owned
    // specialization-family children (NOT implicit ones we may have minted).
    let explicit_generals: Vec<ElementId> = graph
        .children_of(specific)
        .filter(|child| {
            (child.kind == ElementKind::Specialization
                || child.kind.is_subtype_of(ElementKind::Specialization))
                && child.get_prop(IS_IMPLIED).and_then(|v| v.as_bool()) != Some(true)
        })
        .filter_map(|child| specialization_resolved_target(child))
        .collect();

    if explicit_generals.is_empty() {
        return false;
    }

    // Direct hit: an explicit general IS the base.
    if explicit_generals.iter().any(|g| g == general) {
        return true;
    }

    // Transitive: does any explicit general specialize `general`?
    // Walk the combined inheritance graph (user + library) with a visited set
    // seeded with `specific` (pilot: target pre-visited for cycle tolerance).
    let mut visited = std::collections::HashSet::new();
    visited.insert(specific.clone());
    for g in &explicit_generals {
        if specializes(graph, library, g, general, &mut visited) {
            return true;
        }
    }
    false
}

/// Transitive specialization check over the combined user+library graph.
fn specializes(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    subtype: &ElementId,
    supertype: &ElementId,
    visited: &mut std::collections::HashSet<ElementId>,
) -> bool {
    if subtype == supertype {
        return true;
    }
    if !visited.insert(subtype.clone()) {
        return false;
    }
    for g in direct_supertypes(graph, library, subtype) {
        if specializes(graph, library, &g, supertype, visited) {
            return true;
        }
    }
    false
}

/// Direct supertypes of `id`, read from owned specialization-family children in
/// whichever graph (user or library) actually owns `id`.
fn direct_supertypes(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    id: &ElementId,
) -> Vec<ElementId> {
    let mut out = Vec::new();
    collect_direct_supertypes(graph, id, &mut out);
    if let Some(lib) = library {
        collect_direct_supertypes(lib, id, &mut out);
    }
    out
}

fn collect_direct_supertypes(g: &ModelGraph, id: &ElementId, out: &mut Vec<ElementId>) {
    for child in g.children_of(id) {
        if child.kind == ElementKind::Specialization
            || child.kind.is_subtype_of(ElementKind::Specialization)
        {
            if let Some(target) = specialization_resolved_target(child) {
                out.push(target);
            }
        }
    }
}

/// Read the resolved supertype/target id from a specialization-family element,
/// using the property pair appropriate for its concrete kind.
fn specialization_resolved_target(rel: &Element) -> Option<ElementId> {
    let key = if rel.kind == ElementKind::Redefinition
        || rel.kind.is_subtype_of(ElementKind::Redefinition)
    {
        resolved_props::REDEFINED_FEATURE
    } else if rel.kind == ElementKind::Subsetting
        || rel.kind.is_subtype_of(ElementKind::Subsetting)
    {
        resolved_props::SUBSETTED_FEATURE
    } else if rel.kind == ElementKind::Subclassification
        || rel.kind.is_subtype_of(ElementKind::Subclassification)
    {
        resolved_props::SUPERCLASSIFIER
    } else if rel.kind == ElementKind::FeatureTyping
        || rel.kind.is_subtype_of(ElementKind::FeatureTyping)
    {
        resolved_props::TYPE
    } else {
        resolved_props::GENERAL
    };
    rel.get_prop(key).and_then(|v| v.as_ref()).cloned()
}

/// Whether an implicit base edge of this exact shape already exists on the
/// element (idempotence / dedup guard).
fn implicit_edge_exists(graph: &ModelGraph, edge: &PlannedEdge) -> bool {
    graph.children_of(&edge.specific).any(|child| {
        child.kind == edge.rel_kind
            && child.get_prop(IS_IMPLIED).and_then(|v| v.as_bool()) == Some(true)
            && child.get_prop(edge.resolved_prop).and_then(|v| v.as_ref()) == Some(&edge.general)
    })
}

/// Mint a single implicit specialization element owned by `specific`,
/// targeting `general`, flagged `isImplied = true`.
fn mint_implicit_edge(graph: &mut ModelGraph, edge: &PlannedEdge) {
    let src_key = CanonicalKey::root(&edge.specific.to_string());
    let tgt_key = CanonicalKey::root(&edge.general.to_string());
    let key = CanonicalKey::for_relationship(&src_key, edge.rel_kind.as_str(), &tgt_key, 0);

    let mut rel = Element::new_with_key(edge.rel_kind.clone(), &key)
        .with_owner(edge.specific.clone())
        .with_prop(IS_IMPLIED, true)
        .with_prop("specific", Value::Ref(edge.specific.clone()))
        .with_prop(edge.resolved_prop, Value::Ref(edge.general.clone()));
    // `general` is the universal alias the resolver also reads for plain
    // Specialization; set it too so any consumer keyed on `general` sees the edge.
    rel.set_prop(resolved_props::GENERAL, Value::Ref(edge.general.clone()));

    graph.add_element(rel);
}

/// Whether a type element is conjugated, so it gets NO implicit generals
/// (pilot: `computeImplicitGeneralTypes` skips when `getTarget().isConjugated()`).
///
/// IG-1 simplification: SysML conjugation is modeled either as the dedicated
/// `ConjugatedPortDefinition` kind or as a `Conjugation` relationship element
/// (`conjugatedType` pointing at the type). We detect both. (Conjugated port
/// *typing* on usages is rare and deferred to IG-2.)
fn is_conjugated(graph: &ModelGraph, id: &ElementId, kind: &ElementKind) -> bool {
    if *kind == ElementKind::ConjugatedPortDefinition {
        return true;
    }
    graph.children_of(id).any(|child| {
        (child.kind == ElementKind::Conjugation
            || child.kind.is_subtype_of(ElementKind::Conjugation))
            && child.get_prop("conjugatedType").and_then(|v| v.as_ref()) == Some(id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate_with_library;
    use crate::VisibilityKind;

    /// Build a minimal library graph with the base types we need for tests,
    /// arranged as `Connections::Connection :> Links::Link` so the transitive
    /// inherited-feature chain is exercised, plus `Parts::Part`.
    ///
    /// Uses `add_owned_element` so OwningMembership elements exist — resolution
    /// scope tables are membership-driven, mirroring real parsed graphs.
    fn build_library() -> (ModelGraph, ElementId, ElementId, ElementId) {
        let mut lib = ModelGraph::new();
        let vis = VisibilityKind::Public;

        // package Links
        let links_id = lib.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Links"),
        );
        lib.register_library_package(links_id.clone());

        // class Link { feature participant; }
        let link_id = lib.add_owned_element(
            Element::new_with_kind(ElementKind::Classifier).with_name("Link"),
            links_id.clone(),
            vis,
        );
        lib.add_owned_element(
            Element::new_with_kind(ElementKind::Feature).with_name("participant"),
            link_id.clone(),
            vis,
        );

        // feature links;
        let links_feat_id = lib.add_owned_element(
            Element::new_with_kind(ElementKind::Feature).with_name("links"),
            links_id.clone(),
            vis,
        );
        let _ = links_feat_id;

        // package Connections
        let connections_id = lib.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Connections"),
        );
        lib.register_library_package(connections_id.clone());

        // connection def Connection :> Links::Link
        let connection_id = lib.add_owned_element(
            Element::new_with_kind(ElementKind::ConnectionDefinition).with_name("Connection"),
            connections_id.clone(),
            vis,
        );
        // explicit Subclassification Connection :> Link (resolved target).
        lib.add_owned_element(
            Element::new_with_kind(ElementKind::Subclassification)
                .with_prop("superclassifier", Value::Ref(link_id.clone())),
            connection_id.clone(),
            vis,
        );

        // connections : Connection (the usage-level base for ConnectionUsage).
        let connections_feat_id = lib.add_owned_element(
            Element::new_with_kind(ElementKind::Feature).with_name("connections"),
            connections_id.clone(),
            vis,
        );
        // Type `connections` with Connection so its inherited chain reaches
        // Link (and the inherited `participant` becomes reachable).
        lib.add_owned_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_prop("type", Value::Ref(connection_id.clone())),
            connections_feat_id.clone(),
            vis,
        );
        // Also give `connections` a directly-owned `participant` feature so the
        // IG-1 implicit-base edge alone (ConnectionUsage :> connections) makes an
        // inherited member reachable without depending on the deeper
        // typing-hop chain (Connection :> Link), which the inherited-member walk
        // treats separately.
        lib.add_owned_element(
            Element::new_with_kind(ElementKind::Feature).with_name("participant"),
            connections_feat_id.clone(),
            vis,
        );

        // package Parts { part def Part; }
        let parts_id = lib.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Parts"),
        );
        lib.register_library_package(parts_id.clone());
        let part_id = lib.add_owned_element(
            Element::new_with_kind(ElementKind::PartDefinition).with_name("Part"),
            parts_id.clone(),
            vis,
        );

        lib.rebuild_indexes();
        lib.ensure_library_index();
        (lib, connection_id, connections_feat_id, part_id)
    }

    #[test]
    fn connection_usage_gets_implicit_subsetting_to_connections() {
        let (lib, _conn_def, connections_feat_id, _part) = build_library();

        let mut graph = ModelGraph::new();
        let cu = Element::new_with_kind(ElementKind::ConnectionUsage).with_name("link1");
        let cu_id = graph.add_element(cu);

        let mut report = ElaborationReport::default();
        elaborate_implicit_generalization(&mut graph, Some(&lib), None, &mut report);
        assert!(report.implicit_generalizations_minted >= 1);

        // The minted edge is a Subsetting → Connections::connections, isImplied.
        let edge = graph
            .children_of(&cu_id)
            .find(|c| c.kind == ElementKind::Subsetting)
            .expect("implicit Subsetting minted");
        assert_eq!(edge.get_prop(IS_IMPLIED).and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            edge.get_prop("subsettedFeature").and_then(|v| v.as_ref()),
            Some(&connections_feat_id)
        );
    }

    #[test]
    fn connection_usage_resolves_inherited_participant() {
        let (lib, _conn_def, _connections_feat, _part) = build_library();

        // package P { connection link1 { ref inner; } } — resolve `participant`
        // from inside the connection, so the containing type the inherited-member
        // walk searches is the ConnectionUsage (mirrors a real reference site:
        // per the KerML 8.2.3.5.1 redefinedFeature rule the local namespaces
        // are the generals of the redefining feature's OWNER, so we resolve
        // from a nested child rather than from the usage element itself).
        let mut graph = ModelGraph::new();
        let pkg_id =
            graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let cu_id = graph.add_owned_element(
            Element::new_with_kind(ElementKind::ConnectionUsage).with_name("link1"),
            pkg_id.clone(),
            VisibilityKind::Public,
        );
        let inner_id = graph.add_owned_element(
            Element::new_with_kind(ElementKind::Feature).with_name("inner"),
            cu_id.clone(),
            VisibilityKind::Public,
        );

        let mut report = ElaborationReport::default();
        elaborate_implicit_generalization(&mut graph, Some(&lib), None, &mut report);
        graph.rebuild_indexes();

        // With the implicit base specialization (link1 :> Connections::connections)
        // in place, the inherited `participant` (owned by `connections`) is
        // reachable through the supertype walk from inside the connection.
        let mut ctx = crate::resolution::ResolutionContext::new_with_fallback(&graph, &lib);
        let resolved = ctx.resolve_redefined_feature(&inner_id, "participant");
        assert!(
            resolved.is_some(),
            "inherited `participant` should resolve via the implicit base"
        );
    }

    #[test]
    fn part_definition_gets_implicit_subclassification_to_part() {
        let (lib, _conn, _connections_feat, part_id) = build_library();

        let mut graph = ModelGraph::new();
        let pd = Element::new_with_kind(ElementKind::PartDefinition).with_name("Engine");
        let pd_id = graph.add_element(pd);

        let mut report = ElaborationReport::default();
        elaborate_implicit_generalization(&mut graph, Some(&lib), None, &mut report);

        let edge = graph
            .children_of(&pd_id)
            .find(|c| c.kind == ElementKind::Subclassification)
            .expect("implicit Subclassification minted");
        assert_eq!(edge.get_prop(IS_IMPLIED).and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            edge.get_prop("superclassifier").and_then(|v| v.as_ref()),
            Some(&part_id)
        );
    }

    #[test]
    fn suppression_skips_base_when_explicit_subtype_present() {
        let (lib, conn_def, _connections_feat, _part) = build_library();

        // A ConnectionDefinition that already EXPLICITLY subclassifies
        // Connections::Connection should NOT also get the implicit base
        // Subclassification to Connection (it already specializes it directly).
        let mut graph = ModelGraph::new();
        let cd = Element::new_with_kind(ElementKind::ConnectionDefinition).with_name("MyConn");
        let cd_id = graph.add_element(cd);
        // explicit Subclassification MyConn :> Connections::Connection (resolved)
        let explicit = Element::new_with_kind(ElementKind::Subclassification)
            .with_owner(cd_id.clone())
            .with_prop("superclassifier", Value::Ref(conn_def.clone()));
        graph.add_element(explicit);

        let mut report = ElaborationReport::default();
        elaborate_implicit_generalization(&mut graph, Some(&lib), None, &mut report);

        // No implicit edge minted: the explicit one already specializes the base.
        let implicit_count = graph
            .children_of(&cd_id)
            .filter(|c| {
                c.kind == ElementKind::Subclassification
                    && c.get_prop(IS_IMPLIED).and_then(|v| v.as_bool()) == Some(true)
            })
            .count();
        assert_eq!(implicit_count, 0, "explicit subtype should suppress implicit base");
    }

    #[test]
    fn idempotent_runs_mint_once() {
        let (lib, _conn, _connections_feat, _part) = build_library();

        let mut graph = ModelGraph::new();
        let pd = Element::new_with_kind(ElementKind::PartDefinition).with_name("Engine");
        let pd_id = graph.add_element(pd);

        let r1 = elaborate_with_library(&mut graph, Some(&lib), None);
        let first = r1.implicit_generalizations_minted;
        assert!(first >= 1);

        let r2 = elaborate_with_library(&mut graph, Some(&lib), None);
        assert_eq!(
            r2.implicit_generalizations_minted, 0,
            "second elaborate must not re-mint"
        );

        let count = graph
            .children_of(&pd_id)
            .filter(|c| c.kind == ElementKind::Subclassification)
            .count();
        assert_eq!(count, 1, "exactly one implicit Subclassification");
    }
}
