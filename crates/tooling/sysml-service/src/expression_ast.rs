//! Expression AST projection — walks expression-element subtrees in a
//! `ModelGraph` and serializes them as `ExpressionAstNode` trees for
//! frontend rendering (KaTeX, inspectors, MCP consumers).
//!
//! This is a pure **model projection**: it does not involve the runtime
//! `ExprIR` or evaluation. It mirrors the walk that
//! `sysml_runtime::expressions::compiler::compile_expression_ast` performs,
//! including sorting children by `argIndex`.
//!
//! Elements are considered "expression-bearing" (suitable as projection
//! roots) when they are a `ConstraintUsage`/`ConstraintDefinition`,
//! `AssertConstraintUsage`, `CalculationUsage`/`CalculationDefinition`,
//! or `AttributeUsage` with a value, and they have at least one
//! expression-element child.

use std::collections::{BTreeMap, HashSet};

use sysml_core::{is_expression_kind, Element, ElementId, ElementKind, ModelGraph, Value};
use sysml_runtime::expressions::compile_expression_ast;
use sysml_span::Diagnostic;

use crate::types::{ExpressionAstNode, ExpressionAstResult};

const DEFAULT_PROP_KEYS: &[&str] = &["argIndex"];

/// Is this element kind a potential projection root (carries an
/// expression body)?
fn is_expression_owner(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::ConstraintUsage
            | ElementKind::ConstraintDefinition
            | ElementKind::AssertConstraintUsage
            | ElementKind::CalculationUsage
            | ElementKind::CalculationDefinition
            | ElementKind::AttributeUsage
            | ElementKind::AttributeDefinition
    )
}

/// Project a single expression element subtree into an `ExpressionAstNode`.
fn project_node(element: &Element, graph: &ModelGraph) -> ExpressionAstNode {
    let mut tagged: Vec<(i64, &Element)> = graph
        .children_of(&element.id)
        .filter(|c| is_expression_kind(&c.kind))
        .map(|c| {
            let idx = c
                .get_prop("argIndex")
                .and_then(|v| v.as_int())
                .unwrap_or(i64::MAX);
            (idx, c)
        })
        .collect();
    tagged.sort_by_key(|(idx, _)| *idx);

    let children = tagged
        .into_iter()
        .map(|(_, child)| project_node(child, graph))
        .collect();

    let mut props: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let skip: BTreeMap<&str, ()> = DEFAULT_PROP_KEYS.iter().map(|k| (*k, ())).collect();
    for (k, v) in &element.props {
        if skip.contains_key(k.as_ref()) {
            continue;
        }
        props.insert(k.to_string(), v.clone());
    }

    ExpressionAstNode {
        kind: format!("{:?}", element.kind),
        name: element.name.clone(),
        props,
        children,
    }
}

/// Build an `ExpressionAstResult` for an owner element (constraint, attr,
/// calc, ...). Returns `None` if the element isn't an expression owner.
///
/// The `ast` field is `None` when the element has no structured
/// expression children (e.g. an attribute whose value is literal-only
/// or already represented by a direct literal child).
pub fn project_owner(element: &Element, graph: &ModelGraph) -> Option<ExpressionAstResult> {
    if !is_expression_owner(&element.kind) {
        return None;
    }

    let root = graph
        .children_of(&element.id)
        .filter(|c| is_expression_kind(&c.kind))
        .min_by_key(|c| {
            c.get_prop("argIndex")
                .and_then(|v| v.as_int())
                .unwrap_or(i64::MAX)
        })
        .map(|c| project_node(c, graph));

    // Source display string: pretty-print the AST subtree (canonical form).
    // Note: `unresolved_value` is no longer written by either parser
    // (removed in Phase 6D); the AST path is the only live source.
    let source = sysml_core::expression_pretty::pretty_print_owner(element, graph);

    Some(ExpressionAstResult {
        element_id: element.id.clone(),
        element_name: element.name.clone(),
        element_kind: format!("{:?}", element.kind),
        source,
        ast: root,
    })
}

/// Project every expression-bearing element in the graph.
pub fn project_all(graph: &ModelGraph) -> Vec<ExpressionAstResult> {
    graph
        .elements
        .values()
        .filter(|e| is_expression_owner(&e.kind))
        .filter_map(|e| project_owner(e, graph))
        .collect()
}

/// Project a single element by id.
pub fn project_one(graph: &ModelGraph, id: &ElementId) -> Option<ExpressionAstResult> {
    let element = graph.get_element(id)?;
    project_owner(element, graph)
}

/// Collect all free variables referenced by the expression owned by
/// `element` (a constraint/calc/attribute body). Returns `Err` if the
/// element has structured expression children but compilation fails.
///
/// Delegates to `compile_expression_ast` + `ExprIR::free_variables()` —
/// correctly excludes bound variables from collection ops (select,
/// collect, forAll, exists, reject).
pub fn free_variables_of(
    element: &Element,
    graph: &ModelGraph,
) -> Result<HashSet<String>, Vec<Diagnostic>> {
    let expr = compile_expression_ast(element, graph)?;
    Ok(expr.free_variables())
}

