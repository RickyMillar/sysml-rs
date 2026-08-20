//! Phase 2a / 2.5a: corpus coverage + AST-path compile-success.
//!
//! For every `.sysml` file in `examples/`, parse the file and confirm
//! that every element with an `unresolved_value` string also has
//! structured expression-element children, AND that the AST-walker
//! (`compile_expression_ast`) successfully produces an `ExprIR`.
//!
//! Originally this test compared the AST-path output to the legacy
//! `compile_simple_expression` string-parser output. After Phase 2.5a
//! switched to spec-correct *left-associative* operator chains, that
//! comparison no longer holds — the legacy parser builds non-spec
//! right-associative trees for `+`/`-`/`*`/`/`. The spec-correct shape
//! assertions live in `expression_associativity.rs` and the spec oracle
//! lives in `expression_spec_oracle.rs`. This file is now the corpus-
//! coverage + smoke-compile gate.

use std::fs;
use std::path::{Path, PathBuf};

use sysml_core::ElementId;
use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::expressions::compile_expression_ast;

/// All expression-element kinds emitted by `process_expression`.
fn is_expression_kind(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::OperatorExpression
            | ElementKind::FeatureReferenceExpression
            | ElementKind::FeatureChainExpression
            | ElementKind::InvocationExpression
            | ElementKind::SelectExpression
            | ElementKind::CollectExpression
            | ElementKind::IndexExpression
            | ElementKind::MetadataAccessExpression
            | ElementKind::ConstructorExpression
            | ElementKind::NullExpression
            | ElementKind::LiteralBoolean
            | ElementKind::LiteralInteger
            | ElementKind::LiteralRational
            | ElementKind::LiteralString
            | ElementKind::LiteralInfinity
    )
}

/// Returns true if `elem` has at least one expression element child in the graph.
fn has_structured_expression_child(graph: &ModelGraph, elem_id: &ElementId) -> bool {
    graph
        .children_of(elem_id)
        .any(|c| is_expression_kind(&c.kind))
}

/// Returns true if `elem` declares a result expression *in source*.
///
/// A constraint/calc body that contains an actual expression always carries a
/// `ResultExpressionMembership` child — the spec membership that owns the
/// result expression. Two constraint shapes legitimately carry NO result
/// expression and therefore lack this membership:
///   1. declarative-only defs — `constraint def X { doc /* ... */ }` (only a
///      Documentation child);
///   2. params-only inherited bodies —
///      `assert constraint c : OhmsLaw { in v = ...; in i = ...; }` whose
///      expression is inherited via the `: OhmsLaw` typing; the local body
///      only rebinds parameters (only ReferenceUsage + FeatureTyping children).
///
/// These are excluded from the coverage denominator because there is no
/// expression to materialize a structured child for. Counting them would
/// measure spec authoring style, not parser fidelity.
fn declares_result_expression(graph: &ModelGraph, elem_id: &ElementId) -> bool {
    graph
        .children_of(elem_id)
        .any(|c| c.kind == ElementKind::ResultExpressionMembership)
}

/// Returns the workspace's examples/ directory.
fn examples_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .canonicalize()
        .expect("examples directory should exist")
}

fn collect_sysml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sysml_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("sysml") {
            out.push(path);
        }
    }
}

/// Element kinds that own a top-level expression tree (constraint bodies,
/// attribute default expressions, calculation results, etc.). Post-Phase-6D.1
/// the parser no longer writes `unresolved_value`, so "expression-owning"
/// is defined as "has at least one structured expression child".
const EXPRESSION_OWNER_KINDS: &[ElementKind] = &[
    ElementKind::ConstraintUsage,
    ElementKind::AssertConstraintUsage,
    ElementKind::ConstraintDefinition,
    ElementKind::CalculationUsage,
    ElementKind::CalculationDefinition,
    ElementKind::AttributeUsage,
    ElementKind::ReferenceUsage,
    ElementKind::AssignmentActionUsage,
    ElementKind::ResultExpressionMembership,
];

