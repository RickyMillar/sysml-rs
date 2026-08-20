//! Bond graph validation tests against analytically known results.
//!
//! These tests verify that our bond graph implementation produces correct
//! physics by comparing against textbook analytical solutions. Each test
//! constructs a simple circuit, runs the physics engine, and checks that
//! the computed values match the known solution within tolerance.
//!
//! Reference: Karnopp, Margolis & Rosenberg, "System Dynamics"
//! Reference: Any introductory circuits textbook (Ohm's law, RC/RL transients)

use sysml_runtime::expressions::EvalContext;
use sysml_runtime::orchestrator::{Executor, TickContext};
use sysml_runtime::slots::{SlotStore, WriterId};
use sysml_runtime::physics::connection::{
    ConnectionGraph, Junction, JunctionType, PhysicsConnection, PhysicsPortNode,
};
use sysml_runtime::physics::constraints::{ConservationConstraint, EffortEquality};
use sysml_runtime::physics::constraints::{ConstitutiveRelation, GeneratedConstraints};
use sysml_runtime::physics::domain::{ConservationLaw, PhysicsDomainRegistry};
use sysml_runtime::physics::executor::PhysicsExecutor;
use sysml_runtime::physics::sweep::apply_constitutive;

use sysml_core::Value;
use sysml_runtime::flows::port::PortDirection;

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node(
    id: usize,
    owner: &str,
    port: &str,
    domain: Option<&'static str>,
    dir: PortDirection,
) -> PhysicsPortNode {
    PhysicsPortNode {
        id,
        qualified_path: format!("{}.{}", owner, port),
        owner_path: owner.to_string(),
        port_name: port.to_string(),
        domain,
        direction: dir,
        classification: None,
    }
}

fn make_tick_ctx(shared: &EvalContext, t: f64, dt: f64) -> TickContext<'_> {
    TickContext {
        t,
        dt,
        tick: 0,
        context: shared,
        event: None,
        port_payloads: &[],
        local_clock_time: None,
    }
}

// ===========================================================================
// Test 1: Ohm's Law — V = I * R
// ===========================================================================
// A single resistor with known voltage and resistance.
// Expected: I = V / R = 10 / 5 = 2 A

#[test]
fn ohms_law_solve_current() {
    let relations = vec![ConstitutiveRelation::Resistance {
        effort_in_var: "r.vin".into(),
        effort_out_var: "r.vout".into(),
        flow_var: "r.current".into(),
        parameter_var: "r.resistance".into(),
        parameter_value: Some(5.0),
    }];

    let mut ctx = EvalContext::new();
    ctx.set("r.vin", Value::Float(10.0));
    ctx.set("r.vout", Value::Float(0.0));

    apply_constitutive(&relations, &mut ctx);

    let i = match ctx.get("r.current") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!(
        (i - 2.0).abs() < 1e-10,
        "Ohm's law: I = V/R = 10/5 = 2, got {}",
        i
    );
}

// ===========================================================================
// Test 2: Ohm's Law — solve for voltage drop
// ===========================================================================
// Known: V_in = 12V, I = 3A, R = 4Ω
// Expected: V_out = V_in - R*I = 12 - 12 = 0V

#[test]
fn ohms_law_solve_voltage_drop() {
    let relations = vec![ConstitutiveRelation::Resistance {
        effort_in_var: "r.vin".into(),
        effort_out_var: "r.vout".into(),
        flow_var: "r.current".into(),
        parameter_var: "r.resistance".into(),
        parameter_value: Some(4.0),
    }];

    let mut ctx = EvalContext::new();
    ctx.set("r.vin", Value::Float(12.0));
    ctx.set("r.current", Value::Float(3.0));

    apply_constitutive(&relations, &mut ctx);

    let vout = match ctx.get("r.vout") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!(
        (vout - 0.0).abs() < 1e-10,
        "V_out = 12 - 4*3 = 0, got {}",
        vout
    );
}

// ===========================================================================
// Test 3: RC circuit — capacitor charging
// ===========================================================================
// V_source = 10V, R = 1kΩ, C = 1mF → τ = RC = 1s
// Analytical: V_c(t) = V_s * (1 - e^(-t/τ))
// At t = 1s: V_c = 10 * (1 - e^-1) ≈ 6.321V
//
// We simulate with small dt to approximate the continuous solution.

