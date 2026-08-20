#![allow(clippy::unwrap_used, clippy::expect_used)]
//! S2.T19 — per-bucket cross-transport command-response parity.
//!
//! For the service commands shipped during S2 (`sysml.completion.resolve`
//! from T9, `sysml.workspace.files` from T16), every transport must produce
//! byte-identical responses for the same fixture: the typed `SysmlService`
//! call (CLI path), `execute_command` JSON dispatch (MCP path), the REST
//! inventory auto-route (`POST /api/commands/<name>`), and — for
//! `sysml.workspace.files` — the friendly `GET /workspace/files` shim.
//! See the banner comment above `cross_transport_command_parity_t19` for
//! the full transport map and why LSP is intentionally absent today.
//!
//! Archives: raw-id/raw-output JSON written under the gitignored
//! `target/identity-archives/cross-transport-command-parity/` (human-inspection
//! diagnostic only — NOT a committed baseline; the gate is the redacted insta
//! snapshot). The outputs are deterministic per code-state but shift whenever
//! command output changes, so a committed copy would churn the tree without
//! gating anything. The `..__parity_<fixture>.snap` insta snapshots (UUIDs/paths
//! redacted) are the actual gate; this file keeps its historical name so those
//! snapshot files stay valid.
//!
//! The cross-transport *identity* baseline that used to live here (S0.T5 —
//! same source loaded via LSP vs REST must mint identical element-id sets)
//! moved to `identity_invariants.rs`, merged with the reparse-identity gate
//! into one transport-parameterised L4 invariant
//! (testing-architecture-redesign §3C).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::runtime::Runtime;
use tower::ServiceExt;
use tower_lsp::LanguageServer;

use sysml_api::{create_router, AppState};
use sysml_core::ModelGraph;
use sysml_id::ElementId;

// ---------------------------------------------------------------------------
// Path helpers (verbatim from S0.T1 / S0.T2 / S0.T4)
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn the_book_root() -> PathBuf {
    workspace_root().parent().unwrap().join("the-book")
}

/// Absolute-checkout-path → stable-token substitutions applied to every
/// response bundle before it is snapshotted or archived, so the committed
/// baselines carry no developer-specific absolute path (fresh-clone /
/// relocated-checkout portability). Most specific prefix first — the shared
/// repo parent must come last or it would partially rewrite the longer
/// workspace/the-book paths. See `sysml_spec_tests::path_canon`.
fn path_replacements() -> Vec<sysml_spec_tests::path_canon::PathReplacement> {
    use sysml_spec_tests::path_canon::PathReplacement;
    vec![
        PathReplacement::new(workspace_root().to_string_lossy().into_owned(), "<WS>"),
        PathReplacement::new(the_book_root().to_string_lossy().into_owned(), "<BOOK>"),
        PathReplacement::new(
            workspace_root()
                .parent()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "<REPO>",
        ),
    ]
}


// ---------------------------------------------------------------------------
// Fixture catalog (mirrors S0.T1 / S0.T2 — same 4 files)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Fixture {
    label: &'static str,
    resolve: fn() -> PathBuf,
}

fn coffee_definitions_path() -> PathBuf {
    the_book_root()
        .join("examples")
        .join("coffee-machine")
        .join("definitions.sysml")
}

fn coffee_views_path() -> PathBuf {
    the_book_root()
        .join("examples")
        .join("coffee-machine")
        .join("views.sysml")
}

fn espresso_pump_path() -> PathBuf {
    workspace_root()
        .join("examples")
        .join("espresso-pump-hybrid")
        .join("Physics")
        .join("PumpODE.sysml")
}

fn stdlib_base_path() -> PathBuf {
    workspace_root()
        .join("libraries")
        .join("standard")
        .join("library.kernel")
        .join("Base.kerml")
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        label: "coffee_definitions",
        resolve: coffee_definitions_path,
    },
    Fixture {
        label: "coffee_views",
        resolve: coffee_views_path,
    },
    Fixture {
        label: "espresso_pump",
        resolve: espresso_pump_path,
    },
    Fixture {
        label: "stdlib_base",
        resolve: stdlib_base_path,
    },
];


