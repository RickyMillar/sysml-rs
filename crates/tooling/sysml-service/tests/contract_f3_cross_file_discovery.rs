//! F3 (unification-completion wave) — the analysis / trade-study / whatif
//! command family must discover model elements through the workspace-aware
//! graph, not the parse-only `require_graph`.
//!
//! `analysis_run`, `trade_study`, `whatif`, and `whatif.sweep` each currently
//! acquire their graph via `SysmlService::require_graph` — a single-file,
//! parse-only view. Their workspace-aware siblings (`evaluate.analysis_cases`,
//! `evaluate.verification_cases`, `montecarlo`, `verify`, `scenario.run`) route
//! through `workspace_aware_graph`. Invoked on a member URI of a multi-file
//! workspace, the require_graph path cannot see any element defined in a
//! sibling file, so cross-file cases/targets are undiscoverable. Phase 2 of
//! this wave routes the four commands through `workspace_aware_graph`.
//!
//! Fixture: `tests/fixtures/f3-cross-file/` — `Analysis.sysml` (file A, the
//! invoked URI) carries no analysis surfaces; `Defs.sysml` (file B) holds the
//! analysis case `MotorStudy` and the whatif target `rig`.
//!
//! The first test (`discovery_gap_is_load_bearing`) is the ground-truth proof
//! that the require_graph vs workspace_aware_graph choice is observable and
//! load-bearing. The four command tests each assert the correct cross-file
//! behaviour; they were red-by-design (`#[ignore]`'d) until Phase 2 of the
//! unification wave routed analysis_run / trade_study / whatif / whatif.sweep
//! through `workspace_aware_graph` — now they run by default and pass.

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/f3-cross-file")
        .canonicalize()
        .expect("f3-cross-file fixture must exist")
}

/// Open the two-file workspace and return `(service, Analysis.sysml URI)`.
fn open_fixture() -> (SysmlService, String) {
    let svc = SysmlService::empty();
    let ctx = svc
        .open_context(OpenTarget::Folder(fixture_root()))
        .expect("open_context on f3-cross-file folder must succeed");
    let analysis_uri = ctx
        .loaded_uris
        .into_iter()
        .find(|u| u.contains("Analysis"))
        .expect("Analysis.sysml must be loaded");
    (svc, analysis_uri)
}

/// GROUND TRUTH (runs by default): the require_graph vs workspace_aware_graph
/// discovery genuinely differs, and that difference decides whether an analysis
/// case in a sibling file is compilable. This is the load-bearing justification
/// for Phase 2 routing the four commands through `workspace_aware_graph`.
#[test]
fn discovery_gap_is_load_bearing() {
    let (svc, uri) = open_fixture();

    let rq = svc.require_graph(&uri).expect("require_graph");
    let ws = svc.workspace_aware_graph().expect("workspace_aware_graph");

    let has = |g: &sysml_core::ModelGraph, name: &str| {
        g.elements.values().any(|e| e.name.as_deref() == Some(name))
    };

    // The file-B elements the commands need are invisible to the parse-only
    // view of file A, but present in the merged workspace graph.
    for name in ["MotorStudy", "rig"] {
        assert!(
            !has(&rq, name),
            "require_graph(Analysis.sysml) must NOT see file-B element `{name}` \
             (parse-only, single-file)"
        );
        assert!(
            has(&ws, name),
            "workspace_aware_graph(Analysis.sysml) MUST see file-B element `{name}` \
             (merged workspace elaboration)"
        );
    }

    // The gap is not cosmetic: it decides compilability of the cross-file
    // analysis case. This is exactly the red→green the `analysis_run` command
    // test below asserts once Phase 2 switches its accessor.
    assert!(
        sysml_runtime::cases::compile_analysis_case("MotorStudy", &rq).is_err(),
        "compile_analysis_case on the require_graph view must fail — the case is \
         in a sibling file"
    );
    assert!(
        sysml_runtime::cases::compile_analysis_case("MotorStudy", &ws).is_ok(),
        "compile_analysis_case on the workspace-aware graph must succeed — this \
         is the discovery the four commands are missing today"
    );
    // Same for trade_study's compiler (the `analysis MotorStudy` usage form is
    // an AnalysisCaseUsage with part children, which compile_trade_study wants).
    assert!(
        sysml_runtime::cases::compile_trade_study("MotorStudy", &rq).is_err(),
        "compile_trade_study on the require_graph view must fail — case is in a sibling file"
    );
    assert!(
        sysml_runtime::cases::compile_trade_study("MotorStudy", &ws).is_ok(),
        "compile_trade_study on the workspace-aware graph must succeed"
    );
}

