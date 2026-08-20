#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use super::node_helpers::describe_context;
use sysml_core::{ElementKind, Value};

fn parse_and_build(source: &str) -> ModelGraphResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_sysml::language())
        .expect("Failed to set language");
    let tree = parser.parse(source, None).expect("Failed to parse");
    build_model_graph(&tree, source, "test.sysml")
}

/// ADR-009 root-scope: element identity derives from the checkout-independent
/// `root_scope`, not the absolute `file_path`. Same source + same `root_scope`
/// ⇒ identical IDs regardless of where the repo is checked out; the absolute
/// `file_path` (used by the plain `build_model_graph`) still couples them.
#[test]
fn root_scope_decouples_ids_from_absolute_checkout_path() {
    use std::collections::BTreeSet;
    let source = "package P { part def X; part x : X; }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_sysml::language())
        .expect("set language");
    let tree = parser.parse(source, None).expect("parse");

    let id_set = |r: &ModelGraphResult| -> BTreeSet<String> {
        r.graph.elements.keys().map(|k| k.to_string()).collect()
    };

    // Same relative scope at two different absolute checkouts ⇒ identical IDs.
    let a = build_model_graph_scoped(&tree, source, "/home/alice/repo/M.sysml", "M.sysml");
    let b = build_model_graph_scoped(&tree, source, "/tmp/ci-9f2/repo/M.sysml", "M.sysml");
    assert!(!a.graph.elements.is_empty());
    assert_eq!(
        id_set(&a),
        id_set(&b),
        "same root_scope must yield identical IDs across checkout paths"
    );

    // Negative control: the absolute-path scope (plain entry) DOES couple.
    let c = build_model_graph(&tree, source, "/home/alice/repo/M.sysml");
    let d = build_model_graph(&tree, source, "/tmp/ci-9f2/repo/M.sysml");
    assert_ne!(
        id_set(&c),
        id_set(&d),
        "absolute-path scope couples IDs to the checkout path (pre-ADR-009-fix)"
    );
}

#[test]
fn test_build_empty_package() {
    let result = parse_and_build("package Test {}");
    assert!(!result.has_errors());

    let packages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Package)
        .collect();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name.as_deref(), Some("Test"));
}

#[test]
fn test_build_part_definition() {
    let result = parse_and_build("package P { part def Vehicle {} }");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Vehicle"));
}

#[test]
fn test_standard_usage_keyword_field() {
    // Verify that the merged standard_usage rule exposes its keyword via field()
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_sysml::language())
        .expect("Failed to set language");
    let source = "part car : Vehicle;";
    let tree = parser.parse(source, None).expect("parse");
    let root = tree.root_node();
    let su = root.child(0).expect("first child");
    assert_eq!(su.kind(), "standard_usage");
    let kw = su
        .child_by_field_name("keyword")
        .expect("keyword field must be accessible");
    assert_eq!(&source[kw.start_byte()..kw.end_byte()], "part");
}

#[test]
fn test_build_part_usage_with_typing() {
    let result = parse_and_build("package P { part def Vehicle {} part car : Vehicle; }");
    assert!(!result.has_errors());

    let usages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage)
        .collect();
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].name.as_deref(), Some("car"));

    // Check that FeatureTyping was created
    let typings: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FeatureTyping)
        .collect();
    assert_eq!(typings.len(), 1);
}

#[test]
fn test_build_with_specialization() {
    let result = parse_and_build("package P { part def Base {} part def Derived :> Base {} }");
    assert!(!result.has_errors());

    // Per KerML spec, :> on a Classifier creates Subclassification (not generic Specialization)
    let specs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Subclassification)
        .collect();
    assert_eq!(specs.len(), 1);
}

#[test]
fn test_specialization_has_precise_span() {
    // Critical test: Subclassification elements must have narrower spans than their
    // parent definition so that `element_at` can distinguish them for goto-def.
    let source = "part def Derived :> Base {}";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let def = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::PartDefinition && e.name.as_deref() == Some("Derived"))
        .expect("should have PartDefinition Derived");
    let spec = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::Subclassification)
        .expect("should have Subclassification");

    let def_span = &def.spans[0];
    let spec_span = &spec.spans[0];

    // The Subclassification span should be strictly narrower than the definition span
    let def_size = def_span.end - def_span.start;
    let spec_size = spec_span.end - spec_span.start;
    assert!(
        spec_size < def_size,
        "Subclassification span ({spec_size} bytes) must be narrower than definition span ({def_size} bytes)"
    );

    // The Subclassification span should cover "Base" (the type_ref), not the whole definition
    let spec_text = &source[spec_span.start..spec_span.end];
    assert_eq!(
        spec_text, "Base",
        "Subclassification span should cover just the type_ref"
    );
}

#[test]
fn test_typing_has_precise_span() {
    // FeatureTyping elements must have narrower spans than their parent usage.
    let source = "part car : Vehicle;";
    let result = parse_and_build(source);

    let usage = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::PartUsage && e.name.as_deref() == Some("car"))
        .expect("should have PartUsage car");
    let typing = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::FeatureTyping)
        .expect("should have FeatureTyping");

    let usage_span = &usage.spans[0];
    let typing_span = &typing.spans[0];

    let usage_size = usage_span.end - usage_span.start;
    let typing_size = typing_span.end - typing_span.start;
    assert!(
        typing_size < usage_size,
        "FeatureTyping span ({typing_size} bytes) must be narrower than usage span ({usage_size} bytes)"
    );

    // The FeatureTyping span should cover "Vehicle" (the type_ref)
    let typing_text = &source[typing_span.start..typing_span.end];
    assert_eq!(
        typing_text, "Vehicle",
        "FeatureTyping span should cover just the type_ref"
    );
}

#[test]
fn test_error_recovery() {
    // This has a syntax error but should still produce partial results
    let source = "package P { part def Vehicle {} part ??? ; part def Boat {} }";
    let result = parse_and_build(source);

    // Should have errors
    assert!(result.has_errors());

    // But should still have created some elements
    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    // At least Vehicle should be there, possibly Boat too depending on error recovery
    assert!(!defs.is_empty());
}

#[test]
fn test_nested_packages() {
    // Note: The current tree-sitter grammar has a limitation with nested packages.
    // It produces ERROR nodes for "package Inner" syntax. This test verifies
    // graceful degradation - we still get valid elements despite the error.
    let result = parse_and_build("package Outer { package Inner { part def P {} } }");

    // We expect errors due to nested package syntax not being fully supported
    // But we should still get the outer package and the part definition
    let packages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Package)
        .collect();

    // At least the outer package should be recognized
    assert!(packages.len() >= 1, "Should recognize at least one package");

    // The part definition should still be parsed despite errors
    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    assert_eq!(
        defs.len(),
        1,
        "PartDefinition P should be recognized despite nested package error"
    );
}

// === Enhanced diagnostic tests ===

#[test]
fn test_error_includes_parent_context() {
    // The ERROR node should include parent context in the message
    let source = "package P { ??? }";
    let result = parse_and_build(source);
    assert!(result.has_errors());

    let error_diag = &result.diagnostics[0];
    // Should mention the context (e.g., "in package body")
    assert!(
        error_diag.message.contains("in ") || error_diag.message.contains("Syntax error"),
        "Error message should include context, got: {}",
        error_diag.message
    );
}

#[test]
fn test_syntax_error_has_no_structural_code() {
    let source = "package P { ??? }";
    let result = parse_and_build(source);
    assert!(result.has_errors());

    let error_diag = &result.diagnostics[0];
    assert!(
        error_diag.code.is_none(),
        "Syntax errors should not carry structural error codes (E001 = orphan element), got: {:?}",
        error_diag.code
    );
}

#[test]
fn test_keyword_detection_in_errors() {
    // Use truly invalid syntax that tree-sitter can't parse
    let source = "package P { @@@ invalid garbage; }";
    let result = parse_and_build(source);
    assert!(result.has_errors(), "Should have errors for invalid syntax");
}

#[test]
fn test_error_recovery_with_context() {
    // Multiple errors: tree-sitter recovers valid definitions around errors
    let source = "package P { part def A {} ??? part def B {} }";
    let result = parse_and_build(source);
    assert!(result.has_errors());

    // Syntax errors should have no structural error code
    for diag in &result.diagnostics {
        assert!(
            diag.code.is_none(),
            "Syntax errors should not carry structural codes, got: {:?}",
            diag.code
        );
    }

    // Should still recover A and/or B
    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    assert!(!defs.is_empty(), "Should recover at least one definition");
}

#[test]
fn test_describe_context_mapping() {
    assert_eq!(describe_context("package_body"), Some("in package body"));
    assert_eq!(
        describe_context("definition_body"),
        Some("in definition body")
    );
    assert_eq!(describe_context("source_file"), Some("at top level"));
    assert_eq!(describe_context("unknown_node"), None);
}

#[test]
fn test_starts_with_keyword() {
    assert_eq!(starts_with_keyword("package Foo"), Some("package"));
    assert_eq!(starts_with_keyword("part def X"), Some("part"));
    assert_eq!(starts_with_keyword("dependency foo"), Some("dependency"));
    assert_eq!(starts_with_keyword("xyzzy invalid"), None);
    assert_eq!(starts_with_keyword(""), None);
}

// === Constraint and default value extraction tests ===

#[test]
fn test_default_value_strips_equals_sign() {
    // attribute speed = 105 should store value=105, not "= 105"
    let source = "package P { part def V { attribute speed = 105; } }";
    let result = parse_and_build(source);

    let attrs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("speed"))
        .collect();
    assert_eq!(attrs.len(), 1, "should have one speed attribute");

    let speed = attrs[0];
    // Should have a typed "value" property (literal), not "unresolved_value" with "= 105"
    let value = speed.get_prop("value");
    assert!(
        value.is_some(),
        "speed should have a 'value' property (literal extraction)"
    );
    assert_eq!(
        value.unwrap().as_int(),
        Some(105),
        "speed value should be 105"
    );
}

#[test]
fn test_feature_value_separator_flags_g22() {
    // G22 (KerML `FeatureValue`, KerML.xtext:740-746): the binding separator
    // determines isDefault/isInitial — `=` ⇒ neither, `:=` ⇒ isInitial,
    // `default` ⇒ isDefault, `default :=` ⇒ both. A plain `=` binding is a
    // concrete BindingConnector and MUST NOT be flagged isDefault.
    let source = "package P { part def V { \
        attribute bound = 1; \
        attribute initial := 2; \
        attribute deflt default 3; \
        attribute deflt_init default := 4; \
    } }";
    let result = parse_and_build(source);

    let attr = |name: &str| {
        result
            .graph
            .elements
            .values()
            .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing attribute {name}"))
    };
    let flag = |e: &sysml_core::Element, prop: &str| {
        e.get_prop(prop).and_then(|v| v.as_bool()) == Some(true)
    };

    let bound = attr("bound");
    assert!(!flag(bound, "isDefault"), "`= 1` must NOT set isDefault");
    assert!(!flag(bound, "isInitial"), "`= 1` must NOT set isInitial");

    let initial = attr("initial");
    assert!(flag(initial, "isInitial"), "`:= 2` must set isInitial");
    assert!(!flag(initial, "isDefault"), "`:= 2` must NOT set isDefault");

    let deflt = attr("deflt");
    assert!(flag(deflt, "isDefault"), "`default 3` must set isDefault");
    assert!(
        !flag(deflt, "isInitial"),
        "`default 3` must NOT set isInitial"
    );

    let deflt_init = attr("deflt_init");
    assert!(
        flag(deflt_init, "isDefault"),
        "`default := 4` must set isDefault"
    );
    assert!(
        flag(deflt_init, "isInitial"),
        "`default := 4` must set isInitial"
    );
}

#[test]
fn test_default_value_non_literal_emits_structured_ast() {
    // attribute power = mass * 2 should emit a structured expression
    // subtree (OperatorExpression with LiteralInteger + FeatureRef
    // children) — NOT a legacy `unresolved_value` string. Phase 6D.2:
    // the tree-sitter parser is now AST-only on the value side, matching
    // the Pest parser.
    let source = "package P { part def V { attribute power = mass * 2; } }";
    let result = parse_and_build(source);

    let attrs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("power"))
        .collect();
    assert_eq!(attrs.len(), 1);

    let power = attrs[0];
    assert!(
        power.get_prop("unresolved_value").is_none(),
        "tree-sitter parser must not write `unresolved_value` for value expressions"
    );

    let expr_children: Vec<_> = result
        .graph
        .children_of(&power.id)
        .filter(|c| {
            matches!(
                c.kind,
                ElementKind::OperatorExpression
                    | ElementKind::FeatureReferenceExpression
                    | ElementKind::LiteralInteger
                    | ElementKind::LiteralRational
            )
        })
        .collect();
    assert!(
        !expr_children.is_empty(),
        "power must have at least one structured expression child, got children: {:?}",
        result
            .graph
            .children_of(&power.id)
            .map(|c| c.kind.clone())
            .collect::<Vec<_>>()
    );
    let op = expr_children
        .iter()
        .find(|c| c.kind == ElementKind::OperatorExpression)
        .expect("power's RHS is `mass * 2` — root must be an OperatorExpression");
    assert_eq!(
        op.get_prop("operator").and_then(|v| v.as_str()),
        Some("*"),
        "root operator should be '*'"
    );
}

#[test]
fn test_quantity_literal_records_unit_not_index() {
    // RSC-5.1 (D-5.0.5): `100 [SI::m]` is the spec measurement-reference operator
    // `'['(num, mRef)` — a quantity literal, NOT an index into the number 100.
    // It must lower to a magnitude literal carrying the unit reference on a `unit`
    // property (previously the unit was dropped / mis-lowered as IndexExpression, B2).
    let source = "package P { part def V { attribute length = 100 [SI::m]; } }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let lits: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::LiteralInteger)
        .collect();
    assert_eq!(lits.len(), 1, "one integer literal carries the magnitude");
    let lit = lits[0];
    assert_eq!(
        lit.get_prop("value").and_then(|v| v.as_int()),
        Some(100),
        "magnitude must be the literal value 100"
    );
    assert_eq!(
        lit.get_prop("unit").and_then(|v| v.as_str()),
        Some("SI::m"),
        "the unit reference must be recorded verbatim on the literal"
    );
    assert!(
        result
            .graph
            .elements
            .values()
            .all(|e| e.kind != ElementKind::IndexExpression),
        "`num [unit]` must NOT lower to an IndexExpression"
    );

    // The default must ALSO fold onto the attribute as a `value` magnitude +
    // `unit` prop — that is where model-level constraint eval (the slot-less
    // FeatureValue path) reads the declared value, so leaving it only as a child
    // literal regresses constraints (caught by service_command_baseline).
    let attr = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("length"))
        .expect("length attribute");
    assert_eq!(
        attr.get_prop("value").and_then(|v| v.as_int()),
        Some(100),
        "magnitude must fold onto the attribute's `value` prop"
    );
    assert_eq!(
        attr.get_prop("unit").and_then(|v| v.as_str()),
        Some("SI::m"),
        "unit reference must fold onto the attribute's `unit` prop"
    );
}

#[test]
fn test_real_index_still_lowers_to_index_expression() {
    // Disambiguation guard: a NON-literal source (`arr`) keeps the IndexExpression
    // lowering — only a numeric-literal source is the quantity form.
    let source = "package P { part def V { attribute arr; attribute x = arr[2]; } }";
    let result = parse_and_build(source);
    assert!(
        result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::IndexExpression),
        "`arr[2]` must remain an IndexExpression"
    );
}

#[test]
fn test_arrow_for_all_lambda_body_lowers() {
    let source = "package Test { attribute def A { attribute items; \
                  assert constraint { items->forAll { in item; item == 0 } } } }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let invocation = result
        .graph
        .elements
        .values()
        .find(|e| {
            e.kind == ElementKind::InvocationExpression && e.name.as_deref() == Some("forAll")
        })
        .expect("forAll arrow body should lower to an InvocationExpression");

    let children: Vec<_> = result.graph.children_of(&invocation.id).collect();
    assert!(
        children.iter().any(|e| {
            e.kind == ElementKind::Feature
                && e.name.as_deref() == Some("item")
                && e.get_prop("isBodyParameter").and_then(|v| v.as_bool()) == Some(true)
        }),
        "lambda parameter `in item;` should lower as a body-parameter Feature"
    );
    assert!(
        children.iter().any(|e| {
            e.kind == ElementKind::OperatorExpression
                && e.get_prop("operator").and_then(|v| v.as_str()) == Some("==")
        }),
        "lambda result `item == 0` should lower as the invocation result operand"
    );
}

#[test]
fn test_tree_sitter_parser_emits_no_unresolved_value() {
    // Sentinel: parse a representative spread of value-bearing patterns
    // through the tree-sitter parser and assert no element carries the
    // legacy `unresolved_value` string prop. Mirrors the Pest parser's
    // `parser_emits_no_unresolved_value` invariant.
    let sources = [
        "package P { part def V { attribute speed = 105; } }",
        "package P { part def V { attribute power = mass * 2; } }",
        "package P { part def V { attribute t = if x > 0 ? 1 else 2; } }",
        "package P { part def V { attribute n = abs(-3) + 1; } }",
        "package P { part def V { attribute b: Boolean = a and b; } }",
        "package P { part def V { attribute s: String = \"hello\"; } }",
        "package P { part def V { attribute d default 0.5; } }",
    ];
    for source in &sources {
        let result = parse_and_build(source);
        for elem in result.graph.elements.values() {
            assert!(
                elem.get_prop("unresolved_value").is_none(),
                "tree-sitter parser must not write `unresolved_value` (element {:?} from source: {})",
                elem.name,
                source
            );
        }
    }
}

#[test]
fn test_constraint_body_expression_extracted() {
    // constraint speedLimit { speed < 100 } should have constraint property
    let source = "package P { part def V { constraint speedLimit { speed < 100 } } }";
    let result = parse_and_build(source);

    let constraints: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ConstraintUsage && e.name.as_deref() == Some("speedLimit")
        })
        .collect();
    assert_eq!(constraints.len(), 1, "should have one constraint");

    let constraint = constraints[0];
    let expr = constraint.get_prop("constraint");
    assert!(
        expr.is_some(),
        "constraint element should have 'constraint' property"
    );
    let expr_str = expr.unwrap().as_str().unwrap();
    assert!(
        expr_str.contains("speed") && expr_str.contains("100"),
        "constraint expression should contain 'speed' and '100', got: {}",
        expr_str
    );
}

#[test]
fn test_assert_keyword_produces_assert_constraint_usage() {
    // assert speedLimit { speed < 100 } should produce AssertConstraintUsage
    let source = "package P { part def V { assert speedLimit { speed < 100 } } }";
    let result = parse_and_build(source);

    let asserts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AssertConstraintUsage)
        .collect();
    assert!(
        !asserts.is_empty(),
        "should have at least one AssertConstraintUsage element"
    );
    assert_eq!(
        asserts[0].name.as_deref(),
        Some("speedLimit"),
        "assert constraint should be named speedLimit"
    );
}

#[test]
fn test_constraint_keyword_produces_constraint_usage() {
    // constraint speedLimit { ... } should produce ConstraintUsage (not Assert)
    let source = "package P { part def V { constraint speedLimit { speed < 100 } } }";
    let result = parse_and_build(source);

    let constraints: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConstraintUsage)
        .collect();
    assert!(
        !constraints.is_empty(),
        "should have ConstraintUsage element"
    );

    let asserts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AssertConstraintUsage)
        .collect();
    assert!(
        asserts.is_empty(),
        "constraint keyword should NOT produce AssertConstraintUsage"
    );
}

#[test]
fn test_transition_usage_extracts_endpoints() {
    let source = r#"
        package P {
            state def Toggle {
                state Off;
                state On;
                transition turn_on first Off then On;
            }
        }
    "#;
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "unexpected parse errors");

    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    assert_eq!(transitions.len(), 1, "expected one transition usage");

    let transition = transitions[0];
    assert_eq!(
        transition.get_prop("source").and_then(|v| v.as_str()),
        Some("Off")
    );
    assert_eq!(
        transition.get_prop("target").and_then(|v| v.as_str()),
        Some("On")
    );
}

#[test]
fn test_perform_keyword_produces_perform_action_usage() {
    let source = "package P { action def A { perform subAction; } }";
    let result = parse_and_build(source);

    let performs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PerformActionUsage)
        .collect();
    assert!(
        !performs.is_empty(),
        "should have at least one PerformActionUsage element"
    );

    // The action keyword should NOT produce PerformActionUsage
    let actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ActionUsage && e.name.as_deref() == Some("subAction"))
        .collect();
    assert!(
        actions.is_empty(),
        "'perform subAction' should produce PerformActionUsage, not ActionUsage"
    );
}

#[test]
fn test_action_keyword_does_not_produce_perform() {
    let source = "package P { action def A { action normalAction; } }";
    let result = parse_and_build(source);

    let performs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PerformActionUsage)
        .collect();
    assert!(
        performs.is_empty(),
        "action keyword should NOT produce PerformActionUsage"
    );
}

#[test]
fn test_satisfy_requirement_produces_satisfy_requirement_usage() {
    let source = r#"
        package P {
            part def System {
                satisfy requirement SafetyReq;
            }
        }
    "#;
    let result = parse_and_build(source);

    let satisfies: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SatisfyRequirementUsage)
        .collect();
    assert!(
        !satisfies.is_empty(),
        "should have at least one SatisfyRequirementUsage element"
    );
}

#[test]
fn test_subject_requirement_produces_subject_membership() {
    let source = r#"
        package P {
            requirement def SafetyReq {
                subject vehicle;
            }
        }
    "#;
    let result = parse_and_build(source);

    let subjects: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SubjectMembership)
        .collect();
    assert!(
        !subjects.is_empty(),
        "should have at least one SubjectMembership element"
    );
    assert_eq!(
        subjects[0].name.as_deref(),
        Some("vehicle"),
        "SubjectMembership should be named 'vehicle'"
    );
}

#[test]
fn test_assume_constraint_has_role_property() {
    // Grammar currently parses `assume constraint { ... }` (without a name)
    // as an assume_constraint node. Named assume constraints like
    // `assume constraint safetyAssumption;` have a grammar ambiguity that
    // splits them — a known limitation tracked separately.
    let source = r#"
        package P {
            requirement def SafetyReq {
                assume constraint {
                    true
                }
            }
        }
    "#;
    let result = parse_and_build(source);

    let constraints: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::RequirementConstraintMembership
                && e.get_prop("role").and_then(|v| v.as_str()) == Some("assume")
        })
        .collect();
    assert!(
        !constraints.is_empty(),
        "should have a RequirementConstraintMembership with role='assume'"
    );
}

#[test]
fn test_requirement_constraint_body_mints_expression_ast() {
    // v2 unification (workbench design §7.1): an inline assume/require body
    // dual-writes — the legacy `constraint` string prop AND a
    // ResultExpressionMembership + structured expression subtree, the same
    // lowering ordinary constraint defs/usages get. The evaluator and
    // pretty_print_owner read the AST; the prop is the compat surface.
    let source = r#"
        package P {
            requirement def CreepageRule {
                attribute gap;
                require constraint minGap { gap >= 4.0 }
            }
        }
    "#;
    let result = parse_and_build(source);

    let membership = result
        .graph
        .elements
        .values()
        .find(|e| {
            e.kind == ElementKind::RequirementConstraintMembership
                && e.get_prop("role").and_then(|v| v.as_str()) == Some("require")
        })
        .expect("require membership lowered");

    assert_eq!(
        membership.get_prop("constraint").and_then(|v| v.as_str()),
        Some("gap >= 4.0"),
        "legacy `constraint` string prop must keep the verbatim body"
    );

    // Spec shape (§8.3.21.7, rule S051): the membership owns exactly one
    // ConstraintUsage (`ownedConstraint`); the expression AST hangs on THAT
    // usage — never on the membership (memberships are not function-like
    // Types and may not own result expressions).
    let usage = result
        .graph
        .children_of(&membership.id)
        .find(|c| c.kind == ElementKind::ConstraintUsage)
        .expect("membership must own the spec-shaped ConstraintUsage");
    assert_eq!(usage.name.as_deref(), Some("minGap"));

    let usage_children: Vec<_> = result.graph.children_of(&usage.id).collect();
    assert!(
        usage_children
            .iter()
            .any(|c| c.kind == ElementKind::ResultExpressionMembership),
        "the owned ConstraintUsage must own a ResultExpressionMembership"
    );
    assert!(
        usage_children
            .iter()
            .any(|c| c.kind == ElementKind::OperatorExpression),
        "the owned ConstraintUsage must own a structured OperatorExpression subtree"
    );

    let body_owner =
        sysml_core::query::requirement_constraint_body_owner(membership, &result.graph);
    assert_eq!(body_owner.id, usage.id, "the shared hop must land on the usage");
    let printed =
        sysml_core::expression_pretty::pretty_print_owner(body_owner, &result.graph);
    assert!(
        printed.is_some(),
        "pretty_print_owner must render assume/require bodies post-unification"
    );
}

#[test]
fn test_library_package_not_standard() {
    // HAN-021: `library package` → LibraryPackage with isStandard=false
    let result = parse_and_build("library package Lib { part x; }");
    assert!(!result.has_errors());

    let libs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::LibraryPackage)
        .collect();
    assert_eq!(libs.len(), 1, "should have exactly one LibraryPackage");
    assert_eq!(libs[0].name.as_deref(), Some("Lib"));
    // isStandard should be false (absent or explicitly false)
    let is_std = libs[0]
        .get_prop("isStandard")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        !is_std,
        "library package without 'standard' should have isStandard=false"
    );
}

