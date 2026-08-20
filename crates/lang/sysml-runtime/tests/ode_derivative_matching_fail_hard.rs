//! GAP 1 regression: the `GetDerivative` → state-variable matcher must map
//! each state var to exactly one derivative, and fail hard otherwise.
//!
//! Before this fix the multi-state matcher used bare SUBSTRING containment
//! (`return_name.contains(stem)` / `calc_name.contains(stem)`) with two silent
//! degradations:
//!   * a state var matching NO derivative silently took the constant `"0"`
//!     (`.unwrap_or("0")`) — a zero RHS, i.e. a frozen state, with no error;
//!   * a state var whose stem is a substring of another's (e.g. `pressure`
//!     inside `chamberPressure`) matched the FIRST of several derivatives
//!     (`.find(...)`), silently wiring the wrong physics.
//!
//! The matcher now prefers the documented exact conventions — a `d<state>dt`
//! return name or a `<State>Derivative` calc name (on the `t_`/`x_`-stripped
//! stem) — and consults the looser substring match only when no exact match
//! exists, requiring it to be unambiguous. Zero or ambiguous matches are
//! recorded on the detection and enforced as a hard `CompileError` at build.

use sysml_core::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

fn compile(src: &str) -> ModelCompiler {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("t.sysml".to_string(), src.to_string())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    ModelCompiler::new(graph)
}

/// A state var whose name is a substring of another's must be disambiguated by
/// the exact `d<state>dt` return shape — not silently wired to the first
/// substring hit. `pressure` ⊂ `chamberPressure` is the canonical collision.
#[test]
fn substring_collision_disambiguated_by_exact_return_shape() {
    let src = r#"
    package G1 {
        part def Osc {
            out attribute pressure : Real default 1.0;
            out attribute chamberPressure : Real default 2.0;
            attribute k : Real default 3.0;
            calc def PressureDeriv :> GetDerivative {
                return dpressuredt = 0.0 - k * pressure;
            }
            calc def ChamberDeriv :> GetDerivative {
                return dchamberPressuredt = 0.0 - k * chamberPressure;
            }
        }
    }
    "#;
    let compiler = compile(src);
    let odes = compiler.detect_ode_from_ssr();
    let ode = odes
        .iter()
        .find(|o| o.name.as_deref() == Some("Osc"))
        .expect("Osc ODE detected");

    assert!(
        ode.derivative_match_errors.is_empty(),
        "exact shapes must resolve cleanly, got errors: {:?}",
        ode.derivative_match_errors
    );

    // Each state var must own the derivative that reads IT, not its neighbour.
    for (var, expr) in ode.state_vars.iter().zip(&ode.derivative_exprs) {
        match var.as_str() {
            "pressure" => assert!(
                expr.contains("pressure") && !expr.contains("chamberPressure"),
                "state 'pressure' wired to wrong derivative: {expr}"
            ),
            "chamberPressure" => assert!(
                expr.contains("chamberPressure"),
                "state 'chamberPressure' wired to wrong derivative: {expr}"
            ),
            other => panic!("unexpected state var {other}"),
        }
    }
}

/// A state var that matches no derivative is a hard error naming it — never a
/// silent constant-0 RHS.
#[test]
fn non_matching_state_var_records_hard_error() {
    let src = r#"
    package G2 {
        part def Osc {
            out attribute alpha : Real default 1.0;
            out attribute beta : Real default 2.0;
            calc def AlphaDeriv :> GetDerivative {
                return dalphadt = 0.0 - alpha;
            }
            calc def StrayDeriv :> GetDerivative {
                return dgammadt = 0.0 - beta;
            }
        }
    }
    "#;
    let compiler = compile(src);
    let odes = compiler.detect_ode_from_ssr();
    let ode = odes.iter().find(|o| o.name.as_deref() == Some("Osc")).unwrap();

    // `beta` matches no `dbetadt` return nor a `betaDerivative` calc.
    assert!(
        ode.derivative_match_errors.iter().any(|e| e.contains("beta")),
        "expected an unresolved-derivative error naming 'beta', got: {:?}",
        ode.derivative_match_errors
    );
    let err = ode
        .ensure_derivatives_matched()
        .expect_err("build enforcement must reject an unresolved derivative");
    assert!(err.to_string().contains("beta"));
}

/// Two derivatives that both substring-match a state var, with neither exact,
/// is an ambiguity — a hard error listing the collisions, not a first-match.
#[test]
fn ambiguous_non_exact_match_records_hard_error() {
    let src = r#"
    package G3 {
        part def Osc {
            out attribute temp : Real default 1.0;
            out attribute load : Real default 2.0;
            calc def DerivOne :> GetDerivative {
                return dtempAdt = 0.0 - temp;
            }
            calc def DerivTwo :> GetDerivative {
                return dtempBdt = 0.0 - load;
            }
        }
    }
    "#;
    let compiler = compile(src);
    let odes = compiler.detect_ode_from_ssr();
    let ode = odes.iter().find(|o| o.name.as_deref() == Some("Osc")).unwrap();

    // `temp` is a substring of both `dtempAdt` and `dtempBdt`; neither is the
    // exact `dtempdt`, so the match is ambiguous.
    assert!(
        ode.derivative_match_errors
            .iter()
            .any(|e| e.contains("temp") && (e.contains("ambiguous") || e.contains("dtempAdt"))),
        "expected an ambiguity error for 'temp', got: {:?}",
        ode.derivative_match_errors
    );
    assert!(ode.ensure_derivatives_matched().is_err());
}
