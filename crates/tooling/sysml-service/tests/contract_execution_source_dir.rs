//! Cross-command correctness gate for `@DataSource` SampledFunction resolution
//! (execution-entry-unification-plan.md P0 baseline + P3 gate).
//!
//! A model with `@DataSource`-backed `SampledFunction` lookup tables
//! (`examples/espresso-pump-hybrid`) only resolves its `__sf_*` runtime slots when
//! the service threads the project/workspace source directory into the runtime
//! compiler (`Snapshot::with_source_dir`). Before the `execution_snapshot`
//! chokepoint, only `verify_with_simulation` did this; every sibling execution
//! command open-coded `Snapshot::new` and dropped the knob, so they failed:
//!
//!   error[RS003]: unresolved runtime name '__sf_<curve>' ...
//!
//! This gate drives the espresso pump-hybrid workspace through every ODE-bearing execution
//! command and asserts none of them regress to that RS003. It is RED on `main`
//! (pre-chokepoint) for all paths except `verify_with_simulation`, and GREEN
//! once `execution_snapshot` is threaded through every build site.
//!
//! If any path here fails with an `__sf_` RS003 again, a new execution command
//! has dropped the source-dir wiring — funnel it through `execution_snapshot`.

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

fn examples_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

fn pump_root() -> PathBuf {
    examples_root().join("espresso-pump-hybrid")
}

/// A generic, `@DataSource`-free workspace used only to plant a project
/// registration whose root must NOT be reused when a second workspace's CSV
/// paths resolve (the workspace-switch regression). damped-oscillator is a
/// legacy-exempt generic fixture with no data/ dir.
fn no_data_workspace_root() -> PathBuf {
    examples_root().join("damped-oscillator")
}

/// Assert a command did not fail with the `__sf_` RS003 (the source-dir gap).
/// Other errors are surfaced verbatim so a baseline run shows exactly what broke.
fn assert_no_sf_rs003(label: &str, result: Result<serde_json::Value, sysml_service::ServiceError>) {
    match result {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            assert!(
                !(msg.contains("__sf_") && msg.contains("RS003")),
                "{label}: regressed to the @DataSource source-dir gap — {msg}"
            );
            // A non-RS003 error is out of scope for this gate but still a failure
            // worth seeing: these commands are expected to run clean on the fixture under test.
            panic!("{label}: unexpected error (not the __sf_ gap): {msg}");
        }
    }
}

fn open_pump() -> (SysmlService, String) {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(pump_root()))
        .expect("open espresso pump-hybrid workspace");
    let pump_ode_uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("PumpODE"))
        .expect("PumpODE URI");
    (service, pump_ode_uri)
}

/// The reported bug: coupled SM+ODE orchestrator over the merged workspace via
/// the synthetic `__workspace__` URI (where `project_root_dir` returns None and
/// only the `workspace_root` fallback resolves the CSV root).
#[test]
fn orchestrate_workspace_start_resolves_sampled_functions() {
    let (service, _uri) = open_pump();
    let result = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": "__workspace__", "dt_ms": 0.001, "max_time_ms": 2.0 }),
    );
    assert_no_sf_rs003("orchestrate.workspace.start(__workspace__)", result);
}

/// The same command via a real file URI (where `project_root_dir` resolves).
#[test]
fn orchestrate_workspace_start_file_uri_resolves_sampled_functions() {
    let (service, uri) = open_pump();
    let result = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": uri, "dt_ms": 0.001, "max_time_ms": 2.0 }),
    );
    assert_no_sf_rs003("orchestrate.workspace.start(file)", result);
}

/// Sibling of the lone-correct `verify_with_simulation` — same body, missing the
/// three source-dir lines on `main`.
#[test]
fn verify_with_simulation_trace_resolves_sampled_functions() {
    let (service, uri) = open_pump();
    let result = execute_command(
        &service,
        "sysml.verify_with_simulation_trace",
        json!({
            "uri": uri,
            "sm_name": "PumpCycle",
            "overrides": [],
            "dt_ms": 0.001,
            "max_time_ms": 2.0,
            "max_points": 200,
        }),
    );
    assert_no_sf_rs003("verify_with_simulation_trace", result);
}

