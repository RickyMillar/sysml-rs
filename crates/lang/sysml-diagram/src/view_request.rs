//! `ViewRequest` — structured dispatch for diagram generation.
//!
//! Mirrors the spec's view dispatch: a `ViewType` (one of the 8 standard
//! ViewDefinitions) plus optional `viewCondition` filter, layout hints,
//! and an `Expose`d Namespace. Composed by transport layers from a
//! discovered `ViewSummary` (user-authored `ViewUsage` /
//! `ViewDefinition`) and handed to `to_view_model_with_filter_cache`
//! for execution.

use std::collections::{BTreeSet, HashSet, VecDeque};

use sysml_core::{Element, ElementId, ElementKind, ModelGraph, ViewFilter, ViewSummary};

use crate::ir::{DiagramOverlay, RenderingHints};
use crate::ViewType;

/// A structured request for diagram generation.
///
/// Built by transport layers from their wire-format params, then handed
/// to [`crate::to_view_model_with_filter_cache`] for execution. See
/// module docs for the dispatch flow.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ViewRequest {
    /// Which spec ViewDefinition (one of the 8 standard kinds) to render.
    pub view_type: ViewType,

    /// Container ids to render expanded. Defaults to empty (all collapsed).
    #[cfg_attr(feature = "serde", serde(default))]
    pub expanded_ids: HashSet<String>,

    /// Spec `viewCondition`. `None` lets every element through.
    #[cfg_attr(feature = "serde", serde(default))]
    pub filter: Option<ViewFilter>,

    /// Layout overrides (ELK direction, algorithm, spacing).
    #[cfg_attr(feature = "serde", serde(default))]
    pub hints: Option<RenderingHints>,

    /// Spec `Expose`d Namespaces. Generators center the canvas on these elements when set.
    #[cfg_attr(feature = "serde", serde(default))]
    pub exposes: Vec<ElementId>,

    /// The declared `View` element's own id, when this request renders a
    /// discovered `ViewUsage` / `ViewDefinition`. Part of the cache key
    /// and the frame-resolution identity: two declared views that share a
    /// kind + expose target are still distinct diagrams (different frame,
    /// own cache slot). `None` for ad-hoc / LSP projections that have no
    /// backing declared view — those still share cache slots on
    /// `(view_type, expanded_ids, expose)` exactly as before.
    #[cfg_attr(feature = "serde", serde(default))]
    pub view_id: Option<ElementId>,

    /// Diagram overlays — post-processors run after the generator
    /// produces the IR. Lets presets opt into visual fidelity (e.g.
    /// solver value badges, requirement compartments) without forcing
    /// the standard generators to know about preset semantics.
    ///
    /// Carried as `&'static dyn DiagramOverlay` so transports never
    /// allocate; preset registry entries hold these as static slices.
    /// Not serialised over the wire — clients pick presets, the server
    /// resolves the overlay set.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub overlays: Vec<&'static dyn DiagramOverlay>,
}

impl ViewRequest {
    /// New request for `view_type` with no extras (matches pre-4.5
    /// behaviour when fed to `to_payload_with_filter_cache`).
    pub fn new(view_type: ViewType) -> Self {
        Self {
            view_type,
            expanded_ids: HashSet::new(),
            filter: None,
            hints: None,
            exposes: Vec::new(),
            view_id: None,
            overlays: Vec::new(),
        }
    }

    /// Override the view type.
    pub fn with_view_type(mut self, vt: ViewType) -> Self {
        self.view_type = vt;
        self
    }

    /// Replace the expanded-id set.
    pub fn with_expanded(mut self, ids: HashSet<String>) -> Self {
        self.expanded_ids = ids;
        self
    }

