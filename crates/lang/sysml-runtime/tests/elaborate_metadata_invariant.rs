//! Standing invariant: **elaboration preserves metadata-ownership chains.**
//!
//! `detect_solver_selections_from_metadata` walks `owner` chains from a
//! `MetadataUsage` up to its enclosing definition to discover
//! `@ToolExecution { toolName }`. This test asserts the detector returns the
//! same map whether it runs on the raw parse graph or on an elaborated
//! clone — i.e. `elaborate::elaborate` never disrupts the ownership
//! structure that metadata-driven detectors depend on.
//!
//! Lineage: this began life as the RW-2 migration gate for dropping
//! `ModelCompiler::raw_graph` (ADR-011 §Risks, S3.T7) in
//! `sysml-spec-tests/tests/elaborate_metadata_equivalence.rs`. `raw_graph`
//! is gone and that migration is complete; what remains load-bearing is the
//! standing invariant, so it lives here with the detector it pins
//! (testing-architecture-redesign §3C — lowest reasonable crate). The
//! fixture set is unchanged: the four S0.T2 corner cases.

use std::fs;
use std::path::{Path, PathBuf};

use sysml_core::elaborate::elaborate;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::detect_solver_selections_from_metadata;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// `the-book/` sits ABOVE the workspace root in this monorepo layout.
fn the_book_root() -> PathBuf {
    workspace_root().parent().unwrap().join("the-book")
}

fn coffee_definitions_path() -> PathBuf {
    the_book_root()
        .join("examples")
        .join("coffee-machine")
        .join("definitions.sysml")
}

fn coffee_views_path() -> PathBuf {
    the_book_root()
        .join("examples")
        .join("coffee-machine")
        .join("views.sysml")
}

fn stdlib_base_path() -> PathBuf {
    workspace_root()
        .join("libraries")
        .join("standard")
        .join("library.kernel")
        .join("Base.kerml")
}

/// Self-contained generic model carrying a `@ToolExecution { toolName =
/// "builtin:ode-*" }` metadata usage — the one shape
/// `detect_solver_selections_from_metadata` returns a non-empty selection for
/// (it only counts `toolName`s that start with `"builtin:ode-"`). Inline so the
/// invariant exercises a genuine non-empty owner-chain walk with no dependency
/// on any example fixture on disk.
const SOLVER_SELECTION_DEMO: &str = r#"
package SolverSelectionDemo {
    private import ScalarValues::*;
    part def DecayPlant {
        @ToolExecution { attribute toolName = "builtin:ode-rk4"; }
        attribute x : Real default 100.0;
        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative { return dxdt = 0.0 - x; }
        }
    }
}
"#;

/// The fixture set: two file-based generic sources (the-book coffee-machine),
/// one inline solver-selection model, and the standard-library base. Each is a
/// `(label, source_text)` pair — no product example is referenced.
fn fixture_sources() -> Vec<(&'static str, String)> {
    let read = |label: &str, p: PathBuf| {
        let text = fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("fixture {label} unreadable at {}: {e}", p.display()));
        text
    };
    vec![
        ("coffee_definitions", read("coffee_definitions", coffee_definitions_path())),
        ("coffee_views", read("coffee_views", coffee_views_path())),
        ("solver_selection_demo", SOLVER_SELECTION_DEMO.to_owned()),
        ("stdlib_base", read("stdlib_base", stdlib_base_path())),
    ]
}

/// For each fixture: parse → run the detector on the raw graph, clone +
/// elaborate → run it again, and assert the two maps match.
#[test]
fn solver_selections_invariant_under_elaborate() {
    let parser = TreeSitterParser::new();

    for (label, text) in fixture_sources() {
        let file = SysmlFile::new(format!("{label}.sysml"), text);
        let result = parser.parse(std::slice::from_ref(&file));

        // Diagnostics may be non-empty (warnings) without invalidating the
        // test; only a structurally-empty graph signals a hard parse failure
        // (which would be a parser regression, not a metadata question).
        assert!(
            !result.graph.elements.is_empty(),
            "fixture {} parsed to an empty graph (parse failure unrelated to \
             this invariant)",
            label
        );

        let raw_graph = result.graph;
        let raw_selections = detect_solver_selections_from_metadata(&raw_graph);

        let mut elaborated = raw_graph.clone();
        let _ = elaborate(&mut elaborated);
        let elaborated_selections = detect_solver_selections_from_metadata(&elaborated);

        assert_eq!(
            raw_selections, elaborated_selections,
            "metadata-equivalence FAILED for fixture {}:\n\
             raw         = {:?}\n\
             elaborated  = {:?}\n\
             elaborate::elaborate disrupted the MetadataUsage owner-chain walk; \
             the fix lives in sysml-core/src/elaborate/ (see ADR-011 §Risks RW-2).",
            label, raw_selections, elaborated_selections,
        );
    }
}
