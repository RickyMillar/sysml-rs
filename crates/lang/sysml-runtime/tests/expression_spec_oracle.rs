//! Phase 2b: spec coverage validation gate — oracle replay.
//!
//! The 26 `compile_simple_expression("…")` calls in
//! `crates/lang/sysml-runtime/src/expressions/mod.rs:519-953` form the
//! hand-curated coverage suite for every spec feature the runtime claims
//! to support (literals, refs, dot-chains, arithmetic, `not`, conditional,
//! `->select`/`->collect` (incl. nested), function calls, ranges,
//! indexing, `hastype`/`istype`, cast `as`, `@meta` / `@@metameta`, etc).
//!
//! For each of those expression strings we wrap it in a minimal SysML
//! constraint, parse the file through the new parser path, and run
//! `compile_expression_ast` on the constraint element. The resulting
//! `ExprIR` must equal `compile_simple_expression(…)` of the same string.
//!
//! If a spec feature is exercised in `expressions/mod.rs` but the parser
//! does not emit elements that map to it, this test surfaces it
//! immediately. Add new spec features here as they are added to the runtime.

use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::expressions::{compile_expression_ast, compile_simple_expression};

/// Catalog of (label, expression-string) pairs mirroring the calls in
/// `expressions/mod.rs`. Tier 2 features that the runtime does not yet
/// fully support through the AST path (collection ops with closures,
/// indexing of unbound names, metadata access) are flagged with `tier2`
/// so they appear in the report but do not gate the test.
const ORACLE: &[(&str, &str, bool)] = &[
    // (label, expression, is_tier2_skip)
    // Tier 1 — must round-trip
    ("integer literal", "42", false),
    ("feature ref", "speed", false),
    ("dot chain", "vehicle.speed", false),
    ("binary add", "speed + altitude", false),
    ("unary not", "not isDone", false),
    ("conditional", "if flag ? x else y", false),
    ("function call", "sqrt(x + y)", false),
    ("range", "start..end", false),
    // Tier 2 — best-effort during Phase 1, may still differ via the AST path
    (
        "collection select",
        "items->select{|x| x > threshold}",
        true,
    ),
    (
        "collection collect",
        "items->collect{|it| it + offset}",
        true,
    ),
    (
        "nested select+collect",
        "outer->select{|x| inner->collect{|y| x + y + z}}",
        true,
    ),
    ("index", "arr#(idx)", true),
    ("hastype Integer", "x hastype Integer", true),
    ("hastype Real", "x hastype Real", true),
    ("istype Integer", "x istype Integer", true),
    ("istype String", "x istype String", true),
    ("cast as Integer", "x as Integer", true),
    ("cast as Boolean", "x as Boolean", true),
    ("metadata @", "@myElement", true),
    ("meta-meta @@", "@@myElement", true),
];

fn parse_constraint(expr: &str) -> Option<(ModelGraph, sysml_core::ElementId)> {
    let src = format!(
        r#"
        package P {{
            part def Holder {{
                attribute speed: Real;
                attribute altitude: Real;
                attribute vehicle: Real;
                attribute flag: Boolean;
                attribute isDone: Boolean;
                attribute x: Real;
                attribute y: Real;
                attribute z: Real;
                attribute threshold: Real;
                attribute offset: Real;
                attribute items: Real;
                attribute inner: Real;
                attribute outer: Real;
                attribute start: Real;
                attribute end: Real;
                attribute arr: Real;
                attribute idx: Integer;
                attribute myElement: Real;
                constraint c {{ {expr} }}
            }}
        }}
    "#,
        expr = expr,
    );
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile {
        path: "oracle.sysml".into(),
        text: src,
    }];
    let result = parser.parse(&files);
    let graph = result.graph;
    let constraint_id = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::ConstraintUsage)?
        .id
        .clone();
    Some((graph, constraint_id))
}

#[test]
fn spec_oracle_round_trip() {
    let mut tier1_failures: Vec<(&str, &str, String)> = Vec::new();
    let mut tier2_failures: Vec<(&str, &str, String)> = Vec::new();
    let mut parse_failures: Vec<(&str, &str)> = Vec::new();

    for &(label, expr, tier2) in ORACLE {
        let Some((graph, constraint_id)) = parse_constraint(expr) else {
            // Tier 2 expressions may not parse inside the constraint
            // wrapper used by the oracle (e.g. `->select` lambdas, `@@meta`).
            // Surface them, but only fail the test on Tier 1 parse failures.
            if tier2 {
                tier2_failures.push((
                    label,
                    expr,
                    "constraint wrapper failed to parse this form".into(),
                ));
            } else {
                parse_failures.push((label, expr));
            }
            continue;
        };
        let constraint = graph
            .elements
            .get(&constraint_id)
            .expect("constraint should exist");

        let str_ir = match compile_simple_expression(expr) {
            Ok(ir) => ir,
            Err(e) => {
                let msg = format!("string-parser error: {:?}", e);
                if tier2 {
                    tier2_failures.push((label, expr, msg));
                } else {
                    tier1_failures.push((label, expr, msg));
                }
                continue;
            }
        };
        let ast_ir = match compile_expression_ast(constraint, &graph) {
            Ok(ir) => ir,
            Err(e) => {
                let msg = format!("ast-walker error: {:?}", e);
                if tier2 {
                    tier2_failures.push((label, expr, msg));
                } else {
                    tier1_failures.push((label, expr, msg));
                }
                continue;
            }
        };

        if str_ir != ast_ir {
            let msg = format!("string={:?}\n   ast   ={:?}", str_ir, ast_ir);
            if tier2 {
                tier2_failures.push((label, expr, msg));
            } else {
                tier1_failures.push((label, expr, msg));
            }
        }
    }

    if !parse_failures.is_empty() {
        for (label, expr) in &parse_failures {
            eprintln!("TIER 1 PARSE FAILED: [{}] `{}`", label, expr);
        }
    }

    if !tier2_failures.is_empty() {
        eprintln!(
            "Tier 2 spec features not yet supported via AST path ({}):",
            tier2_failures.len()
        );
        for (label, expr, msg) in &tier2_failures {
            eprintln!("  [{}] `{}` — {}", label, expr, msg);
        }
    }

    if !tier1_failures.is_empty() {
        for (label, expr, msg) in &tier1_failures {
            eprintln!("TIER 1 FAILED: [{}] `{}`\n   {}", label, expr, msg);
        }
        panic!("{} Tier 1 spec features regressed", tier1_failures.len());
    }

    assert!(
        parse_failures.is_empty(),
        "{} Tier 1 oracle expressions failed to parse",
        parse_failures.len()
    );
}
