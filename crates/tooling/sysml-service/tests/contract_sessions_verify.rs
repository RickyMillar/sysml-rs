//! Contract gate for `sysml.sessions.verify` verdict-producer wiring — the
//! GENERIC capability that `sessions.verify` stores its outcome on the session
//! and `sysml.diagram.verdict_overlay` joins it to a declared view's scene, with
//! the staleness contract (verdicts SURVIVE later stepping, labeled via
//! `verified_at_tick`, never silently dropped).
//!
//! ## Migration status (MIG-07)
//!
//! The device-calibrated IEC verdict-matrix test (the `I_residual` fault
//! injection + `kMinTripTime` firmware trip-window verdicts) was split out to
//! the private acceptance pack (MP-COMPLIANCE-TIMING); its generic
//! "verify reads the live slot-store context" capability is covered publicly by
//! `espresso_pump_service::svc_verify_matrix_is_non_vacuous` (VER-SIM).
//!
//! The verdict-overlay test below runs on the espresso-production-cell: it binds
//! to `CellViews::verificationTraceability` (a declared view USAGE of
//! `TraceabilityView` exposing `ScenarioVerification::*` +
//! `ProductQualityRequirements::*`, added for exactly this consumer), so
//! `verdict_overlay(view_usage_id)` joins the cell's verification-case verdicts
//! to that scene.

use std::path::PathBuf;

use serde_json::{json, Value};
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

fn espresso_cell_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/espresso-production-cell")
}

/// dt=100ns — a few ticks are enough for the verdict-overlay anchor below.
const DT_MS: f64 = 1e-4;

/// Sidecar-coupling gate (verdict-producer arc, 2026-07-14): `sessions.verify`
/// stores its outcome on the session, and `sysml.diagram.verdict_overlay`
/// joins it to a declared view's scene — so the canvas verdict channel has a
/// live producer. Also pins the staleness contract (steward ruling): verdicts
/// SURVIVE subsequent stepping, labeled via `verified_at_tick`, rather than
/// silently dropping.
#[test]
fn sessions_verify_feeds_the_canvas_verdict_overlay() {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(espresso_cell_dir()))
        .expect("open espresso-production-cell workspace");

    let created = execute_command(
        &service,
        "sysml.sessions.create",
        json!({ "uri": "__workspace__", "dt_ms": DT_MS }),
    )
    .expect("sessions.create");
    let session_id = created
        .get("id")
        .and_then(|v| v.as_str())
        .expect("session id")
        .to_owned();

    // A few ticks so the session has a snapshot for the overlay to anchor on.
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": session_id, "ticks": 10 }),
    )
    .expect("sessions.step");

    // The verificationTraceability view usage exposes ScenarioVerification::* +
    // ProductQualityRequirements::* — the scene that contains the requirement
    // elements the verdicts key to. Discover its id the same way the frontend
    // does (no hardcoded UUIDs).
    let views = execute_command(
        &service,
        "sysml.query",
        json!({
            "uri": "__workspace__",
            "spec": {
                "filter": { "type": "view", "viewpoint_id": null },
                "projection": "summary",
                "limit": 1000
            }
        }),
    )
    .expect("sysml.query views");
    let view_id = views
        .get("rows")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .find(|row| row.get("name").and_then(|n| n.as_str()) == Some("verificationTraceability"))
        .and_then(|row| row.get("id"))
        .and_then(|v| v.as_str())
        .expect("verificationTraceability view id")
        .to_owned();

    // BEFORE verify: the overlay has no verdicts for this scene (live-sim
    // sessions carry no per-tick constraint_results — that WAS the gap).
    let before = execute_command(
        &service,
        "sysml.diagram.verdict_overlay",
        json!({ "session_id": session_id, "view_usage_id": view_id, "expanded_ids": [] }),
    )
    .expect("verdict_overlay before verify");
    assert!(
        before
            .get("elements")
            .and_then(|e| e.as_object())
            .is_some_and(|e| e.is_empty()),
        "no verdicts expected before sessions.verify, got {before:#}"
    );
    assert!(before.get("verified_at_tick").is_none());

    // Verify, then the overlay must carry the requirement verdicts + anchor.
    execute_command(
        &service,
        "sysml.sessions.verify",
        json!({ "session_id": session_id }),
    )
    .expect("sessions.verify");

    let after = execute_command(
        &service,
        "sysml.diagram.verdict_overlay",
        json!({ "session_id": session_id, "view_usage_id": view_id, "expanded_ids": [] }),
    )
    .expect("verdict_overlay after verify");
    let elements = after
        .get("elements")
        .and_then(|e| e.as_object())
        .expect("elements object");
    assert!(
        !elements.is_empty(),
        "sessions.verify verdicts must join the verificationTraceability scene, got {after:#}"
    );
    let verified_at = after
        .get("verified_at_tick")
        .and_then(|v| v.as_u64())
        .expect("verified_at_tick present once verification joined");
    // Every element entry carries a verdict string from the 4-variant enum.
    // All four are now reachable: a constraint the run could not decide joins
    // the overlay as `Inconclusive`, not flattened into a violation.
    for (id, entry) in elements {
        let verdict = entry.get("verdict").and_then(|v| v.as_str()).unwrap_or("?");
        assert!(
            ["Pass", "Fail", "Inconclusive", "Error"].contains(&verdict),
            "element {id} has invalid verdict {verdict}"
        );
    }

    // STALENESS: step past the verify tick — verdicts stay, labeled, not dropped.
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": session_id, "ticks": 10 }),
    )
    .expect("sessions.step after verify");
    let stale = execute_command(
        &service,
        "sysml.diagram.verdict_overlay",
        json!({ "session_id": session_id, "view_usage_id": view_id, "expanded_ids": [] }),
    )
    .expect("verdict_overlay after further stepping");
    assert!(
        stale
            .get("elements")
            .and_then(|e| e.as_object())
            .is_some_and(|e| !e.is_empty()),
        "verdicts must SURVIVE stepping (stale-but-labeled), got {stale:#}"
    );
    assert_eq!(
        stale.get("verified_at_tick").and_then(|v| v.as_u64()),
        Some(verified_at),
        "verified_at_tick must keep anchoring the verify tick"
    );
    let tick_now = stale.get("tick").and_then(|v| v.as_u64()).unwrap_or(0);
    assert!(
        tick_now > verified_at,
        "session advanced past the verify anchor ({tick_now} > {verified_at})"
    );
}
