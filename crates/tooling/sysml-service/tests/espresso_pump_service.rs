//! Service/session contract gates for the espresso-pump-hybrid fixture
//! (plan §10.1 PUMP-SVC-01; matrix rows SVC-SESSION, SES-ARCHIVE, VER-SIM,
//! RES-OVERRIDE service path). Exercises the unified session lifecycle end to
//! end on a public synthetic fixture: create -> bulk-step -> time-series ->
//! stop/archive -> fork, with model/session identity preserved, plus the
//! simulation-backed verdict matrix, the step-override scenario path, and a
//! fail-hard on an unknown target.

use std::path::{Path, PathBuf};

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::execution::{SessionKind, MAX_BULK_STEP_TICKS};
use sysml_service::{execute_command, ServiceError, SysmlService};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/espresso-pump-hybrid")
}

fn open() -> SysmlService {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(fixture_dir()))
        .expect("open pump workspace");
    service
}

const WS: &str = "__workspace__";
const SEVERE: &[(String, String)] = &[];

fn severe() -> Vec<(String, String)> {
    vec![("restrictionConductance".to_string(), "0.3".to_string())]
}

fn pump_state(service: &SysmlService, id: &str) -> String {
    let det = service
        .sessions_info(id, Some(false))
        .expect("info")
        .expect("session detail");
    det.subsystems
        .iter()
        .find(|s| s.name == "PumpCycle")
        .map(|s| s.current_state.clone())
        .expect("PumpCycle subsystem present")
}

// ---------------------------------------------------------------------------
// SVC-SESSION — kind inference, provenance capture, exact-N bulk step, and a
// fail-hard on an unknown target.
// ---------------------------------------------------------------------------

#[test]
fn svc_session_create_infers_kind_and_bulk_steps_exactly() {
    let service = open();
    let s = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), None)
        .expect("create the pump session");

    // Multi-subsystem (ODE + state machine) workspace -> orchestrator session.
    assert_eq!(s.kind, SessionKind::Orchestrator);
    assert_eq!(s.subsystem_count, 2, "ODE + state machine");
    let digest = s
        .provenance
        .as_ref()
        .map(|p| p.model_digest.clone())
        .expect("content-digest provenance captured");
    assert!(!digest.is_empty());

    // Bulk step advances EXACTLY the requested number of ticks.
    let base = s.tick;
    let stepped = service.sessions_step(&s.id, None, None, Some(100)).expect("bulk step");
    assert_eq!(stepped.ticks_advanced, 100);
    assert_eq!(stepped.tick, base + 100, "exactly 100 ticks advanced");
}

// ---------------------------------------------------------------------------
// SVC-SESSION (regression) — the workspace's subsystem SET is EXACTLY
// {PumpCycle, ReciprocatingPump}. Guards against the instance-multiplication
// classification bug where library `StateAction`/`Performance` pseudo-state
// features (`start`, `done`, both typed by the library `Part`) were mis-expanded
// into spurious prefixed subsystems (`start.StateAction`, `done.Part`, …),
// inflating the count from 2 to 6. Fixed by the `is_library_element` filter in
// `ModelCompiler::instance_specs_from_tree` — the same filter the SM and
// root-action discovery paths already apply.
// ---------------------------------------------------------------------------

#[test]
fn svc_subsystem_set_excludes_library_pseudo_states() {
    let service = open();
    let s = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), None)
        .expect("create the pump session");
    let det = service
        .sessions_info(&s.id, Some(false))
        .expect("info")
        .expect("session detail");

    let mut names: Vec<String> = det.subsystems.iter().map(|sub| sub.name.clone()).collect();
    names.sort();

    // EXACTLY the two real subsystems: the PumpCycle state machine and the
    // ReciprocatingPump ODE. No `start`/`done` pseudo-states, no `StateAction`,
    // no `exhibitedStates` Part.
    assert_eq!(
        names,
        vec!["PumpCycle".to_string(), "ReciprocatingPump".to_string()],
        "subsystem set must be exactly {{PumpCycle, ReciprocatingPump}}; \
         spurious library-derived subsystems (start/done/StateAction/Part) leaked in: {names:?}",
    );
    // Belt-and-suspenders: no subsystem name references a state-machine-internal
    // or pseudo-state element even if the model gains real subsystems later.
    for n in &names {
        assert!(
            !n.contains("StateAction")
                && !n.contains("exhibitedStates")
                && !n.starts_with("start.")
                && !n.starts_with("done."),
            "state-machine-internal / pseudo-state element mis-classified as a subsystem: {n}",
        );
    }
}

