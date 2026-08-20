#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Async test harness for protocol-level LSP testing.
//!
//! Wraps `LspService` to provide convenient methods for testing
//! LSP handlers end-to-end without actual stdio transport.
//!
//! The harness spawns a background task that auto-responds to server-to-client
//! requests (like `client/registerCapability`) so handlers don't hang.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tower_lsp::jsonrpc::Response;
use tower_lsp::lsp_types::*;
use tower_lsp::LanguageServer;

use crate::create_service;
use crate::SysmlLanguageServer;
use sysml_core::ModelGraph;
use sysml_service::progress::{LibraryPhase, ProgressEvent};
use sysml_service::SysmlService;

/// Configuration for a [`TestServer`].
///
/// One options shape — not a family of constructors — covering every knob a
/// caller might need: background-task policy, the client capabilities sent at
/// `initialize`, and a per-stage watchdog. Build with [`Default`] and mutate
/// the fields you care about.
#[derive(Clone)]
pub struct TestServerOptions {
    /// Suppress the background library-load + workspace-index tasks that
    /// `initialized` would otherwise spawn. `true` (the default) keeps the
    /// server quiescent — this is the fix for the cross-transport identity
    /// hang (task #225): the external harness previously could not set this
    /// knob (it was `#[cfg(test)]`-only), so it always ran the full stdlib
    /// load and raced/deadlocked. Set `false` only when a test genuinely
    /// needs the real library loaded (e.g. latency baselines).
    pub skip_background_tasks: bool,
    /// Suppress `did_open`'s heavy synchronous disk-project materialization
    /// (`open_context` → project discovery + `enable_stdlib` + full-workspace
    /// elaboration) for on-disk File targets. `false` by default so the
    /// realistic strict-mode / neighbour-visibility behaviour is preserved for
    /// protocol tests. Set `true` for parse-level tests (e.g. cross-transport
    /// identity) that only need the opened file's parse graph — this drops
    /// did_open from ~80s to a few ms. Independent of `skip_background_tasks`;
    /// the file content is still set, so the buffer stays parseable.
    pub skip_disk_project_load: bool,
    /// Client capabilities to send at `initialize`. `None` uses the harness
    /// default (completion snippets + semantic tokens + markdown hover).
    pub client_capabilities: Option<ClientCapabilities>,
    /// Per-stage watchdog. When `Some`, the lifecycle helpers
    /// (`initialize`, `initialize_full`, `open_document`, `require_graph`,
    /// `shutdown`) panic with a stage-named message if they exceed it, rather
    /// than hanging. `None` leaves them un-timed (historical behaviour).
    pub stage_timeout: Option<Duration>,
}

impl Default for TestServerOptions {
    fn default() -> Self {
        TestServerOptions {
            skip_background_tasks: true,
            skip_disk_project_load: false,
            client_capabilities: None,
            stage_timeout: None,
        }
    }
}

/// A test server that wraps the real LSP service for protocol-level testing.
pub struct TestServer {
    service: tower_lsp::LspService<SysmlLanguageServer>,
    /// Handle to the background auto-responder task. Aborted by
    /// [`TestServer::shutdown`]; dropping it merely detaches.
    responder: tokio::task::JoinHandle<()>,
    client_requests: Arc<tokio::sync::Mutex<Vec<tower_lsp::jsonrpc::Request>>>,
    client_capabilities: Option<ClientCapabilities>,
    stage_timeout: Option<Duration>,
}

impl TestServer {
    /// Create a new test server (not yet initialized) with default options:
    /// background tasks skipped, harness-default capabilities, no watchdog.
    ///
    /// Spawns an auto-responder for server-to-client requests.
    pub fn new() -> Self {
        Self::with_options(TestServerOptions::default())
    }

    /// Create a test server from an explicit [`TestServerOptions`].
    pub fn with_options(options: TestServerOptions) -> Self {
        let (service, socket) = create_service();

        service
            .inner()
            .skip_background_tasks
            .store(options.skip_background_tasks, std::sync::atomic::Ordering::Relaxed);
        service
            .inner()
            .skip_disk_project_load
            .store(options.skip_disk_project_load, std::sync::atomic::Ordering::Relaxed);

        // Split the socket into request stream and response sink
        let (mut requests, mut responses) = socket.split();
        let client_requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let client_requests_bg = client_requests.clone();

        // Spawn auto-responder: reads requests from server, sends OK responses
        let responder = tokio::spawn(async move {
            while let Some(request) = requests.next().await {
                client_requests_bg.lock().await.push(request.clone());

                // Auto-respond with success to all server requests that require a response.
                if let Some(id) = request.id().cloned() {
                    let response = Response::from_ok(id, serde_json::Value::Null);
                    if responses.send(response).await.is_err() {
                        break;
                    }
                }
            }
        });

        TestServer {
            service,
            responder,
            client_requests,
            client_capabilities: options.client_capabilities,
            stage_timeout: options.stage_timeout,
        }
    }