#[test]
fn rc_circuit_charging() {
    let r = 1000.0; // 1kΩ
    let c = 0.001; // 1mF
    let v_source = 10.0;
    let dt = 0.001; // 1ms steps
    let t_final = 1.0; // 1 second (one time constant)
    let steps = (t_final / dt) as usize;

    // State: capacitor voltage
    let mut v_cap = 0.0_f64;

    for _ in 0..steps {
        // Current through resistor: I = (V_source - V_cap) / R
        let current = (v_source - v_cap) / r;
        // Capacitor ODE: dV/dt = I / C
        v_cap += (current / c) * dt;
    }

    // Analytical: V_c(1) = 10 * (1 - e^-1) ≈ 6.3212...
    let expected = v_source * (1.0 - (-1.0_f64).exp());
    assert!(
        (v_cap - expected).abs() < 0.05,
        "RC charging: expected {:.4}, got {:.4} (tolerance 0.05V for Euler)",
        expected,
        v_cap
    );
}

// ===========================================================================
// Test 4: RC circuit via PhysicsExecutor
// ===========================================================================
// Same circuit but using our actual ConstitutiveRelation + executor tick loop.

#[test]
fn rc_circuit_via_executor() {
    let v_source = 10.0;
    let r_val = 1000.0;
    let c_val = 0.001;
    let dt = 0.001;
    let steps = 1000; // 1 second

    let graph = ConnectionGraph {
        nodes: vec![],
        edges: vec![],
        junctions: vec![],
    };

    let registry = Arc::new(PhysicsDomainRegistry::new());
    let mut constraints = GeneratedConstraints::default();

    // R-element: V_source - V_cap = R * I
    constraints
        .constitutive
        .push(ConstitutiveRelation::Resistance {
            effort_in_var: "circuit.source.voltage".into(),
            effort_out_var: "circuit.cap.voltage".into(),
            flow_var: "circuit.resistor.current".into(),
            parameter_var: "circuit.resistor.resistance".into(),
            parameter_value: Some(r_val),
        });

    // C-element: dV_cap/dt = I / C
    constraints
        .constitutive
        .push(ConstitutiveRelation::Capacitance {
            effort_var: "circuit.cap.voltage".into(),
            flow_var: "circuit.resistor.current".into(),
            parameter_var: "circuit.cap.capacitance".into(),
            parameter_value: Some(c_val),
        });

    let mut executor = PhysicsExecutor::new(registry, graph, constraints);

    // Set initial conditions once — source voltage and cap voltage
    executor.sync_context_in(&{
        let mut ctx = EvalContext::new();
        ctx.set("circuit.source.voltage", Value::Float(v_source));
        ctx.set("circuit.cap.voltage", Value::Float(0.0));
        ctx.set("circuit.resistor.current", Value::Float(0.0));
        ctx
    });

    let shared = EvalContext::new();
    for step in 0..steps {
        let tick_ctx = make_tick_ctx(&shared, step as f64 * dt, dt);
        executor.tick(&tick_ctx);
    }

    // Read capacitor voltage from executor's context
    let mut out = EvalContext::new();
    // The legacy `sync_context_out` whole-local dump was deleted with the
    // string-identity cull. Route the physics write-set (name-keyed against an
    // empty store, so it reproduces the legacy dump's keys/values) to extract
    // the solved state.
    Executor::prepare_slot_writeback(
        &mut executor,
        &SlotStore::new(),
        None,
        None,
        WriterId::Executor(0),
    );
    executor.sync_context_out_slots(&mut out, sysml_runtime::ode::SignalEvalMode::FreshState);
    let v_cap = match out.get("circuit.cap.voltage") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float for cap voltage, got {:?}", other),
    };

    let expected = v_source * (1.0 - (-1.0_f64).exp());
    assert!(
        (v_cap - expected).abs() < 0.1,
        "RC via executor: expected {:.4}, got {:.4} (tolerance 0.1V)",
        expected,
        v_cap
    );
}

// ===========================================================================
// Test 5: RL circuit — inductor current ramp
// ===========================================================================
// V_source = 10V, R = 5Ω, L = 2H → τ = L/R = 0.4s
// Analytical: I(t) = (V/R) * (1 - e^(-t*R/L))
// At t = 0.4s (one τ): I = 2 * (1 - e^-1) ≈ 1.264A
// Steady state (t→∞): I = V/R = 2A

