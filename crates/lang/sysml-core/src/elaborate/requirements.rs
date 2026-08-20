//! Requirement elaboration.
//!
//! Tags `subject` (resolved `Value::Ref`), `objective` (name string), and
//! `actors`/`stakeholders` (resolved `Value::List` of `Value::Ref`) properties
//! on requirement/case elements from their structural SubjectMembership,
//! ObjectiveMembership, ActorMembership, and StakeholderMembership children.
//! Also normalizes constraint `role` tags for requirement children.

use super::ElaborationReport;
use crate::resolution::resolved_props;
use crate::{
    CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Relationship, RelationshipKind,
    Value,
};

/// Elaborate requirements: tag subject, objective, actor/stakeholder roles,
/// constraint roles, and synthesize Satisfy/Verify relationships.
pub(super) fn elaborate_requirements(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    tag_subjects(graph, report);
    tag_subject_refs(graph);
    tag_objectives(graph, report);
    tag_role_memberships(graph, report, ElementKind::ActorMembership, "actors");
    tag_role_memberships(graph, report, ElementKind::StakeholderMembership, "stakeholders");
    synthesize_satisfy_verify(graph, report);
}

/// Stamp `resolvedSubject` (`Value::Ref`) on every `SubjectMembership` whose
/// `subject <name>` reference resolves. ADDITIVE and self-contained: distinct
/// from `tag_subjects` (which writes the `subject` prop on the owning
/// requirement and skips cases) — this walks memberships directly so the
/// semantic-token emitter can colour the subject reference site by its resolved
/// target's kind, on requirements AND verification/use/analysis cases. Resolves
/// the subject name in the membership's owner scope (`resolve_name`, the shared
/// helper). A miss stamps nothing — the emitter falls back to UNRESOLVED.
fn tag_subject_refs(graph: &mut ModelGraph) {
    let to_tag: Vec<(ElementId, ElementId)> = graph
        .element_ids_by_kind(&ElementKind::SubjectMembership)
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|m| m.get_prop(resolved_props::SUBJECT).is_none())
        // Only the REFERENCE form (`subject rcdModel;`) — the declaration form
        // (`subject v : Vehicle;`) mints a new subject parameter (carries
        // `unresolved_type`) and its name is a declaration, not a colourable
        // reference to some other element.
        .filter(|m| m.get_prop("unresolved_type").is_none())
        .filter_map(|m| {
            let name = m.name.clone().or_else(|| {
                graph.children_of(&m.id).find_map(|gc| gc.name.clone())
            })?;
            // Resolve in the scope ENCLOSING the case/requirement, not the case
            // itself: the SubjectMembership is an owned child named the same as
            // the subject, so resolving from the case would self-match it (owned
            // wins). The referenced part is a sibling of the case, found from the
            // enclosing namespace.
            let case = m.owner.clone()?;
            let scope = graph
                .get_element(&case)
                .and_then(|c| c.owner.clone())
                .unwrap_or(case);
            let target = super::resolve_name(graph, &Some(scope), &name)?;
            if target == m.id {
                return None;
            }
            Some((m.id.clone(), target))
        })
        .collect();

    for (id, target) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.props
                .insert(resolved_props::SUBJECT.into(), Value::Ref(target));
        }
    }
}

/// Tag `subject` on requirement elements from SubjectMembership children.
///
/// When a requirement has a SubjectMembership child, this pass extracts the
/// subject's name and resolves it (via `resolve_name`, the same helper
/// `synthesize_satisfy_verify` uses) to an `ElementId`, stored as
/// `Value::Ref` in the `subject` property. For the declaration form
/// (`subject vehicle : Vehicle;`) the name resolves to the requirement's own
/// SubjectMembership — the subject parameter itself; its declared type rides
/// on that element (`unresolved_type` → FeatureTyping), not on this ref.
/// A name that fails to resolve tags nothing — no string fallback.
fn tag_subjects(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut req_ids = Vec::new();
    req_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::RequirementUsage));
    req_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::RequirementDefinition));
    req_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::SatisfyRequirementUsage));

    let to_tag: Vec<(ElementId, ElementId)> = req_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("subject").is_none())
        .filter_map(|e| {
            // Find SubjectMembership child
            let subject_name = graph
                .children_of(&e.id)
                .filter(|c| c.kind == ElementKind::SubjectMembership)
                .find_map(|c| {
                    // The subject's name may be on the SubjectMembership itself
                    // or on its first named child
                    c.name.clone().or_else(|| {
                        graph
                            .children_of(&c.id)
                            .find_map(|grandchild| grandchild.name.clone())
                    })
                })?;
            let subject_id = super::resolve_name(graph, &Some(e.id.clone()), &subject_name)?;
            Some((e.id.clone(), subject_id))
        })
        .collect();

    for (id, subject) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("subject", Value::Ref(subject));
            report.requirements_elaborated += 1;
        }
    }
}

