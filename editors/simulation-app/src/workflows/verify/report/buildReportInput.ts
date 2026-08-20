/**
 * buildReportInput — assemble a `ReportInput` from the live Verify run
 * state (ninebar Phase 4).
 *
 * The report generators are pure and backend-agnostic; this is the one
 * place the sim app turns "what just ran" into their input. Provenance
 * (§6.2) is populated with everything the frontend actually knows —
 * session id, selected cases, suite — and leaves the model-revision /
 * archive / timestamps to the backend billet (`crates/**`, out of the
 * Phase-4 fence), which renders as `—` until it lands rather than being
 * silently omitted.
 */

import type { Verdict, VerdictKind, VerifyRunResult } from '@/engine/types';
import type { SessionProvenance } from '@/features/sessions/types';
import { caseNameOf } from '../VerdictMatrix';
import type { ReportCase, ReportInput, ReportProvenance, VerdictRollup } from './types';

/** Worst-wins overall (Error > Fail > Inconclusive > Pass). */
function overallOf(counts: { pass: number; fail: number; inconclusive: number; error: number }): VerdictKind {
  if (counts.error > 0) return 'error';
  if (counts.fail > 0) return 'fail';
  if (counts.inconclusive > 0) return 'inconclusive';
  return 'pass';
}

function rollup(verdicts: Verdict[]): VerdictRollup {
  const counts = { pass: 0, fail: 0, inconclusive: 0, error: 0 };
  for (const v of verdicts) counts[v.verdict] += 1;
  return { ...counts, overall: overallOf(counts) };
}

export interface BuildReportInputArgs {
  /** Workspace label for the header + filename. */
  workspaceName: string;
  /** Runner output (preferred — carries the backend summary + duration). */
  result: VerifyRunResult | null;
  /** Fallback verdict list when no completed result is available yet. */
  verdicts: Verdict[];
  /** Human suite label (e.g. "Verification Cases"). */
  suiteLabel: string;
  /** Case names the user selected for the run. */
  selectedCaseNames: string[];
  /** Live session id, when the run verified against a running session. */
  sessionId: string | null;
  /**
   * The active session's provenance block (B6), when the run verified
   * against a live session whose summary carries one. Fills
   * `ReportProvenance.model`; absent → the report renders `—`.
   */
  sessionProvenance?: SessionProvenance | null;
  /** Wall-clock timestamp for the report (must be passed in — scripts/tests
   *  can't call `new Date()` deterministically). */
  runTimestamp: Date;
  /** Backend version string for the environment block. */
  backendVersion?: string;
}

/**
 * Group verdicts into report cases by case name. Each verdict gets a
 * stable id (falling back to `case#index`) so the generator's
 * requirement-id → case grouping is deterministic rather than dumping
 * everything into the synthetic "Ungrouped" bucket.
 */
function buildCases(verdicts: Verdict[]): { cases: ReportCase[]; verdicts: Verdict[] } {
  const withIds: Verdict[] = verdicts.map((v, i) =>
    v.id ? v : { ...v, id: `${caseNameOf(v)}#${i}` },
  );
  const byCase = new Map<string, Verdict[]>();
  for (const v of withIds) {
    const name = caseNameOf(v);
    const bucket = byCase.get(name);
    if (bucket) bucket.push(v);
    else byCase.set(name, [v]);
  }
  const cases: ReportCase[] = [...byCase.entries()].map(([name, vs]) => ({
    id: name,
    label: name,
    requirementIds: vs.map((v) => v.id!).filter(Boolean),
    summary: rollup(vs),
  }));
  return { cases, verdicts: withIds };
}

export function buildReportInput(args: BuildReportInputArgs): ReportInput {
  const sourceVerdicts = args.result?.verdicts ?? args.verdicts;
  const { cases, verdicts } = buildCases(sourceVerdicts);

  const summary: VerdictRollup = args.result
    ? { ...args.result.summary, overall: overallOf(args.result.summary) }
    : rollup(verdicts);

  // Distinct evaluation modes present across the run's verdicts, in a
  // stable order (computed modes first, external provenance last).
  const modeSet = new Set<string>();
  for (const v of verdicts) {
    const m = v.metadata?.evaluation_mode;
    if (typeof m === 'string' && m.length > 0) modeSet.add(m);
  }
  const evaluationModes = (['static', 'trajectory', 'external'] as const).filter((m) =>
    modeSet.has(m),
  );

  const provenance: ReportProvenance = {
    evaluationModes: evaluationModes.length > 0 ? evaluationModes : undefined,
    // archive id / timestamps: still FE wiring debt (fields render `—`).
    model: args.sessionProvenance
      ? {
          manifestHash: args.sessionProvenance.model_digest,
          workspaceRoot: args.sessionProvenance.workspace_root ?? undefined,
          // Per-file manifest (§6.2) — real data now that the backend
          // captures it; empty/absent stays `undefined` so the report
          // renders `—` rather than an empty file list.
          files: args.sessionProvenance.file_manifest?.length
            ? args.sessionProvenance.file_manifest.map((f) => ({
                uri: f.path,
                hash: f.content_hash,
              }))
            : undefined,
        }
      : undefined,
    sessionId: args.sessionId ?? undefined,
    selectedCases: args.selectedCaseNames.length > 0 ? args.selectedCaseNames : undefined,
  };

  return {
    workspaceName: args.workspaceName,
    runTimestamp: args.runTimestamp,
    durationMs: args.result?.durationMs ?? 0,
    environment: {
      backendVersion: args.backendVersion ?? 'unknown',
      suiteName: args.suiteLabel,
    },
    verdicts,
    cases,
    summary,
    provenance,
  };
}
