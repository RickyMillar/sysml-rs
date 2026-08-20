//! AnalysisHost / Analysis pattern for the LSP server.
//!
//! `AnalysisHost` owns the mutable database and is used by the main LSP event
//! loop to apply changes (file edits, config changes).
//!
//! `Analysis` is an immutable snapshot of the database state. LSP request
//! handlers receive an `Analysis` to read from while the host continues
//! accepting edits.
//!
//! This follows rust-analyzer's `AnalysisHost` / `Analysis` pattern from
//! `crates/tooling/sysml-ide-db/src/lib.rs`.

use std::sync::{Arc, OnceLock};

use sysml_core::ModelGraph;
use sysml_project::{Project, ProjectHandle, ProjectMeta, StdlibRegistry};

use crate::analysis::{self, ElaboratedModel, ElaboratedWorkspace, ValidatedModel};
use crate::parse::{self, ParseResult, PositionMap};
use crate::project_inputs::{ProjectFileSet, SalsaProject, WorkspaceConfig};
use crate::resolution::{self, compute_stdlib_library_data, LibraryData, LibraryGraph, ResolvedModel};
use crate::source::{FileId, FileSet, SourceFile};
use crate::symbol_index_query::GlobalSymbolIndex;
use crate::RootDatabase;

/// Reserved [`ProjectHandle`] for the bundled standard-library [`ProjectFileSet`].
///
/// Sits above the per-urn stdlib metadata range (`0..=9`) and below the user
/// workspace project IDs (the service uses `100`). All loaded stdlib source
/// files share this single PFS so that downstream queries can compute the
/// library graph via one salsa-tracked `compute_stdlib_library_data` call.
pub const STDLIB_BUNDLE_PROJECT_ID: u32 = 99;

// LSP_DEFAULT_WORKSPACE_PROJECT_ID (= 101) removed in P4 of the file-loading
// rebuild. The LSP now uses `sysml_service::open_context::DEFAULT_PROJECT_ID`
// (= 100) for the same purpose, sharing the constant with the service layer.
// The two-pid split (100 service / 101 LSP) was an artefact of pre-unified
// hosts; under the shared `AnalysisHost` introduced in S2.T6 they always
// pointed at the same `ProjectFileSet` anyway.

/// Owns the mutable database. Lives in the LSP main loop.
///
/// All mutations (file edits, config changes) go through this type.
/// To serve read requests, call [`analysis()`](Self::analysis) to get
/// an immutable snapshot.
///
/// ## Cancellation
///
/// When `analysis()` is called, it clones the database. While a clone
/// exists, any mutation on the host will:
/// 1. Set salsa's cancellation flag
/// 2. Block until all clones are dropped
///
/// Active queries on cloned databases will detect the flag and panic
/// with `salsa::Cancelled`. LSP handlers should wrap query calls in
/// `salsa::Cancelled::catch()` to handle this gracefully.
pub struct AnalysisHost {
    db: RootDatabase,
    files: FileSet,
    library: Option<LibraryGraph>,
    /// Loaded projects (indexed by ProjectHandle.0).
    projects: Vec<Project>,
    /// Stdlib registry (loaded on demand).
    stdlib_registry: Option<StdlibRegistry>,
    /// Salsa project inputs (for building WorkspaceConfig).
    salsa_projects: Vec<SalsaProject>,
    /// Workspace configuration (singleton salsa input).
    workspace_config: Option<WorkspaceConfig>,
    /// Project file sets (one per project, maps ProjectHandle to all its source files).
    project_file_sets: Vec<crate::ProjectFileSet>,
}

impl std::fmt::Debug for AnalysisHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisHost")
            .field("files", &self.files)
            .field("library", &self.library.map(|_| "<loaded>"))
            .field("projects", &self.projects.len())
            .field("workspace_config", &self.workspace_config.map(|_| "<set>"))
            .field("project_file_sets", &self.project_file_sets.len())
            .finish()
    }
}

impl AnalysisHost {
    /// Create a new host with an empty database.
    pub fn new() -> Self {
        Self {
            db: RootDatabase::default(),
            files: FileSet::new(),
            library: None,
            projects: Vec::new(),
            stdlib_registry: None,
            salsa_projects: Vec::new(),
            workspace_config: None,
            project_file_sets: Vec::new(),
        }
    }

    /// Get an immutable snapshot for serving read requests.
    ///
    /// This clones the database (cheap — just bumps Arc refcounts).
    /// The snapshot can be sent to another thread for processing.
    ///
    /// **Important**: The returned `Analysis` holds a database clone.
    /// Any mutation on the host will block until all `Analysis` clones
    /// are dropped. On clone threads, active queries will receive a
    /// `salsa::Cancelled` panic. Use `salsa::Cancelled::catch()` to
    /// handle this gracefully.
    pub fn analysis(&self) -> Analysis {
        Analysis {
            db: self.db.clone(),
            library: self.library,
            workspace_config: self.workspace_config,
            project_file_sets: Arc::new(self.project_file_sets.clone()),
        }
    }

    /// Get a reference to the underlying database (for advanced use).
    pub fn db(&self) -> &RootDatabase {
        &self.db
    }

    /// Get a mutable reference to the underlying database.
    pub fn db_mut(&mut self) -> &mut RootDatabase {
        &mut self.db
    }

    /// Set or update the source text for a file.
    ///
    /// This is the primary entry point for `textDocument/didChange` events.
    /// It updates the salsa input, which will invalidate all downstream
    /// queries that depend on this file's content.
    ///
    /// **Note**: This will block until all `Analysis` clones are dropped,
    /// triggering cancellation on any in-flight queries.
    pub fn set_file_content(&mut self, uri: &str, text: String) -> FileId {
        self.files.set_file_text(&mut self.db, uri, text)
    }

    /// Set or update the source text for a file with project association.
    ///
    /// Like [`set_file_content`](Self::set_file_content) but also records which
    /// project owns the file, enabling workspace-aware cross-file resolution.
    pub fn set_file_content_in_project(
        &mut self,
        uri: &str,
        text: String,
        project_id: ProjectHandle,
    ) -> FileId {
        let id = self
            .files
            .set_file_text_in_project(&mut self.db, uri, text, project_id);
        self.apply_project_root_scope(uri, project_id);
        id
    }

    /// Compute the checkout-independent identity root scope (ADR-009) for a
    /// file from its owning project's root directory and stamp it on the
    /// `SourceFile` input. `root_scope` = the file path relative to the
    /// project root (forward-slash, machine-independent), so element IDs /
    /// `content_digest` / `CommitId` are stable across checkouts. A no-op for
    /// non-directory (in-memory / `.kpar`) projects or files outside the
    /// project root — those keep the empty scope and fall back to the
    /// absolute `name` at parse time.
    fn apply_project_root_scope(&mut self, uri: &str, project_id: ProjectHandle) {
        // Prefer the owning project's directory root (multi-file projects key
        // every file on its path relative to that root). Fall back to the
        // file's own parent dir for a lone strict open with no directory
        // project — that yields the bare basename, still checkout-independent.
        let root = self
            .project_root_dir_for_handle(project_id)
            .or_else(|| crate::source::file_uri_parent(uri));
        let Some(root) = root else { return };
        if let Some(scope) = crate::source::project_relative_scope(uri, &root) {
            self.files.set_file_root_scope(&mut self.db, uri, scope);
        }
    }

