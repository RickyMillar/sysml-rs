//! Verdict timeline — reconstruct when verdicts flipped across historical
//! verification runs.
//!
//! This module backs the `sysml.verify.timeline` service command (R3.4 of
//! the UX extensibility plan). It walks the [`SessionArchive`] populated by
//! `sessions.stop` and emits a tick-aligned, per-case stream of
//! [`VerdictTimelineEntry`] records the UI can draw as a horizontal timeline
//! with one lane per case.
//!
//! ## Response shape (stable)
//!
//! ```json
//! {
//!   "entries": [
//!     {
//!       "session_id": "session-<uuid>",
//!       "timestamp": 1713484800123,
//!       "case_id": "MyCase",
//!       "verdict": "pass" | "fail" | "inconclusive" | "error",
//!       "evidence": { "session_id": "...", "tick": 42, "element_id": "..." } | null
//!     }
//!   ]
//! }
//! ```
//!
//! `timestamp` is a Unix millisecond timestamp. Entries are returned in
//! ascending time order, oldest first. The UI groups them by `case_id` to
//! render one lane per case.
//!
//! ## Archive wiring (R4.1)
//!
//! As of R4.1, `sysml.sessions.stop` records completed sessions into the
//! [`SessionArchive`]. `build_timeline` queries that archive with an
//! optional workspace-root (session provenance, B6) + since filter,
//! flattens the stored [`ArchivedVerdict`]s across all matching sessions
//! into timeline entries, and returns them in ascending-timestamp order.
//!
//! Sessions that were never populated with verdicts (e.g. simulate-only runs)
//! are silently skipped — they simply contribute no entries. This is the
//! intended behaviour: the timeline is a reconstruction of verify verdicts,
//! not a general session log.

use serde::{Serialize, Serializer};
use sysml_runtime::cases::{EvidenceRef, VerdictKind};
use sysml_store::{
    ArchiveFilter, ArchivedEvidence, ArchivedVerdict, EvaluationMode, ExternalEvidence,
    SessionArchive,
};

use crate::error::ServiceError;

/// Serialize `VerdictKind` as a lowercased string
/// ("pass" / "fail" / "inconclusive" / "error") so the timeline entries
/// speak the same wire language as `VerifyResult.verdict` (which uses
/// `Display`) and the UI's `VerdictBadge`, which expects the lowercase
/// union `'pass' | 'fail' | 'inconclusive' | 'error'`.
fn serialize_verdict_lowercased<S: Serializer>(
    v: &VerdictKind,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.to_string())
}

/// One point on the verdict timeline: "at time T, case C settled into
/// verdict V, from session S".
///
/// The UI uses these to render one lane per `case_id` with a
/// [`crate::types`]-sized marker at each `timestamp`. `evidence` deep-links
/// back to the exact tick and element that produced the verdict, so
/// Agent Q's drill-from-timeline-to-run (R3.5) can consume it.
#[derive(Debug, Clone, Serialize)]
pub struct VerdictTimelineEntry {
    /// Opaque runtime session id that produced this verdict. Same format
    /// as `SessionSummary.id`.
    pub session_id: String,
    /// Unix millisecond timestamp when the verdict was recorded. Entries
    /// returned by `build_timeline()` are sorted ascending by this field.
    pub timestamp: i64,
    /// Stable identifier of the verification case (e.g. "BrakeTorqueCase").
    /// Used as the lane key in the UI timeline.
    pub case_id: String,
    /// The 4-valued verdict. Serialized as lowercase string
    /// ("pass" / "fail" / "inconclusive" / "error") matching the existing
    /// `VerifyResult.verdict` wire format and the UI's `VerdictBadge`.
    #[serde(serialize_with = "serialize_verdict_lowercased")]
    pub verdict: VerdictKind,
    /// Deep-link back to the tick + element that caused this verdict, so
    /// the UI can offer "jump to evidence" (R3.5). Optional because older
    /// archive records may predate the evidence-ref contract. Trajectory
    /// entries only — external entries carry `external` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceRef>,
    /// How the verdict was produced (B10 layer 2) — lowercase string,
    /// rendered always per §2.1a(d).
    pub evaluation_mode: EvaluationMode,
    /// Evidence behind an externally ingested verdict (B10 layer 3),
    /// with server-computed staleness against the CURRENT model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalTimelineEvidence>,
}

