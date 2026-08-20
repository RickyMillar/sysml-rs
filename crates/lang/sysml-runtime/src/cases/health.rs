use crate::expressions::compile_expression;
use sysml_core::element_ordering::primary_span;
use sysml_core::{ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_span::Diagnostic;

/// Diagnose verification-case health issues across all verification cases in a graph.
///
/// This pass is intended for editor diagnostics and preflight checks before
/// verification execution.
pub fn verification_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Iterate verification case definitions and usages
    for elem in graph
        .elements_by_kind(&ElementKind::VerificationCaseDefinition)
        .chain(graph.elements_by_kind(&ElementKind::VerificationCaseUsage))
    {
        let case_name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());

        // Collect direct requirement children
        let req_children: Vec<_> = graph
            .children_of(&elem.id)
            .filter(|c| {
                c.kind == ElementKind::RequirementUsage
                    || c.kind == ElementKind::RequirementDefinition
            })
            .collect();

        // Also check inside ObjectiveMembership children for RequirementVerificationMembership.
        // Per spec, verification cases use `objective { verify SomeReq; }` which creates:
        //   VerificationCaseDefinition → ObjectiveMembership → RequirementVerificationMembership
        let has_objective_with_verification = graph
            .children_of(&elem.id)
            .filter(|c| c.kind == ElementKind::ObjectiveMembership)
            .any(|obj| {
                graph
                    .children_of(&obj.id)
                    .any(|c| c.kind == ElementKind::RequirementVerificationMembership)
            });

        // VC001: No requirements.
        //
        // Warning, not error: the spec derives `verifiedRequirements` from
        // RequirementVerificationMembership and allows it to be empty — a
        // verification case that checks via `assert constraint` alone is
        // legal, just unable to produce a requirement-level verdict.
        if req_children.is_empty() && !has_objective_with_verification {
            diagnostics.push(
                Diagnostic::warning(format!(
                    "verification case '{}' has no requirements",
                    case_name
                ))
                .with_code("VC001")
                .with_span(primary_span(elem))
                .with_note("add requirement members inside the verification case body"),
            );
            continue;
        }

        // Check each requirement
        for req in &req_children {
            let req_name = req.name.clone().unwrap_or_else(|| req.id.to_string());

            check_requirement_constraints(&req_name, req, graph, &mut diagnostics);
        }
    }

    // VC005: Check for requirement references that don't exist in graph
    // Verification cases may reference requirements by name via props
    for elem in graph
        .elements_by_kind(&ElementKind::VerificationCaseDefinition)
        .chain(graph.elements_by_kind(&ElementKind::VerificationCaseUsage))
    {
        let case_name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());

        if let Some(Value::String(ref_name)) = elem.get_prop("requirement") {
            let found = graph
                .elements_by_kind(&ElementKind::RequirementUsage)
                .chain(graph.elements_by_kind(&ElementKind::RequirementDefinition))
                .any(|e| e.name.as_deref() == Some(ref_name.as_str()));
            if !found {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "verification case '{}' references unknown requirement '{}'",
                        case_name, ref_name
                    ))
                    .with_code("VC005")
                    .with_span(primary_span(elem))
                    .with_note("ensure the requirement is defined or imported in scope"),
                );
            }
        }
        if let Some(Value::List(refs)) = elem.get_prop("requirement") {
            for item in refs {
                if let Value::String(ref_name) = item {
                    let found = graph
                        .elements_by_kind(&ElementKind::RequirementUsage)
                        .chain(graph.elements_by_kind(&ElementKind::RequirementDefinition))
                        .any(|e| e.name.as_deref() == Some(ref_name.as_str()));
                    if !found {
                        diagnostics.push(
                            Diagnostic::warning(format!(
                                "verification case '{}' references unknown requirement '{}'",
                                case_name, ref_name
                            ))
                            .with_code("VC005")
                            .with_span(primary_span(elem))
                            .with_note("ensure the requirement is defined or imported in scope"),
                        );
                    }
                }
            }
        }
    }

    // VC007: Check for unknown subject references on requirements
    for elem in graph
        .elements_by_kind(&ElementKind::RequirementUsage)
        .chain(graph.elements_by_kind(&ElementKind::RequirementDefinition))
    {
        let req_name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());
        // Elaboration stores `subject` as a resolved Value::Ref (B2.1 W1);
        // an unresolvable subject name is never tagged, so the remaining
        // failure mode is a ref dangling after graph surgery.
        if let Some(Value::Ref(subject_id)) = elem.get_prop("subject") {
            if graph.get_element(subject_id).is_none() {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "requirement '{}' references unknown subject '{}'",
                        req_name, subject_id
                    ))
                    .with_code("VC007")
                    .with_span(primary_span(elem))
                    .with_note("ensure the subject part is defined in scope"),
                );
            }
        }
    }

    // VC008: Verification case with no explicit subject
    for elem in graph
        .elements_by_kind(&ElementKind::VerificationCaseDefinition)
        .chain(graph.elements_by_kind(&ElementKind::VerificationCaseUsage))
    {
        let case_name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());
        let has_subject = elem.get_prop("subject").is_some();
        // Check for SubjectMembership children, or children named "subject",
        // or children with isSubject prop (parser creates feature_declaration
        // nodes rather than SubjectMembership for `subject vehicle : Vehicle`)
        let has_subject_member = graph.children_of(&elem.id).any(|c| {
            c.kind == ElementKind::SubjectMembership
                || c.name.as_deref() == Some("subject")
                || c.get_prop("isSubject")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
        });
        if !has_subject && !has_subject_member {
            diagnostics.push(
                Diagnostic::info(format!(
                    "verification case '{}' has no explicit subject",
                    case_name
                ))
                .with_code("VC008")
                .with_span(primary_span(elem)),
            );
        }
    }

    // VC009: Assumption constraint with no expression
    for elem in graph
        .elements_by_kind(&ElementKind::RequirementUsage)
        .chain(graph.elements_by_kind(&ElementKind::RequirementDefinition))
    {
        let req_name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());
        for child in graph.children_of(&elem.id) {
            if child.kind != ElementKind::ConstraintUsage
                && child.kind != ElementKind::AssertConstraintUsage
            {
                continue;
            }
            let is_assume = child.get_prop("role").and_then(|v| v.as_str()) == Some("assume")
                || child.get_prop("constraintKind").and_then(|v| v.as_str()) == Some("assumption");
            if is_assume {
                let has_expr = sysml_core::expression_pretty::pretty_print_owner(child, graph)
                    .is_some()
                    || child.get_prop("constraint").is_some()
                    || child.get_prop("expr").is_some();
                if !has_expr {
                    let constraint_name =
                        child.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
                    diagnostics.push(
                        Diagnostic::warning(format!(
                            "assumption constraint '{}' in requirement '{}' has no expression",
                            constraint_name, req_name
                        ))
                        .with_code("VC009")
                        .with_span(primary_span(child))
                        .with_note("add a constraint expression body, e.g. `assume constraint c { x > 0 }`"),
                    );
                }
            }
        }
    }

    // VC010: Satisfy requirement references unknown requirement
    for elem in graph.elements_by_kind(&ElementKind::SatisfyRequirementUsage) {
        let sat_name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());
        // Check unresolved_type (the requirement being satisfied)
        if let Some(ref_name) = elem.get_prop("unresolved_type").and_then(|v| v.as_str()) {
            let found = graph
                .elements_by_kind(&ElementKind::RequirementUsage)
                .chain(graph.elements_by_kind(&ElementKind::RequirementDefinition))
                .any(|e| e.name.as_deref() == Some(ref_name));
            if !found {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "satisfy requirement '{}' references unknown requirement '{}'",
                        sat_name, ref_name
                    ))
                    .with_code("VC010")
                    .with_span(primary_span(elem))
                    .with_note("check that the requirement name matches exactly"),
                );
            }
        }
    }

    diagnostics
}