    /// Assign a project to an already-registered file WITHOUT touching its
    /// source text. Used by the workspace indexer for files that are open
    /// in the editor — did_open has set the (possibly unsaved) buffer
    /// content; the indexer must preserve that content while still
    /// recording the project_id so the workspace `ProjectFileSet`
    /// includes the file. The WaterPort regression came from the indexer
    /// silently skipping pid assignment for such files.
    pub fn set_project_only(&mut self, uri: &str, project_id: ProjectHandle) {
        self.files.set_project_only(uri, project_id);
    }

    /// Mark a file's current text as an authoritative editor overlay (an open
    /// buffer). While set, `open_context` preserves the buffer text instead of
    /// loading disk. Set it under the SAME host lock as the buffer-content
    /// write so the two are established atomically against a racing indexer.
    /// See [`FileSet::set_overlay`].
    pub fn set_overlay(&mut self, uri: &str) {
        self.files.set_overlay(uri);
    }

    /// Clear a file's editor-overlay status (e.g. on `did_close`).
    pub fn clear_overlay(&mut self, uri: &str) {
        self.files.clear_overlay(uri);
    }

    /// Whether the file at `uri` is an active editor overlay.
    pub fn has_overlay(&self, uri: &str) -> bool {
        self.files.has_overlay(uri)
    }

    /// Remove a file from the database.
    ///
    /// Called when a file is closed or deleted.
    pub fn remove_file(&mut self, uri: &str) -> Option<FileId> {
        self.files.remove(uri)
    }

    /// Look up a FileId by URI.
    pub fn file_id(&self, uri: &str) -> Option<FileId> {
        self.files.lookup(uri)
    }

    /// Get the URI for a FileId.
    pub fn file_uri(&self, id: FileId) -> Option<&str> {
        self.files.uri(id)
    }

    /// Get the SourceFile salsa input for a FileId.
    pub fn source_file(&self, id: FileId) -> Option<SourceFile> {
        self.files.source_file(id)
    }

    /// Get the file set (for iteration, etc.).
    pub fn files(&self) -> &FileSet {
        &self.files
    }

    /// Set the standard library graph.
    ///
    /// Called once at startup (or when the library path changes).
    /// Resolution queries depend on this to resolve references to library types.
    pub fn set_library(&mut self, graph: ModelGraph) -> LibraryGraph {
        let data = LibraryData::new(graph);
        self.set_library_data(data)
    }

    /// Set the [`LibraryData`] for the singleton [`LibraryGraph`] input,
    /// creating the input on first call and updating it on subsequent calls.
    fn set_library_data(&mut self, data: LibraryData) -> LibraryGraph {
        use salsa::Setter;
        match self.library {
            Some(lib) => {
                lib.set_data(&mut self.db).to(data);
                lib
            }
            None => {
                let lib = LibraryGraph::new(&self.db, data);
                self.library = Some(lib);
                lib
            }
        }
    }

    /// Get the current library graph, if loaded.
    pub fn library_graph(&self) -> Option<LibraryGraph> {
        self.library
    }

    /// Find the project that owns a file by checking existing mappings
    /// first, then falling back to directory containment against loaded projects.
    ///
    /// This enables `did_open` / `did_change` handlers to associate files with
    /// projects even when workspace indexing hasn't completed yet.
    pub fn find_project_for_uri(&self, uri: &str) -> Option<ProjectHandle> {
        let dep_trace = dependency_trace_enabled();
        // Fast path: check existing id_to_project mapping
        if let Some(id) = self.files.lookup(uri) {
            if let Some(pid) = self.files.project_id(id) {
                if dep_trace {
                    tracing::info!(
                        uri,
                        project_id = pid.0,
                        "dependency trace: find_project_for_uri hit fast path"
                    );
                }
                return Some(pid);
            }
        }

        // Slow path: convert URI to path and check directory containment
        let Some(file_path) = uri_to_file_path(uri) else {
            if dep_trace {
                tracing::info!(
                    uri,
                    "dependency trace: find_project_for_uri cannot parse file URI"
                );
            }
            return None;
        };
        let canonical_file_path = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone());
        let mut best_match: Option<(ProjectHandle, usize, std::path::PathBuf, std::path::PathBuf)> =
            None;

        for project in &self.projects {
            let sysml_project::ProjectRoot::Directory(dir) = &project.root else {
                continue;
            };
            let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            let file_in_project = canonical_file_path.starts_with(&canonical_dir)
                || canonical_file_path.starts_with(dir)
                || file_path.starts_with(&canonical_dir)
                || file_path.starts_with(dir);
            if !file_in_project {
                continue;
            }

            let match_len = canonical_dir.as_os_str().len();
            match &best_match {
                Some((_, best_len, _, _)) if *best_len >= match_len => {}
                _ => {
                    best_match = Some((project.id, match_len, dir.clone(), canonical_dir));
                }
            }
        }