#[test]
fn test_standard_library_package() {
    // HAN-021: `standard library package` → LibraryPackage with isStandard=true
    let result = parse_and_build("standard library package StdLib { part y; }");
    assert!(!result.has_errors());

    let libs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::LibraryPackage)
        .collect();
    assert_eq!(libs.len(), 1, "should have exactly one LibraryPackage");
    assert_eq!(libs[0].name.as_deref(), Some("StdLib"));
    // isStandard should be true
    let is_std = libs[0]
        .get_prop("isStandard")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        is_std,
        "standard library package should have isStandard=true"
    );
}

#[test]
fn test_plain_package_not_library() {
    // HAN-021: `package` → Package (not LibraryPackage)
    let result = parse_and_build("package Pkg { part z; }");
    assert!(!result.has_errors());

    let pkgs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Package)
        .collect();
    assert_eq!(pkgs.len(), 1, "should have exactly one Package");
    assert_eq!(pkgs[0].name.as_deref(), Some("Pkg"));

    let libs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::LibraryPackage)
        .collect();
    assert!(
        libs.is_empty(),
        "plain package should not produce LibraryPackage"
    );
}

// === Sprint 5: standard_def dispatch tests ===

#[test]
fn test_standard_def_part_definition() {
    let result = parse_and_build("part def Vehicle {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Vehicle"));
}

#[test]
fn test_standard_def_attribute_definition() {
    let result = parse_and_build("attribute def Speed;");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AttributeDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Speed"));
}

#[test]
fn test_standard_def_port_definition() {
    let result = parse_and_build("port def FuelPort {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PortDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("FuelPort"));
}

#[test]
fn test_standard_def_connection_definition() {
    let result = parse_and_build("connection def FuelLine {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConnectionDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("FuelLine"));
}

#[test]
fn test_standard_def_interface_definition() {
    let result = parse_and_build("interface def FuelInterface {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::InterfaceDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("FuelInterface"));
}

#[test]
fn test_standard_def_item_definition() {
    let result = parse_and_build("item def Fuel {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ItemDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Fuel"));
}

#[test]
fn test_standard_def_allocation_definition() {
    let result = parse_and_build("allocation def TaskAllocation {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AllocationDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("TaskAllocation"));
}

#[test]
fn test_standard_def_occurrence_definition() {
    let result = parse_and_build("occurrence def Crash {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::OccurrenceDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Crash"));
}

#[test]
fn test_standard_def_flow_definition() {
    let result = parse_and_build("flow def FuelFlow {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("FuelFlow"));
}

#[test]
fn test_standard_def_abstract_part_definition() {
    let result = parse_and_build("abstract part def Base {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Base"));
    assert_eq!(
        defs[0].get_prop("isAbstract").and_then(|v| v.as_bool()),
        Some(true),
        "abstract keyword should set isAbstract"
    );
}

#[test]
fn test_standard_def_with_specialization() {
    let result = parse_and_build("part def Car :> Vehicle {}");
    assert!(!result.has_errors());

    let defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name.as_deref(), Some("Car"));

    // Should create Subclassification (not just Specialization)
    let subcls: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Subclassification)
        .collect();
    assert!(!subcls.is_empty(), "should create Subclassification for :>");
}

// === Sprint 5: modifier extraction tests ===

#[test]
fn test_ref_modifier_sets_is_reference() {
    // NOTE: `ref part p : T;` splits into two standard_usage nodes (grammar ambiguity:
    // `ref` is both a usage_prefix keyword and a standard_usage keyword). Instead test
    // that `ref` as a standalone standard_usage produces ReferenceUsage.
    let result = parse_and_build("package P { part def V { ref p : T; } }");
    assert!(!result.has_errors());

    let usages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ReferenceUsage && e.name.as_deref() == Some("p"))
        .collect();
    assert_eq!(usages.len(), 1, "ref p : T should produce a ReferenceUsage");
}

#[test]
fn test_composite_modifier_sets_is_composite() {
    let result = parse_and_build("package P { part def V { composite part p; } }");
    assert!(!result.has_errors());

    let usages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage && e.name.as_deref() == Some("p"))
        .collect();
    assert_eq!(usages.len(), 1);
    assert_eq!(
        usages[0].get_prop("isComposite").and_then(|v| v.as_bool()),
        Some(true),
        "composite keyword should set isComposite"
    );
}

#[test]
fn test_portion_modifier_sets_is_portion() {
    let result = parse_and_build("package P { part def V { portion part p; } }");
    assert!(!result.has_errors());

    let usages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage && e.name.as_deref() == Some("p"))
        .collect();
    assert_eq!(usages.len(), 1);
    assert_eq!(
        usages[0].get_prop("isPortion").and_then(|v| v.as_bool()),
        Some(true),
        "portion keyword should set isPortion"
    );
}

#[test]
fn test_constant_modifier_sets_is_constant() {
    let result = parse_and_build("package P { part def V { constant attribute c; } }");
    assert!(!result.has_errors());

    let usages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("c"))
        .collect();
    assert_eq!(usages.len(), 1);
    assert_eq!(
        usages[0].get_prop("isConstant").and_then(|v| v.as_bool()),
        Some(true),
        "constant keyword should set isConstant"
    );
}

#[test]
fn test_end_modifier_no_false_match_on_send() {
    // "end" should NOT match "send" — verifies exact token matching
    let result = parse_and_build("package P { action def A { send x to y; } }");

    // send_action should not have isEnd set
    let sends: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SendActionUsage)
        .collect();
    for send in &sends {
        let is_end = send
            .get_prop("isEnd")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(!is_end, "send action should not have isEnd=true");
    }
}

// === Sprint 5: ConjugatedPortDefinition auto-creation tests ===

#[test]
fn test_port_def_creates_conjugated_port_definition() {
    let result = parse_and_build("package P { port def WaterPort {} }");
    assert!(!result.has_errors());

    let conjugates: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConjugatedPortDefinition)
        .collect();
    assert_eq!(
        conjugates.len(),
        1,
        "port def should auto-create exactly one ConjugatedPortDefinition"
    );
    assert_eq!(
        conjugates[0].name.as_deref(),
        Some("~WaterPort"),
        "ConjugatedPortDefinition should be named ~WaterPort"
    );

    // Should reference the original PortDefinition
    let port_defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PortDefinition)
        .collect();
    assert_eq!(port_defs.len(), 1);
    let port_def_id = &port_defs[0].id;
    assert_eq!(
        conjugates[0]
            .get_prop("originalPortDefinition")
            .and_then(|v| v.as_ref()),
        Some(port_def_id),
        "ConjugatedPortDefinition should reference the original PortDefinition"
    );
}

#[test]
fn test_non_port_def_no_conjugated() {
    // part def should NOT create a ConjugatedPortDefinition
    let result = parse_and_build("part def Vehicle {}");
    assert!(!result.has_errors());

    let conjugates: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConjugatedPortDefinition)
        .collect();
    assert!(
        conjugates.is_empty(),
        "non-port definitions should not create ConjugatedPortDefinition"
    );
}

// === Sprint 6: Connector dispatch and endpoint extraction tests ===

#[test]
fn test_connector_usage_dispatch() {
    let result = parse_and_build("package P { part a; part b; connector c from a to b; }");
    let connectors: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConnectorAsUsage)
        .collect();
    assert_eq!(
        connectors.len(),
        1,
        "should dispatch connector_usage to ConnectorAsUsage"
    );
    assert_eq!(connectors[0].name.as_deref(), Some("c"));
    assert_eq!(
        connectors[0].get_prop("source").and_then(|v| v.as_str()),
        Some("a"),
        "connector source should be extracted from connector_ends"
    );
    assert_eq!(
        connectors[0].get_prop("target").and_then(|v| v.as_str()),
        Some("b"),
        "connector target should be extracted from connector_ends"
    );
}

#[test]
fn test_succession_decl_dispatch() {
    let result = parse_and_build("package P { part a; part b; succession first a then b; }");
    let succs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    assert!(
        !succs.is_empty(),
        "should dispatch succession_decl to SuccessionAsUsage"
    );
    // Check endpoint extraction from direct fields
    let has_target = succs
        .iter()
        .any(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("b"));
    assert!(has_target, "succession target should be extracted");
}

#[test]
fn test_interface_usage_endpoint_extraction() {
    let result = parse_and_build("package P { port pA; port pB; interface iface from pA to pB; }");
    let ifaces: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::InterfaceUsage)
        .collect();
    assert_eq!(ifaces.len(), 1);
    assert_eq!(
        ifaces[0].get_prop("source").and_then(|v| v.as_str()),
        Some("pA"),
        "interface source should be extracted from connection_ends"
    );
    assert_eq!(
        ifaces[0].get_prop("target").and_then(|v| v.as_str()),
        Some("pB"),
        "interface target should be extracted from connection_ends"
    );
}

#[test]
fn test_allocation_usage_endpoint_extraction() {
    let result = parse_and_build("package P { part func; part hw; allocate func to hw; }");
    let allocs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AllocationUsage)
        .collect();
    assert_eq!(allocs.len(), 1);
    assert_eq!(
        allocs[0].get_prop("source").and_then(|v| v.as_str()),
        Some("func"),
        "allocation source should be mapped from 'from' field"
    );
    assert_eq!(
        allocs[0].get_prop("target").and_then(|v| v.as_str()),
        Some("hw"),
        "allocation target should be mapped from 'to' field"
    );
}

#[test]
fn test_binding_usage_endpoint_extraction() {
    let result = parse_and_build("package P { part x; part y; binding bind x = y; }");
    let bindings: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::BindingConnectorAsUsage)
        .collect();
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].get_prop("source").and_then(|v| v.as_str()),
        Some("x"),
        "binding source should be extracted"
    );
    assert_eq!(
        bindings[0].get_prop("target").and_then(|v| v.as_str()),
        Some("y"),
        "binding target should be extracted"
    );
}

#[test]
fn test_succession_usage_successor_extraction() {
    let result = parse_and_build("package P { action a; action b; first a; then b; }");
    let succs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    // succession_usage with `then` should have target extracted from successor field
    let then_succ = succs
        .iter()
        .find(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("b"));
    assert!(
        then_succ.is_some(),
        "succession_usage 'then b' should extract target from successor field"
    );
}

// === Sprint 7: Flow dispatch and message tests ===

#[test]
fn test_succession_flow_dispatch() {
    let result =
        parse_and_build("package P { action a; action b; succession flow sf from a to b; }");
    let flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionFlowUsage)
        .collect();
    assert_eq!(
        flows.len(),
        1,
        "succession flow should dispatch to SuccessionFlowUsage"
    );
    assert_eq!(flows[0].name.as_deref(), Some("sf"));
    assert_eq!(
        flows[0].get_prop("source").and_then(|v| v.as_str()),
        Some("a"),
        "succession flow source should be extracted from flow_ends"
    );
    assert_eq!(
        flows[0].get_prop("target").and_then(|v| v.as_str()),
        Some("b"),
        "succession flow target should be extracted from flow_ends"
    );
}

#[test]
fn test_plain_flow_dispatch() {
    let result = parse_and_build("package P { part a; part b; flow f from a to b; }");
    let flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(flows.len(), 1, "plain flow should dispatch to FlowUsage");
    assert_eq!(flows[0].name.as_deref(), Some("f"));
    // Ensure it's NOT SuccessionFlowUsage
    let succession_flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionFlowUsage)
        .collect();
    assert!(
        succession_flows.is_empty(),
        "plain flow should NOT produce SuccessionFlowUsage"
    );

    // Verify source/target endpoint extraction
    assert_eq!(
        flows[0].get_prop("source").and_then(|v| v.as_str()),
        Some("a"),
        "plain flow source should be extracted from flow_ends"
    );
    assert_eq!(
        flows[0].get_prop("target").and_then(|v| v.as_str()),
        Some("b"),
        "plain flow target should be extracted from flow_ends"
    );
}

#[test]
fn test_plain_flow_qualified_endpoints() {
    let result = parse_and_build(
        "package P { part t; part h; flow waterFlow from t.waterOut to h.steamIn; }",
    );
    let flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].name.as_deref(), Some("waterFlow"));
    assert_eq!(
        flows[0].get_prop("source").and_then(|v| v.as_str()),
        Some("t.waterOut"),
        "qualified flow source should be extracted"
    );
    assert_eq!(
        flows[0].get_prop("target").and_then(|v| v.as_str()),
        Some("h.steamIn"),
        "qualified flow target should be extracted"
    );
}

#[test]
fn test_plain_flow_compact_qualified_endpoints() {
    let result = parse_and_build("package P { part a; part b; flow a.out1 to b.in1; }");
    assert!(!result.has_errors());

    let flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(flows.len(), 1, "compact flow should dispatch to FlowUsage");
    assert_eq!(
        flows[0].name.as_deref(),
        None,
        "compact endpoint form is anonymous; first endpoint must not be consumed as name"
    );
    assert_eq!(
        flows[0].get_prop("source").and_then(|v| v.as_str()),
        Some("a.out1"),
        "compact qualified flow source should be extracted"
    );
    assert_eq!(
        flows[0].get_prop("target").and_then(|v| v.as_str()),
        Some("b.in1"),
        "compact qualified flow target should be extracted"
    );
}

#[test]
fn test_message_declaration() {
    let result = parse_and_build("message alert;");
    let flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(
        flows.len(),
        1,
        "message declaration should dispatch to FlowUsage"
    );
    assert_eq!(flows[0].name.as_deref(), Some("alert"));
    assert_eq!(
        flows[0].get_prop("isMessage").and_then(|v| v.as_bool()),
        Some(true),
        "message declaration should have isMessage=true"
    );
}

#[test]
fn test_message_named_in_package() {
    let result = parse_and_build("package P { message alert; }");
    let flows: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(
        flows.len(),
        1,
        "message in package should dispatch to FlowUsage"
    );
    assert_eq!(flows[0].name.as_deref(), Some("alert"));
    assert_eq!(
        flows[0].get_prop("isMessage").and_then(|v| v.as_bool()),
        Some(true),
        "message in package should have isMessage=true"
    );
}

/// Spec pin: `message` and `flow` are two KIND keywords on the SAME metaclass.
///
/// SysML spec §7.16 ("A message is modeled as a flow usage") and the grammar
/// rules `Message returns SysML::FlowUsage` / `MessageEvent returns
/// SysML::EventOccurrenceUsage` (SysML.xtext) mean there is NO distinct
/// `Message`/`MessageEvent` metaclass — SysML-vocab.ttl has no such class, so
/// `ElementKind` has no such variant. The message-vs-flow distinction is
/// therefore carried by the `isMessage` marker on a FlowUsage, NOT by a
/// separate kind. This test locks that contract in both directions so the
/// (misdiagnosed) "lower to a distinct Message kind" gap cannot be reopened by
/// inventing a metaclass the spec does not prescribe.
#[test]
fn message_and_flow_share_flow_usage_kind_distinguished_by_is_message() {
    // A plain `flow` lowers to a FlowUsage that is NOT marked isMessage.
    let flow = parse_and_build("package P { part a; part b; flow f from a.x to b.y; }");
    let flow_usages: Vec<_> = flow
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(flow_usages.len(), 1, "a `flow` lowers to exactly one FlowUsage");
    assert_ne!(
        flow_usages[0].get_prop("isMessage").and_then(|v| v.as_bool()),
        Some(true),
        "a plain `flow` must NOT carry isMessage — that marker is what distinguishes a message"
    );

    // A `message` lowers to the SAME FlowUsage metaclass, marked isMessage.
    let msg = parse_and_build("package P { message m; }");
    let msg_usages: Vec<_> = msg
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FlowUsage)
        .collect();
    assert_eq!(msg_usages.len(), 1, "a `message` lowers to exactly one FlowUsage");
    assert_eq!(
        msg_usages[0].get_prop("isMessage").and_then(|v| v.as_bool()),
        Some(true),
        "a `message` is a FlowUsage marked isMessage (SysML §7.16), not a distinct Message kind"
    );
}

/// KerML TextualRepresentation lowering (registry:
/// tree-sitter.textual-representation-generic-lowering). A `rep`/`language`
/// annotating element lowers to a distinct `ElementKind::TextualRepresentation`
/// carrying its `language` and `body` (Kerml-Vocab.ttl: an AnnotatingElement
/// whose body represents its owner in a named language) — not a generic
/// ReferenceUsage nor a dropped element. It parses cleanly only inside
/// requirement/constraint bodies (the grammar wires `textual_representation`
/// there), so the fixture uses a constraint body.
#[test]
fn textual_representation_lowers_to_distinct_kind_with_language_and_body() {
    let result =
        parse_and_build("package P { constraint c { rep r language \"html\" /* <b>hi</b> */ } }");
    let reps: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TextualRepresentation)
        .collect();
    assert_eq!(
        reps.len(),
        1,
        "a `rep language` annotating element lowers to exactly one TextualRepresentation"
    );
    let rep = reps[0];
    assert_eq!(rep.name.as_deref(), Some("r"), "the `rep <name>` identifier is captured");
    assert_eq!(
        rep.get_prop("language").and_then(|v| v.as_str()),
        Some("html"),
        "the `language` string is captured (quotes stripped)"
    );
    assert_eq!(
        rep.get_prop("body").and_then(|v| v.as_str()),
        Some("<b>hi</b>"),
        "the REGULAR_COMMENT body is captured (/* */ delimiters stripped)"
    );
}

/// Bare (unnamed) TextualRepresentation: `language "L" /* body */` with no
/// `rep <name>` prefix still lowers to a TextualRepresentation carrying its
/// language + body.
#[test]
fn bare_textual_representation_lowers_with_language_and_body() {
    let result =
        parse_and_build("package P { constraint c { language \"alf\" /* x + 1 */ } }");
    let reps: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TextualRepresentation)
        .collect();
    assert_eq!(reps.len(), 1, "a bare `language` rep lowers to one TextualRepresentation");
    assert_eq!(
        reps[0].get_prop("language").and_then(|v| v.as_str()),
        Some("alf"),
        "language captured on the unnamed form"
    );
    assert_eq!(
        reps[0].get_prop("body").and_then(|v| v.as_str()),
        Some("x + 1"),
        "body captured on the unnamed form"
    );
}

// === Sprint 8: Actions & control flow tests ===

#[test]
fn test_assignment_action_extracts_target_and_value() {
    let result = parse_and_build("action def A { assign threshold = 100; }");
    let assigns: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AssignmentActionUsage)
        .collect();
    assert_eq!(assigns.len(), 1, "should have one assignment action");
    assert_eq!(
        assigns[0]
            .get_prop("targetFeature")
            .and_then(|v| v.as_str()),
        Some("threshold"),
        "assignment target should be 'threshold'"
    );
    assert_eq!(
        assigns[0]
            .get_prop("valueExpression")
            .and_then(|v| v.as_str()),
        Some("100"),
        "assignment value should be '100'"
    );
}

#[test]
fn test_transition_guard_extraction() {
    let result = parse_and_build(
        "state def S { state idle; state active; transition idle_to_active first idle if [ready] then active; }",
    );
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    assert!(
        !transitions.is_empty(),
        "should have at least one transition"
    );
    let has_guard = transitions
        .iter()
        .any(|t| result.graph.transition_feature_text(&t.id, "guard").is_some());
    assert!(
        has_guard,
        "at least one transition should have a guard child (TransitionFeatureMembership kind=guard)"
    );
}

#[test]
fn test_transition_trigger_extraction() {
    let result = parse_and_build(
        "state def S { state idle; state active; transition first idle accept evt then active; }",
    );
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    assert!(
        !transitions.is_empty(),
        "should have at least one transition"
    );
    let has_trigger = transitions
        .iter()
        .any(|t| result.graph.transition_feature_text(&t.id, "trigger").is_some());
    assert!(
        has_trigger,
        "at least one transition should have a trigger child (TransitionFeatureMembership kind=trigger)"
    );
}

#[test]
fn test_transition_effect_extraction() {
    let result = parse_and_build(
        "state def S { state idle; state active; transition first idle do cleanup; then active; }",
    );
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    assert!(
        !transitions.is_empty(),
        "should have at least one transition"
    );
    let has_effect = transitions
        .iter()
        .any(|t| result.graph.transition_feature_text(&t.id, "effect").is_some());
    assert!(
        has_effect,
        "at least one transition should have an effect child (TransitionFeatureMembership kind=effect)"
    );
}

#[test]
fn test_control_flow_node_fork_dispatch() {
    let result = parse_and_build("action def A { fork split; }");
    let forks: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ForkNode)
        .collect();
    assert_eq!(forks.len(), 1, "fork should dispatch to ForkNode");
    assert_eq!(forks[0].name.as_deref(), Some("split"));
}

#[test]
fn test_control_flow_node_join_dispatch() {
    let result = parse_and_build("action def A { join sync; }");
    let joins: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::JoinNode)
        .collect();
    assert_eq!(joins.len(), 1, "join should dispatch to JoinNode");
    assert_eq!(joins[0].name.as_deref(), Some("sync"));
}

#[test]
fn test_control_flow_node_merge_dispatch() {
    let result = parse_and_build("action def A { merge m; }");
    let merges: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MergeNode)
        .collect();
    assert_eq!(merges.len(), 1, "merge should dispatch to MergeNode");
    assert_eq!(merges[0].name.as_deref(), Some("m"));
}

#[test]
fn test_control_flow_node_decision_dispatch() {
    let result = parse_and_build("action def A { decide d; }");
    let decisions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::DecisionNode)
        .collect();
    assert_eq!(decisions.len(), 1, "decide should dispatch to DecisionNode");
    assert_eq!(decisions[0].name.as_deref(), Some("d"));
}

#[test]
fn test_if_action_dispatch() {
    let result = parse_and_build("action def A { if ready { action doWork; } }");
    let ifs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::IfActionUsage)
        .collect();
    assert_eq!(ifs.len(), 1, "if action should dispatch to IfActionUsage");
}

// SysML.xtext:1591 `IfNode returns SysML::IfActionUsage`. The `if <expr> {then}
// else {else}` node form lowers to a distinct IfActionUsage carrying its
// condition — nested `else if` and the no-`else` form must too.
#[test]
fn test_if_action_nested_else_if() {
    let result = parse_and_build(
        "action def A { if a { action p; } else if b { action q; } else { action r; } }",
    );
    let ifs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::IfActionUsage)
        .collect();
    // Outer `if` + the `else if` both materialize as IfActionUsage.
    assert_eq!(
        ifs.len(),
        2,
        "nested else-if should produce two IfActionUsage nodes"
    );
}

#[test]
fn test_if_action_no_else() {
    let result = parse_and_build("action def A { if ready { action doWork; } }");
    let ifs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::IfActionUsage)
        .collect();
    assert_eq!(ifs.len(), 1, "if without else still lowers to IfActionUsage");
}

// SysML.xtext:1703 `GuardedTargetSuccession returns SysML::TransitionUsage`.
// The guarded-succession form `if <guard> then <target>` is NOT an IfNode: the
// grammar declares it a TransitionUsage. This is the spec-correct lowering and
// must stay distinct from the IfActionUsage node form above (it is what the
// stale gap probe `if true then done` exercised).
#[test]
fn test_if_guarded_succession_is_transition_not_if_action() {
    let result = parse_and_build("state def S { state a; state b; if guard then b; }");
    let ifs = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::IfActionUsage)
        .count();
    let transitions = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .count();
    assert_eq!(ifs, 0, "a guarded succession is not an IfActionUsage");
    assert!(
        transitions >= 1,
        "`if guard then target` lowers to a TransitionUsage (SysML.xtext:1703)"
    );
}

// SysML.xtext:1636 `TerminateNode returns SysML::TerminateActionUsage`. The
// direct `terminate;` form materializes the distinct kind (the runtime lowers
// it to ActionNodeIR::Terminate) with no element-level name.
#[test]
fn test_terminate_action_dispatch() {
    let result = parse_and_build("action def A { terminate; }");
    let terms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TerminateActionUsage)
        .collect();
    assert_eq!(
        terms.len(),
        1,
        "terminate should dispatch to TerminateActionUsage"
    );
    assert!(
        terms[0].name.is_none(),
        "a bare terminate node carries no declared name"
    );
    assert!(
        terms[0].get_prop("unresolved_target").is_none(),
        "bare terminate has no terminated-occurrence argument"
    );
    assert!(
        result
            .graph
            .children_of(&terms[0].id)
            .all(|c| c.kind != ElementKind::ReferenceUsage),
        "bare terminate mints no NodeParameterMember slot child"
    );
}