/// Diagnose requirement traceability gaps across a model graph.
///
/// This pass checks that requirements have both satisfaction and verification
/// traceability, and that `satisfy` declarations resolve correctly.
///
/// Diagnostic codes:
/// - **RQ001** (Warning): `SatisfyRequirementUsage` has no corresponding `Satisfy` relationship
/// - **RQ002** (Info): `RequirementDefinition` has no incoming `Satisfy` relationship
/// - **RQ003** (Info): `RequirementDefinition` has no incoming `Verify` relationship
pub fn requirement_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    use std::collections::HashSet;
    use sysml_core::element_ordering::{primary_span, sort_elements_by_source_order};

    let mut diagnostics = Vec::new();

    // RQ001: SatisfyRequirementUsage whose requirement reference couldn't be
    // resolved. Elaboration records the resolved requirement as the
    // `satisfiedRequirement` property when (and only when) it successfully
    // synthesizes the Satisfy edge; its absence means the `satisfy <name>`
    // reference is dangling. This is independent of which element the Satisfy
    // edge targets (the satisfying subject, via `by`, or the owner by default).
    let satisfy_usages: Vec<_> = graph
        .elements_by_kind(&ElementKind::SatisfyRequirementUsage)
        .filter(|e| !graph.is_library_element(&e.id))
        .collect();

    for elem in &satisfy_usages {
        let resolved = elem
            .get_prop("satisfiedRequirement")
            .and_then(|v| v.as_ref())
            .is_some();
        if !resolved {
            let name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());
            diagnostics.push(
                Diagnostic::warning(format!(
                    "satisfy '{}' cannot resolve its requirement reference",
                    name
                ))
                .with_code("RQ001")
                .with_span(primary_span(elem))
                .with_note(
                    "ensure the requirement name matches an existing requirement definition",
                ),
            );
        }
    }

    // Collect requirement IDs that appear as target in Satisfy / Verify
    // relationships (edge convention: source = satisfier/verifier,
    // target = requirement — see `RelationshipKind::Satisfy` docs).
    let mut satisfied_req_ids: HashSet<_> = graph
        .relationships_by_kind(&RelationshipKind::Satisfy)
        .map(|r| r.target.clone())
        .collect();

    // Propagate satisfaction from a satisfied RequirementUsage to the
    // RequirementDefinition it instantiates. `satisfy <usage> by <subject>`
    // records satisfaction against the *usage* (the spec satisfiedRequirement),
    // but the definition the usage is typed by is satisfied through it — so a
    // def with a satisfied usage must not be flagged unsatisfied (RQ002).
    for req_usage_id in satisfied_req_ids.iter().cloned().collect::<Vec<_>>() {
        for def_id in
            sysml_core::resolution::scoping::chaining::find_feature_types(graph, &req_usage_id)
        {
            satisfied_req_ids.insert(def_id);
        }
    }

    // RQ002 + RQ003: Check RequirementDefinitions for satisfaction and verification
    let mut req_defs: Vec<_> = graph
        .elements_by_kind(&ElementKind::RequirementDefinition)
        .filter(|e| !graph.is_library_element(&e.id))
        .collect();
    sort_elements_by_source_order(&mut req_defs);

    for req in req_defs {
        let name = req.name.clone().unwrap_or_else(|| req.id.to_string());

        // RQ002: not satisfied
        if !satisfied_req_ids.contains(&req.id) {
            diagnostics.push(
                Diagnostic::info(format!(
                    "requirement '{}' is not satisfied by any element",
                    name
                ))
                .with_code("RQ002")
                .with_span(primary_span(req)),
            );
        }

        // RQ003: not verified. Verification is the shared rollup
        // (`query::elements_verifying`): a direct Verify edge, or an edge on
        // a membership-owned check-usage typed by this requirement — the
        // same answer requirement_rows/unverified give, by construction.
        if sysml_core::query::elements_verifying(graph, &req.id).is_empty() {
            diagnostics.push(
                Diagnostic::info(format!("requirement '{}' has no verification case", name))
                    .with_code("RQ003")
                    .with_span(primary_span(req)),
            );
        }
    }

    // DV001: a Derivation-typed connection must connect one original and at
    // least one derived requirement (DerivationConnections.sysml:36,39 —
    // `originalRequirement[1]`, `derivedRequirements[1..*]`). Elaboration
    // tags classified connections with `isDerivationConnection` and never
    // partially elaborates an under-arity one; this check makes that skip
    // loud instead of silent.
    let mut derivation_conns: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            e.get_prop("isDerivationConnection")
                .and_then(|v| v.as_bool())
                == Some(true)
                && !graph.is_library_element(&e.id)
        })
        .collect();
    sort_elements_by_source_order(&mut derivation_conns);
    for conn in derivation_conns {
        let end_count = graph
            .children_of(&conn.id)
            .filter(|c| c.get_prop("isEnd").is_some())
            .count();
        if end_count < 2 {
            let name = conn.name.clone().unwrap_or_else(|| conn.id.to_string());
            diagnostics.push(
                Diagnostic::warning(format!(
                    "derivation connection '{}' has {} end{} — a Derivation connects one original requirement and at least one derived requirement",
                    name,
                    end_count,
                    if end_count == 1 { "" } else { "s" },
                ))
                .with_code("DV001")
                .with_span(primary_span(conn)),
            );
        }
    }

    diagnostics
}