        let matched = best_match.as_ref().map(|(project_id, _, _, _)| *project_id);
        if dep_trace {
            match &best_match {
                Some((project_id, _, raw_root, canonical_root)) => tracing::info!(
                    uri,
                    parsed_path = %file_path.display(),
                    canonical_path = %canonical_file_path.display(),
                    project_id = project_id.0,
                    matched_root_raw = %raw_root.display(),
                    matched_root_canonical = %canonical_root.display(),
                    "dependency trace: find_project_for_uri matched project root"
                ),
                None => tracing::info!(
                    uri,
                    parsed_path = %file_path.display(),
                    canonical_path = %canonical_file_path.display(),
                    "dependency trace: find_project_for_uri found no matching project root"
                ),
            }
        }
        matched
    }

    /// Directory root of the project that owns `uri`, when it is a
    /// directory-rooted project.
    ///
    /// Resolves project-relative resource paths (e.g. `@DataSource` CSV files)
    /// for runtime compilation: the runtime `ModelCompiler` reads such files
    /// relative to a `source_dir`, which is the owning project's root. Returns
    /// `None` for in-memory / `.kpar` projects or an unknown URI.
    pub fn project_root_dir(&self, uri: &str) -> Option<std::path::PathBuf> {
        let pid = self.find_project_for_uri(uri)?;
        self.project_root_dir_for_handle(pid)
    }

    /// Directory root of a project addressed by its handle, when it is a
    /// directory-rooted project.
    ///
    /// The handle-keyed companion to [`project_root_dir`](Self::project_root_dir):
    /// used to resolve the source root for the synthetic `__workspace__` URI,
    /// which has no file-keyed project to look up via `find_project_for_uri`.
    /// Returns `None` for in-memory / `.kpar` projects or an unknown handle.
    pub fn project_root_dir_for_handle(
        &self,
        handle: ProjectHandle,
    ) -> Option<std::path::PathBuf> {
        self.projects
            .iter()
            .find(|p| p.id == handle)
            .and_then(|p| match &p.root {
                sysml_project::ProjectRoot::Directory(dir) => Some(dir.clone()),
                _ => None,
            })
    }

    /// All directory-rooted projects, as `(handle, root)` pairs.
    ///
    /// Callers that need "the loaded workspace root" (e.g. baseline git
    /// provenance) filter by handle themselves — this crate does not know
    /// which handles are user workspaces vs bundled libraries.
    pub fn project_directory_roots(&self) -> Vec<(ProjectHandle, std::path::PathBuf)> {
        self.projects
            .iter()
            .filter_map(|p| match &p.root {
                sysml_project::ProjectRoot::Directory(dir) => Some((p.id, dir.clone())),
                _ => None,
            })
            .collect()
    }

    /// Number of tracked files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get a snapshot of query execution statistics.
    pub fn query_stats(&self) -> crate::QueryStatsSnapshot {
        self.db.query_stats()
    }

    /// Reset query execution statistics to zero.
    pub fn reset_query_stats(&self) {
        self.db.reset_query_stats();
    }

    // -----------------------------------------------------------------
    // Project management
    // -----------------------------------------------------------------

    /// Return true if any loaded project's root is `path` (canonical or
    /// raw form). Used by `service.open_context` to register synthetic
    /// workspace-root projects exactly once — without this, repeated
    /// `open_context` calls would push duplicate `Project` entries with
    /// the same `ProjectHandle`, corrupting the `WorkspaceConfig`.
    pub fn has_project_at_path(&self, path: &std::path::Path) -> bool {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.projects.iter().any(|p| match &p.root {
            sysml_project::ProjectRoot::Directory(dir) => {
                let dir_canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
                dir_canonical == canonical || *dir == canonical || dir_canonical == *path
            }
            _ => false,
        })
    }

    /// Load a project into the database.
    ///
    /// Creates a `SalsaProject` input and rebuilds the `WorkspaceConfig`.
    /// Returns the project's ID.
    ///
    /// A [`ProjectHandle`] is an identity: at most one `Project` is registered
    /// per pid. Loading a project whose pid is already registered **replaces**
    /// the existing entry (and its `SalsaProject` input) rather than appending a
    /// duplicate. This is load-bearing for workspace switching: the service
    /// re-registers the synthetic workspace project (`SERVICE_WORKSPACE_PROJECT_ID`)
    /// at the new root on every `load_workspace`; without replacement, the old
    /// root lingered as a same-pid duplicate and `project_root_dir_for_handle`
    /// returned the *first* (stale) match, so a switched-to workspace resolved
    /// its relative `@DataSource` paths against the previous workspace's root.
    pub fn load_project(&mut self, project: Project) -> ProjectHandle {
        let pid = project.id;
        let name = project.info.name.clone();
        let info = Arc::new(project.info.clone());
        let meta = Arc::new(project.meta.clone().unwrap_or_else(|| ProjectMeta {
            index: Default::default(),
            created: None,
            metamodel: None,
            checksum: Default::default(),
        }));

        // Supersede any existing registration for this pid (identity, not
        // accumulation). `projects` and `salsa_projects` are NOT index-aligned
        // (stdlib pushes salsa entries without a matching `projects` entry, and
        // stdlib never uses `load_project`), so drop the salsa input by matching
        // its `project_id`.
        self.projects.retain(|p| p.id != pid);
        self.salsa_projects
            .retain(|sp| sp.project_id(&self.db) != pid.0);

        let salsa_proj = SalsaProject::new(&self.db, pid.0, name, info, meta);
        self.salsa_projects.push(salsa_proj);
        self.projects.push(project);
        self.rebuild_workspace_config();
        pid
    }

    /// Load a manifest-discovered project, *superseding* any synthetic
    /// placeholder project previously registered for the same root.
    ///
    /// P4 registers a synthetic `Project` (version `"0.0.0-synthetic"`) per
    /// workspace root in the LSP `initialize` handler so `find_project_for_uri`
    /// resolves any file-entry path before manifest discovery completes. Once
    /// the real manifest project is known it must replace that placeholder —
    /// not coexist with it — so a directory maps to exactly one project
    /// (one home). Coexistence inflated `project_count()` and risked
    /// `find_project_for_uri` returning the `0.0.0-synthetic` project over the
    /// real one. Replacement is scoped to this path (and guarded by the version
    /// sentinel), NOT folded into the general `load_project` API, which tests
    /// and stdlib loading rely on for additive multi-project registration.
    pub fn load_project_superseding_synthetic(&mut self, project: Project) -> ProjectHandle {
        if let sysml_project::ProjectRoot::Directory(root) = &project.root {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            let synthetic_idx = self.projects.iter().position(|p| {
                p.info.version == "0.0.0-synthetic"
                    && matches!(&p.root, sysml_project::ProjectRoot::Directory(d) if {
                        let dc = d.canonicalize().unwrap_or_else(|_| d.clone());
                        dc == canonical || *d == canonical || dc == *root
                    })
            });
            if let Some(idx) = synthetic_idx {
                // Remove by identity (pid): `projects` and `salsa_projects` are
                // NOT index-aligned (stdlib pushes salsa entries without a
                // matching `projects` entry), so drop the salsa input whose
                // project_id matches the synthetic's handle.
                let synthetic_pid = self.projects[idx].id.0;
                self.projects.remove(idx);
                self.salsa_projects
                    .retain(|sp| sp.project_id(&self.db) != synthetic_pid);
            }
        }
        self.load_project(project)
    }

    /// Enable the standard library by loading all 10 stdlib projects.
    ///
    /// Registers per-urn metadata as `SalsaProject` inputs (IDs `0..=9`) and,
    /// when a stdlib filesystem path is available, also loads the stdlib
    /// `.sysml` / `.kerml` source files as [`SourceFile`] inputs under
    /// [`STDLIB_BUNDLE_PROJECT_ID`]. The library [`LibraryGraph`] is then
    /// computed via the salsa-tracked [`compute_stdlib_library_data`] query
    /// and stored on the host.
    ///
    /// Source files are registered with absolute `file://` URIs so that
    /// LSP go-to-definition and find-references can navigate into stdlib
    /// files. If no stdlib path is found (returns `Ok(false)` and the
    /// library graph stays unset — caller should treat this as a graceful
    /// degradation, not an error). Idempotent: a second call when a library
    /// is already loaded is a no-op returning `Ok(true)`.
    pub fn enable_stdlib(&mut self) -> sysml_project::Result<bool> {
        self.enable_stdlib_with_path(None)
    }

    /// Like [`enable_stdlib`] but uses `lib_path` as the stdlib source
    /// directory instead of `LibraryConfig::default_library_path()`.
    ///
    /// When `lib_path` is `None`, falls back to the default path. This is the
    /// path-aware variant the LSP's `library_cache` wrapper uses to honour
    /// `sysml.library.path` overrides and `SYSML_LIBRARY_PATH` env var.
    pub fn enable_stdlib_with_path(
        &mut self,
        lib_path: Option<std::path::PathBuf>,
    ) -> sysml_project::Result<bool> {
        if self.library.is_some() {
            return Ok(true);
        }

        let registry = StdlibRegistry::new()?;
        for (i, stdlib_proj) in registry.iter().enumerate() {
            let pid = i as u32;
            let info = Arc::new(stdlib_proj.info.clone());
            let meta = Arc::new(stdlib_proj.meta.clone());
            let salsa_proj =
                SalsaProject::new(&self.db, pid, stdlib_proj.info.name.clone(), info, meta);
            self.salsa_projects.push(salsa_proj);
        }
        self.stdlib_registry = Some(registry);
        self.rebuild_workspace_config();

        let Some(lib_path) = lib_path
            .or_else(sysml_parser_trait::library::LibraryConfig::default_library_path)
        else {
            tracing::warn!(
                "standard library path not found (set SYSML_LIBRARY_PATH or install to libraries/standard/)"
            );
            return Ok(false);
        };

        let stdlib_files = collect_stdlib_files(&lib_path);
        if stdlib_files.is_empty() {
            tracing::warn!(
                lib_path = %lib_path.display(),
                "standard library directory contains no .sysml/.kerml files"
            );
            return Ok(false);
        }

        // Register every stdlib source file as a SourceFile salsa input under
        // STDLIB_BUNDLE_PROJECT_ID. URIs are absolute file:// paths so LSP
        // navigation can open the files in the editor.
        let bundle_pid = ProjectHandle(STDLIB_BUNDLE_PROJECT_ID);
        let mut source_files: Vec<SourceFile> = Vec::with_capacity(stdlib_files.len());
        for (uri, content) in &stdlib_files {
            let id = self
                .files
                .set_file_text_in_project(&mut self.db, uri, content.clone(), bundle_pid);
            // Checkout-independent identity root (ADR-009): stdlib element IDs
            // must be machine-independent because user elements resolve
            // references INTO the library, so a path-coupled stdlib id would
            // re-couple every user `content_digest` that cites a library type.
            // Relativize against the library root directly (the bundle project
            // has no directory-rooted entry in `self.projects`).
            if let Some(scope) = crate::source::project_relative_scope(uri, &lib_path) {
                self.files.set_file_root_scope(&mut self.db, uri, scope);
            }
            if let Some(sf) = self.files.source_file(id) {
                source_files.push(sf);
            }
        }
        // Sort by URI so the merge inside `compute_stdlib_library_data` is
        // deterministic across runs (matches `ensure_workspace_pfs`).
        source_files.sort_by_key(|sf| sf.name(&self.db).clone());

        let stdlib_pfs = ProjectFileSet::new(
            &self.db,
            STDLIB_BUNDLE_PROJECT_ID,
            Arc::new(source_files),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        self.project_file_sets.push(stdlib_pfs);

        let data = compute_stdlib_library_data(&self.db, stdlib_pfs);
        tracing::info!(
            file_count = stdlib_files.len(),
            elements = data.graph().element_count(),
            "standard library loaded via salsa"
        );
        self.set_library_data(data);
        Ok(true)
    }

    /// Get the workspace configuration, if any projects are loaded.
    pub fn workspace_config(&self) -> Option<WorkspaceConfig> {
        self.workspace_config
    }

    /// Add a project file set to the workspace.
    pub fn add_project_file_set(&mut self, pfs: crate::ProjectFileSet) {
        self.project_file_sets.push(pfs);
    }

    /// Get the project file set for a given project ID, if any.
    pub fn project_file_set(&self, project_id: ProjectHandle) -> Option<crate::ProjectFileSet> {
        self.project_file_sets
            .iter()
            .find(|pfs| pfs.pid(&self.db) == project_id)
            .copied()
    }

    /// Read a source file from a project and add it to the salsa database.
    ///
    /// Returns `None` if the project doesn't exist or the file can't be read.
    pub fn ensure_file_loaded(
        &mut self,
        project_id: ProjectHandle,
        relative_path: &str,
    ) -> Option<FileId> {
        // Check if already loaded (by constructing a URI)
        let uri = format!("project://{}/{}", project_id.0, relative_path);
        if let Some(id) = self.files.lookup(&uri) {
            return Some(id);
        }

        // Find the project and read the source
        let project = self.projects.iter().find(|p| p.id == project_id)?;
        let text = project.read_source(relative_path).ok()?;

        let id = self
            .files
            .set_file_text_in_project(&mut self.db, &uri, text, project_id);
        Some(id)
    }

    /// Number of loaded projects (excluding stdlib).
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// Total number of salsa projects (including stdlib).
    pub fn salsa_project_count(&self) -> usize {
        self.salsa_projects.len()
    }

    /// Rebuild the `WorkspaceConfig` singleton from current salsa projects.
    fn rebuild_workspace_config(&mut self) {
        let projects = Arc::new(self.salsa_projects.clone());
        let include_stdlib = self.stdlib_registry.is_some();

        match self.workspace_config {
            Some(config) => {
                use salsa::Setter;
                config.set_projects(&mut self.db).to(projects);
                config.set_include_stdlib(&mut self.db).to(include_stdlib);
            }
            None => {
                let config = WorkspaceConfig::new(&self.db, projects, include_stdlib);
                self.workspace_config = Some(config);
            }
        }
    }
}

