/**
 * useVerificationCases — the static read behind the Cases sub-view and the
 * case-as-document view (Verify design 1a).
 *
 * Calls `sysml.evaluate.verification_cases` — the array-shaped static
 * evaluate read (§4.1 of the verify-design-consolidation brief). Unlike the
 * runner's `Verdict[]`, which flattens each case into per-requirement rows
 * and DROPS the case-level `requirements` list during expansion, this hook
 * returns the RAW nested case rows verbatim. That nesting — the objective's
 * check occurrences and their recursive `subrequirements` chains — is exactly
 * what the case document renders, so the case view reads it here rather than
 * from the run.
 *
 * ## `evaluation_mode` is always `static` on this read
 * This read recomputes against the current graph on every model edit; it is
 * never a trajectory or external verdict. The mode is carried explicitly so
 * the mode badge has an honest source (§1.3).
 *
 * ## model_digest (§E — steward ruling 2026-07-19)
 * This read stays a **bare array**, permanently — no `{ cases, model_digest }`
 * envelope (steward: wrapping breaks every consumer to solve a need a sibling
 * call already meets; per-row duplication of a run-level scalar is the
 * denormalized-provenance anti-pattern). The 1a frame chip's digest is
 * sourced from `sysml.workspace.verify`'s `model_digest` field instead. This
 * hook carries no digest; the case view renders the chip only when it holds a
 * `workspace.verify` result (deferred — see the case-view billet).
 *
 * ## stdlib noise
 * On the demo workspace this command also returns rows for stdlib base
 * features (`verificationCases`, `self`, `VerificationCase`…). Filed to fix
 * on the wire; until then we skip them by name so the surfaces design
 * against real cases only (brief §2 live-data note).
 */

import { useQuery } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { useWorkspaceUIStore } from '@/features/workspace/store';

// ── Wire types — the §4.1 payload shape ──────────────────────────────

/**
 * One check occurrence's per-requirement result. `subrequirements` is the
 * recursive nested chain (a whole-clause requirement rolls up its
 * sub-obligations, which fail through referenced obligations several levels
 * deep — ClauseFourReview in the demo).
 */
export interface VerificationCaseRequirement {
  requirement_id?: string;
  requirement_name?: string;
  /** ElementId of the verified requirement — the join key to its
   *  Requirements-workbench row. */
  requirement_element_id?: string;
  element_id?: string;
  requirement_text?: string | null;
  /** Lowercase `pass|fail|inconclusive|error`. */
  verdict?: string;
  message?: string;
  actual?: unknown;
  expected?: unknown;
  margin?: number | null;
  error?: string | null;
  /** Binding redefinitions declared on the check (`attribute limit = 5;`). */
  constraints?: unknown[];
  /** The recursive failure chain — sub-obligations of this requirement. */
  subrequirements?: VerificationCaseRequirement[];
}

/** One verification case row (§4.1). */
export interface VerificationCaseRow {
  element_id?: string;
  case_id?: string;
  case_name?: string;
  /** Fully qualified name off the ownership chain (`Pkg::Sub::Case`) —
   *  the model-structure grouping key for the History latest-status
   *  bands. Absent when the case or an ancestor is unnamed. */
  qualified_name?: string | null;
  subject?: string | null;
  /** DECLARED @VerificationMethod kinds (layer 1; [] = none declared). */
  methods?: string[];
  /** How the tool COMPUTED the verdict (layer 2) — always `static` here. */
  evaluation_mode?: string;
  /** Case-level verdict — `Pass|Fail|Inconclusive|Error` (backend casing). */
  verdict?: string;
  display?: string;
  passed_requirements?: number;
  total_requirements?: number;
  requirements?: VerificationCaseRequirement[];
  diagnostics?: string[];
}

// ── stdlib-noise filter ──────────────────────────────────────────────

/** Base features the evaluate read leaks on the demo workspace — never
 *  real user cases. Filed to fix on the wire; skipped by name here. */
