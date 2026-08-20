//! End-to-end pipeline tests: build ModelGraph -> elaborate -> compile -> execute.
//!
//! These tests verify the full action execution pipeline from programmatic
//! graph construction through elaboration, compilation to IR, and execution.

use sysml_core::elaborate::elaborate;
use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::actions::{compile_action, ActionRunner};
use sysml_runtime::expressions::EvalContext;

// ---------------------------------------------------------------------------
// C3: Action execution pipeline
// ---------------------------------------------------------------------------

/// Build a simple sequential action graph, elaborate, compile, and execute.
///
/// Graph: ActionDefinition "BakeACake"
///   ├── ActionUsage "preheatOven"
///   ├── ActionUsage "mixIngredients"
///   └── ActionUsage "putInOven"
///   + SuccessionAsUsage ordering: preheat -> mix -> putIn
#[test]
fn action_sequential_elaborate_compile_execute() {
    let mut graph = ModelGraph::new();

    // Create action definition
    let action_def = Element::new_with_kind(ElementKind::ActionDefinition).with_name("BakeACake");
    let action_id = graph.add_element(action_def);

    // Create step 1: preheatOven
    let step1 = Element::new_with_kind(ElementKind::ActionUsage)
        .with_name("preheatOven")
        .with_owner(action_id.clone());
    let step1_id = graph.add_element(step1);

    // Create step 2: mixIngredients
    let step2 = Element::new_with_kind(ElementKind::ActionUsage)
        .with_name("mixIngredients")
        .with_owner(action_id.clone());
    let step2_id = graph.add_element(step2);

    // Create step 3: putInOven
    let step3 = Element::new_with_kind(ElementKind::ActionUsage)
        .with_name("putInOven")
        .with_owner(action_id.clone());
    let step3_id = graph.add_element(step3);

    // Create succession relationships (step1 -> step2 -> step3)
    let succ1 = Relationship::new(
        RelationshipKind::Transition,
        step1_id.clone(),
        step2_id.clone(),
    );
    graph.add_relationship(succ1);

    let succ2 = Relationship::new(
        RelationshipKind::Transition,
        step2_id.clone(),
        step3_id.clone(),
    );
    graph.add_relationship(succ2);

    // Elaborate
    let report = elaborate(&mut graph);
    eprintln!("BakeACake elaboration: {}", report);

    // Compile
    let ir = compile_action("BakeACake", &graph).expect("Should compile action");

    assert_eq!(ir.name, "BakeACake");
    // Should have: initial, final, plus 3 action step nodes
    assert!(
        ir.nodes.len() >= 5,
        "Should have at least 5 nodes (initial + 3 steps + final), got {}",
        ir.nodes.len()
    );

    // Execute
    let mut runner = ActionRunner::new(ir);
    let ctx = EvalContext::new();

    // Step through until completion
    let mut steps = 0;
    loop {
        let result = runner.step(&ctx);
        steps += 1;
        if result.completed {
            break;
        }
        assert!(steps < 20, "Action should complete within 20 steps");
    }

    assert!(steps >= 3, "Should take at least 3 steps for 3 actions");
}

/// Build an action with a send action, elaborate, compile, and verify
/// messages are produced.
#[test]
fn action_with_send_elaborate_compile_execute() {
    let mut graph = ModelGraph::new();

    let action_def = Element::new_with_kind(ElementKind::ActionDefinition).with_name("NotifyUser");
    let action_id = graph.add_element(action_def);

    // Send action step
    let send = Element::new_with_kind(ElementKind::SendActionUsage)
        .with_name("sendAlert")
        .with_owner(action_id.clone())
        .with_prop("target", "user")
        .with_prop("payload", "temperature warning");
    graph.add_element(send);

    let report = elaborate(&mut graph);
    eprintln!("NotifyUser elaboration: {}", report);

    let ir = compile_action("NotifyUser", &graph).expect("Should compile action");
    assert_eq!(ir.name, "NotifyUser");

    let mut runner = ActionRunner::new(ir);
    let ctx = EvalContext::new();

    let mut messages_produced = Vec::new();
    let mut steps = 0;
    loop {
        let result = runner.step(&ctx);
        messages_produced.extend(result.messages);
        steps += 1;
        if result.completed {
            break;
        }
        assert!(steps < 20, "Action should complete within 20 steps");
    }

    assert!(
        !messages_produced.is_empty(),
        "Send action should produce at least one message"
    );
    assert_eq!(messages_produced[0].target, "user");
}

/// Action that compiles from graph with no children should complete immediately.
#[test]
fn empty_action_compiles_and_completes() {
    let mut graph = ModelGraph::new();

    let action_def = Element::new_with_kind(ElementKind::ActionDefinition).with_name("EmptyAction");
    graph.add_element(action_def);

    elaborate(&mut graph);

    let ir = compile_action("EmptyAction", &graph).expect("Should compile empty action");
    assert_eq!(ir.name, "EmptyAction");
    assert_eq!(ir.nodes.len(), 2, "Should have just initial + final");

    let mut runner = ActionRunner::new(ir);
    let ctx = EvalContext::new();

    // Should complete quickly
    let r1 = runner.step(&ctx);
    // May need one more step to process final
    if !r1.completed {
        let r2 = runner.step(&ctx);
        assert!(r2.completed, "Empty action should complete in 2 steps");
    }
}

// ---------------------------------------------------------------------------
// Parsed .sysml pipeline test
// ---------------------------------------------------------------------------

/// Parse a .sysml string containing an action definition, elaborate, and compile.
#[test]
fn parsed_sysml_action_pipeline() {
    let source = r#"
        action def BakeACake {
            action preheatOven;
            action mixIngredients;
            action putInOven;
            first preheatOven then mixIngredients;
            then putInOven;
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

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("Parsed action elaboration: {}", report);

    // Check we have an action definition with child actions
    let action_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ActionDefinition)
        .collect();
    assert!(!action_defs.is_empty(), "Should have ActionDefinition");
    assert_eq!(
        action_defs[0].name.as_deref(),
        Some("BakeACake"),
        "ActionDefinition should be named 'BakeACake'"
    );

    let action_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ActionUsage)
        .collect();
    assert!(
        action_usages.len() >= 3,
        "Should have at least 3 ActionUsage steps, got {}",
        action_usages.len()
    );

    // Try compilation
    match compile_action("BakeACake", &result.graph) {
        Ok(ir) => {
            eprintln!(
                "Compiled: name={}, nodes={}, edges={}",
                ir.name,
                ir.nodes.len(),
                ir.edges.len()
            );

            // Execute
            let mut runner = ActionRunner::new(ir);
            let ctx = EvalContext::new();
            let mut steps = 0;
            loop {
                let step_result = runner.step(&ctx);
                steps += 1;
                if step_result.completed || steps > 30 {
                    break;
                }
            }
            eprintln!("Action completed in {} steps", steps);
        }
        Err(diags) => {
            eprintln!("Compilation diagnostics: {} errors", diags.len());
            for d in &diags {
                eprintln!("  - {}", d.message);
            }
        }
    }
}
