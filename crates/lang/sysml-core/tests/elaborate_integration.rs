//! Integration tests for the parse -> elaborate pipeline using real corpus files.
//!
//! These tests verify that elaboration correctly derives implicit structure
//! from parsed `.sysml` corpus files. They are gated behind the
//! `SYSML_CORPUS_PATH` environment variable and marked `#[ignore]` so they
//! only run when explicitly requested.
//!
//! Run with:
//! ```sh
//! SYSML_CORPUS_PATH=/path/to/references/sysmlv2 \
//!   cargo test -p sysml-core --test elaborate_integration -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use sysml_core::elaborate::elaborate;
use sysml_core::{ElementKind, RelationshipKind};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

/// Returns the corpus base path from the environment, or `None` if unset.
fn corpus_path() -> Option<PathBuf> {
    std::env::var("SYSML_CORPUS_PATH").ok().map(PathBuf::from)
}

/// Helper: read a corpus file relative to `SYSML_CORPUS_PATH` and parse it.
/// Returns the `ParseResult` or panics with a descriptive message on I/O failure.
fn parse_corpus_file(base: &PathBuf, relative: &str) -> sysml_parser_trait::ParseResult {
    let path = base.join(relative);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));

    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new(path.to_string_lossy().to_string(), source)];
    parser.parse(&files)
}

// ---------------------------------------------------------------------------
// States.sysml tests
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn elaborate_states_from_corpus() {
    let base = match corpus_path() {
        Some(p) => p,
        None => return,
    };

    let mut result = parse_corpus_file(
        &base,
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems/States.sysml",
    );

    // The file should parse (possibly with warnings, but not fatal errors that
    // produce zero elements).
    assert!(
        result.graph.element_count() > 0,
        "States.sysml should produce at least one element, got 0. Errors: {:?}",
        result
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .collect::<Vec<_>>()
    );

    let elements_before = result.graph.element_count();
    let relationships_before = result.graph.relationship_count();

    // Run elaboration
    let report = elaborate(&mut result.graph);

    // --- Verify elaboration ran and the graph is still valid ---
    // Element count must not decrease (elaboration is additive).
    assert!(
        result.graph.element_count() >= elements_before,
        "Elaboration should not remove elements: before={}, after={}",
        elements_before,
        result.graph.element_count()
    );

    // Relationship count must not decrease.
    assert!(
        result.graph.relationship_count() >= relationships_before,
        "Elaboration should not remove relationships: before={}, after={}",
        relationships_before,
        result.graph.relationship_count()
    );

    // Print the report for diagnostic visibility when running with --nocapture.
    eprintln!("States.sysml elaboration report: {}", report);
    eprintln!(
        "  elements: {} -> {}",
        elements_before,
        result.graph.element_count()
    );
    eprintln!(
        "  relationships: {} -> {}",
        relationships_before,
        result.graph.relationship_count()
    );

    // --- Verify that expected element kinds are present ---
    // The States.sysml file defines StateAction (a state def) and related types.
    let state_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .collect();
    let state_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::StateUsage)
        .collect();

    eprintln!(
        "  StateDefinitions: {}, StateUsages: {}",
        state_defs.len(),
        state_usages.len()
    );

    // The file should contain at least one state definition or state usage.
    // (StateAction is `state def StateAction`, so it appears as a StateDefinition.)
    assert!(
        !state_defs.is_empty() || !state_usages.is_empty(),
        "States.sysml should contain at least one StateDefinition or StateUsage"
    );

    // --- Verify idempotency ---
    let report2 = elaborate(&mut result.graph);
    assert!(
        report2.is_empty(),
        "Second elaboration should be a no-op, but report was: {}",
        report2
    );
}

