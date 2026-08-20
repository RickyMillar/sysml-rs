//! Runtime gates for the `espresso-pump-hybrid` synthetic fixture.
//!
//! Each test names the replacement coverage-matrix capability it covers and the
//! plan §10.1 layered-gate id (PUMP-DATA/ODE/SM/HYB/SAFE). All oracles are analytic /
//! invariant — no old device value, waveform, table shape, or numeric assertion
//! is used. The physics and bounds are derived in examples/espresso-pump-hybrid/README.md.

mod common;

use sysml_core::{elaborate, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

const FIXTURE: &str = "espresso-pump-hybrid";
const OMEGA: f64 = 6.2832; // actuator angular frequency (must match PumpODE.sysml)

// --- helpers ----------------------------------------------------------------

/// Step the fixture and return (final snapshot vars as a lookup, first tick the
/// PumpCycle SM entered `relieved`, the ordered list of distinct SM states seen,
/// and the peak accumulator/chamber pressures + peak actuator-energy drift).
struct RunResult {
    relief_ms: Option<f64>,
    state_seq: Vec<String>,
    pa_max: f64,
    pc_max: f64,
    energy_rel_drift: f64,
    all_finite: bool,
}

fn run(overrides: &[(String, String)], dt_ms: f64, horizon_ms: f64) -> RunResult {
    let mut orch = common::load_example_orchestrator(FIXTURE, overrides, dt_ms, horizon_ms);
    let n = (horizon_ms / dt_ms) as usize;
    let mut relief_ms = None;
    let mut state_seq: Vec<String> = Vec::new();
    let (mut pa_max, mut pc_max) = (f64::MIN, f64::MIN);
    let (mut e0, mut e_min, mut e_max) = (f64::NAN, f64::MAX, f64::MIN);
    let mut all_finite = true;
    for _ in 0..n {
        let snap = orch.step();
        let g = |k: &str| {
            snap.variables
                .iter()
                .find(|(nm, _)| nm.as_str() == k)
                .and_then(|(_, v)| match v {
                    Value::Float(f) => Some(*f),
                    Value::Int(i) => Some(*i as f64),
                    _ => None,
                })
                .unwrap_or(f64::NAN)
        };
        let (stroke, vel, pc, pa, z) = (
            g("stroke"),
            g("velocity"),
            g("chamberPressure"),
            g("accumulatorPressure"),
            g("exposure"),
        );
        for x in [stroke, vel, pc, pa, z] {
            if !x.is_finite() {
                all_finite = false;
            }
        }
        pa_max = pa_max.max(pa);
        pc_max = pc_max.max(pc);
        // Undamped-actuator mechanical energy: 0.5 v^2 + 0.5 omega^2 x^2.
        let e = 0.5 * vel * vel + 0.5 * OMEGA * OMEGA * stroke * stroke;
        if e0.is_nan() {
            e0 = e;
        }
        e_min = e_min.min(e);
        e_max = e_max.max(e);
        if let Some(st) = snap.subsystem_states.get("PumpCycle") {
            let s = st.current_state.clone();
            if state_seq.last() != Some(&s) {
                state_seq.push(s.clone());
            }
            if s == "relieved" && relief_ms.is_none() {
                relief_ms = Some(orch.time_ms());
            }
        }
    }
    let energy_rel_drift = (e_max - e_min) / e0.abs().max(1e-9);
    RunResult { relief_ms, state_seq, pa_max, pc_max, energy_rel_drift, all_finite }
}

fn parse_graph(model: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("Inline.sysml".to_owned(), model.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    graph
}

// ---------------------------------------------------------------------------
// PUMP-DATA-02 / RT-ODE-SF — CSV-backed sampled function resolves, endpoints,
// interpolation matches the generating equation, and the two branches differ.
// ---------------------------------------------------------------------------

fn smoothstep(u: f64) -> f64 {
    u * u * (3.0 - 2.0 * u)
}

#[test]
fn pump_data_02_sampled_function_endpoints_and_equation_match() {
    let graph = common::load_example_graph(FIXTURE);
    let dir = common::workspace_example_dir(FIXTURE);
    let compiler = ModelCompiler::new(graph).with_source_dir(&dir);
    let sfs = compiler
        .extract_sampled_functions()
        .expect("sampled functions load from @DataSource CSVs");

    let get = |name: &str| -> (Vec<f64>, Vec<f64>) {
        let (_, v) = sfs.iter().find(|(n, _)| n == name).expect("branch present");
        let Value::Map(m) = v else { panic!("sampled function is a Map") };
        let as_vec = |k: &str| match m.get(k) {
            Some(Value::List(xs)) => xs
                .iter()
                .map(|x| match x {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => panic!("numeric"),
                })
                .collect::<Vec<_>>(),
            _ => panic!("{k} list"),
        };
        (as_vec("domain"), as_vec("range"))
    };

    let (od, or) = get("openingBranch");
    let (cd, cr) = get("closingBranch");

    // 64-row grids, strictly increasing domain over [0, 1].
    assert_eq!(od.len(), 64);
    assert_eq!(cd.len(), 64);
    assert!((od[0] - 0.0).abs() < 1e-9 && (od[63] - 1.0).abs() < 1e-9);
    assert!(od.windows(2).all(|w| w[1] > w[0]), "opening domain strictly increasing");

    // Endpoints pinned: A_min = 0.02, A_max = 1.0 on both branches.
    for (name, r) in [("opening", &or), ("closing", &cr)] {
        assert!((r[0] - 0.02).abs() < 1e-6, "{name}(0) == A_min");
        assert!((r[63] - 1.0).abs() < 1e-6, "{name}(1) == A_max");
        // Monotone non-decreasing range.
        assert!(r.windows(2).all(|w| w[1] >= w[0] - 1e-9), "{name} range monotone");
    }

    // Opening range matches the generating smoothstep equation at every grid point.
    for (u, a) in od.iter().zip(or.iter()) {
        let expected = 0.02 + (1.0 - 0.02) * smoothstep(*u);
        assert!((a - expected).abs() < 1e-5, "opening({u}) matches equation");
    }

    // Branch distinction: the opening branch dominates the closing branch in the
    // interior, and the separation vanishes at the endpoints.
    let mut max_sep = 0.0_f64;
    for i in 0..64 {
        let sep = or[i] - cr[i];
        assert!(sep >= -1e-9, "opening >= closing at i={i}");
        max_sep = max_sep.max(sep);
    }
    assert!((or[0] - cr[0]).abs() < 1e-9 && (or[63] - cr[63]).abs() < 1e-9, "separation vanishes at endpoints");
    assert!(max_sep > 1e-2, "branch separation is observable (got {max_sep})");
}

#[test]
fn pump_data_02_missing_data_source_fails_hard() {
    // RT-ODE-SF fail-hard: a declared @DataSource that cannot be resolved is a
    // precise compile error, never a silent empty table.
    const BAD: &str = r#"
package BadSource {
    private import ScalarValues::*;
    private import SampledFunctions::*;
    metadata def DataSource { attribute file : String; }
    attribute brokenBranch : SampledFunction {
        @DataSource { file = "data/does_not_exist.csv"; }
    }
}
"#;
    let graph = parse_graph(BAD);
    let dir = common::workspace_example_dir(FIXTURE); // a real dir; the file is absent
    let compiler = ModelCompiler::new(graph).with_source_dir(&dir);
    let err = compiler
        .extract_sampled_functions()
        .expect_err("a missing @DataSource file must fail hard");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("DataSource") || msg.to_lowercase().contains("does_not_exist"),
        "error should name the unresolved data source, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// RT-ODE-CORE / PUMP-ODE-01 — ODE detected from SSR metadata; derivative
// evaluation is finite; a state provably moves from its default IC.
// ---------------------------------------------------------------------------

#[test]
fn pump_ode_core_detection_and_state_vars() {
    let graph = common::load_example_graph(FIXTURE);
    let compiler = ModelCompiler::new(graph);
    let composites = compiler.detect_composite_continuous_ssr();
    let ode = composites
        .iter()
        .find(|c| c.name.as_deref() == Some("ReciprocatingPump"))
        .expect("ReciprocatingPump ODE detected from ContinuousStateSpaceDynamics");
    for v in ["stroke", "velocity", "chamberPressure", "accumulatorPressure", "exposure"] {
        assert!(ode.state_vars.contains(&v.to_string()), "state var {v} present");
    }
    for sig in ["aEff", "qOut", "u"] {
        assert!(ode.signal_exprs.contains_key(sig), "signal {sig} present");
    }
}

#[test]
fn pump_ode_01_finite_derivatives_and_state_motion() {
    let r = run(&[], 2.0, 2000.0);
    assert!(r.all_finite, "all states remain finite (no NaN/Inf)");
    // Chamber pressure starts at its default 0.0 and provably moves.
    assert!(r.pc_max > 0.1, "chamberPressure moves from its default IC (pc_max={})", r.pc_max);
}

// ---------------------------------------------------------------------------
// RT-ODE-CORE / PUMP-ODE-02 — conservation invariant within declared tolerance.
// The unforced actuator is an undamped harmonic oscillator, so its mechanical
// energy 0.5 v^2 + 0.5 omega^2 x^2 is conserved; bounded drift is the oracle.
// ---------------------------------------------------------------------------

#[test]
fn pump_ode_02_actuator_energy_conservation() {
    let r = run(&[], 1.0, 4000.0);
    assert!(
        r.energy_rel_drift < 0.02,
        "actuator mechanical energy conserved within 2% over 4 s (drift={})",
        r.energy_rel_drift
    );
}

// ---------------------------------------------------------------------------
// RT-SM / PUMP-SM-01 — the cycle traverses Idle->Intake->Compress->Discharge->
// Recover->Intake in legal order, with no illegal branch, and `relieved` is
// only reached via the safety path.
// ---------------------------------------------------------------------------

const CYCLE: [&str; 4] = ["intake", "compress", "discharge", "recover"];

fn assert_legal_cycle(seq: &[String]) {
    // First state is idle or intake (Idle -> Intake initial branch).
    assert!(matches!(seq[0].as_str(), "idle" | "intake"), "starts idle/intake, got {}", seq[0]);
    // Every non-relief transition follows the cyclic order; relief is terminal.
    let mut idx: Option<usize> = None;
    let mut relieved = false;
    for s in seq {
        assert!(!relieved, "no state entered after `relieved` (it is terminal)");
        match s.as_str() {
            "idle" => {}
            "relieved" => relieved = true,
            other => {
                let pos = CYCLE.iter().position(|c| *c == other).expect("legal cycle state");
                if let Some(prev) = idx {
                    assert_eq!(pos, (prev + 1) % 4, "cycle advances by exactly one step: {seq:?}");
                }
                idx = Some(pos);
            }
        }
    }
}

#[test]
fn pump_sm_01_legal_cycle_order_nominal() {
    let r = run(&[], 2.0, 4000.0);
    assert!(r.state_seq.len() >= 6, "several cycle states visited");
    assert_legal_cycle(&r.state_seq);
    assert!(!r.state_seq.iter().any(|s| s == "relieved"), "nominal never relieves");
}

// ---------------------------------------------------------------------------
// RT-HYBRID / PUMP-HYB-01 — the eight `accept when` comparators are wired as
// located zero-crossings; a crossing drives the SM transition; cycle stays legal.
// ---------------------------------------------------------------------------

#[test]
fn pump_hyb_01_crossings_located_and_drive_transitions() {
    let orch = common::load_example_orchestrator(FIXTURE, &[], 2.0, 100.0);
    assert_eq!(
        orch.crossing_detector_event_count("ReciprocatingPump"),
        8,
        "4 cycle + 4 relief `accept when` comparators are located crossings"
    );
    // Under severe restriction the located exposure crossing drives the SM into
    // `relieved`, and the cycle order remains legal up to that point.
    let r = run(&[("restrictionConductance".into(), "0.3".into())], 2.0, 8000.0);
    assert!(r.relief_ms.is_some(), "severe restriction drives the relief crossing");
    assert_legal_cycle(&r.state_seq);
}

// ---------------------------------------------------------------------------
// RT-RESTEP / PUMP-HYB-02 — the located relief event time converges as dt
// shrinks (dt-independence in the resolved regime); measured, not compared to a
// stored number.
// ---------------------------------------------------------------------------

#[test]
fn pump_hyb_02_event_time_converges_as_dt_shrinks() {
    let sev = [("restrictionConductance".to_string(), "0.3".to_string())];
    let coarse = run(&sev, 4.0, 8000.0).relief_ms.expect("relief at dt=4ms");
    let fine = run(&sev, 1.0, 8000.0).relief_ms.expect("relief at dt=1ms");
    let finer = run(&sev, 0.5, 8000.0).relief_ms.expect("relief at dt=0.5ms");
    // Successive halvings change the located event time by a vanishing amount
    // (well under one coarse step), i.e. it is converging, not dt-dependent.
    assert!((coarse - fine).abs() < 4.0, "dt 4->1 ms event time stable: {coarse} vs {fine}");
    assert!((fine - finer).abs() < 1.0, "dt 1->0.5 ms event time converged: {fine} vs {finer}");
}

// ---------------------------------------------------------------------------
// VER-SIM / PUMP-SAFE-01 — nominal restriction does not latch relief, and the
// exposure stays below the trip level.
// ---------------------------------------------------------------------------

#[test]
fn pump_safe_01_nominal_does_not_relieve() {
    let r = run(&[], 2.0, 6000.0);
    assert!(r.relief_ms.is_none(), "nominal never latches relief");
    // The accumulator settles below the warning threshold (pWarning = 0.5), so
    // the exposure integral never accumulates.
    assert!(r.pa_max < 0.5, "nominal accumulator stays below pWarning (pa_max={})", r.pa_max);
}

// ---------------------------------------------------------------------------
// VER-SIM / PUMP-SAFE-02 — severe restriction latches relief ONLY after the
// modeled dwell, and within an analytically justified interval.
// ---------------------------------------------------------------------------

#[test]
fn pump_safe_02_severe_relieves_after_dwell_within_bound() {
    let r = run(&[("restrictionConductance".into(), "0.3".into())], 2.0, 8000.0);
    let t = r.relief_ms.expect("severe restriction latches relief within the horizon");

    // Anti-chatter/debounce lower bound: exposure z(t) <= (pa_max - pWarning)*t,
    // so relief (z = exposureTrip = 1.0) cannot occur before
    //   t_lo = exposureTrip / (pa_max - pWarning).
    // pWarning = 0.5; pa_max is measured from this very run.
    let t_lo_ms = 1.0 / (r.pa_max - 0.5) * 1000.0;
    assert!(
        t > t_lo_ms,
        "relief cannot precede the exposure-integral dwell: t={t}ms, t_lo={t_lo_ms}ms"
    );
    // And it does occur strictly inside the horizon.
    assert!(t < 8000.0, "relief latches within the horizon");
    // Severe accumulator settles ABOVE the warning threshold (the driver of exposure).
    assert!(r.pa_max > 0.5, "severe accumulator exceeds pWarning (pa_max={})", r.pa_max);
}

// ---------------------------------------------------------------------------
// PUMP-SAFE-03 — the README's two MEASURED severe constants match what the
// model actually produces.
//
// Why this gate exists: every other assertion here derives its bound from the
// run it just made (`t_lo` is computed from that run's own `pa_max`), which is
// the right way to state an invariant — but it means the numbers quoted in
// `examples/espresso-pump-hybrid/README.md` were never checked against
// anything. They had drifted: the README described relief at "≈ 3.19 s" and a
// severe "p_a,max ≈ 2.2" while the model produces ≈ 3.82 s and ≈ 1.62. The
// safety argument was unaffected (t_lo ≈ 0.89 s either way, and 3.82 > 0.89),
// but a reader comparing the UI against the README saw a 600 ms disagreement
// and had no way to tell which side was wrong.
//
// The tolerances are loose enough to survive ordinary solver noise and tight
// enough that another 600 ms of drift fails here instead of in a user's head.
// ---------------------------------------------------------------------------

#[test]
fn pump_safe_03_readme_measured_constants_still_hold() {
    let sev = [("restrictionConductance".to_string(), "0.3".to_string())];
    let r = run(&sev, 2.0, 8000.0);
    let t = r.relief_ms.expect("severe latches relief");

    assert!(
        (t - 3820.0).abs() < 50.0,
        "README documents severe relief at ≈3.82 s; model produced {t}ms.          Re-measure and update README.md §'Restriction scenarios' together with this bound."
    );
    assert!(
        (r.pa_max - 1.62).abs() < 0.05,
        "README documents severe p_a,max ≈1.62; model produced {}.          Re-measure and update README.md's scenario table together with this bound.",
        r.pa_max
    );

    // Nominal's documented peak, same reasoning.
    let nom = run(&[], 2.0, 6000.0);
    assert!(
        nom.pa_max < 0.5,
        "README documents nominal settling below pWarning; got {}",
        nom.pa_max
    );
}

