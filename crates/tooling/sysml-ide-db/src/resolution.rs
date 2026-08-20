//! Resolution layer: tracked queries for name resolution.
//!
//! This is Layer 3 of the salsa query hierarchy. Resolution depends on
//! the parse layer (Layer 1) and produces fully-resolved model graphs.
//!
//! ## Design
//!
//! Resolution uses a "pure function + apply" approach:
//! 1. Clone the parsed ModelGraph from `parse_file`
//! 2. Run `resolve_references_pure` on the clone (returns `Vec<ResolutionUpdate>`)
//! 3. Apply updates to the clone via `apply_resolution_updates`
//! 4. Return the resolved clone wrapped in Arc
//!
//! The pure-function split enables finer-grained salsa incrementality:
//! resolution outputs can be inspected or transformed before application.
//!
//! ## Cross-file resolution
//!
//! For single-file resolution (no library, no imports), we resolve against
//! the file's own graph. For multi-file (workspace) resolution, we build a
//! cached merged graph (`workspace_merged_graph`) from all project files,
//! optionally merge the standard library, and resolve once. Per-file
//! functions then filter the cached results by element ID and span. The
//! library graph is a salsa input that's set once at startup.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rustc_hash::FxHashSet;
use sysml_core::resolution;
use sysml_core::resolution::InheritanceIndex;
use sysml_core::ModelGraph;
use sysml_id::ElementId;
use sysml_span::Diagnostic;

use crate::parse;
use crate::source::SourceFile;
use crate::Db;

// ---------------------------------------------------------------------------
// Result wrapper types
// ---------------------------------------------------------------------------

/// Resolved model: a ModelGraph with all references resolved + resolution diagnostics.
#[derive(Clone, Debug)]
pub struct ResolvedModel(Arc<ResolvedModelData>);

#[derive(Debug)]
struct ResolvedModelData {
    /// The resolved model graph (clone of parse graph with references resolved).
    graph: ModelGraph,
    /// Resolution diagnostics (unresolved references, ambiguities, etc.).
    diagnostics: Vec<Diagnostic>,
    /// Content fingerprint for equality comparison.
    fingerprint: u64,
}

impl ResolvedModel {
    fn new(graph: ModelGraph, diagnostics: Vec<Diagnostic>) -> Self {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            graph.fingerprint().hash(&mut h);
            diagnostics.len().hash(&mut h);
            for d in &diagnostics {
                crate::analysis::hash_diagnostic(d, &mut h);
            }
            h.finish()
        };
        Self(Arc::new(ResolvedModelData {
            graph,
            diagnostics,
            fingerprint,
        }))
    }

    /// The resolved model graph.
    pub fn graph(&self) -> &ModelGraph {
        &self.0.graph
    }

    /// Resolution diagnostics.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0.diagnostics
    }
}

salsa_arc_wrapper!(fingerprint, ResolvedModel, ResolvedModelData);

/// Library graph wrapper (for salsa input compatibility).
#[derive(Clone, Debug)]
pub struct LibraryData(Arc<LibraryDataInner>);

#[derive(Debug)]
struct LibraryDataInner {
    graph: ModelGraph,
    element_ids: FxHashSet<ElementId>,
    inheritance_index: Arc<InheritanceIndex>,
}

impl LibraryData {
    /// Create a new library data wrapper.
    pub fn new(mut graph: ModelGraph) -> Self {
        let element_ids: FxHashSet<ElementId> = graph.elements.keys().cloned().collect();
        // Pre-build the library name index so resolve_in_library() O(1) lookups
        // work immediately. Without this, every name resolution falls back to
        // O(k*d*m) recursive scanning of all 52K+ library elements.
        graph.build_library_index();
        // Pre-build the library inheritance index alongside the name index.
        // The May 29 perf baseline shows InheritanceIndex::collect_specializations
        // taking 39.4 % exclusive on workspace elaborate runs because
        // ResolutionContext::new(lib) rebuilds this O(|library|) closure on
        // every IG-1 candidate, per user file. Building it here once at library
        // load — same lifetime as the name index — lets resolution contexts
        // reuse it via new_with_lib_inheritance_index /
        // new_with_fallback_and_lib_inheritance_index instead of rescanning.
        //
        // Wrapped in `Arc` so the library-only resolution path can clone-share
        // (refcount bump) instead of deep-copying the ~hundreds-of-thousands of
        // map entries on every candidate context.
        let inheritance_index = Arc::new(InheritanceIndex::build(&graph));
        Self(Arc::new(LibraryDataInner {
            graph,
            element_ids,
            inheritance_index,
        }))
    }

