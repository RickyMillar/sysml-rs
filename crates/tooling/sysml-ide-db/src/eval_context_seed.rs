//! `EvalContext` seed walk — graph → context binding extraction.
//!
//! Walks every named element in a `ModelGraph` and binds default values into
//! a new `EvalContext`. Per ADR-011 §3 (Option B principle-clean), this is the
//! pure graph-derivative compilation logic for `EvalContext` and lives in
//! `sysml-ide-db` rather than `sysml-runtime`. The tracked queries in
//! [`crate::eval_context`] (`workspace_eval_context_with_library` and
//! friends) wrap these functions to cache the seed across requests.
//!
//! ISQ auto-tagging travels with the seed: if an attribute's type annotation
//! matches an ISQ quantity type, the numeric default is wrapped as
//! `Value::Quantity` with the matching dimension vector. As of RSC-5.1 (M2) the
//! tagging logic has a single home — `sysml_runtime::compiler::maybe_tag_isq` —
//! and this seed delegates to it rather than carrying an identical copy.
//!
//! `apply_overrides` and `parse_value_string` stay in `sysml-runtime` —
//! they're pure `EvalContext`/`Value` helpers with no graph dependency and
//! are reused across many service call sites.

use std::sync::Arc;

use sysml_core::{ElementKind, ModelGraph, Value};
use sysml_runtime::calculations::CalculationRegistry;
use sysml_runtime::compiler::{is_calc_scoped_seed_feature, maybe_tag_isq};
use sysml_runtime::expressions::EvalContext;

/// Build an [`EvalContext`] from a [`ModelGraph`].
///
/// Equivalent to [`context_from_graph_with_options`] with `include_stdlib =
/// false`. Stdlib elements (anything owned by a library package registered
/// via `ModelGraph::register_library_package`) are excluded from the
/// context — for a large multi-subsystem workspace that drops the live
/// variable map from ~14k entries to ~500.
///
/// See [`context_from_graph_with_options`] for the opt-in variant.
pub fn context_from_graph(graph: &Arc<ModelGraph>) -> EvalContext {
    context_from_graph_with_options(graph, false)
}

