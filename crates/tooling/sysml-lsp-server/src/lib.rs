//! # sysml-lsp-server
//!
//! LSP server implementation for SysML v2.
//!
//! This crate provides a Language Server Protocol server that uses:
//! - sysml-ts for primary parsing (tree-sitter, error-tolerant, incremental)
//! - sysml-text-pest for strict validation mode
//! - sysml-parser-trait for library loading and parser traits
//! - sysml-lsp for protocol types
//!
//! ## Architecture
//!
//! The LSP uses tree-sitter as the primary parser because it:
//! - Is error-tolerant (produces partial AST on syntax errors)
//! - Supports incremental parsing (fast on edits)
//! - Enables graceful degradation (semantic features work on valid regions)
//!
//! Pest is kept for strict validation ("compile" mode) to catch all errors.
//!
//! ## Resolution Strategy
//!
//! Resolution uses a tiered approach to avoid blocking the UI:
//!
//! - **T1 (Syntax)**: Runs synchronously on every edit. Provides highlighting,
//!   outline, and syntax errors in < 50ms.
//!
//! - **T2 (Local)**: Runs after a 200ms debounce. Provides same-file go-to-def
//!   and completion.
//!
//! - **T3 (Full)**: Runs in background when idle. Provides cross-file navigation,
//!   library types, and full validation.
//!
//! ## Library Caching
//!
//! The standard library is cached to `~/.cache/sysml-rs/` after first parse,
//! reducing startup time from > 5s to < 500ms.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

// LSP protocol types (formerly sysml-lsp crate)
pub mod lsp_types;

mod advanced_features;
mod aggregation;
mod background;
mod code_actions;
mod code_lens;
mod command_dispatch;
mod commands;
mod completion;
mod diagnostics;
mod diagram;
mod evaluation;
mod formatting;
mod hover;
mod inlay_hints;
pub(crate) mod kinds;
mod library_manager;
mod navigation;
mod pending_requests;
mod ranges;
mod rename;
mod semantic_tokens;
mod service_edits;
mod symbols;
mod syntax_context;
mod telemetry_control;
mod telemetry_events;
mod type_hierarchy;
pub(crate) mod types;
pub(crate) mod utils;
mod ux_messages;
mod workspace;
mod workspace_index;
mod workspace_snapshot;

#[cfg(test)]
mod diagnose_source_support;
#[cfg(test)]
mod diagnostic_ux_tests;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod ux_workflow_tests;

mod diagnostic_pipeline;
mod manifest_diagnostics;
mod manifest_language_features;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use std::sync::atomic::AtomicU64;

#[cfg(any(test, feature = "test-harness"))]
use std::sync::atomic::AtomicBool;

use dashmap::{DashMap, DashSet};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request;
use tower_lsp::lsp_types::{Url, Diagnostic, SemanticToken, InitializeParams, InitializeResult, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, OneOf, TypeDefinitionProviderCapability, ImplementationProviderCapability, HoverProviderCapability, CompletionOptions, SemanticTokensServerCapabilities, SemanticTokensOptions, SemanticTokensLegend, SemanticTokensFullOptions, RenameOptions, FoldingRangeProviderCapability, SelectionRangeProviderCapability, DocumentLinkOptions, SignatureHelpOptions, CallHierarchyServerCapability, ExecuteCommandOptions, CodeActionProviderCapability, CodeActionOptions, CodeActionKind, CodeLensOptions, WorkspaceServerCapabilities, WorkspaceFoldersServerCapabilities, ServerInfo, InitializedParams, FileSystemWatcher, GlobPattern, WatchKind, Registration, DidChangeWatchedFilesRegistrationOptions, DidChangeConfigurationParams, DidChangeWorkspaceFoldersParams, DidChangeWatchedFilesParams, FileChangeType, ExecuteCommandParams, DidOpenTextDocumentParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, ReferenceParams, Location, HoverParams, Hover, CompletionParams, CompletionResponse, CompletionItem, SemanticTokensParams, SemanticTokensResult, SemanticTokensRangeParams, SemanticTokensRangeResult, SemanticTokensDeltaParams, SemanticTokensFullDeltaResult, FoldingRangeParams, FoldingRange, SelectionRangeParams, SelectionRange, InlayHintParams, InlayHint, CodeLensParams, CodeLens, TextDocumentPositionParams, PrepareRenameResponse, RenameParams, WorkspaceEdit, WorkspaceSymbolParams, SymbolInformation, DocumentLinkParams, DocumentLink, SignatureHelpParams, SignatureHelp, CallHierarchyPrepareParams, CallHierarchyItem, CallHierarchyIncomingCallsParams, CallHierarchyIncomingCall, CallHierarchyOutgoingCallsParams, CallHierarchyOutgoingCall, TypeHierarchyPrepareParams, TypeHierarchyItem, TypeHierarchySupertypesParams, TypeHierarchySubtypesParams, CodeActionParams, CodeActionResponse, DocumentFormattingParams, TextEdit, SignatureInformation, ParameterInformation, ParameterLabel, Documentation, DocumentSymbol, SymbolKind};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::lsp_types::Range as LspRange;

use sysml_ide_db::AnalysisHost;
use sysml_ide_db::Cancelled;

use crate::diagnostic_pipeline::DiagnosticPipeline;
use crate::library_manager::LibraryManager;
use crate::syntax_context::CursorSyntaxContext;
use crate::workspace_index::WorkspaceIndex;

use sysml_service::SysmlService;

// Re-exports for test modules that reference crate:: paths
#[cfg(test)]
use sysml_service::goto_definition::resolve_goto_target;
#[cfg(test)]
pub(crate) use semantic_tokens::compute_semantic_token_edits;
#[cfg(test)]
use semantic_tokens::SemanticTokensBuilder;
#[cfg(test)]
use utils::position_to_offset;

#[cfg(test)]
use kinds::element_kind_to_symbol_kind;
use types::{
    FeatureFlags, SEMANTIC_TOKEN_MODIFIERS, SEMANTIC_TOKEN_TYPES, SYNTHETIC_FILE,
};
#[cfg(test)]
use utils::offset_to_position;
#[cfg(test)]
use utils::range_to_lsp_range;
use utils::parse_uri;

/// Resolved document data sourced from salsa queries.
///
/// All fields are cheap clones (Arc-backed strings, Arc-backed PositionMap).
/// The position map is memoized by salsa and free after the first call per
/// file revision. Includes the fully-resolved (library-aware) model graph.
pub(crate) struct SalsaDoc {
    pub content: String,
    pub graph: sysml_core::ModelGraph,
    pub position_map: sysml_ide_db::PositionMap,
}

/// Parse-only document data sourced from salsa queries.
///
/// Lighter weight than `SalsaDoc` — only runs the parser, not resolution.
/// Suitable for handlers that only need syntax information (document symbols,
/// semantic tokens, folding ranges).
pub(crate) struct SalsaParsedDoc {
    pub content: String,
    pub graph: sysml_core::ModelGraph,
}

fn uri_aliases(uri: &str) -> Vec<String> {
    let mut aliases = vec![uri.to_owned()];
    if let Some(canonical) = workspace::canonical_file_uri(uri) {
        if canonical != uri {
            aliases.push(canonical);
        }
    }
    aliases
}

// NOTE (URI-form invariant, kept from the deleted `lsp_uri_to_service_uri`
// helper): the salsa host keys SourceFiles by URI string, so the LSP and
// the service-side workspace loader MUST agree on the URI form for the
// same physical file — otherwise two SourceFile instances exist for one
// file and only the service-loaded one lands in the workspace
// `ProjectFileSet` (the WaterPort bug). The canonicalization now lives in
// the FileSet itself (`015f2510`), which is why the per-call helper went
// dead and was removed.

/// The SysML language server backend.
#[derive(Clone)]
pub struct SysmlLanguageServer {
    /// The LSP client.
    pub(crate) client: Client,
    /// Unified domain state owner (sessions, parsers, diagrams, registry, hover cache).
    pub(crate) service: Arc<SysmlService>,
    /// Salsa-backed incremental database (source of truth for file content and queries).
    ///
    /// **Shared with `service.host_arc()`** (S2.T6 host unification, 2026-05-08):
    /// LSP and service operate on the *same* `AnalysisHost`, so editor-driven
    /// file content (via `did_open` / `did_change`) and service-driven content
    /// (via `service.load_*` / MCP / REST) live in one salsa DB. The Mutex is
    /// `std::sync::Mutex` — guards must NEVER be held across `.await`. Lock
    /// briefly to set inputs or pull an `Analysis` snapshot, then drop.
    pub(crate) analysis_host: Arc<std::sync::Mutex<AnalysisHost>>,
    /// Feature flags (mutable via did_change_configuration).
    pub(crate) features: Arc<RwLock<FeatureFlags>>,
    /// Diagnostic pipeline: background coordinator and task ID generation.
    pub(crate) diagnostic_pipeline: Arc<DiagnosticPipeline>,
    /// Workspace index: cross-file index and workspace roots.
    pub(crate) workspace_index: Arc<WorkspaceIndex>,
    /// Library manager: a no-op shell post-P-RA4. Kept to preserve
    /// `clone_for_spawn` shape; the canonical lifecycle now lives on
    /// `SysmlService` and is observed via `service.readiness_for(_)`.
    #[allow(dead_code)]
    pub(crate) library_manager: Arc<LibraryManager>,
    /// Request deduplication: discards stale results for superseded requests.
    pub(crate) pending_requests: Arc<pending_requests::PendingRequests>,
    /// Diagnostic fingerprints: skip re-publishing when diagnostics haven't changed.
    /// Maps URI -> hash of the last published diagnostic vector.
    pub(crate) last_published_diagnostics: Arc<DashMap<String, u64>>,
    /// Last published diagnostics payload by URI.
    ///
    /// Used by diagram generation to overlay diagnostic severity on nodes.
    pub(crate) last_published_diagnostics_payload: Arc<DashMap<String, Vec<Diagnostic>>>,
    /// Semantic token delta support: URI → (result_id, previous tokens).
    pub(crate) last_semantic_tokens: Arc<DashMap<String, (String, Vec<SemanticToken>)>>,
    /// Open text documents tracked by URI.
    ///
    /// Used to prevent background workspace indexing from overwriting unsaved
    /// editor buffer content with on-disk file content.
    pub(crate) open_documents: Arc<DashSet<String>>,
    /// Monotonic counter for generating semantic token result IDs.
    pub(crate) semantic_token_counter: Arc<AtomicU64>,
    /// Skip background tasks (library loading, workspace indexing) in tests.
    ///
    /// Gated on `test` OR the explicit `test-harness` feature so the exported
    /// in-process harness (`test_harness::TestServer`) can suppress the
    /// background library-load + workspace-index tasks when driven from
    /// another crate. Without this knob an external harness always ran the
    /// full stdlib load on `initialized`, which is the stage that made the
    /// cross-transport identity test race and hang (task #225).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) skip_background_tasks: Arc<AtomicBool>,
    /// Skip `did_open`'s heavy disk-project materialization for on-disk File
    /// targets (see [`Self::skip_heavy_file_load`]). Opt-in, default `false`,
    /// independent of `skip_background_tasks` — parse-level harness tests set
    /// it; protocol tests that need strict-mode neighbour visibility leave it
    /// off. Fix for the ~80s did_open cost behind task #225.
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) skip_disk_project_load: Arc<AtomicBool>,
    /// Last published manifest diagnostics (`sysml.toml`) by URI for protocol tests.
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) last_manifest_diagnostics: Arc<DashMap<String, Vec<Diagnostic>>>,
}