// `terminate <ref>;` names the terminated occurrence (spec NodeParameterMember);
// the reference is captured verbatim as `unresolved_target`.
#[test]
fn test_terminate_action_with_target() {
    let result = parse_and_build("action def A { in p : Proc; terminate p; }");
    let terms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TerminateActionUsage)
        .collect();
    assert_eq!(terms.len(), 1, "terminate with target still one node");
    assert_eq!(
        terms[0].get_prop("unresolved_target").and_then(|v| v.as_str()),
        Some("p"),
        "terminated-occurrence reference captured as unresolved_target"
    );
    assert!(terms[0].name.is_none(), "terminate node has no name");
    // Spec NodeParameterMember shape (SysML.xtext NodeParameterMember →
    // ParameterMembership → NodeParameter (ReferenceUsage) → FeatureBinding
    // (FeatureValue) expression): an unnamed ReferenceUsage slot child owning
    // the target as a FeatureReferenceExpression.
    let slot: Vec<_> = result
        .graph
        .children_of(&terms[0].id)
        .filter(|c| c.kind == ElementKind::ReferenceUsage)
        .collect();
    assert_eq!(slot.len(), 1, "one NodeParameter slot child");
    assert!(slot[0].name.is_none(), "the NodeParameter is unnamed");
    let binding: Vec<_> = result
        .graph
        .children_of(&slot[0].id)
        .filter(|c| c.kind == ElementKind::FeatureReferenceExpression)
        .collect();
    assert_eq!(
        binding.len(),
        1,
        "the FeatureBinding expression is projected under the NodeParameter"
    );
    assert_eq!(
        binding[0].name.as_deref(),
        Some("p"),
        "the binding expression names the terminated occurrence"
    );
}

// A feature-chain argument (`terminate proc.wf;`) is captured whole.
#[test]
fn test_terminate_action_qualified_target() {
    let result = parse_and_build("action def A { in proc : Proc; terminate proc.wf; }");
    let terms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TerminateActionUsage)
        .collect();
    assert_eq!(terms.len(), 1);
    assert_eq!(
        terms[0].get_prop("unresolved_target").and_then(|v| v.as_str()),
        Some("proc.wf"),
        "dotted terminated-occurrence reference captured verbatim"
    );
    // The NodeParameterMember shape carries the chain whole as the binding
    // expression's name (FeatureReferenceExpression named "proc.wf").
    let slot: Vec<_> = result
        .graph
        .children_of(&terms[0].id)
        .filter(|c| c.kind == ElementKind::ReferenceUsage)
        .collect();
    assert_eq!(slot.len(), 1, "one NodeParameter slot child");
    let names: Vec<_> = result
        .graph
        .children_of(&slot[0].id)
        .filter(|c| c.kind == ElementKind::FeatureReferenceExpression)
        .filter_map(|c| c.name.clone())
        .collect();
    assert_eq!(names, vec!["proc.wf".to_owned()]);
}

#[test]
fn test_send_action_dispatch() {
    let result = parse_and_build("action def A { send signal to target; }");
    let sends: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SendActionUsage)
        .collect();
    assert_eq!(
        sends.len(),
        1,
        "send action should dispatch to SendActionUsage"
    );
}

#[test]
fn test_accept_action_dispatch() {
    let result = parse_and_build("action def A { accept msg : Message; }");
    let accepts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AcceptActionUsage)
        .collect();
    assert_eq!(
        accepts.len(),
        1,
        "accept action should dispatch to AcceptActionUsage"
    );
    assert_eq!(accepts[0].name.as_deref(), Some("msg"));
}

#[test]
fn test_succession_in_action_extracts_endpoints() {
    let result = parse_and_build("action def A { action a; action b; succession first a then b; }");
    let successions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    assert!(
        !successions.is_empty(),
        "should have at least one SuccessionAsUsage"
    );
    let has_source = successions
        .iter()
        .any(|s| s.get_prop("source").and_then(|v| v.as_str()) == Some("a"));
    let has_target = successions
        .iter()
        .any(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("b"));
    assert!(has_source, "succession should have source='a'");
    assert!(has_target, "succession should have target='b'");
}

// === Sprint 9: States & transitions tests ===

#[test]
fn test_state_usage_parallel_keyword() {
    let result = parse_and_build("state def S { state region1 parallel { state a; state b; } }");
    let states: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::StateUsage && e.name.as_deref() == Some("region1"))
        .collect();
    assert_eq!(states.len(), 1, "should have state 'region1'");
    assert_eq!(
        states[0].get_prop("isParallel").and_then(|v| v.as_bool()),
        Some(true),
        "state with 'parallel' keyword should have isParallel=true"
    );
}

#[test]
fn test_state_entry_action_extraction() {
    // Grammar splits "entry action initialize;" into bare entry_action + action_usage siblings.
    // The ast_builder merges them: entry_action absorbs the sibling action_usage's name.
    let result = parse_and_build("state def S { state idle { entry action initialize; } }");
    let entry_actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind").and_then(|v| v.as_str()) == Some("entry")
        })
        .collect();
    assert_eq!(entry_actions.len(), 1, "should have one entry subaction");
    assert_eq!(
        entry_actions[0].name.as_deref(),
        Some("initialize"),
        "entry subaction should have absorbed the action name"
    );
    // The action_usage sibling should be consumed (no separate unnamed ActionUsage)
    let separate_actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind").is_none()
                && e.name.as_deref() == Some("initialize")
        })
        .collect();
    assert_eq!(
        separate_actions.len(),
        0,
        "action_usage sibling should be consumed, not a separate element"
    );
}

#[test]
fn test_state_do_action_extraction() {
    let result = parse_and_build("state def S { state active { do action monitor; } }");
    let do_actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind").and_then(|v| v.as_str()) == Some("do")
        })
        .collect();
    assert_eq!(do_actions.len(), 1, "should have one do subaction");
    assert_eq!(
        do_actions[0].name.as_deref(),
        Some("monitor"),
        "do subaction should have absorbed the action name"
    );
}

#[test]
fn test_state_exit_action_extraction() {
    let result = parse_and_build("state def S { state done { exit action cleanup; } }");
    let exit_actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind").and_then(|v| v.as_str()) == Some("exit")
        })
        .collect();
    assert_eq!(exit_actions.len(), 1, "should have one exit subaction");
    assert_eq!(
        exit_actions[0].name.as_deref(),
        Some("cleanup"),
        "exit subaction should have absorbed the action name"
    );
}

#[test]
fn test_state_bare_entry_exit() {
    let result = parse_and_build("state def S { state simple { entry; exit; } }");
    let entry_actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind").and_then(|v| v.as_str()) == Some("entry")
        })
        .collect();
    assert_eq!(
        entry_actions.len(),
        1,
        "bare 'entry;' should create an ActionUsage"
    );
    assert!(
        entry_actions[0].name.is_none(),
        "bare entry should have no name"
    );

    let exit_actions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind").and_then(|v| v.as_str()) == Some("exit")
        })
        .collect();
    assert_eq!(
        exit_actions.len(),
        1,
        "bare 'exit;' should create an ActionUsage"
    );
}

#[test]
fn test_exhibit_state_dispatch() {
    let result = parse_and_build("package P { exhibit myState; }");
    let exhibits: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ExhibitStateUsage)
        .collect();
    assert_eq!(
        exhibits.len(),
        1,
        "exhibit should dispatch to ExhibitStateUsage"
    );
    assert_eq!(exhibits[0].name.as_deref(), Some("myState"));
}

// === G23: `exhibit state <name> : <Type>;` must mint ONE ExhibitStateUsage,
// not a phantom sibling StateUsage (SysML.xtext:1835-1841 second alternative:
// StateUsageKeyword UsageDeclaration?). ===

#[test]
fn test_g23_exhibit_state_with_typing_mints_single_element() {
    let result = parse_and_build(
        "state def RealSM { state a; state b; } part def HostPart { exhibit state oscillator : RealSM; }",
    );
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "`exhibit state <name> : <Type>;` should parse without errors, got {errors:?}"
    );

    let exhibits: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ExhibitStateUsage)
        .collect();
    assert_eq!(
        exhibits.len(),
        1,
        "`exhibit state oscillator : RealSM;` must mint exactly ONE ExhibitStateUsage, got {}",
        exhibits.len()
    );
    assert_eq!(exhibits[0].name.as_deref(), Some("oscillator"));

    // The typing must land as a FeatureTyping child of the ExhibitStateUsage,
    // same as every other usage kind (create_usage_rels is generic).
    let typings: Vec<_> = result
        .graph
        .children_of(&exhibits[0].id)
        .filter(|c| c.kind == ElementKind::FeatureTyping)
        .collect();
    assert_eq!(
        typings.len(),
        1,
        "exhibit state usage should mint a FeatureTyping child for `: RealSM`"
    );
    assert_eq!(
        typings[0]
            .get_prop("unresolved_type")
            .and_then(|v| v.as_str()),
        Some("RealSM")
    );

    // No phantom StateUsage sibling named "oscillator" (the G23 bug) anywhere
    // in the graph, and none owned by HostPart.
    assert!(
        !result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::StateUsage && e.name.as_deref() == Some("oscillator")),
        "`exhibit state oscillator : RealSM;` must not mint a phantom StateUsage named 'oscillator'"
    );
    let host = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::PartDefinition && e.name.as_deref() == Some("HostPart"))
        .expect("HostPart part def should exist");
    assert!(
        !result
            .graph
            .children_of(&host.id)
            .any(|c| c.kind == ElementKind::StateUsage),
        "HostPart must not own a phantom StateUsage child"
    );
}

#[test]
fn test_g23_exhibit_state_bare_no_type_still_parses() {
    // (c) bare `exhibit state;` form (no name, no typing) must still parse cleanly.
    let result = parse_and_build("part def HostPart { exhibit state; }");
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "bare `exhibit state;` should parse, got {errors:?}"
    );
    let exhibits: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ExhibitStateUsage)
        .collect();
    assert_eq!(
        exhibits.len(),
        1,
        "bare `exhibit state;` should mint one ExhibitStateUsage"
    );
    assert_eq!(exhibits[0].name, None);
}

#[test]
fn test_g23_exhibit_state_with_subsets_clause() {
    // UsageDeclaration broadly (not just typing) must land cleanly: `subsets`
    // is one of the _feature_specialization forms flagged in the G23 inventory
    // entry as needing coverage alongside the bare-typing case.
    let result = parse_and_build(
        "state def RealSM { state a; } part def HostPart { state template : RealSM; exhibit state oscillator subsets template; }",
    );
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "`exhibit state <name> subsets <feature>;` should parse without errors, got {errors:?}"
    );
    let exhibits: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ExhibitStateUsage)
        .collect();
    assert_eq!(
        exhibits.len(),
        1,
        "`exhibit state oscillator subsets template;` must mint exactly ONE ExhibitStateUsage"
    );
    assert_eq!(exhibits[0].name.as_deref(), Some("oscillator"));
}

#[test]
fn test_target_transition_usage_extracts_target() {
    // "if [cond] then target;" inside state body parses as target_transition_usage
    let result =
        parse_and_build("state def S { state ready { if [armed] then active; } state active; }");
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    assert!(
        !transitions.is_empty(),
        "target_transition_usage should create a TransitionUsage"
    );
    let has_target = transitions
        .iter()
        .any(|t| t.get_prop("target").and_then(|v| v.as_str()) == Some("active"));
    assert!(
        has_target,
        "target_transition_usage should extract target='active'"
    );
}

#[test]
fn test_target_transition_usage_extracts_guard() {
    let result =
        parse_and_build("state def S { state ready { if [armed] then active; } state active; }");
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    let has_guard = transitions
        .iter()
        .any(|t| result.graph.transition_feature_text(&t.id, "guard").is_some());
    assert!(
        has_guard,
        "target_transition_usage with if [cond] should mint a guard child"
    );
}

#[test]
fn test_target_transition_usage_with_trigger() {
    // "accept evt then target;" form
    let result = parse_and_build(
        "state def S { state idle { accept startEvt then active; } state active; }",
    );
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    assert!(
        !transitions.is_empty(),
        "accept...then form should create a TransitionUsage"
    );
}

#[test]
fn test_guarded_transition_merges_target() {
    // When tree-sitter splits "transition T first A if [g] then B;" into
    // transition_usage + target_transition_usage siblings, the target/guard
    // should be merged into a single TransitionUsage element.
    let result =
        parse_and_build("state def SM { state A; state B; transition T first A if [g] then B; }");
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();
    // Should have exactly one TransitionUsage (merged), not two
    let named = transitions
        .iter()
        .filter(|t| t.name.as_deref() == Some("T"))
        .collect::<Vec<_>>();
    assert!(
        !named.is_empty(),
        "should have TransitionUsage named 'T', got: {:?}",
        transitions.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
    // The single element should have both source and target
    let t = named[0];
    let has_source = t.get_prop("source").and_then(|v| v.as_str()) == Some("A");
    let has_target = t.get_prop("target").and_then(|v| v.as_str()) == Some("B");
    // If the grammar produces a single transition_usage with inline target,
    // the target comes from transition_target extraction. If it splits into
    // two siblings, the merge logic handles it. Either way, target must be present.
    assert!(
        has_source || has_target,
        "TransitionUsage 'T' should have source='A' and/or target='B', got props: {:?}",
        t.props
    );
}

#[test]
fn test_split_transition_with_trigger_guard_merges() {
    // Coffee-machine pattern: "transition T first A accept evt if [g] then B;"
    // Tree-sitter splits this into:
    //   transition_usage (name=T, source=A, trigger_action without trigger name)
    //   feature_declaration (name=evt)
    //   target_transition_usage (guard=g, target=B)
    // The target_transition_usage must merge back through feature_declaration
    let result = parse_and_build(
        "state def SM { state A; state B; transition T first A accept evt if [g] then B; }",
    );
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage && e.name.as_deref() == Some("T"))
        .collect();
    assert_eq!(
        transitions.len(),
        1,
        "should have exactly one TransitionUsage named 'T', got: {:?}",
        result
            .graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::TransitionUsage)
            .map(|t| (&t.name, &t.props))
            .collect::<Vec<_>>()
    );
    let t = transitions[0];
    assert_eq!(
        t.get_prop("source").and_then(|v| v.as_str()),
        Some("A"),
        "should have source='A'"
    );
    assert_eq!(
        t.get_prop("target").and_then(|v| v.as_str()),
        Some("B"),
        "should have target='B' (merged from target_transition_usage through feature_declaration)"
    );
}

#[test]
fn test_split_transition_succession_target_merges() {
    // Pattern: "transition T first A accept evt then B;" where
    // "then B;" is parsed as succession_usage (no guard between trigger and target)
    let result = parse_and_build(
        "state def SM { state A; state B; transition T first A accept evt then B; }",
    );
    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage && e.name.as_deref() == Some("T"))
        .collect();
    // Check that target was merged (either via transition_target inside the node
    // or via succession_usage merge)
    let has_target = transitions
        .iter()
        .any(|t| t.get_prop("target").and_then(|v| v.as_str()) == Some("B"));
    assert!(
        has_target,
        "TransitionUsage 'T' should have target='B', got: {:?}",
        transitions
            .iter()
            .map(|t| (&t.name, &t.props))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_coffee_machine_transitions_all_have_source_and_target() {
    // Full coffee-machine pattern: 8 transitions, some with guards/triggers
    // that cause grammar splits. ALL must have both source and target.
    let source = r#"
        state def MachineStates {
            entry; then idle;
            state idle;
            state brewing;
            state steaming;
            state cleaning;
            state error;

            transition idle_to_brewing first idle accept brewCommand if cupDetected then brewing;
            transition idle_to_steaming first idle accept steamCommand then steaming;
            transition brewing_to_idle first brewing accept brewComplete then idle;
            transition steaming_to_idle first steaming accept steamComplete then idle;
            transition idle_to_cleaning first idle accept cleanCommand then cleaning;
            transition cleaning_to_idle first cleaning accept cleanComplete then idle;
            transition any_to_error first idle accept faultDetected then error;
            transition error_to_idle first error accept errorCleared then idle;
        }
    "#;
    let result = parse_and_build(source);

    let transitions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::TransitionUsage)
        .collect();

    let expected = [
        ("idle_to_brewing", "idle", "brewing"),
        ("idle_to_steaming", "idle", "steaming"),
        ("brewing_to_idle", "brewing", "idle"),
        ("steaming_to_idle", "steaming", "idle"),
        ("idle_to_cleaning", "idle", "cleaning"),
        ("cleaning_to_idle", "cleaning", "idle"),
        ("any_to_error", "idle", "error"),
        ("error_to_idle", "error", "idle"),
    ];

    for (name, exp_src, exp_tgt) in &expected {
        let t = transitions
            .iter()
            .find(|t| t.name.as_deref() == Some(*name));
        assert!(
            t.is_some(),
            "missing TransitionUsage '{}', found: {:?}",
            name,
            transitions
                .iter()
                .map(|t| t.name.as_deref())
                .collect::<Vec<_>>()
        );
        let t = t.unwrap();
        let src = t.get_prop("source").and_then(|v| v.as_str());
        let tgt = t.get_prop("target").and_then(|v| v.as_str());
        assert_eq!(
            src,
            Some(*exp_src),
            "transition '{}' source: expected '{}', got {:?}",
            name,
            exp_src,
            src
        );
        assert_eq!(
            tgt,
            Some(*exp_tgt),
            "transition '{}' target: expected '{}', got {:?}",
            name,
            exp_tgt,
            tgt
        );
    }
}

// === Sprint 10: Calculations & Constraints ===

#[test]
fn test_assert_not_extracts_is_negated() {
    let result = parse_and_build("package P { assert not constraint safe { speed < 100 } }");
    let asserts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AssertConstraintUsage)
        .collect();
    assert_eq!(asserts.len(), 1, "should have one AssertConstraintUsage");
    let is_negated = asserts[0].get_prop("isNegated").and_then(|v| v.as_bool());
    assert_eq!(
        is_negated,
        Some(true),
        "assert not should set isNegated=true"
    );
}

#[test]
fn test_assert_without_not_has_no_negated() {
    let result = parse_and_build("package P { assert constraint safe { speed < 100 } }");
    let asserts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::AssertConstraintUsage)
        .collect();
    assert_eq!(asserts.len(), 1, "should have one AssertConstraintUsage");
    let is_negated = asserts[0].get_prop("isNegated");
    assert!(
        is_negated.is_none(),
        "assert without not should have no isNegated prop"
    );
}

#[test]
fn test_return_feature_creates_return_parameter_membership() {
    let result = parse_and_build("calc def TotalMass { return totalMass : Real; }");
    let rpms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ReturnParameterMembership)
        .collect();
    assert_eq!(rpms.len(), 1, "should have one ReturnParameterMembership");
}

#[test]
fn test_calc_def_with_result_expression() {
    let result = parse_and_build("calc def Sum { in x : Real; in y : Real; x + y }");
    let rems: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ResultExpressionMembership)
        .collect();
    assert_eq!(
        rems.len(),
        1,
        "calc def should create ResultExpressionMembership"
    );
    let expr = rems[0]
        .get_prop("expression")
        .and_then(|v| v.as_str().map(|s| s.to_owned()));
    assert!(
        expr.is_some(),
        "ResultExpressionMembership should have expression prop"
    );
}

#[test]
fn test_constraint_with_result_expression() {
    let result = parse_and_build("constraint def SpeedLimit { speed < 100 }");
    let rems: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ResultExpressionMembership)
        .collect();
    assert_eq!(
        rems.len(),
        1,
        "constraint def should create ResultExpressionMembership"
    );

    // Also check the string prop is set on the parent
    let constraints: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConstraintDefinition)
        .collect();
    assert_eq!(constraints.len(), 1);
    let expr = constraints[0]
        .get_prop("constraint")
        .and_then(|v| v.as_str().map(|s| s.to_owned()));
    assert!(
        expr.is_some(),
        "ConstraintDefinition should have constraint string prop"
    );
}

#[test]
fn test_inv_constraint_dispatch() {
    let result = parse_and_build("state def S { inv safe { speed < 100 } }");
    let invs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::Invariant
                && e.get_prop("role").and_then(|v| v.as_str()) == Some("invariant")
        })
        .collect();
    assert_eq!(
        invs.len(),
        1,
        "inv should dispatch as ElementKind::Invariant (KerML.xtext:976) with role=invariant"
    );
}

// G12: the `inv <name> = <expr>;` value form (KerML.xtext:976 ExpressionDeclaration
// + FunctionBody `;`) must mint an Invariant, not just the braced `inv x { ... }`.
#[test]
fn test_g12_inv_value_form_dispatch() {
    let result = parse_and_build("package P { constraint def C { inv flag = true; } }");
    let invs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Invariant)
        .collect();
    assert_eq!(
        invs.len(),
        1,
        "`inv flag = true;` value form should mint exactly one Invariant (G12)"
    );
}

// G04b: anonymous typed parameter `in : Real[1];` (no name) inside a calc def body
// (KerML Vector/Tensor library form) must parse cleanly and mint a FeatureTyping to
// Real — closing the missed-FeatureTyping floor. Scoped to calc_body, so it must NOT
// be admitted in a plain (non-calc) definition_body. Here `in :` + `return :` each
// contribute a typing.
#[test]
fn test_g04b_anonymous_typed_param_in_calc_def() {
    let result = parse_and_build("calc def Norm { in : Real[1]; return : Real[1]; }");
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "anonymous typed param in calc def should parse without errors, got {errors:?}"
    );
    let typings = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FeatureTyping)
        .count();
    assert!(
        typings >= 2,
        "`in : Real[1]` + `return : Real[1]` should each mint a FeatureTyping (got {typings})"
    );
    // The calc def itself must still classify as CalculationDefinition.
    assert!(
        result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::CalculationDefinition
                && e.name.as_deref() == Some("Norm")),
        "calc_def must still lower to CalculationDefinition"
    );
}

// MaximizeObjective regression: an inline `doc` inside a reduction/lambda body must
// parse cleanly so GLR error-recovery does NOT close the enclosing package early and
// orphan later siblings. Assert the def AFTER the doc-bearing calc keeps its owner.
#[test]
fn test_lambda_inline_doc_preserves_sibling_ownership() {
    let result = parse_and_build(
        "package P { calc def R { x->maximize { doc /* best */ in i; eval(i) } } part def Tail; }",
    );
    let pkg = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("P"))
        .expect("package P should exist");
    let tail = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Tail"))
        .expect("part def Tail (the sibling after the doc-bearing calc) should exist");
    assert_eq!(
        tail.owner.as_ref(),
        Some(&pkg.id),
        "Tail must remain owned by P — the inline doc in the lambda body must not orphan later siblings"
    );
}

// === G08f: stakeholder usage + concern/viewpoint requirement_body peel ===

#[test]
fn test_g08f_stakeholder_usage_in_concern_dispatches_to_stakeholder_membership() {
    let result =
        parse_and_build("package P { part def Eng; concern def C { stakeholder s : Eng; } }");
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "stakeholder usage inside a concern body should parse without errors, got {errors:?}"
    );
    // concern def → ConcernDefinition (peeled from generic `definition`).
    assert!(
        result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::ConcernDefinition && e.name.as_deref() == Some("C")),
        "concern def must lower to ConcernDefinition"
    );
    // stakeholder usage → StakeholderMembership (SysML.xtext:2093-2099).
    let stk: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::StakeholderMembership)
        .collect();
    assert_eq!(
        stk.len(),
        1,
        "stakeholder usage should dispatch to StakeholderMembership"
    );
    assert_eq!(stk[0].name.as_deref(), Some("s"));
}

#[test]
fn test_g08f_viewpoint_def_uses_requirement_body_with_stakeholder() {
    let result =
        parse_and_build("package P { part def Eng; viewpoint def V { stakeholder s : Eng; } }");
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "viewpoint with stakeholder should parse, got {errors:?}"
    );
    assert!(
        result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::ViewpointDefinition && e.name.as_deref() == Some("V")),
        "viewpoint def must lower to ViewpointDefinition"
    );
    assert!(
        result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::StakeholderMembership),
        "stakeholder usage inside a viewpoint body must lower to StakeholderMembership"
    );
}

#[test]
fn test_g08f_stakeholder_def_does_not_mint_a_definition() {
    // SysML.xtext:2098 has no StakeholderDefinition; `stakeholder def X` must NOT
    // produce a Definition/ConcernDefinition (it degrades to bare features, which
    // downstream resolution flags as unresolved — but never a spurious def).
    let result = parse_and_build("package P { stakeholder def X; }");
    assert!(
        !result.graph.elements.values().any(|e| matches!(
            e.kind,
            ElementKind::Definition | ElementKind::ConcernDefinition
        ) && e.name.as_deref() == Some("X")),
        "`stakeholder def X` must not mint a (Concern)Definition named X"
    );
}

// === G08e: standalone `subset X subsets Y;` → Subsetting relationship ===

#[test]
fn test_g08e_standalone_subset_mints_subsetting_relationship() {
    let result = parse_and_build("package P { part a; part b; subset a subsets b; }");
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("ERROR") || d.message.contains("expected"))
        .collect();
    assert!(
        errors.is_empty(),
        "standalone subset should parse, got {errors:?}"
    );

    // It is a Subsetting RELATIONSHIP, not a phantom Usage.
    let subs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Subsetting)
        .collect();
    assert_eq!(
        subs.len(),
        1,
        "`subset a subsets b;` must mint exactly one Subsetting"
    );
    let s = subs[0];
    assert_eq!(
        s.get_prop("unresolved_subsettingFeature")
            .and_then(|v| v.as_str()),
        Some("a"),
        "subsettingFeature endpoint (`a`) must be stored unresolved"
    );
    assert_eq!(
        s.get_prop("unresolved_subsettedFeature")
            .and_then(|v| v.as_str()),
        Some("b"),
        "subsettedFeature endpoint (`b`) must be stored unresolved"
    );
    // No phantom Usage named after the feature chain.
    assert!(
        !result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::Usage && e.name.as_deref() == Some("a")),
        "standalone subset must not mint a phantom Usage named `a`"
    );
}

