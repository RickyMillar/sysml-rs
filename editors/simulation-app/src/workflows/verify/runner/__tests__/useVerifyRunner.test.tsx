/**
 * Hook tests for useVerifyRunner.
 *
 * Uses React Testing Library's `renderHook` + act. The runner is
 * injected as a mock so no HTTP is issued. Covers:
 *   - idle → running → complete transitions
 *   - idle → running → error transitions
 *   - idempotency: a second run cancels the first
 *   - cancel transitions back to idle
 *   - reset clears all state
 *   - verdict list is final from the returned result, not just the
 *     accumulator of in-progress events
 */

import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useVerifyRunner } from '../useVerifyRunner';
import type { VerifyProgressEvent } from '../VerifyCaseRunner';
import { VerifyCaseRunner } from '../VerifyCaseRunner';
import type { Verdict, VerifyRunConfig, VerifyRunResult } from '@/engine/types';

// ── Mock runner ──────────────────────────────────────────────────────

/**
 * Build a mock that quacks like a VerifyCaseRunner. Tests control the
 * promise lifecycle directly.
 */
function makeMockRunner(): {
  runner: VerifyCaseRunner;
  emitProgress: (ev: VerifyProgressEvent) => void;
  resolve: (result: VerifyRunResult) => void;
  reject: (err: unknown) => void;
  currentRunId: () => string | null;
  runCalls: VerifyRunConfig[];
} {
  const listeners = new Set<(ev: VerifyProgressEvent) => void>();
  let pendingResolve: ((r: VerifyRunResult) => void) | null = null;
  let pendingReject: ((err: unknown) => void) | null = null;
  let runId: string | null = null;
  const runCalls: VerifyRunConfig[] = [];

  const mock = {
    onProgress: (cb: (ev: VerifyProgressEvent) => void) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    activeRunId: () => runId,
    cancel: () => {
      if (pendingReject) {
        const rej = pendingReject;
        pendingReject = null;
        pendingResolve = null;
        runId = null;
        rej(new DOMException('cancelled', 'AbortError'));
      }
    },
    run: (config: VerifyRunConfig): Promise<VerifyRunResult> => {
      runCalls.push(config);
      // Abort any prior pending run.
      if (pendingReject) {
        const rej = pendingReject;
        pendingReject = null;
        pendingResolve = null;
        rej(new DOMException('superseded', 'AbortError'));
      }
      runId = `mock-run-${runCalls.length}`;
      return new Promise<VerifyRunResult>((resolve, reject) => {
        pendingResolve = resolve;
        pendingReject = reject;
      });
    },
  };

  return {
    runner: mock as unknown as VerifyCaseRunner,
    emitProgress: (ev) => {
      for (const cb of Array.from(listeners)) cb(ev);
    },
    resolve: (result) => {
      if (pendingResolve) {
        const r = pendingResolve;
        pendingResolve = null;
        pendingReject = null;
        runId = null;
        r(result);
      }
    },
    reject: (err) => {
      if (pendingReject) {
        const rej = pendingReject;
        pendingReject = null;
        pendingResolve = null;
        runId = null;
        rej(err);
      }
    },
    currentRunId: () => runId,
    runCalls,
  };
}

function pv(kind: Verdict['verdict'], meta: Record<string, unknown> = {}): Verdict {
  return {
    verdict: kind,
    actual: null,
    expected: null,
    margin: null,
    sensitivity: null,
    evidence: null,
    metadata: meta,
  };
}

const CONFIG: VerifyRunConfig = {
  suite: 'constraints',
};

// ── Tests ────────────────────────────────────────────────────────────

describe('useVerifyRunner — initial state', () => {
  it('starts idle with empty verdicts', () => {
    const { runner } = makeMockRunner();
    const { result } = renderHook(() => useVerifyRunner({ runner }));
    expect(result.current.state).toBe('idle');
    expect(result.current.verdicts).toEqual([]);
    expect(result.current.progress).toBeNull();
    expect(result.current.lastResult).toBeNull();
    expect(result.current.error).toBeNull();
  });
});

describe('useVerifyRunner — happy path', () => {
  it('idle → running → complete, final verdict list from runner result', async () => {
    const mock = makeMockRunner();
    const { result } = renderHook(() => useVerifyRunner({ runner: mock.runner }));

    let runPromise: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      runPromise = result.current.run(CONFIG);
    });
    expect(result.current.state).toBe('running');
    expect(result.current.progress).toEqual({ completed: 0, total: 0, caseId: null });

    // Mid-flight: emit a progress event so the hook surfaces it.
    const mockRunId = mock.currentRunId()!;
    act(() => {
      mock.emitProgress({
        runId: mockRunId,
        index: 0,
        total: 2,
        caseId: 'c1',
        verdict: pv('pass', { element_id: 'c1' }),
      });
    });
    expect(result.current.progress).toEqual({ completed: 1, total: 2, caseId: 'c1' });
    expect(result.current.verdicts).toHaveLength(1);

    // Resolve with the canonical result — the hook should replace the
    // in-progress accumulator with the authoritative list.
    const finalResult: VerifyRunResult = {
      verdicts: [pv('pass', { element_id: 'c1' }), pv('fail', { element_id: 'c2' })],
      durationMs: 123,
      summary: { pass: 1, fail: 1, inconclusive: 0, error: 0 },
    };
    await act(async () => {
      mock.resolve(finalResult);
      await runPromise;
    });
    expect(result.current.state).toBe('complete');
    expect(result.current.verdicts).toHaveLength(2);
    expect(result.current.lastResult).toEqual(finalResult);
    expect(result.current.progress).toBeNull();
  });
});