const DID_CHANGE_DIAGNOSTICS_DEBOUNCE_MS: u64 = 150;

impl SysmlLanguageServer {
    fn lsp_cache_dir() -> PathBuf {
        directories::ProjectDirs::from("rs", "sysml", "sysml-lsp")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/tmp/sysml-rs"))
    }

    fn panic_log_path() -> PathBuf {
        Self::lsp_cache_dir().join("lsp-panic.log")
    }

    async fn maybe_warn_recent_panic_log(&self) {
        let panic_log = Self::panic_log_path();
        let Ok(meta) = std::fs::metadata(&panic_log) else {
            return;
        };

        let Ok(modified) = meta.modified() else {
            return;
        };

        let age = match SystemTime::now().duration_since(modified) {
            Ok(age) => age,
            Err(_) => Duration::from_secs(0),
        };

        // Keep this warning focused on recent crashes to avoid stale noise.
        if age > Duration::from_secs(48 * 60 * 60) {
            return;
        }

        ux_messages::warn(
            &self.client,
            format!(
                "Recent sysml-lsp panic detected ({}) - run `sysml.debug.status` and check this log if features look degraded.",
                panic_log.display()
            ),
        )
        .await;
    }

    /// Resolved document data from salsa queries.
    ///
    /// All fields are cheap clones (Arc-backed or already-owned).
    /// The position map is memoized by salsa (free after first call).
    pub(crate) async fn salsa_doc(&self, uri: &str) -> Option<SalsaDoc> {
        let (sf, analysis) = self.salsa_file_context(uri).await?;
        let project_id = self.file_project_id(uri).await;
        Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let content = analysis.file_text(sf).to_owned();
            let resolved = analysis.resolve_file_best(sf, project_id);
            let graph = resolved.graph().clone();
            let position_map = analysis.position_map(sf);
            SalsaDoc {
                content,
                graph,
                position_map,
            }
        }))
        .ok()
    }

    /// Parse-only document data from salsa queries.
    ///
    /// Cheaper than `salsa_doc()` — skips resolution. Use for handlers that
    /// only need syntax-level information (document symbols, semantic tokens).
    pub(crate) async fn salsa_parsed_doc(&self, uri: &str) -> Option<SalsaParsedDoc> {
        let (sf, analysis) = self.salsa_file_context(uri).await?;
        Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let content = analysis.file_text(sf).to_owned();
            let parsed = analysis.parse_file(sf);
            let graph = parsed.graph().clone();
            SalsaParsedDoc { content, graph }
        }))
        .ok()
    }

    /// Get the project ID for a file URI, if any.
    pub(crate) async fn file_project_id(&self, uri: &str) -> Option<sysml_project::ProjectHandle> {
        let host = self.analysis_host.lock().unwrap();
        let file_id = host.file_id(uri)?;
        host.files().project_id(file_id)
    }

    /// Get a salsa SourceFile + Analysis snapshot for a URI (single lock acquisition).
    ///
    /// This replaced the old two-lock pattern (a standalone file lookup, then
    /// a separate snapshot grab — both helpers since deleted as dead code),
    /// ensuring the snapshot corresponds to the exact database state where
    /// the file was looked up.
    pub(crate) async fn salsa_file_context(
        &self,
        uri: &str,
    ) -> Option<(sysml_ide_db::SourceFile, sysml_ide_db::Analysis)> {
        let host = self.analysis_host.lock().unwrap();
        let file_id = host.file_id(uri)?;
        let sf = host.source_file(file_id)?;
        let analysis = host.analysis();
        Some((sf, analysis))
    }

    /// Get all open files + Analysis snapshot (single lock acquisition).
    ///
    /// For iteration sites that need to scan all open documents.
    pub(crate) async fn salsa_all_files(
        &self,
    ) -> (Vec<(String, sysml_ide_db::SourceFile)>, sysml_ide_db::Analysis) {
        let host = self.analysis_host.lock().unwrap();
        let analysis = host.analysis();
        let files: Vec<_> = host
            .files()
            .file_ids()
            .filter_map(|id| {
                let uri = host.files().uri(id)?.to_owned();
                let sf = host.source_file(id)?;
                Some((uri, sf))
            })
            .collect();
        (files, analysis)
    }

    /// Build an on-demand workspace snapshot for cross-file lookups.
    ///
    /// Iterates all files in salsa, collecting element names and qualified
    /// names from their cached parse trees.
    pub(crate) async fn workspace_snapshot(&self) -> workspace_snapshot::WorkspaceSnapshot {
        let host = self.analysis_host.lock().unwrap();
        workspace_snapshot::WorkspaceSnapshot::from_host(&host)
    }

    /// Get the tree-sitter tree for a file via salsa (memoized).
    ///
    /// Returns None if the file isn't tracked or tree-sitter parse fails.
    pub(crate) async fn salsa_tree(&self, uri: &str) -> Option<sysml_ide_db::CachedTree> {
        let (sf, analysis) = self.salsa_file_context(uri).await?;
        Cancelled::catch(std::panic::AssertUnwindSafe(|| analysis.parse_tree(sf)))
            .ok()
            .flatten()
    }

    #[cfg(any(test, feature = "test-harness"))]
    fn should_run_inline_diagnostics(&self) -> bool {
        self.skip_background_tasks
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(not(any(test, feature = "test-harness")))]
    fn should_run_inline_diagnostics(&self) -> bool {
        true
    }

    /// Whether `did_open` should skip the heavy synchronous disk-project
    /// materialization (`open_context` → project discovery + `enable_stdlib`
    /// + full-workspace elaboration) for on-disk File targets.
    ///
    /// Tied to the same `skip_background_tasks` knob: a harness that suppresses
    /// background loading also does not want `did_open` to synchronously pull a
    /// whole manifest-backed project + the standard library. The always-run
    /// content-set below still registers the buffer, so the file stays
    /// parseable (`require_graph`/salsa parse queries work); only the workspace
    /// scaffolding is skipped. This is what removes the ~80s `did_open` cost
    /// that made the cross-transport identity harness look like it hung
    /// (task #225). Never skipped in a normal build.
    #[cfg(any(test, feature = "test-harness"))]
    fn skip_heavy_file_load(&self) -> bool {
        self.skip_disk_project_load
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(not(any(test, feature = "test-harness")))]
    fn skip_heavy_file_load(&self) -> bool {
        false
    }

    async fn refresh_open_diagram_for_uri(&self, uri: &str) {
        let start = std::time::Instant::now();
        let Some(view_type) = self
            .service
            .diagram_manager()
            .open_diagrams
            .get(uri)
            .map(|v| *v.value())
        else {
            return;
        };
        let view_type_str = diagram::view_type_name(view_type);

        // Bucket B / B1 P1: SModel projection + overlay + stale-id prune live
        // on the service. Refresh becomes "re-render via diagram.view, send
        // notifications".
        let model = match self.service.diagram_view(uri, view_type_str) {
            Ok(v) => v,
            Err(_) => return,
        };

        let params = diagram::DiagramSetModelParams {
            uri: uri.to_owned(),
            view_type: view_type_str.to_owned(),
            model,
        };
        tracing::debug!("sending diagram notification: uri={}", uri);
        self.client
            .send_notification::<commands::DiagramSetModelNotification>(params)
            .await;

        if let Ok(graph) = self.service.workspace_aware_graph() {
            let graph_params = diagram::build_set_model_graph_params(uri, &graph, view_type);
            self.client
                .send_notification::<commands::DiagramSetModelGraphNotification>(graph_params)
                .await;
        }

        tracing::debug!(
            uri,
            elapsed_ms = start.elapsed().as_millis() as u64,
            "diagram refresh complete"
        );
    }

    async fn run_diagnostics_cycle(&self, uri: String, version: Option<i32>) {
        // `sysml.toml` uses a dedicated manifest diagnostics pipeline.
        // Guard here to prevent accidental publication of SysML parser
        // diagnostics over manifest diagnostics for TOML files.
        if uri.ends_with("sysml.toml") {
            return;
        }

        // Tree-sitter trees are now managed by salsa (via parse_tree query).
        // No manual tree cache update needed.

        // 1. Run salsa diagnostics — the ONLY diagnostic path.
        // Cancellation can happen under concurrent workspace/index mutations; retry
        // a few times so one cancelled pass does not leave stale diagnostics until
        // the next user edit.
        let mut cancelled_attempts = 0usize;
        let salsa_diags = loop {
            let (diagnostics, was_cancelled) = self.salsa_diagnostics_with_status(&uri).await;
            if !was_cancelled {
                break diagnostics;
            }
            cancelled_attempts += 1;
            if cancelled_attempts >= 3 {
                tracing::debug!(
                    uri,
                    attempts = cancelled_attempts,
                    "diagnostics cancelled repeatedly; rescheduling diagnostics cycle"
                );
                self.schedule_diagnostics_for_change(uri.clone(), version.unwrap_or_default());
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        // 3. Publish diagnostics (skip if identical to last publish).
        //
        // The canonical URI we stored in `FileSet` is the file:// stripped raw
        // path (see `source::canonicalize_uri`). `Url::parse` rejects unprefixed
        // paths, so we must fall back to `Url::from_file_path` when the URI
        // looks like a raw filesystem path — otherwise Phase 4's re-publish
        // silently drops empty diagnostics for every workspace file, and the
        // initial pre-workspace E200 from did_open stays displayed forever.
        // This was the WaterPort "MCP clean, VS Code red" divergence.
        let parsed_for_publish = Url::parse(&uri).ok().or_else(|| {
            if uri.starts_with('/') {
                Url::from_file_path(&uri).ok()
            } else {
                None
            }
        });
        if let Some(parsed_uri) = parsed_for_publish {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            salsa_diags.len().hash(&mut hasher);
            for diag in &salsa_diags {
                diag.range.start.line.hash(&mut hasher);
                diag.range.start.character.hash(&mut hasher);
                diag.range.end.line.hash(&mut hasher);
                diag.range.end.character.hash(&mut hasher);
                format!("{:?}", diag.severity).hash(&mut hasher);
                diag.message.hash(&mut hasher);
                if let Some(ref code) = diag.code {
                    format!("{:?}", code).hash(&mut hasher);
                }
            }
            let fingerprint = hasher.finish();

            // Key the dedup/payload maps by the file:// URL form rather than
            // the as-received `uri` string. did_open hands us "file:///x"
            // (from VS Code); the workspace indexer's Phase 4 re-publish
            // hands us "/x" (from `FileSet`'s canonical raw-path key). Both
            // normalize via `Url::parse` / `Url::from_file_path` to the same
            // `parsed_uri`, so using it as the map key means a second
            // publish for the same physical file overwrites the first —
            // not appears as a fresh entry. Without this the published
            // payload returned via `last_server_published_diagnostics` for
            // a file:// URI is stuck on the FIRST publish's payload and
            // the workspace re-publish only appears under the raw-path
            // key, invisible to anyone querying with the original URI.
            let canonical_key = parsed_uri.as_str().to_owned();
            let should_publish = match self.last_published_diagnostics.get(&canonical_key) {
                Some(prev) => *prev != fingerprint,
                None => true,
            };

            if should_publish {
                self.last_published_diagnostics
                    .insert(canonical_key.clone(), fingerprint);
                self.last_published_diagnostics_payload
                    .insert(canonical_key, salsa_diags.clone());
                self.client
                    .publish_diagnostics(parsed_uri, salsa_diags, version)
                    .await;
            } else {
                tracing::debug!(uri, "diagnostics unchanged, skipping publish");
            }
        }

        // 4. Refresh open diagrams asynchronously — don't block the diagnostic
        //    cycle. Diagram refresh involves elaboration + JSON serialization which
        //    can take 100-500ms+ per diagram. Running this inline causes visible
        //    delays between keystrokes and diagnostic updates appearing.
        if self.service.diagram_manager().open_diagrams.get(&uri).is_some() {
            let all_uris: Vec<String> = self
                .service.diagram_manager()
                .open_diagrams
                .iter()
                .map(|entry| entry.key().clone())
                .collect();
            // Spawn as a background task so diagnostics return immediately
            let this = self.clone();
            tokio::spawn(async move {
                for diagram_uri in all_uris {
                    tracing::debug!(uri = %diagram_uri, "background diagram refresh");
                    this.refresh_open_diagram_for_uri(&diagram_uri).await;
                }
            });
        }
    }

    fn schedule_diagnostics_for_change(&self, uri: String, version: i32) {
        let task_id = self.next_background_task_id("did-change-diagnostics");
        let server = self.clone();
        let delay = Duration::from_millis(DID_CHANGE_DIAGNOSTICS_DEBOUNCE_MS);
        let uri_for_task = uri.clone();
        let handle = tokio::spawn(async move {
            tracing::debug!(
                task_id = %task_id,
                uri = %uri_for_task,
                version,
                debounce_ms = delay.as_millis(),
                "scheduled debounced diagnostics task"
            );
            tokio::time::sleep(delay).await;
            server
                .run_diagnostics_cycle(uri_for_task, Some(version))
                .await;
        });
        self.diagnostic_pipeline
            .replace_diagnostics_task(uri, handle);
    }

    /// Generate a stable correlation ID for background/spawned tasks.
    fn next_background_task_id(&self, prefix: &str) -> String {
        self.diagnostic_pipeline.next_background_task_id(prefix)
    }

    fn hover_source_cache_key(path_or_uri: &str) -> String {
        parse_uri(path_or_uri)
            .map(|url| url.to_string())
            .unwrap_or_else(|| path_or_uri.to_owned())
    }

    async fn invalidate_hover_source_cache_entry(&self, uri: &str) {
        let key = Self::hover_source_cache_key(uri);
        self.service.invalidate_external_source(&key);
    }

    pub(crate) async fn load_external_hover_source(
        &self,
        element: &sysml_core::Element,
        active_uri: &str,
    ) -> Option<Arc<String>> {
        let span = element
            .spans
            .iter()
            .find(|span| !span.file.is_empty() && span.file != SYNTHETIC_FILE)?;

        let span_key = Self::hover_source_cache_key(&span.file);
        let active_key = Self::hover_source_cache_key(active_uri);
        if span_key == active_key {
            return None;
        }

        // Live-editor fast path: if the span's file is open in the LSP host,
        // the salsa-tracked content is fresher than any cache or disk read.
        if let Some(doc) = self.salsa_doc(&span.file).await {
            return Some(Arc::new(doc.content));
        }
        if let Some(span_uri) = parse_uri(&span.file).map(|url| url.to_string()) {
            if let Some(doc) = self.salsa_doc(&span_uri).await {
                return Some(Arc::new(doc.content));
            }
        }

        if let Some(cached) = self.service.cached_external_source(&span_key) {
            return Some(cached);
        }

        let source_path = parse_uri(&span.file)
            .and_then(|url| url.to_file_path().ok())
            .or_else(|| {
                let candidate = PathBuf::from(&span.file);
                if candidate.is_absolute() {
                    Some(candidate)
                } else {
                    None
                }
            })?;
        let source = match tokio::fs::read_to_string(&source_path).await {
            Ok(source) => source,
            Err(error) => {
                tracing::debug!(
                    span_file = %span.file,
                    path = %source_path.display(),
                    error = %error,
                    "failed reading external hover source"
                );
                return None;
            }
        };

        let source = Arc::new(source);
        self.service.cache_external_source(span_key, source.clone());
        Some(source)
    }

    /// Create a new language server with a fresh, empty `SysmlService`.
    ///
    /// Suitable for stdio transports where the LSP owns the only copy of
    /// domain state. For shared-process transports (e.g. the `/lsp`
    /// WebSocket served from `sysml-api`), use `new_with_service` so
    /// every transport sees the same salsa `AnalysisHost`.
    pub fn new(client: Client) -> Self {
        Self::new_with_service(client, Arc::new(SysmlService::empty()))
    }

    /// Create a new language server reusing an existing `SysmlService`.
    ///
    /// The LSP and the caller (REST, MCP, CLI) then operate on the same
    /// salsa `AnalysisHost` — editor-driven `did_change` writes land in
    /// the same store the REST `sysml.*` commands read from, so the FE
    /// does not need to dual-write source edits.
    pub fn new_with_service(client: Client, service: Arc<SysmlService>) -> Self {
        let analysis_host = service.host_arc().clone();
        SysmlLanguageServer {
            client,
            service,
            analysis_host,
            features: Arc::new(RwLock::new(FeatureFlags::default())),
            diagnostic_pipeline: Arc::new(DiagnosticPipeline::new()),
            workspace_index: Arc::new(WorkspaceIndex::new()),
            library_manager: Arc::new(LibraryManager::new()),
            pending_requests: Arc::new(pending_requests::PendingRequests::new()),
            last_published_diagnostics: Arc::new(DashMap::new()),
            last_published_diagnostics_payload: Arc::new(DashMap::new()),
            last_semantic_tokens: Arc::new(DashMap::new()),
            open_documents: Arc::new(DashSet::new()),
            semantic_token_counter: Arc::new(AtomicU64::new(0)),
            #[cfg(any(test, feature = "test-harness"))]
            skip_background_tasks: Arc::new(AtomicBool::new(false)),
            #[cfg(any(test, feature = "test-harness"))]
            skip_disk_project_load: Arc::new(AtomicBool::new(false)),
            #[cfg(any(test, feature = "test-harness"))]
            last_manifest_diagnostics: Arc::new(DashMap::new()),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for SysmlLanguageServer {
    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            root_uri = ?params.root_uri,
            workspace_folders = params.workspace_folders.as_ref().map(|f| f.len()).unwrap_or(0)
        )
    )]
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let disable_inlay_hints = std::env::var("SYSML_LSP_DISABLE_INLAY_HINTS")
            .map(|v| {
                matches!(
                    v.as_str(),
                    "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
                )
            })
            .unwrap_or(false);
        let inlay_hints_enabled = !disable_inlay_hints;
        {
            let mut features = self.features.write().await;
            features.inlay_hints = inlay_hints_enabled;
        }
        if disable_inlay_hints {
            tracing::warn!("inlay hints disabled via SYSML_LSP_DISABLE_INLAY_HINTS");
        }

        // Capture workspace roots
        let mut roots = Vec::new();
        if let Some(folders) = &params.workspace_folders {
            for folder in folders {
                if let Ok(path) = folder.uri.to_file_path() {
                    roots.push(path.display().to_string());
                }
            }
        }
        #[allow(deprecated)]
        if roots.is_empty() {
            if let Some(root_uri) = &params.root_uri {
                if let Ok(path) = root_uri.to_file_path() {
                    roots.push(path.display().to_string());
                }
            }
        }
        {
            let mut wr = self.workspace_index.workspace_roots.write().await;
            *wr = roots.clone();
        }

        // P4 architectural cutover: register every workspace root as a real
        // `Project` on the host *synchronously*, before initialize returns.
        // This makes `host.find_project_for_uri` resolve any subsequent URI
        // (via `did_open`, `did_change`, file-watcher, MCP `load_workspace`)
        // to `DEFAULT_PROJECT_ID` from one lookup. Previously every file-entry
        // path had to compensate with its own workspace_roots check; the
        // resulting drift was the WaterPort bug class.
        let synthetic_pid = sysml_project::ProjectHandle(
            sysml_service::open_context::DEFAULT_PROJECT_ID,
        );
        let analysis_host = self.analysis_host.clone();
        let roots_for_register = roots.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut host = analysis_host.lock().unwrap();
            for raw_root in &roots_for_register {
                let root_path = std::path::PathBuf::from(raw_root);
                if !root_path.exists() {
                    continue;
                }
                if host.has_project_at_path(&root_path) {
                    continue;
                }
                let info = sysml_project::ProjectInfo {
                    name: root_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("workspace")
                        .to_string(),
                    description: None,
                    version: "0.0.0-synthetic".to_string(),
                    topic: Vec::new(),
                    usage: Vec::new(),
                };
                host.load_project(sysml_project::Project {
                    id: synthetic_pid,
                    info,
                    meta: None,
                    root: sysml_project::ProjectRoot::Directory(root_path),
                });
            }
        })
        .await;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ":".to_owned(),
                        ".".to_owned(),
                        "=".to_owned(),
                        "[".to_owned(),
                        "\"".to_owned(),
                    ]),
                    resolve_provider: Some(true),
                    work_done_progress_options: Default::default(),
                    all_commit_characters: Some(vec![
                        ":".to_owned(),
                        ".".to_owned(),
                        "(".to_owned(),
                        ")".to_owned(),
                        ";".to_owned(),
                        " ".to_owned(),
                    ]),
                    completion_item: None,
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: SEMANTIC_TOKEN_TYPES.to_vec(),
                                token_modifiers: SEMANTIC_TOKEN_MODIFIERS.to_vec(),
                            },
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                inlay_hint_provider: if inlay_hints_enabled {
                    Some(OneOf::Left(true))
                } else {
                    None
                },
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec![" ".to_owned(), "(".to_owned()]),
                    retrigger_characters: Some(vec![",".to_owned()]),
                    work_done_progress_options: Default::default(),
                }),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "sysml.cache.clear".to_owned(),
                        "sysml.cache.status".to_owned(),
                        "sysml.cache.rebuild".to_owned(),
                        "sysml.debug.status".to_owned(),
                        "sysml.debug.bundle".to_owned(),
                        "sysml.evaluate".to_owned(),
                        "sysml.verify".to_owned(),
                        "sysml.simulate.start".to_owned(),
                        "sysml.simulate.step".to_owned(),
                        "sysml.simulate.stop".to_owned(),
                        "sysml.simulate.reset".to_owned(),
                        "sysml.action.run".to_owned(),
                        "sysml.action.start".to_owned(),
                        "sysml.action.step".to_owned(),
                        "sysml.action.stop".to_owned(),
                        "sysml.action.reset".to_owned(),
                        "sysml.action.visualize".to_owned(),
                        "sysml.flow.visualize".to_owned(),
                        "sysml.whatif".to_owned(),
                        "sysml.whatif.sweep".to_owned(),
                        "sysml.diagram.whatif".to_owned(),
                        "sysml.project.info".to_owned(),
                        "sysml.workspace.refresh".to_owned(),
                        "sysml.workspace.info".to_owned(),
                        "sysml.dependency.status".to_owned(),
                        "sysml.workspace.verify".to_owned(),
                        "sysml.salsa.stats".to_owned(),
                        "sysml.salsa.stats.reset".to_owned(),
                        "sysml.diagram.open".to_owned(),
                        "sysml.diagram.view".to_owned(),
                        "sysml.diagram.export".to_owned(),
                        "sysml.diagram.expand".to_owned(),
                        "sysml.diagram.edit".to_owned(),
                        "sysml.model.tree".to_owned(),
                        "sysml.scenario.run".to_owned(),
                        "sysml.timeline.getTrace".to_owned(),
                        "sysml.timeline.getSnapshot".to_owned(),
                        "sysml.analysis.run".to_owned(),
                        "sysml.evaluate.all".to_owned(),
                        "sysml.orchestrate.start".to_owned(),
                        "sysml.orchestrate.step".to_owned(),
                        "sysml.orchestrate.inject".to_owned(),
                        "sysml.orchestrate.stop".to_owned(),
                        "sysml.montecarlo.run".to_owned(),
                        "sysml.sessions.step".to_owned(),
                        "sysml.sessions.inject".to_owned(),
                    ],
                    work_done_progress_options: Default::default(),
                }),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_REWRITE,
                            CodeActionKind::SOURCE_ORGANIZE_IMPORTS,
                        ]),
                        resolve_provider: Some(false),
                        work_done_progress_options: Default::default(),
                    },
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "sysml-lsp-server".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn initialized(&self, _: InitializedParams) {
        ux_messages::info(&self.client, "SysML language server initialized").await;

        // Tell user where detailed logs live so they can tail the file for debugging.
        let log_path = Self::lsp_cache_dir().join("lsp.log");
        ux_messages::info(
            &self.client,
            format!("Detailed server log: {}", log_path.display()),
        )
        .await;

        self.maybe_warn_recent_panic_log().await;

        // P-RA4: subscribe once at startup to the service's progress
        // bus and translate every event to a structured `$/progress`
        // notification (when the event has a numeric phase) AND a
        // `window/logMessage` line (back-compat with snapshot tests
        // and log scrapers). Lagged subscribers drop messages rather
        // than block the publisher; capacity is 256.
        {
            let mut rx = self.service.subscribe_progress();
            let client = self.client.clone();
            tokio::spawn(async move {
                use sysml_service::progress::{LibraryPhase, ProgressEvent};
                while let Ok(event) = rx.recv().await {
                    match event {
                        ProgressEvent::LibraryLoad { phase, done, total, detail } => {
                            // Back-compat: log every transition. The
                            // production load pipeline still emits its
                            // own `Loaded standard library: ...` info
                            // line via `ux_messages::info` for
                            // snapshot-stability — these adapter logs
                            // describe phase transitions only.
                            let phase_name = match phase {
                                LibraryPhase::Loading => "Loading",
                                LibraryPhase::Loaded => "Loaded",
                                LibraryPhase::Failed => "Failed",
                            };
                            ux_messages::info(
                                &client,
                                format!("progress[library]: {phase_name} {done}/{total} {detail}"),
                            )
                            .await;
                        }
                        ProgressEvent::WorkspaceIndex { phase, done, total, detail } => {
                            ux_messages::info(
                                &client,
                                format!("progress[workspace phase {phase}]: {done}/{total} {detail}"),
                            )
                            .await;
                        }
                        ProgressEvent::DependencyFetch { name, done, total } => {
                            ux_messages::info(
                                &client,
                                format!("progress[deps]: {name} {done}/{total}"),
                            )
                            .await;
                        }
                        ProgressEvent::Refresh { reason } => {
                            ux_messages::info(
                                &client,
                                format!("progress[refresh]: {reason}"),
                            )
                            .await;
                        }
                        ProgressEvent::Ready => {
                            ux_messages::info(&client, "progress[ready]".to_owned()).await;
                        }
                    }
                    // TODO(P-RA5): also emit `$/progress` structured
                    // notifications keyed by token (`sysml/library-loading`,
                    // `sysml/workspace-indexing`). tower-lsp 0.20 carries
                    // `WorkDoneProgress` types, but the load pipeline
                    // already creates and ends those tokens itself —
                    // duplicating them here would double-report. P-RA5
                    // will fold those existing notifications into this
                    // adapter once the load pipeline stops emitting
                    // them directly.
                }
            });
        }

        // Register file watchers for .sysml and .kerml files
        let registration_id = "sysml-file-watcher".to_owned();
        let watched_patterns = vec!["**/*.sysml".to_owned(), "**/*.kerml".to_owned()];
        let watcher_count = watched_patterns.len();
        let watchers = watched_patterns
            .iter()
            .map(|pattern| FileSystemWatcher {
                glob_pattern: GlobPattern::String(pattern.clone()),
                kind: Some(WatchKind::all()),
            })
            .collect();
        let registration = Registration {
            id: registration_id.clone(),
            method: "workspace/didChangeWatchedFiles".to_owned(),
            register_options: Some(
                serde_json::to_value(DidChangeWatchedFilesRegistrationOptions { watchers })
                    .unwrap_or(serde_json::Value::Null),
            ),
        };
        if let Err(e) = self.client.register_capability(vec![registration]).await {
            tracing::warn!(
                error = %e,
                registration_id = %registration_id,
                watcher_count,
                watched_patterns = ?watched_patterns,
                "failed to register file watcher capability"
            );
        }

        // Register file watchers for project manifests
        {
            let manifest_registration_id = "sysml-project-watcher".to_owned();
            let manifest_patterns = [
                "**/.project.json",
                "**/.meta.json",
                "**/.workspace.json",
                "**/sysml.toml",
                "**/sysml.lock",
            ];
            let manifest_watchers: Vec<FileSystemWatcher> = manifest_patterns
                .iter()
                .map(|pattern| FileSystemWatcher {
                    glob_pattern: GlobPattern::String((*pattern).to_owned()),
                    kind: Some(WatchKind::all()),
                })
                .collect();
            let manifest_reg = Registration {
                id: manifest_registration_id.clone(),
                method: "workspace/didChangeWatchedFiles".to_owned(),
                register_options: Some(
                    serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                        watchers: manifest_watchers,
                    })
                    .unwrap_or(serde_json::Value::Null),
                ),
            };
            if let Err(e) = self.client.register_capability(vec![manifest_reg]).await {
                tracing::warn!(
                    error = %e,
                    registration_id = %manifest_registration_id,
                    "failed to register project manifest file watchers"
                );
            }
        }

        // Discover projects in workspace roots
        let roots = self.workspace_index.workspace_roots.read().await.clone();
        let mut discovered_projects: Vec<sysml_project::Project> = Vec::new();
        let mut should_enable_stdlib = false;
        for root_str in &roots {
            let root = std::path::Path::new(root_str);
            match sysml_service::project_discovery::discover_lsp_workspace(root, true) {
                Ok(discovery) => {
                    let project_count = discovery.projects.len();

                    tracing::info!(
                        root = %root.display(),
                        project_count,
                        include_stdlib = discovery.include_stdlib,
                        description = %discovery.discovery_description,
                        "project discovery complete"
                    );
                    discovered_projects.extend(discovery.projects.into_iter());
                    should_enable_stdlib = should_enable_stdlib || discovery.include_stdlib;

                    ux_messages::info(&self.client, discovery.discovery_description).await;
                }
                Err(e) => {
                    tracing::warn!(
                        root = %root.display(),
                        error = %e,
                        "project discovery failed"
                    );
                }
            }
        }

        // Stable project IDs across startup/re-discovery.
        discovered_projects.sort_by_key(|project| {
            let root = match &project.root {
                sysml_project::ProjectRoot::Directory(dir) => dir
                    .canonicalize()
                    .unwrap_or_else(|_| dir.clone())
                    .display()
                    .to_string(),
                _ => format!("in-memory:{}", project.info.name),
            };
            (root, project.info.name.clone())
        });
        for (idx, project) in discovered_projects.iter_mut().enumerate() {
            project.id = sysml_project::ProjectHandle(10 + idx as u32);
        }

        let all_project_roots: Vec<(sysml_project::ProjectHandle, PathBuf)> = discovered_projects
            .iter()
            .filter_map(|project| match &project.root {
                sysml_project::ProjectRoot::Directory(dir) => Some((project.id, dir.clone())),
                _ => None,
            })
            .collect();

        // Register projects in project registry after IDs are finalized.
        {
            let mut registry = self.service.project_registry().write().unwrap();
            for project in &discovered_projects {
                registry.register(project.clone());
            }
        }

        // Load discovered projects into salsa database once.
        {
            let mut host = self.analysis_host.lock().unwrap();
            for project in discovered_projects {
                // Supersede the synthetic placeholder registered for this root in
                // `initialize` (one project per directory — see host method doc).
                host.load_project_superseding_synthetic(project);
            }
            if should_enable_stdlib {
                if let Err(e) = host.enable_stdlib() {
                    tracing::warn!(error = %e, "failed to enable stdlib in salsa");
                }
            }
        }

        // Start background tasks (skipped in test / test-harness mode for
        // fast, quiescent execution — this is the stage that made the
        // cross-transport identity harness race and hang, task #225).
        #[cfg(any(test, feature = "test-harness"))]
        let skip = self
            .skip_background_tasks
            .load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(any(test, feature = "test-harness")))]
        let skip = false;

        if !skip {
            // Always start library loading — standard library types should be
            // available regardless of project discovery results.
            {
                let library_task_id = self.next_background_task_id("initialized-library-load");
                let server = self.clone_for_spawn();
                tokio::spawn(async move {
                    tracing::info!(
                        task_id = %library_task_id,
                        task = "library_load",
                        trigger = "initialized",
                        "spawned background task"
                    );
                    server.load_library_background(library_task_id).await;
                });
            }

            // Start workspace-wide file indexing in background
            let workspace_index_task_id =
                self.next_background_task_id("initialized-workspace-index");
            let roots = self.workspace_index.workspace_roots.read().await.clone();
            let max_files = self.features.read().await.max_index_files;
            let project_roots = all_project_roots;
            let server = self.clone();
            tokio::spawn(async move {
                tracing::info!(
                    task_id = %workspace_index_task_id,
                    task = "workspace_index",
                    trigger = "initialized",
                    roots = roots.len(),
                    projects = project_roots.len(),
                    max_files,
                    "spawned background task"
                );
                server
                    .index_workspace_files(workspace_index_task_id, roots, project_roots, max_files)
                    .await;
            });
        }
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn shutdown(&self) -> Result<()> {
        self.diagnostic_pipeline.cancel_all_diagnostics_tasks();
        Ok(())
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(has_sysml_section = params.settings.get("sysml").is_some())
    )]
    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        // Try to extract settings from the "sysml" section
        if let Some(sysml_settings) = params.settings.get("sysml") {
            let mut features = self.features.write().await;

            if let Some(timeout) = sysml_settings
                .get("resolutionTimeoutMs")
                .and_then(|v| v.as_u64())
            {
                features.resolution_timeout_ms = timeout;
                tracing::info!(resolution_timeout_ms = timeout, "configuration changed");
            }
            if let Some(resolution) = sysml_settings.get("resolution").and_then(|v| v.as_bool()) {
                features.resolution = resolution;
                tracing::info!(resolution, "configuration changed");
            }
            if let Some(validation) = sysml_settings.get("validation").and_then(|v| v.as_bool()) {
                features.validation = validation;
                tracing::info!(validation, "configuration changed");
            }
            if let Some(max_files) = sysml_settings.get("maxIndexFiles").and_then(|v| v.as_u64()) {
                features.max_index_files = max_files as u32;
                tracing::info!(max_index_files = max_files, "configuration changed");
            }
            if let Some(inlay_hints) = sysml_settings.get("inlayHints").and_then(|v| v.as_bool()) {
                features.inlay_hints = inlay_hints;
                tracing::info!(inlay_hints, "configuration changed");
            }
            if let Some(library_path) = sysml_settings.get("libraryPath").and_then(|v| v.as_str()) {
                let path = PathBuf::from(library_path);
                features.library_path_override = Some(path);
                tracing::info!(library_path = %library_path, "configuration changed");
                // Drop the write lock before spawning
                drop(features);
                // Reset library lifecycle so the service's
                // `readiness_for` falls back to host-derived state
                // until the load completes again (P-RA4 retired the
                // local `LibraryState` enum).
                self.service.reset_library_lifecycle();
                // Spawn library loading with the override path
                let library_task_id = self.next_background_task_id("config-library-load");
                let server = self.clone_for_spawn();
                tokio::spawn(async move {
                    tracing::info!(
                        task_id = %library_task_id,
                        task = "library_load",
                        trigger = "did_change_configuration",
                        "spawned background task"
                    );
                    server.load_library_background(library_task_id).await;
                });
            }
        }
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            added = params.event.added.len(),
            removed = params.event.removed.len()
        )
    )]
    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        {
            let mut roots = self.workspace_index.workspace_roots.write().await;
            for removed in &params.event.removed {
                if let Ok(path) = removed.uri.to_file_path() {
                    let removed_str = path.display().to_string();
                    roots.retain(|root| root != &removed_str);
                }
            }
            for added in &params.event.added {
                if let Ok(path) = added.uri.to_file_path() {
                    let added_str = path.display().to_string();
                    if !roots.contains(&added_str) {
                        roots.push(added_str);
                    }
                }
            }
            roots.sort();
            roots.dedup();
        }

        self.rediscover_workspace_state("workspace folder change")
            .await;
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(changes = params.changes.len())
    )]
    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let mut manifest_changed = false;
        for change in &params.changes {
            let uri = change.uri.to_string();

            // Check if this is a manifest file change (triggers project re-discovery)
            let is_manifest = uri.ends_with("sysml.toml")
                || uri.ends_with("sysml.lock")
                || uri.ends_with(".project.json")
                || uri.ends_with(".workspace.json")
                || uri.ends_with(".meta.json");
            if is_manifest {
                manifest_changed = true;
                tracing::info!(uri = %uri, change_type = ?change.typ, "project manifest changed");
            }

            match change.typ {
                FileChangeType::DELETED => {
                    // Remove from salsa database
                    let analysis_host = self.analysis_host.clone();
                    let uri_c = uri.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        let mut host = analysis_host.lock().unwrap();
                        host.remove_file(&uri_c);
                    })
                    .await
                    {
                        tracing::error!("salsa mutation panicked: {e}");
                    }
                    if telemetry_control::should_log_every_n("watched-file-deleted", 20) {
                        tracing::debug!(uri = %uri, "watched file deleted");
                    }
                }
                FileChangeType::CREATED => {
                    // Skip manifest files — they aren't SysML source files
                    if is_manifest {
                        continue;
                    }
                    // P4-closeout (P4.D): a brand-new file on disk takes
                    // the same path as any other file-entry — route
                    // through open_context(File) so it gets the right
                    // project_id and stdlib hookup. No bespoke
                    // find_project_for_uri / set_file_content fallback.
                    if let Ok(path) = change.uri.to_file_path() {
                        // open_context(File) walks up looking for
                        // sysml.toml; if the file is inside an existing
                        // workspace folder this idempotently refreshes
                        // the same project rather than minting a new one.
                        let service = self.service.clone();
                        if let Err(e) = tokio::task::spawn_blocking(move || {
                            service.open_context(
                                sysml_project::discovery::OpenTarget::File(path),
                            )
                        })
                        .await
                        {
                            tracing::error!("did_change_watched_files CREATE panicked: {e}");
                        }
                        if telemetry_control::should_log_every_n("watched-file-created", 50) {
                            tracing::debug!(
                                uri = %uri,
                                "watched file created — routed through open_context"
                            );
                        }
                    } else if telemetry_control::should_log_every_n(
                        "watched-file-uri-to-path-failed",
                        20,
                    ) {
                        tracing::debug!(uri = %uri, "failed to convert watched file URI to path");
                    }
                }
                FileChangeType::CHANGED => {
                    // Skip manifest files — they aren't SysML source files
                    if is_manifest {
                        continue;
                    }
                    // P4-closeout (P4.D): CHANGED is a pure content
                    // update; the file already has its project_id (set
                    // at CREATE / did_open / index time), which survives
                    // set_file_content. Drop the find_project_for_uri
                    // fallback.
                    if let Ok(path) = change.uri.to_file_path() {
                        if let Ok(content) = tokio::fs::read_to_string(&path).await {
                            let analysis_host = self.analysis_host.clone();
                            let uri_c = uri.clone();
                            if let Err(e) = tokio::task::spawn_blocking(move || {
                                let mut host = analysis_host.lock().unwrap();
                                host.set_file_content(&uri_c, content);
                            })
                            .await
                            {
                                tracing::error!("salsa mutation panicked: {e}");
                            }
                            if telemetry_control::should_log_every_n("watched-file-reindexed", 50) {
                                tracing::debug!(
                                    uri = %uri,
                                    "watched file content updated"
                                );
                            }
                        } else if telemetry_control::should_log_every_n(
                            "watched-file-read-failed",
                            20,
                        ) {
                            tracing::debug!(
                                uri = %uri,
                                path = %path.display(),
                                "failed to read watched file"
                            );
                        }
                    } else if telemetry_control::should_log_every_n(
                        "watched-file-uri-to-path-failed",
                        20,
                    ) {
                        tracing::debug!(uri = %uri, "failed to convert watched file URI to path");
                    }
                }
                _ => {}
            }
        }

        // Apply manifest changes in-process (no window reload required).
        if manifest_changed {
            self.rediscover_workspace_state("manifest change").await;
        }
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(command = %params.command, args = params.arguments.len())
    )]
    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        let ctx = commands::CommandContext {
            client: &self.client,
            analysis_host: &self.analysis_host,
            service: &self.service,
            workspace_roots: &self.workspace_index.workspace_roots,
        };

        let args = &params.arguments;

        // Special case: cache.rebuild needs the server handle for spawning
        if params.command == "sysml.cache.rebuild" {
            let server = self.clone_for_spawn();
            let rebuild_task_id = self.next_background_task_id("command-cache-rebuild");
            let parent_task_id = rebuild_task_id.clone();
            let result = commands::handle_cache_rebuild(&ctx, rebuild_task_id, move || {
                let load_task_id = format!("{parent_task_id}:library-load");
                let parent_for_log = parent_task_id.clone();
                tokio::spawn(async move {
                    tracing::info!(
                        task_id = %load_task_id,
                        parent_task_id = %parent_for_log,
                        task = "library_load",
                        trigger = "sysml.cache.rebuild",
                        "spawned background task"
                    );
                    server.load_library_background(load_task_id).await;
                });
            })
            .await;
            return Ok(Some(result));
        }

        if params.command == "sysml.workspace.refresh" {
            self.rediscover_workspace_state("command refresh").await;
            let mut payload = serde_json::Map::new();
            payload.insert(
                "status".to_owned(),
                serde_json::Value::String("workspace_refreshed".to_owned()),
            );
            return Ok(Some(serde_json::Value::Object(payload)));
        }

        // Dispatch table lookup for all other commands
        let table = command_dispatch::build_dispatch_table();
        match table.get(params.command.as_str()) {
            Some(handler) => Ok(Some(handler(&ctx, args).await)),
            None => Ok(None),
        }
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document.uri,
            version = params.text_document.version,
            bytes = params.text_document.text.len()
        )
    )]
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        // Take sole ownership of the document text; the salsa task below moves
        // it in. The manifest path needs it afterward, so clone only then.
        let content = params.text_document.text;
        let content_len = content.len();
        let version = params.text_document.version;
        let is_manifest = uri.ends_with("sysml.toml");
        let manifest_content = if is_manifest { Some(content.clone()) } else { None };
        for alias in uri_aliases(&uri) {
            self.open_documents.insert(alias);
        }
        self.invalidate_hover_source_cache_entry(&uri).await;

        // P4-closeout (P4.B): route URIs that aren't yet in the host AND
        // aren't going to be picked up by the workspace indexer through
        // the canonical loader. `open_context(File)` picks the correct
        // mode (Strict vs DiscoveredViaManifest), registers the project,
        // and sets the initial SourceFile input from disk. We then
        // overwrite that disk content with the editor's buffer text.
        //
        // We skip this for URIs inside a known workspace root — the
        // indexer (kicked off in `initialized`) owns those files and
        // double-loading would race the readiness gate (P-RA3
        // "pre-Phase-4" tier-gating window collapses). The indexer is
        // routed through the same `open_context`; the file ends up in
        // the same place either way.
        if !is_manifest {
            let analysis_host = self.analysis_host.clone();
            let uri_for_check = uri.clone();
            let already_loaded = tokio::task::spawn_blocking(move || {
                analysis_host
                    .lock()
                    .unwrap()
                    .file_id(&uri_for_check)
                    .is_some()
            })
            .await
            .unwrap_or(false);

            if !already_loaded {
                // Pick the open_context target. A file URI with disk
                // backing inside a workspace root belongs to the indexer;
                // one outside any root loads as OpenTarget::File. A buffer
                // with NO disk backing (unsaved editor buffer, synthetic /
                // non-file scheme) loads as OpenTarget::Synthetic — the
                // workspace-scope collapse (W2) removed the PFS-less
                // per-file fallback from the execution surface, so every
                // open must materialize the workspace ProjectFileSet or
                // eval/whatif/simulate on that buffer fail-hard.
                use sysml_project::discovery::OpenTarget;
                let disk_path = Url::parse(&uri)
                    .ok()
                    .filter(|p| p.scheme() == "file")
                    .and_then(|p| p.to_file_path().ok());
                let target = match disk_path {
                    Some(path) if path.exists() => {
                        let in_workspace = {
                            let roots =
                                self.workspace_index.workspace_roots.read().await;
                            roots.iter().any(|r| {
                                let root_path = std::path::PathBuf::from(r);
                                path.starts_with(&root_path)
                            })
                        };
                        if in_workspace {
                            None // the indexer owns it
                        } else {
                            Some(OpenTarget::File(path))
                        }
                    }
                    _ => Some(OpenTarget::Synthetic {
                        uri: uri.clone(),
                        content: content.clone(),
                    }),
                };
                // A test harness with `skip_background_tasks` set does not want
                // did_open to synchronously discover + load a whole manifest
                // project and enable the standard library (that ~80s of work is
                // what made the cross-transport identity harness look hung —
                // task #225). Skip the materialization for on-disk File targets;
                // the always-run content-set below still registers the buffer so
                // the file stays parseable. Synthetic buffers keep their
                // open_context (cheap, single in-memory file) so protocol tests
                // that rely on the default project's PFS are unaffected.
                let skip_disk_materialization =
                    self.skip_heavy_file_load() && matches!(target, Some(OpenTarget::File(_)));
                if let Some(target) = target.filter(|_| !skip_disk_materialization) {
                    let service = self.service.clone();
                    match tokio::task::spawn_blocking(move || {
                        service.open_context(target)
                    })
                    .await
                    {
                        Ok(Err(e)) => {
                            tracing::error!("did_open open_context failed: {e}");
                        }
                        Err(e) => {
                            tracing::error!(
                                "did_open open_context spawn panicked: {e}"
                            );
                        }
                        Ok(Ok(_)) => {}
                    }
                }
            }
        }

        // Apply the editor's buffer text AND mark the file as an editor
        // overlay — both under the SAME host lock so they're established
        // atomically against the background indexer. While overlaid,
        // `open_context` preserves this buffer instead of overwriting it
        // from disk (the race that previously needed a snapshot/restore
        // band-aid in `index_workspace_files`). The indexer's per-file
        // write and this write both take the host mutex, so they serialize:
        // whichever runs first, the buffer survives.
        // open_context::DEFAULT_PROJECT_ID acts as the fallback pid when
        // find_project_for_uri can't match (e.g. a synthetic / non-file URI
        // or a file the indexer hasn't tagged yet — the indexer will
        // `set_project_only` it via the overlay branch).
        {
            let analysis_host = self.analysis_host.clone();
            let uri_c = uri.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                let mut host = analysis_host.lock().unwrap();
                match host.find_project_for_uri(&uri_c) {
                    Some(pid) => {
                        host.set_file_content_in_project(&uri_c, content, pid);
                    }
                    None => {
                        host.set_file_content(&uri_c, content);
                    }
                }
                host.set_overlay(&uri_c);
            })
            .await
            {
                tracing::error!("salsa mutation panicked: {e}");
            }
        }

        if !is_manifest {
            self.run_diagnostics_cycle(uri.clone(), Some(version)).await;
        }

        // Manifest diagnostics for sysml.toml files
        if is_manifest {
            let manifest_text = manifest_content.as_deref().unwrap_or_default();
            let diagnostics =
                manifest_diagnostics::validate_manifest_with_context(manifest_text, Some(&uri));
            #[cfg(any(test, feature = "test-harness"))]
            self.last_manifest_diagnostics
                .insert(uri.clone(), diagnostics.clone());
            self.last_published_diagnostics_payload
                .insert(uri.clone(), diagnostics.clone());
            if let Ok(parsed_uri) = uri.parse::<Url>() {
                self.client
                    .publish_diagnostics(parsed_uri, diagnostics, None)
                    .await;
            }
        }

        telemetry_events::did_open(&uri, content_len);
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document.uri,
            version = params.text_document.version,
            changes = params.content_changes.len()
        )
    )]
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        let change_count = params.content_changes.len();
        let version = params.text_document.version;
        let is_manifest = uri.ends_with("sysml.toml");
        self.invalidate_hover_source_cache_entry(&uri).await;
        // Invalidate cached diagram graph — file content changed, need re-elaboration
        self.service.diagram_manager().invalidate_graph_cache(&uri);

        if let Some(change) = params.content_changes.into_iter().last() {
            let content = change.text;
            let content_len = content.len();
            // The salsa task moves `content` in; the manifest path needs it
            // afterward, so clone only in that (rare) case.
            let manifest_content = if is_manifest { Some(content.clone()) } else { None };

            // P4-closeout (P4.C): did_change is a pure content update.
            // The project_id was assigned at file-entry time (did_open
            // routed through open_context, or the workspace indexer); the
            // underlying salsa input's `set_text` doesn't clear that
            // association. No find_project_for_uri / set_project_only
            // fallback chain — open_context owns project tagging.
            {
                let analysis_host = self.analysis_host.clone();
                let uri_c = uri.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || {
                    let mut host = analysis_host.lock().unwrap();
                    host.set_file_content(&uri_c, content);
                })
                .await
                {
                    tracing::error!("salsa mutation panicked: {e}");
                }
            }

            if !is_manifest {
                if self.should_run_inline_diagnostics() {
                    self.run_diagnostics_cycle(uri.clone(), Some(version)).await;
                } else {
                    self.schedule_diagnostics_for_change(uri.clone(), version);
                }
            }

            // Manifest diagnostics for sysml.toml files
            if is_manifest {
                let manifest_text = manifest_content.as_deref().unwrap_or_default();
                let diagnostics =
                    manifest_diagnostics::validate_manifest_for_live_edit(manifest_text, Some(&uri));
                #[cfg(any(test, feature = "test-harness"))]
                self.last_manifest_diagnostics
                    .insert(uri.clone(), diagnostics.clone());
                self.last_published_diagnostics_payload
                    .insert(uri.clone(), diagnostics.clone());
                if let Ok(parsed_uri) = uri.parse::<Url>() {
                    self.client
                        .publish_diagnostics(parsed_uri, diagnostics, None)
                        .await;
                }
            }

            telemetry_events::did_change(&uri, content_len, change_count);
        }
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        self.invalidate_hover_source_cache_entry(&uri).await;
        self.diagnostic_pipeline.cancel_diagnostics_task(&uri);
        for alias in uri_aliases(&uri) {
            self.open_documents.remove(&alias);
        }

        // Remove from salsa database.
        // spawn_blocking prevents salsa snapshot cancellation from blocking the async runtime.
        {
            let analysis_host = self.analysis_host.clone();
            let uri_c = uri.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                let mut host = analysis_host.lock().unwrap();
                // The buffer is no longer authoritative — clear the editor
                // overlay so the indexer tracks disk content again.
                host.clear_overlay(&uri_c);
                // Keep indexed workspace/dependency files alive after editor tab close.
                // Preview tabs in VS Code can open+close rapidly; deleting the file here
                // causes workspace symbol snapshots to lose dependency definitions until
                // a full reindex/rediscovery happens.
                let mut restored_from_disk = false;
                if let Ok(parsed) = Url::parse(&uri_c) {
                    if parsed.scheme() == "file" {
                        if let Ok(path) = parsed.to_file_path() {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                match host.find_project_for_uri(&uri_c) {
                                    Some(pid) => {
                                        host.set_file_content_in_project(&uri_c, content, pid);
                                    }
                                    None => {
                                        host.set_file_content(&uri_c, content);
                                    }
                                }
                                restored_from_disk = true;
                            }
                        }
                    }
                }
                if !restored_from_disk {
                    host.remove_file(&uri_c);
                }
            })
            .await
            {
                tracing::error!("salsa mutation panicked: {e}");
            }
        }

        self.last_published_diagnostics.remove(&uri);
        self.last_published_diagnostics_payload.remove(&uri);
        self.last_semantic_tokens.remove(&uri);
        #[cfg(any(test, feature = "test-harness"))]
        self.last_manifest_diagnostics.remove(&uri);

        // Clean up session state for this document by matching the session's
        // stored URI rather than parsing the (opaque) session key.
        self.service
            .sessions()
            .retain(|_, session| session.uri != uri);
        let uri_prefix = format!("{}:", &uri);
        self.service.diagram_manager()
            .open_diagrams
            .retain(|k, _| !k.starts_with(&uri_prefix) && k != &uri);
        self.service.diagram_manager()
            .expanded_nodes
            .retain(|k, _| !k.starts_with(&uri_prefix) && k != &uri);
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        symbols::document_symbol(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            document_uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        navigation::goto_definition(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            document_uri = %params.text_document_position.text_document.uri,
            line = params.text_document_position.position.line,
            character = params.text_document_position.position.character
        )
    )]
    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        navigation::references(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        hover::hover(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            document_uri = %params.text_document_position.text_document.uri,
            line = params.text_document_position.position.line,
            character = params.text_document_position.position.character
        )
    )]
    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        if uri.ends_with("sysml.toml") {
            if let Some(doc) = self.salsa_doc(&uri).await {
                return Ok(manifest_language_features::completion_for_manifest(
                    &doc.content,
                    params.text_document_position.position,
                ));
            }
            return Ok(None);
        }
        completion::completion(self, params).await
    }

    #[tracing::instrument(level = "debug", skip(self, item), fields(label = %item.label))]
    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        completion::completion_resolve(self, item).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        semantic_tokens::semantic_tokens_full(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document.uri,
            start_line = params.range.start.line,
            start_char = params.range.start.character,
            end_line = params.range.end.line,
            end_char = params.range.end.character
        )
    )]
    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        semantic_tokens::semantic_tokens_range(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>> {
        semantic_tokens::semantic_tokens_full_delta(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        ranges::folding_range(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri, positions = params.positions.len())
    )]
    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        ranges::selection_range(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document.uri,
            start_line = params.range.start.line,
            start_char = params.range.start.character,
            end_line = params.range.end.line,
            end_char = params.range.end.character
        )
    )]
    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        inlay_hints::inlay_hint(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        code_lens::code_lens(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document.uri,
            line = params.position.line,
            character = params.position.character
        )
    )]
    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        rename::prepare_rename(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document_position.text_document.uri,
            line = params.text_document_position.position.line,
            character = params.text_document_position.position.character,
            new_name = %params.new_name
        )
    )]
    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        rename::rename(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(query_len = params.query.len())
    )]
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        symbols::symbol(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn goto_type_definition(
        &self,
        params: request::GotoTypeDefinitionParams,
    ) -> Result<Option<request::GotoTypeDefinitionResponse>> {
        navigation::goto_type_definition(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn goto_implementation(
        &self,
        params: request::GotoImplementationParams,
    ) -> Result<Option<request::GotoImplementationResponse>> {
        navigation::goto_implementation(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        advanced_features::document_link(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        advanced_features::signature_help(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        advanced_features::prepare_call_hierarchy(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(item_name = %params.item.name, item_uri = %params.item.uri)
    )]
    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        advanced_features::incoming_calls(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(item_name = %params.item.name, item_uri = %params.item.uri)
    )]
    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        advanced_features::outgoing_calls(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            document_uri = %params.text_document_position_params.text_document.uri,
            line = params.text_document_position_params.position.line,
            character = params.text_document_position_params.position.character
        )
    )]
    async fn prepare_type_hierarchy(
        &self,
        params: TypeHierarchyPrepareParams,
    ) -> Result<Option<Vec<TypeHierarchyItem>>> {
        type_hierarchy::prepare_type_hierarchy(self, params).await
    }

    #[tracing::instrument(level = "debug", skip(self, params))]
    async fn supertypes(
        &self,
        params: TypeHierarchySupertypesParams,
    ) -> Result<Option<Vec<TypeHierarchyItem>>> {
        type_hierarchy::supertypes(self, params).await
    }

    #[tracing::instrument(level = "debug", skip(self, params))]
    async fn subtypes(
        &self,
        params: TypeHierarchySubtypesParams,
    ) -> Result<Option<Vec<TypeHierarchyItem>>> {
        type_hierarchy::subtypes(self, params).await
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(
            uri = %params.text_document.uri,
            diagnostics = params.context.diagnostics.len()
        )
    )]
    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri.to_string();

        let Some(doc) = self.salsa_doc(&uri).await else {
            return Ok(None);
        };

        if uri.ends_with("sysml.toml") {
            let actions = manifest_language_features::code_actions_for_manifest(
                &uri,
                &doc.content,
                &params.context.diagnostics,
            );
            return Ok(if actions.is_empty() {
                None
            } else {
                Some(actions)
            });
        }

        let actions = code_actions::generate_code_actions(
            &self.service,
            &uri,
            &params.context.diagnostics,
            &params.range,
        );

        Ok(if actions.is_empty() {
            None
        } else {
            Some(actions)
        })
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, params),
        fields(uri = %params.text_document.uri)
    )]
    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();
        let edits = formatting::format_document(&self.service, &uri, &params.options);
        Ok(if edits.is_empty() { None } else { Some(edits) })
    }
}

