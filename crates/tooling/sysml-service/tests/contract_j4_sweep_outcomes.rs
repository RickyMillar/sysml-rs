//! J4 follow-up: a sweep measures what the study asked it to measure, and
//! archiving a child costs what one child is worth.
//!
//! Two linked defects, both found on `examples/radiation-cooling`.
//!
//! **A — selected outcomes had no consumer.** A study could select
//! `temperature` as an outcome; the chip lit, the store recorded it, and then
//! nothing happened. `batch.create` never carried the selection, no child ever
//! measured it, and the result surfaces had nowhere to show it. The evidence
//! campaign could truthfully report "5 children complete" and could not
//! produce a single temperature value.
//!
//! **B — the archive cost gigabytes per child.** `ExecutionSnapshot::
//! value_units` is a tick-INVARIANT table the runtime deliberately `Arc`-
//! shares into every snapshot. Serialising snapshots one at a time undid that
//! sharing and wrote a full copy per tick. On this fixture — six attributes,
//! a 453-entry table that is 97% of each snapshot — one `sessions.stop`
//! retained ~1.2 GB and never released it, because the archive is in-memory.
//! A 25-child sweep therefore asked for ~30 GB and was OOM-killed: the
//! "hung for a while, then crashed" report.
//!
//! These tests drive the same sequence `useSweepRunner` drives, through
//! `execute_command` — the dispatch the HTTP transport uses.
//!
//! What they do NOT assert: that the physics is right. The fixture is
//! synthetic. The claims here are that a requested outcome arrives attached to
//! the child that produced it, that an outcome which cannot be read says so
//! rather than reading as zero, and that the archive stores one copy of a
//! constant instead of one per tick.

use serde_json::{json, Value};
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn radiation_cooling() -> SysmlService {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/radiation-cooling");
    let service = SysmlService::empty();
    service
        .load_workspace(&root)
        .unwrap_or_else(|e| panic!("load radiation-cooling: {e}"));
    service
}

/// `sysml.batch.create` with the sweep's wire shape: `children_params` and
/// `outcomes` are both JSON-ENCODED STRINGS, matching what `useSweepRunner`
/// sends over the HTTP bridge.
fn create_sweep(service: &SysmlService, points: &[Value], outcomes: &[&str]) -> Value {
    execute_command(
        service,
        "sysml.batch.create",
        json!({
            "kind": "sweep",
            "uri": "__workspace__",
            "children_params": serde_json::to_string(points).unwrap(),
            "outcomes": serde_json::to_string(outcomes).unwrap(),
            "label": "j4 outcome regression",
        }),
    )
    .unwrap_or_else(|e| panic!("batch.create: {e}"))
}

fn child_ids(created: &Value) -> Vec<String> {
    created["child_session_ids"]
        .as_array()
        .expect("child_session_ids array")
        .iter()
        .map(|v| v.as_str().expect("child id is a string").to_owned())
        .collect()
}

/// Drive one child the way the runner does: bulk-step, verify, stop.
fn drive_child(service: &SysmlService, session_id: &str, ticks: u64) {
    execute_command(
        service,
        "sysml.sessions.step",
        json!({ "session_id": session_id, "ticks": ticks }),
    )
    .unwrap_or_else(|e| panic!("step {session_id}: {e}"));
    let _ = execute_command(
        service,
        "sysml.sessions.verify",
        json!({ "session_id": session_id }),
    );
    execute_command(
        service,
        "sysml.sessions.stop",
        json!({ "session_id": session_id }),
    )
    .unwrap_or_else(|e| panic!("stop {session_id}: {e}"));
}

fn batch_status(service: &SysmlService, batch_id: &str) -> Value {
    execute_command(service, "sysml.batch.status", json!({ "batch_id": batch_id }))
        .unwrap_or_else(|e| panic!("batch.status: {e}"))
}

/// The five-point `ambientTemp` study from the evidence campaign.
fn five_point_study() -> Vec<Value> {
    [250.0, 275.0, 300.0, 325.0, 350.0]
        .iter()
        .map(|t| json!({ "ambientTemp": t }))
        .collect()
}

const HORIZON: u64 = 500;

// ---------------------------------------------------------------------------
// A — a selected outcome reaches the result contract
// ---------------------------------------------------------------------------

