//! Discover user-authored `ViewUsage` / `ViewDefinition` elements and
//! summarise their spec-relevant relationships.
//!
//! A `ViewSummary` answers three questions per view, lifted directly
//! from the SysML v2 spec:
//!
//! 1. **What does it expose?** — `Expose` membership children
//!    (`MembershipExpose` / `NamespaceExpose`, both Import subtypes with
//!    `isImportAll = true`). The `importedReference` property holds the
//!    qualified name being exposed.
//! 2. **How is it rendered?** — `ViewRenderingMembership` children,
//!    each carrying an optional rendering name.
//! 3. **What conditions filter it?** — `ElementFilterMembership` children
//!    (Phase 4.5a). Each owns a Boolean `Expression` whose evaluation
//!    is deferred to Phase 5b.
//!
//! This module is pure (no IO, no global state) and operates on a
//! `ModelGraph` slice. Higher layers wrap the result in service /
//! transport responses.

use crate::{ElementId, ElementKind, ModelGraph};
use sysml_span::Span;

/// One discovered view (user-authored `ViewUsage` or `ViewDefinition`).
///
/// Field shapes are deliberately transport-friendly (owned `String`s,
/// `Option<Span>`) so this struct can be serialised to JSON for the
/// REST / MCP layers without further conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ViewSummary {
    /// The view element's id.
    pub id: ElementId,
    /// The view's name (None if anonymous).
    pub name: Option<String>,
    /// `ViewUsage` or `ViewDefinition`. Other element kinds are never
    /// reported by [`build_view_index`].
    pub kind: ElementKind,
    /// Qualified names of every `Expose` child (in graph-iteration
    /// order — callers that need a stable ordering should sort).
    pub exposed: Vec<ExposeRef>,
    /// Each `ViewRenderingMembership` child, by id and optional name.
    /// Resolving the rendering target (the `:>>` reference) is a
    /// follow-up — the parser does not currently capture it, so this
    /// field only confirms a render member exists.
    pub renderings: Vec<RenderingRef>,
    /// Each `ElementFilterMembership` child's id (the filter membership
    /// itself, not its inner Expression). Phase 5b will evaluate them.
    pub filters: Vec<ElementId>,
    /// First span of the view element, if known.
    pub source_span: Option<Span>,
}

/// One Expose membership reachable from a view body.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExposeRef {
    /// The Expose membership element id.
    pub id: ElementId,
    /// Whether this is `expose Foo::*` (true) or `expose Foo` (false).
    pub is_namespace: bool,
    /// Qualified name of the exposed namespace / member, as written in
    /// source. Stays populated even when resolution fails so callers
    /// can show a useful diagnostic.
    pub qualified_name: Option<String>,
    /// Resolved target element id, if the qualified name could be
    /// matched against an element in the same `ModelGraph`. `None`
    /// means the target wasn't found (cross-file imports that haven't
    /// been wired up yet, typos, or qnames pointing at the standard
    /// library before it is loaded).
    pub exposed_element_id: Option<ElementId>,
}

/// One ViewRenderingMembership child of a view.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RenderingRef {
    pub id: ElementId,
    pub name: Option<String>,
}

/// True if `kind` is a user-authored view kind we should index.
fn is_view_kind(kind: &ElementKind) -> bool {
    matches!(kind, ElementKind::ViewUsage | ElementKind::ViewDefinition)
}

/// True if `kind` is an Expose membership.
fn is_expose_kind(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::MembershipExpose | ElementKind::NamespaceExpose
    )
}

/// Walk every element in `graph`, build a [`ViewSummary`] for each
/// `ViewUsage` / `ViewDefinition`. Returns views in graph-iteration
/// order — callers that need a stable order should sort by name or id.
pub fn build_view_index(graph: &ModelGraph) -> Vec<ViewSummary> {
    let mut out = Vec::new();
    for element in graph.elements.values() {
        if !is_view_kind(&element.kind) {
            continue;
        }

        let mut exposed = Vec::new();
        let mut renderings = Vec::new();
        let mut filters = Vec::new();
        for child in graph.children_of(&element.id) {
            if is_expose_kind(&child.kind) {
                let qualified_name = child
                    .props
                    .get("importedReference")
                    .or_else(|| child.props.get("unresolved_importedNamespace"))
                    .and_then(|v| match v {
                        crate::Value::String(s) => Some(s.clone()),
                        _ => None,
                    });
                let is_namespace = matches!(
                    child.props.get("isNamespace"),
                    Some(crate::Value::Bool(true))
                );
                let exposed_element_id = qualified_name
                    .as_deref()
                    .and_then(|q| resolve_qname(graph, q));
                exposed.push(ExposeRef {
                    id: child.id.clone(),
                    is_namespace,
                    qualified_name,
                    exposed_element_id,
                });
            } else if child.kind == ElementKind::ViewRenderingMembership {
                renderings.push(RenderingRef {
                    id: child.id.clone(),
                    name: child.name.clone(),
                });
            } else if child.kind == ElementKind::ElementFilterMembership {
                filters.push(child.id.clone());
            }
        }

        out.push(ViewSummary {
            id: element.id.clone(),
            name: element.name.clone(),
            kind: element.kind.clone(),
            exposed,
            renderings,
            filters,
            source_span: element.spans.first().cloned(),
        });
    }
    out
}

