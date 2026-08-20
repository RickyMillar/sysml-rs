//! WS-B1 gate (hybrid-sim-robustness-plan §5): `@ToolExecution { toolName =
//! "builtin:ode-bdf" }` routes a model's ODE to the implicit BDF solver, and BDF
//! integrates a STIFF system stably where the explicit default (RK4) diverges at
//! the same coarse step. Generic: nothing fixture-specific.

use sysml_core::{elaborate, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::ExecutionSnapshot;

/// Stiff first-order decay `dx/dt = -k*x`, k=1000 (time constant 1 ms). At a
/// coarse dt=10 ms the explicit RK4 amplification factor `|1 - k*dt| = 9 ≫ 1`
/// so RK4 blows up; implicit BDF is unconditionally stable for linear decay.
/// A trivial state machine lets the single-SM `build_orchestrator` path engage.
/// Parses standalone (SSR detection is structural; `@ToolExecution` is read off
/// the `toolName` attribute value).
fn model(tool_name: &str) -> String {
    format!(
        r#"
package StiffDecay {{
    part def Decay {{
        @ToolExecution {{
            attribute toolName = "{tool_name}";
        }}
        attribute k : Real default 1000.0;
        out attribute x : Real default 1.0;

        action def Dynamics :> ContinuousStateSpaceDynamics {{
            calc def XDeriv :> GetDerivative {{
                return dxdt = 0 - k * x;
            }}
        }}
    }}

    state def Runner {{
        state active;
        state idle;
    }}
}}
"#
    )
}

fn run(tool_name: &str, dt_ms: f64, max_ms: f64) -> Option<ExecutionSnapshot> {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("StiffDecay.sysml", model(tool_name))]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let mut orch = compiler
        .build_orchestrator("Runner", &[], Some(dt_ms), Some(max_ms))
        .expect("single-SM ODE orchestrator builds");
    orch.context.set("x".to_owned(), Value::Float(1.0));
    orch.run_to_completion()
}

/// The kind label of the ODE subsystem in the final snapshot ("bdf"/"ode"/"ode45").
fn ode_kind(snap: &Option<ExecutionSnapshot>) -> Option<String> {
    snap.as_ref().and_then(|s| {
        s.subsystem_states
            .values()
            .find(|st| matches!(st.kind, "bdf" | "ode" | "ode45"))
            .map(|st| st.kind.to_string())
    })
}

fn final_x(snap: &Option<ExecutionSnapshot>) -> f64 {
    snap.as_ref()
        .and_then(|s| match s.variables.get("x") {
            Some(Value::Float(f)) => Some(*f),
            _ => None,
        })
        .unwrap_or(f64::NAN)
}

#[test]
fn bdf_tool_name_routes_to_bdf_and_is_stable_on_stiff_system() {
    // dt = 10 ms over 300 ms (300 steps); ~300 time constants → x → 0.
    let snap = run("builtin:ode-bdf", 10.0, 300.0);

    // (1) Routing: the ODE subsystem is the implicit BDF solver.
    assert_eq!(
        ode_kind(&snap).as_deref(),
        Some("bdf"),
        "toolName=builtin:ode-bdf must select the BDF solver"
    );

    // (2) Stability: BDF stays bounded and decays toward 0 on the stiff system
    //     at a step where explicit RK4 would diverge.
    let x = final_x(&snap);
    assert!(
        x.is_finite() && x.abs() < 0.1,
        "BDF should integrate the stiff decay stably to ≈0, got x={x}"
    );
}

#[test]
fn explicit_rk4_diverges_on_same_stiff_system_at_coarse_step() {
    // Contrast: the explicit fixed-step solver at the SAME coarse step blows up —
    // proving the solver choice (not the model) is what made the BDF run stable.
    // `builtin:ode-rk4` is an explicit opt-in (WS-B2); it is no longer the default.
    let snap = run("builtin:ode-rk4", 10.0, 300.0);
    assert_eq!(ode_kind(&snap).as_deref(), Some("ode"), "rk4 selected");
    let x = final_x(&snap);
    assert!(
        !x.is_finite() || x.abs() > 10.0,
        "explicit RK4 should be unstable on the stiff system at dt=10ms, got x={x}"
    );
}

#[test]
fn unannotated_stiff_system_auto_switches_to_bdf() {
    // WS-B4: an un-annotated ODE (no `@ToolExecution`) is classified at build
    // time. dx/dt = -1000 x at a coarse dt=10 ms gives |λ|·dt = 1000·0.01 = 10,
    // far above the explicit-stability margin → the auto-switch selects the
    // implicit BDF solver instead of the RK45 default. The choice is observable
    // as the ODE subsystem's `kind`. BDF integrates the stiff decay stably to 0.
    let snap = run("ssr:GetDerivative", 10.0, 300.0);
    assert_eq!(
        ode_kind(&snap).as_deref(),
        Some("bdf"),
        "an un-annotated STIFF ODE must auto-switch to BDF (|λ|·dt = 10 ≫ margin)"
    );
    let x = final_x(&snap);
    assert!(
        x.is_finite() && x.abs() < 0.1,
        "auto-selected BDF should integrate the stiff decay stably to ≈0, got x={x}"
    );
}

/// Non-stiff first-order decay `dx/dt = -x` (time constant 1 s). At dt=10 ms
/// `|λ|·dt = 0.01 ≪ margin` — the explicit step resolves the dynamics fine, so
/// the auto-switch must leave it on the RK45 default (BDF would be needless
/// overhead). Same shape as `model`, smaller rate constant.
fn nonstiff_model() -> String {
    r#"
package SoftDecay {
    part def Decay {
        attribute k : Real default 1.0;
        out attribute x : Real default 1.0;

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
"#
    .to_owned()
}

#[test]
fn unannotated_nonstiff_system_stays_on_rk45() {
    // WS-B4: a NON-stiff un-annotated ODE must NOT be auto-switched to BDF —
    // the explicit RK45 default is correct and cheaper.
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("SoftDecay.sysml", nonstiff_model())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let mut orch = compiler
        .build_orchestrator("Runner", &[], Some(10.0), Some(3000.0))
        .expect("single-SM ODE orchestrator builds");
    orch.context.set("x".to_owned(), Value::Float(1.0));
    let snap = orch.run_to_completion();
    assert_eq!(
        ode_kind(&snap).as_deref(),
        Some("ode45"),
        "an un-annotated NON-stiff ODE must stay on the RK45 default (|λ|·dt ≈ 0.01 ≪ margin)"
    );
    let x = final_x(&snap);
    assert!(
        x.is_finite() && x.abs() < 0.1,
        "RK45 should integrate the soft decay to ≈0 over 3 s, got x={x}"
    );
}