#[test]
#[ignore]
fn elaborate_states_initial_tagging() {
    let base = match corpus_path() {
        Some(p) => p,
        None => return,
    };

    let mut result = parse_corpus_file(
        &base,
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems/States.sysml",
    );

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: States.sysml produced no elements (parse failure)");
        return;
    }

    let report = elaborate(&mut result.graph);

    // If elaboration tagged any initial states, verify the property is set.
    if report.initial_states_tagged > 0 {
        let initial_states: Vec<_> = result
            .graph
            .elements_by_kind(&ElementKind::StateUsage)
            .filter(|e| {
                e.get_prop("initial")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .collect();

        assert!(
            !initial_states.is_empty(),
            "Report says {} initial states tagged, but none found with initial=true",
            report.initial_states_tagged
        );

        eprintln!(
            "  Initial states tagged: {} (names: {:?})",
            initial_states.len(),
            initial_states
                .iter()
                .map(|e| e.name.as_deref().unwrap_or("<unnamed>"))
                .collect::<Vec<_>>()
        );
    }

    // If elaboration tagged state actions (entry/do/exit), verify at least one
    // state element has the derived property.
    if report.state_actions_tagged > 0 {
        let states_with_actions: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| {
                (e.kind == ElementKind::StateUsage || e.kind == ElementKind::StateDefinition)
                    && (e.get_prop("entry").is_some()
                        || e.get_prop("do_action").is_some()
                        || e.get_prop("exit").is_some())
            })
            .collect();

        assert!(
            !states_with_actions.is_empty(),
            "Report says {} state actions tagged, but no states have entry/do_action/exit props",
            report.state_actions_tagged
        );

        eprintln!(
            "  States with entry/do/exit: {} (names: {:?})",
            states_with_actions.len(),
            states_with_actions
                .iter()
                .map(|e| e.name.as_deref().unwrap_or("<unnamed>"))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore]
fn elaborate_states_transitions() {
    let base = match corpus_path() {
        Some(p) => p,
        None => return,
    };

    let mut result = parse_corpus_file(
        &base,
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems/States.sysml",
    );

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: States.sysml produced no elements (parse failure)");
        return;
    }

    let report = elaborate(&mut result.graph);

    // Check that TransitionUsage elements exist in the parsed graph.
    let transition_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::TransitionUsage)
        .collect();
    eprintln!("  TransitionUsage elements: {}", transition_usages.len());

    // If transitions were created, verify the Transition relationships exist.
    if report.transitions_created > 0 {
        let transitions: Vec<_> = result
            .graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();

        assert!(
            !transitions.is_empty(),
            "Report says {} transitions created, but no Transition relationships found",
            report.transitions_created
        );

        eprintln!("  Transition relationships: {}", transitions.len());

        // Each transition should reference valid source and target elements.
        for trans in &transitions {
            assert!(
                result.graph.get_element(&trans.source).is_some(),
                "Transition source {:?} not found in graph",
                trans.source
            );
            assert!(
                result.graph.get_element(&trans.target).is_some(),
                "Transition target {:?} not found in graph",
                trans.target
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Constraints.sysml tests
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn elaborate_constraints_from_corpus() {
    let base = match corpus_path() {
        Some(p) => p,
        None => return,
    };

    let mut result = parse_corpus_file(
        &base,
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems/Constraints.sysml",
    );

    assert!(
        result.graph.element_count() > 0,
        "Constraints.sysml should produce at least one element, got 0. Errors: {:?}",
        result
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .collect::<Vec<_>>()
    );

    let elements_before = result.graph.element_count();
    let relationships_before = result.graph.relationship_count();

    let report = elaborate(&mut result.graph);

    // Additive check
    assert!(
        result.graph.element_count() >= elements_before,
        "Elaboration should not remove elements"
    );
    assert!(
        result.graph.relationship_count() >= relationships_before,
        "Elaboration should not remove relationships"
    );

    eprintln!("Constraints.sysml elaboration report: {}", report);
    eprintln!(
        "  elements: {} -> {}",
        elements_before,
        result.graph.element_count()
    );

    // The Constraints.sysml file defines ConstraintCheck (a constraint def)
    // and several constraint usages.
    let constraint_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ConstraintDefinition)
        .collect();
    let constraint_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ConstraintUsage)
        .collect();

    eprintln!(
        "  ConstraintDefinitions: {}, ConstraintUsages: {}",
        constraint_defs.len(),
        constraint_usages.len()
    );

    // Should contain at least one constraint definition or usage.
    assert!(
        !constraint_defs.is_empty() || !constraint_usages.is_empty(),
        "Constraints.sysml should contain at least one ConstraintDefinition or ConstraintUsage"
    );

    // --- Verify constraint elaboration ---
    // If elaboration derived constraint properties, verify they are set.
    if report.constraints_derived > 0 {
        let with_constraint_prop: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| {
                (e.kind == ElementKind::ConstraintUsage
                    || e.kind == ElementKind::AssertConstraintUsage
                    || e.kind == ElementKind::ConstraintDefinition)
                    && e.get_prop("constraint").is_some()
            })
            .collect();

        assert!(
            !with_constraint_prop.is_empty(),
            "Report says {} constraints derived, but no elements have `constraint` prop",
            report.constraints_derived
        );

        // Elaboration writes only `constraint` on ConstraintUsage kinds;
        // `expr` belongs to calc kinds. Verify the split is clean.
        for elem in &with_constraint_prop {
            assert!(
                elem.get_prop("expr").is_none(),
                "ConstraintUsage {:?} should not have `expr` set — that prop is for calc kinds",
                elem.name
            );
        }

        eprintln!(
            "  Constraints with derived props: {} (names: {:?})",
            with_constraint_prop.len(),
            with_constraint_prop
                .iter()
                .map(|e| e.name.as_deref().unwrap_or("<unnamed>"))
                .collect::<Vec<_>>()
        );
    }

    // --- Verify idempotency ---
    let report2 = elaborate(&mut result.graph);
    assert!(
        report2.is_empty(),
        "Second elaboration should be a no-op, but report was: {}",
        report2
    );
}

// ---------------------------------------------------------------------------
// Combined pipeline test
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn elaborate_pipeline_both_files() {
    let base = match corpus_path() {
        Some(p) => p,
        None => return,
    };

    let states_path = base.join(
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems/States.sysml",
    );
    let constraints_path = base.join(
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems/Constraints.sysml",
    );

    let states_source = std::fs::read_to_string(&states_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", states_path.display(), e));
    let constraints_source = std::fs::read_to_string(&constraints_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", constraints_path.display(), e));

    // Parse both files together in a single parse call.
    let parser = TreeSitterParser::new();
    let files = vec![
        SysmlFile::new(states_path.to_string_lossy().to_string(), states_source),
        SysmlFile::new(
            constraints_path.to_string_lossy().to_string(),
            constraints_source,
        ),
    ];
    let mut result = parser.parse(&files);

    assert!(
        result.graph.element_count() > 0,
        "Combined parse should produce elements"
    );

    eprintln!(
        "Combined parse: {} elements, {} relationships, {} errors",
        result.graph.element_count(),
        result.graph.relationship_count(),
        result.error_count()
    );

    // Run elaboration on the combined graph.
    let report = elaborate(&mut result.graph);

    eprintln!("Combined elaboration report: {}", report);

    // Both state and constraint kinds should be present in the combined graph.
    let has_states = result
        .graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .next()
        .is_some()
        || result
            .graph
            .elements_by_kind(&ElementKind::StateUsage)
            .next()
            .is_some();

    let has_constraints = result
        .graph
        .elements_by_kind(&ElementKind::ConstraintDefinition)
        .next()
        .is_some()
        || result
            .graph
            .elements_by_kind(&ElementKind::ConstraintUsage)
            .next()
            .is_some();

    assert!(has_states, "Combined graph should contain state elements");
    assert!(
        has_constraints,
        "Combined graph should contain constraint elements"
    );

    // Verify idempotency of the combined elaboration.
    let report2 = elaborate(&mut result.graph);
    assert!(
        report2.is_empty(),
        "Second elaboration on combined graph should be a no-op: {}",
        report2
    );
}