    /// Attach an explicit filter, overriding any preset-supplied one.
    pub fn with_filter(mut self, filter: ViewFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Attach layout hints, overriding any preset-supplied set.
    pub fn with_hints(mut self, hints: RenderingHints) -> Self {
        self.hints = Some(hints);
        self
    }

    /// Attach a Phase-5 `expose` target. Convenience for single-expose callers;
    /// use `with_exposes` for multi-expose.
    pub fn with_expose(mut self, expose: ElementId) -> Self {
        self.exposes.push(expose);
        self
    }

    /// Attach multiple Phase-5 `expose` targets at once.
    pub fn with_exposes(mut self, exposes: Vec<ElementId>) -> Self {
        self.exposes = exposes;
        self
    }

    /// Derive a stable cache key for this request, or `None` if the
    /// request carries fields the cache key does not capture.
    ///
    /// `Some(DiagramRequestKey)` is returned only when the request can
    /// be fully described by `(view_type, expanded_ids, expose)` — i.e.
    /// `filter`, `hints`, and `overlays` are all absent. Callers that
    /// hold a non-trivial filter / hints / overlay set must bypass the
    /// salsa cache and call `to_view_model` directly; the cache key
    /// would otherwise hash unrelated requests to the same slot.
    ///
    /// `expanded_ids` strings that do not round-trip through
    /// [`ElementId::from_string`]'s direct UUID parser are skipped — they
    /// can't refer to a real graph element anyway.
    pub fn cache_key(&self) -> Option<DiagramRequestKey> {
        if self.filter.is_some() || self.hints.is_some() || !self.overlays.is_empty() {
            return None;
        }
        Some(DiagramRequestKey::from_view_request(self))
    }

    /// Append a single overlay. Order matters when overlays
    /// non-trivially interact (rare — see overlay module docs).
    pub fn with_overlay(mut self, overlay: &'static dyn DiagramOverlay) -> Self {
        self.overlays.push(overlay);
        self
    }

    /// Replace the overlay list wholesale.
    pub fn with_overlays(mut self, overlays: Vec<&'static dyn DiagramOverlay>) -> Self {
        self.overlays = overlays;
        self
    }

    /// Compose a `ViewRequest` from a discovered [`ViewSummary`].
    ///
    /// The mapping mirrors what the spec dictates for rendering a
    /// user-authored `ViewUsage` / `ViewDefinition`:
    ///
    /// - **`view_type`**: inferred via [`resolve_view_kind`] from the
    ///   view's `:>` / `:` supertype chain or render member name — only
    ///   against the canonical standard `*View` def names. No own-name
    ///   matching, no suffix-stripping, no preset overlays. Falls back to
    ///   [`ViewType::General`].
    /// - **`expose`**: the first `ExposeRef` whose `exposed_element_id`
    ///   resolved. Generators centre the canvas on this element when
    ///   set.
    /// - **`filter`**: collects every `ElementFilterMembership` child
    ///   under one [`ViewFilter`]. Each filter's inner Boolean
    ///   Expression id is attached; evaluation is the safe-default-true
    ///   evaluator in `sysml-core`.
    /// - **`expanded_ids`**: when `expose` resolves, that id is added
    ///   so the subject node renders expanded by default.
    pub fn from_view_usage(graph: &ModelGraph, summary: &ViewSummary) -> Self {
        let view_type = resolve_view_kind(graph, summary);
        let exposed_ids: Vec<ElementId> = summary
            .exposed
            .iter()
            .filter_map(|e| e.exposed_element_id.clone())
            .collect();
        let mut expanded_ids = HashSet::new();
        for id in &exposed_ids {
            expanded_ids.insert(id.to_string());
        }

        // Spec clause 7.26.2: "A view usage inherits any filter conditions
        // from its view definition and can declare additional conditions of
        // its own." So the effective filter is the view's OWN
        // ElementFilterMembership children (summary.filters) AND every filter
        // declared on a view definition reached through the `:>` / `:`
        // supertype chain (e.g. `view def X :> Base` inherits any `filter`
        // clauses `Base` declares). All compose as
        // conjunction — every Boolean criterion must hold — which is exactly
        // how `GeneratorContext::passes_filter` evaluates the expression list.
        let mut filter_ids: Vec<ElementId> = summary.filters.iter().cloned().collect();
        if let Some(view_elem) = graph.get_element(&summary.id) {
            filter_ids.extend(collect_inherited_filter_ids(graph, view_elem));
        }
        let filter = if filter_ids.is_empty() {
            None
        } else {
            Some(ViewFilter::new().with_expressions(filter_ids))
        };

        Self {
            view_type,
            expanded_ids,
            filter,
            hints: None,
            exposes: exposed_ids,
            view_id: Some(summary.id.clone()),
            overlays: Vec::new(),
        }
    }
}

/// Stable, hashable cache key for diagram generation.
///
/// Wraps `(view_type, expanded_ids, expose)` with deterministic
/// ordering on `expanded_ids` (a `BTreeSet<ElementId>`, not the
/// `HashSet<String>` `ViewRequest` carries on the wire). This makes the
/// key salsa-cacheable: `Hash + Eq + Clone + Debug + Send + Sync` plus
/// a deterministic hash across calls.
///
/// Build via [`ViewRequest::cache_key`] (returns `None` when the
/// request carries `filter` / `hints` / `overlays` — those bypass the
/// cache) or directly via [`DiagramRequestKey::from_view_request`] /
/// [`DiagramRequestKey::new`] when constructing a key for testing.
///
/// Per ADR-011 §3 / S3.T4: every byte of state that influences the
/// generated legacy graph must be in this key, otherwise the cache returns
/// stale results. The plan deliberately scopes the key to the
/// LSP-`generate_diagram` shape (no filter / hints / overlays) — those
/// fields appear only on user-authored ViewUsage requests, which take
/// the uncached path. T6 caches the precompiled filter expressions
/// separately so the uncached path still amortises its hot inner loop.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiagramRequestKey {
    view_type: ViewType,
    expanded_ids: BTreeSet<ElementId>,
    exposes: std::collections::BTreeSet<ElementId>,
    /// The declared view's own id (`None` for ad-hoc projections). In the
    /// key per ADR-011 §3: the cached `ViewModel` carries a `frame` derived
    /// from this id, so two declared views sharing kind + expose must key to
    /// distinct slots and resolve their own frame.
    view_id: Option<ElementId>,
}

impl DiagramRequestKey {
    /// Construct a key from raw fields. Public so tests / future
    /// callers can build a key without going through `ViewRequest`.
    pub fn new(
        view_type: ViewType,
        expanded_ids: BTreeSet<ElementId>,
        exposes: std::collections::BTreeSet<ElementId>,
        view_id: Option<ElementId>,
    ) -> Self {
        Self {
            view_type,
            expanded_ids,
            exposes,
            view_id,
        }
    }

