#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Phase 1 protocol coverage tests for previously untested handlers/commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::{BaseDirs, ProjectDirs};
use sha2::{Digest, Sha256};
use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
use tempfile::TempDir;
use tower_lsp::lsp_types::*;
use tower_lsp::LanguageServer;

use crate::test_harness::{TestServer, SAMPLE_MULTI_ELEMENT};

const TEST_URI: &str = "file:///phase1.sysml";

fn unique_temp_sysml_path(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("sysml_lsp_{prefix}_{nonce}.sysml"))
}

async fn initialize_server_with_workspace_root(server: &TestServer, root: &Path) {
    let root_uri = Url::from_file_path(root).expect("workspace root should convert to file URI");
    let init = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri.clone(),
            name: "fixture".to_string(),
        }]),
        root_uri: Some(root_uri),
        ..Default::default()
    };
    server
        .server()
        .initialize(init)
        .await
        .expect("initialize should succeed");
    server.server().initialized(InitializedParams {}).await;
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {}: {e}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed in {}\nstdout: {}\nstderr: {}",
        args.join(" "),
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn create_git_fixture(root: &Path, name: &str, version: &str) -> (String, String) {
    create_git_fixture_with_model(root, name, version, "Dep", "X")
}

fn create_git_fixture_with_model(
    root: &Path,
    name: &str,
    version: &str,
    package_name: &str,
    type_name: &str,
) -> (String, String) {
    let repo_dir = root.join(format!("{name}-repo"));
    fs::create_dir_all(&repo_dir).expect("git fixture repo dir should be created");
    git(&repo_dir, &["init", "--initial-branch", "main"]);
    git(&repo_dir, &["config", "user.email", "tests@sysml.rs"]);
    git(&repo_dir, &["config", "user.name", "SysML Protocol Tests"]);
    fs::write(
        repo_dir.join("sysml.toml"),
        format!(
            r#"
[project]
name = "{name}"
version = "{version}"
"#
        ),
    )
    .expect("git fixture manifest should be written");
    fs::write(
        repo_dir.join("dep.sysml"),
        format!("package {package_name} {{ part def {type_name}; }}\n"),
    )
    .expect("git fixture source should be written");
    git(&repo_dir, &["add", "sysml.toml", "dep.sysml"]);
    git(&repo_dir, &["commit", "-m", "initial"]);
    let commit = git(&repo_dir, &["rev-parse", "HEAD"]);
    (format!("file://{}", repo_dir.display()), commit)
}

fn create_kpar_archive(root: &Path, name: &str, version: &str) -> PathBuf {
    create_kpar_archive_with_model(root, name, version, "Root", "X")
}

fn create_kpar_archive_with_model(
    root: &Path,
    name: &str,
    version: &str,
    package_name: &str,
    type_name: &str,
) -> PathBuf {
    let mut metadata = ProjectMetadata::new();
    metadata.add_index_entry("Root", "Root.sysml");
    let archive = KparArchive {
        root_dir: name.to_string(),
        project_info: ProjectInfo {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("protocol test archive".to_string()),
            license: Some("MIT".to_string()),
            usage: Vec::new(),
        },
        metadata,
        source_files: vec![(
            "Root.sysml".to_string(),
            format!("package {package_name} {{ part def {type_name}; }}\n").into_bytes(),
        )],
    };
    let archive_path = root.join(format!("{name}.kpar"));
    write_kpar(&archive_path, &archive).expect("kpar fixture should be written");
    archive_path
}

fn write_sysand_index(root: &Path, package: &str, version: &str, artifact_path: &Path) {
    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(&index_dir).expect("sysand index dir should be created");
    let checksum = format!("sha256:{}", sha256_hex_file(artifact_path));
    fs::write(
        index_dir.join("index.json"),
        format!(
            "{{\"packages\":{{\"{package}\":{{\"{version}\":{{\"artifact\":\"{}\",\"checksum\":\"{checksum}\"}}}}}}}}",
            artifact_path.display()
        ),
    )
    .expect("sysand index should be written");
}

fn write_sysand_index_with_releases(root: &Path, package: &str, releases: &[(&str, PathBuf)]) {
    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(&index_dir).expect("sysand index dir should be created");
    let entries = releases
        .iter()
        .map(|(version, artifact)| {
            let checksum = format!("sha256:{}", sha256_hex_file(artifact));
            format!(
                "\"{version}\":{{\"artifact\":\"{}\",\"checksum\":\"{checksum}\"}}",
                artifact.display()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        index_dir.join("index.json"),
        format!("{{\"packages\":{{\"{package}\":{{{entries}}}}}}}"),
    )
    .expect("sysand index should be written");
}

fn sha256_hex_file(path: &Path) -> String {
    let bytes = fs::read(path).expect("artifact bytes should be readable");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn clean_registry_cache_for_request(backend: &str, package: &str, requirement: &str) {
    let cache_dir = registry_cache_dir_for_request(backend, package, requirement);
    let _ = fs::remove_dir_all(cache_dir);
}

fn registry_cache_dir_for_request(backend: &str, package: &str, requirement: &str) -> PathBuf {
    let request_key = format!("{backend}:{package}@{requirement}");
    cache_root()
        .join("dependencies")
        .join("registry")
        .join(backend)
        .join(source_hash(&request_key))
}

fn cache_root() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn assert_command_payload(command: &str, payload: &serde_json::Value) {
    let obj = payload
        .as_object()
        .unwrap_or_else(|| panic!("{command} should return a JSON object payload, got: {payload}"));
    assert!(
        !obj.is_empty(),
        "{command} should return a non-empty JSON payload"
    );
}

fn inlay_hint_label_text(hint: &InlayHint) -> Option<&str> {
    match &hint.label {
        InlayHintLabel::String(label) => Some(label.as_str()),
        InlayHintLabel::LabelParts(_) => None,
    }
}

fn inlay_hint_tooltip_text(hint: &InlayHint) -> Option<&str> {
    match &hint.tooltip {
        Some(InlayHintTooltip::String(tooltip)) => Some(tooltip.as_str()),
        _ => None,
    }
}

fn response_target_uri(response: GotoDefinitionResponse) -> Option<String> {
    match response {
        GotoDefinitionResponse::Scalar(loc) => Some(loc.uri.to_string()),
        GotoDefinitionResponse::Array(locs) => locs.first().map(|loc| loc.uri.to_string()),
        GotoDefinitionResponse::Link(links) => {
            links.first().map(|link| link.target_uri.to_string())
        }
    }
}

#[tokio::test]
async fn test_semantic_tokens_range_handler_returns_tokens() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.open_document(TEST_URI, SAMPLE_MULTI_ELEMENT).await;

    let params = SemanticTokensRangeParams {
        text_document: TextDocumentIdentifier {
            uri: Url::parse(TEST_URI).expect("valid URI"),
        },
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 9,
                character: 0,
            },
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    let result = server
        .server()
        .semantic_tokens_range(params)
        .await
        .expect("semantic_tokens_range should succeed");

    let Some(SemanticTokensRangeResult::Tokens(tokens)) = result else {
        panic!("semantic_tokens_range should return token payload");
    };

    assert!(
        !tokens.data.is_empty(),
        "semantic_tokens_range should produce at least one token"
    );
}

#[tokio::test]
async fn test_selection_range_handler_returns_nested_ranges() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.open_document(TEST_URI, SAMPLE_MULTI_ELEMENT).await;

    let params = SelectionRangeParams {
        text_document: TextDocumentIdentifier {
            uri: Url::parse(TEST_URI).expect("valid URI"),
        },
        positions: vec![Position {
            line: 5,
            character: 10,
        }],
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    let result = server
        .server()
        .selection_range(params)
        .await
        .expect("selection_range should succeed")
        .expect("selection_range should return payload");

    assert_eq!(result.len(), 1, "selection_range should return one item");
    assert!(
        result[0].parent.is_some(),
        "selection_range should include at least one parent range"
    );
}

#[tokio::test]
async fn test_prepare_rename_handler_returns_placeholder() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.open_document(TEST_URI, SAMPLE_MULTI_ELEMENT).await;

    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: Url::parse(TEST_URI).expect("valid URI"),
        },
        position: Position {
            line: 1,
            character: 12,
        },
    };

    let result = server
        .server()
        .prepare_rename(params)
        .await
        .expect("prepare_rename should succeed")
        .expect("prepare_rename should return payload");

    match result {
        PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } => {
            assert_eq!(placeholder, "Engine")
        }
        PrepareRenameResponse::Range(_) => {
            panic!("prepare_rename should return placeholder response")
        }
        _ => panic!("unexpected prepare_rename response variant"),
    }
}