/// Recursively collect every `.sysml` / `.kerml` file under `root`,
/// returning `(file://<absolute-path>, contents)` pairs in URI-sorted order.
///
/// Used by [`AnalysisHost::enable_stdlib`] to register stdlib source files as
/// salsa inputs. Files that cannot be read are skipped with a warning trace
/// (stdlib loading is best-effort; downstream features degrade gracefully if
/// stdlib is partial).
fn collect_stdlib_files(root: &std::path::Path) -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(it) => it,
            Err(e) => {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %e,
                    "skipping stdlib directory: read_dir failed"
                );
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
                continue;
            }
            if !path
                .extension()
                .is_some_and(|ext| ext == "sysml" || ext == "kerml")
            {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let abs = path.canonicalize().unwrap_or(path.clone());
                    let uri = format!("file://{}", abs.display());
                    out.push((uri, content));
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "skipping stdlib file: read_to_string failed"
                    );
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn uri_to_file_path(uri: &str) -> Option<std::path::PathBuf> {
    let stripped = uri.strip_prefix("file://")?;
    if stripped.is_empty() {
        return None;
    }

    #[cfg(unix)]
    {
        // Some clients use file://localhost/path; normalize that to /path.
        if let Some(localhost_path) = stripped.strip_prefix("localhost/") {
            return Some(std::path::PathBuf::from(format!("/{localhost_path}")));
        }
    }

    Some(std::path::PathBuf::from(stripped))
}

fn parse_env_bool(var: &str) -> bool {
    match std::env::var(var) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn dependency_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        parse_env_bool("SYSML_DEPENDENCY_TRACE") || parse_env_bool("SYSML_LSP_DEPENDENCY_TRACE")
    })
}

impl Default for AnalysisHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable snapshot of the database for serving read requests.
///
/// This can be sent to another thread. All query methods return cached
/// results when possible, only recomputing what's needed.
///
/// **Cancellation**: If the host receives a mutation while this snapshot
/// is alive, active queries will panic with `salsa::Cancelled`. Use
/// `salsa::Cancelled::catch()` to handle this gracefully.
///
/// **Lifetime**: Drop the `Analysis` as soon as you're done with it.
/// The host's mutations block until all `Analysis` clones are dropped.
#[derive(Clone)]
pub struct Analysis {
    db: RootDatabase,
    library: Option<LibraryGraph>,
    workspace_config: Option<WorkspaceConfig>,
    project_file_sets: Arc<Vec<crate::ProjectFileSet>>,
}

impl std::fmt::Debug for Analysis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analysis")
            .field("library", &self.library.map(|_| "<loaded>"))
            .field("workspace_config", &self.workspace_config.map(|_| "<set>"))
            .field("project_file_sets", &self.project_file_sets.len())
            .finish()
    }
}

impl Analysis {
    /// Get a reference to the underlying database.
    pub fn db(&self) -> &RootDatabase {
        &self.db
    }

    /// Get the library graph, if loaded.
    ///
    /// Returns `None` if no standard library has been set on the host.
    pub fn library_graph(&self) -> Option<LibraryGraph> {
        self.library
    }

    /// Get the project file set for a given project ID, if any.
    pub fn project_file_set(&self, project_id: ProjectHandle) -> Option<crate::ProjectFileSet> {
        self.project_file_sets
            .iter()
            .find(|pfs| pfs.pid(&self.db) == project_id)
            .copied()
    }

    /// Read the source text of a file.
    pub fn file_text(&self, source_file: SourceFile) -> &str {
        source_file.text(&self.db)
    }

    /// Parse a file and return the parsed result (model graph + diagnostics).
    ///
    /// The result is an Arc-wrapped struct. Call `.graph()`, `.diagnostics()`,
    /// `.element_count()`, `.has_errors()` to access the data.
    pub fn parse_file(&self, source_file: SourceFile) -> ParseResult {
        parse::parse_file(&self.db, source_file)
    }