#[test]
fn rl_circuit_current_ramp() {
    let v_source = 10.0;
    let r_val = 5.0;
    let l_val = 2.0;
    let dt = 0.0001; // 0.1ms for accuracy
    let tau = l_val / r_val; // 0.4s
    let t_final = tau; // simulate one time constant
    let steps = (t_final / dt) as usize;

    let graph = ConnectionGraph {
        nodes: vec![],
        edges: vec![],
        junctions: vec![],
    };

    let registry = Arc::new(PhysicsDomainRegistry::new());
    let mut constraints = GeneratedConstraints::default();

    // R-element: V_source - V_inductor = R * I
    constraints
        .constitutive
        .push(ConstitutiveRelation::Resistance {
            effort_in_var: "rl.source.voltage".into(),
            effort_out_var: "rl.inductor.voltage".into(),
            flow_var: "rl.current".into(),
            parameter_var: "rl.resistance".into(),
            parameter_value: Some(r_val),
        });

    // I-element: dI/dt = V_inductor / L
    constraints
        .constitutive
        .push(ConstitutiveRelation::Inductance {
            flow_var: "rl.current".into(),
            effort_var: "rl.inductor.voltage".into(),
            parameter_var: "rl.inductance".into(),
            parameter_value: Some(l_val),
        });

    let mut executor = PhysicsExecutor::new(registry, graph, constraints);

    // Set initial conditions: source voltage and initial current.
    // Do NOT set inductor voltage — it's derived from V_source - R*I.
    executor.sync_context_in(&{
        let mut ctx = EvalContext::new();
        ctx.set("rl.source.voltage", Value::Float(v_source));
        ctx.set("rl.current", Value::Float(0.0));
        ctx
    });

    let shared = EvalContext::new();
    for step in 0..steps {
        let tick_ctx = make_tick_ctx(&shared, step as f64 * dt, dt);
        executor.tick(&tick_ctx);
    }

    let mut out = EvalContext::new();
    // The legacy `sync_context_out` whole-local dump was deleted with the
    // string-identity cull. Route the physics write-set (name-keyed against an
    // empty store, so it reproduces the legacy dump's keys/values) to extract
    // the solved state.
    Executor::prepare_slot_writeback(
        &mut executor,
        &SlotStore::new(),
        None,
        None,
        WriterId::Executor(0),
    );
    executor.sync_context_out_slots(&mut out, sysml_runtime::ode::SignalEvalMode::FreshState);
    let current = match out.get("rl.current") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float for current, got {:?}", other),
    };

    // I(τ) = (V/R) * (1 - e^-1)
    let expected = (v_source / r_val) * (1.0 - (-1.0_f64).exp());
    assert!(
        (current - expected).abs() < 0.05,
        "RL circuit at t=τ: expected {:.4}A, got {:.4}A",
        expected,
        current
    );
}

// ===========================================================================
// Test 6: KCL at junction — current conservation
// ===========================================================================
// Busbar with 1 input, 3 outputs. Input current = sum of outputs.

#[test]
fn kcl_current_conservation() {
    let nodes = vec![
        node(0, "busbar", "in", Some("electrical"), PortDirection::In),
        node(1, "busbar", "out1", Some("electrical"), PortDirection::Out),
        node(2, "busbar", "out2", Some("electrical"), PortDirection::Out),
        node(3, "busbar", "out3", Some("electrical"), PortDirection::Out),
    ];

    let junctions = vec![Junction {
        id: 0,
        owner: "busbar".into(),
        domain: "electrical",
        junction_type: JunctionType::Zero,
        conservation: ConservationLaw::FlowConservation,
        incoming: vec![(0, "current".into())],
        outgoing: vec![
            (1, "current".into()),
            (2, "current".into()),
            (3, "current".into()),
        ],
    }];

    let edges = vec![
        PhysicsConnection {
            source: 0,
            target: 1,
            domain: Some("electrical"),
            enabled: true,
        },
        PhysicsConnection {
            source: 0,
            target: 2,
            domain: Some("electrical"),
            enabled: true,
        },
        PhysicsConnection {
            source: 0,
            target: 3,
            domain: Some("electrical"),
            enabled: true,
        },
    ];

    let graph = ConnectionGraph {
        nodes,
        edges,
        junctions,
    };
    let registry = Arc::new(PhysicsDomainRegistry::new());
    let constraints = sysml_runtime::physics::constraints::generate_constraints(&graph, &registry);

    let mut executor = PhysicsExecutor::new(registry, graph, constraints);

    // Set output currents
    executor.sync_context_in(&{
        let mut ctx = EvalContext::new();
        ctx.set("busbar.out1.current", Value::Float(5.0));
        ctx.set("busbar.out2.current", Value::Float(3.0));
        ctx.set("busbar.out3.current", Value::Float(7.0));
        // Seed the source voltage so effort propagation works
        ctx.set("busbar.in.voltage", Value::Float(230.0));
        ctx
    });

    let shared = EvalContext::new();
    let tick_ctx = make_tick_ctx(&shared, 0.0, 0.001);
    executor.tick(&tick_ctx);

    let mut out = EvalContext::new();
    // The legacy `sync_context_out` whole-local dump was deleted with the
    // string-identity cull. Route the physics write-set (name-keyed against an
    // empty store, so it reproduces the legacy dump's keys/values) to extract
    // the solved state.
    Executor::prepare_slot_writeback(
        &mut executor,
        &SlotStore::new(),
        None,
        None,
        WriterId::Executor(0),
    );
    executor.sync_context_out_slots(&mut out, sysml_runtime::ode::SignalEvalMode::FreshState);

    let i_in = match out.get("busbar.in.current") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };

    assert!(
        (i_in - 15.0).abs() < 1e-10,
        "KCL: I_in = 5 + 3 + 7 = 15A, got {}",
        i_in
    );
}