/// Find every `ViewUsage` / `ViewDefinition` that satisfies the given
/// viewpoint. A view satisfies a viewpoint when one of its descendants
/// is a `ViewpointUsage` whose specialization / typing target is
/// `viewpoint_id`, or when the view itself transitively specialises the
/// viewpoint (the latter is rare — viewpoints are usually nested inside
/// the view body).
///
/// Returns a fresh slice of [`ViewSummary`]s for the matching views.
/// The summary shape matches [`build_view_index`] so consumers can
/// reuse the same UI rows.
///
/// TODO(Bucket 3.4): the matching rule here is a coarse first pass.
/// Spec semantics for `satisfiedViewpoint` (SysML-vocab.ttl line 2676)
/// allow ViewUsage-level satisfaction in addition to nested
/// ViewpointUsage children. Fixture 08 (`08-viewpoint-satisfaction`)
/// pins down the exact behaviour we want — refine when wiring it up.
pub fn views_by_viewpoint(graph: &ModelGraph, viewpoint_id: &ElementId) -> Vec<ViewSummary> {
    let index = build_view_index(graph);
    index
        .into_iter()
        .filter(|summary| view_satisfies_viewpoint(graph, &summary.id, viewpoint_id))
        .collect()
}

/// True if `view_id` satisfies `viewpoint_id`. Walks the view's children
/// looking for `ViewpointUsage` elements whose outgoing Specialization /
/// FeatureTyping / Subsetting relationships target `viewpoint_id`. Also
/// considers the view's own outgoing relationships (rare).
fn view_satisfies_viewpoint(
    graph: &ModelGraph,
    view_id: &ElementId,
    viewpoint_id: &ElementId,
) -> bool {
    if outgoing_targets_match(graph, view_id, viewpoint_id) {
        return true;
    }
    for child in graph.children_of(view_id) {
        if matches!(
            child.kind,
            ElementKind::ViewpointUsage | ElementKind::ViewpointDefinition
        ) && outgoing_targets_match(graph, &child.id, viewpoint_id)
        {
            return true;
        }
    }
    false
}

fn outgoing_targets_match(graph: &ModelGraph, source: &ElementId, target: &ElementId) -> bool {
    graph.outgoing(source).any(|rel| &rel.target == target)
}

/// Find every `ViewpointDefinition` / `ViewpointUsage` that frames the
/// concerns of the given stakeholder.
///
/// The spec relationship: a `ViewpointDefinition` (which `:>`
/// `RequirementDefinition`) owns `StakeholderMembership` children whose
/// `ownedStakeholderParameter` is the stakeholder PartUsage. We match
/// by walking each viewpoint's children for `StakeholderMembership`
/// (and its `Stakeholder` companion kind) whose target / referenced
/// element is `stakeholder_id`.
///
/// TODO(Bucket 3.4): tighten the match to the exact `ownedStakeholder
/// Parameter` property. The current pass accepts any outgoing
/// relationship from the StakeholderMembership pointing at the
/// stakeholder id, which over-includes when a viewpoint mentions a
/// stakeholder in another role.
pub fn viewpoints_by_stakeholder(graph: &ModelGraph, stakeholder_id: &ElementId) -> Vec<ElementId> {
    let mut out = Vec::new();
    for element in graph.elements.values() {
        if !matches!(
            element.kind,
            ElementKind::ViewpointDefinition | ElementKind::ViewpointUsage
        ) {
            continue;
        }
        let mut matched = false;
        for child in graph.children_of(&element.id) {
            if !matches!(child.kind, ElementKind::StakeholderMembership) {
                continue;
            }
            // The stakeholder PartUsage may be referenced via an outgoing
            // relationship (ParameterMembership / ReferenceSubsetting) or
            // owned directly as a child PartUsage.
            if outgoing_targets_match(graph, &child.id, stakeholder_id) {
                matched = true;
                break;
            }
            if graph
                .children_of(&child.id)
                .any(|grand| &grand.id == stakeholder_id)
            {
                matched = true;
                break;
            }
        }
        if matched {
            out.push(element.id.clone());
        }
    }
    out
}