// --- Helper methods for new LSP features ---

impl SysmlLanguageServer {
    /// Build signature help based on text and optional CST context before the cursor.
    fn build_signature_help(
        text_before: &str,
        syntax_ctx: Option<&CursorSyntaxContext>,
    ) -> Vec<SignatureInformation> {
        if let Some(ctx) = syntax_ctx {
            // Avoid noisy signatures in string/comment/import contexts.
            if ctx.in_comment_or_string() || ctx.in_import_decl() {
                return Vec::new();
            }
        }

        let trimmed = text_before.trim_end();

        // Match against known declaration patterns
        let patterns: &[(&str, &str, &[ParameterInformation])] = &[
            (
                "part def",
                "part def <name> [specializes <supertype>] { <body> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the part definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "action def",
                "action def <name> (in <params>, out <results>) { <body> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the action definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "state def",
                "state def <name> { entry; do; exit; <states> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the state definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "port def",
                "port def <name> { <features> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the port definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "item def",
                "item def <name> { <features> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the item definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "requirement def",
                "requirement def <name> { doc /* ... */ ; subject <subject>; require constraint <constraints>; }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the requirement definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "constraint def",
                "constraint def <name> { <constraint-expression> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the constraint definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "attribute def",
                "attribute def <name> { <features> }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the attribute definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "connection def",
                "connection def <name> { end <source>; end <target>; }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the connection definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "interface def",
                "interface def <name> { end <port1>; end <port2>; }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the interface definition".to_owned(),
                        )),
                    },
                ],
            ),
            (
                "allocation def",
                "allocation def <name> { end <source>; end <target>; }",
                &[
                    ParameterInformation {
                        label: ParameterLabel::Simple("<name>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Name of the allocation definition".to_owned(),
                        )),
                    },
                ],
            ),
        ];

        for &(keyword, label, params) in patterns {
            if trimmed.ends_with(keyword) || trimmed.ends_with(&format!("{} ", keyword)) {
                return vec![SignatureInformation {
                    label: label.to_owned(),
                    documentation: Some(Documentation::String(format!(
                        "SysML {} declaration",
                        keyword
                    ))),
                    parameters: Some(params.to_vec()),
                    active_parameter: Some(0),
                }];
            }
        }

        // Check for transition pattern
        if trimmed.ends_with("transition") || trimmed.ends_with("transition ") {
            return vec![SignatureInformation {
                label: "transition <source> then <target> [if <guard>] [do <effect>]".to_owned(),
                documentation: Some(Documentation::String(
                    "State transition declaration".to_owned(),
                )),
                parameters: Some(vec![
                    ParameterInformation {
                        label: ParameterLabel::Simple("<source>".to_owned()),
                        documentation: Some(Documentation::String("Source state".to_owned())),
                    },
                    ParameterInformation {
                        label: ParameterLabel::Simple("<target>".to_owned()),
                        documentation: Some(Documentation::String(
                            "Target state after 'then'".to_owned(),
                        )),
                    },
                ]),
                active_parameter: Some(0),
            }];
        }

        Vec::new()
    }
}

