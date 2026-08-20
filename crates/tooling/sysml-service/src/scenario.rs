//! Scenario execution — composition of the orchestrator + state-machine
//! compiler + verification-case runner under one auto-step loop.
//!
//! This is the largest single transport-bypass primitive in Bucket B:
//! the LSP previously open-coded ~250 LOC of orchestrator composition
//! (build + SM compile + event-script extraction + auto-step + per-tick
//! assertion eval) inside `handle_scenario_run`. Moved here so every
//! transport (CLI / LSP / REST / MCP) shares one implementation.
//!
//! The two encoder helpers (`snapshot_to_json`,
//! `evaluate_requirements_at_tick`) also live here because their JSON
//! shape is the wire contract for `sysml.scenario.run` and the
//! `sysml.timeline.*` adjacent commands.

use sysml_core::ModelGraph;
use sysml_runtime::cases::compile_verification_case;
use sysml_runtime::compiler::{context_from_graph, ModelCompiler};
use sysml_runtime::orchestrator::{AssertionCheckpoint, ExecutionSnapshot};

use crate::error::ServiceError;
use sysml_store::EvaluationMode;

/// Run a verification scenario end-to-end against the elaborated
/// workspace graph for `case_name` and return the JSON return shape the
/// LSP `handle_scenario_run` emits today:
///
///   {
///     "evaluation_mode":       "trajectory"  // §2.1a(d): the verdict mode label
///     "verdict":               string  // VerdictKind Display: pass | fail | inconclusive | error
///     "requirement_results":   [ { "requirement_id", "verdict", "message" } ],
///     "assertion_checkpoints": [ { "tick", "time_ms", "requirement_id",
///                                  "requirement_text", "verdict", "message",
///                                  "referenced_variables" } ],
///     "trace":                 [ TickSnapshot, ... ],   // snapshot_to_json shape
///     "final_snapshot":        TickSnapshot | {}        // empty when trace empty
///   }
///
/// `max_ticks` is a hard fail-stop on the auto-step loop. `None` uses
/// the `OrchestratorConfig` default.
pub(crate) fn run_scenario(
    graph: &ModelGraph,
    case_name: &str,
    max_ticks: Option<u32>,
) -> Result<serde_json::Value, ServiceError> {
    // Build the orchestrator through the unified compiler path (ledger F1).
    //
    // This previously hand-rolled `Orchestrator::new` + `add_state_machine`
    // in a loop and inlined `StateMachineCompiler::compile_named` — a
    // documented invariant violation (never inline
    // elaborate+compile_named in a command) that also skipped
    // `ModelCompiler::mint_slot_store`/`bind_expression_slots`, so a
    // transition-effect attribute assignment had no slot-routed writeback and
    // was silently dropped from every snapshot (ledger L44's symptom, on the
    // scenario path). `build_workspace_orchestrator` discovers every state
    // machine (and ODE), runs the full mint/bind/RS003-4-5 gate, and is the
    // exact path production simulation uses. Steward ruling (option b): no
    // third builder — `run_scenario` is not hot, so the extra whole-graph
    // discovery cost is acceptable (perf follow-up = ledger item if it bites).
    let compiler = ModelCompiler::from_arc(std::sync::Arc::new(graph.clone()));
    // Single graph provenance from here on: the compiler's elaborated graph
    // drives the event-script extraction and verification-case compile below,
    // not the caller's separately-held reference.
    let graph = std::sync::Arc::clone(compiler.graph());
    let base_ctx = context_from_graph(&graph);

    let mut orchestrator = compiler
        .build_workspace_orchestrator(base_ctx, None, None, None, None, &[], None, None)
        .map_err(|e| ServiceError::Execution(e.message))?;

    if orchestrator.subsystem_names().is_empty() {
        return Err(ServiceError::Execution(
            "no state machines found in document".to_owned(),
        ));
    }

    // `build_workspace_orchestrator` uses dt_ms = 1.0 by default; keep it in
    // sync here for the event-script cadence and the auto-step loop bound.
    // `max_ticks` overrides the hard fail-stop (default 10_000, matching the
    // former `OrchestratorConfig::default().max_ticks`).
    let dt_ms = 1.0_f64;
    let max_ticks_cap: u64 = max_ticks.map(|mt| mt as u64).unwrap_or(10_000);

    // Extract scripted events from the verification case
    let scripted_events =
        sysml_runtime::cases::extract_event_script(case_name, &graph, dt_ms).map_err(|diags| {
            let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
            ServiceError::Execution(msgs.join("; "))
        })?;

    if scripted_events.is_empty() {
        return Err(ServiceError::Execution(
            "no events found in verification case test script".to_owned(),
        ));
    }

    // Schedule each scripted event to ALL subsystems (broadcast).
    // State machines ignore events they don't handle, so this is safe.
    let subsystem_names = orchestrator.subsystem_names();
    for event in &scripted_events {
        for subsystem_name in &subsystem_names {
            orchestrator.schedule_event(event.delay_ms, subsystem_name, &event.event);
        }
    }

    // Determine the last event time for the auto-step loop boundary
    let last_event_time = scripted_events
        .iter()
        .map(|e| e.delay_ms)
        .fold(0.0_f64, f64::max);
    let buffer_ticks = 10;
    let max_time = last_event_time + (buffer_ticks as f64 * dt_ms);

    // Pre-compile verification requirements for per-tick assertion evaluation
    let requirements = match compile_verification_case(case_name, &graph) {
        Ok(case_ir) => case_ir.requirements.clone(),
        Err(_) => Vec::new(),
    };
    let mut all_assertion_checkpoints: Vec<serde_json::Value> = Vec::new();

    // Auto-step loop: run until completion or past all events + buffer
    let mut trace: Vec<serde_json::Value> = Vec::new();
    while orchestrator.tick() < max_ticks_cap
        && orchestrator.time_ms() < max_time
        && !orchestrator.is_completed()
    {
        let snapshot = orchestrator.step();
        let tmp_ctx = {
            let mut c = sysml_runtime::expressions::EvalContext::new();
            for (k, v) in snapshot.variables.iter() {
                c.set(k.clone(), v.clone());
            }
            c
        };
        let checkpoints = evaluate_requirements_at_tick(
            &requirements,
            &tmp_ctx,
            snapshot.tick,
            snapshot.time_ms,
        );
        for cp in &checkpoints {
            all_assertion_checkpoints.push(serde_json::json!({
                "tick": cp.tick,
                "time_ms": cp.time_ms,
                "requirement_id": cp.requirement_id,
                "requirement_text": cp.requirement_text,
                "verdict": format!("{}", cp.verdict),
                "message": cp.message,
                "referenced_variables": cp.referenced_variables,
            }));
        }
        trace.push(snapshot_to_json(&snapshot));
    }

    // If not completed after the event buffer, step a few more times
    let extra_ticks = 10u64;
    let mut extra = 0u64;
    while !orchestrator.is_completed()
        && extra < extra_ticks
        && orchestrator.tick() < max_ticks_cap
    {
        let snapshot = orchestrator.step();
        let tmp_ctx = {
            let mut c = sysml_runtime::expressions::EvalContext::new();
            for (k, v) in snapshot.variables.iter() {
                c.set(k.clone(), v.clone());
            }
            c
        };
        let checkpoints = evaluate_requirements_at_tick(
            &requirements,
            &tmp_ctx,
            snapshot.tick,
            snapshot.time_ms,
        );
        for cp in &checkpoints {
            all_assertion_checkpoints.push(serde_json::json!({
                "tick": cp.tick,
                "time_ms": cp.time_ms,
                "requirement_id": cp.requirement_id,
                "requirement_text": cp.requirement_text,
                "verdict": format!("{}", cp.verdict),
                "message": cp.message,
                "referenced_variables": cp.referenced_variables,
            }));
        }
        trace.push(snapshot_to_json(&snapshot));
        extra += 1;
    }

    // Get final snapshot
    let final_snapshot = trace.last().cloned().unwrap_or_else(|| serde_json::json!({}));

    // Wire trace into context for temporal query functions.
    // EvalContext.trace uses the light snapshot type in sysml_runtime::expressions;
    // convert runtime's richer ExecutionSnapshot at this orchestration boundary.
    let tick_snapshots: Vec<sysml_runtime::expressions::TickSnapshot> = orchestrator
        .trace()
        .iter()
        .map(|snap| sysml_runtime::expressions::TickSnapshot {
            tick: snap.tick,
            time_ms: snap.time_ms,
            variables: (*snap.variables).clone(),
            subsystem_states: snap
                .subsystem_states
                .iter()
                .map(|(k, s)| {
                    (
                        k.clone(),
                        sysml_runtime::expressions::SubsystemState {
                            name: s.name.clone(),
                            kind: s.kind,
                            current_state: s.current_state.clone(),
                            completed: s.completed,
                            available_transitions: s.available_transitions.clone(),
                            outputs: s.outputs.clone(),
                            sends: s.sends.clone(),
                            active_modes: vec![],
                            variables: std::collections::HashMap::new(),
                            deferred_event_count: s.deferred_event_count,
                            source_element_id: s.source_element_id.clone(),
                        },
                    )
                })
                .collect(),
        })
        .collect();
    orchestrator.context.trace = Some(std::sync::Arc::new(tick_snapshots));

    // Compile and run verification
    let (verdict_str, requirement_results) = match compile_verification_case(case_name, &graph) {
        Ok(case_ir) => {
            let runner = sysml_runtime::cases::VerificationRunner::new();
            let result = runner.verify(&case_ir, &orchestrator.context);

            let req_results: Vec<serde_json::Value> = result
                .requirement_results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "requirement_id": r.requirement_id,
                        "verdict": format!("{}", r.verdict),
                        "message": r.message,
                    })
                })
                .collect();

            (format!("{}", result.verdict), req_results)
        }
        Err(diags) => {
            // Verification compilation failed — still return trace.
            let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
            tracing::warn!(errors = ?msgs, "verification case compilation failed");
            ("inconclusive".to_owned(), Vec::new())
        }
    };

    Ok(serde_json::json!({
        // Trajectory-mode verdict: event-script + auto-step + per-tick
        // evaluation over a live run (§2.1a(d) "rendered ALWAYS"; study §3.4).
        "evaluation_mode": EvaluationMode::Trajectory.as_str(),
        "verdict": verdict_str,
        "requirement_results": requirement_results,
        "assertion_checkpoints": all_assertion_checkpoints,
        "trace": trace,
        "final_snapshot": final_snapshot,
    }))
}