/// Build a `view scratch : InterconnectionView { expose <ids>; }`
/// snippet that the editor can paste into a buffer at the user's cursor.
/// Pure string formatting — does not consult any graph state.
///
/// `scratch` is a view *usage*, so it is *typed by* `InterconnectionView`
/// with `:` (FeatureTyping) — not specialized with `:>` (which requires
/// a feature target and fires E005 on a usage; see views-first-class
/// roadmap 3.8). `InterconnectionView` is the canonical standard-library
/// name (the pristine stdlib defines no bare `Interconnection`) and the
/// most generally applicable spec view kind, matching the Bucket 5
/// "Create view def from selection" UX. Authors can edit the type after
/// insertion.
///
/// Each id is rendered as a single `expose` line; callers that have
/// resolved qualified names should pass those instead of raw element
/// ids when integrating the snippet into source.
pub fn views_create_scratch_snippet(expose_refs: &[String]) -> String {
    use std::fmt::Write;
    let mut out = String::from("view scratch : InterconnectionView {\n");
    for r in expose_refs {
        let _ = writeln!(out, "    expose {};", r);
    }
    out.push_str("}\n");
    out
}

/// Resolve a `Foo::Bar::Baz` qualified name to a concrete element id by
/// walking the graph's name index and ownership chain.
///
/// Heuristic only: if the first segment matches multiple roots (rare but
/// possible in a workspace with duplicate package names), returns the
/// first match that fully resolves. Cross-file workspaces should pass
/// the merged graph so library / sibling-file targets are visible.
///
/// This is a deliberate stand-in for the heavier
/// [`crate::resolution::ResolutionContext`] pipeline — view discovery
/// runs over read-only graphs and doesn't want the mutation cost of a
/// full resolution pass.
fn resolve_qname(graph: &ModelGraph, qname: &str) -> Option<ElementId> {
    let segments: Vec<&str> = qname.split("::").filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }

    // First segment: any element with this name that has no owner (a
    // root) OR whose owner is itself the workspace. We allow
    // `lookup_by_name` candidates because in elaborated graphs the
    // standard library packages are reachable as named children of the
    // root.
    let mut candidates: Vec<ElementId> =
        graph.lookup_by_name(segments[0]).iter().cloned().collect();

    // Descend through subsequent segments by name within children.
    for segment in segments.iter().skip(1) {
        candidates = candidates
            .iter()
            .flat_map(|cand| {
                graph
                    .children_of(cand)
                    .filter(|c| c.name.as_deref() == Some(*segment))
                    .map(|c| c.id.clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
    }

    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ElementFactory, Value};

    fn add_child(graph: &mut ModelGraph, parent: &ElementId, kind: ElementKind) -> ElementId {
        let elem = ElementFactory::create(kind).with_owner(parent.clone());
        let id = elem.id.clone();
        graph.add_element(elem);
        id
    }

    #[test]
    fn empty_graph_returns_no_views() {
        let graph = ModelGraph::new();
        assert!(build_view_index(&graph).is_empty());
    }

    #[test]
    fn ignores_non_view_elements() {
        let mut graph = ModelGraph::new();
        graph.add_element(ElementFactory::create(ElementKind::PartUsage).with_name("p"));
        graph.add_element(ElementFactory::create(ElementKind::Package).with_name("pkg"));
        assert!(build_view_index(&graph).is_empty());
    }

    #[test]
    fn surfaces_view_definition_with_no_children() {
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let id = v.id.clone();
        graph.add_element(v);

        let index = build_view_index(&graph);
        assert_eq!(index.len(), 1);
        let summary = &index[0];
        assert_eq!(summary.id, id);
        assert_eq!(summary.name.as_deref(), Some("V"));
        assert_eq!(summary.kind, ElementKind::ViewDefinition);
        assert!(summary.exposed.is_empty());
        assert!(summary.renderings.is_empty());
        assert!(summary.filters.is_empty());
    }

    #[test]
    fn collects_expose_membership_with_qname_and_namespace_flag() {
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);

        // `expose Foo;` → MembershipExpose, isNamespace = false
        let mut e1 =
            ElementFactory::create(ElementKind::MembershipExpose).with_owner(view_id.clone());
        e1.set_prop("importedReference", "Foo");
        e1.set_prop("isNamespace", Value::Bool(false));
        let e1_id = e1.id.clone();
        graph.add_element(e1);

        // `expose Bar::*;` → NamespaceExpose, isNamespace = true
        let mut e2 =
            ElementFactory::create(ElementKind::NamespaceExpose).with_owner(view_id.clone());
        e2.set_prop("importedReference", "Bar");
        e2.set_prop("isNamespace", Value::Bool(true));
        let e2_id = e2.id.clone();
        graph.add_element(e2);

        let index = build_view_index(&graph);
        assert_eq!(index.len(), 1);
        let summary = &index[0];
        assert_eq!(summary.exposed.len(), 2);

        let foo = summary
            .exposed
            .iter()
            .find(|e| e.id == e1_id)
            .expect("expose Foo present");
        assert_eq!(foo.qualified_name.as_deref(), Some("Foo"));
        assert!(!foo.is_namespace);

        let bar = summary
            .exposed
            .iter()
            .find(|e| e.id == e2_id)
            .expect("expose Bar::* present");
        assert_eq!(bar.qualified_name.as_deref(), Some("Bar"));
        assert!(bar.is_namespace);
    }

    #[test]
    fn collects_rendering_and_filter_children() {
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewUsage).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);

        let render = ElementFactory::create(ElementKind::ViewRenderingMembership)
            .with_owner(view_id.clone());
        let render_id = render.id.clone();
        let mut render = render;
        render.name = Some("StandardRendering".into());
        graph.add_element(render);

        let filter_id = add_child(&mut graph, &view_id, ElementKind::ElementFilterMembership);

        let index = build_view_index(&graph);
        assert_eq!(index.len(), 1);
        let s = &index[0];
        assert_eq!(s.renderings.len(), 1);
        assert_eq!(s.renderings[0].id, render_id);
        assert_eq!(s.renderings[0].name.as_deref(), Some("StandardRendering"));
        assert_eq!(s.filters, vec![filter_id]);
    }

    #[test]
    fn falls_back_to_unresolved_namespace_prop() {
        // The parser writes both `importedReference` and
        // `unresolved_importedNamespace`; we should still pick up the
        // qualified name if only the unresolved variant is present.
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);

        let mut e =
            ElementFactory::create(ElementKind::MembershipExpose).with_owner(view_id.clone());
        e.set_prop("unresolved_importedNamespace", "Foo::Bar");
        graph.add_element(e);

        let index = build_view_index(&graph);
        assert_eq!(
            index[0].exposed[0].qualified_name.as_deref(),
            Some("Foo::Bar")
        );
    }

    #[test]
    fn handles_view_usage_too() {
        let mut graph = ModelGraph::new();
        graph.add_element(ElementFactory::create(ElementKind::ViewUsage).with_name("topView"));
        let index = build_view_index(&graph);
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].kind, ElementKind::ViewUsage);
    }

    #[test]
    fn anonymous_view_has_none_name() {
        let mut graph = ModelGraph::new();
        graph.add_element(ElementFactory::create(ElementKind::ViewDefinition));
        let index = build_view_index(&graph);
        assert_eq!(index.len(), 1);
        assert!(index[0].name.is_none());
    }

    #[test]
    fn ignores_non_expose_non_render_children() {
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);
        // a stray PartUsage child shouldn't show up anywhere.
        add_child(&mut graph, &view_id, ElementKind::PartUsage);
        let index = build_view_index(&graph);
        assert!(index[0].exposed.is_empty());
        assert!(index[0].renderings.is_empty());
        assert!(index[0].filters.is_empty());
    }

    #[test]
    fn expose_resolves_simple_qname_to_root_element_id() {
        let mut graph = ModelGraph::new();

        // Sibling part `engine` at the root.
        let engine = ElementFactory::create(ElementKind::PartUsage).with_name("engine");
        let engine_id = engine.id.clone();
        graph.add_element(engine);

        // View with `expose engine;`
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut e = ElementFactory::create(ElementKind::MembershipExpose).with_owner(view_id);
        e.set_prop("importedReference", "engine");
        graph.add_element(e);

        let index = build_view_index(&graph);
        let exposed = &index[0].exposed[0];
        assert_eq!(exposed.qualified_name.as_deref(), Some("engine"));
        assert_eq!(exposed.exposed_element_id.as_ref(), Some(&engine_id));
    }

    #[test]
    fn expose_resolves_two_segment_qname_via_owner_chain() {
        let mut graph = ModelGraph::new();

        // Package P { part engine; }
        let pkg = ElementFactory::create(ElementKind::Package).with_name("P");
        let pkg_id = pkg.id.clone();
        graph.add_element(pkg);
        let engine = ElementFactory::create(ElementKind::PartUsage)
            .with_name("engine")
            .with_owner(pkg_id.clone());
        let engine_id = engine.id.clone();
        graph.add_element(engine);

        // View with `expose P::engine;`
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut e = ElementFactory::create(ElementKind::MembershipExpose).with_owner(view_id);
        e.set_prop("importedReference", "P::engine");
        graph.add_element(e);

        let index = build_view_index(&graph);
        let exposed = &index[0].exposed[0];
        assert_eq!(exposed.exposed_element_id.as_ref(), Some(&engine_id));
    }

    #[test]
    fn expose_with_unknown_qname_leaves_id_unresolved() {
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut e = ElementFactory::create(ElementKind::MembershipExpose).with_owner(view_id);
        e.set_prop("importedReference", "DoesNot::Exist");
        graph.add_element(e);

        let index = build_view_index(&graph);
        let exposed = &index[0].exposed[0];
        assert_eq!(exposed.qualified_name.as_deref(), Some("DoesNot::Exist"));
        assert!(exposed.exposed_element_id.is_none());
    }

    // ── views_by_viewpoint / viewpoints_by_stakeholder / scratch ──

    #[test]
    fn views_by_viewpoint_finds_view_with_nested_viewpoint_usage() {
        use crate::{Relationship, RelationshipKind};

        let mut graph = ModelGraph::new();
        let vp = ElementFactory::create(ElementKind::ViewpointDefinition).with_name("VP");
        let vp_id = vp.id.clone();
        graph.add_element(vp);

        let v = ElementFactory::create(ElementKind::ViewUsage).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);

        let vp_usage =
            ElementFactory::create(ElementKind::ViewpointUsage).with_owner(view_id.clone());
        let vp_usage_id = vp_usage.id.clone();
        graph.add_element(vp_usage);

        // Specialize: ViewpointUsage :> VP
        graph.add_relationship(Relationship::new(
            RelationshipKind::Specialize,
            vp_usage_id,
            vp_id.clone(),
        ));

        let hits = views_by_viewpoint(&graph, &vp_id);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, view_id);
    }

    #[test]
    fn views_by_viewpoint_returns_empty_when_no_match() {
        let graph = ModelGraph::new();
        let stranger = ElementFactory::create(ElementKind::ViewpointDefinition).id;
        assert!(views_by_viewpoint(&graph, &stranger).is_empty());
    }

    #[test]
    fn viewpoints_by_stakeholder_finds_via_owned_part() {
        let mut graph = ModelGraph::new();
        let stakeholder = ElementFactory::create(ElementKind::PartUsage).with_name("Architect");
        let stakeholder_id = stakeholder.id.clone();
        graph.add_element(stakeholder);

        let vp = ElementFactory::create(ElementKind::ViewpointDefinition).with_name("VP");
        let vp_id = vp.id.clone();
        graph.add_element(vp);

        let membership =
            ElementFactory::create(ElementKind::StakeholderMembership).with_owner(vp_id.clone());
        let membership_id = membership.id.clone();
        graph.add_element(membership);

        // Stakeholder PartUsage owned as a child of the StakeholderMembership.
        let owned = ElementFactory::create(ElementKind::PartUsage).with_owner(membership_id);
        let owned_id = owned.id.clone();
        graph.add_element(owned);

        // Wire the owned PartUsage so it IS the stakeholder for the test.
        // (Real models reach the stakeholder via ReferenceSubsetting; we
        // exercise the simpler "owned child" path here.)
        let _ = stakeholder_id;
        let hits = viewpoints_by_stakeholder(&graph, &owned_id);
        assert_eq!(hits, vec![vp_id]);
    }

    #[test]
    fn create_scratch_snippet_round_trip() {
        let snippet =
            views_create_scratch_snippet(&["engine".to_string(), "Vehicle::wheels".to_string()]);
        assert!(snippet.starts_with("view scratch : InterconnectionView {\n"));
        assert!(snippet.contains("    expose engine;\n"));
        assert!(snippet.contains("    expose Vehicle::wheels;\n"));
        assert!(snippet.ends_with("}\n"));
    }

    #[test]
    fn create_scratch_snippet_no_exposes() {
        let snippet = views_create_scratch_snippet(&[]);
        assert_eq!(snippet, "view scratch : InterconnectionView {\n}\n");
    }
}
