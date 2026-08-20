//! Workflow sidecar composition — the thin service-side assembly over
//! the pure pieces (mirrors `storage::suspect_requirements`'s shape):
//! `sysml_store::workflow_store` owns the event log, `sysml_query::
//! suspect::apply_clearings` owns the display-time clearing predicate,
//! this module fetches, composes, and lifts errors.
//!
//! Predicate (steward ruling 2026-07-16, normative): a requirement R is
//! suspect vs baseline B iff `requirement_suspects(from=B)` contains R
//! AND no clearing attestation for (R, B) exists whose
//! `attested_commit` state R has not changed since. Later content
//! changes supersede an attestation; superseded attestations stay in
//! the log and the folded view, flagged — never dropped.

use std::collections::{HashMap, HashSet};

use sysml_core::ModelGraph;
use sysml_id::{ElementId, ProjectId};
use sysml_query::suspect::{apply_clearings, ClearingInput, SuspectRecord};
use sysml_store::{
    fold_element_state, NewWorkflowEvent, Store, WorkflowEvent, WorkflowEventKind, WorkflowStore,
    WorkflowStoreError,
};

use crate::error::ServiceError;
use crate::storage;

/// Lift a workflow-store error. A missing actor is caller input, not a
/// backend fault.
pub(crate) fn workflow_err(e: WorkflowStoreError) -> ServiceError {
    match e {
        WorkflowStoreError::MissingActor => ServiceError::InvalidInput(e.to_string()),
        other => ServiceError::Store(other.to_string()),
    }
}

/// One suspect record plus the clearing verdict — the wire shape of
/// `sysml.workspace.requirement_suspects` from v1.5b on. `cleared_by`
/// (the seq of the newest non-superseded attestation) is additive over
/// the v1.5a shape; rows stay in the list even when cleared so the UI
/// can show WHY there is no flag.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SuspectRecordView {
    #[serde(flatten)]
    pub record: SuspectRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleared_by: Option<u64>,
}

/// Folded workflow state of one element, wire shape for
/// `sysml.workflow.state`. `suspect_clearings[*].superseded` is filled
/// by the predicate; `orphaned` = the element id no longer exists in
/// the current graph (ADR-009 honesty — history is presented as
/// belonging to a prior identity, never re-attached by name).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ElementWorkflowState {
    #[serde(flatten)]
    pub state: sysml_store::WorkflowElementState,
    pub orphaned: bool,
}