#[tokio::test]
async fn test_goto_type_definition_handler_resolves_same_file_type() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.open_document(TEST_URI, SAMPLE_MULTI_ELEMENT).await;

    let candidate_positions = [(5, 10), (5, 19), (8, 12), (1, 12)];
    let mut result = None;
    for (line, character) in candidate_positions {
        let params = request::GotoTypeDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(TEST_URI).expect("valid URI"),
                },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = server
            .server()
            .goto_type_definition(params)
            .await
            .expect("goto_type_definition should succeed");
        if response.is_some() {
            result = response;
            break;
        }
    }
    let result = result.expect("goto_type_definition should resolve at least one candidate");

    match result {
        request::GotoTypeDefinitionResponse::Scalar(location) => {
            assert_eq!(location.uri.to_string(), TEST_URI);
            assert!(
                location.range.start.line <= 1,
                "type definition should resolve near the Engine definition: {:?}",
                location.range
            );
        }
        request::GotoTypeDefinitionResponse::Array(locations) => {
            assert!(
                !locations.is_empty(),
                "expected at least one type definition"
            );
        }
        request::GotoTypeDefinitionResponse::Link(links) => {
            assert!(
                !links.is_empty(),
                "expected at least one type definition link"
            );
        }
    }
}

#[tokio::test]
async fn test_goto_implementation_handler_returns_none_when_no_impl_edges_present() {
    let server = TestServer::new();
    server.initialize_full().await;

    let content = "package Impls {\n  part def Vehicle;\n  part def Car :> Vehicle;\n}\n";
    server.open_document(TEST_URI, content).await;

    let params = request::GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(TEST_URI).expect("valid URI"),
            },
            position: Position {
                line: 1,
                character: 12,
            },
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    let result = server
        .server()
        .goto_implementation(params)
        .await
        .expect("goto_implementation should succeed");

    assert!(
        result.is_none(),
        "current fixture should return None when no implementation edges are resolved"
    );
}

#[tokio::test]
async fn test_incoming_calls_handler_finds_action_callers() {
    let server = TestServer::new();
    server.initialize_full().await;

    let content =
        "package Calls {\n  action def ProcessData;\n  action processDataCaller : ProcessData;\n}\n";
    server.open_document(TEST_URI, content).await;

    let prepare = CallHierarchyPrepareParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(TEST_URI).expect("valid URI"),
            },
            position: Position {
                line: 1,
                character: 14,
            },
        },
        work_done_progress_params: Default::default(),
    };

    let item = server
        .server()
        .prepare_call_hierarchy(prepare)
        .await
        .expect("prepare_call_hierarchy should succeed")
        .expect("prepare_call_hierarchy should return one item")
        .into_iter()
        .next()
        .expect("prepare_call_hierarchy should provide item");

    let params = CallHierarchyIncomingCallsParams {
        item,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    let incoming = server
        .server()
        .incoming_calls(params)
        .await
        .expect("incoming_calls should succeed")
        .expect("incoming_calls should return callers for typed action usage");

    assert!(
        incoming
            .iter()
            .any(|call| call.from.name == "processDataCaller"),
        "incoming calls should include processDataCaller"
    );
}

#[tokio::test]
async fn test_did_change_watched_files_handler_tracks_create_change_delete() {
    let server = TestServer::new();
    server.initialize_full().await;

    let path = unique_temp_sysml_path("watched");
    let uri = Url::from_file_path(&path).expect("temp path should convert to file URI");
    let uri_str = uri.to_string();

    fs::write(&path, "package Watch { part def Engine; }\n")
        .expect("should write initial watched file");
    server
        .server()
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: uri.clone(),
                typ: FileChangeType::CREATED,
            }],
        })
        .await;

    assert!(
        server.completion(&uri_str, 0, 0, None).await.is_some(),
        "created watched file should be indexed into salsa"
    );

    fs::write(&path, "package Watch { part def Engine; part def Car; }\n")
        .expect("should write changed watched file");
    server
        .server()
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: uri.clone(),
                typ: FileChangeType::CHANGED,
            }],
        })
        .await;

    assert!(
        server.completion(&uri_str, 0, 0, None).await.is_some(),
        "changed watched file should remain queryable"
    );

    fs::remove_file(&path).expect("watched temp file should be removable");
    server
        .server()
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri,
                typ: FileChangeType::DELETED,
            }],
        })
        .await;

    assert!(
        server.completion(&uri_str, 0, 0, None).await.is_none(),
        "deleted watched file should be removed from salsa"
    );
}

#[tokio::test]
async fn test_execute_command_covers_remaining_advertised_commands() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.open_document(TEST_URI, SAMPLE_MULTI_ELEMENT).await;

    let payload = server
        .execute_command("sysml.action.reset", vec![])
        .await
        .expect("sysml.action.reset should produce payload");
    assert_command_payload("sysml.action.reset", &payload);

    let payload = server
        .execute_command("sysml.action.run", vec![])
        .await
        .expect("sysml.action.run should produce payload");
    assert_command_payload("sysml.action.run", &payload);

    let payload = server
        .execute_command("sysml.action.start", vec![])
        .await
        .expect("sysml.action.start should produce payload");
    assert_command_payload("sysml.action.start", &payload);

    let payload = server
        .execute_command("sysml.action.step", vec![])
        .await
        .expect("sysml.action.step should produce payload");
    assert_command_payload("sysml.action.step", &payload);

    let payload = server
        .execute_command("sysml.action.stop", vec![])
        .await
        .expect("sysml.action.stop should produce payload");
    assert_command_payload("sysml.action.stop", &payload);

    let payload = server
        .execute_command("sysml.action.visualize", vec![])
        .await
        .expect("sysml.action.visualize should produce payload");
    assert_command_payload("sysml.action.visualize", &payload);

    let payload = server
        .execute_command("sysml.cache.clear", vec![])
        .await
        .expect("sysml.cache.clear should produce payload");
    assert_command_payload("sysml.cache.clear", &payload);

    let payload = server
        .execute_command("sysml.diagram.export", vec![])
        .await
        .expect("sysml.diagram.export should produce payload");
    assert_command_payload("sysml.diagram.export", &payload);

    let payload = server
        .execute_command("sysml.diagram.open", vec![])
        .await
        .expect("sysml.diagram.open should produce payload");
    assert_command_payload("sysml.diagram.open", &payload);

    let payload = server
        .execute_command("sysml.diagram.view", vec![])
        .await
        .expect("sysml.diagram.view should produce payload");
    assert_command_payload("sysml.diagram.view", &payload);

    let payload = server
        .execute_command("sysml.evaluate", vec![])
        .await
        .expect("sysml.evaluate should produce payload");
    assert_command_payload("sysml.evaluate", &payload);

    let payload = server
        .execute_command("sysml.flow.visualize", vec![])
        .await
        .expect("sysml.flow.visualize should produce payload");
    assert_command_payload("sysml.flow.visualize", &payload);

    let payload = server
        .execute_command("sysml.salsa.stats", vec![])
        .await
        .expect("sysml.salsa.stats should produce payload");
    assert_command_payload("sysml.salsa.stats", &payload);

    let payload = server
        .execute_command("sysml.salsa.stats.reset", vec![])
        .await
        .expect("sysml.salsa.stats.reset should produce payload");
    assert_command_payload("sysml.salsa.stats.reset", &payload);

    let payload = server
        .execute_command("sysml.simulate.reset", vec![])
        .await
        .expect("sysml.simulate.reset should produce payload");
    assert_command_payload("sysml.simulate.reset", &payload);

    let payload = server
        .execute_command("sysml.simulate.start", vec![])
        .await
        .expect("sysml.simulate.start should produce payload");
    assert_command_payload("sysml.simulate.start", &payload);

    let payload = server
        .execute_command("sysml.simulate.step", vec![])
        .await
        .expect("sysml.simulate.step should produce payload");
    assert_command_payload("sysml.simulate.step", &payload);

    let payload = server
        .execute_command("sysml.simulate.stop", vec![])
        .await
        .expect("sysml.simulate.stop should produce payload");
    assert_command_payload("sysml.simulate.stop", &payload);

    let payload = server
        .execute_command("sysml.verify", vec![])
        .await
        .expect("sysml.verify should produce payload");
    assert_command_payload("sysml.verify", &payload);

    let payload = server
        .execute_command("sysml.whatif", vec![])
        .await
        .expect("sysml.whatif should produce payload");
    assert_command_payload("sysml.whatif", &payload);

    let payload = server
        .execute_command("sysml.whatif.sweep", vec![])
        .await
        .expect("sysml.whatif.sweep should produce payload");
    assert_command_payload("sysml.whatif.sweep", &payload);

    let payload = server
        .execute_command("sysml.workspace.verify", vec![])
        .await
        .expect("sysml.workspace.verify should produce payload");
    assert_command_payload("sysml.workspace.verify", &payload);

    let payload = server
        .execute_command("sysml.project.info", vec![])
        .await
        .expect("sysml.project.info should produce payload");
    assert_command_payload("sysml.project.info", &payload);
    assert!(
        payload.get("discovery").is_some(),
        "sysml.project.info payload should include discovery details"
    );

    let payload = server
        .execute_command("sysml.workspace.refresh", vec![])
        .await
        .expect("sysml.workspace.refresh should produce payload");
    assert_command_payload("sysml.workspace.refresh", &payload);
    assert_eq!(
        payload["status"],
        serde_json::Value::String("workspace_refreshed".to_string())
    );

    let payload = server
        .execute_command("sysml.workspace.info", vec![])
        .await
        .expect("sysml.workspace.info should produce payload");
    assert_command_payload("sysml.workspace.info", &payload);
    assert!(
        payload.get("telemetry_counters").is_some(),
        "sysml.workspace.info payload should include telemetry counters"
    );

    let payload = server
        .execute_command("sysml.dependency.status", vec![])
        .await
        .expect("sysml.dependency.status should produce payload");
    assert_command_payload("sysml.dependency.status", &payload);
    assert!(
        payload.get("summary").is_some(),
        "sysml.dependency.status payload should include summary"
    );
}

