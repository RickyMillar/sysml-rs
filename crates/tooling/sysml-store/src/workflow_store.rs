//! Workflow sidecar store — append-only event log for review/process
//! facts that deliberately live OUTSIDE the model (steward ruling,
//! requirements-workbench-design.md §1.1 + the 2026-07-16 v1.5b
//! follow-up ruling).
//!
//! Design contract (binding):
//! - **Append-only.** Never a mutable snapshot table; current state is
//!   derived by folding the log ([`fold_element_state`]).
//! - **`actor` is REQUIRED** on every event — no silent `"local"`
//!   default; an empty actor is a hard error at this layer regardless
//!   of what the command layer validated.
//! - **Keyed `(ProjectId, ElementId)`**; commits/baselines are event
//!   payload, never keys (one requirement's history spans baselines).
//! - **ADR-009 honesty**: events keyed on a dead `ElementId` are never
//!   deleted or name-matched onto a successor; re-linking is itself an
//!   audited event ([`WorkflowEventKind::Relinked`]).
//! - **`event_id == seq`**: store-assigned, monotonic per project,
//!   starting at 1. `seq` is load-bearing for the JSONL backend's
//!   fail-hard integrity check, not just ordering.
//! - Sidecar facts never reach `.sysml` source, canonical JSON export,
//!   or report claims attributed to the model.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use sysml_id::{ElementId, ProjectId};

/// Schema version stamped on every event at append time (per-event, not
/// per-table — the log outlives schema migrations).
pub const WORKFLOW_SCHEMA_VERSION: u32 = 1;

/// Errors from workflow-store operations. All are hard errors — this is
/// an audit trail; there is no degraded mode.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowStoreError {
    /// `actor` was empty/blank. Required on every event (§1.1).
    #[error("workflow event rejected: `actor` is required and must be non-empty (no silent default identity)")]
    MissingActor,

    /// I/O failure on the durable backend.
    #[error("workflow store I/O error: {0}")]
    Io(String),

    /// A malformed line strictly BEFORE the final line of the JSONL
    /// file. Never skipped: mid-file corruption means the audit trail
    /// cannot be trusted (fail-hard rule).
    #[error("workflow log corrupt at line {line}: {message} — refusing to load a damaged audit trail")]
    Corrupt { line: usize, message: String },

    /// Per-project `seq` did not continue monotonically on load —
    /// evidence of truncation or tampering.
    #[error("workflow log seq discontinuity for project {project}: expected {expected}, found {found}")]
    SeqGap {
        project: String,
        expected: u64,
        found: u64,
    },

    /// Event failed to serialize (should be unreachable for valid data).
    #[error("workflow event serialization error: {0}")]
    Serialization(String),
}

/// What happened to a workflow-relevant fact. Tagged on `kind` for the
/// wire; v1 variants per the §1.1 ruling (+ `Relinked`, the audited
/// ADR-009 re-attachment).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowEventKind {
    ApprovalStateChanged {
        from: String,
        to: String,
    },
    Comment {
        body: String,
    },
    SignOffAttestation {
        statement: String,
    },
    /// "Attest unchanged intent": clears computed suspicion of this
    /// requirement against ONE baseline, valid only while the
    /// requirement's content still equals `attested_commit` (the
    /// display-time predicate recomputes this — the store never caches
    /// an is-suspect boolean).
    SuspectClearingAttestation {
        /// Human name the engineer attested against — write-time
        /// provenance (a commit can carry several baseline names, and
        /// names minted later must not retroactively re-label history).
        baseline_name: String,
        /// The baseline's commit (content digest).
        baseline_commit: String,
        /// The content digest that was LATEST when the engineer
        /// attested. Later content changes supersede the attestation.
        attested_commit: String,
        rationale: String,
    },
    EngineerAssigned {
        assignee: String,
    },
    /// Deliberate, audited re-attachment of history from a dead
    /// `ElementId` to its successor (ADR-009: never automatic, never
    /// name-matched).
    Relinked {
        from: ElementId,
        to: ElementId,
        rationale: String,
    },
    /// MANUAL verification of the element (B10 layer 3, the human leg):
    /// "I verified this by inspection/demo/…". An ATTESTATION — actor
    /// required, digest-pinned, supersedable — NEVER a computed verdict:
    /// it must never enter verdict stores, timelines, or rollups, and
    /// never render as a verdict chip (verification-evidence-taxonomy.md
    /// hard line).
    VerificationAttestation {
        /// Layer-1 vocabulary the act corresponds to: one of
        /// `sysml_core::metadata::VERIFICATION_METHOD_KINDS`
        /// (`inspect | analyze | demo | test`) — spec-normative closed
        /// set, validated at the append command, one home in sysml-core.
        method: String,
        /// The attestation statement ("visually inspected creepage
        /// distance against IEC 62752 table 3…").
        statement: String,
        /// The content digest that was LATEST when the engineer
        /// attested. Later content changes supersede the attestation
        /// (display-time predicate, same as `SuspectClearingAttestation`
        /// — never a cached boolean).
        attested_commit: String,
    },
}

