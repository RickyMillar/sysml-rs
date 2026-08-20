//! Analysis layer: tracked queries for validation and elaboration.
//!
//! This is Layer 4 of the salsa query hierarchy. Analysis depends on
//! the resolution layer (Layer 3) and produces validated/elaborated models.
//!
//! ## Design
//!
//! Validation and elaboration use the same "clone and mutate" pattern as
//! resolution:
//! 1. Clone the resolved ModelGraph
//! 2. Run validation/elaboration on the clone
//! 3. Return results wrapped in Arc
//!
//! We combine all validation passes (property, semantic, structural, import
//! health) into a single `validate_file_best` query that returns all diagnostics.
//! This mirrors how the LSP server publishes diagnostics — all at once per file.
//!
//! Elaboration is separate because it mutates the graph (adding derived
//! structure) and is only needed for execution features, not basic editing.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::elaborate::{self, ElaborationReport};
use sysml_core::ModelGraph;
use sysml_core::{SemanticError, ValidationResult};
use sysml_span::{Diagnostic, Severity};

use crate::resolution::{self, LibraryGraph};
use crate::source::SourceFile;
use crate::Db;

// ---------------------------------------------------------------------------
// Result wrapper types
// ---------------------------------------------------------------------------

/// Validation result: all diagnostics from property, semantic, structural,
/// and import health checks combined.
#[derive(Clone, Debug)]
pub struct ValidatedModel(Arc<ValidatedModelData>);

#[derive(Debug)]
struct ValidatedModelData {
    /// All validation diagnostics combined.
    diagnostics: Vec<Diagnostic>,
    /// Property validation result (for detailed access).
    property_result: ValidationResult,
    /// Semantic errors (for detailed access).
    semantic_errors: Vec<SemanticError>,
    /// Number of structural errors found.
    structural_error_count: usize,
    /// Number of import health issues found.
    import_health_count: usize,
    /// Content fingerprint for equality comparison.
    fingerprint: u64,
}

pub(crate) fn hash_diagnostic<H: Hasher>(diag: &Diagnostic, state: &mut H) {
    diag.severity.hash(state);
    diag.code.hash(state);
    diag.message.hash(state);
    diag.span.hash(state);
    diag.notes.hash(state);
    diag.tags.hash(state);
    diag.related.len().hash(state);
    for related in &diag.related {
        related.span.hash(state);
        related.message.hash(state);
    }
}

fn hash_semantic_error<H: Hasher>(error: &SemanticError, state: &mut H) {
    error.element_id.hash(state);
    error.element_name.hash(state);
    error.rule_id.hash(state);
    error.is_warning.hash(state);
    error.kind.to_string().hash(state);
}

impl ValidatedModel {
    fn new(
        diagnostics: Vec<Diagnostic>,
        property_result: ValidationResult,
        semantic_errors: Vec<SemanticError>,
        structural_error_count: usize,
        import_health_count: usize,
    ) -> Self {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            diagnostics.len().hash(&mut h);
            for diag in &diagnostics {
                hash_diagnostic(diag, &mut h);
            }
            property_result.errors.len().hash(&mut h);
            for error in &property_result.errors {
                error.property.hash(&mut h);
                error.kind.to_string().hash(&mut h);
                error.span.hash(&mut h);
            }
            semantic_errors.len().hash(&mut h);
            for err in &semantic_errors {
                hash_semantic_error(err, &mut h);
            }
            structural_error_count.hash(&mut h);
            import_health_count.hash(&mut h);
            h.finish()
        };
        Self(Arc::new(ValidatedModelData {
            diagnostics,
            property_result,
            semantic_errors,
            structural_error_count,
            import_health_count,
            fingerprint,
        }))
    }

    /// All validation diagnostics (property + semantic + structural + import).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0.diagnostics
    }

    /// Property validation result.
    pub fn property_result(&self) -> &ValidationResult {
        &self.0.property_result
    }

    /// Semantic errors found.
    pub fn semantic_errors(&self) -> &[SemanticError] {
        &self.0.semantic_errors
    }

    /// Number of structural errors.
    pub fn structural_error_count(&self) -> usize {
        self.0.structural_error_count
    }

    /// Number of import health issues.
    pub fn import_health_count(&self) -> usize {
        self.0.import_health_count
    }

    /// Whether the model has any validation errors.
    pub fn has_errors(&self) -> bool {
        !self.0.diagnostics.is_empty()
    }

    /// Total number of validation diagnostics.
    pub fn diagnostic_count(&self) -> usize {
        self.0.diagnostics.len()
    }
}

