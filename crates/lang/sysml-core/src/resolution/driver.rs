//! Resolution driver: orchestrates the two-pass resolution algorithm.
//!
//! This module contains the public entry points for name resolution:
//! - [`resolve_references`]: Resolves all unresolved references in a model graph.
//! - [`resolve_references_excluding`]: Resolves references, skipping specified elements.

use std::borrow::Cow;

#[cfg(feature = "parallel")]
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use sysml_id::{ElementId, QualifiedName};
use sysml_span::{Diagnostic, Diagnostics};

use crate::{ElementKind, ModelGraph};

use super::context::{InheritanceIndex, ResolutionContext};
use super::scope_table::ScopeTable;
use super::{
    apply_resolution_updates, pass1, pass2, primitive_type_alias, unresolved_props,
    ResolutionResult, ResolutionUpdate,
};

/// Minimum number of elements with unresolved references to trigger parallel resolution.
/// Below this threshold, the sequential path is used to avoid rayon overhead.
const PARALLEL_THRESHOLD: usize = 100;

/// Resolve all unresolved references in a model graph.
///
/// This is a convenience wrapper around [`resolve_references_pure`] that applies
/// the returned updates to the graph immediately. See that function for details
/// on the two-pass resolution algorithm.
///
/// # Arguments
///
/// * `graph` - The model graph to resolve (mutable)
///
/// # Returns
///
/// A `ResolutionResult` containing statistics and diagnostics.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(graph), fields(element_count = graph.element_count()))
)]
pub fn resolve_references(graph: &mut ModelGraph) -> ResolutionResult {
    let (updates, result) = resolve_references_pure(graph);
    // Trust resolved_count from the pure fn — it counts actual resolutions.
    apply_resolution_updates(graph, &updates);
    result
}

/// Resolve all unresolved references in a model graph without mutating it.
///
/// Returns the collected [`ResolutionUpdate`]s and a [`ResolutionResult`] with
/// diagnostics. The caller is responsible for applying updates (e.g., via
/// [`apply_resolution_updates`]).
///
/// Resolution is performed in two passes:
/// 1. **Type relationships pass**: Resolves Specialization, FeatureTyping, and Subclassification
///    which establish inheritance chains.
/// 2. **Feature relationships pass**: Resolves Subsetting, Redefinition, etc. which depend on
///    inherited members being visible through the resolved type hierarchy.
///
/// **Important**: Because this function does not mutate the graph, pass 2 cannot
/// see pass 1 results applied to the graph. For the sequential path, the function
/// internally applies pass 1 updates to a temporary clone so that pass 2 has correct
/// scope tables. For the parallel path, the same pre-existing two-phase strategy is
/// used. For small graphs (below `PARALLEL_THRESHOLD`), a lightweight approach avoids
/// cloning the entire graph by reusing the inheritance index from pass 1.
pub fn resolve_references_pure(graph: &ModelGraph) -> (Vec<ResolutionUpdate>, ResolutionResult) {
    // Collect elements that need resolution.
    // Skip kinds that never have unresolved cross-references (literals, docs,
    // memberships, etc.) to avoid checking all 75k+ elements.
    let elements_to_resolve: Vec<(ElementId, ElementKind)> = graph
        .elements
        .iter()
        .filter(|(_, e)| !is_never_resolvable(&e.kind) && has_unresolved_refs(e))
        .map(|(id, e)| (id.clone(), e.kind.clone()))
        .collect();

    // Use parallel path for large graphs, sequential for small ones
    if elements_to_resolve.len() > PARALLEL_THRESHOLD {
        return resolve_references_parallel_pure(graph, elements_to_resolve);
    }

    let mut result = ResolutionResult::new();
    let mut all_updates: Vec<ResolutionUpdate> = Vec::new();

    // =========================================================================
    // PASS 1: Resolve type relationships (establishes inheritance chains)
    // =========================================================================
    let mut pass1_raw_updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
    let mut pass1_unresolved: Vec<(ElementId, String, String)> = Vec::new();

    let pass1_inheritance_index = {
        let mut ctx = ResolutionContext::new(graph);

        for (element_id, kind) in &elements_to_resolve {
            let scope_id = graph
                .get_element(element_id)
                .and_then(|e| e.owner.clone())
                .unwrap_or_else(|| element_id.clone());

            let Some(element) = graph.get_element(element_id) else {
                continue;
            };

            match kind {
                // Also handles membership-wrapped role usages (subject,
                // objective) — see `is_role_membership_typing`: they stamp the
                // typing clause's target as `unresolved_type` directly on the
                // membership and must resolve through the same path so a
                // dangling target counts as unresolved (fail-hard) instead of
                // being silently dropped.
                k if k == &ElementKind::FeatureTyping
                    || k.is_subtype_of(ElementKind::FeatureTyping)
                    || is_role_membership_typing(k) =>
                {
                    pass1::resolve_feature_typing(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut pass1_raw_updates,
                        &mut pass1_unresolved,
                    );
                }
                // Subclassification must be checked BEFORE the generic
                // Specialization arm: Subclassification is a subtype of
                // Specialization but stores its supertype under
                // `unresolved_superclassifier`, not `unresolved_general`. If the
                // generic arm caught it first it would call `resolve_specialization`
                // (reads GENERAL) and silently resolve nothing, leaving the
                // supertype edge out of the inheritance index — so inherited
                // features of user-model `:>` definitions never resolve.
                k if k == &ElementKind::Subclassification
                    || k.is_subtype_of(ElementKind::Subclassification) =>
                {
                    pass1::resolve_subclassification(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut pass1_raw_updates,
                        &mut pass1_unresolved,
                    );
                }
                k if k == &ElementKind::Specialization
                    || k.is_subtype_of(ElementKind::Specialization) =>
                {
                    if !k.is_subtype_of(ElementKind::Subsetting) {
                        pass1::resolve_specialization(
                            element,
                            &scope_id,
                            &mut ctx,
                            &mut pass1_raw_updates,
                            &mut pass1_unresolved,
                        );
                    }
                }
                _ => {}
            }
        }

        ctx.take_inheritance_index()
    };

    // Convert pass 1 raw updates to ResolutionUpdate structs
    for (element_id, prop_name, resolved_id) in &pass1_raw_updates {
        all_updates.push(ResolutionUpdate {
            element_id: element_id.clone(),
            property_name: prop_name.clone(),
            resolved_value: resolved_id.clone(),
        });
    }
    result.resolved_count += pass1_raw_updates.len();

    // =========================================================================
    // PASS 2: Resolve feature relationships (uses inheritance chains from pass 1)
    // =========================================================================
    // Pass 2 must see the supertype edges that pass 1 just resolved (e.g. a
    // user-model `part def Car :> Vehicle`, whose `engine` feature is inherited
    // by `Car`). The pass-1 inheritance index was built from the *unmutated*
    // graph, so it does not yet contain those edges. Mirror the parallel path:
    // apply pass-1 updates to a temporary clone and rebuild the index from it so
    // inherited members of freshly-resolved user-model supertypes are visible.
    let _ = pass1_inheritance_index; // built pre-update; superseded below.
    let mut pass2_raw_updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
    let mut pass2_unresolved: Vec<(ElementId, String, String)> = Vec::new();

    let temp_graph = {
        let mut tg = graph.clone();
        apply_resolution_updates(&mut tg, &all_updates);
        tg
    };

    {
        let pass2_index = InheritanceIndex::build(&temp_graph);
        let mut ctx = ResolutionContext::new_with_index(&temp_graph, pass2_index);

        for (element_id, kind) in &elements_to_resolve {
            let scope_id = temp_graph
                .get_element(element_id)
                .and_then(|e| e.owner.clone())
                .unwrap_or_else(|| element_id.clone());

            let Some(element) = temp_graph.get_element(element_id) else {
                continue;
            };

            dispatch_pass2(
                element,
                &scope_id,
                kind,
                &mut ctx,
                &mut pass2_raw_updates,
                &mut pass2_unresolved,
            );
        }

        // Reference-site resolution (FRE): runs after Pass 1 on the pass-1-applied
        // temp_graph, reusing this context's scope tables. Bare names only (B.1);
        // writes additive `resolved_props::FEATURE_REFERENCE`, no diagnostics on
        // miss. See resolution/pass_refs.rs.
        super::pass_refs::resolve_feature_references(
            &temp_graph,
            &mut ctx,
            None,
            &mut pass2_raw_updates,
        );

        result.diagnostics = ctx.take_diagnostics();
    }

    // Convert pass 2 raw updates
    for (element_id, prop_name, resolved_id) in pass2_raw_updates {
        all_updates.push(ResolutionUpdate {
            element_id,
            property_name: prop_name,
            resolved_value: resolved_id,
        });
        result.resolved_count += 1;
    }

    // Record all unresolved references as diagnostics
    for (element_id, prop_name, unresolved_name) in
        pass1_unresolved.into_iter().chain(pass2_unresolved)
    {
        let mut diag =
            build_unresolved_diagnostic(graph, &element_id, &prop_name, &unresolved_name);
        attach_im010_suggestion(&mut diag, &[graph], &unresolved_name);
        result.diagnostics.push(diag);
        result.unresolved_count += 1;
    }

    (all_updates, result)
}