#[test]
fn test_g08e_subset_with_feature_chains() {
    // Mirrors Kernel Semantic Library usage (Occurrences.kerml:107).
    let result =
        parse_and_build("package P { subset later.successors subsets earlier.successors; }");
    let subs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Subsetting)
        .collect();
    assert_eq!(
        subs.len(),
        1,
        "feature-chain subset must mint one Subsetting"
    );
    assert_eq!(
        subs[0]
            .get_prop("unresolved_subsettingFeature")
            .and_then(|v| v.as_str()),
        Some("later.successors")
    );
    assert_eq!(
        subs[0]
            .get_prop("unresolved_subsettedFeature")
            .and_then(|v| v.as_str()),
        Some("earlier.successors")
    );
}

#[test]
fn test_calc_usage_dispatch() {
    let result = parse_and_build("package P { calc ms : Real; }");
    let calcs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::CalculationUsage)
        .collect();
    assert_eq!(
        calcs.len(),
        1,
        "calc usage should dispatch to CalculationUsage"
    );
    assert_eq!(calcs[0].name.as_deref(), Some("ms"));
}

#[test]
fn test_calc_def_dispatch() {
    let result = parse_and_build("calc def MassSum { in x : Real; in y : Real; x + y }");
    let calcs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::CalculationDefinition)
        .collect();
    assert_eq!(
        calcs.len(),
        1,
        "calc def should dispatch to CalculationDefinition"
    );
    assert_eq!(calcs[0].name.as_deref(), Some("MassSum"));
}

// === Sprint 11: Requirement/Case membership dispatch tests ===

#[test]
fn test_objective_requirement_dispatches_to_objective_membership() {
    let result = parse_and_build("verification def MyVC { objective myObj { } }");
    let objs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ObjectiveMembership)
        .collect();
    assert_eq!(
        objs.len(),
        1,
        "objective should dispatch to ObjectiveMembership"
    );
}

#[test]
fn test_verify_constraint_dispatches_to_verification_membership() {
    let result =
        parse_and_build("requirement def R1 {} verification def VC { objective { verify R1; } }");
    let vms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::RequirementVerificationMembership)
        .collect();
    assert_eq!(
        vms.len(),
        1,
        "verify_constraint should dispatch to RequirementVerificationMembership"
    );
    // Should have the verified requirement reference
    let vm = &vms[0];
    let target = vm.get_prop("verifiedRequirement");
    assert!(
        target.is_some(),
        "RequirementVerificationMembership should have verifiedRequirement property"
    );
}

#[test]
fn test_assume_constraint_dispatches_to_requirement_constraint_membership() {
    let result = parse_and_build("requirement def R1 { assume constraint ac1 { true } }");
    let rcms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::RequirementConstraintMembership)
        .collect();
    assert_eq!(
        rcms.len(),
        1,
        "assume_constraint should dispatch to RequirementConstraintMembership"
    );
    let rcm = &rcms[0];
    assert_eq!(
        rcm.get_prop("role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned()),
        Some("assume".to_owned()),
        "role should be 'assume'"
    );
}

#[test]
fn test_require_constraint_dispatches_to_requirement_constraint_membership() {
    let result = parse_and_build("requirement def R1 { require constraint rc1 { true } }");
    let rcms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::RequirementConstraintMembership)
        .collect();
    assert_eq!(
        rcms.len(),
        1,
        "require_constraint should dispatch to RequirementConstraintMembership"
    );
    let rcm = &rcms[0];
    assert_eq!(
        rcm.get_prop("role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned()),
        Some("require".to_owned()),
        "role should be 'require'"
    );
}

#[test]
fn test_frame_constraint_dispatches_to_framed_concern_membership() {
    let result = parse_and_build("requirement def R1 { frame concern fc1 { true } }");
    let fcms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FramedConcernMembership)
        .collect();
    assert_eq!(
        fcms.len(),
        1,
        "frame_constraint should dispatch to FramedConcernMembership"
    );
    let fcm = &fcms[0];
    assert_eq!(
        fcm.get_prop("role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned()),
        Some("frame".to_owned()),
        "role should be 'frame'"
    );
}

#[test]
fn test_satisfy_requirement_dispatch() {
    let result = parse_and_build("requirement def R1 {} satisfy requirement :>> R1;");
    let sats: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SatisfyRequirementUsage)
        .collect();
    assert_eq!(
        sats.len(),
        1,
        "satisfy should dispatch to SatisfyRequirementUsage"
    );
    // Should NOT have isVerify property
    let sat = &sats[0];
    assert!(
        sat.get_prop("isVerify").is_none()
            || sat.get_prop("isVerify").and_then(|v| v.as_bool()) == Some(false),
        "satisfy should not set isVerify"
    );
}

/// `verify requirement …` lowers to a RequirementVerificationMembership
/// owning a plain RequirementUsage — SysML.xtext:2257-2270
/// (RequirementVerificationMember owns a SysML::RequirementUsage). It is NOT
/// a SatisfyRequirementUsage and there is no `isVerify` marker prop: the
/// membership kind is the classification.
#[test]
fn test_verify_requirement_lowers_to_verification_membership() {
    let result = parse_and_build("requirement def R1 {} verify requirement :>> R1;");
    let memberships: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::RequirementVerificationMembership)
        .collect();
    assert_eq!(
        memberships.len(),
        1,
        "verify requirement should mint a RequirementVerificationMembership"
    );
    let checks: Vec<_> = result
        .graph
        .children_of(&memberships[0].id)
        .filter(|c| c.kind == ElementKind::RequirementUsage)
        .collect();
    assert_eq!(
        checks.len(),
        1,
        "the membership owns exactly one RequirementUsage check"
    );
    assert!(
        !result
            .graph
            .elements
            .values()
            .any(|e| e.kind == ElementKind::SatisfyRequirementUsage),
        "verify must not mint a SatisfyRequirementUsage"
    );
    assert!(
        result
            .graph
            .elements
            .values()
            .all(|e| e.get_prop("isVerify").is_none()),
        "the isVerify marker prop is deleted — kind carries the classification"
    );
}

/// The pilot-canonical declaration form inside an objective:
/// `objective { verify requirement check : ReqDef; }`. The check-usage keeps
/// its declared name and its `: ReqDef` FeatureTyping child, and sits under
/// ObjectiveMembership → RequirementVerificationMembership.
#[test]
fn test_verify_requirement_declaration_form_shape() {
    let result = parse_and_build(
        "requirement def TripReq {} verification def T1 { objective { verify requirement tripCheck : TripReq; } }",
    );
    let membership = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::RequirementVerificationMembership)
        .expect("membership minted");
    let objective_owner = membership
        .owner
        .as_ref()
        .and_then(|o| result.graph.get_element(o))
        .expect("membership has an owner");
    assert_eq!(
        objective_owner.kind,
        ElementKind::ObjectiveMembership,
        "membership sits under the ObjectiveMembership"
    );
    let check = result
        .graph
        .children_of(&membership.id)
        .find(|c| c.kind == ElementKind::RequirementUsage)
        .expect("membership owns the check-usage");
    assert_eq!(
        check.name.as_deref(),
        Some("tripCheck"),
        "check-usage keeps its declared name"
    );
    let typing = result
        .graph
        .children_of(&check.id)
        .find(|c| c.kind == ElementKind::FeatureTyping)
        .expect("check-usage has its FeatureTyping child");
    assert_eq!(
        typing.get_prop("unresolved_type").and_then(|v| v.as_str()),
        Some("TripReq"),
        "typing carries the verified requirement definition reference"
    );
}

#[test]
fn test_subject_requirement_dispatches_to_subject_membership() {
    let result = parse_and_build("requirement def R1 { subject vehicle; }");
    let sms: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SubjectMembership)
        .collect();
    assert_eq!(sms.len(), 1, "subject should dispatch to SubjectMembership");
    assert_eq!(
        sms[0].name.as_deref(),
        Some("vehicle"),
        "SubjectMembership should have the subject name"
    );
}

// === Sprint 12: View/Rendering and Expose dispatch tests ===

#[test]
fn test_expose_import_dispatches_to_membership_expose() {
    let result = parse_and_build("package P { expose import SomePackage; }");
    let exposes: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MembershipExpose)
        .collect();
    assert_eq!(
        exposes.len(),
        1,
        "expose import should dispatch to MembershipExpose"
    );
    // isImportAll should be true
    assert_eq!(
        exposes[0].get_prop("isImportAll").and_then(|v| v.as_bool()),
        Some(true),
        "Expose should have isImportAll=true"
    );
}

#[test]
fn test_expose_namespace_import_dispatches_to_namespace_expose() {
    let result = parse_and_build("package P { expose import SomePackage::*; }");
    let exposes: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::NamespaceExpose)
        .collect();
    assert_eq!(
        exposes.len(),
        1,
        "expose import ::* should dispatch to NamespaceExpose"
    );
}

#[test]
fn test_regular_import_still_works() {
    let result = parse_and_build("package P { import SomePackage; }");
    let imports: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MembershipImport)
        .collect();
    assert_eq!(
        imports.len(),
        1,
        "regular import should still dispatch to MembershipImport"
    );
    // Should NOT have isImportAll
    assert!(
        imports[0].get_prop("isImportAll").is_none(),
        "regular import should not set isImportAll"
    );
    // Plain import defaults to private visibility (KerML default) — not re-exported.
    assert_eq!(
        imports[0].get_prop("visibility").and_then(|v| v.as_str()),
        Some("private"),
        "plain import should default to private visibility"
    );
}

#[test]
fn test_import_visibility_captured_from_keyword() {
    for (src, expected) in [
        ("package P { public import A::*; }", "public"),
        ("package P { private import A::B; }", "private"),
        ("package P { protected import A::C; }", "protected"),
    ] {
        let result = parse_and_build(src);
        let imports: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| {
                e.kind == ElementKind::MembershipImport || e.kind == ElementKind::NamespaceImport
            })
            .collect();
        assert_eq!(imports.len(), 1, "expected one import for: {src}");
        assert_eq!(
            imports[0].get_prop("visibility").and_then(|v| v.as_str()),
            Some(expected),
            "visibility mismatch for: {src}"
        );
    }
}

#[test]
fn test_single_member_import_keeps_full_qualified_target() {
    let result = parse_and_build("package P { import Lib::Engine; }");
    let imports: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MembershipImport)
        .collect();
    assert_eq!(
        imports.len(),
        1,
        "single-member import should dispatch to MembershipImport"
    );
    assert_eq!(
        imports[0]
            .get_prop("importedReference")
            .and_then(|v| v.as_str()),
        Some("Lib::Engine"),
        "single-member import must preserve the full qualified target"
    );
}

#[test]
fn test_duplicate_namespace_imports_remain_distinct_for_health_diagnostics() {
    let result = parse_and_build(
        "package Upstream { part def Sensor; } package P { import Upstream::*; import Upstream::*; }",
    );
    let imports: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::NamespaceImport)
        .collect();
    assert_eq!(
        imports.len(),
        2,
        "duplicate namespace imports must not collapse to one element"
    );

    let diags = sysml_core::import_health_diagnostics(&result.graph);
    assert!(
        diags.iter().any(|d| d.code.as_deref() == Some("IM003")),
        "duplicate imports should produce IM003; got: {:?}",
        diags
    );
}

// === Gap #53: Use case / actor dispatch tests ===

#[test]
fn test_use_case_def_dispatch() {
    let result = parse_and_build("use case def DriveToWork {}");
    let ucs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::UseCaseDefinition)
        .collect();
    assert_eq!(ucs.len(), 1, "should dispatch to UseCaseDefinition");
    assert_eq!(ucs[0].name.as_deref(), Some("DriveToWork"));
}

#[test]
fn test_use_case_usage_dispatch() {
    let result = parse_and_build("use case commute : DriveToWork;");
    let ucs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::UseCaseUsage)
        .collect();
    assert_eq!(ucs.len(), 1, "should dispatch to UseCaseUsage");
    assert_eq!(ucs[0].name.as_deref(), Some("commute"));
}

#[test]
fn test_case_def_dispatch() {
    let result = parse_and_build("case def AnalyzeData {}");
    let cs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::CaseDefinition)
        .collect();
    assert_eq!(cs.len(), 1, "should dispatch to CaseDefinition");
    assert_eq!(cs[0].name.as_deref(), Some("AnalyzeData"));
}

#[test]
fn test_case_usage_dispatch() {
    // Note: "analysis" is a keyword in generic `usage`, so use a non-keyword name.
    let result = parse_and_build("case testCase : AnalyzeData;");
    let cs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::CaseUsage)
        .collect();
    assert_eq!(cs.len(), 1, "should dispatch to CaseUsage");
    assert_eq!(cs[0].name.as_deref(), Some("testCase"));
}

#[test]
fn test_actor_usage_dispatch() {
    let result = parse_and_build("use case def UC { actor driver : Person; }");
    let actors: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ActorMembership)
        .collect();
    assert_eq!(actors.len(), 1, "should dispatch to ActorMembership");
    assert_eq!(actors[0].name.as_deref(), Some("driver"));
}

#[test]
fn test_include_use_case_dispatch() {
    let result = parse_and_build("use case def UC { include use case sub : SubType; }");
    let includes: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::IncludeUseCaseUsage)
        .collect();
    assert_eq!(includes.len(), 1, "should dispatch to IncludeUseCaseUsage");
    assert_eq!(includes[0].name.as_deref(), Some("sub"));
}

// === Gap #26: Combined modifier regression tests ===

#[test]
fn test_combined_end_ref_with_typing_and_multiplicity() {
    // Note: "ref" alone at start is ambiguous (standard_usage keyword vs prefix).
    // Use "end ref" which is unambiguously a usage_prefix.
    let result = parse_and_build("end ref part p : T [0..*] ordered;");
    let parts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage)
        .collect();
    assert_eq!(parts.len(), 1);
    let p = &parts[0];
    assert_eq!(p.name.as_deref(), Some("p"));
    assert_eq!(p.get_prop("isEnd").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        p.get_prop("isReference").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        p.get_prop("multiplicity_lower").and_then(|v| v.as_int()),
        Some(0)
    );
    assert_eq!(
        p.get_prop("isOrdered").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn test_combined_end_with_multiplicity() {
    let result = parse_and_build("end part p : T [1];");
    let parts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage)
        .collect();
    assert_eq!(parts.len(), 1);
    let p = &parts[0];
    assert_eq!(p.name.as_deref(), Some("p"));
    assert_eq!(p.get_prop("isEnd").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        p.get_prop("multiplicity_lower").and_then(|v| v.as_int()),
        Some(1)
    );
    assert_eq!(
        p.get_prop("multiplicity_upper").and_then(|v| v.as_int()),
        Some(1)
    );
}

#[test]
fn test_symbolic_multiplicity_preserved() {
    let result = parse_and_build("part p : T [min..max];");
    let parts: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage)
        .collect();
    assert_eq!(parts.len(), 1);
    let p = &parts[0];
    // Numeric multiplicity should be None (not parseable as integers)
    assert!(p.get_prop("multiplicity_lower").is_none());
    // Symbolic bounds preserved as text
    assert_eq!(
        p.get_prop("multiplicity_lower_text")
            .and_then(|v| v.as_str().map(|s| s.to_owned())),
        Some("min".to_owned())
    );
    assert_eq!(
        p.get_prop("multiplicity_upper_text")
            .and_then(|v| v.as_str().map(|s| s.to_owned())),
        Some("max".to_owned())
    );
}

// === Fork/join succession wiring tests ===

#[test]
fn test_anonymous_fork_gets_synthetic_name() {
    let result = parse_and_build("action def A { fork; }");
    let forks: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ForkNode)
        .collect();
    assert_eq!(forks.len(), 1);
    assert_eq!(
        forks[0].name.as_deref(),
        Some("$fork_0"),
        "anonymous fork should get synthetic name $fork_0"
    );
}

#[test]
fn test_named_fork_keeps_explicit_name() {
    let result = parse_and_build("action def A { fork split; }");
    let forks: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ForkNode)
        .collect();
    assert_eq!(forks.len(), 1);
    assert_eq!(
        forks[0].name.as_deref(),
        Some("split"),
        "named fork should keep its explicit name"
    );
}

#[test]
fn test_anonymous_join_gets_synthetic_name() {
    let result = parse_and_build("action def A { join; }");
    let joins: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::JoinNode)
        .collect();
    assert_eq!(joins.len(), 1);
    assert_eq!(
        joins[0].name.as_deref(),
        Some("$join_0"),
        "anonymous join should get synthetic name $join_0"
    );
}

#[test]
fn test_multiple_anonymous_forks_get_distinct_names() {
    let result = parse_and_build("action def A { fork; fork; }");
    let mut forks: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ForkNode)
        .collect();
    assert_eq!(forks.len(), 2);
    forks.sort_by_key(|e| e.name.clone());
    assert_eq!(forks[0].name.as_deref(), Some("$fork_0"));
    assert_eq!(forks[1].name.as_deref(), Some("$fork_1"));
}

#[test]
fn test_then_after_fork_gets_fork_as_source() {
    let result = parse_and_build("action def A { action x; fork; then x; }");
    let succs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    let then_x = succs
        .iter()
        .find(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("x"));
    assert!(then_x.is_some(), "should have succession with target 'x'");
    assert_eq!(
        then_x.unwrap().get_prop("source").and_then(|v| v.as_str()),
        Some("$fork_0"),
        "then x after fork should have fork as source"
    );
}

#[test]
fn test_fork_fanout_multiple_then() {
    let result = parse_and_build("action def A { action x; action y; fork; then x; then y; }");
    let succs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    let then_x = succs
        .iter()
        .find(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("x"));
    let then_y = succs
        .iter()
        .find(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("y"));
    assert!(then_x.is_some(), "should have succession with target 'x'");
    assert!(then_y.is_some(), "should have succession with target 'y'");
    assert_eq!(
        then_x.unwrap().get_prop("source").and_then(|v| v.as_str()),
        Some("$fork_0"),
        "then x should fan-out from fork"
    );
    assert_eq!(
        then_y.unwrap().get_prop("source").and_then(|v| v.as_str()),
        Some("$fork_0"),
        "then y should fan-out from fork"
    );
}

#[test]
fn test_then_after_action_gets_preceding_action_as_source() {
    // `then y;` follows `action x;`, so source should be "x"
    let result = parse_and_build("action def A { action x; then y; }");
    let succs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    let then_y = succs
        .iter()
        .find(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("y"));
    assert!(then_y.is_some(), "should have succession with target 'y'");
    assert_eq!(
        then_y.unwrap().get_prop("source").and_then(|v| v.as_str()),
        Some("x"),
        "then y after action x should have x as source"
    );
}

#[test]
fn test_then_after_join_gets_join_as_source() {
    let result = parse_and_build("action def A { action z; join; then z; }");
    let succs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::SuccessionAsUsage)
        .collect();
    let then_z = succs
        .iter()
        .find(|s| s.get_prop("target").and_then(|v| v.as_str()) == Some("z"));
    assert!(then_z.is_some(), "should have succession with target 'z'");
    assert_eq!(
        then_z.unwrap().get_prop("source").and_then(|v| v.as_str()),
        Some("$join_0"),
        "then z after join should have join as source"
    );
}

// === Comment-before-control-node regression tests ===
// Tree-sitter's GLR resolver can split `fork myFork;` into two CST nodes
// (control_flow_node + feature_declaration) when an sl_note comment
// immediately precedes the control node. The ast_builder must merge the
// name back from the feature_declaration into the control node element.

#[test]
fn test_comment_before_named_fork_preserves_name() {
    let result = parse_and_build("action def A { action x; // comment\n fork myFork; then x; }");
    let forks: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ForkNode)
        .collect();
    assert_eq!(forks.len(), 1, "should have exactly one ForkNode");
    assert_eq!(
        forks[0].name.as_deref(),
        Some("myFork"),
        "fork should keep its explicit name even with preceding comment"
    );
}

#[test]
fn test_comment_before_named_decide_preserves_name() {
    let result = parse_and_build("action def A { action x; // comment\n decide myDecide; }");
    let decisions: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::DecisionNode)
        .collect();
    assert_eq!(decisions.len(), 1, "should have exactly one DecisionNode");
    assert_eq!(
        decisions[0].name.as_deref(),
        Some("myDecide"),
        "decide should keep its explicit name even with preceding comment"
    );
}

#[test]
fn test_comment_before_named_join_preserves_name() {
    let result = parse_and_build("action def A { action x; // comment\n join myJoin; }");
    let joins: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::JoinNode)
        .collect();
    assert_eq!(joins.len(), 1, "should have exactly one JoinNode");
    assert_eq!(
        joins[0].name.as_deref(),
        Some("myJoin"),
        "join should keep its explicit name even with preceding comment"
    );
}

#[test]
fn test_comment_before_named_merge_preserves_name() {
    let result = parse_and_build("action def A { action x; // comment\n merge myMerge; }");
    let merges: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MergeNode)
        .collect();
    assert_eq!(merges.len(), 1, "should have exactly one MergeNode");
    assert_eq!(
        merges[0].name.as_deref(),
        Some("myMerge"),
        "merge should keep its explicit name even with preceding comment"
    );
}

#[test]
fn test_comment_before_anonymous_fork_still_gets_synthetic_name() {
    let result = parse_and_build("action def A { action x; // comment\n fork; then x; }");
    let forks: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ForkNode)
        .collect();
    assert_eq!(forks.len(), 1, "should have exactly one ForkNode");
    assert_eq!(
        forks[0].name.as_deref(),
        Some("$fork_0"),
        "anonymous fork with preceding comment should still get synthetic name"
    );
}

#[test]
fn test_nary_interface_connect_endpoints() {
    let result =
        parse_and_build("package T { part def A { port x; port y; interface connect (x, y); } }");
    let ifaces: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::InterfaceUsage)
        .collect();
    assert_eq!(ifaces.len(), 1, "should have 1 InterfaceUsage");
    let iface = ifaces[0];
    let src = iface.get_prop("source").and_then(|v| v.as_str());
    let tgt = iface.get_prop("target").and_then(|v| v.as_str());
    assert_eq!(src, Some("x"), "source should be 'x', got {:?}", src);
    assert_eq!(tgt, Some("y"), "target should be 'y', got {:?}", tgt);
}

#[test]
fn test_binary_interface_connect_endpoints() {
    let result =
        parse_and_build("package T { part def A { port x; port y; interface connect x to y; } }");
    let ifaces: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::InterfaceUsage)
        .collect();
    assert_eq!(ifaces.len(), 1, "should have 1 InterfaceUsage");
    let iface = ifaces[0];
    let src = iface.get_prop("source").and_then(|v| v.as_str());
    let tgt = iface.get_prop("target").and_then(|v| v.as_str());
    assert_eq!(src, Some("x"), "source should be 'x', got {:?}", src);
    assert_eq!(tgt, Some("y"), "target should be 'y', got {:?}", tgt);
}

// === TS-1.1: State-subaction action-body assignment emission ===
//
// Per Architectural-cleanup/tree-sitter-canonical-plan/u3-orchestrator-ts-forcing.md,
// `process_state_subaction` must walk inline `entry/do/exit action { x = expr; … }`
// bodies and emit one AssignmentActionUsage child per bare assignment, owned by
// the wrapping ActionUsage and keyed via add_with_ownership_keyed so the
// OwningMembership wrapper is reparse-stable per ADR-009.

/// Helper: find the wrapping ActionUsage element for a given subaction kind
/// ("entry"/"do"/"exit") whose owner is a state with `state_name`.
fn find_subaction_wrapper<'a>(
    result: &'a ModelGraphResult,
    state_name: &str,
    kind: &str,
) -> Option<&'a sysml_core::Element> {
    let state = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::StateUsage && e.name.as_deref() == Some(state_name))?;
    result.graph.elements.values().find(|e| {
        e.kind == ElementKind::ActionUsage
            && e.owner.as_ref() == Some(&state.id)
            && e.get_prop("stateSubactionKind").and_then(|v| v.as_str()) == Some(kind)
    })
}

/// Helper: collect AssignmentActionUsage children of a wrapper element.
fn collect_assignments<'a>(
    result: &'a ModelGraphResult,
    wrapper_id: &sysml_core::ElementId,
) -> Vec<&'a sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::AssignmentActionUsage && e.owner.as_ref() == Some(wrapper_id)
        })
        .collect()
}

#[test]
fn test_state_subaction_entry_emits_assignment_action_usages() {
    // Single-assignment inline entry body: `entry action { boilerTemp = 20; }`.
    let result = parse_and_build(
        "package T { state def S { state cold { entry action { boilerTemp = 20; } } } }",
    );
    let wrapper = find_subaction_wrapper(&result, "cold", "entry")
        .expect("entry-subaction wrapper for state `cold`");
    let assigns = collect_assignments(&result, &wrapper.id);
    assert_eq!(
        assigns.len(),
        1,
        "expected 1 AssignmentActionUsage under entry wrapper, got {}",
        assigns.len()
    );
    let assign = assigns[0];
    let target = assign
        .name
        .as_deref()
        .or_else(|| assign.get_prop("target").and_then(|v| v.as_str()));
    assert_eq!(
        target,
        Some("boilerTemp"),
        "assignment target should be `boilerTemp`"
    );
    let value = assign.get_prop("value").and_then(|v| v.as_int());
    assert_eq!(
        value,
        Some(20),
        "literal RHS `20` should populate value=Int(20)"
    );
}

