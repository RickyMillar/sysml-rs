//! Constraint elaboration.
//!
//! Post-Phase-6D.3: the parser emits a structured expression subtree under
//! every `ConstraintUsage` / `AssertConstraintUsage` / `ConstraintDefinition`.
//! This pass pretty-prints that subtree into the legacy `constraint` / `expr`
//! string properties. Most production readers now use AST-first helpers
//! (`compile_expression`, `pretty_print_owner`), so the string props are
//! mainly a convenience for hand-crafted test graphs and for the remaining
//! diagram / LSP display fallbacks.

use super::ElaborationReport;
use crate::expression_pretty::pretty_print_owner;
use crate::{ElementId, ElementKind, ModelGraph, Value};

/// Elaborate constraints: copy values, tag roles, and propagate negation.
pub(super) fn elaborate_constraints(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    copy_constraint_values(graph, report);
    tag_constraint_roles(graph, report);
    tag_negated_constraints(graph, report);
}

/// Populate the `constraint` string prop from the AST subtree (or legacy
/// `unresolved_value` if still set by a non-parser producer) so downstream
/// string-first consumers keep working.
///
/// The spec-aligned prop for ConstraintUsage/Definition/Assert is
/// `constraint`. The `expr` prop belongs to CalculationUsage/Definition —
/// the parser sets it on those kinds directly. This pass is intentionally
/// ConstraintUsage-family only and writes a single prop; earlier versions
/// wrote both, which caused extractors to emit duplicate ConstraintIRs.
fn copy_constraint_values(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut constraint_ids = Vec::new();
    constraint_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConstraintUsage));
    constraint_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::AssertConstraintUsage));
    constraint_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConstraintDefinition));

    let to_elaborate: Vec<(ElementId, String)> = constraint_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter_map(|e| {
            if e.get_prop("constraint").is_some() {
                return None;
            }
            if let Some(text) = pretty_print_owner(e, graph) {
                return Some((e.id.clone(), text));
            }
            let value = e.get_prop("unresolved_value")?.as_str()?.to_owned();
            Some((e.id.clone(), value))
        })
        .collect();

    for (id, value) in to_elaborate {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("constraint", Value::String(value));
            report.constraints_derived += 1;
        }
    }
}

/// Tag `role` property on `ConstraintUsage` children of requirement elements.
///
/// The downstream constraint evaluator reads `child.get_prop("role")` expecting
/// `"assume"` or `"require"`. The parser may set `constraintKind` to `"assumption"`
/// or `"requirement"` on constraint children of requirements. This pass normalises
/// that into the `role` property.
fn tag_constraint_roles(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Find requirement-family element IDs
    let mut req_ids: Vec<ElementId> = Vec::new();
    req_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::RequirementUsage));
    req_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::RequirementDefinition));
    req_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::SatisfyRequirementUsage));

    // Collect (child_id, role) pairs for constraint children that need tagging
    let mut to_tag: Vec<(ElementId, String)> = Vec::new();

    for req_id in &req_ids {
        for child in graph.children_of(req_id) {
            if child.kind != ElementKind::ConstraintUsage
                && child.kind != ElementKind::AssertConstraintUsage
                && child.kind != ElementKind::ConstraintDefinition
            {
                continue;
            }
            // Skip if role already set
            if child.get_prop("role").is_some() {
                continue;
            }
            // Check for constraintKind set by the parser
            if let Some(kind_str) = child.get_prop("constraintKind").and_then(|v| v.as_str()) {
                let role = match kind_str {
                    "assumption" => "assume",
                    "requirement" => "require",
                    _ => continue,
                };
                to_tag.push((child.id.clone(), role.to_owned()));
            }
        }
    }

    for (id, role) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("role", Value::String(role));
            report.constraints_derived += 1;
        }
    }
}