/// Plain-text pretty-printer for `ExpressionAstNode`. Intended for hover
/// tooltips, diagram labels, and any consumer that wants a source-like
/// rendering without building a full KaTeX pipeline.
///
/// Parenthesization follows a conservative "when in doubt, add parens"
/// policy: `a + b * c` prints as `a + b * c`, `(a + b) * c` prints as
/// `(a + b) * c`. The precedence table and render rules live in
/// `sysml_core::expression_pretty` — this function just adapts an
/// `ExpressionAstNode` (JSON projection) into the shared
/// `PrettyPrintTarget` trait so both the graph-based and projection-based
/// callers go through a single formatter.
pub fn pretty_print(node: &ExpressionAstNode) -> String {
    sysml_core::expression_pretty::pretty_print(node)
}

impl sysml_core::expression_pretty::PrettyPrintTarget for ExpressionAstNode {
    fn kind(&self) -> &str {
        self.kind.as_str()
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn prop_value(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }

    fn sorted_children(
        &self,
    ) -> Vec<Box<dyn sysml_core::expression_pretty::PrettyPrintTarget + '_>> {
        // Projection already stores children in argIndex order.
        self.children
            .iter()
            .map(|c| Box::new(c) as Box<dyn sysml_core::expression_pretty::PrettyPrintTarget + '_>)
            .collect()
    }
}

impl sysml_core::expression_pretty::PrettyPrintTarget for &ExpressionAstNode {
    fn kind(&self) -> &str {
        (*self).kind()
    }

    fn name(&self) -> Option<&str> {
        (*self).name()
    }

    fn prop_value(&self, key: &str) -> Option<&Value> {
        (*self).prop_value(key)
    }

    fn sorted_children(
        &self,
    ) -> Vec<Box<dyn sysml_core::expression_pretty::PrettyPrintTarget + '_>> {
        (*self).sorted_children()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use sysml_parser_incremental::TreeSitterParser;
    use sysml_parser_trait::{Parser, SysmlFile};

    fn parse(src: &str) -> ModelGraph {
        let parser = TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile {
            path: "t.sysml".into(),
            text: src.to_string(),
        }]);
        result.graph
    }

    #[test]
    fn projects_simple_constraint() {
        let graph = parse(
            r#"
            package P {
                part def Thing {
                    attribute x: Real;
                    constraint k { x >= 0.0 }
                }
            }
            "#,
        );
        let all = project_all(&graph);
        let k = all
            .iter()
            .find(|r| r.element_name.as_deref() == Some("k"))
            .expect("constraint k must project");
        let ast = k.ast.as_ref().expect("k must have ast");
        assert_eq!(ast.kind, "OperatorExpression");
        assert_eq!(
            ast.props.get("operator").and_then(|v| v.as_str()),
            Some(">=")
        );
        assert_eq!(ast.children.len(), 2);
    }

    #[test]
    fn project_one_matches_project_all() {
        let graph = parse(
            r#"
            package P {
                part def Thing {
                    attribute x: Real;
                    constraint k { x >= 0.0 }
                }
            }
            "#,
        );
        let all = project_all(&graph);
        let k = all
            .iter()
            .find(|r| r.element_name.as_deref() == Some("k"))
            .unwrap();
        let single = project_one(&graph, &k.element_id).unwrap();
        assert_eq!(single.element_id, k.element_id);
        assert_eq!(single.ast.is_some(), k.ast.is_some());
    }

    fn find_named<'a>(graph: &'a ModelGraph, name: &str) -> Option<&'a Element> {
        graph.elements.values().find(|e| e.name.as_deref() == Some(name))
    }

    #[test]
    fn free_variables_simple_constraint() {
        let graph = parse(
            r#"
            package P {
                part def Thing {
                    attribute x: Real;
                    attribute y: Real;
                    constraint k { x + y >= 0.0 }
                }
            }
            "#,
        );
        let k = find_named(&graph, "k").expect("k exists");
        let vars = free_variables_of(k, &graph).expect("compiles");
        assert!(vars.contains("x"), "free vars: {vars:?}");
        assert!(vars.contains("y"), "free vars: {vars:?}");
    }

    #[test]
    fn pretty_print_respects_precedence() {
        let graph = parse(
            r#"
            package P {
                part def Thing {
                    attribute a: Real;
                    attribute b: Real;
                    attribute c: Real;
                    constraint k1 { a + b * c >= 0.0 }
                    constraint k2 { (a + b) * c >= 0.0 }
                }
            }
            "#,
        );
        let k1 = find_named(&graph, "k1").unwrap();
        let k2 = find_named(&graph, "k2").unwrap();
        let r1 = project_owner(k1, &graph).unwrap();
        let r2 = project_owner(k2, &graph).unwrap();
        let p1 = pretty_print(r1.ast.as_ref().unwrap());
        let p2 = pretty_print(r2.ast.as_ref().unwrap());
        // mul binds tighter: no parens in k1
        assert!(!p1.contains("(a + b)"), "p1 = {p1}");
        // parenthesized addition in k2 needs parens
        assert!(p2.contains("(a + b)"), "p2 = {p2}");
    }

    #[test]
    fn pretty_print_literals_and_refs() {
        let graph = parse(
            r#"
            package P {
                part def Thing {
                    attribute temp: Real;
                    constraint k { temp >= 90.0 and temp <= 96.0 }
                }
            }
            "#,
        );
        let k = find_named(&graph, "k").unwrap();
        let r = project_owner(k, &graph).unwrap();
        let p = pretty_print(r.ast.as_ref().unwrap());
        assert!(p.contains("temp"), "p = {p}");
        assert!(p.contains(">="), "p = {p}");
        assert!(p.contains("<="), "p = {p}");
        assert!(p.contains("and"), "p = {p}");
    }
}