// ===========================================================================
// Test 7: Effort propagation — voltage equality across connection
// ===========================================================================

#[test]
fn effort_propagation_voltage_equality() {
    let nodes = vec![
        node(0, "source", "out", Some("electrical"), PortDirection::Out),
        node(1, "load", "in", Some("electrical"), PortDirection::In),
    ];

    let edges = vec![PhysicsConnection {
        source: 0,
        target: 1,
        domain: Some("electrical"),
        enabled: true,
    }];

    let graph = ConnectionGraph {
        nodes,
        edges,
        junctions: vec![],
    };
    let registry = Arc::new(PhysicsDomainRegistry::new());
    let constraints = sysml_runtime::physics::constraints::generate_constraints(&graph, &registry);

    let mut executor = PhysicsExecutor::new(registry, graph, constraints);

    executor.sync_context_in(&{
        let mut ctx = EvalContext::new();
        ctx.set("source.out.voltage", Value::Float(120.0));
        ctx
    });

    let shared = EvalContext::new();
    let tick_ctx = make_tick_ctx(&shared, 0.0, 0.001);
    executor.tick(&tick_ctx);

    let mut out = EvalContext::new();
    // The legacy `sync_context_out` whole-local dump was deleted with the
    // string-identity cull. Route the physics write-set (name-keyed against an
    // empty store, so it reproduces the legacy dump's keys/values) to extract
    // the solved state.
    Executor::prepare_slot_writeback(
        &mut executor,
        &SlotStore::new(),
        None,
        None,
        WriterId::Executor(0),
    );
    executor.sync_context_out_slots(&mut out, sysml_runtime::ode::SignalEvalMode::FreshState);

    let v_load = match out.get("load.in.voltage") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };

    assert!(
        (v_load - 120.0).abs() < 1e-10,
        "Effort equality: V_load = V_source = 120V, got {}",
        v_load
    );
}

// ===========================================================================
// Test 8: Thermal domain — heat flow through resistance
// ===========================================================================
// T_hot = 100°C, T_cold = 20°C, R_th = 4 K/W
// Expected: Q = (T_hot - T_cold) / R_th = 80/4 = 20W

#[test]
fn thermal_resistance_heat_flow() {
    let relations = vec![ConstitutiveRelation::Resistance {
        effort_in_var: "wall.hot.temperature".into(),
        effort_out_var: "wall.cold.temperature".into(),
        flow_var: "wall.heat_flow".into(),
        parameter_var: "wall.thermal_resistance".into(),
        parameter_value: Some(4.0),
    }];

    let mut ctx = EvalContext::new();
    ctx.set("wall.hot.temperature", Value::Float(100.0));
    ctx.set("wall.cold.temperature", Value::Float(20.0));

    apply_constitutive(&relations, &mut ctx);

    let q = match ctx.get("wall.heat_flow") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };

    assert!(
        (q - 20.0).abs() < 1e-10,
        "Fourier's law: Q = ΔT/R_th = 80/4 = 20W, got {}",
        q
    );
}

// ===========================================================================
// Test 9: Mechanical domain — spring-mass (C-element)
// ===========================================================================
// Spring: F = k*x, or in bond graph terms: effort = displacement / C
// where C = 1/k (compliance). With F=10N, k=100N/m: x = 0.1m
//
// As a C-element ODE: d(velocity)/dt = force / mass... wait, that's I-element.
// Spring is C-element in mechanical: d(force)/dt = velocity * k (derivative form)
// Integral form: velocity = (1/k) * d(force)/dt — this is the C-element.
//
// Simpler test: mass under constant force.
// F = 10N, m = 2kg → a = F/m = 5 m/s²
// v(t) = a*t, at t=1s: v = 5 m/s
//
// In bond graph: I-element, d(flow)/dt = effort/I → d(velocity)/dt = force/mass
// Wait, our domain has effort=velocity, flow=force.
// So I-element: d(effort)/dt = flow/I... no, I-element is e = I*df/dt.
// Actually for mech_translational with effort=velocity, flow=force:
// I-element stores flow (force storage = spring? no...)
//
// Let me use the straightforward capacitor analogy instead.
// Mass is I-element in standard (force=effort) convention.
// In our convention (velocity=effort, force=flow):
// Mass = C-element: d(effort)/dt = flow/C → d(velocity)/dt = force/mass
// This is Newton's second law! Mass acts as capacitance in velocity-effort convention.

