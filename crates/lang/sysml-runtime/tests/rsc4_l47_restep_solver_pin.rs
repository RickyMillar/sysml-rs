//! RSC-4.3 L47 — restep-eligibility structurally requires RK45.
//!
//! RED at 454591c1 (the RSC-4.3 Wave 1 landing): WS-B4's auto-stiffness
//! classifier picks a solver against the SESSION'S requested dt, independent
//! of `wire_when_crossings_for_pair`'s restep-eligibility decision. A
//! restep-eligible ODE (a paired SM has a qualifying `accept when` comparator
//! trigger) whose dynamics are stiff at the requested dt silently got a
//! `BdfSolver` — which never gained the Wave-1 `restore_state`/
//! `integrate_interval` overrides — so the first located crossing hit
//! `Executor::restore_state`'s trait default (`false` unconditionally) and
//! panicked with a MISLABELED "length mismatch" (the lengths matched; the
//! solver simply doesn't implement the re-step protocol at all). This is
//! exactly the coarse-dt panic on a stiff restep-eligible ODE workspace at the
//! sim-app's default `dt_ms=1.0` (see commit body of the L47 fix).

use std::sync::Arc;

use sysml_core::{elaborate, Value};
use sysml_ide_db::eval_context_seed;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::{CompileError, ModelCompiler};
use sysml_runtime::orchestrator::Orchestrator;

/// Stiff first-order decay `dx/dt = -k*x`, k=1000 (time constant 1 ms) — the
/// SAME stiffness proven in `bdf_solver_routing.rs`
/// (`unannotated_stiff_system_auto_switches_to_bdf`: at dt=10ms,
/// `|λ|·dt = 1000 * 0.01 = 10`, far above the explicit-stability margin). The
/// ONLY difference from that fixture: `Flipper` has a qualifying
/// `accept when x <= 0.5` trigger, making this ODE restep-eligible — the
/// exact WS-B4-vs-restep-eligibility collision.
fn model(tool_name: Option<&str>) -> String {
    let annotation = match tool_name {
        Some(name) => format!(
            r#"
        @ToolExecution {{
            attribute toolName = "{name}";
        }}"#
        ),
        None => String::new(),
    };
    format!(
        r#"
package StiffRestepPin {{
    part def Decay {{{annotation}
        attribute k : Real default 1000.0;
        out attribute x : Real default 1.0;

        action def Dynamics :> ContinuousStateSpaceDynamics {{
            calc def XDeriv :> GetDerivative {{
                return dxdt = 0 - k * x;
            }}
        }}
    }}

    state def Flipper {{
        in attribute x : Real;

        state active;
        state tripped;

        entry; then active;

        transition trip
            first active
            accept when x <= 0.5
            then tripped;
    }}
}}
"#
    )
}

/// Same stiffness, no `accept when` trigger anywhere (`Runner` has no
/// transitions) — NOT restep-eligible. Used for the negative twin: an
/// explicit BDF annotation with no qualifying trigger must build fine and
/// keep BDF, exactly as `bdf_solver_routing.rs` already established.
fn model_no_trigger(tool_name: &str) -> String {
    format!(
        r#"
package StiffNoTrigger {{
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

fn build(source: &str, dt_ms: f64, max_ms: f64) -> Result<Orchestrator, CompileError> {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("Model.sysml", source.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    compiler.build_workspace_orchestrator(
        base_ctx,
        Some(precompiled),
        None,
        None,
        None,
        &[],
        Some(dt_ms),
        Some(max_ms),
    )
}

fn ode_kind(orch: &Orchestrator) -> Option<String> {
    orch.subsystems()
        .iter()
        .find(|s| matches!(s.executor.kind_label(), "bdf" | "ode" | "ode45"))
        .map(|s| s.executor.kind_label().to_owned())
}

/// THE regression: a restep-eligible ODE whose dynamics are stiff at the
/// requested (coarse) dt must build on RK45, not BDF — and must run several
/// ticks (through the crossing) without panicking. RED at 454591c1: this
/// exact scenario panicked with "RS014: ... rejected a state rollback
/// (length mismatch)" on the first located crossing.
#[test]
fn stiff_restep_eligible_ode_pins_rk45_not_bdf() {
    let mut orch = build(&model(None), 10.0, 300.0).expect("orchestrator should build");
    orch.context.set("x".to_owned(), Value::Float(1.0));

    // Step through the crossing (x decays from 1.0 through 0.5 within the
    // first tick or two at dt=10ms) FIRST — must not panic. RED at
    // 454591c1: this panics with "RS014: ... rejected a state rollback
    // (length mismatch)" the moment the located crossing re-steps a
    // BdfSolver, which never implements `restore_state`.
    for _ in 0..10 {
        let _ = orch.step();
    }

    assert_eq!(
        ode_kind(&orch).as_deref(),
        Some("ode45"),
        "a restep-eligible ODE must be pinned to RK45 even though WS-B4 would \
         otherwise classify it stiff and route to BDF (|λ|·dt = 10 ≫ margin)"
    );
}

/// The orthogonal explicit-annotation case: `@ToolExecution { toolName =
/// "builtin:ode-bdf" }` on a restep-eligible ODE is a hard CompileError —
/// overriding an explicit user choice silently would be wrong (unlike the
/// inferred-default case above, which the compiler is free to override).
#[test]
fn explicit_bdf_annotation_on_restep_eligible_ode_is_hard_compile_error() {
    let err = match build(&model(Some("builtin:ode-bdf")), 10.0, 300.0) {
        Err(e) => e,
        Ok(_) => panic!("explicit BDF + restep-eligible must fail the build, not silently degrade"),
    };
    assert!(
        err.message.contains("BDF") && err.message.contains("re-step"),
        "CompileError should name the BDF/re-step collision, got: {}",
        err.message
    );
}

/// Negative twin: explicit BDF with NO qualifying `accept when` trigger
/// anywhere (not restep-eligible) must build fine and keep BDF — the
/// collision is specifically about restep-eligibility, not BDF in general.
#[test]
fn explicit_bdf_annotation_without_restep_eligibility_builds_and_keeps_bdf() {
    let orch = build(&model_no_trigger("builtin:ode-bdf"), 10.0, 300.0)
        .expect("no qualifying accept-when trigger means no collision — must build");
    assert_eq!(
        ode_kind(&orch).as_deref(),
        Some("bdf"),
        "an explicit BDF annotation with no restep-eligible pairing must keep BDF"
    );
}