/// Evaluate every requirement against the per-tick context and return an
/// `AssertionCheckpoint` for each. Lives here (not in the runtime) because
/// it is the wire-format glue for `sysml.scenario.run`; the runtime owns
/// `RequirementCheck` + `VerificationRunner` + `AssertionCheckpoint`.
///
/// Skips any requirement whose constraint references a variable not
/// present in `ctx` — matches the LSP handler's pre-flight gate so a
/// missing tick variable does not produce a spurious Inconclusive verdict
/// per-tick.
fn evaluate_requirements_at_tick(
    requirements: &[sysml_runtime::cases::RequirementCheck],
    ctx: &sysml_runtime::expressions::EvalContext,
    tick: u64,
    time_ms: f64,
) -> Vec<AssertionCheckpoint> {
    let mut checkpoints = Vec::new();
    let runner = sysml_runtime::cases::VerificationRunner::new();

    for req in requirements {
        let all_vars_present = req
            .constraints
            .iter()
            .all(|c| c.free_variables().iter().all(|v| ctx.get(v).is_some()));
        if !all_vars_present {
            continue;
        }

        let result = runner.check_requirement(req, ctx);

        let referenced_vars: Vec<String> = req
            .constraints
            .iter()
            .flat_map(|c| c.free_variables())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        checkpoints.push(AssertionCheckpoint {
            tick,
            time_ms,
            requirement_id: result.requirement_id.clone(),
            requirement_text: req.text.clone(),
            verdict: result.verdict,
            message: result.message.clone(),
            referenced_variables: referenced_vars,
        });
    }
    checkpoints
}