#[test]
fn svc_session_unknown_target_is_hard_error() {
    let service = open();
    let err = service.sessions_create(WS, Some("NoSuchSubsystem"), Some(2.0), Some(8000.0), None);
    assert!(err.is_err(), "an unknown target must be a hard error, not a silent fallback");
}

/// Bulk-step granular contract (migrated off the legacy oscillator-driven
/// contract_sessions_bulk_step.rs, MIG-07): omitting `ticks` advances exactly
/// one tick (backward compat), `ticks = 0` is a hard `InvalidInput`, and a
/// count over `MAX_BULK_STEP_TICKS` is `InvalidInput` — never a silent clamp.
/// The exact-N bulk advance itself is pinned by
/// `svc_session_create_infers_kind_and_bulk_steps_exactly` above.
#[test]
fn svc_bulk_step_default_zero_and_over_cap_contract() {
    let service = open();
    let s = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), None)
        .expect("create the pump session");

    // Default (no `ticks`) advances exactly one tick.
    let base = s.tick;
    let one = service
        .sessions_step(&s.id, None, None, None)
        .expect("default single step");
    assert_eq!(
        one.tick,
        base + 1,
        "omitting `ticks` must advance exactly one tick (backward compat)"
    );

    // `ticks = 0` is a hard InvalidInput, never a silent clamp/no-op.
    let zero = service.sessions_step(&s.id, None, None, Some(0));
    assert!(
        matches!(zero, Err(ServiceError::InvalidInput(_))),
        "ticks=0 must be InvalidInput, got {zero:?}"
    );

    // Over the cap is InvalidInput, never silently clamped down.
    let over = service.sessions_step(&s.id, None, None, Some(MAX_BULK_STEP_TICKS + 1));
    assert!(
        matches!(over, Err(ServiceError::InvalidInput(_))),
        "over-cap ticks must be InvalidInput, got {over:?}"
    );
}

// ---------------------------------------------------------------------------
// VER-SIM + RES-OVERRIDE (service path) — the step-override scenario selects a
// severe restriction that latches relief, while the un-overridden nominal run
// never relieves. Regression on the bare-key override applying at the service
// layer.
// ---------------------------------------------------------------------------

#[test]
fn svc_nominal_does_not_relieve_severe_override_does() {
    let service = open();

    // Nominal (no override): step well past the severe relief time; never relieves.
    let nom = service.sessions_create(WS, None, Some(2.0), Some(8000.0), None).expect("create nominal");
    service.sessions_step(&nom.id, None, None, Some(3000)).expect("step nominal");
    assert_ne!(pump_state(&service, &nom.id), "relieved", "nominal never relieves");
    let z_nom = service
        .sessions_timeseries(&nom.id, "exposure", None, None)
        .expect("nominal exposure series")
        .points
        .iter()
        .map(|p| p.value)
        .fold(f64::MIN, f64::max);
    assert!(z_nom < 1.0, "nominal exposure stays below the trip (z_max={z_nom})");

    // Severe (restrictionConductance override applied once at step): relieves.
    let sev = service.sessions_create(WS, None, Some(2.0), Some(8000.0), None).expect("create severe");
    service
        .sessions_step(&sev.id, None, Some(&severe()), Some(2500))
        .expect("severe bulk step");
    assert_eq!(pump_state(&service, &sev.id), "relieved", "severe restriction latches relief");
    let z_sev = service
        .sessions_timeseries(&sev.id, "exposure", None, None)
        .expect("severe exposure series")
        .points
        .iter()
        .map(|p| p.value)
        .fold(f64::MIN, f64::max);
    assert!(z_sev > 1.0, "severe exposure crosses the trip (z_max={z_sev})");
    let _ = SEVERE;
}

// ---------------------------------------------------------------------------
// VER-SIM — the simulation-backed verdict matrix is non-vacuous (diagonal pass,
// off-diagonal fail) and every verdict is labeled with an evaluation mode.
// ---------------------------------------------------------------------------

