//! ViewGenerator trait and registry.
//!
//! Each diagram view type implements `ViewGenerator`, providing compile-time
//! enforcement that all required methods are present. Adding a new `ViewType`
//! variant without implementing the trait = compile error.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sysml_core::{Element, ElementId, ModelGraph, ViewFilter};
use sysml_runtime::expressions::ExprIR;

use crate::ir::types::DiagramIR;
use crate::smodel::ViewType;

use super::generators::action::ActionFlowViewGenerator;
use super::generators::browser::BrowserViewGenerator;
use super::generators::general::GeneralViewGenerator;
use super::generators::geometry::GeometryViewGenerator;
use super::generators::grid::GridViewGenerator;
use super::generators::interconnection::InterconnectionViewGenerator;
use super::generators::sequence::SequenceViewGenerator;
use super::generators::state::StateTransitionViewGenerator;

/// Context passed to all generators. Avoids parameter sprawl.
///
/// Construct with [`GeneratorContext::new`] and chain `with_filter` /
/// `with_hints` to attach optional spec filtering and layout overrides.
/// Both extras are `None` by default, matching the pre-4.5 behaviour.
pub struct GeneratorContext<'a> {
    pub graph: &'a ModelGraph,
    pub expanded_ids: &'a HashSet<String>,
    /// Optional spec-faithful element filter (`ElementFilterMembership`
    /// / `viewCondition`). Generators consult [`Self::passes_filter`] in
    /// their top-level element collection. Edges into filtered-out
    /// elements are pruned by the existing dangling-endpoint check.
    pub filter: Option<&'a ViewFilter>,
    /// Spec `Expose` targets — when non-empty, the diagram is centred on these
    /// elements instead of every effectively-top-level element. See
    /// [`Self::is_canvas_root`].
    pub expose_ids: &'a [ElementId],
    /// Optional precompiled-filter cache (S3.T6b). When set,
    /// [`Self::passes_filter`] looks expressions up in this map
    /// before falling back to
    /// `sysml_runtime::view_condition::evaluate_view_condition`,
    /// which compiles on every call. Keyed by
    /// `ElementFilterMembership` element id. Holders missing from the
    /// map fall through to the on-demand path (same fall-through-
    /// to-true safety as before).
    pub filter_cache: Option<&'a HashMap<ElementId, Arc<ExprIR>>>,
    /// Per-render memo of the graph wrapped in an `Arc`, built lazily on the
    /// first [`Self::passes_filter`] call. Sharing one `Arc` across every
    /// element check replaces the former per-element `ModelGraph` deep clone
    /// (which made filtered views over the stdlib-merged workspace graph
    /// effectively never terminate — the Requirement-view render hang).
    graph_arc: OnceCell<Arc<ModelGraph>>,
    /// Per-render memo of the active filter's resolved + compiled expressions
    /// (`Arc<ExprIR>`), built once on the first [`Self::passes_filter`] call.
    /// Avoids recompiling the filter expression for every candidate element on
    /// the cache-miss path.
    resolved_exprs: OnceCell<Vec<Arc<ExprIR>>>,
}

impl<'a> GeneratorContext<'a> {
    /// New context with no filter or hints (pre-4.5 behaviour).
    pub fn new(graph: &'a ModelGraph, expanded_ids: &'a HashSet<String>) -> Self {
        Self {
            graph,
            expanded_ids,
            filter: None,
            expose_ids: &[],
            filter_cache: None,
            graph_arc: OnceCell::new(),
            resolved_exprs: OnceCell::new(),
        }
    }

    /// Attach a precompiled-filter-expression cache. When present,
    /// [`Self::passes_filter`] uses cached `ExprIR` for filter
    /// memberships present in the map, skipping the per-element
    /// compile step. See ADR-011 §3 / S3.T6b.
    pub fn with_filter_cache(
        mut self,
        cache: &'a HashMap<ElementId, Arc<ExprIR>>,
    ) -> Self {
        self.filter_cache = Some(cache);
        self
    }