#[test]
fn a_selected_outcome_is_captured_on_every_child() {
    let service = radiation_cooling();
    let points = five_point_study();
    let created = create_sweep(&service, &points, &["temperature"]);
    let batch_id = created["batch_id"].as_str().expect("batch_id").to_owned();
    let ids = child_ids(&created);
    assert_eq!(ids.len(), 5, "the study's five points must all spawn");

    for id in &ids {
        drive_child(&service, id, HORIZON);
    }

    let status = batch_status(&service, &batch_id);
    let children = status["batch"]["children"]
        .as_array()
        .expect("children array");
    assert_eq!(children.len(), 5);

    // The batch remembers what it was asked to measure.
    assert_eq!(
        status["batch"]["outcomes"],
        json!(["temperature"]),
        "the batch must carry the study's outcome list",
    );

    let mut values = Vec::new();
    for child in children {
        let reading = &child["outcomes"]["temperature"];
        assert!(
            !reading.is_null(),
            "child {} carries no temperature reading: {child}",
            child["index"],
        );
        let value = reading["value"].as_f64().unwrap_or_else(|| {
            panic!("temperature unreadable on child {}: {reading}", child["index"])
        });
        assert!(value.is_finite(), "temperature must be finite, got {value}");
        // Sampled at the end of the run, not at seed.
        assert!(
            reading["time_ms"].as_f64().unwrap_or(0.0) > 0.0,
            "reading must record when it was sampled",
        );
        values.push(value);
    }

    // Five ATTRIBUTABLE values: each child reports its own, not one number
    // repeated across the batch (the failure mode a broadcast bug produces).
    assert_eq!(values.len(), 5);
    let distinct: std::collections::BTreeSet<String> =
        values.iter().map(|v| format!("{v:.9}")).collect();
    assert_eq!(
        distinct.len(),
        5,
        "each swept point must produce its own temperature, got {values:?}",
    );
}

#[test]
fn an_unreadable_outcome_reports_why_instead_of_zero() {
    // The difference between "this run never produced that variable" and
    // "this run produced 0.0" is the difference between an empty cell and a
    // fabricated data point.
    let service = radiation_cooling();
    let created = create_sweep(
        &service,
        &[json!({ "ambientTemp": 300.0 })],
        &["temperature", "noSuchVariable"],
    );
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    for id in child_ids(&created) {
        drive_child(&service, &id, HORIZON);
    }

    let status = batch_status(&service, &batch_id);
    let outcomes = &status["batch"]["children"][0]["outcomes"];

    // The readable one is read.
    assert!(outcomes["temperature"]["value"].as_f64().is_some());

    // The unreadable one is PRESENT — omitting it would be indistinguishable
    // from "never asked for" — with a reason and no value.
    let missing = &outcomes["noSuchVariable"];
    assert!(
        !missing.is_null(),
        "a requested-but-unreadable outcome must still appear: {outcomes}",
    );
    assert!(
        missing["value"].is_null(),
        "an unreadable outcome must not carry a value, got {missing}",
    );
    assert_ne!(missing["value"], json!(0.0), "must never degrade to zero");
    let error = missing["error"].as_str().expect("an unreadable outcome names why");
    assert!(
        error.contains("noSuchVariable"),
        "the reason must name the variable, got {error:?}",
    );
}

#[test]
fn a_batch_that_requested_no_outcomes_carries_none() {
    // The common case stays free: no outcomes requested, no per-child cost,
    // nothing added to the wire.
    let service = radiation_cooling();
    let created = create_sweep(&service, &[json!({ "ambientTemp": 300.0 })], &[]);
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    for id in child_ids(&created) {
        drive_child(&service, &id, HORIZON);
    }
    let status = batch_status(&service, &batch_id);
    let child = &status["batch"]["children"][0];
    assert!(
        child["outcomes"].is_null() || child["outcomes"] == json!({}),
        "no outcomes requested must mean no outcomes reported, got {}",
        child["outcomes"],
    );
    // The child still ran and still reports coherently.
    assert_eq!(child["status"]["status"], json!("complete"));
}