/// Convert an OutlineItem to a DocumentSymbol.
pub(crate) fn outline_item_to_document_symbol(
    item: &sysml_parser_incremental::OutlineItem,
    content: &str,
) -> DocumentSymbol {
    let range = LspRange::from_span(&item.span, content);
    let lsp_range = tower_lsp::lsp_types::Range {
        start: tower_lsp::lsp_types::Position {
            line: range.start.line,
            character: range.start.character,
        },
        end: tower_lsp::lsp_types::Position {
            line: range.end.line,
            character: range.end.character,
        },
    };
    let children = if item.children.is_empty() {
        None
    } else {
        Some(
            item.children
                .iter()
                .map(|child| outline_item_to_document_symbol(child, content))
                .collect(),
        )
    };
    #[allow(deprecated)]
    DocumentSymbol {
        name: item.name.clone(),
        detail: None,
        kind: SymbolKind::PACKAGE,
        tags: None,
        deprecated: None,
        range: lsp_range,
        selection_range: lsp_range,
        children,
    }
}

/// Create an LSP service with a fresh, empty `SysmlService`.
pub fn create_service() -> (LspService<SysmlLanguageServer>, tower_lsp::ClientSocket) {
    LspService::new(SysmlLanguageServer::new)
}

/// Create an LSP service reusing an existing `SysmlService`.
///
/// Used by the `/lsp` WebSocket bridge so LSP `did_change` and REST
/// `sysml.*` commands see the same salsa `AnalysisHost`.
pub fn create_service_with(
    service: Arc<SysmlService>,
) -> (LspService<SysmlLanguageServer>, tower_lsp::ClientSocket) {
    LspService::new(move |client| SysmlLanguageServer::new_with_service(client, service.clone()))
}

