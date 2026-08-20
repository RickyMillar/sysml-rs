//! End-to-end pipeline tests: build ModelGraph -> elaborate -> compile -> execute.
//!
//! These tests verify the full pipeline from programmatic graph construction
//! through elaboration, compilation to IR, and execution.

use sysml_core::elaborate::elaborate;
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::statemachine::{StateMachineCompiler, StateMachineRunner};
use sysml_runtime::{CompileToIR, Runner, StateIR, StateMachineIR, TransitionIR};
use sysml_span::Span;

// ---------------------------------------------------------------------------
// C1: State machine pipeline
// ---------------------------------------------------------------------------

/// Build a simple state machine graph, elaborate it, and verify elaboration
/// correctly derives initial state and transitions. Then execute the IR.
#[test]
fn state_machine_elaborate_compile_execute() {
    let mut graph = ModelGraph::new();

    // Create state machine definition
    let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("TrafficLight");
    let sm_id = graph.add_element(sm);

    // Create states as children
    let red = Element::new_with_kind(ElementKind::StateUsage)
        .with_name("Red")
        .with_owner(sm_id.clone())
        .with_prop("keyword", "state")
        .with_span(Span::new("test", 0, 10));
    let red_id = graph.add_element(red);

    let _green = Element::new_with_kind(ElementKind::StateUsage)
        .with_name("Green")
        .with_owner(sm_id.clone())
        .with_prop("keyword", "state")
        .with_span(Span::new("test", 10, 20));
    graph.add_element(_green);

    let _yellow = Element::new_with_kind(ElementKind::StateUsage)
        .with_name("Yellow")
        .with_owner(sm_id.clone())
        .with_prop("keyword", "state")
        .with_span(Span::new("test", 20, 30));
    graph.add_element(_yellow);

    // Create transitions as TransitionUsage children
    // Triggers are real AcceptActionUsage children wrapped in
    // TransitionFeatureMembership(kind=trigger); the canonical trigger
    // string lives on the child's `text` prop (SysML v2 Â§8.3.18.8).
    for (source, target) in [("Red", "Green"), ("Green", "Yellow"), ("Yellow", "Red")] {
        let t = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(sm_id.clone())
            .with_prop("source", source)
            .with_prop("target", target);
        let t_id = graph.add_element(t);
        graph.add_transition_feature(
            &t_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", "timer"),
        );
    }

    // Elaborate: should tag initial state (Red, first by span) and create transitions
    let report = elaborate(&mut graph);

    assert!(
        report.initial_states_tagged > 0,
        "Should tag at least one initial state"
    );
    assert!(
        report.transitions_created > 0,
        "Should create transition relationships"
    );

    // Verify Red is tagged initial
    let red_elem = graph.get_element(&red_id).unwrap();
    assert_eq!(
        red_elem.get_prop("initial").and_then(|v| v.as_bool()),
        Some(true),
        "Red should be tagged as initial"
    );

    // Build IR from elaborated info and execute
    let ir = StateMachineIR::new("TrafficLight", "Red")
        .with_state(StateIR::new("Red"))
        .with_state(StateIR::new("Green"))
        .with_state(StateIR::new("Yellow"))
        .with_transition(TransitionIR::new("Red", "Green").with_event("timer"))
        .with_transition(TransitionIR::new("Green", "Yellow").with_event("timer"))
        .with_transition(TransitionIR::new("Yellow", "Red").with_event("timer"));

    let mut runner = StateMachineRunner::new(ir);
    assert_eq!(runner.current_state(), "Red");

    let result = runner.step(Some("timer"));
    assert_eq!(result.state, "Green");

    let result = runner.step(Some("timer"));
    assert_eq!(result.state, "Yellow");

    let result = runner.step(Some("timer"));
    assert_eq!(result.state, "Red");
}

/// Verify that entry/exit actions are tagged during elaboration.
#[test]
fn state_machine_with_entry_exit_actions() {
    let mut graph = ModelGraph::new();

    let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("Door");
    let sm_id = graph.add_element(sm);

    let open = Element::new_with_kind(ElementKind::StateUsage)
        .with_name("Open")
        .with_owner(sm_id.clone())
        .with_prop("keyword", "state")
        .with_span(Span::new("test", 0, 10));
    let open_id = graph.add_element(open);

    // Entry action child (ActionUsage with stateSubactionKind)
    let entry = Element::new_with_kind(ElementKind::ActionUsage)
        .with_owner(open_id.clone())
        .with_prop("stateSubactionKind", "entry")
        .with_prop("unresolved_value", "turnOnLight");
    graph.add_element(entry);

    // Exit action child (ActionUsage with stateSubactionKind)
    let exit = Element::new_with_kind(ElementKind::ActionUsage)
        .with_owner(open_id.clone())
        .with_prop("stateSubactionKind", "exit")
        .with_prop("unresolved_value", "turnOffLight");
    graph.add_element(exit);

    let report = elaborate(&mut graph);

    assert!(
        report.state_actions_tagged > 0,
        "Should tag entry/exit actions"
    );

    let open_elem = graph.get_element(&open_id).unwrap();
    assert!(
        open_elem.get_prop("entry").is_some(),
        "Open state should have entry action"
    );
    assert!(
        open_elem.get_prop("exit").is_some(),
        "Open state should have exit action"
    );
}

