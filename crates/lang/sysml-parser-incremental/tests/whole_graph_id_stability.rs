//! S1.T11b — whole-graph reparse identity gate for the tree-sitter ast_builder.
//!
//! Parses the same source twice through `build_model_graph` (the canonical-key
//! path now wired into named-element minting per ADR-009) and asserts that the
//! IDs of every element minted by the walker — packages, definitions, usages,
//! attributes, ports, actions, etc. — are identical across the two parses.
//!
//! This is the next step after `expression_id_stability.rs` (S1.T6) which
//! covers the expression-walker subtree only. Together they validate that
//! the tree-sitter parser's mint surface produces deterministic IDs across
//! reparses.

#![cfg(feature = "semantic")]

use std::collections::BTreeSet;

use sysml_core::ElementId;
use sysml_parser_incremental::build_model_graph;

/// Parse `source` once via `build_model_graph` and return the set of every
/// element ID under the resulting `ModelGraph`. T11b/T11c migrated both
/// the named-element mint surface and the relationship-element mint
/// surface, so the assertion now covers the entire graph (no kind
/// filter).
fn parse_collect_ids(source: &str, file_path: &str) -> BTreeSet<ElementId> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_sysml::language())
        .expect("set language");
    let tree = parser.parse(source, None).expect("ts parse");
    let result = build_model_graph(&tree, source, file_path);

    result
        .graph
        .elements
        .values()
        .map(|e| e.id.clone())
        .collect::<BTreeSet<_>>()
}

fn assert_whole_graph_stable(source: &str, file_path: &str) {
    let a = parse_collect_ids(source, file_path);
    let b = parse_collect_ids(source, file_path);
    assert!(
        !a.is_empty(),
        "expected at least one element for fixture (got 0)\nsource:\n{source}",
    );
    assert_eq!(
        a, b,
        "reparse identity drifted for fixture\n  source:\n{source}\n  parse A: {a:?}\n  parse B: {b:?}",
    );
}

// ---------------------------------------------------------------------------
// Fixtures: each one exercises a different slice of the named-element mint
// surface migrated in S1.T11b. All parses go through the same code path —
// the `ast_builder` walker — so a regression in any mint site shows up as a
// single test failure.
// ---------------------------------------------------------------------------

#[test]
fn id_stable_simple_package() {
    assert_whole_graph_stable("package P {}", "fixture_simple_package.sysml");
}

#[test]
fn id_stable_part_def_and_usage() {
    let src = r#"
        package P {
            part def Vehicle;
            part car : Vehicle;
        }
    "#;
    assert_whole_graph_stable(src, "fixture_part_def_and_usage.sysml");
}

#[test]
fn id_stable_attributes_and_ports() {
    let src = r#"
        package P {
            attribute def Length;
            port def PowerIn;
            part def Battery {
                attribute capacity : Length;
                port supply : PowerIn;
            }
        }
    "#;
    assert_whole_graph_stable(src, "fixture_attributes_and_ports.sysml");
}

#[test]
fn id_stable_actions_and_states() {
    let src = r#"
        package P {
            action def Start;
            state def Running;
            part def Engine {
                action ignite : Start;
                state mode : Running;
            }
        }
    "#;
    assert_whole_graph_stable(src, "fixture_actions_and_states.sysml");
}

#[test]
fn id_stable_constraints_and_calc() {
    let src = r#"
        package P {
            attribute def Speed;
            part def Car {
                attribute speed : Speed;
                constraint maxSpeed { speed <= 200 }
            }
        }
    "#;
    assert_whole_graph_stable(src, "fixture_constraints_and_calc.sysml");
}

#[test]
fn id_stable_imports_and_aliases() {
    let src = r#"
        package P {
            import Foo::Bar;
            import Baz::*;
        }
    "#;
    assert_whole_graph_stable(src, "fixture_imports_and_aliases.sysml");
}