/// Tag `objective` on case elements from ObjectiveMembership children.
///
/// When a case (verification, use case, analysis) has an ObjectiveMembership
/// child, this pass extracts the child's name and sets it as the `objective`
/// property on the case.
fn tag_objectives(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut case_ids = Vec::new();
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::VerificationCaseDefinition));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::VerificationCaseUsage));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::UseCaseDefinition));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::UseCaseUsage));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::AnalysisCaseDefinition));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::AnalysisCaseUsage));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::CaseDefinition));
    case_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::CaseUsage));

    let to_tag: Vec<(ElementId, String, Option<ElementId>)> = case_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("objective").is_none())
        .filter_map(|e| {
            let membership = graph
                .children_of(&e.id)
                .find(|c| c.kind == ElementKind::ObjectiveMembership)?;
            let membership_id = membership.id.clone();
            // Existing string prop (unchanged): the membership's name, or the
            // owned requirement's name.
            let objective_name = membership.name.clone().or_else(|| {
                graph
                    .children_of(&membership_id)
                    .find_map(|grandchild| grandchild.name.clone())
            })?;
            // Additive resolved ref (Phase B.2): per the SysML spec a Case's
            // `/objectiveRequirement` derives from the ObjectiveMembership's
            // *ownedObjectiveRequirement* — the composite, inline-owned
            // `RequirementUsage`. Point `resolvedObjective` at that child (never
            // the membership relationship element). Always inline-owned; there
            // is no cross-reference form. (SysML-spec-r2025-04 §CaseDefinition
            // objectiveRequirement; SysML-vocab.ttl:712-714.)
            let objective_ref = graph
                .children_of(&membership_id)
                .find(|gc| gc.kind == ElementKind::RequirementUsage)
                .map(|gc| gc.id.clone());
            Some((e.id.clone(), objective_name, objective_ref))
        })
        .collect();

    for (id, objective, objective_ref) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("objective", Value::String(objective));
            if let Some(target) = objective_ref {
                elem.props
                    .insert(resolved_props::OBJECTIVE.into(), Value::Ref(target));
            }
            report.requirements_elaborated += 1;
        }
    }
}

/// Tag a role-membership list property (`actors` / `stakeholders`) on every
/// element owning children of `membership_kind`.
///
/// Mirrors `tag_subjects`: each membership's name (on the membership itself
/// or its first named child) is resolved via `resolve_name` in the owner's
/// scope, and the owner gets `prop_key` set to a `Value::List` of
/// `Value::Ref`s in source order. For the declaration form
/// (`actor driver : Driver;`) each ref targets the local membership — the
/// role parameter itself. Memberships whose name fails to resolve are
/// skipped — no string fallback. Owners are keyed off the memberships
/// themselves (ActorMembership appears under requirements AND cases,
/// StakeholderMembership under requirements/concerns/viewpoints — S045/S046),
/// so no owner-kind enumeration can drift out of sync with the spec.
fn tag_role_memberships(
    graph: &mut ModelGraph,
    report: &mut ElaborationReport,
    membership_kind: ElementKind,
    prop_key: &'static str,
) {
    let mut owners: Vec<ElementId> = Vec::new();
    for id in graph.element_ids_by_kind(&membership_kind) {
        if let Some(owner) = graph.get_element(id).and_then(|e| e.owner.clone()) {
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
    }

    let mut to_tag: Vec<(ElementId, Vec<Value>)> = Vec::new();
    for owner_id in owners {
        let Some(owner) = graph.get_element(&owner_id) else {
            continue;
        };
        if owner.get_prop(prop_key).is_some() {
            continue;
        }
        // children_of iterates a hash set — sort by source position (name/id
        // tiebreak) so the ref list is deterministic and reads in file order.
        let mut memberships: Vec<&Element> = graph
            .children_of(&owner_id)
            .filter(|c| c.kind == membership_kind)
            .collect();
        crate::element_ordering::sort_elements_by_source_order(&mut memberships);
        let refs: Vec<Value> = memberships
            .into_iter()
            .filter_map(|c| {
                let name = c.name.clone().or_else(|| {
                    graph
                        .children_of(&c.id)
                        .find_map(|grandchild| grandchild.name.clone())
                })?;
                let target = super::resolve_name(graph, &Some(owner_id.clone()), &name)?;
                Some(Value::Ref(target))
            })
            .collect();
        if !refs.is_empty() {
            to_tag.push((owner_id, refs));
        }
    }

    for (id, refs) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop(prop_key, Value::List(refs));
            report.requirements_elaborated += 1;
        }
    }
}