/// Resolve all cross-references in a model graph, excluding specified elements.
///
/// This is a convenience wrapper around [`resolve_references_excluding_pure`] that
/// applies the returned updates to the graph immediately.
///
/// # Arguments
///
/// * `graph` - The model graph to resolve (mutable)
/// * `exclude_ids` - Set of element IDs to skip during resolution
///
/// # Returns
///
/// A `ResolutionResult` containing statistics and diagnostics.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(graph, exclude_ids), fields(element_count = graph.element_count()))
)]
pub fn resolve_references_excluding(
    graph: &mut ModelGraph,
    exclude_ids: &FxHashSet<ElementId>,
) -> ResolutionResult {
    let (updates, result) = resolve_references_excluding_pure(graph, exclude_ids);
    // Trust resolved_count from the pure fn — it counts actual resolutions.
    apply_resolution_updates(graph, &updates);
    result
}

/// Resolve all cross-references in a model graph without mutating it, excluding
/// specified elements.
///
/// This is useful when resolving user-defined elements while excluding library
/// elements that have already been resolved.
///
/// # Arguments
///
/// * `graph` - The model graph to read (immutable)
/// * `exclude_ids` - Set of element IDs to skip during resolution
///
/// # Returns
///
/// A tuple of (`Vec<ResolutionUpdate>`, `ResolutionResult`).
pub fn resolve_references_excluding_pure(
    graph: &ModelGraph,
    exclude_ids: &FxHashSet<ElementId>,
) -> (Vec<ResolutionUpdate>, ResolutionResult) {
    // Collect elements that need resolution, excluding specified IDs
    let elements_to_resolve: Vec<(ElementId, ElementKind)> = graph
        .elements
        .iter()
        .filter(|(id, e)| !exclude_ids.contains(*id) && has_unresolved_refs(e))
        .map(|(id, e)| (id.clone(), e.kind.clone()))
        .collect();

    // Use parallel path for large element sets
    if elements_to_resolve.len() > PARALLEL_THRESHOLD {
        return resolve_references_excluding_parallel_pure(graph, elements_to_resolve, exclude_ids);
    }

    let mut result = ResolutionResult::new();
    let mut all_updates: Vec<ResolutionUpdate> = Vec::new();

    let mut raw_updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
    let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

    {
        let mut ctx = ResolutionContext::new(graph);

        for (element_id, kind) in &elements_to_resolve {
            let scope_id = graph
                .get_element(element_id)
                .and_then(|e| e.owner.clone())
                .unwrap_or_else(|| element_id.clone());

            let Some(element) = graph.get_element(element_id) else {
                continue;
            };

            // Resolve based on element kind (same logic as resolve_references)
            match kind {
                k if k == &ElementKind::Redefinition
                    || k.is_subtype_of(ElementKind::Redefinition) =>
                {
                    pass2::resolve_redefinition(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::ReferenceSubsetting
                    || k.is_subtype_of(ElementKind::ReferenceSubsetting) =>
                {
                    pass2::resolve_reference_subsetting(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Subsetting || k.is_subtype_of(ElementKind::Subsetting) => {
                    pass2::resolve_subsetting(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::FeatureTyping
                    || k.is_subtype_of(ElementKind::FeatureTyping)
                    || is_role_membership_typing(k) =>
                {
                    pass1::resolve_feature_typing(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Specialization
                    || k.is_subtype_of(ElementKind::Specialization) =>
                {
                    pass1::resolve_specialization(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Dependency || k.is_subtype_of(ElementKind::Dependency) => {
                    pass2::resolve_dependency(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Subclassification
                    || k.is_subtype_of(ElementKind::Subclassification) =>
                {
                    pass1::resolve_subclassification(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Conjugation
                    || k.is_subtype_of(ElementKind::Conjugation) =>
                {
                    pass2::resolve_conjugation(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::TypeFeaturing
                    || k.is_subtype_of(ElementKind::TypeFeaturing) =>
                {
                    pass2::resolve_type_featuring(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Disjoining || k.is_subtype_of(ElementKind::Disjoining) => {
                    pass2::resolve_disjoining(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Unioning || k.is_subtype_of(ElementKind::Unioning) => {
                    pass2::resolve_unioning(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Intersecting
                    || k.is_subtype_of(ElementKind::Intersecting) =>
                {
                    pass2::resolve_intersecting(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Differencing
                    || k.is_subtype_of(ElementKind::Differencing) =>
                {
                    pass2::resolve_differencing(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::FeatureInverting
                    || k.is_subtype_of(ElementKind::FeatureInverting) =>
                {
                    pass2::resolve_feature_inverting(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::FeatureChaining
                    || k.is_subtype_of(ElementKind::FeatureChaining) =>
                {
                    pass2::resolve_feature_chaining(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Annotation || k.is_subtype_of(ElementKind::Annotation) => {
                    pass2::resolve_annotation(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::Membership || k.is_subtype_of(ElementKind::Membership) => {
                    pass2::resolve_membership(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                k if k == &ElementKind::ConjugatedPortDefinition
                    || k.is_subtype_of(ElementKind::ConjugatedPortDefinition) =>
                {
                    pass2::resolve_conjugated_port_definition(
                        element,
                        &scope_id,
                        &mut ctx,
                        &mut raw_updates,
                        &mut unresolved,
                    );
                }
                _ => {}
            }
        }

        // Reference-site resolution (FRE), bare names only (B.1). Single-pass
        // path — bare names resolve in their local declaration namespace with no
        // Pass-1 dependency. Library (excluded) FREs are skipped. See pass_refs.rs.
        super::pass_refs::resolve_feature_references(
            graph,
            &mut ctx,
            Some(exclude_ids),
            &mut raw_updates,
        );

        result.diagnostics = ctx.take_diagnostics();
    }

    // Convert raw updates to ResolutionUpdate structs
    for (element_id, prop_name, resolved_id) in raw_updates {
        all_updates.push(ResolutionUpdate {
            element_id,
            property_name: prop_name,
            resolved_value: resolved_id,
        });
        result.resolved_count += 1;
    }

    // Record unresolved references
    for (element_id, prop_name, unresolved_name) in unresolved {
        let mut diag =
            build_unresolved_diagnostic(graph, &element_id, &prop_name, &unresolved_name);
        attach_im010_suggestion(&mut diag, &[graph], &unresolved_name);
        result.diagnostics.push(diag);
        result.unresolved_count += 1;
    }

    (all_updates, result)
}

/// Merge a library graph into `base` (by reference) and resolve cross-references
/// in the combined graph, skipping library element IDs.
///
/// `base` is mutated by the merge step. The resolve step is pure — returned
/// updates are NOT applied to `base`; the caller decides when to apply them
/// via [`apply_resolution_updates`].
///
/// This is the shared recipe behind:
/// - `sysml_parser_trait::ParseResult::into_resolved_with_library`
/// - `sysml_ide_db::resolution::cached_workspace_resolution_with_library`
///
/// # Arguments
///
/// * `base` - The graph to merge into and resolve (mutated by merge)
/// * `library` - The library graph (merged by reference)
/// * `library_ids` - Element IDs of `library`, skipped during resolution
/// * `register_library_roots` - If true, register library's root packages as
///   library packages (use `true` for one-shot parses where no other roots
///   exist; use `false` when `base` already has its own registered roots)
pub fn resolve_with_library_pure(
    base: &mut ModelGraph,
    library: &ModelGraph,
    library_ids: &FxHashSet<ElementId>,
    register_library_roots: bool,
) -> (Vec<ResolutionUpdate>, ResolutionResult) {
    base.merge_from_ref(library, register_library_roots);
    resolve_references_excluding_pure(base, library_ids)
}

/// Merge a library graph into `base` and resolve cross-references, applying
/// the resulting updates in place.
///
/// Convenience wrapper around [`resolve_with_library_pure`] that mirrors the
/// [`resolve_references`] / [`resolve_references_pure`] pairing.
pub fn resolve_with_library(
    base: &mut ModelGraph,
    library: &ModelGraph,
    library_ids: &FxHashSet<ElementId>,
    register_library_roots: bool,
) -> ResolutionResult {
    let (updates, result) =
        resolve_with_library_pure(base, library, library_ids, register_library_roots);
    apply_resolution_updates(base, &updates);
    result
}

/// Resolve references in a file graph using a separate library graph as fallback.
///
/// This avoids merging library elements into the file graph. Instead, the
/// `ResolutionContext` is configured with a fallback graph that provides
/// library types for name resolution. Only file-owned elements (those not
/// in `exclude_ids`) are resolved.
///
/// This is more efficient than `resolve_references_excluding_pure` when the
/// library graph is large, because it avoids O(L) element clones and index
/// rebuilds per file.
///
/// # Arguments
///
/// * `file_graph` - The file's parsed model graph (immutable)
/// * `library_graph` - The standard library graph (used for fallback lookups)
/// * `exclude_ids` - Set of element IDs to skip (typically library element IDs)
///
/// # Returns
///
/// A tuple of (`Vec<ResolutionUpdate>`, `ResolutionResult`).
/// ADR-016 P2/D3: the import-gate for the USER (fallback) resolution path is now
/// ON by default — bare unqualified cross-package/library *member* names resolve
/// only via explicit import (spec-correct), else a hard "missing import" diagnostic
/// (+ auto-import quick-fix). Library self-resolution (`resolve_references`/`_pure`,
/// no fallback) is never gated. The `SYSML_IMPORT_GATE` env var remains an override
/// for measurement / rollback: set it to `0`/`off`/`false`/`no` to disable the gate.
fn import_gate_enabled() -> bool {
    match std::env::var("SYSML_IMPORT_GATE") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no" | ""
        ),
        Err(_) => true,
    }
}

pub fn resolve_references_with_fallback_pure(
    file_graph: &ModelGraph,
    library_graph: &ModelGraph,
    exclude_ids: &FxHashSet<ElementId>,
) -> (Vec<ResolutionUpdate>, ResolutionResult) {
    let gate = import_gate_enabled();
    // Collect file elements that need resolution (excluding library elements)
    let elements_to_resolve: Vec<(ElementId, ElementKind)> = file_graph
        .elements
        .iter()
        .filter(|(id, e)| !exclude_ids.contains(*id) && has_unresolved_refs(e))
        .map(|(id, e)| (id.clone(), e.kind.clone()))
        .collect();

    let mut result = ResolutionResult::new();
    let mut all_updates: Vec<ResolutionUpdate> = Vec::new();

    let mut raw_updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
    let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

    {
        // Create context with library as fallback — no merge needed
        let mut ctx = ResolutionContext::new_with_fallback(file_graph, library_graph);
        ctx.set_bare_library_gate(gate);

        for (element_id, kind) in &elements_to_resolve {
            let scope_id = file_graph
                .get_element(element_id)
                .and_then(|e| e.owner.clone())
                .unwrap_or_else(|| element_id.clone());

            let Some(element) = file_graph.get_element(element_id) else {
                continue;
            };

            // Resolve based on element kind (same as _excluding_pure)
            dispatch_all_with_pass1(
                element,
                &scope_id,
                kind,
                &mut ctx,
                &mut raw_updates,
                &mut unresolved,
            );
        }

        // Reference-site resolution (FRE), bare names only (B.1). Uses the same
        // library-fallback context, so bare refs to library functions
        // (`interpolateSaturating`, `min`) resolve via the fallback graph.
        // Library (excluded) FREs skipped. See pass_refs.rs.
        super::pass_refs::resolve_feature_references(
            file_graph,
            &mut ctx,
            Some(exclude_ids),
            &mut raw_updates,
        );

        // ADR-016 D5: capture ambiguous-import resolutions before `take_diagnostics`
        // consumes `ctx`. This sink is populated only on the user (fallback) path.
        let ambiguities = ctx.take_ambiguities();
        result.diagnostics = ctx.take_diagnostics();

        for (namespace_id, name, candidates) in ambiguities {
            let diag = build_ambiguity_diagnostic(
                file_graph,
                library_graph,
                &namespace_id,
                &name,
                &candidates,
            );
            result.diagnostics.push(diag);
        }
    }

    // Convert raw updates to ResolutionUpdate structs
    for (element_id, prop_name, resolved_id) in raw_updates {
        all_updates.push(ResolutionUpdate {
            element_id,
            property_name: prop_name,
            resolved_value: resolved_id,
        });
        result.resolved_count += 1;
    }

    // Record unresolved references
    for (element_id, prop_name, unresolved_name) in unresolved {
        let mut diag =
            build_unresolved_diagnostic(file_graph, &element_id, &prop_name, &unresolved_name);
        attach_im010_suggestion(
            &mut diag,
            &[file_graph, library_graph],
            &unresolved_name,
        );
        result.diagnostics.push(diag);
        result.unresolved_count += 1;
    }

    (all_updates, result)
}

/// Dispatch all resolution (pass 1 + pass 2) for a single element.
///
/// Same as `dispatch_all` but also handles pass 1 Specialization subtypes
/// that are Subsetting (which `resolve_references_excluding_pure` dispatches
/// in its mixed pass).
fn dispatch_all_with_pass1(
    element: &crate::Element,
    scope_id: &ElementId,
    kind: &ElementKind,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    dispatch_all(element, scope_id, kind, ctx, updates, unresolved);
}

/// Dispatch pass 2 resolution for a single element based on its kind.
fn dispatch_pass2(
    element: &crate::Element,
    scope_id: &ElementId,
    kind: &ElementKind,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    match kind {
        // Most specific subtypes first
        k if k == &ElementKind::Redefinition || k.is_subtype_of(ElementKind::Redefinition) => {
            pass2::resolve_redefinition(element, scope_id, ctx, updates, unresolved);
        }
        k if k == &ElementKind::ReferenceSubsetting
            || k.is_subtype_of(ElementKind::ReferenceSubsetting) =>
        {
            pass2::resolve_reference_subsetting(element, scope_id, ctx, updates, unresolved);
        }
        k if k == &ElementKind::Subsetting || k.is_subtype_of(ElementKind::Subsetting) => {
            pass2::resolve_subsetting(element, scope_id, ctx, updates, unresolved);
        }
        // Dependency is a separate hierarchy
        k if k == &ElementKind::Dependency || k.is_subtype_of(ElementKind::Dependency) => {
            pass2::resolve_dependency(element, scope_id, ctx, updates, unresolved);
        }

        // === Additional cross-reference resolution ===

        // Conjugation (conjugatedType, originalType)
        k if k == &ElementKind::Conjugation || k.is_subtype_of(ElementKind::Conjugation) => {
            pass2::resolve_conjugation(element, scope_id, ctx, updates, unresolved);
        }

        // TypeFeaturing (featuringType)
        k if k == &ElementKind::TypeFeaturing || k.is_subtype_of(ElementKind::TypeFeaturing) => {
            pass2::resolve_type_featuring(element, scope_id, ctx, updates, unresolved);
        }

        // Disjoining (disjoiningType)
        k if k == &ElementKind::Disjoining || k.is_subtype_of(ElementKind::Disjoining) => {
            pass2::resolve_disjoining(element, scope_id, ctx, updates, unresolved);
        }

        // Unioning (unioningType)
        k if k == &ElementKind::Unioning || k.is_subtype_of(ElementKind::Unioning) => {
            pass2::resolve_unioning(element, scope_id, ctx, updates, unresolved);
        }

        // Intersecting (intersectingType)
        k if k == &ElementKind::Intersecting || k.is_subtype_of(ElementKind::Intersecting) => {
            pass2::resolve_intersecting(element, scope_id, ctx, updates, unresolved);
        }

        // Differencing (differencingType)
        k if k == &ElementKind::Differencing || k.is_subtype_of(ElementKind::Differencing) => {
            pass2::resolve_differencing(element, scope_id, ctx, updates, unresolved);
        }

        // FeatureInverting (invertingFeature)
        k if k == &ElementKind::FeatureInverting
            || k.is_subtype_of(ElementKind::FeatureInverting) =>
        {
            pass2::resolve_feature_inverting(element, scope_id, ctx, updates, unresolved);
        }

        // FeatureChaining (crossedFeature)
        k if k == &ElementKind::FeatureChaining
            || k.is_subtype_of(ElementKind::FeatureChaining) =>
        {
            pass2::resolve_feature_chaining(element, scope_id, ctx, updates, unresolved);
        }

        // Annotation (annotatedElement)
        k if k == &ElementKind::Annotation || k.is_subtype_of(ElementKind::Annotation) => {
            pass2::resolve_annotation(element, scope_id, ctx, updates, unresolved);
        }

        // Membership (memberElement) - only for elements that have unresolved memberElement
        k if (k == &ElementKind::Membership
            || k == &ElementKind::OwningMembership
            || k == &ElementKind::FeatureMembership
            || k.is_subtype_of(ElementKind::Membership))
            && element.props.contains_key(unresolved_props::MEMBER_ELEMENT) =>
        {
            pass2::resolve_membership(element, scope_id, ctx, updates, unresolved);
        }

        // ConjugatedPortDefinition (conjugatedPortDefinition)
        k if k == &ElementKind::ConjugatedPortDefinition
            || k.is_subtype_of(ElementKind::ConjugatedPortDefinition) =>
        {
            pass2::resolve_conjugated_port_definition(element, scope_id, ctx, updates, unresolved);
        }

        _ => {}
    }
}

// =========================================================================
// Parallel resolution
// =========================================================================

/// Dispatch all resolution (pass 1 + pass 2) for a single element.
///
/// Used by `resolve_references_excluding_parallel` which does a single combined pass.
fn dispatch_all(
    element: &crate::Element,
    scope_id: &ElementId,
    kind: &ElementKind,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    // Pass 1 kinds
    if is_pass1_element(kind) {
        match kind {
            k if k == &ElementKind::FeatureTyping
                || k.is_subtype_of(ElementKind::FeatureTyping)
                || is_role_membership_typing(k) =>
            {
                pass1::resolve_feature_typing(element, scope_id, ctx, updates, unresolved);
                return;
            }
            k if k == &ElementKind::Specialization
                || k.is_subtype_of(ElementKind::Specialization) =>
            {
                if !k.is_subtype_of(ElementKind::Subsetting) {
                    pass1::resolve_specialization(element, scope_id, ctx, updates, unresolved);
                    return;
                }
            }
            k if k == &ElementKind::Subclassification
                || k.is_subtype_of(ElementKind::Subclassification) =>
            {
                pass1::resolve_subclassification(element, scope_id, ctx, updates, unresolved);
                return;
            }
            _ => {}
        }
    }
    // Pass 2 kinds
    dispatch_pass2(element, scope_id, kind, ctx, updates, unresolved);
}

/// Parallel implementation of resolve_references_excluding (pure).
///
/// Uses a single combined pass (pass 1 + pass 2) with rayon par_chunks.
#[allow(clippy::needless_pass_by_value)] // ElementKind is not Copy due to #[non_exhaustive]
fn resolve_references_excluding_parallel_pure(
    graph: &ModelGraph,
    elements_to_resolve: Vec<(ElementId, ElementKind)>,
    exclude_ids: &FxHashSet<ElementId>,
) -> (Vec<ResolutionUpdate>, ResolutionResult) {
    let mut result = ResolutionResult::new();

    // Pre-build scope tables (single pass, so include inheritance index)
    let prebuilt_tables = std::sync::Arc::new(prebuild_scope_tables(graph, None));

    let (raw_updates, all_unresolved, all_diagnostics) = {
        #[cfg(feature = "parallel")]
        let chunk_results: Vec<_> = elements_to_resolve
            .par_chunks(64)
            .map(|chunk| {
                let mut ctx = ResolutionContext::new_with_prebuilt(
                    graph,
                    std::sync::Arc::clone(&prebuilt_tables),
                    None,
                );
                let mut updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
                let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

                for (element_id, kind) in chunk {
                    let scope_id = graph
                        .get_element(element_id)
                        .and_then(|e| e.owner.clone())
                        .unwrap_or_else(|| element_id.clone());

                    let Some(element) = graph.get_element(element_id) else {
                        continue;
                    };

                    dispatch_all(
                        element,
                        &scope_id,
                        kind,
                        &mut ctx,
                        &mut updates,
                        &mut unresolved,
                    );
                }

                let diagnostics = ctx.take_diagnostics();
                (updates, unresolved, diagnostics)
            })
            .collect();

        #[cfg(not(feature = "parallel"))]
        let chunk_results: Vec<_> = elements_to_resolve
            .chunks(64)
            .map(|chunk| {
                let mut ctx = ResolutionContext::new_with_prebuilt(
                    graph,
                    std::sync::Arc::clone(&prebuilt_tables),
                    None,
                );
                let mut updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
                let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

                for (element_id, kind) in chunk {
                    let scope_id = graph
                        .get_element(element_id)
                        .and_then(|e| e.owner.clone())
                        .unwrap_or_else(|| element_id.clone());

                    let Some(element) = graph.get_element(element_id) else {
                        continue;
                    };

                    dispatch_all(
                        element,
                        &scope_id,
                        kind,
                        &mut ctx,
                        &mut updates,
                        &mut unresolved,
                    );
                }

                let diagnostics = ctx.take_diagnostics();
                (updates, unresolved, diagnostics)
            })
            .collect();

        // Merge results from all chunks
        let mut updates = Vec::new();
        let mut unresolved = Vec::new();
        let mut diagnostics = Diagnostics::new();
        for (u, ur, d) in chunk_results {
            updates.extend(u);
            unresolved.extend(ur);
            for diag in d {
                diagnostics.push(diag);
            }
        }
        (updates, unresolved, diagnostics)
    };

    // Convert to ResolutionUpdate structs
    let mut all_updates: Vec<ResolutionUpdate> = Vec::with_capacity(raw_updates.len());
    for (element_id, prop_name, resolved_id) in raw_updates {
        all_updates.push(ResolutionUpdate {
            element_id,
            property_name: prop_name,
            resolved_value: resolved_id,
        });
        result.resolved_count += 1;
    }

    // Reference-site resolution (FRE), bare names only (B.1). Single-threaded over
    // the prebuilt scope tables; library (excluded) FREs skipped. See pass_refs.rs.
    {
        let mut ctx = ResolutionContext::new_with_prebuilt(
            graph,
            std::sync::Arc::clone(&prebuilt_tables),
            None,
        );
        let mut fre_updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
        super::pass_refs::resolve_feature_references(
            graph,
            &mut ctx,
            Some(exclude_ids),
            &mut fre_updates,
        );
        for (element_id, prop_name, resolved_id) in fre_updates {
            all_updates.push(ResolutionUpdate {
                element_id,
                property_name: prop_name,
                resolved_value: resolved_id,
            });
            result.resolved_count += 1;
        }
    }

    result.diagnostics = all_diagnostics;

    // Record unresolved references
    for (element_id, prop_name, unresolved_name) in all_unresolved {
        let mut diag =
            build_unresolved_diagnostic(graph, &element_id, &prop_name, &unresolved_name);
        attach_im010_suggestion(&mut diag, &[graph], &unresolved_name);
        result.diagnostics.push(diag);
        result.unresolved_count += 1;
    }

    (all_updates, result)
}

/// Check if an element kind belongs to pass 1 (type relationships).
fn is_pass1_element(kind: &ElementKind) -> bool {
    if kind == &ElementKind::FeatureTyping || kind.is_subtype_of(ElementKind::FeatureTyping) {
        return true;
    }
    // Membership-wrapped role usages resolve their typing clause via the same
    // pass-1 `resolve_feature_typing` path (see `is_role_membership_typing`).
    if is_role_membership_typing(kind) {
        return true;
    }
    if kind == &ElementKind::Subclassification || kind.is_subtype_of(ElementKind::Subclassification)
    {
        return true;
    }
    // Specialization but NOT Subsetting (which is a subtype of Specialization handled in pass 2)
    if (kind == &ElementKind::Specialization || kind.is_subtype_of(ElementKind::Specialization))
        && !kind.is_subtype_of(ElementKind::Subsetting)
    {
        return true;
    }
    false
}

/// Pre-build all scope tables for namespaces in the graph.
///
/// Collects all element IDs that own at least one other element (plus roots and
/// library packages), then forces their full scope tables to be built. The resulting
/// map is extracted and can be cloned into per-thread resolution contexts.
fn prebuild_scope_tables(
    graph: &ModelGraph,
    inheritance_index: Option<InheritanceIndex>,
) -> rustc_hash::FxHashMap<ElementId, ScopeTable> {
    let mut ctx = match inheritance_index {
        Some(idx) => ResolutionContext::new_with_index(graph, idx),
        None => ResolutionContext::new(graph),
    };

    // Collect all namespace IDs: elements that own at least one other element
    let mut namespace_ids: FxHashSet<ElementId> = FxHashSet::default();
    for elem in graph.elements.values() {
        if let Some(owner_id) = &elem.owner {
            namespace_ids.insert(owner_id.clone());
        }
    }
    // Include root elements and library packages
    for root in graph.roots() {
        namespace_ids.insert(root.id.clone());
    }
    for lib_id in graph.library_packages() {
        namespace_ids.insert(lib_id.clone());
    }

    // Force-build full scope tables for all collected namespaces
    for ns_id in &namespace_ids {
        ctx.get_full_scope_table(ns_id);
    }

    ctx.take_scope_tables()
}

/// Parallel implementation of resolve_references (pure).
///
/// Uses rayon `par_chunks` to dispatch resolution across multiple threads.
/// Each thread gets its own `ResolutionContext` with cloned pre-built scope tables.
///
/// For the two-pass approach, pass 1 updates must be applied to the graph before
/// pass 2 can see inherited members. Since we cannot mutate the graph, we apply
/// pass 1 updates to a temporary clone for building pass 2 scope tables.
#[allow(clippy::needless_pass_by_value)] // ElementKind is not Copy due to #[non_exhaustive]
fn resolve_references_parallel_pure(
    graph: &ModelGraph,
    elements_to_resolve: Vec<(ElementId, ElementKind)>,
) -> (Vec<ResolutionUpdate>, ResolutionResult) {
    let mut result = ResolutionResult::new();
    let mut all_updates: Vec<ResolutionUpdate> = Vec::new();

    // =========================================================================
    // PASS 1: Resolve type relationships in parallel
    // =========================================================================
    let pass1_tables = std::sync::Arc::new(prebuild_scope_tables(graph, None));

    let (pass1_raw_updates, pass1_unresolved) = {
        #[cfg(feature = "parallel")]
        let chunk_results: Vec<_> = elements_to_resolve
            .par_chunks(64)
            .map(|chunk| {
                let mut ctx = ResolutionContext::new_with_prebuilt(
                    graph,
                    std::sync::Arc::clone(&pass1_tables),
                    None,
                );
                let mut updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
                let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

                for (element_id, kind) in chunk {
                    if !is_pass1_element(kind) {
                        continue;
                    }

                    let scope_id = graph
                        .get_element(element_id)
                        .and_then(|e| e.owner.clone())
                        .unwrap_or_else(|| element_id.clone());

                    let Some(element) = graph.get_element(element_id) else {
                        continue;
                    };

                    match kind {
                        k if k == &ElementKind::FeatureTyping
                            || k.is_subtype_of(ElementKind::FeatureTyping)
                            || is_role_membership_typing(k) =>
                        {
                            pass1::resolve_feature_typing(
                                element,
                                &scope_id,
                                &mut ctx,
                                &mut updates,
                                &mut unresolved,
                            );
                        }
                        // Subclassification must be checked BEFORE the generic
                        // Specialization arm: Subclassification is a subtype of
                        // Specialization but stores its supertype under
                        // `unresolved_superclassifier`, not `unresolved_general`.
                        // If the generic arm caught it first it would call
                        // `resolve_specialization` (reads GENERAL) and silently
                        // resolve nothing, leaving the supertype edge missing from
                        // the inheritance index.
                        k if k == &ElementKind::Subclassification
                            || k.is_subtype_of(ElementKind::Subclassification) =>
                        {
                            pass1::resolve_subclassification(
                                element,
                                &scope_id,
                                &mut ctx,
                                &mut updates,
                                &mut unresolved,
                            );
                        }
                        k if k == &ElementKind::Specialization
                            || k.is_subtype_of(ElementKind::Specialization) =>
                        {
                            if !k.is_subtype_of(ElementKind::Subsetting) {
                                pass1::resolve_specialization(
                                    element,
                                    &scope_id,
                                    &mut ctx,
                                    &mut updates,
                                    &mut unresolved,
                                );
                            }
                        }
                        _ => {}
                    }
                }

                (updates, unresolved)
            })
            .collect();

        #[cfg(not(feature = "parallel"))]
        let chunk_results: Vec<_> = elements_to_resolve
            .chunks(64)
            .map(|chunk| {
                let mut ctx = ResolutionContext::new_with_prebuilt(
                    graph,
                    std::sync::Arc::clone(&pass1_tables),
                    None,
                );
                let mut updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
                let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

                for (element_id, kind) in chunk {
                    if !is_pass1_element(kind) {
                        continue;
                    }

                    let scope_id = graph
                        .get_element(element_id)
                        .and_then(|e| e.owner.clone())
                        .unwrap_or_else(|| element_id.clone());

                    let Some(element) = graph.get_element(element_id) else {
                        continue;
                    };

                    match kind {
                        k if k == &ElementKind::FeatureTyping
                            || k.is_subtype_of(ElementKind::FeatureTyping)
                            || is_role_membership_typing(k) =>
                        {
                            pass1::resolve_feature_typing(
                                element,
                                &scope_id,
                                &mut ctx,
                                &mut updates,
                                &mut unresolved,
                            );
                        }
                        // Subclassification must be checked BEFORE the generic
                        // Specialization arm (see the parallel branch above for
                        // the rationale): it stores its supertype under
                        // `unresolved_superclassifier`, not `unresolved_general`.
                        k if k == &ElementKind::Subclassification
                            || k.is_subtype_of(ElementKind::Subclassification) =>
                        {
                            pass1::resolve_subclassification(
                                element,
                                &scope_id,
                                &mut ctx,
                                &mut updates,
                                &mut unresolved,
                            );
                        }
                        k if k == &ElementKind::Specialization
                            || k.is_subtype_of(ElementKind::Specialization) =>
                        {
                            if !k.is_subtype_of(ElementKind::Subsetting) {
                                pass1::resolve_specialization(
                                    element,
                                    &scope_id,
                                    &mut ctx,
                                    &mut updates,
                                    &mut unresolved,
                                );
                            }
                        }
                        _ => {}
                    }
                }

                (updates, unresolved)
            })
            .collect();

        let mut all_updates = Vec::new();
        let mut all_unresolved = Vec::new();
        for (u, ur) in chunk_results {
            all_updates.extend(u);
            all_unresolved.extend(ur);
        }
        (all_updates, all_unresolved)
    };

    // Convert pass 1 raw updates and apply to a temporary graph for pass 2
    let pass1_resolution_updates: Vec<ResolutionUpdate> = pass1_raw_updates
        .into_iter()
        .map(|(element_id, prop_name, resolved_id)| ResolutionUpdate {
            element_id,
            property_name: prop_name,
            resolved_value: resolved_id,
        })
        .collect();
    result.resolved_count += pass1_resolution_updates.len();
    all_updates.extend(pass1_resolution_updates.iter().cloned());

    // Apply pass 1 to a temporary clone so pass 2 can see inherited members
    let mut temp_graph = graph.clone();
    apply_resolution_updates(&mut temp_graph, &pass1_resolution_updates);

    // =========================================================================
    // PASS 2: Resolve feature relationships in parallel
    // =========================================================================
    let inheritance_index = InheritanceIndex::build(&temp_graph);
    let pass2_tables = std::sync::Arc::new(prebuild_scope_tables(
        &temp_graph,
        Some(inheritance_index.clone()),
    ));

    let (pass2_raw_updates, pass2_unresolved, pass2_diagnostics) = {
        #[cfg(feature = "parallel")]
        let chunk_results: Vec<_> = elements_to_resolve
            .par_chunks(64)
            .map(|chunk| {
                let mut ctx = ResolutionContext::new_with_prebuilt(
                    &temp_graph,
                    std::sync::Arc::clone(&pass2_tables),
                    Some(inheritance_index.clone()),
                );
                let mut updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
                let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

                for (element_id, kind) in chunk {
                    let scope_id = temp_graph
                        .get_element(element_id)
                        .and_then(|e| e.owner.clone())
                        .unwrap_or_else(|| element_id.clone());

                    let Some(element) = temp_graph.get_element(element_id) else {
                        continue;
                    };

                    dispatch_pass2(
                        element,
                        &scope_id,
                        kind,
                        &mut ctx,
                        &mut updates,
                        &mut unresolved,
                    );
                }

                let diagnostics = ctx.take_diagnostics();
                (updates, unresolved, diagnostics)
            })
            .collect();

        #[cfg(not(feature = "parallel"))]
        let chunk_results: Vec<_> = elements_to_resolve
            .chunks(64)
            .map(|chunk| {
                let mut ctx = ResolutionContext::new_with_prebuilt(
                    &temp_graph,
                    std::sync::Arc::clone(&pass2_tables),
                    Some(inheritance_index.clone()),
                );
                let mut updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
                let mut unresolved: Vec<(ElementId, String, String)> = Vec::new();

                for (element_id, kind) in chunk {
                    let scope_id = temp_graph
                        .get_element(element_id)
                        .and_then(|e| e.owner.clone())
                        .unwrap_or_else(|| element_id.clone());

                    let Some(element) = temp_graph.get_element(element_id) else {
                        continue;
                    };

                    dispatch_pass2(
                        element,
                        &scope_id,
                        kind,
                        &mut ctx,
                        &mut updates,
                        &mut unresolved,
                    );
                }

                let diagnostics = ctx.take_diagnostics();
                (updates, unresolved, diagnostics)
            })
            .collect();

        let mut all_updates = Vec::new();
        let mut all_unresolved = Vec::new();
        let mut all_diagnostics = Diagnostics::new();
        for (u, ur, d) in chunk_results {
            all_updates.extend(u);
            all_unresolved.extend(ur);
            for diag in d {
                all_diagnostics.push(diag);
            }
        }
        (all_updates, all_unresolved, all_diagnostics)
    };

    // Convert pass 2 raw updates
    for (element_id, prop_name, resolved_id) in pass2_raw_updates {
        all_updates.push(ResolutionUpdate {
            element_id,
            property_name: prop_name,
            resolved_value: resolved_id,
        });
        result.resolved_count += 1;
    }

    // Reference-site resolution (FRE): after Pass 1, over the pass-1-applied
    // temp_graph, reusing the pass-2 inheritance index + prebuilt scope tables.
    // Bare names only (B.1); additive `resolved_props::FEATURE_REFERENCE`. Run
    // single-threaded here — FRE counts are modest; parallelise in a later phase
    // if it profiles hot.
    {
        let mut ctx = ResolutionContext::new_with_prebuilt(
            &temp_graph,
            std::sync::Arc::clone(&pass2_tables),
            Some(inheritance_index.clone()),
        );
        let mut fre_updates: Vec<(ElementId, Cow<'static, str>, ElementId)> = Vec::new();
        super::pass_refs::resolve_feature_references(
            &temp_graph,
            &mut ctx,
            None,
            &mut fre_updates,
        );
        for (element_id, prop_name, resolved_id) in fre_updates {
            all_updates.push(ResolutionUpdate {
                element_id,
                property_name: prop_name,
                resolved_value: resolved_id,
            });
            result.resolved_count += 1;
        }
    }

    result.diagnostics = pass2_diagnostics;

    // Record all unresolved references as diagnostics
    for (element_id, prop_name, unresolved_name) in
        pass1_unresolved.into_iter().chain(pass2_unresolved)
    {
        let mut diag =
            build_unresolved_diagnostic(&temp_graph, &element_id, &prop_name, &unresolved_name);
        attach_im010_suggestion(&mut diag, &[&temp_graph, graph], &unresolved_name);
        result.diagnostics.push(diag);
        result.unresolved_count += 1;
    }

    (all_updates, result)
}

/// Check if an element has any unresolved references.
/// Fast pre-filter: kinds that never carry unresolved cross-references.
/// These make up the bulk of elements in large graphs (literals, docs,
/// memberships) and can be skipped entirely before the 21-key property check.
/// A membership-wrapped role usage that stamps its typing clause's target as an
/// `unresolved_type` prop directly on the membership (rather than on a lowered
/// intermediate usage). These must be routed through `resolve_feature_typing`
/// so the type reference resolves and a missing target counts as unresolved.
/// Kept in sync with the ast_builder sites that stamp `unresolved_type` on a
/// membership: `process_subject_requirement` / `process_objective_requirement`.
fn is_role_membership_typing(kind: &crate::ElementKind) -> bool {
    matches!(
        kind,
        crate::ElementKind::SubjectMembership | crate::ElementKind::ObjectiveMembership
    )
}

fn is_never_resolvable(kind: &crate::ElementKind) -> bool {
    use crate::ElementKind::*;
    matches!(
        kind,
        LiteralInteger
            | LiteralRational
            | LiteralBoolean
            | LiteralString
            | LiteralExpression
            | Documentation
            | Comment
            | OwningMembership
            | Membership
    )
}

fn has_unresolved_refs(element: &crate::Element) -> bool {
    element.props.contains_key(unresolved_props::GENERAL)
        || element.props.contains_key(unresolved_props::TYPE)
        || element.props.contains_key(unresolved_props::SUBSETTED_FEATURE)
        || element.props.contains_key(unresolved_props::REDEFINED_FEATURE)
        || element.props.contains_key(unresolved_props::REFERENCED_FEATURE)
        || element.props.contains_key(unresolved_props::SOURCES)
        || element.props.contains_key(unresolved_props::TARGETS)
        // Phase B: Additional cross-references
        || element.props.contains_key(unresolved_props::SUPERCLASSIFIER)
        || element.props.contains_key(unresolved_props::CONJUGATED_TYPE)
        || element.props.contains_key(unresolved_props::ORIGINAL_TYPE)
        || element.props.contains_key(unresolved_props::FEATURING_TYPE)
        || element.props.contains_key(unresolved_props::DISJOINING_TYPE)
        || element.props.contains_key(unresolved_props::UNIONING_TYPE)
        || element.props.contains_key(unresolved_props::INTERSECTING_TYPE)
        || element.props.contains_key(unresolved_props::DIFFERENCING_TYPE)
        || element.props.contains_key(unresolved_props::INVERTING_FEATURE)
        || element.props.contains_key(unresolved_props::CROSSED_FEATURE)
        || element.props.contains_key(unresolved_props::ANNOTATED_ELEMENT)
        || element.props.contains_key(unresolved_props::MEMBER_ELEMENT)
        || element.props.contains_key(unresolved_props::CLIENT)
        || element.props.contains_key(unresolved_props::SUPPLIER)
        || element.props.contains_key(unresolved_props::CONJUGATED_PORT_DEFINITION)
}

fn build_unresolved_diagnostic(
    graph: &ModelGraph,
    element_id: &ElementId,
    prop_name: &str,
    unresolved_name: &str,
) -> Diagnostic {
    // Build a contextual primary message
    let primary_message = if let Some(element) = graph.get_element(element_id) {
        let element_desc = match &element.name {
            Some(name) => format!("{} '{}'", element.kind.display_name(), name),
            None => element.kind.display_name().to_owned(),
        };
        format!(
            "no definition '{}' found in scope of {}",
            unresolved_name, element_desc
        )
    } else {
        format!(
            "unresolved reference '{}' for property '{}'",
            unresolved_name, prop_name
        )
    };

    // P-RA2 Slice 4: E200 is conservatively tagged NameResWorkspace.
    //
    // Same-file unresolved references would technically be NameResLocal, but
    // splitting that out requires a `is_cross_file` predicate at the unresolved-
    // prop creation site that we don't yet thread through here. The conservative
    // gate means single-file refs also wait for PFS indexing before publishing —
    // acceptable precision loss that we can refine in a follow-up phase if it
    // becomes a UX problem.
    let mut diagnostic = Diagnostic::error(primary_message)
        .with_code("E200")
        .with_tier(sysml_span::DiagnosticTier::NameResWorkspace);

    if let Some(element) = graph.get_element(element_id) {
        if let Some(span) = element.spans.first() {
            diagnostic = diagnostic.with_span(span.clone());
        }

        if let Some(qname) = graph.build_qualified_name(element_id) {
            diagnostic = diagnostic.with_note(format!("qualified name: {}", qname));
        }

        if let Some(owner_id) = &element.owner {
            if let Some(owner) = graph.get_element(owner_id) {
                if let Some(owner_span) = owner.spans.first() {
                    let owner_label = match &owner.name {
                        Some(name) => format!("{} '{}'", owner.kind.display_name(), name),
                        None => owner.kind.display_name().to_owned(),
                    };
                    diagnostic = diagnostic
                        .with_related(owner_span.clone(), format!("owner: {}", owner_label));
                }
            }
        }
    }

    if graph.library_packages().is_empty() && looks_like_stdlib_type(unresolved_name) {
        diagnostic = diagnostic.with_note(
            "standard library not loaded \u{2014} this may resolve when the library is available",
        );
    }

    diagnostic = diagnostic.with_note("ensure the name is defined or imported in scope");
    diagnostic
}

/// Find an unresolved name across the given graphs.
///
/// Looks for the name first as a fully-qualified path
/// ([`ModelGraph::resolve_qname`]), then as a bare-name match against any
/// definition element. Returns the qualified path of the first match (in
/// graph order) so the caller can build a "did you mean to import X?"
/// suggestion.
fn find_unresolved_qualified(
    graphs: &[&ModelGraph],
    unresolved_name: &str,
) -> Option<QualifiedName> {
    for graph in graphs {
        if let Some(elem) = graph.resolve_qname(unresolved_name) {
            if let Some(qn) = graph.build_qualified_name(&elem.id) {
                return Some(qn);
            }
        }
        for (id, element) in graph.elements.iter() {
            if element.name.as_deref() == Some(unresolved_name) && element.kind.is_definition() {
                if let Some(qn) = graph.build_qualified_name(id) {
                    return Some(qn);
                }
            }
        }
    }
    None
}

/// Attach IM010 suggestion data when an unresolved reference's name DOES
/// exist somewhere in the loaded graphs but isn't reachable from the current
/// scope. Replaces the legacy "lenient-fallback" silent downgrade to Info
/// (file-loading model §5.4): the diagnostic stays an Error, but its code
/// upgrades from E200 to IM010 and notes are added pointing at the qualified
/// path plus a recommended `import X::*;`.
fn attach_im010_suggestion(
    diag: &mut Diagnostic,
    graphs: &[&ModelGraph],
    unresolved_name: &str,
) {
    let Some(qname) = find_unresolved_qualified(graphs, unresolved_name) else {
        return;
    };
    diag.code = Some("IM010".to_string());
    // P-RA2 Slice 4: code upgraded E200 -> IM010, tier follows from
    // NameResWorkspace -> ImportHealth.
    diag.tier = sysml_span::DiagnosticTier::ImportHealth;
    if let Some(parent) = qname.parent() {
        diag.notes.push(format!(
            "'{}' is defined as `{}`",
            unresolved_name, qname
        ));
        diag.notes.push(format!(
            "help: add `import {}::*;` at the top of this file, or use the qualified name `{}`",
            parent, qname
        ));
    } else {
        diag.notes.push(format!(
            "'{}' is defined as `{}` at the top level",
            unresolved_name, qname
        ));
        diag.notes.push(format!(
            "help: use the qualified name `{}` if it is in a sibling root namespace",
            qname
        ));
    }
}

/// Build the ambiguous-import diagnostic (ADR-016 D5).
///
/// Emitted only for user-authored namespaces (the user/fallback resolution
/// path); the candidate ids are the sorted, distinct elements that two+ imports
/// brought in under the same name. The span is anchored to the importing
/// namespace element.
fn build_ambiguity_diagnostic(
    file_graph: &ModelGraph,
    library_graph: &ModelGraph,
    namespace_id: &ElementId,
    name: &str,
    candidates: &[ElementId],
) -> Diagnostic {
    // Best-effort qualified name (falling back to simple name, then the raw id)
    // for each candidate, drawn from whichever graph owns it.
    let qnames: Vec<String> = candidates
        .iter()
        .map(|id| {
            file_graph
                .build_qualified_name(id)
                .or_else(|| library_graph.build_qualified_name(id))
                .map(|qn| qn.to_string())
                .or_else(|| {
                    file_graph
                        .get_element(id)
                        .or_else(|| library_graph.get_element(id))
                        .and_then(|e| e.name.clone())
                })
                .unwrap_or_else(|| id.to_string())
        })
        .collect();

    let msg = format!(
        "ambiguous reference '{}': imported from multiple sources ({}); qualify the name to disambiguate",
        name,
        qnames.join(", ")
    );
    let mut diagnostic = Diagnostic::error(msg).with_code("E201");

    // Anchor the span to the importing namespace element.
    if let Some(element) = file_graph.get_element(namespace_id) {
        if let Some(span) = element.spans.first() {
            diagnostic = diagnostic.with_span(span.clone());
        }
    }

    diagnostic
}

fn looks_like_stdlib_type(name: &str) -> bool {
    if primitive_type_alias(name).is_some() {
        return true;
    }

    matches!(
        name,
        "Anything"
            | "DataValue"
            | "Boolean"
            | "Integer"
            | "Real"
            | "String"
            | "Complex"
            | "Rational"
            | "Natural"
            | "ScalarValues"
    )
}

#[cfg(test)]
mod tier_tests {
    use super::*;
    use crate::Element;
    use sysml_id::ElementId;

    /// P-RA2 Slice 4: bare E200 diagnostics from the resolver must be tagged
    /// `NameResWorkspace` so the readiness filter can withhold them until the
    /// project file set is indexed. (Conservative: same-file unresolved refs
    /// are tagged the same — see the comment in `build_unresolved_diagnostic`.)
    #[test]
    fn e200_diagnostic_tagged_name_res_workspace() {
        let mut graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::PartUsage).with_name("p");
        let id = graph.add_element(elem);
        let diag = build_unresolved_diagnostic(&graph, &id, "type", "DanglingName");
        assert_eq!(diag.code.as_deref(), Some("E200"));
        assert_eq!(diag.tier, sysml_span::DiagnosticTier::NameResWorkspace);
    }

    /// When the resolver finds a qualified candidate, the diagnostic is
    /// upgraded to IM010 with the ImportHealth tier.
    #[test]
    fn im010_upgrade_switches_tier_to_import_health() {
        // Hand-build a graph that has a `Foo::Bar` qualified name to find.
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Foo");
        let pkg_id = graph.add_element(pkg);
        let bar = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Bar")
            .with_owner(pkg_id.clone());
        graph.add_element(bar);

        let usage_id: ElementId = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage).with_name("p"),
        );
        let mut diag = build_unresolved_diagnostic(&graph, &usage_id, "type", "Bar");
        assert_eq!(diag.tier, sysml_span::DiagnosticTier::NameResWorkspace);
        attach_im010_suggestion(&mut diag, &[&graph], "Bar");
        assert_eq!(diag.code.as_deref(), Some("IM010"));
        assert_eq!(diag.tier, sysml_span::DiagnosticTier::ImportHealth);
    }
}
