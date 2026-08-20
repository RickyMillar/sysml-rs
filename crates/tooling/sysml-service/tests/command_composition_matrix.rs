//! Composition matrix gate (unification-completion wave).
//!
//! A dumb, cheap source-text tripwire pinning which orchestrator-composition
//! builder and which graph accessor each simulate / verify / eval / analysis
//! command uses. Same technique as `no_compile_pass_bypass.rs`: it reads the
//! service source and asserts each command's body contains the expected
//! builder/accessor substring. No runtime registry, no execution.
//!
//! The table pins TODAY'S truth so any drift is loud. Rows the unification
//! wave's Phase 2 will change carry an explicit `// PHASE 2:` note naming the
//! value they must flip to; when Phase 2 edits the command, it must update the
//! matching row in lockstep or this gate goes red (that is the tripwire
//! working). Never a lying-green: every row reflects the code as committed.
//!
//! Concepts pinned:
//!   - orchestrator composition builder: `build_sm_orchestrator` (single-SM,
//!     mint/bind), `build_workspace_orchestrator` (all subsystems + ODE),
//!     `run_simulation` (single-SM snapshot run).
//!   - graph accessor: `workspace_aware_graph` (cross-file, the convention for
//!     the eval/verify family) vs `require_graph` (parse-only, single-file).

use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Return the source slice of a method body: from the line declaring
/// `fn <name>(` up to (but not including) the next method declaration in the
/// same file. Good enough for a substring tripwire — commands are colocated
/// methods indented in an impl block.
fn fn_body<'a>(source: &'a str, fn_name: &str) -> &'a str {
    let needle = format!("fn {fn_name}(");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("fn `{fn_name}` not found in source"));
    let after = &source[start + needle.len()..];
    // Next method declaration bounds this body.
    let end_rel = after
        .find("\n    pub fn ")
        .or_else(|| after.find("\n    fn "))
        .unwrap_or(after.len());
    &after[..end_rel]
}

/// One matrix row: `command`'s body (in `file`, function `fn_name`) must
/// currently contain `expect`. `phase2` documents the wave's intended change.
struct Row {
    command: &'static str,
    file: &'static str,
    fn_name: &'static str,
    expect: &'static str,
    phase2: &'static str,
}

const MATRIX: &[Row] = &[
    // -- orchestrator composition builders --
    Row {
        command: "sysml.simulate.start",
        file: "src/lib.rs",
        fn_name: "simulate_start",
        expect: "build_sm_orchestrator",
        phase2: "stable — single-SM mint/bind (ledger L44)",
    },
    Row {
        command: "sysml.orchestrate.workspace.start",
        file: "src/lib.rs",
        fn_name: "orchestrate_workspace_start",
        expect: "build_workspace_orchestrator",
        phase2: "stable",
    },
    Row {
        command: "spawn_batch_child (batch children)",
        file: "src/lib.rs",
        fn_name: "spawn_batch_child",
        expect: "build_sm_orchestrator",
        phase2: "stable — F1 landed Phase 2: Some(sm_name) arm mints via build_sm_orchestrator (None arm still build_workspace_orchestrator)",
    },
    Row {
        command: "sysml.scenario.run (composition)",
        file: "src/scenario.rs",
        fn_name: "run_scenario",
        expect: "build_workspace_orchestrator",
        phase2: "stable — landed this wave (F1)",
    },
    // -- workspace-aware graph accessor (the eval/verify convention) --
    Row {
        command: "sysml.evaluate.analysis_cases",
        file: "src/lib.rs",
        fn_name: "evaluate_analysis_cases",
        expect: "workspace_aware_graph",
        phase2: "stable",
    },
    Row {
        command: "sysml.evaluate.verification_cases",
        file: "src/lib.rs",
        fn_name: "evaluate_verification_cases",
        expect: "workspace_aware_graph",
        phase2: "stable",
    },
    Row {
        command: "sysml.montecarlo",
        file: "src/lib.rs",
        fn_name: "montecarlo",
        expect: "workspace_aware_graph",
        phase2: "stable",
    },
    Row {
        command: "sysml.verify",
        file: "src/lib.rs",
        fn_name: "verify",
        expect: "workspace_aware_graph",
        phase2: "stable",
    },
    Row {
        command: "sysml.scenario.run (graph accessor)",
        file: "src/lib.rs",
        fn_name: "scenario_run",
        expect: "workspace_aware_graph",
        phase2: "stable",
    },
    // -- workspace_aware_graph (F3 landed Phase 2; were require_graph pre-wave) --
    Row {
        command: "sysml.analysis.run",
        file: "src/lib.rs",
        fn_name: "analysis_run",
        expect: "workspace_aware_graph",
        phase2: "stable — F3 landed Phase 2 (was require_graph)",
    },
    Row {
        command: "sysml.trade_study",
        file: "src/lib.rs",
        fn_name: "trade_study",
        expect: "workspace_aware_graph",
        phase2: "stable — F3 landed Phase 2 (was require_graph)",
    },
    Row {
        command: "sysml.whatif",
        file: "src/lib.rs",
        fn_name: "whatif",
        expect: "workspace_aware_graph",
        phase2: "stable — F3 landed Phase 2 (was require_graph)",
    },
    Row {
        command: "sysml.whatif.sweep",
        file: "src/lib.rs",
        fn_name: "whatif_sweep",
        expect: "workspace_aware_graph",
        phase2: "stable — F3 landed Phase 2 (was require_graph)",
    },
    // -- documented exception: single-SM by current contract (F2 pending) --
    Row {
        command: "sysml.verify_with_simulation",
        file: "src/lib.rs",
        fn_name: "verify_with_simulation",
        expect: "run_simulation",
        phase2: "EXCEPTION: single-SM run_simulation by current contract; F2 decision pending. \
                 Pinned so drift is loud.",
    },
];

#[test]
fn command_composition_matrix_matches_source() {
    let lib = read("src/lib.rs");
    let scenario = read("src/scenario.rs");

    let mut violations = Vec::new();
    for row in MATRIX {
        let source = match row.file {
            "src/lib.rs" => &lib,
            "src/scenario.rs" => &scenario,
            other => panic!("unknown file in matrix: {other}"),
        };
        let body = fn_body(source, row.fn_name);
        if !body.contains(row.expect) {
            violations.push(format!(
                "{} ({}::{}): expected body to contain `{}` [{}]",
                row.command, row.file, row.fn_name, row.expect, row.phase2
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "command composition matrix drifted from source ({} row(s)). Either the \
         command changed and this table must be updated in lockstep, or a \
         regression slipped in:\n\n{}",
        violations.len(),
        violations.join("\n"),
    );
}
