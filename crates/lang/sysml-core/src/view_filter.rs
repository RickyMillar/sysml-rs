//! Spec-faithful view filtering — `ElementFilterMembership` / `viewCondition`.
//!
//! A `ViewFilter` is a conjunction (or disjunction) of three independent
//! criteria: kind whitelist, stereotype whitelist, and an optional
//! Boolean-valued Expression (spec `viewCondition`). It mirrors the
//! `ElementFilterMembership` mechanism described in `SysML-vocab.ttl`
//! (line 325): "ElementFilterMembership is a Membership between a
//! Namespace and a model-level evaluable Boolean-valued Expression,
//! asserting that imported members of the Namespace should be filtered
//! using the condition Expression."
//!
//! ## Design choices
//!
//! - **Strict kind matching.** `kinds: [PartUsage]` matches only
//!   `ElementKind::PartUsage`, not subtypes. Subtype expansion is a
//!   user-layer concern — presets that want to include subtypes list
//!   them explicitly. This keeps sysml-core spec-faithful (filter says
//!   exactly what it means) and pushes UX shortcuts up the stack.
//! - **Bombproof stereotype matching.** Reuses the same four-strategy
//!   `is_metadata_typed_as` matcher used by ToolExecution / ToolVariable
//!   queries — handles unresolved type names, parser-named metadata
//!   blocks, `TypeOf` outgoing relationships, and explicit `FeatureTyping`
//!   elements pointing at the metadata.
//! - **Expression evaluation deferred.** The `expression` field stores
//!   a Boolean Expression's `ElementId`. Evaluating it requires the
//!   expression-eval engine that lands with Phase 5 (user-authored
//!   ViewUsage rendering). Until then, presence of `expression` is
//!   treated as `true` — a NOT-YET-EVALUATED criterion never excludes.
//! - **Empty filter passes everything.** A default-constructed
//!   `ViewFilter` has no active criteria and `matches()` returns `true`
//!   for all elements. This makes "no filter" a safe identity.
//!
//! ## Composition
//!
//! Multiple `ElementFilterMembership`s on the same Namespace compose as
//! conjunction per spec — every condition must hold. `FilterCombine::And`
//! is the default and matches that semantics. `Or` exists for UX
//! convenience presets.

use crate::{Element, ElementId, ElementKind, ModelGraph};

/// A spec-faithful element filter combining kind, stereotype, and
/// expression-based criteria.
///
/// See module docs for design rationale.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ViewFilter {
    /// Whitelist of element kinds. `None` = no kind constraint.
    /// Strict equality — `kinds: [Feature]` does NOT match `PartUsage`.
    pub kinds: Option<Vec<ElementKind>>,

    /// Whitelist of stereotype type names. `None` = no stereotype
    /// constraint. An element matches if any of its `MetadataUsage`
    /// children is typed as one of the listed names (via the
    /// bombproof [`crate::metadata::is_metadata_typed_as`] matcher).
    pub stereotypes: Option<Vec<String>>,

    /// Boolean-valued Expressions (spec `viewCondition`). Empty = no
    /// expression criterion. Each entry references an
    /// `ElementFilterMembership` whose Boolean Expression is evaluated by
    /// the runtime via `passes_filter`. Multiple expressions compose as
    /// conjunction per spec — every entry must evaluate to `true` for
    /// the element to pass. `combine` controls how the *expression-as-a-
    /// criterion* combines with kinds and stereotypes; the AND across
    /// individual expressions is independent of that combine mode.
    pub expressions: Vec<ElementId>,

    /// How active criteria combine. `And` (default) matches spec
    /// `ElementFilterMembership` composition. `Or` is a UX convenience.
    pub combine: FilterCombine,
}

/// How active filter criteria combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FilterCombine {
    /// All active criteria must match. Default; matches spec semantics
    /// for stacked `ElementFilterMembership`s on a Namespace.
    #[default]
    And,
    /// At least one active criterion must match.
    Or,
}