salsa_arc_wrapper!(fingerprint, ValidatedModel, ValidatedModelData);

/// Elaborated model: a ModelGraph with derived structure added.
#[derive(Clone, Debug)]
pub struct ElaboratedModel(Arc<ElaboratedModelData>);

#[derive(Debug)]
struct ElaboratedModelData {
    /// The elaborated model graph (resolved graph + derived structure).
    graph: ModelGraph,
    /// Elaboration report with counts of derived elements.
    report: ElaborationReport,
    /// Content fingerprint for equality comparison.
    fingerprint: u64,
}

impl ElaboratedModel {
    fn new(graph: ModelGraph, report: ElaborationReport) -> Self {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            graph.fingerprint().hash(&mut h);
            report.total().hash(&mut h);
            h.finish()
        };
        Self(Arc::new(ElaboratedModelData {
            graph,
            report,
            fingerprint,
        }))
    }

    /// The elaborated model graph.
    pub fn graph(&self) -> &ModelGraph {
        &self.0.graph
    }

    /// The elaboration report.
    pub fn report(&self) -> &ElaborationReport {
        &self.0.report
    }

    /// Total number of derived elements/properties added.
    pub fn total_derived(&self) -> usize {
        self.0.report.total()
    }
}

salsa_arc_wrapper!(fingerprint, ElaboratedModel, ElaboratedModelData);

/// Whole-workspace elaborated graph: merge → resolve → elaborate of every
/// file in a [`ProjectFileSet`] (optionally with the standard library).
///
/// Mirrors the historical `__workspace__` graph that `sysml-service` built
/// inline on every `load_workspace` call (S2 migration). Wrapped in `Arc` so
/// callers that need an `Arc<ModelGraph>` can clone cheaply.
#[derive(Clone, Debug)]
pub struct ElaboratedWorkspace(Arc<ElaboratedWorkspaceData>);

#[derive(Debug)]
struct ElaboratedWorkspaceData {
    graph: Arc<ModelGraph>,
    report: ElaborationReport,
    fingerprint: u64,
}

impl ElaboratedWorkspace {
    fn new(graph: ModelGraph, report: ElaborationReport) -> Self {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            graph.fingerprint().hash(&mut h);
            report.total().hash(&mut h);
            h.finish()
        };
        Self(Arc::new(ElaboratedWorkspaceData {
            graph: Arc::new(graph),
            report,
            fingerprint,
        }))
    }

    /// The elaborated workspace graph (Arc for cheap cloning).
    pub fn graph(&self) -> &Arc<ModelGraph> {
        &self.0.graph
    }

    /// The elaboration report.
    pub fn report(&self) -> &ElaborationReport {
        &self.0.report
    }
}

salsa_arc_wrapper!(fingerprint, ElaboratedWorkspace, ElaboratedWorkspaceData);

// ---------------------------------------------------------------------------
// Tracked query functions
// ---------------------------------------------------------------------------

/// Run all validation passes on a model graph.
///
/// `library` provides standard library types for import existence checks.
/// `workspace` provides the full workspace-merged graph for cross-file
/// import namespace and member lookups.
fn validate_graph(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    workspace: Option<&ModelGraph>,
) -> ValidatedModel {
    let mut diagnostics = Vec::new();

    // 1. Property validation (shape constraints)
    let property_result = sysml_core::validate_graph_properties(graph);
    for error in &property_result.errors {
        let mut diag: Diagnostic = error.clone().into();
        // Keep property diagnostics as warnings to match existing UX policy.
        diag.severity = Severity::Warning;
        diagnostics.push(diag);
    }

    // 2. Semantic validation (generated dispatcher: 86 rules)
    let semantic_errors = sysml_core::validate_semantic(graph);
    for error in &semantic_errors {
        diagnostics.push(error.to_diagnostic_with_graph(graph));
    }

    // 3. Structural validation (orphans, cycles, dangling refs)
    let structural_errors = graph.validate_structure();
    let structural_error_count = structural_errors.len();
    for error in structural_errors {
        let diag: Diagnostic = error.into();
        diagnostics.push(diag);
    }

    // 4. Import health diagnostics (workspace-aware when available)
    let import_diags =
        sysml_core::import_health_diagnostics_with_context(graph, library, workspace);
    let import_health_count = import_diags.len();
    diagnostics.extend(import_diags);

    ValidatedModel::new(
        diagnostics,
        property_result,
        semantic_errors,
        structural_error_count,
        import_health_count,
    )
}

