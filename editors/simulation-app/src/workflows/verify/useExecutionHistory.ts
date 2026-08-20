/**
 * useExecutionHistory — the execution-model reads behind the History
 * redesign (design turn 3; test-management study P1/P2).
 *
 * Two wires, one concept: an EXECUTION is one recorded performance of
 * cases in a context — a simulation run or an ingested external run —
 * with provenance and per-case results. ("Run" is fine in UI copy; the
 * concept is `execution` everywhere in code — steward ruling P1.)
 *
 *   · `sysml.verify.executions`    — newest-first execution rows, only
 *     verdict-carrying sessions; per-result `case_digest` pinned at mint
 *     and `case_changed_since` (null = unresolvable → render NOTHING).
 *   · `sysml.verify.latest_status` — per case, per mode: the latest
 *     trajectory and the latest external execution. A mode key is ABSENT
 *     when no such execution exists (render nothing). NEVER one flat
 *     consolidated verdict — the modes answer different questions.
 *
 * The FE composes these with the static read it already fetches
 * (`useVerificationCases`) — the static desk check is the third mode and
 * lives on that wire, not here.
 *
 * All timestamps are Unix milliseconds (session-archive parity).
 */

import { useQuery } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import {
  isBareObjectiveRow,
  normalizeCaseVerdict,
  type VerificationCaseRow,
} from './useVerificationCases';

// ── Wire types (mirrors sysml-service/src/executions.rs) ─────────────

export interface ExecutionResultWire {
  case_id: string;
  verdict: string;
  evaluation_mode: string;
  timestamp: number;
  case_digest?: string | null;
  /** Trajectory verdicts only — the run this verdict came out of. */
  evidence?: ArchivedEvidenceWire | null;
  /** null = unresolvable (case renamed/deleted, or pre-digest record) —
   *  render nothing, never a fabricated flag. */
  case_changed_since?: boolean | null;
}

export interface ExecutionExternalWire {
  tool: string;
  declared_digest: string;
  run_ref?: string | null;
  matches_current_model?: boolean | null;
}

export interface ExecutionRowWire {
  execution_id: string;
  origin: string;
  label?: string | null;
  timestamp: number;
  evaluation_mode: string;
  /** Full B6 SessionProvenance as archived (executions.rs serializes the
   *  whole struct). `file_manifest` is the §6.2 per-file capture at mint —
   *  empty/absent on pre-§6.2 records, which render NOTHING for it. */
  provenance?: {
    model_digest?: string | null;
    git?: { commit?: string | null; dirty?: boolean | null } | null;
    workspace_root?: string | null;
    file_manifest?: Array<{ path: string; content_hash: string }> | null;
  } | null;
  external?: ExecutionExternalWire | null;
  results: ExecutionResultWire[];
  counts: { pass: number; fail: number; inconclusive: number; error: number };
}

/** Session id, tick and simulated time a trajectory verdict was evaluated at. */
export interface ArchivedEvidenceWire {
  session_id: string;
  tick: number;
  /**
   * SIMULATED time at that tick, in milliseconds — the model's clock, not wall
   * clock. Absent on records minted before the field; render the tick alone
   * rather than computing `tick × dt`, which is an inference about the run
   * being inspected and simply wrong for a variable-step or resumed session.
   */
  time_ms?: number | null;
  element_id?: string | null;
}

export interface LatestTrajectoryWire {
  verdict: string;
  execution_id: string;
  timestamp: number;
  case_changed_since?: boolean | null;
  model_digest?: string | null;
  /**
   * The run behind the verdict. Three states, and they mean different things:
   *
   *   · an object  — the run is known: session id + tick.
   *   · `null`     — the server looked and this record genuinely has none
   *                  (a verdict minted before B10 evidence capture).
   *   · `undefined`— the server never sent the key, i.e. it is older than
   *                  this projection. Nothing is known about the record
   *                  either way, and saying "predates evidence capture" would
   *                  be asserting something about data we did not receive.
   *
   * The third case is not theoretical: a brand-new run at tick 5001 was
   * reported to a user as legacy data because the server answering had been
   * built before the field existed. The backend now always serializes the
   * key, so `undefined` specifically means a version skew.
   */
  evidence?: ArchivedEvidenceWire | null;
}