const STDLIB_CASE_NAMES = new Set(['verificationCases', 'self', 'VerificationCase']);

function isRealCase(row: VerificationCaseRow): boolean {
  const name = row.case_name ?? row.case_id ?? '';
  return name.length > 0 && !STDLIB_CASE_NAMES.has(name);
}

// ── Response normalisation (unconditional bare array) ────────────────

/**
 * Normalise the bare-array response, dropping stdlib noise. A non-array
 * response (malformed / older-shape) yields an empty list rather than a
 * throw. Exported for tests.
 */
export function normalizeVerificationCasesResponse(raw: unknown): VerificationCaseRow[] {
  if (!Array.isArray(raw)) return [];
  return (raw as VerificationCaseRow[]).filter(isRealCase);
}

// ── Query ────────────────────────────────────────────────────────────

export const verificationCasesKeys = {
  all: ['verification-cases'] as const,
  forWorkspace: (root: string | null) => [...verificationCasesKeys.all, root ?? ''] as const,
};

export async function fetchVerificationCases(): Promise<VerificationCaseRow[]> {
  const raw = await httpPost<unknown>('/api/command', {
    command: 'sysml.evaluate.verification_cases',
    params: {},
  });
  return normalizeVerificationCasesResponse(raw);
}

/**
 * The static verification-case read for the whole workspace. Gated on a
 * loaded workspace (fail-honest: no fetch without one). Findable by
 * `case_id` / `case_name` for the case-view lookup.
 */
export function useVerificationCases() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  return useQuery({
    queryKey: verificationCasesKeys.forWorkspace(workspaceRoot),
    queryFn: fetchVerificationCases,
    enabled: !!workspaceRoot,
    staleTime: 15_000,
  });
}

// ── Lookup + rollup helpers (pure, exported for reuse + tests) ────────

/** Stable identity for a case row — the id the matrix/list open on. */
export function caseIdOf(row: VerificationCaseRow): string {
  return row.case_id ?? row.element_id ?? row.case_name ?? '';
}

/** Find a case by the id the matrix/list opened (case_id, element_id, or name). */
export function findCase(
  cases: VerificationCaseRow[],
  id: string | null,
): VerificationCaseRow | null {
  if (!id) return null;
  return (
    cases.find((c) => c.case_id === id) ??
    cases.find((c) => c.element_id === id) ??
    cases.find((c) => c.case_name === id) ??
    null
  );
}

export interface SuiteRollup {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
  total: number;
}

/**
 * Suite-level verdict counts across the static case read — the "suite —
 * 2 pass · 1 fail · …" header that absorbs the retired Aggregate sub-view.
 * A case that binds no checks (`total_requirements === 0`) mints no verdict
 * (1e), so it is counted as neither pass nor fail — it drops out of the
 * rollup rather than fabricating an inconclusive.
 */
export function suiteRollup(cases: VerificationCaseRow[]): SuiteRollup {
  const out: SuiteRollup = { pass: 0, fail: 0, inconclusive: 0, error: 0, total: 0 };
  for (const c of cases) {
    if (isBareObjectiveRow(c)) continue;
    const kind = normalizeCaseVerdict(c.verdict);
    out[kind] += 1;
    out.total += 1;
  }
  return out;
}

/** A case whose objective binds no checks — mints no verdict (1e). */
export function isBareObjectiveRow(row: VerificationCaseRow): boolean {
  return typeof row.total_requirements === 'number' && row.total_requirements === 0;
}

/** Normalise a case-level verdict string to the canonical ladder. */
export function normalizeCaseVerdict(raw: unknown): 'pass' | 'fail' | 'inconclusive' | 'error' {
  const lower = String(raw ?? '').toLowerCase();
  if (lower === 'pass' || lower === 'fail' || lower === 'error') return lower;
  return 'inconclusive';
}