describe('useVerifyRunner — error path', () => {
  it('idle → running → error when the runner rejects (non-abort)', async () => {
    const mock = makeMockRunner();
    const { result } = renderHook(() => useVerifyRunner({ runner: mock.runner }));

    let runPromise: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      runPromise = result.current.run(CONFIG);
    });
    expect(result.current.state).toBe('running');

    await act(async () => {
      mock.reject(new Error('transport failure'));
      await runPromise;
    });
    expect(result.current.state).toBe('error');
    expect(result.current.error?.message).toBe('transport failure');
  });
});

describe('useVerifyRunner — idempotency', () => {
  it('calling run() while running cancels the prior run', async () => {
    const mock = makeMockRunner();
    const cancelSpy = vi.spyOn(mock.runner, 'cancel');
    const { result } = renderHook(() => useVerifyRunner({ runner: mock.runner }));

    let first: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      first = result.current.run(CONFIG);
    });
    expect(result.current.state).toBe('running');

    // Second call — must cancel the first.
    let second: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      second = result.current.run(CONFIG);
    });
    expect(cancelSpy).toHaveBeenCalled();
    // First promise resolves to null (cancelled -> idle, but we
    // transitioned to running for the second; final state becomes
    // complete once second resolves). We just need to not crash here.
    await act(async () => {
      // Resolve the second. First was already rejected with AbortError
      // by the mock's run() cancel behaviour.
      mock.resolve({
        verdicts: [pv('pass')],
        durationMs: 1,
        summary: { pass: 1, fail: 0, inconclusive: 0, error: 0 },
      });
      await Promise.allSettled([first, second]);
    });
    expect(result.current.state).toBe('complete');
    expect(result.current.verdicts).toHaveLength(1);
    expect(mock.runCalls).toHaveLength(2);
  });

  it('stale progress events from a superseded run are ignored', async () => {
    const mock = makeMockRunner();
    const { result } = renderHook(() => useVerifyRunner({ runner: mock.runner }));

    let first: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      first = result.current.run(CONFIG);
    });
    const staleRunId = mock.currentRunId()!;

    // Supersede.
    let second: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      second = result.current.run(CONFIG);
    });

    // A late event under the first (now-stale) run id must not
    // mutate visible state.
    act(() => {
      mock.emitProgress({
        runId: staleRunId,
        index: 10,
        total: 10,
        caseId: 'stale',
        verdict: pv('fail'),
      });
    });
    expect(result.current.verdicts).toEqual([]);

    await act(async () => {
      mock.resolve({
        verdicts: [pv('pass')],
        durationMs: 1,
        summary: { pass: 1, fail: 0, inconclusive: 0, error: 0 },
      });
      await Promise.allSettled([first, second]);
    });
    expect(result.current.verdicts).toEqual([pv('pass')]);
  });
});

describe('useVerifyRunner — cancel / reset', () => {
  it('cancel() transitions running → idle silently', async () => {
    const mock = makeMockRunner();
    const { result } = renderHook(() => useVerifyRunner({ runner: mock.runner }));

    let runPromise: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      runPromise = result.current.run(CONFIG);
    });
    expect(result.current.state).toBe('running');

    act(() => {
      result.current.cancel();
    });
    // The mock's cancel rejects the pending promise with AbortError,
    // which the hook must interpret as idle, not error.
    await act(async () => {
      await runPromise;
    });
    // Hook may briefly stay in 'running' if the cancel + reject race
    // lands after cancel's synchronous setState. Use waitFor.
    await waitFor(() => expect(result.current.state).toBe('idle'));
    expect(result.current.error).toBeNull();
  });

  it('reset() clears verdicts / lastResult / error', async () => {
    const mock = makeMockRunner();
    const { result } = renderHook(() => useVerifyRunner({ runner: mock.runner }));

    let runPromise: Promise<VerifyRunResult | null> = Promise.resolve(null);
    act(() => {
      runPromise = result.current.run(CONFIG);
    });
    const finalResult: VerifyRunResult = {
      verdicts: [pv('pass')],
      durationMs: 1,
      summary: { pass: 1, fail: 0, inconclusive: 0, error: 0 },
    };
    await act(async () => {
      mock.resolve(finalResult);
      await runPromise;
    });
    expect(result.current.state).toBe('complete');

    act(() => {
      result.current.reset();
    });
    expect(result.current.state).toBe('idle');
    expect(result.current.verdicts).toEqual([]);
    expect(result.current.lastResult).toBeNull();
    expect(result.current.error).toBeNull();
  });
});