/// Tag `isNegated` on `AssertConstraintUsage` elements.
///
/// The parser grammar has `isNegated ?= 'not'` which may already set this
/// property. This pass ensures the property is present as a `Value::Bool` when
/// the parser has set it (possibly as a string "true"), and propagates negation
/// from parent to child assert constraints if applicable.
fn tag_negated_constraints(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Find AssertConstraintUsage elements that might need isNegated normalisation
    let assert_ids: Vec<ElementId> = graph
        .element_ids_by_kind(&ElementKind::AssertConstraintUsage)
        .to_vec();

    let mut to_tag: Vec<ElementId> = Vec::new();

    for id in &assert_ids {
        let Some(elem) = graph.get_element(id) else {
            continue;
        };

        // Check if isNegated is already set as a bool
        if let Some(val) = elem.get_prop("isNegated") {
            if val.as_bool().is_some() {
                // Already a proper bool, nothing to do
                continue;
            }
            // Parser may have set it as a string "true"/"false" — normalise
            if let Some(s) = val.as_str() {
                if s == "true" {
                    to_tag.push(id.clone());
                }
                continue;
            }
        }

        // Check parent: if the owner is also an AssertConstraintUsage with
        // isNegated=true, propagate to this child
        if let Some(owner_id) = &elem.owner {
            if let Some(owner) = graph.get_element(owner_id) {
                if owner.kind == ElementKind::AssertConstraintUsage {
                    if let Some(val) = owner.get_prop("isNegated") {
                        if val.as_bool() == Some(true) {
                            to_tag.push(id.clone());
                        }
                    }
                }
            }
        }
    }

    for id in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("isNegated", Value::Bool(true));
            report.constraints_derived += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    #[test]
    fn copies_unresolved_value_to_constraint() {
        // Phase 6D.1 bridge: hand-crafted test graphs (no AST subtree) with a
        // legacy `unresolved_value` string still flow through elaboration into
        // the `constraint`/`expr` string props that downstream consumers read.
        // Parser-produced graphs exercise the AST-first path (see
        // `copies_ast_subtree_to_constraint` below).
        let mut graph = ModelGraph::new();

        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_prop("unresolved_value", "speed < 100");
        let c_id = graph.add_element(c);

        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("constraint").and_then(|v| v.as_str()),
            Some("speed < 100")
        );
        // `expr` belongs to calc kinds — ConstraintUsage elaboration no
        // longer populates it (the duplicate write was the source of the
        // double-emit bug in the constraint extractors).
        assert!(elem.get_prop("expr").is_none());
    }

    #[test]
    fn copies_ast_subtree_to_constraint() {
        // Phase 6D.1 primary path: a structured expression subtree (as the
        // parser emits it) is pretty-printed into `unresolved_value` /
        // `constraint` / `expr` during elaboration, so legacy string-first
        // consumers keep working.
        use crate::{Element, ModelGraph};

        let mut graph = ModelGraph::new();

        let constraint =
            Element::new_with_kind(ElementKind::ConstraintUsage).with_name("SpeedLimit");
        let c_id = graph.add_element(constraint);

        let op = Element::new_with_kind(ElementKind::OperatorExpression)
            .with_owner(c_id.clone())
            .with_prop("operator", "<")
            .with_prop("argIndex", Value::Int(0));
        let op_id = graph.add_element(op);

        let lhs = Element::new_with_kind(ElementKind::FeatureReferenceExpression)
            .with_name("speed")
            .with_owner(op_id.clone())
            .with_prop("argIndex", Value::Int(0));
        graph.add_element(lhs);

        let rhs = Element::new_with_kind(ElementKind::LiteralInteger)
            .with_owner(op_id)
            .with_prop("value", Value::Int(100))
            .with_prop("argIndex", Value::Int(1));
        graph.add_element(rhs);

        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("constraint").and_then(|v| v.as_str()),
            Some("speed < 100")
        );
        assert!(elem.get_prop("expr").is_none());
    }

    #[test]
    fn does_not_overwrite_existing_constraint() {
        let mut graph = ModelGraph::new();

        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_prop("constraint", "speed < 200")
            .with_prop("unresolved_value", "speed < 100");
        let c_id = graph.add_element(c);

        let report = elaborate(&mut graph);

        assert_eq!(report.constraints_derived, 0);

        // Original value preserved
        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("constraint").and_then(|v| v.as_str()),
            Some("speed < 200")
        );
    }

    #[test]
    fn handles_assert_constraint() {
        let mut graph = ModelGraph::new();

        let c = Element::new_with_kind(ElementKind::AssertConstraintUsage)
            .with_name("SafetyCheck")
            .with_prop("unresolved_value", "temp < 500");
        let c_id = graph.add_element(c);

        let report = elaborate(&mut graph);

        assert_eq!(report.constraints_derived, 1);
        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("constraint").and_then(|v| v.as_str()),
            Some("temp < 500")
        );
    }

    // --- A4: tag_constraint_roles tests ---

    #[test]
    fn tags_assume_role_on_constraint() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("SafetyReq");
        let req_id = graph.add_element(req);

        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("Assumption1")
            .with_owner(req_id.clone())
            .with_prop("constraintKind", "assumption");
        let c_id = graph.add_element(c);

        let report = elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("role").and_then(|v| v.as_str()),
            Some("assume")
        );
        assert!(report.constraints_derived >= 1);
    }

    #[test]
    fn tags_require_role_on_constraint() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementDefinition).with_name("PerfReq");
        let req_id = graph.add_element(req);

        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedReq")
            .with_owner(req_id.clone())
            .with_prop("constraintKind", "requirement");
        let c_id = graph.add_element(c);

        let report = elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("role").and_then(|v| v.as_str()),
            Some("require")
        );
        assert!(report.constraints_derived >= 1);
    }

    #[test]
    fn does_not_overwrite_existing_role() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("SafetyReq");
        let req_id = graph.add_element(req);

        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("AlreadyTagged")
            .with_owner(req_id.clone())
            .with_prop("constraintKind", "assumption")
            .with_prop("role", "require"); // pre-set to different value
        let c_id = graph.add_element(c);

        elaborate(&mut graph);

        // Should keep the original value, not overwrite
        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("role").and_then(|v| v.as_str()),
            Some("require")
        );
    }

    #[test]
    fn tags_role_under_satisfy_requirement() {
        let mut graph = ModelGraph::new();

        let sat =
            Element::new_with_kind(ElementKind::SatisfyRequirementUsage).with_name("SatisfyReq");
        let sat_id = graph.add_element(sat);

        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("Assumption")
            .with_owner(sat_id.clone())
            .with_prop("constraintKind", "assumption");
        let c_id = graph.add_element(c);

        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("role").and_then(|v| v.as_str()),
            Some("assume")
        );
    }

    // --- A5: tag_negated_constraints tests ---

    #[test]
    fn preserves_existing_negated_bool() {
        let mut graph = ModelGraph::new();

        let c = Element::new_with_kind(ElementKind::AssertConstraintUsage)
            .with_name("NegatedCheck")
            .with_prop("isNegated", Value::Bool(true));
        let c_id = graph.add_element(c);

        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("isNegated").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn normalises_string_negated_to_bool() {
        let mut graph = ModelGraph::new();

        // Parser might set isNegated as a string "true"
        let c = Element::new_with_kind(ElementKind::AssertConstraintUsage)
            .with_name("NegatedCheck")
            .with_prop("isNegated", "true");
        let c_id = graph.add_element(c);

        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("isNegated").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn propagates_negation_from_parent() {
        let mut graph = ModelGraph::new();

        let parent = Element::new_with_kind(ElementKind::AssertConstraintUsage)
            .with_name("ParentAssert")
            .with_prop("isNegated", Value::Bool(true));
        let parent_id = graph.add_element(parent);

        let child = Element::new_with_kind(ElementKind::AssertConstraintUsage)
            .with_name("ChildAssert")
            .with_owner(parent_id.clone());
        let child_id = graph.add_element(child);

        elaborate(&mut graph);

        let elem = graph.get_element(&child_id).unwrap();
        assert_eq!(
            elem.get_prop("isNegated").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn does_not_negate_without_indicator() {
        let mut graph = ModelGraph::new();

        let c =
            Element::new_with_kind(ElementKind::AssertConstraintUsage).with_name("NormalAssert");
        let c_id = graph.add_element(c);

        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert!(elem.get_prop("isNegated").is_none());
    }
}
