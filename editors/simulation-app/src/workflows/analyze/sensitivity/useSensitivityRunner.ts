/**
 * useSensitivityRunner — batch runner for the Sensitivity workflow
 * (R7.4).
 *
 * Flow:
 *
 *   1. `start(...)` generates design rows (Morris trajectories or Sobol
 *      A/B/C concatenation) in the same order the backend expects.
 *   2. Calls `sysml.batch.create { kind: 'sensitivity', children_params }`.
 *   3. Polls `sysml.batch.status { batch_id }` at `pollIntervalMs` until
 *      the batch settles.
 *   4. On success, calls `sysml.sensitivity.analyze` and stores the
 *      per-parameter results.
 *
 * The poller + dispatcher pattern mirrors `useSweepRunner` /
 * `useMonteCarloRunner` so the three analyze workflows present the
 * same ergonomic surface to their configs and results shells.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { httpPost } from '@/shared/api/http';
import type {
  BatchSession,
  ChildDescriptor,
  ParamRange,
  SensitivityAnalyzeResult,
  SensitivityMethod,
} from '@/engine/types';
import { unwrapWireChild } from '../sweep/useSweepRunner';
import { morrisSample, toChildrenParams as morrisToChildren } from './morrisSample';
import {
  sobolSample,
  sobolConcat,
  toChildrenParams as sobolToChildren,
} from './sobolSample';

// ── Dispatcher (tests inject a stub) ────────────────────────────────

export type HttpPoster = <T>(
  path: string,
  body?: unknown,
) => Promise<T>;

async function defaultCmd<T>(
  command: string,
  params: Record<string, unknown>,
): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Runner surface ──────────────────────────────────────────────────

export type SensitivityRunnerState =
  | 'idle'
  | 'creating'
  | 'running'
  | 'analyzing'
  | 'complete'
  | 'error';

export interface SensitivityRunnerStartArgs {
  /** Source URI of the loaded model — `sysml.batch.create` param. */
  uri: string;
  /** Which method to run. */
  method: SensitivityMethod;
  /** Parameter ranges, in the exact order the sampler should use. */
  params: ParamRange[];
  /** Morris: trajectory count (defaults to 10 if undefined). */
  r?: number;
  /** Morris: level count (defaults to 4). */
  p?: number;
  /** Sobol: base sample count N (defaults to 64). */
  n?: number;
  /** RNG seed (determinism pin). */
  seed?: number;
  /** Metric key passed to `sysml.sensitivity.analyze`. */
  outputMetric: string;
  /** Optional human label for the batch tab. */
  label?: string;
}

export interface UseSensitivityRunnerResult {
  state: SensitivityRunnerState;
  batchId: string | null;
  children: ChildDescriptor[];
  results: SensitivityAnalyzeResult | null;
  error: string | null;
  start: (args: SensitivityRunnerStartArgs) => Promise<string | null>;
  cancel: () => void;
  reset: () => void;
}

export interface UseSensitivityRunnerOptions {
  /** Poll cadence (ms) — defaults to 500. Tests pass 1. */
  pollIntervalMs?: number;
  /** Injected dispatcher (for tests). */
  poster?: HttpPoster;
}

// ── Hook ────────────────────────────────────────────────────────────

