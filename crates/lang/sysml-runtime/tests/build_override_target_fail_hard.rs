//! GAP 2 regression: a build-time override that names no real target must fail
//! hard, the compiler-harness counterpart of the session path's RS002
//! (`Orchestrator::apply_overrides_with_aliases`).
//!
//! Before this fix, `build_orchestrator[_from_prepared]` applied its
//! `overrides` param through a bare-keyed `HashMap` with an
//! `.unwrap_or(default)` fallback: a typo'd or wrongly-qualified key baked
//! nothing and was silently ignored. The concrete harm: `sysml.ode_sweep` over
//! a mistyped `parameter_name` silently produced a *flat* sweep (every variant
//! identical to the baseline) with no diagnostic. ODE parameters are baked into
//! the solver RHS from their *bare* key, so a qualified spelling can never
//! reach the constant — that earns a precise "use the bare key" error.

use sysml_core::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

const MODEL: &str = r#"
package G {
    part def Sys {
        out attribute x : Real default 1.0;
        attribute rate : Real default 0.5;
        calc def XDeriv :> GetDerivative {
            return dxdt = 0.0 - rate * x;
        }
    }
    state def Ctrl {
        entry action init;
        state running;
        transition init then running;
    }
}
"#;

fn build(overrides: &[(&str, &str)]) -> Result<(), String> {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("g.sysml".to_string(), MODEL.to_string())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let ov: Vec<(String, String)> = overrides
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    compiler
        .build_orchestrator("Ctrl", &ov, Some(1.0), Some(100.0))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The real ODE parameter, spelled bare, still overrides — no behavior change.
#[test]
fn valid_bare_param_override_builds() {
    build(&[("rate", "0.9")]).expect("a real bare param override must build");
}

/// A typo'd target is a hard error naming the offending key — was a silent
/// flat sweep.
#[test]
fn unknown_override_target_fails_hard() {
    let err = build(&[("raet", "0.9")]).expect_err("typo'd override target must fail hard");
    assert!(
        err.contains("unknown override target") && err.contains("raet"),
        "error must name the offending key: {err}"
    );
}

/// A qualified spelling of an ODE parameter is refused with precise guidance —
/// it can never reach the RHS-baked constant.
#[test]
fn qualified_param_override_gives_precise_error() {
    let err = build(&[("Sys.rate", "0.9")])
        .expect_err("qualified ODE-param override must fail hard, not silently no-op");
    assert!(
        err.contains("use the bare key") && err.contains("rate"),
        "error must guide to the bare key: {err}"
    );
}
