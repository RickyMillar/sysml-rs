//! Verification EXECUTIONS projection (P1) + execution-side latest-status
//!
//! The archive (`SessionArchive` + `ArchivedVerdict`s) already IS the
//! execution store; nothing here is a new store. An **execution** is a
//! recorded performance of one-or-more verification cases in a context: an
//! archived session (trajectory `sessions.verify`, or an external
//! `record_external` ingest) that carries at least one verdict. A pure
//! simulation run with no verdicts is NOT a verification execution and is
//! filtered out honestly.
//!
//! Naming (steward, binding): the concept is **execution** in every code
//! identifier — never an unqualified "run" (which collides with
//! `SessionOrigin::Run`).
//!
//! ## One home, two commands
//!
//! Both `sysml.verify.executions` (P1) and `sysml.verify.latest_status`
//! (P2) compose the SAME core: [`build_executions`] produces the newest-
//! first execution rows, and [`build_latest_status`] is a single reduction
//! over those rows (latest verdict per case × evaluation_mode). There is no
//! second walk of the archive and no second scoping rule.
//!
//! ## Scoping
//!
//! Reuses `verify_timeline`'s discipline exactly: sessions are filtered by
//! B6 provenance `workspace_root` (the real workspace identity). `None`
//! means "the whole archive" — the honest shape when the caller has no
//! resolvable root. See [`crate::verify_timeline::build_timeline`].
//!
//! ## Staleness (P6)
//!
//! Each per-case result carries the case's stored subtree digest
//! (`ArchivedVerdict::case_digest`, pinned at record time) and a
//! server-computed `case_changed_since` = stored-digest ≠ current-subtree-
//! digest. It is `null` when either side is unresolvable (no stored digest,
//! or the case no longer resolves in the current model) — never fabricated.
//! External executions additionally carry `matches_current_model` over the
//! whole-model digest, identical to `verify.timeline`.

use std::collections::HashMap;

use serde::Serialize;
use sysml_store::{
    ArchiveFilter, ArchivedSession, ArchivedVerdict, EvaluationMode, SessionArchive,
    SessionOrigin, SessionProvenance, VerdictCounts,
};

use crate::error::ServiceError;

/// Current resolution of a verification case, keyed by NAME (the archive's
/// `ArchivedVerdict.case_id`). Supplied by the command layer, which holds
/// the workspace graph — the projection itself stays graph-free.
#[derive(Debug, Clone)]
pub struct CaseResolution {
    /// The case's element id in the current workspace.
    pub element_id: String,
    /// The case's CURRENT subtree digest (`ModelGraph::subtree_digest`).
    /// `None` if it could not be computed — leaves `case_changed_since`
    /// honestly unanswered rather than fabricating a comparison.
    pub subtree_digest: Option<String>,
}

/// One per-case result inside an execution.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionResult {
    /// Verification-case name (the archive's stable key).
    pub case_id: String,
    /// The 4-valued verdict, lowercase string.
    pub verdict: String,
    /// How this verdict was produced (B10 layer 2).
    pub evaluation_mode: EvaluationMode,
    /// When this verdict was recorded (Unix ms). Per-verdict, not per-
    /// execution: a long-lived `sessions.verify` session appends verdicts
    /// at different times into one record.
    pub timestamp: i64,
    /// The case's subtree digest as pinned at record time (P6). Absent on
    /// records predating the field — honest "unknown".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_digest: Option<String>,
    /// `Some(true)` = the case's content changed since this execution,
    /// `Some(false)` = unchanged, `null` = unresolvable (no stored digest,
    /// or the case no longer resolves). Server-computed; never fabricated.
    pub case_changed_since: Option<bool>,
    /// The run this verdict came out of: session id + the tick it was
    /// evaluated at (+ the element, when one resolved).
    ///
    /// Populated on TRAJECTORY verdicts only — a static desk check has no run
    /// to point at. `null` on a record that genuinely has none (pre-B10):
    /// honest "unknown", never a fabricated session id.
    ///
    /// ALWAYS SERIALIZED, deliberately. With `skip_serializing_if` a reader
    /// could not tell "this record has no evidence" from "this server is too
    /// old to report evidence" — both arrived as a missing key. That
    /// ambiguity is not hypothetical: a brand-new run at tick 5001 was
    /// reported to the user as "predates evidence capture" because the server
    /// answering had been built before this field existed. An explicit `null`
    /// makes absence a statement the server made, rather than one the client
    /// inferred from silence.
    pub evidence: Option<sysml_store::ArchivedEvidence>,
}

