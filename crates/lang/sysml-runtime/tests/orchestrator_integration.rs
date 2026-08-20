//! Integration test for the orchestrator: multi-machine espresso scenario.
//!
//! Models the coffee-machine/orchestration.sysml fixture:
//! - BoilerController: cold → heating → ready (via events)
//! - BrewController: standby → preInfusion → extraction → complete (guard reads shared context)
//! - Shared context: boilerTemp, machineReady, brewProgress
//! - Scheduled events drive the scenario on a timeline

use std::collections::HashMap;
use sysml_core::Value;
use sysml_runtime::{
    expressions::EvalContext,
    orchestrator::{Orchestrator, OrchestratorConfig},
    statemachine::StateMachineRunner,
    AssignmentIR, StateIR, StateMachineIR, StepResult, TransitionActionIR, TransitionIR,
};

/// Build the BoilerController state machine.
/// States: cold → heating → ready → (back to heating or cold)
/// On heating→ready transition, sets boilerTemp=93 and machineReady=true.
fn build_boiler_controller() -> StateMachineRunner {
    let ir = StateMachineIR {
        name: "BoilerController".to_string(),
        states: vec![
            StateIR::new("cold"),
            StateIR::new("heating"),
            StateIR::new("ready"),
            StateIR::new("overheated"),
        ],
        transitions: vec![
            TransitionIR::new("cold", "heating").with_event("powerOn"),
            TransitionIR::new("heating", "ready")
                .with_event("tempReached")
                .with_action(TransitionActionIR::Structured {
                    assignments: vec![
                        AssignmentIR::set("boilerTemp", 93.0),
                        AssignmentIR::set("machineReady", 1.0), // 1.0 = true
                    ],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                }),
            TransitionIR::new("ready", "heating").with_event("tempDropped"),
            TransitionIR::new("heating", "overheated").with_event("tempOverLimit"),
            TransitionIR::new("overheated", "cold")
                .with_event("cooledDown")
                .with_action(TransitionActionIR::Structured {
                    assignments: vec![
                        AssignmentIR::set("boilerTemp", 20.0),
                        AssignmentIR::set("machineReady", 0.0),
                    ],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                }),
            TransitionIR::new("ready", "cold")
                .with_event("powerOff")
                .with_action(TransitionActionIR::Structured {
                    assignments: vec![AssignmentIR::set("machineReady", 0.0)],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                }),
        ],
        initial: "cold".to_string(),
        regions: vec![],
    };
    StateMachineRunner::new(ir)
}

/// Build the BrewController state machine.
/// States: standby → preInfusion → extraction → complete
/// The standby→preInfusion guard checks machineReady > 0.
fn build_brew_controller() -> StateMachineRunner {
    let ir = StateMachineIR {
        name: "BrewController".to_string(),
        states: vec![
            StateIR::new("standby"),
            StateIR::new("preInfusion"),
            StateIR::new("extraction"),
            StateIR::new("complete"),
            StateIR::new("error"),
        ],
        transitions: vec![
            TransitionIR::new("standby", "preInfusion")
                .with_event("startBrew")
                .with_guard("machineReady > 0"),
            TransitionIR::new("preInfusion", "extraction").with_event("preInfusionComplete"),
            TransitionIR::new("extraction", "complete")
                .with_event("extractionDone")
                .with_action(TransitionActionIR::Structured {
                    assignments: vec![AssignmentIR::set("brewProgress", 100.0)],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                }),
            TransitionIR::new("complete", "standby")
                .with_event("resetBrew")
                .with_action(TransitionActionIR::Structured {
                    assignments: vec![AssignmentIR::set("brewProgress", 0.0)],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                }),
            TransitionIR::new("extraction", "error").with_event("pressureFault"),
            TransitionIR::new("error", "standby").with_event("clearError"),
        ],
        initial: "standby".to_string(),
        regions: vec![],
    };
    StateMachineRunner::new(ir)
}

