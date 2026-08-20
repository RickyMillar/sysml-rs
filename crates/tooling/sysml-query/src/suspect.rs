//! Suspect attribution — map a [`GraphDiff`] onto requirement rows (R9).
//!
//! A requirement row is *suspect* against a baseline when the model
//! changed underneath it since that baseline. The diff itself is
//! kind-agnostic and reports changes on the elements that actually
//! changed — which for the common case (a statement-text edit) is an
//! owned `Documentation` CHILD, not the requirement element. This module
//! owns the attribution step: walk each changed element's owner chain to
//! the nearest requirement, then propagate suspicion downstream along
//! `Derive` edges (design doc R9: "flag rows changed-upstream since
//! baseline" — grounded in the `RequirementDerivation` library's
//! `originalImpliesDerived` semantics: if the original changed, every
//! requirement checked to imply it is suspect by construction).
//!
//! ## Identity contract (ADR-009, binding)
//!
//! Attribution reports exactly what [`sysml_core::diff::diff_graphs`]
//! reports — id-strict, no name-matching, no positional smoothing. A
//! requirement whose id is absent from the baseline gets
//! [`SuspectCause::NotInBaseline`]: it is *either* newly authored *or*
//! the replacement half of a scope rename, and the diff cannot tell the
//! two apart. Consumers must present that honestly ("identity changed"),
//! never manufacture continuity the identity model doesn't have.

use std::collections::{BTreeMap, HashSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sysml_core::diff::{FieldDelta, GraphDiff};
use sysml_core::{is_requirement_kind, ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_id::ElementId;

/// Why a requirement row is suspect. Tagged on `kind` for the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuspectCause {
    /// Statement text changed: a `body` prop edit on an owned
    /// `Documentation` element. Carries the before/after text for the
    /// suspect popover's diff excerpt.
    TextChanged {
        /// The `Documentation` element that changed.
        element: ElementId,
        from: String,
        to: String,
    },
    /// Any other scalar prop edit (a constraint's `constraint` text, an
    /// attribute's `value` — string or numeric/quantity, rendered to
    /// display text) on the requirement or an owned descendant. Generic
    /// on purpose — `key` names the prop, so one variant covers every
    /// semantic prop the elaborator stores, and the popover can show an
    /// honest before/after instead of a bare "element changed" (R18/W4).
    /// Structural values (refs, lists, maps) stay [`Self::ContentChanged`].
    PropTextChanged {
        element: ElementId,
        element_kind: ElementKind,
        key: String,
        from: String,
        to: String,
    },
    /// A non-text field or relationship delta on the requirement itself
    /// or one of its owned (non-requirement) descendants.
    ContentChanged {
        element: ElementId,
        element_kind: ElementKind,
    },
    /// An owned descendant appeared since the baseline.
    ChildAdded {
        element: ElementId,
        element_kind: ElementKind,
    },
    /// An owned descendant present at the baseline is gone.
    ChildRemoved {
        element: ElementId,
        element_kind: ElementKind,
    },
    /// The requirement's own id is absent from the baseline: newly
    /// authored OR a scope-rename replacement — the id-strict diff
    /// cannot distinguish (ADR-009). Present as "identity changed";
    /// never name-match the removed and added sides together.
    NotInBaseline,
    /// An upstream requirement this one (transitively) derives from is
    /// itself suspect. `via` is the immediate upstream requirement.
    UpstreamSuspect { via: ElementId },
}

/// One suspect requirement row and every attributed cause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SuspectRecord {
    /// The requirement (definition or usage) the row is keyed by.
    pub requirement: ElementId,
    pub causes: Vec<SuspectCause>,
}

/// Render a prop value for a before/after cause. Strings and enums are
/// their bare text; other scalars use the model `Display` form (so a
/// quantity edit reads `40 [ms]` → `25 [ms]`). Structural values have no
/// honest one-line text — callers fall back to `ContentChanged`. Shared
/// with `requirement_detail`'s attribute-value rendering (same honesty
/// contract both places).
pub(crate) fn prop_display_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Enum(e) => Some(e.clone()),
        Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::Complex { .. }
        | Value::Quantity { .. } => Some(value.to_string()),
        Value::Ref(_) | Value::List(_) | Value::Map(_) | Value::Null => None,
    }
}