    /// Build a key from a [`ViewRequest`]. Filter / hints / overlays
    /// on the request are ignored — call sites that carry those must
    /// bypass the cache via [`ViewRequest::cache_key`] returning
    /// `None`.
    ///
    /// `expanded_ids` strings are parsed via [`ElementId::from_string`],
    /// which roundtrips UUID strings directly and falls back to a
    /// deterministic hash for everything else. The string-to-key
    /// mapping is therefore stable across runs even on malformed
    /// inputs (a wrong-shape id keys to a unique bucket that won't
    /// collide with a real one).
    pub fn from_view_request(req: &ViewRequest) -> Self {
        let expanded_ids = req
            .expanded_ids
            .iter()
            .map(|s| ElementId::from_string(s.clone()))
            .collect();
        Self {
            view_type: req.view_type,
            expanded_ids,
            exposes: req.exposes.iter().cloned().collect::<std::collections::BTreeSet<_>>(),
            view_id: req.view_id.clone(),
        }
    }

    /// The view type this key targets.
    pub fn view_type(&self) -> ViewType {
        self.view_type
    }

    /// Borrow the expanded-id set.
    pub fn expanded_ids(&self) -> &BTreeSet<ElementId> {
        &self.expanded_ids
    }

    /// Borrow the expose targets.
    pub fn exposes(&self) -> &std::collections::BTreeSet<ElementId> {
        &self.exposes
    }

    /// Borrow the optional declared-view id (the frame-resolution identity).
    pub fn view_id(&self) -> Option<&ElementId> {
        self.view_id.as_ref()
    }

    /// Materialise this key back into a [`ViewRequest`]. Used by the
    /// cache-fill path to call the existing `to_view_model` body
    /// without duplicating its parameter list. The returned request
    /// has `filter` / `hints` / `overlays` empty by definition.
    pub fn to_view_request(&self) -> ViewRequest {
        let mut req = ViewRequest::new(self.view_type);
        req.expanded_ids = self
            .expanded_ids
            .iter()
            .map(|id| id.to_string())
            .collect();
        req.exposes = self.exposes.iter().cloned().collect();
        req.view_id = self.view_id.clone();
        req
    }
}

/// Map a standard view-definition name reached via a `:>` / `:` supertype
/// (or a rendering reference) to a [`ViewType`]. Only the canonical names
/// of the 8 standard ViewDefinitions the OMG standard library actually
/// defines are recognised (`GeneralView`, `InterconnectionView`, …).
///
/// Bare-name spellings (`:> Interconnection`) do NOT classify: they were
/// aliases invented by a retired local patch to the standard library, not
/// spec names. A model writing one now carries a dangling supertype and
/// falls through [`resolve_view_kind`]'s unresolved branch (General, with
/// a warning). There is likewise no `RequirementView` and no `Parametric`
/// kind — a "requirement view" is a filtered General view, and constraint /
/// binding notation renders in `Interconnection`.
fn name_to_view_type(name: &str) -> Option<ViewType> {
    match name {
        "GeneralView" => Some(ViewType::General),
        "InterconnectionView" => Some(ViewType::Interconnection),
        "ActionFlowView" => Some(ViewType::ActionFlow),
        "StateTransitionView" => Some(ViewType::StateTransition),
        "SequenceView" => Some(ViewType::Sequence),
        "BrowserView" => Some(ViewType::Browser),
        "GridView" => Some(ViewType::Grid),
        "GeometryView" => Some(ViewType::Geometry),
        _ => None,
    }
}

/// Match a possibly-qualified supertype/rendering name
/// (`StandardViewDefinitions::InterconnectionView`) by stripping to its
/// last path segment before running [`name_to_view_type`].
fn match_view_name(name: &str) -> Option<ViewType> {
    let leaf = name.rsplit("::").next().unwrap_or(name).trim();
    name_to_view_type(leaf)
}

/// Walk the `:>` specialization / `:` typing chain of a view element to the
/// first standard view definition that maps to a [`ViewType`].
///
/// BFS over the supertype names ([`supertype_names`]), checking each name
/// against [`match_view_name`] *before* descending. The first
/// (most-derived) match wins: `:> StateTransitionView` stops at
/// `StateTransition` and never walks on to its `InterconnectionView`
/// supertype, preserving the spec distinction. Names that don't match are
/// resolved to their own declaration (if present in the graph) and their
/// supertypes enqueued, so user-defined intermediate view defs chain
/// through. Depth-capped to guard against specialization cycles.
///
/// The name check matches only the canonical `*View` spellings of the 8
/// standard ViewDefinitions ([`name_to_view_type`]) — the names the OMG
/// standard library really declares. A dangling supertype (e.g.
/// `:> Interconnection`, which names nothing in the pristine stdlib) has
/// no declaration to descend into and never classifies.
fn walk_supertypes_for_view_type(graph: &ModelGraph, start: &sysml_core::Element) -> Option<ViewType> {
    use crate::visual_kind::{definition_by_name, supertype_names};

    const MAX_DEPTH: u32 = 8;
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    for n in supertype_names(graph, start) {
        queue.push_back((n, 0));
    }
    while let Some((name, depth)) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(vt) = match_view_name(&name) {
            return Some(vt);
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        let leaf = name.rsplit("::").next().unwrap_or(&name);
        if let Some(def) = definition_by_name(graph, leaf) {
            for n in supertype_names(graph, def) {
                queue.push_back((n, depth + 1));
            }
        }
    }
    None
}