/// External-run identity block on an execution (External origin only).
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionExternal {
    /// Producing tool (e.g. `"pytest-7.4"`, `"hil-bench-2"`).
    pub tool: String,
    /// Digest the client declared the results were produced against.
    pub declared_digest: String,
    /// Opaque run reference in the tool's namespace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<String>,
    /// `declared_digest == <current whole-model digest>`, server-computed
    /// (same rule as `verify.timeline`). `None` when nothing is loaded.
    pub matches_current_model: Option<bool>,
}

/// One execution row (P1).
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionRow {
    /// = the archived session id.
    pub execution_id: String,
    /// Session origin as archived (snake_case string on the wire).
    pub origin: SessionOrigin,
    /// User-visible label, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// When the execution ran (the archive entry's `created_at`, Unix ms).
    pub timestamp: i64,
    /// Execution-level mode: External origin → `external`, else
    /// `trajectory`.
    pub evaluation_mode: EvaluationMode,
    /// B6 provenance block as stored (`model_digest`, `git?`,
    /// `workspace_root?`). Absent on records predating capture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<SessionProvenance>,
    /// External-run identity, present for External-origin executions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<ExecutionExternal>,
    /// Per-case results.
    pub results: Vec<ExecutionResult>,
    /// Verdict tally over `results`.
    pub counts: VerdictCounts,
}

/// Response of `sysml.verify.executions`.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionsResult {
    /// Executions, newest-first.
    pub executions: Vec<ExecutionRow>,
}

/// Latest trajectory verdict for a case (P2).
#[derive(Debug, Clone, Serialize)]
pub struct LatestTrajectory {
    /// Lowercase verdict string.
    pub verdict: String,
    /// Execution that produced it.
    pub execution_id: String,
    /// Verdict timestamp (Unix ms).
    pub timestamp: i64,
    /// `case_changed_since` for this case at that execution.
    pub case_changed_since: Option<bool>,
    /// Whole-model digest the trajectory ran against (from execution
    /// provenance). Absent if the execution had no provenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_digest: Option<String>,
    /// Session id + tick this verdict was evaluated at. A trajectory verdict
    /// is a claim ABOUT A RUN, so the run has to travel with it — without
    /// this, "latest run: PASS" is unfalsifiable from the UI.
    ///
    /// Always serialized; `null` means the record genuinely has none. See
    /// [`ExecutionResult::evidence`] for why a missing key and a null are not
    /// allowed to look the same.
    pub evidence: Option<sysml_store::ArchivedEvidence>,
}

/// Latest external verdict for a case (P2).
#[derive(Debug, Clone, Serialize)]
pub struct LatestExternal {
    /// Lowercase verdict string.
    pub verdict: String,
    /// Execution that produced it.
    pub execution_id: String,
    /// Verdict timestamp (Unix ms).
    pub timestamp: i64,
    /// Producing tool, if recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Whole-model staleness of the external run.
    pub matches_current_model: Option<bool>,
    /// `case_changed_since` for this case at that execution.
    pub case_changed_since: Option<bool>,
}

/// Context-qualified latest status for one case — NEVER one flat
/// consolidated verdict (the §2.1a(d) discipline).
#[derive(Debug, Clone, Default, Serialize)]
pub struct LatestByMode {
    /// Latest trajectory-evaluated verdict, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trajectory: Option<LatestTrajectory>,
    /// Latest externally-ingested verdict, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<LatestExternal>,
}

/// One case's latest status across executions.
#[derive(Debug, Clone, Serialize)]
pub struct CaseLatest {
    /// Verification-case name.
    pub case_id: String,
    /// Case element id in the current workspace, when it still resolves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_element_id: Option<String>,
    /// Latest verdict per evaluation mode.
    pub latest: LatestByMode,
}

/// Response of `sysml.verify.latest_status`.
#[derive(Debug, Clone, Serialize)]
pub struct LatestStatusResult {
    /// One entry per case that appears in any execution.
    pub cases: Vec<CaseLatest>,
}