/// Serialize an `ExecutionSnapshot` to the JSON shape every transport
/// consumes today. Wire contract: see `sysml.scenario.run` /
/// `sysml.timeline.*`.
pub(crate) fn snapshot_to_json(snapshot: &ExecutionSnapshot) -> serde_json::Value {
    let subsystems: Vec<serde_json::Value> = snapshot
        .subsystem_states
        .iter()
        .map(|(name, state)| {
            serde_json::json!({
                "name": name,
                "kind": state.kind,
                "state": state.current_state,
                "completed": state.completed,
                "outputs": state.outputs,
                "sends": state.sends,
                "available_transitions": state.available_transitions.iter().map(|(ev, tgt)| {
                    serde_json::json!({"event": ev, "target": tgt})
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    let context: serde_json::Map<String, serde_json::Value> = snapshot
        .variables
        .iter()
        .map(|(k, v)| {
            let json_val = match v {
                sysml_core::Value::Int(i) => serde_json::json!(i),
                sysml_core::Value::Float(f) => serde_json::json!(f),
                sysml_core::Value::Bool(b) => serde_json::json!(b),
                sysml_core::Value::String(s) => serde_json::json!(s),
                _ => serde_json::json!(format!("{:?}", v)),
            };
            (k.clone(), json_val)
        })
        .collect();

    serde_json::json!({
        "tick": snapshot.tick,
        "time_ms": snapshot.time_ms,
        "subsystems": subsystems,
        "completed": snapshot.completed,
        "context": context,
        "constraint_results": snapshot.constraint_results.iter().map(|cr| serde_json::json!({
            "name": cr.name,
            "verdict": cr.verdict.to_string(),
            "expression": cr.expression,
        })).collect::<Vec<_>>(),
        "assertion_checkpoints": snapshot.assertion_checkpoints.iter().map(|cp| serde_json::json!({
            "tick": cp.tick,
            "time_ms": cp.time_ms,
            "requirement_id": cp.requirement_id,
            "requirement_text": cp.requirement_text,
            "verdict": format!("{}", cp.verdict),
            "message": cp.message,
            "referenced_variables": cp.referenced_variables,
        })).collect::<Vec<_>>(),
        "guard_diagnoses": snapshot.guard_diagnoses.iter().map(|gd| serde_json::json!({
            "guard_expr": gd.guard_expr,
            "transition_from": gd.transition.0,
            "transition_to": gd.transition.1,
            "event": gd.event,
            "dependencies": gd.dependencies.iter().collect::<Vec<_>>(),
            "dependency_values": gd.dependency_values.iter().map(|(k, v)| {
                let json_val = match v {
                    sysml_core::Value::Int(i) => serde_json::json!(i),
                    sysml_core::Value::Float(f) => serde_json::json!(f),
                    sysml_core::Value::Bool(b) => serde_json::json!(b),
                    sysml_core::Value::String(s) => serde_json::json!(s),
                    _ => serde_json::json!(format!("{:?}", v)),
                };
                (k.clone(), json_val)
            }).collect::<serde_json::Map<String, serde_json::Value>>(),
            "satisfied": gd.satisfied,
            "explanation": gd.explanation,
        })).collect::<Vec<_>>(),
        "messages": snapshot.messages.iter().map(|msg| serde_json::json!({
            "flow_id": msg.flow_id,
            "source": msg.source,
            "target": msg.target,
            "payload": match &msg.payload {
                sysml_core::Value::Int(i) => serde_json::json!(i),
                sysml_core::Value::Float(f) => serde_json::json!(f),
                sysml_core::Value::Bool(b) => serde_json::json!(b),
                sysml_core::Value::String(s) => serde_json::json!(s),
                sysml_core::Value::Null => serde_json::json!(null),
                other => serde_json::json!(format!("{:?}", other)),
            },
        })).collect::<Vec<_>>(),
        "causation_links": snapshot.causation_links.iter().map(|cl| serde_json::json!({
            "tick": cl.tick,
            "variable": cl.variable,
            "old_value": format!("{:?}", cl.old_value),
            "new_value": format!("{:?}", cl.new_value),
            "writer_subsystem": cl.writer_subsystem,
            "affected_guards": cl.affected_guards.iter().map(|(ss, desc, newly)| serde_json::json!({
                "subsystem": ss,
                "transition": desc,
                "newly_satisfied": newly,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}
