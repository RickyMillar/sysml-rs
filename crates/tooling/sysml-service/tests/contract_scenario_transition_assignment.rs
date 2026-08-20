//! F1 (unification-completion wave) — `sysml.scenario.run` must route SM
//! compilation through the mint/bind orchestrator builder so a
//! transition-effect attribute assignment survives into the trace, exactly
//! as `simulate.start` already does (ledger L44).
//!
//! Before the F1 fix, `scenario::run_scenario` hand-built its orchestrator via
//! bare `Orchestrator::new` + `add_state_machine` and inlined
//! `StateMachineCompiler::compile_named` (a documented never-inline invariant
//! violation). That path never ran `ModelCompiler::mint_slot_store` /
//! `bind_expression_slots`, so a transition effect's attribute assignment
//! (`speed = 100;`) had no slot-routed writeback — the RSC-4 string-identity
//! cull deleted the only other one — and silently vanished from every trace
//! snapshot's context. The fix routes `run_scenario` through
//! `ModelCompiler::build_workspace_orchestrator` (steward ruling, option b).
//!
//! This is the exact scenario-path twin of
//! `contract_simulate.rs::contract_simulate_start_transition_assignment_survives_into_snapshot`
//! (the L44 oracle on the `simulate.start` path).

use serde_json::json;
use sysml_service::{execute_command, SysmlService};

const F1_URI: &str = "inline://f1_scenario_transition_assignment.sysml";

/// A state def whose `idle -> moving` transition effect assigns
/// `speed = 100`, plus a verification case whose event script fires the
/// triggering event `go`. Mirrors the deterministic baseline fixture in
/// `contract_scenario_run_baseline.rs`, adding the transition effect that is
/// the F1 regression oracle.
const F1_SOURCE: &str = r#"
package F1Scenario {
    state def SM {
        attribute speed : Real;
        state idle;
        state moving;
        transition first idle accept go do action { speed = 100; } then moving;
    }

    verification def ScenarioCase {
        requirement SpeedReached {
            doc /* speed reaches 100 after the transition fires */
        }
        action go;
    }
}
"#;

#[test]
fn scenario_run_transition_assignment_survives_into_trace() {
    let service = SysmlService::empty();
    service
        .load_source(F1_URI, F1_SOURCE)
        .expect("load_source must succeed");

    let result = execute_command(
        &service,
        "sysml.scenario.run",
        json!({ "uri": F1_URI, "case_name": "ScenarioCase" }),
    )
    .expect("sysml.scenario.run must succeed");

    let trace = result
        .get("trace")
        .and_then(|v| v.as_array())
        .expect("trace must be a JSON array");
    assert!(!trace.is_empty(), "trace must be non-empty (1 event + buffer ticks)");

    // Sanity: the transition must actually FIRE (SM reaches `moving`), so the
    // only reason `speed` could be absent below is the dropped writeback —
    // not a non-firing fixture. This holds both pre- and post-fix.
    let reached_moving = trace.iter().any(|snap| {
        snap.get("subsystems")
            .and_then(|v| v.as_array())
            .map(|subs| {
                subs.iter().any(|s| {
                    s.get("state").and_then(|v| v.as_str()) == Some("moving")
                })
            })
            .unwrap_or(false)
    });
    assert!(
        reached_moving,
        "fixture sanity: the `idle -> moving` transition must fire on event `go`; \
         states seen: {:?}",
        trace
            .iter()
            .filter_map(|s| s.get("subsystems"))
            .collect::<Vec<_>>(),
    );

    // The `speed = 100;` transition effect must appear in at least one tick's
    // context. Pre-fix the assignment was silently dropped (no slot-routed
    // writeback), so `speed` never appeared in any snapshot.
    let speed_seen = trace.iter().any(|snap| {
        snap.get("context")
            .and_then(|c| c.get("speed"))
            .and_then(|v| v.as_f64())
            .map(|f| (f - 100.0).abs() < 1e-9)
            .unwrap_or(false)
    });
    assert!(
        speed_seen,
        "transition effect `speed = 100;` must survive into a scenario.run trace \
         snapshot context (ledger F1). Contexts seen: {:?}",
        trace
            .iter()
            .map(|s| s.get("context").cloned())
            .collect::<Vec<_>>(),
    );
}