/// One appended event. `seq` is store-assigned and doubles as the
/// event id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowEvent {
    pub seq: u64,
    pub schema_version: u32,
    pub project: ProjectId,
    pub element_id: ElementId,
    /// Who performed the act. Required, explicit — resolved from a
    /// per-user setting, never an OS-username or `"local"` default.
    pub actor: String,
    /// Unix milliseconds (session-archive convention).
    pub timestamp_ms: i64,
    #[serde(flatten)]
    pub kind: WorkflowEventKind,
}

/// Append input — everything the caller supplies; the store stamps
/// `seq`, `schema_version`, and `timestamp_ms`.
#[derive(Debug, Clone)]
pub struct NewWorkflowEvent {
    pub project: ProjectId,
    pub element_id: ElementId,
    pub actor: String,
    pub kind: WorkflowEventKind,
}

/// The workflow sidecar store. One instance per service process, set at
/// construction (`SysmlService::with_workflow_store`) — commands carry
/// an explicit `project`, so there is no per-call path resolution.
pub trait WorkflowStore: Send + Sync {
    /// Append one event; returns it with `seq`/`schema_version`/
    /// `timestamp_ms` stamped. Hard-errors on a blank actor.
    fn append(&self, event: NewWorkflowEvent) -> Result<WorkflowEvent, WorkflowStoreError>;

    /// Events for a project, oldest-first (natural log order); filtered
    /// to one element when `element_id` is given.
    fn events(
        &self,
        project: &ProjectId,
        element_id: Option<&ElementId>,
    ) -> Result<Vec<WorkflowEvent>, WorkflowStoreError>;
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn validate_actor(actor: &str) -> Result<(), WorkflowStoreError> {
    if actor.trim().is_empty() {
        return Err(WorkflowStoreError::MissingActor);
    }
    Ok(())
}

// ── In-memory backend ────────────────────────────────────────────────

/// Test/default backend: the same per-project log vectors the JSONL
/// backend folds into, minus durability.
#[derive(Default)]
pub struct InMemoryWorkflowStore {
    inner: RwLock<HashMap<ProjectId, Vec<WorkflowEvent>>>,
}

impl InMemoryWorkflowStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WorkflowStore for InMemoryWorkflowStore {
    fn append(&self, event: NewWorkflowEvent) -> Result<WorkflowEvent, WorkflowStoreError> {
        validate_actor(&event.actor)?;
        let mut inner = self
            .inner
            .write()
            .map_err(|e| WorkflowStoreError::Io(format!("lock poisoned: {e}")))?;
        let log = inner.entry(event.project.clone()).or_default();
        let stamped = WorkflowEvent {
            seq: log.len() as u64 + 1,
            schema_version: WORKFLOW_SCHEMA_VERSION,
            project: event.project,
            element_id: event.element_id,
            actor: event.actor,
            timestamp_ms: now_ms(),
            kind: event.kind,
        };
        log.push(stamped.clone());
        Ok(stamped)
    }

