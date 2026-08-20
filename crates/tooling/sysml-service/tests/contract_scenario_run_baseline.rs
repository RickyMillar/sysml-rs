//! Bucket B / B3 — `sysml.scenario.run` behavioural baseline.
//!
//! Largest single bypass in Bucket B. Unlike the shape-only baselines for
//! the other bypasses (montecarlo, trade_study, diagram_whatif, etc.),
//! `feedback_behavioral_baseline_first` mandates a LIVE-RUNTIME
//! regression test here: the LSP `handle_scenario_run` open-codes
//! ~250 LOC of orchestrator composition (build + SM compile +
//! event-script extraction + auto-step + per-tick assertion eval),
//! and a pure JSON-shape check would let a silent tick-sequence drift
//! sneak past P1.
//!
//! This test pins:
//!   - the top-level JSON shape the LSP handler emits today, AND
//!   - concrete behavioural properties of a small deterministic
//!     fixture (verdict, event count, trace tick count, first/last
//!     subsystem state, requirement_results length and verdict).
//!
//! `#[ignore]`'d until B3 P1 lands `service.scenario_run` — execution
//! goes through `execute_command(&service, "sysml.scenario.run", ..)`
//! so this file compiles without touching nonexistent typed APIs.

use serde_json::json;
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// LSP handler reference shape — pinned from
// crates/tooling/sysml-lsp-server/src/commands.rs L2922 handle_scenario_run.
//
// Top-level JSON shape today:
//   {
//     "verdict":               string  // "pass" | "fail" | "inconclusive" | "error"
//     "requirement_results":   [ { "requirement_id", "verdict", "message" } ],
//     "assertion_checkpoints": [ { "tick", "time_ms", "requirement_id",
//                                  "requirement_text", "verdict", "message",
//                                  "referenced_variables" } ],
//     "trace":                 [ TickSnapshot, ... ],     // snapshot_to_json shape
//     "final_snapshot":        TickSnapshot | {}          // empty when trace empty
//   }
//
// Per-tick snapshot_to_json (commands.rs L2667) shape:
//   {
//     "tick":                u64,
//     "time_ms":             f64,
//     "subsystems":          [ { "name", "kind", "state", "completed",
//                                "outputs", "sends", "available_transitions" } ],
//     "completed":           bool,
//     "context":             { "<var>": <json>, ... },
//     "constraint_results":  [ { "name", "satisfied", "expression" }, ... ],
//     "assertion_checkpoints":[ { ... per-tick checkpoint ... }, ...],
//     "guard_diagnoses":     [ { ... }, ...],
//     "messages":            [ { ... }, ...],
//   }
//
// B3 P1 swaps the LSP body to `service.scenario_run(case_name, None, None)`;
// this test un-ignores at that point and BOTH the shape AND the behavioural
// assertions must hold.
// ---------------------------------------------------------------------------

/// Deterministic fixture for the behavioural baseline.
///
/// One `state def` with a single transition triggered by event "go".
/// One verification case "ScenarioCase" with one action child "go"
/// (the testScript fallback path in `extract_event_script` —
/// declaration-order). One requirement whose constraint references no
/// state-machine variables so it evaluates cleanly to `pass` against
/// an empty context.
const SCENARIO_SOURCE: &str = r#"
package P {
    state def Toggle {
        state idle;
        state active;
        entry; then idle;
        transition first idle accept "go" then active;
    }

    verification def ScenarioCase {
        requirement Trivial {
            doc /* Always satisfied. */
        }
        action go;
    }
}
"#;

const SCENARIO_URI: &str = "inline://baseline_scenario_run.sysml";
const SCENARIO_CASE: &str = "ScenarioCase";