#[test]
fn svc_verify_matrix_is_non_vacuous() {
    let service = open();

    // The per-requirement verdict, keyed on the `verify requirement <name>`
    // usage id (nominalCheck / severeCheck / stabilityCheck).
    let verdict = |rs: &[sysml_service::types::VerifyResult], req_id: &str| -> String {
        rs.iter()
            .flat_map(|r| r.requirements.iter())
            .find(|rq| rq.requirement_id == req_id)
            .map(|rq| rq.verdict.to_lowercase())
            .unwrap_or_default()
    };

    // Nominal run, then verify all declared cases.
    let nom = service.sessions_create(WS, None, Some(2.0), Some(8000.0), None).expect("create nominal");
    service.sessions_step(&nom.id, None, None, Some(3000)).expect("step nominal");
    let nom_verdicts = service.sessions_verify(&nom.id, None).expect("verify nominal");

    // Every case verdict is labeled with an evaluation mode.
    assert!(nom_verdicts.iter().all(|r| !r.evaluation_mode.is_empty()), "evaluation mode labeled");

    // Diagonal: nominal satisfies SafeUnderNominal, fails ProtectsUnderSevere.
    assert_eq!(verdict(&nom_verdicts, "nominalCheck"), "pass");
    assert_eq!(verdict(&nom_verdicts, "severeCheck"), "fail");
    assert_eq!(verdict(&nom_verdicts, "stabilityCheck"), "pass", "bounded envelope holds");

    // Severe run: the diagonal flips (non-vacuity).
    let sev = service.sessions_create(WS, None, Some(2.0), Some(8000.0), None).expect("create severe");
    service.sessions_step(&sev.id, None, Some(&severe()), Some(2500)).expect("step severe");
    let sev_verdicts = service.sessions_verify(&sev.id, None).expect("verify severe");
    assert_eq!(verdict(&sev_verdicts, "nominalCheck"), "fail");
    assert_eq!(verdict(&sev_verdicts, "severeCheck"), "pass");
}

// ---------------------------------------------------------------------------
// SES-ARCHIVE + SVC-SESSION — create/step/time-series/stop/archive round-trip
// preserves model/session identity; fork inherits provenance.
// ---------------------------------------------------------------------------

#[test]
fn svc_timeseries_archive_and_fork_round_trip() {
    let service = open();
    let s = service.sessions_create(WS, None, Some(2.0), Some(8000.0), None).expect("create");
    let digest = s.provenance.as_ref().unwrap().model_digest.clone();
    let id = s.id.clone();

    let stepped = service.sessions_step(&id, None, None, Some(200)).expect("step");

    // Time series has one point per advanced tick, time strictly increasing.
    let ts = service.sessions_timeseries(&id, "exposure", None, None).expect("timeseries");
    assert_eq!(ts.points.len() as u64, stepped.tick, "one recorded sample per advanced tick");
    assert!(
        ts.points.windows(2).all(|w| w[1].time_ms > w[0].time_ms),
        "time series is monotone in time"
    );

    // Fork inherits the parent's content-digest provenance; the child is a new id.
    let child = service.sessions_fork(&id).expect("fork");
    assert_ne!(child.id, id, "fork mints a new session id");
    assert_eq!(
        child.provenance.as_ref().unwrap().model_digest,
        digest,
        "fork inherits the model digest"
    );

    // Stop -> archived; the archived record carries the same identity.
    service.sessions_stop(&id).expect("stop");
    let list = service.sessions_archive_list(None, None, None, None).expect("archive list");
    let found = list
        .entries
        .iter()
        .find(|e| archived_id(e) == id)
        .expect("stopped session is archived under its id");
    let got = service.sessions_archive_get(&archived_id(found)).expect("archive get");
    let entry = got.entry.expect("archived session present");
    assert_eq!(
        archived_digest(&entry),
        digest,
        "archived session preserves the model digest"
    );
}

// --- archive-record identity shims: match the id and the model digest without
// --- over-asserting the archived record's internal shape.