    /// Get the tree-sitter tree for a file, memoized by salsa.
    pub fn parse_tree(&self, source_file: SourceFile) -> Option<crate::parse::CachedTree> {
        parse::parse_tree(&self.db, source_file)
    }

    /// Get the outline for a file (for document symbols).
    pub fn outline(&self, source_file: SourceFile) -> crate::parse::Outline {
        parse::file_outline(&self.db, source_file)
    }

    /// Get the document symbol tree for a file (graph-based, richer than outline).
    pub fn document_symbols(&self, source_file: SourceFile) -> crate::symbols::DocumentSymbolTree {
        crate::symbols::file_document_symbols(&self.db, source_file)
    }

    /// Get semantic tokens for a file (model + reference + CST tokens).
    ///
    /// Reference tokens are resolution-backed, so this forwards the host's
    /// optional [`LibraryGraph`] + the [`ProjectFileSet`] selected by
    /// `project_id` (same "best-available" sourcing as `resolve_file_best` /
    /// `validate_file_best`).
    pub fn semantic_tokens(
        &self,
        source_file: SourceFile,
        project_id: Option<ProjectHandle>,
    ) -> crate::tokens::FileTokens {
        let library = self.library_graph();
        let project_files = project_id.and_then(|pid| self.project_file_set(pid));
        crate::tokens::file_semantic_tokens(&self.db, source_file, project_files, library)
    }

    /// Get the position map for a file (byte offset -> ElementId).
    pub fn position_map(&self, source_file: SourceFile) -> PositionMap {
        parse::file_position_map(&self.db, source_file)
    }

    /// Get the public exports for a file (top-level named elements).
    pub fn file_exports(&self, source_file: SourceFile) -> crate::exports::FileExports {
        crate::exports::file_exports(&self.db, source_file)
    }

    // -----------------------------------------------------------------
    // Canonical "best-available" accessors
    //
    // Every IDE feature (goto, hover, diagnostics, …) needs a resolved/
    // elaborated/validated model and picks one of four dispatch shapes
    // based on what's loaded into the host. The branch lives in the
    // salsa-tracked dispatcher inside `resolution` / `analysis`; these
    // accessors just extract the host's optional `ProjectFileSet` +
    // `LibraryGraph` and forward to it.
    //
    // Pass `project_id = guard.files().project_id(file_id)` (the same
    // `Option<ProjectHandle>` every caller already extracts).
    //
    // P3 of the resolution tier collapse deleted the twelve named
    // `resolve_file*` / `validate_file*` / `elaborate_file*` variants
    // these accessors used to wrap; the dispatchers now own the work
    // -----------------------------------------------------------------

    /// Resolve a file using the strongest context available on the host.
    ///
    /// Forwards to [`resolution::resolve_file_best`] with the host's
    /// optional [`LibraryGraph`] and the [`ProjectFileSet`] selected by
    /// `project_id`. See that function for the four-arm dispatch shape.
    pub fn resolve_file_best(
        &self,
        source_file: SourceFile,
        project_id: Option<ProjectHandle>,
    ) -> ResolvedModel {
        let library = self.library_graph();
        let project_files = project_id.and_then(|pid| self.project_file_set(pid));
        resolution::resolve_file_best(&self.db, source_file, project_files, library)
    }

    /// Validate a file using the strongest context available on the host.
    ///
    /// Forwards to [`analysis::validate_file_best`] with the host's
    /// optional [`LibraryGraph`] and the [`ProjectFileSet`] selected by
    /// `project_id`. Runs property + semantic + structural + import-health
    /// passes; see that function for the four-arm dispatch shape.
    pub fn validate_file_best(
        &self,
        source_file: SourceFile,
        project_id: Option<ProjectHandle>,
    ) -> ValidatedModel {
        let library = self.library_graph();
        let project_files = project_id.and_then(|pid| self.project_file_set(pid));
        analysis::validate_file_best(&self.db, source_file, project_files, library)
    }

    /// Elaborate a file using the strongest context available on the host.
    ///
    /// Forwards to [`analysis::elaborate_file_best`] with the host's
    /// optional [`LibraryGraph`] and the [`ProjectFileSet`] selected by
    /// `project_id`. See that function for the four-arm dispatch shape
    /// (the workspace+library arm uses `elaborate_with_library` for IG-1).
    pub fn elaborate_file_best(
        &self,
        source_file: SourceFile,
        project_id: Option<ProjectHandle>,
    ) -> ElaboratedModel {
        let library = self.library_graph();
        let project_files = project_id.and_then(|pid| self.project_file_set(pid));
        analysis::elaborate_file_best(&self.db, source_file, project_files, library)
    }

    /// Elaborate the whole workspace into a single `ModelGraph`.
    ///
    /// Mirrors what `sysml-service` historically built as `__workspace__`:
    /// merge every file in the project file set, resolve cross-file
    /// references, and run elaboration once. Memoized — only re-runs when a
    /// file changes.
    pub fn elaborate_workspace(
        &self,
        project_files: crate::ProjectFileSet,
    ) -> ElaboratedWorkspace {
        analysis::elaborate_workspace(&self.db, project_files)
    }

