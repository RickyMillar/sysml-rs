//! WS-A2 gate (hybrid-sim-robustness-plan §5): the compiler auto-routes
//! `accept when <comparator>` state-machine triggers over continuous ODE state
//! through zero-crossing event LOCATION, so a transition fires at the located
//! crossing instant instead of a missed tick-boundary rising edge.
//!
//! Generic — nothing fixture-specific. A linear-ramp ODE (`dx/dt = v`, exact under
//! any explicit solver, so the test isolates EVENT LOCATION from solver
//! accuracy) drives a 3-state machine through:
//!   - a STATE-VARIABLE comparator   `accept when x >= 0.5`
//!   - an OUTPUT-SIGNAL comparator   `accept when sig >= 4.5`  (sig = 3*x)
//! The signal comparator exercises the signal-recompute path (the residual is
//! re-evaluated from the interpolated state at each bisection point, not read
//! stale from the post-step context).

use sysml_core::{elaborate, Value};
use sysml_ide_db::eval_context_seed;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

const MODEL: &str = r#"
package RampCross {
    private import ScalarValues::*;

    part def Ramp {
        attribute v : Real default 1.0;
        out attribute x : Real default 0.0;
        out attribute sig : Real default 0.0;

        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative {
                return dxdt = v;
            }
            calc def SigOut :> GetOutput {
                return sig = 3.0 * x;
            }
        }
    }

    state def Mover {
        in attribute x : Real;
        in attribute sig : Real;

        state low;
        state mid;
        state high;

        entry; then low;

        transition low_to_mid
            first low
            accept when x >= 0.5
            then mid;

        transition mid_to_high
            first mid
            accept when sig >= 4.5
            then high;
    }
}
"#;

/// Build the workspace orchestrator for the ramp+mover model at the given step.
fn build(dt_ms: f64, max_ms: f64) -> sysml_runtime::orchestrator::Orchestrator {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("RampCross.sysml", MODEL.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    compiler
        .build_workspace_orchestrator(
            base_ctx,
            None,
            None,
            None,
            None,
            &[],
            Some(dt_ms),
            Some(max_ms),
        )
        .expect("workspace orchestrator builds")
}

#[test]
fn accept_when_continuous_comparators_are_crossing_located() {
    // 1 ms steps; v = 1.0 (units/s), so x advances 0.001 per tick. Crossings:
    //   x >= 0.5  at t = 500 ms
    //   sig = 3x >= 4.5  ⇔  x >= 1.5  at t = 1500 ms
    let dt_ms = 1.0;
    let mut orch = build(dt_ms, 2500.0);

    // The ODE state must start at x = 0 (seed Float over any Ref binding).
    orch.context.set("x".to_owned(), Value::Float(0.0));

    // (a) The compiler registered both `accept when` comparators as located
    //     crossings on the ODE detector (state-var + signal).
    assert_eq!(
        orch.crossing_detector_event_count("Ramp"),
        2,
        "both `accept when` comparators should be wired as located crossings"
    );

    // (b) Run and record when each state is first entered.
    let mut entered_mid_ms: Option<f64> = None;
    let mut entered_high_ms: Option<f64> = None;
    for _ in 0..2500 {
        let snap = orch.step();
        if let Some(st) = snap
            .subsystem_states
            .get("Mover")
            .map(|s| s.current_state.clone())
        {
            let t = orch.time_ms();
            if st == "mid" && entered_mid_ms.is_none() {
                entered_mid_ms = Some(t);
            }
            if st == "high" && entered_high_ms.is_none() {
                entered_high_ms = Some(t);
            }
        }
    }

    // (c) Both transitions fired — the located crossing drove them.
    let mid = entered_mid_ms.expect("SM should reach `mid` via the x>=0.5 crossing");
    let high = entered_high_ms.expect("SM should reach `high` via the sig>=4.5 crossing");

    // (d) Each fired near its analytic crossing time (≤ ~2 ticks late — the
    //     located crossing + injection delay, far tighter than freezing).
    assert!(
        (mid - 500.0).abs() <= 2.0 * dt_ms,
        "x>=0.5 crossing should fire near t=500ms, fired at {mid}ms"
    );
    assert!(
        (high - 1500.0).abs() <= 2.0 * dt_ms,
        "sig>=4.5 crossing should fire near t=1500ms, fired at {high}ms"
    );
}