    /// The library model graph.
    pub fn graph(&self) -> &ModelGraph {
        &self.0.graph
    }

    /// The set of element IDs in the library.
    pub fn element_ids(&self) -> &FxHashSet<ElementId> {
        &self.0.element_ids
    }

    /// The pre-built inheritance index for the library graph.
    ///
    /// Use this when constructing a [`sysml_core::resolution::ResolutionContext`]
    /// that resolves against the library, to avoid the O(|library|) rebuild on
    /// every context creation. Returned by `Arc` so the library-only context
    /// path can refcount-bump instead of cloning the map.
    pub fn inheritance_index(&self) -> &Arc<InheritanceIndex> {
        &self.0.inheritance_index
    }
}

salsa_arc_wrapper!(identity, LibraryData, LibraryDataInner);

/// Salsa input for the standard library graph.
///
/// Set once at startup (or when the library path changes). Resolution
/// queries depend on this to resolve references to library types.
#[salsa::input(singleton)]
pub struct LibraryGraph {
    #[returns(ref)]
    pub data: LibraryData,
}

// ---------------------------------------------------------------------------
// Cached workspace merged graph
// ---------------------------------------------------------------------------

/// Workspace merged graph: all files in a ProjectFileSet merged into one ModelGraph.
///
/// Wrapped in Arc for cheap cloning. PartialEq/Hash use content fingerprinting
/// so salsa can detect when file changes produce a structurally-identical merged graph.
#[derive(Clone, Debug)]
pub struct MergedGraph(Arc<MergedGraphData>);

#[derive(Debug)]
struct MergedGraphData {
    graph: ModelGraph,
    fingerprint: u64,
}

impl MergedGraph {
    fn new(graph: ModelGraph) -> Self {
        let fingerprint = graph.fingerprint();
        Self(Arc::new(MergedGraphData { graph, fingerprint }))
    }

    /// The merged workspace model graph.
    pub fn graph(&self) -> &ModelGraph {
        &self.0.graph
    }
}

salsa_arc_wrapper!(fingerprint, MergedGraph, MergedGraphData);

/// Compute the merged graph for all files in a project.
///
/// Merges every file's parsed ModelGraph into a single combined graph using
/// `merge_from_ref`, which copies elements, relationships, and all index
/// structures needed for cross-file name resolution and import health checks.
///
/// This is cached by salsa: when any file changes, salsa invalidates the
/// affected `parse_file` result, which invalidates this function's memoized
/// result, causing one O(N) rebuild. Without caching, each per-file
/// resolution/validation call would rebuild the merged graph independently,
/// leading to O(N²) merge operations.
///
/// Depends on: `parse_file()` (Layer 1) for each file in the project
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_merged_graph(
    db: &dyn Db,
    project_files: crate::project_inputs::ProjectFileSet,
) -> MergedGraph {
    let all_files = project_files.files(db);
    tracing::debug!(file_count = all_files.len(), "building cached workspace merged graph");
    let mut merged = ModelGraph::new();
    for file in all_files.iter() {
        let parsed = crate::parse::parse_file(db, *file);
        merged.merge_from_ref(parsed.graph(), false);
    }
    MergedGraph::new(merged)
}