/// Whole-workspace elaboration: merge → resolve → elaborate every file in
/// the project, returning a single elaborated `ModelGraph` that mirrors the
/// historical `__workspace__` graph from `sysml-service`.
///
/// Memoized: a single file edit invalidates the merged graph and re-runs
/// resolution + elaboration once for the whole workspace, not once per file.
///
/// Depends on: `workspace_merged_graph()` + `cached_workspace_resolution()` +
/// `ProjectFileSet` (input)
#[tracing::instrument(level = "debug", skip(db, project_files))]
#[salsa::tracked]
pub(crate) fn elaborate_workspace(
    db: &dyn Db,
    project_files: crate::project_inputs::ProjectFileSet,
) -> ElaboratedWorkspace {
    let merged = resolution::workspace_merged_graph(db, project_files);
    let cached = resolution::cached_workspace_resolution(db, project_files);

    let mut graph = merged.graph().clone();
    sysml_core::resolution::apply_resolution_updates(&mut graph, cached.updates());

    tracing::info!(
        elements_pre_elaborate = graph.elements.len(),
        "elaborating workspace merged graph"
    );
    let report = elaborate::elaborate(&mut graph);
    graph.rebuild_indexes();

    ElaboratedWorkspace::new(graph, report)
}

/// Whole-workspace elaboration with the standard library merged in.
///
/// Mirrors what `sysml-service` did via `__workspace__` after S0:
/// merge all doc graphs + stdlib, run `resolve_references`, then run
/// `elaborate`. The cached resolution updates are applied to a workspace +
/// library combined graph (the same shape `cached_workspace_resolution_with_library`
/// resolves against), then elaborated.
///
/// Depends on: `workspace_merged_graph()` + `cached_workspace_resolution_with_library()` +
/// `ProjectFileSet` + `LibraryGraph` (inputs)
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub(crate) fn elaborate_workspace_with_library(
    db: &dyn Db,
    project_files: crate::project_inputs::ProjectFileSet,
    library: LibraryGraph,
) -> ElaboratedWorkspace {
    let merged = resolution::workspace_merged_graph(db, project_files);
    let lib_data = library.data(db);
    let cached =
        resolution::cached_workspace_resolution_with_library(db, project_files, library);

    // Build the same combined graph (workspace + library) that the cached
    // resolution iterated over, then apply its updates and elaborate.
    let mut graph = merged.graph().clone();
    // `as_library = true`: register the merged library's root packages as
    // library packages on the combined graph. Downstream runtime consumers
    // walk this elaborated graph with `ModelGraph::is_library_element` to scope
    // their processing to user-model elements — e.g. `detect_computed_expressions`
    // (which must NOT register SI/ISQ unit definitions like
    // `attribute <Pa> pascal = N/m^2;` as per-tick orchestrator computed
    // expressions), and the library skips in `constraints.rs` / `cases/health.rs`.
    // Without this the predicate silently returns `false` for every library
    // element on this graph. Elaboration itself does not consult library
    // membership, so registering here does not change elaboration output.
    graph.merge_from_ref(lib_data.graph(), true);
    sysml_core::resolution::apply_resolution_updates(&mut graph, cached.updates());

    tracing::info!(
        elements_pre_elaborate = graph.elements.len(),
        "elaborating workspace+library merged graph"
    );
    // Ridge A.2: pass the library graph + prebuilt InheritanceIndex through to
    // elaborate. IG-1's `resolve_base` would otherwise see `library = None`
    // here and construct a fresh `ResolutionContext::new(graph)` per candidate,
    // which lazy-builds the inheritance closure over the full 77k merged
    // elements every time. With the prebuilt index supplied, the library-only
    // fast path is an O(1) `Arc::clone` (the InheritanceIndexHandle::Shared
    // seam from Ridge A v2, commit 3b69d9d1). Stdlib base qnames like
    // `Connections::Connection` resolve through that fast path, so the
    // expensive dual-graph fallback (`build_user_overlay`) almost never fires.
    let report = elaborate::elaborate_with_library(
        &mut graph,
        Some(lib_data.graph()),
        Some(lib_data.inheritance_index()),
    );
    graph.rebuild_indexes();

    ElaboratedWorkspace::new(graph, report)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn elaborate_workspace_best(
    db: &dyn Db,
    project_files: crate::project_inputs::ProjectFileSet,
    library: Option<LibraryGraph>,
) -> ElaboratedWorkspace {
    match library {
        Some(lib) => elaborate_workspace_with_library(db, project_files, lib),
        None => elaborate_workspace(db, project_files),
    }
}

/// Validate a file using the strongest context available.
///
/// Single public validation entry point — runs property + semantic +
/// structural + import-health passes on the resolved model for this file.
/// Dispatches on the optional inputs (`ProjectFileSet`, `LibraryGraph`)
/// to four shapes:
///
/// | `project_files` | `library`  | Shape                                                  |
/// |-----------------|------------|--------------------------------------------------------|
/// | `Some(pfs)`     | `Some(lib)`| Workspace + library resolve; import-health sees both   |
/// | `Some(pfs)`     | `None`     | Workspace resolve; import-health sees workspace graph  |
/// | `None`          | `Some(lib)`| Single-file with library fallback; import-health: lib  |
/// | `None`          | `None`     | Single-file in isolation                               |
///
/// Resolution is delegated to [`resolution::resolve_file_best`]; this
/// function only owns the post-resolve validation step and supplies the
/// workspace-merged graph + library graph to [`validate_graph`] for the
/// import-health namespace lookups.
///
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub fn validate_file_best(
    db: &dyn Db,
    source_file: SourceFile,
    project_files: Option<crate::project_inputs::ProjectFileSet>,
    library: Option<LibraryGraph>,
) -> ValidatedModel {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting validation (best)"
    );
    let resolved = resolution::resolve_file_best(db, source_file, project_files, library);
    let graph = resolved.graph();

    // Import-health namespace lookups need the workspace-merged graph
    // (cross-file namespaces) and the library graph (stdlib namespaces)
    // when available — both are non-None salsa inputs in their respective
    // arms.
    let lib_graph_owner = library.map(|lib| lib.data(db));
    let lib_graph = lib_graph_owner.as_ref().map(|d| d.graph());
    let merged_owner = project_files.map(|pfs| resolution::workspace_merged_graph(db, pfs));
    let merged_graph = merged_owner.as_ref().map(|m| m.graph());

    validate_graph(graph, lib_graph, merged_graph)
}