/// Collect verdict-carrying archived sessions in the requested workspace
/// scope, newest-first. The ONE archive walk both commands share.
///
/// Scoping mirrors [`crate::verify_timeline::build_timeline`] exactly:
/// sessions are filtered by B6 provenance `workspace_root`; a session with
/// no recorded root is excluded when the filter is active (unattributable).
/// `None` spans the whole archive.
fn scoped_verdict_sessions(
    archive: &dyn SessionArchive,
    workspace_root: Option<&str>,
) -> Vec<ArchivedSession> {
    // `archive.list` is newest-first by `created_at`, so the result carries
    // that order through.
    let mut out = Vec::new();
    for summary in archive.list(ArchiveFilter::default()) {
        // Only sessions carrying at least one verdict are executions.
        if summary.verdict_counts.total() == 0 {
            continue;
        }
        let Some(full) = archive.get(&summary.id) else {
            continue;
        };
        if let Some(expected_root) = workspace_root {
            let recorded_root = full
                .provenance
                .as_ref()
                .and_then(|p| p.workspace_root.as_deref());
            if recorded_root != Some(expected_root) {
                continue;
            }
        }
        out.push(full);
    }
    out
}

/// `stored case_digest` vs the case's `current subtree digest` → the
/// per-entry stale flag. `None` (unresolvable) whenever either side is
/// missing — never fabricated.
fn case_changed_since(
    v: &ArchivedVerdict,
    current_cases: &HashMap<String, CaseResolution>,
) -> Option<bool> {
    let stored = v.case_digest.as_deref()?;
    let current = current_cases.get(&v.case_id)?.subtree_digest.as_deref()?;
    Some(stored != current)
}

fn execution_result(
    v: &ArchivedVerdict,
    current_cases: &HashMap<String, CaseResolution>,
) -> ExecutionResult {
    ExecutionResult {
        case_id: v.case_id.clone(),
        verdict: v.verdict.clone(),
        evaluation_mode: v.evaluation_mode,
        timestamp: v.timestamp,
        case_digest: v.case_digest.clone(),
        case_changed_since: case_changed_since(v, current_cases),
        evidence: v.evidence.clone(),
    }
}

fn execution_row(
    session: &ArchivedSession,
    current_model_digest: Option<&str>,
    current_cases: &HashMap<String, CaseResolution>,
) -> ExecutionRow {
    // Execution-level mode is derived from origin (§4 P1): External ingests
    // are external; everything else is a trajectory execution.
    let evaluation_mode = if session.origin == SessionOrigin::External {
        EvaluationMode::External
    } else {
        EvaluationMode::Trajectory
    };
    // record_external writes one tool + one declared_digest for the whole
    // batch, so any external verdict is representative of the execution.
    let external = session.verdicts.iter().find_map(|v| v.external.as_ref()).map(|ext| {
        ExecutionExternal {
            tool: ext.tool.clone(),
            declared_digest: ext.declared_digest.clone(),
            run_ref: ext.run_ref.clone(),
            matches_current_model: current_model_digest.map(|d| d == ext.declared_digest),
        }
    });
    ExecutionRow {
        execution_id: session.id.clone(),
        origin: session.origin,
        label: session.label.clone(),
        timestamp: session.created_at,
        evaluation_mode,
        provenance: session.provenance.clone(),
        external,
        results: session
            .verdicts
            .iter()
            .map(|v| execution_result(v, current_cases))
            .collect(),
        counts: VerdictCounts::from_verdicts(&session.verdicts),
    }
}

/// Build the newest-first execution projection (P1).
///
/// - `workspace_root` — B6 provenance scope (see [`scoped_verdict_sessions`]).
/// - `current_model_digest` — current whole-model `content_digest`, for the
///   external `matches_current_model` label. `None` = nothing loaded.
/// - `current_cases` — case NAME → current resolution (element id + subtree
///   digest), for `case_changed_since` and `case_element_id`.
/// - `case_name` — if `Some`, keep only executions that touched that case.
pub fn build_executions(
    archive: &dyn SessionArchive,
    workspace_root: Option<&str>,
    current_model_digest: Option<&str>,
    current_cases: &HashMap<String, CaseResolution>,
    case_name: Option<&str>,
) -> Result<ExecutionsResult, ServiceError> {
    let sessions = scoped_verdict_sessions(archive, workspace_root);
    let mut executions = Vec::with_capacity(sessions.len());
    for session in &sessions {
        if let Some(name) = case_name {
            if !session.verdicts.iter().any(|v| v.case_id == name) {
                continue;
            }
        }
        executions.push(execution_row(session, current_model_digest, current_cases));
    }
    Ok(ExecutionsResult { executions })
}

