#![allow(clippy::unwrap_used, clippy::expect_used)]
//! S0.T4 — Perf baselines (LSP keystroke / REST cold-warm / sim-start).
//!
//! ## Why this exists
//!
//! S2 will rewire `sysml-service` onto the `sysml-ide-db` salsa cache.
//! Today, the service path always re-parses; LSP and REST own their own
//! parser caches; runtime owns its own elaboration result. After S2/S3,
//! all three should hit a shared analysis cache. This baseline captures
//! today's wall-clock cost of three flows so post-S2 we can say
//! "did the cleanup regress LSP keystroke latency by 2× or 10×, or was
//! it neutral / better?" without needing to remember "what was it before".
//!
//! Numbers are coarse, not micro-benchmarked: 5–20 sample medians + p95.
//! Wall-clock noise on a dev machine is roughly 10%. Don't read more
//! precision out of the artifact than that.
//!
//! ## What is measured
//!
//! - **M1 — LSP `didChange → hover` round-trip (warm)** —
//!   Drives the `tower_lsp::LspService` in-process via the `LanguageServer`
//!   trait, the same way `crates/tooling/sysml-lsp-server/src/test_harness.rs`
//!   drives `protocol_tests`. 20 samples, first 5 dropped as warmup;
//!   median + p95 + min + max over the remaining 15.
//!
//! - **M2 — REST `POST /sources` cold vs warm** —
//!   Builds the live `sysml-api` axum router via `create_router(AppState)`
//!   and dispatches via `tower::ServiceExt::oneshot`. Real socket bytes
//!   are skipped because at 127.0.0.1:0 they're sub-millisecond noise that
//!   won't shift under S2's cache wiring; the router/handler/service
//!   chain (which is what S2 touches) is fully exercised. 1 cold call,
//!   then 10 warm calls; median + p95 of warm.
//!
//! - **M3 — `simulate.start` against `examples/espresso-production-cell/`** —
//!   Loads the workspace via `SysmlService::load_workspace` then
//!   dispatches `simulate.start` against `BrewStation::StationLifecycle`.
//!   Whatever heavy lifting the start path does (workspace_aware_graph
//!   construction, elaboration, state-machine compile, runner build)
//!   is part of the measurement — that's the point. M3a is the full
//!   load+start sequence; M3b is the load step alone.
//!
//! ## Run command
//!
//! ```bash
//! cargo test --release -p sysml-spec-tests --test perf_baseline -- \
//!   --ignored --nocapture --test-threads=1
//! ```
//!
//! `--release` is non-negotiable: debug builds OOM and skew numbers by
//! 50–500×. `--test-threads=1` is mandatory because cargo-test runs
//! tests in parallel by default and parallel timing measurements are
//! garbage. The test is `#[ignore]` so it stays out of the default
//! `cargo test` run.
//!
//! ## Deliverables
//!
//! - This file: measures + prints + writes the artifact.
//! - `Architectural-cleanup/perf-baseline.json` (top of cleanup folder,
//!   matching HANDOVER's wording): durable record of today's numbers
//!   pinned to the branch HEAD short SHA.
//! - `Architectural-cleanup/STATUS.md`: ticks S0.T4, fills "Perf snapshot".
//!
//! No latency thresholds are asserted. Today's numbers are the baseline;
//! the whole point of the file is to make post-S2 regression visible.
//! The only assertion is that all three measurements ran (each result
//! struct has a non-zero sample count where a sample count exists).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value as JsonValue};
use tokio::runtime::Runtime;
use tower::ServiceExt;

use sysml_api::{create_router, AppState};
use sysml_lsp_server::test_harness::{TestServer, TestServerOptions};
use sysml_service::SysmlService;

// ---------------------------------------------------------------------------
// Path helpers (verbatim shape from S0.T1/T2/T3)
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

fn architectural_cleanup_dir() -> PathBuf {
    workspace_root().join("Architectural-cleanup")
}

