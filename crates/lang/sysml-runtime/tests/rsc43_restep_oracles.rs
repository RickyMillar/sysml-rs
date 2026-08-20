//! RSC-4.3 — time-accurate zero-crossing re-step ORACLES.
//!
//! These pin the CONFORMS-required behaviour of the re-step fix (plan
//! `rsc-4.3-time-accurate-restep.md` § Oracles), on self-contained analytic
//! models. Every oracle here uses an inline, analytically-known fixture so the
//! measured quantity is a pure event-location artifact with no dependence on
//! any product model. (The device-period-convergence oracle that originally
//! accompanied these lives with the espresso pump hybrid gate,
//! `espresso_pump_hybrid::pump_hyb_02_event_time_converges_as_dt_shrinks`.)
//!
//!   (b) OVERSHOOT-BOUNDED — steady-state peak overshoot past the flip threshold
//!       is bounded by `K × bisection_tol`, dt-INDEPENDENT — NOT ∝ dt. Uses an
//!       analytically-known relay oscillator so the overshoot is a pure
//!       event-location artifact (linear segments, exact under any solver).
//!   (c) NO-CROSSING behaviour is unchanged — a model whose detector never
//!       fires takes the exact current code path.
//!   (freeze) STEP-COUNT freeze regression — a located crossing past the
//!       10_000-tick step budget still fires on the general orchestrator path.

use std::sync::Arc;

use sysml_core::{elaborate, Value};
use sysml_ide_db::eval_context_seed;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::Orchestrator;

/// bisection tolerance in `ode_events.rs` (`ZeroCrossingDetector::default`).
const BISECTION_TOL: f64 = 1e-6;

// ===========================================================================
// (b) Overshoot-bounded, dt-independent — relay oscillator.
// ===========================================================================

/// A single-shot relay: `dx/dt = drive`, starting `drive = +1`. One located
/// crossing (`accept when x >= 1`) flips `drive` to −1. The segment is linear
/// (exact under any explicit solver), so the PEAK of `x` past the threshold is
/// a pure event-location artifact: today `x` runs a tick's worth past 1 before
/// the flip takes effect (∝ dt); post-fix the flip lands at the located crossing
/// so the peak is bounded by the crossing residual (dt-independent). Single-shot
/// avoids the detector re-arm path so the oracle isolates ONE crossing cleanly.
const RELAY_MODEL: &str = r#"
package Relay {
    private import ScalarValues::*;

    part def Osc {
        attribute drive : Real default 1.0;
        out attribute x : Real default 0.0;

        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative {
                return dxdt = drive;
            }
        }
    }

    state def Flipper {
        in attribute x : Real;

        state rising {
            entry action { drive = 1.0; }
        }
        state falling {
            entry action { drive = 0.0 - 1.0; }
        }

        entry; then rising;

        transition rising_to_falling
            first rising
            accept when x >= 1.0
            then falling;
    }
}
"#;

