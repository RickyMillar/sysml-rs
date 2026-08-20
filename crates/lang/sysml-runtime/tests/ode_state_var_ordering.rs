//! Regression test for a HashMap/HashSet-iteration-order nondeterminism bug
//! in `ModelCompiler::detect_ode_from_ssr` (ledger; task #8).
//!
//! `ModelGraph::children_of` is backed by `FxHashSet<ElementId>`, not an
//! ordered collection. `detect_ode_from_ssr`'s state-var extraction used to
//! iterate `children_of(owner_id)` directly, so with a single `GetDerivative`
//! calc (state_vars.len() == 1, every corpus model until now) the order never
//! mattered — but with two or more state vars in one part, `state_vars` came
//! out in hash-bucket order, not source declaration order. Downstream code
//! indexes ODE state vectors positionally (`get_state()[i]`), so this
//! silently mismatched values to the wrong state variable for any
//! multi-state-var part.
//!
//! Uses a minimal synthetic fixture (not the parked oscillator fixture files) with
//! two `GetDerivative` calcs on one part, named to mirror the real-world
//! case (`B` / `faultIntegral`) that the WS-D probe confirmed reproduces the
//! reordering.

use sysml_core::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

const SOURCE: &str = r#"
package OrderingBug {
    part def FaultModel {
        out attribute B : ScalarValues::Real = 0.0;
        out attribute faultIntegral : ScalarValues::Real = 0.0;

        calc def BDerivative :> GetDerivative {
            return dBdt = 1.0;
        }
        calc def FaultIntegralDerivative :> GetDerivative {
            return dFIdt = 2.0;
        }
    }
}
"#;

fn detect_state_vars() -> Vec<String> {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("ordering_bug.sysml".to_owned(), SOURCE.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);

    let detections = compiler.detect_ode_from_ssr();
    assert_eq!(
        detections.len(),
        1,
        "expected exactly one OdeDetection for FaultModel, got {detections:?}"
    );
    detections[0].state_vars.clone()
}

/// The core correctness assertion: state_vars must come out in source
/// declaration order (`B` then `faultIntegral`), not hash-bucket order.
#[test]
fn state_vars_match_declaration_order() {
    let state_vars = detect_state_vars();
    assert_eq!(
        state_vars,
        vec!["B".to_owned(), "faultIntegral".to_owned()],
        "state_vars must preserve source declaration order, not \
         children_of's FxHashSet iteration order"
    );
}

/// Build determinism: parsing + detecting from the identical source twice
/// must produce byte-identical state_vars, not just "some" order.
#[test]
fn state_vars_order_is_build_to_build_stable() {
    let first = detect_state_vars();
    let second = detect_state_vars();
    assert_eq!(first, second, "state_vars order must be stable build-to-build");
}