impl ViewFilter {
    /// New empty filter — no criteria active. `matches()` returns `true`
    /// for every element.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a kind whitelist. Replaces any prior kind list.
    pub fn with_kinds(mut self, kinds: impl IntoIterator<Item = ElementKind>) -> Self {
        self.kinds = Some(kinds.into_iter().collect());
        self
    }

    /// Add a stereotype whitelist. Replaces any prior stereotype list.
    pub fn with_stereotypes(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.stereotypes = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// Append a `viewCondition` Expression element id. Multiple calls
    /// stack — each expression is AND-combined at evaluation time.
    pub fn with_expression(mut self, expr: ElementId) -> Self {
        self.expressions.push(expr);
        self
    }

    /// Replace the expression list with the supplied ids.
    pub fn with_expressions(mut self, exprs: impl IntoIterator<Item = ElementId>) -> Self {
        self.expressions = exprs.into_iter().collect();
        self
    }

    /// Switch combine mode to `Or`.
    pub fn or(mut self) -> Self {
        self.combine = FilterCombine::Or;
        self
    }

    /// Switch combine mode to `And` (default).
    pub fn and(mut self) -> Self {
        self.combine = FilterCombine::And;
        self
    }

    /// True if no criterion is active. An empty filter passes everything.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_none() && self.stereotypes.is_none() && self.expressions.is_empty()
    }

    /// True if the element passes this filter.
    ///
    /// Active-criteria semantics:
    /// - `kinds = None` → inactive (does not constrain)
    /// - `kinds = Some([])` → matches nothing
    /// - `kinds = Some(list)` → strict equality against the list
    /// - `stereotypes = None` → inactive
    /// - `stereotypes = Some([])` → matches nothing
    /// - `stereotypes = Some(list)` → at least one MetadataUsage child
    ///   typed as a listed name
    /// - `expression = None` → inactive
    /// - `expression = Some(_)` → `true` (evaluation deferred to Phase 5)
    ///
    /// Active criteria combine per [`FilterCombine`]. If no criterion
    /// is active, returns `true`.
    pub fn matches(&self, element: &Element, graph: &ModelGraph) -> bool {
        let mut active: Vec<bool> = Vec::with_capacity(3);

        if let Some(list) = self.kinds.as_ref() {
            active.push(list.iter().any(|k| element.kind == *k));
        }

        if let Some(list) = self.stereotypes.as_ref() {
            active.push(matches_any_stereotype(graph, element, list));
        }

        if !self.expressions.is_empty() {
            // Spec evaluation happens in `passes_filter` (the runtime-
            // aware caller). Here we treat the expression-as-a-criterion
            // as a single contribution to the active list — neutral
            // under AND, contributing under OR. The N-way AND across
            // individual expressions is enforced separately by
            // `passes_filter`, independent of `combine`.
            active.push(true);
        }

        if active.is_empty() {
            return true;
        }
        match self.combine {
            FilterCombine::And => active.iter().all(|b| *b),
            FilterCombine::Or => active.iter().any(|b| *b),
        }
    }

    /// Iterator over every element in `graph` that passes this filter.
    pub fn collect<'a>(&'a self, graph: &'a ModelGraph) -> impl Iterator<Item = &'a Element> + 'a {
        graph
            .elements
            .values()
            .filter(move |e| self.matches(e, graph))
    }
}

