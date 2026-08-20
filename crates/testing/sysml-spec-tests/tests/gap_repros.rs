//! GAP REPROS — minimal standalone evidence that GAP-1 and GAP-SM-EXEC are
//! IMPLEMENTATION defects in the compiled IR, NOT artifacts of how the
//! conformance gates construct things.
//!
//! Each repro is self-contained: real `.sysml` source → parse → elaborate →
//! compile, then it ASSERTS the buggy IR/verdict mechanism directly (so these
//! tests PASS — they pin the diagnosis) and prints the compiled IR for the
//! record. Run with:
//!   cargo test -p sysml-spec-tests --test gap_repros -- --nocapture
//!
//! These are the adversarial cross-check the director asked for: they prove the
//! defect is in `ConstraintIR` (no `isNegated`) and in `compile_state` /
//! `compile_transition_usage` (exit/do/effect bodies lowered to `Simple("")`),
//! not in the gate fixtures.

use sysml_core::{ElementKind, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::constraints::{extract_and_precompile, EvalContext};
use sysml_runtime::TransitionActionIR;

fn parse(source: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("gap_repros.sysml", source)]);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "fixture must parse cleanly: {errors:?}");
    result.graph
}

// ===========================================================================
// GAP-1 REPRO — `assert not constraint` negation is dropped between the parsed
// element and the evaluated verdict.
//
// CHAIN (each step asserted below):
//   1. Parser + elaboration DO capture the negation: the AssertConstraintUsage
//      element carries prop `isNegated = Bool(true)`.
//   2. But `ConstraintIR` (sysml-runtime/src/lib.rs) has only {expr, description,
//      owner_id} — no negation field — so `extract_constraints` cannot carry it.
//   3. Therefore the per-instance check verdict is the UN-INVERTED inner boolean:
//      `assert not constraint { x < 10 }` with x=5 reports SATISFIED, when the
//      spec (SysML §7.20) says a negated assertion whose inner constraint is
//      TRUE must be VIOLATED.
// ===========================================================================

#[test]
fn gap1_repro_negation_lost_between_element_and_verdict() {
    let src = "part def P {\n\
               \x20   attribute x = 5;\n\
               \x20   assert not constraint c { x < 10 }\n\
               }\n\
               part p : P;\n";
    let mut graph = parse(src);
    sysml_core::elaborate::elaborate(&mut graph);

    // STEP 1 — the negation IS present on the parsed+elaborated element.
    let elem = graph
        .elements_by_kind(&ElementKind::AssertConstraintUsage)
        .find(|e| e.name.as_deref() == Some("c"))
        .expect("assert constraint element present");
    let is_negated = elem.get_prop("isNegated").cloned();
    println!("GAP-1 step 1 — element `c` isNegated prop = {is_negated:?}");
    assert_eq!(
        is_negated,
        Some(Value::Bool(true)),
        "parser+elaboration captured the negation on the element"
    );

    // STEP 2 — the compiled ConstraintIR (Debug) shows no negation field; print it.
    let precompiled = extract_and_precompile(&graph);
    println!("GAP-1 step 2 — compiled ConstraintIR set = {precompiled:#?}");

    // STEP 3 — the per-instance check verdict is the UN-INVERTED inner boolean.
    let compiler = ModelCompiler::new(graph);
    let results = compiler
        .evaluate_constraints_per_instance(&precompiled, &EvalContext::new())
        .expect("constraint evaluation");
    let v = results
        .iter()
        .find_map(|r| {
            // the single negated-assert constraint
            Some(&r.result)
        })
        .expect("one per-instance verdict");
    println!(
        "GAP-1 step 3 — verdict: satisfied={}, inconclusive={} (inner x<10 with x=5 is TRUE)",
        v.satisfied, v.inconclusive
    );
    // GAP-1 CLOSED: `ConstraintIR.is_negated` (set from the element's isNegated
    // at extraction) now inverts the decided verdict at the eval chokepoint, so
    // a negated assertion whose inner constraint is TRUE is VIOLATED (SysML
    // §7.20). This was the repro that pinned the bug; it is now a regression
    // guard — if it flips back to satisfied, the negation was dropped again.
    assert!(
        !v.satisfied && !v.inconclusive,
        "REGRESSION: negated assertion with inner-true (x<10, x=5) must be VIOLATED \
         (SysML §7.20). A satisfied verdict here means ConstraintIR.is_negated was dropped."
    );
}

