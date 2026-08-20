/**
 * Report types for the Verify workflow's HTML + Markdown exporters.
 *
 * Locked contract, consumed by:
 *   - `generateHtmlReport.ts`  — standalone HTML string
 *   - `generateMarkdownReport.ts` — GFM string
 *   - `DownloadReportButton.tsx` — React trigger (pure-function backed)
 *   - Node-side tooling (server exports, CI artifacts) via the same
 *     generators (they are pure functions, no DOM / React deps)
 *
 * See the Round 3 task spec R3.7 in
 */

import type { Verdict, VerdictKind } from '@/engine/types';

/**
 * Pre-computed verdict rollup, mirrors the backend's
 * `sysml_service::VerifySummary` shape (which wraps
 * `sysml_runtime::aggregates::VerdictRollup` with the worst-wins
 * `overall` pre-stringified). Clients never re-aggregate from a
 * `Verdict[]` — the backend owns the projection and the generators
 * read these counts + `overall` directly.
 */
export interface VerdictRollup {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
  overall: VerdictKind;
}

/**
 * One entry in the run-environment block displayed at the top of the
 * report. All fields are optional — the header renders `—` for missing
 * values rather than collapsing the row.
 */
export interface ReportEnvironment {
  /** e.g. "sysml-runtime 0.3.1 (rev abc123)" */
  backendVersion: string;
  /** Simulation clock dt (seconds) if fixed-step. */
  simDt?: number;
  /** Seeds array when the workflow is Monte Carlo / stochastic. */
  seeds?: number[];
  /** Suite label — e.g. "Espresso cell acceptance suite". */
  suiteName?: string;
}

/**
 * Minimal case→requirement map the HTML expand-per-case view needs.
 * The generator groups verdicts by case using `requirementIds`; verdicts
 * whose `id` is not in any case's list fall under a synthetic
 * "Ungrouped" case.
 */
export interface ReportCase {
  /** Stable id (element QualifiedName or backend case key). */
  id: string;
  /** Human label for the case row (e.g. "GroupHead verdict matrix — B6 @ 30A"). */
  label: string;
  /** Ordered list of requirement ids this case aggregates. */
  requirementIds: string[];
  /**
   * Pre-computed rollup + overall verdict for this case's
   * requirements. Sourced from the backend's `VerifySummary`; the
   * generator never re-aggregates from the filtered verdict bucket.
   */
  summary: VerdictRollup;
}

/**
 * Model-revision pointer (ninebar Phase 4 §6.2). The backend billet —
 * capture a whole-graph content hash + per-file manifest at session mint,
 * persist on `ArchivedSession`/`BatchSession` — has landed in
 * `SysmlService::capture_session_provenance` (`SessionProvenance.file_manifest`).
 * Fields stay optional and render as `—` for the honest absent cases: a
 * static (session-free) verify, or a pre-§6.2 archive with no manifest.
 */
export interface ReportModelRevision {
  /** Content hash of the merged workspace graph (backend `cache_key`-style). */
  manifestHash?: string;
  /** Absolute workspace root the run executed against. */
  workspaceRoot?: string;
  /** Per-file manifest — URI + content hash. */
  files?: Array<{ uri: string; hash?: string }>;
}

/**
 * Run provenance — everything a reviewer needs to reconstitute the run
 * (§6.2). Model identity + revision, the executed session/archive, the
 * selected cases + overrides + run config, and the run/stop timestamps.
 * All optional: a static (session-free) verify carries no session id;
 * model revision waits on the backend billet.
 */
export interface ReportProvenance {
  /**
   * Distinct evaluation modes across this run's verdicts (B10 layer 2,
   * §2.1a(d)) — `static` (against current/default values), `trajectory`
   * (against a live run), `external` (ingested provenance). Derived from
   * the verdicts, not authored. Empty when no verdict carried a mode. A
   * downloaded compliance report MUST state this: a static desk check and
   * a trajectory run answer different questions, and external evidence is
   * provenance, never a tool-computed pass.
   */
  evaluationModes?: string[];
  /** Model identity + source revision (backend billet — may be absent). */
  model?: ReportModelRevision;
  /** Live/archived session the verdicts were produced against, if any. */
  sessionId?: string;
  /** Archived-session id, when the run was archived (`sessions.stop`). */
  archiveId?: string;
  /** `SessionOrigin` label (how the session was created). */
  origin?: string;
  /** Verification case names included in this run. */
  selectedCases?: string[];
  /** Applied overrides (name → value) at run time. */
  overrides?: Array<[string, string]>;
  /** Run configuration knobs. */
  runConfig?: { dt?: number; scenario?: string; view?: string };
  /** ISO-8601 run start / stop timestamps. */
  startedAt?: string;
  stoppedAt?: string;
}

/**
 * Full input to the generators. This is the sole entry point; callers
 * gather the data (from the verify workflow store, from a CLI export
 * command, from a server-side batch job) and call the generator.
 */
export interface ReportInput {
  /** Workspace label — used in filename and header. */
  workspaceName: string;
  /** Wall-clock timestamp when the run finished. */
  runTimestamp: Date;
  /** Total run duration in milliseconds. */
  durationMs: number;
  /** Execution environment details. */
  environment: ReportEnvironment;
  /** Flat list of verdicts. Ordering is preserved in the HTML output. */
  verdicts: Verdict[];
  /** Case rows. Order controls the order of case cards in the report. */
  cases: ReportCase[];
  /**
   * Workspace-level rollup over all `verdicts`. Pre-computed by the
   * backend (`VerifySummary`); generators read it verbatim instead
   * of counting on the client.
   */
  summary: VerdictRollup;
  /**
   * Run provenance (§6.2). Optional — a report without it still renders;
   * the generators show `—` for missing fields so a partial provenance
   * (e.g. no model revision yet) is honestly visible rather than hidden.
   */
  provenance?: ReportProvenance;
}

/**
 * Generator output — the HTML string plus the conventional filename
 * (`verify-{workspace}-{YYYYMMDD-HHMMSS}.html`). Callers pipe these
 * through `URL.createObjectURL` + a download anchor, or write them
 * directly to disk.
 */
export interface ReportOutput {
  html: string;
  filename: string;
}