/// Synthesize `Satisfy` and `Verify` relationships from structural elements.
///
/// Edge direction follows the `RelationshipKind` doc convention (the one
/// every consumer — `query::elements_satisfying`, `workspace.requirement_rows`,
/// the aggregation code lens, `cli trace` — was written against):
/// source = the satisfying/verifying element, target = the requirement.
///
/// - `SatisfyRequirementUsage`: The subject (part) satisfies the requirement
///   referenced by `unresolved_type`. Creates `Relationship::Satisfy(subject_id, req_id)`.
/// - `VerificationCaseUsage`/`VerificationCaseDefinition`: One Verify edge per
///   owned `RequirementVerificationMembership` (directly in the case body or
///   under the objective), targeted per the spec's `referencedConstraint`
///   algorithm — see [`verification_membership_target`] — plus legacy
///   `unresolved_type` name shapes. Creates `Relationship::Verify(case_id, target)`.
fn synthesize_satisfy_verify(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // --- Satisfy relationships ---
    // The requirement name can be in `unresolved_type` (from FeatureTyping)
    // or in a ReferenceSubsetting child (from `satisfy ReqName by X;` syntax
    // where the parser creates OwnedReferenceSubsetting, not FeatureTyping).
    let satisfy_ids: Vec<ElementId> = graph
        .element_ids_by_kind(&ElementKind::SatisfyRequirementUsage)
        .to_vec();

    // (satisfy_usage_id, satisfied_requirement_id, satisfying_subject_id)
    let satisfy_rels: Vec<(ElementId, ElementId, ElementId)> = satisfy_ids
        .iter()
        .filter_map(|id| {
            let e = graph.get_element(id)?;
            // Try unresolved_type directly on the element first
            let req_name = e
                .get_prop("unresolved_type")
                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                .or_else(|| {
                    // Check FeatureTyping children — the parser stores the type
                    // reference on a child FeatureTyping element for the
                    // `satisfy requirement name : RequirementType;` syntax.
                    graph.children_of(id).find_map(|child| {
                        if child.kind == ElementKind::FeatureTyping {
                            child
                                .get_prop("unresolved_type")
                                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| {
                    // Fall back to ReferenceSubsetting child's unresolved target.
                    // The parser creates OwnedReferenceSubsetting with property
                    // "unresolved_referencedFeature" for `satisfy ReqName by X;` syntax.
                    graph.children_of(id).find_map(|child| {
                        if child.kind == ElementKind::ReferenceSubsetting {
                            child
                                .get_prop("unresolved_referencedFeature")
                                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                        } else {
                            None
                        }
                    })
                })?;
            let req_id = super::resolve_name(graph, &e.owner, &req_name)?;
            // The satisfying subject. `satisfy <req> by <subject>` names the
            // satisfyingFeature explicitly (SysML §8.3.20 SatisfyRequirementUsage;
            // SysML-vocab `satisfyingFeature` — "the actual subject that is
            // asserted to satisfy the satisfiedRequirement"). The parser records
            // it as the `satisfiedBy` property. With no `by` clause the subject
            // defaults to the enclosing context (the SatisfyRequirementUsage's
            // owner), e.g. `part p { satisfy req; }`.
            let subject_id = match e.get_prop("satisfiedBy").and_then(|v| v.as_str()) {
                Some(subject) => super::resolve_name(graph, &e.owner, subject)?,
                None => e.owner.clone()?,
            };
            Some((id.clone(), req_id, subject_id))
        })
        .collect();

    for (satisfy_id, req_id, subject_id) in satisfy_rels {
        // Record the resolved satisfiedRequirement on the SatisfyRequirementUsage
        // so traceability checks (RQ001) can distinguish a resolved satisfy from a
        // dangling one without re-deriving the reference, independent of which
        // element the synthesized Satisfy edge targets.
        if let Some(elem) = graph.get_element_mut(&satisfy_id) {
            if elem.get_prop("satisfiedRequirement").is_none() {
                elem.set_prop("satisfiedRequirement", Value::Ref(req_id.clone()));
            }
        }

        let already_exists = graph
            .relationships_by_kind(&RelationshipKind::Satisfy)
            .any(|r| r.source == subject_id && r.target == req_id);

        if !already_exists {
            let src_key = CanonicalKey::root(&subject_id.to_string());
            let tgt_key = CanonicalKey::root(&req_id.to_string());
            let edge_key = CanonicalKey::for_relationship(
                &src_key,
                RelationshipKind::Satisfy.as_str(),
                &tgt_key,
                0,
            );
            let rel = Relationship::new_with_key(
                RelationshipKind::Satisfy,
                subject_id,
                req_id,
                &edge_key,
            );
            graph.add_relationship(rel);
            report.requirements_elaborated += 1;
        }
    }

    // --- Verify relationships ---
    let mut verify_candidate_ids = Vec::new();
    verify_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::VerificationCaseUsage));
    verify_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::VerificationCaseDefinition));

    let verify_rels: Vec<(ElementId, ElementId)> = verify_candidate_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .flat_map(|e| {
            // A case can verify SEVERAL requirements: one target per
            // RequirementVerificationMembership (resolved by
            // `verification_membership_target` below), plus the legacy
            // name-carrying shapes resolved at the end.
            let mut targets: Vec<ElementId> = Vec::new();
            let mut req_names: Vec<String> = Vec::new();

            // Legacy shapes: `unresolved_type` on the case itself or on
            // FeatureTyping / ObjectiveMembership children (and
            // ObjectiveMembership grandchildren).
            if let Some(name) = e.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                req_names.push(name.to_owned());
            }
            for c in graph.children_of(&e.id) {
                if c.kind == ElementKind::FeatureTyping
                    || c.kind == ElementKind::ObjectiveMembership
                {
                    if let Some(name) = c.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                        req_names.push(name.to_owned());
                    }
                    for gc in graph.children_of(&c.id) {
                        // `objective { verify … }` — both the reference form
                        // and the declaration form lower to an
                        // ObjectiveMembership-owned
                        // RequirementVerificationMembership.
                        if gc.kind == ElementKind::RequirementVerificationMembership {
                            targets
                                .extend(verification_membership_target(graph, gc, &e.owner));
                            continue;
                        }
                        if let Some(name) =
                            gc.get_prop("unresolved_type").and_then(|v| v.as_str())
                        {
                            req_names.push(name.to_owned());
                        }
                    }
                }
                // `verify …;` directly in the case body (no objective wrapper).
                if c.kind == ElementKind::RequirementVerificationMembership {
                    targets.extend(verification_membership_target(graph, c, &e.owner));
                }
            }

            let case_id = e.id.clone();
            let owner = e.owner.clone();
            targets.extend(
                req_names
                    .into_iter()
                    .filter_map(|req_name| super::resolve_name(graph, &owner, &req_name)),
            );
            targets
                .into_iter()
                .map(|req_id| (req_id, case_id.clone()))
                .collect::<Vec<_>>()
        })
        .collect();

    for (req_id, case_id) in verify_rels {
        let already_exists = graph
            .relationships_by_kind(&RelationshipKind::Verify)
            .any(|r| r.source == case_id && r.target == req_id);

        if !already_exists {
            let src_key = CanonicalKey::root(&case_id.to_string());
            let tgt_key = CanonicalKey::root(&req_id.to_string());
            let edge_key = CanonicalKey::for_relationship(
                &src_key,
                RelationshipKind::Verify.as_str(),
                &tgt_key,
                0,
            );
            let rel =
                Relationship::new_with_key(RelationshipKind::Verify, case_id, req_id, &edge_key);
            graph.add_relationship(rel);
            report.requirements_elaborated += 1;
        }
    }
}

