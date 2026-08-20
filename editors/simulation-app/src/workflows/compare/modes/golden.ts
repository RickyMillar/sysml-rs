/**
 * Golden-baseline mode (R4.3) — verdict vs a designated golden run.
 *
 * UX goal: is this new candidate session still "good" vs the locked
 * golden run? For every (session × variable) we emit a Verdict (the
 * canonical 4-valued one from engine/types). A cell is `pass` iff
 * every tick in the configured compare-window is within tolerance of
 * the golden; otherwise `fail`. Non-golden sessions can be rolled up
 * to an overall per-session verdict.
 *
 * Tolerance model:
 *   - per-variable, user-configurable
 *   - kind = 'relative' (fraction, default 0.05 = 5%) OR 'absolute'
 *     (raw value). Relative degrades to absolute when the golden
 *     sample is ≈ 0 (`|golden| < 1e-12`) — prevents div-by-zero.
 *
 * The `PassFailGridViewer` from R3.3 renders the verdict grid
 * (sessions = rows, variables = columns). The shared-playhead waveform
 * is Agent W's responsibility; we just emit verdicts with
 * `metadata.case_name` / `metadata.requirement_id` hooked up so the
 * viewer picks them up automatically.
 *
 * Backend `fork_with_overrides(golden_id, at_tick)` is Agent Z's work.
 * This mode exposes `buildForkAtTickIntent` + gates the UI behind a
 * capability flag; when the flag is false, the shell shows the archived
 * snapshot at the playhead tick instead (safe fallback).
 */
import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { CompareContext, CompareMode } from '../compareMode';
import type { TimePoint } from '../../../features/sessions/types';
import type { Verdict, VerdictKind } from '../../../engine/types';
import { isFiniteNumber, sampleAtTick } from './seriesAccess';

// ── Tolerance model ──────────────────────────────────────────────────

/**
 * How we compare a candidate sample to the golden at one tick.
 *
 * `relative` is a fractional band around the golden value
 * (`|cand − gold| <= relative · |gold|`). Falls back to absolute when
 * `|gold|` is too small to meaningfully multiply (the 1e-12 floor is
 * arbitrary but smaller than any sensible engineering scale).
 *
 * `absolute` is a raw numeric band (`|cand − gold| <= absolute`).
 */
export type Tolerance =
  | { kind: 'relative'; value: number }
  | { kind: 'absolute'; value: number };

/** Default 5 % relative tolerance. Chosen so rounding noise doesn't flip verdicts. */
export const DEFAULT_TOLERANCE: Tolerance = { kind: 'relative', value: 0.05 };

/**
 * Which ticks to compare over. Matches the plan doc's three windows.
 *
 * `whole-run`      — every tick in the candidate series.
 * `last-fraction`  — last `fraction` (0..1] of the candidate's range.
 * `range`          — inclusive tick interval, user-drawn.
 */
export type CompareWindow =
  | { kind: 'whole-run' }
  | { kind: 'last-fraction'; fraction: number }
  | { kind: 'range'; startTick: number; endTick: number };

/** Default window — whole run. */
export const DEFAULT_COMPARE_WINDOW: CompareWindow = { kind: 'whole-run' };

// ── Pure helpers ─────────────────────────────────────────────────────

/**
 * Resolve a `CompareWindow` against a candidate's tick range. Returns
 * `null` when the candidate has no samples.
 */
export function resolveWindow(
  candidate: TimePoint[],
  window: CompareWindow,
): { startTick: number; endTick: number } | null {
  if (candidate.length === 0) return null;
  const first = candidate[0].t;
  const last = candidate[candidate.length - 1].t;
  switch (window.kind) {
    case 'whole-run':
      return { startTick: first, endTick: last };
    case 'last-fraction': {
      const f = Math.min(1, Math.max(0, window.fraction));
      const span = last - first;
      return { startTick: last - span * f, endTick: last };
    }
    case 'range':
      return {
        startTick: Math.max(first, Math.min(window.startTick, window.endTick)),
        endTick: Math.min(last, Math.max(window.startTick, window.endTick)),
      };
  }
}

/**
 * Decide whether `candidate` is within `tolerance` of `golden` at a
 * single sample. Non-finite inputs → non-pass.
 */