// ---------------------------------------------------------------------------
// Red-by-design command oracles (Phase 2). Each asserts the correct cross-file
// behaviour; each is red today because the command uses `require_graph`.
// Phase 2 routes the command through `workspace_aware_graph` and un-ignores.
// ---------------------------------------------------------------------------

/// `analysis_run` on file A must resolve the analysis case defined in file B.
/// Red today: require_graph(A) → "analysis case 'MotorStudy' not found".
/// Green side confirmed by `discovery_gap_is_load_bearing`
/// (compile_analysis_case on the workspace graph succeeds).
#[test]
fn analysis_run_resolves_cross_file_case() {
    let (svc, uri) = open_fixture();
    let result = execute_command(
        &svc,
        "sysml.analysis.run",
        json!({ "uri": uri, "case_name": "MotorStudy", "overrides": [] }),
    );
    assert!(
        result.is_ok(),
        "analysis_run must resolve the cross-file analysis case `MotorStudy`; got {result:?}"
    );
}

/// `trade_study` on file A must resolve the analysis case defined in file B.
/// Red today: require_graph(A) → "no analysis case 'MotorStudy' found".
///
/// Correction trail: this was initially misdiagnosed as needing a separate
/// `analysis case NAME`→`AnalysisCaseUsage` parser fix. That was wrong —
/// `analysis case NAME` is simply invalid SysML; the analysis-case USAGE
/// keyword is bare `analysis` (SysML.xtext:2215-2224). The fixture now uses
/// the valid `analysis MotorStudy { … }` usage form, which `compile_trade_study`
/// discovers directly, so this oracle flips green on the workspace_aware_graph
/// switch alone — no parser work.
#[test]
fn trade_study_resolves_cross_file_case() {
    let (svc, uri) = open_fixture();
    let result = execute_command(
        &svc,
        "sysml.trade_study",
        json!({ "uri": uri, "study_name": "MotorStudy", "overrides": [] }),
    );
    assert!(
        result.is_ok(),
        "trade_study must resolve the cross-file analysis case `MotorStudy`; got {result:?}"
    );
}

/// `whatif` on file A must resolve the constraint owner (`rig`) defined in file
/// B so the baseline constraint results carry real satisfied verdicts. Red
/// today: require_graph(A) lacks `rig`, so `owner_scoped_context` finds nothing
/// and every baseline result is `satisfied: null`.
#[test]
fn whatif_resolves_cross_file_target() {
    let (svc, uri) = open_fixture();
    let result = execute_command(
        &svc,
        "sysml.whatif",
        json!({ "uri": uri, "variable_name": "speed", "override_value": "150.0" }),
    )
    .expect("whatif returns Ok (constraints come from the workspace precompile)");
    let baseline = result
        .get("baseline")
        .and_then(|v| v.as_array())
        .expect("whatif result has a `baseline` array");
    let any_resolved = baseline
        .iter()
        .any(|r| r.get("satisfied").map(|s| s.is_boolean()).unwrap_or(false));
    assert!(
        any_resolved,
        "whatif baseline must carry at least one resolved (non-null) constraint \
         verdict once the cross-file target `rig` is discoverable; got {baseline:?}"
    );
}

/// `whatif.sweep` on file A must resolve the swept element (`rig`) defined in
/// file B. Red today: require_graph(A) lacks `rig`'s element id →
/// `ElementNotFound`.
#[test]
fn whatif_sweep_resolves_cross_file_element() {
    let (svc, uri) = open_fixture();
    // The swept element id is workspace-global; take it from the workspace graph.
    let ws = svc.workspace_aware_graph().expect("workspace_aware_graph");
    let rig_id = ws
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("rig"))
        .map(|e| e.id.to_string())
        .expect("rig element id");
    let result = execute_command(
        &svc,
        "sysml.whatif.sweep",
        json!({
            "uri": uri,
            "element_id": rig_id,
            "variable_name": "speed",
            "start": 0.0,
            "end": 200.0,
            "steps": 4,
        }),
    );
    assert!(
        result.is_ok(),
        "whatif.sweep must resolve the cross-file swept element `rig`; got {result:?}"
    );
}
