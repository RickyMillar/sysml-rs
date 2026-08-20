//! espresso-production-cell — structure + physics gates (Stage A/B).
//!
//! Clean-room replacement fixture gates. Each test names the COV capability_id
//! it discharges (see the IP-extraction plan §10.2 and its coverage matrix).

mod common;
use common::{load_example_graph, load_example_orchestrator};
use sysml_core::Value;

const FIXTURE: &str = "espresso-production-cell";

/// Advance the smoke-scale cell to steady state; return the last two snapshots
/// (100 ticks apart) so tests can assert both a settled value and that it settled.
fn run_smoke() -> (
    sysml_runtime::orchestrator::ExecutionSnapshot,
    sysml_runtime::orchestrator::ExecutionSnapshot,
) {
    let mut orch = load_example_orchestrator(FIXTURE, &[], 100.0, 60_000.0);
    let mut prev = orch.step();
    for _ in 0..500 {
        prev = orch.step();
    }
    let mut last = prev.clone();
    for _ in 0..100 {
        last = orch.step();
    }
    (prev, last)
}

fn f(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Float(x)) => *x,
        Some(Value::Int(i)) => *i as f64,
        other => panic!("expected a number, got {other:?}"),
    }
}

/// CELL-LOAD-01 / LANG-STRUCT — the multi-file workspace loads deterministically
/// and produces the expected element-kind census.
#[test]
fn cell_load_element_census() {
    let graph = load_example_graph(FIXTURE);
    let count = |k: sysml_core::ElementKind| {
        graph.elements.values().filter(|e| e.kind == k).count()
    };
    use sysml_core::ElementKind::*;
    // Structural spine (independently declared; stable across reparse).
    assert!(count(PartDefinition) >= 8, "part defs: {}", count(PartDefinition));
    assert!(count(PortDefinition) >= 12, "port defs: {}", count(PortDefinition));
    assert!(count(ItemDefinition) >= 8, "item defs: {}", count(ItemDefinition));
    // Two stations wired as usages of one BrewStation def.
    assert!(count(PartUsage) >= 4, "part usages: {}", count(PartUsage));
    // Determinism of load: a second load yields the same census.
    let graph2 = load_example_graph(FIXTURE);
    assert_eq!(graph.elements.len(), graph2.elements.len());
}

/// CELL-INST-01 / RT-INSTANCE — the two stations multiply into independent
/// per-instance subsystems with canonical qualified slot paths.
#[test]
fn cell_instances_are_independent_with_qualified_slots() {
    let (_, last) = run_smoke();
    // Per-instance ODE state exists under its own qualified slot path (the
    // nested-part state var surfaces as `<station>.temp`, plus the canonical
    // `ProductionCell.<station>.temp`).
    for st in ["station1", "station2"] {
        let t = f(last.variables.get(&format!("{st}.temp")));
        assert!((22.0..=100.0).contains(&t), "{st}.temp = {t} out of brew band");
        assert!(
            last.variables.contains_key(&format!("ProductionCell.{st}.temp")),
            "canonical path ProductionCell.{st}.temp missing"
        );
    }
    // Two distinct per-instance subsystems (independence) + single shared plants.
    let subs: Vec<_> = last.subsystem_states.keys().cloned().collect();
    for want in ["station1.StationThermal", "station2.StationThermal",
                 "ManifoldDynamics", "ThermalSourceDynamics"] {
        assert!(subs.iter().any(|s| s == want), "missing subsystem {want} in {subs:?}");
    }
}

/// CELL-PHYS-01 / CELL-PHYS-02 — hydraulic + thermal states integrate to
/// bounded equilibria consistent with the declared lumped equations. Residuals
/// are computed from the SAME quantities exposed to users; equilibrium is
/// confirmed by settling (Δ over the final 100 ticks is negligible).
#[test]
fn cell_physics_residuals_bounded() {
    let (prev, last) = run_smoke();
    let settled = |k: &str| {
        let a = f(prev.variables.get(k));
        let b = f(last.variables.get(k));
        assert!((a - b).abs() < 1e-2, "{k} not settled: {a} -> {b}");
        b
    };

    // Manifold: settled, inside envelope, and its own steady-state residual ≈ 0.
    //   dp_m/dt/Km = (q_pump - q_bypass) - k_draw*(p_m - p_return) → 0
    let p_m = settled("ManifoldDynamics.p_m");
    assert!((1.0..=20.0).contains(&p_m), "manifold pressure {p_m} out of envelope");
    let (q_pump, q_bypass, k_draw, p_return) = (6.0, 0.5, 0.4, 1.0);
    let hyd_resid = (q_pump - q_bypass) - k_draw * (p_m - p_return);
    assert!(hyd_resid.abs() < 1e-2, "hydraulic residual {hyd_resid} (p_m={p_m})");

    // Boiler: settled, inside envelope, own residual ≈ 0.
    let t_src = settled("ThermalSourceDynamics.tsource");
    assert!((20.0..=140.0).contains(&t_src), "source temp {t_src} out of envelope");
    let (p_source, q_loss, h_load, t_ref) = (100.0, 10.0, 1.0, 22.0);
    let src_resid = (p_source - q_loss) - h_load * (t_src - t_ref);
    assert!(src_resid.abs() < 1e-2, "thermal-source residual {src_resid} (T={t_src})");

    // Each station: settled, inside brew band, own energy residual ≈ 0.
    //   P_heater + h_s(T_s - T) - (h_a(T - T_a) + Q_product) → 0
    for st in ["station1", "station2"] {
        let t = settled(&format!("{st}.temp"));
        assert!((22.0..=100.0).contains(&t), "{st} temp {t} out of brew band");
        let (p_heater, h_s, t_s, h_a, t_a, q_prod) = (10.0, 0.5, 96.0, 0.2, 22.0, 0.0);
        let resid = (p_heater + h_s * (t_s - t)) - (h_a * (t - t_a) + q_prod);
        assert!(resid.abs() < 1e-2, "{st} thermal residual {resid} (T={t})");
    }
}

/// CELL-PHYS-01 (accounting) — the model's aggregate q_total equals an
/// independent sum of the per-instance branch flows (mass-accounting identity),
/// computed from the exposed slots.
#[test]
fn cell_aggregate_accounting_identity() {
    let (_, last) = run_smoke();
    let q1 = f(last.variables.get("station1.qBranch"));
    let q2 = f(last.variables.get("station2.qBranch"));
    let q_total = f(last.variables.get("qTotal"));
    assert!(q1 > 0.0 && q2 > 0.0, "branch flows must be positive: {q1}, {q2}");
    assert!(
        (q_total - (q1 + q2)).abs() < 1e-6,
        "qTotal {q_total} != station branch sum {}",
        q1 + q2
    );
}