export function withinTolerance(
  golden: number | null,
  candidate: number | null,
  tolerance: Tolerance,
): boolean {
  if (!isFiniteNumber(golden) || !isFiniteNumber(candidate)) return false;
  const delta = Math.abs(candidate - golden);
  if (tolerance.kind === 'absolute') return delta <= tolerance.value;
  // Relative — degrade to absolute when golden is ~0 to avoid div-by-zero semantics.
  const goldMag = Math.abs(golden);
  if (goldMag < 1e-12) return delta <= tolerance.value;
  return delta <= tolerance.value * goldMag;
}

/**
 * Result of a golden comparison for one (session × variable) pair.
 *
 * `verdict` matches the canonical snake-case `VerdictKind`
 * (`'pass' | 'fail' | 'inconclusive' | 'error'`). `'error'` surfaces
 * when the evaluation itself went sideways (non-finite delta,
 * negative tolerance); analysts want that distinct from `'fail'` so
 * they don't mistake instrumentation breakage for design regression.
 */
export interface GoldenVerdictOutcome {
  /** Canonical `VerdictKind` (R3 reconciled — includes `'error'`). */
  verdict: VerdictKind;
  /** Largest absolute delta seen across the evaluated ticks (0 when pass + equal). */
  maxDelta: number;
  /** First tick at which the candidate exceeded tolerance. Null when pass / no-data. */
  firstFailTick: number | null;
  /** Number of ticks that were actually evaluated (finite on both sides). */
  evaluatedTicks: number;
  /** Populated when `verdict === 'error'`. */
  errorReason?: string;
}

/**
 * Decide pass/fail over a candidate series vs a golden series.
 *
 * Samples both series at the union of their tick grids (inside the
 * resolved window) using last-known-value semantics. A single out-of-
 * tolerance tick drives the overall verdict to `fail`.
 *
 * Error rules (return `'error'`, not `'fail'` or `'inconclusive'`):
 *   - Tolerance value is negative or non-finite.
 *   - Any evaluated tick produces a non-finite delta (shouldn't happen
 *     because `isFiniteNumber` gates the inputs, but we guard against
 *     corrupt payloads: `Infinity − Infinity = NaN`).
 *
 * Inconclusive when there is simply no overlap / no evaluable tick.
 */
export function computeGoldenVerdict(
  golden: TimePoint[],
  candidate: TimePoint[],
  tolerance: Tolerance,
  window: CompareWindow = DEFAULT_COMPARE_WINDOW,
): GoldenVerdictOutcome {
  // Tolerance sanity — negative / non-finite is a configuration error,
  // not a data-driven fail. Surface it as `'error'` so the UI can show
  // "fix your inputs" instead of "your design regressed".
  if (!isFiniteNumber(tolerance.value) || tolerance.value < 0) {
    return {
      verdict: 'error',
      maxDelta: 0,
      firstFailTick: null,
      evaluatedTicks: 0,
      errorReason: `Invalid tolerance: ${tolerance.kind}=${tolerance.value}`,
    };
  }

  const range = resolveWindow(candidate, window);
  if (!range) {
    return { verdict: 'inconclusive', maxDelta: 0, firstFailTick: null, evaluatedTicks: 0 };
  }

  // Union of tick grids that fall inside [startTick, endTick].
  const ticks = new Set<number>();
  for (const p of golden) {
    if (p.t >= range.startTick && p.t <= range.endTick) ticks.add(p.t);
  }
  for (const p of candidate) {
    if (p.t >= range.startTick && p.t <= range.endTick) ticks.add(p.t);
  }
  const sorted = [...ticks].sort((a, b) => a - b);

  let maxDelta = 0;
  let firstFailTick: number | null = null;
  let evaluatedTicks = 0;
  for (const t of sorted) {
    const g = sampleAtTick(golden, t);
    const c = sampleAtTick(candidate, t);
    if (!isFiniteNumber(g) || !isFiniteNumber(c)) continue;
    const delta = Math.abs(c - g);
    // Defence in depth — `c − g` with both finite can't really produce
    // NaN, but a corrupted payload that slipped past `isFiniteNumber`
    // (e.g. boxed Number(NaN)) would. Surface as `'error'`.
    if (!Number.isFinite(delta)) {
      return {
        verdict: 'error',
        maxDelta,
        firstFailTick,
        evaluatedTicks,
        errorReason: `Non-finite delta at tick ${t}`,
      };
    }
    evaluatedTicks += 1;
    if (delta > maxDelta) maxDelta = delta;
    if (!withinTolerance(g, c, tolerance) && firstFailTick === null) {
      firstFailTick = t;
    }
  }

  if (evaluatedTicks === 0) {
    return { verdict: 'inconclusive', maxDelta: 0, firstFailTick: null, evaluatedTicks: 0 };
  }
  return {
    verdict: firstFailTick === null ? 'pass' : 'fail',
    maxDelta,
    firstFailTick,
    evaluatedTicks,
  };
}

