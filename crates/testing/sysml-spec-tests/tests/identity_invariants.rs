#![allow(clippy::unwrap_used, clippy::expect_used)]
//! L4 — Identity invariants, parameterised by transport.
//!
//! ONE obligation, pinned at two scopes (testing-architecture-redesign §3C):
//! **deterministic element IDs are reparse-stable and transport-identical.**
//! Post-S1 canonical IDs, the same source bytes must mint the same id set no
//! matter how many times they are parsed (`reparse_identity`) and no matter
//! which transport loads them (`cross_transport_identity`, LSP vs REST).
//!
//! ## Capture surface — PARSE-LEVEL, not elaboration
//!
//! Both gates compare the **parse-level** id set: the ids tree-sitter mints
//! when building the `ModelGraph` for the source, *before* name resolution or
//! cross-file elaboration. Both sides read `require_graph(<file uri>)`, which
//! is `GraphScope::File` = parse-only (`parse_file`, no resolve/elaborate).
//! This is deliberate — id *minting* happens at parse time, so parse-level is
//! the correct and sufficient surface for an identity invariant. It is **not**
//! an elaboration-identity gate: it says nothing about resolved references,
//! synthesized members, or library types. That is precisely why the LSP side
//! may (and does) skip workspace/library elaboration in `did_open`
//! (`TestServerOptions::skip_disk_project_load`) without weakening the check —
//! skipping elaboration cannot change the parse-level id set being captured.
//!
//! This file absorbs the former `reparse_identity_baseline.rs` (S0.T2) and
//! the identity half of `cross_transport_identity_baseline.rs` (S0.T5) —
//! same 4 fixtures, same partition-shape archives, same insta discipline.
//! The S2.T19 command-parity gate stays in
//! `cross_transport_identity_baseline.rs` (owned by the transport lane; its
//! fixtures under `fixtures/cross-transport-command-parity/` are not touched
//! here). `diff_identity_baseline.rs` pins a *different* obligation (the
//! `sysml_core::diff` correlation contract, ADR-009) and stays its own gate.
//!
//! ## Partition shape
//!
//! For each fixture the two sides' element-id sets are diffed into
//! `{side_a, side_b, only_in_a, only_in_b, shared}`. The invariant is
//! `only_in_* == 0 && shared == total` — any missed mint site shows up as a
//! non-empty `only_in_*`.
//!
//! - **JSON archives** (raw ids, human-inspection diagnostic only — NOT a
//!   gated baseline and NOT committed): written under the gitignored
//!   `target/identity-archives/{reparse,cross-transport}-identity-baseline/`.
//!   The raw ids are deterministic per code-state but legitimately shift
//!   whenever elaboration adds/removes elements (new canonical keys), so a
//!   *committed* raw-id archive dirties the tree on every such change while
//!   carrying no gate value (the gate is the redacted insta snapshot below).
//!   Keeping them out of tree removes that churn; the snapshot still pins the
//!   partition shape and the LSP-vs-REST / reparse identity invariant.
//! - **Insta snapshots** (regression gate, UUIDs redacted to `<UUID>`):
//!   `identity_invariants__reparse_<fixture>.snap` /
//!   `identity_invariants__cross_transport_<fixture>.snap`.
//!
//! Reporting order is stabilised by sorting elements on
//! `(name, kind, span_key)` — never the raw id — so snapshots survive
//! HashMap iteration randomness.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::runtime::Runtime;
use tower::ServiceExt;

use sysml_api::{create_router, AppState};
use sysml_core::ModelGraph;
use sysml_lsp_server::test_harness::{TestServer, TestServerOptions};
use sysml_service::SysmlService;

// ---------------------------------------------------------------------------
// Path helpers (verbatim lineage: S0.T1 service_command_baseline.rs)
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

/// Path to `the-book/` (the directory ABOVE the workspace root in this
/// monorepo layout).
fn the_book_root() -> PathBuf {
    workspace_root().parent().unwrap().join("the-book")
}

/// Where the raw-id JSON archive is written. This is a human-inspection
/// diagnostic, NOT a committed baseline: the ids are deterministic per
/// code-state but shift whenever elaboration changes the element set, so a
/// tracked copy would churn the tree without gating anything (the redacted
/// insta snapshot is the gate). Write under the gitignored workspace `target/`
/// so the diagnostic stays available locally without dirtying the tree.
fn archive_dir(name: &str) -> PathBuf {
    workspace_root()
        .join("target")
        .join("identity-archives")
        .join(name)
}