fn build_relay(dt_ms: f64, max_ms: f64) -> Orchestrator {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("Relay.sysml", RELAY_MODEL.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let mut orch = compiler
        .build_workspace_orchestrator(
            base_ctx, None, None, None, None, &[], Some(dt_ms), Some(max_ms),
        )
        .expect("relay orchestrator builds");
    orch.context.set("x".to_owned(), Value::Float(0.0));
    orch.context.set("drive".to_owned(), Value::Float(1.0));
    orch
}

/// Peak overshoot of `x` past the `x = 1` flip threshold on the single-shot
/// relay: run past the crossing (which occurs at t = 1000 ms since dx/dt = 1),
/// well before the horizon, and return `max(x) − 1`.
fn relay_flip_overshoot(dt_ms: f64) -> f64 {
    // Cross at t=1000ms; run to t=2000ms so the flip has fully taken effect and
    // x has turned around, but the horizon (5000ms) is nowhere near.
    let max_ms = 5_000.0;
    let mut orch = build_relay(dt_ms, max_ms);
    let stop = 2_000.0;
    let n = (stop / dt_ms) as usize + 10;
    let mut peak = f64::MIN;
    let mut prev_x = 0.0_f64;
    for _ in 0..n {
        let _ = orch.step();
        if orch.time_ms() > stop {
            break;
        }
        let x = match orch.context.get("x") {
            Some(Value::Float(f)) => *f,
            _ => prev_x,
        };
        if x > peak {
            peak = x;
        }
        prev_x = x;
    }
    peak - 1.0
}

// The overshoot oracle is the CLEANEST proof of the re-step: the relay's
// crossing is on the STATE `x` directly (`accept when x >= 1`), and `dx/dt` is
// piecewise-constant, so the located crossing is exact under any explicit
// solver and the peak of `x` past 1 is a PURE event-location artifact.
//   * Pre-RSC-4.3: the flip fired at tick-end, so `x` ran a full tick past 1
//     before the drive reversed — overshoot ≈ (dt/1000)·|drive| ∝ dt (≈1e-3 at
//     dt=1ms). (And post-step_count-fix the fine-dt crossing now FIRES — before
//     the step_count freeze it was missed above tick 10_000, the "detector-miss"
//     that had this oracle blocked.)
//   * Post-RSC-4.3: the flip fires AT the located crossing (bisection residual),
//     so the peak is bounded by ~bisection_tol and is dt-INDEPENDENT. `x` in fact
//     turns around at/just-before 1, so the signed overshoot is ~machine-epsilon
//     (slightly negative) — hence the `.abs()` bound below.
#[test]
fn oracle_b_overshoot_bounded_and_dt_independent() {
    // The flip fires AT the located `x=1` crossing, so |peak−1| is bounded by
    // the crossing residual (~bisection_tol), NOT the step. K absorbs the
    // residual + one continuation increment.
    const K: f64 = 100.0;
    let bound = K * BISECTION_TOL; // 1e-4

    let over_coarse = relay_flip_overshoot(1.0); // dt = 1 ms
    let over_fine = relay_flip_overshoot(0.1); // dt = 0.1 ms

    // (b.1) Absolute bound at BOTH steps. This single check is the
    // dt-independence proof: the pre-fix disease gives overshoot ≈ 1e-3 at
    // dt=1ms (∝ dt) — three orders of magnitude ABOVE this bound — so a coarse
    // overshoot within `bound` can only be event-located, not tick-quantized.
    assert!(
        over_coarse.abs() < bound,
        "coarse-dt (1ms) flip overshoot {over_coarse:.3e} must be within ±{bound:.0e} \
         (K×bisection_tol). The ∝dt disease gives ≈1e-3 here."
    );
    assert!(
        over_fine.abs() < bound,
        "fine-dt (0.1ms) flip overshoot {over_fine:.3e} must be within ±{bound:.0e}."
    );

    // (b.2) dt-INDEPENDENCE, explicit: a 10× smaller step must NOT give a ~10×
    // smaller overshoot (the ∝dt signature). Post-fix both are residual-sized
    // (~machine-epsilon), so the coarse magnitude is NOT ~10× the fine one.
    // Guard the ratio against the near-zero denominator.
    if over_fine.abs() > 1e-12 {
        let ratio = over_coarse.abs() / over_fine.abs();
        assert!(
            ratio < 3.0,
            "overshoot must be dt-INDEPENDENT: coarse/fine magnitude ratio {ratio:.2} \
             (coarse={over_coarse:.3e}, fine={over_fine:.3e}). A ratio ≈10 means it is \
             still ∝ dt (the pre-fix disease)."
        );
    }
}

// ===========================================================================
// (c) No firing crossing — behaviour unchanged (structural guarantee).
// ===========================================================================

/// A pure ramp whose `accept when x >= 0.5` crossing is at t=500ms; sampled only
/// through tick 100 → the detector is REGISTERED but NEVER fires, so the model
/// must take the exact current (no-re-step) code path. This is a local
/// witness that a present-but-unfired detector changes nothing.
const RAMP_NO_CROSS: &str = r#"
package RampNoCross {
    private import ScalarValues::*;
    part def Ramp {
        attribute v : Real default 1.0;
        out attribute x : Real default 0.0;
        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative { return dxdt = v; }
        }
    }
    state def Mover {
        in attribute x : Real;
        state low; state high;
        entry; then low;
        transition low_to_high first low accept when x >= 0.5 then high;
    }
}
"#;