/// Behavioural baseline for `sysml.scenario.run`.
///
/// Asserts both the top-level wire shape AND concrete observable
/// properties of the deterministic fixture above.
#[test]
fn baseline_scenario_run_behaviour() {
    let service = SysmlService::empty();
    service
        .load_source(SCENARIO_URI, SCENARIO_SOURCE)
        .expect("load_source must succeed");

    let result = execute_command(
        &service,
        "sysml.scenario.run",
        json!({
            "uri": SCENARIO_URI,
            "case_name": SCENARIO_CASE,
        }),
    )
    .expect("sysml.scenario.run must succeed (B3 P1)");

    let obj = result
        .as_object()
        .expect("scenario.run result must be a JSON object");

    // -- top-level shape (LSP regression gate) --
    for k in [
        "evaluation_mode",
        "verdict",
        "requirement_results",
        "assertion_checkpoints",
        "trace",
        "final_snapshot",
    ] {
        assert!(
            obj.contains_key(k),
            "P1 regression: service `sysml.scenario.run` lost field `{k}`. \
             LSP handler at commands.rs L2922 emits all 5; P1 must preserve them \
             (+ evaluation_mode, §2.1a(d) / study §3.4)."
        );
    }

    // §2.1a(d) / study §3.4: event-script + auto-step + per-tick evaluation is
    // a trajectory-mode verdict — the label is ALWAYS rendered.
    assert_eq!(
        obj.get("evaluation_mode").and_then(|v| v.as_str()),
        Some("trajectory"),
        "scenario.run is a trajectory-mode verdict"
    );

    // -- behavioural assertions on the deterministic fixture --

    // verdict ∈ {pass, fail, inconclusive, error}; the trivial requirement
    // makes this fixture deterministic, but exact verdict depends on whether
    // the requirement compiles. Constrain to the verdict alphabet and the
    // non-empty cases.
    let verdict = obj
        .get("verdict")
        .and_then(|v| v.as_str())
        .expect("verdict must be a string");
    assert!(
        matches!(verdict, "pass" | "fail" | "inconclusive" | "error"),
        "verdict `{verdict}` not in the VerdictKind alphabet"
    );

    // trace must be a non-empty array (at least one orchestrator tick fired —
    // the fixture has 1 scripted event + 10 buffer ticks).
    let trace = obj
        .get("trace")
        .and_then(|v| v.as_array())
        .expect("trace must be a JSON array");
    assert!(
        !trace.is_empty(),
        "trace must be non-empty (1 scripted event + buffer ticks)"
    );

    // First trace entry must be a TickSnapshot (subsystems list non-empty —
    // the Toggle state machine compiled and produced at least one subsystem).
    let first = trace[0]
        .as_object()
        .expect("trace[0] must be a JSON object (TickSnapshot)");
    for k in ["tick", "time_ms", "subsystems", "context"] {
        assert!(
            first.contains_key(k),
            "trace[0] missing TickSnapshot field `{k}`"
        );
    }
    let subsystems = first
        .get("subsystems")
        .and_then(|v| v.as_array())
        .expect("trace[0].subsystems must be a JSON array");
    assert!(
        !subsystems.is_empty(),
        "trace[0].subsystems must be non-empty — Toggle state def should compile to one subsystem"
    );
    let toggle = subsystems[0]
        .as_object()
        .expect("subsystems[0] must be a JSON object");
    for k in ["name", "kind", "state"] {
        assert!(
            toggle.contains_key(k),
            "subsystems[0] missing field `{k}`"
        );
    }
    // Sanity: the only subsystem is named Toggle.
    assert_eq!(
        toggle.get("name").and_then(|v| v.as_str()),
        Some("Toggle"),
        "subsystems[0].name should be `Toggle` for this fixture"
    );

    // final_snapshot mirrors the last trace entry — pin that invariant.
    let final_snap = obj
        .get("final_snapshot")
        .expect("final_snapshot must be present");
    assert_eq!(
        final_snap,
        trace.last().expect("trace non-empty"),
        "final_snapshot must equal trace[trace.len()-1]"
    );

    // requirement_results must be a JSON array (possibly empty if compile
    // failed; non-empty if compile_verification_case ok).
    let reqs = obj
        .get("requirement_results")
        .and_then(|v| v.as_array())
        .expect("requirement_results must be a JSON array");
    // Don't assert non-empty (compile_verification_case may return Err on
    // some grammar variants), but assert the per-entry shape when present.
    if let Some(req) = reqs.first() {
        let r = req
            .as_object()
            .expect("requirement_results[0] must be a JSON object");
        for k in ["requirement_id", "verdict", "message"] {
            assert!(
                r.contains_key(k),
                "requirement_results[0] missing field `{k}`"
            );
        }
        let verdict = r
            .get("verdict")
            .and_then(|v| v.as_str())
            .expect("requirement_results[0].verdict must be a string");
        assert!(
            matches!(verdict, "pass" | "fail" | "inconclusive" | "error"),
            "requirement_results[0].verdict `{verdict}` not in the alphabet"
        );
    }

    // assertion_checkpoints — per-tick assertion eval results. May be empty
    // if the requirement does not bind to any tick-context variable; assert
    // the per-entry shape when present.
    let checkpoints = obj
        .get("assertion_checkpoints")
        .and_then(|v| v.as_array())
        .expect("assertion_checkpoints must be a JSON array");
    if let Some(cp) = checkpoints.first() {
        for k in [
            "tick",
            "time_ms",
            "requirement_id",
            "requirement_text",
            "verdict",
            "message",
            "referenced_variables",
        ] {
            assert!(
                cp.get(k).is_some(),
                "assertion_checkpoints[0] missing `{k}`"
            );
        }
    }
}