fn archived_id(e: &sysml_service::types::ArchivedSessionSummary) -> String {
    e.id.clone()
}
fn archived_digest(e: &sysml_service::types::ArchivedSession) -> String {
    format!("{e:?}")
        .split("model_digest")
        .nth(1)
        .and_then(|s| s.split('"').nth(1))
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// J3 SCENARIO-AT-CREATE (charter `core-mbse-loop.charter.yaml`, J3 precondition
// "a severe run is configured with restrictionConductance = 0.3" and invariant
// "a scenario/override is distinguishable from the underlying model source").
//
// The pre-existing RES-OVERRIDE test above configures severe by passing
// overrides to `sessions_step` — i.e. it starts a nominal session and alters a
// live run. That is what these gates replace: the scenario is chosen when the
// session is BUILT, so no tick ever ran under the wrong parameter.
// ---------------------------------------------------------------------------

/// The seed tick. `orchestrate_workspace_start` steps once as part of
/// construction, so the first snapshot any client can observe is tick 1 — and
/// create-time overrides must already be in force for it. There is no
/// caller-visible tick 0 snapshot to assert against; asserting on this one is
/// the strictest available statement of "before the first tick the user
/// advances".
const SEED_TICK: u64 = 1;

#[test]
fn j3_create_time_scenario_holds_from_the_first_tick() {
    let service = open();

    let sev = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), Some(&severe()))
        .expect("create the severe session");
    let nom = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), None)
        .expect("create the nominal session");

    // Scenario provenance rides on the summary, so a UI can say WHICH run this
    // is without re-deriving it from behaviour.
    assert_eq!(
        sev.create_overrides,
        vec![("restrictionConductance".to_string(), "0.3".to_string())],
    );
    assert!(
        nom.create_overrides.is_empty(),
        "an un-overridden run is the model's own declared baseline, not an empty override set          that happens to match it",
    );

    // The parameter itself, at the earliest observable point, with neither
    // session ever having been stepped by this test.
    let value_at_seed = |id: &str| -> f64 {
        let det = service
            .sessions_info(id, Some(true))
            .expect("info")
            .expect("session detail");
        let snap = det.latest_snapshot.expect("seed snapshot");
        assert_eq!(snap.tick, SEED_TICK, "no caller step has run yet");
        match snap
            .variables
            .get("restrictionConductance")
            .expect("restrictionConductance is a snapshot variable")
        {
            sysml_core::Value::Float(f) => *f,
            other => panic!("restrictionConductance should be numeric, got {other:?}"),
        }
    };
    assert_eq!(value_at_seed(&sev.id), 0.3, "severe scenario in force at the seed tick");
    assert_eq!(
        value_at_seed(&nom.id),
        1.5,
        "nominal is the model default from Physics/PumpODE.sysml, not a second override",
    );
}

#[test]
fn j3_create_time_severe_relieves_without_any_step_override() {
    let service = open();

    // The behavioural half: the ONLY difference between these two runs is what
    // they were built with. Neither `sessions_step` call passes an override, so
    // relief can only come from the create-time scenario.
    let sev = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), Some(&severe()))
        .expect("create severe");
    service
        .sessions_step(&sev.id, None, None, Some(2500))
        .expect("step severe");
    assert_eq!(
        pump_state(&service, &sev.id),
        "relieved",
        "a session BUILT severe latches relief with no step-time override",
    );

    let nom = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), None)
        .expect("create nominal");
    service
        .sessions_step(&nom.id, None, None, Some(2500))
        .expect("step nominal");
    assert_ne!(
        pump_state(&service, &nom.id),
        "relieved",
        "the baseline must not relieve, or the comparison proves nothing",
    );
}