/// Compute the standard library [`LibraryData`] from a stdlib [`ProjectFileSet`].
///
/// Iterates the stdlib files, parses each via the salsa-tracked `parse_file`
/// query (so spans use file:// URIs and goto-definition into stdlib works),
/// merges them into one `ModelGraph`, registers root packages as library
/// packages, and resolves internal cross-references.
///
/// Memoized by salsa: changes to any stdlib file invalidate only this query
/// and downstream resolution. Stdlib files don't change at runtime in
/// practice, so this is computed once per session.
///
/// Depends on: `parse_file()` (Layer 1) for each stdlib file in the pfs.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn compute_stdlib_library_data(
    db: &dyn Db,
    stdlib_files: crate::project_inputs::ProjectFileSet,
) -> LibraryData {
    let all_files = stdlib_files.files(db);
    tracing::info!(
        file_count = all_files.len(),
        "computing stdlib library data via salsa-tracked query"
    );
    let mut combined = ModelGraph::new();
    for file in all_files.iter() {
        let parsed = crate::parse::parse_file(db, *file);
        combined.merge_from_ref(parsed.graph(), false);
    }
    combined.rebuild_indexes();

    // Register all root packages as library packages so resolve_in_library()
    // can find them. Mirrors `register_library_packages` in
    // sysml-parser-trait::library.
    let root_package_ids: Vec<ElementId> = combined
        .elements
        .values()
        .filter(|e| {
            e.owner.is_none()
                && (e.kind == sysml_core::ElementKind::Package
                    || e.kind == sysml_core::ElementKind::LibraryPackage
                    || e.kind.is_subtype_of(sysml_core::ElementKind::Package))
        })
        .map(|e| e.id.clone())
        .collect();
    for id in root_package_ids {
        combined.register_library_package(id);
    }

    // Resolve internal library cross-references (library files import each
    // other; e.g., ISQThermodynamics imports MeasurementReferences::*).
    let _ = sysml_core::resolution::resolve_references(&mut combined);

    tracing::info!(
        elements = combined.element_count(),
        relationships = combined.relationship_count(),
        packages = combined.library_packages().len(),
        "stdlib library data ready"
    );
    LibraryData::new(combined)
}

// ---------------------------------------------------------------------------
// Cached workspace resolution
// ---------------------------------------------------------------------------

/// Cached resolution result: updates + diagnostics from resolving the full workspace graph.
///
/// This is computed once for the entire workspace and shared by all per-file
/// resolution functions, avoiding the O(N) repeated resolution where N is the
/// number of files (each taking ~2 minutes on large workspaces).
#[derive(Clone, Debug)]
pub struct CachedResolution(Arc<CachedResolutionData>);

#[derive(Debug)]
struct CachedResolutionData {
    /// All resolution updates for the entire workspace.
    updates: Vec<resolution::ResolutionUpdate>,
    /// All resolution diagnostics for the entire workspace.
    diagnostics: Vec<Diagnostic>,
    /// Content fingerprint for salsa equality comparison.
    fingerprint: u64,
}

impl CachedResolution {
    fn new(updates: Vec<resolution::ResolutionUpdate>, diagnostics: Vec<Diagnostic>) -> Self {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            let mut h = DefaultHasher::new();
            updates.len().hash(&mut h);
            for u in &updates {
                u.element_id.hash(&mut h);
                u.property_name.hash(&mut h);
                u.resolved_value.hash(&mut h);
            }
            diagnostics.len().hash(&mut h);
            for d in &diagnostics {
                crate::analysis::hash_diagnostic(d, &mut h);
            }
            h.finish()
        };
        Self(Arc::new(CachedResolutionData {
            updates,
            diagnostics,
            fingerprint,
        }))
    }

    /// All resolution updates for the workspace.
    pub fn updates(&self) -> &[resolution::ResolutionUpdate] {
        &self.0.updates
    }

    /// All resolution diagnostics for the workspace.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0.diagnostics
    }
}

salsa_arc_wrapper!(fingerprint, CachedResolution, CachedResolutionData);