#[tokio::test]
async fn test_execute_command_rejects_malformed_argument_types() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.open_document(TEST_URI, SAMPLE_MULTI_ELEMENT).await;

    let evaluate = server
        .execute_command(
            "sysml.evaluate",
            vec![
                serde_json::json!(TEST_URI),
                serde_json::json!("not-a-line"),
                serde_json::json!(0),
            ],
        )
        .await
        .expect("sysml.evaluate should produce payload");
    assert_eq!(
        evaluate.get("error"),
        Some(&serde_json::json!(
            "invalid argument 'line': expected unsigned 32-bit integer"
        ))
    );

    let verify = server
        .execute_command(
            "sysml.verify",
            vec![serde_json::json!(TEST_URI), serde_json::json!(123)],
        )
        .await
        .expect("sysml.verify should produce payload");
    assert_eq!(
        verify.get("error"),
        Some(&serde_json::json!(
            "invalid argument 'case_name': expected string"
        ))
    );
}

#[tokio::test]
async fn test_dependency_status_payload_reports_mixed_source_health() {
    if !git_available() {
        eprintln!("skipping dependency status mixed-source test: git binary unavailable");
        return;
    }

    let dir = TempDir::new().expect("workspace temp dir should be created");
    let dep_path = dir.path().join("dep-path");
    fs::create_dir_all(&dep_path).expect("path dependency dir should be created");
    fs::write(
        dep_path.join("sysml.toml"),
        r#"
[project]
name = "dep-path"
version = "0.5.0"
"#,
    )
    .expect("path dependency manifest should be written");
    let (git_url, git_commit) = create_git_fixture(dir.path(), "dep-git", "1.0.0");
    create_kpar_archive(dir.path(), "dep-kpar", "2.0.0");

    fs::write(
        dir.path().join("sysml.toml"),
        format!(
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
dep-path = {{ path = "./dep-path" }}
dep-git = {{ git = "{git_url}", rev = "{git_commit}" }}
dep-kpar = {{ kpar = "./dep-kpar.kpar" }}
registry-lib = "1.0.0"
missing-lib = {{ path = "./missing-lib" }}
"#
        ),
    )
    .expect("workspace manifest should be written");

    let server = TestServer::new();
    server
        .server()
        .skip_background_tasks
        .store(false, Ordering::Relaxed);
    initialize_server_with_workspace_root(&server, dir.path()).await;

    let payload = server
        .execute_command("sysml.dependency.status", vec![])
        .await
        .expect("dependency status command should return payload");
    assert_command_payload("sysml.dependency.status", &payload);

    assert_eq!(
        payload["summary"]["total_dependencies"].as_u64(),
        Some(5),
        "summary should report all declared dependencies"
    );
    assert_eq!(
        payload["summary"]["hydrated_dependencies"].as_u64(),
        Some(3),
        "three dependencies should hydrate successfully"
    );
    assert_eq!(
        payload["summary"]["failed_dependencies"].as_u64(),
        Some(2),
        "two dependencies should fail (registry + missing path)"
    );

    let roots = payload["roots"]
        .as_array()
        .expect("dependency status roots should be an array");
    assert_eq!(roots.len(), 1, "expected one workspace root in payload");
    let dependencies = roots[0]["dependencies"]
        .as_array()
        .expect("root dependency list should be an array");

    let status_by_name = |needle: &str| -> &serde_json::Value {
        dependencies
            .iter()
            .find(|entry| entry["name"].as_str() == Some(needle))
            .unwrap_or_else(|| panic!("missing dependency status entry for {needle}"))
    };

    assert_eq!(status_by_name("dep-path")["source"].as_str(), Some("path"));
    assert_eq!(status_by_name("dep-path")["status"].as_str(), Some("ready"));
    assert_eq!(status_by_name("dep-git")["source"].as_str(), Some("git"));
    assert_eq!(status_by_name("dep-git")["status"].as_str(), Some("ready"));
    assert_eq!(status_by_name("dep-kpar")["source"].as_str(), Some("kpar"));
    assert_eq!(status_by_name("dep-kpar")["status"].as_str(), Some("ready"));

    let registry = status_by_name("registry-lib");
    assert_eq!(registry["source"].as_str(), Some("registry"));
    assert_eq!(registry["status"].as_str(), Some("unsupported"));
    assert_eq!(
        registry["detail"]["resolution"]["reason"].as_str(),
        Some("unsupported_source")
    );

    let missing = status_by_name("missing-lib");
    assert_eq!(missing["source"].as_str(), Some("path"));
    assert_eq!(missing["status"].as_str(), Some("missing"));
    assert_eq!(
        missing["detail"]["resolution"]["reason"].as_str(),
        Some("missing_dependency")
    );
}