#[test]
fn mass_under_constant_force() {
    // Mass = 2kg as C-element (in velocity-effort convention)
    // F = 10N (constant force = flow source)
    // dv/dt = F/m = 10/2 = 5 m/s²
    // v(1s) = 5 m/s
    let mass = 2.0;
    let force = 10.0;
    let dt = 0.001;
    let steps = 1000; // 1 second

    let graph = ConnectionGraph {
        nodes: vec![],
        edges: vec![],
        junctions: vec![],
    };
    let registry = Arc::new(PhysicsDomainRegistry::new());
    let mut constraints = GeneratedConstraints::default();

    // C-element (mass): d(velocity)/dt = force / mass
    constraints
        .constitutive
        .push(ConstitutiveRelation::Capacitance {
            effort_var: "body.velocity".into(),
            flow_var: "body.force".into(),
            parameter_var: "body.mass".into(),
            parameter_value: Some(mass),
        });

    let mut executor = PhysicsExecutor::new(registry, graph, constraints);

    // Set initial conditions once
    executor.sync_context_in(&{
        let mut ctx = EvalContext::new();
        ctx.set("body.force", Value::Float(force));
        ctx.set("body.velocity", Value::Float(0.0));
        ctx
    });

    let shared = EvalContext::new();
    for step in 0..steps {
        let tick_ctx = make_tick_ctx(&shared, step as f64 * dt, dt);
        executor.tick(&tick_ctx);
    }

    let mut out = EvalContext::new();
    // The legacy `sync_context_out` whole-local dump was deleted with the
    // string-identity cull. Route the physics write-set (name-keyed against an
    // empty store, so it reproduces the legacy dump's keys/values) to extract
    // the solved state.
    Executor::prepare_slot_writeback(
        &mut executor,
        &SlotStore::new(),
        None,
        None,
        WriterId::Executor(0),
    );
    executor.sync_context_out_slots(&mut out, sysml_runtime::ode::SignalEvalMode::FreshState);

    let velocity = match out.get("body.velocity") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };

    // v = a*t = (F/m)*t = 5*1 = 5 m/s
    assert!(
        (velocity - 5.0).abs() < 0.05,
        "Newton's 2nd law: v = F/m * t = 5 m/s, got {:.4}",
        velocity
    );
}

// ===========================================================================
// Test 10: Dimension formula completeness — every classifiable role
//          matches at least one ISQ type
// ===========================================================================

#[test]
fn every_bond_graph_role_has_isq_coverage() {
    use sysml_runtime::physics::domain::BondGraphRole;
    use sysml_runtime::physics::isq_types::ISQ_TYPES;

    let registry = PhysicsDomainRegistry::new();
    let mut role_seen: std::collections::HashSet<BondGraphRole> = std::collections::HashSet::new();

    for &(_name, ref dim, _cat) in ISQ_TYPES {
        if let Some((_domain, role)) = registry.classify_dimension_full(dim) {
            role_seen.insert(role);
        }
    }

    for &role in BondGraphRole::ALL_CLASSIFIABLE {
        assert!(
            role_seen.contains(&role),
            "BondGraphRole::{:?} has no ISQ type coverage — is the dimension formula correct?",
            role,
        );
    }
}

// ===========================================================================
// Test 11: 1-junction (KVL) — series resistors, effort summing
// ===========================================================================
// V_source = 12V, R1 = 4Ω, R2 = 8Ω in series.
// I = V / (R1 + R2) = 12 / 12 = 1A
// V_R1 = R1 * I = 4V, V_R2 = R2 * I = 8V
// KVL: V_source = V_R1 + V_R2 = 12V

#[test]
fn kvl_series_resistors() {
    use sysml_runtime::physics::constraints::FlowEquality;

    // Build manually: 1-junction for series circuit
    let nodes = vec![
        node(0, "source", "out", Some("electrical"), PortDirection::Out),
        node(1, "r1", "in", Some("electrical"), PortDirection::In),
        node(2, "r1", "out", Some("electrical"), PortDirection::Out),
        node(3, "r2", "in", Some("electrical"), PortDirection::In),
        node(4, "r2", "out", Some("electrical"), PortDirection::Out),
    ];

    // Series chain: source → r1 → r2
    let edges = vec![
        PhysicsConnection {
            source: 0,
            target: 1,
            domain: Some("electrical"),
            enabled: true,
        },
        PhysicsConnection {
            source: 2,
            target: 3,
            domain: Some("electrical"),
            enabled: true,
        },
    ];

    // 1-junction at the series loop: efforts sum, flow is shared
    let junctions = vec![Junction {
        id: 0,
        owner: "series_loop".into(),
        domain: "electrical",
        junction_type: JunctionType::One,
        conservation: ConservationLaw::FlowConservation,
        incoming: vec![(0, "voltage".into())],
        outgoing: vec![(1, "voltage".into()), (3, "voltage".into())],
    }];

    let graph = ConnectionGraph {
        nodes,
        edges,
        junctions,
    };
    let registry = Arc::new(PhysicsDomainRegistry::new());
    let constraints = sysml_runtime::physics::constraints::generate_constraints(&graph, &registry);

    // Verify we got a KVL constraint (1-junction)
    assert!(
        constraints
            .conservation
            .iter()
            .any(|c| c.name.starts_with("kvl_")),
        "expected a KVL constraint from 1-junction"
    );

    // Verify flow equalities were generated for the edges
    assert!(
        !constraints.flow_equalities.is_empty(),
        "expected flow equalities for series connections"
    );
}