export function useSensitivityRunner(
  opts: UseSensitivityRunnerOptions = {},
): UseSensitivityRunnerResult {
  const { pollIntervalMs = 500, poster } = opts;

  const [state, setState] = useState<SensitivityRunnerState>('idle');
  const [batchId, setBatchId] = useState<string | null>(null);
  const [children, setChildren] = useState<ChildDescriptor[]>([]);
  const [results, setResults] = useState<SensitivityAnalyzeResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const tokenRef = useRef(0);

  const call = useCallback(
    async <T,>(command: string, params: Record<string, unknown>): Promise<T> => {
      if (poster) return poster<T>('/api/command', { command, params });
      return defaultCmd<T>(command, params);
    },
    [poster],
  );

  const cancel = useCallback(() => {
    tokenRef.current += 1;
    setState((prev) =>
      prev === 'creating' || prev === 'running' || prev === 'analyzing' ? 'idle' : prev,
    );
  }, []);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    setState('idle');
    setBatchId(null);
    setChildren([]);
    setResults(null);
    setError(null);
  }, []);

  const start = useCallback(
    async (args: SensitivityRunnerStartArgs): Promise<string | null> => {
      tokenRef.current += 1;
      const token = tokenRef.current;

      setState('creating');
      setBatchId(null);
      setChildren([]);
      setResults(null);
      setError(null);

      // Generate design rows + children_params in the order the
      // backend will replay for the analyzer.
      let rows: number[][];
      if (args.method === 'morris') {
        const r = args.r ?? 10;
        const p = args.p ?? 4;
        rows = morrisSample(args.params, { r, p, seed: args.seed ?? 42 });
      } else {
        const n = args.n ?? 64;
        const mats = sobolSample(args.params, { n, seed: args.seed ?? 42 });
        rows = sobolConcat(mats);
      }
      const childrenParams =
        args.method === 'morris'
          ? morrisToChildren(args.params, rows)
          : sobolToChildren(args.params, rows);

      let created: {
        batch_id?: string | null;
        child_session_ids?: string[] | null;
      } | null = null;
      try {
        const createParams: Record<string, unknown> = {
          kind: 'sensitivity',
          uri: args.uri,
          children_params: JSON.stringify(childrenParams),
        };
        if (args.label) createParams.label = args.label;
        created = await call<{
          batch_id?: string | null;
          child_session_ids?: string[] | null;
        }>('sysml.batch.create', createParams);
      } catch (err) {
        if (tokenRef.current !== token) return null;
        setError(err instanceof Error ? err.message : String(err));
        setState('error');
        return null;
      }
      if (tokenRef.current !== token) return null;

      const id = created?.batch_id ?? null;
      if (!id) {
        setError('Backend did not return a batch_id');
        setState('error');
        return null;
      }
      setBatchId(id);
      setState('running');

      // ── Drive children to terminal state ────────────────────────
      // See BUG 26 in docs/test-checklist-2026-04-20.md. Same pattern
      // as useMonteCarloRunner: step each child up to a cap, then
      // `sessions.stop` to flip the batch descriptor to Complete.
      const childIds = created?.child_session_ids ?? [];
      const MAX_STEPS_PER_CHILD = 32;
      const driveChild = async (sid: string): Promise<void> => {
        for (let i = 0; i < MAX_STEPS_PER_CHILD; i++) {
          if (tokenRef.current !== token) return;
          let summary: { completed?: boolean } | null = null;
          try {
            summary = await call<{ completed?: boolean }>(
              'sysml.sessions.step',
              { session_id: sid },
            );
          } catch {
            break;
          }
          if (summary?.completed) break;
        }
        if (tokenRef.current !== token) return;
        try {
          await call<unknown>('sysml.sessions.stop', { session_id: sid });
        } catch {
          /* best-effort */
        }
      };
      void Promise.all(childIds.map(driveChild));

      // ── Poll loop ─────────────────────────────────────────────────
      while (tokenRef.current === token) {
        let status: BatchSession | null = null;
        try {
          const resp = await call<{ batch?: BatchSession } | BatchSession>(
            'sysml.batch.status',
            { batch_id: id },
          );
          // The service wraps `batch_status` in `{ batch: ... }`; the
          // frontend poller also accepts the unwrapped form so test
          // dispatchers can return it directly.
          status =
            resp && typeof resp === 'object' && 'batch' in resp
              ? (resp as { batch: BatchSession }).batch
              : (resp as BatchSession);
        } catch (err) {
          if (tokenRef.current !== token) return id;
          setError(err instanceof Error ? err.message : String(err));
          setState('error');
          return id;
        }
        if (tokenRef.current !== token) return id;

        if (status) {
          // Child `status` arrives tag-wrapped on the wire ({status: '…'})
          // — unwrap via the sweep runner's shared helper (same bug class,
          // one fix home).
          if (Array.isArray(status.children)) setChildren(status.children.map(unwrapWireChild));
          // Backend `BatchStatus` is serde-tagged with `status`, not
          // `kind`. Read the wire tag regardless of which frontend
          // type the poller was given.
          const raw = (status.status ?? {}) as {
            status?: string;
            kind?: string;
            reason?: string;
          };
          const tag = raw.status ?? raw.kind;
          if (tag === 'failed') {
            setError(raw.reason ?? 'Batch failed');
            setState('error');
            return id;
          }
          if (tag === 'complete') break;
        }

        await sleep(pollIntervalMs);
      }

      if (tokenRef.current !== token) return id;

      // ── Analyze ───────────────────────────────────────────────────
      setState('analyzing');
      try {
        const payload = {
          batch_id: id,
          method: args.method,
          parameters_of_interest: JSON.stringify(args.params),
          output_metric: args.outputMetric,
          ...(args.method === 'morris' && args.p !== undefined
            ? { morris_levels: args.p }
            : {}),
        };
        const result =
          await call<SensitivityAnalyzeResult>('sysml.sensitivity.analyze', payload);
        if (tokenRef.current !== token) return id;
        setResults(result);
        setState('complete');
      } catch (err) {
        if (tokenRef.current !== token) return id;
        setError(err instanceof Error ? err.message : String(err));
        setState('error');
      }
      return id;
    },
    [call, pollIntervalMs],
  );

  // Abandon any in-flight run on unmount so polls don't write into
  // stale state.
  useEffect(() => {
    return () => {
      tokenRef.current += 1;
    };
  }, []);

  return { state, batchId, children, results, error, start, cancel, reset };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
