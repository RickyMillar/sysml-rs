//! Workspace management: library loading, workspace indexing, server handle.
//!
//! Contains the background library loading pipeline (with caching and progress),
//! workspace file discovery and indexing, and the SysmlLanguageServerHandle for
//! spawning background tasks.

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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};

use tokio::sync::{oneshot, RwLock};
use tower_lsp::lsp_types::request;
use tower_lsp::lsp_types::{Url, NumberOrString, WorkDoneProgressCreateParams, ProgressParams, ProgressParamsValue, WorkDoneProgress, WorkDoneProgressBegin, notification, WorkDoneProgressReport, WorkDoneProgressEnd};
use tower_lsp::Client;

use sysml_service::library_cache::LibraryCache;
use sysml_service::progress::{LibraryPhase, ProgressEvent};
use crate::manifest_diagnostics;
use crate::telemetry_control;
use crate::utils::parse_uri;
use crate::ux_messages;
use crate::SysmlLanguageServer;

// P4-closeout retired `IndexFileError` / `find_project_for_path` /
// `is_within_allowed_roots`: the workspace indexer now delegates discovery
// and project-tagging to `SysmlService::open_context`, which owns
// canonicalization, the file walk, nested-manifest isolation, and project
// pid assignment.

pub(crate) fn canonical_file_uri(uri: &str) -> Option<String> {
    let parsed = parse_uri(uri)?;
    if parsed.scheme() != "file" {
        return None;
    }
    let path = parsed.to_file_path().ok()?;
    let canonical = path.canonicalize().ok()?;
    Url::from_file_path(canonical).ok().map(|u| u.to_string())
}

/// Find library configuration from environment or default paths.
///
/// Re-exported from `sysml_service::library_cache::find_library_config`
/// for backwards-compat call sites in `commands.rs` / this module.
pub(crate) use sysml_service::library_cache::{
    find_library_config, find_library_config_with_override,
};

/// Handle for background tasks that need access to server state.
pub(crate) struct SysmlLanguageServerHandle {
    pub(crate) client: Client,
    pub(crate) service: Arc<sysml_service::SysmlService>,
    pub(crate) features: Arc<RwLock<crate::types::FeatureFlags>>,
    pub(crate) analysis_host: Arc<std::sync::Mutex<sysml_ide_db::AnalysisHost>>,
}

impl SysmlLanguageServerHandle {
    #[tracing::instrument(level = "info", skip(self), fields(task_id = %task_id))]
    pub(crate) async fn load_library_background(&self, task_id: String) {
        // Check if already loading/loaded — derive lifecycle from
        // `service.readiness_for(_)`. The service overlays Loading /
        // Failed onto the host-derived Loaded/Unloaded state via the
        // ProgressBus side-channel (P-RA4). If the host already has a
        // library graph, or the bus already marked us Loading, bail.
        {
            use sysml_service::readiness::LibraryReadiness;
            let r = self.service.readiness_for("__library__");
            if matches!(r.library, LibraryReadiness::Loaded | LibraryReadiness::Loading) {
                return;
            }
        }

        // Mark as loading via the bus — overlays Loading onto
        // `readiness_for` and notifies subscribers (LSP adapter
        // forwards to `$/progress` + `window/logMessage`).
        self.service.publish_progress(ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Loading,
            done: 0,
            total: 0,
            detail: String::new(),
        });