#[test]
fn outcomes_survive_a_multi_factor_grid() {
    // The two-factor study from the crash report, at a small horizon: every
    // combination runs and every one reports its own measurement.
    let service = radiation_cooling();
    let mut points = Vec::new();
    for ambient in [250.0_f64, 300.0, 350.0] {
        for emissivity in [0.5_f64, 0.7, 0.9] {
            points.push(json!({ "ambientTemp": ambient, "emissivity": emissivity }));
        }
    }
    let created = create_sweep(&service, &points, &["temperature"]);
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    let ids = child_ids(&created);
    assert_eq!(ids.len(), 9, "3 x 3 must expand to nine children");
    for id in &ids {
        drive_child(&service, id, HORIZON);
    }

    let status = batch_status(&service, &batch_id);
    let children = status["batch"]["children"].as_array().unwrap();
    assert_eq!(children.len(), 9, "no child may be dropped");
    for child in children {
        assert_eq!(
            child["status"]["status"],
            json!("complete"),
            "child {} did not complete",
            child["index"],
        );
        assert!(
            child["outcomes"]["temperature"]["value"].as_f64().is_some(),
            "child {} lost its measurement",
            child["index"],
        );
        // Both factors are still attached, so a value is attributable.
        assert!(child["params"]["ambientTemp"].is_number());
        assert!(child["params"]["emissivity"].is_number());
    }
}

// ---------------------------------------------------------------------------
// B — archiving a child does not duplicate a per-run constant per tick
// ---------------------------------------------------------------------------

#[test]
fn the_archive_stores_the_measurement_table_once_not_once_per_tick() {
    // `value_units` is fixed for a run. Before this fix every archived
    // snapshot carried its own full copy, so the record grew with the tick
    // count times the model's slot count — the memory blowup that OOM-killed
    // a 25-child sweep.
    let service = radiation_cooling();
    let created = create_sweep(&service, &[json!({ "ambientTemp": 300.0 })], &[]);
    let ids = child_ids(&created);
    let session_id = ids[0].clone();
    drive_child(&service, &session_id, HORIZON);

    let entry = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": session_id }),
    )
    .unwrap_or_else(|e| panic!("archive.get: {e}"));
    let record = &entry["entry"];

    let snapshots = record["snapshots"].as_array().expect("archived snapshots");
    assert!(
        snapshots.len() > 1,
        "the run must have archived a history to make this meaningful",
    );

    // No snapshot carries the table...
    for (i, snap) in snapshots.iter().enumerate() {
        assert!(
            snap.get("value_units").is_none(),
            "snapshot {i} still carries a per-tick copy of value_units",
        );
    }

    // ...and the record carries exactly one, so nothing was lost.
    let hoisted = record["snapshot_value_units"]
        .as_object()
        .expect("the record must hold the run's measurement table once");
    assert!(
        !hoisted.is_empty(),
        "this fixture's slots do carry measurement metadata; an empty table \
         would mean the hoist dropped it rather than moved it",
    );

    // The tick trace itself is intact — this is a de-duplication, not a
    // truncation.
    assert!(snapshots.iter().all(|s| s.get("tick").is_some()));
}

// ---------------------------------------------------------------------------
// C — a sweep child can reach its model's own timescale
// ---------------------------------------------------------------------------

/// `sysml.batch.create` with an explicit simulation step and time budget.
fn create_timed_sweep(
    service: &SysmlService,
    points: &[Value],
    outcomes: &[&str],
    dt_ms: f64,
    max_time_ms: f64,
) -> Value {
    execute_command(
        service,
        "sysml.batch.create",
        json!({
            "kind": "sweep",
            "uri": "__workspace__",
            "children_params": serde_json::to_string(points).unwrap(),
            "outcomes": serde_json::to_string(outcomes).unwrap(),
            "dt_ms": dt_ms,
            "max_time_ms": max_time_ms,
            "label": "j4 horizon regression",
        }),
    )
    .unwrap_or_else(|e| panic!("batch.create: {e}"))
}

/// Ticks a single `sessions.step` actually advanced, which is not necessarily
/// the number requested.
fn step_advanced(service: &SysmlService, session_id: &str, ticks: u64) -> u64 {
    let result = execute_command(
        service,
        "sysml.sessions.step",
        json!({ "session_id": session_id, "ticks": ticks }),
    )
    .unwrap_or_else(|e| panic!("step: {e}"));
    result["ticks_advanced"].as_u64().unwrap_or(0)
}