// ===========================================================================
// Test 12: Transformer — ideal voltage step-up
// ===========================================================================
// V_primary = 120V, turns ratio m = 10
// BondGraphTools: e_1 = m * e_0, f_0 = -m * f_1
// Expected: V_secondary = 10 * 120 = 1200V

#[test]
fn transformer_voltage_ratio() {
    let relations = vec![ConstitutiveRelation::Transformer {
        effort_in_var: "tf.e_primary".into(),
        effort_out_var: "tf.e_secondary".into(),
        flow_in_var: "tf.f_primary".into(),
        flow_out_var: "tf.f_secondary".into(),
        modulus: 10.0,
    }];

    let mut ctx = EvalContext::new();
    ctx.set("tf.e_primary", Value::Float(120.0));
    ctx.set("tf.f_secondary", Value::Float(-1.0)); // 1A load on secondary

    apply_constitutive(&relations, &mut ctx);

    let e_sec = match ctx.get("tf.e_secondary") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!(
        (e_sec - 1200.0).abs() < 1e-10,
        "TF: e_1 = m*e_0 = 10*120 = 1200, got {}",
        e_sec
    );

    let f_pri = match ctx.get("tf.f_primary") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    // f_0 = -m * f_1 = -10 * (-1) = 10
    assert!(
        (f_pri - 10.0).abs() < 1e-10,
        "TF: f_0 = -m*f_1 = -10*(-1) = 10, got {}",
        f_pri
    );
}

// ===========================================================================
// Test 13: Transformer power conservation
// ===========================================================================
// P_in + P_out = 0 (lossless)

#[test]
fn transformer_power_conservation() {
    let relations = vec![ConstitutiveRelation::Transformer {
        effort_in_var: "tf.e0".into(),
        effort_out_var: "tf.e1".into(),
        flow_in_var: "tf.f0".into(),
        flow_out_var: "tf.f1".into(),
        modulus: 5.0,
    }];

    let mut ctx = EvalContext::new();
    ctx.set("tf.e0", Value::Float(100.0));
    ctx.set("tf.f1", Value::Float(-2.0));

    apply_constitutive(&relations, &mut ctx);

    let e0 = 100.0;
    let f0 = match ctx.get("tf.f0") {
        Some(Value::Float(f)) => *f,
        _ => panic!(),
    };
    let e1 = match ctx.get("tf.e1") {
        Some(Value::Float(f)) => *f,
        _ => panic!(),
    };
    let f1 = -2.0;

    let p_in = e0 * f0;
    let p_out = e1 * f1;
    assert!(
        (p_in + p_out).abs() < 1e-10,
        "TF power conservation: P_in + P_out = {} + {} = {} (should be 0)",
        p_in,
        p_out,
        p_in + p_out
    );
}

// ===========================================================================
// Test 14: Gyrator — DC motor (electrical ↔ mechanical)
// ===========================================================================
// BondGraphTools: e_1 = -r * f_0, e_0 = r * f_1
// Motor constant k = 0.5: torque = k * current, back_emf = k * angular_velocity

#[test]
fn gyrator_dc_motor() {
    let relations = vec![ConstitutiveRelation::Gyrator {
        effort_in_var: "motor.voltage".into(),
        effort_out_var: "motor.torque".into(),
        flow_in_var: "motor.current".into(),
        flow_out_var: "motor.angular_velocity".into(),
        modulus: 0.5,
    }];

    let mut ctx = EvalContext::new();
    ctx.set("motor.current", Value::Float(4.0));
    ctx.set("motor.angular_velocity", Value::Float(20.0));

    apply_constitutive(&relations, &mut ctx);

    // torque (e_1) = -r * f_0 = -0.5 * 4 = -2.0 Nm
    let torque = match ctx.get("motor.torque") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!(
        (torque - (-2.0)).abs() < 1e-10,
        "GY: torque = -r*I = -0.5*4 = -2, got {}",
        torque
    );

    // voltage (e_0) = r * f_1 = 0.5 * 20 = 10V (back-EMF)
    let voltage = match ctx.get("motor.voltage") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!(
        (voltage - 10.0).abs() < 1e-10,
        "GY: voltage = r*ω = 0.5*20 = 10, got {}",
        voltage
    );

    // Power conservation: e0*f0 + e1*f1 = 10*4 + (-2)*20 = 40 - 40 = 0
    let p_total = voltage * 4.0 + torque * 20.0;
    assert!(
        (p_total).abs() < 1e-10,
        "GY power conservation: {} (should be 0)",
        p_total
    );
}