export interface LatestExternalWire {
  verdict: string;
  execution_id: string;
  timestamp: number;
  tool?: string | null;
  matches_current_model?: boolean | null;
  case_changed_since?: boolean | null;
}

export interface CaseLatestWire {
  case_id: string;
  case_element_id?: string | null;
  latest: {
    trajectory?: LatestTrajectoryWire | null;
    external?: LatestExternalWire | null;
  };
}

// ── Queries ──────────────────────────────────────────────────────────

export const executionHistoryKeys = {
  executions: (root: string | null, caseName: string | null) =>
    ['verify-executions', root ?? '', caseName ?? ''] as const,
  latest: (root: string | null) => ['verify-latest-status', root ?? ''] as const,
};

/** Newest-first execution rows; optionally filtered to one case. */
export function useExecutions(caseName: string | null = null) {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  return useQuery({
    queryKey: executionHistoryKeys.executions(workspaceRoot, caseName),
    enabled: !!workspaceRoot,
    staleTime: 15_000,
    queryFn: async () => {
      const raw = await httpPost<{ executions?: ExecutionRowWire[] }>('/api/command', {
        command: 'sysml.verify.executions',
        params: caseName ? { case_name: caseName } : {},
      });
      return Array.isArray(raw?.executions) ? raw.executions : [];
    },
  });
}

/** Per-case, per-mode latest execution status. */
export function useLatestStatus() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  return useQuery({
    queryKey: executionHistoryKeys.latest(workspaceRoot),
    enabled: !!workspaceRoot,
    staleTime: 15_000,
    queryFn: async () => {
      const raw = await httpPost<{ cases?: CaseLatestWire[] }>('/api/command', {
        command: 'sysml.verify.latest_status',
        params: {},
      });
      return Array.isArray(raw?.cases) ? raw.cases : [];
    },
  });
}

/** Index the latest-status rows by case id for row composition. */
export function latestByCase(cases: CaseLatestWire[]): Map<string, CaseLatestWire> {
  const map = new Map<string, CaseLatestWire>();
  for (const c of cases) map.set(c.case_id, c);
  return map;
}

// ── Pure helpers (exported for tests) ────────────────────────────────

/** Sentence-position form: `just now` / `5m ago` / `2h ago`. */
export function relativeAgePhrase(timestampMs: number, nowMs: number = Date.now()): string {
  const age = relativeAge(timestampMs, nowMs);
  return age === 'now' ? 'just now' : `${age} ago`;
}

/** Relative age for row density — `5m` / `2h` / `3d`; `now` under a minute. */
export function relativeAge(timestampMs: number, nowMs: number = Date.now()): string {
  const delta = Math.max(0, nowMs - timestampMs);
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 1) return 'now';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

/** The model-structure grouping key: the case's OWNER chain
 *  (`Pkg::Sub` off `Pkg::Sub::Case`). The industry's "suite" is our
 *  containment — no tool suite object exists (steward ruling P5).
 *  Cases without a qualified name group under the model root. */
export function groupKeyOf(row: VerificationCaseRow): string {
  const qn = row.qualified_name;
  if (typeof qn !== 'string' || qn.length === 0) return '';
  const segments = qn.split('::');
  return segments.slice(0, -1).join('::');
}

export const ROOT_GROUP_LABEL = 'model root';

/** One composed latest-status table row. */
export interface LatestStatusRow {
  row: VerificationCaseRow;
  caseName: string;
  elementId: string | null;
  group: string;
  /** Static desk check — null for bare-objective cases (no verdict minted). */
  staticVerdict: 'pass' | 'fail' | 'inconclusive' | 'error' | null;
  trajectory: LatestTrajectoryWire | null;
  external: LatestExternalWire | null;
  failing: boolean;
  stale: boolean;
  changed: boolean;
}