/// Workspace-tier warning (rule **S005**) for the same top-level package name
/// declared in two or more DIFFERENT files of one project.
///
/// SysML v2 treats those as two DISTINCT root-namespace members, not one
/// "reopened" package — each file is its own root namespace (KerML §7.2.5.3 /
/// §10.2, SysML §7.5.5) and our identity is per-file (ADR-009, 2026-08-05
/// amendment). The spec PERMITS the collision — cross-unit resolution of a
/// simple name is implementation-defined (KerML §8.2.3.5.3) — so this is a
/// **warning, never an error** (erroring would invent a constraint the spec
/// does not impose). It is inherently cross-file, so it lives here at the
/// workspace tier (`DiagnosticTier::NameResWorkspace`) over the merged project
/// graph, NOT in the file-local sysml-core validation pass (S001–S004).
///
/// Scope guards:
/// - **top-level only** — a nested `package P { package P { } }` has an owner,
///   so it is excluded (only `owner.is_none()` packages count);
/// - **across different files only** — two `package P` in ONE file is the
///   file-local distinguishability case (S001), not this one;
/// - **one project only** — the caller passes the project's own merged graph
///   (never the library), so a user package sharing a stdlib name, or two
///   projects sharing a name, never trip it.
///
/// Emits one warning per top-level declaration site (so each file that declares
/// the name is flagged, at the package's own span), naming the other file(s) in
/// the message and attaching related locations pointing at them.
fn duplicate_top_level_package_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    use std::collections::{BTreeMap, BTreeSet};

    // name -> declaration sites (file, span) for top-level, file-attributed packages.
    let mut sites_by_name: BTreeMap<&str, Vec<(String, sysml_span::Span)>> = BTreeMap::new();
    for e in graph.elements.values() {
        // Top-level only: a nested same-named package has an owner.
        if e.owner.is_some() {
            continue;
        }
        if e.kind != sysml_core::ElementKind::Package
            && !e.kind.is_subtype_of(sysml_core::ElementKind::Package)
        {
            continue;
        }
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        // Need a file-attributed span so the diagnostic routes to a file.
        let Some(span) = e.spans.first() else {
            continue;
        };
        if span.file.is_empty() {
            continue;
        }
        sites_by_name
            .entry(name)
            .or_default()
            .push((span.file.clone(), span.clone()));
    }

    let mut diagnostics = Vec::new();
    for (name, mut sites) in sites_by_name {
        // A cross-file collision needs the name in ≥2 DISTINCT files. Fewer ⇒
        // single-file duplication, which is S001's concern, not this rule's.
        // (Owned so it does not borrow `sites` across the sort below.)
        let distinct_files: BTreeSet<String> = sites.iter().map(|(f, _)| f.clone()).collect();
        if distinct_files.len() < 2 {
            continue;
        }
        // Deterministic order (file, then start offset) for stable diagnostics.
        sites.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.start.cmp(&b.1.start)));

        for (file, span) in &sites {
            let others: Vec<&str> = distinct_files
                .iter()
                .filter(|f| f.as_str() != file.as_str())
                .map(String::as_str)
                .collect();
            let others_list = others
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = format!(
                "top-level package `{name}` is also declared in {others_list}; SysML v2 treats \
                 same-named packages in different files as distinct root-namespace members, not \
                 one reopened package (KerML §8.2.3.5.3 — cross-unit name resolution is \
                 implementation-defined)"
            );
            let mut diag = Diagnostic::warning(message)
                .with_code("S005")
                .with_span(span.clone())
                .with_tier(sysml_span::DiagnosticTier::NameResWorkspace);
            // Related locations: the other declaration sites.
            for (other_file, other_span) in &sites {
                if other_file != file {
                    diag = diag.with_related(
                        other_span.clone(),
                        format!("also declared as top-level package `{name}` here"),
                    );
                }
            }
            diagnostics.push(diag);
        }
    }
    diagnostics
}

/// Resolve all references in the workspace merged graph (no library).
///
/// This is the key performance optimization: instead of calling
/// `resolve_references_pure(merged_graph)` once per file (O(N) × ~2 min each),
/// we resolve the entire workspace graph ONCE and cache the result.
/// Per-file resolution functions then filter the cached updates/diagnostics.
///
/// Depends on: `workspace_merged_graph()` (which depends on `parse_file()` for each file)
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn cached_workspace_resolution(
    db: &dyn Db,
    project_files: crate::project_inputs::ProjectFileSet,
) -> CachedResolution {
    let merged = workspace_merged_graph(db, project_files);
    tracing::info!(
        element_count = merged.graph().elements.len(),
        "resolving workspace merged graph (cached)"
    );
    let start = std::time::Instant::now();
    let (updates, result) = resolution::resolve_references_pure(merged.graph());
    let elapsed = start.elapsed();
    tracing::info!(
        resolve_ms = elapsed.as_millis() as u64,
        resolved = result.resolved_count,
        unresolved = result.unresolved_count,
        update_count = updates.len(),
        diagnostic_count = result.diagnostics.len(),
        "workspace resolution complete"
    );
    let mut diagnostics: Vec<Diagnostic> = result.diagnostics.into_iter().collect();
    // S005 (workspace tier): same top-level package name across different files.
    diagnostics.extend(duplicate_top_level_package_diagnostics(merged.graph()));
    CachedResolution::new(updates, diagnostics)
}

