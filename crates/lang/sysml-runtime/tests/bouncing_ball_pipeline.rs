//! Pipeline test for the bouncing ball hybrid model.
//!
//! Loads BouncingBall.sysml, compiles via workspace orchestrator,
//! and verifies the ODE produces correct free-fall physics.

use std::path::PathBuf;

use sysml_core::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

fn bouncing_ball_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("bouncing-ball")
}

fn load_bouncing_ball() -> sysml_core::ModelGraph {
    let dir = bouncing_ball_dir();
    let path = dir.join("BouncingBall.sysml");
    let source = std::fs::read_to_string(&path).expect("read BouncingBall.sysml");
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("BouncingBall.sysml", source)]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    graph
}

#[test]
fn test_bouncing_ball_ode_detected() {
    let graph = load_bouncing_ball();
    let compiler = ModelCompiler::new(graph);
    // Debug: check what the parser produced
    {
        use sysml_core::ElementKind;
        let graph = compiler.graph();
        for elem in graph.elements.values() {
            if elem.kind == ElementKind::MetadataUsage {
                let owner = elem.owner.as_ref().and_then(|id| graph.get_element(id));
                let owner_kind = owner.map(|o| format!("{:?}", o.kind)).unwrap_or_default();
                let owner_name = owner.and_then(|o| o.name.clone()).unwrap_or_default();
                println!(
                    "Meta: name={:?} type={:?} owner={}({})",
                    elem.name,
                    elem.get_prop("unresolvedTypeName"),
                    owner_name,
                    owner_kind
                );
            }
        }
    }

    let odes = compiler.detect_all_odes_unified();
    println!("Detected {} ODEs", odes.len());
    for ode in &odes {
        println!(
            "  ODE: name={:?} vars={:?} derivs={:?}",
            ode.name, ode.state_vars, ode.derivative_exprs
        );
    }

    assert!(!odes.is_empty(), "should detect at least one ODE");

    let ode = &odes[0];
    println!(
        "ODE: name={:?} vars={:?} derivs={:?}",
        ode.name, ode.state_vars, ode.derivative_exprs
    );

    assert!(
        ode.state_vars.contains(&"y".to_string()) && ode.state_vars.contains(&"v".to_string()),
        "should have y and v as state vars, got {:?}",
        ode.state_vars
    );
}

#[test]
fn test_bouncing_ball_workspace_orchestrator() {
    let graph = load_bouncing_ball();
    let compiler = ModelCompiler::new(graph);
    let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = std::sync::Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));

    let mut orch = compiler
        .build_workspace_orchestrator(
            base_ctx,
            Some(precompiled),
            None,
            None,
            None,
            &[],
            Some(1.0),
            Some(5000.0),
        )
        .expect("workspace orchestrator should build");

    // Seed y and v as Floats (replace Refs from context_from_graph)
    orch.context
        .set("y".to_owned(), sysml_core::Value::Float(10.0));
    orch.context
        .set("v".to_owned(), sysml_core::Value::Float(0.0));

    // Step for 100 ticks (100ms) — ball should be in free fall
    for _ in 0..100 {
        orch.step();
    }

    let y = orch
        .context
        .get("y")
        .and_then(|v| match v {
            sysml_core::Value::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(10.0);
    let v = orch
        .context
        .get("v")
        .and_then(|v| match v {
            sysml_core::Value::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(0.0);

    println!("After 100ms: y={y:.3}m, v={v:.3}m/s");

    // After 100ms of free fall from 10m: y ≈ 10 - 0.5*9.81*0.1² ≈ 9.951m
    // v ≈ -9.81 * 0.1 ≈ -0.981 m/s
    assert!(y < 10.0, "y should decrease under gravity, got {y:.3}");
    assert!(v < 0.0, "v should be negative (falling), got {v:.3}");
}