#[test]
fn a_child_stops_at_its_time_budget_without_raising_an_error() {
    // The shortfall this pins is invisible from the outside: no error, the
    // child still archives as `complete`, and the outcome still reads a
    // plausible number. `ticks_advanced` is the ONLY signal, which is why the
    // runner has to look at it.
    let service = radiation_cooling();
    // 1 ms step, 5 s budget — then ask for 20 s of ticks.
    let created = create_timed_sweep(
        &service,
        &[json!({ "ambientTemp": 300.0 })],
        &["temperature"],
        1.0,
        5_000.0,
    );
    let session_id = child_ids(&created)[0].clone();

    let advanced = step_advanced(&service, &session_id, 20_000);
    assert!(
        advanced < 20_000,
        "the child should have been stopped by its 5 s budget, advanced {advanced}",
    );
    assert!(
        advanced >= 5_000,
        "it should still have run up TO the budget, advanced only {advanced}",
    );

    // A further step barely moves: the run is against the wall, silently.
    let again = step_advanced(&service, &session_id, 20_000);
    assert!(again <= 1, "a run at its budget should not keep advancing, got {again}");
}

#[test]
fn a_coarser_step_lets_a_slow_model_reach_its_own_timescale() {
    // `RadiationCooling`'s doc says the body cools from 1000 K to near the
    // 300 K ambient over ~2000 s. At the default 1 ms step that is 2,000,000
    // ticks, far past any horizon the study surface can express, so every
    // reading was taken in the first fraction of a percent of the transient.
    //
    // The step size is what makes the timescale reachable — the same tick
    // count at 100 ms covers 2000 s.
    let service = radiation_cooling();
    let created = create_timed_sweep(
        &service,
        &[json!({ "ambientTemp": 300.0 })],
        &["temperature"],
        100.0,
        2_000_000.0,
    );
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    let session_id = child_ids(&created)[0].clone();

    let advanced = step_advanced(&service, &session_id, 20_000);
    assert_eq!(
        advanced, 20_000,
        "with a budget sized to the horizon the child must run all of it",
    );

    let _ = execute_command(&service, "sysml.sessions.verify", json!({ "session_id": session_id }));
    execute_command(&service, "sysml.sessions.stop", json!({ "session_id": session_id }))
        .unwrap_or_else(|e| panic!("stop: {e}"));

    let status = batch_status(&service, &batch_id);
    let reading = &status["batch"]["children"][0]["outcomes"]["temperature"];
    let temperature = reading["value"].as_f64().expect("a temperature reading");
    let time_ms = reading["time_ms"].as_f64().expect("a sample time");

    assert!(
        (time_ms - 2_000_000.0).abs() < 1_000.0,
        "the run should have reached ~2000 s of model time, reached {time_ms} ms",
    );
    // The assertion that matters: the body actually cooled, rather than
    // sitting at its 1000 K initial value the way every prior sweep reported.
    assert!(
        temperature < 400.0,
        "after ~2000 s the body should be near the 300 K ambient, got {temperature} K",
    );
    assert!(
        temperature > 300.0,
        "it cannot cool below ambient, got {temperature} K",
    );
}

#[test]
fn the_default_timing_is_unchanged_when_no_step_is_given() {
    // Threading the parameter must not move the default out from under
    // callers that never asked for one.
    let service = radiation_cooling();
    let created = create_sweep(&service, &[json!({ "ambientTemp": 300.0 })], &["temperature"]);
    let session_id = child_ids(&created)[0].clone();
    let advanced = step_advanced(&service, &session_id, 1_000);
    assert_eq!(advanced, 1_000);
    let info = execute_command(&service, "sysml.sessions.info", json!({ "session_id": session_id }))
        .unwrap_or_else(|e| panic!("info: {e}"));
    assert_eq!(
        info["latest_snapshot"]["time_ms"].as_f64(),
        Some(1_000.0),
        "the default step is still 1 ms, so 1000 ticks is still 1000 ms",
    );
}

// ---------------------------------------------------------------------------
// D — the shape behind the number
// ---------------------------------------------------------------------------