        // Send progress notification: Begin
        let progress_token = NumberOrString::String("sysml/library-loading".to_owned());
        let _ = self
            .client
            .send_request::<request::WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
                token: progress_token.clone(),
            })
            .await;

        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token: progress_token.clone(),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                    WorkDoneProgressBegin {
                        title: "Loading SysML Standard Library".to_owned(),
                        cancellable: Some(false),
                        message: Some("Searching for library files...".to_owned()),
                        percentage: Some(0),
                    },
                )),
            })
            .await;

        // If library loading takes unusually long, surface that we're running
        // in a degraded mode instead of silently stalling.
        let (slow_warn_done_tx, slow_warn_done_rx) = oneshot::channel::<()>();
        let slow_warn_client = self.client.clone();
        let slow_warn_token = progress_token.clone();
        let slow_warn_service = self.service.clone();
        let slow_warn_task_id = task_id.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(4)) => {
                    let still_loading = matches!(
                        slow_warn_service.readiness_for("__library__").library,
                        sysml_service::readiness::LibraryReadiness::Loading
                    );
                    if still_loading {
                        slow_warn_client
                            .send_notification::<notification::Progress>(ProgressParams {
                                token: slow_warn_token,
                                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                                    WorkDoneProgressReport {
                                        cancellable: Some(false),
                                        message: Some("Library loading is taking longer than expected. Running with limited type resolution for now...".to_owned()),
                                        percentage: Some(20),
                                    },
                                )),
                            })
                            .await;
                        ux_messages::warn(
                            &slow_warn_client,
                            "SysML library load is still running. Type completion and cross-file features are temporarily limited.",
                        )
                        .await;
                        tracing::warn!(
                            task_id = %slow_warn_task_id,
                            threshold_ms = 4000u64,
                            "library loading exceeded slow threshold"
                        );
                    }
                }
                _ = slow_warn_done_rx => {}
            }
        });
        let mut slow_warn_done_tx = Some(slow_warn_done_tx);

        let override_path = {
            let features = self.features.read().await;
            features.library_path_override.clone()
        };
        let config = match find_library_config_with_override(override_path.as_deref()) {
            Some(c) => c,
            None => {
                self.service.publish_progress(ProgressEvent::LibraryLoad {
                    phase: LibraryPhase::Failed,
                    done: 0,
                    total: 0,
                    detail: "Standard library not found".to_owned(),
                });
                // End progress
                self.client
                    .send_notification::<notification::Progress>(ProgressParams {
                        token: progress_token,
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                            WorkDoneProgressEnd {
                                message: Some("Library not found".to_owned()),
                            },
                        )),
                    })
                    .await;
                // User-visible warning with remediation
                ux_messages::show_warn(
                    &self.client,
                    "SysML standard library not found. Cross-file features disabled. Set SYSML_LIBRARY_PATH environment variable or install to ./libraries/standard",
                ).await;
                if let Some(tx) = slow_warn_done_tx.take() {
                    let _ = tx.send(());
                }
                return;
            }
        };

        // Progress: checking cache
        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token: progress_token.clone(),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                    WorkDoneProgressReport {
                        cancellable: Some(false),
                        message: Some("Checking library cache...".to_owned()),
                        percentage: Some(10),
                    },
                )),
            })
            .await;

        let cache = LibraryCache::new(config);
        let start = Instant::now();

        // Progress: loading via unified salsa path
        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token: progress_token.clone(),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                    WorkDoneProgressReport {
                        cancellable: Some(false),
                        message: Some("Loading standard library (cache or parse)...".to_owned()),
                        percentage: Some(30),
                    },
                )),
            })
            .await;

        // Run the unified load_via_host on a blocking thread because it does
        // synchronous filesystem I/O and holds the AnalysisHost mutex. The
        // helper handles cache-hit (fast deserialize + set_library) and
        // cache-miss (salsa-tracked enable_stdlib + save_cache) uniformly,
        // so the LSP, MCP and CLI now converge on a single code path.
        let load_task_id = format!("{task_id}:load-via-host");
        let load_task_id_for_blocking = load_task_id.clone();
        let host_arc_blocking = self.analysis_host.clone();
        let load_result = tokio::task::spawn_blocking(move || {
            tracing::debug!(
                task_id = %load_task_id_for_blocking,
                "running library load_via_host in spawn_blocking"
            );
            let mut host = host_arc_blocking.lock().unwrap();
            cache.load_via_host(&mut host)
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(task_id = %load_task_id, "Library load task panicked: {e}");
            Err(sysml_parser_trait::library::LibraryLoadError::ReadError {
                path: std::path::PathBuf::from("<spawn_blocking>"),
                source: std::io::Error::other(format!("task panicked: {e}")),
            })
        });

        match load_result {
            Ok(library) => {
                let elapsed = start.elapsed();
                let element_count = library.element_count();
                let source_detail = "salsa".to_owned();

                // Progress: registering
                self.client
                    .send_notification::<notification::Progress>(ProgressParams {
                        token: progress_token.clone(),
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                            WorkDoneProgressReport {
                                cancellable: Some(false),
                                message: Some("Registering library types...".to_owned()),
                                percentage: Some(90),
                            },
                        )),
                    })
                    .await;

                // Library is already wired into salsa by load_via_host
                // (set_library on cache hit, enable_stdlib_with_path on miss).
                // Publish the Loaded lifecycle so subscribers (and the
                // service-tracked override behind `readiness_for`)
                // observe the terminus.
                self.service.publish_progress(ProgressEvent::LibraryLoad {
                    phase: LibraryPhase::Loaded,
                    done: element_count as u32,
                    total: element_count as u32,
                    detail: format!("{element_count} elements in {elapsed:?} ({source_detail})"),
                });

                // Library availability changes diagnostic confidence for open docs.
                // Ask the client to refresh diagnostics so stale unresolved-type
                // results can be recalculated.
                match self.client.workspace_diagnostic_refresh().await {
                    Ok(()) => {
                        tracing::info!(
                            task_id = %task_id,
                            "requested workspace diagnostic refresh after library load"
                        );
                    }
                    Err(e) => {
                        tracing::debug!(
                            task_id = %task_id,
                            error = %e,
                            "workspace diagnostic refresh request failed or unsupported"
                        );
                    }
                }

                // End progress: success
                self.client
                    .send_notification::<notification::Progress>(ProgressParams {
                        token: progress_token,
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                            WorkDoneProgressEnd {
                                message: Some(format!(
                                    "Loaded {} elements in {:?} ({})",
                                    element_count, elapsed, source_detail
                                )),
                            },
                        )),
                    })
                    .await;

                ux_messages::info(
                    &self.client,
                    format!(
                        "Loaded standard library: {} elements in {:?} ({})",
                        element_count, elapsed, source_detail
                    ),
                )
                .await;
            }
            Err(err) => {
                self.service.publish_progress(ProgressEvent::LibraryLoad {
                    phase: LibraryPhase::Failed,
                    done: 0,
                    total: 0,
                    detail: err.to_string(),
                });
                // End progress: failure
                self.client
                    .send_notification::<notification::Progress>(ProgressParams {
                        token: progress_token,
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                            WorkDoneProgressEnd {
                                message: Some("Failed to load library".to_owned()),
                            },
                        )),
                    })
                    .await;
                // User-visible error with remediation
                ux_messages::show_error(
                    &self.client,
                    format!("Failed to load SysML standard library: {}. Cross-file navigation and type resolution will be limited.", err),
                ).await;
            }
        }

        if let Some(tx) = slow_warn_done_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl SysmlLanguageServer {
    /// Rediscover projects/workspace state and restart background indexing without requiring
    /// an editor window reload.
    pub(crate) async fn rediscover_workspace_state(&self, reason: &str) {
        let dep_trace = telemetry_control::dependency_trace_enabled();
        let rediscovery_started_at = Instant::now();
        let roots = self.workspace_index.workspace_roots.read().await.clone();
        if roots.is_empty() {
            return;
        }
        if dep_trace {
            tracing::info!(
                reason,
                root_count = roots.len(),
                "dependency trace: workspace rediscovery started"
            );
        }

        // Capture in-memory unsaved-edit buffers BEFORE the host reset.
        // Service.workspace_refresh re-reads project files from disk; any
        // unsaved editor buffers must be restored by us after the call.
        // Dedupe canonical URI aliases so stale indexed copies don't
        // surface as separate files after restore.
        let open_doc_aliases: std::collections::HashSet<String> = self
            .open_documents
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        let open_buffers: Vec<(String, String)> = {
            let host = self.analysis_host.lock().unwrap();
            let analysis = host.analysis();
            let mut deduped: HashMap<String, (String, String, bool)> = HashMap::new();

            for file_id in host.files().file_ids() {
                let Some(uri) = host.files().uri(file_id).map(ToString::to_string) else {
                    continue;
                };
                if !open_doc_aliases.contains(&uri) {
                    continue;
                }
                let Some(source) = host.source_file(file_id) else {
                    continue;
                };
                let text = analysis.file_text(source).to_owned();
                let canonical_key = canonical_file_uri(&uri).unwrap_or_else(|| uri.clone());
                let uri_is_noncanonical_alias = canonical_key != uri;
                match deduped.get(&canonical_key) {
                    Some((_, _, existing_noncanonical)) if *existing_noncanonical => {}
                    _ => {
                        deduped.insert(canonical_key, (uri, text, uri_is_noncanonical_alias));
                    }
                }
            }

            deduped
                .into_values()
                .map(|(uri, text, _)| (uri, text))
                .collect()
        };
        if dep_trace {
            tracing::info!(
                reason,
                open_buffer_count = open_buffers.len(),
                open_alias_count = open_doc_aliases.len(),
                "dependency trace: captured open buffers for rediscovery"
            );
        }

        // Delegate the structural reset (project discovery + ID
        // assignment + host reset + project_registry sync + stdlib
        // enable) to the service. The LSP shell owns buffer
        // preservation + indexer + diagnostics + UX messages.
        let refresh_result = match self.service.workspace_refresh(&roots, None) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(reason, error = %error, "service workspace_refresh failed");
                return;
            }
        };

        // Rebuild project_roots from the service result so the indexer
        // sees the same deterministic project IDs as the host.
        let project_roots: Vec<(sysml_project::ProjectHandle, PathBuf)> = refresh_result
            .projects
            .iter()
            .filter_map(|p| {
                p.root
                    .as_ref()
                    .map(|root| (sysml_project::ProjectHandle(p.id), PathBuf::from(root)))
            })
            .collect();

        if dep_trace {
            for project in &refresh_result.projects {
                if let Some(root) = &project.root {
                    let canonical = std::path::Path::new(root)
                        .canonicalize()
                        .unwrap_or_else(|_| std::path::PathBuf::from(root));
                    tracing::info!(
                        reason,
                        project_id = project.id,
                        project_name = %project.name,
                        root_raw = %root,
                        root_canonical = %canonical.display(),
                        "dependency trace: rediscovery project root"
                    );
                }
            }
        }

        // Restore in-memory unsaved-edit buffers — service didn't
        // preserve them because the LSP shell owns the open-doc concept.
        // `workspace_refresh` did `*host = AnalysisHost::new()`, wiping the
        // editor-overlay bits, so we re-establish them HERE — content +
        // overlay together, under one lock, BEFORE `index_workspace_files`
        // runs below. The re-index's open_context then sees the overlay
        // and preserves these buffers (set_project_only) instead of
        // clobbering them from disk. (Steward Shape B re-establishment.)
        {
            let mut host = self.analysis_host.lock().unwrap();
            for (uri, text) in &open_buffers {
                match host.find_project_for_uri(uri) {
                    Some(pid) => {
                        if dep_trace {
                            tracing::info!(
                                reason,
                                uri,
                                project_id = pid.0,
                                "dependency trace: restored open buffer in project"
                            );
                        }
                        host.set_file_content_in_project(uri, text.clone(), pid);
                    }
                    None => {
                        if dep_trace {
                            tracing::info!(
                                reason,
                                uri,
                                "dependency trace: restored open buffer without project match"
                            );
                        }
                        host.set_file_content(uri, text.clone());
                    }
                }
                host.set_overlay(uri);
            }
        }

        // Kick off fresh workspace indexing.
        let task_id = self.next_background_task_id("rediscovery-workspace-index");
        let max_files = self.features.read().await.max_index_files;
        let roots_for_task = roots.clone();
        let project_roots_for_task = project_roots.clone();
        if dep_trace {
            tracing::info!(
                reason,
                task_id = %task_id,
                max_files,
                "dependency trace: rediscovery indexing start"
            );
        }
        self.index_workspace_files(task_id, roots_for_task, project_roots_for_task, max_files)
            .await;
        if dep_trace {
            tracing::info!(reason, "dependency trace: rediscovery indexing complete");
        }

        // Re-publish diagnostics for open docs (especially sysml.toml diagnostics).
        for (uri, text) in open_buffers {
            if uri.ends_with("sysml.toml") {
                if let Ok(parsed_uri) = uri.parse::<Url>() {
                    let diagnostics =
                        manifest_diagnostics::validate_manifest_with_context(&text, Some(&uri));
                    self.client
                        .publish_diagnostics(parsed_uri, diagnostics, None)
                        .await;
                }
            } else if uri.ends_with(".sysml") || uri.ends_with(".kerml") {
                self.run_diagnostics_cycle(uri.clone(), None).await;
            }
        }

        self.service.publish_progress(ProgressEvent::Refresh { reason: "workspace-update" });
        ux_messages::info(
            &self.client,
            format!(
                "Workspace updated ({}): {} project(s), {} root(s)",
                reason,
                refresh_result.projects.len(),
                roots.len()
            ),
        )
        .await;
        if dep_trace {
            tracing::info!(
                reason,
                elapsed_ms = rediscovery_started_at.elapsed().as_millis(),
                "dependency trace: workspace rediscovery finished"
            );
        }
    }

    /// Clone for spawning background tasks.
    pub(crate) fn clone_for_spawn(&self) -> SysmlLanguageServerHandle {
        SysmlLanguageServerHandle {
            client: self.client.clone(),
            service: self.service.clone(),
            features: self.features.clone(),
            analysis_host: self.analysis_host.clone(),
        }
    }

    /// Index all .sysml and .kerml files in the workspace roots for cross-file navigation.
    ///
    /// Reads file contents and feeds them into the salsa database. Parsing and
    /// indexing happens lazily via salsa's memoized queries when WorkspaceSnapshot
    /// is built.
    ///
    /// ## Project-aware indexing
    ///
    /// When `project_roots` is provided, each file is associated with the project
    /// whose root directory contains it. This enables workspace-aware cross-file
    /// resolution (e.g., `import Definitions::*` resolving across files in the
    /// same project).
    #[tracing::instrument(
        level = "info",
        skip(self, project_roots),
        fields(task_id = %task_id, roots = roots.len(), projects = project_roots.len(), max_files)
    )]
    pub(crate) async fn index_workspace_files(
        &self,
        task_id: String,
        roots: Vec<String>,
        project_roots: Vec<(sysml_project::ProjectHandle, PathBuf)>,
        max_files: u32,
    ) {
        let started_at = Instant::now();
        let roots_count = roots.len();
        let max_files = if max_files == 0 { u32::MAX } else { max_files };
        let mut missing_roots = 0u32;
        let mut open_context_failures = 0u32;

        // Pre-canonicalize workspace roots.
        let workspace_root_paths: Vec<PathBuf> = roots
            .iter()
            .filter_map(|r| PathBuf::from(r).canonicalize().ok())
            .collect();

        // Build the effective scan roots:
        // - all workspace roots
        // - dependency/member project roots discovered from sysml.toml
        //   that aren't already covered by a workspace root.
        // Mirrors the pre-P4-closeout scan-root computation: workspace
        // roots are scanned wholesale; dep/member project roots OUTSIDE
        // any workspace root are added (cache dirs from git/kpar/registry).
        let canonical_project_roots: Vec<(sysml_project::ProjectHandle, PathBuf)> = project_roots
            .iter()
            .filter_map(|(pid, root)| root.canonicalize().ok().map(|c| (*pid, c)))
            .collect();

        let mut scan_roots = workspace_root_paths.clone();
        for (_, project_root) in &canonical_project_roots {
            if scan_roots
                .iter()
                .any(|existing_root| project_root.starts_with(existing_root))
            {
                continue;
            }
            scan_roots.retain(|existing_root| !existing_root.starts_with(project_root));
            scan_roots.push(project_root.clone());
        }

        // Send progress notification (LSP $/progress is preserved as the
        // user-facing channel for VS Code; ProgressEvent is the new
        // transport-agnostic channel).
        let progress_token = NumberOrString::String("sysml/workspace-indexing".to_owned());
        let _ = self
            .client
            .send_request::<request::WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
                token: progress_token.clone(),
            })
            .await;
        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token: progress_token.clone(),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                    WorkDoneProgressBegin {
                        title: "Indexing Workspace".to_owned(),
                        cancellable: Some(false),
                        message: Some("Discovering SysML files...".to_owned()),
                        percentage: Some(0),
                    },
                )),
            })
            .await;

        // =====================================================================
        // Phases 1+2+3 consolidated via SysmlService::open_context.
        // =====================================================================
        // open_context owns discovery, file loading, project-tagging, and
        // ProjectFileSet construction. The LSP shell emits the same
        // multi-phase progress events for back-compat with subscribers.
        //
        // Open editor buffers are preserved WITHOUT a snapshot/restore
        // band-aid: did_open marks each open file as an editor overlay
        // (`host.set_overlay`), and open_context's per-file write checks
        // that flag — overlaid files get `set_project_only` (project tag,
        // buffer kept) instead of a disk overwrite. open_context holds the
        // host lock across its whole file loop and did_open sets
        // content+overlay under that same lock, so the two serialize and
        // the buffer can't be clobbered. (Steward-ruled Shape B, 2026-06-23.)

        // Drive open_context per scan root. Each call is idempotent:
        // re-opening the same folder refreshes the ProjectFileSet without
        // duplicating projects (open_context::has_project_at_path guard).
        let mut total_loaded: HashSet<String> = HashSet::new();
        let mut cap_exceeded = false;
        for root_path in &scan_roots {
            if !root_path.exists() {
                missing_roots += 1;
                tracing::debug!(
                    root = %root_path.display(),
                    "index root does not exist, skipping"
                );
                continue;
            }

            // open_context locks the host briefly inside; call it on a
            // blocking task to keep the async runtime responsive.
            let service = self.service.clone();
            let root_clone = root_path.clone();
            let ctx_result = tokio::task::spawn_blocking(move || {
                service.open_context(sysml_project::discovery::OpenTarget::Folder(root_clone))
            })
            .await;

            match ctx_result {
                Ok(Ok(ctx)) => {
                    if (total_loaded.len() as u32) >= max_files {
                        cap_exceeded = true;
                        break;
                    }
                    for uri in ctx.loaded_uris {
                        if (total_loaded.len() as u32) >= max_files {
                            cap_exceeded = true;
                            break;
                        }
                        total_loaded.insert(uri);
                    }
                    if ctx
                        .diagnostics
                        .iter()
                        .any(|d| d.code.as_deref() == Some("discovery-cap"))
                    {
                        cap_exceeded = true;
                    }
                }
                Ok(Err(e)) => {
                    open_context_failures += 1;
                    tracing::warn!(
                        root = %root_path.display(),
                        error = %e,
                        "open_context failed for index root"
                    );
                }
                Err(join_err) => {
                    open_context_failures += 1;
                    tracing::error!("open_context spawn_blocking panicked: {join_err}");
                }
            }
        }

        // (No buffer restore needed: open_context preserved overlaid open
        // buffers in place via the editor-overlay branch — see the note
        // above. The old snapshot/restore band-aid was racy under
        // did_open-vs-indexer interleaving and is gone.)

        let file_count = total_loaded.len() as u32;
        let total_to_index = file_count;
        let phase_elapsed_ms = started_at.elapsed().as_millis();

        // Re-publish phase 1/2/3 events as a single consolidated burst.
        // P-RA4 subscribers (LSP `window/logMessage`, MCP, REST SSE, CLI
        // stderr) see the same phase enum as before; the consolidation
        // is purely a timing change.
        self.service.publish_progress(ProgressEvent::WorkspaceIndex {
            phase: 1,
            done: total_to_index,
            total: total_to_index,
            detail: format!("discovered in {}ms", phase_elapsed_ms),
        });
        self.service.publish_progress(ProgressEvent::WorkspaceIndex {
            phase: 2,
            done: file_count,
            total: total_to_index,
            detail: format!("loaded into database in {}ms", phase_elapsed_ms),
        });
        ux_messages::info(
            &self.client,
            format!(
                "Workspace indexing: discovered+loaded {} SysML/KerML files (phases 1-2 in {}ms)",
                total_to_index, phase_elapsed_ms
            ),
        )
        .await;

        // Phase 3: count distinct projects from canonical_project_roots
        // plus the default project (registered by open_context for any
        // manifest-less workspace root). open_context already constructed
        // the ProjectFileSets — phase 3 is now just a status emission.
        let project_count: usize = {
            let host = self.analysis_host.lock().unwrap();
            let mut pids: HashSet<sysml_project::ProjectHandle> = HashSet::new();
            for fid in host.files().file_ids() {
                if let Some(pid) = host.files().project_id(fid) {
                    pids.insert(pid);
                }
            }
            pids.len()
        };
        let phase3_elapsed_ms = started_at.elapsed().as_millis();
        self.service.publish_progress(ProgressEvent::WorkspaceIndex {
            phase: 3,
            done: project_count as u32,
            total: project_count as u32,
            detail: format!("built resolution context in {}ms", phase3_elapsed_ms),
        });
        ux_messages::info(
            &self.client,
            format!(
                "Workspace indexing: built resolution context for {} projects (phase 3 in {}ms)",
                project_count, phase3_elapsed_ms
            ),
        )
        .await;

        if cap_exceeded {
            tracing::warn!(
                task_id = %task_id,
                roots_count,
                indexed_files = file_count,
                missing_roots,
                open_context_failures,
                max_files,
                elapsed_ms = started_at.elapsed().as_millis(),
                "workspace indexing reached configured file cap"
            );
            ux_messages::warn(
                &self.client,
                format!("Workspace indexing capped at {} files", max_files),
            )
            .await;
        }

        // End $/progress
        self.client
            .send_notification::<notification::Progress>(ProgressParams {
                token: progress_token,
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(WorkDoneProgressEnd {
                    message: Some(if file_count > 0 {
                        format!("Indexed {} files", file_count)
                    } else {
                        "No SysML files found".to_owned()
                    }),
                })),
            })
            .await;

        tracing::info!(
            task_id = %task_id,
            roots_count,
            missing_roots,
            max_files,
            indexed_files = file_count,
            open_context_failures,
            elapsed_ms = started_at.elapsed().as_millis(),
            "workspace indexing finished"
        );

        // =====================================================================
        // Phase 4: Refresh diagnostics for cross-file resolution
        // =====================================================================
        // Workspace indexing changes the resolution context: ProjectFileSets now
        // exist, enabling cross-file import resolution. Ask the client to pull
        // fresh diagnostics (pull model) and also explicitly push diagnostics
        // for all tracked files (push model / belt-and-suspenders).

        match self.client.workspace_diagnostic_refresh().await {
            Ok(()) => {
                tracing::info!(
                    task_id = %task_id,
                    "requested workspace diagnostic refresh after indexing"
                );
            }
            Err(e) => {
                tracing::debug!(
                    task_id = %task_id,
                    error = %e,
                    "workspace diagnostic refresh request failed or unsupported"
                );
            }
        }

        // Explicitly re-publish diagnostics for all workspace files so push-model
        // clients get updated diagnostics even if workspace_diagnostic_refresh()
        // is unsupported.
        // IMPORTANT: drop the `Analysis` snapshot immediately. Binding it
        // (even as `_analysis`) would keep a salsa database handle alive for
        // the entire phase-4 diagnostics loop below. salsa makes any host
        // mutation (e.g. a concurrent `did_open` → `open_context` →
        // `set_file_content`) wait for every outstanding snapshot to drop
        // before it can take `&mut` access; if such a mutation grabs the host
        // mutex and then parks in `cancel_others` waiting for this snapshot,
        // while this loop's `salsa_file_context` blocks on that same mutex,
        // the indexer deadlocks. Each `run_diagnostics_cycle` takes its own
        // short-lived snapshot, so the loop does not need one held here.
        let (files, _) = self.salsa_all_files().await;
        let file_count_to_rediagnose = files.len();
        let diag_started_at = Instant::now();

        self.service.publish_progress(ProgressEvent::WorkspaceIndex {
            phase: 4,
            done: 0,
            total: file_count_to_rediagnose as u32,
            detail: "running diagnostics".to_owned(),
        });
        ux_messages::info(
            &self.client,
            format!(
                "Workspace indexing: running diagnostics for {} files (phase 4)...",
                file_count_to_rediagnose
            ),
        )
        .await;

        let diag_progress = Arc::new(AtomicUsize::new(0));
        stream::iter(files)
            .map(|(uri, _sf)| {
                let server = self.clone();
                let progress = diag_progress.clone();
                let total = file_count_to_rediagnose;
                let client = self.client.clone();
                let service = self.service.clone();
                async move {
                    server.run_diagnostics_cycle(uri, None).await;
                    let done = progress.fetch_add(1, Ordering::Relaxed) + 1;
                    // Log progress every 50 files
                    if done.is_multiple_of(50) || done == total {
                        service.publish_progress(ProgressEvent::WorkspaceIndex {
                            phase: 4,
                            done: done as u32,
                            total: total as u32,
                            detail: "diagnostics".to_owned(),
                        });
                        ux_messages::info(
                            &client,
                            format!(
                                "Workspace diagnostics: {}/{} files done",
                                done, total
                            ),
                        )
                        .await;
                    }
                }
            })
            .buffer_unordered(8)
            .for_each(|_| async {})
            .await;

        let diag_elapsed = diag_started_at.elapsed();
        tracing::info!(
            task_id = %task_id,
            rediagnosed_files = file_count_to_rediagnose,
            elapsed_ms = diag_elapsed.as_millis(),
            "re-published diagnostics with workspace context"
        );

        let total_ms = started_at.elapsed().as_millis();
        self.service.publish_progress(ProgressEvent::WorkspaceIndex {
            phase: 4,
            done: file_count_to_rediagnose as u32,
            total: file_count_to_rediagnose as u32,
            detail: format!(
                "complete: diagnosed in {:.1}s (total {}ms)",
                diag_elapsed.as_secs_f64(),
                total_ms,
            ),
        });
        self.service.publish_progress(ProgressEvent::Ready);
        ux_messages::info(
            &self.client,
            format!(
                "Workspace indexing complete: diagnosed {} files in {:.1}s (total {}ms)",
                file_count_to_rediagnose,
                diag_elapsed.as_secs_f64(),
                total_ms
            ),
        )
        .await;
    }
}