// ---------------------------------------------------------------------------
// Tiny stats helpers (we only need median / p95 / min / max).
// ---------------------------------------------------------------------------

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn percentile(samples: &[f64], pct: f64) -> f64 {
    debug_assert!((0.0..=100.0).contains(&pct));
    if samples.is_empty() {
        return 0.0;
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Nearest-rank — fine at our sample sizes (5–15) and free of off-by-one
    // ambiguity vs linear interpolation.
    let idx = ((pct / 100.0) * (s.len() as f64 - 1.0)).round() as usize;
    s[idx.min(s.len() - 1)]
}

fn stats(samples: &[f64]) -> (f64, f64, f64, f64) {
    let median = percentile(samples, 50.0);
    let p95 = percentile(samples, 95.0);
    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (round1(median), round1(p95), round1(min), round1(max))
}

// ---------------------------------------------------------------------------
// Host metadata (best-effort; missing fields → null, never fabricated).
// ---------------------------------------------------------------------------

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn cpu_model() -> Option<String> {
    // Linux: /proc/cpuinfo `model name` line. Other OSes: skip (null).
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in cpuinfo.lines() {
        if let Some(rest) = line.strip_prefix("model name") {
            if let Some(idx) = rest.find(':') {
                return Some(rest[idx + 1..].trim().to_owned());
            }
        }
    }
    None
}

fn branch_head_short_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace_root())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ---------------------------------------------------------------------------
// M1 — LSP didChange → hover round-trip (warm).
// ---------------------------------------------------------------------------
//
// Drives the LSP service in-process through the ONE supported harness
// (`sysml_lsp_server::test_harness::TestServer`) — the same driver
// sysml-lsp-server's own protocol_tests use.
//
// Background tasks (library loading, workspace indexing) are deliberately NOT
// skipped here (`skip_background_tasks: false`): this baseline measures the
// real keystroke path including library warmup. The first 5 samples are
// dropped as warmup to absorb the first-time cost. `stage_timeout: None` —
// this is a latency measurement, not a hang guard.

const M1_TOTAL_SAMPLES: usize = 20;
const M1_WARMUP: usize = 5;

async fn run_m1_lsp() -> JsonValue {
    let fixture_path = workspace_root()
        .join("examples")
        .join("the-book-corpus")
        .join("coffee-machine")
        .join("definitions.sysml");
    let source = std::fs::read_to_string(&fixture_path).expect("read coffee-machine definitions");
    let uri = format!("file://{}", fixture_path.display());

    let ts = TestServer::with_options(TestServerOptions {
        skip_background_tasks: false,
        skip_disk_project_load: false,
        client_capabilities: None,
        stage_timeout: None,
    });
    ts.initialize_full().await;
    ts.open_document(&uri, &source).await;

    // Pick a known-good hover position. Line 0 is the comment header in
    // definitions.sysml; line 2 is `package Definitions {`. Land the cursor
    // on the `D` of `Definitions` (column 8). We're measuring time-to-response,
    // not response content.
    let hover_line: u32 = 2;
    let hover_char: u32 = 8;

    // Build alternating "small change" content snapshots. Even iteration
    // appends a single space at end of file; odd iteration removes it.
    // Both are guaranteed not to invalidate the hover position above.
    let make_changed = |i: usize| -> String {
        if i % 2 == 0 {
            format!("{source} ")
        } else {
            source.clone()
        }
    };

    let mut samples_ms: Vec<f64> = Vec::with_capacity(M1_TOTAL_SAMPLES);

    for i in 0..M1_TOTAL_SAMPLES {
        let new_text = make_changed(i);
        let version = (i as i32) + 1;

        let t0 = Instant::now();

        ts.change_document(&uri, version, &new_text).await;
        let _hover = ts.hover(&uri, hover_line, hover_char).await;

        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        samples_ms.push(elapsed_ms);
    }

    ts.shutdown().await;

    let measured = &samples_ms[M1_WARMUP..];
    let (median, p95, min, max) = stats(measured);

    eprintln!(
        "[M1 LSP didChange→hover] samples={} (warmup_dropped={}) median={}ms p95={}ms min={}ms max={}ms",
        measured.len(),
        M1_WARMUP,
        median,
        p95,
        min,
        max,
    );

    json!({
        "samples": measured.len(),
        "warmup_dropped": M1_WARMUP,
        "median": median,
        "p95": p95,
        "min": min,
        "max": max,
    })
}

// ---------------------------------------------------------------------------
// M2 — REST POST /sources cold vs warm.
// ---------------------------------------------------------------------------
//
// Builds the live axum router via sysml_api::create_router and dispatches
// requests via tower::ServiceExt::oneshot. The REST endpoint is named
// `/sources` (POST) — confirmed in `crates/tooling/sysml-api/src/lib.rs`
// at the route declaration `/sources -> post(load_source)`. So the
// artifact field is `m2_rest_post_sources` exactly.
//
// "Cold" = first-ever call into a freshly constructed AppState (service has
// no graphs loaded, no parser cache warm, library not yet loaded). "Warm"
// = subsequent calls against the same AppState with the same payload.
// The cold/warm ratio is the most direct empirical anchor for whether
// S2 (service onto ide-db) and S3 (runtime out of Salsa) actually hit
// the cache redundancy goal — a wide ratio today should compress
// significantly post-S2.