/** Compose the static read with the latest-status read into table rows. */
export function composeLatestRows(
  cases: VerificationCaseRow[],
  latest: Map<string, CaseLatestWire>,
): LatestStatusRow[] {
  return cases.map((row) => {
    const caseName = row.case_name ?? row.case_id ?? '';
    const entry = latest.get(caseName) ?? (row.case_id ? latest.get(row.case_id) : undefined);
    const trajectory = entry?.latest?.trajectory ?? null;
    const external = entry?.latest?.external ?? null;
    const staticVerdict = isBareObjectiveRow(row) ? null : normalizeCaseVerdict(row.verdict);
    const verdicts = [
      staticVerdict,
      trajectory ? normalizeCaseVerdict(trajectory.verdict) : null,
      external ? normalizeCaseVerdict(external.verdict) : null,
    ];
    return {
      row,
      caseName,
      elementId: row.element_id ?? entry?.case_element_id ?? null,
      group: groupKeyOf(row),
      staticVerdict,
      trajectory,
      external,
      failing: verdicts.some((v) => v === 'fail' || v === 'error'),
      stale: external?.matches_current_model === false,
      changed: trajectory?.case_changed_since === true || external?.case_changed_since === true,
    };
  });
}

export interface LatestStatusGroup {
  key: string;
  label: string;
  rows: LatestStatusRow[];
  counts: { pass: number; fail: number; inconclusive: number; error: number };
}

/**
 * One row's standing for rollup purposes — NOT a flat verdict field (the
 * per-mode cells stay the truth); this is only how a row folds into band
 * and facet counts. Any-mode fail/error wins; otherwise the LATEST
 * execution's verdict (that is what "latest status" means — a green run
 * must not roll up as the desk check's inconclusive, which was exactly
 * the felt problem); a case with no executions stands on its static read.
 */
export function rowStanding(
  row: LatestStatusRow,
): 'pass' | 'fail' | 'inconclusive' | 'error' | null {
  if (row.failing) return 'fail';
  const executions = [row.trajectory, row.external].filter(
    (e): e is NonNullable<typeof e> => !!e,
  );
  if (executions.length > 0) {
    executions.sort((a, b) => b.timestamp - a.timestamp);
    return normalizeCaseVerdict(executions[0].verdict);
  }
  return row.staticVerdict;
}

/**
 * Group rows by model structure, model order preserved between groups;
 * failing rows sort FIRST within their group (reconciled design: trouble
 * floats via in-group order + band rollups + facet chips — rows are never
 * duplicated into a pinned strip). Band counts fold the row's worst
 * standing (any-mode fail counts as fail).
 */
export function groupLatestRows(rows: LatestStatusRow[]): LatestStatusGroup[] {
  const groups: LatestStatusGroup[] = [];
  const byKey = new Map<string, LatestStatusGroup>();
  for (const row of rows) {
    let group = byKey.get(row.group);
    if (!group) {
      group = {
        key: row.group,
        label: row.group.length > 0 ? row.group : ROOT_GROUP_LABEL,
        rows: [],
        counts: { pass: 0, fail: 0, inconclusive: 0, error: 0 },
      };
      byKey.set(row.group, group);
      groups.push(group);
    }
    group.rows.push(row);
    const standing = rowStanding(row);
    if (standing) group.counts[standing] += 1;
  }
  for (const group of groups) {
    group.rows.sort((a, b) => Number(b.failing) - Number(a.failing));
  }
  return groups;
}

/** The last `limit` verdicts of any recorded mode for one case, newest
 *  first — the `recent` trend slot (verdict colours are legal: these ARE
 *  verdicts). Derived from the executions read; flaky detection grows
 *  into this slot later. */
export function recentVerdicts(
  executions: ExecutionRowWire[],
  caseName: string,
  limit = 5,
): Array<'pass' | 'fail' | 'inconclusive' | 'error'> {
  const out: Array<'pass' | 'fail' | 'inconclusive' | 'error'> = [];
  for (const execution of executions) {
    for (const result of execution.results) {
      if (result.case_id !== caseName) continue;
      out.push(normalizeCaseVerdict(result.verdict));
      if (out.length >= limit) return out;
    }
    if (out.length >= limit) break;
  }
  return out;
}
