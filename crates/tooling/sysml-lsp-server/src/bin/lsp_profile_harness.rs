// Profile harness binary uses println/eprintln for reporting.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::map_err_ignore
)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::{Id, Response};
use tower_lsp::lsp_types::request;
use tower_lsp::lsp_types::{Url, InitializeParams, WorkspaceFolder, ClientCapabilities, TextDocumentClientCapabilities, CompletionClientCapabilities, CompletionItemCapability, SemanticTokensClientCapabilities, SemanticTokensClientCapabilitiesRequests, SemanticTokensFullOptions, TokenFormat, WorkspaceClientCapabilities, InitializedParams, DidOpenTextDocumentParams, TextDocumentItem, DidChangeTextDocumentParams, VersionedTextDocumentIdentifier, TextDocumentContentChangeEvent, CompletionParams, TextDocumentPositionParams, TextDocumentIdentifier, CompletionContext, CompletionTriggerKind, HoverParams, GotoDefinitionParams, ReferenceParams, ReferenceContext, SemanticTokensParams, SemanticTokensRangeParams, Range, Position, DocumentSymbolParams, FoldingRangeParams, SelectionRangeParams, InlayHintParams, RenameParams, WorkspaceSymbolParams, DocumentLinkParams, SignatureHelpParams, SignatureHelpContext, SignatureHelpTriggerKind, CallHierarchyPrepareParams, CallHierarchyIncomingCallsParams, CallHierarchyOutgoingCallsParams, CodeActionParams, CodeActionContext, DocumentFormattingParams, FormattingOptions, ExecuteCommandParams, CallHierarchyItem, CompletionResponse, CompletionItem, GotoDefinitionResponse, SemanticTokensResult, SemanticTokensRangeResult, DocumentSymbolResponse, CodeActionResponse};
use tower_lsp::LanguageServer;

use sysml_lsp_server::{create_service, SysmlLanguageServer};

const DEFAULT_ITERATIONS: usize = 120;
const DEFAULT_WARMUP: usize = 20;