/// Check constraint expressions on a requirement element, emitting VC002/VC003/VC004/VC006.
fn check_requirement_constraints(
    req_name: &str,
    req: &sysml_core::Element,
    graph: &ModelGraph,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // VC006: Check for assumptions (informational)
    if let Some(Value::List(assumptions)) = req.get_prop("assumption") {
        if !assumptions.is_empty() {
            let count = assumptions.len();
            let noun = if count == 1 {
                "assumption"
            } else {
                "assumptions"
            };
            diagnostics.push(
                Diagnostic::info(format!(
                    "requirement '{}' has {} {} \u{2014} will be vacuously satisfied if any assumption fails",
                    req_name, count, noun
                ))
                .with_code("VC006")
                .with_span(primary_span(req)),
            );
        }
    } else if let Some(Value::String(_)) = req.get_prop("assumption") {
        diagnostics.push(
            Diagnostic::info(format!(
                "requirement '{}' has 1 assumption \u{2014} will be vacuously satisfied if it fails",
                req_name
            ))
            .with_code("VC006")
            .with_span(primary_span(req)),
        );
    }

    // AST-first: walk constraint children and try compile_expression on each.
    // Also check legacy `constraint` string prop for test graphs.
    let constraint_children: Vec<_> = graph
        .children_of(&req.id)
        .filter(|c| {
            matches!(
                c.kind,
                ElementKind::ConstraintUsage | ElementKind::AssertConstraintUsage
            )
        })
        .collect();

    let has_constraint_prop = req.get_prop("constraint").is_some();

    if constraint_children.is_empty() && !has_constraint_prop {
        // VC003: No constraint expressions at all
        diagnostics.push(
            Diagnostic::warning(format!(
                "requirement '{}' has no constraint expressions",
                req_name
            ))
            .with_code("VC003")
            .with_span(primary_span(req)),
        );
        return;
    }

    let mut valid_count = 0usize;
    let mut errors = Vec::new();

    // AST-first: compile each constraint child element
    for child in &constraint_children {
        match compile_expression(child, graph) {
            Ok(_) => valid_count += 1,
            Err(diags) => {
                let msg = diags
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                errors.push(msg);
            }
        }
    }

    // Legacy fallback: if no AST children were found, check string prop
    if constraint_children.is_empty() {
        if let Some(constraint_val) = req.get_prop("constraint") {
            match constraint_val {
                Value::String(s) => match crate::expressions::compile_simple_expression(s) {
                    Ok(_) => valid_count += 1,
                    Err(diags) => {
                        let msg = diags
                            .iter()
                            .map(|d| d.message.as_str())
                            .collect::<Vec<_>>()
                            .join("; ");
                        errors.push(msg);
                    }
                },
                Value::List(items) => {
                    for item in items {
                        if let Value::String(s) = item {
                            match crate::expressions::compile_simple_expression(s) {
                                Ok(_) => valid_count += 1,
                                Err(diags) => {
                                    let msg = diags
                                        .iter()
                                        .map(|d| d.message.as_str())
                                        .collect::<Vec<_>>()
                                        .join("; ");
                                    errors.push(msg);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // VC002: Individual invalid constraint expressions
    for err in &errors {
        diagnostics.push(
            Diagnostic::error(format!(
                "requirement '{}' has invalid constraint expression: {}",
                req_name, err
            ))
            .with_code("VC002")
            .with_span(primary_span(req))
            .with_note("supported operators: ==, !=, <, >, <=, >=, +, -, *, /"),
        );
    }

    // VC004: All constraints failed to compile (none valid)
    if valid_count == 0 && !errors.is_empty() {
        diagnostics.push(
            Diagnostic::warning(format!(
                "requirement '{}' has no valid constraints — will vacuously pass",
                req_name
            ))
            .with_code("VC004")
            .with_span(primary_span(req)),
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementId, ModelGraph};
    use sysml_span::Span;

    #[test]
    fn reports_verification_case_no_requirements() {
        let mut graph = ModelGraph::new();
        let vc = Element::new(ElementId::new_v4(), ElementKind::VerificationCaseDefinition)
            .with_name("EmptyVC")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        let diags = verification_health_diagnostics(&graph);
        let vc001 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("VC001") && d.message.contains("EmptyVC"))
            .unwrap_or_else(|| {
                panic!(
                    "expected VC001 for verification case with no requirements, got: {:?}",
                    diags
                )
            });
        // Requirement-less verification cases are legal per spec
        // (verifiedRequirements is derived and may be empty) — VC001 is a
        // completeness lint, not an error.
        assert_eq!(
            vc001.severity,
            sysml_span::Severity::Warning,
            "VC001 must be warning severity, got: {:?}",
            vc001.severity
        );
    }

    #[test]
    fn reports_invalid_constraint_expression() {
        let mut graph = ModelGraph::new();
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("BadConstraintVC")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("bad-req")
            .with_owner(vc_id)
            .with_prop("constraint", Value::String("speed <<< 100".into()))
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(req);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("VC002") && d.message.contains("bad-req")),
            "expected VC002 for invalid constraint expression, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_requirement_no_constraints() {
        let mut graph = ModelGraph::new();
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("NoConstraintVC")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        // Requirement with text but no constraint prop
        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("no-constraint-req")
            .with_owner(vc_id)
            .with_prop("text", Value::String("Should have constraint".into()))
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(req);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("VC003")
                    && d.message.contains("no-constraint-req")),
            "expected VC003 for requirement without constraints, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_all_constraints_failed() {
        let mut graph = ModelGraph::new();
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("AllFailVC")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("all-bad")
            .with_owner(vc_id)
            .with_prop(
                "constraint",
                Value::List(vec![
                    Value::String("a <<< b".into()),
                    Value::String("x >>> y".into()),
                ]),
            )
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(req);

        let diags = verification_health_diagnostics(&graph);
        // Should have VC002 for each invalid expression
        let vc002_count = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("VC002"))
            .count();
        assert!(
            vc002_count >= 2,
            "expected at least 2 VC002 diagnostics, got {}",
            vc002_count
        );
        // Should also have VC004
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("VC004") && d.message.contains("all-bad")),
            "expected VC004 for requirement with all invalid constraints, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_diagnostics_for_valid_verification_case() {
        let mut graph = ModelGraph::new();
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("GoodVC")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("good-req")
            .with_owner(vc_id)
            .with_prop("constraint", Value::String("speed < 100".into()))
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(req);

        let diags = verification_health_diagnostics(&graph);
        // Should have no errors or warnings
        let errors_and_warnings: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.code.as_deref().map_or(false, |c| {
                    c.starts_with("VC00") && c != "VC006" && c != "VC008"
                })
            })
            .collect();
        assert!(
            errors_and_warnings.is_empty(),
            "expected no error/warning diagnostics for valid case, got: {:?}",
            errors_and_warnings
        );
    }

    #[test]
    fn reports_unknown_requirement_reference() {
        let mut graph = ModelGraph::new();
        let vc = Element::new(ElementId::new_v4(), ElementKind::VerificationCaseDefinition)
            .with_name("RefVC")
            .with_prop("requirement", Value::String("nonexistent-req".into()))
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags.iter().any(
                |d| d.code.as_deref() == Some("VC005") && d.message.contains("nonexistent-req")
            ),
            "expected VC005 for unknown requirement reference, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_unknown_subject() {
        let mut graph = ModelGraph::new();
        // `subject` is a resolved Value::Ref post-elaboration; a dangling ref
        // (element no longer in the graph) is the unknown-subject case.
        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("TestReq")
            .with_prop("subject", Value::Ref(ElementId::new_v4()))
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(req);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("VC007")),
            "expected VC007 for unknown subject reference, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_verification_case_no_subject() {
        let mut graph = ModelGraph::new();
        let vc = Element::new(ElementId::new_v4(), ElementKind::VerificationCaseDefinition)
            .with_name("NoSubjectVC")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(vc);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("VC008")),
            "expected VC008 for verification case with no subject, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_assumption_no_expression() {
        let mut graph = ModelGraph::new();
        let req_id = ElementId::new_v4();
        let req = Element::new(req_id.clone(), ElementKind::RequirementUsage)
            .with_name("AssumeReq")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(req);

        let constraint = Element::new(ElementId::new_v4(), ElementKind::ConstraintUsage)
            .with_name("assume1")
            .with_owner(req_id)
            .with_prop("role", Value::String("assume".into()))
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(constraint);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("VC009")),
            "expected VC009 for assumption with no expression, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_vc001_when_objective_has_verify() {
        // Verification cases using `objective { verify SomeReq; }` should NOT
        // trigger VC001. The requirement is referenced via ObjectiveMembership →
        // RequirementVerificationMembership, not as a direct RequirementUsage child.
        let mut graph = ModelGraph::new();
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("CheckSafety")
            .with_span(Span::new("file:///test.sysml", 0, 50));
        graph.add_element(vc);

        let obj_id = ElementId::new_v4();
        let obj = Element::new(obj_id.clone(), ElementKind::ObjectiveMembership)
            .with_name("safetyObjective")
            .with_owner(vc_id)
            .with_span(Span::new("file:///test.sysml", 10, 40));
        graph.add_element(obj);

        let verify = Element::new(
            ElementId::new_v4(),
            ElementKind::RequirementVerificationMembership,
        )
        .with_owner(obj_id)
        .with_prop("verifiedRequirement", Value::String("SafetyReq".into()))
        .with_span(Span::new("file:///test.sysml", 20, 35));
        graph.add_element(verify);

        let diags = verification_health_diagnostics(&graph);
        let vc001: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("VC001"))
            .collect();
        assert!(
            vc001.is_empty(),
            "should NOT emit VC001 when objective has verify, got: {:?}",
            vc001
        );
    }

    #[test]
    fn reports_satisfy_unknown_requirement() {
        let mut graph = ModelGraph::new();
        let sat = Element::new(ElementId::new_v4(), ElementKind::SatisfyRequirementUsage)
            .with_name("SatReq")
            .with_prop("unresolved_type", Value::String("MissingReq".into()))
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(sat);

        let diags = verification_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("VC010")),
            "expected VC010 for satisfy referencing unknown requirement, got: {:?}",
            diags
        );
    }

    // --- requirement_health_diagnostics tests ---

    #[test]
    fn rq001_satisfy_without_relationship() {
        let mut graph = ModelGraph::new();
        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartDefinition)
            .with_name("Vehicle")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(owner);

        let sat = Element::new(ElementId::new_v4(), ElementKind::SatisfyRequirementUsage)
            .with_name("satisfySpeed")
            .with_owner(owner_id)
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(sat);

        // No Satisfy relationship created — should trigger RQ001
        let diags = requirement_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("RQ001") && d.message.contains("satisfySpeed")),
            "expected RQ001 for satisfy without relationship, got: {:?}",
            diags
        );
    }

    #[test]
    fn rq001_no_false_positive_when_relationship_exists() {
        use sysml_core::Relationship;

        let mut graph = ModelGraph::new();
        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartDefinition)
            .with_name("Vehicle")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(owner);

        let req_id = ElementId::new_v4();
        let req = Element::new(req_id.clone(), ElementKind::RequirementDefinition)
            .with_name("SpeedReq")
            .with_span(Span::new("file:///test.sysml", 40, 60));
        graph.add_element(req);

        let sat = Element::new(ElementId::new_v4(), ElementKind::SatisfyRequirementUsage)
            .with_name("satisfySpeed")
            .with_owner(owner_id.clone())
            // Elaboration records the resolved requirement here on success.
            .with_prop("satisfiedRequirement", sysml_core::Value::Ref(req_id.clone()))
            .with_span(Span::new("file:///test.sysml", 11, 30));
        graph.add_element(sat);

        // Add the Satisfy relationship (source=owner/satisfier, target=req)
        let rel = Relationship::new(sysml_core::RelationshipKind::Satisfy, owner_id, req_id);
        graph.add_relationship(rel);

        let diags = requirement_health_diagnostics(&graph);
        let rq001: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("RQ001"))
            .collect();
        assert!(
            rq001.is_empty(),
            "should NOT emit RQ001 when Satisfy relationship exists, got: {:?}",
            rq001
        );
    }

    #[test]
    fn rq002_unsatisfied_requirement() {
        let mut graph = ModelGraph::new();
        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition)
            .with_name("SafetyReq")
            .with_span(Span::new("file:///test.sysml", 0, 20));
        graph.add_element(req);

        let diags = requirement_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("RQ002") && d.message.contains("SafetyReq")),
            "expected RQ002 for unsatisfied requirement, got: {:?}",
            diags
        );
    }

    #[test]
    fn rq003_unverified_requirement() {
        let mut graph = ModelGraph::new();
        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition)
            .with_name("PerformanceReq")
            .with_span(Span::new("file:///test.sysml", 0, 20));
        graph.add_element(req);

        let diags = requirement_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("RQ003")
                && d.message.contains("PerformanceReq")),
            "expected RQ003 for unverified requirement, got: {:?}",
            diags
        );
    }

    #[test]
    fn rq002_rq003_no_false_positive_when_satisfied_and_verified() {
        use sysml_core::Relationship;

        let mut graph = ModelGraph::new();
        let req_id = ElementId::new_v4();
        let req = Element::new(req_id.clone(), ElementKind::RequirementDefinition)
            .with_name("TracedReq")
            .with_span(Span::new("file:///test.sysml", 0, 20));
        graph.add_element(req);

        let part_id = ElementId::new_v4();
        let part = Element::new(part_id.clone(), ElementKind::PartDefinition)
            .with_name("Impl")
            .with_span(Span::new("file:///test.sysml", 30, 40));
        graph.add_element(part);

        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("CheckReq")
            .with_span(Span::new("file:///test.sysml", 50, 60));
        graph.add_element(vc);

        // Satisfy: source=part (satisfier), target=req
        graph.add_relationship(Relationship::new(
            sysml_core::RelationshipKind::Satisfy,
            part_id,
            req_id.clone(),
        ));
        // Verify: source=vc (verifier), target=req
        graph.add_relationship(Relationship::new(
            sysml_core::RelationshipKind::Verify,
            vc_id,
            req_id,
        ));

        let diags = requirement_health_diagnostics(&graph);
        let rq_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref().map_or(false, |c| c.starts_with("RQ")))
            .collect();
        assert!(
            rq_diags.is_empty(),
            "should NOT emit RQ diagnostics for fully traced requirement, got: {:?}",
            rq_diags
        );
    }

    #[test]
    fn dv001_flags_under_arity_derivation_connection() {
        let mut graph = ModelGraph::new();

        let conn_id = ElementId::new_v4();
        let conn = Element::new(conn_id.clone(), ElementKind::ConnectionUsage)
            .with_name("badDerivation")
            .with_prop("isDerivationConnection", true)
            .with_span(Span::new("file:///test.sysml", 0, 20));
        graph.add_element(conn);

        // One lone end — under the Derivation arity floor of 2.
        let end = Element::new(ElementId::new_v4(), ElementKind::ReferenceUsage)
            .with_owner(conn_id.clone())
            .with_prop("isEnd", true)
            .with_span(Span::new("file:///test.sysml", 5, 10));
        graph.add_element(end);

        let diags = requirement_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("DV001")),
            "expected DV001 for a 1-end derivation connection, got: {:?}",
            diags
        );

        // A well-formed 2-end derivation must NOT be flagged.
        let mut ok_graph = ModelGraph::new();
        let ok_id = ElementId::new_v4();
        let ok_conn = Element::new(ok_id.clone(), ElementKind::ConnectionUsage)
            .with_name("goodDerivation")
            .with_prop("isDerivationConnection", true)
            .with_span(Span::new("file:///test.sysml", 0, 20));
        ok_graph.add_element(ok_conn);
        for i in 0..2 {
            let end = Element::new(ElementId::new_v4(), ElementKind::ReferenceUsage)
                .with_owner(ok_id.clone())
                .with_prop("isEnd", true)
                .with_span(Span::new("file:///test.sysml", 5 + i, 10 + i));
            ok_graph.add_element(end);
        }
        let ok_diags = requirement_health_diagnostics(&ok_graph);
        assert!(
            !ok_diags.iter().any(|d| d.code.as_deref() == Some("DV001")),
            "2-end derivation must not be flagged: {:?}",
            ok_diags
        );
    }
}