// ===========================================================================
// GAP-SM-EXEC REPRO — exit / do / transition-effect action BODIES are dropped
// at compile (lowered to `Simple("")` / left as a bare string), while ENTRY
// action bodies are lowered to `Structured { assignments }`.
//
// Asserted directly on the compiled StateMachineIR (no execution needed):
//   - state B `entry_action` IS `Structured` (entry lowering works).
//   - state A `exit_action` is NOT `Structured` (body dropped).
//   - state A `do_action`   is NOT `Structured` (body dropped).
//   - the transition `action` (effect) is NOT `Structured` (effect dropped).
// ===========================================================================

fn is_structured(a: &Option<TransitionActionIR>) -> bool {
    matches!(a, Some(TransitionActionIR::Structured { .. }))
}

#[test]
fn gapsmexec_repro_exit_do_effect_bodies_dropped_in_ir() {
    let src = r#"
        package ReproSM {
            state def RSM {
                state A {
                    do action   { aDo = aDo + 1; }
                    exit action { aExit = seq; seq = seq + 1; }
                }
                state B {
                    entry action { bEntry = 1; }
                }
                transition first A accept go do action { eff = 1; } then B;
            }
        }
    "#;
    let compiler = ModelCompiler::new(parse(src));
    let ir = compiler
        .compile_state_machine("RSM")
        .expect("state machine compiles");

    println!("GAP-SM-EXEC — compiled StateMachineIR =\n{ir:#?}");

    let state_a = ir.states.iter().find(|s| s.name == "A").expect("state A");
    let state_b = ir.states.iter().find(|s| s.name == "B").expect("state B");
    let effect = ir.transitions.iter().find_map(|t| t.action.clone());

    println!(
        "GAP-SM-EXEC — A.exit={:?}  A.do={:?}  B.entry={:?}  transition.effect={:?}",
        state_a.exit_action, state_a.do_action, state_b.entry_action, effect
    );

    // ALL FOUR bodies (entry/exit/do + transition effect) now lower to
    // Structured — regression guard for the GAP-SM-EXEC + GAP-SM-EFFECT fix-wave
    // (2026-06-21): compile_state walks the tagged exit/do children like entry;
    // the grammar's effect_action accepts `do action {…}` and the effect lowers
    // through parse_action. SysML §7.18.1 / §7.18.3.
    assert!(is_structured(&state_b.entry_action), "entry body must be Structured");
    assert!(is_structured(&state_a.exit_action), "REGRESSION: exit body must be Structured");
    assert!(is_structured(&state_a.do_action), "REGRESSION: do body must be Structured");
    assert!(
        is_structured(&effect),
        "REGRESSION (GAP-SM-EFFECT): transition `do action` effect must lower to Structured"
    );

    // Assignment ORDER must follow source order (children_of iterates an
    // unordered set, so compile_action_from_children sorts statement children by
    // span). The exit body is `aExit = seq; seq = seq + 1` — the first assignment
    // must be to `aExit`, else a misordered `seq = seq + 1` would clobber the
    // read. Regression guard for the assignment-ordering fix.
    if let Some(TransitionActionIR::Structured { assignments, .. }) = &state_a.exit_action {
        assert_eq!(
            assignments.first().map(|a| a.variable.as_str()),
            Some("aExit"),
            "exit action assignments must be in source order (aExit first), got {assignments:?}"
        );
    } else {
        panic!("exit action must be Structured");
    }
}