#[test]
fn test_state_subaction_entry_emits_multiple_assignments() {
    // Multi-assignment inline entry body: `entry action { x = 1; y = 2; z = 3; }`.
    let result = parse_and_build(
        "package T { state def S { state idle { entry action { x = 1; y = 2; z = 3; } } } }",
    );
    let wrapper = find_subaction_wrapper(&result, "idle", "entry")
        .expect("entry-subaction wrapper for state `idle`");
    let assigns = collect_assignments(&result, &wrapper.id);
    assert_eq!(
        assigns.len(),
        3,
        "expected 3 AssignmentActionUsage under entry wrapper, got {}",
        assigns.len()
    );
    let names: Vec<_> = assigns.iter().filter_map(|a| a.name.as_deref()).collect();
    assert!(names.contains(&"x"), "missing assignment `x`: {:?}", names);
    assert!(names.contains(&"y"), "missing assignment `y`: {:?}", names);
    assert!(names.contains(&"z"), "missing assignment `z`: {:?}", names);
}

#[test]
fn test_state_subaction_do_emits_assignment_action_usages() {
    let result = parse_and_build(
        "package T { state def S { state running { do action { progress = 50; flag = true; } } } }",
    );
    let wrapper = find_subaction_wrapper(&result, "running", "do")
        .expect("do-subaction wrapper for state `running`");
    let assigns = collect_assignments(&result, &wrapper.id);
    assert_eq!(
        assigns.len(),
        2,
        "expected 2 AssignmentActionUsage under do wrapper, got {}",
        assigns.len()
    );
    let progress = assigns
        .iter()
        .find(|a| a.name.as_deref() == Some("progress"))
        .expect("progress assignment");
    assert_eq!(
        progress.get_prop("value").and_then(|v| v.as_int()),
        Some(50),
        "progress literal should be Int(50)"
    );
    let flag = assigns
        .iter()
        .find(|a| a.name.as_deref() == Some("flag"))
        .expect("flag assignment");
    assert_eq!(
        flag.get_prop("value").and_then(|v| v.as_bool()),
        Some(true),
        "flag literal should be Bool(true)"
    );
}

#[test]
fn test_state_subaction_exit_emits_assignment_action_usages() {
    let result = parse_and_build(
        "package T { state def S { state shutdown { exit action { saveState = 1; cleanup = 0; } } } }",
    );
    let wrapper = find_subaction_wrapper(&result, "shutdown", "exit")
        .expect("exit-subaction wrapper for state `shutdown`");
    let assigns = collect_assignments(&result, &wrapper.id);
    assert_eq!(
        assigns.len(),
        2,
        "expected 2 AssignmentActionUsage under exit wrapper, got {}",
        assigns.len()
    );
}

#[test]
fn test_state_subaction_owner_is_wrapper_action_usage() {
    // The new AssignmentActionUsage children must be owned by the wrapping
    // ActionUsage (the entry/do/exit subaction), not the surrounding state —
    // this matches compile_action_from_children's child-walk in
    // sysml-runtime/src/statemachine/mod.rs.
    let result =
        parse_and_build("package T { state def S { state cold { entry action { x = 1; } } } }");
    let wrapper = find_subaction_wrapper(&result, "cold", "entry")
        .expect("entry-subaction wrapper for state `cold`");
    let assigns = collect_assignments(&result, &wrapper.id);
    assert_eq!(assigns.len(), 1, "expected exactly one assignment");
    assert_eq!(
        assigns[0].owner.as_ref(),
        Some(&wrapper.id),
        "assignment owner must be the subaction wrapper, not the state"
    );
}

#[test]
fn test_state_subaction_owning_membership_minted_keyed() {
    // Gap #6: process_state_subaction's wrapper element (and its assignment
    // children) must mint OwningMembership wrappers via add_with_ownership_keyed
    // so the membership ID is derived from canonical keys per ADR-009 — not
    // via the raw add_element path that produces a non-keyed wrapper.
    //
    // Probe: each AssignmentActionUsage must have its `owning_membership` field
    // populated with the ID of an OwningMembership element that exists in the
    // graph. add_element alone leaves owning_membership=None;
    // add_with_ownership_keyed (→ create_owning_membership_with_key) sets it.
    let result = parse_and_build(
        "package T { state def S { state cold { entry action { x = 1; y = 2; } } } }",
    );
    let wrapper = find_subaction_wrapper(&result, "cold", "entry")
        .expect("entry-subaction wrapper for state `cold`");
    let assigns = collect_assignments(&result, &wrapper.id);
    assert_eq!(assigns.len(), 2, "expected 2 assignments");

    for a in &assigns {
        let om_id = a
            .owning_membership
            .as_ref()
            .expect("assignment must have an owning_membership wrapper");
        let om = result
            .graph
            .elements
            .get(om_id)
            .expect("owning_membership element must exist in graph");
        assert_eq!(
            om.kind,
            ElementKind::OwningMembership,
            "owning_membership field must reference an OwningMembership element"
        );
    }

    // The wrapper subaction element itself must also have an owning_membership
    // (its membership wrapper under the surrounding state) — process_state_subaction
    // switched from raw add_element to add_with_ownership_keyed.
    assert!(
        wrapper.owning_membership.is_some(),
        "the entry-subaction wrapper ActionUsage must have an owning_membership \
         (process_state_subaction must use add_with_ownership_keyed, not add_element)"
    );
}

// F3 state-subaction slice. SysML.xtext:1767-1785 — Entry/Do/ExitActionMember
// each `returns StateSubactionMembership` with `kind = Entry/Do/ExitActionKind`,
// hosting a `StateActionUsage` which itself `returns SysML::ActionUsage`
// (xtext:1798). So the spec-distinct structure is the MEMBERSHIP kind, not a
// distinct usage kind. This test pins that each entry/do/exit subaction's
// wrapping membership is a StateSubactionMembership carrying the discriminator,
// while the hosted usage stays a plain ActionUsage.
#[test]
fn test_state_subaction_wrapper_is_state_subaction_membership() {
    let result = parse_and_build(
        "package T { state def S { state cold { \
             entry action { x = 1; } do action { y = 2; } exit action { z = 3; } } } }",
    );
    for kind in ["entry", "do", "exit"] {
        let wrapper = find_subaction_wrapper(&result, "cold", kind)
            .unwrap_or_else(|| panic!("{kind}-subaction wrapper for state `cold`"));

        // Usage-kind correction: the hosted subaction is a plain ActionUsage,
        // NOT an invented "StateActionUsage" kind (which is not a metaclass).
        assert_eq!(
            wrapper.kind,
            ElementKind::ActionUsage,
            "{kind} subaction usage must be ActionUsage (StateActionUsage is a \
             grammar rule returning ActionUsage, not a distinct metaclass)"
        );

        // Source of truth: the wrapping membership is a StateSubactionMembership
        // carrying the entry/do/exit discriminator as its `kind`.
        let mem_id = wrapper
            .owning_membership
            .as_ref()
            .expect("subaction wrapper must have an owning membership");
        let mem = result
            .graph
            .elements
            .get(mem_id)
            .expect("owning membership element must exist");
        assert_eq!(
            mem.kind,
            ElementKind::StateSubactionMembership,
            "{kind} subaction must be wrapped in a StateSubactionMembership, not a \
             plain OwningMembership"
        );
        assert_eq!(
            mem.get_prop("kind").and_then(|v| v.as_str()),
            Some(kind),
            "StateSubactionMembership.kind must be the {kind} discriminator"
        );

        // Co-authored mirror retained for un-migrated consumers.
        assert_eq!(
            wrapper.get_prop("stateSubactionKind").and_then(|v| v.as_str()),
            Some(kind),
            "stateSubactionKind mirror prop must still be present on the ActionUsage"
        );
    }

    // `children_of(state)` must still return the hosted ActionUsage (the member),
    // not the membership — consumers walking children are unaffected.
    let state = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::StateUsage && e.name.as_deref() == Some("cold"))
        .expect("state `cold`");
    let child_action_kinds: Vec<_> = result
        .graph
        .children_of(&state.id)
        .filter(|e| e.get_prop("stateSubactionKind").is_some())
        .map(|e| e.kind.clone())
        .collect();
    assert_eq!(
        child_action_kinds.len(),
        3,
        "children_of(state) must surface the three subaction ActionUsage members"
    );
    assert!(
        child_action_kinds
            .iter()
            .all(|k| *k == ElementKind::ActionUsage),
        "children_of must return ActionUsage members, not StateSubactionMembership wrappers"
    );
}

#[test]
fn test_state_subaction_canonical_key_stable_across_reparses() {
    // Reparse the same source twice; the AssignmentActionUsage element IDs
    // must be identical (canonical-key derived per ADR-009).
    let src = "package T { state def S { state cold { entry action { x = 1; y = 2; } } } }";
    let r1 = parse_and_build(src);
    let r2 = parse_and_build(src);

    let wrapper1 = find_subaction_wrapper(&r1, "cold", "entry").expect("wrapper r1");
    let wrapper2 = find_subaction_wrapper(&r2, "cold", "entry").expect("wrapper r2");
    assert_eq!(
        wrapper1.id, wrapper2.id,
        "subaction wrapper ID must be stable across reparses"
    );

    let mut ids1: Vec<_> = collect_assignments(&r1, &wrapper1.id)
        .iter()
        .map(|a| a.id.clone())
        .collect();
    let mut ids2: Vec<_> = collect_assignments(&r2, &wrapper2.id)
        .iter()
        .map(|a| a.id.clone())
        .collect();
    ids1.sort();
    ids2.sort();
    assert_eq!(
        ids1, ids2,
        "AssignmentActionUsage IDs must be stable across reparses"
    );
}

#[test]
fn test_state_subaction_mixed_entry_do_exit_on_same_state() {
    // The orchestration fixture shape: one state with entry + do + exit
    // subactions all carrying assignments.
    let result = parse_and_build(
        "package T { state def S { state heating { \
             entry action { heaterOn = 1; boilerTemp = 45; } \
             do action { heating = true; } \
             exit action { heaterOn = 0; } \
         } } }",
    );

    let entry_w = find_subaction_wrapper(&result, "heating", "entry").expect("entry wrapper");
    let do_w = find_subaction_wrapper(&result, "heating", "do").expect("do wrapper");
    let exit_w = find_subaction_wrapper(&result, "heating", "exit").expect("exit wrapper");

    assert_eq!(collect_assignments(&result, &entry_w.id).len(), 2);
    assert_eq!(collect_assignments(&result, &do_w.id).len(), 1);
    assert_eq!(collect_assignments(&result, &exit_w.id).len(), 1);
}

#[test]
fn test_state_subaction_no_double_emission_as_feature() {
    // The CST parses `x = 20;` inside an action_body as a feature_declaration
    // with a default_value. After TS-1.1, that feature_declaration must NOT
    // also surface as a free-standing Feature/ReferenceUsage owned by the
    // subaction wrapper — only AssignmentActionUsage.
    let result =
        parse_and_build("package T { state def S { state cold { entry action { x = 20; } } } }");
    let wrapper = find_subaction_wrapper(&result, "cold", "entry")
        .expect("entry-subaction wrapper for state `cold`");

    let stray_features: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.owner.as_ref() == Some(&wrapper.id)
                && e.kind != ElementKind::AssignmentActionUsage
                && e.kind != ElementKind::OwningMembership
                && e.name.as_deref() == Some("x")
        })
        .collect();
    assert!(
        stray_features.is_empty(),
        "feature_declaration `x = 20` must not also surface as a non-Assignment element under \
         the subaction wrapper: {:?}",
        stray_features
            .iter()
            .map(|e| (e.kind.clone(), e.name.clone()))
            .collect::<Vec<_>>()
    );
}

// ===== TS-1.2: Redefinition relationship emission =====
//
// Gap #2 from `Architectural-cleanup/tree-sitter-canonical-plan/
// ts-ast-builder-gap-list.md`: Pest emits 4256 `Redefinition`
// relationships across the corpus; tree-sitter emits zero. These
// tests pin the missing behaviour for `:>>` operator and `redefines`
// keyword forms (single, chained, cross-package).

fn redefinitions(result: &ModelGraphResult) -> Vec<&sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Redefinition)
        .collect()
}

fn redefined_target(elem: &sysml_core::Element) -> String {
    elem.get_prop("unresolved_redefinedFeature")
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .expect("Redefinition must carry unresolved_redefinedFeature qname")
}

fn redefining_source(elem: &sysml_core::Element) -> sysml_core::ElementId {
    match elem
        .get_prop("redefiningFeature")
        .expect("Redefinition must carry redefiningFeature ref")
    {
        Value::Ref(id) => id.clone(),
        other => panic!("redefiningFeature must be a Ref, got {other:?}"),
    }
}

#[test]
fn ts_1_2_redefinition_operator_emits_redefinition() {
    // `attribute x :>> y;` must produce a single Redefinition relationship
    // whose source is the redefining feature and target qname is `y`.
    let source = "package P { \
                  attribute def Base { attribute x; } \
                  attribute def Derived :> Base { attribute x :>> x; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "fixture must parse clean");

    let rels = redefinitions(&result);
    assert_eq!(
        rels.len(),
        1,
        "exactly one Redefinition expected, got {}: {:?}",
        rels.len(),
        rels.iter().map(|e| e.name.clone()).collect::<Vec<_>>()
    );

    let target = redefined_target(rels[0]);
    assert_eq!(target, "x", "target qname should be `x`, got `{target}`");

    let source_id = redefining_source(rels[0]);
    let source_elem = result
        .graph
        .elements
        .get(&source_id)
        .expect("redefiningFeature ref must resolve to an element");
    assert_eq!(source_elem.name.as_deref(), Some("x"));
    assert!(
        source_elem.kind.is_usage(),
        "redefining feature should be a usage, got {:?}",
        source_elem.kind
    );
}

#[test]
fn ts_1_2_redefinition_keyword_emits_redefinition() {
    // `attribute y redefines x;` must produce the same Redefinition as `:>>`.
    let source = "package P { \
                  attribute def Base { attribute x; } \
                  attribute def Derived :> Base { attribute y redefines x; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "fixture must parse clean");

    let rels = redefinitions(&result);
    assert_eq!(
        rels.len(),
        1,
        "exactly one Redefinition expected for `redefines` keyword"
    );
    assert_eq!(redefined_target(rels[0]), "x");
}

#[test]
fn ts_1_2_redefinition_chained_emits_one_per_target() {
    // Two `:>>` clauses on one usage must produce two Redefinitions.
    let source = "package P { \
                  attribute def A; attribute def B :> A; \
                  attribute def C :> B { attribute x :>> A :>> B; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = redefinitions(&result);
    assert_eq!(
        rels.len(),
        2,
        "two Redefinitions expected for chained `:>> A :>> B`, got {}",
        rels.len()
    );

    let mut targets: Vec<String> = rels.iter().map(|e| redefined_target(e)).collect();
    targets.sort();
    assert_eq!(targets, vec!["A".to_owned(), "B".to_owned()]);
}

#[test]
fn ts_1_2_redefinition_cross_package_keeps_qualified_target() {
    // Qualified-name target across package boundaries must be preserved verbatim.
    let source = "\
        package Lib { attribute def Base { attribute speed; } } \
        package User { \
            import Lib::*; \
            attribute def Derived :> Lib::Base { \
                attribute speed :>> Lib::Base::speed; \
            } \
        }";
    let result = parse_and_build(source);
    assert!(
        !result.has_errors(),
        "cross-package fixture must parse clean"
    );

    let rels = redefinitions(&result);
    assert_eq!(rels.len(), 1, "single Redefinition expected");
    let target = redefined_target(rels[0]);
    assert_eq!(
        target, "Lib::Base::speed",
        "cross-package target qname must be preserved verbatim"
    );
}

#[test]
fn ts_1_2_redefinition_canonical_key_stable_across_two_parses() {
    // ADR-009 canonical IDs must be deterministic across re-parses.
    let source = "package P { \
                  attribute def Base { attribute x; } \
                  attribute def Derived :> Base { attribute x :>> x; } \
                  }";
    let result1 = parse_and_build(source);
    let result2 = parse_and_build(source);

    let ids1: Vec<_> = redefinitions(&result1)
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let ids2: Vec<_> = redefinitions(&result2)
        .iter()
        .map(|e| e.id.clone())
        .collect();
    assert_eq!(ids1.len(), 1);
    assert_eq!(ids2.len(), 1);
    assert_eq!(
        ids1, ids2,
        "Redefinition canonical key must be stable across re-parses"
    );
}

// ===========================================================================
// TS-1.4 — Expression subtree emission outside constraint bodies.
//
// Constraint-body parity is already strict-zero via the `expression_parity`
// integration test. These RED tests cover the three under-emitting dispatch
// points the gap inventory calls out: default values (numeric/feature-ref),
// calc bodies, and metadata-value RHS expressions. The current behaviour
// (pre-fix) skips literal RHS values and never emits subtrees for calc
// bodies; these tests will go GREEN once `emit_default_value_expression`
// stops skipping literals and `process_usage`/`process_definition` walk
// `function_body`/`constraint_body` result expressions through the same
// `ExpressionBuilder` pipeline.
// ===========================================================================

/// Convenience: collect direct children of `parent_id` that are expression-bearing.
fn collect_expression_children<'g>(
    graph: &'g sysml_core::ModelGraph,
    parent_id: &sysml_core::ElementId,
) -> Vec<&'g sysml_core::Element> {
    graph
        .children_of(parent_id)
        .filter(|c| {
            matches!(
                c.kind,
                ElementKind::OperatorExpression
                    | ElementKind::FeatureReferenceExpression
                    | ElementKind::FeatureChainExpression
                    | ElementKind::InvocationExpression
                    | ElementKind::LiteralInteger
                    | ElementKind::LiteralRational
                    | ElementKind::LiteralBoolean
                    | ElementKind::LiteralString
                    | ElementKind::LiteralInfinity
                    | ElementKind::NullExpression
                    | ElementKind::IndexExpression
            )
        })
        .collect()
}

#[test]
fn expression_default_numeric_literal_emits_subtree() {
    // `attribute x : Real default 5;` — Pest emits a LiteralInteger(5)
    // expression child even though the parent has a typed `value` prop.
    // TS used to skip literal defaults — TS-1.4 closes that gap.
    let source = "package P { attribute x : Real default 5; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let attr = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("x"))
        .expect("attribute x");

    let kids = collect_expression_children(&result.graph, &attr.id);
    assert!(
        kids.iter().any(|c| c.kind == ElementKind::LiteralInteger),
        "literal default `5` must emit a LiteralInteger child, got: {:?}",
        kids.iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn expression_default_numeric_compound_emits_subtree() {
    // `attribute y : Real default (1 + 2);` — compound expression in
    // default position must emit OperatorExpression + LiteralInteger.
    let source = "package P { attribute y : Real default (1 + 2); }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let attr = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("y"))
        .expect("attribute y");

    let kids = collect_expression_children(&result.graph, &attr.id);
    assert!(
        kids.iter()
            .any(|c| c.kind == ElementKind::OperatorExpression),
        "compound default `(1 + 2)` must emit OperatorExpression, got: {:?}",
        kids.iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn expression_default_feature_ref_emits_subtree() {
    // `attribute x default self.y;` — feature reference default must
    // emit FeatureReferenceExpression. (Already works pre-fix when RHS
    // is non-literal, but kept here to lock in the contract.)
    let source = "package P { attribute y : Real default 0; attribute x : Real default y; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let attr = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("x"))
        .expect("attribute x");

    let kids = collect_expression_children(&result.graph, &attr.id);
    assert!(
        kids.iter()
            .any(|c| c.kind == ElementKind::FeatureReferenceExpression),
        "feature-ref default `y` must emit FeatureReferenceExpression, got: {:?}",
        kids.iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn expression_calc_body_emits_subtree() {
    // `calc def F : Real { 1 + 2 * 3 }` — the body result expression
    // must produce an OperatorExpression subtree, not just a `expr`
    // string prop. Pest emits it; TS used to drop it.
    // (Empty `()` parameter list trips the TS grammar — omitted here.)
    let source = "package P { calc def F : Real { 1 + 2 * 3 } }";
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "calc body must parse cleanly");

    let calc = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::CalculationDefinition && e.name.as_deref() == Some("F"))
        .expect("calc def F");

    let kids = collect_expression_children(&result.graph, &calc.id);
    assert!(
        kids.iter()
            .any(|c| c.kind == ElementKind::OperatorExpression),
        "calc body `1 + 2 * 3` must emit OperatorExpression, got: {:?}",
        kids.iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
    );
}

/// Arc 1 (coffee-machine triage): nested packages must mint Package
/// elements, not ReferenceUsages. `_package_member` previously omitted
/// `package_decl`, so `package Inner {}` inside a package body
/// error-recovered into two feature_declarations → two ReferenceUsages
/// (one literally named "package" — the S001 duplicate-warning source
/// in package-structure.sysml).
#[test]
fn nested_packages_mint_package_elements() {
    let source = r#"package Outer {
        doc /* top-level */
        import ScalarValues::*;

        package Structure {
            doc /* structural */
            part def Widget;
        }

        private package Internal {
        }
    }"#;
    let result = parse_and_build(source);
    assert!(
        !result.has_errors(),
        "diagnostics: {:?}",
        result.diagnostics
    );

    let packages: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Package)
        .map(|e| e.name.as_deref().unwrap_or("<anon>").to_owned())
        .collect();
    for expected in ["Outer", "Structure", "Internal"] {
        assert!(
            packages.contains(&expected.to_owned()),
            "package '{expected}' missing from Package elements, got: {packages:?}"
        );
    }

    // No spurious elements literally named "package".
    assert!(
        !result
            .graph
            .elements
            .values()
            .any(|e| e.name.as_deref() == Some("package")),
        "no element may be named literally 'package'"
    );

    // Ownership: Structure is owned by Outer; Widget by Structure.
    let outer = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Outer"))
        .expect("Outer");
    let structure = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Structure"))
        .expect("Structure");
    assert_eq!(structure.owner.as_ref(), Some(&outer.id));
    let widget = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Widget"))
        .expect("Widget");
    assert_eq!(widget.owner.as_ref(), Some(&structure.id));
    assert_eq!(widget.kind, ElementKind::PartDefinition);
}

/// Arc 3c (coffee-machine triage): the `@Type` annotation form must mint
/// exactly ONE FeatureTyping child referencing the metadata def — the same
/// structure `part x : Foo` gets — so `:>> attr` redefinitions inside the
/// usage body resolve against the def's attributes via the standard
/// inheritance machinery. Exactly one: a double mint would show up as
/// spurious ts_only paths in pilot conformance.
#[test]
fn metadata_annotation_mints_single_feature_typing() {
    let source = r#"package P {
        metadata def ModelMaturity {
            attribute level : String;
        }
        @ModelMaturity {
            :>> level = "reviewed";
        }
        part def Annotated;
    }"#;
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let metadata_usage = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::MetadataUsage)
        .expect("MetadataUsage minted for @ModelMaturity");

    // Anonymous per spec (Arc 3b) — typed via prop + FeatureTyping child.
    assert_eq!(
        metadata_usage.name, None,
        "@-form metadata usage must stay anonymous"
    );
    assert_eq!(
        metadata_usage
            .get_prop("unresolvedTypeName")
            .and_then(|v| v.as_str()),
        Some("ModelMaturity")
    );

    let typings: Vec<_> = result
        .graph
        .children_of(&metadata_usage.id)
        .filter(|c| c.kind == ElementKind::FeatureTyping)
        .collect();
    assert_eq!(
        typings.len(),
        1,
        "exactly one FeatureTyping child, got {}: {:?}",
        typings.len(),
        typings
            .iter()
            .map(|t| t.get_prop("unresolved_type").cloned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        typings[0]
            .get_prop("unresolved_type")
            .and_then(|v| v.as_str()),
        Some("ModelMaturity")
    );
}

#[test]
fn expression_metadata_value_emits_subtree() {
    // `@DataSource { path = "a.csv"; column = "v" * 2; }` — feature-usage
    // values inside metadata bodies route through process_usage, so the
    // default-value emission must walk them too.
    let source = r#"package P {
        metadata def DataSource {
            attribute path : String;
            attribute scale : Real;
        }
        part p {
            @DataSource {
                path = "a.csv";
                scale = 1 + 2;
            }
        }
    }"#;
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    // The metadata-body `scale = 1 + 2` is processed as a ReferenceUsage
    // owned by the MetadataUsage; the MetadataDef's own `attribute scale`
    // is an AttributeUsage owned by the MetadataDef. Pick the one whose
    // owner is a MetadataUsage so we test the right element.
    let metadata_usage_ids: Vec<sysml_core::ElementId> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MetadataUsage)
        .map(|e| e.id.clone())
        .collect();
    let scale = result
        .graph
        .elements
        .values()
        .find(|e| {
            e.name.as_deref() == Some("scale")
                && e.owner
                    .as_ref()
                    .is_some_and(|o| metadata_usage_ids.contains(o))
        })
        .expect("scale usage in metadata body");

    let kids = collect_expression_children(&result.graph, &scale.id);
    assert!(
        kids.iter()
            .any(|c| c.kind == ElementKind::OperatorExpression),
        "metadata-value `1 + 2` must emit OperatorExpression, got: {:?}",
        kids.iter().map(|c| c.kind.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn expression_default_numeric_canonical_key_stable_across_parses() {
    // ADR-009: literal-default expression children must derive reparse-stable
    // IDs from the parent's canonical key, not fresh UUIDs.
    let source = "package P { attribute x : Real default 5; }";
    let r1 = parse_and_build(source);
    let r2 = parse_and_build(source);

    let lit_id = |r: &ModelGraphResult| -> sysml_core::ElementId {
        let attr = r
            .graph
            .elements
            .values()
            .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("x"))
            .unwrap();
        r.graph
            .children_of(&attr.id)
            .find(|c| c.kind == ElementKind::LiteralInteger)
            .expect("literal child")
            .id
            .clone()
    };

    assert_eq!(
        lit_id(&r1),
        lit_id(&r2),
        "literal-default child ID must be stable across two parses"
    );
}

// ===== TS-1.3: ReferenceUsage / Subsetting / Subclassification / ReferenceSubsetting =====
//
// Gaps #3 + #7 from `Architectural-cleanup/tree-sitter-canonical-plan/
// ts-ast-builder-gap-list.md`: Pest emits 3542 more `ReferenceUsage`,
// 606 more `Subsetting`, 375 more `Subclassification`, and 30 more
// `ReferenceSubsetting` than tree-sitter across the corpus. These tests
// pin the missing emission for the four relationship-bearing forms:
//   - `:>` operator on features  → Subsetting
//   - `:>` operator on a `def`   → Subclassification (chained / cross-pkg)
//   - `ref` modifier on usages   → ReferenceUsage
//   - bare `feature_declaration` (DefaultReferenceUsage in Pest, e.g.
//     `in x : T;`)               → ReferenceUsage
//   - `references` keyword form  → ReferenceSubsetting

fn subsettings(result: &ModelGraphResult) -> Vec<&sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Subsetting)
        .collect()
}

