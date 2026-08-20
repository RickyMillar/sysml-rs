//! RSC-6.2 equivalence gate: the prepared single-SM ODE path is byte-identical
//! to the monolithic `build_orchestrator`.
//!
//! `ode_sweep` was re-running the graph-invariant work (SM compile, the
//! full-graph SSR ODE detection, derivative parse) on every sweep variant via
//! `build_orchestrator`. RSC-6.2 splits that into `prepare_single_ode` (paid
//! once) + `build_orchestrator_from_prepared` (per variant). This gate proves
//! the refactor changed nothing observable: for a range of parameter
//! overrides, assembling from a SINGLE reused `prepared` produces the same
//! simulation as calling the original `build_orchestrator` fresh each time.
//!
//! It's an *equivalence* gate (path A vs path B on the same in-memory graph),
//! not a golden-output gate, so it is robust to unrelated model changes.

use sysml_core::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

/// Minimal self-contained single-SM hybrid model: a first-order linear decay
/// ODE (`dx/dt = -k * x`) with a swept parameter `k`, plus a trivial state
/// machine so the single-SM `build_orchestrator(sm_name, ...)` path engages.
/// Parses standalone (no stdlib) — SSR detection is structural, matching
/// `:> GetDerivative` by name (same as the BouncingBall pipeline test).
const MODEL: &str = r#"
package HybridSweep {
    part def Decay {
        attribute k : Real default 1.0;
        out attribute x : Real default 10.0;

        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative {
                return dxdt = 0 - k * x;
            }
        }
    }

    state def Runner {
        state active;
        state idle;
    }
}
"#;

fn compiler() -> ModelCompiler {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("HybridSweep.sysml", MODEL.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    ModelCompiler::new(graph)
}

/// Final state-variable values keyed by name, the observable result of a run.
fn final_x(snap: &Option<sysml_runtime::orchestrator::ExecutionSnapshot>) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = snap
        .as_ref()
        .map(|s| {
            s.variables
                .iter()
                .filter_map(|(k, v)| match v {
                    sysml_core::Value::Float(f) => Some((k.clone(), *f)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn prepared_path_matches_monolithic_build_orchestrator() {
    let compiler = compiler();

    // Sanity: the model really does compile a single-SM ODE orchestrator.
    let probe = compiler
        .build_orchestrator("Runner", &[], Some(10.0), Some(2000.0))
        .expect("single-SM ODE orchestrator builds from the minimal model");
    drop(probe);

    // Prepare ONCE — this is what `ode_sweep` now does before its loop.
    let prepared = compiler
        .prepare_single_ode("Runner")
        .expect("prepare_single_ode succeeds");

    // Sweep `k` across several values. For each, the reused-prepared assembly
    // must produce the identical final state as a fresh monolithic build.
    let dt = Some(10.0);
    let max_t = Some(2000.0);
    for k in ["0.5", "1.0", "2.0", "5.0", "10.0"] {
        let overrides = vec![("k".to_owned(), k.to_owned())];

        // Path A: the original monolithic build (recompiles per call).
        let mut orch_a = compiler
            .build_orchestrator("Runner", &overrides, dt, max_t)
            .expect("monolithic build");
        let snap_a = orch_a.run_to_completion();

        // Path B: assemble from the single reused `prepared`.
        let mut orch_b = compiler
            .build_orchestrator_from_prepared(&prepared, &overrides, dt, max_t)
            .expect("prepared build");
        let snap_b = orch_b.run_to_completion();

        assert_eq!(
            final_x(&snap_a),
            final_x(&snap_b),
            "prepared assembly diverged from monolithic build for k={k}"
        );
        // The decay must actually have moved (guards against a vacuous all-zero
        // match that would hide a regression).
        let xa: f64 = final_x(&snap_a)
            .into_iter()
            .find(|(n, _)| n == "x")
            .map(|(_, v)| v)
            .expect("state var x present in snapshot");
        assert!(
            xa < 10.0,
            "x should decay below its initial 10.0 (k={k}, got {xa})"
        );
    }
}