#[test]
fn a_captured_outcome_keeps_the_trace_that_produced_it() {
    // The final value cannot say whether a run settled or stopped part-way.
    // This fixture reported ~990 K across a whole study while barely having
    // started, and nothing on the result surface could tell the difference.
    let service = radiation_cooling();
    let created = create_timed_sweep(
        &service,
        &[json!({ "emissivity": 0.9 })],
        &["temperature"],
        100.0,
        2_000_000.0,
    );
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    let session_id = child_ids(&created)[0].clone();
    step_advanced(&service, &session_id, 20_000);
    execute_command(&service, "sysml.sessions.stop", json!({ "session_id": session_id }))
        .unwrap_or_else(|e| panic!("stop: {e}"));

    let status = batch_status(&service, &batch_id);
    let reading = &status["batch"]["children"][0]["outcomes"]["temperature"];
    let series = reading["series"].as_array().expect("a retained trace");

    assert!(series.len() > 2, "a 20,000-tick run must retain a shape, got {}", series.len());
    assert!(
        series.len() <= 200,
        "the trace must be decimated, got {} points",
        series.len(),
    );

    // Oldest first, monotonically increasing in time.
    let times: Vec<f64> = series.iter().map(|p| p[0].as_f64().unwrap()).collect();
    assert!(
        times.windows(2).all(|w| w[1] >= w[0]),
        "the trace must be ordered oldest-first",
    );

    // It spans the RUN, not just its tail — this is the property the archived
    // snapshot history cannot provide, because MAX_HISTORY keeps only the
    // last 1000 ticks (here, the final 5% of the run).
    assert!(
        times[0] < 10_000.0,
        "the trace must start near the beginning of the run, started at {} ms",
        times[0],
    );
    assert!(
        *times.last().unwrap() > 1_900_000.0,
        "and reach the end, ended at {} ms",
        times.last().unwrap(),
    );

    // The shape is the cooling curve: it starts hot and ends where the
    // reported value is.
    let values: Vec<f64> = series.iter().map(|p| p[1].as_f64().unwrap()).collect();
    assert!(values[0] > 900.0, "should start near the 1000 K initial, got {}", values[0]);
    let reported = reading["value"].as_f64().unwrap();
    assert!(
        (values.last().unwrap() - reported).abs() < 1.0,
        "the trace must end at the value the outcome reports: {} vs {reported}",
        values.last().unwrap(),
    );
}

#[test]
fn an_unreadable_outcome_carries_no_trace() {
    let service = radiation_cooling();
    let created = create_sweep(
        &service,
        &[json!({ "ambientTemp": 300.0 })],
        &["temperature", "noSuchVariable"],
    );
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    for id in child_ids(&created) {
        drive_child(&service, &id, HORIZON);
    }
    let status = batch_status(&service, &batch_id);
    let outcomes = &status["batch"]["children"][0]["outcomes"];
    assert!(
        outcomes["noSuchVariable"]["series"].is_null(),
        "an outcome that could not be read has no shape to show",
    );
    // The readable sibling still has one.
    assert!(outcomes["temperature"]["series"].as_array().is_some_and(|s| !s.is_empty()));
}

// ---------------------------------------------------------------------------
// E — an archived sweep child says which point of the study it was
// ---------------------------------------------------------------------------

#[test]
fn an_archived_child_records_the_point_it_ran() {
    // Two records exist for a finished sweep child, with different lifetimes:
    // the batch descriptor (params + outcomes + traces, held for the life of
    // the process, and NOT enumerable — there is no `batch.list`), and the
    // archive entry. The archive is the durable one, and it was storing a run
    // with no statement of what was varied to produce it, which makes an
    // archived sweep child anonymous the moment its batch id is lost.
    let service = radiation_cooling();
    let created = create_sweep(
        &service,
        &[json!({ "emissivity": 0.5 }), json!({ "emissivity": 0.9 })],
        &["temperature"],
    );
    let ids = child_ids(&created);
    for id in &ids {
        drive_child(&service, id, HORIZON);
    }

    let mut seen: Vec<String> = Vec::new();
    for id in &ids {
        let entry = execute_command(
            &service,
            "sysml.sessions.archive.get",
            json!({ "id": id }),
        )
        .unwrap_or_else(|e| panic!("archive.get: {e}"));
        let overrides = entry["entry"]["overrides"]
            .as_array()
            .expect("an archived child records its overrides");
        assert!(
            !overrides.is_empty(),
            "child {id} archived with no record of which point it ran",
        );
        // `(name, value)` pairs, so the point is reconstructable.
        let pair = overrides[0].as_array().expect("(name, value) pair");
        assert_eq!(pair[0].as_str(), Some("emissivity"));
        seen.push(pair[1].as_str().unwrap_or_default().to_owned());
    }

    seen.sort();
    assert_eq!(
        seen,
        vec!["0.5".to_owned(), "0.9".to_owned()],
        "each child must record ITS OWN point, not a shared one",
    );
}