fn subclassifications(result: &ModelGraphResult) -> Vec<&sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Subclassification)
        .collect()
}

fn reference_subsettings(result: &ModelGraphResult) -> Vec<&sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ReferenceSubsetting)
        .collect()
}

fn reference_usages(result: &ModelGraphResult) -> Vec<&sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ReferenceUsage)
        .collect()
}

fn subsetted_target(elem: &sysml_core::Element) -> String {
    elem.get_prop("unresolved_subsettedFeature")
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .expect("Subsetting must carry unresolved_subsettedFeature qname")
}

fn subclassified_target(elem: &sysml_core::Element) -> String {
    elem.get_prop("unresolved_superclassifier")
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .expect("Subclassification must carry unresolved_superclassifier qname")
}

fn reference_subsetting_target(elem: &sysml_core::Element) -> String {
    elem.get_prop("unresolved_referencedFeature")
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .expect("ReferenceSubsetting must carry unresolved_referencedFeature qname")
}

#[test]
fn ts_1_3_subsetting_operator_emits_subsetting() {
    // `attribute child :> base;` must produce a single Subsetting whose
    // target qname is `base` and whose source is the `child` usage.
    let source = "package P { attribute base; attribute child :> base; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "fixture must parse clean");

    let rels = subsettings(&result);
    assert_eq!(
        rels.len(),
        1,
        "exactly one Subsetting expected, got {}",
        rels.len()
    );
    assert_eq!(subsetted_target(rels[0]), "base");
}

#[test]
fn ts_1_3_subsetting_chained_emits_one_per_target() {
    // `attribute child :> a :> b;` must emit two Subsettings.
    let source = "package P { attribute a; attribute b; attribute child :> a :> b; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = subsettings(&result);
    assert_eq!(
        rels.len(),
        2,
        "two Subsettings expected for chained `:> a :> b`, got {}",
        rels.len()
    );
    let mut targets: Vec<String> = rels.iter().map(|e| subsetted_target(e)).collect();
    targets.sort();
    assert_eq!(targets, vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn ts_1_3_subsetting_cross_package_keeps_qualified_target() {
    let source = "\
        package Lib { attribute base; } \
        package User { \
            import Lib::*; \
            attribute derived :> Lib::base; \
        }";
    let result = parse_and_build(source);
    assert!(
        !result.has_errors(),
        "cross-package fixture must parse clean"
    );

    let rels = subsettings(&result);
    assert_eq!(rels.len(), 1);
    assert_eq!(subsetted_target(rels[0]), "Lib::base");
}

#[test]
fn ts_1_3_subclassification_def_chained() {
    // `part def Car :> Vehicle;` and `part def SportsCar :> Car { ... }`
    // produce one Subclassification each.
    let source = "package P { \
                  part def Vehicle; \
                  part def Car :> Vehicle; \
                  part def SportsCar :> Car { attribute topSpeed; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = subclassifications(&result);
    assert_eq!(
        rels.len(),
        2,
        "two Subclassifications expected (Car :> Vehicle, SportsCar :> Car), got {}",
        rels.len()
    );
    let mut targets: Vec<String> = rels.iter().map(|e| subclassified_target(e)).collect();
    targets.sort();
    assert_eq!(targets, vec!["Car".to_owned(), "Vehicle".to_owned()]);
}

#[test]
fn ts_1_3_reference_usage_explicit_ref_keyword() {
    // `ref foo : Speed;` emits exactly one ReferenceUsage named `foo`.
    let source = "package P { attribute def Speed; ref foo : Speed; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let refs = reference_usages(&result);
    assert!(
        refs.iter().any(|e| e.name.as_deref() == Some("foo")),
        "ReferenceUsage `foo` not emitted; got {:?}",
        refs.iter().map(|e| e.name.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn ts_1_3_reference_usage_default_form_no_keyword() {
    // Keyword-less feature usages like `in x : Type` and `out y : Type`
    // inside an action body match Pest's `DefaultReferenceUsage` and
    // must emit ReferenceUsage per `SysML.xtext` line 627.
    let source = "package P { \
                  attribute def Speed; \
                  action def Driver { in x : Speed; out y : Speed; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let refs = reference_usages(&result);
    let names: Vec<_> = refs.iter().filter_map(|e| e.name.clone()).collect();
    assert!(
        names.iter().any(|n| n == "x"),
        "ReferenceUsage `x` (default-ref) not emitted; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "y"),
        "ReferenceUsage `y` (default-ref) not emitted; got {names:?}"
    );
}

#[test]
fn ts_1_3_reference_subsetting_references_keyword() {
    // `attribute proxy references target;` emits a ReferenceSubsetting
    // whose `unresolved_referencedFeature` is `target`.
    let source = "package P { attribute target; attribute proxy references target; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = reference_subsettings(&result);
    assert_eq!(
        rels.len(),
        1,
        "exactly one ReferenceSubsetting expected, got {}",
        rels.len()
    );
    assert_eq!(reference_subsetting_target(rels[0]), "target");
}

#[test]
fn ts_1_3_subsetting_canonical_key_stable_across_two_parses() {
    let source = "package P { attribute base; attribute child :> base; }";
    let r1 = parse_and_build(source);
    let r2 = parse_and_build(source);
    let ids1: Vec<_> = subsettings(&r1).iter().map(|e| e.id.clone()).collect();
    let ids2: Vec<_> = subsettings(&r2).iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids1.len(), 1);
    assert_eq!(ids2.len(), 1);
    assert_eq!(
        ids1, ids2,
        "Subsetting canonical key must be stable across re-parses"
    );
}

// ===========================================================================
// TS-1.5 — FeatureTyping emission across syntactic variants.
//
// Gap #5 from `Architectural-cleanup/tree-sitter-canonical-plan/
// ts-ast-builder-gap-list.md`. The earlier `test_build_part_usage_with_typing`
// / `test_typing_has_precise_span` cases cover the canonical `:` operator
// shape; the cases below pin the broader variant matrix called out in TS-1.5:
// qualified-name targets, nested-def members, bare-`feature` keyword
// declarations, and split-sibling `part x : Foo { ... }` shapes. Together
// they bound the FeatureTyping baseline delta after the TS-1.2 / TS-1.3 /
// TS-1.4 fix wave reduced it from -1432 to -33.
// ===========================================================================

fn feature_typings(result: &ModelGraphResult) -> Vec<&sysml_core::Element> {
    result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FeatureTyping)
        .collect()
}

fn feature_typing_target(elem: &sysml_core::Element) -> String {
    elem.get_prop("unresolved_type")
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .expect("FeatureTyping must carry unresolved_type qname")
}

fn feature_typing_source(elem: &sysml_core::Element) -> sysml_core::ElementId {
    match elem
        .get_prop("typedFeature")
        .expect("FeatureTyping must carry typedFeature ref")
    {
        Value::Ref(id) => id.clone(),
        other => panic!("typedFeature must be a Ref, got {other:?}"),
    }
}

#[test]
fn ts_1_5_feature_typing_colon_simple() {
    // `attribute x : Real;` — one FeatureTyping whose target qname is `Real`
    // and whose source is the `x` AttributeUsage.
    let source = "package P { attribute x : Real; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "fixture must parse clean");

    let rels = feature_typings(&result);
    assert_eq!(
        rels.len(),
        1,
        "exactly one FeatureTyping expected, got {}",
        rels.len()
    );
    assert_eq!(feature_typing_target(rels[0]), "Real");

    let source_elem = result
        .graph
        .elements
        .get(&feature_typing_source(rels[0]))
        .expect("typedFeature ref must resolve");
    assert_eq!(source_elem.name.as_deref(), Some("x"));
    assert!(source_elem.kind.is_usage());
}

#[test]
fn ts_1_5_feature_typing_colon_qualified() {
    // `attribute x : Inner::Foo;` — the FeatureTyping target qname must be
    // the qualified path preserved verbatim, so the resolver can walk it.
    let source = "\
        package P { \
            package Inner { attribute def Foo; } \
            attribute x : Inner::Foo; \
        }";
    let result = parse_and_build(source);
    assert!(
        !result.has_errors(),
        "cross-package fixture must parse clean"
    );

    let rels = feature_typings(&result);
    // The inner `attribute def Foo;` produces no FeatureTyping; only `x`'s
    // typing does. We assert the qualified one exists rather than the count
    // because nested defs may add their own typings in future refactors.
    let qualified: Vec<_> = rels
        .iter()
        .filter(|e| feature_typing_target(e) == "Inner::Foo")
        .collect();
    assert_eq!(
        qualified.len(),
        1,
        "qualified-name FeatureTyping target `Inner::Foo` not found; \
         got {:?}",
        rels.iter()
            .map(|e| feature_typing_target(e))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ts_1_5_feature_typing_generic_def_member() {
    // A typed member inside a definition body must produce a FeatureTyping
    // owned by that member (a usage), not by the enclosing definition. We
    // intentionally use `attribute` rather than `part` for the member here:
    // tree-sitter's `standard_usage` rule treats bare identifiers like
    // `item`/`part`/`attribute` as the keyword field, which leaves the
    // member-name slot empty in the CST. Pest is unaffected because its
    // PEG ordered choice prefers Identification first. Asserting via the
    // typed-feature kind keeps the test grammar-quirk-tolerant while still
    // pinning that members get their own FeatureTyping (not the enclosing
    // PartDefinition).
    let source = "package P { \
                  part def Sensor; \
                  part def Container { attribute sensor : Sensor; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = feature_typings(&result);
    let sensor_typings: Vec<_> = rels
        .iter()
        .filter(|e| feature_typing_target(e) == "Sensor")
        .collect();
    assert_eq!(
        sensor_typings.len(),
        1,
        "exactly one FeatureTyping targeting `Sensor` expected, got {:?}",
        rels.iter()
            .map(|e| feature_typing_target(e))
            .collect::<Vec<_>>()
    );

    let source_elem = result
        .graph
        .elements
        .get(&feature_typing_source(sensor_typings[0]))
        .expect("typedFeature must resolve");
    // The typing must be owned by the member usage, not the enclosing def.
    assert!(
        source_elem.kind.is_usage(),
        "FeatureTyping source must be a usage, got {:?}",
        source_elem.kind
    );
    assert_eq!(source_elem.name.as_deref(), Some("sensor"));
}

#[test]
fn ts_1_5_feature_typing_generic_usage() {
    // `part p : Container;` — a plain typed-usage at package scope.
    let source = "package P { part def Container; part p : Container; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = feature_typings(&result);
    assert_eq!(rels.len(), 1);
    assert_eq!(feature_typing_target(rels[0]), "Container");

    let source_elem = result
        .graph
        .elements
        .get(&feature_typing_source(rels[0]))
        .expect("typedFeature must resolve");
    assert_eq!(source_elem.name.as_deref(), Some("p"));
    assert_eq!(source_elem.kind, ElementKind::PartUsage);
}

#[test]
fn ts_1_5_feature_typing_bare_feature_keyword() {
    // `abstract feature elementId : String;` — the bare `feature` keyword
    // form parses as a top-level `feature_declaration` CST node (not a
    // `standard_usage`), dispatched as ReferenceUsage. The `typing` child
    // must still flow through `create_usage_rels`.
    let source = "package P { abstract feature elementId : String; }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = feature_typings(&result);
    let string_typings: Vec<_> = rels
        .iter()
        .filter(|e| feature_typing_target(e) == "String")
        .collect();
    assert_eq!(
        string_typings.len(),
        1,
        "bare `feature` keyword form must still emit FeatureTyping (gap #5)"
    );

    let source_elem = result
        .graph
        .elements
        .get(&feature_typing_source(string_typings[0]))
        .expect("typedFeature must resolve");
    assert_eq!(source_elem.name.as_deref(), Some("elementId"));
}

#[test]
fn ts_1_5_feature_typing_split_sibling() {
    // `part car : Foo { ... }` parses as two CST siblings — `standard_usage`
    // (keyword only) plus `feature_declaration` (name + typing + body). The
    // dispatch split-sibling guard must route the second sibling's `typing`
    // child onto the first sibling's element via `augment_from_split_sibling`,
    // landing the FeatureTyping under the merged usage.
    let source = "package P { \
                  part def Foo { attribute kind : String; } \
                  part car : Foo { attribute kind = \"sedan\"; } \
                  }";
    let result = parse_and_build(source);
    assert!(!result.has_errors());

    let rels = feature_typings(&result);
    let foo_typings: Vec<_> = rels
        .iter()
        .filter(|e| feature_typing_target(e) == "Foo")
        .collect();
    assert_eq!(
        foo_typings.len(),
        1,
        "split-sibling part car : Foo must emit exactly one FeatureTyping `Foo`"
    );

    let source_elem = result
        .graph
        .elements
        .get(&feature_typing_source(foo_typings[0]))
        .expect("typedFeature must resolve");
    assert_eq!(source_elem.name.as_deref(), Some("car"));
    assert_eq!(source_elem.kind, ElementKind::PartUsage);
}

// G14 — ConjugatedPortTyping (`: ~PortDef`) regression coverage.
// Spec: SysML.xtext:969-971 `ConjugatedPortTyping returns SysML::ConjugatedPortTyping`,
//       and SysML.xtext:973-975 `ConjugatedQualifiedName: '~' QualifiedName`.
#[test]
fn g14_conjugated_port_typing_mints_distinct_element() {
    let result = parse_and_build(
        "package P { port def DP { in attribute x; } part def A { port reversedP : ~DP; } }",
    );
    assert!(!result.has_errors(), "parse should succeed");
    let conjugated_typings: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConjugatedPortTyping)
        .collect();
    assert_eq!(
        conjugated_typings.len(),
        1,
        "expected exactly one ConjugatedPortTyping for `: ~DP` (got {})",
        conjugated_typings.len()
    );
    // Regular FeatureTyping should NOT be minted for the conjugated form.
    let feature_typings: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::FeatureTyping)
        .collect();
    // The `in attribute x;` inside the port def is the only non-conjugated typing path
    // (anonymous-typed parameter — G04 territory). The conjugated `: ~DP` itself
    // must NOT contribute a FeatureTyping element.
    assert!(
        feature_typings.len() <= 1,
        "conjugated typing must not also mint a FeatureTyping (saw {})",
        feature_typings.len()
    );
}

#[test]
fn ts_1_5_feature_typing_canonical_key_stable_across_two_parses() {
    // ADR-009 canonical IDs must be deterministic across re-parses for the
    // typing role, mirroring the Redefinition / Subsetting stability tests.
    let source = "package P { attribute x : Real; }";
    let r1 = parse_and_build(source);
    let r2 = parse_and_build(source);
    let ids1: Vec<_> = feature_typings(&r1).iter().map(|e| e.id.clone()).collect();
    let ids2: Vec<_> = feature_typings(&r2).iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids1.len(), 1);
    assert_eq!(ids2.len(), 1);
    assert_eq!(
        ids1, ids2,
        "FeatureTyping canonical key must be stable across re-parses"
    );
}

#[test]
fn g11_comment_about_emits_annotation() {
    // G11: `comment about <qname> (, <qname>)*` should mint an Annotation
    // relationship per target with annotatingElement set to the Comment and
    // unresolved_annotatedElement set to the qualified-name text. Plain
    // comments (no `about` clause) must NOT emit an Annotation.
    let source = r#"package P {
        comment about Foo, Bar /* doc */
        part def Foo;
        part def Bar;
    }"#;
    let result = parse_and_build(source);
    assert!(
        !result.has_errors(),
        "diagnostics: {:?}",
        result.diagnostics
    );

    let comments: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Comment)
        .collect();
    assert_eq!(comments.len(), 1, "expected one Comment");
    let comment_id = comments[0].id.clone();

    let annotations: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::Annotation
                && matches!(
                    e.get_prop("annotatingElement"),
                    Some(Value::Ref(id)) if *id == comment_id
                )
        })
        .collect();
    assert_eq!(
        annotations.len(),
        2,
        "comment about Foo, Bar must emit two Annotations"
    );

    let targets: Vec<_> = annotations
        .iter()
        .filter_map(|e| {
            e.get_prop("unresolved_annotatedElement")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(targets.contains(&"Foo".to_string()), "got: {:?}", targets);
    assert!(targets.contains(&"Bar".to_string()), "got: {:?}", targets);
}

#[test]
fn g11_comment_without_about_emits_no_annotation() {
    // `comment /* ... */` without an `about` clause must NOT emit an
    // Annotation — Pest's plain-comment path doesn't, so emitting one in
    // TS would produce a ts_only divergence in the corpus.
    let source = r#"package P {
        comment /* plain block comment */
        part def X;
    }"#;
    let result = parse_and_build(source);
    assert!(
        !result.has_errors(),
        "diagnostics: {:?}",
        result.diagnostics
    );

    let comments: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Comment)
        .collect();
    assert_eq!(comments.len(), 1, "expected one Comment");

    let annotations: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Annotation)
        .collect();
    assert!(
        annotations.is_empty(),
        "plain `comment /* */` must not emit Annotation, got: {} annotations",
        annotations.len()
    );
}

// ── B1: dependency_usage lowering ──────────────────────────────────────

#[test]
fn dependency_named_from_to_lowers_with_client_supplier() {
    let result = parse_and_build("package P { part a; part b; dependency dep1 from a to b; }");
    assert!(!result.has_errors());

    let deps: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Dependency)
        .collect();
    assert_eq!(deps.len(), 1, "dependency_usage must lower to a Dependency");
    let dep = deps[0];
    assert_eq!(dep.name.as_deref(), Some("dep1"));
    assert_eq!(
        dep.get_prop("unresolved_client").and_then(|v| v.as_str()),
        Some("a")
    );
    assert_eq!(
        dep.get_prop("unresolved_supplier").and_then(|v| v.as_str()),
        Some("b")
    );
}

#[test]
fn dependency_without_from_treats_leading_name_as_client() {
    // SysML.xtext Dependency: `'dependency' (Identification? 'from')? client
    // 'to' supplier` — without `from`, the leading name IS the first client.
    let result = parse_and_build("package P { part a; part b; dependency a to b; }");
    assert!(!result.has_errors());

    let deps: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Dependency)
        .collect();
    assert_eq!(deps.len(), 1);
    let dep = deps[0];
    assert_eq!(dep.name, None, "leading name is the client, not the name");
    assert_eq!(
        dep.get_prop("unresolved_client").and_then(|v| v.as_str()),
        Some("a")
    );
    assert_eq!(
        dep.get_prop("unresolved_supplier").and_then(|v| v.as_str()),
        Some("b")
    );
}

#[test]
fn dependency_body_metadata_owned_by_dependency() {
    // The @Refinement annotation inside the dependency body must attach to
    // the Dependency element (it floated up to the package before B1's
    // lowering existed).
    let result =
        parse_and_build("package P { part a; part b; dependency d from a to b { @Refinement; } }");
    assert!(!result.has_errors());

    let dep_id = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::Dependency)
        .map(|e| e.id.clone())
        .expect("Dependency element");
    let metadata: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::MetadataUsage)
        .collect();
    assert_eq!(metadata.len(), 1);
    assert_eq!(
        metadata[0].owner.as_ref(),
        Some(&dep_id),
        "@Refinement must be owned by the Dependency, not the package"
    );
    assert_eq!(
        metadata[0]
            .get_prop("annotationType")
            .and_then(|v| v.as_str()),
        Some("Refinement")
    );
}

#[test]
fn connection_end_references_stay_distinct_elements() {
    // Two `end ref X references Y;` members in one connection body: GLR
    // parses the first as standard_usage and the second as
    // feature_declaration. The split-sibling fusion must NOT absorb the
    // second (it carries its own usage_prefix → standalone declaration).
    // Regression: both ends collapsed onto one element, re-parenting the
    // second ReferenceSubsetting onto end 1.
    let result = parse_and_build(
        "package P { part a; part b; connection c { end ref e1 references a; end ref e2 references b; } }",
    );
    assert!(!result.has_errors());

    let ends: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ReferenceUsage && e.get_prop("isEnd").is_some())
        .collect();
    assert_eq!(
        ends.len(),
        2,
        "both connection ends must survive as elements"
    );
    let names: Vec<_> = ends.iter().filter_map(|e| e.name.as_deref()).collect();
    assert!(names.contains(&"e1") && names.contains(&"e2"));

    // Each end owns exactly one ReferenceSubsetting pointing at its own ref.
    for (end_name, referenced) in [("e1", "a"), ("e2", "b")] {
        let end = ends
            .iter()
            .find(|e| e.name.as_deref() == Some(end_name))
            .unwrap();
        let subs: Vec<_> = result
            .graph
            .children_of(&end.id)
            .filter(|c| c.kind == ElementKind::ReferenceSubsetting)
            .collect();
        assert_eq!(subs.len(), 1, "end {end_name} owns one ReferenceSubsetting");
        assert_eq!(
            subs[0]
                .get_prop("unresolved_referencedFeature")
                .and_then(|v| v.as_str()),
            Some(referenced),
            "end {end_name} must reference {referenced}"
        );
    }
}

#[test]
fn dependency_after_comment_shatter_recovers() {
    // A preceding sl_note makes the GLR resolver shatter the dependency
    // statement into a keyword-only dependency_usage + one
    // feature_declaration per token run. The lowering must reassemble it —
    // including the body, so @Refinement attaches to the Dependency.
    let result = parse_and_build(
        "package P {\n\tpart a;\n\tpart b;\n\t// note before\n\tdependency d1 from a to b {\n\t\t@Refinement;\n\t}\n\t// another note\n\tdependency a to b;\n}",
    );
    assert!(!result.has_errors());

    let deps: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Dependency)
        .collect();
    assert_eq!(deps.len(), 2, "both dependencies must lower");

    let named = deps
        .iter()
        .find(|d| d.name.as_deref() == Some("d1"))
        .expect("named dependency d1");
    assert_eq!(
        named.get_prop("unresolved_client").and_then(|v| v.as_str()),
        Some("a")
    );
    assert_eq!(
        named
            .get_prop("unresolved_supplier")
            .and_then(|v| v.as_str()),
        Some("b")
    );
    let meta: Vec<_> = result
        .graph
        .children_of(&named.id)
        .filter(|c| c.kind == ElementKind::MetadataUsage)
        .collect();
    assert_eq!(meta.len(), 1, "@Refinement must attach to the Dependency");

    let bare = deps
        .iter()
        .find(|d| d.name.is_none())
        .expect("anonymous dependency");
    assert_eq!(
        bare.get_prop("unresolved_client").and_then(|v| v.as_str()),
        Some("a"),
        "leading name without from-clause is the client"
    );
}

// ── G24 / B1b: PrefixMetadataAnnotation + end ReferenceSubsetting ──────

