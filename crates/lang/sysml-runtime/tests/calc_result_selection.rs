//! A calc's result is its RETURN parameter — not whichever owned child the
//! hasher happens to visit first.
//!
//! `extract_calc_result_expr` used to do `children_of(calc).find_map(has an
//! expression)`. `children_of` yields an UNORDERED hash set, so for a calc
//! owning more than one expression-bearing child the "result" was whatever came
//! out first. Every SSR fixture in the corpus writes a calc with exactly one
//! such child, so the arbitrary pick landed correctly by luck and nothing ever
//! went red.
//!
//! `examples/damped-oscillator` is the exception, and the only reason this was
//! ever observed: its `getNextState` binds an input (`in timeStep = 0.001`) and
//! declares a local intermediate (`attribute v_next = …`) alongside the return.
//! The extractor picked the bound `timeStep`, so the model's "next state" was
//! the constant 0.001 — a zeta sweep over it returned five identical numbers
//! with every child reporting success.
//!
//! This suite pins the real fixture deliberately. A synthetic model would not
//! do: element ids are content-derived and stable, so hash order is fixed per
//! model, and only THIS model is known to order the wrong child first. Pinning
//! the case that actually broke is what makes the gate bite.

use std::path::PathBuf;

use sysml_core::{elaborate, ElementKind};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

fn load(subdir: &str, filename: &str) -> sysml_core::ModelGraph {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples")
        .join(subdir)
        .join(filename);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut graph = TreeSitterParser::new()
        .parse(&[SysmlFile::new(filename.to_owned(), source)])
        .graph;
    elaborate::elaborate(&mut graph);
    graph
}

/// The `getNextState` calc of `damped-oscillator`, plus every owned child that
/// carries an expression. The competing candidates are the point.
fn oscillator_next_state_candidates() -> (String, Vec<(String, String)>) {
    let graph = load("damped-oscillator", "DampedOscillator.sysml");
    let dynamics = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("dynamics"))
        .expect("the fixture declares `action dynamics :> StateSpaceDynamics`");

    for calc in graph.children_of(&dynamics.id) {
        if calc.kind != ElementKind::CalculationUsage {
            continue;
        }
        let is_next_state = graph.children_of(&calc.id).any(|c| {
            c.kind == ElementKind::FeatureTyping
                && c.get_prop("unresolved_type").and_then(|v| v.as_str()) == Some("GetNextState")
        });
        if !is_next_state {
            continue;
        }
        let mut candidates: Vec<(String, String)> = graph
            .children_of(&calc.id)
            .filter_map(|c| {
                let text = sysml_core::expression_pretty::pretty_print_owner(c, &graph)?;
                Some((c.name.clone().unwrap_or_default(), text))
            })
            .collect();
        candidates.sort();
        let result = sysml_runtime::compiler::extract_calc_result_expr_for_test(&graph, calc)
            .expect("the calc has a result");
        return (result, candidates);
    }
    panic!("no GetNextState calc found on the dynamics action");
}

#[test]
fn the_calc_owns_several_competing_expressions() {
    // Guard on the premise. If the fixture is ever simplified to a single
    // expression-bearing child, the test below stops proving anything and this
    // says so, rather than passing vacuously.
    let (_, candidates) = oscillator_next_state_candidates();
    assert!(
        candidates.len() >= 2,
        "this suite only means something while the calc owns competing \
         expressions; got {candidates:?}"
    );
    let names: Vec<&str> = candidates.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"timeStep"),
        "expected the bound input among the candidates, got {names:?}"
    );
    assert!(
        names.contains(&"result"),
        "expected the return among the candidates, got {names:?}"
    );
}

#[test]
fn the_result_is_the_return_not_a_bound_input() {
    let (result, candidates) = oscillator_next_state_candidates();
    let expected = candidates
        .iter()
        .find(|(n, _)| n == "result")
        .map(|(_, e)| e.clone())
        .expect("the return parameter");

    assert_eq!(
        result, expected,
        "the calc's result must be its return parameter. Candidates were: {candidates:?}"
    );
}

#[test]
fn the_result_is_not_the_bound_time_step() {
    // Named separately because THIS is the observed failure: the extractor
    // returned `0.001`, the bound value of the `timeStep` input, and the
    // model's state became that constant.
    let (result, _) = oscillator_next_state_candidates();
    assert_ne!(
        result, "0.001",
        "the bound `in timeStep = 0.001` is an input parameter, never the result"
    );
    assert!(
        result.contains("v_next"),
        "expected the authored next-state expression, got {result:?}"
    );
}