// ---------------------------------------------------------------------------
// Fixture catalog (same 4 files as S0.T1/T2/T5)
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

// ---------------------------------------------------------------------------
// Captured-element helpers (one home; formerly duplicated verbatim across
// the two baseline files)
// ---------------------------------------------------------------------------

/// One element captured for the partition. Carries the raw id (the thing
/// under test) plus the `(name, kind, span_key)` triple used as the stable
/// sort key — never sort by id (pre-S1 ids rotated per parse; the sort must
/// not depend on the value being tested either way).
struct CapturedElement {
    id: String,
    name: String,
    kind: String,
    span_key: String,
}

/// Collect every element from the graph, sorted by `(name, kind, span_key)`.
/// ALL elements are kept (named or anonymous): an anonymous helper element
/// re-minting its id breaks downstream caches just as badly as a named one.
fn capture_graph(graph: &ModelGraph) -> Vec<CapturedElement> {
    let mut out: Vec<CapturedElement> = graph
        .elements
        .values()
        .map(|e| {
            let span_key = e
                .spans
                .first()
                .map(|s| format!("{}:{}:{}", s.file, s.start, s.end))
                .unwrap_or_default();
            CapturedElement {
                id: e.id.to_string(),
                name: e.name.clone().unwrap_or_default(),
                kind: format!("{:?}", e.kind),
                span_key,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.kind.cmp(&b.kind))
            .then(a.span_key.cmp(&b.span_key))
    });
    out
}

/// Archive JSON key names for one transport pairing. Kept parameterised so
/// each absorbed baseline keeps its ORIGINAL key names (`parse_a`/`parse_b`,
/// `transport_lsp`/`transport_rest`) in the diagnostic archive and its
/// redacted insta snapshot.
struct PartitionKeys {
    side_a: &'static str,
    side_b: &'static str,
    only_in_a: &'static str,
    only_in_b: &'static str,
}