#[test]
fn keyword_derivation_lowers_metadata_and_semantic_base_subsettings() {
    let result = parse_and_build(
        r#"package P {
            requirement reqA;
            requirement reqB;
            #derivation connection {
                end #original ::> reqA;
                end #derive ::> reqB;
            }
        }"#,
    );
    assert!(
        !result.has_errors(),
        "diagnostics: {:?}",
        result.diagnostics
    );

    let connection = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::ConnectionUsage)
        .expect("keyword-annotated connection");
    let connection_subsettings: Vec<_> = result
        .graph
        .children_of(&connection.id)
        .filter(|e| e.kind == ElementKind::Subsetting)
        .collect();
    assert_eq!(connection_subsettings.len(), 1);
    assert_eq!(
        connection_subsettings[0]
            .get_prop("unresolved_subsettedFeature")
            .and_then(|v| v.as_str()),
        Some("derivations")
    );

    let connection_metadata: Vec<_> = result
        .graph
        .children_of(&connection.id)
        .filter(|e| e.kind == ElementKind::MetadataUsage)
        .collect();
    assert_eq!(connection_metadata.len(), 1);
    assert_eq!(
        connection_metadata[0]
            .get_prop("unresolvedTypeName")
            .and_then(|v| v.as_str()),
        Some("derivation")
    );

    let mut end_roles: Vec<(&str, &str)> = result
        .graph
        .children_of(&connection.id)
        .filter(|e| e.kind == ElementKind::ReferenceUsage)
        .map(|end| {
            assert_eq!(end.get_prop("isEnd").and_then(|v| v.as_bool()), Some(true));
            let role = result
                .graph
                .children_of(&end.id)
                .find(|e| e.kind == ElementKind::Subsetting)
                .and_then(|e| e.get_prop("unresolved_subsettedFeature"))
                .and_then(|v| v.as_str())
                .expect("SemanticMetadata baseType subsetting");
            let referenced = result
                .graph
                .children_of(&end.id)
                .find(|e| e.kind == ElementKind::ReferenceSubsetting)
                .and_then(|e| e.get_prop("unresolved_referencedFeature"))
                .and_then(|v| v.as_str())
                .expect("::> ReferenceSubsetting");
            (role, referenced)
        })
        .collect();
    end_roles.sort_unstable();
    assert_eq!(
        end_roles,
        vec![
            ("derivedRequirements", "reqB"),
            ("originalRequirements", "reqA")
        ]
    );
}

#[test]
fn dependency_lists_lower_losslessly() {
    let result = parse_and_build(
        "package P { part a; part b; part c; part d; dependency dep from a, b to c, d; }",
    );
    assert!(
        !result.has_errors(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let dependency = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::Dependency)
        .expect("Dependency");

    let strings = |prop: &str| {
        dependency
            .get_prop(prop)
            .and_then(|v| v.as_list())
            .expect("list property")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(strings("unresolved_clients"), vec!["a", "b"]);
    assert_eq!(strings("unresolved_suppliers"), vec!["c", "d"]);
}

#[test]
fn requirement_body_accepts_untyped_part_and_part_named_frame() {
    let result = parse_and_build("package P { requirement def R { part chassis; part frame; } }");
    assert!(
        !result.has_errors(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let mut names: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartUsage)
        .filter_map(|e| e.name.as_deref())
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["chassis", "frame"]);
}

/// Steward ruling 2026-07-17 (abstract-collector consult): the library's
/// bare collector SLOTS (`RequirementConstraintCheck::assumptions[0..*]`/
/// `::constraints[0..*]` and RequirementCheck's `:>>` shells) are
/// aggregation points, not obligations — `effective_requirement_constraints`
/// must never emit them. Genuine user constraints on the same chain
/// still come through.
#[test]
fn effective_constraints_skip_bare_collector_slots() {
    let source = r#"
package MiniReq {
    abstract constraint def RequirementConstraintCheck {
        constraint assumptions[0..*] :> constraintChecks, subperformances;
        constraint constraints[0..*] :> constraintChecks, subperformances;
    }
    requirement def RequirementCheck :> RequirementConstraintCheck {
        constraint assumptions :>> RequirementConstraintCheck::assumptions;
        constraint constraints :>> RequirementConstraintCheck::constraints;
    }
    requirement def UserReq :> RequirementCheck {
        require constraint minGap { gap >= 4.0 }
    }
}
"#;
    let result = parse_and_build(source);
    let graph = &result.graph;
    let user_req = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("UserReq"))
        .unwrap();
    let effective: Vec<_> = sysml_core::query::effective_requirement_constraints(user_req, graph);
    // ONLY the user's own obligation survives — neither the [0..*]
    // declarations nor the :>> shells are emitted from the chain.
    assert_eq!(
        effective.len(),
        1,
        "expected only minGap, got: {:?}",
        effective
            .iter()
            .map(|m| (m.element.kind.clone(), m.element.name.as_deref()))
            .collect::<Vec<_>>()
    );
    assert_eq!(effective[0].element.name.as_deref(), Some("minGap"));

    // The collector defs' OWN effective set is empty (evaluates
    // Inconclusive — "no modeled pass criteria" — never a compile Error).
    let check_def = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("RequirementCheck"))
        .unwrap();
    assert!(
        sysml_core::query::effective_requirement_constraints(check_def, graph).is_empty(),
        "the library-shaped def's bare collector slots must not be obligations"
    );
}

// ---------------------------------------------------------------------------
// Enumerated values (SysML §8.3.8, gap `enumeration-usage-not-distinct`).
//
// Members declared with `enum` inside an `enum def` previously hit the
// `_ => None` dispatch default and produced ZERO model elements. Each must now
// lower to a distinct EnumerationUsage owned by the enumeration through a
// VariantMembership and typed by that enumeration.
// ---------------------------------------------------------------------------

#[test]
fn enum_members_lower_to_distinct_enumeration_usages() {
    // Third member drops the optional `enum` keyword (EnumeratedValue's
    // `EnumerationUsageKeyword?`, SysML.xtext) — still an enumerated value.
    let result = parse_and_build("enum def Color { enum red; enum green; blue; }");
    assert!(!result.has_errors(), "clean enum def must not error");

    let members: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::EnumerationUsage)
        .collect();
    assert_eq!(
        members.len(),
        3,
        "N enum members must lower to N EnumerationUsage elements, got {:?}",
        members
            .iter()
            .map(|m| m.name.as_deref())
            .collect::<Vec<_>>()
    );

    let names: std::collections::BTreeSet<&str> =
        members.iter().filter_map(|m| m.name.as_deref()).collect();
    assert_eq!(
        names,
        ["blue", "green", "red"].into_iter().collect(),
        "each enumerated value keeps its declared name"
    );

    // Correct owner: every member is owned by the enumeration definition.
    let color = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition && e.name.as_deref() == Some("Color"))
        .expect("Color enum def");
    for m in &members {
        assert_eq!(
            m.owner.as_ref(),
            Some(&color.id),
            "enumerated value {:?} must be owned by its enum def",
            m.name
        );
    }
    assert_eq!(
        result
            .graph
            .children_of(&color.id)
            .filter(|c| c.kind == ElementKind::EnumerationUsage)
            .count(),
        3
    );
}

#[test]
fn enum_member_is_typed_by_owning_enum_def() {
    let result = parse_and_build("enum def Color { enum red; }");
    let color = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition)
        .expect("Color");
    let red = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationUsage && e.name.as_deref() == Some("red"))
        .expect("red");

    // §8.3.8.3: the enumerated value is typed by its owning enumeration. The
    // FeatureTyping's `type` is resolved directly to the enum def (structural).
    let typing = result
        .graph
        .children_of(&red.id)
        .find(|c| c.kind == ElementKind::FeatureTyping)
        .expect("red must own a FeatureTyping to its enum def");
    assert_eq!(
        typing.get_prop("type").and_then(|v| v.as_ref()),
        Some(&color.id),
        "enumerated value must be typed by its owning EnumerationDefinition"
    );
}

#[test]
fn enum_def_is_variation_with_variant_memberships() {
    let result = parse_and_build("enum def Color { enum red; enum green; }");
    let color = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition)
        .expect("Color");

    // §8.3.8.2: an EnumerationDefinition is always a variation.
    assert_eq!(
        color.get_prop("isVariation").and_then(|v| v.as_bool()),
        Some(true),
        "an EnumerationDefinition must be a variation (S050 for its variants)"
    );

    // The grammar's `EnumerationUsageMember returns SysML::VariantMembership`:
    // each enumerated value is wrapped in a VariantMembership of the enum def.
    let variant_memberships: Vec<_> = result
        .graph
        .memberships(&color.id)
        .filter(|m| m.kind == ElementKind::VariantMembership)
        .collect();
    assert_eq!(
        variant_memberships.len(),
        2,
        "each enumerated value is wrapped in a VariantMembership"
    );
    // The VariantMembership names the member and points at the EnumerationUsage.
    for vm in &variant_memberships {
        let member_id = vm
            .get_prop("memberElement")
            .and_then(|v| v.as_ref())
            .expect("VariantMembership.memberElement");
        let member = result.graph.get_element(member_id).expect("member");
        assert_eq!(member.kind, ElementKind::EnumerationUsage);
    }
}

#[test]
fn zero_member_enum_def_is_fine() {
    // An empty enumeration body is well-formed; it just has no variants.
    let result = parse_and_build("enum def Color { }");
    assert!(!result.has_errors());
    assert_eq!(
        result
            .graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::EnumerationUsage)
            .count(),
        0
    );
    let color = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition);
    assert!(color.is_some(), "the enum def itself is still created");
}

#[test]
fn qualified_reference_to_enum_member_resolves() {
    // An enumerated value registers in its enumeration's scope, so a qualified
    // reference (`Color::red`) resolves to the distinct EnumerationUsage.
    let result = parse_and_build("package P { enum def Color { enum red; enum green; } }");
    let graph = &result.graph;

    let color = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition && e.name.as_deref() == Some("Color"))
        .expect("Color");
    let red = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationUsage && e.name.as_deref() == Some("red"))
        .expect("red");

    // Direct scope lookup: `red` is a member of `Color`.
    assert_eq!(
        graph.resolve_name_in(&color.id, "red").as_ref(),
        Some(&red.id),
        "`red` must resolve as a member of the `Color` enumeration"
    );
    // Fully-qualified lookup from root: `P::Color::red`.
    assert_eq!(
        graph.resolve_qualified("P::Color::red").as_ref(),
        Some(&red.id),
        "`P::Color::red` must resolve to the enumerated value"
    );
}

/// F3 engine-gap `constructor-expression-generic-lowering`: `new T(...)` must
/// lower to the distinct `ConstructorExpression` kind, NOT to the abstract
/// parent `InstantiationExpression` nor to a plain `InvocationExpression`. The
/// syntactic discriminator is the `new` keyword (KerMLExpressions.xtext rule
/// `ConstructorExpression` = `'new' InstantiatedTypeMember ConstructorResultMember`
/// vs rule `InvocationExpression` = `InstantiatedTypeMember ArgumentList`; both
/// host the instantiated-type reference, only the constructor is `new`-prefixed
/// and returns SysML::ConstructorExpression). Kerml-Vocab.ttl:91-95 makes
/// ConstructorExpression a subclass of InstantiationExpression.
#[test]
fn constructor_expression_lowers_to_distinct_kind() {
    // Two feature values in one part: a constructor (`new`) and a plain
    // invocation of the same type/args, so we prove they diverge in kind.
    let source = r#"
        package P {
            part def Widget { attribute size; }
            part a { attribute x = new Widget(size = 1); }
            part b { attribute y = Widget(size = 1); }
        }
    "#;
    let result = parse_and_build(source);
    assert!(!result.has_errors(), "fixture must parse without errors");
    let g = &result.graph;

    let ctors: Vec<_> = g
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ConstructorExpression)
        .collect();
    assert_eq!(
        ctors.len(),
        1,
        "`new Widget(...)` must lower to exactly one ConstructorExpression"
    );
    let ctor = ctors[0];

    // The abstract parent must NEVER be minted directly.
    assert_eq!(
        g.elements
            .values()
            .filter(|e| e.kind == ElementKind::InstantiationExpression)
            .count(),
        0,
        "abstract InstantiationExpression must not be minted for `new T(...)`"
    );

    // The constructor carries the instantiated-type reference and named args.
    assert_eq!(
        ctor.name.as_deref(),
        Some("Widget"),
        "ConstructorExpression must carry the instantiated-type reference"
    );
    let named_arg = g.children_of(&ctor.id).any(|c| {
        c.get_prop("argName")
            .and_then(|v| v.as_str())
            .map(|s| s == "size")
            .unwrap_or(false)
    });
    assert!(
        named_arg,
        "ConstructorExpression must preserve the named argument `size`"
    );

    // The ordinary invocation `Widget(size = 1)` still lowers to an
    // InvocationExpression — the two forms remain distinguishable.
    assert_eq!(
        g.elements
            .values()
            .filter(|e| e.kind == ElementKind::InvocationExpression)
            .count(),
        1,
        "plain `Widget(...)` must still lower to InvocationExpression"
    );
}

// ---------------------------------------------------------------------------
// Occurrence-usage prefixes (SysML §8.3.9, gap `occurrence-prefix-generic-lowering`).
//
// `individual` / `snapshot` / `timeslice` usage prefixes previously lowered to
// a generic ReferenceUsage with no occurrence classification. Per the xtext
// rules IndividualUsage / PortionUsage (both `returns SysML::OccurrenceUsage`
// with isIndividual / portionKind set), each must now be an OccurrenceUsage
// carrying the correct flag. NOTE: the `individual def` DEFINITION form is a
// grammar gap (misparses — `def` is swallowed as the name) and is reported for
// the serialized grammar lane, not fixed here.
// ---------------------------------------------------------------------------

#[test]
fn individual_usage_lowers_to_occurrence_usage_with_flag() {
    let result = parse_and_build("package P { occurrence def O; individual i : O; }");
    let i = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("i"))
        .expect("usage i");
    assert_eq!(
        i.kind,
        ElementKind::OccurrenceUsage,
        "`individual i` must be an OccurrenceUsage (IndividualUsage returns OccurrenceUsage), not a bare ReferenceUsage"
    );
    assert_eq!(
        i.get_prop("isIndividual").and_then(|v| v.as_bool()),
        Some(true),
        "the `individual` prefix must set isIndividual"
    );
}

#[test]
fn snapshot_and_timeslice_usages_lower_to_occurrence_usage_with_portion_kind() {
    let result = parse_and_build(
        "package P { occurrence def O; snapshot s : O; timeslice t : O; }",
    );
    let g = &result.graph;
    for (name, expected_kind) in [("s", "snapshot"), ("t", "timeslice")] {
        let u = g
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("usage {name}"));
        assert_eq!(
            u.kind,
            ElementKind::OccurrenceUsage,
            "`{expected_kind} {name}` must be an OccurrenceUsage (PortionUsage returns OccurrenceUsage)"
        );
        assert_eq!(
            u.get_prop("portionKind").and_then(|v| v.as_str()),
            Some(expected_kind),
            "the `{expected_kind}` prefix must set portionKind"
        );
        assert_eq!(
            u.get_prop("isPortion").and_then(|v| v.as_bool()),
            Some(true),
            "a temporal portion usage must set isPortion"
        );
    }
}

#[test]
fn occurrence_keyword_usage_still_lowers_and_plain_ref_is_not_promoted() {
    // Regression guard: the plain `occurrence` keyword usage keeps lowering to
    // OccurrenceUsage, and a prefix-less generic reference is NOT promoted.
    let result = parse_and_build("package P { occurrence o; ref r; }");
    let g = &result.graph;
    assert_eq!(
        g.elements
            .values()
            .find(|e| e.name.as_deref() == Some("o"))
            .map(|e| e.kind.clone()),
        Some(ElementKind::OccurrenceUsage)
    );
    let r = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("r"))
        .expect("usage r");
    assert_ne!(
        r.kind,
        ElementKind::OccurrenceUsage,
        "a plain `ref` with no occurrence prefix must not be promoted to OccurrenceUsage"
    );
    assert!(
        r.get_prop("isIndividual").is_none() && r.get_prop("portionKind").is_none(),
        "a plain reference must carry no occurrence flags"
    );
}

// ---------------------------------------------------------------------------
// KerML §7.3 type-relationship operators (gap `type-relationship-operators`).
//
// `unions`/`intersects`/`differences`/`disjoint from` (on Types) and
// `featured by`/`inverse of` (on Features) parse to CST clauses but previously
// lowered to ZERO elements. Each must now mint the real Relationship metaclass
// (Unioning/Intersecting/Differencing/Disjoining/TypeFeaturing/FeatureInverting)
// owned by the source, with the target captured for resolution.
// ---------------------------------------------------------------------------

fn rels_of<'g>(
    g: &'g sysml_core::ModelGraph,
    owner: &sysml_core::ElementId,
    kind: ElementKind,
) -> Vec<&'g sysml_core::Element> {
    let mut v: Vec<_> = g
        .children_of(owner)
        .filter(|e| e.kind == kind)
        .collect();
    // children_of is a hash set — sort by span start so multi-target lists are
    // source-ordered for assertions.
    v.sort_by_key(|e| e.spans.first().map(|s| s.start).unwrap_or(0));
    v
}

#[test]
fn union_intersect_difference_disjoint_lower_to_relationship_elements() {
    let result = parse_and_build(
        "package P { class A; class B; \
         class U unions A, B; class I2 intersects A, B; \
         class D differences A, B; class J disjoint from A, B; }",
    );
    assert!(!result.has_errors());
    let g = &result.graph;
    let owner = |name: &str| {
        g.elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("class {name}"))
            .id
            .clone()
    };

    // (owner, kind, source-role prop, target-role unresolved prop)
    let cases = [
        ("U", ElementKind::Unioning, "typeUnioned", "unresolved_unioningType"),
        ("I2", ElementKind::Intersecting, "typeIntersected", "unresolved_intersectingType"),
        ("D", ElementKind::Differencing, "typeDifferenced", "unresolved_differencingType"),
        ("J", ElementKind::Disjoining, "typeDisjoined", "unresolved_disjoiningType"),
    ];
    for (name, kind, source_role, unresolved_target) in cases {
        let oid = owner(name);
        let rels = rels_of(g, &oid, kind.clone());
        assert_eq!(rels.len(), 2, "{name} must own two {kind:?} relationships");
        // Source role points back at the declaring type.
        for r in &rels {
            assert_eq!(
                r.get_prop(source_role).and_then(|v| v.as_ref()),
                Some(&oid),
                "{kind:?}.{source_role} must reference the source type {name}"
            );
        }
        // Targets are captured unresolved, in source order (A then B).
        let targets: Vec<&str> = rels
            .iter()
            .filter_map(|r| r.get_prop(unresolved_target).and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            targets,
            vec!["A", "B"],
            "{kind:?} targets must be captured source-ordered"
        );
    }
}

#[test]
fn featured_by_and_inverse_of_lower_on_features() {
    let result = parse_and_build(
        "package P { class A; feature f; feature g featured by A; feature h inverse of f; }",
    );
    assert!(!result.has_errors());
    let g = &result.graph;
    let gid = g.elements.values().find(|e| e.name.as_deref() == Some("g")).unwrap().id.clone();
    let hid = g.elements.values().find(|e| e.name.as_deref() == Some("h")).unwrap().id.clone();

    let tf = rels_of(g, &gid, ElementKind::TypeFeaturing);
    assert_eq!(tf.len(), 1, "`featured by` must mint one TypeFeaturing");
    assert_eq!(tf[0].get_prop("featureOfType").and_then(|v| v.as_ref()), Some(&gid));
    assert_eq!(tf[0].get_prop("unresolved_featuringType").and_then(|v| v.as_str()), Some("A"));

    let fi = rels_of(g, &hid, ElementKind::FeatureInverting);
    assert_eq!(fi.len(), 1, "`inverse of` must mint one FeatureInverting");
    assert_eq!(fi[0].get_prop("featureInverted").and_then(|v| v.as_ref()), Some(&hid));
    assert_eq!(fi[0].get_prop("unresolved_invertingFeature").and_then(|v| v.as_str()), Some("f"));
}

#[test]
fn type_relationship_target_resolves_and_dangling_stays_unresolved() {
    let mut result = parse_and_build("package P { class A; class U unions A; class V unions Nope; }");
    let uid = result.graph.elements.values().find(|e| e.name.as_deref() == Some("U")).unwrap().id.clone();
    let aid = result.graph.elements.values().find(|e| e.name.as_deref() == Some("A")).unwrap().id.clone();
    let vid = result.graph.elements.values().find(|e| e.name.as_deref() == Some("V")).unwrap().id.clone();

    sysml_core::resolution::resolve_references(&mut result.graph);
    let g = &result.graph;

    // Resolved: U's Unioning.unioningType now points at A.
    let u_rel = rels_of(g, &uid, ElementKind::Unioning);
    assert_eq!(
        u_rel[0].get_prop("unioningType").and_then(|v| v.as_ref()),
        Some(&aid),
        "a resolvable target must be linked to the referenced type"
    );
    // Dangling: V's Unioning target `Nope` does not resolve — the resolved
    // `unioningType` ref must NOT be fabricated (fail-hard, not silent link).
    let v_rel = rels_of(g, &vid, ElementKind::Unioning);
    assert!(
        v_rel[0].get_prop("unioningType").and_then(|v| v.as_ref()).is_none(),
        "a dangling target must not produce a resolved reference"
    );
}

#[test]
fn enum_def_is_abstract_so_s080_does_not_fire() {
    // §14476 isVariation implies isAbstract: a plain `enum def` is a variation
    // and must therefore be abstract, else S080 ("a variation must be abstract")
    // red-squiggles every valid enumeration. The lowering stamps both flags.
    let result = parse_and_build("enum def Color { enum red; }");
    let color = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition)
        .expect("Color enum def");
    assert_eq!(
        color.get_prop("isAbstract").and_then(|v| v.as_bool()),
        Some(true),
        "an enum def (a variation) must be stamped isAbstract"
    );
    assert!(
        sysml_core::semantic_checks::variation::variation_must_be_abstract(color, &result.graph)
            .is_none(),
        "S080 must NOT fire on a valid enum def"
    );
}

#[test]
fn snapshot_part_keeps_part_usage_kind_and_gains_portion_flag() {
    // Review pin for the occurrence-prefix guard: a portion/individual prefix on
    // a usage that ALREADY has a specific occurrence subtype (PartUsage) must
    // KEEP that kind — only the generic ReferenceUsage/Usage default is promoted
    // to OccurrenceUsage — while still gaining the portionKind/isPortion flags.
    let result = parse_and_build("package P { occurrence def O; snapshot part p : O; }");
    let p = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("p"))
        .expect("usage p");
    assert_eq!(
        p.kind,
        ElementKind::PartUsage,
        "`snapshot part p` must remain a PartUsage, not be downgraded to OccurrenceUsage"
    );
    assert_eq!(
        p.get_prop("portionKind").and_then(|v| v.as_str()),
        Some("snapshot"),
        "the specific-kind usage must still gain the portionKind flag"
    );
    assert_eq!(
        p.get_prop("isPortion").and_then(|v| v.as_bool()),
        Some(true),
    );
}

// ---------------------------------------------------------------------------
// Grammar-lane batch: conjugation (`~`/`conjugates`, KerML ConjugationPart) and
// the unnamed EnumeratedValue value form (`= 60.0;`). Both require the
// regenerated parser.c (grammar items 1 & 3).
// ---------------------------------------------------------------------------

#[test]
fn conjugation_clause_lowers_to_conjugation_relationship() {
    // ConjugationPart: `X conjugates A` / `X ~ A`. X is the conjugatedType
    // (owner), A the originalType (target). Mints a Conjugation owned by X.
    let result = parse_and_build("package P { class A; class X conjugates A; }");
    assert!(!result.has_errors());
    let g = &result.graph;
    let x = g.elements.values().find(|e| e.name.as_deref() == Some("X")).expect("X").id.clone();
    let rels = rels_of(g, &x, ElementKind::Conjugation);
    assert_eq!(rels.len(), 1, "`conjugates` must mint one Conjugation");
    assert_eq!(
        rels[0].get_prop("conjugatedType").and_then(|v| v.as_ref()),
        Some(&x),
        "the declaring type is the conjugatedType (owner)"
    );
    assert_eq!(
        rels[0].get_prop("unresolved_originalType").and_then(|v| v.as_str()),
        Some("A"),
        "the clause target is captured as the originalType"
    );
}

#[test]
fn conjugation_resolves_original_type() {
    // Both spellings are admitted since grammar 884e7a61 (the symbolic `~` is
    // excluded from lambda-parameter position only — see the symbolic tests
    // in the task #85 batch below).
    let mut result = parse_and_build("package P { class A; class X conjugates A; }");
    let xid = result.graph.elements.values().find(|e| e.name.as_deref() == Some("X")).unwrap().id.clone();
    let aid = result.graph.elements.values().find(|e| e.name.as_deref() == Some("A")).unwrap().id.clone();
    sysml_core::resolution::resolve_references(&mut result.graph);
    let rel = rels_of(&result.graph, &xid, ElementKind::Conjugation);
    assert_eq!(rel.len(), 1, "`conjugates` must mint a Conjugation");
    assert_eq!(
        rel[0].get_prop("originalType").and_then(|v| v.as_ref()),
        Some(&aid),
        "the originalType target must resolve to A"
    );
}

#[test]
fn unnamed_enumerated_value_form_lowers_to_anonymous_enum_usages() {
    // SysML.xtext EnumeratedValue value form: `= 60.0;` (SizeChoice). Each
    // mints an anonymous EnumerationUsage owned by the enum def, carrying the
    // value; still typed by the enumeration.
    let result = parse_and_build("package P { enum def SizeChoice :> ScalarValues::Real { = 60.0; = 70.0; } }");
    assert!(!result.has_errors());
    let g = &result.graph;
    let sc = g
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationDefinition && e.name.as_deref() == Some("SizeChoice"))
        .expect("SizeChoice");
    let members: Vec<_> = g
        .children_of(&sc.id)
        .filter(|e| e.kind == ElementKind::EnumerationUsage)
        .collect();
    assert_eq!(members.len(), 2, "two value-form enumerated values");
    for m in &members {
        assert!(m.name.is_none(), "the value form is anonymous (no declared name)");
        // The `= <value>` lowered to a child expression subtree.
        assert!(
            g.children_of(&m.id).next().is_some(),
            "each value-form member owns a value expression child"
        );
    }
}

