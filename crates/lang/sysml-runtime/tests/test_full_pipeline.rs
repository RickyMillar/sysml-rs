use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::statemachine::StateMachineCompiler;
use sysml_runtime::Runner;

#[test]
fn test_orchestration_file_produces_context_values() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/boiler-orchestration.sysml"
    );
    let source =
        std::fs::read_to_string(fixture).unwrap_or_else(|e| panic!("read fixture {fixture}: {e}"));

    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("boiler-orchestration.sysml", source)];
    let result = parser.parse(&files);

    println!("=== Parse diagnostics ({}) ===", result.diagnostics.len());
    for d in &result.diagnostics {
        println!("  [{:?}] {}", d.severity, d.message);
    }

    let mut graph = result.graph;
    println!("\nElements before elaborate: {}", graph.element_count());
    sysml_core::elaborate::elaborate(&mut graph);
    println!("Elements after elaborate: {}", graph.element_count());

    // Check what state elements exist
    println!("\n=== States found ===");
    for e in graph.elements.values() {
        if e.kind == sysml_core::ElementKind::StateUsage
            || e.kind == sysml_core::ElementKind::StateDefinition
        {
            if let Some(name) = &e.name {
                let entry = e.get_prop("entry");
                println!("  {} ({:?}): entry={:?}", name, e.kind, entry);

                // Check children for ActionUsage with assignments
                for child in graph.children_of(&e.id) {
                    if child.kind == sysml_core::ElementKind::ActionUsage {
                        println!(
                            "    ActionUsage child: name={:?} subactionKind={:?}",
                            child.name,
                            child.get_prop("stateSubactionKind")
                        );
                        for gc in graph.children_of(&child.id) {
                            println!(
                                "      grandchild: {:?} name={:?} target={:?} value={:?}",
                                gc.kind,
                                gc.name,
                                gc.get_prop("target"),
                                gc.get_prop("value")
                            );
                        }
                    }
                }
            }
        }
    }

    // Compile BoilerController
    println!("\n=== Compile BoilerController ===");
    match StateMachineCompiler::compile_named(&graph, "BoilerController") {
        Ok(ir) => {
            for state in &ir.states {
                println!("  State '{}': entry={:?}", state.name, state.entry_action);
            }

            // Run: initial step should enter cold → set boilerTemp=20
            let mut runner = sysml_runtime::statemachine::StateMachineRunner::new(ir);
            let r = Runner::step(&mut runner, None);
            println!(
                "\n  After step(None): state={} sends={:?}",
                Runner::current_state(&runner),
                r.sends
            );
            println!("  Context: {:?}", runner.eval_ctx.variables);

            // Step with powerOn → enter heating → set boilerTemp=45, heaterOn=1
            let r = Runner::step(&mut runner, Some("powerOn"));
            println!(
                "\n  After step(powerOn): state={} sends={:?}",
                Runner::current_state(&runner),
                r.sends
            );
            println!("  Context: {:?}", runner.eval_ctx.variables);

            // Step with tempReached → enter ready → set boilerTemp=93, machineReady=1
            let r = Runner::step(&mut runner, Some("tempReached"));
            println!(
                "\n  After step(tempReached): state={} sends={:?}",
                Runner::current_state(&runner),
                r.sends
            );
            println!("  Context: {:?}", runner.eval_ctx.variables);

            // Verify values
            // Entry actions with literal integer values preserve Int type
            let bt = runner.eval_ctx.get("boilerTemp").cloned();
            assert!(
                bt == Some(sysml_core::Value::Int(93))
                    || bt == Some(sysml_core::Value::Float(93.0)),
                "boilerTemp should be 93 after entering ready, got {bt:?}"
            );
            let mr = runner.eval_ctx.get("machineReady").cloned();
            assert!(
                mr == Some(sysml_core::Value::Int(1)) || mr == Some(sysml_core::Value::Float(1.0)),
                "machineReady should be 1 after entering ready, got {mr:?}"
            );
        }
        Err(diags) => {
            for d in &diags {
                println!("  ERROR: {}", d.message);
            }
            panic!("Failed to compile BoilerController");
        }
    }
}