    fn events(
        &self,
        project: &ProjectId,
        element_id: Option<&ElementId>,
    ) -> Result<Vec<WorkflowEvent>, WorkflowStoreError> {
        let inner = self
            .inner
            .read()
            .map_err(|e| WorkflowStoreError::Io(format!("lock poisoned: {e}")))?;
        Ok(inner
            .get(project)
            .map(|log| {
                log.iter()
                    .filter(|e| element_id.is_none_or(|id| &e.element_id == id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }
}

// ── Durable JSONL backend ────────────────────────────────────────────

/// What `JsonlWorkflowStore::open` recovered. A torn FINAL line (POSIX
/// partial append) is the one tolerated corruption: it is truncated
/// away and reported here so the caller can log it LOUDLY — never
/// silently. Mid-file corruption is a hard error instead.
#[derive(Debug, Default)]
pub struct JsonlRecovery {
    /// The discarded torn tail, if any (raw text, for the caller's log).
    pub torn_tail_discarded: Option<String>,
}

/// Durable single-file backend: one JSONL file for ALL projects (each
/// line carries its project), per the §1.1 "durable single-file" ruling.
/// Lives in a local per-project-machine data directory — NOT inside the
/// workspace repo (model-content history and process history have
/// different lifecycles; ruled 2026-07-16).
pub struct JsonlWorkflowStore {
    path: PathBuf,
    inner: RwLock<JsonlInner>,
}

struct JsonlInner {
    events: HashMap<ProjectId, Vec<WorkflowEvent>>,
}

impl JsonlWorkflowStore {
    /// Open (creating parent dirs + file as needed), replaying the log
    /// into memory with fail-hard integrity checks.
    pub fn open(path: impl Into<PathBuf>) -> Result<(Self, JsonlRecovery), WorkflowStoreError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| WorkflowStoreError::Io(format!("create {}: {e}", parent.display())))?;
        }

        let mut recovery = JsonlRecovery::default();
        let mut events: HashMap<ProjectId, Vec<WorkflowEvent>> = HashMap::new();

        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| WorkflowStoreError::Io(format!("read {}: {e}", path.display())))?;
            let lines: Vec<&str> = raw.lines().collect();
            let mut keep_bytes: u64 = 0;
            for (idx, line) in lines.iter().enumerate() {
                if line.trim().is_empty() {
                    // Blank line: tolerate only as a torn tail.
                    if idx + 1 == lines.len() {
                        break;
                    }
                    return Err(WorkflowStoreError::Corrupt {
                        line: idx + 1,
                        message: "blank line inside the log".to_owned(),
                    });
                }
                match serde_json::from_str::<WorkflowEvent>(line) {
                    Ok(event) => {
                        let log = events.entry(event.project.clone()).or_default();
                        let expected = log.len() as u64 + 1;
                        if event.seq != expected {
                            return Err(WorkflowStoreError::SeqGap {
                                project: event.project.to_string(),
                                expected,
                                found: event.seq,
                            });
                        }
                        log.push(event);
                        // +1 for the newline `lines()` stripped.
                        keep_bytes += line.len() as u64 + 1;
                    }
                    Err(e) if idx + 1 == lines.len() => {
                        // Torn final append: truncate it away, report it.
                        recovery.torn_tail_discarded = Some((*line).to_owned());
                        let file = std::fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .map_err(|err| {
                                WorkflowStoreError::Io(format!("open for truncate: {err}"))
                            })?;
                        file.set_len(keep_bytes).map_err(|err| {
                            WorkflowStoreError::Io(format!("truncate torn tail: {err}"))
                        })?;
                        let _ = e; // parse error detail folded into recovery
                    }
                    Err(e) => {
                        return Err(WorkflowStoreError::Corrupt {
                            line: idx + 1,
                            message: e.to_string(),
                        });
                    }
                }
            }
        }

        Ok((
            Self {
                path,
                inner: RwLock::new(JsonlInner { events }),
            },
            recovery,
        ))
    }