/// Walk `id`'s owner chain (in `graph`) to the nearest element of a
/// requirement kind — `id` itself counts. Mirrors `owning_package_ref`'s
/// walk shape in the row builder.
fn nearest_requirement(graph: &ModelGraph, id: &ElementId) -> Option<ElementId> {
    let mut current = Some(id.clone());
    while let Some(cur) = current {
        let el = graph.get_element(&cur)?;
        if is_requirement_kind(el.kind.clone()) {
            return Some(cur);
        }
        current = el.owner.clone();
    }
    None
}

/// Attribute a diff between `old` (the baseline graph) and `new` (the
/// current graph) to the requirement rows of `new`, including transitive
/// downstream propagation along `Derive` edges. Output is sorted by
/// requirement id; causes are deduplicated.
///
/// `old` is required because removed elements exist only in the baseline
/// graph — their owner chains (and, for removed requirements, their
/// outgoing derivation fan-out) can only be walked there.
pub fn attribute_diff_to_requirements(
    old: &ModelGraph,
    new: &ModelGraph,
    diff: &GraphDiff,
) -> Vec<SuspectRecord> {
    // requirement id (in `new`) → causes, insertion-deduped.
    let mut records: BTreeMap<ElementId, Vec<SuspectCause>> = BTreeMap::new();
    fn push(
        records: &mut BTreeMap<ElementId, Vec<SuspectCause>>,
        req: ElementId,
        cause: SuspectCause,
    ) {
        let causes = records.entry(req).or_default();
        if !causes.contains(&cause) {
            causes.push(cause);
        }
    }

    // Removed requirements (only in `old`): no live row to flag, but
    // their downstream derivations are still suspect — seed them.
    let mut removed_requirements: Vec<ElementId> = Vec::new();

    // 1. Modified elements: attribute to the nearest requirement in the
    //    NEW graph (ids in `modified` exist in both graphs).
    for element_diff in &diff.modified {
        let Some(req) = nearest_requirement(new, &element_diff.id) else {
            continue;
        };
        let mut non_text_delta = !element_diff.relationship_deltas.is_empty();
        for field in &element_diff.changed_fields {
            match field {
                FieldDelta::PropChanged { key, from, to } => {
                    match (prop_display_text(from), prop_display_text(to)) {
                        // Statement text keeps its dedicated cause: the
                        // popover treats it as THE requirement text, not
                        // "a prop named body".
                        (Some(from_text), Some(to_text))
                            if key == "body" && element_diff.kind == ElementKind::Documentation =>
                        {
                            push(
                                &mut records,
                                req.clone(),
                                SuspectCause::TextChanged {
                                    element: element_diff.id.clone(),
                                    from: from_text,
                                    to: to_text,
                                },
                            )
                        }
                        (Some(from_text), Some(to_text)) => push(
                            &mut records,
                            req.clone(),
                            SuspectCause::PropTextChanged {
                                element: element_diff.id.clone(),
                                element_kind: element_diff.kind.clone(),
                                key: key.clone(),
                                from: from_text,
                                to: to_text,
                            },
                        ),
                        _ => non_text_delta = true,
                    }
                }
                _ => non_text_delta = true,
            }
        }
        if non_text_delta {
            push(
                &mut records,
                req.clone(),
                SuspectCause::ContentChanged {
                    element: element_diff.id.clone(),
                    element_kind: element_diff.kind.clone(),
                },
            );
        }
    }

    // 2. Added elements: attribute in the NEW graph. An added requirement
    //    itself is NotInBaseline; an added descendant is ChildAdded on
    //    its nearest surviving requirement (skipping requirement
    //    ancestors that are themselves added — they already carry
    //    NotInBaseline, which subsumes their subtree).
    let added: HashSet<&ElementId> = diff.added.iter().collect();
    for id in &diff.added {
        let Some(req) = nearest_requirement(new, id) else {
            continue;
        };
        if req == *id {
            push(&mut records, req, SuspectCause::NotInBaseline);
        } else if !added.contains(&req) {
            let Some(el) = new.get_element(id) else {
                continue;
            };
            push(
                &mut records,
                req,
                SuspectCause::ChildAdded {
                    element: id.clone(),
                    element_kind: el.kind.clone(),
                },
            );
        }
    }

    // 3. Removed elements: owner chains only exist in the OLD graph. A
    //    removed descendant of a SURVIVING requirement is ChildRemoved on
    //    the live row; a removed requirement seeds downstream
    //    propagation.
    for id in &diff.removed {
        let Some(req) = nearest_requirement(old, id) else {
            continue;
        };
        if req == *id {
            removed_requirements.push(req);
        } else if new.get_element(&req).is_some() {
            let Some(el) = old.get_element(id) else {
                continue;
            };
            push(
                &mut records,
                req,
                SuspectCause::ChildRemoved {
                    element: id.clone(),
                    element_kind: el.kind.clone(),
                },
            );
        }
    }

    // 4. Downstream propagation along Derive edges (source = derived
    //    requirement, target = original). BFS with a visited set —
    //    nothing enforces derivation-graph acyclicity, so don't assume
    //    it. Removed upstream requirements fan out through OLD-graph
    //    edges (their edges are gone from `new`); live suspects fan out
    //    through NEW-graph edges.
    let mut queue: VecDeque<(ElementId, bool)> = records
        .keys()
        .cloned()
        .map(|id| (id, false))
        .chain(removed_requirements.into_iter().map(|id| (id, true)))
        .collect();
    let mut visited: HashSet<ElementId> = queue.iter().map(|(id, _)| id.clone()).collect();
    while let Some((upstream, use_old_edges)) = queue.pop_front() {
        let graph = if use_old_edges { old } else { new };
        let derived_ids: Vec<ElementId> = graph
            .incoming(&upstream)
            .filter(|rel| rel.kind == RelationshipKind::Derive)
            .map(|rel| rel.source.clone())
            .collect();
        for derived in derived_ids {
            // Only live rows can be flagged.
            if new.get_element(&derived).is_none() {
                continue;
            }
            push(
                &mut records,
                derived.clone(),
                SuspectCause::UpstreamSuspect {
                    via: upstream.clone(),
                },
            );
            if visited.insert(derived.clone()) {
                queue.push_back((derived, false));
            }
        }
    }

    records
        .into_iter()
        .map(|(requirement, causes)| SuspectRecord {
            requirement,
            causes,
        })
        .collect()
}