/// Diff two captured sides into the partition archive value. Arrays follow
/// the `(name, kind, span_key)` stable order of their side; `shared` follows
/// side A's order.
fn build_partition_value(
    fixture_label: &str,
    side_a: &[CapturedElement],
    side_b: &[CapturedElement],
    keys: &PartitionKeys,
) -> Value {
    let ids_a: BTreeSet<&str> = side_a.iter().map(|e| e.id.as_str()).collect();
    let ids_b: BTreeSet<&str> = side_b.iter().map(|e| e.id.as_str()).collect();

    let a_ids: Vec<String> = side_a.iter().map(|e| e.id.clone()).collect();
    let b_ids: Vec<String> = side_b.iter().map(|e| e.id.clone()).collect();

    let only_in_a: Vec<String> = side_a
        .iter()
        .filter(|e| !ids_b.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect();
    let only_in_b: Vec<String> = side_b
        .iter()
        .filter(|e| !ids_a.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect();
    let shared: Vec<String> = side_a
        .iter()
        .filter(|e| ids_b.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect();

    json!({
        "fixture": fixture_label,
        keys.side_a: a_ids,
        keys.side_b: b_ids,
        keys.only_in_a: only_in_a,
        keys.only_in_b: only_in_b,
        "shared": shared,
    })
}

/// Insta settings shared by both transports: redact UUIDs (ids are the
/// data under test; the snapshot pins the partition SHAPE) and absolute
/// fixture paths (cross-machine portability).
fn identity_snapshot_settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    let uuid_re =
        r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b";
    settings.add_filter(uuid_re, "<UUID>");
    let abs_path_re = r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*\.(?:sysml|kerml)""#;
    settings.add_filter(abs_path_re, "\"<PATH>\"");
    settings
}

/// Shared invariant assertions + diagnostics + archive write + snapshot.
#[allow(clippy::too_many_arguments)]
fn assert_partition_invariant(
    gate: &str,
    fixture_label: &str,
    side_a: &[CapturedElement],
    side_b: &[CapturedElement],
    keys: &PartitionKeys,
    archive_root: &Path,
    snapshot_name: &str,
) {
    assert_eq!(
        side_a.len(),
        side_b.len(),
        "fixture {}: side {} produced {} elements but side {} produced {} — a \
         cardinality mismatch is a bigger bug than identity (different load \
         paths doing different elaboration)",
        fixture_label,
        keys.side_a,
        side_a.len(),
        keys.side_b,
        side_b.len(),
    );

    let bundle = build_partition_value(fixture_label, side_a, side_b, keys);

    let only_a = bundle[keys.only_in_a].as_array().unwrap().len();
    let only_b = bundle[keys.only_in_b].as_array().unwrap().len();
    let shared = bundle["shared"].as_array().unwrap().len();
    eprintln!(
        "[{gate}] fixture={fixture_label:<20} elements={}  {}={only_a}  {}={only_b}  shared={shared}",
        side_a.len(),
        keys.only_in_a,
        keys.only_in_b,
    );

    // The S1.T11c invariant: identical id sets — anything in only_in_*
    // means a mint site was missed.
    assert_eq!(
        only_a, 0,
        "fixture {fixture_label}: {} must be 0 post-S1.T11c (got {only_a})",
        keys.only_in_a
    );
    assert_eq!(
        only_b, 0,
        "fixture {fixture_label}: {} must be 0 post-S1.T11c (got {only_b})",
        keys.only_in_b
    );
    assert_eq!(
        shared,
        side_a.len(),
        "fixture {fixture_label}: shared must equal the full element set post-S1.T11c",
    );

    let archive_path = archive_root.join(format!("{fixture_label}.json"));
    let pretty = serde_json::to_string_pretty(&bundle).expect("serialise bundle");
    std::fs::write(&archive_path, pretty).expect("write archive json");

    insta::assert_json_snapshot!(snapshot_name, bundle);
}

// ---------------------------------------------------------------------------
// Transport 1: single file parsed twice (formerly reparse_identity_baseline)
// ---------------------------------------------------------------------------

#[test]
fn reparse_identity() {
    let archive_root = archive_dir("reparse-identity-baseline");
    std::fs::create_dir_all(&archive_root).expect("create fixtures dir");

    let settings = identity_snapshot_settings();
    let _guard = settings.bind_to_scope();

    let keys = PartitionKeys {
        side_a: "parse_a",
        side_b: "parse_b",
        only_in_a: "only_in_a",
        only_in_b: "only_in_b",
    };

    for fixture in FIXTURES {
        let path = (fixture.resolve)();
        assert!(
            path.exists(),
            "fixture file missing: {} (label={})",
            path.display(),
            fixture.label
        );

        let mut captures = Vec::with_capacity(2);
        for side in ["parse A", "parse B"] {
            // Two fully independent services — mirrors two production
            // processes parsing the same file.
            let service = SysmlService::empty();
            let uri = service.load_file(&path).unwrap_or_else(|e| {
                panic!(
                    "load_file failed ({side}) for {} ({}): {e}",
                    path.display(),
                    fixture.label
                )
            });
            let graph = service
                .require_graph(&uri)
                .unwrap_or_else(|e| panic!("graph not loaded ({side}) for {}: {e}", fixture.label));
            captures.push(capture_graph(&graph));
        }
        let captured_b = captures.pop().expect("two captures");
        let captured_a = captures.pop().expect("two captures");

        assert_partition_invariant(
            "reparse_identity",
            fixture.label,
            &captured_a,
            &captured_b,
            &keys,
            &archive_root,
            &format!("reparse_{}", fixture.label),
        );
    }
}

// ---------------------------------------------------------------------------
// Transport 2: LSP vs REST (formerly cross_transport_identity_baseline).
//
// LSP side: drive the real LSP handler chain through the ONE supported
// in-process harness (`sysml_lsp_server::test_harness::TestServer`) —
// initialize + initialized + did_open — then capture ids from the LSP's OWN
// `SysmlService`/`AnalysisHost` (which `SysmlLanguageServer` shares) via
// `TestServer::require_graph`. This is the real LSP state, not the old shadow
// `SysmlService::empty().load_source(...)` extraction (which existed only
// because the LSP's service used to be unpopulated by did_open — no longer
// true). Background tasks are skipped so the server is quiescent: the stdlib
// library-load task is what made this test race/hang (task #225). A per-stage
// watchdog turns any residual hang into a named failure.
//
// REST side: POST /sources against a live, INDEPENDENT axum router, read ids
// from `state.service` for real — so LSP-vs-REST identity stays meaningful.
//
// Task #225 is FIXED (not merely un-ignored): the hang was the background
// stdlib library-load task spawned on `initialized`, which the external
// harness previously could not suppress (`skip_background_tasks` was
// `#[cfg(test)]`-only, invisible to a dependency build). The harness now
// exposes that knob under the `test-harness` feature and this test sets it.
// ---------------------------------------------------------------------------

async fn drive_rest_post_sources(uri: &str, source: &str) -> Arc<AppState> {
    let state = Arc::new(AppState::new());
    let app = create_router(state.clone());

    let body_json = serde_json::to_string(&json!({
        "uri": uri,
        "source": source,
    }))
    .expect("serialise POST /sources body");

    let req = Request::builder()
        .method("POST")
        .uri("/sources")
        .header("content-type", "application/json")
        .body(Body::from(body_json))
        .expect("build POST /sources request");

    let resp = app.oneshot(req).await.expect("router oneshot");
    let status = resp.status();
    // Drain so we don't leave the body half-read.
    let _ = resp.into_body().collect().await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "POST /sources returned {} (expected 201 CREATED) for uri={uri}",
        status,
    );

    state
}

#[test]
fn cross_transport_identity() {
    let archive_root = archive_dir("cross-transport-identity-baseline");
    std::fs::create_dir_all(&archive_root).expect("create fixtures dir");

    let settings = identity_snapshot_settings();
    let _guard = settings.bind_to_scope();

    let keys = PartitionKeys {
        side_a: "transport_lsp",
        side_b: "transport_rest",
        only_in_a: "only_in_lsp",
        only_in_b: "only_in_rest",
    };

    // Single tokio runtime drives both async harnesses; the test is sync.
    let rt = Runtime::new().expect("build tokio runtime");

    // Per-stage watchdog. Each LSP lifecycle stage (initialize / initialized /
    // did_open / graph-capture / shutdown) must finish well within this or the
    // harness panics with the offending stage's name. 30s is ~40x a healthy
    // run; it converts a hang into a *named* failure — it is NOT the fix for
    // task #225 (that fix is skipping the background library-load task, below).
    let stage_timeout = Duration::from_secs(30);

    for fixture in FIXTURES {
        let path = (fixture.resolve)();
        assert!(
            path.exists(),
            "fixture file missing: {} (label={})",
            path.display(),
            fixture.label
        );

        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {} ({}): {e}", path.display(), fixture.label));
        let uri = format!("file://{}", path.display());

        // ---- LSP transport: capture from the REAL LSP-owned host ----
        let captured_lsp = rt.block_on(async {
            let ts = TestServer::with_options(TestServerOptions {
                skip_background_tasks: true,
                // Parse-level identity only needs the opened file's parse
                // graph; skip did_open's ~80s disk-project + stdlib load
                // (task #225). require_graph(File) is parse-only, so this
                // doesn't change the captured id set — only the wall time.
                skip_disk_project_load: true,
                client_capabilities: None,
                stage_timeout: Some(stage_timeout),
            });
            ts.initialize_full().await;
            ts.open_document(&uri, &source).await;
            let lsp_graph = ts.require_graph(&uri).await;
            let captured = capture_graph(&lsp_graph);
            // Cancel diagnostic tasks + auto-responder so nothing outlives
            // this fixture's server.
            ts.shutdown().await;
            captured
        });

        // ---- REST transport: independent service instance ----
        let captured_rest = rt.block_on(async {
            let rest_state = drive_rest_post_sources(&uri, &source).await;
            let rest_graph = rest_state.service.require_graph(&uri).unwrap_or_else(|_| {
                panic!("graph not loaded (REST-side service) for {}", fixture.label)
            });
            capture_graph(&rest_graph)
        });

        assert_partition_invariant(
            "cross_transport_identity",
            fixture.label,
            &captured_lsp,
            &captured_rest,
            &keys,
            &archive_root,
            &format!("cross_transport_{}", fixture.label),
        );
    }
}