// ── Verdict-grid producer ────────────────────────────────────────────

/** One candidate session's time series per variable name. */
export interface GoldenCandidate {
  sessionId: string;
  /** Optional display label — falls back to `sessionId`. */
  label?: string;
  /** `variable name → TimePoint[]`. */
  variables: Record<string, TimePoint[]>;
}

/** Golden reference — same shape, just semantically "the truth". */
export interface GoldenReference {
  sessionId: string;
  label?: string;
  variables: Record<string, TimePoint[]>;
}

/**
 * Produce a flat `Verdict[]` ready to hand to `PassFailGridViewer`.
 *
 * Rows = candidate sessions (via `metadata.case_name`), columns = variables
 * (via `metadata.requirement_id`). This is exactly the taxonomy the
 * viewer already derives at R3.3; no fork of that component is needed.
 *
 * Tolerance is resolved per-variable via `tolerances[name]`, falling
 * back to the global `defaultTolerance`.
 */
export function buildGoldenGridVerdicts(
  golden: GoldenReference,
  candidates: GoldenCandidate[],
  variables: string[],
  defaultTolerance: Tolerance,
  tolerances: Record<string, Tolerance> = {},
  window: CompareWindow = DEFAULT_COMPARE_WINDOW,
): Verdict[] {
  const verdicts: Verdict[] = [];
  for (const cand of candidates) {
    const caseName = cand.label ?? cand.sessionId;
    for (const variable of variables) {
      const tol = tolerances[variable] ?? defaultTolerance;
      const goldSeries = golden.variables[variable] ?? [];
      const candSeries = cand.variables[variable] ?? [];
      const outcome = computeGoldenVerdict(goldSeries, candSeries, tol, window);
      verdicts.push({
        verdict: outcome.verdict,
        id: `${cand.sessionId}::${variable}`,
        label: `${caseName} · ${variable}`,
        margin: outcome.maxDelta,
        reason:
          outcome.verdict === 'inconclusive'
            ? 'No evaluable tick in compare window'
            : outcome.verdict === 'error'
              ? (outcome.errorReason ?? 'Non-finite comparison')
              : null,
        // Mirror the evidence shape used everywhere else (snake_case per R3 reconcile).
        evidence:
          outcome.firstFailTick != null
            ? { session_id: cand.sessionId, tick: outcome.firstFailTick }
            : { session_id: cand.sessionId, tick: 0 },
        metadata: {
          case_name: caseName,
          requirement_id: variable,
          tolerance_kind: tol.kind,
          tolerance_value: tol.value,
          evaluated_ticks: outcome.evaluatedTicks,
          max_delta: outcome.maxDelta,
          golden_session_id: golden.sessionId,
          error_reason: outcome.errorReason ?? undefined,
          message:
            outcome.verdict === 'pass'
              ? `within ${tol.kind} tolerance ${tol.value}`
              : outcome.verdict === 'fail'
                ? `max Δ=${outcome.maxDelta.toPrecision(4)} at tick ${outcome.firstFailTick}`
                : outcome.verdict === 'error'
                  ? (outcome.errorReason ?? 'Comparison error')
                  : 'no overlap with golden',
        },
      });
    }
  }
  return verdicts;
}