/// Wire form of [`ExternalEvidence`] on the timeline: the stored payload
/// plus the read-time staleness comparison.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalTimelineEvidence {
    /// Producing tool (e.g. `"pytest-7.4"`).
    pub tool: String,
    /// The digest the client declared the results were produced against.
    pub declared_digest: String,
    /// Opaque run reference in the tool's namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<String>,
    /// Artifact pointers (log/report URIs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// Resolved `ElementId` of the verification case at ingestion time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// `declared_digest == <current workspace digest>`, computed at read
    /// time. `false` = the staleness signal ("produced against an older
    /// model" — recorded honestly, never rejected at ingestion). `None`
    /// ONLY when no current digest resolves (nothing loaded) — never
    /// fabricated.
    pub matches_current_model: Option<bool>,
}

/// Response of `sysml.verify.timeline`. Wraps the entry list in an object
/// so future fields (pagination cursor, truncation flag, archive schema
/// version) can be added without breaking clients.
#[derive(Debug, Clone, Serialize)]
pub struct VerdictTimelineResult {
    /// Verdict flips across the requested scope, ascending by timestamp.
    pub entries: Vec<VerdictTimelineEntry>,
}

/// Map the archive's string-valued verdict onto the typed [`VerdictKind`].
/// Silently drops any unknown verdict string by returning `None` so a
/// malformed archive entry cannot poison the timeline.
fn parse_verdict(s: &str) -> Option<VerdictKind> {
    match s {
        "pass" => Some(VerdictKind::Pass),
        "fail" => Some(VerdictKind::Fail),
        "inconclusive" => Some(VerdictKind::Inconclusive),
        "error" => Some(VerdictKind::Error),
        _ => None,
    }
}

fn evidence_from_archived(ev: &ArchivedEvidence) -> EvidenceRef {
    EvidenceRef {
        session_id: ev.session_id.clone(),
        tick: ev.tick,
        element_id: ev.element_id.clone(),
    }
}

fn external_to_wire(
    ext: &ExternalEvidence,
    current_digest: Option<&str>,
) -> ExternalTimelineEvidence {
    ExternalTimelineEvidence {
        tool: ext.tool.clone(),
        declared_digest: ext.declared_digest.clone(),
        run_ref: ext.run_ref.clone(),
        artifacts: ext.artifacts.clone(),
        element_id: ext.element_id.clone(),
        matches_current_model: current_digest.map(|d| d == ext.declared_digest),
    }
}

fn archived_verdict_to_entry(
    session_id: &str,
    v: &ArchivedVerdict,
    current_digest: Option<&str>,
) -> Option<VerdictTimelineEntry> {
    let verdict = parse_verdict(&v.verdict)?;
    Some(VerdictTimelineEntry {
        session_id: session_id.to_owned(),
        timestamp: v.timestamp,
        case_id: v.case_id.clone(),
        verdict,
        evidence: v.evidence.as_ref().map(evidence_from_archived),
        evaluation_mode: v.evaluation_mode,
        external: v.external.as_ref().map(|e| external_to_wire(e, current_digest)),
    })
}