/// True if the element has at least one `MetadataUsage` child typed as
/// one of the listed stereotype names.
fn matches_any_stereotype(graph: &ModelGraph, element: &Element, names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    for child in graph.children_of(&element.id) {
        if child.kind != ElementKind::MetadataUsage {
            continue;
        }
        for name in names {
            if crate::metadata::is_metadata_typed_as(graph, child, name) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ElementFactory, ModelGraph};

    fn graph_with(kind: ElementKind, name: &str) -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();
        let elem = ElementFactory::create(kind).with_name(name);
        let id = elem.id.clone();
        graph.add_element(elem);
        (graph, id)
    }

    #[test]
    fn default_filter_matches_everything() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter::new();
        assert!(filter.is_empty());
        assert!(filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn kinds_filter_matches_self_strictly() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter::new().with_kinds([ElementKind::PartUsage]);
        assert!(filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn kinds_filter_does_not_lift_to_supertype() {
        // Strictness: a filter on Feature must NOT match PartUsage,
        // even though PartUsage is_subtype_of(Feature).
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter::new().with_kinds([ElementKind::Feature]);
        assert!(!filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn kinds_filter_rejects_other_kinds() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter::new().with_kinds([ElementKind::ActionUsage]);
        assert!(!filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn empty_kind_list_matches_nothing() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter {
            kinds: Some(Vec::new()),
            ..Default::default()
        };
        assert!(!filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn stereotype_filter_matches_metadata_usage_typename() {
        let mut graph = ModelGraph::new();
        let part = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        let part_id = part.id.clone();
        graph.add_element(part);

        // Add a MetadataUsage child typed as "ToolExecution" via
        // `unresolvedTypeName` (Strategy 1 of is_metadata_typed_as).
        // `add_element` reads `element.owner` to populate the children
        // index, so set the owner before insertion.
        let mut meta =
            ElementFactory::create(ElementKind::MetadataUsage).with_owner(part_id.clone());
        meta.set_prop("unresolvedTypeName", "ToolExecution");
        graph.add_element(meta);

        let filter = ViewFilter::new().with_stereotypes(["ToolExecution".to_owned()]);
        assert!(filter.matches(graph.get_element(&part_id).unwrap(), &graph));
    }

    #[test]
    fn stereotype_filter_rejects_when_no_matching_metadata_child() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter::new().with_stereotypes(["ToolExecution".to_owned()]);
        assert!(!filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn empty_stereotype_list_matches_nothing() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter {
            stereotypes: Some(Vec::new()),
            ..Default::default()
        };
        assert!(!filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn and_combine_requires_all_active_criteria() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        // kinds matches, but stereotype does not — AND should fail.
        let filter = ViewFilter::new()
            .with_kinds([ElementKind::PartUsage])
            .with_stereotypes(["NoSuch".to_owned()]);
        assert!(!filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn or_combine_passes_if_any_criterion_passes() {
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        // kinds matches, stereotype does not — OR should pass.
        let filter = ViewFilter::new()
            .with_kinds([ElementKind::PartUsage])
            .with_stereotypes(["NoSuch".to_owned()])
            .or();
        assert!(filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn expression_presence_treated_as_true_in_matches() {
        // `matches()` is the spec-criteria gate (kinds + stereotypes +
        // expression-presence). It does not evaluate Boolean
        // expressions — that work belongs to the runtime-aware
        // `GeneratorContext::passes_filter`. So an expression-only
        // filter must pass `matches()` for every element.
        let (graph, id) = graph_with(ElementKind::PartUsage, "p");
        let filter = ViewFilter::new().with_expression(ElementId::new_v4());
        assert!(filter.matches(graph.get_element(&id).unwrap(), &graph));
    }

    #[test]
    fn multiple_expressions_stack_into_vec() {
        let a = ElementId::new_v4();
        let b = ElementId::new_v4();
        let filter = ViewFilter::new()
            .with_expression(a.clone())
            .with_expression(b.clone());
        assert_eq!(filter.expressions, vec![a, b]);
    }

    #[test]
    fn collect_returns_only_matching_elements() {
        let mut graph = ModelGraph::new();
        graph.add_element(ElementFactory::create(ElementKind::PartUsage).with_name("p1"));
        graph.add_element(ElementFactory::create(ElementKind::PartUsage).with_name("p2"));
        graph.add_element(ElementFactory::create(ElementKind::ActionUsage).with_name("a"));

        let filter = ViewFilter::new().with_kinds([ElementKind::PartUsage]);
        let names: Vec<&str> = filter
            .collect(&graph)
            .filter_map(|e| e.name.as_deref())
            .collect();
        // Order is HashMap-iteration-dependent; check membership only.
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"p1"));
        assert!(names.contains(&"p2"));
    }
}