#[derive(Debug)]
struct CliArgs {
    scenario: Option<PathBuf>,
    output: Option<PathBuf>,
    iterations_override: Option<usize>,
    warmup_override: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScenarioConfig {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    defaults: ScenarioDefaults,
    documents: Vec<DocumentConfig>,
    workloads: Vec<WorkloadConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ScenarioDefaults {
    #[serde(default = "default_iterations")]
    iterations: usize,
    #[serde(default = "default_warmup")]
    warmup: usize,
}

impl Default for ScenarioDefaults {
    fn default() -> Self {
        Self {
            iterations: default_iterations(),
            warmup: default_warmup(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocumentConfig {
    id: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkloadConfig {
    name: String,
    #[serde(default)]
    iterations: Option<usize>,
    #[serde(default)]
    warmup: Option<usize>,
    operation: OperationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PositionSelector {
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    character: Option<u32>,
    #[serde(default)]
    marker: Option<String>,
    #[serde(default)]
    marker_offset: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OperationConfig {
    Completion {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
        #[serde(default)]
        trigger: Option<String>,
    },
    CompletionAfterDidChange {
        document: String,
        marker: String,
        typed_text: String,
        #[serde(default)]
        trigger: Option<String>,
    },
    CompletionResolve {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
        #[serde(default)]
        trigger: Option<String>,
    },
    Hover {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    GotoDefinition {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    GotoTypeDefinition {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    GotoImplementation {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    References {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    SemanticTokensFull {
        document: String,
    },
    SemanticTokensRange {
        document: String,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
    },
    DocumentSymbol {
        document: String,
    },
    FoldingRange {
        document: String,
    },
    SelectionRange {
        document: String,
        positions: Vec<PositionSelector>,
    },
    InlayHint {
        document: String,
    },
    PrepareRename {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    Rename {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
        new_name: String,
    },
    WorkspaceSymbol {
        query: String,
    },
    DocumentLink {
        document: String,
    },
    SignatureHelp {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
        #[serde(default)]
        trigger: Option<String>,
    },
    PrepareCallHierarchy {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    IncomingCalls {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    OutgoingCalls {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    CodeActionAt {
        document: String,
        #[serde(flatten)]
        position: PositionSelector,
    },
    Formatting {
        document: String,
        #[serde(default = "default_tab_size")]
        tab_size: u32,
    },
    ExecuteCommand {
        command: String,
        #[serde(default)]
        arguments: Vec<serde_json::Value>,
    },
}

#[derive(Debug, Clone)]
struct LoadedDocument {
    uri: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct RunReport {
    generated_at_unix_ms: u128,
    scenario_name: String,
    scenario_description: Option<String>,
    defaults: ScenarioDefaults,
    overrides: OverrideConfig,
    document_count: usize,
    workload_count: usize,
    workloads: Vec<WorkloadReport>,
    aggregate: AggregateReport,
}

#[derive(Debug, Serialize)]
struct OverrideConfig {
    iterations: Option<usize>,
    warmup: Option<usize>,
}

#[derive(Debug, Serialize)]
struct WorkloadReport {
    name: String,
    operation_kind: String,
    iterations: usize,
    warmup: usize,
    samples: usize,
    errors: usize,
    latency_us: LatencyStats,
    output_count: CountStats,
    throughput_ops_per_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
struct LatencyStats {
    min: u64,
    p50: u64,
    p95: u64,
    p99: u64,
    max: u64,
    mean: f64,
    stddev: f64,
}

#[derive(Debug, Clone, Serialize)]
struct CountStats {
    min: u64,
    p50: u64,
    p95: u64,
    max: u64,
    mean: f64,
}

#[derive(Debug, Serialize)]
struct AggregateReport {
    total_samples: usize,
    total_errors: usize,
    min_latency_us: u64,
    p50_latency_us: u64,
    p95_latency_us: u64,
    p99_latency_us: u64,
    max_latency_us: u64,
    mean_latency_us: f64,
}

struct ProfileServer {
    service: tower_lsp::LspService<SysmlLanguageServer>,
    versions: Mutex<HashMap<String, i32>>,
    _responder: tokio::task::JoinHandle<()>,
}

impl ProfileServer {
    fn new() -> Self {
        let (service, socket) = create_service();
        let (mut requests, mut responses) = socket.split();

        let responder = tokio::spawn(async move {
            while let Some(request) = requests.next().await {
                let id = request.id().cloned().unwrap_or(Id::Number(0));
                let response = Response::from_ok(id, serde_json::Value::Null);
                if responses.send(response).await.is_err() {
                    break;
                }
            }
        });

        Self {
            service,
            versions: Mutex::new(HashMap::new()),
            _responder: responder,
        }
    }

    fn server(&self) -> &SysmlLanguageServer {
        self.service.inner()
    }

    async fn initialize_full(&self, root_uri: Url) -> Result<(), String> {
        let params = InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: "profile-workspace".to_owned(),
            }]),
            capabilities: ClientCapabilities {
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
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..Default::default()
                        },
                        token_types: vec![],
                        token_modifiers: vec![],
                        formats: vec![TokenFormat::RELATIVE],
                        overlapping_token_support: Some(false),
                        multiline_token_support: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                workspace: Some(WorkspaceClientCapabilities {
                    workspace_folders: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        self.server()
            .initialize(params)
            .await
            .map_err(|e| format!("initialize failed: {e}"))?;
        self.server().initialized(InitializedParams {}).await;
        Ok(())
    }

    async fn open_document(&self, uri: &str, content: &str) {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: Url::parse(uri).expect("valid uri"),
                language_id: "sysml".to_owned(),
                version: 1,
                text: content.to_owned(),
            },
        };
        self.server().did_open(params).await;
        let mut versions = self.versions.lock().await;
        versions.insert(uri.to_owned(), 1);
    }

    async fn change_document(&self, uri: &str, content: &str) -> Result<(), String> {
        let uri_parsed = Url::parse(uri).map_err(|e| format!("invalid uri '{uri}': {e}"))?;
        let version = {
            let mut versions = self.versions.lock().await;
            let entry = versions.entry(uri.to_owned()).or_insert(1);
            *entry += 1;
            *entry
        };

        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri_parsed,
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content.to_owned(),
            }],
        };
        self.server().did_change(params).await;
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = parse_cli_args()?;
    let mut scenario = match &args.scenario {
        Some(path) => load_scenario(path)?,
        None => default_scenario(),
    };

    if scenario.documents.is_empty() {
        return Err("scenario has no documents".to_owned());
    }
    if scenario.workloads.is_empty() {
        return Err("scenario has no workloads".to_owned());
    }

    let workspace_root = workspace_root()?;
    apply_overrides(
        &mut scenario,
        args.iterations_override,
        args.warmup_override,
    );

    let loaded_docs = load_documents(&scenario, &workspace_root)?;

    let server = ProfileServer::new();
    server
        .initialize_full(path_to_file_url(&workspace_root)?)
        .await?;

    for doc in loaded_docs.values() {
        server.open_document(&doc.uri, &doc.content).await;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut workload_reports = Vec::new();

    for workload in &scenario.workloads {
        let iterations = workload
            .iterations
            .unwrap_or(scenario.defaults.iterations)
            .max(1);
        let warmup = workload.warmup.unwrap_or(scenario.defaults.warmup);

        let mut latencies_us = Vec::with_capacity(iterations);
        let mut outputs = Vec::with_capacity(iterations);
        let mut errors = 0usize;

        for idx in 0..(warmup + iterations) {
            let started = Instant::now();
            let result = run_operation(&server, workload, &loaded_docs, idx).await;
            let elapsed_us = started.elapsed().as_micros() as u64;

            match result {
                Ok(output_count) => {
                    if idx >= warmup {
                        latencies_us.push(elapsed_us);
                        outputs.push(output_count as u64);
                    }
                }
                Err(_) => {
                    errors += 1;
                }
            }
        }

        let total_measured_us: u128 = latencies_us.iter().map(|v| *v as u128).sum();
        let throughput_ops_per_sec = if total_measured_us == 0 {
            0.0
        } else {
            (latencies_us.len() as f64) / (total_measured_us as f64 / 1_000_000.0)
        };

        let report = WorkloadReport {
            name: workload.name.clone(),
            operation_kind: workload.operation.kind_name().to_owned(),
            iterations,
            warmup,
            samples: latencies_us.len(),
            errors,
            latency_us: latency_stats(&latencies_us),
            output_count: count_stats(&outputs),
            throughput_ops_per_sec,
        };

        workload_reports.push(report);
    }

    let aggregate = aggregate_report(&workload_reports);
    let report = RunReport {
        generated_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis(),
        scenario_name: scenario.name.clone(),
        scenario_description: scenario.description.clone(),
        defaults: scenario.defaults,
        overrides: OverrideConfig {
            iterations: args.iterations_override,
            warmup: args.warmup_override,
        },
        document_count: scenario.documents.len(),
        workload_count: scenario.workloads.len(),
        workloads: workload_reports,
        aggregate,
    };

    print_summary(&report);

    let output_path = resolve_output_path(args.output, &workspace_root, &report.scenario_name)?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create output directory {}: {e}",
                parent.display()
            )
        })?;
    }
    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("failed to serialize report: {e}"))?;
    std::fs::write(&output_path, json)
        .map_err(|e| format!("failed to write report {}: {e}", output_path.display()))?;

    println!("\nreport written: {}", output_path.display());
    Ok(())
}

async fn run_operation(
    server: &ProfileServer,
    workload: &WorkloadConfig,
    docs: &HashMap<String, LoadedDocument>,
    sample_index: usize,
) -> Result<usize, String> {
    match &workload.operation {
        OperationConfig::Completion {
            document,
            position,
            trigger,
        } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: trigger.as_ref().map(|t| CompletionContext {
                    trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
                    trigger_character: Some(t.clone()),
                }),
            };
            let response = server
                .server()
                .completion(params)
                .await
                .map_err(|e| format!("completion failed: {e}"))?;
            Ok(completion_item_count(response))
        }
        OperationConfig::CompletionAfterDidChange {
            document,
            marker,
            typed_text,
            trigger,
        } => {
            let doc = docs
                .get(document)
                .ok_or_else(|| format!("unknown document id: {document}"))?;

            let prefix = cyclic_prefix(typed_text, sample_index)?;
            let (changed_content, cursor_offset) =
                insert_text_after_marker(&doc.content, marker, &prefix)?;

            server
                .change_document(&doc.uri, &changed_content)
                .await
                .map_err(|e| format!("did_change failed: {e}"))?;

            let uri =
                Url::parse(&doc.uri).map_err(|e| format!("invalid doc uri {}: {e}", doc.uri))?;
            let position = offset_to_position(&changed_content, cursor_offset);
            let params = CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: trigger.as_ref().map(|t| CompletionContext {
                    trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
                    trigger_character: Some(t.clone()),
                }),
            };
            let response = server
                .server()
                .completion(params)
                .await
                .map_err(|e| format!("completion after did_change failed: {e}"))?;
            Ok(completion_item_count(response))
        }
        OperationConfig::CompletionResolve {
            document,
            position,
            trigger,
        } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let completion_params = CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: trigger.as_ref().map(|t| CompletionContext {
                    trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
                    trigger_character: Some(t.clone()),
                }),
            };
            let completion = server
                .server()
                .completion(completion_params)
                .await
                .map_err(|e| format!("completion for resolve failed: {e}"))?;

            let mut items = completion_items(completion);
            if items.is_empty() {
                return Ok(0);
            }
            let item = items.remove(0);
            let resolved = server
                .server()
                .completion_resolve(item)
                .await
                .map_err(|e| format!("completion_resolve failed: {e}"))?;
            Ok(usize::from(!resolved.label.is_empty()))
        }
        OperationConfig::Hover { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
            };
            let response = server
                .server()
                .hover(params)
                .await
                .map_err(|e| format!("hover failed: {e}"))?;
            Ok(usize::from(response.is_some()))
        }
        OperationConfig::GotoDefinition { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = GotoDefinitionParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .goto_definition(params)
                .await
                .map_err(|e| format!("goto_definition failed: {e}"))?;
            Ok(goto_response_count(response))
        }
        OperationConfig::GotoTypeDefinition { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = request::GotoTypeDefinitionParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .goto_type_definition(params)
                .await
                .map_err(|e| format!("goto_type_definition failed: {e}"))?;
            Ok(goto_type_response_count(response))
        }
        OperationConfig::GotoImplementation { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = request::GotoImplementationParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .goto_implementation(params)
                .await
                .map_err(|e| format!("goto_implementation failed: {e}"))?;
            Ok(goto_impl_response_count(response))
        }
        OperationConfig::References { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = ReferenceParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: ReferenceContext {
                    include_declaration: true,
                },
            };
            let response = server
                .server()
                .references(params)
                .await
                .map_err(|e| format!("references failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::SemanticTokensFull { document } => {
            let uri = document_uri(docs, document)?;
            let params = SemanticTokensParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .semantic_tokens_full(params)
                .await
                .map_err(|e| format!("semantic_tokens_full failed: {e}"))?;
            Ok(semantic_tokens_result_count(response))
        }
        OperationConfig::SemanticTokensRange {
            document,
            start_line,
            start_character,
            end_line,
            end_character,
        } => {
            let uri = document_uri(docs, document)?;
            let params = SemanticTokensRangeParams {
                text_document: TextDocumentIdentifier { uri },
                range: Range {
                    start: Position {
                        line: *start_line,
                        character: *start_character,
                    },
                    end: Position {
                        line: *end_line,
                        character: *end_character,
                    },
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .semantic_tokens_range(params)
                .await
                .map_err(|e| format!("semantic_tokens_range failed: {e}"))?;
            Ok(semantic_tokens_range_count(response))
        }
        OperationConfig::DocumentSymbol { document } => {
            let uri = document_uri(docs, document)?;
            let params = DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .document_symbol(params)
                .await
                .map_err(|e| format!("document_symbol failed: {e}"))?;
            Ok(document_symbol_count(response))
        }
        OperationConfig::FoldingRange { document } => {
            let uri = document_uri(docs, document)?;
            let params = FoldingRangeParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .folding_range(params)
                .await
                .map_err(|e| format!("folding_range failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::SelectionRange {
            document,
            positions,
        } => {
            let doc = docs
                .get(document)
                .ok_or_else(|| format!("unknown document id: {document}"))?;
            let uri =
                Url::parse(&doc.uri).map_err(|e| format!("invalid doc uri {}: {e}", doc.uri))?;
            let resolved = positions
                .iter()
                .map(|p| resolve_position(p, &doc.content))
                .collect::<Result<Vec<_>, _>>()?;
            let params = SelectionRangeParams {
                text_document: TextDocumentIdentifier { uri },
                positions: resolved,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .selection_range(params)
                .await
                .map_err(|e| format!("selection_range failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::InlayHint { document } => {
            let uri = document_uri(docs, document)?;
            let params = InlayHintParams {
                text_document: TextDocumentIdentifier { uri },
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 10_000,
                        character: 0,
                    },
                },
                work_done_progress_params: Default::default(),
            };
            let response = server
                .server()
                .inlay_hint(params)
                .await
                .map_err(|e| format!("inlay_hint failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::PrepareRename { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            };
            let response = server
                .server()
                .prepare_rename(params)
                .await
                .map_err(|e| format!("prepare_rename failed: {e}"))?;
            Ok(usize::from(response.is_some()))
        }
        OperationConfig::Rename {
            document,
            position,
            new_name,
        } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = RenameParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                new_name: new_name.clone(),
                work_done_progress_params: Default::default(),
            };
            let response = server
                .server()
                .rename(params)
                .await
                .map_err(|e| format!("rename failed: {e}"))?;
            Ok(response
                .as_ref()
                .map(|edit| edit.changes.as_ref().map(|c| c.len()).unwrap_or(0))
                .unwrap_or(0))
        }
        OperationConfig::WorkspaceSymbol { query } => {
            #[allow(deprecated)]
            let params = WorkspaceSymbolParams {
                query: query.clone(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };

            let response = server
                .server()
                .symbol(params)
                .await
                .map_err(|e| format!("workspace symbol failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::DocumentLink { document } => {
            let uri = document_uri(docs, document)?;
            let params = DocumentLinkParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .document_link(params)
                .await
                .map_err(|e| format!("document_link failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::SignatureHelp {
            document,
            position,
            trigger,
        } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = SignatureHelpParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
                context: trigger.as_ref().map(|t| SignatureHelpContext {
                    trigger_kind: SignatureHelpTriggerKind::TRIGGER_CHARACTER,
                    trigger_character: Some(t.clone()),
                    is_retrigger: false,
                    active_signature_help: None,
                }),
            };
            let response = server
                .server()
                .signature_help(params)
                .await
                .map_err(|e| format!("signature_help failed: {e}"))?;
            Ok(response.as_ref().map(|h| h.signatures.len()).unwrap_or(0))
        }
        OperationConfig::PrepareCallHierarchy { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = CallHierarchyPrepareParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri },
                    position: pos,
                },
                work_done_progress_params: Default::default(),
            };
            let response = server
                .server()
                .prepare_call_hierarchy(params)
                .await
                .map_err(|e| format!("prepare_call_hierarchy failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::IncomingCalls { document, position } => {
            let item = prepare_call_hierarchy_item(server, docs, document, position).await?;
            let Some(item) = item else {
                return Ok(0);
            };
            let params = CallHierarchyIncomingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .incoming_calls(params)
                .await
                .map_err(|e| format!("incoming_calls failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::OutgoingCalls { document, position } => {
            let item = prepare_call_hierarchy_item(server, docs, document, position).await?;
            let Some(item) = item else {
                return Ok(0);
            };
            let params = CallHierarchyOutgoingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .server()
                .outgoing_calls(params)
                .await
                .map_err(|e| format!("outgoing_calls failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::CodeActionAt { document, position } => {
            let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
            let params = CodeActionParams {
                text_document: TextDocumentIdentifier { uri },
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
            let response = server
                .server()
                .code_action(params)
                .await
                .map_err(|e| format!("code_action failed: {e}"))?;
            Ok(code_action_count(response))
        }
        OperationConfig::Formatting { document, tab_size } => {
            let uri = document_uri(docs, document)?;
            let params = DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri },
                options: FormattingOptions {
                    tab_size: *tab_size,
                    insert_spaces: true,
                    ..Default::default()
                },
                work_done_progress_params: Default::default(),
            };
            let response = server
                .server()
                .formatting(params)
                .await
                .map_err(|e| format!("formatting failed: {e}"))?;
            Ok(response.as_ref().map(|v| v.len()).unwrap_or(0))
        }
        OperationConfig::ExecuteCommand { command, arguments } => {
            let params = ExecuteCommandParams {
                command: command.clone(),
                arguments: arguments.clone(),
                work_done_progress_params: Default::default(),
            };
            let response = server
                .server()
                .execute_command(params)
                .await
                .map_err(|e| format!("execute_command failed: {e}"))?;
            Ok(usize::from(response.is_some()))
        }
    }
}

async fn prepare_call_hierarchy_item(
    server: &ProfileServer,
    docs: &HashMap<String, LoadedDocument>,
    document: &str,
    position: &PositionSelector,
) -> Result<Option<CallHierarchyItem>, String> {
    let (uri, pos) = resolve_doc_and_position(docs, document, position)?;
    let params = CallHierarchyPrepareParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: pos,
        },
        work_done_progress_params: Default::default(),
    };
    let prepared = server
        .server()
        .prepare_call_hierarchy(params)
        .await
        .map_err(|e| format!("prepare_call_hierarchy failed: {e}"))?;
    Ok(prepared.and_then(|mut items| items.drain(..).next()))
}

fn resolve_doc_and_position(
    docs: &HashMap<String, LoadedDocument>,
    document: &str,
    selector: &PositionSelector,
) -> Result<(Url, Position), String> {
    let doc = docs
        .get(document)
        .ok_or_else(|| format!("unknown document id: {document}"))?;
    let uri = Url::parse(&doc.uri).map_err(|e| format!("invalid doc uri {}: {e}", doc.uri))?;
    let pos = resolve_position(selector, &doc.content)?;
    Ok((uri, pos))
}

fn resolve_position(selector: &PositionSelector, content: &str) -> Result<Position, String> {
    if let (Some(line), Some(character)) = (selector.line, selector.character) {
        return Ok(Position { line, character });
    }

    let marker = selector
        .marker
        .as_deref()
        .ok_or_else(|| "position selector requires either line+character or marker".to_owned())?;

    let idx = content
        .find(marker)
        .ok_or_else(|| format!("marker not found in document: {marker}"))?;
    let mut offset = idx + marker.len();
    if selector.marker_offset < 0 {
        offset = offset.saturating_sub(selector.marker_offset.unsigned_abs() as usize);
    } else if selector.marker_offset > 0 {
        offset = offset.saturating_add(selector.marker_offset as usize);
    }

    Ok(offset_to_position(content, offset.min(content.len())))
}

fn cyclic_prefix(typed_text: &str, sample_index: usize) -> Result<String, String> {
    let chars: Vec<char> = typed_text.chars().collect();
    if chars.is_empty() {
        return Err("typed_text cannot be empty for completion_after_did_change".to_owned());
    }
    let prefix_len = (sample_index % chars.len()) + 1;
    Ok(chars.into_iter().take(prefix_len).collect())
}

fn insert_text_after_marker(
    base_content: &str,
    marker: &str,
    inserted_text: &str,
) -> Result<(String, usize), String> {
    let marker_start = base_content
        .find(marker)
        .ok_or_else(|| format!("marker not found in document: {marker}"))?;
    let insert_at = marker_start + marker.len();

    let mut changed = String::with_capacity(base_content.len() + inserted_text.len());
    changed.push_str(&base_content[..insert_at]);
    changed.push_str(inserted_text);
    changed.push_str(&base_content[insert_at..]);

    Ok((changed, insert_at + inserted_text.len()))
}

fn offset_to_position(content: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut character = 0u32;

    for ch in content[..offset].chars() {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

fn document_uri(docs: &HashMap<String, LoadedDocument>, document: &str) -> Result<Url, String> {
    let doc = docs
        .get(document)
        .ok_or_else(|| format!("unknown document id: {document}"))?;
    Url::parse(&doc.uri).map_err(|e| format!("invalid doc uri {}: {e}", doc.uri))
}

fn completion_item_count(response: Option<CompletionResponse>) -> usize {
    match response {
        Some(CompletionResponse::Array(items)) => items.len(),
        Some(CompletionResponse::List(list)) => list.items.len(),
        None => 0,
    }
}

fn completion_items(response: Option<CompletionResponse>) -> Vec<CompletionItem> {
    match response {
        Some(CompletionResponse::Array(items)) => items,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    }
}

fn goto_response_count(response: Option<GotoDefinitionResponse>) -> usize {
    match response {
        Some(GotoDefinitionResponse::Scalar(_)) => 1,
        Some(GotoDefinitionResponse::Array(v)) => v.len(),
        Some(GotoDefinitionResponse::Link(v)) => v.len(),
        None => 0,
    }
}

fn goto_type_response_count(response: Option<request::GotoTypeDefinitionResponse>) -> usize {
    match response {
        Some(request::GotoTypeDefinitionResponse::Scalar(_)) => 1,
        Some(request::GotoTypeDefinitionResponse::Array(v)) => v.len(),
        Some(request::GotoTypeDefinitionResponse::Link(v)) => v.len(),
        None => 0,
    }
}

fn goto_impl_response_count(response: Option<request::GotoImplementationResponse>) -> usize {
    match response {
        Some(request::GotoImplementationResponse::Scalar(_)) => 1,
        Some(request::GotoImplementationResponse::Array(v)) => v.len(),
        Some(request::GotoImplementationResponse::Link(v)) => v.len(),
        None => 0,
    }
}

fn semantic_tokens_result_count(response: Option<SemanticTokensResult>) -> usize {
    match response {
        Some(SemanticTokensResult::Tokens(tokens)) => tokens.data.len(),
        Some(SemanticTokensResult::Partial(partial)) => partial.data.len(),
        None => 0,
    }
}

fn semantic_tokens_range_count(response: Option<SemanticTokensRangeResult>) -> usize {
    match response {
        Some(SemanticTokensRangeResult::Tokens(tokens)) => tokens.data.len(),
        Some(SemanticTokensRangeResult::Partial(partial)) => partial.data.len(),
        None => 0,
    }
}

fn document_symbol_count(response: Option<DocumentSymbolResponse>) -> usize {
    match response {
        Some(DocumentSymbolResponse::Nested(v)) => v.len(),
        Some(DocumentSymbolResponse::Flat(v)) => v.len(),
        None => 0,
    }
}

fn code_action_count(response: Option<CodeActionResponse>) -> usize {
    response.map(|v| v.len()).unwrap_or(0)
}

impl OperationConfig {
    fn kind_name(&self) -> &'static str {
        match self {
            OperationConfig::Completion { .. } => "completion",
            OperationConfig::CompletionAfterDidChange { .. } => "completion_after_did_change",
            OperationConfig::CompletionResolve { .. } => "completion_resolve",
            OperationConfig::Hover { .. } => "hover",
            OperationConfig::GotoDefinition { .. } => "goto_definition",
            OperationConfig::GotoTypeDefinition { .. } => "goto_type_definition",
            OperationConfig::GotoImplementation { .. } => "goto_implementation",
            OperationConfig::References { .. } => "references",
            OperationConfig::SemanticTokensFull { .. } => "semantic_tokens_full",
            OperationConfig::SemanticTokensRange { .. } => "semantic_tokens_range",
            OperationConfig::DocumentSymbol { .. } => "document_symbol",
            OperationConfig::FoldingRange { .. } => "folding_range",
            OperationConfig::SelectionRange { .. } => "selection_range",
            OperationConfig::InlayHint { .. } => "inlay_hint",
            OperationConfig::PrepareRename { .. } => "prepare_rename",
            OperationConfig::Rename { .. } => "rename",
            OperationConfig::WorkspaceSymbol { .. } => "workspace_symbol",
            OperationConfig::DocumentLink { .. } => "document_link",
            OperationConfig::SignatureHelp { .. } => "signature_help",
            OperationConfig::PrepareCallHierarchy { .. } => "prepare_call_hierarchy",
            OperationConfig::IncomingCalls { .. } => "incoming_calls",
            OperationConfig::OutgoingCalls { .. } => "outgoing_calls",
            OperationConfig::CodeActionAt { .. } => "code_action_at",
            OperationConfig::Formatting { .. } => "formatting",
            OperationConfig::ExecuteCommand { .. } => "execute_command",
        }
    }
}

fn default_scenario() -> ScenarioConfig {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    ScenarioConfig {
        name: "default_full_lsp_suite".to_owned(),
        description: Some(
            "Broad LSP profiling suite across completion, navigation, symbols, semantic tokens, and editing features".to_owned(),
        ),
        defaults: ScenarioDefaults::default(),
        documents: vec![
            DocumentConfig {
                id: "profile".to_owned(),
                uri: Some("file:///profile/profile_harness.sysml".to_owned()),
                path: None,
                text: Some(DEFAULT_PROFILE_DOC.to_owned()),
            },
            DocumentConfig {
                id: "simple_vehicle".to_owned(),
                uri: None,
                path: Some(
                    fixture_dir
                        .join("valid/simple_vehicle.sysml")
                        .to_string_lossy()
                        .to_string(),
                ),
                text: None,
            },
            DocumentConfig {
                id: "clean".to_owned(),
                uri: None,
                path: Some(fixture_dir.join("valid/clean.sysml").to_string_lossy().to_string()),
                text: None,
            },
        ],
        workloads: default_workloads(),
    }
}

fn default_workloads() -> Vec<WorkloadConfig> {
    vec![
        WorkloadConfig {
            name: "completion_general".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::Completion {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedGeneral = car".to_owned()),
                    marker_offset: 0,
                },
                trigger: None,
            },
        },
        WorkloadConfig {
            name: "completion_namespace".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::Completion {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("private import ScalarValues::".to_owned()),
                    marker_offset: 0,
                },
                trigger: Some(":".to_owned()),
            },
        },
        WorkloadConfig {
            name: "completion_type_reference".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::Completion {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedType : Integ".to_owned()),
                    marker_offset: 0,
                },
                trigger: None,
            },
        },
        WorkloadConfig {
            name: "completion_feature_chain".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::Completion {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedMember = car.engine.r".to_owned()),
                    marker_offset: 0,
                },
                trigger: None,
            },
        },
        WorkloadConfig {
            name: "completion_resolve".to_owned(),
            iterations: Some(60),
            warmup: Some(10),
            operation: OperationConfig::CompletionResolve {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedGeneral = car".to_owned()),
                    marker_offset: 0,
                },
                trigger: None,
            },
        },
        WorkloadConfig {
            name: "hover".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::Hover {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("part car : Car".to_owned()),
                    marker_offset: -2,
                },
            },
        },
        WorkloadConfig {
            name: "goto_definition".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::GotoDefinition {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("part engine : Engine".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "goto_type_definition".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::GotoTypeDefinition {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedMember = car.engine.r".to_owned()),
                    marker_offset: -1,
                },
            },
        },
        WorkloadConfig {
            name: "goto_implementation".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::GotoImplementation {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("part def Vehicle".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "references".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::References {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("part def Engine".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "document_symbol".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::DocumentSymbol {
                document: "profile".to_owned(),
            },
        },
        WorkloadConfig {
            name: "semantic_tokens_full".to_owned(),
            iterations: Some(50),
            warmup: Some(8),
            operation: OperationConfig::SemanticTokensFull {
                document: "profile".to_owned(),
            },
        },
        WorkloadConfig {
            name: "semantic_tokens_range".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::SemanticTokensRange {
                document: "profile".to_owned(),
                start_line: 0,
                start_character: 0,
                end_line: 30,
                end_character: 0,
            },
        },
        WorkloadConfig {
            name: "folding_range".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::FoldingRange {
                document: "profile".to_owned(),
            },
        },
        WorkloadConfig {
            name: "selection_range".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::SelectionRange {
                document: "profile".to_owned(),
                positions: vec![PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedMember = car.engine.r".to_owned()),
                    marker_offset: 0,
                }],
            },
        },
        WorkloadConfig {
            name: "inlay_hint".to_owned(),
            iterations: Some(60),
            warmup: Some(8),
            operation: OperationConfig::InlayHint {
                document: "profile".to_owned(),
            },
        },
        WorkloadConfig {
            name: "prepare_rename".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::PrepareRename {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedMember = car.engine.r".to_owned()),
                    marker_offset: -2,
                },
            },
        },
        WorkloadConfig {
            name: "rename".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::Rename {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedMember = car.engine.r".to_owned()),
                    marker_offset: -2,
                },
                new_name: "typedMemberRenamed".to_owned(),
            },
        },
        WorkloadConfig {
            name: "workspace_symbol".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::WorkspaceSymbol {
                query: "Engine".to_owned(),
            },
        },
        WorkloadConfig {
            name: "document_link".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::DocumentLink {
                document: "profile".to_owned(),
            },
        },
        WorkloadConfig {
            name: "signature_help".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::SignatureHelp {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("startEngine : StartEngine".to_owned()),
                    marker_offset: 0,
                },
                trigger: Some("(".to_owned()),
            },
        },
        WorkloadConfig {
            name: "prepare_call_hierarchy".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::PrepareCallHierarchy {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("action def StartEngine".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "incoming_calls".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::IncomingCalls {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("action def StartEngine".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "outgoing_calls".to_owned(),
            iterations: None,
            warmup: None,
            operation: OperationConfig::OutgoingCalls {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("action def Drive".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "code_action_at".to_owned(),
            iterations: Some(80),
            warmup: Some(10),
            operation: OperationConfig::CodeActionAt {
                document: "profile".to_owned(),
                position: PositionSelector {
                    line: None,
                    character: None,
                    marker: Some("attribute typedType : Integ".to_owned()),
                    marker_offset: 0,
                },
            },
        },
        WorkloadConfig {
            name: "formatting".to_owned(),
            iterations: Some(50),
            warmup: Some(8),
            operation: OperationConfig::Formatting {
                document: "profile".to_owned(),
                tab_size: 4,
            },
        },
        WorkloadConfig {
            name: "typing_import_completion_after_change".to_owned(),
            iterations: Some(120),
            warmup: Some(20),
            operation: OperationConfig::CompletionAfterDidChange {
                document: "profile".to_owned(),
                marker: "private import ScalarValues/*typing_marker*/".to_owned(),
                typed_text: "::Integer".to_owned(),
                trigger: None,
            },
        },
        WorkloadConfig {
            name: "execute_command_cache_status".to_owned(),
            iterations: Some(30),
            warmup: Some(5),
            operation: OperationConfig::ExecuteCommand {
                command: "sysml.cache.status".to_owned(),
                arguments: vec![],
            },
        },
    ]
}

const DEFAULT_PROFILE_DOC: &str = r#"package ProfileHarness {
    private import ScalarValues::Integer;
    private import ScalarValues/*typing_marker*/

    part def Engine {
        attribute rpm : Integer;
    }

    part def Vehicle {
        part engine : Engine;
        attribute mass : Integer;
    }

    part def Car specializes Vehicle {
        attribute speed : Integer;
    }

    part car : Car {
        attribute typedGeneral = car
        attribute typedType : Integ
        attribute typedMember = car.engine.r
    }

    action def StartEngine {
    }

    action def Drive {
        action startEngine : StartEngine;
    }
}
"#;

fn load_scenario(path: &Path) -> Result<ScenarioConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read scenario {}: {e}", path.display()))?;
    let scenario: ScenarioConfig = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse scenario {}: {e}", path.display()))?;
    Ok(scenario)
}

fn apply_overrides(
    scenario: &mut ScenarioConfig,
    iterations_override: Option<usize>,
    warmup_override: Option<usize>,
) {
    if let Some(v) = iterations_override {
        scenario.defaults.iterations = v.max(1);
        for workload in &mut scenario.workloads {
            workload.iterations = Some(v.max(1));
        }
    }

    if let Some(v) = warmup_override {
        scenario.defaults.warmup = v;
        for workload in &mut scenario.workloads {
            workload.warmup = Some(v);
        }
    }
}

fn load_documents(
    scenario: &ScenarioConfig,
    workspace_root: &Path,
) -> Result<HashMap<String, LoadedDocument>, String> {
    let mut docs = HashMap::new();

    for doc in &scenario.documents {
        let content = match (&doc.path, &doc.text) {
            (Some(path), None) => {
                let resolved = resolve_path(path, workspace_root);
                std::fs::read_to_string(&resolved).map_err(|e| {
                    format!(
                        "failed to read document '{}' from {}: {e}",
                        doc.id,
                        resolved.display()
                    )
                })?
            }
            (None, Some(text)) => text.clone(),
            (Some(_), Some(_)) => {
                return Err(format!(
                    "document '{}' should specify exactly one of 'path' or 'text'",
                    doc.id
                ));
            }
            (None, None) => {
                return Err(format!(
                    "document '{}' is missing content source (path/text)",
                    doc.id
                ));
            }
        };

        let uri = match &doc.uri {
            Some(uri) => uri.clone(),
            None => {
                if let Some(path) = &doc.path {
                    let resolved = resolve_path(path, workspace_root);
                    path_to_file_url(&resolved)?.to_string()
                } else {
                    format!("file:///profile/{}.sysml", doc.id)
                }
            }
        };

        if docs
            .insert(doc.id.clone(), LoadedDocument { uri, content })
            .is_some()
        {
            return Err(format!("duplicate document id '{}'", doc.id));
        }
    }

    Ok(docs)
}

fn resolve_path(path: &str, workspace_root: &Path) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return p;
    }
    workspace_root.join(p)
}

fn path_to_file_url(path: &Path) -> Result<Url, String> {
    Url::from_file_path(path)
        .map_err(|_| format!("failed to convert path to file URI: {}", path.display()))
}

fn workspace_root() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("failed to determine current directory: {e}"))
}

fn resolve_output_path(
    output: Option<PathBuf>,
    workspace_root: &Path,
    scenario_name: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = output {
        if path.is_absolute() {
            return Ok(path);
        }
        return Ok(workspace_root.join(path));
    }

    let mut safe_name = String::new();
    for ch in scenario_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            safe_name.push(ch);
        } else {
            safe_name.push('_');
        }
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs();

    Ok(workspace_root
        .join("benchmarks")
        .join("profiles")
        .join(format!("lsp_profile_{}_{}.json", safe_name, ts)))
}

fn latency_stats(samples: &[u64]) -> LatencyStats {
    if samples.is_empty() {
        return LatencyStats {
            min: 0,
            p50: 0,
            p95: 0,
            p99: 0,
            max: 0,
            mean: 0.0,
            stddev: 0.0,
        };
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();

    let sum: u128 = sorted.iter().map(|v| *v as u128).sum();
    let mean = sum as f64 / sorted.len() as f64;

    let variance = if sorted.len() > 1 {
        let ss: f64 = sorted
            .iter()
            .map(|v| {
                let d = *v as f64 - mean;
                d * d
            })
            .sum();
        ss / sorted.len() as f64
    } else {
        0.0
    };

    LatencyStats {
        min: *sorted.first().unwrap_or(&0),
        p50: percentile(&sorted, 0.50),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
        max: *sorted.last().unwrap_or(&0),
        mean,
        stddev: variance.sqrt(),
    }
}

fn count_stats(samples: &[u64]) -> CountStats {
    if samples.is_empty() {
        return CountStats {
            min: 0,
            p50: 0,
            p95: 0,
            max: 0,
            mean: 0.0,
        };
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let sum: u128 = sorted.iter().map(|v| *v as u128).sum();

    CountStats {
        min: *sorted.first().unwrap_or(&0),
        p50: percentile(&sorted, 0.50),
        p95: percentile(&sorted, 0.95),
        max: *sorted.last().unwrap_or(&0),
        mean: sum as f64 / sorted.len() as f64,
    }
}

fn percentile(sorted: &[u64], quantile: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn aggregate_report(workloads: &[WorkloadReport]) -> AggregateReport {
    let mut all_latencies = Vec::new();
    let mut total_samples = 0usize;
    let mut total_errors = 0usize;

    for wl in workloads {
        total_samples += wl.samples;
        total_errors += wl.errors;
        if wl.samples > 0 {
            all_latencies.push(wl.latency_us.min);
            all_latencies.push(wl.latency_us.p50);
            all_latencies.push(wl.latency_us.p95);
            all_latencies.push(wl.latency_us.p99);
            all_latencies.push(wl.latency_us.max);
        }
    }

    all_latencies.sort_unstable();
    let mean = if all_latencies.is_empty() {
        0.0
    } else {
        all_latencies.iter().map(|v| *v as f64).sum::<f64>() / all_latencies.len() as f64
    };

    AggregateReport {
        total_samples,
        total_errors,
        min_latency_us: *all_latencies.first().unwrap_or(&0),
        p50_latency_us: percentile(&all_latencies, 0.50),
        p95_latency_us: percentile(&all_latencies, 0.95),
        p99_latency_us: percentile(&all_latencies, 0.99),
        max_latency_us: *all_latencies.last().unwrap_or(&0),
        mean_latency_us: mean,
    }
}

fn print_summary(report: &RunReport) {
    println!("scenario: {}", report.scenario_name);
    if let Some(desc) = &report.scenario_description {
        println!("description: {desc}");
    }
    println!(
        "documents={} workloads={} defaults(iterations={}, warmup={})",
        report.document_count,
        report.workload_count,
        report.defaults.iterations,
        report.defaults.warmup
    );

    println!();
    println!(
        "{:<30} {:<24} {:>6} {:>6} {:>10} {:>10} {:>10} {:>10} {:>9}",
        "workload", "operation", "n", "err", "p50(ms)", "p95(ms)", "p99(ms)", "mean(ms)", "ops/s"
    );
    println!("{}", "-".repeat(130));

    for wl in &report.workloads {
        println!(
            "{:<30} {:<24} {:>6} {:>6} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>9.1}",
            truncate(&wl.name, 30),
            truncate(&wl.operation_kind, 24),
            wl.samples,
            wl.errors,
            wl.latency_us.p50 as f64 / 1000.0,
            wl.latency_us.p95 as f64 / 1000.0,
            wl.latency_us.p99 as f64 / 1000.0,
            wl.latency_us.mean / 1000.0,
            wl.throughput_ops_per_sec,
        );
    }

    println!();
    println!(
        "aggregate: total_samples={} total_errors={} p50={:.3}ms p95={:.3}ms p99={:.3}ms",
        report.aggregate.total_samples,
        report.aggregate.total_errors,
        report.aggregate.p50_latency_us as f64 / 1000.0,
        report.aggregate.p95_latency_us as f64 / 1000.0,
        report.aggregate.p99_latency_us as f64 / 1000.0,
    );
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_owned();
    }
    let mut out = String::with_capacity(max_len);
    for ch in text.chars() {
        if out.len() + ch.len_utf8() >= max_len.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn parse_cli_args() -> Result<CliArgs, String> {
    let mut args = std::env::args().skip(1);

    let mut parsed = CliArgs {
        scenario: None,
        output: None,
        iterations_override: None,
        warmup_override: None,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--scenario requires a path".to_owned())?;
                parsed.scenario = Some(PathBuf::from(val));
            }
            "--output" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--output requires a path".to_owned())?;
                parsed.output = Some(PathBuf::from(val));
            }
            "--iterations" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--iterations requires an integer".to_owned())?;
                parsed.iterations_override = Some(parse_usize_flag("--iterations", &val)?);
            }
            "--warmup" => {
                let val = args
                    .next()
                    .ok_or_else(|| "--warmup requires an integer".to_owned())?;
                parsed.warmup_override = Some(parse_usize_flag("--warmup", &val)?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown argument '{other}'. Use --help for usage."));
            }
        }
    }

    Ok(parsed)
}

fn parse_usize_flag(flag: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|e| format!("{flag} expects integer, got '{value}': {e}"))
}

fn print_help() {
    let mut text = String::new();
    let _ = writeln!(
        &mut text,
        "Generic LSP profiling harness for sysml-lsp-server"
    );
    let _ = writeln!(&mut text);
    let _ = writeln!(
        &mut text,
        "Usage: cargo run -p sysml-lsp-server --bin lsp_profile_harness -- [options]"
    );
    let _ = writeln!(&mut text);
    let _ = writeln!(&mut text, "Options:");
    let _ = writeln!(
        &mut text,
        "  --scenario <path>     JSON scenario file (default: built-in full suite)"
    );
    let _ = writeln!(
        &mut text,
        "  --output <path>       Output report path (default: benchmarks/profiles/lsp_profile_<scenario>_<ts>.json)"
    );
    let _ = writeln!(
        &mut text,
        "  --iterations <n>      Override iterations for all workloads"
    );
    let _ = writeln!(
        &mut text,
        "  --warmup <n>          Override warmup runs for all workloads"
    );
    let _ = writeln!(&mut text, "  --help, -h            Show this message");
    let _ = writeln!(&mut text);
    let _ = writeln!(
        &mut text,
        "Scenario schema (JSON): name, documents[], workloads[] with operation.kind"
    );
    let _ = writeln!(
        &mut text,
        "See default suite in source: sysml-lsp-server/src/bin/lsp_profile_harness.rs"
    );
    print!("{text}");
}

const fn default_iterations() -> usize {
    DEFAULT_ITERATIONS
}

const fn default_warmup() -> usize {
    DEFAULT_WARMUP
}

const fn default_tab_size() -> u32 {
    4
}
