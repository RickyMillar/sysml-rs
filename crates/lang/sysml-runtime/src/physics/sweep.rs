//! Phase 5 — RadialSweepSolver for tree topologies.
//!
//! A two-pass solver for tree/radial network topologies:
//! 1. **Forward sweep** — propagate effort variables (voltage, temperature, pressure)
//!    from source to load along edges.
//! 2. **Backward sweep** — aggregate flow variables (current, heat flow, mass flow)
//!    at junctions using the conservation law (KCL, mass balance, etc.).
//!
//! This solver works for any domain with tree topology — electrical, hydraulic,
//! thermal, and mechanical translational all use the same algorithm.

use sysml_core::Value;

use super::connection::ConnectionGraph;
use super::constraints::{ConservationConstraint, ConstitutiveRelation, EffortEquality};
use super::domain::ConservationLaw;
use super::solver::{DomainSolver, PhysicsSolverError, SolveResult};

use crate::expressions::EvalContext;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a numeric value from the context, returning 0.0 if absent or non-numeric.
fn get_numeric(ctx: &EvalContext, var: &str) -> f64 {
    match ctx.get(var) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        _ => 0.0,
    }
}

/// Check whether a variable has an explicit value in the context.
fn has_value(ctx: &EvalContext, var: &str) -> bool {
    ctx.get(var).is_some()
}

// ---------------------------------------------------------------------------
// RadialSweepSolver
// ---------------------------------------------------------------------------

/// A radial (tree-topology) solver that uses forward/backward sweeps.
///
/// - Forward sweep: propagates effort equalities (e.g., voltage from source to load).
/// - Backward sweep: aggregates flow at junctions (e.g., KCL: parent current = sum of child currents).
#[derive(Debug, Clone)]
pub struct RadialSweepSolver {
    /// The physics domain this solver handles (e.g., `"electrical"`).
    pub domain: String,
}

impl DomainSolver for RadialSweepSolver {
    fn domain(&self) -> &str {
        &self.domain
    }

    fn can_solve(&self, graph: &ConnectionGraph) -> bool {
        graph.is_tree()
    }

    fn clone_boxed(&self) -> Box<dyn DomainSolver> {
        Box::new(self.clone())
    }

    fn solve(
        &self,
        _subgraph: &ConnectionGraph,
        constraints: &[ConservationConstraint],
        equalities: &[EffortEquality],
        ctx: &mut EvalContext,
    ) -> Result<SolveResult, PhysicsSolverError> {
        let mut variables_solved: usize = 0;

        // ---------------------------------------------------------------
        // Pass 1: Forward sweep — effort propagation
        // ---------------------------------------------------------------
        // For each effort equality (source_var → target_var), if the source
        // has a value, copy it to the target.
        for eq in equalities {
            if has_value(ctx, &eq.source_var) {
                let val = get_numeric(ctx, &eq.source_var);
                if !has_value(ctx, &eq.target_var) || get_numeric(ctx, &eq.target_var) != val {
                    ctx.set(eq.target_var.clone(), Value::Float(val));
                    variables_solved += 1;
                }
            }
        }

        // ---------------------------------------------------------------
        // Pass 2: Backward sweep — flow aggregation at junctions
        // ---------------------------------------------------------------
        for constraint in constraints {
            match constraint.law {
                ConservationLaw::FlowConservation | ConservationLaw::EnergyBalance => {
                    // Sum all outgoing flow values.
                    let total: f64 = constraint
                        .outgoing_vars
                        .iter()
                        .map(|var| get_numeric(ctx, var))
                        .sum();

                    // Write the total into each incoming flow variable.
                    for var in &constraint.incoming_vars {
                        if !has_value(ctx, var) || get_numeric(ctx, var) != total {
                            ctx.set(var.clone(), Value::Float(total));
                            variables_solved += 1;
                        }
                    }
                }
                ConservationLaw::SignalRouting => {
                    // Signal routing: no conservation, skip.
                }
            }
        }

        Ok(SolveResult {
            variables_solved,
            residual: 0.0, // exact for tree topologies
        })
    }
}