    /// Attach a [`ViewFilter`].
    pub fn with_filter(mut self, filter: &'a ViewFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Attach a single spec `Expose` target — convenience for single-expose.
    pub fn with_expose(mut self, expose: &'a ElementId) -> Self {
        self.expose_ids = std::slice::from_ref(expose);
        self
    }

    /// Attach multiple spec `Expose` targets — generators centre the canvas
    /// on these elements when set.
    pub fn with_exposes(mut self, exposes: &'a [ElementId]) -> Self {
        self.expose_ids = exposes;
        self
    }

    /// True if `element` passes the active filter, or no filter is set.
    ///
    /// Wraps [`ViewFilter::matches`] (which handles kinds + stereotypes
    /// and stubs the expression criterion to `true`) and additionally
    /// evaluates any `viewCondition` Boolean Expression via
    /// [`sysml_runtime::view_condition::evaluate_view_condition`]. If
    /// the runtime evaluator returns `false`, the element is excluded
    /// regardless of what `ViewFilter::matches` reports.
    pub fn passes_filter(&self, element: &Element) -> bool {
        let Some(f) = self.filter else { return true };
        if !f.matches(element, self.graph) {
            return false;
        }
        if f.expressions.is_empty() {
            return true;
        }

        // Resolve + compile each filter expression ONCE per render (not per
        // element). Cache-first (S3.T6b precompiled `ExprIR`), else compile
        // on-demand via `resolve_filter_expr_ir`. A holder that resolves to no
        // expression is dropped — same fall-through-to-true safety as before
        // (an unresolvable filter never silently deletes elements).
        let exprs = self.resolved_exprs.get_or_init(|| {
            f.expressions
                .iter()
                .filter_map(|expr_id| {
                    if let Some(arc) =
                        self.filter_cache.and_then(|c| c.get(expr_id).cloned())
                    {
                        return Some(arc);
                    }
                    let holder = self.graph.get_element(expr_id)?;
                    sysml_runtime::view_condition::resolve_filter_expr_ir(self.graph, holder)
                        .map(Arc::new)
                })
                .collect()
        });
        if exprs.is_empty() {
            return true;
        }

        // Build the graph `Arc` once and share it across every element check.
        // The previous code wrapped `Arc::new(graph.clone())` *inside*
        // `evaluate_view_condition_*` on every call, deep-cloning the whole
        // (stdlib-merged) graph per candidate — quadratic, and effectively a
        // hang for a filtered view over a real workspace.
        let graph_arc = self.graph_arc.get_or_init(|| Arc::new(self.graph.clone()));

        // Spec ElementFilterMembership composes as conjunction — every
        // expression must evaluate to `true`. This is independent of
        // ViewFilter::combine, which only governs how kinds / stereotypes /
        // expression-presence stack.
        for expr in exprs {
            if !sysml_runtime::view_condition::evaluate_view_condition_with_compiled_shared(
                Arc::clone(graph_arc),
                expr.as_ref(),
                element,
            ) {
                return false;
            }
        }
        true
    }

    /// True if `element` should be treated as a canvas root by a
    /// generator's top-level collection step.
    ///
    /// - When `expose` is set: only the exposed element itself qualifies
    ///   — it becomes the diagram subject and ancestors are hidden.
    /// - Otherwise: falls back to
    ///   [`crate::visual_kind::is_effectively_top_level`], which lifts
    ///   through Package / LibraryPackage owners.
    pub fn is_canvas_root(&self, element: &Element) -> bool {
        if !self.expose_ids.is_empty() {
            return self.expose_ids.contains(&element.id);
        }
        crate::visual_kind::is_effectively_top_level(element, self.graph)
    }

    /// True if `element` sits inside the exposed scope: it IS an exposed
    /// element or an ownership-chain descendant of one (so exposing a
    /// package admits the definitions inside it). With no exposes set,
    /// everything qualifies — the fence only narrows declared views.
    ///
    /// This is the expose fence for generators whose top-level collection
    /// scans the whole graph by kind (state/action): without it, a declared
    /// `expose X` behavior view still swept in every same-kind definition in
    /// the merged graph — including the standard library.
    pub fn in_exposed_scope(&self, element: &Element) -> bool {
        if self.expose_ids.is_empty() {
            return true;
        }
        let mut cur = Some(&element.id);
        let mut hops = 0usize;
        while let Some(id) = cur {
            if self.expose_ids.iter().any(|e| e == id) {
                return true;
            }
            cur = self.graph.get_element(id).and_then(|e| e.owner.as_ref());
            hops += 1;
            if hops > 64 {
                return false; // ownership cycle guard
            }
        }
        false
    }
}

/// Trait for diagram view generators.
///
/// Each view type (General, State, Action, etc.) implements this trait.
/// The registry dispatches to the correct implementation at runtime.
pub trait ViewGenerator: Send + Sync {
    /// Which view type this generator handles.
    fn view_type(&self) -> ViewType;

    /// ELK algorithm for this view's top-level graph.
    fn elk_algorithm(&self) -> &str;