const M2_WARM_SAMPLES: usize = 10;

async fn run_m2_rest() -> JsonValue {
    let fixture_path = workspace_root()
        .join("examples")
        .join("the-book-corpus")
        .join("coffee-machine")
        .join("definitions.sysml");
    let source = std::fs::read_to_string(&fixture_path).expect("read coffee-machine definitions");

    let body_json = serde_json::to_string(&json!({
        "uri": "file:///perf-baseline-m2.sysml",
        "source": source,
    }))
    .unwrap();

    let make_request = || {
        Request::builder()
            .method("POST")
            .uri("/sources")
            .header("content-type", "application/json")
            .body(Body::from(body_json.clone()))
            .unwrap()
    };

    // Fresh state for the cold call.
    let state = Arc::new(AppState::new());
    let app = create_router(state);

    let t0 = Instant::now();
    let cold_resp = app.clone().oneshot(make_request()).await.unwrap();
    let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let cold_status = cold_resp.status();
    // Drain body so the latency includes full response handling.
    let _ = cold_resp.into_body().collect().await;

    if cold_status != StatusCode::CREATED {
        eprintln!(
            "[M2 REST POST /sources] WARNING: cold call returned {} (expected 201 CREATED)",
            cold_status
        );
    }

    let mut warm_samples_ms: Vec<f64> = Vec::with_capacity(M2_WARM_SAMPLES);
    for _ in 0..M2_WARM_SAMPLES {
        let t = Instant::now();
        let resp = app.clone().oneshot(make_request()).await.unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let _ = resp.into_body().collect().await;
        warm_samples_ms.push(ms);
    }

    let (warm_median, warm_p95, _, _) = stats(&warm_samples_ms);
    let cold_ms_r = round1(cold_ms);
    let ratio = if warm_median > 0.0 {
        round1(cold_ms_r / warm_median)
    } else {
        0.0
    };

    eprintln!(
        "[M2 REST POST /sources] cold={}ms warm_median={}ms warm_p95={}ms cold_to_warm_ratio={}",
        cold_ms_r, warm_median, warm_p95, ratio,
    );

    json!({
        "cold_ms": cold_ms_r,
        "warm_samples": warm_samples_ms.len(),
        "warm_median_ms": warm_median,
        "warm_p95_ms": warm_p95,
        "cold_to_warm_ratio": ratio,
    })
}

// ---------------------------------------------------------------------------
// M3 — simulate.start for espresso-production-cell.
// ---------------------------------------------------------------------------
//
// Dispatch through the live `SysmlService` (NOT a subprocess). The
// service.simulate_start() return value (Ok((session_key, StepResult)))
// IS the ready-to-step signal — there's no out-of-band event channel
// for state-machine sim sessions, so timing the call return is
// authoritative.
//
// espresso-production-cell is multi-file with cross-file imports, so each
// sample needs a fresh service that has loaded the workspace. That
// load step is captured separately as M3b for separability.
//
// The state-machine name passed to simulate.start is the bare def name
// `StationLifecycle` (a direct `state def` of `BrewStation`, multiplied per
// station). `compile_state_machine` resolves bare names against elaborated owners.

const M3_SAMPLES: usize = 5;

