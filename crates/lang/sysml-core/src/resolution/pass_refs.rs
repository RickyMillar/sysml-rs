//! Reference-resolution pass: resolves reference-site identifiers that fall
//! outside the two-pass type/feature resolver.
//!
//! Phase B.1 handles `FeatureReferenceExpression` (FRE) — the identifiers inside
//! constraint / calc / binding expressions (`V_applied`, `N_drive`, `Ae`,
//! `i_drive`, ...). FREs are minted outside the `unresolved_*` prop machinery
//! (`expression_elements.rs::emit_feature_ref_from_text` sets only `name` +
//! `spans[0]`), so `has_unresolved_refs` never selects them. This pass selects
//! them by KIND instead — which also sidesteps the false-positive risk of a
//! generic name-presence gate (steward Q2).
//!
//! A successful resolution writes an ADDITIVE `resolved_props::FEATURE_REFERENCE`
//! (`Value::Ref`) — nothing reads that prop today, so blast radius is zero; the
//! semantic-token emitter (Phase C) will colour the reference by its resolved
//! target's kind. A miss leaves the FRE untouched and emits **no diagnostic**:
//! diagnostics own the "unresolved name" error signal, and the token layer marks
//! misses with the UNRESOLVED modifier (Phase D). Letting this pass also raise a
//! diagnostic would be a duplicate signal.
//!

use std::borrow::Cow;

use rustc_hash::FxHashSet;
use sysml_id::ElementId;

use super::context::ResolutionContext;
use super::resolved_props;
use super::scoping::non_expression::find_non_expression_namespace;
use crate::{ElementKind, ModelGraph};

/// Resolve bare-name `FeatureReferenceExpression`s in `graph`, appending each
/// successful resolution to `updates` as a `resolved_props::FEATURE_REFERENCE`
/// (`Value::Ref`) write.
///
/// **Bare single-segment names only (Phase B.1).** Dotted feature chains
/// (`w.mass`) and `::`-qualified names are skipped: they depend on Pass-1 type
/// resolution of the chain root (`chaining.rs`) and are the gated Phase B.1.2.
///
/// `graph` must be the same graph `ctx` was built over. Callers run this after
/// Pass 1 (on the pass-1-applied `temp_graph` where one exists) so the eventual
/// dotted-chain extension sees resolved supertypes; bare names within the local
/// declaration namespace do not strictly require it.
///
/// `exclude` (typically the library element IDs, from the excluding/with-library
/// paths) is skipped so already-resolved library FREs are neither re-resolved nor
/// re-emitted; pass `None` on the non-excluding paths.
///
/// Idempotent: an FRE that already carries `FEATURE_REFERENCE` is skipped.
pub(crate) fn resolve_feature_references(
    graph: &ModelGraph,
    ctx: &mut ResolutionContext<'_>,
    exclude: Option<&FxHashSet<ElementId>>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
) {
    for fre_id in graph.element_ids_by_kind(&ElementKind::FeatureReferenceExpression) {
        if exclude.is_some_and(|ex| ex.contains(fre_id)) {
            continue;
        }
        let Some(element) = graph.get_element(fre_id) else {
            continue;
        };
        let Some(name) = element.name.as_deref() else {
            continue;
        };
        // B.1 = bare single-segment names only. Skip dotted / qualified refs so
        // we never mint a wrong ref before the chain-aware B.1.2 lands.
        if name.is_empty() || name.contains('.') || name.contains("::") {
            continue;
        }
        // Idempotent re-run guard.
        if element.props.contains_key(resolved_props::FEATURE_REFERENCE) {
            continue;
        }
        // Scope root = the enclosing NON-expression declaration namespace,
        // walking past the expression wrappers the FRE lives inside.
        let start = element.owner.clone().unwrap_or_else(|| fre_id.clone());
        let scope_id = find_non_expression_namespace(graph, &start);
        if let Some(resolved_id) = ctx.resolve_qualified_name(&scope_id, name) {
            updates.push((
                fre_id.clone(),
                Cow::Borrowed(resolved_props::FEATURE_REFERENCE),
                resolved_id,
            ));
        }
        // Miss → leave untouched (see module doc).
    }
}