/// Extract clearing attestations against `baseline_commit` from raw
/// events, as predicate inputs.
fn clearings_against(events: &[WorkflowEvent], baseline_commit: &str) -> Vec<ClearingInput> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            WorkflowEventKind::SuspectClearingAttestation {
                baseline_commit: bc,
                attested_commit,
                ..
            } if bc == baseline_commit => Some(ClearingInput {
                seq: e.seq,
                element: e.element_id.clone(),
                attested_commit: attested_commit.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Per distinct `attested_commit`, which requirements are still suspect
/// from it. A commit that can no longer be resolved (evicted from the
/// in-memory store) yields NO entry — `apply_clearings` treats that
/// conservatively as superseded (an attestation never clears without
/// proof its state still holds).
fn attested_suspect_sets(
    store: &dyn Store,
    project: &ProjectId,
    clearings: &[ClearingInput],
) -> HashMap<String, HashSet<ElementId>> {
    let mut sets = HashMap::new();
    for clearing in clearings {
        if sets.contains_key(&clearing.attested_commit) {
            continue;
        }
        if let Ok(records) =
            storage::suspect_requirements(store, project, &clearing.attested_commit, None)
        {
            sets.insert(
                clearing.attested_commit.clone(),
                records.into_iter().map(|r| r.requirement).collect(),
            );
        }
    }
    sets
}

/// Suspect records vs a baseline with the clearing predicate applied —
/// the one home feeding the popover, the Suspect view, and reports.
pub fn suspects_with_clearings(
    store: &dyn Store,
    workflow: &dyn WorkflowStore,
    project: &ProjectId,
    from: &str,
    to: Option<&str>,
) -> Result<Vec<SuspectRecordView>, ServiceError> {
    let base = storage::suspect_requirements(store, project, from, to)?;
    let from_commit = storage::resolve_ref(store, project, from)?;
    let events = workflow.events(project, None).map_err(workflow_err)?;
    let clearings = clearings_against(&events, from_commit.as_str());
    let attested = attested_suspect_sets(store, project, &clearings);
    let outcome = apply_clearings(&base, &clearings, &attested);
    Ok(base
        .into_iter()
        .map(|record| {
            let cleared_by = outcome.cleared_by.get(&record.requirement).copied();
            SuspectRecordView { record, cleared_by }
        })
        .collect())
}

/// "Attest unchanged intent": records that `actor` reviewed `element`'s
/// changes since `baseline_ref` and vouches the intent still holds.
///
/// Steps (ruling 2026-07-16): (1) mint/resolve the CURRENT content
/// commit via the idempotent workspace snapshot — the only correct
/// source of `attested_commit`; (2) the element must actually be
/// suspect vs the baseline (attesting a clean row is a caller error);
/// (3) append the event with the baseline REF AS GIVEN preserved as
/// write-time provenance.
#[allow(clippy::too_many_arguments)]
pub fn attest_suspect_clearing(
    store: &mut dyn Store,
    workflow: &dyn WorkflowStore,
    project: &ProjectId,
    graph: &ModelGraph,
    element_id: &ElementId,
    baseline_ref: &str,
    rationale: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    let attested = storage::save_workspace_snapshot(store, project, graph, "attest snapshot")?;
    let baseline_commit = storage::resolve_ref(store, project, baseline_ref)?;

    let suspects = storage::suspect_requirements(store, project, baseline_ref, None)?;
    if !suspects.iter().any(|r| &r.requirement == element_id) {
        return Err(ServiceError::InvalidInput(format!(
            "element {element_id} is not suspect against baseline '{baseline_ref}' — nothing to attest"
        )));
    }

    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            element_id: element_id.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::SuspectClearingAttestation {
                baseline_name: baseline_ref.to_owned(),
                baseline_commit: baseline_commit.as_str().to_owned(),
                attested_commit: attested.commit.as_str().to_owned(),
                rationale: rationale.to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Record a MANUAL verification act on a live element (B10 layer 3, human
/// leg — verification-evidence-taxonomy.md §3.5). An ATTESTATION, never a
/// computed verdict: it lands in the workflow sidecar only, and must never
/// enter verdict stores, timelines, or rollups.
///
/// `method` is validated against the spec's closed
/// `VerificationMethodKind` set (`sysml_core::metadata::
/// VERIFICATION_METHOD_KINDS` — the ONE home; an append-only audit log can
/// never be typo-corrected, so `"inspekt"` dies here). `attested_commit`
/// pins the workspace content the engineer looked at — later content
/// changes supersede the attestation at display time, same discipline as
/// suspect clearing.
#[allow(clippy::too_many_arguments)]
pub fn attest_verification(
    store: &mut dyn Store,
    workflow: &dyn WorkflowStore,
    project: &ProjectId,
    graph: &ModelGraph,
    element_id: &ElementId,
    method: &str,
    statement: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    let method = nonblank("method", method)?;
    if !sysml_core::metadata::VERIFICATION_METHOD_KINDS.contains(&method) {
        return Err(ServiceError::InvalidInput(format!(
            "unknown verification method '{method}' — expected one of {:?} \
             (VerificationCases::VerificationMethodKind)",
            sysml_core::metadata::VERIFICATION_METHOD_KINDS
        )));
    }
    require_element_exists(graph, element_id)?;
    let statement = nonblank("statement", statement)?;
    let attested = storage::save_workspace_snapshot(store, project, graph, "attest snapshot")?;
    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            element_id: element_id.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::VerificationAttestation {
                method: method.to_owned(),
                statement: statement.to_owned(),
                attested_commit: attested.commit.as_str().to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Approval-state vocabulary (steward ruling 2026-07-16): a CLOSED set,
/// validated at the write boundary — an append-only audit log can never
/// be typo-corrected, so `"aproved"` must die here, not live forever.
/// This is deliberately sidecar/process vocabulary, distinct from the
/// spec's model-side maturity (`StatusKind`).
pub const APPROVAL_STATES: [&str; 4] = ["draft", "in_review", "approved", "rejected"];

/// Every element is always in exactly one approval state: "no event
/// yet" IS the initial state, never a sentinel (steward ruling).
pub const APPROVAL_INITIAL: &str = "draft";

/// New workflow writes require a LIVE element id — a dead id is
/// unrecoverable pollution in an append-only log (mirrors `relink`'s
/// target check). History on ids that died later is handled at read
/// time (`orphaned`), never by permitting new writes.
fn require_element_exists(graph: &ModelGraph, element_id: &ElementId) -> Result<(), ServiceError> {
    if graph.get_element(element_id).is_none() {
        return Err(ServiceError::ElementNotFound(format!(
            "element {element_id} does not exist in the current workspace"
        )));
    }
    Ok(())
}

/// Blank payload fields are caller errors, rejected before the store is
/// touched (same posture as blank actors, enforced one layer up).
fn nonblank<'a>(field: &str, value: &'a str) -> Result<&'a str, ServiceError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ServiceError::InvalidInput(format!(
            "`{field}` is required and must be non-empty"
        )));
    }
    Ok(trimmed)
}

/// Record a review comment on a live element.
pub fn comment(
    workflow: &dyn WorkflowStore,
    graph: &ModelGraph,
    project: &ProjectId,
    element_id: &ElementId,
    body: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    require_element_exists(graph, element_id)?;
    let body = nonblank("body", body)?;
    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            element_id: element_id.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::Comment {
                body: body.to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Assign an engineer to a live element (folded state keeps the latest
/// assignee; the log keeps them all).
pub fn assign(
    workflow: &dyn WorkflowStore,
    graph: &ModelGraph,
    project: &ProjectId,
    element_id: &ElementId,
    assignee: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    require_element_exists(graph, element_id)?;
    let assignee = nonblank("assignee", assignee)?;
    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            element_id: element_id.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::EngineerAssigned {
                assignee: assignee.to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Transition a live element's approval state. `from` is derived
/// server-side from the folded log (a stale client must never forge a
/// transition's starting point); `to` must be a vocabulary member and
/// differ from the current state (a no-op records nothing — mirrors
/// `relink`'s same-endpoints rejection).
pub fn set_approval(
    workflow: &dyn WorkflowStore,
    graph: &ModelGraph,
    project: &ProjectId,
    element_id: &ElementId,
    to: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    require_element_exists(graph, element_id)?;
    let to = nonblank("to", to)?;
    if !APPROVAL_STATES.contains(&to) {
        return Err(ServiceError::InvalidInput(format!(
            "unknown approval state '{to}' — must be one of: {}",
            APPROVAL_STATES.join(", ")
        )));
    }
    let events = workflow
        .events(project, Some(element_id))
        .map_err(workflow_err)?;
    let state = fold_element_state(&events);
    let from = state
        .approval
        .map_or_else(|| APPROVAL_INITIAL.to_owned(), |(current, _, _)| current);
    if from == to {
        return Err(ServiceError::InvalidInput(format!(
            "element is already in approval state '{to}' — nothing to record"
        )));
    }
    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            element_id: element_id.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::ApprovalStateChanged {
                from,
                to: to.to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Record a sign-off attestation statement against a live element.
pub fn sign_off(
    workflow: &dyn WorkflowStore,
    graph: &ModelGraph,
    project: &ProjectId,
    element_id: &ElementId,
    statement: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    require_element_exists(graph, element_id)?;
    let statement = nonblank("statement", statement)?;
    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            element_id: element_id.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::SignOffAttestation {
                statement: statement.to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Deliberate, audited re-link of history from a dead id to its
/// successor (ADR-009: never automatic). The target must exist in the
/// current graph; the source must be a different identity.
pub fn relink(
    workflow: &dyn WorkflowStore,
    graph: &ModelGraph,
    project: &ProjectId,
    from: &ElementId,
    to: &ElementId,
    rationale: &str,
    actor: &str,
) -> Result<WorkflowEvent, ServiceError> {
    if from == to {
        return Err(ServiceError::InvalidInput(
            "relink source and target are the same element".to_owned(),
        ));
    }
    if graph.get_element(to).is_none() {
        return Err(ServiceError::ElementNotFound(format!(
            "relink target {to} does not exist in the current workspace"
        )));
    }
    workflow
        .append(NewWorkflowEvent {
            project: project.clone(),
            // The event is keyed on the NEW identity so it appears in the
            // successor's history; the payload records both ends.
            element_id: to.clone(),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::Relinked {
                from: from.clone(),
                to: to.clone(),
                rationale: rationale.to_owned(),
            },
        })
        .map_err(workflow_err)
}

/// Folded workflow state of one element, superseded flags filled and
/// orphan status computed against the current graph.
pub fn element_state(
    store: &dyn Store,
    workflow: &dyn WorkflowStore,
    graph: &ModelGraph,
    project: &ProjectId,
    element_id: &ElementId,
) -> Result<ElementWorkflowState, ServiceError> {
    let events = workflow
        .events(project, Some(element_id))
        .map_err(workflow_err)?;
    let mut state = fold_element_state(&events);

    // Superseded verdicts: one predicate application per distinct
    // baseline the element was attested against.
    let clearing_inputs: Vec<ClearingInput> = state
        .suspect_clearings
        .iter()
        .map(|c| ClearingInput {
            seq: c.seq,
            element: element_id.clone(),
            attested_commit: c.attested_commit.clone(),
        })
        .collect();
    let attested = attested_suspect_sets(store, project, &clearing_inputs);
    let outcome = apply_clearings(&[], &clearing_inputs, &attested);
    for clearing in &mut state.suspect_clearings {
        clearing.superseded = outcome.superseded.get(&clearing.seq).copied().unwrap_or(true);
    }

    // Verification attestations supersede on a direct content predicate:
    // "did THIS element change since `attested_commit`?" (a plain diff,
    // deliberately NOT the suspect machinery — suspicion is
    // baseline-relative with upstream propagation, the wrong semantics
    // for "I verified this element at this content"). An unresolvable
    // attested commit (evicted) is conservatively superseded — an
    // attestation that cannot prove the content is unchanged never
    // stands (same posture as suspect clearing).
    for attestation in &mut state.verification_attestations {
        attestation.superseded =
            storage::diff_snapshots(
                store,
                project,
                &attestation.attested_commit,
                None,
                Some(std::slice::from_ref(element_id)),
            )
            .map(|diff| !diff.is_empty())
            .unwrap_or(true);
    }

    Ok(ElementWorkflowState {
        state,
        orphaned: graph.get_element(element_id).is_none(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind};
    use sysml_store::{InMemoryStore, InMemoryWorkflowStore};

    /// B10 §3.5: manual-verification attestation supersedes on a DIRECT
    /// content predicate (this element changed since `attested_commit`) —
    /// deliberately not the baseline-relative suspect machinery.
    #[test]
    fn verification_attestation_supersedes_on_content_change() {
        let mut store = InMemoryStore::new();
        let workflow = InMemoryWorkflowStore::new();
        let project = ProjectId::new("p");

        let mut req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        req.set_prop("body", "v1");
        let req_id = req.id.clone();
        let mut g1 = ModelGraph::new();
        g1.add_element(req);
        storage::save_workspace_snapshot(&mut store, &project, &g1, "v1").unwrap();

        // Method outside the spec's closed set dies at the write boundary.
        let bad = attest_verification(
            &mut store, &workflow, &project, &g1, &req_id, "inspekt", "looked", "analyst",
        );
        assert!(matches!(bad, Err(ServiceError::InvalidInput(_))));

        let event = attest_verification(
            &mut store, &workflow, &project, &g1, &req_id, "inspect", "looked closely", "analyst",
        )
        .unwrap();
        assert!(matches!(
            event.kind,
            WorkflowEventKind::VerificationAttestation { .. }
        ));

        // Unchanged content → the attestation stands.
        let state = element_state(&store, &workflow, &g1, &project, &req_id).unwrap();
        assert_eq!(state.state.verification_attestations.len(), 1);
        assert!(!state.state.verification_attestations[0].superseded);

        // Edit the element → superseded at display time; the record stays.
        let mut g2 = g1.clone();
        if let Some(el) = g2.elements.get_mut(&req_id) {
            el.set_prop("body", "v2");
        }
        storage::save_workspace_snapshot(&mut store, &project, &g2, "v2").unwrap();
        let state = element_state(&store, &workflow, &g2, &project, &req_id).unwrap();
        assert_eq!(state.state.verification_attestations.len(), 1);
        assert!(state.state.verification_attestations[0].superseded);
    }

    /// End-to-end: doc edit → suspect → attest → cleared → edit again →
    /// suspicion re-fires and the attestation shows superseded.
    #[test]
    fn attest_clears_until_the_next_change() {
        let mut store = InMemoryStore::new();
        let workflow = InMemoryWorkflowStore::new();
        let project = ProjectId::new("p");

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let mut doc = Element::new_with_kind(ElementKind::Documentation);
        doc.owner = Some(req.id.clone());
        doc.set_prop("body", "v1");
        let doc_id = doc.id.clone();

        let mut g1 = ModelGraph::new();
        g1.add_element(req.clone());
        g1.add_element(doc);
        storage::save_workspace_snapshot(&mut store, &project, &g1, "v1").unwrap();
        storage::create_baseline(&mut store, &project, "B1", None, None).unwrap();

        // Edit → suspect appears, uncleared.
        let mut g2 = g1.clone();
        if let Some(el) = g2.elements.get_mut(&doc_id) {
            el.set_prop("body", "v2");
        }
        storage::save_workspace_snapshot(&mut store, &project, &g2, "v2").unwrap();
        let views = suspects_with_clearings(&store, &workflow, &project, "B1", None).unwrap();
        assert_eq!(views.len(), 1);
        assert!(views[0].cleared_by.is_none());

        // Attest at v2 → cleared; folded state shows a live attestation.
        let missing_actor = attest_suspect_clearing(
            &mut store, &workflow, &project, &g2, &req.id, "B1", "why", "  ",
        );
        assert!(matches!(missing_actor, Err(ServiceError::InvalidInput(_))));
        let event = attest_suspect_clearing(
            &mut store, &workflow, &project, &g2, &req.id, "B1", "intent unchanged", "analyst",
        )
        .unwrap();
        let views = suspects_with_clearings(&store, &workflow, &project, "B1", None).unwrap();
        assert_eq!(views[0].cleared_by, Some(event.seq));
        let state = element_state(&store, &workflow, &g2, &project, &req.id).unwrap();
        assert_eq!(state.state.suspect_clearings.len(), 1);
        assert!(!state.state.suspect_clearings[0].superseded);
        assert!(!state.orphaned);

        // Edit AGAIN → suspicion re-fires; the attestation is superseded.
        let mut g3 = g2.clone();
        if let Some(el) = g3.elements.get_mut(&doc_id) {
            el.set_prop("body", "v3");
        }
        storage::save_workspace_snapshot(&mut store, &project, &g3, "v3").unwrap();
        let views = suspects_with_clearings(&store, &workflow, &project, "B1", None).unwrap();
        assert_eq!(views.len(), 1);
        assert!(views[0].cleared_by.is_none());
        let state = element_state(&store, &workflow, &g3, &project, &req.id).unwrap();
        assert!(state.state.suspect_clearings[0].superseded);
    }

    #[test]
    fn attest_rejects_non_suspect_rows() {
        let mut store = InMemoryStore::new();
        let workflow = InMemoryWorkflowStore::new();
        let project = ProjectId::new("p");
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let mut g1 = ModelGraph::new();
        g1.add_element(req.clone());
        storage::save_workspace_snapshot(&mut store, &project, &g1, "v1").unwrap();
        storage::create_baseline(&mut store, &project, "B1", None, None).unwrap();
        // Nothing changed — attesting must fail loudly.
        let err = attest_suspect_clearing(
            &mut store, &workflow, &project, &g1, &req.id, "B1", "why", "analyst",
        )
        .unwrap_err();
        assert!(err.to_string().contains("not suspect"));
    }

    /// The four per-kind writes: existence gate, blank-payload
    /// rejection, and the happy path landing the right event kind.
    #[test]
    fn typed_writes_validate_and_append() {
        let workflow = InMemoryWorkflowStore::new();
        let project = ProjectId::new("p");
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let mut graph = ModelGraph::new();
        graph.add_element(req.clone());
        let dead = ElementId::from_string("dead-id");

        // Existence gate (all four share it — spot-check two).
        assert!(matches!(
            comment(&workflow, &graph, &project, &dead, "hi", "analyst"),
            Err(ServiceError::ElementNotFound(_))
        ));
        assert!(matches!(
            set_approval(&workflow, &graph, &project, &dead, "in_review", "analyst"),
            Err(ServiceError::ElementNotFound(_))
        ));

        // Blank payloads die in the service layer.
        assert!(matches!(
            comment(&workflow, &graph, &project, &req.id, "  ", "analyst"),
            Err(ServiceError::InvalidInput(_))
        ));
        assert!(matches!(
            assign(&workflow, &graph, &project, &req.id, "", "analyst"),
            Err(ServiceError::InvalidInput(_))
        ));
        assert!(matches!(
            sign_off(&workflow, &graph, &project, &req.id, " \n", "analyst"),
            Err(ServiceError::InvalidInput(_))
        ));

        // Happy path: payloads are stored trimmed, kinds are right.
        let c = comment(&workflow, &graph, &project, &req.id, " looks fine ", "analyst").unwrap();
        assert_eq!(
            c.kind,
            WorkflowEventKind::Comment {
                body: "looks fine".to_owned()
            }
        );
        let a = assign(&workflow, &graph, &project, &req.id, "sam", "analyst").unwrap();
        assert_eq!(
            a.kind,
            WorkflowEventKind::EngineerAssigned {
                assignee: "sam".to_owned()
            }
        );
        let s = sign_off(&workflow, &graph, &project, &req.id, "reviewed rev B", "analyst").unwrap();
        assert_eq!(
            s.kind,
            WorkflowEventKind::SignOffAttestation {
                statement: "reviewed rev B".to_owned()
            }
        );

        // Blank actor still dies at the store layer, lifted to InvalidInput.
        assert!(matches!(
            comment(&workflow, &graph, &project, &req.id, "hi", "  "),
            Err(ServiceError::InvalidInput(_))
        ));
    }

    /// Approval transitions: server-derived `from`, closed vocabulary,
    /// no-op rejection.
    #[test]
    fn set_approval_derives_from_and_validates_vocabulary() {
        let workflow = InMemoryWorkflowStore::new();
        let project = ProjectId::new("p");
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let mut graph = ModelGraph::new();
        graph.add_element(req.clone());

        // Unknown state is rejected before anything is written.
        let err = set_approval(&workflow, &graph, &project, &req.id, "aproved", "analyst")
            .unwrap_err();
        assert!(err.to_string().contains("unknown approval state"));

        // No-op: the initial state IS draft — "transitioning" to it
        // records nothing.
        let err =
            set_approval(&workflow, &graph, &project, &req.id, "draft", "analyst").unwrap_err();
        assert!(err.to_string().contains("already in approval state"));

        // First real transition derives from == the initial state.
        let e1 = set_approval(&workflow, &graph, &project, &req.id, "in_review", "analyst").unwrap();
        assert_eq!(
            e1.kind,
            WorkflowEventKind::ApprovalStateChanged {
                from: APPROVAL_INITIAL.to_owned(),
                to: "in_review".to_owned()
            }
        );

        // Second transition chains off the folded state, not the client.
        let e2 = set_approval(&workflow, &graph, &project, &req.id, "approved", "sam").unwrap();
        assert_eq!(
            e2.kind,
            WorkflowEventKind::ApprovalStateChanged {
                from: "in_review".to_owned(),
                to: "approved".to_owned()
            }
        );

        // Repeating the current state is a no-op error, not a duplicate.
        assert!(
            set_approval(&workflow, &graph, &project, &req.id, "approved", "sam").is_err()
        );
    }

    #[test]
    fn relink_validates_target_and_keys_on_successor() {
        let workflow = InMemoryWorkflowStore::new();
        let project = ProjectId::new("p");
        let alive = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R");
        let dead = ElementId::from_string("dead-id");
        let mut graph = ModelGraph::new();
        graph.add_element(alive.clone());

        assert!(relink(&workflow, &graph, &project, &dead, &dead, "r", "analyst").is_err());
        assert!(relink(
            &workflow,
            &graph,
            &project,
            &dead,
            &ElementId::from_string("also-missing"),
            "r",
            "analyst"
        )
        .is_err());

        let event =
            relink(&workflow, &graph, &project, &dead, &alive.id, "successor", "analyst").unwrap();
        assert_eq!(event.element_id, alive.id);
    }
}