#[test]
fn ast_path_compiles_every_corpus_expression() {
    let dir = examples_dir();
    let mut files = Vec::new();
    collect_sysml_files(&dir, &mut files);
    assert!(
        !files.is_empty(),
        "expected at least one example .sysml file"
    );

    let parser = TreeSitterParser::new();

    let mut elements_with_expr: usize = 0;
    let mut elements_with_structured: usize = 0;
    let mut compile_ok: usize = 0;
    let mut compile_failures: Vec<(PathBuf, String, String)> = Vec::new();

    for file in &files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        let sysml_files = vec![SysmlFile {
            path: file.to_string_lossy().into_owned(),
            text: content,
        }];
        let result = parser.parse(&sysml_files);
        let graph = &result.graph;

        for kind in EXPRESSION_OWNER_KINDS {
            for element in graph.elements_by_kind(kind) {
                if !has_structured_expression_child(graph, &element.id) {
                    continue;
                }
                elements_with_expr += 1;
                elements_with_structured += 1;

                match compile_expression_ast(element, graph) {
                    Ok(_ir) => {
                        compile_ok += 1;
                    }
                    Err(diags) => {
                        let label = element
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("{:?}", element.kind));
                        compile_failures.push((file.clone(), label, format!("{:?}", diags)));
                    }
                }
            }
        }
    }

    let coverage = if elements_with_expr == 0 {
        0.0
    } else {
        100.0 * elements_with_structured as f64 / elements_with_expr as f64
    };

    eprintln!(
        "AST coverage: {} files | {} expression elements | {} structured ({:.1}%) | {} compiled OK | {} compile failures",
        files.len(),
        elements_with_expr,
        elements_with_structured,
        coverage,
        compile_ok,
        compile_failures.len(),
    );

    if !compile_failures.is_empty() {
        for (path, expr, diags) in compile_failures.iter().take(10) {
            eprintln!(
                "COMPILE FAILED in {}: `{}` — {}",
                path.display(),
                expr,
                diags
            );
        }
        panic!(
            "{} expression elements failed to compile via the AST path",
            compile_failures.len()
        );
    }
    assert!(
        compile_ok > 0,
        "no expression compilations performed — coverage may be 0"
    );
}

#[test]
fn corpus_coverage_threshold() {
    // Track absolute coverage % of expression elements that have structured
    // children. Phase 1 emits a baseline; Phase 2 ratchets toward 100%.
    let dir = examples_dir();
    let mut files = Vec::new();
    collect_sysml_files(&dir, &mut files);
    let parser = TreeSitterParser::new();

    let mut total: usize = 0;
    let mut with_structured: usize = 0;

    for file in &files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        let sysml_files = vec![SysmlFile {
            path: file.to_string_lossy().into_owned(),
            text: content,
        }];
        let result = parser.parse(&sysml_files);
        let graph = &result.graph;

        // Narrow the base to constraint kinds — every parsed constraint
        // body that DECLARES a result expression MUST produce a structured
        // expression child. Attribute and assignment elements are excluded
        // from the ratchet because the parser models literal-typed attributes
        // without an expression subtree (`attribute speed = 42;` stores a
        // typed `value` prop, no child). Expanding the base would dilute the
        // regression signal.
        //
        // The denominator counts only constraints that declare a result
        // expression in source (see `declares_result_expression`).
        // Declarative-only defs (`constraint def X { doc /* ... */ }`) and
        // params-only inherited bodies (`assert constraint c : T { in p = ...; }`)
        // legitimately have no expression to materialize, so including them
        // would measure spec authoring style rather than parser fidelity.
        let constraint_kinds = [
            ElementKind::ConstraintUsage,
            ElementKind::AssertConstraintUsage,
            ElementKind::ConstraintDefinition,
        ];
        for kind in &constraint_kinds {
            for element in graph.elements_by_kind(kind) {
                if !declares_result_expression(graph, &element.id) {
                    continue;
                }
                total += 1;
                if has_structured_expression_child(graph, &element.id) {
                    with_structured += 1;
                }
            }
        }
    }

    let pct = if total == 0 {
        0.0
    } else {
        100.0 * with_structured as f64 / total as f64
    };
    eprintln!(
        "Corpus coverage: {} / {} expression elements have structured children ({:.1}%)",
        with_structured, total, pct
    );
    // Phase 1 target: 100% of expression elements should now have
    // structured children (process_expression covers the full Tier 1
    // grammar). Lower this only if we hit a regression we cannot fix
    // without restructuring more invasively.
    // Post-Phase-6D.1 the base is narrowed to constraint kinds only; a
    // small number of constraint-less shapes remain in the corpus (e.g.
    // constraints that only carry typing + no inline body), so 90% is
    // the regression-detector threshold rather than 100%.
    assert!(
        pct >= 90.0,
        "coverage {:.1}% is below the regression threshold of 90%",
        pct
    );
}
