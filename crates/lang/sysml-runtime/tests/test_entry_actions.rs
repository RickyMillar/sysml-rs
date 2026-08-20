use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::statemachine::StateMachineCompiler;
use sysml_runtime::{AssignmentIR, AssignmentOp, TransitionActionIR};

/// Helper: find an assignment by variable name in a list.
fn find_assignment<'a>(assignments: &'a [AssignmentIR], var: &str) -> &'a AssignmentIR {
    assignments
        .iter()
        .find(|a| a.variable == var)
        .unwrap_or_else(|| {
            let vars: Vec<_> = assignments.iter().map(|a| &a.variable).collect();
            panic!("assignment for '{var}' not found; got: {vars:?}");
        })
}

/// Verify that `entry action { boilerTemp = 20; machineReady = 0; }` parses into
/// AssignmentActionUsage elements and compiles to a Structured entry action IR.
#[test]
fn entry_action_bare_assignments_produce_structured_ir() {
    let source = r#"
package CoffeeMachine {
    state def BoilerSM {
        entry; then cold;

        state cold {
            entry action { boilerTemp = 20; machineReady = 0; }
        }

        state heating {
            entry action { boilerTemp = 93; machineReady = 1; }
        }

        transition t1 first cold accept heatCmd then heating;
    }
}
"#;

    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("boiler.sysml", source)];
    let result = parser.parse(&files);

    assert!(
        result.diagnostics.is_empty(),
        "Parse should succeed without errors: {:?}",
        result.diagnostics
    );

    let mut graph = result.graph;
    sysml_core::elaborate::elaborate(&mut graph);

    let ir = StateMachineCompiler::compile_named(&graph, "BoilerSM")
        .expect("SM compilation should succeed");

    // Verify the "cold" state has a structured entry action
    let cold = ir
        .states
        .iter()
        .find(|s| s.name == "cold")
        .expect("cold state");
    let entry = cold
        .entry_action
        .as_ref()
        .expect("cold should have entry action");

    match entry {
        TransitionActionIR::Structured {
            assignments, sends, ..
        } => {
            assert_eq!(assignments.len(), 2, "cold entry should have 2 assignments");

            let bt = find_assignment(assignments, "boilerTemp");
            assert_eq!(bt.operator, AssignmentOp::Set);
            assert!((bt.as_f64().expect("numeric value") - 20.0).abs() < f64::EPSILON);

            let mr = find_assignment(assignments, "machineReady");
            assert_eq!(mr.operator, AssignmentOp::Set);
            assert!((mr.as_f64().expect("numeric value") - 0.0).abs() < f64::EPSILON);

            assert!(sends.is_empty());
        }
        TransitionActionIR::Simple(s) => {
            panic!("Expected Structured action, got Simple({s:?})");
        }
    }

    // Verify the "heating" state entry action
    let heating = ir
        .states
        .iter()
        .find(|s| s.name == "heating")
        .expect("heating state");
    let entry = heating
        .entry_action
        .as_ref()
        .expect("heating should have entry action");

    match entry {
        TransitionActionIR::Structured { assignments, .. } => {
            assert_eq!(
                assignments.len(),
                2,
                "heating entry should have 2 assignments"
            );

            let bt = find_assignment(assignments, "boilerTemp");
            assert!((bt.as_f64().expect("numeric value") - 93.0).abs() < f64::EPSILON);

            let mr = find_assignment(assignments, "machineReady");
            assert!((mr.as_f64().expect("numeric value") - 1.0).abs() < f64::EPSILON);
        }
        TransitionActionIR::Simple(s) => {
            panic!("Expected Structured action, got Simple({s:?})");
        }
    }
}

/// Standard `assign x := value;` syntax should still work inside entry actions.
#[test]
fn entry_action_assign_keyword_produces_structured_ir() {
    let source = r#"
package TestPkg {
    state def TestSM {
        entry; then s1;

        state s1 {
            entry action {
                assign boilerTemp := 20;
            }
        }

        state s2;
        transition t1 first s1 accept go then s2;
    }
}
"#;

    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("test.sysml", source)];
    let result = parser.parse(&files);

    assert!(
        result.diagnostics.is_empty(),
        "Parse should succeed: {:?}",
        result.diagnostics
    );

    let mut graph = result.graph;
    sysml_core::elaborate::elaborate(&mut graph);

    let ir = StateMachineCompiler::compile_named(&graph, "TestSM")
        .expect("SM compilation should succeed");

    let s1 = ir.states.iter().find(|s| s.name == "s1").expect("s1 state");
    let entry = s1
        .entry_action
        .as_ref()
        .expect("s1 should have entry action");

    match entry {
        TransitionActionIR::Structured { assignments, .. } => {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "boilerTemp");
            assert!((assignments[0].as_f64().expect("numeric value") - 20.0).abs() < f64::EPSILON);
        }
        TransitionActionIR::Simple(s) => {
            panic!("Expected Structured action, got Simple({s:?})");
        }
    }
}