/// Collect every `ElementFilterMembership` id declared on a view
/// definition reached through `start`'s `:>` / `:` supertype chain
/// (spec clause 7.26.2 filter inheritance). `start`'s OWN filter children
/// are NOT included — the caller already has those from `ViewSummary.filters`;
/// this returns only the *inherited* ids, AND-composed with the own set.
///
/// BFS over [`supertype_names`], resolving each name to its declaration via
/// [`definition_by_name`] and collecting that ancestor's direct filter
/// memberships, then enqueueing its own supertypes. Cycle-guarded by a seen
/// set and depth-capped, mirroring [`walk_supertypes_for_view_type`]. The
/// returned ids are real graph elements (filter memberships on the ancestor
/// view defs), so they resolve through the existing precompiled-filter-expr
/// cache exactly like a view's own filters.
fn collect_inherited_filter_ids(graph: &ModelGraph, start: &Element) -> Vec<ElementId> {
    use crate::visual_kind::{definition_by_name, supertype_names};

    const MAX_DEPTH: u32 = 8;
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    for n in supertype_names(graph, start) {
        queue.push_back((n, 0));
    }
    let mut out = Vec::new();
    while let Some((name, depth)) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let leaf = name.rsplit("::").next().unwrap_or(&name);
        let Some(def) = definition_by_name(graph, leaf) else {
            continue;
        };
        for child in graph.children_of(&def.id) {
            if child.kind == ElementKind::ElementFilterMembership {
                out.push(child.id.clone());
            }
        }
        if depth < MAX_DEPTH {
            for n in supertype_names(graph, def) {
                queue.push_back((n, depth + 1));
            }
        }
    }
    out
}