fn build_ramp_no_cross() -> Orchestrator {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("RampNoCross.sysml", RAMP_NO_CROSS.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let mut orch = compiler
        .build_workspace_orchestrator(base_ctx, None, None, None, None, &[], Some(1.0), Some(400.0))
        .expect("ramp orchestrator builds");
    orch.context.set("x".to_owned(), Value::Float(0.0));
    orch
}

#[test]
fn oracle_c_present_but_unfired_crossing_is_unchanged() {
    // The detector must be present (proves the branch guard is exercised).
    let mut probe = build_ramp_no_cross();
    let _ = probe.step();
    assert_eq!(
        probe.crossing_detector_event_count("Ramp"),
        1,
        "the no-cross ramp must still register its `accept when` detector"
    );

    // Through tick 100 the crossing (t=500ms) never fires: x integrates
    // untouched and the SM stays in `low`.
    let mut orch = build_ramp_no_cross();
    for _ in 0..100 {
        let _ = orch.step();
    }
    let final_x = orch.context.get("x").map(|v| format!("{v:?}")).unwrap_or_default();
    assert_eq!(
        final_x, "Float(0.10000000000000007)",
        "no-firing-crossing ramp must integrate to the untouched value at tick 100; got {final_x}"
    );
    let mover_state = orch
        .subsystems()
        .iter()
        .find(|s| s.name == "Mover")
        .map(|s| s.executor.current_state_name().to_owned());
    assert_eq!(
        mover_state.as_deref(),
        Some("low"),
        "no crossing should have fired (x<0.5 through tick 100)"
    );
}

// ===========================================================================
// (Step 1) step_count freeze regression — orchestrator-driven SM must keep
// transitioning PAST 10_000 ticks. Live gate (independent of the re-step arc).
// ===========================================================================

/// Regression for the step_count accumulation freeze (mod.rs `Executor::tick`).
///
/// `Runner`'s `max_steps` budget (default 10_000) is meant to bound the
/// WITHIN-tick run-to-completion auto-chain. Because the orchestrator drives
/// each SM executor once per tick and `step_count` was never reset between
/// ticks, the counter accumulated one per tick and tripped
/// `step_count >= max_steps` after 10_000 ticks — after which `step_inner`
/// early-returned "step limit exceeded" and SILENTLY froze EVERY transition.
/// Any simulation longer than 10_000 ticks (e.g. a long hybrid run in the GUI,
/// or any fine-dt hybrid model) stopped delivering located `accept when` crossings.
///
/// A harmonic oscillator x(t)=sin(t) with `accept when x >= 0.9` crosses the
/// threshold at t = arcsin(0.9) ≈ 1.1198 s = tick 11198 at dt=0.1ms — strictly
/// PAST the 10_000-tick budget. Pre-fix this transition never fired; post-fix it
/// fires at its true tick. (The sub-budget crossing x>=0.5 at tick ~5236 always
/// fired, which is why the freeze hid for so long.)
#[test]
fn step_count_freeze_regression_sm_fires_past_10k_ticks() {
    const HARM_PAST_BUDGET: &str = r#"
package HarmPastBudget {
    private import ScalarValues::*;
    part def Osc {
        attribute sign : Real default 1.0;
        out attribute x : Real default 0.0;
        out attribute v : Real default 1.0;
        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative { return dxdt = v; }
            calc def VDeriv :> GetDerivative { return dvdt = 0.0 - x; }
        }
    }
    state def Flipper {
        in attribute x : Real;
        state a { entry action { sign = 1.0; } }
        state b { entry action { sign = 0.0 - 1.0; } }
        entry; then a;
        transition a_to_b first a accept when x >= 0.9 then b;
    }
}
"#;
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new(
        "HarmPastBudget.sysml",
        HARM_PAST_BUDGET.to_owned(),
    )]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let mut orch = compiler
        .build_workspace_orchestrator(base_ctx, None, None, None, None, &[], Some(0.1), Some(3000.0))
        .expect("harmonic orchestrator builds");
    orch.context.set("x".to_owned(), Value::Float(0.0));
    orch.context.set("v".to_owned(), Value::Float(1.0));

    let mut fired_tick: Option<usize> = None;
    for i in 0..15_000 {
        let _ = orch.step();
        let st = orch
            .subsystems()
            .iter()
            .find(|s| s.name == "Flipper")
            .map(|s| s.executor.current_state_name().to_owned())
            .unwrap_or_default();
        if st == "b" {
            fired_tick = Some(i);
            break;
        }
    }

    let fired = fired_tick.expect(
        "SM must transition on the located x>=0.9 crossing — pre-fix it froze silently after \
         tick 10_000 (step_count budget) and this never fired",
    );
    assert!(
        fired > 10_000,
        "the crossing is at tick ~11197 (past the 10_000-tick budget); firing at tick {fired} \
         proves the freeze is gone on the GENERAL orchestrator path, not just sub-budget crossings"
    );
}