// ===========================================================================
// S2.T19 — per-bucket cross-transport command-response parity
// ===========================================================================
//
// The sibling `cross_transport_identity_baseline` test above proves that
// loading the same source via LSP and REST produces identical element-id
// sets (the post-S1 invariant S2 leans on). T19 layers a per-command
// gate on top: for the new commands shipped during S2 (`sysml.completion.
// resolve` from T9, `sysml.workspace.files` from T16), every transport
// must produce byte-identical responses for the same fixture.
//
// Transports exercised here:
//
// - **CLI path** — typed method on `SysmlService` (e.g.
//   `service.completion_resolve(uri, &element_id)`). The CLI binary
//   composes typed calls directly; no JSON dispatch.
// - **MCP path** — `sysml_service::execute_command(&service, name,
//   body)`. The MCP server's `dispatch_to_service` helper is a thin
//   wrapper around exactly this call (see `sysml-mcp/src/lib.rs:225`).
// - **REST inventory auto-route** — HTTP `POST /api/commands/<name>`.
//   The route implementation (`sysml-api/src/lib.rs:1054`) calls
//   `execute_command` with the request body as JSON. Verifies that
//   axum's JSON serialization round-trip preserves byte-identity.
// - **REST friendly route** — for `sysml.workspace.files` only:
//   `GET /workspace/files?root=...`. Hand-written shim that calls
//   `service.workspace_files(...)` directly with `max_depth: None`
//   (default 5) — the friendly route doesn't expose `max_depth`, so
//   we compare its response against the typed call with `None`.
//
// LSP is intentionally absent: today the LSP doesn't dispatch
// service commands (its only exec path is `executeCommand`, the
// migration target of S2 buckets D-G). Once that lands, T19 will
// extend with an LSP harness that calls the real executeCommand
// chain.
//
// The captured per-fixture archive contains the response JSON (the
// thing all transports must agree on). The insta snapshot scrubs
// UUIDs/paths so it is byte-stable across machines.

/// Tower oneshot helper: send a JSON POST and parse the response body
/// as JSON. Asserts 200 OK.
async fn rest_post_json(app: &axum::Router, path: &str, body: &Value) -> Value {
    let body_str = serde_json::to_string(body).expect("serialise body");
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body_str))
        .expect("build POST");
    let resp = app.clone().oneshot(req).await.expect("router oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "POST {path} returned {status}; body={}",
        String::from_utf8_lossy(&bytes),
    );
    serde_json::from_slice(&bytes).expect("parse json body")
}

/// Tower oneshot helper: send a GET with a query string and parse the
/// response body as JSON. Asserts 200 OK.
async fn rest_get_json(app: &axum::Router, path_with_query: &str) -> Value {
    let req = Request::builder()
        .method("GET")
        .uri(path_with_query)
        .body(Body::empty())
        .expect("build GET");
    let resp = app.clone().oneshot(req).await.expect("router oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "GET {path_with_query} returned {status}; body={}",
        String::from_utf8_lossy(&bytes),
    );
    serde_json::from_slice(&bytes).expect("parse json body")
}

/// Compute (line, col) at the start span of the first named element in the
/// fixture file. Reads the file from disk to convert byte offset → UTF-16
/// line/character (LSP convention) — matches the `first_named_element_position`
/// helper in `service_command_baseline.rs`.
fn first_named_element_position_for_parity(uri: &str, graph: &ModelGraph) -> Option<(u32, u32)> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let content = std::fs::read_to_string(path).ok()?;
    let mut hits: Vec<(String, String, usize)> = graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .filter_map(|e| {
            let span = e.spans.iter().find(|s| s.file == uri || s.file == path)?;
            Some((
                e.name.clone().unwrap_or_default(),
                format!("{:?}", e.kind),
                span.start,
            ))
        })
        .collect();
    hits.sort();
    let (_, _, start) = hits.into_iter().next()?;
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (idx, ch) in content.char_indices() {
        if idx >= start {
            let prefix = &content[line_start..idx.min(content.len())];
            let character: u32 = prefix.chars().map(|c| c.len_utf16() as u32).sum();
            return Some((line, character));
        }
        if ch == '\n' {
            line += 1;
            line_start = idx + 1;
        }
    }
    Some((line, 0))
}

