//! Contract gate for `sysml.analysis.run`'s objective verdict
//! (verification-analysis-model-study.md §3.2 ENDORSED billet).
//!
//! `analysis.run` surfaces the objective verdict its runtime already computes
//! (`AnalysisCaseIR::verify_solved_objective`, the no-second-solve core that
//! `run_and_verify` delegates to), through the ONE `VerificationRunner` and
//! projected onto the shared `VerifyResult` wire shape. Two obligations pinned
//! here:
//!
//!   - a case WITH a `verify`'d objective (§7.23.2: objective subject binds to
//!     the analysis result) carries `objective_verdict`, labeled
//!     `evaluation_mode: "static"` — a one-shot solve has no live/archived
//!     session, so it is NOT trajectory (study §3.2 steward ruling); the solver
//!     mechanism is disclosed by `tool_name`/`converged`/`iterations`, not a new
//!     enum variant;
//!   - a case WITHOUT a verified objective has NO `objective_verdict` key —
//!     absent, never null-faked (fail-honest).

use serde_json::json;
use sysml_service::{execute_command, SysmlService};

const URI: &str = "analysis_objective.sysml";

// PassExec: solver finds x = target-5 = 3, result = 3 < 10 → objective PASS.
// PlainSolve: same solver shape but NO `objective` block → no objective verdict.
const SOURCE: &str = r#"
    package AnalysisObjectiveGate {
        requirement def ResultBelowLimit {
            subject measuredMass : Real;
            require constraint { measuredMass < 10.0 }
        }
        analysis def PassExec {
            @ToolExecution { attribute toolName = "builtin:bisection"; }
            in attribute target : Real = 8.0;
            attribute x : Real;
            constraint { x - (target - 5.0) }
            return attribute result : Real = x;
            objective { verify ResultBelowLimit; }
        }
        analysis def PlainSolve {
            @ToolExecution { attribute toolName = "builtin:bisection"; }
            in attribute target : Real = 8.0;
            attribute x : Real;
            constraint { x - (target - 5.0) }
            return attribute result : Real = x;
        }
    }
"#;

fn run_analysis(case_name: &str) -> serde_json::Value {
    let service = SysmlService::empty();
    service
        .load_source(URI, SOURCE)
        .expect("load_source must succeed");
    execute_command(
        &service,
        "sysml.analysis.run",
        json!({ "case_name": case_name, "overrides": [] }),
    )
    .unwrap_or_else(|e| panic!("analysis.run({case_name}) must succeed: {e:?}"))
}

#[test]
fn analysis_with_verified_objective_carries_static_objective_verdict() {
    let result = run_analysis("PassExec");

    let objective = result
        .get("objective_verdict")
        .unwrap_or_else(|| panic!("PassExec declares a verify'd objective — objective_verdict must be present; got {result:?}"));

    // Study §3.2 steward ruling: a one-shot solve is `static`, never trajectory.
    assert_eq!(
        objective.get("evaluation_mode").and_then(|v| v.as_str()),
        Some("static"),
        "objective_verdict must be labeled static (no session/ticks)"
    );

    // Verdict through the ONE engine: solver x = 8-5 = 3, result 3 < 10 → pass.
    assert_eq!(
        objective.get("verdict").and_then(|v| v.as_str()),
        Some("pass"),
        "executed result 3.0 < 10.0 → objective PASS"
    );

    // Shared VerifyResult shape: carries the per-requirement rollup.
    assert!(
        objective.get("requirements").and_then(|r| r.as_array()).is_some(),
        "objective_verdict reuses VerifyResult (carries a `requirements` array)"
    );
}

#[test]
fn analysis_without_objective_omits_objective_verdict() {
    let result = run_analysis("PlainSolve");

    assert!(
        result.get("objective_verdict").is_none(),
        "a case with no verified objective must OMIT objective_verdict (never null-faked); got {result:?}"
    );

    // The analysis still produced its result surface.
    assert!(
        result.get("outputs").is_some(),
        "the analysis result surface (outputs) is still present"
    );
}