#[test]
fn test_full_brew_scenario() {
    let boiler = build_boiler_controller();
    let brew = build_brew_controller();

    let mut orch = Orchestrator::new(OrchestratorConfig {
        dt_ms: 100.0, // 100ms per tick
        ..Default::default()
    });

    orch.add_state_machine("boiler", boiler);
    orch.add_state_machine("brew", brew);

    // Set initial context
    orch.context.set("boilerTemp", Value::Float(20.0));
    orch.context.set("machineReady", Value::Float(0.0));
    orch.context.set("brewProgress", Value::Float(0.0));
    // Mint the SM assignment-target slots so writeback routes by SlotId (the
    // legacy no-slot writeback was deleted with the string-identity cull):
    // boiler (idx 0) writes boilerTemp+machineReady; brew (idx 1) writes
    // brewProgress. brew reads machineReady through the routed slot.
    orch.mint_state_slots_for_test(&[
        (0, "boilerTemp", 20.0),
        (0, "machineReady", 0.0),
        (1, "brewProgress", 0.0),
    ]);

    // Schedule the test scenario events
    orch.schedule_event(100.0, "boiler", "powerOn"); // t=100ms: power on
    orch.schedule_event(1000.0, "boiler", "tempReached"); // t=1s: boiler ready
    orch.schedule_event(1500.0, "brew", "startBrew"); // t=1.5s: start brew
    orch.schedule_event(2000.0, "brew", "preInfusionComplete"); // t=2s: pre-infusion done
    orch.schedule_event(7000.0, "brew", "extractionDone"); // t=7s: extraction done
    orch.schedule_event(7500.0, "brew", "resetBrew"); // t=7.5s: reset

    // Run the scenario
    let snapshots = orch.step_until(8000.0);

    // Verify the timeline
    assert!(!snapshots.is_empty(), "should have produced snapshots");

    // Check that boiler went through cold → heating → ready
    let at_500ms = snapshots.iter().find(|s| s.time_ms >= 500.0).unwrap();
    assert_eq!(at_500ms.subsystem_states["boiler"].current_state, "heating");
    assert_eq!(at_500ms.subsystem_states["brew"].current_state, "standby");

    // After tempReached (t=1s), boiler should be ready and machineReady=1
    let at_1100ms = snapshots.iter().find(|s| s.time_ms >= 1100.0).unwrap();
    assert_eq!(at_1100ms.subsystem_states["boiler"].current_state, "ready");
    assert_eq!(
        at_1100ms.variables.get("machineReady"),
        Some(&Value::Float(1.0)),
        "boiler should have set machineReady=1"
    );
    assert_eq!(
        at_1100ms.variables.get("boilerTemp"),
        Some(&Value::Float(93.0)),
        "boiler should have set boilerTemp=93"
    );

    // After startBrew (t=1.5s), brew should be in preInfusion (guard passed because machineReady=1)
    let at_1600ms = snapshots.iter().find(|s| s.time_ms >= 1600.0).unwrap();
    assert_eq!(
        at_1600ms.subsystem_states["brew"].current_state, "preInfusion",
        "brew should start because machineReady > 0"
    );

    // After extractionDone (t=7s), brewProgress should be 100
    let at_7100ms = snapshots.iter().find(|s| s.time_ms >= 7100.0).unwrap();
    assert_eq!(at_7100ms.subsystem_states["brew"].current_state, "complete");
    assert_eq!(
        at_7100ms.variables.get("brewProgress"),
        Some(&Value::Float(100.0)),
        "extraction should set brewProgress=100"
    );

    // After resetBrew (t=7.5s), brew back to standby, progress reset
    let at_7600ms = snapshots.iter().find(|s| s.time_ms >= 7600.0).unwrap();
    assert_eq!(at_7600ms.subsystem_states["brew"].current_state, "standby");
    assert_eq!(
        at_7600ms.variables.get("brewProgress"),
        Some(&Value::Float(0.0)),
        "reset should clear brewProgress"
    );
}

#[test]
fn test_brew_blocked_without_boiler() {
    // If the boiler never reaches temperature, the brew should stay in standby
    // because the guard `machineReady > 0` fails.
    let boiler = build_boiler_controller();
    let brew = build_brew_controller();

    let mut orch = Orchestrator::new(OrchestratorConfig {
        dt_ms: 100.0,
        ..Default::default()
    });

    orch.add_state_machine("boiler", boiler);
    orch.add_state_machine("brew", brew);

    orch.context.set("machineReady", Value::Float(0.0));

    // Try to start brew WITHOUT boiler reaching temperature
    orch.schedule_event(100.0, "brew", "startBrew");

    let snapshots = orch.step_until(500.0);
    let last = snapshots.last().unwrap();

    // Brew should still be in standby — guard failed
    assert_eq!(
        last.subsystem_states["brew"].current_state, "standby",
        "brew should be blocked: machineReady is 0"
    );
    assert_eq!(
        last.subsystem_states["boiler"].current_state, "cold",
        "boiler never powered on"
    );
}

#[test]
fn test_overheated_recovery() {
    let boiler = build_boiler_controller();

    let mut orch = Orchestrator::new(OrchestratorConfig {
        dt_ms: 100.0,
        ..Default::default()
    });
    orch.add_state_machine("boiler", boiler);
    orch.context.set("boilerTemp", Value::Float(20.0));
    // boiler (idx 0) writes boilerTemp + machineReady; mint their slots so the
    // routed writeback publishes them (legacy no-slot writeback was deleted).
    orch.mint_state_slots_for_test(&[(0, "boilerTemp", 20.0), (0, "machineReady", 0.0)]);

    // Power on → heat → overheat → cool down → back to cold
    orch.schedule_event(100.0, "boiler", "powerOn");
    orch.schedule_event(500.0, "boiler", "tempOverLimit");
    orch.schedule_event(1000.0, "boiler", "cooledDown");

    let snapshots = orch.step_until(1500.0);

    let at_600ms = snapshots.iter().find(|s| s.time_ms >= 600.0).unwrap();
    assert_eq!(
        at_600ms.subsystem_states["boiler"].current_state,
        "overheated"
    );

    let at_1100ms = snapshots.iter().find(|s| s.time_ms >= 1100.0).unwrap();
    assert_eq!(at_1100ms.subsystem_states["boiler"].current_state, "cold");
    assert_eq!(
        at_1100ms.variables.get("boilerTemp"),
        Some(&Value::Float(20.0)),
        "cooledDown should reset boilerTemp"
    );
    assert_eq!(
        at_1100ms.variables.get("machineReady"),
        Some(&Value::Float(0.0)),
        "cooledDown should clear machineReady"
    );
}