/// Run the LSP server on stdin/stdout.
pub async fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = create_service();
    Server::new(stdin, stdout, socket).serve(service).await;
}

/// Read file content for a URI, for cross-file range calculations.
///
/// Attempts to parse the URI as a file path and read from disk.
/// Returns `None` if the URI can't be resolved or the file can't be read.
pub(crate) fn read_file_content_for_uri(uri: &str) -> Option<String> {
    let path = parse_uri(uri)
        .and_then(|url| url.to_file_path().ok())
        .or_else(|| {
            let candidate = PathBuf::from(uri);
            if candidate.is_absolute() {
                Some(candidate)
            } else {
                None
            }
        })?;
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod feature_tests;

#[cfg(test)]
mod integration_tests;

/// In-process tower-lsp test harness (`TestServer` + `TestServerOptions`).
///
/// Public under `test` OR the explicit non-default `test-harness` feature so
/// external integration crates (e.g. `sysml-spec-tests`) drive the real LSP
/// through one supported harness instead of hand-rolling their own tower-lsp
/// service/socket/auto-responder wiring.
#[cfg(any(test, feature = "test-harness"))]
pub mod test_harness;

#[cfg(test)]
mod protocol_tests;

#[cfg(test)]
mod protocol_phase1_tests;

#[cfg(test)]
mod utils_tests;

#[cfg(test)]
mod library_conformance_tests;

#[cfg(test)]
mod import_resolution_lsp_tests;

#[cfg(test)]
mod commands_fail_hard_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use sysml_core::{Element, ElementKind, ModelGraph, Value};
    use tower_lsp::lsp_types::{DiagnosticSeverity, Position};
    use sysml_id::ElementId;
    use sysml_span::{Diagnostic as SysmlDiagnostic, Span};

    use crate::diagnostics::to_lsp_diagnostic;

    #[test]
    fn service_creation() {
        let (_service, _socket) = create_service();
    }

    #[test]
    fn test_find_library_config() {
        // May or may not find config depending on environment
        let _ = workspace::find_library_config();
    }

    // ── Position conversion tests ──────────────────────────────────────

    #[test]
    fn position_to_offset_start() {
        let source = "line one\nline two\nline three";
        let pos = Position {
            line: 0,
            character: 0,
        };
        assert_eq!(position_to_offset(&pos, source), 0);
    }

    #[test]
    fn position_to_offset_second_line() {
        let source = "line one\nline two\nline three";
        let pos = Position {
            line: 1,
            character: 0,
        };
        assert_eq!(position_to_offset(&pos, source), 9); // "line one\n" = 9 bytes
    }

    #[test]
    fn position_to_offset_end_of_line() {
        let source = "line one\nline two\nline three";
        let pos = Position {
            line: 0,
            character: 8,
        };
        assert_eq!(position_to_offset(&pos, source), 8); // end of "line one"
    }

    #[test]
    fn position_to_offset_past_end() {
        let source = "short";
        let pos = Position {
            line: 5,
            character: 0,
        };
        assert_eq!(position_to_offset(&pos, source), source.len());
    }

    #[test]
    fn position_to_offset_utf16_multibyte() {
        // Euro sign is 3 bytes in UTF-8, 1 unit in UTF-16
        let source = "a\u{20AC}b"; // "a<euro>b"
        let pos = Position {
            line: 0,
            character: 2,
        }; // after 'a' and euro sign
           // 'a' = 1 byte, euro = 3 bytes, so offset should be 4
        assert_eq!(position_to_offset(&pos, source), 4);
    }

    // ── Offset to position tests ───────────────────────────────────────

    #[test]
    fn offset_to_position_start() {
        let source = "line one\nline two";
        let pos = offset_to_position(0, source);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 0);
    }

    #[test]
    fn offset_to_position_newline_boundary() {
        let source = "line one\nline two";
        let pos = offset_to_position(9, source); // first char of "line two"
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 0);
    }

    #[test]
    fn offset_to_position_mid_line() {
        let source = "line one\nline two";
        let pos = offset_to_position(12, source); // 'e' in "line two" (9 + 3)
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 3);
    }

    #[test]
    fn offset_to_position_end_of_file() {
        let source = "abc";
        let pos = offset_to_position(3, source);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 3);
    }

    // ── Range conversion tests ─────────────────────────────────────────

    #[test]
    fn range_to_lsp_range_single_line() {
        let source = "part engine : Engine;";
        let range = range_to_lsp_range(5, 11, source); // "engine"
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 5);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 11);
    }

    #[test]
    fn range_to_lsp_range_multiline() {
        // "package Pkg {\n  part a;\n}" = 25 bytes
        // line 0: "package Pkg {" (13 bytes + newline)
        // line 1: "  part a;" (9 bytes + newline)
        // line 2: "}"
        let source = "package Pkg {\n  part a;\n}";
        let range = range_to_lsp_range(0, source.len(), source);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 2);
        assert_eq!(range.end.character, 1);
    }

    // ── Position roundtrip test ────────────────────────────────────────

    #[test]
    fn position_roundtrip() {
        let source = "line one\nline two\nline three";
        let original = Position {
            line: 1,
            character: 5,
        };
        let offset = position_to_offset(&original, source);
        let back = offset_to_position(offset, source);
        assert_eq!(back.line, original.line);
        assert_eq!(back.character, original.character);
    }

    // ── Diagnostic conversion tests ────────────────────────────────────

    #[test]
    fn to_lsp_diagnostic_error() {
        let source = "part engine : Engine;";
        let span = Span::new("file:///test.sysml", 0, 4);
        let diag = SysmlDiagnostic::error("test error").with_span(span);
        let lsp = to_lsp_diagnostic(&diag, source);
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(lsp.message, "test error");
    }

    #[test]
    fn to_lsp_diagnostic_warning() {
        let source = "part a;";
        let diag = SysmlDiagnostic::warning("test warning").with_span(Span::new(
            "file:///test.sysml",
            0,
            4,
        ));
        let lsp = to_lsp_diagnostic(&diag, source);
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn to_lsp_diagnostic_no_span() {
        let source = "";
        let diag = SysmlDiagnostic::error("no span error");
        let lsp = to_lsp_diagnostic(&diag, source);
        assert_eq!(lsp.message, "no span error");
        // Should still produce a valid diagnostic even without span
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn to_lsp_diagnostic_with_notes() {
        let source = "part a;";
        let diag = SysmlDiagnostic::error("main error")
            .with_span(Span::new("file:///test.sysml", 0, 4))
            .with_note("hint: check types");
        let lsp = to_lsp_diagnostic(&diag, source);
        assert_eq!(lsp.message, "main error");
    }

    // ── URI parsing tests ──────────────────────────────────────────────

    #[test]
    fn parse_uri_file_scheme() {
        let result = parse_uri("file:///home/test.sysml");
        assert!(result.is_some());
        assert_eq!(result.unwrap().scheme(), "file");
    }

    #[test]
    fn parse_uri_plain_path() {
        let result = parse_uri("/home/test.sysml");
        assert!(result.is_some());
        let url = result.unwrap();
        assert_eq!(url.scheme(), "file");
    }

    // ── resolve_goto_target tests ──────────────────────────────────────

    fn make_graph_with_typing() -> (ModelGraph, ElementId, ElementId, ElementId) {
        let mut graph = ModelGraph::new();

        // Create a definition element (the target)
        let def_id = ElementId::new_v4();
        let def = Element::new(def_id.clone(), ElementKind::PartDefinition)
            .with_name("Engine")
            .with_span(Span::new("file:///test.sysml", 0, 20));
        graph.add_element(def);

        // Create a usage element (owner of the typing relationship)
        let usage_id = ElementId::new_v4();
        let usage = Element::new(usage_id.clone(), ElementKind::PartUsage)
            .with_name("engine")
            .with_span(Span::new("file:///test.sysml", 30, 50));
        graph.add_element(usage);

        // Create a FeatureTyping relationship with resolved reference
        let typing_id = ElementId::new_v4();
        let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
            .with_span(Span::new("file:///test.sysml", 40, 46));
        typing.owner = Some(usage_id.clone());
        typing
            .props
            .insert(Cow::Borrowed("type"), Value::Ref(def_id.clone()));
        graph.add_element(typing);

        (graph, typing_id, def_id, usage_id)
    }

    #[test]
    fn resolve_goto_target_feature_typing_resolved() {
        let (graph, typing_id, def_id, _) = make_graph_with_typing();
        let typing_elem = graph.get_element(&typing_id).unwrap();
        let target = resolve_goto_target(typing_elem, &graph);
        assert_eq!(target.id, def_id);
        assert_eq!(target.name.as_deref(), Some("Engine"));
    }

    #[test]
    fn resolve_goto_target_unresolved_name() {
        let mut graph = ModelGraph::new();

        let def_id = ElementId::new_v4();
        let def = Element::new(def_id.clone(), ElementKind::PartDefinition)
            .with_name("Vehicle")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(def);

        let typing_id = ElementId::new_v4();
        let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
            .with_span(Span::new("file:///test.sysml", 20, 30));
        typing.props.insert(
            Cow::Borrowed("unresolved_type"),
            Value::String("Vehicle".to_string()),
        );
        graph.add_element(typing);

        let typing_elem = graph.get_element(&typing_id).unwrap();
        let target = resolve_goto_target(typing_elem, &graph);
        assert_eq!(target.id, def_id);
    }

    #[test]
    fn resolve_goto_target_unresolved_simple_name_lookup() {
        // Tests the fallback path: when resolve_qname fails, the function
        // falls through to simple name lookup among definition elements.
        let mut graph = ModelGraph::new();

        let def_id = ElementId::new_v4();
        let def = Element::new(def_id.clone(), ElementKind::PartDefinition)
            .with_name("Motor")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(def);

        let typing_id = ElementId::new_v4();
        let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
            .with_span(Span::new("file:///test.sysml", 20, 30));
        // Use a simple name so the simple name lookup path can find it
        typing.props.insert(
            Cow::Borrowed("unresolved_type"),
            Value::String("Motor".to_string()),
        );
        graph.add_element(typing);

        let typing_elem = graph.get_element(&typing_id).unwrap();
        let target = resolve_goto_target(typing_elem, &graph);
        assert_eq!(target.id, def_id);
    }

    #[test]
    fn resolve_goto_target_specialization() {
        let mut graph = ModelGraph::new();

        let general_id = ElementId::new_v4();
        let general = Element::new(general_id.clone(), ElementKind::PartDefinition)
            .with_name("Base")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(general);

        let spec_id = ElementId::new_v4();
        let mut spec = Element::new(spec_id.clone(), ElementKind::Specialization)
            .with_span(Span::new("file:///test.sysml", 20, 30));
        spec.props
            .insert(Cow::Borrowed("general"), Value::Ref(general_id.clone()));
        graph.add_element(spec);

        let spec_elem = graph.get_element(&spec_id).unwrap();
        let target = resolve_goto_target(spec_elem, &graph);
        assert_eq!(target.id, general_id);
    }

    #[test]
    fn resolve_goto_target_subsetting() {
        let mut graph = ModelGraph::new();

        let feature_id = ElementId::new_v4();
        let feature = Element::new(feature_id.clone(), ElementKind::PartUsage)
            .with_name("baseFeature")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(feature);

        let sub_id = ElementId::new_v4();
        let mut sub = Element::new(sub_id.clone(), ElementKind::Subsetting).with_span(Span::new(
            "file:///test.sysml",
            20,
            30,
        ));
        sub.props.insert(
            Cow::Borrowed("subsettedFeature"),
            Value::Ref(feature_id.clone()),
        );
        graph.add_element(sub);

        let sub_elem = graph.get_element(&sub_id).unwrap();
        let target = resolve_goto_target(sub_elem, &graph);
        assert_eq!(target.id, feature_id);
    }

    #[test]
    fn resolve_goto_target_redefinition() {
        let mut graph = ModelGraph::new();

        let feature_id = ElementId::new_v4();
        let feature = Element::new(feature_id.clone(), ElementKind::PartUsage)
            .with_name("origFeature")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(feature);

        let redef_id = ElementId::new_v4();
        let mut redef = Element::new(redef_id.clone(), ElementKind::Redefinition)
            .with_span(Span::new("file:///test.sysml", 20, 30));
        redef.props.insert(
            Cow::Borrowed("redefinedFeature"),
            Value::Ref(feature_id.clone()),
        );
        graph.add_element(redef);

        let redef_elem = graph.get_element(&redef_id).unwrap();
        let target = resolve_goto_target(redef_elem, &graph);
        assert_eq!(target.id, feature_id);
    }

    #[test]
    fn resolve_goto_target_reference_subsetting() {
        let mut graph = ModelGraph::new();

        let feature_id = ElementId::new_v4();
        let feature = Element::new(feature_id.clone(), ElementKind::PartUsage)
            .with_name("refFeature")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(feature);

        let refsub_id = ElementId::new_v4();
        let mut refsub = Element::new(refsub_id.clone(), ElementKind::ReferenceSubsetting)
            .with_span(Span::new("file:///test.sysml", 20, 30));
        refsub.props.insert(
            Cow::Borrowed("referencedFeature"),
            Value::Ref(feature_id.clone()),
        );
        graph.add_element(refsub);

        let refsub_elem = graph.get_element(&refsub_id).unwrap();
        let target = resolve_goto_target(refsub_elem, &graph);
        assert_eq!(target.id, feature_id);
    }

    #[test]
    fn resolve_goto_target_fallback_to_owner() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage)
            .with_name("myPart")
            .with_span(Span::new("file:///test.sysml", 0, 20));
        graph.add_element(owner);

        // FeatureTyping with no resolved or unresolved props, but has an owner
        let typing_id = ElementId::new_v4();
        let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
            .with_span(Span::new("file:///test.sysml", 10, 15));
        typing.owner = Some(owner_id.clone());
        graph.add_element(typing);

        let typing_elem = graph.get_element(&typing_id).unwrap();
        let target = resolve_goto_target(typing_elem, &graph);
        assert_eq!(target.id, owner_id);
    }

    #[test]
    fn resolve_goto_target_non_relationship_passthrough() {
        let mut graph = ModelGraph::new();

        let part_id = ElementId::new_v4();
        let part = Element::new(part_id.clone(), ElementKind::PartUsage)
            .with_name("myPart")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(part);

        let part_elem = graph.get_element(&part_id).unwrap();
        let target = resolve_goto_target(part_elem, &graph);
        assert_eq!(target.id, part_id); // Returns the element itself
    }

    // ── Symbol kind mapping tests ──────────────────────────────────────

    #[test]
    fn element_kind_to_symbol_kind_package() {
        assert_eq!(
            element_kind_to_symbol_kind(&ElementKind::Package),
            SymbolKind::PACKAGE
        );
        assert_eq!(
            element_kind_to_symbol_kind(&ElementKind::LibraryPackage),
            SymbolKind::PACKAGE
        );
    }

    #[test]
    fn element_kind_to_symbol_kind_definitions_and_usages() {
        assert_eq!(
            element_kind_to_symbol_kind(&ElementKind::PartDefinition),
            SymbolKind::CLASS
        );
        assert_eq!(
            element_kind_to_symbol_kind(&ElementKind::ActionDefinition),
            SymbolKind::FUNCTION
        );
        assert_eq!(
            element_kind_to_symbol_kind(&ElementKind::PartUsage),
            SymbolKind::FIELD
        );
        assert_eq!(
            element_kind_to_symbol_kind(&ElementKind::AttributeUsage),
            SymbolKind::PROPERTY
        );
    }

    // ── Semantic tokens builder tests ──────────────────────────────────

    #[test]
    fn semantic_tokens_builder_single_token() {
        let source = "part engine;";
        let mut builder = SemanticTokensBuilder::new(source);
        builder.add_token(5, 11, 4, 0); // "engine" as PROPERTY
        let tokens = builder.build();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].delta_line, 0);
        assert_eq!(tokens[0].delta_start, 5);
        assert_eq!(tokens[0].length, 6); // "engine" = 6 chars
        assert_eq!(tokens[0].token_type, 4);
    }

    #[test]
    fn semantic_tokens_builder_sorted_output() {
        let source = "part engine : Engine;";
        let mut builder = SemanticTokensBuilder::new(source);
        // Add out of order
        builder.add_token(14, 20, 1, 0); // "Engine" (TYPE)
        builder.add_token(5, 11, 4, 0); // "engine" (PROPERTY)
        let tokens = builder.build();
        assert_eq!(tokens.len(), 2);
        // First token should be "engine" (earlier in source)
        assert_eq!(tokens[0].delta_start, 5);
        // Second token delta_start is relative to first on same line: 14 - 5 = 9
        assert_eq!(tokens[1].delta_start, 9);
    }

    #[test]
    fn semantic_tokens_builder_delta_encoding_same_line() {
        let source = "a b c";
        let mut builder = SemanticTokensBuilder::new(source);
        builder.add_token(0, 1, 1, 0); // "a"
        builder.add_token(2, 3, 1, 0); // "b"
        builder.add_token(4, 5, 1, 0); // "c"
        let tokens = builder.build();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].delta_line, 0);
        assert_eq!(tokens[0].delta_start, 0);
        assert_eq!(tokens[1].delta_line, 0);
        assert_eq!(tokens[1].delta_start, 2); // relative to previous
        assert_eq!(tokens[2].delta_line, 0);
        assert_eq!(tokens[2].delta_start, 2); // relative to previous
    }

    #[test]
    fn semantic_tokens_builder_multiline() {
        let source = "aaa\nbbb";
        let mut builder = SemanticTokensBuilder::new(source);
        builder.add_token(0, 3, 1, 0); // "aaa" on line 0
        builder.add_token(4, 7, 1, 0); // "bbb" on line 1
        let tokens = builder.build();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].delta_line, 0);
        assert_eq!(tokens[0].delta_start, 0);
        assert_eq!(tokens[1].delta_line, 1); // new line
        assert_eq!(tokens[1].delta_start, 0); // start of new line
    }

}