/// Reduce execution rows to the latest verdict per (case, evaluation_mode)
/// — the execution-side latest-status projection (P2). Composes the P1
/// internals directly: one pass over the rows [`build_executions`] built,
/// no second archive walk.
///
/// "Latest" is by verdict `timestamp` (not row order), since one session
/// record can accumulate verdicts recorded at different times.
pub fn build_latest_status(
    rows: &[ExecutionRow],
    current_cases: &HashMap<String, CaseResolution>,
) -> LatestStatusResult {
    let mut acc: HashMap<String, LatestByMode> = HashMap::new();
    // First-seen (newest-execution) order for deterministic output.
    let mut order: Vec<String> = Vec::new();

    for row in rows {
        for result in &row.results {
            // Static verdicts are never archived (B10 §3.3); an execution-
            // side projection has none. Ignore defensively rather than
            // misfile one into a mode slot.
            if result.evaluation_mode == EvaluationMode::Static {
                continue;
            }
            let entry = acc.entry(result.case_id.clone()).or_insert_with(|| {
                order.push(result.case_id.clone());
                LatestByMode::default()
            });
            match result.evaluation_mode {
                EvaluationMode::Trajectory => {
                    let newer = entry
                        .trajectory
                        .as_ref()
                        .is_none_or(|t| result.timestamp > t.timestamp);
                    if newer {
                        entry.trajectory = Some(LatestTrajectory {
                            verdict: result.verdict.clone(),
                            execution_id: row.execution_id.clone(),
                            timestamp: result.timestamp,
                            case_changed_since: result.case_changed_since,
                            model_digest: row
                                .provenance
                                .as_ref()
                                .map(|p| p.model_digest.clone()),
                            evidence: result.evidence.clone(),
                        });
                    }
                }
                EvaluationMode::External => {
                    let newer = entry
                        .external
                        .as_ref()
                        .is_none_or(|e| result.timestamp > e.timestamp);
                    if newer {
                        entry.external = Some(LatestExternal {
                            verdict: result.verdict.clone(),
                            execution_id: row.execution_id.clone(),
                            timestamp: result.timestamp,
                            tool: row.external.as_ref().map(|x| x.tool.clone()),
                            matches_current_model: row
                                .external
                                .as_ref()
                                .and_then(|x| x.matches_current_model),
                            case_changed_since: result.case_changed_since,
                        });
                    }
                }
                EvaluationMode::Static => unreachable!("static filtered above"),
            }
        }
    }

    let cases = order
        .into_iter()
        .map(|name| {
            let latest = acc.remove(&name).unwrap_or_default();
            let case_element_id = current_cases.get(&name).map(|c| c.element_id.clone());
            CaseLatest {
                case_id: name,
                case_element_id,
                latest,
            }
        })
        .collect();
    LatestStatusResult { cases }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_store::{
        ArchivedEvidence, ExternalEvidence, InMemorySessionArchive, SessionArchive,
    };

    fn cases(pairs: &[(&str, &str, Option<&str>)]) -> HashMap<String, CaseResolution> {
        pairs
            .iter()
            .map(|(name, elem, digest)| {
                (
                    (*name).to_owned(),
                    CaseResolution {
                        element_id: (*elem).to_owned(),
                        subtree_digest: digest.map(str::to_owned),
                    },
                )
            })
            .collect()
    }

    fn trajectory_session(
        id: &str,
        root: &str,
        created_at: i64,
        verdicts: Vec<ArchivedVerdict>,
    ) -> ArchivedSession {
        ArchivedSession {
            id: id.to_owned(),
            label: None,
            origin: SessionOrigin::Verify,
            workspace_uri: "__workspace__".to_owned(),
            created_at,
            ended_at: created_at + 1,
            ticks: 5,
            overrides: Vec::new(),
            verdicts,
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: Some(SessionProvenance {
                model_digest: "model-digest".to_owned(),
                git: None,
                workspace_root: Some(root.to_owned()),
                file_manifest: Vec::new(),
            }),
        }
    }

    fn external_session(
        id: &str,
        root: &str,
        created_at: i64,
        verdicts: Vec<ArchivedVerdict>,
    ) -> ArchivedSession {
        let mut s = trajectory_session(id, root, created_at, verdicts);
        s.origin = SessionOrigin::External;
        s
    }

    fn traj_verdict(case: &str, v: &str, ts: i64, case_digest: Option<&str>) -> ArchivedVerdict {
        ArchivedVerdict::trajectory(
            case,
            v,
            ts,
            Some(ArchivedEvidence {
                time_ms: None,
                session_id: "s".to_owned(),
                tick: 1,
                element_id: Some(format!("{case}-el")),
            }),
            case_digest.map(str::to_owned),
        )
    }

    fn ext_verdict(
        case: &str,
        v: &str,
        ts: i64,
        tool: &str,
        declared: &str,
        case_digest: Option<&str>,
    ) -> ArchivedVerdict {
        ArchivedVerdict::external(
            case,
            v,
            ts,
            ExternalEvidence {
                tool: tool.to_owned(),
                declared_digest: declared.to_owned(),
                run_ref: None,
                artifacts: Vec::new(),
                element_id: Some(format!("{case}-el")),
            },
            case_digest.map(str::to_owned),
        )
    }

    /// A verdict-less session (pure simulation run) is NOT an execution.
    #[test]
    fn verdict_less_sessions_are_excluded() {
        let archive = InMemorySessionArchive::new();
        archive
            .record(trajectory_session("sim-only", "/w", 100, Vec::new()))
            .unwrap();
        archive
            .record(trajectory_session(
                "verified",
                "/w",
                200,
                vec![traj_verdict("CaseA", "pass", 200, None)],
            ))
            .unwrap();
        let result =
            build_executions(&archive, Some("/w"), None, &HashMap::new(), None).unwrap();
        assert_eq!(result.executions.len(), 1, "only the verdict-carrying session is an execution");
        assert_eq!(result.executions[0].execution_id, "verified");
    }

    /// Executions are newest-first; origin drives execution-level mode; the
    /// external block computes whole-model staleness.
    #[test]
    fn executions_ordering_mode_and_external_block() {
        let archive = InMemorySessionArchive::new();
        archive
            .record(trajectory_session(
                "old",
                "/w",
                100,
                vec![traj_verdict("CaseA", "pass", 100, None)],
            ))
            .unwrap();
        archive
            .record(external_session(
                "new",
                "/w",
                300,
                vec![ext_verdict("CaseA", "fail", 300, "pytest", "stale-digest", None)],
            ))
            .unwrap();
        let result =
            build_executions(&archive, Some("/w"), Some("model-digest"), &HashMap::new(), None)
                .unwrap();
        assert_eq!(result.executions.len(), 2);
        // Newest first.
        assert_eq!(result.executions[0].execution_id, "new");
        assert_eq!(result.executions[0].evaluation_mode, EvaluationMode::External);
        let ext = result.executions[0].external.as_ref().expect("external block");
        assert_eq!(ext.tool, "pytest");
        assert_eq!(ext.matches_current_model, Some(false), "declared != current whole-model digest");
        assert_eq!(result.executions[1].execution_id, "old");
        assert_eq!(result.executions[1].evaluation_mode, EvaluationMode::Trajectory);
        assert!(result.executions[1].external.is_none());
        assert_eq!(result.executions[0].counts.fail, 1);
    }

    /// `case_changed_since`: stored digest vs current subtree digest, with
    /// `null` for both unresolvable directions (no stored digest; case no
    /// longer resolves).
    #[test]
    fn case_changed_since_semantics() {
        let archive = InMemorySessionArchive::new();
        archive
            .record(trajectory_session(
                "e1",
                "/w",
                100,
                vec![
                    // stored digest == current → unchanged
                    traj_verdict("Same", "pass", 100, Some("dig-1")),
                    // stored digest != current → changed
                    traj_verdict("Changed", "pass", 100, Some("old-dig")),
                    // no stored digest → null
                    traj_verdict("NoStored", "pass", 100, None),
                    // case no longer resolves → null
                    traj_verdict("Gone", "pass", 100, Some("dig-x")),
                ],
            ))
            .unwrap();
        let current = cases(&[
            ("Same", "same-el", Some("dig-1")),
            ("Changed", "chg-el", Some("dig-2")),
            ("NoStored", "ns-el", Some("dig-3")),
            // "Gone" absent from current resolution
        ]);
        let result = build_executions(&archive, Some("/w"), None, &current, None).unwrap();
        let results = &result.executions[0].results;
        let by = |name: &str| results.iter().find(|r| r.case_id == name).unwrap();
        assert_eq!(by("Same").case_changed_since, Some(false));
        assert_eq!(by("Changed").case_changed_since, Some(true));
        assert_eq!(by("NoStored").case_changed_since, None);
        assert_eq!(by("Gone").case_changed_since, None);
    }

    /// The `case_name` filter keeps only executions touching that case.
    #[test]
    fn case_name_filter_restricts_executions() {
        let archive = InMemorySessionArchive::new();
        archive
            .record(trajectory_session(
                "e-a",
                "/w",
                100,
                vec![traj_verdict("CaseA", "pass", 100, None)],
            ))
            .unwrap();
        archive
            .record(trajectory_session(
                "e-b",
                "/w",
                200,
                vec![traj_verdict("CaseB", "pass", 200, None)],
            ))
            .unwrap();
        let result =
            build_executions(&archive, Some("/w"), None, &HashMap::new(), Some("CaseB")).unwrap();
        assert_eq!(result.executions.len(), 1);
        assert_eq!(result.executions[0].execution_id, "e-b");
    }

    /// Latest-status is context-qualified per mode and picks newest per
    /// (case, mode) by verdict timestamp — never one flat consolidated
    /// field.
    #[test]
    fn latest_status_is_context_qualified_per_mode() {
        let archive = InMemorySessionArchive::new();
        // Two trajectory executions of CaseA — the newer verdict wins.
        archive
            .record(trajectory_session(
                "traj-old",
                "/w",
                100,
                vec![traj_verdict("CaseA", "fail", 100, Some("d-old"))],
            ))
            .unwrap();
        archive
            .record(trajectory_session(
                "traj-new",
                "/w",
                200,
                vec![traj_verdict("CaseA", "pass", 200, Some("d-new"))],
            ))
            .unwrap();
        // One external execution of CaseA — a separate mode slot.
        archive
            .record(external_session(
                "ext",
                "/w",
                150,
                vec![ext_verdict("CaseA", "inconclusive", 150, "hil", "model-digest", Some("d-new"))],
            ))
            .unwrap();

        let current = cases(&[("CaseA", "CaseA-el", Some("d-new"))]);
        let rows = build_executions(&archive, Some("/w"), Some("model-digest"), &current, None)
            .unwrap()
            .executions;
        let status = build_latest_status(&rows, &current);
        assert_eq!(status.cases.len(), 1);
        let c = &status.cases[0];
        assert_eq!(c.case_id, "CaseA");
        assert_eq!(c.case_element_id.as_deref(), Some("CaseA-el"));

        let traj = c.latest.trajectory.as_ref().expect("trajectory latest");
        assert_eq!(traj.verdict, "pass", "newest trajectory verdict wins");
        assert_eq!(traj.execution_id, "traj-new");
        assert_eq!(traj.case_changed_since, Some(false));
        assert_eq!(traj.model_digest.as_deref(), Some("model-digest"));

        let ext = c.latest.external.as_ref().expect("external latest");
        assert_eq!(ext.verdict, "inconclusive");
        assert_eq!(ext.execution_id, "ext");
        assert_eq!(ext.tool.as_deref(), Some("hil"));
        assert_eq!(ext.matches_current_model, Some(true));
    }

    /// The root filter excludes other roots and unattributable sessions,
    /// exactly like `verify_timeline`.
    #[test]
    fn root_filter_matches_timeline_discipline() {
        let archive = InMemorySessionArchive::new();
        archive
            .record(trajectory_session(
                "here",
                "/w",
                100,
                vec![traj_verdict("A", "pass", 100, None)],
            ))
            .unwrap();
        archive
            .record(trajectory_session(
                "elsewhere",
                "/other",
                100,
                vec![traj_verdict("B", "pass", 100, None)],
            ))
            .unwrap();
        // Unattributable: no provenance at all.
        let mut orphan = trajectory_session("orphan", "/w", 100, vec![traj_verdict("C", "pass", 100, None)]);
        orphan.provenance = None;
        archive.record(orphan).unwrap();

        let scoped = build_executions(&archive, Some("/w"), None, &HashMap::new(), None).unwrap();
        let ids: Vec<_> = scoped.executions.iter().map(|e| e.execution_id.as_str()).collect();
        assert_eq!(ids, vec!["here"], "only the matching-root, attributed execution");
    }
}
