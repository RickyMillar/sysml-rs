//! Whole-graph elaboration idempotency gate (RSC-6.1 prerequisite).
//!
//! `ModelCompiler::from_arc` trusts `ModelGraph::is_elaborated()` to skip a
//! redundant re-elaborate of an already-elaborated graph. That skip is only
//! byte-identical to the old "always re-elaborate" behaviour if `elaborate` is
//! idempotent over the WHOLE graph — i.e. a second pass adds nothing and leaves
//! the content (fingerprint, element/relationship counts) unchanged.
//!
//! Per-pass double-run checks already exist in `elaborate_integration.rs`, but
//! that whole-graph guarantee was previously only doc-asserted. This test gates
//! it, and the marker contract `elaborate` relies on, in the normal test run
//! (not corpus-gated / not `#[ignore]`).

use sysml_core::elaborate::elaborate;
use sysml_core::ModelGraph;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

/// Parse inline SysML source into a (raw, un-elaborated) `ModelGraph`.
fn parse(src: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("idem.sysml".to_string(), src.to_string())];
    parser.parse(&files).graph
}

/// A multi-domain model so the first elaboration genuinely derives structure
/// across several passes (states/transitions, constraints, actions/successions,
/// requirements), making the second-pass no-op assertion meaningful.
const SRC: &str = r#"
package P {
    part def Vehicle {
        attribute mass;
        state def Motion {
            entry; then idle;
            state idle;
            state moving;
            transition idle then moving;
        }
        constraint massPositive { mass > 0 }
    }
    action def Drive {
        first start;
        then done;
    }
    requirement def MassReq {
        subject v : Vehicle;
        require constraint { v.mass > 0 }
    }
}
"#;

#[test]
fn elaborate_marker_is_unset_on_fresh_parse() {
    let graph = parse(SRC);
    assert!(
        !graph.is_elaborated(),
        "a freshly parsed graph must not claim to be elaborated"
    );
}

#[test]
fn elaborate_sets_marker_and_is_whole_graph_idempotent() {
    let mut graph = parse(SRC);
    assert!(!graph.is_elaborated());

    // First elaboration: derives structure and sets the marker.
    let report1 = elaborate(&mut graph);
    assert!(
        graph.is_elaborated(),
        "elaborate() must set the is_elaborated marker"
    );
    assert!(
        !report1.is_empty(),
        "this fixture should exercise at least one elaboration pass, got an \
         empty report — the test no longer guards idempotency of real work"
    );

    let fp = graph.fingerprint();
    let elems = graph.element_count();
    let rels = graph.relationship_count();

    // Second elaboration: must add nothing and leave content byte-equal. This
    // is exactly the invariant `from_arc`'s skip-when-elaborated relies on.
    let report2 = elaborate(&mut graph);
    assert!(
        report2.is_empty(),
        "second elaboration must be a no-op, got: {report2}"
    );
    assert_eq!(
        graph.fingerprint(),
        fp,
        "fingerprint must be unchanged after a redundant re-elaborate"
    );
    assert_eq!(
        graph.element_count(),
        elems,
        "element count must be unchanged after a redundant re-elaborate"
    );
    assert_eq!(
        graph.relationship_count(),
        rels,
        "relationship count must be unchanged after a redundant re-elaborate"
    );
    assert!(graph.is_elaborated());
}

#[test]
fn content_mutation_clears_the_marker() {
    use sysml_core::{Element, ElementKind};

    let mut graph = parse(SRC);
    elaborate(&mut graph);
    assert!(graph.is_elaborated());

    // Any content mutation must drop the elaboration claim, so a later
    // `from_arc` re-elaborates the new content instead of trusting a stale flag.
    graph.add_element(Element::new_with_kind(ElementKind::PartUsage));
    assert!(
        !graph.is_elaborated(),
        "adding an element must clear the elaboration marker"
    );
}