// ---------------------------------------------------------------------------
// Constitutive relation solver
// ---------------------------------------------------------------------------

/// Apply all algebraic constitutive relations by fixed-point iteration.
///
/// Handles R, G, TF, GY, Se, Sf elements. C/I elements are ODE-stepped
/// separately by the executor. Respects ODE state variables: variables
/// owned by C/I elements are never overwritten.
///
/// Sign conventions follow BondGraphTools `base.json`:
/// - R: `e_in - e_out = R * f`
/// - G: `f = G * e`
/// - TF: `e_1 = m * e_0`, `f_0 = -m * f_1`
/// - GY: `e_1 = -r * f_0`, `e_0 = r * f_1`
/// - Se: `e_0 = e_source`
/// - Sf: `f_0 = -f_source`
///
/// Returns the number of variables newly solved.
pub fn apply_constitutive(relations: &[ConstitutiveRelation], ctx: &mut EvalContext) -> usize {
    // Collect state variables owned by C/I elements.
    let mut state_vars: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for rel in relations {
        match rel {
            ConstitutiveRelation::Capacitance { effort_var, .. } => {
                state_vars.insert(effort_var.as_str());
            }
            ConstitutiveRelation::Inductance { flow_var, .. } => {
                state_vars.insert(flow_var.as_str());
            }
            _ => {}
        }
    }

    let mut total_solved = 0;
    let max_iterations = 10;

    for _ in 0..max_iterations {
        let mut solved_this_pass = 0;

        for rel in relations {
            match rel {
                // --- R-element: e_in - e_out = R * f ---
                ConstitutiveRelation::Resistance {
                    effort_in_var,
                    effort_out_var,
                    flow_var,
                    parameter_var,
                    parameter_value,
                } => {
                    let r = parameter_value.unwrap_or_else(|| get_numeric(ctx, parameter_var));
                    if r == 0.0 {
                        continue;
                    }

                    let have_vin = has_value(ctx, effort_in_var);
                    let have_vout = has_value(ctx, effort_out_var);
                    let have_i = has_value(ctx, flow_var);
                    let flow_is_state = state_vars.contains(flow_var.as_str());
                    let vout_is_state = state_vars.contains(effort_out_var.as_str());
                    let vin_is_state = state_vars.contains(effort_in_var.as_str());

                    if have_vin && have_i && !vout_is_state {
                        let new = get_numeric(ctx, effort_in_var) - r * get_numeric(ctx, flow_var);
                        if !have_vout || (get_numeric(ctx, effort_out_var) - new).abs() > 1e-15 {
                            ctx.set(effort_out_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    } else if have_vin && have_vout && !flow_is_state {
                        let new = (get_numeric(ctx, effort_in_var)
                            - get_numeric(ctx, effort_out_var))
                            / r;
                        if !have_i || (get_numeric(ctx, flow_var) - new).abs() > 1e-15 {
                            ctx.set(flow_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    } else if have_vout && have_i && !vin_is_state {
                        let new = get_numeric(ctx, effort_out_var) + r * get_numeric(ctx, flow_var);
                        if !have_vin || (get_numeric(ctx, effort_in_var) - new).abs() > 1e-15 {
                            ctx.set(effort_in_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                }

                // --- G-element (conductance): f = G * e ---
                ConstitutiveRelation::Conductance {
                    effort_var,
                    flow_var,
                    parameter_var,
                    parameter_value,
                } => {
                    let g = parameter_value.unwrap_or_else(|| get_numeric(ctx, parameter_var));
                    if g == 0.0 {
                        continue;
                    }

                    let flow_is_state = state_vars.contains(flow_var.as_str());
                    let effort_is_state = state_vars.contains(effort_var.as_str());

                    if has_value(ctx, effort_var) && !flow_is_state {
                        let new = g * get_numeric(ctx, effort_var);
                        if !has_value(ctx, flow_var)
                            || (get_numeric(ctx, flow_var) - new).abs() > 1e-15
                        {
                            ctx.set(flow_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    } else if has_value(ctx, flow_var) && !effort_is_state {
                        let new = get_numeric(ctx, flow_var) / g;
                        if !has_value(ctx, effort_var)
                            || (get_numeric(ctx, effort_var) - new).abs() > 1e-15
                        {
                            ctx.set(effort_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                }

                // --- Se: e_0 = e_source ---
                ConstitutiveRelation::EffortSource {
                    effort_var,
                    source_value,
                } => {
                    if let Some(val) = source_value {
                        if !has_value(ctx, effort_var)
                            || (get_numeric(ctx, effort_var) - val).abs() > 1e-15
                        {
                            ctx.set(effort_var.clone(), Value::Float(*val));
                            solved_this_pass += 1;
                        }
                    }
                }

                // --- Sf: f_0 = -f_source (BondGraphTools: f_0 + f = 0) ---
                ConstitutiveRelation::FlowSource {
                    flow_var,
                    source_value,
                } => {
                    if let Some(val) = source_value {
                        let negated = -val;
                        if !has_value(ctx, flow_var)
                            || (get_numeric(ctx, flow_var) - negated).abs() > 1e-15
                        {
                            ctx.set(flow_var.clone(), Value::Float(negated));
                            solved_this_pass += 1;
                        }
                    }
                }

                // --- TF: e_1 = m * e_0, f_0 = -m * f_1 ---
                ConstitutiveRelation::Transformer {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    modulus,
                } => {
                    let m = *modulus;
                    if m == 0.0 {
                        continue;
                    }

                    // Effort: e_1 = m * e_0
                    if has_value(ctx, effort_in_var) {
                        let new = m * get_numeric(ctx, effort_in_var);
                        if !has_value(ctx, effort_out_var)
                            || (get_numeric(ctx, effort_out_var) - new).abs() > 1e-15
                        {
                            ctx.set(effort_out_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    } else if has_value(ctx, effort_out_var) {
                        let new = get_numeric(ctx, effort_out_var) / m;
                        if !has_value(ctx, effort_in_var)
                            || (get_numeric(ctx, effort_in_var) - new).abs() > 1e-15
                        {
                            ctx.set(effort_in_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }

                    // Flow: f_0 = -m * f_1 (power conservation)
                    if has_value(ctx, flow_out_var) {
                        let new = -m * get_numeric(ctx, flow_out_var);
                        if !has_value(ctx, flow_in_var)
                            || (get_numeric(ctx, flow_in_var) - new).abs() > 1e-15
                        {
                            ctx.set(flow_in_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    } else if has_value(ctx, flow_in_var) {
                        let new = -get_numeric(ctx, flow_in_var) / m;
                        if !has_value(ctx, flow_out_var)
                            || (get_numeric(ctx, flow_out_var) - new).abs() > 1e-15
                        {
                            ctx.set(flow_out_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                }

                // --- GY: e_1 = -r * f_0, e_0 = r * f_1 ---
                ConstitutiveRelation::Gyrator {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    modulus,
                } => {
                    let r = *modulus;
                    if r == 0.0 {
                        continue;
                    }

                    // e_1 = -r * f_0
                    if has_value(ctx, flow_in_var) {
                        let new = -r * get_numeric(ctx, flow_in_var);
                        if !has_value(ctx, effort_out_var)
                            || (get_numeric(ctx, effort_out_var) - new).abs() > 1e-15
                        {
                            ctx.set(effort_out_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                    // e_0 = r * f_1
                    if has_value(ctx, flow_out_var) && !state_vars.contains(effort_in_var.as_str())
                    {
                        let new = r * get_numeric(ctx, flow_out_var);
                        if !has_value(ctx, effort_in_var)
                            || (get_numeric(ctx, effort_in_var) - new).abs() > 1e-15
                        {
                            ctx.set(effort_in_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                    // Inverse: f_0 = -e_1 / r
                    if has_value(ctx, effort_out_var) && !state_vars.contains(flow_in_var.as_str())
                    {
                        let new = -get_numeric(ctx, effort_out_var) / r;
                        if !has_value(ctx, flow_in_var)
                            || (get_numeric(ctx, flow_in_var) - new).abs() > 1e-15
                        {
                            ctx.set(flow_in_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                    // Inverse: f_1 = e_0 / r
                    if has_value(ctx, effort_in_var) && !state_vars.contains(flow_out_var.as_str())
                    {
                        let new = get_numeric(ctx, effort_in_var) / r;
                        if !has_value(ctx, flow_out_var)
                            || (get_numeric(ctx, flow_out_var) - new).abs() > 1e-15
                        {
                            ctx.set(flow_out_var.clone(), Value::Float(new));
                            solved_this_pass += 1;
                        }
                    }
                }

                // C/I elements are ODEs — handled by executor, not here.
                ConstitutiveRelation::Capacitance { .. }
                | ConstitutiveRelation::Inductance { .. } => {}
            }
        }

        total_solved += solved_this_pass;
        if solved_this_pass == 0 {
            break;
        }
    }

    total_solved
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::expressions::EvalContext;
    use crate::physics::connection::ConnectionGraph;
    use crate::physics::constraints::{ConservationConstraint, EffortEquality};
    use crate::physics::domain::ConservationLaw;
    use sysml_core::Value;

    /// Helper: create an empty tree graph for the solver.
    fn empty_tree_graph() -> ConnectionGraph {
        ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        }
    }

    /// Test 1: Forward effort propagation (voltage chain A → B → C).
    #[test]
    fn forward_effort_propagation() {
        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };

        let equalities = vec![
            EffortEquality {
                source_var: "A.voltage".to_string(),
                target_var: "B.voltage".to_string(),
            },
            EffortEquality {
                source_var: "B.voltage".to_string(),
                target_var: "C.voltage".to_string(),
            },
        ];

        let mut ctx = EvalContext::new();
        ctx.set("A.voltage", Value::Float(230.0));

        let graph = empty_tree_graph();
        let result = solver
            .solve(&graph, &[], &equalities, &mut ctx)
            .expect("solve should succeed");

        assert_eq!(
            ctx.get("B.voltage"),
            Some(&Value::Float(230.0)),
            "B.voltage should be propagated from A"
        );
        assert_eq!(
            ctx.get("C.voltage"),
            Some(&Value::Float(230.0)),
            "C.voltage should be propagated from B"
        );
        assert_eq!(result.variables_solved, 2);
        assert_eq!(result.residual, 0.0);
    }

    /// Test 2: Backward flow aggregation (KCL: incoming = sum of outgoing).
    #[test]
    fn backward_flow_aggregation_kcl() {
        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };

        let constraints = vec![ConservationConstraint {
            name: "KCL@busbar".to_string(),
            junction_id: 0,
            law: ConservationLaw::FlowConservation,
            incoming_vars: vec!["busbar.powerIn.current".to_string()],
            outgoing_vars: vec![
                "busbar.out1.current".to_string(),
                "busbar.out2.current".to_string(),
                "busbar.out3.current".to_string(),
            ],
        }];

        let mut ctx = EvalContext::new();
        ctx.set("busbar.out1.current", Value::Float(4.0));
        ctx.set("busbar.out2.current", Value::Float(10.0));
        ctx.set("busbar.out3.current", Value::Float(20.0));

        let graph = empty_tree_graph();
        let result = solver
            .solve(&graph, &constraints, &[], &mut ctx)
            .expect("solve should succeed");

        assert_eq!(
            ctx.get("busbar.powerIn.current"),
            Some(&Value::Float(34.0)),
            "incoming current should be sum of outgoing"
        );
        assert_eq!(result.variables_solved, 1);
        assert_eq!(result.residual, 0.0);
    }

    /// Test 3: Combined forward effort + backward flow in one solve call.
    #[test]
    fn combined_effort_and_flow() {
        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };

        // Voltage source → busbar → 2 loads
        let equalities = vec![
            EffortEquality {
                source_var: "source.out.voltage".to_string(),
                target_var: "busbar.in.voltage".to_string(),
            },
            EffortEquality {
                source_var: "busbar.out1.voltage".to_string(),
                target_var: "load1.in.voltage".to_string(),
            },
            EffortEquality {
                source_var: "busbar.out2.voltage".to_string(),
                target_var: "load2.in.voltage".to_string(),
            },
        ];

        let constraints = vec![ConservationConstraint {
            name: "KCL@busbar".to_string(),
            junction_id: 0,
            law: ConservationLaw::FlowConservation,
            incoming_vars: vec!["busbar.in.current".to_string()],
            outgoing_vars: vec![
                "busbar.out1.current".to_string(),
                "busbar.out2.current".to_string(),
            ],
        }];

        let mut ctx = EvalContext::new();
        // Known: source voltage
        ctx.set("source.out.voltage", Value::Float(120.0));
        // Known: busbar effort (would normally be propagated through internal junction)
        ctx.set("busbar.out1.voltage", Value::Float(120.0));
        ctx.set("busbar.out2.voltage", Value::Float(120.0));
        // Known: load currents (downstream)
        ctx.set("busbar.out1.current", Value::Float(5.0));
        ctx.set("busbar.out2.current", Value::Float(3.0));

        let graph = empty_tree_graph();
        let result = solver
            .solve(&graph, &constraints, &equalities, &mut ctx)
            .expect("solve should succeed");

        // Voltage propagated to loads
        assert_eq!(ctx.get("busbar.in.voltage"), Some(&Value::Float(120.0)));
        assert_eq!(ctx.get("load1.in.voltage"), Some(&Value::Float(120.0)));
        assert_eq!(ctx.get("load2.in.voltage"), Some(&Value::Float(120.0)));

        // KCL: busbar incoming = 5.0 + 3.0 = 8.0
        assert_eq!(ctx.get("busbar.in.current"), Some(&Value::Float(8.0)));

        assert!(result.variables_solved >= 4, "at least 4 variables solved");
        assert_eq!(result.residual, 0.0);
    }

    /// Test 4: Empty graph yields 0 variables solved.
    #[test]
    fn empty_graph_solve() {
        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };

        let mut ctx = EvalContext::new();
        let graph = empty_tree_graph();
        let result = solver
            .solve(&graph, &[], &[], &mut ctx)
            .expect("solve should succeed on empty graph");

        assert_eq!(result.variables_solved, 0);
        assert_eq!(result.residual, 0.0);
    }

    /// Test 5: Integer values are correctly converted to f64 during propagation.
    #[test]
    fn integer_value_propagation() {
        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };

        let equalities = vec![EffortEquality {
            source_var: "A.voltage".to_string(),
            target_var: "B.voltage".to_string(),
        }];

        let mut ctx = EvalContext::new();
        ctx.set("A.voltage", Value::Int(24));

        let graph = empty_tree_graph();
        solver
            .solve(&graph, &[], &equalities, &mut ctx)
            .expect("solve should succeed");

        assert_eq!(
            ctx.get("B.voltage"),
            Some(&Value::Float(24.0)),
            "integer voltage should propagate as float"
        );
    }

    /// Test 6: apply_constitutive solves I = (V_in - V_out) / R.
    #[test]
    fn constitutive_resistance_solve_current() {
        use crate::physics::constraints::ConstitutiveRelation;

        let relations = vec![ConstitutiveRelation::Resistance {
            effort_in_var: "r1.vin".to_string(),
            effort_out_var: "r1.vout".to_string(),
            flow_var: "r1.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        }];

        let mut ctx = EvalContext::new();
        ctx.set("r1.vin", Value::Float(10.0));
        ctx.set("r1.vout", Value::Float(0.0));

        let solved = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved, 1);
        assert_eq!(ctx.get("r1.current"), Some(&Value::Float(2.0)));
    }

    /// Test 7: apply_constitutive solves V_out = V_in - R*I.
    #[test]
    fn constitutive_resistance_solve_voltage_out() {
        use crate::physics::constraints::ConstitutiveRelation;

        let relations = vec![ConstitutiveRelation::Resistance {
            effort_in_var: "r1.vin".to_string(),
            effort_out_var: "r1.vout".to_string(),
            flow_var: "r1.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        }];

        let mut ctx = EvalContext::new();
        ctx.set("r1.vin", Value::Float(10.0));
        ctx.set("r1.current", Value::Float(2.0));

        let solved = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved, 1);
        assert_eq!(ctx.get("r1.vout"), Some(&Value::Float(0.0)));
    }

    /// Test 8: apply_constitutive with no knowns solves nothing.
    #[test]
    fn constitutive_resistance_nothing_to_solve() {
        use crate::physics::constraints::ConstitutiveRelation;

        let relations = vec![ConstitutiveRelation::Resistance {
            effort_in_var: "r1.vin".to_string(),
            effort_out_var: "r1.vout".to_string(),
            flow_var: "r1.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        }];

        let mut ctx = EvalContext::new();
        let solved = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved, 0);
    }

    /// Test 9: Energy balance uses same aggregation as flow conservation.
    #[test]
    fn energy_balance_aggregation() {
        let solver = RadialSweepSolver {
            domain: "thermal".to_string(),
        };

        let constraints = vec![ConservationConstraint {
            name: "EnergyBalance@heatsink".to_string(),
            junction_id: 0,
            law: ConservationLaw::EnergyBalance,
            incoming_vars: vec!["heatsink.in.heat_flow".to_string()],
            outgoing_vars: vec![
                "heatsink.out1.heat_flow".to_string(),
                "heatsink.out2.heat_flow".to_string(),
            ],
        }];

        let mut ctx = EvalContext::new();
        ctx.set("heatsink.out1.heat_flow", Value::Float(50.0));
        ctx.set("heatsink.out2.heat_flow", Value::Float(30.0));

        let graph = empty_tree_graph();
        let result = solver
            .solve(&graph, &constraints, &[], &mut ctx)
            .expect("solve should succeed");

        assert_eq!(ctx.get("heatsink.in.heat_flow"), Some(&Value::Float(80.0)));
        assert_eq!(result.variables_solved, 1);
    }

    // -----------------------------------------------------------------------
    // Phase 3.1/3.2: Variable impedance — context-driven R/C/L
    // -----------------------------------------------------------------------

    /// Variable resistance: parameter_value = None, R read from context each call.
    /// Simulates temperature-dependent resistance.
    #[test]
    fn constitutive_variable_resistance_from_context() {
        use crate::physics::constraints::ConstitutiveRelation;

        let relations = vec![ConstitutiveRelation::Resistance {
            effort_in_var: "wire.vin".to_string(),
            effort_out_var: "wire.vout".to_string(),
            flow_var: "wire.current".to_string(),
            parameter_var: "wire.resistance".to_string(),
            parameter_value: None, // <-- read from context
        }];

        // Cold: R = 5.0Ω → I = 10V / 5Ω = 2A
        let mut ctx = EvalContext::new();
        ctx.set("wire.vin", Value::Float(10.0));
        ctx.set("wire.vout", Value::Float(0.0));
        ctx.set("wire.resistance", Value::Float(5.0));

        let solved = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved, 1);
        assert_eq!(ctx.get("wire.current"), Some(&Value::Float(2.0)));

        // Hot: R = 10.0Ω → I = 10V / 10Ω = 1A (same call, new R)
        ctx.set("wire.resistance", Value::Float(10.0));
        // Clear previous current to force re-solve
        std::sync::Arc::make_mut(&mut ctx.variables).remove("wire.current");

        let solved2 = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved2, 1);
        assert_eq!(ctx.get("wire.current"), Some(&Value::Float(1.0)));
    }

    /// Variable conductance from context (e.g., valve opening percentage).
    #[test]
    fn constitutive_variable_conductance_from_context() {
        use crate::physics::constraints::ConstitutiveRelation;

        let relations = vec![ConstitutiveRelation::Conductance {
            effort_var: "pipe.pressure".to_string(),
            flow_var: "pipe.flow".to_string(),
            parameter_var: "pipe.conductance".to_string(),
            parameter_value: None,
        }];

        // Full open: G = 2.0 → flow = 2.0 * 50 = 100
        let mut ctx = EvalContext::new();
        ctx.set("pipe.pressure", Value::Float(50.0));
        ctx.set("pipe.conductance", Value::Float(2.0));

        let solved = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved, 1);
        assert_eq!(ctx.get("pipe.flow"), Some(&Value::Float(100.0)));

        // Half open: G = 1.0 → flow = 1.0 * 50 = 50
        ctx.set("pipe.conductance", Value::Float(1.0));
        std::sync::Arc::make_mut(&mut ctx.variables).remove("pipe.flow");

        let solved2 = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved2, 1);
        assert_eq!(ctx.get("pipe.flow"), Some(&Value::Float(50.0)));
    }

    /// SF-parameterized resistance: R interpolated from a SampledFunction
    /// stored in context, driven by temperature.
    #[test]
    fn constitutive_sf_parameterized_resistance() {
        use crate::physics::constraints::ConstitutiveRelation;
        use std::collections::BTreeMap;

        // Create a SampledFunction for R(T): R = 5Ω at 20°C, R = 15Ω at 100°C
        let mut sf_map = BTreeMap::new();
        sf_map.insert(
            "__type".to_string(),
            Value::String("SampledFunction".to_string()),
        );
        sf_map.insert(
            "domain".to_string(),
            Value::List(vec![Value::Float(20.0), Value::Float(100.0)]),
        );
        sf_map.insert(
            "range".to_string(),
            Value::List(vec![Value::Float(5.0), Value::Float(15.0)]),
        );

        let mut ctx = EvalContext::new();
        ctx.set("__sf_resistance_curve", Value::Map(sf_map));

        // Evaluate SF at T=60°C → R = 5 + (60-20)/(100-20) * (15-5) = 5 + 5 = 10Ω
        let evaluator = crate::expressions::ExpressionEvaluator::new();
        let interp_expr =
            crate::ode_builder::parse_derivative("interpolateLinear(__sf_resistance_curve, 60.0)")
                .unwrap();
        let r_val = evaluator.eval(&interp_expr, &ctx).unwrap();
        assert_eq!(r_val, Value::Float(10.0));

        // Now use this R in a constitutive relation
        ctx.set("wire.resistance", r_val);
        ctx.set("wire.vin", Value::Float(100.0));
        ctx.set("wire.vout", Value::Float(0.0));

        let relations = vec![ConstitutiveRelation::Resistance {
            effort_in_var: "wire.vin".to_string(),
            effort_out_var: "wire.vout".to_string(),
            flow_var: "wire.current".to_string(),
            parameter_var: "wire.resistance".to_string(),
            parameter_value: None,
        }];

        let solved = super::apply_constitutive(&relations, &mut ctx);
        assert_eq!(solved, 1);
        // I = 100V / 10Ω = 10A
        assert_eq!(ctx.get("wire.current"), Some(&Value::Float(10.0)));
    }
}