/// ODE parameter sweep — SampledFunction-bearing by nature.
#[test]
fn ode_sweep_resolves_sampled_functions() {
    let (service, uri) = open_pump();
    let result = execute_command(
        &service,
        "sysml.trade_study.ode_sweep",
        json!({
            "uri": uri,
            "sm_name": "PumpCycle",
            "parameter_name": "restrictionConductance",
            "min_value": 0.3,
            "max_value": 1.5,
            "steps": 3,
            "dt_ms": 0.001,
            "max_time_ms": 2.0,
        }),
    );
    assert_no_sf_rs003("trade_study.ode_sweep", result);
}

/// Continuous auto-discovery path (`build_orchestrator`).
#[test]
fn continuous_auto_resolves_sampled_functions() {
    let (service, uri) = open_pump();
    let result = execute_command(
        &service,
        "sysml.simulate.continuous.auto",
        json!({
            "uri": uri,
            "sm_name": "PumpCycle",
            "dt_ms": 0.001,
            "max_time_ms": 2.0,
        }),
    );
    assert_no_sf_rs003("simulate.continuous.auto", result);
}

/// Workspace-switch regression: load workspace A (damped-oscillator, which has
/// NO `data/` directory), then switch to workspace B (espresso-pump-hybrid) via a
/// second `load_workspace`, and run B's coupled orchestrator over `__workspace__`.
///
/// The bug (reproduced Jul 4 via the sim app + REST): the source-dir resolver
/// used workspace A's root — a stale project registration surviving the switch —
/// so B's relative `@DataSource { file = "data/generated_pump_closing.csv" }`
/// resolved against `.../damped-oscillator/data/...` and hard-errored with "No
/// such file or directory". `load_project` pushed a second project sharing the workspace
/// pid instead of superseding the first, so `project_root_dir_for_handle`
/// returned the first (stale, workspace-A) entry.
///
/// After the fix, `__workspace__` resolves the CURRENT workspace root (B) and
/// the SampledFunction slots load cleanly.
#[test]
fn workspace_switch_resolves_second_workspace_source_dir() {
    let service = SysmlService::empty();

    // Workspace A first — damped-oscillator has no @DataSource / data dir, but
    // registering it plants the project whose root the buggy resolver reused.
    service
        .load_workspace(&no_data_workspace_root())
        .expect("load workspace A (damped-oscillator)");

    // Switch to workspace B — espresso-pump-hybrid, whose PumpCharacteristic
    // declares the relative CSV paths that must resolve against B's root, not A's.
    service
        .load_workspace(&pump_root())
        .expect("load workspace B (espresso-pump-hybrid)");

    let result = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": "__workspace__", "dt_ms": 0.001, "max_time_ms": 2.0 }),
    );
    // A stale workspace-A root surfaces as a CSV read failure against
    // `.../damped-oscillator/data/...`; assert it did not happen.
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("damped-oscillator"),
            "workspace switch left a stale source_dir — resolved against \
             workspace A: {msg}"
        );
    }
    assert_no_sf_rs003("orchestrate.workspace.start after switch", result);
}

/// The reference path that already threads source-dir — must stay green
/// throughout (anchors the others).
#[test]
fn verify_with_simulation_reference_stays_green() {
    let (service, uri) = open_pump();
    let result = execute_command(
        &service,
        "sysml.verify_with_simulation",
        json!({
            "uri": uri,
            "sm_name": "PumpCycle",
            "overrides": [],
            "dt_ms": 0.001,
            "max_time_ms": 2.0,
        }),
    );
    assert_no_sf_rs003("verify_with_simulation (reference)", result);
}