/// Infer the view kind for a `ViewSummary` from (in order):
/// 1. The view element's `:>` / `:` supertype chain, walked to the first
///    standard view definition ([`walk_supertypes_for_view_type`]).
/// 2. Any `ViewRenderingMembership` child's name.
///
/// Each candidate goes through [`match_view_name`], which only recognises
/// the canonical `*View` names of the 8 standard ViewDefinitions. The
/// view's own name never classifies — a view def named `Interconnection`
/// is just a name, not a kind declaration. No preset overlays, no
/// name-suffix heuristic.
///
/// **Default vs. failure (no silent fallback).** Reaching the end without a
/// match has two distinct meanings, which this function keeps separate:
///  - The view declares *no* kind signal at all (no `:>` / `:` supertype,
///    no rendering kind). Per spec clause 9.2.20
///    `GeneralView` is the root ("the most general view"), so an
///    unspecialised view *is* a General view — returning [`ViewType::General`]
///    here is the correct spec default, not a fallback.
///  - The view *did* declare a supertype / rendering kind that we could not
///    resolve to any standard `ViewType` (broken/unloaded standard library,
///    typo'd or unsupported supertype). That is a real failure — we still
///    render as General to stay usable, but we **never do so silently**: a
///    `warn!` names the unresolved supertypes/renderings so the gap is
///    observable rather than masquerading as an intentional General view.
fn resolve_view_kind(graph: &ModelGraph, summary: &ViewSummary) -> ViewType {
    let view_elem = graph.get_element(&summary.id);
    if let Some(view_elem) = view_elem {
        if let Some(vt) = walk_supertypes_for_view_type(graph, view_elem) {
            return vt;
        }
    }
    for r in &summary.renderings {
        if let Some(vt) = r.name.as_deref().and_then(match_view_name) {
            return vt;
        }
    }

    // No standard view kind resolved. Distinguish "nothing declared" (legit
    // General default) from "declared a kind we couldn't resolve" (surface it).
    let declared_supertypes = view_elem
        .map(|e| crate::visual_kind::supertype_names(graph, e))
        .unwrap_or_default();
    let declared_renderings: Vec<&str> = summary
        .renderings
        .iter()
        .filter_map(|r| r.name.as_deref())
        .collect();
    if !declared_supertypes.is_empty() || !declared_renderings.is_empty() {
        tracing::warn!(
            view = summary.name.as_deref().unwrap_or("<anonymous>"),
            supertypes = ?declared_supertypes,
            renderings = ?declared_renderings,
            "view declares a kind that resolves to no standard ViewType; \
             rendering as GeneralView. Check the supertype chain / that the \
             standard library is loaded."
        );
    }
    ViewType::General
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{
        build_view_index, ElementFactory, ElementKind, ExposeRef, ModelGraph, RenderingRef,
        ViewSummary,
    };

    #[test]
    fn new_has_no_extras() {
        let r = ViewRequest::new(ViewType::General);
        assert_eq!(r.view_type, ViewType::General);
        assert!(r.expanded_ids.is_empty());
        assert!(r.filter.is_none());
        assert!(r.hints.is_none());
        assert!(r.exposes.is_empty());
    }

    #[test]
    fn builders_chain() {
        let mut expanded = HashSet::new();
        expanded.insert("id-1".to_owned());
        let r = ViewRequest::new(ViewType::Interconnection)
            .with_expanded(expanded.clone())
            .with_filter(ViewFilter::new().with_kinds([ElementKind::PartUsage]))
            .with_hints(RenderingHints::new().with_direction("DOWN"));
        assert_eq!(r.view_type, ViewType::Interconnection);
        assert_eq!(r.expanded_ids, expanded);
        assert!(r.filter.is_some());
        assert!(r.hints.is_some());
    }

    #[test]
    fn from_view_usage_defaults_to_general_with_no_extras() {
        let graph = ModelGraph::new();
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: Some("V".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert_eq!(r.view_type, ViewType::General);
        assert!(r.exposes.is_empty());
        assert!(r.filter.is_none());
        assert!(r.expanded_ids.is_empty());
        assert!(r.overlays.is_empty());
    }

    #[test]
    fn from_view_usage_adopts_first_resolved_expose_target() {
        let mut graph = ModelGraph::new();
        let engine = ElementFactory::create(ElementKind::PartUsage).with_name("engine");
        let engine_id = engine.id.clone();
        graph.add_element(engine);

        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("V");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut e = ElementFactory::create(ElementKind::MembershipExpose).with_owner(view_id);
        e.set_prop("importedReference", "engine");
        graph.add_element(e);

        let summary = build_view_index(&graph).into_iter().next().unwrap();
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert!(r.exposes.contains(&engine_id));
        assert!(r.expanded_ids.contains(&engine_id.to_string()));
    }

    #[test]
    fn from_view_usage_skips_unresolved_exposes_and_takes_next() {
        // First Expose target (DoesNot::Exist) won't resolve, so the
        // composer should fall through to the second one.
        let mut graph = ModelGraph::new();
        let part = ElementFactory::create(ElementKind::PartUsage).with_name("real");
        let part_id = part.id.clone();
        graph.add_element(part);

        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: Some("V".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![
                ExposeRef {
                    id: ElementFactory::create(ElementKind::MembershipExpose).id,
                    is_namespace: false,
                    qualified_name: Some("DoesNot::Exist".into()),
                    exposed_element_id: None,
                },
                ExposeRef {
                    id: ElementFactory::create(ElementKind::MembershipExpose).id,
                    is_namespace: false,
                    qualified_name: Some("real".into()),
                    exposed_element_id: Some(part_id.clone()),
                },
            ],
            renderings: vec![RenderingRef {
                id: ElementFactory::create(ElementKind::ViewRenderingMembership).id,
                name: None,
            }],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert!(r.exposes.contains(&part_id));
    }

    #[test]
    fn from_view_usage_collects_all_resolved_expose_targets() {
        let mut graph = ModelGraph::new();
        let req1 = ElementFactory::create(ElementKind::RequirementUsage).with_name("SafetyReq");
        let req1_id = req1.id.clone();
        graph.add_element(req1);
        let req2 = ElementFactory::create(ElementKind::RequirementUsage).with_name("PerfReq");
        let req2_id = req2.id.clone();
        graph.add_element(req2);
        let req3 = ElementFactory::create(ElementKind::RequirementUsage).with_name("DurabilityReq");
        let req3_id = req3.id.clone();
        graph.add_element(req3);

        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: Some("AllReqs".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![
                ExposeRef {
                    id: ElementFactory::create(ElementKind::MembershipExpose).id,
                    is_namespace: false,
                    qualified_name: Some("SafetyReq".into()),
                    exposed_element_id: Some(req1_id.clone()),
                },
                ExposeRef {
                    id: ElementFactory::create(ElementKind::MembershipExpose).id,
                    is_namespace: false,
                    qualified_name: Some("PerfReq".into()),
                    exposed_element_id: Some(req2_id.clone()),
                },
                ExposeRef {
                    id: ElementFactory::create(ElementKind::MembershipExpose).id,
                    is_namespace: false,
                    qualified_name: Some("DurabilityReq".into()),
                    exposed_element_id: Some(req3_id.clone()),
                },
            ],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert!(r.exposes.contains(&req1_id), "SafetyReq must be exposed");
        assert!(r.exposes.contains(&req2_id), "PerfReq must be exposed");
        assert!(r.exposes.contains(&req3_id), "DurabilityReq must be exposed");
        assert_eq!(r.exposes.len(), 3);
        for id in &r.exposes {
            assert!(r.expanded_ids.contains(&id.to_string()), "each expose must be in expanded_ids");
        }
    }

    #[test]
    fn from_view_usage_own_name_never_classifies() {
        // A view's own name is just a name, not a kind declaration. A view
        // def literally named `Interconnection` with no `:>` supertype is an
        // unspecialised view — the spec root, General.
        let graph = ModelGraph::new();
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: Some("Interconnection".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert_eq!(r.view_type, ViewType::General);
    }

    #[test]
    fn from_view_usage_canonical_own_name_never_classifies_either() {
        // The canonical spelling as an own name is equally inert — a view
        // named `InterconnectionView` (without `:> InterconnectionView`)
        // falls through to General.
        let graph = ModelGraph::new();
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: Some("InterconnectionView".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert_eq!(r.view_type, ViewType::General);
    }

    /// Build a graph containing a `view def` element that `:>`-specializes
    /// `super_name` (a `Subclassification` child carrying
    /// `unresolved_superclassifier`), plus a `ViewSummary` for it.
    fn view_with_supertype(super_name: &str) -> (ModelGraph, ViewSummary) {
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("MyView");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(view_id.clone());
        sub.set_prop("unresolved_superclassifier", super_name);
        graph.add_element(sub);
        let summary = ViewSummary {
            id: view_id,
            name: Some("MyView".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        (graph, summary)
    }

    #[test]
    fn resolve_kind_bare_supertype_does_not_classify() {
        // `view def MyView :> Interconnection` — the pristine stdlib defines
        // no `Interconnection` (only `InterconnectionView`), so the
        // supertype dangles and must NOT silently classify. It takes the
        // declared-but-unresolved branch: General, with a warning.
        let (graph, summary) = view_with_supertype("Interconnection");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::General);
    }

    #[test]
    fn resolve_kind_follows_canonical_view_supertype() {
        // `view def MyView :> InterconnectionView` → Interconnection (the
        // canonical std-lib spelling, the only one that classifies).
        let (graph, summary) = view_with_supertype("InterconnectionView");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::Interconnection);
    }

    #[test]
    fn resolve_kind_stops_at_most_derived_supertype() {
        // `StateTransitionView specializes InterconnectionView`. Writing
        // `:> StateTransitionView` must stop at StateTransition and NOT
        // walk on to its Interconnection supertype.
        let (graph, summary) = view_with_supertype("StateTransitionView");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::StateTransition);
    }

    #[test]
    fn resolve_kind_strips_qualified_supertype() {
        // A qualified supertype name resolves by its last path segment.
        let (graph, summary) =
            view_with_supertype("StandardViewDefinitions::InterconnectionView");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::Interconnection);
    }

    #[test]
    fn resolve_kind_declared_but_unresolved_defaults_to_general() {
        // `view def MyView :> NotAViewKind` where the supertype resolves to
        // no standard ViewType and has no definition in the graph. We render
        // as General (stay usable) — the `warn!` (side-effect, not asserted
        // here) keeps it observable rather than a silent masquerade.
        let (graph, summary) = view_with_supertype("NotAViewKind");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::General);
    }

    #[test]
    fn resolve_kind_no_signal_defaults_to_general_silently() {
        // A view def with NO supertype / rendering / recognised own-name is
        // the spec root (GeneralView) — the legitimate default.
        let mut graph = ModelGraph::new();
        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("Plain");
        let view_id = v.id.clone();
        graph.add_element(v);
        let summary = ViewSummary {
            id: view_id,
            name: Some("Plain".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::General);
    }

    #[test]
    fn from_view_usage_inherits_supertype_filter() {
        // `view def MyView :> Base`, where `Base` declares a filter. MyView
        // has no OWN filter, but must INHERIT Base's via the `:>` chain
        // (spec 7.26.2).
        let mut graph = ModelGraph::new();
        let base = ElementFactory::create(ElementKind::ViewDefinition).with_name("Base");
        let base_id = base.id.clone();
        graph.add_element(base);
        let filter_mem = ElementFactory::create(ElementKind::ElementFilterMembership)
            .with_owner(base_id.clone());
        let filter_id = filter_mem.id.clone();
        graph.add_element(filter_mem);

        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("MyView");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(view_id.clone());
        sub.set_prop("unresolved_superclassifier", "Base");
        graph.add_element(sub);

        let summary = ViewSummary {
            id: view_id,
            name: Some("MyView".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![], // no OWN filter — must come from the supertype
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        let filter = r.filter.expect("inherited filter should be present");
        assert!(
            filter.expressions.contains(&filter_id),
            "MyView should inherit Base's ElementFilterMembership through `:>`"
        );
    }

    #[test]
    fn from_view_usage_composes_own_and_inherited_filters() {
        // Own + inherited filters AND-compose: both ids present.
        let mut graph = ModelGraph::new();
        let base = ElementFactory::create(ElementKind::ViewDefinition).with_name("Base");
        let base_id = base.id.clone();
        graph.add_element(base);
        let inherited = ElementFactory::create(ElementKind::ElementFilterMembership)
            .with_owner(base_id.clone());
        let inherited_id = inherited.id.clone();
        graph.add_element(inherited);

        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("MyView");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(view_id.clone());
        sub.set_prop("unresolved_superclassifier", "Base");
        graph.add_element(sub);
        let own = ElementFactory::create(ElementKind::ElementFilterMembership)
            .with_owner(view_id.clone());
        let own_id = own.id.clone();
        graph.add_element(own);

        let summary = ViewSummary {
            id: view_id,
            name: Some("MyView".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![own_id.clone()],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        let filter = r.filter.expect("filter present");
        assert!(filter.expressions.contains(&own_id), "own filter retained");
        assert!(
            filter.expressions.contains(&inherited_id),
            "inherited filter added"
        );
    }

    #[test]
    fn from_view_usage_inherits_filter_transitively_two_levels() {
        // `Reqs :> Mid`, `Mid :> Base`, `Base { filter }`. Reqs must inherit
        // Base's filter through the 2-level chain — the shape a model uses
        // to build a filtered "requirement view" on top of an intermediate
        // view def of its own.
        let mut graph = ModelGraph::new();
        let base = ElementFactory::create(ElementKind::ViewDefinition).with_name("Base");
        let base_id = base.id.clone();
        graph.add_element(base);
        let filter_mem = ElementFactory::create(ElementKind::ElementFilterMembership)
            .with_owner(base_id.clone());
        let filter_id = filter_mem.id.clone();
        graph.add_element(filter_mem);

        let mid = ElementFactory::create(ElementKind::ViewDefinition).with_name("Mid");
        let mid_id = mid.id.clone();
        graph.add_element(mid);
        let mut mid_sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(mid_id.clone());
        mid_sub.set_prop("unresolved_superclassifier", "Base");
        graph.add_element(mid_sub);

        let reqs = ElementFactory::create(ElementKind::ViewDefinition).with_name("Reqs");
        let reqs_id = reqs.id.clone();
        graph.add_element(reqs);
        let mut reqs_sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(reqs_id.clone());
        reqs_sub.set_prop("unresolved_superclassifier", "Mid");
        graph.add_element(reqs_sub);

        let summary = ViewSummary {
            id: reqs_id,
            name: Some("Reqs".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        let filter = r.filter.expect("transitively-inherited filter should be present");
        assert!(
            filter.expressions.contains(&filter_id),
            "Reqs should inherit Base's filter through Mid (2-level `:>` chain)"
        );
    }

    #[test]
    fn resolve_kind_walks_transitively_through_user_view_def() {
        // view def MyView :> Mid ; view def Mid :> InterconnectionView.
        let mut graph = ModelGraph::new();
        let mid = ElementFactory::create(ElementKind::ViewDefinition).with_name("Mid");
        let mid_id = mid.id.clone();
        graph.add_element(mid);
        let mut mid_sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(mid_id.clone());
        mid_sub.set_prop("unresolved_superclassifier", "InterconnectionView");
        graph.add_element(mid_sub);

        let v = ElementFactory::create(ElementKind::ViewDefinition).with_name("MyView");
        let view_id = v.id.clone();
        graph.add_element(v);
        let mut sub =
            ElementFactory::create(ElementKind::Subclassification).with_owner(view_id.clone());
        sub.set_prop("unresolved_superclassifier", "Mid");
        graph.add_element(sub);

        let summary = ViewSummary {
            id: view_id,
            name: Some("MyView".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::Interconnection);
    }

    #[test]
    fn resolve_kind_unknown_supertype_falls_to_general() {
        let (graph, summary) = view_with_supertype("SomethingElse");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::General);
    }

    #[test]
    fn resolve_kind_dangling_requirement_supertype_falls_to_general() {
        // The pristine stdlib defines no `Requirement` / `RequirementView`
        // view defs (those were inventions of a retired local patch), so
        // `view def MyReqs :> Requirement` is a dangling supertype: it must
        // NOT classify as anything, taking the declared-but-unresolved
        // branch (General + warning) instead.
        let (graph, summary) = view_with_supertype("Requirement");
        assert_eq!(resolve_view_kind(&graph, &summary), ViewType::General);
    }

    #[test]
    fn from_view_usage_infers_kind_from_rendering_name() {
        let graph = ModelGraph::new();
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: None,
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![RenderingRef {
                id: ElementFactory::create(ElementKind::ViewRenderingMembership).id,
                name: Some("ActionFlowView".into()),
            }],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert_eq!(r.view_type, ViewType::ActionFlow);
    }

    #[test]
    fn from_view_usage_parametric_own_name_no_longer_special_cases() {
        // `Parametric` was retired as a view kind. A view literally named
        // `Parametric` with no `:>` supertype now resolves to General (the
        // spec root) — the legacy own-name escape hatch is gone. No
        // overlays/hints are ever attached by this constructor either.
        let graph = ModelGraph::new();
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: Some("Parametric".into()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        assert_eq!(r.view_type, ViewType::General);
        assert!(r.overlays.is_empty());
        assert!(r.hints.is_none());
    }

    #[test]
    fn from_view_usage_attaches_filter_when_view_has_filter_member() {
        let graph = ModelGraph::new();
        let filter_id = ElementFactory::create(ElementKind::ElementFilterMembership).id;
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: None,
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![filter_id.clone()],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        let f = r.filter.as_ref().unwrap();
        assert_eq!(f.expressions, vec![filter_id]);
    }

    // -- T4 — DiagramRequestKey ------------------------------------

    #[test]
    fn cache_key_returns_some_for_plain_request() {
        let r = ViewRequest::new(ViewType::General);
        let key = r.cache_key().expect("plain request must cache");
        assert_eq!(key.view_type(), ViewType::General);
        assert!(key.expanded_ids().is_empty());
        assert!(key.exposes().is_empty());
    }

    #[test]
    fn cache_key_returns_none_when_filter_set() {
        let r = ViewRequest::new(ViewType::General)
            .with_filter(ViewFilter::new().with_kinds([ElementKind::PartUsage]));
        assert!(r.cache_key().is_none());
    }

    #[test]
    fn cache_key_returns_none_when_hints_set() {
        let r = ViewRequest::new(ViewType::General).with_hints(RenderingHints::new());
        assert!(r.cache_key().is_none());
    }

    /// Minimal `&'static dyn DiagramOverlay` for cache-key tests — the
    /// structural overlays that used to be re-exported here are gone (the
    /// Requirements/Parametric peers were retired), so the test owns one.
    #[derive(Debug)]
    struct NoopOverlay;
    impl crate::ir::DiagramOverlay for NoopOverlay {
        fn name(&self) -> &'static str {
            "test-noop"
        }
        fn apply(&self, _ir: &mut crate::ir::types::DiagramIR, _graph: &ModelGraph) {}
    }
    static NOOP_OVERLAY: NoopOverlay = NoopOverlay;

    #[test]
    fn cache_key_returns_none_when_overlays_present() {
        let r = ViewRequest::new(ViewType::General);
        let key = r.cache_key();
        // Sanity: with no overlays yet, this is cacheable.
        assert!(key.is_some());
        // Once an overlay is attached, the key disappears.
        let with_overlay = r.with_overlay(&NOOP_OVERLAY);
        assert!(with_overlay.cache_key().is_none());
    }

    #[test]
    fn cache_key_orders_expanded_ids_deterministically() {
        // Two requests with the same `expanded_ids` strings inserted in
        // different orders must produce equal keys (and equal hashes).
        let id_a = ElementId::from_string("a-element");
        let id_b = ElementId::from_string("b-element");
        let id_c = ElementId::from_string("c-element");

        let mut s1 = HashSet::new();
        s1.insert(id_a.to_string());
        s1.insert(id_b.to_string());
        s1.insert(id_c.to_string());

        let mut s2 = HashSet::new();
        s2.insert(id_c.to_string());
        s2.insert(id_a.to_string());
        s2.insert(id_b.to_string());

        let k1 = ViewRequest::new(ViewType::General)
            .with_expanded(s1)
            .cache_key()
            .unwrap();
        let k2 = ViewRequest::new(ViewType::General)
            .with_expanded(s2)
            .cache_key()
            .unwrap();
        assert_eq!(k1, k2);

        let mut h1 = std::collections::hash_map::DefaultHasher::new();
        let mut h2 = std::collections::hash_map::DefaultHasher::new();
        use std::hash::Hasher;
        std::hash::Hash::hash(&k1, &mut h1);
        std::hash::Hash::hash(&k2, &mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn cache_key_distinguishes_expose_targets() {
        let id_a = ElementId::from_string("a-element");
        let id_b = ElementId::from_string("b-element");
        let k1 = ViewRequest::new(ViewType::General)
            .with_expose(id_a)
            .cache_key()
            .unwrap();
        let k2 = ViewRequest::new(ViewType::General)
            .with_expose(id_b)
            .cache_key()
            .unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_distinguishes_view_id_for_same_kind_and_expose() {
        // Two declared views with identical kind + expose are distinct
        // diagrams (own frame, own cache slot) — the view's own id keys
        // them apart. Without view_id in the key they would collide.
        let expose = ElementId::from_string("shared-expose");
        let mut a = ViewRequest::new(ViewType::General).with_expose(expose.clone());
        a.view_id = Some(ElementId::from_string("view-a"));
        let mut b = ViewRequest::new(ViewType::General).with_expose(expose);
        b.view_id = Some(ElementId::from_string("view-b"));
        let ka = a.cache_key().unwrap();
        let kb = b.cache_key().unwrap();
        assert_ne!(ka, kb);
        assert_eq!(ka.view_id(), Some(&ElementId::from_string("view-a")));
    }

    #[test]
    fn cache_key_roundtrip_preserves_view_id() {
        let mut req = ViewRequest::new(ViewType::General);
        req.view_id = Some(ElementId::from_string("v-7"));
        let key = req.cache_key().expect("cacheable");
        assert_eq!(key.view_id(), Some(&ElementId::from_string("v-7")));
        assert_eq!(key.to_view_request().view_id, req.view_id);
    }

    #[test]
    fn cache_key_roundtrip_to_view_request_drops_only_uncacheable_fields() {
        let id = ElementId::from_string("e-1");
        let mut expanded = HashSet::new();
        expanded.insert(id.to_string());
        let req = ViewRequest::new(ViewType::Interconnection)
            .with_expanded(expanded.clone())
            .with_expose(id.clone());
        let key = req.cache_key().expect("cacheable");
        let roundtripped = key.to_view_request();
        assert_eq!(roundtripped.view_type, ViewType::Interconnection);
        assert_eq!(roundtripped.expanded_ids, expanded);
        assert!(roundtripped.exposes.contains(&id));
        assert!(roundtripped.filter.is_none());
        assert!(roundtripped.hints.is_none());
        assert!(roundtripped.overlays.is_empty());
    }

    #[test]
    fn from_view_usage_collects_every_filter_member() {
        // Spec ElementFilterMembership composes as conjunction — when a
        // view authors multiple `filter` clauses they must all stack
        // into the request, not get truncated to the last one.
        let graph = ModelGraph::new();
        let f1 = ElementFactory::create(ElementKind::ElementFilterMembership).id;
        let f2 = ElementFactory::create(ElementKind::ElementFilterMembership).id;
        let f3 = ElementFactory::create(ElementKind::ElementFilterMembership).id;
        let summary = ViewSummary {
            id: ElementFactory::create(ElementKind::ViewDefinition).id,
            name: None,
            kind: ElementKind::ViewDefinition,
            exposed: vec![],
            renderings: vec![],
            filters: vec![f1.clone(), f2.clone(), f3.clone()],
            source_span: None,
        };
        let r = ViewRequest::from_view_usage(&graph, &summary);
        let filt = r.filter.as_ref().expect("filter attached");
        assert_eq!(filt.expressions, vec![f1, f2, f3]);
    }
}