    /// ELK layout direction. `None` means the algorithm doesn't use direction.
    fn elk_direction(&self) -> Option<&str> {
        Some("DOWN")
    }

    /// Generate diagram IR from a model graph.
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR;

    /// Generate subtree for embedding inside another view.
    /// Returns `None` if this view doesn't support embedding.
    fn generate_for_owner(
        &self,
        _ctx: &GeneratorContext,
        _owner_id: &str,
    ) -> Option<DiagramIR> {
        None
    }
}

/// Get the generator for a view type.
///
/// All 8 view types have ViewGenerator implementations.
/// This is exhaustive — adding a new ViewType variant requires
/// implementing ViewGenerator for it (compile error otherwise).
pub fn get_generator(vt: ViewType) -> &'static dyn ViewGenerator {
    match vt {
        ViewType::ActionFlow => &ActionFlowViewGenerator,
        ViewType::Browser => &BrowserViewGenerator,
        ViewType::General => &GeneralViewGenerator,
        ViewType::Geometry => &GeometryViewGenerator,
        ViewType::Grid => &GridViewGenerator,
        ViewType::Interconnection => &InterconnectionViewGenerator,
        ViewType::Sequence => &SequenceViewGenerator,
        ViewType::StateTransition => &StateTransitionViewGenerator,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{ElementFactory, ElementKind};

    #[test]
    fn is_canvas_root_falls_back_to_top_level_when_expose_unset() {
        let mut graph = ModelGraph::new();
        let part = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        graph.add_element(part);
        let part_ref = graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("p"))
            .unwrap();
        let expanded = HashSet::new();
        let ctx = GeneratorContext::new(&graph, &expanded);
        assert!(ctx.is_canvas_root(part_ref));
    }

    #[test]
    fn is_canvas_root_only_passes_exposed_element_when_expose_set() {
        let mut graph = ModelGraph::new();
        let part_a = ElementFactory::create(ElementKind::PartUsage).with_name("a");
        let id_a = part_a.id.clone();
        graph.add_element(part_a);
        let part_b = ElementFactory::create(ElementKind::PartUsage).with_name("b");
        let id_b = part_b.id.clone();
        graph.add_element(part_b);

        let expanded = HashSet::new();
        let ctx = GeneratorContext::new(&graph, &expanded).with_expose(&id_a);

        let a = graph
            .elements
            .values()
            .find(|e| e.id == id_a)
            .unwrap();
        let b = graph
            .elements
            .values()
            .find(|e| e.id == id_b)
            .unwrap();
        assert!(ctx.is_canvas_root(a));
        assert!(!ctx.is_canvas_root(b));
    }

    /// Regression: a view with an expression filter must evaluate per element
    /// WITHOUT deep-cloning the whole `ModelGraph` each time. Before the
    /// Jun-2026 fix, `passes_filter` cloned the graph inside the runtime
    /// evaluator on every call, so a filtered view over the stdlib-merged
    /// workspace graph never terminated (the Requirement-view render hang).
    /// Here we just assert the filter discriminates correctly and that
    /// repeated evaluation (which now reuses one memoized graph `Arc` and the
    /// compiled expression) stays correct.
    #[test]
    fn passes_filter_expression_discriminates_and_is_reusable() {
        use sysml_core::ViewFilter;

        let mut graph = ModelGraph::new();
        // viewCondition holder: only PartUsage elements pass.
        let mut holder = ElementFactory::create(ElementKind::ElementFilterMembership);
        holder.set_prop("filterExpression", "kind == \"PartUsage\"");
        let holder_id = holder.id.clone();
        graph.add_element(holder);

        let part = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        let part_id = part.id.clone();
        graph.add_element(part);
        let attr = ElementFactory::create(ElementKind::AttributeUsage).with_name("a");
        let attr_id = attr.id.clone();
        graph.add_element(attr);

        let mut filter = ViewFilter::new();
        filter.expressions.push(holder_id);

        let expanded = HashSet::new();
        let ctx = GeneratorContext::new(&graph, &expanded).with_filter(&filter);

        let part_ref = graph.get_element(&part_id).unwrap();
        let attr_ref = graph.get_element(&attr_id).unwrap();

        // Evaluate repeatedly — exercises the per-render memo (one shared graph
        // Arc + one compiled expression) across many element checks.
        for _ in 0..64 {
            assert!(ctx.passes_filter(part_ref), "PartUsage should pass the filter");
            assert!(
                !ctx.passes_filter(attr_ref),
                "AttributeUsage should be excluded by the filter"
            );
        }
    }
}