/// Build an [`EvalContext`] from a [`ModelGraph`] with explicit control over
/// whether stdlib-owned elements are bound in the context.
///
/// Extracts attribute values from elements so that constraint and verification
/// evaluation can reference them by name. Attaches the graph for lazy
/// feature-chain resolution of `Value::Ref` entries.
///
/// Also performs ISQ auto-tagging: if an attribute's type annotation matches
/// an ISQ quantity type (e.g., `LengthValue`, `MassValue`), the numeric default
/// is automatically wrapped as `Value::Quantity` with the corresponding
/// dimension vector from `sysml_core::physics::ISQ_TYPES`.
///
/// `include_stdlib = true` reinstates stdlib bindings; consumers that need
/// to evaluate expressions against stdlib-defined calc defs or
/// `ConvertQuantity` lookups can opt in. The default (`false`) matches the
/// pattern already used by `evaluation.rs` and `constraints.rs`, which skip
/// library elements via `graph.is_library_element`.
pub fn context_from_graph_with_options(
    graph: &Arc<ModelGraph>,
    include_stdlib: bool,
) -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.graph = Some(Arc::clone(graph));

    // `graph.elements` is an `FxHashMap`, whose iteration order is per-process
    // random. Several seed paths write the same context key from different
    // elements (e.g. each multiplied instance's `config.loadName`), so a
    // last-writer-wins collision must resolve DETERMINISTICALLY for two builds
    // of the same graph to agree (WS-C build determinism). Iterate in sorted
    // `ElementId` order — stable for a given elaborated graph.
    let mut elements: Vec<&sysml_core::Element> = graph.elements.values().collect();
    elements.sort_by(|a, b| a.id.cmp(&b.id));
    for element in elements {
        // Skip expression-AST element kinds — these carry `name` copied from
        // the source identifier (e.g. a `FeatureReferenceExpression` named
        // "bimetalTemp") but are not value-carrying model features. Binding
        // them into the context shadows the real AttributeUsage with the
        // same name and causes infinite recursion in `try_eval_unresolved`.
        if is_expression_ast_kind(&element.kind) {
            continue;
        }
        // Skip calc-def-internal (invocation-scoped) features — a calc `return
        // B`/`in Bs`/local must not shadow a model attribute under its bare
        // name in the runtime context. See
        // `sysml_runtime::compiler::is_calc_scoped_seed_feature` (RSC-3.7
        // amendment §A; KerML §8.3.4.6.4/§8.3.4.7.8). Single home for the
        // predicate — both this seed and the runtime twin call it.
        if is_calc_scoped_seed_feature(graph, element) {
            continue;
        }
        // Skip stdlib-owned elements unless the caller has opted in. This
        // drops ~13.5k entries on typical workspaces where nearly every
        // resolution-reachable element lives inside `ISQ`, `SI`,
        // `QuantityCalculations`, etc. Name resolution still sees stdlib
        // types via the scope tables; only the scalar bindings in the
        // live variable map are affected.
        if !include_stdlib && graph.is_library_element(&element.id) {
            continue;
        }
        if let Some(name) = &element.name {
            // Bind "value" property.
            if let Some(val) = element.get_prop("value") {
                let val = maybe_tag_isq(graph, element, val.clone());
                ctx.set(name.clone(), val);
                continue;
            }
            // Bind "default" values (only if "value" not already set).
            if let Some(val) = element.get_prop("default") {
                let val = maybe_tag_isq(graph, element, val.clone());
                ctx.set(name.clone(), val);
                continue;
            }
            // Check literal children: the parser creates child LiteralInteger/
            // LiteralRational elements for `attribute x = 85` rather than setting
            // a "value" property on the attribute itself.
            let mut found_literal = false;
            for child in graph.children_of(&element.id) {
                if matches!(
                    child.kind,
                    ElementKind::LiteralInteger
                        | ElementKind::LiteralRational
                        | ElementKind::LiteralBoolean
                        | ElementKind::LiteralString
                ) {
                    if let Some(val) = child.get_prop("value") {
                        let val = maybe_tag_isq(graph, element, val.clone());
                        ctx.set(name.clone(), val);
                        found_literal = true;
                        break;
                    }
                }
            }
            // Store as Ref for lazy feature chain resolution, but only if no
            // concrete value has already been bound under this name by an
            // earlier iteration. SysML allows the same feature name to appear
            // on a definition (no value) and a usage (with a value, e.g.
            // `attribute level = 50` on a usage of a def declaring `level`).
            // `graph.elements` iteration is keyed by ElementId, so the
            // value-less definition may be visited AFTER the value-bearing
            // usage; without this guard its Ref overwrites the usage's concrete
            // value and constraint evaluation degrades to "undefined variable —
            // inconclusive" for an attribute the model actually does bind.
            // Concrete values are sticky; only Ref-with-Ref is overwritten so
            // the latest-seen element id wins for chaining. This mirrors the
            // runtime twin `sysml_runtime::compiler::context_from_graph` — the
            // two seed walks must agree (RSC-3.1 duplicate-path parity).
            if !found_literal {
                let new_val = Value::Ref(element.id.clone());
                match ctx.get(name) {
                    Some(existing) if !matches!(existing, Value::Ref(_)) => {
                        // Keep the concrete value already bound; don't shadow with a Ref.
                    }
                    _ => {
                        ctx.set(name.clone(), new_val);
                    }
                }
            }
        }
    }
    ctx.occurrence_registry = Some(std::sync::Arc::new(
        std::sync::Mutex::new(sysml_core::occurrence::OccurrenceRegistry::new()),
    ));

    // Attach the calculation registry compiled from this graph. The walk is
    // O(N_calcs) — every `CalculationDefinition` becomes a `CalculationIR`
    // — and was previously re-run on every `build_workspace_orchestrator`
    // call. Bundling it into the cached `EvalContext` makes it free after
    // the salsa-cached first call. Diagnostics are dropped (production
    // discards them; the LSP surfaces calc diagnostics through other
    // queries).
    let (calc_registry, _calc_diags) = CalculationRegistry::compile_all_from_graph(graph);
    if !calc_registry.is_empty() {
        ctx.calculations = Some(Arc::new(calc_registry));
    }

    // Attach the spatial frame registry detected from this graph. Pure
    // graph derivative — `sysml_core::spatial::detect_spatial_frames` is
    // a one-pass element scan looking for `SpatialFrame` /
    // `CartesianSpatialFrame` / `CoordinateFrame` type names.
    let frame_registry = sysml_core::spatial::detect_spatial_frames(graph);
    if frame_registry.frame_count() > 0 {
        ctx.frame_registry = Some(Arc::new(frame_registry));
    }

    ctx
}

/// True for element kinds produced by the expression AST builder. These are
/// structural nodes inside an expression subtree (operator/literal/feature-ref
/// expression elements), not value-carrying features. They are excluded from
/// `context_from_graph` so that identifiers inside expressions don't shadow
/// the real attribute bindings they refer to.
fn is_expression_ast_kind(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::LiteralBoolean
            | ElementKind::LiteralInteger
            | ElementKind::LiteralRational
            | ElementKind::LiteralString
            | ElementKind::LiteralInfinity
            | ElementKind::LiteralExpression
            | ElementKind::NullExpression
            | ElementKind::OperatorExpression
            | ElementKind::InvocationExpression
            | ElementKind::FeatureReferenceExpression
            | ElementKind::FeatureChainExpression
            | ElementKind::SelectExpression
            | ElementKind::CollectExpression
            | ElementKind::IndexExpression
            | ElementKind::MetadataAccessExpression
            | ElementKind::ConstructorExpression
    )
}

// RSC-5.1 (M2): `maybe_tag_isq` + its `resolve_attribute_type_name` helper were
// deleted here and now live solely in `sysml_runtime::compiler` (imported above).
// This seed delegates to that single home — see the module doc.