/// Resolve all references in the workspace merged graph with library.
///
/// Merges the library graph INTO a clone of the workspace merged graph,
/// then uses `resolve_references_excluding_pure` which has a parallel
/// (rayon) path for large element sets. Library elements are present in
/// the combined graph for name resolution but excluded from the resolution
/// iteration via `exclude_ids`.
///
/// Depends on: `workspace_merged_graph()` + `LibraryGraph` (input)
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn cached_workspace_resolution_with_library(
    db: &dyn Db,
    project_files: crate::project_inputs::ProjectFileSet,
    library: LibraryGraph,
) -> CachedResolution {
    let merged = workspace_merged_graph(db, project_files);
    let lib_data = library.data(db);

    // Clone the workspace graph so the shared helper can merge library
    // elements into it (the salsa-tracked input cannot be mutated in place).
    // `register_library_roots=false` because `workspace_merged_graph` has
    // already registered the workspace's own roots.
    let mut combined = merged.graph().clone();

    tracing::info!(
        workspace_elements = merged.graph().elements.len(),
        library_elements = lib_data.graph().elements.len(),
        "resolving workspace+library merged graph (parallel, cached)"
    );
    let start = std::time::Instant::now();

    let (updates, result) = resolution::resolve_with_library_pure(
        &mut combined,
        lib_data.graph(),
        lib_data.element_ids(),
        false,
    );

    let elapsed = start.elapsed();
    tracing::info!(
        resolve_ms = elapsed.as_millis() as u64,
        resolved = result.resolved_count,
        unresolved = result.unresolved_count,
        update_count = updates.len(),
        diagnostic_count = result.diagnostics.len(),
        "workspace resolution with library complete (parallel)"
    );
    let mut diagnostics: Vec<Diagnostic> = result.diagnostics.into_iter().collect();
    // S005 (workspace tier): same top-level package name across different files.
    // Computed over the workspace graph only (`merged`), never `combined` —
    // library packages must not collide with a user package of the same name.
    diagnostics.extend(duplicate_top_level_package_diagnostics(merged.graph()));
    CachedResolution::new(updates, diagnostics)
}

// ---------------------------------------------------------------------------
// Tracked query function
// ---------------------------------------------------------------------------