/// The element a `RequirementVerificationMembership` identifies as verified —
/// the Verify mint target. This comment is the rule's one home.
///
/// The spec defines it as `verifiedRequirement` (SysML-vocab.ttl:2954-2958):
/// "the referencedConstraint of the RequirementVerificationMembership
/// considered as a RequirementConstraintMembership, which must be a
/// RequirementUsage" — and `referencedConstraint` (SysML-vocab.ttl:2577-2579)
/// is "the referencedFeature of the ownedReferenceSubsetting of the
/// ownedConstraint, if there is one, and, otherwise, the ownedConstraint
/// itself." Two branches, nothing else:
///
/// 1. **Reference form** `verify R;` — the owned RequirementUsage carries an
///    ownedReferenceSubsetting to `R`, so the target is `R`. Two parse
///    shapes carry this: an owned check-usage with a ReferenceSubsetting
///    child, or the parser-flattened shorthand where the reference lands as
///    the `verifiedRequirement` name prop on the membership itself —
///    resolving that name resolves the same reference the subsetting
///    carries. Single hop only: `referencedFeature` is already a resolved
///    endpoint; there is NO recursion through chained subsetting.
/// 2. **Declaration form** `verify requirement check : ReqDef;` — no
///    reference subsetting, so the target is the ownedConstraint itself:
///    the LOCAL check-usage, NOT the def it is typed by. Typing is what the
///    verification engine evaluates; it is not what the membership
///    identifies. (A FeatureTyping walk here was considered and rejected as
///    invented indirection — the def-level rollup is a query-layer opinion,
///    `query::elements_verifying`, never a second edge.)
///
/// Exactly one Verify edge per RequirementVerificationMembership.
fn verification_membership_target(
    graph: &ModelGraph,
    membership: &Element,
    scope: &Option<ElementId>,
) -> Option<ElementId> {
    if let Some(check) = graph
        .children_of(&membership.id)
        .find(|c| c.kind == ElementKind::RequirementUsage)
    {
        // Branch 1 via an explicit owned reference subsetting on the check.
        if let Some(name) = graph
            .children_of(&check.id)
            .find(|c| c.kind == ElementKind::ReferenceSubsetting)
            .and_then(|c| c.get_prop("unresolved_referencedFeature"))
            .and_then(|v| v.as_str())
        {
            return super::resolve_name(graph, scope, name);
        }
        // Branch 2: the ownedConstraint itself — the local check-usage.
        return Some(check.id.clone());
    }
    // Branch 1, parser-flattened shorthand (`verifiedRequirement` name prop).
    let name = membership
        .get_prop("verifiedRequirement")
        .and_then(|v| v.as_str())?;
    super::resolve_name(graph, scope, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    #[test]
    fn tags_subject_from_subject_membership() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("SafetyReq");
        let req_id = graph.add_element(req);

        let subject = Element::new_with_kind(ElementKind::SubjectMembership)
            .with_name("vehicle")
            .with_owner(req_id.clone());
        let subject_id = graph.add_element(subject);

        let report = elaborate(&mut graph);

        assert!(report.requirements_elaborated >= 1);
        let elem = graph.get_element(&req_id).unwrap();
        assert_eq!(
            elem.get_prop("subject").and_then(|v| v.as_ref()),
            Some(&subject_id),
            "declaration form must resolve to the local SubjectMembership"
        );
    }

    #[test]
    fn tags_subject_from_nested_child() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementDefinition).with_name("PerfReq");
        let req_id = graph.add_element(req);

        // SubjectMembership without a name, but with a named child
        let subject_mem =
            Element::new_with_kind(ElementKind::SubjectMembership).with_owner(req_id.clone());
        let sub_id = graph.add_element(subject_mem);

        let child = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("engine")
            .with_owner(sub_id);
        let child_id = graph.add_element(child);

        let report = elaborate(&mut graph);

        assert!(report.requirements_elaborated >= 1);
        let elem = graph.get_element(&req_id).unwrap();
        assert_eq!(
            elem.get_prop("subject").and_then(|v| v.as_ref()),
            Some(&child_id),
            "nested-name form must resolve to the named subject usage"
        );
    }

    #[test]
    fn does_not_overwrite_existing_subject() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("SafetyReq")
            .with_prop("subject", "originalSubject");
        let req_id = graph.add_element(req);

        let subject = Element::new_with_kind(ElementKind::SubjectMembership)
            .with_name("newSubject")
            .with_owner(req_id.clone());
        graph.add_element(subject);

        elaborate(&mut graph);

        let elem = graph.get_element(&req_id).unwrap();
        assert_eq!(
            elem.get_prop("subject").and_then(|v| v.as_str()),
            Some("originalSubject")
        );
    }

    #[test]
    fn tags_objective_from_objective_membership() {
        let mut graph = ModelGraph::new();

        let vc =
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("TestCase");
        let vc_id = graph.add_element(vc);

        let obj = Element::new_with_kind(ElementKind::ObjectiveMembership)
            .with_name("mainObjective")
            .with_owner(vc_id.clone());
        graph.add_element(obj);

        let report = elaborate(&mut graph);

        assert!(report.requirements_elaborated >= 1);
        let elem = graph.get_element(&vc_id).unwrap();
        assert_eq!(
            elem.get_prop("objective").and_then(|v| v.as_str()),
            Some("mainObjective")
        );
    }

    #[test]
    fn stamps_resolved_subject_ref_on_membership_for_case() {
        // B.2b: tag_subject_refs stamps resolvedSubject on the SubjectMembership
        // (Value::Ref) even for verification cases (which tag_subjects skips), so
        // the `subject <name>` reference site can be coloured by its target.
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("P");
        let pkg_id = graph.add_element(pkg);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("rcdModel")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        let vc = Element::new_with_kind(ElementKind::VerificationCaseDefinition)
            .with_name("VC")
            .with_owner(pkg_id.clone());
        let vc_id = graph.add_element(vc);

        let subj = Element::new_with_kind(ElementKind::SubjectMembership)
            .with_name("rcdModel")
            .with_owner(vc_id.clone());
        let subj_id = graph.add_element(subj);

        let _ = elaborate(&mut graph);

        let m = graph.get_element(&subj_id).unwrap();
        assert_eq!(
            m.get_prop(crate::resolution::resolved_props::SUBJECT)
                .and_then(|v| v.as_ref()),
            Some(&part_id),
            "resolvedSubject must point at the referenced part"
        );
    }

    #[test]
    fn stamps_resolved_objective_ref_to_owned_requirement() {
        // Phase B.2: per the SysML spec a Case's /objectiveRequirement is the
        // ObjectiveMembership's owned RequirementUsage. `resolvedObjective`
        // (ADDITIVE Value::Ref) must point at that owned usage; the existing
        // `objective` string prop is left intact.
        let mut graph = ModelGraph::new();

        let vc =
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("TestCase");
        let vc_id = graph.add_element(vc);

        let membership =
            Element::new_with_kind(ElementKind::ObjectiveMembership).with_owner(vc_id.clone());
        let membership_id = graph.add_element(membership);

        // The composite, inline-owned objective RequirementUsage.
        let obj = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("tripObjective")
            .with_owner(membership_id.clone());
        let obj_id = graph.add_element(obj);

        let _ = elaborate(&mut graph);

        let elem = graph.get_element(&vc_id).unwrap();
        assert_eq!(
            elem.get_prop(crate::resolution::resolved_props::OBJECTIVE)
                .and_then(|v| v.as_ref()),
            Some(&obj_id),
            "resolvedObjective must point at the owned RequirementUsage"
        );
        // Existing string prop still derived (from the owned usage's name).
        assert_eq!(
            elem.get_prop("objective").and_then(|v| v.as_str()),
            Some("tripObjective")
        );
    }

    #[test]
    fn does_not_overwrite_existing_objective() {
        let mut graph = ModelGraph::new();

        let vc = Element::new_with_kind(ElementKind::VerificationCaseDefinition)
            .with_name("TestCase")
            .with_prop("objective", "existingObj");
        let vc_id = graph.add_element(vc);

        let obj = Element::new_with_kind(ElementKind::ObjectiveMembership)
            .with_name("newObj")
            .with_owner(vc_id.clone());
        graph.add_element(obj);

        elaborate(&mut graph);

        let elem = graph.get_element(&vc_id).unwrap();
        assert_eq!(
            elem.get_prop("objective").and_then(|v| v.as_str()),
            Some("existingObj")
        );
    }

    #[test]
    fn idempotent() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("SafetyReq");
        let req_id = graph.add_element(req);

        let subject = Element::new_with_kind(ElementKind::SubjectMembership)
            .with_name("vehicle")
            .with_owner(req_id);
        graph.add_element(subject);

        let r1 = elaborate(&mut graph);
        assert!(r1.requirements_elaborated > 0);

        let r2 = elaborate(&mut graph);
        assert_eq!(
            r2.requirements_elaborated, 0,
            "second elaborate should be no-op"
        );
    }

    #[test]
    fn tags_actors_and_stakeholders_as_resolved_ref_lists() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementDefinition).with_name("SafeReq");
        let req_id = graph.add_element(req);

        let actor = Element::new_with_kind(ElementKind::ActorMembership)
            .with_name("driver")
            .with_owner(req_id.clone());
        let actor_id = graph.add_element(actor);

        let stakeholder = Element::new_with_kind(ElementKind::StakeholderMembership)
            .with_name("owner")
            .with_owner(req_id.clone());
        let stakeholder_id = graph.add_element(stakeholder);

        let report = elaborate(&mut graph);
        assert!(report.requirements_elaborated >= 2);

        let elem = graph.get_element(&req_id).unwrap();
        let actors = elem.get_prop("actors").and_then(|v| v.as_list()).unwrap();
        assert_eq!(actors.len(), 1);
        assert_eq!(actors[0].as_ref(), Some(&actor_id));
        let stakeholders = elem
            .get_prop("stakeholders")
            .and_then(|v| v.as_list())
            .unwrap();
        assert_eq!(stakeholders.len(), 1);
        assert_eq!(stakeholders[0].as_ref(), Some(&stakeholder_id));
    }

    #[test]
    fn tags_multiple_actors_on_case_in_source_order() {
        let mut graph = ModelGraph::new();

        let uc = Element::new_with_kind(ElementKind::UseCaseDefinition).with_name("Drive");
        let uc_id = graph.add_element(uc);

        // Names deliberately in reverse alphabetical order: source spans, not
        // names, must drive the list order.
        let a1 = Element::new_with_kind(ElementKind::ActorMembership)
            .with_name("passenger")
            .with_owner(uc_id.clone())
            .with_span(sysml_span::Span::new("file:///t.sysml", 10, 19));
        let a1_id = graph.add_element(a1);
        let a2 = Element::new_with_kind(ElementKind::ActorMembership)
            .with_name("driver")
            .with_owner(uc_id.clone())
            .with_span(sysml_span::Span::new("file:///t.sysml", 30, 36));
        let a2_id = graph.add_element(a2);

        elaborate(&mut graph);

        let elem = graph.get_element(&uc_id).unwrap();
        let actors = elem.get_prop("actors").and_then(|v| v.as_list()).unwrap();
        assert_eq!(
            actors.iter().map(|v| v.as_ref().unwrap()).collect::<Vec<_>>(),
            vec![&a1_id, &a2_id],
            "actor refs must preserve source order"
        );
        assert!(elem.get_prop("stakeholders").is_none());
    }

    #[test]
    fn actor_name_on_nested_child_resolves() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req");
        let req_id = graph.add_element(req);

        // ActorMembership without a name, but with a named child usage
        let mem = Element::new_with_kind(ElementKind::ActorMembership).with_owner(req_id.clone());
        let mem_id = graph.add_element(mem);
        let usage = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("operator")
            .with_owner(mem_id);
        let usage_id = graph.add_element(usage);

        elaborate(&mut graph);

        let elem = graph.get_element(&req_id).unwrap();
        let actors = elem.get_prop("actors").and_then(|v| v.as_list()).unwrap();
        assert_eq!(actors[0].as_ref(), Some(&usage_id));
    }

    #[test]
    fn does_not_overwrite_existing_actors() {
        let mut graph = ModelGraph::new();

        let existing = Value::List(vec![]);
        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("Req")
            .with_prop("actors", existing);
        let req_id = graph.add_element(req);

        let actor = Element::new_with_kind(ElementKind::ActorMembership)
            .with_name("driver")
            .with_owner(req_id.clone());
        graph.add_element(actor);

        elaborate(&mut graph);

        let elem = graph.get_element(&req_id).unwrap();
        assert_eq!(
            elem.get_prop("actors").and_then(|v| v.as_list()).map(|l| l.len()),
            Some(0),
            "pre-existing actors prop must not be overwritten"
        );
    }

    #[test]
    fn role_tagging_idempotent() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementDefinition).with_name("Req");
        let req_id = graph.add_element(req);
        let actor = Element::new_with_kind(ElementKind::ActorMembership)
            .with_name("driver")
            .with_owner(req_id.clone());
        graph.add_element(actor);
        let stakeholder = Element::new_with_kind(ElementKind::StakeholderMembership)
            .with_name("owner")
            .with_owner(req_id);
        graph.add_element(stakeholder);

        let r1 = elaborate(&mut graph);
        assert!(r1.requirements_elaborated >= 2);
        let r2 = elaborate(&mut graph);
        assert_eq!(
            r2.requirements_elaborated, 0,
            "second elaborate should be no-op"
        );
    }

    #[test]
    fn synthesizes_satisfy_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("perfReq")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        // SatisfyRequirementUsage owned by part, referencing requirement
        let satisfy = Element::new_with_kind(ElementKind::SatisfyRequirementUsage)
            .with_owner(part_id.clone())
            .with_prop("unresolved_type", "perfReq");
        graph.add_element(satisfy);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Satisfy)
            .collect();
        assert_eq!(rels.len(), 1, "should synthesize Satisfy relationship");
        assert_eq!(
            rels[0].source, part_id,
            "source should be the satisfying part"
        );
        assert_eq!(rels[0].target, req_id, "target should be the requirement");
    }

    #[test]
    fn satisfy_by_subject_targets_named_subject_not_owner() {
        // `package P { requirement r; part sys; satisfy r by sys; }`
        // The synthesized Satisfy must target `sys` (the `by` subject), not the
        // package owner. Also records `satisfiedRequirement` on the satisfy usage.
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("P");
        let pkg_id = graph.add_element(pkg);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("r")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("sys")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        let satisfy = Element::new_with_kind(ElementKind::SatisfyRequirementUsage)
            .with_owner(pkg_id.clone())
            .with_prop("unresolved_type", "r")
            .with_prop("satisfiedBy", "sys");
        let satisfy_id = graph.add_element(satisfy);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Satisfy)
            .collect();
        assert_eq!(rels.len(), 1, "should synthesize one Satisfy relationship");
        assert_eq!(
            rels[0].source, part_id,
            "source should be the `by` subject (sys), not the package owner"
        );
        assert_eq!(rels[0].target, req_id, "target should be the requirement");

        // satisfiedRequirement recorded for traceability (RQ001).
        let satisfy_elem = graph.get_element(&satisfy_id).unwrap();
        assert_eq!(
            satisfy_elem.get_prop("satisfiedRequirement").and_then(|v| v.as_ref()),
            Some(&req_id),
            "satisfy usage should record the resolved requirement"
        );
    }

    #[test]
    fn synthesizes_verify_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("safetyReq")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        let vc = Element::new_with_kind(ElementKind::VerificationCaseUsage)
            .with_name("testSafety")
            .with_owner(pkg_id)
            .with_prop("unresolved_type", "safetyReq");
        let vc_id = graph.add_element(vc);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Verify)
            .collect();
        assert_eq!(rels.len(), 1, "should synthesize Verify relationship");
        assert_eq!(rels[0].source, vc_id, "source should be the verification case");
        assert_eq!(rels[0].target, req_id, "target should be the requirement");
    }

    #[test]
    fn verify_membership_declaration_form_targets_local_check_usage() {
        // `verification def T1 { objective { verify requirement tripCheck :
        // TripReq; } }` — referencedConstraint branch 2: no reference
        // subsetting, so the Verify edge targets the LOCAL check-usage,
        // never the def it is typed by. Exactly one edge per membership.
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let req_def = Element::new_with_kind(ElementKind::RequirementDefinition)
            .with_name("TripReq")
            .with_owner(pkg_id.clone());
        let req_def_id = graph.add_element(req_def);

        let case = Element::new_with_kind(ElementKind::VerificationCaseDefinition)
            .with_name("T1")
            .with_owner(pkg_id.clone());
        let case_id = graph.add_element(case);

        let obj =
            Element::new_with_kind(ElementKind::ObjectiveMembership).with_owner(case_id.clone());
        let obj_id = graph.add_element(obj);

        let membership = Element::new_with_kind(ElementKind::RequirementVerificationMembership)
            .with_owner(obj_id);
        let membership_id = graph.add_element(membership);

        let check = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("tripCheck")
            .with_owner(membership_id);
        let check_id = graph.add_element(check);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(check_id.clone())
            .with_prop("unresolved_type", "TripReq");
        graph.add_element(typing);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Verify)
            .collect();
        assert_eq!(rels.len(), 1, "exactly one Verify edge per membership");
        assert_eq!(rels[0].source, case_id, "source is the verification case");
        assert_eq!(
            rels[0].target, check_id,
            "target is the local check-usage (referencedConstraint branch 2)"
        );
        assert_ne!(
            rels[0].target, req_def_id,
            "the typed def must NOT be the edge target — def rollup is a query-layer opinion"
        );
    }

    #[test]
    fn verify_membership_reference_subsetting_targets_referenced_requirement() {
        // referencedConstraint branch 1: the owned check-usage carries an
        // ownedReferenceSubsetting → the edge targets its referencedFeature.
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("safetyReq")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        let case = Element::new_with_kind(ElementKind::VerificationCaseUsage)
            .with_name("checkSafety")
            .with_owner(pkg_id.clone());
        let case_id = graph.add_element(case);

        let membership = Element::new_with_kind(ElementKind::RequirementVerificationMembership)
            .with_owner(case_id.clone());
        let membership_id = graph.add_element(membership);

        let check =
            Element::new_with_kind(ElementKind::RequirementUsage).with_owner(membership_id);
        let check_id = graph.add_element(check);

        let subsetting = Element::new_with_kind(ElementKind::ReferenceSubsetting)
            .with_owner(check_id.clone())
            .with_prop("unresolved_referencedFeature", "safetyReq");
        graph.add_element(subsetting);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Verify)
            .collect();
        assert_eq!(rels.len(), 1, "exactly one Verify edge per membership");
        assert_eq!(rels[0].source, case_id);
        assert_eq!(
            rels[0].target, req_id,
            "target is the referencedFeature (branch 1), not the check-usage"
        );
        assert_ne!(rels[0].target, check_id);
    }

    #[test]
    fn verify_membership_shorthand_prop_targets_named_requirement() {
        // Parser-flattened `verify safetyReq;` directly in the case body —
        // the `verifiedRequirement` name prop resolves the same reference
        // the subsetting carries (branch 1).
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("safetyReq")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        let case = Element::new_with_kind(ElementKind::VerificationCaseUsage)
            .with_name("checkSafety")
            .with_owner(pkg_id.clone());
        let case_id = graph.add_element(case);

        let membership = Element::new_with_kind(ElementKind::RequirementVerificationMembership)
            .with_owner(case_id.clone())
            .with_prop("verifiedRequirement", "safetyReq");
        graph.add_element(membership);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Verify)
            .collect();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source, case_id);
        assert_eq!(rels[0].target, req_id);
    }

    #[test]
    fn satisfy_verify_idempotent() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("req")
            .with_owner(pkg_id.clone());
        graph.add_element(req);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("part")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        let satisfy = Element::new_with_kind(ElementKind::SatisfyRequirementUsage)
            .with_owner(part_id)
            .with_prop("unresolved_type", "req");
        graph.add_element(satisfy);

        elaborate(&mut graph);
        let count_1 = graph
            .relationships_by_kind(&RelationshipKind::Satisfy)
            .count();

        elaborate(&mut graph);
        let count_2 = graph
            .relationships_by_kind(&RelationshipKind::Satisfy)
            .count();

        assert_eq!(
            count_1, count_2,
            "should not duplicate Satisfy relationships"
        );
    }
}
