/**
 * useTradeStudyRunner — wraps the trade-study backend call into a
 * React-state-oriented hook (idle → running → complete | error).
 *
 * **Backend path chosen** (R5.10 brief asked for a decision):
 *   - Agent AA's `sysml.batch.create { kind: 'trade_study', ... }` does
 *     not exist in this cut of the repo yet — no grep hits for
 *     `batch.create` or `BatchKind` in either crates/tooling or the
 *     frontend. Falling back to the **legacy `sysml.trade_study`
 *     command** that already ships (crates/tooling/sysml-service/src/lib.rs
 *     @ 2549). The legacy command takes `(uri, study_name, overrides)`
 *     and runs the alternatives inside the compiled IR, returning
 *     `{ alternatives: [{name, score}], best, best_score }`.
 *
 * **Shape of the per-alternative run**:
 *   - We still want to run each alternative as its own parameterised
 *     invocation (the brief says alternatives carry `overrides:
 *     Record<string, value>`). The legacy command accepts overrides in
 *     the `[[name, value], ...]` tuple list shape that the rest of the
 *     runner code uses, so we call `sysml.trade_study` once per
 *     alternative, tag the returned score with the alternative's label,
 *     and assemble a `TradeStudyRunResult`.
 *   - When AA's `sysml.batch.create { kind: 'trade_study' }` lands we
 *     swap the inner loop for a single `batch.create` invocation. The
 *     result shape stays the same so HH's viewer keeps working.
 *
 * Progress is reported through a plain callback — a full SessionEventBus
 * integration is overkill for a synchronous N-alternative fan-out.
 */

import { useCallback, useMemo, useRef, useState } from 'react';
import { httpPost, ApiError } from '@/shared/api/http';
import type { Value } from '@/engine/types';
import type { AlternativeConfig, CriterionConfig, TradeStudyObjective } from './useTradeStudyConfig';

// ── Shared types ─────────────────────────────────────────────────────

/** Raw per-alternative entry as returned by `sysml.trade_study`. */
interface RawTradeStudyAlternative {
  name: string;
  score: number;
}

/** Raw `sysml.trade_study` envelope. */
interface RawTradeStudyResult {
  study_name: string;
  alternatives: RawTradeStudyAlternative[];
  best: string | null;
  best_score: number | null;
}

/** Per-alternative score row, after client-side assembly. */
export interface AlternativeScore {
  /** The submitted label (preserved — HH's viewer displays in order). */
  label: string;
  /** The raw scalar score the backend returned for this alternative. */
  score: number;
  /** Index in the submission order. */
  index: number;
  /** The overrides used to produce this score (echoed for drill-back). */
  overrides: Record<string, Value>;
  /** True when the backend errored for this alternative. */
  error?: string;
}

/** Result summary — full output of one trade study run. */
export interface TradeStudyRunResult {
  /** One entry per alternative, in submission order. */
  rows: AlternativeScore[];
  /** Which alternative had the highest-ranked combined objective. */
  bestLabel: string | null;
  /** The best combined score (objective-aware — see `combineScore`). */
  bestScore: number | null;
  /** The criteria used (echoed for HH's viewer). */
  criteria: CriterionConfig[];
  /** Normalised weights (sum to 1) used on submit. */
  weights: number[];
  /** Study name passed to the backend (synthesised from the timestamp
   *  when the UI doesn't have one). */
  studyName: string;
  /** End-to-end duration in ms. */
  durationMs: number;
}

/** Progress callback payload. */
export interface TradeStudyProgressEvent {
  index: number;
  total: number;
  label: string;
}

// ── Config shape passed to the runner ────────────────────────────────

export interface TradeStudyRunConfig {
  /** Backend study name — the `sysml.trade_study` command needs an
   *  analysis-case name. We pass a synthetic name when none is given. */
  studyName?: string;
  /** Alternatives in submission order. */
  alternatives: AlternativeConfig[];
  /** Ranked criteria. */
  criteria: CriterionConfig[];
  /** Weights, index-aligned with `criteria`. Normalised by
   *  `useTradeStudyConfig.normalizedWeights` before submit. */
  weights: number[];
}

// ── Helpers ──────────────────────────────────────────────────────────