    /// Get a reference to the inner language server.
    pub fn server(&self) -> &SysmlLanguageServer {
        self.service.inner()
    }

    /// The LSP-owned [`SysmlService`]. `SysmlLanguageServer` shares its
    /// `AnalysisHost` with this service, so after `did_open` the document is
    /// in the same salsa store the REST/CLI transports read from — this is the
    /// real LSP state, not a shadow parse.
    pub fn service(&self) -> Arc<SysmlService> {
        self.server().service.clone()
    }

    /// Apply the configured per-stage watchdog to a future, panicking with a
    /// stage-named message on timeout. A no-op when `stage_timeout` is `None`.
    async fn with_watchdog<F, T>(&self, stage: &str, fut: F) -> T
    where
        F: Future<Output = T>,
    {
        match self.stage_timeout {
            Some(dur) => match tokio::time::timeout(dur, fut).await {
                Ok(value) => value,
                Err(_) => panic!(
                    "LSP harness stage '{stage}' exceeded watchdog {dur:?} \
                     (task #225: background-task hang regression?)"
                ),
            },
            None => fut.await,
        }
    }

    /// Inject a loaded library graph for tests without running background loading.
    ///
    /// P-RA4: with `LibraryState` retired, the only source of truth is
    /// `AnalysisHost::library_graph()`. Publishing a `LibraryLoad`
    /// event keeps the service-tracked lifecycle override in sync so
    /// `readiness_for` reports `Loaded` for callers that consult it.
    pub async fn set_library_graph(&self, mut graph: ModelGraph) {
        graph.rebuild_indexes();
        if !graph.library_packages().is_empty() {
            graph.build_library_index();
        }
        let element_count = graph.elements.len();

        {
            let mut host = self.server().analysis_host.lock().unwrap();
            host.set_library(graph);
        }

        // Mirror production: surface the Loaded lifecycle so any
        // subscriber-driven UX (and `readiness_for`) sees the same
        // state the production load pipeline produces.
        self.server().service.publish_progress(ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Loaded,
            done: element_count as u32,
            total: element_count as u32,
            detail: "test fixture".to_owned(),
        });
    }

    /// The harness-default client capabilities (completion snippets +
    /// semantic tokens + markdown hover). Used when
    /// `TestServerOptions::client_capabilities` is `None`.
    fn default_capabilities() -> ClientCapabilities {
        ClientCapabilities {
            text_document: Some(TextDocumentClientCapabilities {
                completion: Some(CompletionClientCapabilities {
                    completion_item: Some(CompletionItemCapability {
                        snippet_support: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                semantic_tokens: Some(SemanticTokensClientCapabilities {
                    dynamic_registration: Some(false),
                    requests: SemanticTokensClientCapabilitiesRequests {
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        range: Some(true),
                        ..Default::default()
                    },
                    token_types: vec![],
                    token_modifiers: vec![],
                    formats: vec![TokenFormat::RELATIVE],
                    overlapping_token_support: Some(false),
                    multiline_token_support: Some(false),
                    ..Default::default()
                }),
                hover: Some(HoverClientCapabilities {
                    content_format: Some(vec![MarkupKind::Markdown]),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Initialize the server with the configured (or default) capabilities.
    pub async fn initialize(&self) -> InitializeResult {
        let capabilities = self
            .client_capabilities
            .clone()
            .unwrap_or_else(Self::default_capabilities);
        let params = InitializeParams {
            capabilities,
            ..Default::default()
        };
        self.with_watchdog("initialize", self.server().initialize(params))
            .await
            .expect("initialize should succeed")
    }

    /// Initialize and send the `initialized` notification.
    pub async fn initialize_full(&self) -> InitializeResult {
        let result = self.initialize().await;
        self.with_watchdog("initialized", self.server().initialized(InitializedParams {}))
            .await;
        result
    }

    /// Capture the LSP-owned, parse-level [`ModelGraph`] for a URI.
    ///
    /// Reads through the shared `SysmlService`/`AnalysisHost` after
    /// `did_open` — this is the real LSP state. The read is synchronous
    /// salsa work, so it runs on a blocking thread under the stage watchdog
    /// (if configured) rather than blocking the async executor.
    pub async fn require_graph(&self, uri: &str) -> Arc<ModelGraph> {
        let service = self.service();
        let uri = uri.to_owned();
        let fut = async move {
            tokio::task::spawn_blocking(move || {
                service
                    .require_graph(&uri)
                    .expect("LSP-owned graph for opened URI")
            })
            .await
            .expect("graph-capture task panicked")
        };
        self.with_watchdog("graph-capture", fut).await
    }

    /// Cleanly shut the server down: run the LSP `shutdown` handler (cancels
    /// diagnostic tasks) and abort the background auto-responder so no task
    /// outlives the harness.
    pub async fn shutdown(&self) {
        let _ = self
            .with_watchdog("shutdown", self.server().shutdown())
            .await;
        self.responder.abort();
    }

    /// Open a document with the given URI and content.
    pub async fn open_document(&self, uri: &str, content: &str) {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: Url::parse(uri).expect("valid URI"),
                language_id: "sysml".to_string(),
                version: 0,
                text: content.to_string(),
            },
        };
        self.with_watchdog("did_open", self.server().did_open(params))
            .await;
    }

    /// Change a document's content (full replacement).
    pub async fn change_document(&self, uri: &str, version: i32, content: &str) {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content.to_string(),
            }],
        };
        self.server().did_change(params).await;
    }

    /// Close a document.
    pub async fn close_document(&self, uri: &str) {
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
        };
        self.server().did_close(params).await;
    }

    /// Request document symbols.
    pub async fn document_symbol(&self, uri: &str) -> Option<DocumentSymbolResponse> {
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .document_symbol(params)
            .await
            .expect("document_symbol should succeed")
    }

    /// Request code lenses for a document.
    pub async fn code_lens(&self, uri: &str) -> Option<Vec<CodeLens>> {
        let params = CodeLensParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .code_lens(params)
            .await
            .expect("code_lens should succeed")
    }

    /// Request hover at a position.
    pub async fn hover(&self, uri: &str, line: u32, character: u32) -> Option<Hover> {
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).expect("valid URI"),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };
        self.server()
            .hover(params)
            .await
            .expect("hover should succeed")
    }

    /// Request completion at a position.
    pub async fn completion(
        &self,
        uri: &str,
        line: u32,
        character: u32,
        trigger: Option<&str>,
    ) -> Option<CompletionResponse> {
        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).expect("valid URI"),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: trigger.map(|t| CompletionContext {
                trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
                trigger_character: Some(t.to_string()),
            }),
        };
        self.server()
            .completion(params)
            .await
            .expect("completion should succeed")
    }

    /// Resolve additional completion item details.
    pub async fn completion_resolve(&self, item: CompletionItem) -> CompletionItem {
        self.server()
            .completion_resolve(item)
            .await
            .expect("completion_resolve should succeed")
    }

    /// Request goto definition.
    pub async fn goto_definition(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Option<GotoDefinitionResponse> {
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).expect("valid URI"),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .goto_definition(params)
            .await
            .expect("goto_definition should succeed")
    }

    /// Request references.
    pub async fn references(&self, uri: &str, line: u32, character: u32) -> Option<Vec<Location>> {
        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).expect("valid URI"),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: ReferenceContext {
                include_declaration: true,
            },
        };
        self.server()
            .references(params)
            .await
            .expect("references should succeed")
    }

    /// Request semantic tokens for full document.
    pub async fn semantic_tokens_full(&self, uri: &str) -> Option<SemanticTokensResult> {
        let params = SemanticTokensParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .semantic_tokens_full(params)
            .await
            .expect("semantic_tokens_full should succeed")
    }

    /// Request semantic tokens full delta.
    pub async fn semantic_tokens_full_delta(
        &self,
        uri: &str,
        previous_result_id: &str,
    ) -> Option<SemanticTokensFullDeltaResult> {
        let params = SemanticTokensDeltaParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            previous_result_id: previous_result_id.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .semantic_tokens_full_delta(params)
            .await
            .expect("semantic_tokens_full_delta should succeed")
    }

    /// Request folding ranges.
    pub async fn folding_range(&self, uri: &str) -> Option<Vec<FoldingRange>> {
        let params = FoldingRangeParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .folding_range(params)
            .await
            .expect("folding_range should succeed")
    }

    /// Request rename.
    pub async fn rename(
        &self,
        uri: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).expect("valid URI"),
                },
                position: Position { line, character },
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };
        self.server()
            .rename(params)
            .await
            .expect("rename should succeed")
    }

    /// Request inlay hints.
    pub async fn inlay_hint(&self, uri: &str) -> Option<Vec<InlayHint>> {
        let params = InlayHintParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            range: tower_lsp::lsp_types::Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 10000,
                    character: 0,
                },
            },
            work_done_progress_params: Default::default(),
        };
        self.server()
            .inlay_hint(params)
            .await
            .expect("inlay_hint should succeed")
    }

    /// Request workspace symbols.
    #[allow(deprecated)]
    pub async fn workspace_symbol(&self, query: &str) -> Option<Vec<SymbolInformation>> {
        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .symbol(params)
            .await
            .expect("workspace_symbol should succeed")
    }

    /// Request code actions for diagnostics in a range.
    pub async fn code_action(
        &self,
        uri: &str,
        diagnostics: Vec<Diagnostic>,
    ) -> Option<CodeActionResponse> {
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 10000,
                    character: 0,
                },
            },
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .code_action(params)
            .await
            .expect("code_action should succeed")
    }

    /// Request code actions at a specific cursor position (for refactorings).
    pub async fn code_action_at(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Option<CodeActionResponse> {
        let pos = Position { line, character };
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            range: Range {
                start: pos,
                end: pos,
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.server()
            .code_action(params)
            .await
            .expect("code_action should succeed")
    }

    /// Request document formatting.
    pub async fn formatting(&self, uri: &str, tab_size: u32) -> Option<Vec<TextEdit>> {
        let params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("valid URI"),
            },
            options: FormattingOptions {
                tab_size,
                insert_spaces: true,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };
        self.server()
            .formatting(params)
            .await
            .expect("formatting should succeed")
    }

    /// Execute an LSP command through `workspace/executeCommand`.
    pub async fn execute_command(
        &self,
        command: &str,
        arguments: Vec<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let params = ExecuteCommandParams {
            command: command.to_string(),
            arguments,
            work_done_progress_params: Default::default(),
        };
        self.server()
            .execute_command(params)
            .await
            .expect("execute_command should succeed")
    }

    /// Clear captured server->client requests/notifications.
    pub async fn clear_client_requests(&self) {
        self.client_requests.lock().await.clear();
    }

    /// Wait for the latest manifest diagnostics captured by the server for a URI.
    pub async fn wait_for_manifest_diagnostics(
        &self,
        uri: &str,
        required_code: Option<&str>,
        timeout: Duration,
    ) -> Option<Vec<Diagnostic>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(entry) = self.server().last_manifest_diagnostics.get(uri) {
                let diagnostics = entry.value().clone();
                if let Some(code) = required_code {
                    let has_code = diagnostics.iter().any(|diag| {
                        matches!(
                            diag.code.as_ref(),
                            Some(NumberOrString::String(value)) if value == code
                        )
                    });
                    if has_code {
                        return Some(diagnostics);
                    }
                } else {
                    return Some(diagnostics);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Return the latest diagnostics payload published by the server for a URI.
    pub fn last_server_published_diagnostics(&self, uri: &str) -> Option<Vec<Diagnostic>> {
        self.server()
            .last_published_diagnostics_payload
            .get(uri)
            .map(|entry| entry.value().clone())
    }

    /// Wait for the LSP's async workspace indexer (kicked off in
    /// `initialized`) to register `expected_uris` and build the workspace
    /// `ProjectFileSet`. Indexing has no public "complete" signal, so we
    /// poll for its side effects: every expected URI registered in the host
    /// AND the default project's `ProjectFileSet` holding at least that many
    /// files. Only meaningful when the server was initialized with a
    /// workspace root and `skip_background_tasks` is `false`. Workspace-aware
    /// resolution (and `project_indexed`-gated diagnostics such as E200) only
    /// become active once this returns.
    pub async fn wait_for_workspace_index(&self, expected_uris: &[String], timeout: Duration) {
        let default_pid = sysml_project::ProjectHandle(
            sysml_service::open_context::DEFAULT_PROJECT_ID,
        );
        let start = tokio::time::Instant::now();
        loop {
            let (all_loaded, pfs_ready) = {
                let host = self.server().analysis_host.lock().unwrap();
                let files = host.files();
                let all_loaded = expected_uris.iter().all(|u| files.lookup(u).is_some());
                let pfs_ready = host
                    .project_file_set(default_pid)
                    .map(|pfs| pfs.files(host.db()).len() >= expected_uris.len())
                    .unwrap_or(false);
                (all_loaded, pfs_ready)
            };
            if all_loaded && pfs_ready {
                return;
            }
            assert!(
                start.elapsed() <= timeout,
                "workspace index did not reach ready state within {timeout:?} \
                 (expected {} file(s))",
                expected_uris.len()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// Standard test SysML content for reuse across tests.
pub const SAMPLE_PACKAGE: &str = "package TestPkg {\n  part def Vehicle {\n    attribute mass : Real;\n  }\n  part car : Vehicle;\n}\n";

pub const SAMPLE_ENUM: &str = "package Colors {\n  enum def Color {\n    enum Red;\n    enum Green;\n    enum Blue;\n  }\n}\n";

pub const SAMPLE_MULTI_ELEMENT: &str = "\
package Models {
  part def Engine {
    attribute horsepower : Real;
  }
  part def Car {
    part engine : Engine;
    attribute color : String;
  }
  part myCar : Car;
}
";