// ── Clearing attestations (v1.5b) ────────────────────────────────────
//
// The display-time predicate ruled 2026-07-16 (steward, WorkflowStore
// follow-up): a suspect-clearing attestation clears exactly the changes
// up to the content state (`attested_commit`) that was latest when the
// engineer attested; any later change supersedes it. All computed —
// never a cached is-suspect boolean. This crate stays store-agnostic:
// callers map workflow-store events into [`ClearingInput`] and
// precompute, per distinct `attested_commit`, which requirements are
// still suspect from that commit.

/// One suspect-clearing attestation, reduced to what the predicate
/// needs. `seq` is the workflow event id (monotonic per project).
#[derive(Debug, Clone, PartialEq)]
pub struct ClearingInput {
    pub seq: u64,
    pub element: ElementId,
    /// Content digest that was latest when the engineer attested.
    pub attested_commit: String,
}

/// Outcome of applying clearings to a base suspect set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClearingOutcome {
    /// requirement id → seq of the newest NON-superseded attestation
    /// clearing it. Rows present here are not suspect for display.
    pub cleared_by: BTreeMap<ElementId, u64>,
    /// attestation seq → superseded? (true = the requirement changed
    /// again after it was attested). Every input clearing gets a verdict
    /// — stale attestations are flagged, never dropped (append-only
    /// history is not forgotten).
    pub superseded: BTreeMap<u64, bool>,
}