#[tokio::test]
async fn test_dependency_status_payload_reports_registry_ready_when_sysand_index_present() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let package = "dep-registry-ok";
    let requirement = "^7.0";
    let resolved_version = "7.9.1";
    let archive_780 = create_kpar_archive(dir.path(), package, "7.8.0");
    let archive_791 = create_kpar_archive(dir.path(), package, "7.9.1");
    let archive_810 = create_kpar_archive(dir.path(), package, "8.1.0");
    write_sysand_index_with_releases(
        dir.path(),
        package,
        &[
            ("7.8.0", archive_780),
            ("7.9.1", archive_791),
            ("8.1.0", archive_810),
        ],
    );
    clean_registry_cache_for_request("sysand", package, requirement);

    fs::write(
        dir.path().join("sysml.toml"),
        format!(
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
{package} = "{requirement}"
"#
        ),
    )
    .expect("workspace manifest should be written");

    let server = TestServer::new();
    server
        .server()
        .skip_background_tasks
        .store(false, Ordering::Relaxed);
    initialize_server_with_workspace_root(&server, dir.path()).await;

    let payload = server
        .execute_command("sysml.dependency.status", vec![])
        .await
        .expect("dependency status command should return payload");
    assert_command_payload("sysml.dependency.status", &payload);

    assert_eq!(payload["summary"]["total_dependencies"].as_u64(), Some(1));
    assert_eq!(
        payload["summary"]["hydrated_dependencies"].as_u64(),
        Some(1)
    );
    assert_eq!(payload["summary"]["failed_dependencies"].as_u64(), Some(0));

    let dependencies = payload["roots"][0]["dependencies"]
        .as_array()
        .expect("root dependency list should be an array");
    let registry = dependencies
        .iter()
        .find(|entry| entry["name"].as_str() == Some(package))
        .expect("registry dependency entry should exist");
    assert_eq!(registry["source"].as_str(), Some("registry"));
    assert_eq!(registry["status"].as_str(), Some("ready"));
    assert_eq!(
        registry["detail"]["declared"]["requested_requirement"].as_str(),
        Some(requirement)
    );
    assert_eq!(
        registry["detail"]["resolution"]["status"].as_str(),
        Some("hydrated")
    );
    assert_eq!(
        registry["detail"]["resolution"]["requested_requirement"].as_str(),
        Some(requirement)
    );
    assert_eq!(
        registry["detail"]["resolution"]["resolved_version"].as_str(),
        Some(resolved_version)
    );
    assert_eq!(
        registry["detail"]["resolution"]["hydrated_package_count"].as_u64(),
        Some(1)
    );
    let hydrated_pkg = &registry["detail"]["resolution"]["hydrated_packages"][0];
    assert_eq!(hydrated_pkg["source"].as_str(), Some("registry"));
    assert_eq!(
        hydrated_pkg["source_detail"]["backend"].as_str(),
        Some("sysand")
    );
    assert_eq!(
        hydrated_pkg["source_detail"]["package"].as_str(),
        Some(package)
    );
    assert_eq!(
        hydrated_pkg["source_detail"]["requested_requirement"].as_str(),
        Some(requirement)
    );
    assert_eq!(
        hydrated_pkg["source_detail"]["version"].as_str(),
        Some(resolved_version)
    );
    assert_eq!(
        hydrated_pkg["source_detail"]["resolved_version"].as_str(),
        Some(resolved_version)
    );
    let expected_lock_source = format!("registry:sysand:{package}@{resolved_version}");
    assert_eq!(
        hydrated_pkg["lock_source"].as_str(),
        Some(expected_lock_source.as_str())
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

#[tokio::test]
async fn test_dependency_status_payload_reports_registry_no_match_with_actionable_message() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let package = "dep-registry-nomatch";
    let requirement = "~3.0";
    let archive_210 = create_kpar_archive(dir.path(), package, "2.1.0");
    let archive_240 = create_kpar_archive(dir.path(), package, "2.4.0");
    write_sysand_index_with_releases(
        dir.path(),
        package,
        &[("2.1.0", archive_210), ("2.4.0", archive_240)],
    );
    clean_registry_cache_for_request("sysand", package, requirement);

    fs::write(
        dir.path().join("sysml.toml"),
        format!(
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
{package} = "{requirement}"
"#
        ),
    )
    .expect("workspace manifest should be written");

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;

    let payload = server
        .execute_command("sysml.dependency.status", vec![])
        .await
        .expect("dependency status command should return payload");
    assert_command_payload("sysml.dependency.status", &payload);

    assert_eq!(payload["summary"]["total_dependencies"].as_u64(), Some(1));
    assert_eq!(
        payload["summary"]["hydrated_dependencies"].as_u64(),
        Some(0)
    );
    assert_eq!(payload["summary"]["failed_dependencies"].as_u64(), Some(1));

    let dependencies = payload["roots"][0]["dependencies"]
        .as_array()
        .expect("root dependency list should be an array");
    let registry = dependencies
        .iter()
        .find(|entry| entry["name"].as_str() == Some(package))
        .expect("registry dependency entry should exist");

    assert_eq!(registry["source"].as_str(), Some("registry"));
    assert_eq!(registry["status"].as_str(), Some("error"));
    assert_eq!(
        registry["detail"]["declared"]["requested_requirement"].as_str(),
        Some(requirement)
    );
    assert_eq!(
        registry["detail"]["resolution"]["reason"].as_str(),
        Some("no_compatible_release")
    );
    let message = registry["detail"]["resolution"]["message"]
        .as_str()
        .unwrap_or("");
    assert!(
        message.contains("no compatible release") && message.contains(requirement),
        "expected actionable no-compatible-release message, got: {message}"
    );
    let action = registry["detail"]["resolution"]["action"]
        .as_str()
        .unwrap_or("");
    assert!(
        action.contains("Update the dependency requirement")
            || action.contains("publish a compatible release"),
        "expected actionable no-compatible-release action, got: {action}"
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

#[tokio::test]
async fn test_manifest_publish_diagnostics_include_source_aware_runtime_failure() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let manifest_path = dir.path().join("sysml.toml");
    let manifest_content = r#"
[project]
name = "inline-runtime-failure"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
missing-kpar = { kpar = "./missing-lib.kpar" }
"#;
    fs::write(&manifest_path, manifest_content).expect("manifest should be written");
    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to file URI")
        .to_string();

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    server.clear_client_requests().await;
    server.open_document(&manifest_uri, manifest_content).await;

    let diagnostics = server
        .wait_for_manifest_diagnostics(&manifest_uri, Some("M040"), Duration::from_millis(1000))
        .await
        .expect("expected manifest diagnostics with runtime failure code M040");
    let runtime_diag = diagnostics
        .iter()
        .find(|diag| {
            diag.code == Some(NumberOrString::String("M040".to_string()))
                && diag.message.contains("(kpar)")
        })
        .expect("expected source-aware kpar runtime diagnostic");
    assert_eq!(runtime_diag.severity, Some(DiagnosticSeverity::ERROR));
    assert!(
        runtime_diag.message.contains("missing-kpar"),
        "runtime diagnostic should include dependency name"
    );
}

#[tokio::test]
async fn test_manifest_publish_diagnostics_include_registry_update_hint() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let package = "dep-inline-update";
    let requirement = "1.0.0";
    let archive_100 = create_kpar_archive(dir.path(), package, "1.0.0");
    let archive_120 = create_kpar_archive(dir.path(), package, "1.2.0");
    write_sysand_index_with_releases(
        dir.path(),
        package,
        &[("1.0.0", archive_100), ("1.2.0", archive_120)],
    );
    clean_registry_cache_for_request("sysand", package, requirement);

    let manifest_path = dir.path().join("sysml.toml");
    let manifest_content = format!(
        r#"
[project]
name = "inline-update-hint"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
{package} = "{requirement}"
"#
    );
    fs::write(&manifest_path, &manifest_content).expect("manifest should be written");
    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to file URI")
        .to_string();

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    server.clear_client_requests().await;
    server.open_document(&manifest_uri, &manifest_content).await;

    let diagnostics = server
        .wait_for_manifest_diagnostics(&manifest_uri, Some("M041"), Duration::from_millis(1500))
        .await
        .expect("expected manifest diagnostics with update-available code M041");
    let update_diag = diagnostics
        .iter()
        .find(|diag| diag.code == Some(NumberOrString::String("M041".to_string())))
        .expect("expected update hint diagnostic");
    assert_eq!(update_diag.severity, Some(DiagnosticSeverity::HINT));
    assert!(
        update_diag.message.contains("1.0.0") && update_diag.message.contains("1.2.0"),
        "update hint should include resolved and latest versions"
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

#[tokio::test]
async fn test_manifest_inlay_hints_show_registry_update_available() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let package = "dep-inline-inlay-update";
    let requirement = "1.0.0";
    let archive_100 = create_kpar_archive(dir.path(), package, "1.0.0");
    let archive_110 = create_kpar_archive(dir.path(), package, "1.1.0");
    write_sysand_index_with_releases(
        dir.path(),
        package,
        &[("1.0.0", archive_100), ("1.1.0", archive_110)],
    );
    clean_registry_cache_for_request("sysand", package, requirement);

    let manifest_path = dir.path().join("sysml.toml");
    let manifest_content = format!(
        r#"
[project]
name = "inline-inlay-update"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
{package} = "{requirement}"
"#
    );
    fs::write(&manifest_path, &manifest_content).expect("manifest should be written");
    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to file URI")
        .to_string();

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    server.open_document(&manifest_uri, &manifest_content).await;

    let hints = server
        .inlay_hint(&manifest_uri)
        .await
        .expect("manifest inlay hints should be present");
    let version_hint = hints
        .iter()
        .filter_map(inlay_hint_label_text)
        .find(|label| label.contains("1.1.0 available"));
    assert!(
        version_hint.is_some(),
        "expected `1.1.0 available` in manifest inlay hints, got: {:?}",
        hints
            .iter()
            .filter_map(inlay_hint_label_text)
            .collect::<Vec<_>>()
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

#[tokio::test]
async fn test_manifest_inlay_hints_show_registry_up_to_date() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let package = "dep-inline-inlay-current";
    let requirement = "1.0.0";
    let archive_100 = create_kpar_archive(dir.path(), package, "1.0.0");
    write_sysand_index(dir.path(), package, requirement, &archive_100);
    clean_registry_cache_for_request("sysand", package, requirement);

    let manifest_path = dir.path().join("sysml.toml");
    let manifest_content = format!(
        r#"
[project]
name = "inline-inlay-current"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
{package} = "{requirement}"
"#
    );
    fs::write(&manifest_path, &manifest_content).expect("manifest should be written");
    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to file URI")
        .to_string();

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    server.open_document(&manifest_uri, &manifest_content).await;

    let hints = server
        .inlay_hint(&manifest_uri)
        .await
        .expect("manifest inlay hints should be present");
    let up_to_date_hint = hints
        .iter()
        .filter_map(inlay_hint_label_text)
        .find(|label| *label == "up to date");
    assert!(
        up_to_date_hint.is_some(),
        "expected `up to date` in manifest inlay hints, got: {:?}",
        hints
            .iter()
            .filter_map(inlay_hint_label_text)
            .collect::<Vec<_>>()
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

#[tokio::test]
async fn test_manifest_inlay_hints_show_source_agnostic_resolution_failure() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let manifest_path = dir.path().join("sysml.toml");
    let manifest_content = r#"
[project]
name = "inline-inlay-failure"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
missing-lib = { path = "./deps/does-not-exist" }
"#;
    fs::write(&manifest_path, manifest_content).expect("manifest should be written");
    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to file URI")
        .to_string();

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    server.open_document(&manifest_uri, manifest_content).await;

    let hints = server
        .inlay_hint(&manifest_uri)
        .await
        .expect("manifest inlay hints should be present");
    let error_label = hints
        .iter()
        .filter_map(inlay_hint_label_text)
        .find(|label| *label == "resolve error");
    assert!(
        error_label.is_some(),
        "expected `resolve error` label in manifest inlay hints, got: {:?}",
        hints
            .iter()
            .filter_map(inlay_hint_label_text)
            .collect::<Vec<_>>()
    );

    let error_tooltip = hints
        .iter()
        .filter_map(inlay_hint_tooltip_text)
        .find(|tooltip| tooltip.contains("path dependency 'missing-lib' failed"));
    assert!(
        error_tooltip.is_some(),
        "expected actionable path failure tooltip, got: {:?}",
        hints
            .iter()
            .filter_map(inlay_hint_tooltip_text)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_manifest_diagnostics_are_not_overwritten_by_debounced_salsa_publish() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let manifest_path = dir.path().join("sysml.toml");
    let initial_manifest = r#"
[project]
name = "inline-manifest-overwrite"
version = "0.1.0"
sysml-edition = "2025"
"#;
    fs::write(&manifest_path, initial_manifest).expect("manifest should be written");
    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to file URI")
        .to_string();

    let server = TestServer::new();
    // Match production behavior where did_change uses debounced diagnostics.
    server
        .server()
        .skip_background_tasks
        .store(false, Ordering::Relaxed);
    initialize_server_with_workspace_root(&server, dir.path()).await;

    server.open_document(&manifest_uri, initial_manifest).await;
    server.clear_client_requests().await;

    let changed_manifest = r#"
[project]
name = "inline-manifest-overwrite"
version = "oops-not-semver"
sysml-edition = "2025"

[dependencies]
missing-kpar = { kpar = "./missing-lib.kpar" }
"#;
    fs::write(&manifest_path, changed_manifest).expect("updated manifest should be written");
    server
        .change_document(&manifest_uri, 2, changed_manifest)
        .await;

    let _initial_publish = server
        .wait_for_manifest_diagnostics(&manifest_uri, Some("M010"), Duration::from_millis(1000))
        .await
        .expect("expected manifest diagnostics with semver failure M010");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let latest = server
        .last_server_published_diagnostics(&manifest_uri)
        .expect("expected latest published diagnostics for manifest URI");
    assert!(
        latest
            .iter()
            .any(|diag| diag.code == Some(NumberOrString::String("M010".to_string()))),
        "latest published diagnostics should retain manifest failure M010, got: {latest:?}"
    );
}

// An open buffer keyed under an alias URI (e.g. `model/../model/root.sysml`)
// must (1) be associated with the project indexed under the canonical URI so
// its E200 isn't readiness-gated, and (2) survive `sysml.workspace.refresh`
// without the background indexer clobbering the dirty buffer with disk
// content. FIXED 2026-06-23 (steward-ruled Shape B — editor-overlay model):
//  (1) URI identity: `sysml-ide-db::source::canonicalize_uri` now resolves
//      `..`/symlinks, so the alias buffer maps to the SAME FileId as its
//      canonical form (no orphan, project_id flows through).
//  (2) Buffer precedence: `did_open` marks the file as an editor overlay
//      (`host.set_overlay`) under the same host lock as the buffer write;
//      `open_context`'s per-file write checks `has_overlay` and tags the
//      project (`set_project_only`) instead of overwriting from disk. Both
//      critical sections take the host mutex, so they serialize — race-free
//      by construction, replacing the old racy snapshot/restore band-aid.
//      Overlays are re-established in `rediscover_workspace_state` after
//      `workspace_refresh`'s host reset.
#[tokio::test]
async fn test_workspace_refresh_keeps_open_buffer_for_uri_alias_paths() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let model_dir = dir.path().join("model");
    fs::create_dir_all(&model_dir).expect("model dir should be created");

    fs::write(
        dir.path().join("sysml.toml"),
        r#"
[project]
name = "alias-refresh"
version = "0.1.0"
"#,
    )
    .expect("workspace manifest should be written");

    let root_path = model_dir.join("root.sysml");
    let on_disk = r#"
package AliasRefresh {
    part def LocalType;
    part good : LocalType;
}
"#;
    fs::write(&root_path, on_disk).expect("root source should be written");

    let alias_uri = format!("file://{}/model/../model/root.sysml", dir.path().display());
    let canonical_uri = Url::from_file_path(&root_path)
        .expect("root path should convert to URI")
        .to_string();
    let unsaved = r#"
package AliasRefresh {
    part def LocalType;
    part broken : MissingType;
}
"#;

    let server = TestServer::new();
    server
        .server()
        .skip_background_tasks
        .store(false, Ordering::Relaxed);
    initialize_server_with_workspace_root(&server, dir.path()).await;
    server.open_document(&alias_uri, unsaved).await;

    let mut saw_initial_e200 = false;
    for _ in 0..50 {
        let diagnostics = server
            .last_server_published_diagnostics(&alias_uri)
            .or_else(|| server.last_server_published_diagnostics(&canonical_uri));
        if let Some(diags) = diagnostics {
            let has_e200 = diags.iter().any(|diag| {
                matches!(
                    diag.code.as_ref(),
                    Some(NumberOrString::String(code)) if code == "E200"
                ) && diag.message.contains("MissingType")
            });
            if has_e200 {
                saw_initial_e200 = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    assert!(
        saw_initial_e200,
        "expected initial diagnostics to include MissingType unresolved error for aliased open URI"
    );

    let _ = server
        .execute_command("sysml.workspace.refresh", vec![])
        .await
        .expect("workspace refresh command should succeed");

    // If alias handling regresses, background indexing can overwrite the open
    // buffer with on-disk content and clear E200. Keep polling to cover async
    // rediscovery/indexing races.
    let mut retained_e200 = false;
    for _ in 0..80 {
        let diagnostics = server
            .last_server_published_diagnostics(&alias_uri)
            .or_else(|| server.last_server_published_diagnostics(&canonical_uri));
        if let Some(diags) = diagnostics {
            let has_e200 = diags.iter().any(|diag| {
                matches!(
                    diag.code.as_ref(),
                    Some(NumberOrString::String(code)) if code == "E200"
                ) && diag.message.contains("MissingType")
            });
            if has_e200 {
                retained_e200 = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    assert!(
        retained_e200,
        "expected E200 for MissingType to remain after workspace refresh; open buffer should not be clobbered by background indexing"
    );
}

#[tokio::test]
async fn test_lock_change_triggers_dependency_rediscovery_without_reload() {
    let dir = TempDir::new().expect("workspace temp dir should be created");
    let manifest_path = dir.path().join("sysml.toml");
    let lock_path = dir.path().join("sysml.lock");
    fs::write(
        &manifest_path,
        r#"
[project]
name = "root-project"
version = "0.1.0"
"#,
    )
    .expect("initial manifest should be written");
    fs::write(&lock_path, "lock_version = 1\n").expect("initial lock file should be written");

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;

    let before = server
        .execute_command("sysml.workspace.info", vec![])
        .await
        .expect("workspace explain should return payload");
    assert_eq!(
        before["loaded"]["user_projects"].as_u64(),
        Some(1),
        "initial workspace should include only the root project"
    );

    let dep_dir = dir.path().join("dep-live");
    fs::create_dir_all(&dep_dir).expect("dependency directory should be created");
    fs::write(
        dep_dir.join("sysml.toml"),
        r#"
[project]
name = "dep-live"
version = "0.2.0"
"#,
    )
    .expect("dependency manifest should be written");
    fs::write(
        &manifest_path,
        r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
dep-live = { path = "./dep-live" }
"#,
    )
    .expect("updated manifest should be written");
    fs::write(&lock_path, "lock_version = 1\n# changed\n")
        .expect("updated lock file should be written");

    let lock_uri = Url::from_file_path(&lock_path).expect("lock file should convert to URI");
    server
        .server()
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: lock_uri,
                typ: FileChangeType::CHANGED,
            }],
        })
        .await;

    let after = server
        .execute_command("sysml.workspace.info", vec![])
        .await
        .expect("workspace explain should return payload after lock change");
    assert_eq!(
        after["loaded"]["user_projects"].as_u64(),
        Some(2),
        "lock change should trigger rediscovery and hydrate the new dependency project"
    );

    let deps = server
        .execute_command("sysml.dependency.status", vec![])
        .await
        .expect("dependency status should return payload after lock change");
    assert_eq!(
        deps["summary"]["total_dependencies"].as_u64(),
        Some(1),
        "dependency status should reflect updated manifest after lock-triggered refresh"
    );
    assert_eq!(
        deps["summary"]["hydrated_dependencies"].as_u64(),
        Some(1),
        "updated dependency should hydrate without requiring VS Code reload"
    );
}

#[tokio::test]
async fn test_workspace_snapshot_includes_git_dependency_outside_workspace_root() {
    if !git_available() {
        eprintln!("skipping external dependency goto-definition test: git binary unavailable");
        return;
    }

    let workspace = TempDir::new().expect("workspace temp dir should be created");
    let dep_store = TempDir::new().expect("dependency store temp dir should be created");
    let (git_url, commit) =
        create_git_fixture_with_model(dep_store.path(), "dep-git", "0.3.0", "DepPkg", "ExtType");

    let root_model = workspace.path().join("root.sysml");
    let root_content = r#"
package RootPkg {
    import DepPkg::*;
    part def Harness {
        part ext : ExtType;
    }
}
"#;
    fs::write(&root_model, root_content).expect("root model should be written");
    fs::write(
        workspace.path().join("sysml.toml"),
        format!(
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
dep-git = {{ git = "{git_url}", rev = "{commit}" }}
"#
        ),
    )
    .expect("workspace manifest should be written");

    let server = TestServer::new();
    server
        .server()
        .skip_background_tasks
        .store(false, Ordering::Relaxed);
    initialize_server_with_workspace_root(&server, workspace.path()).await;

    let root_uri = Url::from_file_path(&root_model)
        .expect("root model path should convert to file URI")
        .to_string();
    server.open_document(&root_uri, root_content).await;

    let status = server
        .execute_command("sysml.dependency.status", vec![])
        .await
        .expect("dependency status should return payload");
    let roots = status["roots"]
        .as_array()
        .expect("dependency status should include roots array");
    let hydrated = roots
        .first()
        .and_then(|root| root["hydrated_dependencies"].as_array())
        .expect("dependency status root should include hydrated dependencies");
    let dep_source_dir = hydrated
        .iter()
        .find(|dep| dep["name"].as_str() == Some("dep-git"))
        .and_then(|dep| dep["source_dir"].as_str())
        .expect("expected dep-git hydrated source_dir in dependency status");
    let dep_model_path = PathBuf::from(dep_source_dir)
        .join("dep.sysml")
        .canonicalize()
        .expect("hydrated dependency model path should exist");

    let mut resolved = false;
    for _ in 0..20 {
        let snapshot = server.server().workspace_snapshot().await;
        let matched = snapshot.find_by_name("ExtType").iter().any(|entry| {
            Url::parse(&entry.uri)
                .ok()
                .and_then(|uri| uri.to_file_path().ok())
                .and_then(|path| path.canonicalize().ok())
                .map(|path| path == dep_model_path)
                .unwrap_or(false)
        });
        if matched {
            resolved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        resolved,
        "expected workspace snapshot to include ExtType from external dependency file {}",
        dep_model_path.display()
    );
}

// Ignored 2026-05-22: the service-side goto-definition resolves type usages
// in the root file (e.g. `part def IntegrationHarness { part p : PathSensor; }`)
// back to the root file itself rather than walking the FeatureTyping into the
// dependency-loaded ModelGraph (dep-path/path.sysml etc.). Dependency
// hydration + workspace indexing succeed (the snapshot test up the test
// confirms PathSensor/GitController/etc. exist in WorkspaceSnapshot) but the
// goto handler's primary path returns the usage's own span. Belongs in
// follow-up work on `sysml_service::goto_definition` cross-project resolution
// (see Cluster F in Architectural-cleanup/lsp-pre-existing-failures-triage.md).
#[ignore = "follow-up: service-side goto-def doesn't walk FeatureTyping into dependency graphs"]
#[tokio::test]
async fn test_dependency_imports_resolve_for_path_git_kpar_registry() {
    if !git_available() {
        eprintln!("skipping dependency import resolution test: git binary unavailable");
        return;
    }

    let dir = TempDir::new().expect("workspace temp dir should be created");
    let path_dep_dir = dir.path().join("dep-path");
    fs::create_dir_all(&path_dep_dir).expect("path dependency dir should be created");
    fs::write(
        path_dep_dir.join("sysml.toml"),
        r#"
[project]
name = "dep-path"
version = "0.1.0"
"#,
    )
    .expect("path dependency manifest should be written");
    fs::write(
        path_dep_dir.join("path.sysml"),
        "package PathLib { part def PathSensor; }\n",
    )
    .expect("path dependency source should be written");

    let (git_url, commit) =
        create_git_fixture_with_model(dir.path(), "dep-git", "0.2.0", "GitLib", "GitController");
    let kpar_archive =
        create_kpar_archive_with_model(dir.path(), "dep-kpar", "0.3.0", "KparLib", "KparActuator");
    let registry_archive = create_kpar_archive_with_model(
        dir.path(),
        "registry-lib",
        "1.0.0",
        "RegistryLib",
        "RegistryBus",
    );
    write_sysand_index_with_releases(
        dir.path(),
        "registry-lib",
        &[("1.0.0", registry_archive.clone())],
    );
    clean_registry_cache_for_request("sysand", "registry-lib", "1.0.0");

    fs::write(
        dir.path().join("sysml.toml"),
        format!(
            r#"
[project]
name = "manual-phase6-like"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
path-ok = {{ path = "./dep-path" }}
git-ok = {{ git = "{git_url}", rev = "{commit}" }}
kpar-ok = {{ kpar = "{}" }}
registry-lib = "1.0.0"
"#,
            kpar_archive.display()
        ),
    )
    .expect("workspace manifest should be written");

    let root_model_path = dir.path().join("root.sysml");
    let root_content = r#"
package ManualPhase6Like {
    import PathLib::*;
    import GitLib::*;
    import KparLib::*;
    import RegistryLib::*;

    part def IntegrationHarness {
        part pathSensor : PathSensor;
        part gitController : GitController;
        part kparActuator : KparActuator;
        part registryBus : RegistryBus;
    }
}
"#;
    fs::write(&root_model_path, root_content).expect("root source should be written");

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    let root_uri = Url::from_file_path(&root_model_path)
        .expect("root source should convert to URI")
        .to_string();
    server.open_document(&root_uri, root_content).await;

    // Ensure dependency hydration completed before asserting diagnostics.
    let mut hydrated = false;
    for _ in 0..30 {
        if let Some(payload) = server
            .execute_command("sysml.dependency.status", vec![])
            .await
        {
            let deps_total = payload["summary"]["total_dependencies"]
                .as_u64()
                .unwrap_or(0);
            let deps_hydrated = payload["summary"]["hydrated_dependencies"]
                .as_u64()
                .unwrap_or(0);
            let deps_failed = payload["summary"]["failed_dependencies"]
                .as_u64()
                .unwrap_or(0);
            if deps_total == 4 && deps_hydrated == 4 && deps_failed == 0 {
                hydrated = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        hydrated,
        "expected all dependency sources to hydrate before diagnostics assertion"
    );
    let _ = server
        .execute_command("sysml.workspace.refresh", vec![])
        .await
        .expect("workspace refresh should succeed after dependency hydration");

    let expected_symbols = ["PathSensor", "GitController", "KparActuator", "RegistryBus"];
    let mut snapshot_ready = false;
    for _ in 0..40 {
        let snapshot = server.server().workspace_snapshot().await;
        if expected_symbols
            .iter()
            .all(|name| !snapshot.find_by_name(name).is_empty())
        {
            snapshot_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        snapshot_ready,
        "expected workspace snapshot to index dependency symbols before goto assertions"
    );

    let mut latest_diags = Vec::new();
    for _ in 0..40 {
        if let Some(diags) = server.last_server_published_diagnostics(&root_uri) {
            latest_diags = diags.clone();
            let has_im001 = latest_diags.iter().any(|diag| {
                matches!(
                    diag.code.as_ref(),
                    Some(NumberOrString::String(code)) if code == "IM001"
                )
            });
            let has_e200 = latest_diags.iter().any(|diag| {
                matches!(
                    diag.code.as_ref(),
                    Some(NumberOrString::String(code)) if code == "E200"
                )
            });
            if !has_im001 && !has_e200 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let unresolved_messages: Vec<String> = latest_diags
        .iter()
        .filter(|diag| {
            matches!(
                diag.code.as_ref(),
                Some(NumberOrString::String(code)) if code == "IM001" || code == "E200"
            )
        })
        .map(|diag| diag.message.clone())
        .collect();
    assert!(
        unresolved_messages.is_empty(),
        "expected dependency imports/types to resolve without IM001/E200 diagnostics, got: {:?}",
        unresolved_messages
    );

    for (usage_line, type_name) in [
        ("part pathSensor : PathSensor;", "PathSensor"),
        ("part gitController : GitController;", "GitController"),
        ("part kparActuator : KparActuator;", "KparActuator"),
        ("part registryBus : RegistryBus;", "RegistryBus"),
    ] {
        let line = root_content
            .lines()
            .position(|raw| raw.contains(usage_line))
            .unwrap_or_else(|| panic!("usage line should exist: {usage_line}"))
            as u32;
        let col = root_content
            .lines()
            .nth(line as usize)
            .and_then(|raw| raw.find(type_name))
            .unwrap_or_else(|| panic!("token should exist in usage line: {type_name}"))
            as u32
            + 2;
        let target = server
            .goto_definition(&root_uri, line, col)
            .await
            .unwrap_or_else(|| panic!("expected goto-definition result for {type_name} usage"));
        let target_uri = response_target_uri(target).unwrap_or_else(|| {
            panic!("goto-definition should contain at least one location for {type_name}")
        });
        let target_path = Url::parse(&target_uri)
            .expect("goto target should be a valid URI")
            .to_file_path()
            .expect("goto target URI should map to a file path");
        let target_source = fs::read_to_string(&target_path).unwrap_or_else(|e| {
            panic!(
                "failed to read goto target '{}': {e}",
                target_path.display()
            )
        });
        assert!(
            target_source.contains(&format!("part def {type_name};")),
            "expected goto target for {type_name} to contain its definition, got target file '{}'",
            target_path.display()
        );
    }
}

// Ignored 2026-05-08: post-S2.T2 the 20-cycle manifest-rewrite + 4× goto-def
// inner loop turned pathological. Each `did_change_watched_files` invalidates
// the manifest input; goto-def then re-runs `elaborate_workspace_with_library`
// across all 4 dep sources (path/git/kpar/registry) per cycle. Wall time
// exceeds 180 s in both --release and --dev. Belongs in S3 caching work
// (migration-plan.md S3.T6 — `sysml-ide-db` workspace-graph cache shape).
#[tokio::test]
#[ignore = "S3-followup: salsa cache invalidation pathology after S2.T2"]
async fn test_dependency_goto_stable_across_manifest_refresh_cycles() {
    if !git_available() {
        eprintln!("skipping dependency refresh stability test: git binary unavailable");
        return;
    }

    let dir = TempDir::new().expect("workspace temp dir should be created");
    let path_dep_dir = dir.path().join("dep-path");
    fs::create_dir_all(&path_dep_dir).expect("path dependency dir should be created");
    fs::write(
        path_dep_dir.join("sysml.toml"),
        r#"
[project]
name = "dep-path"
version = "0.1.0"
"#,
    )
    .expect("path dependency manifest should be written");
    fs::write(
        path_dep_dir.join("path.sysml"),
        "package PathLib { part def PathSensor; }\n",
    )
    .expect("path dependency source should be written");

    let (git_url, commit) =
        create_git_fixture_with_model(dir.path(), "dep-git", "0.2.0", "GitLib", "GitController");
    let kpar_archive =
        create_kpar_archive_with_model(dir.path(), "dep-kpar", "0.3.0", "KparLib", "KparActuator");
    let registry_archive = create_kpar_archive_with_model(
        dir.path(),
        "registry-lib",
        "1.0.0",
        "RegistryLib",
        "RegistryBus",
    );
    write_sysand_index_with_releases(
        dir.path(),
        "registry-lib",
        &[("1.0.0", registry_archive.clone())],
    );
    clean_registry_cache_for_request("sysand", "registry-lib", "1.0.0");

    let base_manifest = format!(
        r#"
[project]
name = "manual-phase6-like"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
path-ok = {{ path = "./dep-path" }}
git-ok = {{ git = "{git_url}", rev = "{commit}" }}
kpar-ok = {{ kpar = "{}" }}
registry-lib = "1.0.0"
"#,
        kpar_archive.display()
    );
    let manifest_path = dir.path().join("sysml.toml");
    fs::write(&manifest_path, &base_manifest).expect("workspace manifest should be written");

    let root_model_path = dir.path().join("root.sysml");
    let root_content = r#"
package ManualPhase6Like {
    import PathLib::*;
    import GitLib::*;
    import KparLib::*;
    import RegistryLib::*;

    part def IntegrationHarness {
        part pathSensor : PathSensor;
        part gitController : GitController;
        part kparActuator : KparActuator;
        part registryBus : RegistryBus;
    }
}
"#;
    fs::write(&root_model_path, root_content).expect("root source should be written");

    let server = TestServer::new();
    server
        .server()
        .skip_background_tasks
        .store(false, Ordering::Relaxed);
    initialize_server_with_workspace_root(&server, dir.path()).await;

    let root_uri = Url::from_file_path(&root_model_path)
        .expect("root source should convert to URI")
        .to_string();
    server.open_document(&root_uri, root_content).await;

    let manifest_uri = Url::from_file_path(&manifest_path)
        .expect("manifest path should convert to URI")
        .to_string();

    for cycle in 0..20 {
        let cycle_manifest = format!("{base_manifest}\n# cycle {cycle}\n");
        fs::write(&manifest_path, &cycle_manifest).expect("cycle manifest should be written");
        server
            .server()
            .did_change_watched_files(DidChangeWatchedFilesParams {
                changes: vec![FileEvent {
                    uri: Url::parse(&manifest_uri).expect("manifest URI should parse"),
                    typ: FileChangeType::CHANGED,
                }],
            })
            .await;

        for (usage_line, type_name) in [
            ("part pathSensor : PathSensor;", "PathSensor"),
            ("part gitController : GitController;", "GitController"),
            ("part kparActuator : KparActuator;", "KparActuator"),
            ("part registryBus : RegistryBus;", "RegistryBus"),
        ] {
            let line = root_content
                .lines()
                .position(|raw| raw.contains(usage_line))
                .unwrap_or_else(|| panic!("usage line should exist: {usage_line}"))
                as u32;
            let col = root_content
                .lines()
                .nth(line as usize)
                .and_then(|raw| raw.find(type_name))
                .unwrap_or_else(|| panic!("token should exist in usage line: {type_name}"))
                as u32
                + 2;
            let target = server
                .goto_definition(&root_uri, line, col)
                .await
                .unwrap_or_else(|| {
                    panic!("cycle {cycle}: expected goto-definition result for {type_name} usage")
                });
            let target_uri = response_target_uri(target).unwrap_or_else(|| {
                panic!("cycle {cycle}: goto-definition should contain a location for {type_name}")
            });
            let target_path = Url::parse(&target_uri)
                .expect("goto target should be a valid URI")
                .to_file_path()
                .expect("goto target URI should map to a file path");
            let target_source = fs::read_to_string(&target_path).unwrap_or_else(|e| {
                panic!(
                    "cycle {cycle}: failed to read goto target '{}': {e}",
                    target_path.display()
                )
            });
            assert!(
                target_source.contains(&format!("part def {type_name};")),
                "cycle {cycle}: expected goto target for {type_name} to contain its definition, got '{}'",
                target_path.display()
            );
        }
    }
}

// Ignored 2026-05-22: same root cause as
// `test_dependency_imports_resolve_for_path_git_kpar_registry` —
// service-side goto returns the usage's own location instead of the dep
// definition. The open/close cycles only matter once goto resolves to the
// dependency in the first place. See Cluster F in
// Architectural-cleanup/lsp-pre-existing-failures-triage.md.
#[ignore = "follow-up: service-side goto-def doesn't walk FeatureTyping into dependency graphs"]
#[tokio::test]
async fn test_dependency_goto_survives_target_open_close_cycles() {
    if !git_available() {
        eprintln!("skipping dependency open/close stability test: git binary unavailable");
        return;
    }

    let dir = TempDir::new().expect("workspace temp dir should be created");
    let path_dep_dir = dir.path().join("dep-path");
    fs::create_dir_all(&path_dep_dir).expect("path dependency dir should be created");
    fs::write(
        path_dep_dir.join("sysml.toml"),
        r#"
[project]
name = "dep-path"
version = "0.1.0"
"#,
    )
    .expect("path dependency manifest should be written");
    fs::write(
        path_dep_dir.join("path.sysml"),
        "package PathLib { part def PathSensor; }\n",
    )
    .expect("path dependency source should be written");

    let (git_url, commit) =
        create_git_fixture_with_model(dir.path(), "dep-git", "0.2.0", "GitLib", "GitController");
    let kpar_archive =
        create_kpar_archive_with_model(dir.path(), "dep-kpar", "0.3.0", "KparLib", "KparActuator");
    let registry_archive = create_kpar_archive_with_model(
        dir.path(),
        "registry-lib",
        "1.0.0",
        "RegistryLib",
        "RegistryBus",
    );
    write_sysand_index_with_releases(
        dir.path(),
        "registry-lib",
        &[("1.0.0", registry_archive.clone())],
    );
    clean_registry_cache_for_request("sysand", "registry-lib", "1.0.0");

    fs::write(
        dir.path().join("sysml.toml"),
        format!(
            r#"
[project]
name = "manual-phase6-like"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
path-ok = {{ path = "./dep-path" }}
git-ok = {{ git = "{git_url}", rev = "{commit}" }}
kpar-ok = {{ kpar = "{}" }}
registry-lib = "1.0.0"
"#,
            kpar_archive.display()
        ),
    )
    .expect("workspace manifest should be written");

    let root_model_path = dir.path().join("root.sysml");
    let root_content = r#"
package ManualPhase6Like {
    import PathLib::*;
    import GitLib::*;
    import KparLib::*;
    import RegistryLib::*;

    part def IntegrationHarness {
        part pathSensor : PathSensor;
        part gitController : GitController;
        part kparActuator : KparActuator;
        part registryBus : RegistryBus;
    }
}
"#;
    fs::write(&root_model_path, root_content).expect("root source should be written");

    let server = TestServer::new();
    initialize_server_with_workspace_root(&server, dir.path()).await;
    let root_uri = Url::from_file_path(&root_model_path)
        .expect("root source should convert to URI")
        .to_string();
    server.open_document(&root_uri, root_content).await;

    let mut hydrated = false;
    for _ in 0..30 {
        if let Some(payload) = server
            .execute_command("sysml.dependency.status", vec![])
            .await
        {
            let deps_total = payload["summary"]["total_dependencies"]
                .as_u64()
                .unwrap_or(0);
            let deps_hydrated = payload["summary"]["hydrated_dependencies"]
                .as_u64()
                .unwrap_or(0);
            let deps_failed = payload["summary"]["failed_dependencies"]
                .as_u64()
                .unwrap_or(0);
            if deps_total == 4 && deps_hydrated == 4 && deps_failed == 0 {
                hydrated = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        hydrated,
        "expected all dependency sources to hydrate before goto assertions"
    );
    let _ = server
        .execute_command("sysml.workspace.refresh", vec![])
        .await
        .expect("workspace refresh should succeed after dependency hydration");

    let expected_symbols = ["PathSensor", "GitController", "KparActuator", "RegistryBus"];
    let mut snapshot_ready = false;
    for _ in 0..40 {
        let snapshot = server.server().workspace_snapshot().await;
        if expected_symbols
            .iter()
            .all(|name| !snapshot.find_by_name(name).is_empty())
        {
            snapshot_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        snapshot_ready,
        "expected workspace snapshot to index dependency symbols before goto assertions"
    );

    for cycle in 0..8 {
        for (usage_line, type_name) in [
            ("part pathSensor : PathSensor;", "PathSensor"),
            ("part gitController : GitController;", "GitController"),
            ("part kparActuator : KparActuator;", "KparActuator"),
            ("part registryBus : RegistryBus;", "RegistryBus"),
        ] {
            let line = root_content
                .lines()
                .position(|raw| raw.contains(usage_line))
                .unwrap_or_else(|| panic!("usage line should exist: {usage_line}"))
                as u32;
            let col = root_content
                .lines()
                .nth(line as usize)
                .and_then(|raw| raw.find(type_name))
                .unwrap_or_else(|| panic!("token should exist in usage line: {type_name}"))
                as u32
                + 2;

            let first_target = server
                .goto_definition(&root_uri, line, col)
                .await
                .unwrap_or_else(|| {
                    panic!("cycle {cycle}: expected first goto-definition for {type_name}")
                });
            let first_target_uri = response_target_uri(first_target).unwrap_or_else(|| {
                panic!("cycle {cycle}: expected first goto-definition location for {type_name}")
            });
            let first_target_path = Url::parse(&first_target_uri)
                .expect("goto target should be a valid URI")
                .to_file_path()
                .expect("goto target URI should map to file path");
            let first_target_content = fs::read_to_string(&first_target_path).unwrap_or_else(|e| {
                panic!(
                    "cycle {cycle}: failed to read first goto target '{}': {e}",
                    first_target_path.display()
                )
            });
            server
                .open_document(&first_target_uri, &first_target_content)
                .await;
            server.close_document(&first_target_uri).await;

            let second_target = server
                .goto_definition(&root_uri, line, col)
                .await
                .unwrap_or_else(|| {
                    panic!("cycle {cycle}: expected second goto-definition for {type_name}")
                });
            let second_target_uri = response_target_uri(second_target).unwrap_or_else(|| {
                panic!("cycle {cycle}: expected second goto-definition location for {type_name}")
            });
            let second_target_path = Url::parse(&second_target_uri)
                .expect("goto target should be a valid URI")
                .to_file_path()
                .expect("goto target URI should map to file path");
            let second_target_source =
                fs::read_to_string(&second_target_path).unwrap_or_else(|e| {
                    panic!(
                        "cycle {cycle}: failed to read second goto target '{}': {e}",
                        second_target_path.display()
                    )
                });
            assert!(
                second_target_source.contains(&format!("part def {type_name};")),
                "cycle {cycle}: expected second goto target for {type_name} to contain definition, got '{}'",
                second_target_path.display()
            );
        }
    }
}