/// Pick the first named element id in the graph using the same
/// deterministic selector as `service_command_baseline.rs` — sort on
/// (name, kind, span_key, id) so ties resolve identically across runs.
fn first_named_id_for_parity(graph: &ModelGraph) -> Option<String> {
    let mut hits: Vec<(String, String, String, String)> = graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .map(|e| {
            let span_key = e
                .spans
                .first()
                .map(|s| format!("{}:{}:{}", s.file, s.start, s.end))
                .unwrap_or_default();
            (
                e.name.clone().unwrap_or_default(),
                format!("{:?}", e.kind),
                span_key,
                e.id.to_string(),
            )
        })
        .collect();
    hits.sort();
    hits.into_iter().next().map(|(_, _, _, id)| id)
}

#[test]
fn cross_transport_command_parity_t19() {
    // Raw-output archives are a human-inspection diagnostic, NOT a committed
    // baseline (the redacted insta snapshot is the gate). Write under the
    // gitignored workspace `target/` so per-code-state output shifts don't churn
    // the tree — same rationale as identity_invariants.rs::archive_dir.
    let fixtures_root = workspace_root()
        .join("target")
        .join("identity-archives")
        .join("cross-transport-command-parity");
    std::fs::create_dir_all(&fixtures_root).expect("create archive dir");

    // Insta filters — same redaction strategy as the sibling identity
    // test, plus a wider absolute-path filter for non-`.sysml/.kerml`
    // paths surfaced by `sysml.workspace.files` (root + directory
    // entries). Mirrors the wider regex that landed in T18 on
    // `service_command_baseline.rs`.
    let mut settings = insta::Settings::clone_current();
    let uuid_re =
        r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b";
    settings.add_filter(uuid_re, "<UUID>");
    let abs_path_re = r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*\.(?:sysml|kerml)""#;
    settings.add_filter(abs_path_re, "\"<PATH>\"");
    let abs_path_any_re = r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*""#;
    settings.add_filter(abs_path_any_re, "\"<PATH>\"");
    // Salsa query telemetry counters (S2.T11) — values shift with catalog
    // order and parallelism. Snapshot the wire shape only.
    let salsa_int_re = r#""(executions|validations)":\s*[0-9]+"#;
    settings.add_filter(salsa_int_re, "\"$1\":<N>");
    let salsa_float_re = r#""hit_ratio":\s*[0-9.]+"#;
    settings.add_filter(salsa_float_re, "\"hit_ratio\":<F>");
    // Library cache snapshot fields (S2.T11) — depend on the runner's
    // filesystem state (whether `~/.cache/sysml-rs/library-v*.bin`
    // exists). Snapshot the wire shape only.
    let cache_bool_re = r#""exists":\s*(true|false)"#;
    settings.add_filter(cache_bool_re, "\"exists\":<B>");
    let cache_int_re = r#""(size_bytes|element_count)":\s*[0-9]+"#;
    settings.add_filter(cache_int_re, "\"$1\":<N>");
    let cache_ver_re = r#""crate_version":\s*("[^"]*"|null)"#;
    settings.add_filter(cache_ver_re, "\"crate_version\":<V>");
    let _guard = settings.bind_to_scope();

    let rt = Runtime::new().expect("build tokio runtime");

    for fixture in FIXTURES {
        let path = (fixture.resolve)();
        assert!(
            path.exists(),
            "fixture file missing: {} (label={})",
            path.display(),
            fixture.label
        );
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let uri = format!("file://{}", path.display());

        // ---- Single shared SysmlService via REST AppState ----
        // The REST `POST /sources` handler calls
        // `state.service.load_source(...)` which lands the parsed graph
        // in `state.service.graphs[uri]`. The same `Arc<SysmlService>`
        // backs the typed-method calls and the `execute_command`
        // calls below — that's the whole point of S2: one service,
        // many transports.
        let state = Arc::new(AppState::new());
        let app = create_router(state.clone());
        let load_body = json!({ "uri": &uri, "source": &source });
        let load_req = Request::builder()
            .method("POST")
            .uri("/sources")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&load_body).unwrap()))
            .expect("build POST /sources");
        let load_resp = rt
            .block_on(app.clone().oneshot(load_req))
            .expect("router oneshot");
        assert_eq!(
            load_resp.status(),
            StatusCode::CREATED,
            "shared-state load via POST /sources failed",
        );
        let _ = rt.block_on(load_resp.into_body().collect());

        let graph = state
            .service
            .require_graph(&uri)
            .unwrap_or_else(|e| panic!("graph not loaded (shared service) for {}: {e}", fixture.label));
        let element_id_str = first_named_id_for_parity(&graph)
            .unwrap_or_else(|| panic!("no named element in {}", fixture.label));
        let element_id: ElementId = element_id_str
            .parse()
            .unwrap_or_else(|e| panic!("parse element id {element_id_str:?}: {e}"));
        let parent_dir = path
            .parent()
            .unwrap_or_else(|| panic!("no parent dir for {}", path.display()))
            .to_string_lossy()
            .into_owned();

        // ---------- sysml.completion.resolve ----------
        // CLI path — typed method.
        let cli_resolve = state
            .service
            .completion_resolve(Some(&uri), &element_id)
            .expect("completion_resolve typed");
        let cli_resolve_json = serde_json::to_value(&cli_resolve).expect("serialise typed resp");
        // MCP path — execute_command.
        let mcp_resolve = sysml_service::execute_command(
            &state.service,
            "sysml.completion.resolve",
            json!({ "uri": &uri, "element_id": &element_id_str }),
        )
        .expect("completion_resolve via execute_command");
        // REST auto-route — POST /api/commands/sysml.completion.resolve.
        let rest_resolve_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.completion.resolve",
            &json!({ "uri": &uri, "element_id": &element_id_str }),
        ));
        assert_eq!(
            cli_resolve_json, mcp_resolve,
            "completion.resolve parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_resolve, rest_resolve_auto,
            "completion.resolve parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.workspace.files (max_depth=2 — bounded archive) ----------
        // CLI typed.
        let cli_files = state
            .service
            .workspace_files(&parent_dir, Some(2))
            .expect("workspace_files typed depth=2");
        let cli_files_json = serde_json::to_value(&cli_files).expect("serialise typed resp");
        // MCP — execute_command.
        let mcp_files = sysml_service::execute_command(
            &state.service,
            "sysml.workspace.files",
            json!({ "root": &parent_dir, "max_depth": 2 }),
        )
        .expect("workspace_files via execute_command depth=2");
        // REST auto-route.
        let rest_files_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.workspace.files",
            &json!({ "root": &parent_dir, "max_depth": 2 }),
        ));
        assert_eq!(
            cli_files_json, mcp_files,
            "workspace.files (depth=2) parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_files, rest_files_auto,
            "workspace.files (depth=2) parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.workspace.files (default depth=5 — REST friendly route) ----------
        // The hand-written REST shim `GET /workspace/files?root=...`
        // doesn't expose `max_depth`, so we compare its response
        // against the typed call with `None` (default 5). Path is
        // appended raw (axum's serde_urlencoded handles `/`-bearing
        // values correctly within query params).
        let cli_files_default = state
            .service
            .workspace_files(&parent_dir, None)
            .expect("workspace_files typed default");
        let cli_files_default_json =
            serde_json::to_value(&cli_files_default).expect("serialise typed resp");
        let rest_files_friendly = rt.block_on(rest_get_json(
            &app,
            &format!("/workspace/files?root={}", parent_dir),
        ));
        assert_eq!(
            cli_files_default_json, rest_files_friendly,
            "workspace.files (default depth) parity (CLI typed vs REST friendly route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.rename (prepare + apply) ----------
        // Cursor at the first named element. Prepare-mode (new_name=null)
        // returns placeholder + range; apply-mode rewrites every reference
        // across the workspace. Both modes go through CLI typed / MCP
        // execute_command / REST auto-route and must be byte-identical.
        let (rn_line, rn_col) = first_named_element_position_for_parity(&uri, &graph)
            .unwrap_or_else(|| panic!("no element position in {}", fixture.label));
        // Prepare-mode.
        let cli_prepare = state
            .service
            .rename(&uri, rn_line, rn_col, None)
            .expect("rename typed prepare");
        let cli_prepare_json = serde_json::to_value(&cli_prepare).expect("serialise prepare");
        let mcp_prepare = sysml_service::execute_command(
            &state.service,
            "sysml.rename",
            json!({
                "uri": &uri,
                "line": rn_line,
                "col": rn_col,
                "new_name": serde_json::Value::Null,
            }),
        )
        .expect("rename via execute_command (prepare)");
        let rest_prepare_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.rename",
            &json!({
                "uri": &uri,
                "line": rn_line,
                "col": rn_col,
                "new_name": serde_json::Value::Null,
            }),
        ));
        assert_eq!(
            cli_prepare_json, mcp_prepare,
            "rename prepare parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_prepare, rest_prepare_auto,
            "rename prepare parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // Apply-mode — pick a fresh non-keyword name unlikely to collide.
        // The resulting workspace edit must agree across transports
        // byte-for-byte; the snapshot keeps only the apply-mode result
        // because it's the larger, more discriminating artifact.
        let new_name = "RenamedForParityT19";
        let cli_apply = state
            .service
            .rename(&uri, rn_line, rn_col, Some(new_name))
            .expect("rename typed apply");
        let cli_apply_json = serde_json::to_value(&cli_apply).expect("serialise apply");
        let mcp_apply = sysml_service::execute_command(
            &state.service,
            "sysml.rename",
            json!({
                "uri": &uri,
                "line": rn_line,
                "col": rn_col,
                "new_name": new_name,
            }),
        )
        .expect("rename via execute_command (apply)");
        let rest_apply_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.rename",
            &json!({
                "uri": &uri,
                "line": rn_line,
                "col": rn_col,
                "new_name": new_name,
            }),
        ));
        assert_eq!(
            cli_apply_json, mcp_apply,
            "rename apply parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_apply, rest_apply_auto,
            "rename apply parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.format.document ----------
        // Default formatting options (4-space indent, spaces). The whitespace
        // edit set must be byte-identical across CLI typed / MCP execute_command
        // / REST auto-route.
        let cli_format = state
            .service
            .format_document(&uri, Some(4), Some(true))
            .expect("format_document typed");
        let cli_format_json = serde_json::to_value(&cli_format).expect("serialise format");
        let mcp_format = sysml_service::execute_command(
            &state.service,
            "sysml.format.document",
            json!({
                "uri": &uri,
                "tab_size": 4,
                "insert_spaces": true,
            }),
        )
        .expect("format_document via execute_command");
        let rest_format_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.format.document",
            &json!({
                "uri": &uri,
                "tab_size": 4,
                "insert_spaces": true,
            }),
        ));
        assert_eq!(
            cli_format_json, mcp_format,
            "format.document parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_format, rest_format_auto,
            "format.document parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.code_action.list ----------
        // Synthetic E200 diagnostic at the first named element's range —
        // exercises auto-import / use-qualified-name / create-definition
        // paths plus the cursor refactorings (the same range fires
        // expand-body / keyword-toggles / toggle-abstract / add-doc-comment
        // when the cursor sits on a definition). Action set must be
        // byte-identical across CLI / MCP / REST.
        let synthetic_diag = json!({
            "line_start": rn_line,
            "col_start": rn_col,
            "line_end": rn_line,
            "col_end": rn_col,
            "code": "E200",
            "message": "Unresolved reference 'Real' for property 'general'",
        });
        let cli_actions = state
            .service
            .code_action_list(
                &uri,
                rn_line,
                rn_col,
                rn_line,
                rn_col,
                &json!([synthetic_diag.clone()]),
            )
            .expect("code_action_list typed");
        let cli_actions_json = serde_json::to_value(&cli_actions).expect("serialise actions");
        let mcp_actions = sysml_service::execute_command(
            &state.service,
            "sysml.code_action.list",
            json!({
                "uri": &uri,
                "range_start_line": rn_line,
                "range_start_col": rn_col,
                "range_end_line": rn_line,
                "range_end_col": rn_col,
                "diagnostics": [synthetic_diag.clone()],
            }),
        )
        .expect("code_action_list via execute_command");
        let rest_actions_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.code_action.list",
            &json!({
                "uri": &uri,
                "range_start_line": rn_line,
                "range_start_col": rn_col,
                "range_end_line": rn_line,
                "range_end_col": rn_col,
                "diagnostics": [synthetic_diag],
            }),
        ));
        assert_eq!(
            cli_actions_json, mcp_actions,
            "code_action.list parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_actions, rest_actions_auto,
            "code_action.list parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.diagram.edit (`create` action) ----------
        // Compute a workspace edit for adding a PartDefinition. The wire
        // shape is `{ request: <DiagramEditRequest> }`. Result includes
        // the line/col edit + status payload — must be byte-identical
        // across CLI / MCP / REST.
        let diagram_create_req = json!({
            "request": {
                "uri": &uri,
                "action": "create",
                "elementTypeId": "PartDefinition",
                "containerId": serde_json::Value::Null,
            }
        });
        let cli_diagram = state
            .service
            .diagram_edit(&diagram_create_req["request"])
            .expect("diagram_edit typed");
        let cli_diagram_json = serde_json::to_value(&cli_diagram).expect("serialise diagram");
        let mcp_diagram = sysml_service::execute_command(
            &state.service,
            "sysml.diagram.edit",
            diagram_create_req.clone(),
        )
        .expect("diagram_edit via execute_command");
        let rest_diagram_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.diagram.edit",
            &diagram_create_req,
        ));
        assert_eq!(
            cli_diagram_json, mcp_diagram,
            "diagram.edit (create) parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_diagram, rest_diagram_auto,
            "diagram.edit (create) parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.workspace.refresh (S2.T11 — Bucket F / LSP-04+07+70) ----------
        // Empty-roots + stdlib disabled — deterministic
        // `{projects:[], stdlib_loaded:false, roots_count:0}` shape
        // independent of filesystem. The full discovery path is
        // exercised by LSP integration tests. **Side effect**: this
        // resets the shared host. We call it BEFORE the next dispatches
        // that depend on `state.service.graph(&uri)` having the loaded
        // fixture — we don't, since the bundle below doesn't read from
        // the graph. Run order matters: this must come AFTER all
        // dispatches that need the loaded fixture and BEFORE the
        // salsa.stats reset (which would otherwise show pre-reset
        // counters).
        let cli_ws_refresh = state
            .service
            .workspace_refresh(&[], Some(false))
            .expect("workspace_refresh typed");
        let cli_ws_refresh_json =
            serde_json::to_value(&cli_ws_refresh).expect("serialise ws_refresh");
        let mcp_ws_refresh = sysml_service::execute_command(
            &state.service,
            "sysml.workspace.refresh",
            json!({ "roots": [], "enable_stdlib": false }),
        )
        .expect("workspace_refresh via execute_command");
        let rest_ws_refresh_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.workspace.refresh",
            &json!({ "roots": [], "enable_stdlib": false }),
        ));
        assert_eq!(
            cli_ws_refresh_json, mcp_ws_refresh,
            "workspace.refresh parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_ws_refresh, rest_ws_refresh_auto,
            "workspace.refresh parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.dependency.status (S2.T11 — Bucket F / LSP-63) ----------
        // Pass empty roots so the payload is the deterministic
        // `{roots: [], summary: {... all zeros ...}}` shape and the
        // dispatch is filesystem-independent. The richer hydrated
        // shape is exercised by inlay-hints integration tests on the
        // LSP side.
        let cli_dep_status = state
            .service
            .dependency_status(&[])
            .expect("dependency_status typed");
        let mcp_dep_status = sysml_service::execute_command(
            &state.service,
            "sysml.dependency.status",
            json!({ "roots": [] }),
        )
        .expect("dependency_status via execute_command");
        let rest_dep_status_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.dependency.status",
            &json!({ "roots": [] }),
        ));
        assert_eq!(
            cli_dep_status, mcp_dep_status,
            "dependency.status parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_dep_status, rest_dep_status_auto,
            "dependency.status parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.cache.status (S2.T11 — Bucket F / LSP-46) ----------
        // Filesystem-dependent payload — if `~/.cache/sysml-rs/library-v*.bin`
        // exists, all three transports observe the same on-disk state via
        // the shared `library_cache::find_library_config` + `LibraryCache::stats`.
        // The cache_size/element_count/crate_version/exists fields are
        // redacted by the snapshot filters below so the gate compares
        // wire shape only.
        let cli_cache_status = state.service.cache_status().expect("cache_status typed");
        let mcp_cache_status =
            sysml_service::execute_command(&state.service, "sysml.cache.status", json!({}))
                .expect("cache_status via execute_command");
        let rest_cache_status_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.cache.status",
            &json!({}),
        ));
        assert_eq!(
            cli_cache_status, mcp_cache_status,
            "cache.status parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_cache_status, rest_cache_status_auto,
            "cache.status parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---------- sysml.salsa.stats / .reset (S2.T11 — Bucket F) ----------
        // Stats counters drift run-to-run (catalog ordering, parallelism)
        // so the snapshot only gates the wire shape via redaction filters
        // applied below. Both commands must be byte-identical across CLI /
        // MCP / REST regardless of the numeric values.
        let cli_salsa_stats = state.service.salsa_stats().expect("salsa_stats typed");
        let cli_salsa_stats_json =
            serde_json::to_value(&cli_salsa_stats).expect("serialise salsa_stats");
        let mcp_salsa_stats =
            sysml_service::execute_command(&state.service, "sysml.salsa.stats", json!({}))
                .expect("salsa_stats via execute_command");
        let rest_salsa_stats_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.salsa.stats",
            &json!({}),
        ));
        assert_eq!(
            cli_salsa_stats_json, mcp_salsa_stats,
            "salsa.stats parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_salsa_stats, rest_salsa_stats_auto,
            "salsa.stats parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // Reset is fully deterministic — `{"status": "reset"}`.
        let cli_salsa_reset = state
            .service
            .salsa_stats_reset()
            .expect("salsa_stats_reset typed");
        let cli_salsa_reset_json =
            serde_json::to_value(&cli_salsa_reset).expect("serialise salsa_stats_reset");
        let mcp_salsa_reset =
            sysml_service::execute_command(&state.service, "sysml.salsa.stats.reset", json!({}))
                .expect("salsa_stats_reset via execute_command");
        let rest_salsa_reset_auto = rt.block_on(rest_post_json(
            &app,
            "/api/commands/sysml.salsa.stats.reset",
            &json!({}),
        ));
        assert_eq!(
            cli_salsa_reset_json, mcp_salsa_reset,
            "salsa.stats.reset parity (CLI typed vs MCP execute_command) failed for {}",
            fixture.label,
        );
        assert_eq!(
            mcp_salsa_reset, rest_salsa_reset_auto,
            "salsa.stats.reset parity (MCP execute_command vs REST auto-route) failed for {}",
            fixture.label,
        );

        // ---- Bundle for archive + snapshot ----
        // Capture only the depth=2 result for `workspace.files` so the
        // snapshot stays bounded; the friendly-route check above is
        // an in-test assertion only (not snapshotted).
        let mut bundle = json!({
            "fixture": fixture.label,
            "completion_resolve": cli_resolve_json,
            "workspace_files_depth_2": cli_files_json,
            "rename_prepare": cli_prepare_json,
            "rename_apply": cli_apply_json,
            "format_document": cli_format_json,
            "code_action_list": cli_actions_json,
            "diagram_edit_create": cli_diagram_json,
            "salsa_stats": cli_salsa_stats_json,
            "salsa_stats_reset": cli_salsa_reset_json,
            "cache_status": cli_cache_status,
            "dependency_status_empty": cli_dep_status,
            "workspace_refresh_empty": cli_ws_refresh_json,
        });

        // Project absolute checkout paths onto stable tokens ONCE, over the
        // whole bundle, before it lands in either the JSON archive or the
        // insta snapshot — the sole seam that keeps both baselines
        // checkout-independent.
        sysml_spec_tests::path_canon::canonicalize_paths(&mut bundle, &path_replacements());

        let archive_path = fixtures_root.join(format!("{}.json", fixture.label));
        let pretty = serde_json::to_string_pretty(&bundle).expect("serialise bundle");
        std::fs::write(&archive_path, pretty).expect("write archive json");

        let snap_name = format!("parity_{}", fixture.label);
        insta::assert_json_snapshot!(snap_name, bundle);

        eprintln!(
            "[cross_transport_command_parity_t19] fixture={:<20} completion_resolve.kind={}  workspace_files.entries={}",
            fixture.label,
            // Compact diagnostic: print top-level field count for the
            // resolve response and entry count for workspace.files.
            if cli_resolve_json.is_null() {
                "null".to_string()
            } else {
                "present".to_string()
            },
            cli_files_json
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "?".to_string()),
        );
    }
}