/// Apply clearing attestations (all against ONE baseline — the caller
/// filters by baseline commit) to `base` = the suspect records computed
/// from that baseline. `attested_suspects` maps each distinct
/// `attested_commit` to the requirement ids still suspect from it.
///
/// A clearing whose `attested_commit` is missing from
/// `attested_suspects` is treated as superseded (conservative: an
/// attestation never clears without proof its state still holds).
pub fn apply_clearings(
    base: &[SuspectRecord],
    clearings: &[ClearingInput],
    attested_suspects: &std::collections::HashMap<String, HashSet<ElementId>>,
) -> ClearingOutcome {
    let mut outcome = ClearingOutcome::default();
    for clearing in clearings {
        let is_superseded = attested_suspects
            .get(&clearing.attested_commit)
            .is_none_or(|still| still.contains(&clearing.element));
        outcome.superseded.insert(clearing.seq, is_superseded);
    }
    for record in base {
        let winner = clearings
            .iter()
            .filter(|c| c.element == record.requirement)
            .filter(|c| outcome.superseded.get(&c.seq) == Some(&false))
            .max_by_key(|c| c.seq);
        if let Some(clearing) = winner {
            outcome
                .cleared_by
                .insert(record.requirement.clone(), clearing.seq);
        }
    }
    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::diff::diff_graphs;
    use sysml_core::{Element, Relationship};

    fn requirement(name: &str) -> Element {
        Element::new_with_kind(ElementKind::RequirementUsage).with_name(name)
    }

    fn doc_child(owner: &Element, body: &str) -> Element {
        let mut doc = Element::new_with_kind(ElementKind::Documentation);
        doc.owner = Some(owner.id.clone());
        doc.set_prop("body", body);
        doc
    }

    /// Text edit on a Documentation child attributes to the owning
    /// requirement as TextChanged with before/after bodies.
    #[test]
    fn doc_body_edit_attributes_to_owning_requirement() {
        let req = requirement("R1");
        let doc = doc_child(&req, "trip within 40 ms");

        let mut old = ModelGraph::new();
        old.add_element(req.clone());
        old.add_element(doc.clone());

        let mut new_graph = old.clone();
        if let Some(el) = new_graph.elements.get_mut(&doc.id) {
            el.set_prop("body", "trip within 25 ms");
        }

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].requirement, req.id);
        assert_eq!(
            records[0].causes,
            vec![SuspectCause::TextChanged {
                element: doc.id.clone(),
                from: "trip within 40 ms".to_owned(),
                to: "trip within 25 ms".to_owned(),
            }]
        );
    }

    /// An added requirement is NotInBaseline — and its own doc children
    /// don't produce redundant ChildAdded noise.
    #[test]
    fn added_requirement_is_not_in_baseline() {
        let old = ModelGraph::new();
        let req = requirement("R-new");
        let doc = doc_child(&req, "brand new");
        let mut new_graph = ModelGraph::new();
        new_graph.add_element(req.clone());
        new_graph.add_element(doc);

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].requirement, req.id);
        assert_eq!(records[0].causes, vec![SuspectCause::NotInBaseline]);
    }

    /// Derivation propagation: original changes → derived requirement is
    /// UpstreamSuspect, transitively, without looping on cycles.
    #[test]
    fn derive_propagation_is_transitive_and_cycle_safe() {
        let original = requirement("Original");
        let derived = requirement("Derived");
        let leaf = requirement("Leaf");
        let doc = doc_child(&original, "v1");

        let mut old = ModelGraph::new();
        for el in [&original, &derived, &leaf, &doc] {
            old.add_element(el.clone());
        }
        // derived --Derive--> original, leaf --Derive--> derived, and a
        // cycle back: original --Derive--> leaf.
        old.add_relationship(Relationship::new(
            RelationshipKind::Derive,
            derived.id.clone(),
            original.id.clone(),
        ));
        old.add_relationship(Relationship::new(
            RelationshipKind::Derive,
            leaf.id.clone(),
            derived.id.clone(),
        ));
        old.add_relationship(Relationship::new(
            RelationshipKind::Derive,
            original.id.clone(),
            leaf.id.clone(),
        ));

        let mut new_graph = old.clone();
        if let Some(el) = new_graph.elements.get_mut(&doc.id) {
            el.set_prop("body", "v2");
        }

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);

        let by_id: BTreeMap<_, _> = records
            .iter()
            .map(|r| (r.requirement.clone(), &r.causes))
            .collect();
        assert!(by_id[&original.id]
            .iter()
            .any(|c| matches!(c, SuspectCause::TextChanged { .. })));
        assert_eq!(
            by_id[&derived.id].as_slice(),
            &[SuspectCause::UpstreamSuspect {
                via: original.id.clone()
            }]
        );
        assert_eq!(
            by_id[&leaf.id].as_slice(),
            &[SuspectCause::UpstreamSuspect {
                via: derived.id.clone()
            }]
        );
        // The cycle (original derives from leaf) must not re-flag or loop.
        assert_eq!(records.len(), 3);
    }

    /// A removed requirement has no row, but its downstream derivation
    /// (found via OLD-graph edges) is flagged.
    #[test]
    fn removed_upstream_flags_surviving_derived() {
        let original = requirement("Removed");
        let derived = requirement("Survivor");
        let mut old = ModelGraph::new();
        old.add_element(original.clone());
        old.add_element(derived.clone());
        old.add_relationship(Relationship::new(
            RelationshipKind::Derive,
            derived.id.clone(),
            original.id.clone(),
        ));

        let mut new_graph = ModelGraph::new();
        new_graph.add_element(derived.clone());

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].requirement, derived.id);
        // Two honest causes: the Derive edge itself vanished (a content
        // change on the surviving row) AND the removed upstream flags it.
        assert!(records[0].causes.contains(&SuspectCause::UpstreamSuspect {
            via: original.id.clone()
        }));
        assert!(records[0].causes.contains(&SuspectCause::ContentChanged {
            element: derived.id.clone(),
            element_kind: ElementKind::RequirementUsage,
        }));
    }

    /// A removed doc child of a surviving requirement is ChildRemoved.
    #[test]
    fn removed_child_of_surviving_requirement() {
        let req = requirement("R1");
        let doc = doc_child(&req, "to be deleted");
        let mut old = ModelGraph::new();
        old.add_element(req.clone());
        old.add_element(doc.clone());

        let mut new_graph = ModelGraph::new();
        new_graph.add_element(req.clone());

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].causes,
            vec![SuspectCause::ChildRemoved {
                element: doc.id.clone(),
                element_kind: ElementKind::Documentation,
            }]
        );
    }

    /// A constraint-body edit (the `constraint` prop on a
    /// RequirementConstraintMembership child) is PropTextChanged with the
    /// prop key and before/after text — not an opaque ContentChanged
    /// (R18/W4).
    #[test]
    fn constraint_prop_edit_is_prop_text_changed() {
        let req = requirement("R1");
        let mut constraint = Element::new_with_kind(ElementKind::RequirementConstraintMembership);
        constraint.owner = Some(req.id.clone());
        constraint.set_prop("constraint", "actualTime <= 40");

        let mut old = ModelGraph::new();
        old.add_element(req.clone());
        old.add_element(constraint.clone());

        let mut new_graph = old.clone();
        if let Some(el) = new_graph.elements.get_mut(&constraint.id) {
            el.set_prop("constraint", "actualTime <= 25");
        }

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].requirement, req.id);
        assert_eq!(
            records[0].causes,
            vec![SuspectCause::PropTextChanged {
                element: constraint.id.clone(),
                element_kind: ElementKind::RequirementConstraintMembership,
                key: "constraint".to_owned(),
                from: "actualTime <= 40".to_owned(),
                to: "actualTime <= 25".to_owned(),
            }]
        );
    }

    /// A numeric/quantity `value` prop edit renders through the model
    /// Display form — an attribute-value change is a verdict-input
    /// change, not an opaque "element changed".
    #[test]
    fn quantity_value_edit_renders_display_text() {
        use sysml_core::physics::dimension::DimensionVector;

        let req = requirement("R1");
        let mut attr = Element::new_with_kind(ElementKind::AttributeUsage);
        attr.owner = Some(req.id.clone());
        attr.set_prop(
            "value",
            Value::quantity(40.0, DimensionVector::default(), Some("ms".to_owned())),
        );

        let mut old = ModelGraph::new();
        old.add_element(req.clone());
        old.add_element(attr.clone());

        let mut new_graph = old.clone();
        if let Some(el) = new_graph.elements.get_mut(&attr.id) {
            el.set_prop(
                "value",
                Value::quantity(25.0, DimensionVector::default(), Some("ms".to_owned())),
            );
        }

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].causes,
            vec![SuspectCause::PropTextChanged {
                element: attr.id.clone(),
                element_kind: ElementKind::AttributeUsage,
                key: "value".to_owned(),
                from: "40 [ms]".to_owned(),
                to: "25 [ms]".to_owned(),
            }]
        );
    }

    /// A structural prop change (a Ref retarget) has no honest one-line
    /// text — it stays ContentChanged.
    #[test]
    fn ref_prop_change_stays_content_changed() {
        let req = requirement("R1");
        let target_a = requirement("A");
        let target_b = requirement("B");
        let mut child = Element::new_with_kind(ElementKind::SatisfyRequirementUsage);
        child.owner = Some(req.id.clone());
        child.set_prop("satisfiedRequirement", Value::Ref(target_a.id.clone()));

        let mut old = ModelGraph::new();
        for el in [&req, &target_a, &target_b, &child] {
            old.add_element(el.clone());
        }

        let mut new_graph = old.clone();
        if let Some(el) = new_graph.elements.get_mut(&child.id) {
            el.set_prop("satisfiedRequirement", Value::Ref(target_b.id.clone()));
        }

        let diff = diff_graphs(&old, &new_graph);
        let records = attribute_diff_to_requirements(&old, &new_graph, &diff);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].causes,
            vec![SuspectCause::ContentChanged {
                element: child.id.clone(),
                element_kind: ElementKind::SatisfyRequirementUsage,
            }]
        );
    }

    /// Predicate: a non-superseded attestation clears; a later content
    /// change supersedes it and suspicion re-fires; newest wins.
    #[test]
    fn apply_clearings_predicate() {
        let r = ElementId::from_string("req");
        let base = vec![SuspectRecord {
            requirement: r.clone(),
            causes: vec![SuspectCause::NotInBaseline],
        }];
        let mut attested: std::collections::HashMap<String, HashSet<ElementId>> =
            std::collections::HashMap::new();
        // After commit "c1" the requirement changed again → still suspect
        // from c1 → attestation at c1 is superseded.
        attested.insert("c1".into(), HashSet::from([r.clone()]));
        // From commit "c2" nothing further changed.
        attested.insert("c2".into(), HashSet::new());

        let clearings = vec![
            ClearingInput { seq: 1, element: r.clone(), attested_commit: "c1".into() },
            ClearingInput { seq: 2, element: r.clone(), attested_commit: "c2".into() },
        ];
        let outcome = apply_clearings(&base, &clearings, &attested);
        assert_eq!(outcome.superseded.get(&1), Some(&true));
        assert_eq!(outcome.superseded.get(&2), Some(&false));
        assert_eq!(outcome.cleared_by.get(&r), Some(&2));

        // Only the superseded attestation → nothing cleared.
        let outcome = apply_clearings(&base, &clearings[..1], &attested);
        assert!(outcome.cleared_by.is_empty());

        // Unknown attested_commit → conservative: superseded, no clear.
        let unknown = vec![ClearingInput { seq: 3, element: r.clone(), attested_commit: "??".into() }];
        let outcome = apply_clearings(&base, &unknown, &attested);
        assert_eq!(outcome.superseded.get(&3), Some(&true));
        assert!(outcome.cleared_by.is_empty());
    }
}