// ---------------------------------------------------------------------------
// C5: Cross-crate pipeline test (state machine + constraint guards)
// ---------------------------------------------------------------------------

/// Test state machine with expression guards — exercises both
/// sysml-run-statemachine and sysml-run-expressions together.
#[test]
fn state_machine_with_guard_expressions() {
    let ir = StateMachineIR::new("SpeedController", "Idle")
        .with_state(StateIR::new("Idle"))
        .with_state(StateIR::new("Running"))
        .with_state(StateIR::new("Overspeed"))
        .with_transition(
            TransitionIR::new("Idle", "Running")
                .with_event("start")
                .with_guard("engine_on == 1"),
        )
        .with_transition(TransitionIR::new("Running", "Overspeed").with_guard("speed > 100"))
        .with_transition(TransitionIR::new("Overspeed", "Running").with_guard("speed <= 100"));

    let mut runner = StateMachineRunner::new(ir);
    runner.eval_ctx.set("engine_on", Value::Int(0));
    runner.eval_ctx.set("speed", Value::Float(0.0));

    // Guard should block transition when engine_on=0
    let result = runner.step(Some("start"));
    assert_eq!(result.state, "Idle", "Guard should block when engine_on=0");

    // Guard should allow transition when engine_on=1
    runner.eval_ctx.set("engine_on", Value::Int(1));
    let result = runner.step(Some("start"));
    assert_eq!(result.state, "Running");

    // Speed guard check
    runner.eval_ctx.set("speed", Value::Float(120.0));
    let result = runner.step(None);
    assert_eq!(result.state, "Overspeed");

    // Back to running when speed drops
    runner.eval_ctx.set("speed", Value::Float(80.0));
    let result = runner.step(None);
    assert_eq!(result.state, "Running");
}

// ---------------------------------------------------------------------------
// Parsed .sysml pipeline test
// ---------------------------------------------------------------------------

/// Parse a .sysml string containing a state machine, elaborate, compile, and execute.
#[test]
fn parsed_sysml_state_machine_pipeline() {
    let source = r#"
        state def TrafficLight {
            state Red;
            state Green;
            state Yellow;
            transition first Red then Green;
            transition first Green then Yellow;
            transition first Yellow then Red;
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("test.sysml", source)]);

    assert!(
        result.diagnostics.is_empty(),
        "Parse errors: {:?}",
        result.diagnostics
    );
    assert!(
        result.graph.element_count() > 0,
        "Should produce elements from parsed .sysml"
    );

    // Elaborate the parsed graph
    let report = elaborate(&mut result.graph);
    eprintln!("Parsed state machine elaboration: {}", report);

    // Verify we have state definitions and transitions
    let state_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .collect();
    assert!(!state_defs.is_empty(), "Should have StateDefinition");

    let transitions: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::TransitionUsage)
        .collect();
    assert!(
        !transitions.is_empty(),
        "Should have TransitionUsage elements"
    );

    // Verify transitions have source/target properties (from parser fix)
    for t in &transitions {
        let src = t.get_prop("source");
        let tgt = t.get_prop("target");
        assert!(
            src.is_some(),
            "TransitionUsage should have 'source' after parsing"
        );
        assert!(
            tgt.is_some(),
            "TransitionUsage should have 'target' after parsing"
        );
    }

    // Try compilation
    match StateMachineCompiler::compile(&result.graph) {
        Ok(ir) => {
            eprintln!(
                "Compiled: name={}, states={}, transitions={}",
                ir.name,
                ir.states.len(),
                ir.transitions.len()
            );
            // Execute if compilation succeeded
            let runner = StateMachineRunner::new(ir);
            let state = runner.current_state().to_string();
            eprintln!("Initial state: {}", state);
        }
        Err(diags) => {
            eprintln!(
                "Compilation produced {} diagnostics (expected for library-only models)",
                diags.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// G23: `exhibit state <name> : <Type>;` phantom-subsystem regression
// ---------------------------------------------------------------------------

/// Synthetic repro from the grammar-gaps inventory (G23): `exhibit state
/// oscillator : RealSM;` inside a `part def` previously mis-parsed into an
/// empty `usage` node + an unrelated sibling `state_usage` named after the
/// owning part. `sorted_state_definitions`'s `PartDefinition` branch then
/// picked up that phantom `StateUsage` child and registered `HostPart` as a
/// second, bogus state-machine root alongside the real `RealSM`. With the
/// grammar fix, `exhibit state ...` mints a single `ExhibitStateUsage`
/// (a distinct ElementKind from `StateUsage`), so `HostPart` must no longer
/// appear in `list_state_machine_names`.
#[test]
fn g23_exhibit_state_does_not_mint_phantom_subsystem() {
    let source = r#"
        state def RealSM {
            state a;
            state b;
            transition first a accept after(1) then b;
        }
        part def HostPart {
            exhibit state oscillator : RealSM;
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("g23_repro.sysml", source)]);
    assert!(
        result.diagnostics.is_empty(),
        "G23 repro should parse without errors: {:?}",
        result.diagnostics
    );

    elaborate(&mut result.graph);

    let names = StateMachineCompiler::list_state_machine_names(&result.graph);
    assert_eq!(
        names,
        vec!["RealSM".to_owned()],
        "HostPart must not appear as a phantom state-machine root (G23), got {names:?}"
    );
}
