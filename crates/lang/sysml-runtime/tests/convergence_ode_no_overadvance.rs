//! GAP 3 regression: the orchestrator convergence loop must not perturb a
//! continuous ODE trajectory.
//!
//! Root cause (confirmed): `build_workspace_orchestrator` auto-enables
//! `convergence_max_iterations = 3` whenever a model has physics + ODE + SM
//! all present (orchestrator_build.rs step 7b). The convergence loop re-ticks
//! every subsystem each iteration to resolve *algebraic* multi-domain coupling
//! to a fixpoint. But a continuous-dynamics executor is a *stateful time
//! integrator*: `ContinuousDynamicsExecutor::tick` / `Rk4Solver::tick` advance
//! `self.state` by one `dt` from its CURRENT value on every call, and
//! `sync_context_in` never resets that state. So each convergence re-tick
//! advanced the ODE another full `dt` — with 3 iterations the ODE advanced
//! `1 + 3 = 4` steps per orchestrator tick. Adding a *disconnected* structural
//! bond-graph (no equations of its own) therefore silently moved an ODE-driven
//! event ~4x earlier (the pump-agent repro: relief 1593 → 399 ticks; 1593/399 =
//! 3.99). The bond-graph injected no equations into the ODE — the perturbation
//! was a pure scheduler artifact of re-ticking a stateful integrator.
//!
//! Fix (orchestrator.rs): before each convergence re-integration the loop now
//! rewinds every ContinuousDynamics executor to its captured start-of-tick
//! state (`get_state_snapshot` / `restore_state`) and re-integrates the SAME
//! `dt`-step from there with the iteration's updated couplings. For a system
//! with no algebraic coupling to converge, the first re-integration reproduces
//! the main pass exactly, so convergence is a no-op on the trajectory.
//!
//! This test isolates the mechanism without needing a physics topology: it
//! toggles `convergence_iterations` directly on a standalone ODE model and
//! asserts the trajectory is identical with convergence off vs on. Before the
//! fix the convergence-on run over-advanced and diverged.

use sysml_runtime::orchestrator::Orchestrator;
use sysml_core::Value;

mod common;
use common::load_example_orchestrator;

fn float_at(orch: &Orchestrator, key: &str) -> f64 {
    match orch.context.get(key) {
        Some(Value::Float(f)) => *f,
        other => panic!("expected float for '{key}', got {other:?}"),
    }
}

/// Run `coulomb-friction` (a standalone 2-state ODE — `position`, `velocity`;
/// no SM, no physics, so convergence is disabled by default) for `ticks`
/// steps, optionally forcing the convergence loop on, and return the final
/// (position, velocity).
fn run_coulomb(convergence: u32, ticks: usize) -> (f64, f64) {
    let mut orch = load_example_orchestrator("coulomb-friction", &[], 1.0, 10_000.0);
    if convergence > 0 {
        orch.set_convergence_iterations(convergence);
    }
    for _ in 0..ticks {
        orch.step();
    }
    (float_at(&orch, "position"), float_at(&orch, "velocity"))
}

/// The convergence loop must leave a disconnected ODE's trajectory untouched.
/// This is exactly the state a model enters when a structural (equation-free)
/// bond-graph is added alongside an ODE+SM: physics auto-enables convergence,
/// and that flip alone must not move the ODE.
#[test]
fn convergence_does_not_over_advance_standalone_ode() {
    let ticks = 200;
    let baseline = run_coulomb(0, ticks);
    let converged = run_coulomb(3, ticks);

    assert!(
        (baseline.0 - converged.0).abs() < 1e-9,
        "position drifted when convergence was enabled: off={} on={} \
         (over-advance regression — the convergence loop re-integrated the ODE)",
        baseline.0,
        converged.0
    );
    assert!(
        (baseline.1 - converged.1).abs() < 1e-9,
        "velocity drifted when convergence was enabled: off={} on={}",
        baseline.1,
        converged.1
    );
}

/// Guard against a vacuous pass: the ODE must actually be integrating (the
/// state moves over 200 ticks), so the equality above is a real invariant, not
/// two frozen zeros.
#[test]
fn coulomb_ode_actually_advances() {
    let (pos, _vel) = run_coulomb(0, 200);
    let (pos0, _vel0) = run_coulomb(0, 1);
    assert!(
        (pos - pos0).abs() > 1e-3,
        "ODE state should evolve over 200 ticks (pos@1={pos0}, pos@200={pos})"
    );
}