fn run_m3_simulate_start() -> JsonValue {
    let cell_dir = workspace_root()
        .join("examples")
        .join("espresso-production-cell");
    if !cell_dir.exists() {
        eprintln!(
            "[M3 simulate.start] SKIP: espresso-production-cell dir not found at {}",
            cell_dir.display()
        );
        return json!({
            "samples": 0,
            "median": null,
            "min": null,
            "max": null,
            "load_workspace_median_ms": null,
            "skipped": true,
            "skip_reason": "espresso-production-cell dir missing",
        });
    }
    // Entry file carrying the target state machine (BrewStation::StationLifecycle,
    // a direct state def multiplied per station in the cell).
    let entry_uri = format!(
        "file://{}",
        cell_dir.join("Structure").join("BrewStation.sysml").display()
    );

    let mut start_samples_ms: Vec<f64> = Vec::with_capacity(M3_SAMPLES);
    let mut load_samples_ms: Vec<f64> = Vec::with_capacity(M3_SAMPLES);
    let mut last_error: Option<String> = None;

    for sample_idx in 0..M3_SAMPLES {
        let service = SysmlService::empty();

        let t_load = Instant::now();
        let load_result = service.load_workspace(&cell_dir);
        let load_ms = t_load.elapsed().as_secs_f64() * 1000.0;

        match load_result {
            Ok(_) => {}
            Err(e) => {
                last_error = Some(format!("sample {sample_idx}: load_workspace failed: {e}"));
                eprintln!(
                    "[M3 simulate.start] sample {} load_workspace error: {e}",
                    sample_idx
                );
                continue;
            }
        }

        let t_start = Instant::now();
        let start_result = service.simulate_start(&entry_uri, "StationLifecycle");
        let start_ms = t_start.elapsed().as_secs_f64() * 1000.0;

        match start_result {
            Ok((_session_key, _step)) => {
                load_samples_ms.push(load_ms);
                start_samples_ms.push(start_ms);
            }
            Err(e) => {
                let msg = format!("sample {sample_idx}: simulate_start failed: {e}");
                eprintln!("[M3 simulate.start] {msg}");
                if last_error.is_none() {
                    last_error = Some(msg);
                }
            }
        }
    }

    if start_samples_ms.is_empty() {
        return json!({
            "samples": 0,
            "median": null,
            "min": null,
            "max": null,
            "load_workspace_median_ms": null,
            "error": last_error,
        });
    }

    let (median, _, min, max) = stats(&start_samples_ms);
    let (load_median, _, _, _) = stats(&load_samples_ms);

    eprintln!(
        "[M3 simulate.start (espresso-production-cell)] samples={} median={}ms min={}ms max={}ms load_workspace_median={}ms",
        start_samples_ms.len(),
        median,
        min,
        max,
        load_median,
    );

    let mut out = json!({
        "samples": start_samples_ms.len(),
        "median": median,
        "min": min,
        "max": max,
        "load_workspace_median_ms": load_median,
    });
    if let Some(err) = last_error {
        out["last_error"] = JsonValue::String(err);
    }
    out
}

// ---------------------------------------------------------------------------
// Main entry point.
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn perf_baseline() {
    // Single-threaded multi-async runtime: M1/M2 are async, M3 is sync.
    let rt = Runtime::new().expect("build tokio runtime");

    let m1 = rt.block_on(run_m1_lsp());
    let m2 = rt.block_on(run_m2_rest());
    let m3 = run_m3_simulate_start();

    // ---------------- Sanity assertions ----------------
    //
    // No latency thresholds. Just: each measurement reports a non-zero
    // sample count where one exists. M3 may legitimately report
    // samples=0 if simulate.start panics on the espresso fixture; in that
    // case we still write the artifact (with the failure mode noted)
    // and fail the assertion so the user knows to investigate.

    assert!(
        m1.get("samples").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
        "M1 produced zero samples — LSP harness broken: {m1}",
    );
    assert!(
        m2.get("warm_samples").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
        "M2 produced zero warm samples — REST harness broken: {m2}",
    );
    assert!(
        m3.get("samples").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
        "M3 produced zero samples — simulate.start broken on espresso-production-cell: {m3}",
    );

    // ---------------- Assemble artifact ----------------

    let captured_at_commit = branch_head_short_sha()
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null);

    let cpu = cpu_model()
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null);

    let artifact = json!({
        "captured_at_commit": captured_at_commit,
        "captured_at_utc": chrono::Utc::now().to_rfc3339(),
        "host": {
            "rustc": rustc_version(),
            "profile": "release",
            "cpu_model": cpu,
        },
        "m1_lsp_didchange_to_hover_ms": m1,
        "m2_rest_post_sources": m2,
        "m3_simulate_start_espresso_cell_ms": m3,
        "notes": "Baseline captured pre-S2. Order-of-magnitude only. \
                  M2 cold/warm ratio is expected to compress after S2 \
                  wires service onto sysml-ide-db cache. M1 is in-process \
                  via the LanguageServer trait (same pattern as \
                  sysml-lsp-server protocol_tests); first-call library \
                  loading is amortised by the warmup samples. M2 uses \
                  tower::ServiceExt::oneshot against the live axum router \
                  rather than a real TCP bind — socket bytes at \
                  127.0.0.1 are sub-millisecond noise that S2 won't \
                  shift. M3 is dispatched through SysmlService directly \
                  (same code path as the REST /api/command and LSP \
                  workspace/executeCommand transports).",
    });

    // ---------------- Write artifact ----------------

    let artifact_path = architectural_cleanup_dir().join("perf-baseline.json");
    std::fs::create_dir_all(architectural_cleanup_dir()).ok();
    std::fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&artifact).expect("serialise artifact"),
    )
    .expect("write perf-baseline.json");

    eprintln!(
        "[perf_baseline] wrote artifact to {}",
        artifact_path.display()
    );
}