    /// Elaborate the whole workspace with the standard library merged in.
    pub fn elaborate_workspace_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
    ) -> ElaboratedWorkspace {
        analysis::elaborate_workspace_with_library(&self.db, project_files, library)
    }

    // -----------------------------------------------------------------
    // Eval-context queries (S2.T17 — tracked, replaces per-call walks)
    // -----------------------------------------------------------------

    /// Build a salsa-cached `EvalContext` for a single-file model graph.
    pub fn file_eval_context(
        &self,
        source_file: SourceFile,
    ) -> crate::eval_context::CachedEvalContext {
        crate::eval_context::file_eval_context(&self.db, source_file)
    }

    /// Build a salsa-cached `EvalContext` for the workspace-merged graph
    /// (no library overlay).
    pub fn workspace_eval_context(
        &self,
        project_files: crate::ProjectFileSet,
    ) -> crate::eval_context::CachedEvalContext {
        crate::eval_context::workspace_eval_context(&self.db, project_files)
    }

    /// Build a salsa-cached `EvalContext` for the workspace-merged graph
    /// with the standard library merged in.
    pub fn workspace_eval_context_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
    ) -> crate::eval_context::CachedEvalContext {
        crate::eval_context::workspace_eval_context_with_library(
            &self.db,
            project_files,
            library,
        )
    }

    // -----------------------------------------------------------------
    // Element-index queries (S2.T17 (2/N) — tracked, replaces O(n) walks)
    // -----------------------------------------------------------------

    /// Build a salsa-cached name index for a single-file model graph.
    pub fn file_name_index(
        &self,
        source_file: SourceFile,
    ) -> crate::element_index::CachedNameIndex {
        crate::element_index::file_name_index(&self.db, source_file)
    }

    /// Build a salsa-cached kind index for a single-file model graph.
    pub fn file_kind_index(
        &self,
        source_file: SourceFile,
    ) -> crate::element_index::CachedKindIndex {
        crate::element_index::file_kind_index(&self.db, source_file)
    }

    /// Build a salsa-cached name index for the workspace-merged graph.
    pub fn workspace_name_index(
        &self,
        project_files: crate::ProjectFileSet,
    ) -> crate::element_index::CachedNameIndex {
        crate::element_index::workspace_name_index(&self.db, project_files)
    }

    /// Build a salsa-cached kind index for the workspace-merged graph.
    pub fn workspace_kind_index(
        &self,
        project_files: crate::ProjectFileSet,
    ) -> crate::element_index::CachedKindIndex {
        crate::element_index::workspace_kind_index(&self.db, project_files)
    }

    /// Build a salsa-cached name index for the workspace-merged graph
    /// with the standard library merged in.
    pub fn workspace_name_index_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
    ) -> crate::element_index::CachedNameIndex {
        crate::element_index::workspace_name_index_with_library(
            &self.db,
            project_files,
            library,
        )
    }

    /// Build a salsa-cached kind index for the workspace-merged graph
    /// with the standard library merged in.
    pub fn workspace_kind_index_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
    ) -> crate::element_index::CachedKindIndex {
        crate::element_index::workspace_kind_index_with_library(
            &self.db,
            project_files,
            library,
        )
    }

    // -----------------------------------------------------------------
    // Reverse-reference index (Phase 2 of resolution-features-audit) —
    // ElementId -> Vec<RefSite> across the workspace-merged graph.
    // Same pattern as workspace_name_index above.
    // -----------------------------------------------------------------

    /// Build the salsa-cached reverse-reference index for the workspace-
    /// merged graph (no library overlay).
    pub fn workspace_ref_index(
        &self,
        project_files: crate::ProjectFileSet,
    ) -> crate::ref_index::CachedRefIndex {
        crate::ref_index::workspace_ref_index(&self.db, project_files)
    }

    /// Build the salsa-cached reverse-reference index for the workspace-
    /// merged graph with the standard library overlay merged in.
    pub fn workspace_ref_index_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
    ) -> crate::ref_index::CachedRefIndex {
        crate::ref_index::workspace_ref_index_with_library(
            &self.db,
            project_files,
            library,
        )
    }

    /// Build the reverse-reference index using the strongest context
    /// available on the host. Returns `None` when no `ProjectFileSet` is
    /// loaded for `project_id` — references work only with a workspace.
    pub fn ref_index_best(
        &self,
        project_id: Option<ProjectHandle>,
    ) -> Option<crate::ref_index::CachedRefIndex> {
        let pfs = self.project_file_set(project_id?)?;
        let library = self.library_graph();
        Some(match library {
            Some(lib) => self.workspace_ref_index_with_library(pfs, lib),
            None => self.workspace_ref_index(pfs),
        })
    }

    /// Build the workspace name-index using the strongest context
    /// available. Returns `None` when no `ProjectFileSet` is loaded.
    /// Used by code_actions auto-import to enumerate cross-file
    /// user-package definitions matching an unresolved name.
    pub fn workspace_name_index_best(
        &self,
        project_id: Option<ProjectHandle>,
    ) -> Option<crate::element_index::CachedNameIndex> {
        let pfs = self.project_file_set(project_id?)?;
        let library = self.library_graph();
        Some(match library {
            Some(lib) => self.workspace_name_index_with_library(pfs, lib),
            None => self.workspace_name_index(pfs),
        })
    }

    /// Elaborate the workspace-merged graph using the strongest context
    /// available. Returns `None` when no `ProjectFileSet` is loaded.
    /// Needed alongside [`workspace_name_index_best`] for qname lookups
    /// off the indexed element ids.
    pub fn elaborate_workspace_best(
        &self,
        project_id: Option<ProjectHandle>,
    ) -> Option<ElaboratedWorkspace> {
        let pfs = self.project_file_set(project_id?)?;
        let library = self.library_graph();
        Some(match library {
            Some(lib) => self.elaborate_workspace_with_library(pfs, lib),
            None => self.elaborate_workspace(pfs),
        })
    }

    /// Precompile filter ExprIRs using the strongest context available
    /// on the host. Forwards to [`view_filter_exprs::view_filter_exprs_best`]
    /// with the host's optional [`LibraryGraph`] and the [`ProjectFileSet`]
    /// selected by `project_id`. See that function for the three-arm
    /// dispatch shape (mirrors `resolve_file_best` minus the
    /// single-file-with-library arm).
    pub fn view_filter_exprs_best(
        &self,
        source_file: SourceFile,
        project_id: Option<ProjectHandle>,
    ) -> crate::view_filter_exprs::CachedViewFilterExprs {
        let library = self.library_graph();
        let project_files = project_id.and_then(|pid| self.project_file_set(pid));
        crate::view_filter_exprs::view_filter_exprs_best(
            &self.db,
            source_file,
            project_files,
            library,
        )
    }

    // -----------------------------------------------------------------
    // Descendants queries (S2.T17 (4/N) — tracked, replaces O(subtree)
    // walks)
    // -----------------------------------------------------------------

    /// Build a salsa-cached descendants list for a single-file model
    /// graph.
    pub fn file_descendants(
        &self,
        sf: SourceFile,
        id: sysml_core::ElementId,
    ) -> crate::descendants_query::CachedDescendants {
        crate::descendants_query::file_descendants(&self.db, sf, id)
    }

    /// Build a salsa-cached descendants list for the workspace-merged
    /// graph.
    pub fn workspace_descendants(
        &self,
        project_files: crate::ProjectFileSet,
        id: sysml_core::ElementId,
    ) -> crate::descendants_query::CachedDescendants {
        crate::descendants_query::workspace_descendants(&self.db, project_files, id)
    }

    /// Build a salsa-cached descendants list for the workspace-merged
    /// graph with the standard library merged in.
    pub fn workspace_descendants_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
        id: sysml_core::ElementId,
    ) -> crate::descendants_query::CachedDescendants {
        crate::descendants_query::workspace_descendants_with_library(
            &self.db,
            project_files,
            library,
            id,
        )
    }

    // -----------------------------------------------------------------
    // Trace-matrix queries (S2.T17 (5/N) — tracked, replaces O(rels)
    // walks)
    // -----------------------------------------------------------------

    /// Build a salsa-cached trace matrix for a single-file model graph.
    pub fn file_trace_matrix(
        &self,
        sf: SourceFile,
        source_kind: sysml_core::ElementKind,
        rel_kind: sysml_core::RelationshipKind,
        target_kind: sysml_core::ElementKind,
    ) -> crate::trace_matrix_query::CachedTraceMatrix {
        crate::trace_matrix_query::file_trace_matrix(
            &self.db,
            sf,
            source_kind,
            rel_kind,
            target_kind,
        )
    }

    /// Build a salsa-cached trace matrix for the workspace-merged graph.
    pub fn workspace_trace_matrix(
        &self,
        project_files: crate::ProjectFileSet,
        source_kind: sysml_core::ElementKind,
        rel_kind: sysml_core::RelationshipKind,
        target_kind: sysml_core::ElementKind,
    ) -> crate::trace_matrix_query::CachedTraceMatrix {
        crate::trace_matrix_query::workspace_trace_matrix(
            &self.db,
            project_files,
            source_kind,
            rel_kind,
            target_kind,
        )
    }

    /// Build a salsa-cached trace matrix for the workspace-merged graph
    /// with the standard library merged in.
    pub fn workspace_trace_matrix_with_library(
        &self,
        project_files: crate::ProjectFileSet,
        library: LibraryGraph,
        source_kind: sysml_core::ElementKind,
        rel_kind: sysml_core::RelationshipKind,
        target_kind: sysml_core::ElementKind,
    ) -> crate::trace_matrix_query::CachedTraceMatrix {
        crate::trace_matrix_query::workspace_trace_matrix_with_library(
            &self.db,
            project_files,
            library,
            source_kind,
            rel_kind,
            target_kind,
        )
    }

    // -----------------------------------------------------------------
    // Project queries
    // -----------------------------------------------------------------

    /// Get the workspace configuration, if any.
    pub fn workspace_config(&self) -> Option<WorkspaceConfig> {
        self.workspace_config
    }

    /// Build the global symbol index from all workspace projects.
    ///
    /// Returns `None` if no workspace config is set.
    pub fn symbol_index(&self) -> Option<GlobalSymbolIndex> {
        let config = self.workspace_config?;
        Some(crate::symbol_index_query::symbol_index(&self.db, config))
    }

    /// Look up a symbol across all workspace projects.
    ///
    /// Returns the project ID and source file if found.
    pub fn resolve_symbol(&self, name: &str) -> Option<(ProjectHandle, String)> {
        let config = self.workspace_config?;
        crate::symbol_index_query::resolve_symbol(&self.db, config, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectFileSet;
    use sysml_span::Severity;

    /// Minimal SysML source that defines a package with items.
    const DEFINITIONS_SRC: &str = r#"
package Definitions {
    item def CoffeeBeans;
    item def Water;
    part def Grinder;
    part def CoffeeMachine {
        part grinder : Grinder;
    }
}
"#;

    /// Source file that imports from Definitions.
    const TYPING_SRC: &str = r#"
package TypingAndSpecialization {
    import Definitions::*;
    part def EspressoMachine :> CoffeeMachine {
        part fineGrinder : Grinder;
    }
}
"#;

    fn make_project(host: &mut AnalysisHost, id: u32, name: &str) {
        let project = sysml_project::Project {
            id: ProjectHandle(id),
            info: sysml_project::ProjectInfo {
                name: name.to_string(),
                description: None,
                version: "1.0.0".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::InMemory,
        };
        host.load_project(project);
    }

    /// Count resolution errors (E200) in diagnostics.
    fn count_resolution_errors(diagnostics: &[sysml_span::Diagnostic]) -> usize {
        diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error && d.code.as_deref() == Some("E200"))
            .count()
    }

    #[test]
    fn single_file_resolution_fails_cross_file_imports() {
        let mut host = AnalysisHost::new();
        make_project(&mut host, 10, "test-project");

        // Add typing file WITHOUT project association (simulates did_open before indexing)
        host.set_file_content("file:///typing.sysml", TYPING_SRC.to_string());

        let analysis = host.analysis();
        let sf = host
            .source_file(host.file_id("file:///typing.sysml").unwrap())
            .unwrap();

        // Single-file resolution: imports should FAIL (Definitions not in scope)
        let resolved = analysis.resolve_file_best(sf, None);
        let e200_count = count_resolution_errors(resolved.diagnostics());
        assert!(
            e200_count > 0,
            "single-file resolution should produce E200 errors for cross-file imports, got 0"
        );
    }

    #[test]
    fn workspace_resolution_resolves_cross_file_imports() {
        let mut host = AnalysisHost::new();
        make_project(&mut host, 10, "test-project");

        // Add BOTH files with project association (simulates workspace indexing)
        let pid = ProjectHandle(10);
        host.set_file_content_in_project(
            "file:///definitions.sysml",
            DEFINITIONS_SRC.to_string(),
            pid,
        );
        host.set_file_content_in_project("file:///typing.sysml", TYPING_SRC.to_string(), pid);

        // Create ProjectFileSet
        let sf_def = host
            .source_file(host.file_id("file:///definitions.sysml").unwrap())
            .unwrap();
        let sf_typ = host
            .source_file(host.file_id("file:///typing.sysml").unwrap())
            .unwrap();
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(vec![sf_def, sf_typ]),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);

        let analysis = host.analysis();

        // Workspace resolution: imports should SUCCEED
        let resolved = analysis.resolve_file_best(sf_typ, Some(pid));
        let _ = pfs; // assertion holds via Analysis::resolve_file_best lookup
        let e200_count = count_resolution_errors(resolved.diagnostics());
        assert_eq!(
            e200_count,
            0,
            "workspace resolution should resolve cross-file imports, but got {} E200 errors: {:?}",
            e200_count,
            resolved
                .diagnostics()
                .iter()
                .filter(|d| d.code.as_deref() == Some("E200"))
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn did_open_before_indexing_then_rediagnose_works() {
        // This simulates the exact LSP race condition:
        // 1. did_open fires → set_file_content (no project)
        // 2. diagnostics run → single-file → errors
        // 3. workspace indexing → set_file_content_in_project + ProjectFileSet
        // 4. re-diagnose → workspace resolution → no errors

        let mut host = AnalysisHost::new();
        make_project(&mut host, 10, "test-project");
        let pid = ProjectHandle(10);

        // Step 1: did_open (no project association)
        host.set_file_content("file:///typing.sysml", TYPING_SRC.to_string());

        // Step 2: diagnostics with single-file resolution → errors
        {
            let analysis = host.analysis();
            let sf = host
                .source_file(host.file_id("file:///typing.sysml").unwrap())
                .unwrap();
            let resolved = analysis.resolve_file_best(sf, None);
            assert!(
                count_resolution_errors(resolved.diagnostics()) > 0,
                "before indexing: should have E200 errors"
            );
        }

        // Step 3: workspace indexing adds both files with project association
        host.set_file_content_in_project(
            "file:///definitions.sysml",
            DEFINITIONS_SRC.to_string(),
            pid,
        );
        // Re-set typing file WITH project (workspace indexing reads from disk)
        host.set_file_content_in_project("file:///typing.sysml", TYPING_SRC.to_string(), pid);

        // Create ProjectFileSet
        let sf_def = host
            .source_file(host.file_id("file:///definitions.sysml").unwrap())
            .unwrap();
        let sf_typ = host
            .source_file(host.file_id("file:///typing.sysml").unwrap())
            .unwrap();
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(vec![sf_def, sf_typ]),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);

        // Step 4: re-diagnose with workspace resolution → no errors
        {
            let file_id = host.file_id("file:///typing.sysml").unwrap();
            let project_id = host.files().project_id(file_id);
            assert_eq!(
                project_id,
                Some(pid),
                "project_id should be set after indexing"
            );

            let pfs = host.project_file_set(pid);
            assert!(pfs.is_some(), "ProjectFileSet should exist after indexing");

            let analysis = host.analysis();
            let resolved = analysis.resolve_file_best(sf_typ, Some(pid));
            let _ = pfs; // assertion holds via Analysis::resolve_file_best lookup
            let e200_count = count_resolution_errors(resolved.diagnostics());
            assert_eq!(
                e200_count, 0,
                "after indexing + rediagnose: workspace resolution should have 0 E200 errors, got {}: {:?}",
                e200_count,
                resolved.diagnostics().iter()
                    .filter(|d| d.code.as_deref() == Some("E200"))
                    .map(|d| &d.message)
                    .collect::<Vec<_>>()
            );
        }
    }

    /// Count diagnostics with a specific code.
    fn count_diags_with_code(diagnostics: &[sysml_span::Diagnostic], code: &str) -> usize {
        diagnostics
            .iter()
            .filter(|d| d.code.as_deref() == Some(code))
            .count()
    }

    #[test]
    fn workspace_validation_no_im001_for_cross_file_import() {
        // IM001 ("unresolved in current workspace context") should NOT fire
        // when the namespace exists in another workspace file.
        let mut host = AnalysisHost::new();
        make_project(&mut host, 10, "test-project");
        let pid = ProjectHandle(10);

        host.set_file_content_in_project(
            "file:///definitions.sysml",
            DEFINITIONS_SRC.to_string(),
            pid,
        );
        host.set_file_content_in_project("file:///typing.sysml", TYPING_SRC.to_string(), pid);

        let sf_def = host
            .source_file(host.file_id("file:///definitions.sysml").unwrap())
            .unwrap();
        let sf_typ = host
            .source_file(host.file_id("file:///typing.sysml").unwrap())
            .unwrap();
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(vec![sf_def, sf_typ]),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);

        let analysis = host.analysis();

        // Workspace validation should NOT produce IM001 for "Definitions"
        let _ = pfs;
        let validated = analysis.validate_file_best(sf_typ, Some(pid));
        let im001_count = count_diags_with_code(validated.diagnostics(), "IM001");
        assert_eq!(
            im001_count,
            0,
            "workspace validation should not produce IM001 for cross-file namespace, got: {:?}",
            validated
                .diagnostics()
                .iter()
                .filter(|d| d.code.as_deref() == Some("IM001"))
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn workspace_validation_no_im005_for_cross_file_wildcard() {
        // IM005 ("unused import") should NOT fire when the imported
        // namespace has members in another workspace file.
        let mut host = AnalysisHost::new();
        make_project(&mut host, 10, "test-project");
        let pid = ProjectHandle(10);

        host.set_file_content_in_project(
            "file:///definitions.sysml",
            DEFINITIONS_SRC.to_string(),
            pid,
        );
        host.set_file_content_in_project("file:///typing.sysml", TYPING_SRC.to_string(), pid);

        let sf_def = host
            .source_file(host.file_id("file:///definitions.sysml").unwrap())
            .unwrap();
        let sf_typ = host
            .source_file(host.file_id("file:///typing.sysml").unwrap())
            .unwrap();
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(vec![sf_def, sf_typ]),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);

        let analysis = host.analysis();

        // Workspace validation should NOT produce IM005 for "Definitions"
        // because the Definitions package HAS members (CoffeeBeans, Water, etc.)
        let _ = pfs;
        let validated = analysis.validate_file_best(sf_typ, Some(pid));
        let im005_count = count_diags_with_code(validated.diagnostics(), "IM005");
        assert_eq!(
            im005_count,
            0,
            "workspace validation should not produce IM005 for namespace with members, got: {:?}",
            validated
                .diagnostics()
                .iter()
                .filter(|d| d.code.as_deref() == Some("IM005"))
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn workspace_validation_im005_for_truly_empty_namespace() {
        // IM005 SHOULD still fire when importing an empty namespace,
        // even in workspace mode.
        let mut host = AnalysisHost::new();
        make_project(&mut host, 10, "test-project");
        let pid = ProjectHandle(10);

        // Empty package with no members
        let empty_src = "package EmptyPkg {}";
        // File that imports the empty package
        let importer_src = r#"
package Importer {
    import EmptyPkg::*;
}
"#;

        host.set_file_content_in_project("file:///empty.sysml", empty_src.to_string(), pid);
        host.set_file_content_in_project("file:///importer.sysml", importer_src.to_string(), pid);

        let sf_empty = host
            .source_file(host.file_id("file:///empty.sysml").unwrap())
            .unwrap();
        let sf_imp = host
            .source_file(host.file_id("file:///importer.sysml").unwrap())
            .unwrap();
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(vec![sf_empty, sf_imp]),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);

        let analysis = host.analysis();
        let _ = pfs;
        let validated = analysis.validate_file_best(sf_imp, Some(pid));
        let im005_count = count_diags_with_code(validated.diagnostics(), "IM005");
        assert!(
            im005_count > 0,
            "workspace validation should produce IM005 for truly empty namespace"
        );
        // Verify the message says "unused import"
        let im005 = validated
            .diagnostics()
            .iter()
            .find(|d| d.code.as_deref() == Some("IM005"))
            .unwrap();
        assert!(
            im005.message.contains("unused import"),
            "IM005 should say 'unused import', got: {}",
            im005.message
        );
    }

    #[test]
    fn find_project_for_uri_directory_containment() {
        let mut host = AnalysisHost::new();

        // Load a project with a directory root
        let project = sysml_project::Project {
            id: ProjectHandle(10),
            info: sysml_project::ProjectInfo {
                name: "coffee-machine".to_string(),
                description: None,
                version: "1.0.0".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::Directory("/home/user/coffee-machine".into()),
        };
        host.load_project(project);

        // File inside project root → should find project
        let pid = host.find_project_for_uri("file:///home/user/coffee-machine/typing.sysml");
        assert_eq!(pid, Some(ProjectHandle(10)));

        // File outside project root → should return None
        let pid = host.find_project_for_uri("file:///home/user/other-project/foo.sysml");
        assert_eq!(pid, None);
    }

    #[cfg(unix)]
    #[test]
    fn find_project_for_uri_directory_containment_handles_symlink_file_uri() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temp dir should be created");
        let canonical_root = temp.path().join("canonical-root");
        std::fs::create_dir_all(&canonical_root).expect("canonical project root should be created");
        let model_path = canonical_root.join("root.sysml");
        std::fs::write(&model_path, "package CanonicalRoot {}\n")
            .expect("model file should be written");

        let alias_root = temp.path().join("alias-root");
        symlink(&canonical_root, &alias_root).expect("symlink alias should be created");
        let alias_model_path = alias_root.join("root.sysml");

        let mut host = AnalysisHost::new();
        let project = sysml_project::Project {
            id: ProjectHandle(22),
            info: sysml_project::ProjectInfo {
                name: "canonical-root".to_string(),
                description: None,
                version: "1.0.0".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::Directory(canonical_root.clone()),
        };
        host.load_project(project);

        let alias_uri = format!("file://{}", alias_model_path.display());
        let pid = host.find_project_for_uri(&alias_uri);
        assert_eq!(
            pid,
            Some(ProjectHandle(22)),
            "symlink URI should resolve to canonical project root"
        );
    }

    /// A `ProjectHandle` is an identity: re-loading a project at the same pid
    /// but a new root must *replace* the prior registration, not accumulate a
    /// same-pid duplicate. Regression guard for the workspace-switch stale
    /// `source_dir` bug — `project_root_dir_for_handle` returned the first
    /// (stale) match when duplicates piled up, so a switched-to workspace
    /// resolved relative `@DataSource` paths against the previous root.
    #[test]
    fn load_project_supersedes_same_pid_root() {
        let mut host = AnalysisHost::new();
        let pid = ProjectHandle(100);

        let workspace_a = sysml_project::Project {
            id: pid,
            info: sysml_project::ProjectInfo {
                name: "workspace-a".to_string(),
                description: None,
                version: "0.0.0-synthetic".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::Directory("/home/user/workspace-a".into()),
        };
        host.load_project(workspace_a);
        assert_eq!(host.project_count(), 1);
        assert_eq!(
            host.project_root_dir_for_handle(pid),
            Some(std::path::PathBuf::from("/home/user/workspace-a"))
        );

        // Switch to workspace B at the SAME pid.
        let workspace_b = sysml_project::Project {
            id: pid,
            info: sysml_project::ProjectInfo {
                name: "workspace-b".to_string(),
                description: None,
                version: "0.0.0-synthetic".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::Directory("/home/user/workspace-b".into()),
        };
        host.load_project(workspace_b);

        // Exactly one project at this pid, resolving to B — not two, not A.
        assert_eq!(
            host.project_count(),
            1,
            "same-pid re-load must supersede, not accumulate a duplicate"
        );
        assert_eq!(
            host.project_root_dir_for_handle(pid),
            Some(std::path::PathBuf::from("/home/user/workspace-b")),
            "handle must resolve to the current (B) root, not the stale (A) one"
        );
    }
}
