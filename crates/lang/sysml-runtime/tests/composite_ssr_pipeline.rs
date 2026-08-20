//! Integration tests for composite SSR pattern detection.
//!
//! Tests `action def :> ContinuousStateSpaceDynamics` and
//! `action def :> DiscreteStateSpaceDynamics` composite patterns.

use std::path::PathBuf;

use sysml_core::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

fn examples_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("..").join("..").join("..").join("examples")
}

fn load_model(subdir: &str, filename: &str) -> sysml_core::ModelGraph {
    let path = examples_dir().join(subdir).join(filename);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new(filename.to_string(), source)]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    graph
}

// -----------------------------------------------------------------------
// Continuous: Bouncing Ball with ContinuousStateSpaceDynamics
// -----------------------------------------------------------------------

#[test]
fn test_bouncing_ball_composite_detection() {
    let graph = load_model("bouncing-ball", "BouncingBall.sysml");
    let compiler = ModelCompiler::new(graph);

    // Test composite detection specifically
    let composites = compiler.detect_composite_continuous_ssr();
    println!("Composite continuous detections: {}", composites.len());
    for c in &composites {
        println!(
            "  name={:?} vars={:?} derivs={:?} signals={:?} tool={}",
            c.name,
            c.state_vars,
            c.derivative_exprs,
            c.signal_exprs.keys().collect::<Vec<_>>(),
            c.tool_name
        );
    }

    assert!(
        !composites.is_empty(),
        "should detect ContinuousStateSpaceDynamics composite"
    );

    let ode = &composites[0];
    assert_eq!(ode.name.as_deref(), Some("BouncingBallPart"));
    assert_eq!(ode.tool_name, "ssr:ContinuousStateSpaceDynamics");

    // Should have y and v as state variables
    assert!(
        ode.state_vars.contains(&"y".to_string()),
        "should have 'y', got {:?}",
        ode.state_vars
    );
    assert!(
        ode.state_vars.contains(&"v".to_string()),
        "should have 'v', got {:?}",
        ode.state_vars
    );

    // Should have non-empty derivative expressions
    for (i, expr) in ode.derivative_exprs.iter().enumerate() {
        assert!(!expr.is_empty(), "derivative {} should not be empty", i);
    }

    // Should detect GetOutput for kinetic_energy
    assert!(
        ode.signal_exprs.contains_key("kinetic_energy"),
        "should have kinetic_energy output, got {:?}",
        ode.signal_exprs.keys().collect::<Vec<_>>()
    );
}

#[test]
fn test_bouncing_ball_composite_unified_priority() {
    let graph = load_model("bouncing-ball", "BouncingBall.sysml");
    let compiler = ModelCompiler::new(graph);

    // Unified detection should use composite path (not individual)
    let odes = compiler.detect_all_odes_unified();
    assert_eq!(
        odes.len(),
        1,
        "should detect exactly 1 ODE (composite takes priority)"
    );
    assert_eq!(
        odes[0].tool_name, "ssr:ContinuousStateSpaceDynamics",
        "should be composite detection, not individual"
    );
}

#[test]
fn test_bouncing_ball_composite_simulation() {
    let graph = load_model("bouncing-ball", "BouncingBall.sysml");
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
        .expect("orchestrator should build");

    // Seed state vars
    orch.context
        .set("y".to_owned(), sysml_core::Value::Float(10.0));
    orch.context
        .set("v".to_owned(), sysml_core::Value::Float(0.0));

    // Step 100 ticks (100ms of free fall)
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

    println!("Composite simulation after 100ms: y={y:.3}m, v={v:.3}m/s");

    // Physics check: free fall from 10m for 0.1s
    // y ≈ 10 - 0.5*9.81*0.01 ≈ 9.951
    // v ≈ -9.81 * 0.1 ≈ -0.981
    assert!(
        y < 10.0 && y > 9.0,
        "y should decrease under gravity, got {y:.3}"
    );
    assert!(v < 0.0, "v should be negative (falling), got {v:.3}");
}

// -----------------------------------------------------------------------
// Discrete: Digital Filter with DiscreteStateSpaceDynamics
// -----------------------------------------------------------------------

#[test]
fn test_digital_filter_composite_detection() {
    let graph = load_model("digital-filter", "DigitalFilter.sysml");
    let compiler = ModelCompiler::new(graph);

    let composites = compiler.detect_composite_discrete_ssr();
    println!("Composite discrete detections: {}", composites.len());
    for (label, solver) in &composites {
        println!("  label={} vars={:?}", label.label, solver.state_names());
    }

    // Should detect at least the EMAFilter's EMADynamics
    assert!(
        !composites.is_empty(),
        "should detect DiscreteStateSpaceDynamics composite"
    );

    // Check that EMA filter has 'filtered' as state var
    let ema = composites.iter().find(|(claim, _)| claim.label.contains("EMA"));
    assert!(
        ema.is_some(),
        "should find EMAFilter, got labels: {:?}",
        composites.iter().map(|(c, _)| &c.label).collect::<Vec<_>>()
    );
    let (label, solver) = ema.unwrap();
    println!(
        "EMA filter: label={}, vars={:?}",
        label.label,
        solver.state_names()
    );
    assert!(
        solver.state_names().contains(&"filtered".to_string()),
        "EMA should have 'filtered' state var, got {:?}",
        solver.state_names()
    );

    // Check DualFilter has 'slow' and 'fast'
    let dual = composites.iter().find(|(claim, _)| claim.label.contains("Dual"));
    if let Some((_, solver)) = dual {
        assert!(
            solver.state_names().contains(&"slow".to_string()),
            "DualFilter should have 'slow'"
        );
        assert!(
            solver.state_names().contains(&"fast".to_string()),
            "DualFilter should have 'fast'"
        );
    }
}

#[test]
fn test_digital_filter_workspace_orchestrator() {
    let graph = load_model("digital-filter", "DigitalFilter.sysml");
    let compiler = ModelCompiler::new(graph);
    let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = std::sync::Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));

    let orch = compiler.build_workspace_orchestrator(
        base_ctx,
        Some(precompiled),
        None,
        None,
        None,
        &[],
        Some(1.0),
        Some(100.0),
    );
    match &orch {
        Ok(o) => {
            println!(
                "Digital filter orchestrator subsystems: {:?}",
                o.subsystem_names()
            );
        }
        Err(e) => {
            println!("Digital filter orchestrator error: {}", e);
        }
    }
    // At minimum, orchestrator should build (even if no SM present)
    assert!(
        orch.is_ok(),
        "workspace orchestrator should build for digital filter"
    );
}