#[test]
fn j3_create_time_overrides_fail_hard_rather_than_drop_silently() {
    let service = open();

    // Unknown target: caller input, so InvalidInput (400) — not a 500, and
    // never an accepted-then-ignored override.
    let typo = vec![("restrictionConductanceX".to_string(), "0.3".to_string())];
    let err = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), Some(&typo))
        .expect_err("an unknown override target must be refused");
    assert!(
        matches!(err, ServiceError::InvalidInput(_)),
        "expected InvalidInput for a mistyped scenario key, got {err:?}",
    );

    // Naming the state machine on THIS fixture does not select the single-SM
    // builder: the workspace has ODE dynamics, so `sessions_create` routes it
    // to the orchestrator (documented kind inference — an SM coupled to
    // continuous dynamics must advance in lockstep with it). So the scenario
    // is still honoured here, and this asserts that rather than a refusal.
    let named = service
        .sessions_create(WS, Some("PumpCycle"), Some(2.0), Some(8000.0), Some(&severe()))
        .expect("a coupled SM target still runs the orchestrator, which seeds overrides");
    assert_eq!(named.kind, SessionKind::Orchestrator);
    assert_eq!(
        named.create_overrides,
        vec![("restrictionConductance".to_string(), "0.3".to_string())],
    );
    // The genuine non-orchestrator refusal needs an ODE-free workspace; it is
    // gated in `contract_sessions_create_overrides.rs`.
}

#[test]
fn j3_create_time_scenario_reaches_the_archive() {
    let service = open();
    let sev = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), Some(&severe()))
        .expect("create severe");
    service.sessions_step(&sev.id, None, None, Some(10)).expect("step");
    service.sessions_stop(&sev.id).expect("stop archives the run");

    let archived = service
        .sessions_archive_get(&sev.id)
        .expect("archive get")
        .entry
        .expect("the stopped session is archived");
    // `ArchivedSession::overrides` is documented as "overrides applied at
    // session start" and had no producer until create-time overrides existed.
    assert_eq!(
        archived.overrides,
        vec![("restrictionConductance".to_string(), "0.3".to_string())],
        "a stored run must carry the scenario it was run under, or the evidence is unreadable later",
    );
}

// ---------------------------------------------------------------------------
// J5 EVIDENCE, END TO END — `sessions.verify` on a live session must land the
// session id and tick in `verify.latest_status`, which is what the case view's
// primary run card reads.
//
// Every evidence gate in `contract_executions.rs` seeds the archive by hand,
// so none of them exercises the real mint path or the wire the UI consumes.
// This one runs a session, verifies it, and reads the projection back.
// ---------------------------------------------------------------------------

#[test]
fn j5_sessions_verify_evidence_reaches_latest_status() {
    let service = open();
    let created = service
        .sessions_create(WS, None, Some(2.0), Some(8000.0), None)
        .expect("create the pump session");
    let stepped = service
        .sessions_step(&created.id, None, None, Some(50))
        .expect("advance the session");
    let tick_at_verify = stepped.tick;

    let results = service
        .sessions_verify(&created.id, None)
        .expect("verify the live session");
    assert!(!results.is_empty(), "the pump fixture declares verification cases");

    let latest = execute_command(&service, "sysml.verify.latest_status", json!({}))
        .expect("verify.latest_status");
    let cases = latest["cases"].as_array().expect("cases array");
    let trajectories: Vec<_> = cases
        .iter()
        .filter_map(|c| c["latest"]["trajectory"].as_object())
        .collect();
    assert!(
        !trajectories.is_empty(),
        "sessions.verify produced no trajectory entry in latest_status"
    );

    for traj in trajectories {
        let evidence = traj
            .get("evidence")
            .expect("the evidence key is always serialized")
            .as_object()
            .expect("a freshly minted trajectory verdict carries its run, not null");
        assert_eq!(
            evidence["session_id"].as_str(),
            Some(created.id.as_str()),
            "the run's own session id, never a placeholder"
        );
        assert_eq!(
            evidence["tick"].as_u64(),
            Some(tick_at_verify),
            "the tick the verdict was evaluated at"
        );
        // Simulated time is RECORDED, not derived. dt is 2.0 ms here, so the
        // model clock and the tick count are different numbers — which is the
        // point: a reader cannot recover one from the other without knowing
        // dt, and would be wrong anyway for a variable-step or resumed run.
        let time_ms = evidence["time_ms"]
            .as_f64()
            .expect("a freshly minted verdict carries the model clock");
        assert!(
            (time_ms - stepped.time_ms).abs() < f64::EPSILON,
            "evidence time {time_ms} must equal the session's own clock {}",
            stepped.time_ms
        );
        assert!(
            (time_ms - tick_at_verify as f64).abs() > 1.0,
            "at dt=2ms the clock must not coincide with the tick count, or this \
             assertion proves nothing: time={time_ms} tick={tick_at_verify}"
        );
    }
}