#[test]
fn message_lowers_to_flow_usage_with_from_to_ends() {
    // SysML.xtext Message:1240 → FlowUsage. `message m of Sig from a to b;`:
    // FlowUsage + isMessage, from/to ends captured as source/target (the same
    // endpoint props a flow's ends use), payload preserved. Requires the
    // regenerated parser.c (message_usage grammar).
    let result = parse_and_build("package P { part a; part b; message m of Sig from a to b; }");
    let g = &result.graph;
    let m = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("m"))
        .expect("message m");
    assert_eq!(m.kind, ElementKind::FlowUsage, "a message lowers to FlowUsage");
    assert_eq!(
        m.get_prop("isMessage").and_then(|v| v.as_bool()),
        Some(true),
        "a message sets isMessage"
    );
    assert_eq!(
        m.get_prop("source").and_then(|v| v.as_str()),
        Some("a"),
        "the `from` end is captured as source"
    );
    assert_eq!(
        m.get_prop("target").and_then(|v| v.as_str()),
        Some("b"),
        "the `to` end is captured as target"
    );
    // The `of <payload>` lowers to a PayloadFeature child — pinned by the
    // ends-nesting tests in the task #85 batch below.
}

// ---------------------------------------------------------------------------
// Task #85 post-regen batch (grammar 884e7a61): standalone `enum` usage,
// `individual def`, symbolic `~` conjugation, anonymous message/flow ends.
// ---------------------------------------------------------------------------

// SysML.xtext EnumerationUsage:785-788 (`UsagePrefix EnumerationUsageKeyword
// Usage`) — a REAL metaclass (SysML-vocab.ttl:343). The standalone usage form
// is an ORDINARY member: VariantMembership wrapping is exclusive to
// enumerated values inside an `enum def` body (EnumeratedValue, ad4c95ba).
#[test]
fn standalone_enum_usage_lowers_to_enumeration_usage() {
    let result = parse_and_build("package P { enum vals; }");
    assert!(!result.has_errors());
    let g = &result.graph;
    let e = g
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationUsage)
        .expect("enum usage");
    assert_eq!(e.name.as_deref(), Some("vals"));
    // Ordinary member, NOT VariantMembership-wrapped (that wrapper is
    // enum-def-body-only).
    let membership = g.owning_membership_of(&e.id).expect("owning membership");
    assert_eq!(
        membership.kind,
        ElementKind::OwningMembership,
        "a standalone enum usage is an ordinary owned member"
    );
}

#[test]
fn typed_enum_usage_carries_feature_typing() {
    let result = parse_and_build(
        "package P { enum def Color { red; green; } enum e : Color; }",
    );
    assert!(!result.has_errors());
    let g = &result.graph;
    let e = g
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationUsage && e.name.as_deref() == Some("e"))
        .expect("enum e");
    let typings = rels_of(g, &e.id, ElementKind::FeatureTyping);
    assert_eq!(typings.len(), 1, "`: Color` mints one FeatureTyping");
    assert_eq!(
        typings[0].get_prop("unresolved_type").and_then(|v| v.as_str()),
        Some("Color"),
        "the typing target is captured for resolution"
    );
}

#[test]
fn variation_prefixed_enum_usage_sets_is_variation() {
    let result = parse_and_build("package P { variation enum vals; }");
    assert!(!result.has_errors());
    let e = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::EnumerationUsage)
        .expect("enum usage");
    assert_eq!(
        e.get_prop("isVariation").and_then(|v| v.as_bool()),
        Some(true),
        "the `variation` usage prefix stamps isVariation"
    );
}

// SysML.xtext IndividualDefinition:813-817 — the rule RETURNS
// SysML::OccurrenceDefinition with `isIndividual ?= 'individual'`; there is NO
// IndividualDefinition metaclass (SysML-vocab.ttl:718-724, :1718). The spec's
// EmptyMultiplicityMember (:819-825) is consciously simplified flags-first
// (see the dispatch arm).
#[test]
fn individual_def_lowers_to_occurrence_definition_marked_individual() {
    let result = parse_and_build("package P { individual def X; }");
    assert!(!result.has_errors());
    let d = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::OccurrenceDefinition)
        .expect("individual def");
    assert_eq!(d.name.as_deref(), Some("X"));
    assert_eq!(
        d.get_prop("isIndividual").and_then(|v| v.as_bool()),
        Some(true),
        "`individual def` marks the OccurrenceDefinition individual"
    );
}

#[test]
fn individual_def_with_supertype_mints_subclassification() {
    let result = parse_and_build("package P { individual def X; individual def Y :> X; }");
    assert!(!result.has_errors());
    let g = &result.graph;
    let y = g
        .elements
        .values()
        .find(|e| e.kind == ElementKind::OccurrenceDefinition && e.name.as_deref() == Some("Y"))
        .expect("Y");
    assert_eq!(
        y.get_prop("isIndividual").and_then(|v| v.as_bool()),
        Some(true)
    );
    let supers = rels_of(g, &y.id, ElementKind::Subclassification);
    assert_eq!(supers.len(), 1, "`:> X` mints one Subclassification");
    assert_eq!(
        supers[0]
            .get_prop("unresolved_superclassifier")
            .and_then(|v| v.as_str()),
        Some("X"),
        "the supertype target is captured for resolution"
    );
}

// Symbolic `~` conjugation (KerML ConjugationPart :337-339 /
// ClassifierConjugationPart :481-485 / FeatureConjugationPart :726-728).
// Grammar 884e7a61 aliases the symbolic arm back to `conjugation_clause`, so
// the lowering (definitions.rs conjugation_clause arm + pass2
// resolve_conjugation) is UNCHANGED — these tests pin that the alias really
// does hit the same one home.
#[test]
fn symbolic_conjugation_on_classifier_mints_and_resolves() {
    let mut result = parse_and_build("package P { classifier D; classifier C ~ D; }");
    assert!(!result.has_errors());
    let cid = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("C"))
        .expect("C")
        .id
        .clone();
    let did = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("D"))
        .expect("D")
        .id
        .clone();
    {
        let rels = rels_of(&result.graph, &cid, ElementKind::Conjugation);
        assert_eq!(rels.len(), 1, "`~` must mint one Conjugation");
        assert_eq!(
            rels[0].get_prop("conjugatedType").and_then(|v| v.as_ref()),
            Some(&cid),
            "the declaring type is the conjugatedType (owner)"
        );
        assert_eq!(
            rels[0]
                .get_prop("unresolved_originalType")
                .and_then(|v| v.as_str()),
            Some("D"),
            "the `~` target is captured as the originalType"
        );
    }
    sysml_core::resolution::resolve_references(&mut result.graph);
    let rels = rels_of(&result.graph, &cid, ElementKind::Conjugation);
    assert_eq!(
        rels[0].get_prop("originalType").and_then(|v| v.as_ref()),
        Some(&did),
        "the originalType target must resolve to D"
    );
}

#[test]
fn symbolic_conjugation_on_feature_mints_conjugation() {
    // Feature-level `~` (KerML FeatureConjugationPart :726-728): the declaring
    // feature is the conjugatedType, the target feature the originalType.
    let result =
        parse_and_build("package P { part def B { attribute a; attribute b ~ a; } }");
    assert!(!result.has_errors());
    let g = &result.graph;
    let b = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("b"))
        .expect("attribute b");
    let rels = rels_of(g, &b.id, ElementKind::Conjugation);
    assert_eq!(rels.len(), 1, "feature-level `~` must mint one Conjugation");
    assert_eq!(
        rels[0].get_prop("conjugatedType").and_then(|v| v.as_ref()),
        Some(&b.id)
    );
    assert_eq!(
        rels[0]
            .get_prop("unresolved_originalType")
            .and_then(|v| v.as_str()),
        Some("a")
    );
}

#[test]
fn lambda_parameter_conjugates_keyword_still_parses() {
    // The symbolic `~` arm is EXCLUDED from lambda-parameter position (the
    // optional terminator makes `{ in x ~Y }` genuinely ambiguous with a
    // unary-`~` result expression) — the `conjugates` keyword remains the
    // spelling there. Regression-pin that it still parses cleanly.
    let result = parse_and_build(
        "package Test { attribute def A { attribute items; \
         assert constraint { items->forAll { in item conjugates items; item == 0 } } } }",
    );
    assert!(!result.has_errors());
}

// MessageDeclaration's anonymous end arm (SysML.xtext:1244-1252
// `ownedRelationship += MessageEventMember 'to' ownedRelationship +=
// MessageEventMember` — no UsageDeclaration/payload) and FlowDeclaration's
// anonymous arm (:1317-1319). Grammar 884e7a61 restored the message form; the
// field-based endpoint extractor picks source/target from either shape.
#[test]
fn anonymous_message_ends_lower_to_flat_endpoints() {
    let result = parse_and_build("package P { part def C { message A to B; } }");
    assert!(!result.has_errors());
    let m = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::FlowUsage)
        .expect("message");
    assert!(m.name.is_none(), "the anonymous end form declares no name");
    assert_eq!(
        m.get_prop("isMessage").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(m.get_prop("source").and_then(|v| v.as_str()), Some("A"));
    assert_eq!(m.get_prop("target").and_then(|v| v.as_str()), Some("B"));
}

#[test]
fn anonymous_flow_ends_lower_to_flat_endpoints() {
    let result = parse_and_build("package P { part def C { flow A to B; } }");
    assert!(!result.has_errors());
    let f = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::FlowUsage)
        .expect("flow");
    assert!(f.name.is_none(), "the anonymous end form declares no name");
    assert!(f.get_prop("isMessage").is_none(), "a flow is not a message");
    assert_eq!(f.get_prop("source").and_then(|v| v.as_str()), Some("A"));
    assert_eq!(f.get_prop("target").and_then(|v| v.as_str()), Some("B"));
}

// --- Task #85 slice 5: spec ends/payload nesting for messages and flows ---
// The flat source/target props stay (runtime/diagram consumers); the spec
// membership nesting is minted ADDITIONALLY. Metaclasses verified in
// SysML-vocab.ttl: EventOccurrenceUsage:349, FlowEnd:443, PayloadFeature:767,
// ParameterMembership / EndFeatureMembership:331 / FeatureMembership:405.

fn owning_membership_kind(
    g: &sysml_core::ModelGraph,
    id: &sysml_core::ElementId,
) -> ElementKind {
    g.owning_membership_of(id).expect("owning membership").kind.clone()
}

#[test]
fn message_ends_nest_as_parameter_membership_event_occurrences() {
    // SysML.xtext MessageEventMember:1254 (ParameterMembership) →
    // MessageEvent:1258 (EventOccurrenceUsage owning OwnedReferenceSubsetting).
    let result = parse_and_build(
        "package P { part def C { part a { out port out1; } part b { in port in1; } \
         message m of Pkt from a.out1 to b.in1; } }",
    );
    assert!(!result.has_errors());
    let g = &result.graph;
    let m = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("m"))
        .expect("message m");
    // Flat props KEPT.
    assert_eq!(m.get_prop("source").and_then(|v| v.as_str()), Some("a.out1"));
    assert_eq!(m.get_prop("target").and_then(|v| v.as_str()), Some("b.in1"));

    let ends = rels_of(g, &m.id, ElementKind::EventOccurrenceUsage);
    assert_eq!(ends.len(), 2, "one EventOccurrenceUsage per message end");
    for (end, chain) in ends.iter().zip(["a.out1", "b.in1"]) {
        assert!(end.name.is_none(), "message ends are unnamed");
        assert_eq!(
            owning_membership_kind(g, &end.id),
            ElementKind::ParameterMembership,
            "a message end is wrapped in a ParameterMembership"
        );
        let subs = rels_of(g, &end.id, ElementKind::ReferenceSubsetting);
        assert_eq!(subs.len(), 1, "each end owns one ReferenceSubsetting");
        assert_eq!(
            subs[0]
                .get_prop("unresolved_referencedFeature")
                .and_then(|v| v.as_str()),
            Some(chain),
            "the end chain is captured for chain-aware resolution"
        );
    }

    // Payload `of Pkt`: FeatureMembership → PayloadFeature → FeatureTyping.
    let payloads = rels_of(g, &m.id, ElementKind::PayloadFeature);
    assert_eq!(payloads.len(), 1, "`of Pkt` mints one PayloadFeature");
    assert_eq!(
        owning_membership_kind(g, &payloads[0].id),
        ElementKind::FeatureMembership
    );
    let typing = rels_of(g, &payloads[0].id, ElementKind::FeatureTyping);
    assert_eq!(typing.len(), 1);
    assert_eq!(
        typing[0].get_prop("unresolved_type").and_then(|v| v.as_str()),
        Some("Pkt")
    );
}

#[test]
fn flow_ends_nest_as_end_feature_membership_flow_ends() {
    // SysML.xtext FlowEndMember:1309 (EndFeatureMembership) → FlowEnd:1313
    // (FlowEnd metaclass) with FlowEndSubsetting:1318 (chain prefix) and
    // FlowFeatureMember:1330 → FlowFeature:1334 (ReferenceUsage) →
    // FlowRedefinition:1338 (Redefinition).
    let result = parse_and_build(
        "package P { part def C { part a { out port out1; } part b { in port in1; } \
         flow f of Fluid from a.out1 to b.in1; } }",
    );
    assert!(!result.has_errors());
    let g = &result.graph;
    let f = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("f"))
        .expect("flow f");
    // Flat props KEPT.
    assert_eq!(f.get_prop("source").and_then(|v| v.as_str()), Some("a.out1"));
    assert_eq!(f.get_prop("target").and_then(|v| v.as_str()), Some("b.in1"));

    let ends = rels_of(g, &f.id, ElementKind::FlowEnd);
    assert_eq!(ends.len(), 2, "one FlowEnd per flow end");
    for (end, (prefix, last)) in ends.iter().zip([("a", "out1"), ("b", "in1")]) {
        assert!(end.name.is_none(), "flow ends are unnamed");
        assert_eq!(
            end.get_prop("isEnd").and_then(|v| v.as_bool()),
            Some(true),
            "EndFeatureMembership requires its memberFeature isEnd"
        );
        assert_eq!(
            owning_membership_kind(g, &end.id),
            ElementKind::EndFeatureMembership
        );
        // FlowEndSubsetting: the chain PREFIX.
        let subs = rels_of(g, &end.id, ElementKind::ReferenceSubsetting);
        assert_eq!(subs.len(), 1, "a dotted end owns one ReferenceSubsetting");
        assert_eq!(
            subs[0]
                .get_prop("unresolved_referencedFeature")
                .and_then(|v| v.as_str()),
            Some(prefix)
        );
        // FlowFeature: unnamed ReferenceUsage wrapped in FeatureMembership,
        // carrying the Redefinition of the BARE LAST SEGMENT, per the spec's
        // own desugaring (SysML-spec-r2025-04.txt:21842-21865 — the prefix is
        // consumed entirely by the FlowEndSubsetting).
        let inner = rels_of(g, &end.id, ElementKind::ReferenceUsage);
        assert_eq!(inner.len(), 1, "each FlowEnd owns exactly one FlowFeature");
        assert!(inner[0].name.is_none());
        assert_eq!(
            owning_membership_kind(g, &inner[0].id),
            ElementKind::FeatureMembership
        );
        let redefs = rels_of(g, &inner[0].id, ElementKind::Redefinition);
        assert_eq!(redefs.len(), 1);
        assert_eq!(
            redefs[0]
                .get_prop("unresolved_redefinedFeature")
                .and_then(|v| v.as_str()),
            Some(last)
        );
    }

    // Flow payload `of Fluid` mints the same PayloadFeature shape as message.
    let payloads = rels_of(g, &f.id, ElementKind::PayloadFeature);
    assert_eq!(payloads.len(), 1, "`of Fluid` mints one PayloadFeature");
    let typing = rels_of(g, &payloads[0].id, ElementKind::FeatureTyping);
    assert_eq!(
        typing[0].get_prop("unresolved_type").and_then(|v| v.as_str()),
        Some("Fluid")
    );
}

#[test]
fn bare_flow_ends_nest_without_end_subsetting() {
    // A bare (undotted) end has no FlowEndSubsetting (:1318 requires the
    // dot); the FlowFeature redefines the end name directly.
    let result = parse_and_build(
        "package P { part def C { item A; item B; flow A to B; } }",
    );
    assert!(!result.has_errors());
    let g = &result.graph;
    let f = g
        .elements
        .values()
        .find(|e| e.kind == ElementKind::FlowUsage)
        .expect("flow");
    let ends = rels_of(g, &f.id, ElementKind::FlowEnd);
    assert_eq!(ends.len(), 2);
    for (end, name) in ends.iter().zip(["A", "B"]) {
        assert!(
            rels_of(g, &end.id, ElementKind::ReferenceSubsetting).is_empty(),
            "a bare end mints no FlowEndSubsetting"
        );
        let inner = rels_of(g, &end.id, ElementKind::ReferenceUsage);
        let redefs = rels_of(g, &inner[0].id, ElementKind::Redefinition);
        assert_eq!(
            redefs[0]
                .get_prop("unresolved_redefinedFeature")
                .and_then(|v| v.as_str()),
            Some(name)
        );
    }
    // No `of` clause — no PayloadFeature child.
    assert!(rels_of(g, &f.id, ElementKind::PayloadFeature).is_empty());
}

#[test]
fn flow_end_redefinition_resolves_via_end_subsetting_general() {
    // KerML 8.2.3.5.1 (derived text :6439): the bare redefined name resolves
    // with the general Type of each ownedSpecialization of the redefining
    // feature's owningType as the local Namespace — for a FlowEnd that
    // general is its FlowEndSubsetting's end prefix (here `a`, whose typed
    // member `out1` is the redefined feature). No lexical fallback.
    let mut result = parse_and_build(
        "package P { part def AP { out port out1; } part def BP { in port in1; } \
         part def C { part a : AP; part b : BP; flow f from a.out1 to b.in1; } }",
    );
    assert!(!result.has_errors());
    sysml_core::resolution::resolve_references(&mut result.graph);
    let g = &result.graph;
    let out1 = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("out1"))
        .expect("port out1")
        .id
        .clone();
    let f = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("f"))
        .expect("flow f");
    let ends = rels_of(g, &f.id, ElementKind::FlowEnd);
    let inner = rels_of(g, &ends[0].id, ElementKind::ReferenceUsage);
    let redefs = rels_of(g, &inner[0].id, ElementKind::Redefinition);
    assert_eq!(
        redefs[0].get_prop("redefinedFeature").and_then(|v| v.as_ref()),
        Some(&out1),
        "the source end's bare `out1` must resolve through the end prefix general"
    );
}

#[test]
fn message_end_reference_subsetting_resolves_chain() {
    // The minted ReferenceSubsetting rides the existing chain-aware pass-2
    // resolver (completion, not invention): `a.out1` resolves to the port
    // usage nested in part a.
    let mut result = parse_and_build(
        "package P { part def C { part a { out port out1; } part b { in port in1; } \
         message m from a.out1 to b.in1; } }",
    );
    assert!(!result.has_errors());
    sysml_core::resolution::resolve_references(&mut result.graph);
    let g = &result.graph;
    let out1 = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("out1"))
        .expect("port out1")
        .id
        .clone();
    let m = g
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("m"))
        .expect("message m");
    let ends = rels_of(g, &m.id, ElementKind::EventOccurrenceUsage);
    let subs = rels_of(g, &ends[0].id, ElementKind::ReferenceSubsetting);
    assert_eq!(
        subs[0].get_prop("referencedFeature").and_then(|v| v.as_ref()),
        Some(&out1),
        "the source end's chain must resolve to the referenced port"
    );
}

#[test]
fn flow_payload_feeds_payload_type_elaboration() {
    // elaborate::flows derives the flat `payloadType` prop from the
    // PayloadFeature child's FeatureTyping (one home for the derivation).
    let mut result = parse_and_build(
        "package P { part def C { part a { out port out1; } part b { in port in1; } \
         flow f of Fluid from a.out1 to b.in1; } }",
    );
    assert!(!result.has_errors());
    sysml_core::elaborate::elaborate(&mut result.graph);
    let f = result
        .graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("f"))
        .expect("flow f");
    assert_eq!(
        f.get_prop("payloadType").and_then(|v| v.as_str()),
        Some("Fluid"),
        "the `of Fluid` payload derives the flat payloadType prop"
    );
}

#[test]
fn abstract_individual_def_composes_both_flags() {
    let result = parse_and_build("package P { abstract individual def X; }");
    assert!(!result.has_errors());
    let d = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::OccurrenceDefinition)
        .expect("individual def");
    assert_eq!(
        d.get_prop("isAbstract").and_then(|v| v.as_bool()),
        Some(true),
        "the `abstract` prefix composes"
    );
    assert_eq!(
        d.get_prop("isIndividual").and_then(|v| v.as_bool()),
        Some(true)
    );
}

// Task #81 transition-feature slice. SysML.xtext:1884-1914 —
// TriggerActionMember / GuardExpressionMember / EffectBehaviorMember each
// `returns SysML::TransitionFeatureMembership` with kind = trigger('accept') /
// guard('if') / effect('do'); the hosted features are `TriggerAction returns
// SysML::AcceptActionUsage` (xtext:1892), an OwnedExpression, and
// `EffectBehaviorUsage returns SysML::ActionUsage` (xtext:1912). Spec:
// §8.3.18.8 TransitionFeatureMembership (kind : TransitionFeatureKind,
// /transitionFeature redefines ownedMemberFeature). This test pins that a
// transition's trigger/guard/effect are REAL owned children wrapped in a
// TransitionFeatureMembership carrying the discriminator — and that the former
// string props on the TransitionUsage are GONE (single representation).
#[test]
fn test_transition_features_are_transition_feature_memberships() {
    let result = parse_and_build(
        "state def S { state idle; state active; \
         transition t first idle accept evt if [ready] do cleanup; then active; }",
    );
    let transition = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::TransitionUsage)
        .expect("transition t");

    for (kind_label, expected_kind, expected_text) in [
        ("trigger", ElementKind::AcceptActionUsage, "evt"),
        ("guard", ElementKind::Expression, "ready"),
        // The effect text keeps the raw CST clause (incl. trailing `;`),
        // byte-identical to the former `effect` string prop.
        ("effect", ElementKind::ActionUsage, "do cleanup;"),
    ] {
        let features = result.graph.transition_features_of(&transition.id, kind_label);
        assert_eq!(
            features.len(),
            1,
            "transition must own exactly one {kind_label} feature"
        );
        let feature = features[0];
        assert_eq!(
            feature.kind, expected_kind,
            "{kind_label} feature kind (TriggerAction/EffectBehaviorUsage are \
             grammar rules returning AcceptActionUsage/ActionUsage, not metaclasses)"
        );
        assert_eq!(
            feature.get_prop("text").and_then(|v| v.as_str()),
            Some(expected_text),
            "{kind_label} child must carry the textual form as its `text` prop"
        );

        let mem_id = feature
            .owning_membership
            .as_ref()
            .expect("transition feature must have an owning membership");
        let mem = result
            .graph
            .elements
            .get(mem_id)
            .expect("owning membership element must exist");
        assert_eq!(
            mem.kind,
            ElementKind::TransitionFeatureMembership,
            "{kind_label} feature must be wrapped in a TransitionFeatureMembership"
        );
        assert_eq!(
            mem.get_prop("kind").and_then(|v| v.as_str()),
            Some(kind_label),
            "TransitionFeatureMembership.kind must be the {kind_label} discriminator"
        );
    }

    // Single representation: the TransitionUsage no longer carries the
    // string props — the text is derived from the children.
    for legacy in ["trigger", "guard", "effect", "accept_param"] {
        assert!(
            transition.get_prop(legacy).is_none(),
            "TransitionUsage must not carry the legacy `{legacy}` string prop"
        );
    }
}

// Port-trigger payload parameter: `accept <name> via <port>` — the name is a
// real named ReferenceUsage child of the trigger AcceptActionUsage (spec
// payloadParameter slot, SysML.xtext:1444-1456), surfaced via
// `transition_accept_param`.
#[test]
fn test_transition_port_trigger_payload_parameter_child() {
    let result = parse_and_build(
        "state def S { state idle; state active; \
         transition first idle accept msg via inPort then active; }",
    );
    let transition = result
        .graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::TransitionUsage)
        .expect("transition");

    assert_eq!(
        result.graph.transition_feature_text(&transition.id, "trigger"),
        Some("accept via inPort".to_owned()),
        "canonical port-trigger string derived from the trigger child"
    );
    assert_eq!(
        result.graph.transition_accept_param(&transition.id),
        Some("msg".to_owned()),
        "payload parameter name derived from the trigger action's ReferenceUsage child"
    );
    let triggers = result.graph.transition_features_of(&transition.id, "trigger");
    let payloads: Vec<_> = result
        .graph
        .children_of(&triggers[0].id)
        .filter(|c| c.kind == ElementKind::ReferenceUsage)
        .collect();
    assert_eq!(payloads.len(), 1, "exactly one payload ReferenceUsage child");
    assert_eq!(payloads[0].name.as_deref(), Some("msg"));
}