/// Build a verdict timeline for a workspace from the supplied archive.
///
/// Filters:
/// - `workspace_root` — if `Some`, only sessions whose recorded
///   [`sysml_store::SessionProvenance::workspace_root`] matches EXACTLY are
///   included. That field is the real workspace identity (B6, 2026-07-17):
///   the absolute root the session executed against, captured at mint by
///   the same root-resolution rule the caller uses at query time, so the
///   keys compare byte-for-byte. Sessions with no recorded root (pre-B6
///   records, or minted while no single root resolved) are EXCLUDED when
///   the filter is active — they are unattributable, and including them
///   would reintroduce the cross-workspace bleed this key exists to
///   prevent. `None` means "the whole archive" — the honest shape when the
///   caller itself has no resolvable root to key on.
///   (This replaced the earlier exact-match filter on the archived
///   run-scope `workspace_uri` — a `sessions.create` argument, never a
///   workspace identity; see W7b in
/// - `case_ids` — if `Some`, only entries whose `case_id` is in this list
///   are returned. `None` means "all cases in this workspace".
/// - `since_timestamp` — if `Some`, only entries with
///   `timestamp >= since_timestamp` are returned. `None` means "all time".
/// - `current_digest` — the CURRENT workspace `content_digest`, used to
///   compute `matches_current_model` on external entries (B10 staleness
///   label). `None` (nothing loaded) leaves the comparison honestly
///   unanswered, never fabricated.
///
/// Archive entries whose `verdict` string does not match one of
/// `pass|fail|inconclusive|error` are silently dropped.
pub fn build_timeline(
    archive: &dyn SessionArchive,
    workspace_root: Option<&str>,
    case_ids: Option<&[String]>,
    since_timestamp: Option<i64>,
    current_digest: Option<&str>,
) -> Result<VerdictTimelineResult, ServiceError> {
    // Pull every session summary; the provenance-root filter applies on
    // the full record below (summaries don't carry provenance). We
    // intentionally do NOT push `since_timestamp` into the session filter
    // because individual verdicts may be more recent than the session's
    // `created_at` (a long-running session can emit verdicts hours after
    // it started).
    let summaries = archive.list(ArchiveFilter::default());

    let mut entries: Vec<VerdictTimelineEntry> = Vec::new();
    for summary in summaries {
        // Skip cheap when we know there are no verdicts on this session.
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
        for v in &full.verdicts {
            // Post-filter `since` on individual verdicts too: a session's
            // `created_at` may be older than `since_timestamp` but contain
            // verdicts recorded after that lower bound.
            if let Some(since) = since_timestamp {
                if v.timestamp < since {
                    continue;
                }
            }
            if let Some(ids) = case_ids {
                if !ids.iter().any(|id| id == &v.case_id) {
                    continue;
                }
            }
            if let Some(entry) = archived_verdict_to_entry(&full.id, v, current_digest) {
                entries.push(entry);
            }
        }
    }

    // Ascending by timestamp so the UI can stream entries left-to-right.
    entries.sort_by_key(|e| e.timestamp);
    Ok(VerdictTimelineResult { entries })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_store::{
        ArchivedEvidence, ArchivedSession, ArchivedVerdict, InMemorySessionArchive,
        SessionArchive, SessionOrigin,
    };

    fn empty_archive() -> InMemorySessionArchive {
        InMemorySessionArchive::new()
    }

    fn session_with_verdicts(
        id: &str,
        workspace: &str,
        created_at: i64,
        verdicts: Vec<ArchivedVerdict>,
    ) -> ArchivedSession {
        ArchivedSession {
            id: id.to_owned(),
            label: None,
            origin: SessionOrigin::Verify,
            workspace_uri: workspace.to_owned(),
            created_at,
            ended_at: created_at + 1_000,
            ticks: 10,
            overrides: Vec::new(),
            verdicts,
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: None,
        }
    }

    /// Session carrying B6 provenance with the given workspace root — the
    /// identity `build_timeline`'s root filter keys on.
    fn session_with_root(
        id: &str,
        root: &str,
        created_at: i64,
        verdicts: Vec<ArchivedVerdict>,
    ) -> ArchivedSession {
        let mut s = session_with_verdicts(id, "__workspace__", created_at, verdicts);
        s.provenance = Some(sysml_store::SessionProvenance {
            model_digest: "digest-x".to_owned(),
            git: None,
            workspace_root: Some(root.to_owned()),
            file_manifest: Vec::new(),
        });
        s
    }

    fn verdict(case: &str, v: &str, timestamp: i64, tick: Option<u64>) -> ArchivedVerdict {
        ArchivedVerdict::trajectory(
            case,
            v,
            timestamp,
            tick.map(|t| ArchivedEvidence {
                time_ms: None,
                session_id: "s-x".to_owned(),
                tick: t,
                element_id: Some("El".to_owned()),
            }),
            None,
        )
    }

    #[test]
    fn empty_archive_yields_empty_entries() {
        let archive = empty_archive();
        let result = build_timeline(&archive, Some("/any-root"), None, None, None).unwrap();
        assert!(result.entries.is_empty());
    }

    #[test]
    fn single_session_multiple_verdicts_ascending() {
        let archive = empty_archive();
        let session = session_with_root(
            "s-1",
            "/w",
            100,
            vec![
                verdict("CaseA", "pass", 200, Some(1)),
                verdict("CaseB", "fail", 150, Some(2)),
            ],
        );
        archive.record(session).unwrap();
        let result = build_timeline(&archive, Some("/w"), None, None, None).unwrap();
        assert_eq!(result.entries.len(), 2);
        // Ascending by timestamp.
        assert_eq!(result.entries[0].timestamp, 150);
        assert_eq!(result.entries[0].case_id, "CaseB");
        assert_eq!(result.entries[0].verdict, VerdictKind::Fail);
        assert_eq!(result.entries[1].timestamp, 200);
    }

    /// The root filter keys on session provenance (B6): only sessions whose
    /// recorded `workspace_root` matches are included. Sessions from another
    /// root AND sessions with no recorded root (unattributable) are
    /// excluded — including the latter would reintroduce the
    /// cross-workspace bleed this key exists to prevent.
    #[test]
    fn root_filter_excludes_other_roots_and_unattributable_sessions() {
        let archive = empty_archive();
        archive
            .record(session_with_root(
                "s-a",
                "/workspace/a",
                100,
                vec![verdict("A", "pass", 100, None)],
            ))
            .unwrap();
        archive
            .record(session_with_root(
                "s-b",
                "/workspace/b",
                100,
                vec![verdict("B", "pass", 100, None)],
            ))
            .unwrap();
        archive
            .record(session_with_verdicts(
                "s-no-prov",
                "__workspace__",
                100,
                vec![verdict("C", "pass", 100, None)],
            ))
            .unwrap();
        let result = build_timeline(&archive, Some("/workspace/a"), None, None, None).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].case_id, "A");
    }

    /// `workspace_root: None` spans the whole archive — the honest shape
    /// when the caller has no resolvable root to key on (nothing loaded /
    /// multiple project roots). Run-scope `workspace_uri` values and
    /// missing provenance are both irrelevant here. See the
    /// `build_timeline` doc.
    #[test]
    fn no_root_filter_spans_archive() {
        let archive = empty_archive();
        archive
            .record(session_with_root(
                "s-rooted",
                "/w",
                100,
                vec![verdict("A", "pass", 100, None)],
            ))
            .unwrap();
        archive
            .record(session_with_verdicts(
                "s-file",
                "file:///w/target.sysml",
                100,
                vec![verdict("B", "fail", 200, None)],
            ))
            .unwrap();
        let result = build_timeline(&archive, None, None, None, None).unwrap();
        let cases: Vec<_> = result.entries.iter().map(|e| e.case_id.as_str()).collect();
        assert_eq!(cases, vec!["A", "B"], "rooted and unattributed must both appear");
    }

    #[test]
    fn case_ids_filter_restricts_to_listed_cases() {
        let archive = empty_archive();
        archive
            .record(session_with_verdicts(
                "s-1",
                "file:///w",
                100,
                vec![
                    verdict("CaseA", "pass", 100, None),
                    verdict("CaseB", "fail", 101, None),
                    verdict("CaseC", "error", 102, None),
                ],
            ))
            .unwrap();
        let wanted = vec!["CaseA".to_owned(), "CaseC".to_owned()];
        let result = build_timeline(&archive, None, Some(&wanted), None, None).unwrap();
        assert_eq!(result.entries.len(), 2);
        let cases: Vec<_> = result.entries.iter().map(|e| e.case_id.as_str()).collect();
        assert_eq!(cases, vec!["CaseA", "CaseC"]);
    }

    #[test]
    fn since_filter_drops_older_verdicts() {
        let archive = empty_archive();
        archive
            .record(session_with_verdicts(
                "s-1",
                "file:///w",
                100,
                vec![
                    verdict("C", "pass", 100, None),
                    verdict("C", "fail", 500, None),
                ],
            ))
            .unwrap();
        let result = build_timeline(&archive, None, None, Some(300), None).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].timestamp, 500);
    }

    #[test]
    fn unknown_verdict_string_is_dropped() {
        let archive = empty_archive();
        archive
            .record(session_with_verdicts(
                "s-1",
                "file:///w",
                100,
                vec![
                    verdict("C", "pass", 100, None),
                    verdict("C", "unknown_verdict", 200, None),
                ],
            ))
            .unwrap();
        let result = build_timeline(&archive, None, None, None, None).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].verdict, VerdictKind::Pass);
    }

    #[test]
    fn response_serializes_to_expected_json_shape() {
        // Contract lock: the UI builds against this shape.
        let entry = VerdictTimelineEntry {
            session_id: "session-abc".to_owned(),
            timestamp: 1_713_484_800_123,
            case_id: "BrakeTorque".to_owned(),
            verdict: VerdictKind::Fail,
            evidence: Some(EvidenceRef {
                session_id: "session-abc".to_owned(),
                tick: 42,
                element_id: Some("Req::MinTorque".to_owned()),
            }),
            evaluation_mode: EvaluationMode::Trajectory,
            external: None,
        };
        let result = VerdictTimelineResult {
            entries: vec![entry],
        };
        let json = serde_json::to_value(&result).unwrap();

        assert!(json.get("entries").is_some());
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["session_id"], "session-abc");
        assert_eq!(entries[0]["timestamp"], 1_713_484_800_123_i64);
        assert_eq!(entries[0]["case_id"], "BrakeTorque");
        assert_eq!(entries[0]["verdict"], "fail");
        assert_eq!(entries[0]["evidence"]["tick"], 42);
        assert_eq!(entries[0]["evidence"]["element_id"], "Req::MinTorque");
        assert_eq!(entries[0]["evaluation_mode"], "trajectory");
        assert!(entries[0].get("external").is_none(), "no external block on trajectory entries");
    }

    #[test]
    fn evidence_is_omitted_when_none() {
        let entry = VerdictTimelineEntry {
            session_id: "s1".to_owned(),
            timestamp: 0,
            case_id: "C1".to_owned(),
            verdict: VerdictKind::Pass,
            evidence: None,
            evaluation_mode: EvaluationMode::Trajectory,
            external: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert!(
            json.get("evidence").is_none(),
            "evidence must be skipped when None so UI tooltips can test `?.`"
        );
    }

    /// JSON-dispatch contract lock — the command must be reachable via
    /// `execute_command("sysml.verify.timeline", ...)` with the
    /// documented parameter shape, and must return the documented
    /// response shape. This mirrors how the REST/MCP transports reach
    /// the command in production.
    #[test]
    fn execute_command_roundtrip_returns_entries_from_archive() {
        use crate::{execute_command, SysmlService};
        use std::sync::Arc;

        let archive = Arc::new(InMemorySessionArchive::new());
        archive
            .record(session_with_verdicts(
                "session-xyz",
                "file:///workspace",
                100,
                vec![verdict("CaseA", "pass", 200, Some(3))],
            ))
            .unwrap();
        let service = SysmlService::with_archive(archive);

        let params = serde_json::json!({
            "case_ids": null,
            "since_timestamp": null,
        });
        let result = execute_command(&service, "sysml.verify.timeline", params)
            .expect("sysml.verify.timeline must be registered");
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["case_id"], "CaseA");
        assert_eq!(entries[0]["verdict"], "pass");

        // Filter variant — exercises the Option<&[String]> / Option<i64>
        // wire mapping end-to-end.
        let filtered_params = serde_json::json!({
            "case_ids": ["CaseZ"],
            "since_timestamp": null,
        });
        let filtered = execute_command(
            &service,
            "sysml.verify.timeline",
            filtered_params,
        )
        .expect("filtered dispatch must succeed");
        assert!(filtered["entries"].as_array().unwrap().is_empty());
    }
}
