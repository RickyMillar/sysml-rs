/**
 * useVerifyRunner — React hook that owns a VerifyCaseRunner's state.
 *
 * R3.2 contract:
 *   - States: `idle | running | complete | error`.
 *   - `verdicts: Verdict[]` — last completed (or in-progress) run's output.
 *   - `progress: { index, total } | null` — live counter for the "N of M"
 *     display while running.
 *   - `run(config)` — start a run; if one is already running it is
 *     cancelled first (idempotent per the brief).
 *   - `cancel()` — abort the in-flight run.
 *   - `reset()` — clear state back to `idle`.
 *
 * The underlying runner is injected (for tests) but defaults to a
 * freshly-constructed `VerifyCaseRunner` shared across renders.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  Verdict,
  VerifyRunConfig,
  VerifyRunResult,
} from '@/engine/types';
import {
  VerifyCaseRunner,
  type VerifyProgressEvent,
} from './VerifyCaseRunner';

export type VerifyRunnerState = 'idle' | 'running' | 'complete' | 'error';

export interface VerifyRunnerProgress {
  /** 0-based completed case count. */
  completed: number;
  /** Total cases to process in this run. */
  total: number;
  /** The case id of the most recent progress event (for status lines). */
  caseId: string | null;
}

export interface UseVerifyRunnerResult {
  state: VerifyRunnerState;
  verdicts: Verdict[];
  progress: VerifyRunnerProgress | null;
  lastResult: VerifyRunResult | null;
  error: Error | null;
  /** Kick off a new run. A run already in flight is cancelled first. */
  run: (config: VerifyRunConfig) => Promise<VerifyRunResult | null>;
  /** Cancel any in-flight run; transitions to `idle`. */
  cancel: () => void;
  /** Clear state back to a pristine `idle`. */
  reset: () => void;
}

export interface UseVerifyRunnerOptions {
  /**
   * Inject a custom runner (tests supply a mock here). When omitted the
   * hook constructs its own and keeps it for the component lifetime.
   */
  runner?: VerifyCaseRunner;
}

/**
 * React hook — owns a VerifyCaseRunner's lifecycle and reactive state.
 *
 * Stable across renders; the returned object only changes identity when
 * state transitions. Safe to call `run(...)` from event handlers without
 * `useCallback` gymnastics at the call site.
 */
export function useVerifyRunner(
  opts: UseVerifyRunnerOptions = {},
): UseVerifyRunnerResult {
  // A stable runner instance. Tests pass their own; otherwise we make one.
  const runnerRef = useRef<VerifyCaseRunner | null>(null);
  if (!runnerRef.current) {
    runnerRef.current = opts.runner ?? new VerifyCaseRunner();
  }
  // If the caller swaps `opts.runner` later we swap too (rare but honoured).
  if (opts.runner && opts.runner !== runnerRef.current) {
    runnerRef.current = opts.runner;
  }

  const [state, setState] = useState<VerifyRunnerState>('idle');
  const [verdicts, setVerdicts] = useState<Verdict[]>([]);
  const [progress, setProgress] = useState<VerifyRunnerProgress | null>(null);
  const [lastResult, setLastResult] = useState<VerifyRunResult | null>(null);
  const [error, setError] = useState<Error | null>(null);

  // Track the "run id" of the currently-active run so a late-arriving
  // progress event from a superseded run doesn't clobber fresh state.
  const activeRunIdRef = useRef<string | null>(null);

  // Subscribe once for the lifetime of the hook; the handler filters by
  // the active run id.
  useEffect(() => {
    const runner = runnerRef.current!;
    const unsub = runner.onProgress((ev: VerifyProgressEvent) => {
      if (ev.runId !== activeRunIdRef.current) return;
      setVerdicts((prev) => [...prev, ev.verdict]);
      setProgress({
        completed: ev.index + 1,
        total: ev.total,
        caseId: ev.caseId,
      });
    });
    return unsub;
  }, []);

  const cancel = useCallback(() => {
    const runner = runnerRef.current;
    if (!runner) return;
    runner.cancel();
    activeRunIdRef.current = null;
    setState('idle');
    setProgress(null);
  }, []);

  const reset = useCallback(() => {
    const runner = runnerRef.current;
    if (runner) runner.cancel();
    activeRunIdRef.current = null;
    setState('idle');
    setVerdicts([]);
    setProgress(null);
    setLastResult(null);
    setError(null);
  }, []);

  const run = useCallback(
    async (config: VerifyRunConfig): Promise<VerifyRunResult | null> => {
      const runner = runnerRef.current!;
      // Cancel any prior run before starting a new one. The ref guard
      // ensures stale progress events from the prior run are ignored.
      runner.cancel();
      // Reset per-run state but keep `lastResult` around so UIs can
      // still see the previous run's output while a new one is loading.
      setVerdicts([]);
      setProgress({ completed: 0, total: 0, caseId: null });
      setError(null);
      setState('running');

      let result: VerifyRunResult | null = null;
      try {
        const promise = runner.run(config);
        // Capture the runner's assigned run id once it becomes visible.
        // `activeRunId()` returns the current id synchronously after
        // `run()` was called.
        activeRunIdRef.current = runner.activeRunId();
        result = await promise;
        // Final canonical verdict list comes from the result — replaces
        // the accumulated in-progress list (defensive: if a step's
        // progress event fired synchronously before we could capture
        // the id, we'd still end up with the correct state here).
        setVerdicts(result.verdicts);
        setLastResult(result);
        setState('complete');
        setProgress(null);
        return result;
      } catch (err) {
        // AbortError from `cancel()` does not transition to 'error' —
        // the user asked to stop; go back to idle silently.
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
        activeRunIdRef.current = null;
      }
    },
    [],
  );

  // Clean up on unmount: cancel any in-flight run.
  useEffect(() => {
    return () => {
      const runner = runnerRef.current;
      if (runner) runner.cancel();
    };
  }, []);

  return useMemo<UseVerifyRunnerResult>(
    () => ({ state, verdicts, progress, lastResult, error, run, cancel, reset }),
    [state, verdicts, progress, lastResult, error, run, cancel, reset],
  );
}

// ── Helpers ──────────────────────────────────────────────────────────

function isAbort(err: unknown): boolean {
  if (err instanceof DOMException && err.name === 'AbortError') return true;
  if (err instanceof Error && err.name === 'AbortError') return true;
  return false;
}