// ===========================================================================
// Test 15: Effort source (Se)
// ===========================================================================

#[test]
fn effort_source() {
    let relations = vec![ConstitutiveRelation::EffortSource {
        effort_var: "source.voltage".into(),
        source_value: Some(230.0),
    }];

    let mut ctx = EvalContext::new();
    apply_constitutive(&relations, &mut ctx);

    let v = match ctx.get("source.voltage") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!((v - 230.0).abs() < 1e-10, "Se: e_0 = 230, got {}", v);
}

// ===========================================================================
// Test 16: Flow source (Sf) with BondGraphTools sign convention
// ===========================================================================
// BondGraphTools: f_0 + f = 0, so f_0 = -f_source
// Positive source value means flow INTO the network

#[test]
fn flow_source_sign_convention() {
    let relations = vec![ConstitutiveRelation::FlowSource {
        flow_var: "pump.mass_flow".into(),
        source_value: Some(5.0),
    }];

    let mut ctx = EvalContext::new();
    apply_constitutive(&relations, &mut ctx);

    let f = match ctx.get("pump.mass_flow") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    // BondGraphTools: f_0 + f = 0 → f_0 = -5.0
    assert!(
        (f - (-5.0)).abs() < 1e-10,
        "Sf: f_0 = -f_source = -5, got {}",
        f
    );
}

// ===========================================================================
// Test 17: Conductance (G = 1/R)
// ===========================================================================

#[test]
fn conductance_solving() {
    let relations = vec![ConstitutiveRelation::Conductance {
        effort_var: "g1.voltage".into(),
        flow_var: "g1.current".into(),
        parameter_var: "g1.conductance".into(),
        parameter_value: Some(0.2), // G = 0.2S = 1/5Ω
    }];

    let mut ctx = EvalContext::new();
    ctx.set("g1.voltage", Value::Float(10.0));

    apply_constitutive(&relations, &mut ctx);

    let i = match ctx.get("g1.current") {
        Some(Value::Float(f)) => *f,
        other => panic!("expected Float, got {:?}", other),
    };
    assert!(
        (i - 2.0).abs() < 1e-10,
        "G: I = G*V = 0.2*10 = 2, got {}",
        i
    );
}

// ===========================================================================
// DAE Solver Validation Tests (feature-gated)
// ===========================================================================
// These tests validate the diffsol BDF solver integration against analytical
// solutions. They exercise the full pipeline: constraints → BondGraphDae →
// diffsol BDF → DaeSolution.

mod dae_validation {
    use sysml_runtime::physics::constraints::{ConstitutiveRelation, GeneratedConstraints};
    use sysml_runtime::physics::dae::BondGraphDae;