/// Elaborate a file using the strongest context available.
///
/// Single public elaboration entry point — adds derived structure
/// (implicit state transitions, constraint bindings, succession edges,
/// flow connections, …) to the resolved model graph for this file.
/// Dispatches on the optional inputs (`ProjectFileSet`, `LibraryGraph`)
/// to four shapes:
///
/// | `project_files` | `library`  | Shape                                                                |
/// |-----------------|------------|----------------------------------------------------------------------|
/// | `Some(pfs)`     | `Some(lib)`| Workspace + library resolve; IG-1 walks library inheritance index    |
/// | `Some(pfs)`     | `None`     | Workspace resolve; plain `elaborate` (no IG-1 library walk)          |
/// | `None`          | `Some(lib)`| Single-file with library fallback resolve; plain `elaborate`         |
/// | `None`          | `None`     | Single-file in isolation; plain `elaborate`                          |
///
/// Resolution is delegated to [`resolution::resolve_file_best`]; this
/// function only owns the post-resolve elaboration step. When the
/// library is loaded, [`elaborate::elaborate_with_library`] is used so
/// IG-1 implicit-generalization base types can be resolved against the
/// library inheritance index without an O(L) merge.
///
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub fn elaborate_file_best(
    db: &dyn Db,
    source_file: SourceFile,
    project_files: Option<crate::project_inputs::ProjectFileSet>,
    library: Option<LibraryGraph>,
) -> ElaboratedModel {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting elaboration (best)"
    );
    let resolved = resolution::resolve_file_best(db, source_file, project_files, library);
    let mut graph = resolved.graph().clone();

    // IG-1: only the workspace+library arm originally used
    // `elaborate_with_library` (which walks the library inheritance index
    // for implicit-generalization base types). The other three named
    // variants used plain `elaborate`. Preserve that asymmetry — see
    // item 2 (validate / elaborate inlining lossy audit).
    let report = match (project_files, library) {
        (Some(_), Some(library)) => {
            let lib_data = library.data(db);
            elaborate::elaborate_with_library(
                &mut graph,
                Some(lib_data.graph()),
                Some(lib_data.inheritance_index()),
            )
        }
        _ => elaborate::elaborate(&mut graph),
    };

    ElaboratedModel::new(graph, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;

    #[test]
    fn validate_simple_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let validated = validate_file_best(&db, sf, None, None);

        // Simple package should have minimal or no validation errors
        let _diags = validated.diagnostics();
        let _count = validated.diagnostic_count();
    }

    #[test]
    fn validate_with_elements() {
        let db = RootDatabase::default();
        let source = r#"
            package Vehicle {
                part def Engine;
                part engine : Engine;
            }
        "#;
        let sf = SourceFile::new(&db, "test.sysml".to_string(), source.to_string());
        let validated = validate_file_best(&db, sf, None, None);

        // Just verify it doesn't panic and produces a result
        let _diags = validated.diagnostics();
    }

    #[test]
    fn elaborate_simple_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let elaborated = elaborate_file_best(&db, sf, None, None);

        // Simple package has nothing to elaborate
        let _graph = elaborated.graph();
        let _report = elaborated.report();
    }

    #[test]
    fn elaborate_with_states() {
        let db = RootDatabase::default();
        let source = r#"
            package SM {
                state def VehicleState;
                state vehicleState : VehicleState {
                    entry; then idle;
                    state idle;
                    state running;
                }
            }
        "#;
        let sf = SourceFile::new(&db, "test.sysml".to_string(), source.to_string());
        let elaborated = elaborate_file_best(&db, sf, None, None);

        // State machine elaboration may tag initial/final states
        let _report = elaborated.report();
        let _graph = elaborated.graph();
    }

    #[test]
    fn incremental_validation() {
        let mut db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package A {}".to_string());

        // First validation
        let v1 = validate_file_best(&db, sf, None, None);
        let _count1 = v1.diagnostic_count();

        // Update source
        use salsa::Setter;
        sf.set_text(&mut db)
            .to("package A {} package B {}".to_string());

        // Re-validate (salsa detects input changed and recomputes)
        let v2 = validate_file_best(&db, sf, None, None);
        let _count2 = v2.diagnostic_count();

        // With content-based equality, both produce zero errors so they're
        // fingerprint-equal. Verify salsa recomputed by checking they are
        // different Arc allocations.
        assert!(
            !Arc::ptr_eq(&v1.0, &v2.0),
            "Should be different Arc allocations after reparse"
        );
    }

    #[test]
    fn memoized_validation() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());

        let v1 = validate_file_best(&db, sf, None, None);
        let v2 = validate_file_best(&db, sf, None, None);

        // Same input → same memoized result (pointer-equal)
        assert_eq!(v1, v2);
    }

    #[test]
    fn memoized_elaboration() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());

        let e1 = elaborate_file_best(&db, sf, None, None);
        let e2 = elaborate_file_best(&db, sf, None, None);

        // Same input → same memoized result
        assert_eq!(e1, e2);
    }

    #[test]
    fn content_based_validation_equality() {
        let db = RootDatabase::default();
        let sf1 = SourceFile::new(&db, "a.sysml".to_string(), "package Foo {}".to_string());
        let sf2 = SourceFile::new(&db, "b.sysml".to_string(), "package Foo {}".to_string());
        let v1 = validate_file_best(&db, sf1, None, None);
        let v2 = validate_file_best(&db, sf2, None, None);
        // Different Arc pointers but same content -> equal via fingerprint
        assert_eq!(v1, v2);
    }

    #[test]
    fn validation_fingerprint_changes_with_diagnostic_content() {
        let v1 = ValidatedModel::new(
            vec![Diagnostic::warning("first warning")],
            ValidationResult::new(),
            Vec::new(),
            0,
            0,
        );
        let v2 = ValidatedModel::new(
            vec![Diagnostic::warning("second warning")],
            ValidationResult::new(),
            Vec::new(),
            0,
            0,
        );

        assert_ne!(
            v1, v2,
            "Validation identity must change when diagnostic content changes"
        );
    }

    #[test]
    fn content_based_elaboration_equality() {
        // Re-blessed 2026-07-16 (content-true fingerprint): equality is
        // across ALLOCATIONS of the same file+content, not across
        // different files with identical text (whose spans/ids
        // genuinely differ — see parse::same_text_different_file_is_not_equal).
        let db1 = RootDatabase::default();
        let db2 = RootDatabase::default();
        let sf1 = SourceFile::new(&db1, "a.sysml".to_string(), "package Foo {}".to_string());
        let sf2 = SourceFile::new(&db2, "a.sysml".to_string(), "package Foo {}".to_string());
        let e1 = elaborate_file_best(&db1, sf1, None, None);
        let e2 = elaborate_file_best(&db2, sf2, None, None);
        // Different Arc pointers, same file+content -> equal via fingerprint
        assert_eq!(e1, e2);
    }
}