    /// The backing file path (for the caller's startup log line).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl WorkflowStore for JsonlWorkflowStore {
    fn append(&self, event: NewWorkflowEvent) -> Result<WorkflowEvent, WorkflowStoreError> {
        validate_actor(&event.actor)?;
        let mut inner = self
            .inner
            .write()
            .map_err(|e| WorkflowStoreError::Io(format!("lock poisoned: {e}")))?;
        let log = inner.events.entry(event.project.clone()).or_default();
        let stamped = WorkflowEvent {
            seq: log.len() as u64 + 1,
            schema_version: WORKFLOW_SCHEMA_VERSION,
            project: event.project,
            element_id: event.element_id,
            actor: event.actor,
            timestamp_ms: now_ms(),
            kind: event.kind,
        };
        let line = serde_json::to_string(&stamped)
            .map_err(|e| WorkflowStoreError::Serialization(e.to_string()))?;

        // Durability before visibility: the event is only added to the
        // in-memory log after it is on disk (audit-trail posture).
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| WorkflowStoreError::Io(format!("open {}: {e}", self.path.display())))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_data())
            .map_err(|e| WorkflowStoreError::Io(format!("append {}: {e}", self.path.display())))?;

        log.push(stamped.clone());
        Ok(stamped)
    }

    fn events(
        &self,
        project: &ProjectId,
        element_id: Option<&ElementId>,
    ) -> Result<Vec<WorkflowEvent>, WorkflowStoreError> {
        let inner = self
            .inner
            .read()
            .map_err(|e| WorkflowStoreError::Io(format!("lock poisoned: {e}")))?;
        Ok(inner
            .events
            .get(project)
            .map(|log| {
                log.iter()
                    .filter(|e| element_id.is_none_or(|id| &e.element_id == id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }
}

// ── Derived (folded) state ───────────────────────────────────────────

/// One suspect-clearing attestation, as folded state. `superseded` is
/// filled by the service-layer predicate (it needs suspect computations
/// this crate cannot do) — the record itself never disappears from the
/// list: append-only logs don't forget stale history.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClearingRecord {
    pub seq: u64,
    pub baseline_name: String,
    pub baseline_commit: String,
    pub attested_commit: String,
    pub actor: String,
    pub timestamp_ms: i64,
    pub rationale: String,
    /// True when the requirement changed again after this attestation
    /// (display-time computed; false as folded default).
    pub superseded: bool,
}

/// One manual-verification attestation, as folded state (B10 layer 3,
/// human leg). Same supersession discipline as [`ClearingRecord`]:
/// `superseded` is a display-time predicate computed by the service
/// (current content digest moved past `attested_commit`) — the record
/// itself never disappears from the list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VerificationAttestationRecord {
    pub seq: u64,
    /// Layer-1 method vocabulary (`inspect | analyze | demo | test`).
    pub method: String,
    pub statement: String,
    pub attested_commit: String,
    pub actor: String,
    pub timestamp_ms: i64,
    /// True when the element changed again after this attestation
    /// (display-time computed; false as folded default).
    pub superseded: bool,
}

/// Current workflow state of one element, derived by folding its log.
/// Never authored, never cached — recompute from events.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct WorkflowElementState {
    /// Latest approval transition, if any: `(state, by, at_ms)`.
    pub approval: Option<(String, String, i64)>,
    /// Latest assignee, if any.
    pub assignee: Option<String>,
    /// All sign-off statements, oldest-first: `(statement, by, at_ms)`.
    pub sign_offs: Vec<(String, String, i64)>,
    /// All suspect-clearing attestations, oldest-first.
    pub suspect_clearings: Vec<ClearingRecord>,
    /// All manual-verification attestations, oldest-first (B10).
    pub verification_attestations: Vec<VerificationAttestationRecord>,
    pub comment_count: usize,
}

/// Fold one element's events (oldest-first) into current state.
pub fn fold_element_state(events: &[WorkflowEvent]) -> WorkflowElementState {
    let mut state = WorkflowElementState::default();
    for event in events {
        match &event.kind {
            WorkflowEventKind::ApprovalStateChanged { to, .. } => {
                state.approval = Some((to.clone(), event.actor.clone(), event.timestamp_ms));
            }
            WorkflowEventKind::EngineerAssigned { assignee } => {
                state.assignee = Some(assignee.clone());
            }
            WorkflowEventKind::SignOffAttestation { statement } => {
                state
                    .sign_offs
                    .push((statement.clone(), event.actor.clone(), event.timestamp_ms));
            }
            WorkflowEventKind::SuspectClearingAttestation {
                baseline_name,
                baseline_commit,
                attested_commit,
                rationale,
            } => {
                state.suspect_clearings.push(ClearingRecord {
                    seq: event.seq,
                    baseline_name: baseline_name.clone(),
                    baseline_commit: baseline_commit.clone(),
                    attested_commit: attested_commit.clone(),
                    actor: event.actor.clone(),
                    timestamp_ms: event.timestamp_ms,
                    rationale: rationale.clone(),
                    superseded: false,
                });
            }
            WorkflowEventKind::VerificationAttestation {
                method,
                statement,
                attested_commit,
            } => {
                state.verification_attestations.push(VerificationAttestationRecord {
                    seq: event.seq,
                    method: method.clone(),
                    statement: statement.clone(),
                    attested_commit: attested_commit.clone(),
                    actor: event.actor.clone(),
                    timestamp_ms: event.timestamp_ms,
                    superseded: false,
                });
            }
            WorkflowEventKind::Comment { .. } => {
                state.comment_count += 1;
            }
            WorkflowEventKind::Relinked { .. } => {
                // Relink is history, not element state; consumers read it
                // from the raw log.
            }
        }
    }
    state
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn new_event(project: &str, actor: &str) -> NewWorkflowEvent {
        NewWorkflowEvent {
            project: ProjectId::new(project),
            element_id: ElementId::from_string("el-1"),
            actor: actor.to_owned(),
            kind: WorkflowEventKind::Comment {
                body: "hello".to_owned(),
            },
        }
    }

    #[test]
    fn append_stamps_seq_per_project_and_requires_actor() {
        let store = InMemoryWorkflowStore::new();
        assert!(matches!(
            store.append(new_event("p1", "  ")),
            Err(WorkflowStoreError::MissingActor)
        ));
        let a = store.append(new_event("p1", "ricky")).unwrap();
        let b = store.append(new_event("p1", "ricky")).unwrap();
        let c = store.append(new_event("p2", "ricky")).unwrap();
        assert_eq!((a.seq, b.seq, c.seq), (1, 2, 1));
        assert_eq!(a.schema_version, WORKFLOW_SCHEMA_VERSION);
        assert_eq!(store.events(&ProjectId::new("p1"), None).unwrap().len(), 2);
    }

    #[test]
    fn events_filters_by_element() {
        let store = InMemoryWorkflowStore::new();
        let mut e = new_event("p", "r");
        e.element_id = ElementId::from_string("a");
        store.append(e).unwrap();
        let mut e = new_event("p", "r");
        e.element_id = ElementId::from_string("b");
        store.append(e).unwrap();
        let only_a = store
            .events(&ProjectId::new("p"), Some(&ElementId::from_string("a")))
            .unwrap();
        assert_eq!(only_a.len(), 1);
    }

    #[test]
    fn jsonl_roundtrip_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.jsonl");
        {
            let (store, rec) = JsonlWorkflowStore::open(&path).unwrap();
            assert!(rec.torn_tail_discarded.is_none());
            store.append(new_event("p1", "ricky")).unwrap();
            store.append(new_event("p1", "ricky")).unwrap();
        }
        let (store, rec) = JsonlWorkflowStore::open(&path).unwrap();
        assert!(rec.torn_tail_discarded.is_none());
        let events = store.events(&ProjectId::new("p1"), None).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].seq, 2);
        // Appends continue the seq after replay.
        let next = store.append(new_event("p1", "ricky")).unwrap();
        assert_eq!(next.seq, 3);
    }

    #[test]
    fn jsonl_torn_final_line_is_truncated_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.jsonl");
        {
            let (store, _) = JsonlWorkflowStore::open(&path).unwrap();
            store.append(new_event("p1", "ricky")).unwrap();
        }
        // Simulate a torn append.
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{\"seq\":2,\"schema_ver").unwrap();
        }
        let (store, rec) = JsonlWorkflowStore::open(&path).unwrap();
        assert!(rec.torn_tail_discarded.is_some());
        assert_eq!(store.events(&ProjectId::new("p1"), None).unwrap().len(), 1);
        // And the file itself was repaired: a third open sees no tail.
        drop(store);
        let (_, rec2) = JsonlWorkflowStore::open(&path).unwrap();
        assert!(rec2.torn_tail_discarded.is_none());
    }

    #[test]
    fn jsonl_midfile_corruption_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.jsonl");
        {
            let (store, _) = JsonlWorkflowStore::open(&path).unwrap();
            store.append(new_event("p1", "ricky")).unwrap();
            store.append(new_event("p1", "ricky")).unwrap();
        }
        // Corrupt the FIRST line.
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = raw.lines().map(str::to_owned).collect();
        lines[0] = "garbage".to_owned();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        assert!(matches!(
            JsonlWorkflowStore::open(&path),
            Err(WorkflowStoreError::Corrupt { line: 1, .. })
        ));
    }

    #[test]
    fn jsonl_seq_gap_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.jsonl");
        {
            let (store, _) = JsonlWorkflowStore::open(&path).unwrap();
            store.append(new_event("p1", "ricky")).unwrap();
            store.append(new_event("p1", "ricky")).unwrap();
        }
        // Delete the first line (history truncated from the front).
        let raw = std::fs::read_to_string(&path).unwrap();
        let tail: Vec<&str> = raw.lines().skip(1).collect();
        std::fs::write(&path, tail.join("\n") + "\n").unwrap();
        assert!(matches!(
            JsonlWorkflowStore::open(&path),
            Err(WorkflowStoreError::SeqGap { .. })
        ));
    }

    #[test]
    fn fold_derives_state_and_keeps_all_clearings() {
        let store = InMemoryWorkflowStore::new();
        let el = ElementId::from_string("req-1");
        let project = ProjectId::new("p");
        for kind in [
            WorkflowEventKind::ApprovalStateChanged {
                from: "draft".into(),
                to: "in_review".into(),
            },
            WorkflowEventKind::EngineerAssigned {
                assignee: "sam".into(),
            },
            WorkflowEventKind::SuspectClearingAttestation {
                baseline_name: "B1 — PDR".into(),
                baseline_commit: "aaa".into(),
                attested_commit: "bbb".into(),
                rationale: "intent unchanged".into(),
            },
            WorkflowEventKind::Comment {
                body: "looks fine".into(),
            },
            WorkflowEventKind::SuspectClearingAttestation {
                baseline_name: "B1 — PDR".into(),
                baseline_commit: "aaa".into(),
                attested_commit: "ccc".into(),
                rationale: "re-checked".into(),
            },
        ] {
            store
                .append(NewWorkflowEvent {
                    project: project.clone(),
                    element_id: el.clone(),
                    actor: "ricky".into(),
                    kind,
                })
                .unwrap();
        }
        let state = fold_element_state(&store.events(&project, Some(&el)).unwrap());
        assert_eq!(state.approval.as_ref().unwrap().0, "in_review");
        assert_eq!(state.assignee.as_deref(), Some("sam"));
        assert_eq!(state.comment_count, 1);
        // BOTH clearings retained (append-only history), oldest first.
        assert_eq!(state.suspect_clearings.len(), 2);
        assert_eq!(state.suspect_clearings[0].attested_commit, "bbb");
        assert_eq!(state.suspect_clearings[1].attested_commit, "ccc");
    }
}