/** Serialise Value overrides into the `[name, string]` tuples the
 *  backend expects (mirrors VerifyCaseRunner.overridesToTuples). */
export function overridesToTuples(
  overrides: Record<string, Value> | undefined,
): [string, string][] {
  if (!overrides) return [];
  return Object.entries(overrides).map(([k, v]) => [k, serializeValue(v)]);
}

function serializeValue(v: unknown): string {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return JSON.stringify(v);
}

/**
 * Combine a per-criterion score vector into a single weighted-sum
 * objective, sign-flipping for `'min'` criteria so a higher combined
 * score is always "better" regardless of direction.
 *
 * Exported so HH's viewer / the ranker can reuse it.
 */
export function combineScore(
  scoreByMetric: Record<string, number>,
  criteria: readonly CriterionConfig[],
  weights: readonly number[],
): number {
  let total = 0;
  for (let i = 0; i < criteria.length; i += 1) {
    const c = criteria[i];
    const w = weights[i] ?? 0;
    const raw = scoreByMetric[c.metricId];
    if (!Number.isFinite(raw)) continue;
    const signed = c.objective === 'min' ? -raw : raw;
    total += w * signed;
  }
  return total;
}

/**
 * Flatten an alternative + its criteria into a single score map. Today
 * the legacy `sysml.trade_study` command collapses to a single backend
 * score per alternative (the analysis case's own objective), so we only
 * have one metric dimension to populate. When AA's batch-backed path
 * lands we fill in one entry per criterion here.
 */
function scoreMapForAlternative(
  score: number,
  criteria: readonly CriterionConfig[],
): Record<string, number> {
  const out: Record<string, number> = {};
  for (const c of criteria) out[c.metricId] = score;
  return out;
}

// ── Hook state ───────────────────────────────────────────────────────

export type TradeStudyRunnerState = 'idle' | 'running' | 'complete' | 'error';

export interface TradeStudyRunnerProgress {
  /** 0-based completed alternative count. */
  completed: number;
  /** Total alternatives. */
  total: number;
  /** Label of the most-recently-completed alternative. */
  label: string | null;
}

export interface UseTradeStudyRunnerResult {
  state: TradeStudyRunnerState;
  progress: TradeStudyRunnerProgress | null;
  result: TradeStudyRunResult | null;
  error: Error | null;
  /** Kick off a new run. If a run is already in flight it is cancelled. */
  run: (config: TradeStudyRunConfig) => Promise<TradeStudyRunResult | null>;
  /** Cancel any in-flight run. */
  cancel: () => void;
  /** Clear state back to pristine `idle`. */
  reset: () => void;
}

/** HTTP poster injection point — tests override this. */
type HttpPoster = <T>(path: string, body?: unknown) => Promise<T>;

export interface UseTradeStudyRunnerOptions {
  poster?: HttpPoster;
  /** Optional progress subscriber — the hook also maintains its own
   *  `progress` state; both are driven from the same events. */
  onProgress?: (ev: TradeStudyProgressEvent) => void;
}

/**
 * Dispatch one trade-study run — one backend call per alternative (see
 * the header comment for the rationale on why we fan out rather than
 * call a single batch endpoint).
 *
 * Exported for testing.
 */