/**
 * Roll up per-(session × variable) verdicts into a single per-session
 * verdict. Precedence (worst-wins): `error` > `fail` > `inconclusive`
 * > `pass`. Matches the R3 reconcile ordering so the Compare workflow
 * agrees with Verify rollups.
 */
export function rollupPerSession(verdicts: Verdict[]): Map<string, VerdictKind> {
  const rank: Record<VerdictKind, number> = {
    pass: 0,
    inconclusive: 1,
    fail: 2,
    error: 3,
  };
  const by = new Map<string, VerdictKind>();
  for (const v of verdicts) {
    const sid = (v.metadata?.case_name as string) ?? (v.id ?? 'unknown');
    const prev = by.get(sid);
    if (prev === undefined || rank[v.verdict] > rank[prev]) {
      by.set(sid, v.verdict);
    }
  }
  return by;
}

// ── Fork-at-tick intent (gated on Agent Z) ───────────────────────────

/** Capability flags the Compare shell can pass into golden mode. */
export interface GoldenCapabilities {
  /** Backend exposes `sysml.sessions.fork_with_overrides` with `at_tick`. */
  has_fork_at_tick: boolean;
}

/** Default — Agent Z's work may or may not have landed. Conservative. */
export const DEFAULT_GOLDEN_CAPABILITIES: GoldenCapabilities = {
  has_fork_at_tick: false,
};

/**
 * A declarative intent the shell can hand off to the RunWorkflow:
 * "fork the golden at this tick and replay". Pure data — no IPC here.
 * When `has_fork_at_tick` is false, the shell should fall back to
 * showing the archived snapshot at `tick` instead.
 */
export interface ForkAtTickIntent {
  kind: 'fork-at-tick' | 'show-snapshot';
  goldenSessionId: string;
  tick: number;
}

/** Build a fork-at-tick intent, gated on backend capability. */
export function buildForkAtTickIntent(
  goldenSessionId: string,
  tick: number,
  caps: GoldenCapabilities,
): ForkAtTickIntent {
  return {
    kind: caps.has_fork_at_tick ? 'fork-at-tick' : 'show-snapshot',
    goldenSessionId,
    tick,
  };
}

// ── React surface ────────────────────────────────────────────────────

function GoldenConfigPanel(_ctx: CompareContext): ReactNode {
  return createElement(
    'div',
    {
      'data-testid': 'golden-config',
      style: {
        padding: 12,
        display: 'flex',
        flexDirection: 'column',
        gap: 10,
        fontSize: 12,
        color: 'var(--on-surface)',
      },
    },
    createElement('div', { style: { fontWeight: 600 } }, 'Golden baseline'),
    createElement(
      'div',
      { style: { opacity: 0.8 } },
      'Pick a session marked as golden in the archive. Every other picked session is compared to it over the chosen window.',
    ),
    createElement('div', { style: { fontWeight: 600, marginTop: 6 } }, 'Tolerance'),
    createElement(
      'label',
      { style: { display: 'flex', gap: 6, alignItems: 'center' } },
      createElement('span', null, 'Default relative'),
      createElement('input', {
        type: 'number',
        defaultValue: 0.05,
        step: 0.01,
        min: 0,
        'data-testid': 'golden-default-rel-input',
        style: {
          width: 72,
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          padding: '2px 6px',
        },
      }),
    ),
    createElement('div', { style: { fontWeight: 600, marginTop: 6 } }, 'Compare window'),
    createElement(
      'select',
      {
        'data-testid': 'golden-window-select',
        defaultValue: 'whole-run',
        style: {
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          padding: '2px 6px',
        },
      },
      createElement('option', { value: 'whole-run' }, 'Whole run'),
      createElement('option', { value: 'last-10' }, 'Last 10%'),
      createElement('option', { value: 'range' }, 'Custom range…'),
    ),
  );
}

/**
 * The golden-baseline mode. `mainRender` is intentionally omitted — the
 * shell composes its default waveform overlay with the PassFailGrid;
 * the grid consumes `buildGoldenGridVerdicts` output directly.
 */
export const goldenMode: CompareMode = {
  id: 'golden',
  label: 'Golden baseline',
  description: 'Compare candidate sessions to a designated golden — verdict per variable over time.',
  configRender: GoldenConfigPanel,
};