/// Resolve a file using the strongest context available.
///
/// Single public resolution entry point — dispatches on the optional
/// inputs (`ProjectFileSet`, `LibraryGraph`) to one of four shapes:
///
/// | `project_files` | `library`  | Shape                                                  |
/// |-----------------|------------|--------------------------------------------------------|
/// | `Some(pfs)`     | `Some(lib)`| Cached workspace + library resolution (parallel path)  |
/// | `Some(pfs)`     | `None`     | Cached workspace-only resolution                       |
/// | `None`          | `Some(lib)`| Single-file with library fallback (no merge)           |
/// | `None`          | `None`     | Single-file in isolation                               |
///
/// The workspace arms read [`cached_workspace_resolution`] /
/// [`cached_workspace_resolution_with_library`] (Layer 1 strategy
/// primitives) and filter their updates/diagnostics to this file. The
/// non-workspace arms run a one-shot pure resolve over the file graph.
///
/// Salsa cache key: `(SourceFile, Option<ProjectFileSet>, Option<LibraryGraph>)`.
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub fn resolve_file_best(
    db: &dyn Db,
    source_file: SourceFile,
    project_files: Option<crate::project_inputs::ProjectFileSet>,
    library: Option<LibraryGraph>,
) -> ResolvedModel {
    let file_name = source_file.name(db).clone();
    let parsed = parse::parse_file(db, source_file);
    let mut graph = parsed.graph().clone();

    match (project_files, library) {
        (Some(project_files), Some(library)) => {
            tracing::debug!(
                document_uri = &file_name,
                "starting resolution with workspace files and library (cached)"
            );
            // Get cached workspace-wide resolution with library (resolved once,
            // shared by all files).
            let cached =
                cached_workspace_resolution_with_library(db, project_files, library);

            // Apply all updates — apply_resolution_updates silently skips
            // elements not in this file's graph, so only this file's elements
            // are updated.
            resolution::apply_resolution_updates(&mut graph, cached.updates());

            // Filter diagnostics to just this file (by span file name).
            // Spanless diagnostics are excluded — they lack file attribution
            // and would otherwise be duplicated across every file in the
            // workspace.
            let diagnostics: Vec<Diagnostic> = cached
                .diagnostics()
                .iter()
                .filter(|d| {
                    d.span
                        .as_ref()
                        .is_some_and(|s| s.file == file_name || s.file.is_empty())
                })
                .cloned()
                .collect();

            ResolvedModel::new(graph, diagnostics)
        }
        (Some(project_files), None) => {
            tracing::debug!(
                document_uri = &file_name,
                "starting resolution with workspace files (cached)"
            );
            // Get cached workspace-wide resolution (resolved once, shared by
            // all files).
            let cached = cached_workspace_resolution(db, project_files);

            // Apply all updates — apply_resolution_updates silently skips
            // elements not in this file's graph, so only this file's elements
            // are updated.
            resolution::apply_resolution_updates(&mut graph, cached.updates());

            // Filter diagnostics to just this file (by span file name).
            let diagnostics: Vec<Diagnostic> = cached
                .diagnostics()
                .iter()
                .filter(|d| {
                    d.span
                        .as_ref()
                        .is_some_and(|s| s.file == file_name || s.file.is_empty())
                })
                .cloned()
                .collect();

            ResolvedModel::new(graph, diagnostics)
        }
        (None, Some(library)) => {
            tracing::debug!(
                document_uri = &file_name,
                "starting resolution with library"
            );
            let lib_data = library.data(db);

            // Resolve with library as fallback, excluding library elements.
            // The library graph is passed as a fallback to the resolution
            // context, which checks it for name lookups, inheritance
            // expansion, and imports. Library elements are NOT merged into
            // the file graph — that would mean O(L) element clones per file
            // on every edit.
            let (updates, result) = resolution::resolve_references_with_fallback_pure(
                &graph,
                lib_data.graph(),
                lib_data.element_ids(),
            );
            resolution::apply_resolution_updates(&mut graph, &updates);

            let diagnostics: Vec<Diagnostic> = result.diagnostics.into_iter().collect();
            ResolvedModel::new(graph, diagnostics)
        }
        (None, None) => {
            tracing::debug!(document_uri = &file_name, "starting resolution");
            // Single-file resolution: resolve within the file's own scope only.
            let (updates, result) = resolution::resolve_references_pure(&graph);
            resolution::apply_resolution_updates(&mut graph, &updates);

            let diagnostics: Vec<Diagnostic> = result.diagnostics.into_iter().collect();
            ResolvedModel::new(graph, diagnostics)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;

    #[test]
    fn resolve_simple_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let resolved = resolve_file_best(&db, sf, None, None);

        // Simple package with no references should resolve cleanly
        let _graph = resolved.graph();
        let _diags = resolved.diagnostics();
    }

    #[test]
    fn resolve_with_type_reference() {
        let db = RootDatabase::default();
        let source = r#"
            package Vehicle {
                part def Engine;
                part engine : Engine;
            }
        "#;
        let sf = SourceFile::new(&db, "test.sysml".to_string(), source.to_string());
        let resolved = resolve_file_best(&db, sf, None, None);

        // Should have some resolved references (Engine type)
        let graph = resolved.graph();
        assert!(!graph.elements.is_empty());
    }

    #[test]
    fn incremental_resolution() {
        let mut db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package A {}".to_string());

        // First resolution
        let resolved1 = resolve_file_best(&db, sf, None, None);
        let count1 = resolved1.graph().elements.len();

        // Update source
        use salsa::Setter;
        sf.set_text(&mut db)
            .to("package A {} package B {}".to_string());

        // Re-resolve (salsa detects input changed)
        let resolved2 = resolve_file_best(&db, sf, None, None);
        let count2 = resolved2.graph().elements.len();

        assert!(count2 > count1);
    }

    #[test]
    fn memoized_resolution() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());

        let r1 = resolve_file_best(&db, sf, None, None);
        let r2 = resolve_file_best(&db, sf, None, None);

        // Same input → same memoized result (pointer-equal)
        assert_eq!(r1, r2);
    }

    #[test]
    fn resolve_with_library() {
        let db = RootDatabase::default();

        // Create a minimal "library" with a type definition
        let mut lib_graph = ModelGraph::new();
        let lib_type_id = ElementId::new_v4();
        let mut elem =
            sysml_core::Element::new(lib_type_id.clone(), sysml_core::ElementKind::PartDefinition);
        elem.name = Some("Base".to_string());
        lib_graph.elements.insert(lib_type_id.clone(), elem);
        lib_graph.rebuild_indexes();

        let lib_data = LibraryData::new(lib_graph);
        let library = LibraryGraph::new(&db, lib_data);

        // Parse a file that references the library type
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package Test { part myPart : Base; }".to_string(),
        );

        let resolved = resolve_file_best(&db, sf, None, Some(library));
        // Just verify it doesn't panic and produces a result
        let _graph = resolved.graph();
    }

    #[test]
    fn resolve_with_library_uses_fallback_not_merge() {
        let db = RootDatabase::default();

        // Create a library with both named and unnamed elements
        let mut lib_graph = ModelGraph::new();

        // Named element
        let named_id = ElementId::new_v4();
        let mut named_elem =
            sysml_core::Element::new(named_id.clone(), sysml_core::ElementKind::PartDefinition);
        named_elem.name = Some("LibType".to_string());
        lib_graph.elements.insert(named_id.clone(), named_elem);

        // Unnamed element
        let unnamed_id = ElementId::new_v4();
        let unnamed_elem =
            sysml_core::Element::new(unnamed_id.clone(), sysml_core::ElementKind::Membership);
        lib_graph.elements.insert(unnamed_id.clone(), unnamed_elem);

        lib_graph.rebuild_indexes();

        let lib_data = LibraryData::new(lib_graph);
        let library = LibraryGraph::new(&db, lib_data);

        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Test {}".to_string());
        let resolved = resolve_file_best(&db, sf, None, Some(library));
        let graph = resolved.graph();

        // With dual-graph resolution, library elements are NOT in the resolved graph.
        // They are only used as a fallback during name resolution.
        assert!(
            !graph.elements.contains_key(&named_id),
            "Library elements should not be merged into file graph"
        );
        assert!(
            !graph.elements.contains_key(&unnamed_id),
            "Library elements should not be merged into file graph"
        );
    }

    #[test]
    fn content_based_resolution_equality() {
        // Re-blessed 2026-07-16 (content-true fingerprint): equality is
        // across ALLOCATIONS of the same file+content, not across
        // different files with identical text (whose spans/ids
        // genuinely differ — see parse::same_text_different_file_is_not_equal).
        let db1 = RootDatabase::default();
        let db2 = RootDatabase::default();
        let sf1 = SourceFile::new(&db1, "a.sysml".to_string(), "package Foo {}".to_string());
        let sf2 = SourceFile::new(&db2, "a.sysml".to_string(), "package Foo {}".to_string());
        let r1 = resolve_file_best(&db1, sf1, None, None);
        let r2 = resolve_file_best(&db2, sf2, None, None);
        // Different Arc pointers, same file+content -> equal via fingerprint
        assert_eq!(r1, r2);
    }

    // -----------------------------------------------------------------------
    // S005 — duplicate top-level package name across files (F4 condition (c))
    // -----------------------------------------------------------------------

    /// Add a package element with a file-attributed span to `g`.
    #[cfg(test)]
    fn add_pkg(
        g: &mut ModelGraph,
        name: &str,
        file: &str,
        owner: Option<ElementId>,
    ) -> ElementId {
        let id = ElementId::new_v4();
        let mut e = sysml_core::Element::new(id.clone(), sysml_core::ElementKind::Package);
        e.name = Some(name.to_string());
        e.owner = owner;
        e.spans.push(sysml_span::Span::new(file.to_string(), 0, 1));
        g.elements.insert(id.clone(), e);
        id
    }

    #[test]
    fn s005_helper_flags_only_cross_file_top_level_duplicates() {
        let mut g = ModelGraph::new();
        // Cross-file collision: `P` at top level in two different files.
        add_pkg(&mut g, "P", "a.sysml", None);
        add_pkg(&mut g, "P", "b.sysml", None);
        // Single-file duplicate `R` twice in ONE file — that's S001's job, not S005.
        add_pkg(&mut g, "R", "c.sysml", None);
        add_pkg(&mut g, "R", "c.sysml", None);
        // Nested `U`: a top-level `U` in one file plus a NESTED `U` (owner set) in
        // another must NOT collide — only one top-level U exists.
        add_pkg(&mut g, "U", "f.sysml", None);
        let gp = add_pkg(&mut g, "Container", "g.sysml", None);
        add_pkg(&mut g, "U", "g.sysml", Some(gp));
        g.rebuild_indexes();

        let diags = duplicate_top_level_package_diagnostics(&g);
        // Only the `P` pair trips: one warning per declaration site.
        assert_eq!(diags.len(), 2, "only cross-file top-level `P`: {diags:?}");
        for d in &diags {
            assert_eq!(d.severity, sysml_span::Severity::Warning, "never an error");
            assert_eq!(d.code.as_deref(), Some("S005"));
            assert_eq!(d.tier, sysml_span::DiagnosticTier::NameResWorkspace);
            assert!(d.message.contains("`P`"));
            assert!(!d.related.is_empty(), "cross-points to the other site");
        }
        let a = diags
            .iter()
            .find(|d| d.span.as_ref().unwrap().file == "a.sysml")
            .expect("warning on a.sysml");
        assert!(a.message.contains("`b.sysml`"), "a names b: {}", a.message);
        let b = diags
            .iter()
            .find(|d| d.span.as_ref().unwrap().file == "b.sysml")
            .expect("warning on b.sysml");
        assert!(b.message.contains("`a.sysml`"), "b names a: {}", b.message);
    }

    #[test]
    fn s005_no_warning_for_distinct_names() {
        let mut g = ModelGraph::new();
        add_pkg(&mut g, "Alpha", "a.sysml", None);
        add_pkg(&mut g, "Beta", "b.sysml", None);
        g.rebuild_indexes();
        assert!(duplicate_top_level_package_diagnostics(&g).is_empty());
    }

    #[test]
    fn s005_surfaces_per_file_through_workspace_resolution() {
        use crate::host::AnalysisHost;
        use crate::project_inputs::{ProjectFileSet, PROJECT_KIND_DISCOVERED};
        use sysml_project::ProjectHandle;

        let mut host = AnalysisHost::new();
        let pid = ProjectHandle(7);
        host.load_project(sysml_project::Project {
            id: pid,
            info: sysml_project::ProjectInfo {
                name: "dup-pkg-test".to_string(),
                description: None,
                version: "1.0.0".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::InMemory,
        });

        let files = [
            ("file:///a.sysml", "package P { part a; }"),
            ("file:///b.sysml", "package P { part b; }"),
        ];
        let mut source_files = Vec::new();
        for (uri, content) in files {
            host.set_file_content_in_project(uri, content.to_string(), pid);
            let fid = host.file_id(uri).expect("file_id");
            source_files.push(host.source_file(fid).expect("source_file"));
        }
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(source_files.clone()),
            PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);

        // a.sysml: exactly one S005 warning that names b.sysml, spanned in a.sysml.
        let ra = resolve_file_best(host.analysis().db(), source_files[0], Some(pfs), None);
        let a_s005: Vec<_> = ra
            .diagnostics()
            .iter()
            .filter(|d| d.code.as_deref() == Some("S005"))
            .collect();
        assert_eq!(a_s005.len(), 1, "a.sysml one S005: {:?}", ra.diagnostics());
        assert_eq!(a_s005[0].severity, sysml_span::Severity::Warning);
        assert_eq!(a_s005[0].span.as_ref().unwrap().file, "file:///a.sysml");
        assert!(a_s005[0].message.contains("b.sysml"), "{}", a_s005[0].message);

        // b.sysml: symmetric.
        let rb = resolve_file_best(host.analysis().db(), source_files[1], Some(pfs), None);
        let b_s005: Vec<_> = rb
            .diagnostics()
            .iter()
            .filter(|d| d.code.as_deref() == Some("S005"))
            .collect();
        assert_eq!(b_s005.len(), 1, "b.sysml one S005: {:?}", rb.diagnostics());
        assert!(b_s005[0].message.contains("a.sysml"), "{}", b_s005[0].message);
    }
}