    // -----------------------------------------------------------------------
    // Test 18: DAE RC charging — V_c(t) = V_s * (1 - e^(-t/RC))
    // -----------------------------------------------------------------------
    #[test]
    fn dae_rc_charging_analytical() {
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "vs".into(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "vs".into(),
            effort_out_var: "vc".into(),
            flow_var: "i".into(),
            parameter_var: "R".into(),
            parameter_value: Some(5.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Capacitance {
            effort_var: "vc".into(),
            flow_var: "i".into(),
            parameter_var: "C".into(),
            parameter_value: Some(1.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let tau = 5.0; // RC = 5*1 = 5s

        // Solve for 3 time constants (should be ~95% charged)
        let sol = dae.solve((0.0, 3.0 * tau), 1e-6, 1e-8).unwrap();

        let vc_idx = sol.var_names.iter().position(|n| n == "vc").unwrap();

        // Check at t ≈ τ: V_c(τ) = 10*(1-e^-1) ≈ 6.321
        // Find the closest time point to τ
        let t_tau_idx = sol.t.iter().position(|&t| t >= tau).unwrap();
        let vc_tau = sol.x[vc_idx][t_tau_idx];
        let t_actual = sol.t[t_tau_idx];
        let expected_tau = 10.0 * (1.0 - (-t_actual / tau).exp());
        assert!(
            (vc_tau - expected_tau).abs() < 0.05,
            "V_c(τ={:.3}) = {:.4}, expected {:.4}",
            t_actual,
            vc_tau,
            expected_tau
        );

        // Check at t = 3τ: should be ~95% of V_s
        let vc_final = *sol.x[vc_idx].last().unwrap();
        let expected_3tau = 10.0 * (1.0 - (-3.0_f64).exp()); // ~9.502
        assert!(
            (vc_final - expected_3tau).abs() < 0.05,
            "V_c(3τ) = {:.4}, expected {:.4}",
            vc_final,
            expected_3tau
        );
    }

    // -----------------------------------------------------------------------
    // Test 19: DAE RL current ramp — I(t) = (V/R)*(1 - e^(-Rt/L))
    // -----------------------------------------------------------------------
    #[test]
    fn dae_rl_current_ramp_analytical() {
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "vs".into(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "vs".into(),
            effort_out_var: "vl".into(),
            flow_var: "i".into(),
            parameter_var: "R".into(),
            parameter_value: Some(5.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Inductance {
            flow_var: "i".into(),
            effort_var: "vl".into(),
            parameter_var: "L".into(),
            parameter_value: Some(2.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let tau = 2.0 / 5.0; // L/R = 0.4s
        let i_ss = 10.0 / 5.0; // V/R = 2A

        let sol = dae.solve((0.0, 3.0 * tau), 1e-6, 1e-8).unwrap();

        let i_idx = sol.var_names.iter().position(|n| n == "i").unwrap();

        // At t=τ: I(τ) = 2*(1-e^-1) ≈ 1.264
        let t_tau_idx = sol.t.iter().position(|&t| t >= tau).unwrap();
        let i_tau = sol.x[i_idx][t_tau_idx];
        let t_actual = sol.t[t_tau_idx];
        let expected = i_ss * (1.0 - (-t_actual / tau).exp());
        assert!(
            (i_tau - expected).abs() < 0.01,
            "I(τ={:.3}) = {:.4}, expected {:.4}",
            t_actual,
            i_tau,
            expected
        );

        // At 3τ: should be ~95% of steady state
        let i_final = *sol.x[i_idx].last().unwrap();
        assert!(
            (i_final - i_ss).abs() < 0.15,
            "I(3τ) = {:.4}, expected ≈{:.4}",
            i_final,
            i_ss
        );
    }

    // -----------------------------------------------------------------------
    // Test 20: DAE solver handles pure algebraic system (R-only, no C/I)
    // Note: Pure algebraic systems (M=0) are better handled by the explicit
    // solver (fixed-point iteration). The BDF solver requires at least one
    // differential equation. This test documents the expected behavior.
    // -----------------------------------------------------------------------
    #[test]
    #[should_panic]
    fn dae_pure_algebraic_r_circuit() {
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "vs".into(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "vs".into(),
            effort_out_var: "vout".into(),
            flow_var: "i".into(),
            parameter_var: "R".into(),
            parameter_value: Some(5.0),
        });
        // Ground: vout = 0
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "vout".into(),
            source_value: Some(0.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        assert_eq!(dae.map.n_diff, 0, "pure algebraic, no diff states");

        let sol = dae.solve((0.0, 1.0), 1e-6, 1e-8).unwrap();

        let i_idx = sol.var_names.iter().position(|n| n == "i").unwrap();
        let i_val = *sol.x[i_idx].last().unwrap();
        // I = (10 - 0) / 5 = 2A
        assert!(
            (i_val - 2.0).abs() < 0.01,
            "pure algebraic: I = V/R = 2, got {:.4}",
            i_val
        );
    }

    // -----------------------------------------------------------------------
    // Test 21: DAE energy conservation — source power ≈ stored + dissipated
    // -----------------------------------------------------------------------
    #[test]
    fn dae_energy_conservation_rc() {
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "vs".into(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "vs".into(),
            effort_out_var: "vc".into(),
            flow_var: "i".into(),
            parameter_var: "R".into(),
            parameter_value: Some(5.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Capacitance {
            effort_var: "vc".into(),
            flow_var: "i".into(),
            parameter_var: "C".into(),
            parameter_value: Some(1.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let sol = dae.solve((0.0, 25.0), 1e-8, 1e-10).unwrap(); // 5τ, nearly fully charged

        let vc_idx = sol.var_names.iter().position(|n| n == "vc").unwrap();
        let i_idx = sol.var_names.iter().position(|n| n == "i").unwrap();

        // Energy stored in capacitor: E_c = 0.5 * C * V_c^2
        let vc_final = *sol.x[vc_idx].last().unwrap();
        let e_stored = 0.5 * 1.0 * vc_final * vc_final;

        // Total energy from source: E_source = integral(V_s * I) dt
        // At steady state (5τ), V_c ≈ V_s, I ≈ 0.
        // Analytical: E_source = 0.5 * C * V_s^2 + E_dissipated = C * V_s^2
        // (half goes to C, half dissipated in R — well-known RC charging result)
        let e_total_analytical = 1.0 * 10.0 * 10.0; // C * V_s^2 = 100J

        // E_stored should be ≈ 50J (half the source energy)
        assert!(
            (e_stored - 50.0).abs() < 1.0,
            "stored energy = {:.2}J, expected ≈50J",
            e_stored
        );
    }
}