export async function runTradeStudyOnce(
  config: TradeStudyRunConfig,
  opts: {
    poster?: HttpPoster;
    signal?: AbortSignal;
    onProgress?: (ev: TradeStudyProgressEvent) => void;
    now?: () => number;
  } = {},
): Promise<TradeStudyRunResult> {
  const poster = opts.poster ?? httpPost;
  const now = opts.now ?? (() => Date.now());
  const start = now();
  const studyName = config.studyName ?? `trade-study-${start.toString(36)}`;
  const rows: AlternativeScore[] = [];
  const total = config.alternatives.length;

  for (let i = 0; i < total; i += 1) {
    throwIfAborted(opts.signal);
    const alt = config.alternatives[i];
    let score = NaN;
    let error: string | undefined;
    try {
      const raw = await poster<RawTradeStudyResult>('/api/command', {
        command: 'sysml.trade_study',
        params: {
          study_name: studyName,
          overrides: overridesToTuples(alt.overrides),
        },
      });
      // The legacy command already evaluates every alternative baked
      // into the analysis case. We read the score for the alternative
      // whose name matches the submitted label; fall back to best_score
      // when the match isn't there (e.g. the model declares a single
      // alternative per run).
      const match = (raw.alternatives ?? []).find((a) => a.name === alt.label);
      score = match?.score ?? raw.best_score ?? NaN;
    } catch (err) {
      if (isAbort(err)) throw err;
      error = err instanceof ApiError ? err.message : String(err);
    }

    const row: AlternativeScore = {
      label: alt.label,
      score,
      index: i,
      overrides: { ...alt.overrides },
      error,
    };
    rows.push(row);
    opts.onProgress?.({ index: i, total, label: alt.label });
  }

  // Rank using the weighted-sum objective.
  let bestIdx = -1;
  let bestCombined = -Infinity;
  for (let i = 0; i < rows.length; i += 1) {
    const row = rows[i];
    if (!Number.isFinite(row.score)) continue;
    const combined = combineScore(
      scoreMapForAlternative(row.score, config.criteria),
      config.criteria,
      config.weights,
    );
    if (combined > bestCombined) {
      bestCombined = combined;
      bestIdx = i;
    }
  }

  return {
    rows,
    bestLabel: bestIdx >= 0 ? rows[bestIdx].label : null,
    bestScore: bestIdx >= 0 ? rows[bestIdx].score : null,
    criteria: config.criteria.map((c) => ({ ...c })),
    weights: [...config.weights],
    studyName,
    durationMs: Math.max(0, now() - start),
  };
}

// ── React hook ───────────────────────────────────────────────────────

export function useTradeStudyRunner(
  opts: UseTradeStudyRunnerOptions = {},
): UseTradeStudyRunnerResult {
  const [state, setState] = useState<TradeStudyRunnerState>('idle');
  const [progress, setProgress] = useState<TradeStudyRunnerProgress | null>(null);
  const [result, setResult] = useState<TradeStudyRunResult | null>(null);
  const [error, setError] = useState<Error | null>(null);

  const abortRef = useRef<AbortController | null>(null);
  const posterRef = useRef<HttpPoster>(opts.poster ?? httpPost);
  posterRef.current = opts.poster ?? httpPost;

  const cancel = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
    setState('idle');
    setProgress(null);
  }, []);

  const reset = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
    setState('idle');
    setProgress(null);
    setResult(null);
    setError(null);
  }, []);

  const run = useCallback(
    async (config: TradeStudyRunConfig): Promise<TradeStudyRunResult | null> => {
      if (abortRef.current) abortRef.current.abort();
      const ctl = new AbortController();
      abortRef.current = ctl;
      setProgress({ completed: 0, total: config.alternatives.length, label: null });
      setError(null);
      setResult(null);
      setState('running');

      try {
        const out = await runTradeStudyOnce(config, {
          poster: posterRef.current,
          signal: ctl.signal,
          onProgress: (ev) => {
            setProgress({
              completed: ev.index + 1,
              total: ev.total,
              label: ev.label,
            });
            opts.onProgress?.(ev);
          },
        });
        setResult(out);
        setState('complete');
        setProgress(null);
        return out;
      } catch (err) {
        if (isAbort(err)) {
          setState('idle');
          setProgress(null);
          return null;
        }
        setError(err instanceof Error ? err : new Error(String(err)));
        setState('error');
        setProgress(null);
        return null;
      } finally {
        if (abortRef.current === ctl) abortRef.current = null;
      }
    },
    // `opts.onProgress` is intentionally *not* tracked — the hook
    // re-reads it via closure on every call, but a changing identity
    // should not recreate the stable `run` callback.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  return useMemo(
    () => ({ state, progress, result, error, run, cancel, reset }),
    [state, progress, result, error, run, cancel, reset],
  );
}

// ── Helpers ──────────────────────────────────────────────────────────

function isAbort(err: unknown): boolean {
  if (err instanceof DOMException && err.name === 'AbortError') return true;
  if (err instanceof Error && err.name === 'AbortError') return true;
  return false;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (!signal || !signal.aborted) return;
  if (typeof DOMException !== 'undefined') {
    throw new DOMException('trade study run cancelled', 'AbortError');
  }
  const err = new Error('trade study run cancelled');
  err.name = 'AbortError';
  throw err;
}
